# Design review — `ORG-AUTHORITY-ARCHITECTURE.md` against the code

> **Status:** review finding 4 real defects in my own design. 2026-10-01.
> Every claim below is a grep or a file read against `origin/main` = `11c7c0f0`,
> with the command recorded so it can be re-checked.
> **Outcome:** the architecture is executable, but **not as written**. §4
> (persistent sessions) had to be rewritten from the ground up, and the
> sequencing changed as a result.

---

## Summary

| # | Defect | Severity | Effect on the plan |
|---|---|---|---|
| **D1** | There is exactly **one** `OperantAgent` with **one** `conversation: Arc<RwLock<Vec<Message>>>`. My §4 assumed a per-employee session slot that does not exist. | **blocking** | §4 rewritten; a new per-employee session substrate is now step 1 |
| **D2** | The scheduler **calls `clear_history()` before every job**. My §4 assumed history could survive between runs. | **blocking** | becomes the load-bearing refactor, not a detail |
| **D3** | Cron runs jobs **strictly sequentially**. My §11 assumed "six parallel meetings". | material | §11 fan-out becomes new work, not existing machinery |
| **D4** | `LlmCompressor::compress` needs a `&mut self` and an explicit `bind_persistence`. There is **no** compaction entry point on the agent. | material | §4.3 autocompact is new wiring, not a flag |

Plus one under-count: the Wave 1 test total I have been citing (83) is
**actually 68**. Corrected in §5.

---

## D1 — There is one agent and one conversation slot, not 102

**What I wrote (§4.1):** "each employee has at most one live `AgentSession`,
keyed by employee id... Every invocation loads that session, injects a fresh
volatile suffix, runs, and saves back."

**What the code says:**

```
$ grep -rn 'Arc<OperantAgent>' crates/ --include=*.rs
crates/operant-cli/src/gateway_runner.rs:427:    agent: Arc<OperantAgent>,
crates/operant-core/src/cronjobs/scheduler.rs:24:    agent: Arc<OperantAgent>,
```

Two owners, **one agent object**. And the conversation is a single field on
it (`agent/mod.rs:346`):

```rust
conversation: Arc<RwLock<Vec<Message>>>,
```

`set_session_id` (`events.rs:352`) sets a **string label**, not a slot. The
existing doc comment at `builders.rs:359` says this outright: *"so there is
exactly one session slot"*. `turn_context.rs:109` confirms it — the session id
is minted once and reused precisely **because** there is one slot.

**Consequence.** Today "session" in operant means *"the label under which this
one conversation is filed"*. My design assumed it meant *"a conversation that
survives between runs"*. Those are different things, and only the first exists.

So 102 employees cannot each hold a session — **they would all share one
conversation and interleave into each other's context.** That is not a
performance problem, it is a correctness problem: Rowan the HOD would see
content's messages in its own context.

**Fix.** A `SessionStore` keyed by employee, owning one conversation each, and
an `EmployeeRegistry` that hands out a *bound* agent per employee (its
conversation seeded from the store) rather than sharing one. The agent's
`conversation` field becomes the injection point: load-on-bind, save-on-release.

This is the single largest piece of work in the whole plan. I had it as "step
7". It is actually **step 1**, and everything that assumes an agent can be
addressed per-employee depends on it.

Note this is **not** the same as `operant-infra`'s `SessionStore`
(`session_store.rs:15`) — that one is JSONL for *channel* conversations with a
different `ChatMessage` type and a `compact()` that only strips corrupt lines.
Reusing it would drag channel semantics into the org layer. But it proves JSONL
session persistence with a per-key lock is already a solved shape in this repo.

---

## D2 — `clear_history()` destroys the conversation before every cron run

**What I wrote (§4.1):** sessions are "persisted across runs".

**What the code says** (`cronjobs/scheduler.rs:277`):

```rust
async fn run_agent_job(&self, job: &CronJob) -> (bool, String, String, Option<String>) {
    self.agent.clear_history().await;
    match self.agent.run(job.prompt.clone()).await {
```

`clear_history` **empties the vector** (`events.rs:169`). So every cron job
starts from zero context. This is deliberate — it is why cron prompts are
written to be self-contained — and it is the direct cause of GAP-2.7 (the
worklog is never fed), because there is nothing in context to write a log *from*.

**Consequence.** "Autocompact after the cron job runs" (§4.3) currently has
nothing to compact. The compaction must come **after** the per-employee session
substrate of D1, and the `clear_history()` call must become
`session.close_and_persist()` — scoped to that employee's session, not global.

**This is good news for the design.** It means the write barrier (§7) and the
autocompact (§4.3) both attach at a point that already exists and is already
central, rather than needing a new call site per invocation path. And
`clear_history()` is exactly the right place: it is the one function every
session-ending path already funnels through.

---

## D3 — Cron is sequential, so "six parallel meetings" is new work

**What I wrote (§11.1):** "That maps cleanly onto existing machinery", with a
table implying the CEO loop is mostly wiring.

**What the code says:**

```
$ grep -rn 'join_all\|FuturesUnordered\|tokio::spawn' crates/operant-core/src/cronjobs/scheduler.rs
(no output)
```

`tick()` iterates jobs in a plain `for` and `await`s each. Sequential.

**Consequence.** Two distinct problems:

1. The CEO loop's fan-out over HODs needs real concurrency (`tokio::join!` or
   `FuturesUnordered` over one thread per HOD). Without it, six meetings
   serialise — each is a full agent run with tool calls, so this is minutes,
   not seconds.
2. **This is the safety argument for the 3-turn thread budget (§10) landing
   early.** A sequential scheduler cannot deadlock, but once §11 adds
   concurrency, a bounded budget is what prevents N agents each waiting on a
   DM reply from another agent. So D3 *raises* the priority of step 5 in §14.

Note the contrast with tool execution, which **is** concurrent: an 8-worker
pool exists (iter-56). Concurrency in this codebase is a known pattern, so the
CEO fan-out is not exotic — it is a new *site*, not a new *technique*.

---

## D4 — Autocompact needs new wiring, and `compress` needs `&mut self`

**What I wrote (§4.3):** "the session is compacted. Always", phrased as though
the machinery were one call away.

**What the code says.** `LlmCompressor` (`llm_compressor.rs`) exposes:

- `bind_persistence(&mut self, database, session_id)` — line 169
- `should_compress(&self, estimated_tokens) -> bool` — line 298
- `compress(...)` — line 313, and it takes `&mut self`

And `grep 'pub async fn compact|pub fn compact' crates/operant-core/src/agent/*.rs`
returns **nothing**. There is no compaction entry point on the agent. The
compressor is constructed and bound by a host, not self-driven.

Also worth noting: `operant-infra`'s `SessionStore::compact` (`session_store.rs:89`)
is a *corruption stripper*, not a summariser. Two different things named
"compact". I should not have written "compaction is available" — the
summarising path exists but is not attached to the agent.

**Fix.** An `Autocompactor` on the agent that (a) holds the compressor,
(b) binds persistence to the employee's session id at bind time, (c) runs after
the write barrier, (d) honours the floor from §4.3. Because `compress` is
`&mut self`, the per-employee session object must own its compressor — which is
consistent with D1's per-employee session, so no new conflict.

---

## D5 — Correction: the Wave 1 org test total is 68, not 83

I have cited "83 org/core integration tests" in iter-529 and iter-530. Recounted:

```
$ for f in crates/operant-core/tests/org_*.rs; do
    echo "$f: $(grep -cE '^\s*(#\[test\]|#\[tokio::test)' $f)"; done
org_employee_registry.rs:     26
org_identity_gate.rs:          8
org_notice_board.rs:          26
org_worklog_framework_only.rs:  8
                             ────
                              68
```

**83 is wrong.** The 83 figure came from the iter-518 report and I propagated
it without recounting. The correction matters because §14 of the architecture
uses test counts as its acceptance evidence, and 68 is the honest baseline the
new tests are added to.

This is the second time in this program that I propagated a number from a
report instead of measuring it. Recording it so the pattern is visible.

---

## Revised sequencing

The old §14 had the session substrate at step 7. It is step 1 now. Everything
that assumed per-employee addressability moves behind it.

| # | Step | Status | Note |
|---|---|---|---|
| **1** | **Per-employee session substrate (D1)** | **new, first** | `EmployeeRegistry` + session-keyed conversation store + per-employee lock. Replaces `clear_history()` as the session boundary (D2). |
| 2 | Schema versioning helpers (§12) | was 1 | still unblocks every column add |
| 3 | Department membership + NULL surfacing (§3.1) | was 2 | Q4 |
| 4 | `departments` table + CLI (§3.3) | was 3 | mandate/protocols/rules |
| 5 | Recipient resolver (§3.2) | was 4 | keystone — board becomes readable |
| 6 | DM thread + 3-turn budget (§10) | was 5 | the loop guard; **priority raised by D3** |
| 7 | Write barrier (§7) | was 6 | attaches at the D1 session boundary |
| 8 | `Autocompactor` wiring (D4) | was 7 | needs the per-employee compressor from 1 |
| 9 | `subjective_log` + `org_decisions` (§8) | was 8 | builds on the barrier |
| 10 | Persona synthesis + frozen-prefix injection (§9, §6) | was 9 | |
| 11 | `reports_to` / `peers` / authority scopes (§1, §2) | was 10 | |
| 12 | Authority-filtered tool registry (§2.4) | was 11 | |
| 13 | `assignments` board + grant check (§5) | was 12 | first governed mutation |
| 14 | CEO loop + artifacts (§11) | was 14 | now includes the D3 fan-out work |
| 15 | `org check` exit contract (§11.4) | was 15 | |

**Net effect on effort.** The plan grew by roughly one step and moved its
heaviest item to the front. That is the correct outcome — the design is now
ordered by actual dependency rather than by conceptual tidiness, and the risk
(item 1 is large and touches the agent core) is front-loaded where it can be
de-risked early rather than discovered at step 7.

---

## What did **not** turn out to be wrong

Worth recording, so the review is not read as uniformly negative:

- **The write barrier (§7) is correctly placed.** `TurnEnd` already fires once
  per completed turn at a single chokepoint (`run.rs:1260`, via
  `TurnEndBus::emit(&session_id, iteration, total_tool_calls, &result.content)`),
  and it is *already* the seam reflection/advisor/dreaming attach to. The
  barrier attaches to an existing seam with the payload it needs.
- **`no_agent` script jobs are correctly out of scope.** `scheduler.rs:174`
  dispatches them to `run_script_job` and never touches the agent, so the
  barrier cannot accidentally apply to them.
- **The 3-turn budget is on the right object.** A thread-level shared budget
  survives the D3 concurrency finding; a per-notice cap would not.
- **`lru = "0.16"` is already a dependency** of `operant-channels` and
  `operant-runtime`. The warm-LRU in §4.2 needs no new dependency.

---

## Verification commands

```bash
grep -rn 'Arc<OperantAgent>' crates/ --include=*.rs          # D1: two owners, one agent
grep -n 'conversation: Arc<RwLock<Vec<Message>>>' \
     crates/operant-core/src/agent/mod.rs                     # D1: one slot
sed -n '350,365p' crates/operant-core/src/agent/builders.rs   # D1: "exactly one session slot"
sed -n '275,282p' crates/operant-core/src/cronjobs/scheduler.rs  # D2
grep -rn 'join_all\|FuturesUnordered\|tokio::spawn' \
     crates/operant-core/src/cronjobs/scheduler.rs           # D3: no output
grep -n 'pub async fn compact\|pub fn compact' \
     crates/operant-core/src/agent/*.rs                       # D4: no output
grep -n 'bind_persistence\|should_compress\|pub async fn compress' \
     crates/operant-core/src/agent/llm_compressor.rs          # D4
for f in crates/operant-core/tests/org_*.rs; do \
    echo "$f: $(grep -cE '^\s*(#\[test\]|#\[tokio::test)' $f)"; done   # D5: 68
```

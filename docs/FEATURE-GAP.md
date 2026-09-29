# Feature gap — cognition loop, across four reference projects

Written 2026-09-29, iter-486. Sources measured, not assumed:

| project | state it was read at | how |
|---|---|---|
| hermes-agent | `origin/main` @ `7728574` | `git ls-tree` / `git show` / `git grep` — working tree untouched (it has your 2 dirty files) |
| zeroclaw | `origin/master` | same; extracted read-only via `git archive` to `/tmp/zc-master` |
| oh-my-pi | working tree, current | — |
| jcode | working tree, current | TUI half still running, §6 |
| **operant** | `9b5b1240` | the thing being compared against |

The comparison that matters is not "does hermes have X" but **"does X exist
anywhere, and is it wired"** — because two of the four features you named turn
out to be absent in *every* reference project, and one exists in operant but
dead.

---

## 1. The four features you named

| feature | hermes-agent | zeroclaw | **operant** |
|---|---|---|---|
| **prewalk** | **ABSENT** — 0 hits for `prewalk`/`pre_walk` | **ABSENT** — `api/plan.rs` is data types only; its sole production consumer is `tools/todo_write.rs`, which **the model** calls mid-loop | **ABSENT** |
| **advisor** | present, but as **MoA reference-advisors** (excluded) plus `review_engine.py` — a user-invoked background subagent | **ABSENT** — `critique` appears only as a keyword string in `eval.rs:34` | MoA only |
| **reflection** | **PRESENT, WIRED** — the strongest find in this audit | present but **DEAD** (`evaluate_response`: 0 external callers) | **ABSENT** |
| **dreaming** | **PRESENT, named "curator"** | **PRESENT** — inline consolidation + idle heartbeat | **ABSENT** |

### 1.1 prewalk does not exist. Anywhere.

This is the most consequential negative result in the audit, and it is worth
being blunt about: **prewalk is not a port.** Neither reference project has it.

Two specific traps, both of which would have wasted your time:

- **Do not read hermes' `turn_preflight.py` as prewalk.** It is a
  *context-compression* gate (`ContextWindowExceeded` handling), nothing to do
  with planning. The name collides and the collision is a trap.
- **zeroclaw has `plan.rs`, but it is not a planner.** It defines
  `PlanEntry`/`PlanStatus`/`PlanPriority` data types; the only production
  consumer is a `todo_write` tool the *model* chooses to call. That is a
  todo-list feature, not a pre-execution phase.

What exists instead, in both, is the closest adjacent thing:
- hermes: `/plan` — a **user-invoked** slash command injecting plan-mode rules
  for one turn (`plan_prompt.py:11 _PLAN_MODE_RULES`: no edits, deliverable is
  a markdown plan). Manual only.
- hermes: `kanban_decompose.py` — an LLM task-graph decomposer, but wired to
  the Kanban triage queue, not the turn loop.
- zeroclaw: `should_execute_tools_in_parallel` serialises the batch when >1
  file mutation or any call needs approval — a *serialisation* policy, not a
  planning pass.

**So prewalk is greenfield design, and the design question is open.** The
version that would be worth building is not "a plan step" (hermes has that, on
demand) but a **pre-execution reconnaissance pass**: before the first tool
call, gather just enough state to make the first tool call correct — read the
relevant files, check the tool surface is present, surface the project
conventions. That is the thing that would prevent the class of failure a
self-correction loop then has to clean up.

### 1.2 reflection — hermes has it, operant does not, and this is the big one

`hermes-agent/agent/background_review.py` is a genuinely good design and it is
the thing worth porting first.

**Mechanism.** After each turn, the turn finaliser may spawn a daemon thread
that forks a fresh `AIAgent`, replays the conversation snapshot into it, and
asks one question: *should any skill or memory be saved or updated?* Writes go
straight to the memory and skill stores. The main conversation and its prompt
cache are never touched.

**Why it is well-built:**
- The fork inherits the parent's provider, model, credentials and cached system
  prompt, so it hits the same prefix cache instead of paying to rebuild one.
- It runs under a **dispatch-side tool whitelist** — it physically cannot
  touch the terminal or the user's files.
- It is spawned from `turn_finalizer.py:757-767`, i.e. **after** the user-visible
  response is delivered, so self-improvement never adds latency to the answer.
- `review_idle_queue.py` defers the fork to machine-idle on the managed local
  runtime, to avoid GPU contention with the live turn.

**Triggers.** Memory review every `memory.nudge_interval` user turns (default
**10**). Skill review every `skills.creation_nudge_interval` tool iterations
(default **10**). Both configurable, both fail-open.

**Also worth having:** `/refine` as a manual trigger for the same machinery,
and cron suppressing it explicitly (`scheduler.py:2442`, "~30K tok/event").

**Two limits you should know before scoping.** It reflects *into memory and
skills* — it does **not** critique-and-revise the current turn's answer. And
there is **no self-critique of the current reply anywhere in hermes** either:
grep for `critique`/`self-critique` in non-test Python returns two hits, both
a *prohibition* in a prompt ("Do NOT critique the main task").

### 1.3 dreaming — hermes calls it "curator", and it is idle-time by design

`hermes-agent/agent/curator.py` (`maybe_run_curator()` at `:1198`).

Inactivity-triggered, no cron daemon. When the agent has been idle longer than
`interval_hours` (default **7 days**), it auto-transitions skill lifecycle
states from activity timestamps — stale at 14 days, archive at 30 — and
optionally forks a real `AIAgent` (`_run_llm_review()` at `:1085`,
`max_iterations=9999`, toolset `skills` only) that may pin, archive,
consolidate or patch skills via `skill_manage`. State lives in `.curator_state`.
LLM consolidation is **opt-in** (`DEFAULT_CONSOLIDATE = False`).

Called from three idle-gated sites (gateway poll, TUI runtime mixin, web server
sessions). CLI: `hermes curator status|run|pause|pin`.

**zeroclaw's version is the more interesting one to port**, because it splits
the two concerns that hermes conflates:

- `zeroclaw-memory/src/consolidation.rs:60` —
  `consolidate_turn(provider, model, memory, cfg, user_msg, assistant_resp)` is
  an LLM turn-summariser emitting `{history_entry, memory_update, kind, facts,
  trend}`, with a 3-phase write (Episodic history → gated typed-facts → Core)
  behind `policy_gate::validate_store` and `dedup::dedup_gate`. Wired at
  `channels/orchestrator/mod.rs:10394` (detached) and `gateway/ws.rs:2163`.
- `zeroclaw-runtime/src/heartbeat/engine.rs:161` — a genuinely idle-time
  task runner: parses runnable tasks from a `HEARTBEAT.md`, builds a decision
  prompt, parses chosen indices, and `compute_adaptive_interval` backs off when
  nothing is ready. Spawned under `if config.heartbeat.enabled`.

**operant already has the substrate for the second one**: `heartbeat/` and
`periodic_scheduler` exist in `operant-runtime`. So the idle-time *runner* is
largely there and what is missing is the **dreaming payload** — something for it
to do.

---

## 2. The cheapest real win: operant already owns a dead self-critique

This is the single highest-leverage finding in the audit, and it is not a port
at all.

zeroclaw has a 4-heuristic response evaluator at
`zeroclaw-runtime/src/agent/eval.rs:115`:

```rust
pub fn evaluate_response(...) -> EvalResult   // { score, checks, retry_hint }
```

Weighted checks — non-empty (.30), not-a-cop-out (.25), sufficient length
(.20), code-presence (.25) — and it emits a `retry_hint` when
`score <= EvalConfig::min_quality_score`. That is a self-critique-and-revise
mechanism, and it is **dead in zeroclaw**: `evaluate_response` has 7 hits
tree-wide, all inside `eval.rs` (1 definition, 1 comment, 5 self-tests). The
`retry_hint` never reaches a regeneration path. Only its *sister* function,
`estimate_complexity` (`eval.rs:38`), is live.

**operant has the identical split.** Verified independently before publishing,
because this is the load-bearing claim of the whole document:

- `operant-runtime/src/agent/eval.rs:131` `evaluate_response` — matches at its
  definition and at `:287, 293, 308, 323, 346, 360`, and **every one of those is
  inside `eval.rs`'s own test module** (which begins at the
  `// ── evaluate_response ───` marker on `:283`). **Zero external callers.**
- `operant-runtime/src/agent/eval.rs:90` `EvalResult` — same file only, no
  external use.
- `operant-runtime/src/agent/eval.rs:45` `estimate_complexity` — **live**,
  called at `operant-runtime/src/agent/agent.rs:1456`.

**A trap worth naming, because I nearly walked into it:** there are **two**
`EvalResult` types in the tree. The live one is
`operant-runtime/src/skillforge/evaluate.rs:47`, used at
`skillforge/mod.rs:15,98,173` — and it scores **skills**, not responses. A
grep for `EvalResult` looks like it shows a live type and would be read as
disproving the deadness of `agent/eval.rs`'s. They are unrelated.

So operant ported zeroclaw's `eval.rs` **including its dead half**, and has been
carrying an unused self-critique scorer ever since. Every comment in the file
says "No LLM call" — which is the right design for a cheap gate, and also why it
was never noticed as dead: a pure function with no callers produces no warning
that anyone reads.

**The work is one call site.** On turn finalisation, if `score <= threshold`,
inject the `retry_hint` as a synthetic user-role nudge and continue the turn —
which is structurally the same mechanism zeroclaw's `verification_stop.py`
already uses for evidence-gating, and which operant can therefore copy from
*itself* rather than from either reference project.

**Cost:** tens of lines. **Payoff:** it converts the second-largest gap in §1
into a working feature immediately, and it is a genuine self-critique loop,
which §1.2 established *nobody* has.

---

## 3. Two Rust defects worth fixing regardless of any feature work

These came out of the zeroclaw engineering-patterns pass and are not
cognition features. Both are concrete and cheap.

### 3.1 Six unguarded stream-parser tasks (real leak)

`tokio::spawn` + `stream::unfold` **leaks the spawned task and its socket**
whenever the consumer drops the stream early — turn cancel, timeout, client
disconnect. zeroclaw wrote a 20-line guard for exactly this
(`stream_guard.rs:9`): an `AbortOnDrop` whose `Drop` calls `AbortHandle::abort()`,
carried *inside* the returned stream's `unfold` state so abort fires exactly
when the consumer drops it, and a no-op once the task has finished.

Measured in operant:

| file | un-guarded `tokio::spawn`+`unfold` pairs |
|---|---|
| `operant-providers/src/compatible.rs` | **6** (L1216/1293, 1314/1472, 2313/2429, 2454/2560, 2582/2654) |
| `operant-providers/src/anthropic.rs` | **1** (L1132/1174) |
| `operant-providers/src/openrouter.rs` | 0 — **already has `AbortOnDrop` at `:57`** |

operant wrote the guard, applied it to one provider, and left the other seven
sites leaking. This is a bug, not a style preference.

### 3.2 Tool fan-out ignores write-conflicts and approval ordering

zeroclaw encodes the decision as one pure predicate,
`should_execute_tools_in_parallel` (`tool_execution.rs:~500`): serialise when
more than one file mutation is present, or when **any** call needs approval, so
the CLI's prompt/deny policy stays consistent.

operant runs an unconditional 8-worker concurrent pool (iter-56). Two
consequences, both real: two `write_file` calls in one batch can race, and
approval prompts can interleave out of order.

**Cheap, and the existing 8-worker pool is not the thing that has to change** —
the predicate in front of it does.

---

## 4. A third pattern, worth taking but not urgent

**Background review placement.** zeroclaw mines failed skill slugs from history
and forks `maybe_run_skill_review` in a background task placed *after* the
user-visible response (`loop_.rs:2336-2360`). The placement is the entire
trick — self-improvement that blocks the user gets dropped, and self-improvement
that delays the user is worse than none.

operant has `operant-core/src/agent/background_review.rs` (1,037 lines). Worth
checking whether it already has the after-emit placement; if it fires *before*
the response, that is a latency bug, not a feature.

---

## 5. What I would build, in order

1. **Wire operant's dead `evaluate_response`** (§2). Tens of lines. Gives a
   real self-critique-and-revise loop that no reference project has. Do this
   first — it is the cheapest real capability in this entire audit.
2. **`AbortOnDrop` across the remaining seven stream sites** (§3.1). Small,
   fixes a leak.
3. **The parallel-execution predicate** (§3.2). Small, fixes a correctness gap.
4. **Background reflection forked post-response** (§1.2), modelled on hermes'
   `background_review.py`: default-10 turn nudge, tool-whitelisted fork,
   prefix-cache reuse, idle deferral. This is the real port.
5. **Dreaming**, split as zeroclaw does (§1.3): a `consolidate_turn` for memory
   and a payload for the heartbeat runner that already exists.
6. **Prewalk** (§1.1) — greenfield, design first. Do not start it before 1–5,
   because a self-correction loop plus a reconnaissance pass is a much stronger
   combination than either alone, and 1 is nearly free.

---

## 6. TUI — pending

jcode's terminal-UI inventory is still running and lands here as §6. The
operant TUI is `ratatui`, ~57k lines across 50+ modules under
`crates/operant-cli/src/tui/`, so the question is which interaction ideas are
worth reimplementing, not what to port.

---

## 7. Corrections I made while doing this

- **"sourcehound has no git remote"** — false, already corrected at iter-484.
  It does, as does memory-wire.
- **"AFT is not integrated"** — false. **20 tools**, `aft_enabled: true` by
  default. My "0 registered tools" was a wrong grep pattern.
- **"AFT is missing 16 tools we don't wrap"** — I produced this from a `name:`
  grep and it was **garbage**: the "not-wrapped" list was Rust identifiers
  (`constructor`, `helper`, `foo`). Discarded. Whether operant is behind AFT is
  genuinely **unverified** and needs a real read of AFT's tool definitions.
- **"hermes and zeroclaw are synced"** — they are not. hermes-agent is
  **21,698 commits** behind `origin/main`, zeroclaw **2,205** behind
  `origin/master`. I did not stash your uncommitted work; both were read from
  the remote via git plumbing, so the analysis is against current upstream.
- **My first feature grep was too broad to conclude anything** — 249 matches
  across 95 files for a pattern containing `journal`, `experiment`, `critic`,
  `reflect`, `dream`, almost all matching comments and SQLite. Replaced with
  one-term-at-a-time checks, which is how the real zeros (`prewalk`,
  `dreaming`, `hypothesis`) were found.

## 8. Verification limits

Stated so the next agent does not over-trust this:

- The `hermes-agent` and `zeroclaw` audits were grep-bounded, not exhaustive
  reads of 7,971 and 2,165 files respectively. Absence claims are strong but
  not proofs.
- zeroclaw's `delegate.rs` (18.7k), `loop_.rs` (22k) and
  `orchestrator/mod.rs` (50k) were sampled, not fully read.
- Whether `zeroclaw eval` is a shipped subcommand or dev-only is **unverified**.
- The four load-bearing claims in the zeroclaw report (dead `evaluate_response`,
  0-ref `routines`, unregistered `VerifiableIntentTool`, live `reserve()` sites)
  were each independently re-verified before being repeated here. The
  `evaluate_response` one is the load-bearing claim of §2.

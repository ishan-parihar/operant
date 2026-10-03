# Organism OS — next implementation outline

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `1cb88511` (iter-551)
**Supersedes**: the 2026-10-01 revision of this same file, written at iter-541.
That revision opened with "Step A is the blocker - not started"; iter-548
resolved it. Its "Predecessor" line pointed at this file, which is a
self-reference and is what this revision removes.

Every claim below was re-verified against the code at `1cb88511` on 2026-10-03,
not carried over. Where the predecessor said a thing was unbuilt and it now
exists, that is recorded rather than silently renumbered.

**Corrections in this revision** (the iter-552 text shipped three errors):

1. Step A is **not** on a run path. Only the gateway adopted iter-548.
2. `WriteBarrier::apply` is **synchronous**; the iter-552 text wrote
   `await WriteBarrier::apply(...)`, which does not compile.
3. This file's predecessor line self-referenced.

---

## 0. What iter-550 and iter-551 actually changed

Both packets had been sitting as **untracked files** — never committed, and one
of them never even compiled.

`session/autocompact.rs` was an orphan: `lib.rs:110` declares `pub mod session`,
but `session/mod.rs` never declared `mod autocompact`, so the file was outside
the module tree. `cargo check --lib` passed *because it never saw it*.
`tests/session_autocompact.rs` failed `E0432`. iter-550 added the declaration;
the 766 lines type-checked for the first time and 15 tests now run.

`write_barrier.rs` shipped two failing tests, both test-side:

- `a_run_that_changed_the_organisation_writes_its_decision_objects` — the
  `authorless_decision` helper passed its meeting note into the 6th argument of
  `OrgDecision::new(id, subject, scope, decided_by, rationale, reason)`, i.e.
  the `reason` slot. It was therefore non-blank and the substitution at
  `write_barrier.rs:701` correctly declined to fire. Fixed the helper.
- `the_barrier_leaves_the_kanban_user_version_untouched` — dropped the fixture,
  and with it the `TempDir`, before reopening, so it asserted on a
  `SqliteFailure` rather than on `user_version`. Removed the drop; a second
  `Connection::open` on a live file is valid SQLite.

iter-551 then repaired forward a defect iter-550 introduced: it committed
`tests/org_authority_tools.rs` but not the `tools.rs` implementation the test
imports, publishing a test target that could not compile. Force-pushing `main`
is forbidden, so it was fixed in a following commit rather than amended.

**Current measured state:** 2235 lib tests pass and **2 fail** —
`tools::kernel::{ping_roundtrip, harness_apply_and_rollback_roundtrip}`. Those
two are **pre-existing debt at iter-549**, unrelated to this work: the same two
fail in a clean worktree at `origin/main` (measured 2197 passed / 2 failed), no
`tools::kernel.rs` file exists, and neither iter-550 nor iter-551 touches
`tools::kernel`. They deserve their own iteration; they are not absorbed here.

Do not read the working tree's 2237/0 as a repair. That run had the
unstaged `Cargo.toml` `[profile.dev]` change applied, which alters the build;
the committed tree measures 2235/2.

Plus 8 `agent_session_isolation`, 6 `org_authority_tools` and 15
`session_autocompact`, all green in a clean worktree at the pushed commit.

---

## 1. The honest status table

"Built" here means *compiled and tested*. It does **not** mean *on a run path*.

### 1.0 Read this first — the top item is a live defect, not a planned one

**A cron agent job destroys a gateway user's live conversation on `origin/main`
today.** This is filed as `BUGS.md` **D-1 (P1, unowned)** and it outranks every
step below. The distinction matters: steps B-F are unwired subsystems that only
matter once someone wires them, whereas D-1 is reachable by ordinary
configuration — run a gateway adapter with at least one enabled agent cron job.

The mechanism, in one line: `scheduler.rs:277` calls `clear_history()` on the
**same `Arc<OperantAgent>`** the gateway chats on
(`gateway_runner.rs:1082` → `:1094`), and `clear_history` clears the hot
conversation (`events.rs:200`) **and** discards the persisted transcript from the
`SessionStore` (`events.rs:205-206`). The gateway does not rehydrate: the block at
`gateway_runner.rs:569-583` only tracks state, and the real rehydration lives
inside `set_session_id`, whose swap is conditional on the id changing
(`events.rs:379`).

Two consequences that shape the rest of this outline:

- **It is unrecoverable.** `sessions.discard` deletes the rows, so switching the
  conversation away and back does not restore it. Full transcript loss.
- **Step A′ is the fix, not merely the prerequisite.** This re-ranks A′ from
  "first of six planned steps" to "P1 remediation".

### 1.1 The planned work

| Step | Built | On a run path | Note |
|---|---|---|---|
| A per-employee session substrate | ✅ iter-548/570 | ✅ **gateway + cron** | 8 isolation + 4 cron tests |
| B write barrier | ✅ iter-550 | ❌ **no caller** (wave-1 slice D) | 38 inline tests |
| C autocompaction | ✅ iter-550 | ❌ **no caller** | 15 tests |
| D hierarchy + §2.4 tool authority | ✅ iter-550/551 | ❌ **no caller** (wave-2 slice E) | 6 + inline tests |
| G permission genome — `decide()` engine | ✅ iter-573 | ❌ **no caller** (wave-2 slice E) | 10 adversarial tests |
| E assignment boards | ❌ | ❌ | absent |
| F concurrent CEO loop | ❌ | ❌ | absent |

**A′ is landed** (iters 570–571): cron runs its own `OperantAgent`
(`gateway_runner.rs:1126`, built via `create_runtime_agent_with(.., false)` so it
does not spawn a duplicate LCM maintenance pair), targets a stable per-job
session via `set_session_id(derive_employee_id(&job.id))` at
`scheduler.rs:290-291`, and the compressor reset lives in `set_session_id`'s
retarget guard. Four cron-isolation regression tests drive the real
`tick → run_job → run_agent_job` path
(`tests/cron_session_isolation.rs`). The previously-shared-agent constraint
this outline carried is resolved.

What remains unwired is now: the barrier mount (B′, wave-1 slice D), the
permission genome's run-path caller (wave-2 slice E), and the hierarchy
consumer (wave-2 slice F). The gap ledger predates all of this and its Wave
2-4 rows are stale; this outline supersedes its status column, not its
findings.

---

## 2. Step A′ — give cron a per-job session

This was not in the previous revision. It should have been.

### Blocker first: cron and the gateway share one agent

Before any of the plumbing below, a constraint this revision originally hid.
The scheduler does not own its agent. It is handed one:

```
scheduler.rs:24           agent: Arc<OperantAgent>
gateway_runner.rs:1082    let cron_agent = agent.clone();
gateway_runner.rs:1068    agent = create_runtime_agent(...)
gateway_runner.rs:1081    let agent = Arc::new(agent.with_permissions(permission_tx))
gateway_runner.rs:427     GatewayMessageHandler { agent: Arc<OperantAgent>, ... }
```

`cron_agent` is a clone of the **same `Arc`** that `GatewayMessageHandler` chats
on, and the handler already calls `set_session_id` at `:544` and `run()` at
`:655`.

So A′ as originally written — "call `set_session_id` in `run_agent_job`" — would
retarget an agent out from under a live user. A cron job ticking mid-conversation
would swap the hot conversation slot and rehydrate from disk
(`events.rs:379-397`), destroying the user's in-flight context. This is the D2
defect's cousin: not leakage between cron jobs, but **cron and the gateway
fighting over one session pointer**.

**Constraint: A′ must not call `set_session_id` on the shared agent.** Two ways
out, and this is the owner's call:

1. **Give cron its own `OperantAgent`.** Clean, and removes the whole class of
   problem. Cost: a second agent means a second model client, tool registry and
   memory provider at construction (`create_runtime_agent` is the only builder).
2. **Prove the paths do not overlap at runtime** — that the gateway is not
   serving traffic while the scheduler ticks. That is an operational assumption,
   not a property of the code, and a busy Telegram gateway almost certainly
   violates it.

Recommendation: (1). Until one is chosen, A′ is not implementable, and the
stable-id / compressor-reset decisions below stand but have no mount point.

### Option (1) is NOT the cheap fix — two side effects need an explicit decision

The cost note above is incomplete, and both omissions change what option (1)
means. Verified against current `origin/main`; filed as `BUGS.md` D-2.

**1a. Permissions fail OPEN, not closed.** This is the sharp one. `permission_tx`
is `Option<mpsc::Sender<ToolPermissionRequest>>` (`agent/mod.rs:361`) and
defaults to `None` in both builders (`builders.rs:67`, `:134`);
`with_permissions` (`builders.rs:356-359`) is the only setter. The guard at
`agent/stream.rs:737` is literally `if let Some(ref permission_tx) =
self.permission_tx` — an Option test, not a call. With it `None` the whole block
is skipped and control falls through to `pending.push(...)` at `stream.rs:838`,
the queue for actual execution. **There is no fail-closed default anywhere in
that path.** A cron agent built without an explicit `with_permissions` runs
unattended with no approval gate at all.

The "smart" approval mode (`approval.rs:608`, default per `mod.rs:177`) does not
rescue it. Its dangerous-pattern verdict only logs
`warn!(tool = %name, "Tool call flagged — will prompt user")` at
`stream.rs:730` and then falls through — it prompts nobody, because in an
unattended job there is nobody to prompt. So the "obvious" implementation
(don't wrap it, matching the gateway's interactive UX) silently gives unattended
cron jobs `bash` and `file_write` with no approval.

**Decision owed**: what permissions an unattended cron agent gets. This must be
an explicit call, not the `None` fallback.

> **Correction — the obvious "fail-closed" fix does not work.** An earlier
> revision of this file recommended wrapping cron in a permission channel with no
> responder and relying on deny-on-timeout. Read the code and that recommendation
> is wrong: the timeout is real and long. `stream.rs:788-791` is
> ```rust
> let response = tokio::select! {
>     r = resp_rx => r.unwrap_or(ToolPermissionResponse::Deny),
>     _ = tokio::time::sleep(Duration::from_secs(120)) => ToolPermissionResponse::Deny,
> };
> ```
> A channel nobody drains therefore does not fail closed promptly — it **stalls
> 120 wall-clock seconds per dangerous tool call and then denies**. On a cron
> job that is a silent two-minute stall per tool, for every job, forever. The
> fail-closed options are therefore "deny without prompting at all" (needs a new
> code path, not a channel) or "accept the stall" — and neither is a free
> choice. **This is an owner decision with real cost on both sides, not a
> detail.**

**1b. A second Ctrl-C handler.** `create_runtime_agent` spawns an unconditional
`tokio::signal::ctrl_c()` handler at `main.rs:1754-1761` — no config gate, no
feature flag, and the `JoinHandle` is dropped, so it is fire-and-forget. A second
call registers a second listener.

Verified nuance, because the raw claim overstates it: this is a **broadcast**,
not a contested single-slot handler. tokio's `signal_hook_registry` fans SIGINT
out to every registered listener, so the blast radius was already "every
listener" before A′ existed, and the gateway already registers further
process-level listeners (`cmd_gateway.rs:392-396`). A second agent therefore does
not *widen* Ctrl-C's reach. Record it as a cost, not a new hazard.

### The plumbing, once that is settled

`run_agent_job` has no session identity at all: it calls
`self.agent.clear_history().await` at `:277` and then `self.agent.run(...)`. The
defect is **the absence of per-job identity, not cross-job leakage** — the wipe
at `:277` means no job inherits another's turns today, so all 102 scheduled
jobs share one slot but none contaminates another. That wipe is also the only
reason the bug is invisible, and it exists *only* because the slot is global.

The consequence is that `clear_history()` at `:277` should be **deleted** once
A′ lands, not preserved. Under iter-548 it is scoped to the addressed session
and drops it from the store; the gateway's wipe-and-reload was removed for
exactly this reason. Keeping the line after A′ would leave the implementer
guessing whether it still means "start the org from nothing".

iter-548 built the substrate to give every run its own session and cron never
adopted it — `set_session_id` is called only by the gateway
(`gateway_runner.rs:544`).

The job id is already the employee identity in this codebase:
`derive_employee_id(&job.id)` is used by the org gate at `scheduler.rs:117`.
So the whole of A′ is threading that existing value into
`agent.set_session_id(...)` before `run()` and deleting `:277`. **No schema
change, no new derivation — plumbing, not design.**

The shape:

```
1. reuse derive_employee_id(&job.id)  (already called at :117 for the gate)
2. agent.set_session_id(<STABLE per-job session id>) before run
3. delete the clear_history() at :277
4. reset the LLM compressor inside set_session_id, on session change
```

### The coupled decision: stable-per-job id, and the compressor reset

Steps 2 and 4 are one decision. Leaving the id as a bare `<per-job session id>`
placeholder would let an implementer silently settle three things at once.

**Stable per job, not fresh per run.** The session id must be a function of the
job (or employee), not a uuid minted per invocation, so the employee's
transcript accumulates across its runs. Fresh-per-run would start every run
empty and:

- destroy employee continuity — the whole point of iter-548;
- make C′ dead code, because a session that starts empty never reaches the
  4,000-token floor (§4).

**The compressor reset must move into `set_session_id`.** `clear_history` resets
the compressor for a stated reason (`events.rs:208-213`): "Without this, a
previous session's summary would bleed into the new session's compression
context." Dropping `:277` without moving that reset reintroduces exactly the
bleed A′ exists to prevent — job A's summary leaking into job B's compression.
`set_session_id` swaps the conversation and persists the outgoing transcript
but **never touches the compressor** (verified: no compressor or memory call in
`events.rs:365-405`).

So the reset belongs where the session actually changes, which is
`set_session_id`, guarded by the same `previous != new_id` condition that
already gates the conversation swap.

**The reset is not a no-op on cron — verified, not assumed.** The compressor is
`Option<Mutex<LlmCompressor>>` (`agent/mod.rs:475`), so the reset only fires if
one is attached. The construction path: the scheduler receives
`Arc<OperantAgent>` (`scheduler.rs:24`); the only caller passes a clone of the
gateway's agent (`gateway_runner.rs:1082`, from `cron_agent = agent.clone()`);
and that agent is built by `create_runtime_agent` (`main.rs:1719`), which
attaches a compressor at `main.rs:1775`. So the cron agent **does** carry one,
and dropping the reset would be a real regression rather than dead code.

### Deciding the memory session-boundary event

Step 3 is not a pure deletion, and the implementer should know that before
making it. `clear_history()` is not only a wipe. At `events.rs:175-198` it
snapshots the conversation and fires a **memory session-boundary
notification**: `submit_session_end` when the sync executor is available, else
`provider.on_session_end(&snapshot)`. It then resets the LLM compressor and
calls `provider.on_session_switch(&old_id, &old_id, true)` at `:220`.

An earlier revision of this document claimed the gateway "still fires it via
`set_session_id`". **That was wrong.** `set_session_id` contains no memory call
at all, and `gateway_runner.rs` has no `on_session_end` / `submit_session_end` /
`on_session_switch` — its three `clear_history` mentions are comments recording
that the call was removed there. `clear_history` has exactly two live callers,
`main.rs:2447` (`/new`) and `scheduler.rs:277`, so **deleting `:277` removes the
memory boundary from the cron path specifically, and from nowhere else.**

**Decision: do not preserve it on cron.** One `on_session_end` per scheduled
job is noise — each job would close a session the organ never opened, and memory
would accumulate a boundary per job per run. `/new` (`main.rs:2447`) keeps it,
which is where a genuine boundary exists. If memory later wants run
boundaries, that is a different event with a different name, not this one.

**Record the graph-structure consequence.** Today memory receives one boundary
per cron job per tick — roughly 102 events per tick across ~133 employees.
After A′ lands it receives none from cron. That is a deliberate change to the
memory graph's shape, not a silent deletion.

**Does anything downstream read run-level boundaries as signal? Checked 2026-10-03.**
Enumerated every `MemoryProvider` implementation rather than sampling one:

- `BuiltinProvider` (`memory_provider.rs:379`) — overrides it with a debug log
  only.
- `PluginMemoryProvider` (`plugin_memory.rs:107`) — does **not** override it, so
  it inherits the no-op trait default (`:298`).
- `MemoryWireProvider` — the **default** provider per AGENTS.md. It does not
  override it either. Its own crate
  (`~/.cargo/git/checkouts/memory-wire-*/src/`) has no `on_session_end` method at
  all; the single textual hit is a plugin-manifest test listing supported hook
  *names*, not an implementation.
- `runtime/hooks/traits.rs:35` — an unrelated channel-hook trait this path never
  calls.

So the boundary event reaches no logic on any provider today. That is the basis
for dropping cron's, and it was worth checking properly: an earlier revision of
this document generalised from `BuiltinProvider`, which is the *fallback*, not
the default, which would have made the safety argument about the wrong
implementation.

**Why first:** B′ must attach the barrier to a session, and the barrier writes a
worklog row keyed by session. Without a per-job session key there is nothing to
key it to, and the previous revision's claim that the mount point already held
"the run outcome and the session key" was wrong on the second half.

---

## 3. Step B′ — mount the write barrier

The barrier has two entry points, deliberately distinct
(`write_barrier.rs:743`):

- `WriteBarrier::apply(&request)` — the **postcondition**. Returns `Err`, and
  the caller fails the run. This is the one §11 wants.
- `WriteBarrier::attach_subscriber(...)` — the **passive observer**. Cannot fail
  a run; records to a `BarrierFailureLog`.

A `TurnEndBus` subscriber cannot make a run fail — that is why these are two
functions. So the postcondition has to be called by the run path itself, not
merely observed.

**Mount point:** `run_agent_job` (`cronjobs/scheduler.rs:274`). It already has
the run's outcome in hand as `(success, output, final_response, error_msg)`.

`apply` is **synchronous** — `pub fn apply(&self, request: &WriteBarrierRequest)
-> Result<BarrierReport, Error>` at `:636`. The previous revision wrote
`await WriteBarrier::apply(...)`, which does not compile. Only
`Autocompactor::compact_summarising` (`:448`) is async.

The shape is roughly:

```
1. build WriteBarrier::for_app(&db_path)
2. WriteBarrier::apply(&request)  after self.agent.run(...) returns   [no await]
3. on Err: the run is a failure regardless of `success`
```

Constraint from the design that must be honoured: the barrier writes the
**objective** worklog row, **subjective** reasoning, and **decision** objects.
A′ removes the `clear_history()` at `:277`, so by the time B′ runs the barrier
reads a session holding exactly this run's turns. The two steps are one change
in sequence, not two independent wires.

**Why this before C or E:** the design's own sequencing says B before D,
otherwise the first governed action in the system is also the first
*unauditable* one. D's enforcement point is built and waiting; B is what makes
a governed action attributable.

**Verification that matters:** a scheduled run that completes must leave
exactly one worklog row; a run whose barrier write fails must report failure
and must not be recorded as successful.

---

## 4. Step C′ — autocompact after scheduled execution

`Autocompactor::compact(&key)` (`session/autocompact.rs:429`) and the
summarising variant `:448`. It takes an `Arc<SessionStore>` and an
`Arc<Database>` — both already exist after iter-548.

The design wants compaction **after scheduled execution**, with a
**compaction floor** so short sessions are not reduced to a
summary-of-summary. That floor is implemented: `COMPACTION_FLOOR_TOKENS = 4_000`,
`KEEP_HEAD_MESSAGES = 3`, `KEEP_TAIL_TOKENS = 8_000`.

This is the same `run_agent_job` seam as B′, and the order matters: **compact
after the barrier has read what the run produced**, or the barrier is
compacting against a transcript it is about to write a row about.

### C′'s viability depends on A′'s id choice

`preflight` gates on exactly one thing (`autocompact.rs:501`):

```
if tokens_before <= self.config.floor_tokens { return SkippedByFloor }
```

There is no structural precondition — no message count, no run count, nothing.
The floor guards per-session **tokens**, not session **shape**.

So C′ is viable or inert depending entirely on §2's id decision:

- **stable per-job id** (what A′ now specifies): the session accumulates across
  runs and eventually exceeds 4,000 tokens. C′ fires, and periodically, which is
  the intent.
- **fresh per run**: every session starts empty and never grows toward the
  floor. C′ is **dead code** on the cron path — a subsystem that compiles,
  tests, and never fires once.

This is why §2 pins the id rather than leaving it a placeholder. It is also why
the floor must not be lowered to make cron compact: the floor exists so short
sessions are not reduced to a summary-of-summary (§4.5).

Two settings remain a decision when C′ is wired, and neither is urgent:

- compact the employee's session across runs (needs the stable key A′ provides)
  vs. compact on the gateway path, where sessions are genuinely long-lived;
- `compact` (`:429`) writes a marker, `compact_summarising` (`:448`) summarises
  the middle through an LLM and falls back to the marker if the summary fails.

Do not wire C′ before the id question is answered — but §2 has now answered it,
so the remaining choice is only *where* compaction runs, not whether it can.

### The cost of stable ids before C′ lands

This cuts against the "not urgent" framing above, so state it plainly. A′ gives
each of the ~102 cron jobs a **stable** session that accumulates across runs,
and iter-548's store is backed by the existing `Database` message tables — there
is no second persistence path. `set_session_id` persists the outgoing transcript
and rehydrates from disk (`events.rs:379-397`), so every run's turns stay
durable.

That means **after A′ and before C′, the message tables grow without
bound by design.** Nothing trims a cron session in between: the only compaction
that exists is the autocompactor C′ has yet to wire. Survivable for an iteration
or two, and it is the price of employee continuity — but it is a real
consequence of §2's choice, not a free win.

If unbounded growth bites before C′ is wired, the interim lever is session
discard (`SessionStore::discard`, which `clear_history` already calls at
`:206`) on an explicit retention rule — not lowering the compaction floor.

Note `compress_context_overflow` (`agent/compress.rs`) is a *different*
mechanism and the two must not be conflated.

---

## 5. Step D′ — hierarchy and §2.4 enforcement, on a real agent

`hierarchy.rs` models the reporting line as explicit edges over
`HierarchyEntry` rather than as a field on `Employee`. That was forced:
`Employee` is constructed from struct literals in eight files, none using
`..Default::default()`, so adding a field breaks the crate. When a later packet
adds the column, `HierarchyEntry` is what reads it.

Two things remain for D to be real:

1. **The edges have to come from somewhere.** `hierarchy.rs` is a pure query
   surface over a set of edges. Nothing populates that set from the employee
   table, because the column does not exist yet. Until it does, the tree is
   empty and every hierarchy answer is trivially "no relation".
2. **The registry has to be consulted at agent construction.** `ToolRegistry`
   now carries `tool_authority` and the filtering methods exist, but
   `OperantAgent::new` at `cmd_acp.rs:156` and `main.rs:1844` construct a
   registry with no actor attached, so every lookup is unrestricted.

Decide explicitly whether to (a) add `reports_to` to `Employee` and fix the
eight construction sites, or (b) populate edges from a separate table. (a) is
the design's stated intent; (b) is cheaper. **This is an owner decision, not a
sequencing detail.**

---

## 6. Step E — assignment boards

Not started. First genuinely *governed* mutation: the cross-department grant
check gates it, and it is the first place the authority lattice is exercised by
something other than a test.

Requires D′ to be live first — a governed mutation needs something to check
against. E before D would be a board nobody can enforce.

---

## 7. Step F — concurrent CEO loop

Not started. Fan out to HODs, collect report artifacts, synthesize a
trajectory, optionally reconvene once for a bounded alignment round.

Carries the D3 concurrency design and the largest open design risk in the
program. `scheduler.rs:95` is still `for job in due_jobs` — strictly
sequential — so nothing about fan-out exists yet. This also widens the `org
check` exit contract from `org department findings` to the umbrella §11.4 gate.

**Concurrency safety note:** iter-548 gave every session its own slot, which
removed the reason fan-out was unsafe. It did not make fan-out *correct* —
shared SQLite writes and the barrier's write ordering across N concurrent runs
are unexamined.

---

## 8. Sequencing constraints

1. **A′ before everything.** B′, C′ and D′ all key off a per-job session.
   Without it there is one shared slot, and every scheduled employee run starts
   blind — which is the pre-iter-548 defect, still live on the cron path.
2. **B′ before D′.** The barrier is what makes an unauthorized mutation
   attributable.
3. **B′ before C′**, same seam: the barrier reads the run's artifacts.
4. **D′ before E.** A governed mutation needs something to enforce against.
5. **E before F.** The CEO loop reads boards.
6. **The identity gate stays dark.** `with_org_gate` exists
   (`scheduler.rs:60`) but activation is an owner policy decision, not a step.
   The department `rules` array it waits on now exists, so it is *possible* —
   still opt-in.

### Unscheduled but red on mainline

`tools::kernel::{ping_roundtrip, harness_apply_and_rollback_roundtrip}` fail on
`origin/main`. Pre-existing since iter-549, unowned, and the only red tests on
mainline. They deserve their own iteration rather than being folded into an org
change.

**Answered 2026-10-03:** `ping_roundtrip` **fails standalone** in a clean
worktree at `origin/main` — `kernel/mod.rs:285`, `left: Bool(false), right:
Bool(true)`. So this is a real standalone bug, not order dependence under
parallelism, and the open question this paragraph previously carried is closed.
Nothing in this program touches `tools::kernel`; it is a separate fix.

---

## 9. What is deliberately out of scope

- Importing organism cron jobs, prompts, ventures, or business data. Port the
  architecture, not the business.
- `retention_gc` wiring (GAP-2.6) and the CHANGELOG entry for the Wave 1
  breaking CLI change (GAP-X.1). Both real, both housekeeping.
- `GAP-4.4` AD/RG lifecycle — a separate packet.

---

## 10. Suggested first concrete task

**STATUS 2026-10-03: A′ landed (iters 570–571). B′ is in flight as wave-1
slice D of §11. This section stands as the acceptance record for both.**

Two code increments, in this order: **A′ then B′**.

**They carry no fixed numbers.** A reservation written as an absolute label has
a lifetime of exactly one commit, because the next commit consumes a label and
the reservation silently expires. So: *take the next two free labels above the
highest committed on `origin/main`, in order, at the time you read this.* That
survives arbitrarily many intervening commits. The `iter-56x` numbers that
appeared here previously are removed on purpose — they were a standing assertion
about a moving target, and six of the seven collisions this session came from
exactly that.

**Label discipline, because seven collisions have happened this session.**
Every label already committed to `origin/main` is SPENT (code and docs).

Read from `origin/main` after `git fetch`, immediately before committing. Never
amend a pushed commit to fix a label; renumber the plan forward instead.

### A′ — stable per-job session, and the compressor reset

**A′ is not implementable until the shared-agent constraint above is resolved.** Give cron its
own `OperantAgent`, or establish the paths do not overlap. Everything below
assumes that is settled.

Reuse `derive_employee_id(&job.id)` — the org gate already calls it at `:117` —
and pass a **stable** per-job id (not a fresh uuid) to
`agent.set_session_id(...)` before `run()`. Delete the `clear_history()` at
`:277`. Move the compressor reset into `set_session_id`, gated on
`previous != new_id`. No schema change, no new derivation.

Acceptance — each needs a **negative control**, because a test that cannot fail
proves nothing:

- a cron job ticking while a gateway conversation is in flight does **not**
  disturb that conversation's turns. Control: the assertion must fail if cron
  and the gateway share one agent — which is the state today.

- two scheduled jobs run back to back through the real scheduler on one agent,
  and neither sees the other's turns. Control: the test must also fail when the
  same job id is used twice with `set_session_id` removed, or when the harness
  hands each run its own agent. An isolation test that passes because each run
  got a fresh agent measures nothing.
- a second run of the **same** job sees the first run's history — proving the id
  is stable, not per-invocation. Control: assert this fails under a fresh-uuid
  scheme.
- the compressor is reset on session change: job A's summary does not appear in
  job B's compression context. Control: confirm the assertion fails with the
  reset removed.
- `clear_history()` is gone from the cron path. Control: confirm the grep
  pattern finds it at `:277` *before* the deletion and returns nothing after.
- `cargo test -p operant-core --lib` at 2235 passed / 2 pre-existing
  `tools::kernel` failures, unchanged;

### B′ — mount `WriteBarrier::apply` in `run_agent_job`

Acceptance:

- a completed scheduled run leaves exactly one worklog row;
- a barrier write failure makes the run report failure, not success;

### Both increments

- **verified in a clean worktree at the pushed commit, not the working tree** —
  the iter-550 defect shipped precisely because the working tree masked it.
## 11. Canonical fleet execution plan (2026-10-03)

This section is the canonical sequencing for the in-flight work. It supersedes
§10's ordering for anything not yet landed, and it is what the 2026-10-03
sub-agent fleet executes against. The eight alignment questions in
`docs/PERMISSION-SCOPING-PLAN.md` §6 are **resolved for execution** by the
owner's full-scale-execution directive: each recommended default applies,
implemented so that a later answer changes a *value or adapter*, never an
architecture. Q6 (edge table) ships in wave 1 so P2 is not blocked.

State at canonicalization:

- **A′** landed — iters 570–571 (per-job stable session, cron's own agent,
  compressor retarget guard, `spawn_long_lived_maintenance=false` for cron).
- **Genome P0 engine** landed — iter-573 (`org/seat_policy.rs`: `SeatMode`,
  `SeatPolicy`, `decide()`, namespace globs, 10 adversarial tests,
  fault-injection-proven).
- **B′, D-5, seat-policy storage, hierarchy edges** — not started; these are
  wave 1.

### Wave 1 — four parallel slices, file-disjoint

| Slice | Deliverable | Owns (nothing else) | Pattern to mirror |
|---|---|---|---|
| A | `SeatPolicyDb` sqlite store + `SeatPolicySource` trait | `org/seat_policy.rs` (trait only), `org/seat_policy_db.rs` (new), `org/mod.rs` (one `pub mod` line) | `GrantDb` (`org/authority.rs:551`): DDL const, idempotent init, `Arc<Mutex<Connection>>` |
| B | `hierarchy_edges` table → `Vec<HierarchyEntry>` adapter feeding `Hierarchy::new` | `org/hierarchy_edges.rs` (new), `org/mod.rs` (one line) | `employee_db` store style |
| C | D-5 fix: hoist `(MemoryManager, Option<Arc<dyn MemoryProvider>>)`; thread the same pair into both agent builds | `operant-cli/src/main.rs`, `gateway_runner.rs` (build seam only), `BUGS.md` (D-5 entry) | BUGS.md D-5 names the preferred fix |
| D | B′: mount `WriteBarrier::apply` in `run_agent_job` per §3 acceptance | `cronjobs/scheduler.rs`, `org/write_barrier.rs` (only if a helper is missing), `tests/` (extend `cron_session_isolation.rs` or new) | §3 of this outline is the spec |

Fleet rules:

1. Each slice works in its **own git worktree off `origin/main`** tip —
   per-worktree `target/` means no cargo lock contention. Source
   `scripts/dev-env.sh` before any cargo command. Scoped tests only
   (`-p operant-core --lib -- <filter>`; slice C: `-p operant-cli --bin operant`).
2. **No slice commits or pushes.** The integration owner applies each diff to
   the working tree, stages ONLY the slice's files (never `git add -A` — the
   tree carries a concurrent agent's WIP in `Cargo.toml`, `Cargo.lock`,
   `tui/*`), runs the combined suite once, and lands each slice as its own
   iteration — label taken from `origin/main` immediately before each commit.
3. Evidence rules unchanged: every green carries a negative control; restores
   proven by sha256; every cited `file:line` read back before commit; no
   `.unwrap()`/`.expect()` in lib code; no explicit `ref` in patterns.
4. The working-tree lib-suite baseline is **2235 passed / 2 failed** (the 2 =
   `tools::kernel` K-1, unowned, standalone-failing — not a wave-1 concern).

### Wave 2 — after wave-1 integration

| Slice | Deliverable | Depends on |
|---|---|---|
| E | P1 enforcement: `AuthorityActor` per running employee (cron: `derive_employee_id`; gateway: the chat's seat), `decide()` consulted before the guards at `stream.rs:709/737` | A (source trait), C (agent-build seam) |
| F | P2 escalation: `pending_requests` store, `manager_of` routing, approval → `issue_grant` with TTL, cron deny-after-TTL (not the 120s interactive stall) | B (edges), E (decision seam) |

Wave-2 acceptance criteria live in `docs/PERMISSION-SCOPING-PLAN.md` §5
(P1: same tool call succeeds under `yolo` and escalates under `lockdown`;
P2: lockdown cron denied → request surfaces → approval mints grant row →
re-run executes without re-asking).

### Owner-decision defaults now in force (overridable by value, not architecture)

| Question | Default in force |
|---|---|
| Q1 policy storage | sqlite `seat_policies` table (wave-1 slice A) |
| Q2 unattended TTL | deny-after-TTL, configurable; HoD default 46 min |
| Q3 approval → grant | yes; `expires_at` required; 7-day HoD default; standing only CEO+ |
| Q4 precedence | plan §4 as implemented in `decide()` (iter-573) |
| Q5 terminal authority | the human operator; HR on-chain only when named |
| Q6 hierarchy edges | separate edges table (wave-1 slice B) |
| Q7 scoped + dangerous-unlisted | escalate (missing ≠ denied) |
| Q8 granting topology | HoD own department, CEO anywhere, HR read-only |

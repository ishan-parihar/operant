# Plan — subagent swarm and kanban execution gaps: one identity model, one audited executor

**Date:** 2026-11-16 (revision 2 — supersedes revision 1, which was written
untracked and deleted before it could be executed. Revision 2 is committed. It
folds in the red-team of revision 1: two file lists that would not compile, one
unreachable acceptance check, one index that would have broken retries, and two
audit findings revision 1 dropped.)
**Base:** `origin/main` @ `2b8bc1ce` (revision 1 was based on `94c75fb6`; the
tip advanced 16 commits. `iter-2444` added 73 lines to `sub_agent_tool.rs`, so
**every line number below is re-cited from `2b8bc1ce`** — revision 1's numbers
were stale by +32/+45.)
**Status:** outline for execution. Revision 2 authorizes nothing by itself; each
tranche below is one gated iteration.

## Why revision 2 exists

Revision 1 was correct about the *findings* and wrong about the *mechanics*. A
red-team against `2b8bc1ce` found four defects that would have stopped an
executor mid-tranche:

| Rev-1 defect | Consequence | Fixed in rev 2 |
|---|---|---|
| T2 deleted `claim_lock` but omitted `dispatcher.rs`, which INSERTs it | compile break in a file the tranche never named | T1 file list now carries it |
| T3 made `persist()` async but omitted the sync startup chain | compile break in `main.rs` | T2 names `main.rs` |
| T5 asserted a `done when` needing owned `String` from a `-> &str` trait method | unachievable check | T4 adds a *defaulted* trait method |
| T1 said "a unique index" without saying *partial* | a full index breaks every retry | T1 specifies the partial form |

It also dropped two findings without a row, and gave one finding two
contradictory dispositions. Both are corrected here.

## The structural finding this plan turns on

**Operant models a delegated child as a cheaper copy of its parent; both
references model it as a first-class unit with its own identity, budget and
address.**

That single cause sits behind F7 (the parent's model), F9 (the parent's client),
F5 (the parent's cwd), F13 (the parent's id space) and F12 (the parent's own
judgement). Breaking the copy starts with giving a delegation its own model
(**T5**), which unblocks the per-model lane (**T6**).

The kanban findings have a different cause: `kanban/` is a **second execution
engine** (1944 LOC, 7 tables, 8 actions) that has had far less audit attention
than the swarm's nine passes. F1/F2 are ordinary lifecycle enforcement in
unaudited code, and they are the first tranche because they are the only
findings where a wrong answer corrupts durable state.

## Findings → tranches

| Gap | Sev | Tranche | Note |
|---|---|---|---|
| F1 kanban double-claim | High | **T1** | partial unique index + one transaction |
| F2 lease declared, never enforced | High | **T1** | columns deleted (DD-4 settled) |
| F3 blocking `flock` on tokio | Med-High | **T2** | `spawn_blocking` |
| F4 terminal write silently dropped | Med | **T2** | retryable terminal write |
| R2 retention rule duplicated | Low | **T3** | one ordering |
| F8 limits undisclosed to the model | Med | **T4** | defaulted trait method |
| F7 per-delegation model | Med | **T5** | DD-1 resolves the fallback fork |
| F9 no provider/model lane | Med | **T6** | plus the per-child rate-limiter class |
| P8b two live-worker registries | Low | **T7** | new in rev 2 |
| F10 kill switch unreachable | Low | **T8** | config-schema change |

## Design decisions

**DD-1 · A delegation gets an explicit model, defaulting to the parent's; the child client IS wrapped in `FallbackModelClient`.**
Revision 1 left this a fork. It is now decided: add `model: Option<String>` to
`SubAgentArgs` (:119); absent → inherit `self.model` exactly as today; present →
resolve through `ensure_supported_model` and construct the child against it. The
child client at :944 becomes a `FallbackModelClient`, so the `fallback_models`
list a child already inherits from `AgentConfig::default()` finally has a
consumer. *Rejected:* stopping the inheritance instead — that closes the
"populated but inert" defect (F7) while removing a capability the operator
configured globally and would reasonably expect a child to honour.
*Reopens if:* a measurement shows child fallback causing retry storms that the
parent's fallback does not.

**DD-2 · Provider admission is a lane keyed on the resolved model, not a new knob.**
`max_global_workers` stays the process-wide spend cap; the lane is derived from
the model each worker resolved (model → provider → default). *Rejected:* a second
operator-facing concurrency number — two ceilings for one question is the defect
the audit flags elsewhere. *Reopens if:* an operator needs per-provider limits
inexpressible as a model lane.

**DD-3 · kanban and delegation stay separate systems; no bridge.**
Delegation is ephemeral, in-process, spend-capped; kanban is durable,
DB-backed, cross-session. A card is not a worker. *Rejected:* letting
`delegate_task` claim a card — that couples a 120 s in-process deadline to a
7-table durable store. *Reopens if:* an operator asks for card→worker dispatch,
which needs the kanban lifecycle real first (T1).

**DD-4 · SETTLED — delete the dead lease columns; do not implement the lease.**
`claim_expires`, `claim_lock`, `worker_pid` and `task_runs.last_heartbeat_at` are
written by nothing and read by nothing (verified: every occurrence is a struct
field, a schema line, or a `row.get`). Implementing expiry + reclaim is a new
subsystem; deleting makes the schema honest. Revision 1 both committed to this
and left it as an open question — it is now only a decision.
*Reopens if:* an operator needs automatic reclaim of a task whose dispatcher
died, which is the one capability the lease would have bought.

**DD-5 · `max_runtime_seconds` is deliberately retained and classified.**
It is written ([db.rs:419](crates/operant-core/src/kanban/db.rs#L419)), selected
([dispatcher.rs:29](crates/operant-core/src/kanban/dispatcher.rs#L29)) and then
**discarded** by its only consumer
([cmd_kanban.rs:816](crates/operant-cli/src/cmd_kanban.rs#L816)
`for (task_id, title, _max_runtime)`). It is *not* dead — it is an operator's
declared intent with no enforcement — so it is out of scope here and recorded as
a separate future finding, not silently swept with the lease columns.

## Tranches

Each is one commit, one iteration label, one gate run. `done when` is an
observable check at that commit.

### T1 — kanban claim atomicity + honest schema (closes F1, F2)

Wrap `claim_task` in one `BEGIN IMMEDIATE` … `COMMIT` so the `task_runs` insert
and the `tasks` update commit together, and add a **partial** unique index so a
second *concurrent* claim fails at the database:

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_runs_one_running
    ON task_runs(task_id) WHERE status = 'running';
```

**Partial is load-bearing.** [`fail_run`](crates/operant-core/src/kanban/dispatcher.rs#L111)
sets the task back to `todo` and marks the old row `failed`, so a retry inserts a
**second** `task_runs` row for the same `task_id`. A full unique index would make
every retry fail forever.

Then delete the dead lease columns (DD-4) across **all** sites:
`claim_expires` (db.rs :61, :85, :188, :238, :321, :368, :740),
`claim_lock` (:60, :84, :187, :237, :320, :367, :739) and
`task_runs.last_heartbeat_at` (:69, :88, :196, :241, :329, :376, :743) —
**plus `dispatcher.rs:59`, whose INSERT names `claim_lock`** and which revision 1
omitted. `max_runtime_seconds` stays (DD-5).

- **Files:** `crates/operant-core/src/kanban/dispatcher.rs` (:52-:78, :111-:135),
  `crates/operant-core/src/kanban/db.rs` (schema :187-:196, :231-:241; structs
  :60-:69, :84-:88, :129; mappings :320-:329, :367-:376, :739-:743),
  `crates/operant-core/src/tools/kanban_tool.rs` (only if an arg surfaces a
  deleted column)
- **Depends on:** nothing — **first executable tranche**
- **done when:** two `claim_task` calls on one task id, driven through **separate
  real `Arc<Mutex<Connection>>` handles**, yield exactly one `running`
  `task_runs` row and an error on the second; AND a `fail_run` → `claim_task`
  retry on the same task succeeds and produces a second `running` row (proving
  the index is partial, not full); AND `cargo check -p operant-core` passes with
  no reference to a deleted column.
- **Rollback:** revert the T1 commit. The index is additive (`DROP INDEX`); the
  column drop is **not** revert-safe — copy `operant_kanban.db` aside before the
  migration and name the copy in the commit body.

### T2 — the delegation ledger persists off the async thread (closes F3, F4)

Move the whole read-merge-write in `persist` (:218) onto `spawn_blocking`, and
make a terminal write that cannot take the lock **retryable** rather than
silently dropped (:225 returns today). A record whose terminal write failed is
marked so the next status change or a shutdown flush re-attempts it, and
`settle_interrupted` (:394) must never report a completed delegation as failed.

**Do not convert the startup path to async.** `persist()` is reachable from
`recover_interrupted()` (:375) → `enable_persistence()` (:155) → called at
[main.rs:1281](crates/operant-cli/src/main.rs#L1281) inside the **sync**
`fn create_model_client_with_fallback` (:1111). Keep a sync entry point for that
path — either a `persist_blocking()` the startup path calls directly, or a
`block_in_place`/current-runtime handle — and make only the *status-change* call
sites (:616, :629, :642) async.

- **Files:** `crates/operant-core/src/tools/async_delegation.rs` (:218, :225, :246,
  :281, :350, :375, :394, :616, :629, :642),
  `crates/operant-cli/src/main.rs` (:1281 only if the entry point must change)
- **Depends on:** nothing (independent seam from T1)
- **done when:** a test holds the ledger lock from another process, completes a
  delegation, releases the lock, and the record reaches `Completed` in the file
  **without a restart**; AND a record completed in memory but unwritten does
  **not** come back as `Failed` after `settle_interrupted`.
- **Rollback:** revert the T2 commit. Ledger format unchanged, no data migration.

### T3 — one retention rule (closes R2)

`foreground_swarm` keeps its own `MAX_RETAINED_TERMINAL = 50` (:47) and its own
`prune` with its own sort (:99). Extract the ordering into one helper both call,
or have `foreground_swarm` reuse `enforce_terminal_cap`'s ordering
(async_delegation :281) over a shared key.

- **Files:** `crates/operant-core/src/tools/foreground_swarm.rs` (:47, :99-:123),
  `crates/operant-core/src/tools/async_delegation.rs` (:281)
- **Depends on:** none; land with or after T2 (same module family, one gate run)
- **done when:** a grep finds exactly one definition of the terminal cap and one
  total ordering for it, and `the_registry_follows_a_worker_from_start_to_finish`
  still passes unchanged.
- **Rollback:** revert the T3 commit.

### T4 — tell the model the limits it is under (closes F8)

`fn description` (:1768) promises workers "all running in parallel" and names
neither `max_global_workers` (:81), nor `max_concurrent_children`, nor
`SWARM_RESULT_DEADLINE_SECS` (:1657). When the roster exceeds what can finish
inside the deadline, the result must say so beside the existing `CutOff` text.

**Mechanism (revision 1's blocker):** `OperantTool::description(&self) -> &str`
([tools.rs:323](crates/operant-core/src/tools.rs#L323)) returns a borrow, so it
cannot interpolate runtime atomics. Add a **defaulted** trait method beside it —
`fn runtime_description(&self) -> Option<String> { None }` — and have the
registry prefer it when present. ~40+ implementors (20 in `aft_tools.rs` alone)
keep compiling untouched; only `SubAgentTool` overrides it. **Do not change the
existing signature.**

- **Files:** `crates/operant-core/src/tools.rs` (:323, add the defaulted method),
  `crates/operant-core/src/tools/sub_agent_tool.rs` (:1657, :1711, :1768),
  and the registry site that reads a tool's description for the model
- **Depends on:** none — smallest tranche, no gate risk
- **done when:** a test asserts the runtime description contains the live
  `max_global_workers` and `max_concurrent_children` values, and changes them at
  runtime to prove the string follows; AND a swarm whose roster exceeds the
  in-flight cap reports the throughput limit in its result text.
- **Rollback:** revert the T4 commit.

### T5 — a delegation can name its model (closes F7; unblocks T6)

Add `model: Option<String>` to `SubAgentArgs` (:119). Absent → inherit
`self.model` (today's behaviour). Present → resolve via `ensure_supported_model`
and build the child against it. Per **DD-1**, the child client at :944 becomes a
`FallbackModelClient` so the inherited `fallback_models` list gains a consumer.

- **Files:** `crates/operant-core/src/tools/sub_agent_tool.rs` (:119, `WorkerSpec`,
  :944), `crates/operant-core/src/tools/delegation_output_schema.rs` only if a
  schema field is needed
- **Depends on:** none. **Blocks:** T6
- **done when:** a delegation with `model="X"` runs its child against X and one
  without runs against the parent's model; AND a test proves the child either has
  a live fallback path or an empty `fallback_models` — never a populated-but-inert
  one.
- **Rollback:** revert the T5 commit. A child that already ran on another model
  has spent tokens; the rollback is code-only and says so.

### T6 — provider/model-aware admission (closes F9 + the rate-limiter class)

With a per-task model from T5, derive a lane key from the resolved model and bound
each lane (model → provider → default), keeping `max_global_workers` as the
process-wide spend cap.

**Revision 1 dropped half of F9.** Each child builds
`OpenAIClient::from_shared_http_client(...)` (:944), so N children hold **N
independent copies** of `[client].rate_limit` — the per-slot policy
multiplication the audit names. A model lane alone does not fix that: the lane
must bound against **one shared limiter**, not N private ones.

- **Files:** `crates/operant-core/src/tools/sub_agent_tool.rs` (:81, :944, :1025),
  the client construction path, plus the CLI config that sets the caps
- **Depends on:** T5
- **done when:** a swarm of N workers on one provider cannot exceed the lane
  bound even when `max_global_workers` would admit them; AND a test asserts the
  **aggregate** rate-limit budget is one shared limiter, not N private ones.
- **Rollback:** revert the T6 commit. No persistent state.

### T7 — one live-worker registry, not two (new in rev 2)

`foreground_swarm::LIVE` and `async_delegation::registry` are near-identical:
same shape, same retention, same newest-first ordering, same `now_secs`. T3
unifies only the retention rule; the duplicate registries remain. Make one the
store and the other a projection, or delete the redundant one if a single
registry can serve both surfaces.

- **Files:** `crates/operant-core/src/tools/foreground_swarm.rs`,
  `crates/operant-core/src/tools/async_delegation.rs`, the TUI/CLI readers
  (`App::swarm_gallery_members`, the hub's `Working` count)
- **Depends on:** T3
- **done when:** a grep finds one definition each of the registry, its retention
  and its clock; both reader surfaces still render the same rows they did before.
- **Rollback:** revert the T7 commit.

### T8 — thread the loop-detection kill switch (closes F10)

`loop_detection_enabled: true` is hardcoded at
[agent/mod.rs:295](crates/operant-core/src/agent/mod.rs#L295) while the field doc
(:166) claims facades thread the operator's switch. **This is a config-schema
change, not a threading** (revision 1 under-costed it): `config.rs` has **zero**
occurrences of `loop_detection`, so the field must be *added* to
`BehaviorSettings`, parsed from `operant.toml`, threaded through
`From<&BehaviorSettings>`, and declared in `operant.example.toml` — which
CLAUDE.md requires in the same change.

- **Files:** `crates/operant-core/src/agent/mod.rs` (:166, :295),
  `crates/operant-core/src/config.rs` (`BehaviorSettings`),
  `crates/operant-cli/src/config.rs` if the CLI mirrors the struct,
  `operant.example.toml`
- **Depends on:** none; may ride with T5 (shares `agent/mod.rs`)
- **done when:** setting the config key to `false` yields
  `loop_detection_enabled == false` on a constructed `AgentConfig`, asserted by
  reading the value back, not the file; AND `operant.example.toml` declares the
  key.
- **Rollback:** revert the T8 commit.

## Acceptance

**First useful check:** T4's runtime-description test — smallest tranche, no
migration, proves the gate runs before any schema or concurrency change.

**Full acceptance set** (from `genome.toml`):

```
source scripts/dev-env.sh
cargo fmt --all
./scripts/check.sh check -p operant-core --lib
cargo test -p operant-core --lib
cargo clippy -p operant-core --all-targets -- -D warnings
./scripts/check.sh check --workspace
./scripts/check.sh test  --workspace
```

plus each tranche's own regression named in its `done when`.

## Gap → proof → evidence

| Gap | Proof (at `2b8bc1ce`) | Evidence artifact |
|---|---|---|
| F1 | `claim_task` has no transaction; `task_runs` has no unique index | T1 concurrent-claim test |
| F2 | every `claim_expires`/`claim_lock` occurrence is a field, schema line or `row.get` | T1's grep + the removed-column diff |
| F3 | `persist` runs `flock`+read+write inline in a `tokio::spawn`; no `spawn_blocking` anywhere | T2's held-lock test |
| F4 | `persist` returns silently on lock failure; `settle_interrupted` rewrites `Pending`→`Failed` | T2's unwritten-completion test |
| F8 | description names no limit | T4's runtime-description test |
| F10 | `loop_detection_enabled: true` hardcoded :295; zero hits in `config.rs` | T8's read-back test |
| R2 | `MAX_RETAINED_TERMINAL` + `prune` defined twice | T3's single-definition grep |
| F7 | no `model` field in `SubAgentArgs`; child client is a bare `OpenAIModelClient` | T5's two-model test |
| F9 | no lane; each child builds its own client | T6's aggregate-limiter test |

## Non-goals

- **F13 (addressable children + follow-up)** — needs an address scheme, a
  follow-up tool and mailbox semantics. Its own plan.
- **F12 (independent reviewer for a child's tool calls)** — codex's equivalent is
  **49 files / 13,582 lines** (measured) with its own circuit breaker. Its own
  plan, and it should not start before T1 lands: a reviewer that stops a child is
  worth much less while a child can still be double-claimed.
- **F5 (worker filesystem isolation)** — at parity with codex today (its
  `worktree` crate is depended on by `exec` and `tui`, **not** by
  `core/src/agent/`), an omo-only gap. Its own plan; it interacts with
  `resolve_child_cwd`'s deliberate refusal, so it is a design change.
- **F6 (DAG) — no action, single disposition.** Codex has no DAG either (its
  `agent-graph-store` is recovery-only, never scheduling), so this is a
  **single-reference** gap against oh-my-openagent, whose `senpi-task/src/dag/` is
  **72 files / 20,814 lines** (measured; the 35/13.9k figure in revision 1 came
  from that tree's own `AGENTS.md` and was wrong). Revisit only if the operator
  wants omo-shaped durable workflow.
- **F11 (dropped completion notification)** — parity with codex, which is also
  best-effort and explicitly not deduplicated. Recorded, not planned.
- **R1 (kanban ↔ delegation bridge)** — closed as DD-3.
- **DD-5 (`max_runtime_seconds` unenforced)** — recorded above, separate finding.

## Execution order and labels

T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8, one iteration label each, label taken
from `origin/main` after a fetch and re-derived immediately before each push
(AGENTS.md §Iteration Numbering). Base for this revision is `iter-2451`, so the
first code tranche is `iter-2453` or later — **derive it, do not copy it.**

## What changes when this is executed

A kanban task becomes impossible to claim twice while remaining retryable; the
kanban schema stops advertising a lease nothing keeps; the delegation ledger's
blocking file I/O leaves the async runtime and a completed delegation can no
longer be reported as failed; one retention rule replaces two; the model learns
the concurrency and deadline limits it is actually under; a delegation can name
its model and its inherited fallback list becomes live; each model gets a lane
against one shared rate limiter; the two live-worker registries become one; and
the loop-detection kill switch becomes reachable from config.
---

## Execution record (revision 3, 2026-11-16)

All eight tranches are executed and pushed. Each landed as its own commit with its
own gate run, and each regression was **mutation-checked** — the test was shown to
FAIL when the fix was reverted, which is the only evidence that a test guards
anything.

| Tranche | Label | What landed | Mutation check |
|---|---|---|---|
| T1 | iter-2455 | kanban claim atomicity + partial unique index + 6 dead columns dropped | plain index ⇒ `left: 2, right: 1` |
| T2 | iter-2457 | ledger work off the async runtime; terminal write reports failure | retries 3→1 still passes (see below) |
| T3 | iter-2458 | one retention rule, one cap | second literal (25) ⇒ `left: 25, right: 50` |
| T4 | iter-2459 | delegate description states the live limits | literal cap (3) ⇒ assertion fails |
| T5 | iter-2460 | per-delegation model; child fallback chain live | override ignored ⇒ `left: "parent-model", right: "cheap-scout"` |
| T6 | iter-2461 | N children share ONE rate-limit bucket | own limiter ⇒ aggregate assertion fails |
| T7 | — | **closed with no code change** — see below | n/a |
| T8 | iter-2462 | `loop_detection_enabled` is a real config key | hardcoded `true` ⇒ assertion fails |

### T7 — closed as already satisfied by T3; no code change

T7 proposed unifying "two live-worker registries". Measured, the premise does not
hold, and the only real duplication had already been removed by T3.

| Concern | foreground_swarm | async_delegation | Verdict |
|---|---|---|---|
| Cap constant | `MAX_RETAINED_TERMINAL = 50` | same, `= 50` | merged in T3 — foreground now aliases the async constant |
| Retention ordering | own `sort_by` on `(started_at, id)` | own `sort_by` on `(time, id)` | merged in T3 — both call `oldest_terminal_ids` |
| Clock | `fn now_secs()` | `pub fn now_secs()` | already delegated — foreground's is a one-line forwarder |
| Registry | `OnceLock<Mutex<Vec<Row>>>` | `OnceLock<Mutex<HashMap<String, Record>>>` | not duplicates |

The registries hold different things and must stay apart. `foreground_swarm` is
process-local UI membership for FOREGROUND swarms — a `Vec<Row>` of
`{id, label, status, detail, started_at}`, ordered, pruned, read by exactly one
surface (`App::swarm_gallery_members` via `list_filtered`), whose own header says a
DB-backed projection "is deliberately NOT here". `async_delegation` is a durable
keyed record store for BACKGROUND delegations — a `HashMap` by delegation id, with
ledger persistence, `request_key` dedup and `settle_interrupted` recovery.

Merging them would mean keying the UI membership list by delegation id and giving
foreground workers ledger persistence they deliberately do not have. That is not a
consolidation, it is a different feature. Recorded here rather than as a commit
because the useful output is the finding, not a diff.

### T6 — the model lane is still open

T6 asked for a per-model lane AND for one shared rate limiter. The aggregate-bucket
half landed (iter-2461). **The model lane did not**, deliberately: a lane needs a
`model → provider` key and a second ceiling beside `max_global_workers`, which is a
new operator-facing policy knob. With the bucket now global, the acute half of F9 is
gone, and a lane belongs with T5's per-delegation model rather than bolted on
afterwards. It is recorded as open rather than half-built.

### Corrections to this plan, found during execution

1. **T1's index had to be PARTIAL, and had to dedupe first.** A full unique index
   breaks every retry, because `fail_run` resets the task to `todo` and leaves the
   old run `failed`. And `CREATE UNIQUE INDEX` FAILS OUTRIGHT on a database that
   already carries the corruption — verified against a real v1 schema seeded with
   two running rows — so the migration would have aborted on exactly the operator
   F1 already hurt, taking every `kanban` command down with it.
2. **T4 needed no trait change.** The plan proposed a defaulted
   `OperantTool::runtime_description()` because `description()` returns `&str`.
   Unnecessary: `SubAgentTool::schema()` builds its own `ToolSchema` and
   `from_type` takes `impl Into<String>`, so the owned `String` goes straight in.
   No trait change, no ~40 implementors touched.
3. **T5 needed no `with_fallback_models` builder.** I added one plus a struct
   field, then deleted both: `AgentConfig::default()` already inherits
   `fallback_models` via `AgentConfig::from(&BehaviorSettings)`, so the builder was
   a second plumbing path that could drift from the one that already works.
4. **T2's "lock contention" framing was wrong.** `LedgerLock::acquire` uses a
   BLOCKING `flock` with no `LOCK_NB`, so a contended lock waits and then succeeds —
   contention is not the failure the guard catches. What reaches the failure branch
   is `acquire` being unable to open or lock the `.lock` file at all. The code
   comments now say that, and the test's stated scope is narrower as a result.
5. **T8 is a config-schema change, as revision 2 predicted.** It needed a
   `BehaviorSettings` field, a `From` thread, a `config_registry` entry in THREE
   structures, and `operant.example.toml`. The parking-walk gate caught the missed
   `KNOWN_KEYS` entry.

### Still open after execution

- **F9's model lane** (above).
- **F6, F5, F12, F13** remain non-goals with their own plans needed.
- **Pre-existing defects reported, not fixed** (AGENTS.md rule 7):
  - `cargo check -p operant-cli` fails with **E0364** at
    `crates/operant-cli/src/tui/settings_screen.rs:221` — `pub(crate) use keys::{…
    toggle_or_cycle_current}` re-exports a `pub(super)` fn. Proven pre-existing by
    stashing my change and reproducing the identical error. This blocks the CLI
    gate for every iteration in this set.
  - `config::tests::lifeos_key_is_gone_from_config` fails, and several config tests
    are order-dependent (they share process-global paths): 75/75 pass with
    `--test-threads=1`. Proven pre-existing by re-running without my changes.
  - 9 clippy errors in `database.rs`, `cronjobs/scheduler.rs`, `config_backup.rs`,
    `agent/turn_context.rs`, `send_message_tool.rs`, `home_assistant_tool.rs`,
    `config_heal.rs`, `delegation_output_schema.rs` — all byte-identical at
    origin/main in files these iterations did not touch.

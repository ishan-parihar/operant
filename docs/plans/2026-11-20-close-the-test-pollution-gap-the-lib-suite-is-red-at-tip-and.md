# Close the test-pollution gap: the lib suite is red at tip and nobody can tell a real regression from a race

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent after iter-2699/2700 landed. Every number below was
MEASURED at `origin/main` = `895b0810` (iter-2702) in a clean throwaway worktree before
it was written down.

## Claim rule

Never take a number from a header or from a previous plan. `git fetch origin`, read the
top label from `origin/main`, add one. Re-run every command in the baseline before
trusting the row that cites it.

## Why this plan exists

`cargo test -p operant-core --lib` is **RED AT TIP**. Measured: `3628 passed; 27
failed; 2 ignored`. This is not a new regression — it is a *steady state*, and that is
the defect. A suite that is always red carries no information (the same argument
`scripts/audit-config-surface.py` makes in its own docstring, and the reason that script
carries an `ACCOUNTED_FOR` baseline instead of failing on everything).

The consequence is concrete and it already bit this session: while auditing the tip I
found what looked like a config defect, and the ONLY way to tell it apart from
pollution was to run each candidate test in isolation, then serially, then from the repo
root. Three runs per finding. The suite's redness is now a tax on every future audit.

Worse, the failures **mask each other**. `agent::` tests fail on a missing sqlite file
and `checkpoint_tool::` tests fail because `git` cannot resolve its environment; both
look like "the feature is broken" in a summary line, and neither is.

## Measured baseline (all commands re-runnable)

At `origin/main` = `895b0810`, clean worktree, `source scripts/dev-env.sh`:

```text
$ cargo test -p operant-core --lib
test result: FAILED. 3628 passed; 27 failed; 2 ignored; 0 measured; 0 filtered out

$ cargo test -p operant-core --lib -- --test-threads=1 tools::checkpoint_tool::tests::
test result: ok. 27 passed; 0 failed          # <- ALL of them pass serially

$ cargo test -p operant-core --lib -- tools::checkpoint_tool::tests::test_shadow_checkpoint_roundtrip
test result: ok. 1 passed                      # <- and individually

$ cargo test -p operant-core --lib -- --test-threads=1 agent::tests::
test result: FAILED. 96 passed; 6 failed       # <- still fails serially (different class)
```

So the 27 split into **two independent defect classes**, and neither is a broken feature:

### Class A — parallel-only (21 tests): an unguarded process-global

All 11 `tools::checkpoint_tool::tests::*` and the `gateway::*` / `config::*` /
`tools::sub_agent_tool::*` / `tools::kernel::*` failures pass with
`--test-threads=1` and individually. They fail only under parallelism.

The mechanism is documented in this repo already, in
`crates/operant-core/src/platform.rs:10-22`, which is worth quoting because it is the
exact hazard:

> The environment is process-global, so any test that changes `HOME`, `HERMES_HOME` or
> a path derived from them changes what EVERY other test sees. Each module used to have
> its OWN `env_lock()`, so the modules did not exclude each other — and
> `get_default_operant_root()` computes the root from the CURRENT `HERMES_HOME` and
> `HOME`, so an interleaved mutation changes the root mid-test.

The fix for THAT was the crate-wide `test_env_lock_blocking()`. It exists, it is correct,
and **the checkpoint tests never adopted it**:

```text
$ grep -c 'test_env_lock\|runtime_config_test_lock' crates/operant-core/src/tools/checkpoint_tool.rs
0
```

`checkpoint_tool` has **28 tests** and not one takes the lock. Its `ensure_checkpoint`
shells out to `git` (`checkpoint_tool.rs:263`), and `git` reads `$HOME`. A parallel
`config::` test that sets `HERMES_HOME`/`HOME` to a tempdir makes those 11 git-backed
tests fail. The observed assertion is `mgr.ensure_checkpoint(dir, "one")` returning
false — i.e. the git call failed, not the checkpoint logic.

Same class, measured but not yet attributed per-test: the five `tools/*` modules whose
tests mutate env and hold no lock (`tts_tool`, `todo_tool`, `terminal_backend`,
`kernel/mod`, `cli_roster`).

### Class B — serial too (6 tests): relative paths resolved against the process cwd

These fail even at `--test-threads=1`, so they are NOT pollution:

```text
agent::tests::build_messages_inserts_the_pre_compress_insight_after_the_system_prefix
  -> Agent("Failed to open database: unable to open database file: precompress-insert-test.sqlite")
agent::tests::build_messages_over_threshold_runs_preflight_ladder
  -> ... unable to open database file: ladder-fire-test.sqlite
agent::tests::build_messages_under_threshold_is_ladder_noop
  -> ... unable to open database file: ladder-noop-test.sqlite
agent::tests::micro_compaction_folds_the_stored_conversation_at_build_time
  -> ... unable to open database file: micro-compact-test.sqlite
agent::tests::lcm_engine_injects_stored_rollup_into_build_messages
  -> rollup block must be injected, got system: "You are Operant, a helpful AI assistant..."
agent::tests::a_holon_run_gets_no_workspace_or_global_context_r07
  -> personal mode keeps both sources: R07 LEGACY WORKSPACE MARKER — stale protocol
```

The first four are one line each in `agent/mod.rs` (`:3266`, `:3338`, `:4089`,
`:4206`):

```rust
Database::init(std::path::PathBuf::from("precompress-insert-test.sqlite")).unwrap();
```

A **bare relative path**. It resolves against the process cwd, which under `cargo test`
is the WORKSPACE ROOT, not the crate root — so the file lands somewhere nothing cleans
up, and the test's own precondition (a writable cwd) is not established. It passes from
the repo root by accident (`ladder-fire-test.sqlite` etc. are right there, git-ignored)
and fails anywhere else. That is the definition of an accidental pass.

The remaining two are different assertions and need their own diagnosis (see T5).

## Why not the three open ledger rows

`RT-190` (memory-wire bank family split), `RT-280` (gateway resume/reset) and `RT-543`
(two daemons, one socket) are all **owner-decision rows**, not coding rows: each says
"which of these two designs do you want" and the ledger explicitly parks them for a
ruling. Writing a plan to implement one would be deciding the operator's question for
them. This plan does not touch them.

## Why not the two open 2026-11-20 plans

`docs/plans/2026-11-20-close-the-consolidation-gaps-*` and
`docs/plans/2026-11-20-close-the-tui-audit-round-*` are both `ready-for-execution` and
cover: E015/E104/E009 genome errors, the RT-555 id collision, the missing pub-fn surface
audit, and seven TUI tranches. **All still live and unclaimed** — re-verified:
`genome lint` prints exactly `E009`, `E015`, `E104`; `audit-ledger-index.py` prints the
four RT-555/553/554 FAILs.

They are *documentation* plans. This one is about the **test harness**, which neither
covers and which is a precondition for trusting any of their verification steps. Run
those first if you prefer; just know their "verify" commands are being run against a red
suite.

## The tranches

Numbering is deliberately unlike the other plans' so the three can be cited side by side.

### T1 — Adopt the crate-wide env lock in `checkpoint_tool` (Class A, 11 tests)

Every test in `crates/operant-core/src/tools/checkpoint_tool.rs` that calls
`ensure_checkpoint` / `configure` / `list_checkpoints` takes
`let _guard = crate::platform::test_env_lock_blocking();` as its first statement.

28 tests; expect the lock to be held briefly. Do NOT add `#[serial_test::serial]` as a
substitute — the crate's own `platform.rs` doc explains why per-module locks failed
(the modules did not exclude each other). One lock or none.

**Verify:** `cargo test -p operant-core --lib` (parallel, default threads) reports
`checkpoint_tool` failures = 0. Mutation: delete one guard, watch that module's tests
redden again under parallelism.

### T2 — Adopt the lock in the other four unlocked modules (Class A, remainder)

`tools/tts_tool.rs`, `tools/todo_tool.rs`, `tools/terminal_backend.rs`,
`tools/kernel/mod.rs`, `tools/cli_roster.rs`. Each mutates env (`HERMES_HOME`,
`OPERANT_ENV_FILE`, `PATH`, …) and holds no lock. Add the same guard to the tests that
mutate, not to every test in the file — the lock is cheap but the point is excluding
*mutators*.

**Verify:** the `gateway::`, `config::` and `sub_agent_tool` failures from the baseline
list are gone from a parallel run. If any remain, attribute them individually rather
than widening the lock — a blanket lock that hides a real race is worse than a red test.

### T3 — Give the four sqlite tests a real tempdir (Class B, 4 tests)

Replace each bare relative filename in `agent/mod.rs` with a path under a per-test
`tempfile::TempDir` (or the crate's existing tempdir helper if one exists — check before
adding a dependency). The test must not depend on the process cwd for a writable
location, and must clean up after itself.

**Verify:** each of the four passes from ANY cwd, e.g.
`cd / && cargo test -p operant-core --lib -- agent::tests::build_messages_under_threshold_is_ladder_noop`.
Mutation: revert one to the relative path and confirm it fails from `/`.

### T4 — Make the suite's redness detectable: a pollution canary

The structural fix, and the reason this plan is not just T1-T3. Add a test that
**fails when the suite is red for an unattributed reason** — concretely, a guard that
asserts the known-good count and names the failures, so a NEW failure is visible against
a baseline instead of being one of 27.

The precedent is `scripts/audit-config-surface.py`'s `ACCOUNTED_FOR` baseline, and its
docstring is the argument: *"a check that is always red carries no information"*. Do the
same for the lib test suite. The baseline file must be **regenerated, never
hand-edited**, for the same reason `config_registry.rs` is generated (iter-2699: a
hand-maintained list rots).

**Verify:** with T1-T3 landed the canary is green; reintroduce any one pollution bug and
it fails BY NAME.

### T5 — Diagnose the two remaining agent assertions (Class B, 2 tests)

`lcm_engine_injects_stored_rollup_into_build_messages` and
`a_holon_run_gets_no_workspace_or_global_context_r07` fail serially with assertion
messages, not io errors. These are NOT the relative-path class and must not be folded
into T3. Diagnose each: is the expectation stale (the feature moved and the test did
not) or is the feature wrong? Record the verdict in the row before changing anything.

**Verify:** each has a written verdict in the ledger row and either a fix or a
`WONTFIX`-style disposition with the evidence.

## Gates / success criteria

(fixed before the run)

1. `cargo test -p operant-core --lib` — **0 failed**, from a clean worktree at
   `origin/main`. This is the whole point; every other gate is secondary.
2. The same run from a DIFFERENT cwd (e.g. `/`) also reports 0 failed — the accidental
   pass is gone.
3. `cargo clippy -p operant-core --all-targets` introduces no new warnings
   (`bash scripts/clippy-warning-gate.sh` reports `new=0`).
4. `python3 scripts/genome lint` introduces no new error class (E009/E015/E104 are
   pre-existing and owned by the consolidation plan).
5. `bash scripts/check-doc-claims.sh` reports no new diff.
6. Every fix ships a mutation proof: reintroduce the defect, watch the named test fail.

## Cost

T1 is mechanical (28 guards) and low-risk. T2 is smaller. T3 is four one-line changes
plus a tempdir import. T4 is the only design work — a baseline file and a generator, on
the `audit-config-surface.py` pattern. T5 is two diagnoses and may end in "stale
expectation" rather than a code fix.

Estimate ~6 iterations. T1 and T2 are independent of T3/T5 and can run in either order;
T4 must come last because it derives its baseline from T1-T3's result.

## Order and dependency

```text
T1 (checkpoint lock)  ─┐
T2 (other locks)      ─┼─> T4 (canary + baseline)
T3 (tempdir sqlite)   ─┤
T5 (two diagnoses)    ─┘
```

## What this plan explicitly does NOT do

- It does not touch the three open ledger rows (RT-190/280/543) — owner decisions.
- It does not re-do the consolidation or TUI plans' work; it unblocks their verification.
- It does not add `#[serial_test::serial]` as a blanket fix. The crate already learned
  that per-module locks do not exclude each other (`platform.rs:10-22`); one crate-wide
  lock or none.
- It does not "fix" a failing test by deleting it. A test that cannot be made
  deterministic gets a written disposition in the ledger, not a quiet removal.
- It does not regenerate or hand-edit the baseline file in T4; it writes the generator
  first.

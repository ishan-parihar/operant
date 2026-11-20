# Close the last unlocked-global read in the tool tests: the unattended refusal races the generative-UI kill switch

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent at `origin/main` = `677e802d` (iter-2740). Every number
below was MEASURED in a CLEAN worktree at that tip before it was written down.

## Claim rule

Never take a number from a header or a previous plan. `git fetch origin`, read the top
label from `origin/main`, add one. Re-run every command in the baseline before trusting
the row that cites it.

## Why this plan is small and why it exists anyway

The five iter-2726 rows are closed and the gate battery is FULLY GREEN at this tip:

```text
audit-config-surface.py --check     EXIT 0
audit-config-keys.py --check        EXIT 0
supply-chain-gate.sh                EXIT 0   (seen=5 allowlist=5 new=0)
audit-ledger-index.py               EXIT 0
check-doc-claims.sh                 EXIT 0
genome lint                         EXIT 0
clippy-warning-gate.sh              seen=0 allowlist=0 new=0 stale=0
```

That is the first time in this campaign that all six gates AND the clippy gate are green
at once. So this is not a plan about a red gate. It is about ONE remaining test failure
that the suite still produces, which the canary would call "unaccounted" if it were run
against the wrong baseline.

## The measured baseline

```text
$ cargo test -p operant-core --lib
test result: FAILED. 3664 passed; 6 failed; 2 ignored

tools::kernel::tests::a_harness_upsert_invalidates_the_injection_cache
tools::kernel::tests::a_holon_run_gates_the_continual_harness_before_its_lessons_r10
tools::kernel::tests::harness_apply_and_rollback_roundtrip
tools::kernel::tests::kernel_state_lists_the_requested_kind
tools::kernel::tests::ping_roundtrip
tools::ui_tool::tests::request_refuses_unattended     <-- the one this plan owns
```

Five are the accounted `tools::kernel::tests::*` environmental set: the worktree has no
`kernel-sidecar/.venv` (untracked; AGENTS fleet rule 6; 12 pass in the main tree).

The sixth is NOT environmental:

```text
$ cargo test -p operant-core --lib -- tools::ui_tool::tests::request_refuses_unattended
test tools::ui_tool::tests::request_refuses_unattended ... ok
test result: ok. 1 passed; 0 failed
```

Passes alone, fails in the suite. That is the signature of a shared resource.

## The defect: one test reads a process-global without holding the lock

`crates/operant-core/src/tools/ui_tool.rs`. The test module already knows this exact
hazard and has a guard for it:

```rust
/// Set `[ui] generative_enabled` for one test and RESTORE the previous
/// value on drop, holding the crate's runtime-config test lock for the
/// whole body.
///
/// The flag lives in the process-global runtime config, so a test that
/// flips it and does not restore it poisons every test that runs after it
/// in the same process ...
struct GenEnabledGuard { _lock: MutexGuard<'static, ()>, was: bool }
```

Five of the module's SIX tests take that guard. MEASURED:

```text
$ grep -c 'GenEnabledGuard::set|runtime_config_test_lock'  -> 5
$ grep -c 'async fn <test>()'                             -> 6
```

The ONE test that does not is the one that fails: `request_refuses_unattended`
(ui_tool.rs:868). It builds a `ToolContext` with `unattended = "true"` and asserts:

```rust
assert!(result.error...contains("unattended"));
```

But `UiRequestInputTool::execute` runs the GATE FIRST and the unattended guard second
(ui_tool.rs:546 then :558):

```rust
let config = crate::config::runtime_config();          // <-- process-global read
if let Err(refusal) = gate::validate_surface(&mut op, &caps, &config) { ... }
// ... only after that:
if context.metadata.get("unattended")... == Some("true") { /* "unattended run: ..." */ }
```

And the gate's first check is the kill switch (ui/gate.rs:72):

```rust
if !config.ui.generative_enabled { return Err(UiRefusal::Disabled); }
```

So when `render_refuses_when_disabled` (ui_tool.rs:769) runs concurrently and holds the
lock while setting `generative_enabled = false`, `request_refuses_unattended` can read
that value WITHOUT the lock and get `Disabled` back. The error then says "generative UI
is disabled", the substring `"unattended"` is absent, and the assertion fails.

The `GenEnabledGuard` comment already states the rule this violates: "The lock is what
makes the window closed rather than merely narrow." It is correct. This test simply does
not take it.

## The fix

Give `request_refuses_unattended` the same lock the other five take, so its read of the
global cannot interleave with a writer. Two acceptable shapes, and the choice matters:

(a) `let _guard = GenEnabledGuard::set(true);` at the top of the test -- explicit, says
    "this test depends on generative UI being enabled", and it is what its five siblings
    do. It also makes the test's precondition visible instead of implicit.

(b) take `crate::config::runtime_config_test_lock()` alone if the test should not care
    about the flag's value, only about not racing a writer.

Prefer (a). The test's meaning is "an unattended run is refused with an unattended
message", and that is only meaningful when the surface is enabled; (a) states that.

Do NOT reorder the tool to check unattended BEFORE the gate. That would make the
production behaviour depend on a test's convenience: an operator who disabled generative
UI should get "disabled", not "unattended". The tool's order is correct; the test's lock
is missing.

## Optional second tranche: make the ordering explicit in the test

The deeper lesson is that a test asserting on a SPECIFIC refusal message must control
every gate that can refuse first. A comment naming the gate and why the lock is needed
belongs beside the guard, so the next reader does not delete it as noise. That is one
comment, not a mechanism.

## Gates / success criteria

(fixed before the run)

1. `cargo test -p operant-core --lib` -- the ONLY failures are the accounted
   `tools::kernel::tests::*` set (5 in a clean worktree with no `.venv`).
   `request_refuses_unattended` is GONE from the list. Not "fewer": exactly these.
2. The test still passes ALONE (the fix must not make it pass by weakening it).
3. `python3 scripts/test-pollution-canary.py --check` exits 0, 5 accounted, 0 unaccounted.
4. All six gates stay at EXIT 0 and `clippy-warning-gate.sh` stays at `new=0`.
5. Mutation proof: remove the guard line and re-run the full suite under load; the test
   fails again by name. If the race does not reproduce in a reasonable number of runs,
   say so in the commit rather than claiming a proof not observed (this is the same
   honest-outcome rule iter-2736's commit body follows for its own non-reproducing
   mutation).

## Cost

One line in one test plus one comment. Trivially small.

Estimate 1 iteration.

## Order and dependency

Standalone. No dependency on any other plan.

## What this plan explicitly does NOT do

- It does not touch the 3 owner-gated ledger rows (RT-190 / RT-280 / RT-543) -- those
  need an operator ruling, not a patch. They have been open across this whole campaign
  and are correctly recorded as out of scope.
- It does not add the 5 kernel-sidecar failures to the canary baseline as "fixed" -- they
  are environmental and stay environmental until a worktree has the untracked `.venv`.
- It does not reorder the tool's gate to suit the test (see above).
- It does not work iter-2710's pub-fn queue (135 candidates at this tip). That plan owns
  it and the queue is shrinking on its own (146 -> 135) as peers land acid-test groups.
- It does not claim the gates are green because of anything here. They were green at
  `677e802d` BEFORE this plan; the only open item this plan closes is the one test.

## If the operator wants nothing further

This is the last NON-owner-gated, NON-environmental failure I can find at the tip. After
this row, the honest remainder is: 3 owner decisions (RT-190/280/543), 5 environmental
kernel tests, and the pub-fn queue which is owned by an existing plan. At that point the
session can close without leaving a known-red suite behind.

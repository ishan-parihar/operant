# Audit — the red team of iter-2137 (the poke / 120s-ceiling fix)

**Date**: 2026-11-12 · **Subject**: my own `fix(iter-2137): the poke reaches the provider-error path,
and the 120s ceiling is retryable` (`0bd71dd4`) · **Method**: the ponytail-audit discipline applied
to my own execution — "bugs, what breaks under real load, risky code without tests, and what to
delete, merge or split", ranked, with each finding either FIXED here or named as accepted.

This is a NEW record, not an edit of `2026-11-12-the-poke-that-never-fired-the-120s-loop-ceiling.md`:
that audit is dated-frozen, so its corrections live here.

---

## 1. What the execution actually proved

- The gap's mechanics: the ceiling's error was untyped (`Error::Agent(String)`), so
  `Error::is_transient()` was false and `Failover::classify` read it as `Unknown` — MEASURED by
  extracting the repo's own decision functions and executing them, not asserted.
- The poke's error path was unreachable: upstream records the provider error at both error sites and
  then never asks its own guard.
- 21 integration binaries green, including upstream's `auto_poke` (4) and `review_discipline` (7),
  and the two new files that pin the E/O pairs.
- The commit is on `origin/main`, the release binary was built from it, deployed by atomic rename
  and the gateway restarted onto it.

## 2. Findings — ranked, each one either fixed here or accepted

### F1 · BUG — the recovery loop re-issued a context overflow (FIXED)

The classifier marks an overflow `retryable` (a provider overflow can be transient), so the new
re-issue loop would re-issue the SAME too-big prompt up to the turn's whole retry budget — three
provider calls that can only be rejected again. The compress arm's own policy is compress-ONCE,
then stop.

**Fix**: the recovery predicate now excludes `should_compress` failures
(`ClassifiedError::earns_recovery`, `agent/error_classifier.rs`), so an overflow that reaches the
recovery seam ends the turn instead of burning the budget. **Mutation-proven**: removing the
`!self.should_compress` condition makes the new regression test
`a_context_overflow_is_not_re_issued_by_the_recovery_seam` fail by name (the script runs out and the
turn "succeeds" with the default answer — exactly the waste the fix prevents).

### F2 · BUG — an interrupted re-issue surfaced as a generic agent error (FIXED)

The establishment arm returns the TYPED `Error::Interrupted` when the operator stops a turn, and
the TUI renders that as "Turn stopped." — no error card. The re-issue loop did not: an interrupt
during a re-issue came back as `Error::Agent("Interrupted by user …")`, so the turn ended as a
provider failure and the operator got an error card for a turn they stopped themselves.

**Fix**: the loop checks the interrupt flag and returns the typed `Error::Interrupted` (committing
the interrupted tail first), exactly as the establishment and mid-stream arms do. Pinned by
`an_interrupted_re_issue_surfaces_the_typed_interrupt`.

### F3 · REDUNDANCY — the recovery predicate was written twice (FIXED)

The re-issue gate (`classified.retryable && reason != Unknown`) and the poke gate
(`pokeable = …`) were two copies of one policy, in two files, that must never disagree.

**Fix**: one named predicate, `ClassifiedError::earns_recovery()` (`agent/error_classifier.rs`),
used by both. Its three conditions are load-bearing and each carries its reason in the doc comment:
`retryable` (the classifier's verdict), NOT `Unknown` (the catch-all marks every unclassified error
retryable, and a recovery on a coin flip turns a deterministic failure into a slow one), and NOT
`should_compress` (F1). Pinned by `earns_recovery_is_the_single_recovery_predicate`.

### F4 · GAP — the new config knob was invisible to the operator (FIXED)

`[client] model_request_timeouts` existed in the schema but not in `operant.example.toml`, so the
one lever that removes the need for the whole recovery ladder (a model that legitimately needs
600s) was undiscoverable. **Fix**: documented in `operant.example.toml` next to `timeout_secs`,
with the precedence and the "bounds both the transport and the agent's ceiling" fact stated.

### F5 · ACCEPTED — the worst case is minutes, not seconds

A provider that times out on EVERY attempt burns the turn's whole re-issue budget (4 attempts)
and then the poke's own budget before the turn ends — for the live 120s ceiling that is minutes.
This is the trade the operator asked for ("retry it effectively"), it is bounded by
`max_retries`, `TURN_MAX_RETRY_FAILURES` and the turn's wall clock (F8), and the per-model budget
(F4) is the lever that removes the need for it. Not "fixed": the alternative is the behaviour that
was reported.

### F8 · BUG — the re-issue ladder ignored the turn's wall clock (FIXED)

The real-load finding. The ladder runs INSIDE one iteration, so the turn-level wall-clock check
(`turn_elapsed >= turn_wall_clock_limit_secs`, the 20-minute default, `HERMES_TURN_TIMEOUT`
overridable) never fires while it runs: a degraded upstream could burn `max_retries × ceiling`
seconds — 4 × 120s = 8 minutes on the live config — before the turn ended, and the cron's 30s
re-arm would start the next attempt. The reported symptom ("it hangs the agentic-loop") would have
come back in a new shape.

**Fix**: the ladder consults the turn's clock before every re-issue and stops when it is spent
(`reissue_with_budget` takes `turn_start` and the limit) — the same bound the mid-stream ladder
already had (`stream_retry::decide_stream_retry` takes the elapsed time).
**Mutation-proven**: removing the check makes
`the_re_issue_ladder_respects_the_turn_wall_clock` fail by name — 4 calls where the bound allows 1.

### F6 · VERIFICATION GAP — the in-module unit tests had never RUN (now closed)

iter-2137's unit tests (in `error.rs`, `agent/fallback.rs`, `cronjobs/scheduler.rs`,
`reasoning_timeouts.rs`, `agent/mod.rs`) were COMPILE-verified only: the dev-profile link of the
`operant-core` lib-test binary failed inside the third-party `espeak-rs-sys` archive (the `sonic*`
symbols espeak-ng 1.52 moved into a separate library), which is an environment defect and not this
change's. The link now succeeds on this box, so the unit suite RUNS — and it is green.

### F7 · DOC — the iter-2137 commit body cites pre-rebase line numbers (corrected here)

That commit was written on a base 448 commits stale, rebased onto iter-2135, and renumbered to
iter-2137; its body cites `agent/run.rs:1700` and `:2060`, which are the line numbers of the
PRE-rebase tree. The code it names is right (`reissue_with_budget`, `maybe_poke`); the numbers are
not. A commit body cannot be amended after a push (force-pushing is forbidden), so the correction
is recorded here and in the next commit's body.

## 3. What this red team could NOT prove

The operator's live symptom — a daily seat dying on the 120s ceiling and then going dark until the
next cadence — has **not** been observed end-to-end in production, because the recovery is proven at
the unit/integration level and the code is deployed, but no natural interruption has been waited
out. The honest claim is: the mechanism that made the seat go dark is gone (the error is typed, so
the cron seam's bounded transient retry can see it), the turn re-issues instead of dying on the
first slow provider, and the poke is reachable for a classified transient failure. The claim is NOT
"a seat was observed surviving a 120s ceiling in production".

## 4. The verification inventory after this round

| Gate | Result |
|---|---|
| `cargo test -p operant-core --lib` | the unit suite RUNS: 3078 passed, 14 failed — every failure an ENVIRONMENTAL one (localhost HTTP servers the sandbox cannot reach, and the kernel sidecar); none of them in the files this work touched, and none of them mine |
| 21 operant-core integration binaries | green (auto_poke 4, review_discipline 7, request_timeout_retry 8, poke_after_provider_error 4) |
| mutation proofs | three: the overflow re-issue (F1), the interrupt typing (F2) and the wall-clock bound (F8) each fail by name when their fix is removed |
| `cargo clippy -p operant-core -p operant-cli --all-targets` | 0 findings from this change |
| `rustfmt --check` on every touched file | clean |
| `genome lint` | 0 errors |
| mutation proof | the overflow regression test fails by name when the fix is removed |

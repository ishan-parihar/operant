# Close the residual suite-interaction and cleanup gaps: the cwd poisoner has more victims than iter-2711 found

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent at `origin/main` = `c9541f14` (iter-2721). Every number
below was MEASURED in a CLEAN worktree at that tip before it was written down.

## Claim rule

Never take a number from a header or a previous plan. `git fetch origin`, read the top
label from `origin/main`, add one. Re-run every command in the baseline before trusting
the row that cites it.

## What is already CLOSED (so this plan does not re-open it)

Measured at `c9541f14`, clean worktree:

| gate | at iter-2710's plan | now | who closed it |
|---|---|---|---|
| `audit-config-keys.py --check` | EXIT 1, 31 keys | **EXIT 0** | iter-2719 / iter-2720 |
| `supply-chain-gate.sh` | EXIT non-zero, 3 advisories | **EXIT 0** (`seen=5 allowlist=5 new=0`) | triaged |
| `audit-ledger-index.py` | 4 FAIL | **OK** | earlier campaign |
| `clippy` warnings (16, from iter-2697) | red | **`new=2`** | mostly pruned |

So of iter-2710's G1 and G2, both are GREEN. Only G3 (the 146 pub-fn queue) remains
open, and G4 (3 ledger rows) is owner-gated. This plan owns what those plans do NOT.

## The measured baseline

```text
$ cargo test -p operant-core --lib
test result: FAILED. 3657 passed; 11 failed; 2 ignored; 0 measured; 0 filtered out

$ python3 scripts/audit-config-surface.py --check
AppConfig: 2/39 ... ui <-- NOT ACCOUNTED FOR      EXIT 1

$ ./scripts/genome lint
E019 DRIFT TUI overlays: AGENTS.md says 29, the array holds 32
E104 AGENTS.md#stack: zone `stack` is stale            2 errors

$ bash scripts/clippy-warning-gate.sh
[clippy-gate] seen=2 allowlist=0 new=2 stale=0
```

## Row 1 -- the cwd poisoner has TWO more victims than iter-2711 found

iter-2711 fixed the chdir/restore race in `config.rs` (`CwdRestore`) and `agent/mod.rs`
(`ChdirGuard`) and took the suite from 27 failures to 5. MEASURED now: the SAME
mechanism still bites `tools::working_diff_tool::tests::{untracked_cap_50,
working_diff_staged_vs_working}`.

The evidence is direct. From the suite log, the tests that PASS and then FAIL:

```text
test tools::working_diff_tool::tests::not_a_git_repo_errors ... ok
test tools::working_diff_tool::tests::diff_output_truncated ... ok
test tools::working_diff_tool::tests::untracked_cap_50 ... FAILED
```

And the panic itself:

```text
thread 'tools::working_diff_tool::tests::untracked_cap_50' panicked at
  crates/operant-core/src/tools/working_diff_tool.rs:203:18:
  git: Os { code: 2, kind: NotFound, message: "No such file or directory" }
```

`ENOENT` on `Command::new("git")` is the dead-cwd signature, not a missing git binary
(`which git` resolves). Both tests pass in isolation -- 5/5 runs each:

```text
$ cargo test -p operant-core --lib -- tools::working_diff_tool::tests::
test result: ok. 6 passed; 0 failed        (x5 consecutive runs, 0 failures)
```

The tool itself is fine: it takes an explicit `path` in every test and calls
`Command::current_dir(&root)`. What it inherits is the PROCESS cwd, and the process cwd
is what a poisoned test leaves behind. So this is the same class as iter-2711, not a new
one, and it needs the same discipline applied to the SITE THAT STILL LEAKS.

**Diagnose before fixing.** iter-2711 added guards to two sites; a third site is still
leaking, or a guard's tempdir is deleted before its guard drops. The fix is to find the
actual leaker (instrument `set_current_dir` with a panic-hook probe, or run the suite
with the working_diff pair marked and bisect the file list), then apply the SAME
`CwdRestore` pattern. Do NOT add `working_diff` to the canary baseline until the leaker
is named -- an unexplained environmental row is how a real bug becomes invisible, which
is the whole point of iter-2712.

## Row 2 -- the ephemeral-port tests fail at CONNECT, not at bind

`gateway::tests::test_webhook_hmac_live_server` and both `gateway::slack::tests::*`
fail only in the full suite. The webhook test's own comment names a TOCTOU race and
retries on it:

```rust
// The probe is DROPPED before the adapter binds, and that is a
// TOCTOU race: another test's ephemeral bind can take the port in
// between, and the adapter then fails with `AddrInUse`. So try a
// few ports -- the retry is on the HARNESS side only.
for _ in 0..5 { ... if !in_use { break; } }
```

But the OBSERVED failure is not `AddrInUse`:

```text
panicked at crates/operant-core/src/gateway/mod.rs:2574:14:
  reqwest::Error { kind: Request, ... ConnectError("tcp connect error",
  127.0.0.1:45539, Os { code: 111, kind: ConnectionRefused }) }
```

The retry only catches a bind failure. A port that binds fine and then goes away (or a
server task that dies before the client's readiness loop ends) surfaces as
`ConnectionRefused` at connect, which the retry does not cover and the readiness loop
treats as "not ready yet" until it falls through to a hard `unwrap`. The slack pair fails
the same way (`error sending request for url (http://127.0.0.1:NNNNN/chat.postMessage)`).

A peer already cut the worker count to 2 for the slack tests and documented why:

```text
// a multi-thread runtime defaults to `num_cpus` workers, so on a 24-core
// box every concurrent test starts 24 of them and the box drowns in
// runnable threads ... the request fails with "error sending request"
```

That is a real mitigation and it is still not sufficient at full-suite parallelism. So
this row is: make the harness fail loudly on the FIRST unready attempt with the real
error, and make the retry cover connect-class errors, not just `AddrInUse`. If the box
is simply oversubscribed, the honest fix is a shared concurrency limit for the
network-touching tests, not a longer sleep.

**Verify:** run the full suite three times; zero failures in this class. Mutation:
shorten the readiness loop and watch it fail by name.

## Row 3 -- `audit-config-surface.py` cannot see a 2-character field (FALSE POSITIVE)

```python
for ident in set(re.findall(r"\b[a-z_][a-z0-9_]{2,}\b", text)):
```

`{2,}` needs THREE characters, so `ui` and `pk` are invisible. Exactly two fields are
that short. `pk` is already in `ACCOUNTED_FOR`, so its blind spot is masked and
harmless. `AppConfig.ui` is NOT, and the ratchet is RED on it -- while `ui` IS read at
five sites:

```text
crates/operant-core/src/ui/gate.rs:72       if !config.ui.generative_enabled {
crates/operant-core/src/ui/gate.rs:118      config.ui.generative_enabled = enabled;
crates/operant-core/src/tools/ui_tool.rs:697    crate::config::runtime_config().ui.generative_enabled
crates/operant-core/src/tools/ui_tool.rs:699    config.ui.generative_enabled = enabled;
crates/operant-core/src/tools/ui_tool.rs:708    config.ui.generative_enabled = self.was;
```

So the gate reports a live setting as dead. It surfaced only now because iter-2699
registered the `[ui]` section, making the field reachable to the ratchet's struct walk.
**Fix the DETECTOR** (scan each field name explicitly with a word-boundary match, keep
the general identifier scan for everything else). Do NOT add `AppConfig.ui` to
`ACCOUNTED_FOR` -- that records a lie. This is the same T3 that iter-2716's plan carries;
whichever lands first, the other should cite it rather than duplicate it.

**Verify:** `--check` prints 0 new candidates, AND a mutation removing the
`ui/gate.rs:72` read makes it fail by name.

## Row 4 -- the two new clippy warnings (mechanical)

```text
+ continue_session|clippy::redundant_closure|crates/operant-cli/tests/continue_session.rs
+ operant|unused_imports|crates/operant-cli/src/tui/operant_ui/mod.rs
```

Both are one-line mechanical fixes. `continue_session.rs:476,480` use `.map(|o| ...)` /
`.unwrap_or_else(|e| e.to_string())` where the closure is redundant; the
`operant_ui/mod.rs` import is simply unused. Either fix them or annotate with
`#[expect(..., reason = ...)]` per the gate's own protocol -- never `--update` to hide.

**Verify:** `clippy-warning-gate.sh` reports `new=0`.

## Row 5 -- two empty TUI stubs and the two genome zones

`crates/operant-cli/src/tui/rustle.rs` (`tui/mod.rs:114`) and `tui/figures.rs`
(`tui/mod.rs:32`) are both declared `pub mod` and both contain ZERO lines of code -- only
a comment and an empty `#[cfg(test)] mod tests {}` in `rustle.rs`, and only a module doc
comment plus a pruned-constants note in `figures.rs`. The 2026-11-20 dead-code audit
(`docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`)
classified both as purgable and its disposition table still says "this iter". Nothing
imported them: `grep` for their symbols outside the two files returns only unrelated
comment matches. Verify `PROCESS_TITLE` truly has no caller, then delete both files and
their `pub mod` lines.

The two genome errors are generator fixes:

```text
E019 DRIFT TUI overlays: AGENTS.md says 29, the array holds 32
E104 AGENTS.md#stack: zone `stack` is stale
```

`genome outline` clears E104. For E019, derive the overlay count from
`App::overlay_flags()` in `check-doc-claims.sh` -- the authority AGENTS.md already
names -- rather than hand-typing it, because a hand-typed number beside a growing array
is exactly how it drifted.

**Verify:** `genome lint` prints 0 errors, and a mutation (set the prose to 29) fails
`check-doc-claims.sh` by name.

## Gates / success criteria

(fixed before the run)

1. `cargo test -p operant-core --lib` -- the ONLY failures are the accounted
   `tools::kernel::tests::*` environmental set (5 in a clean worktree with no `.venv`).
   `working_diff_tool`, `lifeos`, `slack` and `webhook_hmac` are all GONE. Not "fewer":
   exactly these.
2. `python3 scripts/test-pollution-canary.py --check` exits 0 with 5 accounted and 0
   unaccounted.
3. `python3 scripts/audit-config-surface.py --check` exits 0, no new candidates.
4. `./scripts/genome lint` prints 0 errors.
5. `bash scripts/clippy-warning-gate.sh` reports `new=0`.
6. `python3 scripts/audit-config-keys.py --check` and `bash scripts/supply-chain-gate.sh`
   stay at EXIT 0 (do not trade Row 1-5 for a regression there).
7. Every fix ships a mutation proof: reintroduce the defect, watch the named test or
   gate fail by name.

## Cost

Row 1 is diagnosis plus one guard -- the diagnosis is the work, and it is the row most
likely to be mis-fixed by adding a sleep. Row 2 is one harness change plus a
concurrency decision. Row 3 is one regex plus a per-field check. Row 4 is two lines.
Row 5 is two file deletions plus two generator runs.

Estimate 4-5 iterations. Row 1 first, because it is the one with a wrong-fix trap.

## Order and dependency

```text
Row 1 (find the leaker)  ----> Row 2 (harness connect-class)   both touch the suite
Row 3 (ratchet regex)     ----> independent (shared with iter-2716 T3)
Row 4 (clippy)            ----> independent, cheapest, do first for a quick green
Row 5 (stubs + zones)     ----> independent
```

Row 4 first (cheapest, unblocks the "clippy new=0" gate). Row 1 and Row 2 both change
suite-level behaviour, so land them separately and re-run the FULL suite between them.

## What this plan explicitly does NOT do

- It does not re-open G1 (config keys, closed iter-2719/2720) or G2 (supply chain,
  triaged). It asserts they STAY green.
- It does not touch the 3 owner-gated ledger rows (RT-190/280/543).
- It does not work iter-2710's G3 pub-fn queue (146 candidates) -- that plan owns it.
- It does not add an unexplained failure to iter-2712's canary baseline. Rows 1 and 2
  must be DIAGNOSED before anything is declared environmental.
- It does not fix a gate by deleting it. Row 3 fixes the detector; Row 5 deletes only
  files that provably contain zero code.
- It does not re-do iter-2716's plan; where they overlap (Row 3 == iter-2716's T3),
  whichever lands first wins and the other cites it.

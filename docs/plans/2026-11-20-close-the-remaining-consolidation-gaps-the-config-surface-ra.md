# Close the remaining consolidation gaps: the config-surface ratchet is red again, the supply gate has 3 new advisories, and 146 pub-fn candidates wait on their acid tests

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Why this plan exists

The previous tranche (iters 2680-2709) closed the doc/ledger/genome debt: genome lint
went 16 errors -> 0, the ledger index's 4 FAILs cleared, and the pub-fn surface auditor
landed. Re-running the FULL gate battery against `origin/main` (`c99382d1`) after peers
advanced it found the debt had already come back, in three new places. Every row below
is measured on `origin/main`, not inferred.

## Mechanism

### The rows

**G1 (blocking, 31 keys) - `scripts/audit-config-keys.py --check` exits 1.**

31 registered config keys miss one or more of the four required surfaces (note / heal /
template / doctor). The gate is a RATCHET by design (`LEGACY` baseline, it can only
shrink), and a key NOT in `LEGACY` that misses ANY surface is a hard FAIL. These 31 are
not in `LEGACY`, so the ratchet is genuinely violated, not merely untidy:

    advisor.*           (4)  proxy.*            (6)  observability.*   (6)
    compression.*       (3)  providers.*        (5)  ui.generative_enabled (1)
    recap.instructions  (1)  tools.kernel.*     (4)

`[proxy]` is the newest offender and the clearest illustration: `ProxyConfig`
(`crates/operant-config/src/schema/proxy.rs`) landed with `enabled`/`http_proxy`/
`https_proxy`/`all_proxy`/`no_proxy`/`scope`/`services` and none of them reached
`config_heal.rs`, `operant.example.toml` or `doctor.rs`. A key the operator can set but
no surface explains or validates is exactly the failure this gate was built to catch.

Per key, the fix is one of: add the heal entry, add the template line, add the doctor
row, or add the key to `NO_HEAL_NEEDED`/`NO_TEMPLATE`/`NO_DOCTOR` WITH the reason. The
`DRIFT` block is the worklist.

**G2 (blocking, 3 advisories) - `scripts/supply-chain-gate.sh` exits non-zero.**

`seen=5 allowlist=2 new=3`:

    RUSTSEC-2024-0436  paste@1.0.15          unmaintained (transitive, widely held)
    RUSTSEC-2026-0543  chrono@0.4.45         unmaintained
    RUSTSEC-2026-0459  libsqlite3-sys@0.30.1 vulnerability, via rusqlite 0.32.1
                       <- memory-wire -> operant-core -> operant-cli/operant-os

`libsqlite3-sys` is the one with a real fix path (bump `rusqlite` so the sys crate moves
off 0.30.1, or pin to a patched 0.30.x). `paste` is a proc-macro helper with no
replacement and no exploit; `chrono` is a candidate for `time`/`jiff` or for a version
bump. Each needs a triage record in `docs/audit/2026-06-19-BUGS.md` (the gate's own
instruction: "triage each one and record it in BUGS.md before allowlisting it") before it
may be allowlisted. Do not allowlist an untriaged advisory.

**G3 (large, 146 candidates) - the pub-fn surface queue from iter-2708.**

146 pub fns remain in `scripts/audit-pub-fn-surface.py`'s queue. A candidate is NOT a
verdict: the acid test is to comment out the `pub`, `cargo check -p operant-core --lib`,
restore. The queue is deliberately not all-or-nothing; it is worked in small coherent
groups, one iteration per group, each acid-proven and guarded.

Three groups where the acid test is likely to be cheap and the win is real:
  - `tests=0` candidates (no test even names them) - the strongest prior for dead:
    `all_records_mut`, `approver_of`, `build_authorization_url`, `build_memory_context`,
    `child_sessions`, and ~30 more.
  - the single-candidate files (`context_window.rs`, `estop.rs`, `rate_limiter.rs`,
    `turn_end.rs`, `config_heal.rs` ...) - one acid test closes the file.
  - `database.rs` (16) and `memory.rs` (9) - the two densest files; likely a mix of
    genuinely dead and per-crate-public API, so acid-test each.

**G4 (report only, not actionable from here) - 3 open ledger rows, owner-gated.**

RT-190 (memory-wire bank topology: ruled option (c), the (b) bridge stays a future
tranche), RT-280 (gateway/ACP single-server design decision), RT-543 (session
resume-pending after restart). Each is blocked on an owner ruling or an architectural
decision, not on code. Record them as remaining, do not fake a fix.

## Gates / success criteria

(fixed before the run)

1. `python3 scripts/audit-config-keys.py --check` exits 0 on `origin/main` + the branch.
2. `bash scripts/supply-chain-gate.sh` exits 0, with every advisory either fixed or
   triaged in BUGS.md with its reasoning.
3. `./scripts/genome lint` stays at 0 errors (do not trade G1/G2 for a genome error).
4. `python3 scripts/audit-ledger-index.py` stays OK; `bash scripts/check-doc-claims.sh`
   stays green; `python3 scripts/audit-pub-fn-surface.py --check` passes or is updated
   only by entries whose acid test was actually run.
5. Each code change has a regression test that fails without it, mutation-proven.
6. Every gate row above is verified green on a FRESH worktree at the branch tip before
   the final push, because peers advance `origin/main` mid-run (observed twice on
   2026-11-19).

## Cost

G1: 31 keys x ~3 surfaces, mostly mechanical, but each needs a real decision (heal
entry vs. `NO_*` exemption with reason). Estimate 2-3 iterations, one per key family
(proxy / observability / advisor+recap+ui / compression+providers+kernel).

G2: 1-2 iterations. `libsqlite3-sys` needs a dependency bump + full test run; `paste`
and `chrono` are triage-and-record.

G3: unbounded by nature. One iteration per coherent group, acid-proven. G1+G2 are the
blocking half; G3 is the compounding half that can run for as many iterations as the
operator wants.

G4: zero cost, report only.

## Out of scope

- The 3 owner-gated ledger rows (G4): a ruling, not a patch.
- `genome lint`'s 8 warnings (W006/W010/W014): pre-existing, belong to other plans.
- Re-litigating iter-2693's TUI-count work or the RT-555 renumbering (done).

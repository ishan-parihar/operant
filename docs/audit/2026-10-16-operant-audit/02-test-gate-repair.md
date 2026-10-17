# 02 — Make the test gate real again

**Size:** S. **Do first.** Every other plan's "write the RED test" step depends on this.

## Evidence (2026-10-16, `main` @ 0ee25739)

`cargo check --offline --keep-going --tests -p operant-core -p operant-cli -p operant-os` → exit 101:

| Target | Errors | Cause |
|---|---|---|
| `operant-cli` bin test | 11 | `gateway_commands.rs:2611-2690` (`P3Stores`, `seed_p3`, `p3_seed_employee`) uses `org::{authority, employee_db, hierarchy_edges, pending_requests, notice}`; `gateway_runner.rs:4558` uses `org::worklog_db` |
| `operant-core` test `org_wave2_shared_file` | 22 | retired `notice`, `employee_db`, `authority`, `resolver`, `dm_thread`, `schema`, `pending_requests`, `notice_db`, `hierarchy_edges`, `employee`, `department_db`, `decisions_db` |
| `governance_escalation` | 18 | retired org modules |
| `seat_policy_run_path` | 9 | `org::authority`, `org::pending_requests`; `SeatPolicy.delegation` field gone; one fn arity change (`:340`) |
| `cron_session_isolation` | 7 | `org::write_barrier`, `org::WorklogDb/WorklogQuery`, `org::employee`, `CronScheduler::with_write_barrier` |
| `org_notice_board` | 3 | `org::notice`, `org::notice_db` |
| `org_employee_registry` | 2 | `org::employee`, `EmployeeDb`… |
| `org_identity_gate` | 2 | `org::employee`, `org::identity_gate` |
| `org_worklog_framework_only` | 1 | `org::worklog_db` |
| `integration_test` | 1 | `tools::builtin::CamofoxStateTool` (purged in 11C2) |

`operant-os` tests compile.

Consequence: gate 2 (`cargo test -p operant-cli --bin operant`) has been red since
iter-838 (G+1 deleted the org layer). The 2026-10-15 brief §3.5 lists tests that
were "committed, not yet run": `tool_output_budget_evicts_oldest_first` and the
rewind-selector tests. They are still unrun.

## Rule for each target (from `docs/agentic-os/ORG-LAYER-DISPOSITION.md`)

Kept org modules: `seat_policy`, `seat_policy_db`, `seat_budgets`,
`seat_authority` (personal mode), `context_injection`, and `worklog`'s accumulator.
Everything else is Holon's now.

- A test whose **subject** is a retired module: **delete the file or test.** Do
  not port it. Holon owns that behaviour and has its own tests.
- A test whose subject is **kept** but which uses a retired module only as
  **fixture**: rewire the fixture to the kept module and keep the assertions.

| Target | Action |
|---|---|
| `gateway_commands.rs` tests `grant_*`, `revoke_*` (7 tests, `:2697-2909`) | **Delete.** Production `grant`/`revoke` now just print a Holon redirect (`gateway_commands.rs:2254`). Delete `P3Stores`, `seed_p3` and `p3_seed_employee` too. |
| `gateway_commands.rs` `permissions_shows_posture_and_gates_other_seats` (`:2910`) | **Rewire.** `/permissions` reads `seat_policy`. Seed only a `SeatPolicyDb` row (see `crates/operant-core/src/org/seat_policy_db.rs` for the constructor) and keep the posture assertions. |
| `gateway_runner.rs:4558` test | Read the test. If it asserts on worklog **rows** (retired `worklog_db`), delete it. If it asserts on the accumulator, use `org::worklog`'s kept API. |
| `org_wave2_shared_file.rs`, `governance_escalation.rs`, `org_notice_board.rs`, `org_employee_registry.rs`, `org_identity_gate.rs`, `org_worklog_framework_only.rs` | **Delete the files.** Every subject is retired. |
| `seat_policy_run_path.rs` | **Rewire.** The subject (seat policy on the run path) is kept. Drop the `authority`/`pending_requests` fixtures, remove the `delegation:` field from the `SeatPolicy` literals (`:422,473,572,619,666`), and fix the call at `:340` to the current signature. Delete a test only when it asserts on a pending-request row or grant row. |
| `cron_session_isolation.rs` | **Rewire.** Cron session isolation is kept behaviour (BUGS D-1). Remove the write-barrier and worklog parts (`:37-38,261-267,380,413`). Keep the conversation-isolation assertions. |
| `integration_test.rs:20` | Remove the `CamofoxStateTool` import and its test. |

Also delete the stale comments that name retired modules as live, at
`crates/operant-core/src/agent/stream.rs:1083-1120` (`pending_requests` queue,
`org/authority.rs (can_delegate)`). They mislead the next reader.

## Steps

1. Worktree from `origin/main`. Apply the table above.
2. `cargo check --offline --keep-going --tests -p operant-core -p operant-cli -p operant-os`
   → exit 0.
3. `cargo test -p operant-cli --bin operant`, then `cargo test -p operant-core`
   (lib plus integration). Record the failures. Allowed: K-1 (`tools::kernel`
   `ping_roundtrip`, `harness_apply_and_rollback_roundtrip`) and the known
   parallel-DB flake. Any other failure is a real regression that was hidden by
   the red gate. Put each one in `BUGS.md` with its name and first error line,
   and fix it in its own iteration.
4. Run the brief's deferred tests by name and report them:
   `cargo test -p operant-cli --bin operant tool_output_budget_evicts_oldest_first`
   and the rewind-selector tests (`grep -rn "fn .*rewind_selector" crates/operant-cli/src`).
5. Add a CI-style guard to `scripts/` (or extend the existing gate script) that runs
   `cargo check --tests --workspace` so a module deletion can't leave dead tests again.

## Acceptance

- `cargo check --tests --workspace` exits 0. (I checked only core, cli and os.
  If `operant-channels`, `operant-gateway` or `operant-runtime` test targets
  also fail, don't repair them. Those crates are slated for deletion (`10`).
  Use `-p` scopes until they are gone.)
- Gate 2 and gate 3 have been run, and the failures (if any) are listed in
  `BUGS.md`, each with an owner iteration.
- The commit message lists every deleted test by name. That is the audit trail
  that these tests went with their feature and were not just silenced.

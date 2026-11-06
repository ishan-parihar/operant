# close the executional limitations found by the red-team protocol

Dated-frozen: published `2026-11-05`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Mechanism

Work the register in `kb/gap-register.md`, one row per iteration, in this order:

1. **G-006** — widen the MCP client's response `id` from `u64` to `serde_json::Value`.
   The server's own type is already `Value` (`crates/operant-cli/src/mcp_serve.rs:39`) and
   it emits string ids (`mcp_serve.rs:822`). One line in the client, one test.
2. **G-010** — add a cron overlay to the TUI, registered in the ONE catalogue
   (`overlay_flags()` in `crates/operant-cli/src/tui/app/commands.rs`). A second list is
   the RT-251 bug again.
3. **G-007** — MCP-backed tools must not inherit the 30 s tool timeout.
4. **G-009** — the TUI/gateway command-parity sweep (canonical RT-179). This is what makes
   the TUI the *entire* operator interface rather than nearly all of it.
5. **G-005** — land and push the MCP drain fix (iter-1822; ready, 27/27 green).
6. **G-004 / G-008 / G-013** — deploy and config, not code: re-deploy the aft bridge,
   prefix the cron job models, gitignore or delete the junk dirs.
7. **G-011 / G-012** — make the index-backed search paths use the index (or say they did
   not), and settle `file_state`'s documented-but-absent `read` arm.

Before any row: re-reconcile against `origin/main` (see `kb/red-team/03-reconciliation-vs-upstream.md`).
Seven of the sweep's original fourteen rows were closed upstream while the sweep was
running — an unreconciled register sends the executor to re-fix closed work.

## Gates / success criteria

- `cargo test -p operant-core --lib mcp::` → 27 passed, 0 failed.
- `cargo test -p operant-core --test auto_poke` → pass.
- `bash operant-testing/kb/red-team/docs/verify.sh` → no CLOSED finding regressed.
- `python3 scripts/genome lint` (in `operant-testing/`) → clean.
- `cargo fmt --all && cargo check --workspace && cargo test --workspace` → green.
- `bash scripts/clippy-warning-gate.sh` → green.

## Cost

Roughly one iteration per row. G-006 and G-010 are each well under an hour of work plus
their tests; G-009 is the largest (a 72-command parity sweep) and may deserve its own
plan. The DEPLOY rows are minutes each once the operator decides Q1/Q2 in
`docs/kb/ASKS.md`.

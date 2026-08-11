# Operant Production-Readiness Remediation Plans

Stamped against commit `d394c136` (R30, binary v0.1.4). Audit date: 2026-08-09.
Audit sources: 30 prior parity rounds (R1–R30, see `BUGS.md`), full workspace scan
(19 crates, ~430k LOC), live binary smoke tests, hermes-agent (Python parent) references.

## Execution model (per user decision)

- **CI/CD is manual.** CI workflows are tag/manual-triggered and are **not** part of the
  gate. All validation is **local**, via the commands below. No tags/releases until the
  baseline is finalized.
- Every plan follows the established **R-round protocol** from `AGENTS.md` / `BUGS.md`:
  surgical change → local verify → commit+push with `R##:` prefix → update `BUGS.md`
  with findings/fixes → deploy binary + live smoke where behavior is user-visible.

### Local validation gates (every plan)

```bash
source scripts/dev-env.sh 2>/dev/null || true   # if present
cargo fmt --all && cargo fmt --all --check
export LIBCLANG_PATH=/usr/lib/llvm21/lib
cargo clippy -p <touched-crate> --all-targets -- -D warnings     # touched crates
cargo test  -p <touched-crate> --lib                             # touched crates
# Final per-plan gate (slow, run once at the end):
cargo test --workspace --all-features --lib
```

## Priority order & dependency graph

```
P0 — green baseline + hard blockers (do first)
  001-local-quality-baseline      ← nothing depends on it; everything builds on it
  002-config-secret-permissions   (independent)
  003-gateway-stub-surface        (independent; feeds 013)
  004-db-migration-framework      (independent)
P1 — core parity gaps (user-designated core functionality)
  008-parity-working-diff         (independent)
  009-parity-remote-skill-sync    (independent; touches skills registry)
  010-parity-tui-niceties         (independent; TUI)
  011-parity-daemon-delegation    (independent; delegation)
  012-parity-security-approvals   (independent; security)
  006-agent-loop-reconciliation   (large; independent — biggest single item)
P1/P2 — structural debt
  005-dead-code-decommission      (do after 001; overlaps 003/007 surfaces)
  007-telemetry-consolidation     (independent, small)
  013-integration-tests           (after 003 + 005 so surfaces are final)
  014-docs-truthfulness           (last; reflects all finalized decisions)
```

Recommended round assignment: R31 = 001 · R32 = 002+003+004 (small blockers, one round)
· R33 = 008 · R34 = 009 · R35 = 010 · R36 = 011 · R37 = 012 · R38–R39 = 006 ·
R40 = 005 · R41 = 007+013 · R42 = 014. Adjust to reality; keep the dependency order.

## Status

| Plan | Priority | Status |
|------|----------|--------|
| 001-local-quality-baseline | P0 | ⬜ planned |
| 002-config-secret-permissions | P0 | ⬜ planned |
| 003-gateway-stub-surface | P0 | ⬜ planned |
| 004-db-migration-framework | P0 | ⬜ planned |
| 005-dead-code-decommission | P1 | ⬜ planned |
| 006-agent-loop-reconciliation | P1 | ⬜ planned |
| 007-telemetry-consolidation | P1 | ⬜ planned |
| 008-parity-working-diff | P1 | ⬜ planned |
| 009-parity-remote-skill-sync | P1 | ⬜ planned |
| 010-parity-tui-niceties | P1 | ⬜ planned |
| 011-parity-daemon-delegation | P1 | ⬜ planned |
| 012-parity-security-approvals | P1 | ⬜ planned |
| 013-integration-tests | P2 | ⬜ planned |
| 014-docs-truthfulness | P2 | ⬜ planned |

Executor updates the Status column after each plan ships.

## Explicitly out of scope (user decision — separate CLI/MCP add-ons exist)

Tencent/Yuanbao, X/Twitter search, video generation (image/video gen), desktop_ui,
website_policy, and any other integration reachable via the separate CLI/MCP interface.
These are NOT core; do not plan or implement them here. Rejected during vetting, do
not re-audit.
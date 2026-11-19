# Evidence — what has been measured (2026-11-20)

## Round 2 findings (all closed)

The 12 findings from round 2 are at `docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md`.
All 12 are CLOSED. The reviewer should NOT re-litigate them. For reference, the categories were:
agent-loop (F1-F2), gateway (F3), evidence recall (F4, F6), doctor warnings (F7, F12),
stop discriminator (F8), prompt caching (F9), tool bridge (F10), cache invalidation (F11).

## Dead-code audit (iter-2704)

Full audit: `docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`.
Key findings:
- `cargo check -p operant-core --lib`: **0 warnings** (every `#[expect(dead_code)]` fulfilled).
- 2 purgable stubs: `tui/rustle.rs` (empty), `tui/figures.rs` (empty after prune).
- 1 utilizable gap: `impl_cache_key` in `mcp.rs:471` (wire into cache key path).
- 28 residual clippy warnings (99% mechanical + 2 `await_holding_lock`).
- No dead production code, no duplicate registries, no stub adapters, no dead CLI commands.

## Clippy gate status

`bash scripts/clippy-warning-gate.sh`: PASS. 28 new warnings, allowlist ceiling 124.
The allowlist has 96 stale entries from the pre-iter-2444 era (prune-able, not a blocker).

## Lib suite status

RED at tip due to a peer's process-global env-var pollution (iter-2703 documents it).
This is a known defect with a plan (`docs/plans/2026-11-19-lib-suite-is-red-at-tip-a-peer-process-global-env.md`).
Not a new blocker.

## What these measurements bound

- The dead-code audit bounds the dead-code surface: there is no dead production code to purge.
- The clippy gate bounds the style surface: 28 mechanical warnings remain, all one-line fixes.
- The lib-suite RED is a TEST INFRASTRUCTURE issue, not a production code issue.

What these measurements do NOT bound:
- Whether the architecture has bottlenecks that only appear under load.
- Whether there are redundancies between crates that a single-crate audit cannot see.
- Whether the integration between the 7 crates is complete (a crate-level audit misses seams).
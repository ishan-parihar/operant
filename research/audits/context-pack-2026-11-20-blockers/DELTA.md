# Delta since round 2 (2026-11-19 → 2026-11-20)

This section carries what landed on `origin/main` since round 2's pack was cut
(`df1ea4c0..origin/main`). Each row is a pushed commit with a verified status.

## The Hydrogen campaign closed (iters 2671–2692)

Round 2 returned 12 findings. Every one was verified against the live tree and dispositioned:

| # | finding | verdict | fix | commit |
|---|---|---|---|---|
| F1 | no circuit breaker on agent loop | CONFIRMED | `PrimaryBreaker` + per-candidate `clamp_max_tokens_to_model` | iter-2679 |
| F2 | max_tokens unbounded per candidate | CONFIRMED | `clamp_max_tokens_to_model` in `process_candidates` | iter-2679 |
| F3 | gateway turn retry span unbounded | CONFIRMED | `gateway_turn_ceiling_secs()` caps retry span | iter-2680 |
| F4 | evidence recall not wired to persisted facts | CONFIRMED (peer) | `guard_tool_result` accepts `persisted_key` | iter-2688 (F6) |
| F5 | `HistorianKey` never read | REFUTED (already wired) | — | — |
| F6 | tool result dropped evidence key | CONFIRMED | `guard_tool_result` points at `evidence_recall "<key>"` | iter-2688 |
| F7 | doctor never warns on parked key | CONFIRMED | `historian_key_is_parked` warning | iter-2672 |
| F8–F11 | (round-2 secondary) | CONFIRMED-NARROWER | various | iters 2684–2687 |
| F12 | `historian_key_is_parked` not a doctor warning | CONFIRMED | `historian_key_is_parked` in doctor | iter-2672 |

## Residual cleanup closed (iters 2693–2702)

- **iter-2694**: OUTLINE.md crate-graph claim corrected (operant-core does NOT reach operant-plugins);
  MCP server attribution fixed (server is in operant-cli); AGENTS.md slack.rs handle_update stale
  claim corrected; TUI count 73→74. Flipped RT-136/RT-285 checkboxes (stale, closed by code).
- **iter-2695**: Deleted `gateway/admin.rs` (179 lines, zero production callers) + re-exports.
- **iter-2698**: Fixed 5 E303 dead file citations in library docs (write_approval.rs, schema/media.rs,
  tts_provider.rs, tts_registry.rs, tts_command_provider.rs all deleted).
- **iter-2701**: Added 6 E311 exemptions to genome.toml coverage_exempt; tombstoned 2 superseded plans.
- **iter-2702**: Clippy cleanup — empty_line_after_doc_comments, manual_clamp×2,
  unfulfilled_lint_expectations, unused_variables, 11 `#![allow(dead_code)]` on vendored jcode TUI.

## Dead-code audit (iter-2703, iter-2704)

Full audit at `docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`.
Summary: operant-core lib is CLEAN (0 warnings, every `#[expect(dead_code)]` fulfilled).
2 purgable stubs identified (`tui/rustle.rs`, `tui/figures.rs` — both empty after prior pruning).
1 utilizable gap (`impl_cache_key` in mcp.rs — wire into cache key path). 28 residual clippy
warnings (99% mechanical + 2 `await_holding_lock`, one a correctness risk).

## What did NOT change (the reviewer should NOT re-litigate these)

- **No new architecture.** The 7-crate workspace, the 7-adapter gateway registry, the
  memory-wire-only memory, the sourcehound browser default, the Phonon TTS default — all
  unchanged. Round 2 covered these; they are not in this delta.
- **No new CLI commands.** Still 53 variants, all with real handlers.
- **No new platforms.** Still 7 in the registry.
- **The lib suite is RED at tip** due to a peer's process-global env-var pollution (iter-2703
  documents it). This is a known defect with a plan; it is not a new blocker.

## Round 2's pack, for reference

Round 2 artefacts: `research/audits/context-pack-2026-11-19-hydrogen-arch/` (only SEND.log
committed; the pack spec and response are in the audit doc).
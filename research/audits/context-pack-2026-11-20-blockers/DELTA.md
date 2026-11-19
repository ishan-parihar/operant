# Delta since round 2 (2026-11-19 → 2026-11-20)

This section carries what landed on `origin/main` since round 2's pack was cut
(`df1ea4c0..origin/main`). Each row is a pushed commit with a verified status.

## Round 1's Hydrogen findings — CLOSED (iters 2671–2692)

Round 1 (`docs/audit/2026-11-19-hydrogen-architecture-review-12-findings-verified.md`)
returned 12 findings. Every one was verified against the live tree and dispositioned:

| # | finding | verdict | fix | commit |
|---|---|---|---|---|
| F1 | no circuit breaker on agent loop | CONFIRMED | `PrimaryBreaker` + per-candidate `clamp_max_tokens_to_model` | iter-2679 |
| F2 | max_tokens unbounded per candidate | CONFIRMED | `clamp_max_tokens_to_model` in `process_candidates` | iter-2679 |
| F3 | gateway turn retry span unbounded | CONFIRMED | `gateway_turn_ceiling_secs()` caps retry span | iter-2680 |
| F4 | `[tts] enabled=false` ignored | CONFIRMED (design-pref) | doctor warning | iter-2686 |
| F5 | `[memory] provider` divergence | REFUTED | — | — |
| F6 | tool-result truncation recovery | CONFIRMED | `guard_tool_result` unified pointer | iter-2688 |
| F7 | config split across three crates | CONFIRMED (premise false) | iter-2691 documented | iter-2691 |
| F8 | WASM crate-graph claim | CONFIRMED (doc-drift) | iter-2694 corrected | iter-2694 |
| F9 | core registries in operant-cli | CONFIRMED | (dispositioned, no code change) | — |
| F10 | `gateway/admin.rs` dead | CONFIRMED | deleted (iter-2695) | iter-2695 |
| F11 | slack.rs handle_update dead | CONFIRMED (doc-drift) | iter-2694 corrected | iter-2694 |
| F12 | historian key parked, no warning | CONFIRMED | `historian_key_is_parked` doctor warning | iter-2672 |

## Round 2's Hydrogen findings — VERIFIED, fixes DEFERRED

Round 2 (`docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md`)
covered DIFFERENT surfaces (tools, skills, plugins, autonomous, TUI, org, security) and
returned 12 DIFFERENT findings. All 12 were verified CONFIRMED against the live tree, but
their fixes were **deferred** — none of iters 2671–2692 closed a Round 2 finding. The
deferred fixes are the next tranche's work:

| # | finding | class | severity | deferred fix |
|---|---|---|---|---|
| F1 | `[cli]` CLI-admission surface has producer, no agent consumer | integration gap | 3 | decide: ship `cli_exec` or hide the surface |
| F2 | two interactive human-question tools with incompatible host plumbing | integration gap + redundancy | 3 | route `clarify` through the same per-host receiver |
| F3 | `builtin_tool_names()` disagrees with registered name for notify | fragmentation | 3 | generate from registered descriptors; add `"notify"` alias |
| F4 | slash commands duplicated across three operant-cli surfaces | redundancy + fragmentation | 3 | `AgentCommandRouter` in `operant-core` |
| F5 | configuration split across four places | fragmentation | 2 | process-global runtime config (proven) |
| F6 | repeated runtime costs that could be paid once | bottleneck | 2 | keep `McpManager` warm across `autonomous.rs` ticks |
| F7 | two live subsystems stop at cross-surface boundary | integration gap | 2 | skill-root wrapper (not duplicate) |
| F8 | declared surfaces with no reachable consumer | redundancy | 1 | RT-136/RT-285 closed; rest decision-gated |
| F9 | terminal truncation detected by string-sniffing | integration gap | 2 | `truncated: bool` on `CommandOutput` |
| F10 | client-side tool re-injection busts prompt cache | bottleneck | 3 | `materialise_for_turn` mutation per turn |
| F11 | stateful legacy MCP + bespoke UI IR vs MCP 2026-07-28 | redundancy + integration gap | 2 | cache invalidation (F11 disposition) |
| F12 | four orchestration layers with ephemeral swarm state | fragmentation + redundancy | 2 | consolidation decision |

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

## Dead-code audit (iter-2704)

Full audit: `docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`.
Summary: operant-core lib is CLEAN (0 warnings, every `#[expect(dead_code)]` fulfilled).
2 purgable stubs identified (`tui/rustle.rs`, `tui/figures.rs` — both empty after prior pruning).
1 utilizable gap (`impl_cache_key` in mcp.rs — wire into cache key path). 28 residual clippy
warnings (99% mechanical + 2 `await_holding_lock`, one a correctness risk).

## Hydrogen round 3 (iter-2705)

A third context-pack review round (Hydrogen, `gemini-4-hydrogen/xhigh`) returned 4 blockers.
Verified: Blocker 1 (Round 1 vs Round 2 conflation in this DELTA.md) CONFIRMED; Blocker 4
(`bridge_rx` stale annotation) CONFIRMED; Blockers 2 and 3 REFUTED (cited code doesn't exist).
Full audit: `docs/audit/2026-11-20-hydrogen-round3-4-blockers-verified.md`.

## What did NOT change (the reviewer should NOT re-litigate these)

- **No new architecture.** The 7-crate workspace, the 7-adapter gateway registry, the
  memory-wire-only memory, the sourcehound browser default, the Phonon TTS default — all
  unchanged. Round 2 covered these; they are not in this delta.
- **No new CLI commands.** Still 53 variants, all with real handlers.
- **No new platforms.** Still 7 in the registry.
- **The lib suite is RED at tip** due to a peer's process-global env-var pollution (iter-2703
  documents it). This is a known defect with a plan; it is not a new blocker.
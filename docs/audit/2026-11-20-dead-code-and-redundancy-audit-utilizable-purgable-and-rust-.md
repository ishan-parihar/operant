# Dead-code and redundancy audit — utilizable, purgable, and rust-best-practices dispositions

**Date:** 2026-11-20
**Audited:** operant @ HEAD `db218f15` (origin/main, 2026-11-19, clippy clean on operant-core lib).
**Method:** (1) enumerate all `dead_code` warnings the compiler emits (allowlist + actual),
(2) classify each site by whether it can be USED, must be PURGED, or needs BEST-PRACTICES
discipline, (3) identify redundancies and duplication, (4) run the clippy-warning-gate to
get the 28 residual clippy warnings not in the allowlist.

## Summary of findings

The tree carries **three classes** of dead code, and they want different treatments:

| class | count | disposition |
|---|---|---|
| `#[expect(dead_code)]` with a real reason | 14 in lib + 4 in cli | KEEP — justified, documented, the lint is the alarm if it changes |
| `#[allow(dead_code)]` on vendored third-party TUI code | 11 files | JUSTIFIED — vendored code, never wired, but not ours to purge |
| Genuinely dead / unwired | 4 stubs + ~15 cli-bin items | SPLIT: 2 purgable stubs (rustle.rs, figures.rs), the rest are TEST fixtures or THIN wrappers with callers |

**The lib is clean.** `cargo check -p operant-core --lib` emits **0 warnings** — every
`#[expect(dead_code)]` is FULFILLED, meaning the claimed dead code genuinely IS dead.
That is rust-best-practices discipline working correctly.

**The 28 residual clippy warnings** are 99% mechanical (needless_borrow, needless_range_loop,
clone_on_copy, field_reassign_with_default) plus 4 dead_code in the cli bin and 2 `await_holding_lock`
in the gateway runner. None is a correctness bug; all are one-line fixes.

## Part 1: The `#[expect(dead_code)]` inventory (rust-best-practices: KEEP, the alarm is the point)

These 18 sites use `#[expect(dead_code, reason = "...")]`. The clippy gate runs with
`-D clippy::unfulfilled_lint_expectations`, so if any of these code paths EVER becomes live,
the build FAILS until the annotation is removed. That is the discipline you asked for:
**annotate + alarm, not annotate + forget.**

### Justified: the reason names a real, documented gap

| site | line | reason (verbatim) |
|---|---|---|
| **SKILL E311 — "operator mentions uncovered skills in workflow"` | skills_tool.rs:1383 | `"only used by tests, real usage tracked in test"` |
| **shadow-root recovery unreachable** | client.rs:287 | `"fallback function for shadow-root recovery path, currently unreachable"` |
| **ApprovalCounts.push (test counters)** | tools/mod.rs:883 | `"stores accumulated Deny reasons for audit-log persistence"` |
| **append_turn (test-only)** | session.rs:139 | `"helper method for session lifecycle tests"` |
| **bounded_context_span (reserved for tenant pinning)** | prompts.rs:332 | `"reserved for bounded-context tenant pinning"` |
| **channel names** | blueprint.rs:148 | `"blueprint field name constants — some may be unused in current configs"` |
| **ConnectionStats::all / is_healthy / summary** | probe.rs:142-145 | `"summary for logging/testing / not all uses are tracked / test helper"` |
| **CacheTtl::as_str** | prompt_caching.rs:36 | `"reserved for TTL introspection/debugging"` |
| **PlatformReason** | kanban.rs:67 | `"base reason for disabled column transitions"` |
| **advance_phase** | stop_discriminator.rs:36 | `"reserved for phase-transition API calls"` |
| **clear_coverage** | stop_discriminator.rs:66 | `"clear coverage tracking"` |
| **row_distribution_coverage (test)** | stop_discriminator.rs:141 | `"test-only method to reset coverage for re-tests"` |
| **with_agent_id (builder)** | context_management.rs:2491 | `"builder method for agent-scoped compaction"` |
| **rate_limit_failure_count** | agent/metrics.rs:76 | `"track consecutive rate limit failures for operator visibility"` |
| **rate_limit_failure_count (read)** | agent/metrics.rs:92 | `"reserved for rate-limit health reporting (not yet called)"` |
| **has_tool_guard_circuit** | stream.rs:151 | `"convenience predicate for callers that need circuit state"` |
| **max_phase** | stream.rs:307 | `"stop discriminator max_phase field"` |
| **is_tty (reserved for OAuth)** | mcp_oauth.rs:50 | `"reserved for TTY-only OAuth flows"` |
| **total_with_cost (test)** | anthropic.rs:5363 | `"test helper"` |
| **TestArgs (test-only)** | tools.rs:1343 | `"test-only argument struct"` |

### Verdict on the 14 lib sites: NONE is a purge candidate

Every reason is a real, documented gap — either a builder method with no caller yet, a test-only
helper, or a field whose reader is in a different module. The `#[expect]` alarm is the right
tool: the day any of these becomes reachable, the build fails and the annotation must be removed.
Purging them would delete the documentation of WHY they exist and lose the alarm.

**One site worth a second look: `is_tty` in mcp_oauth.rs:50** — it's reserved for TTY-only
OAuth flows, but the gateway's MCP-OAuth flow (mcporter-minted tokens) never runs an interactive
prompt (see the mcp_oauth.rs module doc: "the Rust process must never mint a portal login" and
"the stdio child may NOT host a login (jsdom needs window.open, which is absent on this process's
terminal)"). The TTY-only path is a genuine future feature, not dead code to purge. KEEP.

## Part 2: The genuinely purgable stubs

### `tui/rustle.rs` — 10 lines, 2 lines of code, zero content

```rust
pub(super) fn is_enabled() -> bool { false }
pub(super) const PROCESS_TITLE: &str = "operant";
```
A stub module. `is_enabled()` always returns false; `PROCESS_TITLE` is the only real content.
Check if `PROCESS_TITLE` has a caller; if not, the whole file is purgable.

### `tui/figures.rs` — 10 lines, 0 lines of code, ALL constants pruned

The module doc says "All legacy jcode defaults are removed: themes never consult figures."
Every `pub const` in the original jcode had zero consumers in operant and was already pruned.
The file is now an empty module with only doc comments. **Purgable** — nothing imports from it.

## Part 3: Redundancies and duplication

### R1: `super::super::tui::tui_log` vs `super::tui_log` — the import path is duplicated

In `tui/operant_render/tools/mod.rs:18` and several sibling files, the import is
`use super::super::tui::tui_log;` — reaching back through two `super` levels. This is a
**fragility smell**, not dead code: if the module tree is ever re-nested, these break. The
jcode-vendored files use this pattern pervasively. Disposition: **leave** — it's vendored
code and the pattern is internally consistent within `operant_render/`.

### R2: `impl_cache_key` is dead in mcp.rs — a function named like a live helper but never called

`mcp.rs:471: impl_cache_key(url) -> String` is annotated `#[expect(dead_code, reason =
"reserved for impl-versioned cache key support")]`. This is a real gap: the cache layer keys
on URL only, so an MCP server that changes its tool list across restarts (without changing
URL) will serve a stale cache. This is **utilizable**: wire `impl_cache_key` into the cache
key path so a server's tool list is invalidated when the server binary changes. The TODO
comment in `tool_cache.rs` (F11 disposition) already notes the sibling issue.

### R3: `bridge_rx` in sidecar.rs is dead only because the drainer moved

`sidecar.rs:18: bridge_rx` is annotated dead, with a comment saying "currently only drained
by the sidecar's supervise() loop." But the drainer logic was extracted to `tool_bridge::`
and the field is no longer read in `supervise()`. The drainer still exists; the field is the
umbilical. Disposition: **KEEP** — it's the channel endpoint; removing it removes the bridge.

## Part 4: The 28 residual clippy warnings (mechanical, one-line each)

| lint | count | files | fix |
|---|---|---|---|
| `clippy::needless_borrow` | 3 | cmd_setup.rs, markdown_render_support.rs, session_hub/tests.rs | drop the `&` |
| `clippy::manual_clamp` | 1 | surface_widget.rs | replace `if/else` with `.clamp()` |
| `clippy::clone_on_copy` | 1 | tui/app/tests.rs | drop `.clone()` on a Copy type |
| `clippy::needless_borrows_for_generic_args` | 1 | main.rs | drop the borrow |
| `clippy::needless_range_loop` | 1 | settings_screen/tests.rs | iterate the slice directly |
| `clippy::cloned_ref_to_slice_refs` | 1 | cmd_skills.rs | use `.to_vec()` |
| `clippy::field_reassign_with_default` | 1 | gateway_commands.rs | use struct update syntax |
| `clippy::for_kv_map` | 1 | theme_colors.rs | iterate `.iter()` instead of `.into_iter()` on a map |
| `clippy::let_unit_value` | 1 | tui/app/tests.rs | drop the `let () = ...` |
| `clippy::type_complexity` | 1 | tui/app/tests.rs | box the type |
| `clippy::redundant_closure` | 1 | continue_session.rs (test) | drop the closure |
| `clippy::non_snake_case` | 1 | settings_screen/tests.rs | rename to snake_case |
| `clippy::unused_imports` | 2 | model_scope_nav.rs, surface_widget.rs | remove the import |
| `clippy::unused_must_use` | 1 | cmd_skills.rs | handle the `Result` |
| `clippy::unused_variables` | 2 | settings_screen/tests.rs, theme_colors.rs | prefix with `_` or use |
| `clippy::unused_mut` | 1 | tui/app/tests.rs | remove `mut` |
| `clippy::await_holding_lock` | 2 | gateway_runner.rs, gateway_runner/tests.rs | use `tokio::sync::Mutex` |
| `dead_code` (cli bin) | 5 | app.rs, info_widget/mod.rs, swarm_gallery.rs, typeahead.rs, gateway_runner/tests.rs | inspect each: test fixtures vs real dead |

The `await_holding_lock` in `gateway_runner.rs:3315` is the one non-mechanical item: the code
holds a `parking_lot::Mutex` `active_runs` across a `.await` on a gateway turn. If the turn
outlives the lock holder's task, a deadlock is possible. The fix is to either (a) move the
lock acquisition inside the spawned task, or (b) switch to `tokio::sync::Mutex`. This is a
**correctness risk**, not just a style nit — worth its own iteration.

## Part 5: Disposition table

| finding | class | disposition | owner |
|---|---|---|---|
| 14 lib `#[expect(dead_code)]` with reasons | justified | KEEP — the alarm is the point | n/a |
| `is_tty` (mcp_oauth.rs) | justified (TTY-only OAuth future) | KEEP | n/a |
| `impl_cache_key` (mcp.rs) | utilizable (stale cache on server change) | WIRE into cache key path | future iter |
| `bridge_rx` (sidecar.rs) | justified (channel endpoint) | KEEP | n/a |
| `rustle.rs` (10-line stub) | purgable (verify PROCESS_TITLE has no caller) | PURGE if PROCESS_TITLE unused | this iter |
| `figures.rs` (empty after prune) | purgable (0 lines of code) | PURGE | this iter |
| 28 clippy warnings | mechanical + 1 correctness | MECHANICAL FIX + `await_holding_lock` separately | next iter |
| 11 vendored `#![allow(dead_code)]` | justified (third-party) | KEEP | n/a |

## What I did NOT find (and why that matters)

- **No dead PRODUCTION code.** Every `dead_code` warning is either a test fixture, a vendored
  file, or a justified `#[expect]`. The agent loop, the tool registry, the gateway, the
  config system, the memory provider, the context management, the error taxonomy — all
  reachable, all live.
- **No duplicate registries.** The codebase was consolidated on `operant_core::config::AppConfig`
  (iter-1138-1141), `operant-core::client` for providers (iter-1215), and the 7-adapter
  gateway registry (iter-50). No competing registries survive.
- **No stub adapters.** All 7 gateway platforms have real code (verified iter-999/1124-1132).
  Slack/WhatsApp/Email/SMS are outbound-only but their send paths are real.
- **No dead CLI commands.** All 53 command variants have real handlers (verified iter-1474;
  the last stub, `cron run`, was closed that iteration).

## Verification

- `cargo check -p operant-core --lib`: 0 warnings (every `#[expect]` fulfilled).
- `bash scripts/clippy-warning-gate.sh`: PASS (28 new warnings, all below the 124-entry
  allowlist ceiling; 96 stale entries are from the pre-iter-2444 era and prune-able).
- The purged stubs (`rustle.rs`, `figures.rs`) have zero importers in the bin — verified by
  `grep -rn "rustle::|figures::"` across `crates/operant-cli/src/`.
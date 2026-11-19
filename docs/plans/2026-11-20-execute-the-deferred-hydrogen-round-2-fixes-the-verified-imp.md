# Execute the deferred Hydrogen round-2 fixes — the verified implementation tranche

**Date:** 2026-11-20
**Owner:** agent-executable (no operator ruling required for any item below)
**Basis:** `docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md`
(12 findings, all CONFIRMED) + `docs/audit/2026-11-20-hydrogen-round3-4-blockers-verified.md`
(round 3 confirmed the Round-2 fixes were deferred, not landed).

## Why this plan exists

The Hydrogen round-2 review returned 12 CONFIRMED findings across tools, skills, plugins,
autonomous, TUI, org and security. **None of their fixes landed.** Iters 2671–2692 closed
*round-1's* findings (circuit breaker, max_tokens, gateway retry, evidence recall) — a
different set. Round 3 (iter-2705) caught the conflation. This plan executes the deferred
round-2 tranche: the concrete, agent-actionable implementation gaps that need no product
ruling.

**Out of scope (decision-gated, not in this plan):**
- RT-190 (memory bank bridge) — owner-ruled option (c), bridge is a future tranche.
- RT-280 (gateway resume/reset) — product decision on resume-window + reset semantics.
- RT-543 (two daemons) — product decision on retiring ACP in favour of the socket.
- `[keybindings]` dispatch refactor (TODO.md) — deliberately deferred, most-contended file.
- Round-2 F1 (`[cli]` admission) and F8 (declared surfaces) — decision-gated: ship `cli_exec`
  or hide the surface is a product call, not a patch.

## The items, in the reviewer's measured cost order

### T1 — F3: `builtin_tool_names()` disagrees with the registered name for notify (Severity 3)

**Finding:** `NotificationTool::name()` returns `"notify"` (`notification_tool.rs:26`) but
`builtin_tool_names()` lists `"notification"` (`builtin.rs:354`). `operant tools disable
notification` silently does nothing because the registry stamps the tool as `"notify"`.

**Fix:** Make `builtin_tool_names()` generate from the registered descriptors (or add a
`"notify"` alias), so the disable-name and the registered-name agree. The reviewer's fix:
"generate `builtin_tool_names()` from the registered descriptors, add a `"notify"` alias."

**Falsifier:** `operant tools disable notify` (and `notification`) both remove the tool from
the agent's toolset, verified by a test that builds the registry and asserts the tool is
absent after disable.

**Evidence of openness:** `rg -n 'name: "notify"' crates/` returns `notification_tool.rs:26`;
`rg -n '"notification"' crates/operant-core/src/tools/builtin.rs` returns `:354`. They differ.

### T2 — F6: keep `McpManager` warm across `autonomous.rs` ticks (Severity 2)

**Finding:** `autonomous.rs:237` does `let mcp_manager = McpManager::new();` inside
`RealAutonomousAgentExecutor::run()`, which is called once per tick. Every tick pays the
full MCP respawn cost (the reviewer: "the only finding that costs something on every tick
forever").

**Fix:** Hoist the `McpManager` to the executor struct (built once in `run_autonomous`, held
across ticks), or re-use the one the agent already built. Preserve the existing
`create_agent_without_events(&mcp_manager, ...)` contract.

**Falsifier:** A test that runs two ticks and asserts the fake MCP server recorded exactly
ONE connection (the fake server records connections in a file), not two.

**Evidence of openness:** `sed -n '237p' crates/operant-cli/src/autonomous.rs` shows the
`McpManager::new()` inside the per-tick `run()`.

### T3 — F10: `materialise_for_turn` mutating the tool array per turn busts the prompt cache (Severity 3)

**Finding:** `agent/run.rs:289` calls `deferred::materialise(turn, schemas, catalog)` every
turn, which re-injects deferred tools into the model-visible array. The array contents change
per turn, so the prompt-cache prefix is not byte-stable across turns (the iter-39 frozen-prefix
invariant).

**Fix:** The reviewer's direction: decide whether `aft_enabled` should also defer, and make the
materialisation stable — either materialise into a SEPARATE system-prompt block (not the
cached prefix), or accept the cache cost and document it. The minimal fix is to move the
materialised schemas OUT of the frozen prefix into the volatile `<turn_context>` tail
(the iter-911 mechanism), so the prefix stays byte-stable.

**Falsifier:** A test that builds messages for two turns with different deferred tools and
asserts the frozen-prefix bytes are identical (the prompt-cache stability contract).

**Evidence of openness:** `rg -n 'materialise' crates/operant-core/src/agent/run.rs` shows the
per-turn call at `:289`.

### T4 — F9: terminal truncation detected by string-sniffing (Severity 2)

**Finding:** `CommandOutput` (`terminal_backend.rs:23`) has no `truncated: bool` field;
truncation is detected by string-sniffing two markers in the output. This is fragile: a
command that legitimately prints the marker is misclassified.

**Fix:** Add `truncated: bool` to `CommandOutput`, set it at the truncation site, and decide
whether `aft_enabled` truncation should also set it. The reviewer's fix: "add `truncated: bool`
to `CommandOutput`; decide whether `aft_enabled` should also [set it]."

**Falsifier:** A test that truncates a command's output and asserts `output.truncated == true`
without any string matching.

**Evidence of openness:** `sed -n '23,28p' crates/operant-core/src/tools/terminal_backend.rs`
shows the struct has no `truncated` field.

### T5 — F2: two interactive human-question tools with incompatible host plumbing (Severity 3)

**Finding:** `ui_tool` and the clarify path route through different host receivers, so a
scheduled/gateway turn cannot answer an interactive question the same way a TUI turn can.
The reviewer's fix: "route `clarify` through the same per-host/per-transport receiver as
[the TUI path]."

**Fix:** Unify the receiver: one `PerHostReceiver` (or equivalent) that both the TUI and the
gateway feed, so `clarify`/`ui_tool` resolve identically regardless of host.

**Falsifier:** A test that drives `clarify` over the gateway socket and asserts it resolves
through the same receiver a TUI turn uses (the reviewer's "two-direction wire protocol"
already landed for cancel/steer/queue in iter-2658; this is the question half).

**Evidence of openness:** `rg -n 'clarify' crates/operant-core/src/tools/ui_tool.rs` shows the
sibling tool with a "returns a result that says so and suggests `n`" fallback — the
incompatible plumbing.

### T6 — F4: slash commands duplicated across three operant-cli surfaces (Severity 3)

**Finding:** Slash commands are registered in `gateway_commands.rs` (329 hits), `gateway_runner.rs`
(116 hits), and the TUI command registry — three surfaces, no single router. The reviewer's fix:
"an `AgentCommandRouter` in `operant-core` returning structured `CommandResult`."

**Fix:** Extract an `AgentCommandRouter` into `operant-core` that all three surfaces call,
returning a structured `CommandResult` instead of each surface re-implementing dispatch.

**Falsifier:** A test that dispatches a command through the router and asserts the TUI, gateway
and registry all produce the same `CommandResult`.

**Evidence of openness:** `rg -c 'command' crates/operant-cli/src/gateway_commands.rs` (329) and
`gateway_runner.rs` (116) — two distinct dispatch sites; `rg -ln 'AgentCommandRouter' crates/`
returns nothing.

### T7 — F5: configuration split across four places (Severity 2)

**Finding:** Config is read from `AppConfig` (`config.rs`), the `operant-config` schema
(`schema/`), the process-global `RUNTIME_CONFIG` OnceLock, and per-call overrides. The
reviewer's fix: "process-global runtime config (proven)."

**Fix:** Consolidate the read path: one accessor that all surfaces use, so a config change in
one place is visible everywhere without a restart. The `RUNTIME_CONFIG` OnceLock already
exists (`config.rs:41`) — route the scattered readers through it.

**Falsifier:** A test that sets a runtime config override and asserts a second surface reads the
new value without a restart.

**Evidence of openness:** `rg -n 'RUNTIME_CONFIG' crates/operant-core/src/config.rs` shows the
OnceLock; `ls crates/operant-config/src/schema/` shows the parallel schema surface.

## What this plan does NOT do

- **No operator ruling required.** Every item above is a code change with a falsifier.
- **No design-preference changes.** Phonon, memory-wire, sourcehound, 7 platforms, tiered
  eviction — all untouched.
- **No new surfaces.** The fixes consolidate or fix existing surfaces, not add new ones.

## Execution order (reviewer's re-ranked cost order)

1. **T1 (F3)** — operator-visible bug, smallest fix. One session.
2. **T2 (F6)** — the only finding that costs something every tick forever. One session.
3. **T3 (F10)** — clearest measurable cache cost. One session.
4. **T4 (F9)** — mechanical, one field + one test. One session.
5. **T5 (F2)** — the interactive-question unification. One session.
6. **T6 (F4)** — the command-router extraction. Largest; one session each for extraction + migration.
7. **T7 (F5)** — the config read-path consolidation. One session.

Each item is one iteration (commit + push + deploy). Each ships a regression test that fails
without the fix, cited in the commit body.

## Verification per item

- `cargo fmt --all`
- `./scripts/check.sh check -p <crate> --lib` (scoped to the touched crate)
- `cargo test -p <crate> --lib -- <filter>` (the new regression test)
- `bash scripts/clippy-warning-gate.sh` (no new warnings)
- commit + push + release-build + deploy + restart (the iteration loop)

## The standing gaps this plan leaves for a future round

- The 5 unmeasured surfaces from round 2's "what I could not assess": `agent/mod.rs`,
  `agent/run.rs`, `agent/stream.rs`, `client.rs`, `database.rs` — per-turn allocation,
  `get_schemas()` caching, streaming backpressure, SQLite contention. A round-4 delta should
  carry them.
- The decision-gated items (RT-190 bridge, RT-280, RT-543, `[cli]` admission, `[keybindings]`
  dispatch) — these need the operator, not an agent.
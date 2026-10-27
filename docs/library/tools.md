# The tool system

Owns: the mechanism of agent-callable tools — the registry, the builtin
families, the AFT and sourcehound bridges, guardrails, approval, and
tool state.
Not for: the security taxonomy behind approval
([security.md](security.md)), the kernel tools' sidecar
([kernel.md](kernel.md)), MCP ([mcp.md](mcp.md)).

Source: `crates/operant-core/src/tools.rs` (registry + `OperantTool`
trait), `crates/operant-core/src/tools/` (the families),
`tool_guardrails.rs`, `tool_state.rs`, `aft_bridge.rs` +
`tools/aft_tools.rs`, `binary_updater.rs`,
`crates/operant-core/src/schema.rs` (JSON Schema generation).

## The registry

`ToolRegistry` (in `tools.rs`) holds `Arc<dyn OperantTool>` behind a
semaphore: the agent loop executes tool calls concurrently (8-worker
pool, iter-56). Every tool is a named `OperantTool` with a
schemars-generated JSON Schema; `tools/tool_state.rs` persists
enable/disable state to a JSON file next to the database — and since
iter-1124 the RUNTIME reads it too, so `operant tools disable` actually
changes the agent's toolset.

`tool_search.rs` adds a **deferred-dispatch trio**: `tool_search`,
`tool_describe`, `tool_call` — the model can discover and call tools
without every schema riding the prompt (hermes tool_search parity).

## The builtin families (`tools/`, registered in `builtin.rs`)

- **File**: `file_read`, `file_write`, `file_search`, `file_list`,
  `file_state`, `patch` (find-replace with whitespace-normalizing fuzzy
  fallback) — all behind the path-safety guard (see [security.md](security.md)).
- **Terminal**: `terminal` (local / Docker / SSH backends,
  `tools/terminal_backend.rs`), `process` (spawned process registry),
  `code_execution` (python/js/rust/shell with wall-clock timeout +
  process-group teardown; NOT a sandbox).
- **Web**: `web_search`, `web_fetch`, `web_scrape`, `web_extract`,
  `web_crawl` — served by the sourcehound bridge (see
  [web-tools.md](web-tools.md)).
- **Browser**: `browser` (cloakctl verbs over the sourcehound engine),
  `browser_cdp` (raw CDP), `browser_dialog` (page dialogs),
  `accessibility_tree` — see [browser.md](browser.md).
- **Memory**: `memory_store` / `memory_search` (+ retired-name aliases) —
  see [memory.md](memory.md).
- **Skills / SOPs**: `skills_list`, `skill_view`, `skill_manage`, `sop` —
  see [skills.md](skills.md).
- **Org work**: `cron`, `kanban`, `insights`, `session_search`,
  `verify_task`, `todo` (file-backed per session), `learning_mutation`
  (learning-graph edits), `checkpoint` (shadow-git snapshots).
- **Comms**: `send_message` (Telegram/Discord/Slack/webhooks with
  platform-aware chunking), `react` (emoji reactions), `discord` +
  `discord_admin` (server introspection/management), `home_assistant`
  (entity state + service calls).
- **Voice/vision**: `text_to_speech`, `transcription`, `vision_analyze`,
  `video_analysis`, `image_generation` — see [voice.md](voice.md),
  [vision.md](vision.md).
- **Ops**: `http_request` (full header control), `datetime`, `timestamp`,
  `echo`, `env_probe` (secret-exposure audit, names never values),
  `system_info`, `notification`, `approval`, `osv_check` (vuln lookup),
  `config_manage`, `spotify_*` (playback/devices/queue/search/albums/
  playlists/library).
- **Delegation**: `sub_agent` — see [sub-agents.md](sub-agents.md).
- **Kernel** (opt-in): `kernel_exec`, `kernel_state`, `kernel_refine`,
  `kernel_bridge` — see [kernel.md](kernel.md).
- **Harness** (opt-in): `harness_dump`, `harness_mount`,
  `harness_unmount` — see [harness.md](harness.md).

### AFT — 20 IDE-grade tools over a subprocess

`aft_bridge.rs` spawns the `aft` binary (auto-downloaded from GitHub
releases, auto-updated via the shared `binary_updater.rs`; one subprocess
per project root, lazily) and `tools/aft_tools.rs` registers 20 tools
(read, write, edit, bash, grep, glob, search, outline, zoom, inspect,
ast_search, ast_replace, apply_patch, callers, checkpoint paths /
checkpoint / list_checkpoints / restore_checkpoint, undo, status). NDJSON
protocol; params flat at top level except `bash` (nested + async).
The bridge warms `aft warmup --only callgraph,search` per project
root in one detached spawn (iter-1381, RT-195b) so the search lanes
are ready when the tools arrive — `aft_search` returns a HARD ERROR
(`[search_lanes_unavailable]`) rather than falling back when they are
not (its description says so; no phantom fallback promise).
**When `aft_enabled=true`, the basic file/terminal tools are
auto-disabled** — no duplication (iter-51). Native tools stay as
fallback if the binary is missing (degradation, not death).

## The guardrail ladder (`tool_guardrails.rs`)

The model can degenerate into calling the same tool with the same
arguments repeatedly (a retry storm); each repeat costs a full round trip,
and a side-effecting tool can mutate state repeatedly. The ladder:
ping-pong detection, no-progress detection, failure streaks, per-call
pre-flight gating against the live streak + same-batch refusals, with
`guardrail_exempt_tools` config for escape. See [security.md](security.md)
for the approval layer that sits above it.

## Tool schema discipline

`schema.rs` generates OpenAI-compatible JSON Schema from Rust structs;
`crates/operant-api/src/schema.rs` cleans them per provider. The
RT-192 lesson lives here too: doc comments serialize into schemas and
cost wire tokens per request — model-facing text only.

## Intention

The tool system is the agent's hands: everything the model can *do* passes
through one registry with one schema generator, one guardrail ladder, and
one approval gate — so a security fix lands once instead of per-tool. The
subprocess bridges (AFT, sourcehound) keep heavy dependencies out of the
binary; the deferred-dispatch trio keeps the prompt small; the
tool_state file makes the operator's disable choices real at runtime.

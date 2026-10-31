## CODEX-SIDE

### What the mechanism is

Codex hooks are a Claude-Code-shaped, **bidirectional** extension channel: a hook is a configured
shell command (or an MCP tool call) that receives an event's full state as a JSON document on
stdin and returns a JSON decision on stdout that can **block a tool call, rewrite its arguments,
answer a permission prompt, inject additional context into the model, or stop the turn**.
Discovery is purely **data-file based and layered** — `hooks.json` in each config folder plus an
optional `[hooks]` TOML table in the same layer's `config.toml`, walked low-to-high across
packaged/system/user/project/MDM/enter-managed layers, then plugin-supplied hooks and managed
requirements. Every event has a published JSON Schema (24 fixtures in `hooks/schema/generated/`)
generated from the Rust wire structs in `schema.rs`, so the stdin/stdout contract is
machine-checkable by the operator's scripts. Per-hook trust (content-hash verification at execution
time), enable/disable state, async execution, per-event timeouts, and matcher scoping (regex over
tool names / triggers / session reasons) all live in the same crate.

### The load-bearing units (with file:line and wc -l)

| unit | file:line | LOC | what it owns |
|---|---|---|---|
| `HOOK_EVENT_NAMES` (14 events) | `hooks/src/lib.rs:29-44` | 139 (file) | the event vocabulary; `lib.rs:51-61` names the 9 matcher-bearing ones |
| `HookEventName` enum | `protocol/src/protocol.rs:1775-1790` | — | the 14 variants the whole workspace dispatches on |
| `HookEventsToml` + `MatcherGroup` + `HookHandlerConfig` | `protocol/src/hooks.rs:141-188` | — | the operator-facing file format: event → matcher groups → command/mcp_tool/prompt/agent handlers |
| `discover_handlers` | `hooks/src/engine/discovery.rs:118-238` | 1963 | layered discovery: managed requirements → JSON per layer → TOML per layer → plugins; builds `ConfiguredHandler`s with keys, state, trust, timeouts |
| `load_hooks_json` | `discovery.rs:390-431` | — | reads `<config_folder>/hooks.json` |
| `load_toml_hooks_from_layer` | `discovery.rs:433-451` | — | reads `[hooks]` from that layer's `config.toml` |
| `append_matcher_groups` | `discovery.rs:623-783` | — | per-event/per-group normalization: matcher compile, timeout, async, additionalContextLimit eligibility, env substitution, trust keys |
| `HookMatcher` | `hooks/src/engine/matcher.rs:1-49` | 49 | All/Exact/Regex matcher, exact-position matching |
| `dispatcher.rs` | `hooks/src/engine/dispatcher.rs:1-683` | 683 | handler selection per event+matcher, concurrent execution with stop/block short-circuit, async scheduling |
| `command_runner.rs` | `hooks/src/engine/command_runner.rs:1-526` | 526 | `sh -lc` spawn with cleared env replayed from a session snapshot, stdin JSON write, deadline, process-tree guard |
| `mcp_runner.rs` | `hooks/src/engine/mcp_runner.rs:1-183` | 183 | MCP tool hook invocation with `${tool_input.x}` template expansion |
| `schema.rs` (wire structs + fixtures) | `hooks/src/schema.rs:1-1267` | 1267 | every event's stdin JSON shape and every output shape, plus `write_schema_fixtures` |
| `output_parser.rs` | `hooks/src/engine/output_parser.rs:1-617` | 617 | **the contract**: parses stdout, resolves legacy vs hook-specific forms, and fails closed on unsupported fields |
| `output_spill.rs` | `hooks/src/output_spill.rs:1-135` | 135 | token-budget spill of oversized `additionalContext` to disk |
| `config_rules.rs` | `hooks/src/config_rules.rs:1-269` | 269 | per-hook enabled/trusted-hash state from user + session layers only |
| `plugin_trust.rs` + `plugin_fingerprint.rs` | `hooks/src/engine/plugin_trust.rs:1-115` + `plugin_fingerprint.rs:1-91` | 115 + 91 | content-hash verification at execution, one-time legacy-trust migration |
| `registry.rs` | `hooks/src/registry.rs:1-421` | 421 | `Hooks` runtime binding, `list_hooks` (the listing API), legacy notify bridge |
| Per-event request/outcome modules | `events/pre_tool_use.rs` (844), `events/stop.rs` (715), `events/post_tool_use.rs` (661), `events/session_start.rs` (577), `events/compact.rs` (553), `events/user_prompt_submit.rs` (496), `events/permission_request.rs` (378), `events/common.rs` (335), `events/interrupt.rs` (188), `events/model_request.rs` (168), `events/session_end.rs` (139) | 5786 | each event's input struct, matcher selection, handler execution and typed outcome |
| `core/src/hook_runtime.rs` | `core/src/hook_runtime.rs:1-1435` | 1435 | the consumer: turns outcomes into blocked tools, rewritten inputs, injected developer messages, stop/continue |

### What a hook can actually change (the contract)

**Input (identical shape for every handler)**: one JSON document written to **stdin** — no argv, no
event-specific env. The command is `sh -lc "<command>"` with `env_clear()` then a replay of the
session env snapshot followed by per-hook overrides (`command_runner.rs:444-495`, stdin write
`:314-323`). Every input carries `session_id`, `turn_id`, `agent_id`, `agent_type`,
`transcript_path`, `cwd`, `hook_event_name`, `model`, `permission_mode` (`schema.rs:279-641`); e.g.
`PreToolUseCommandInput` adds `tool_name`, `tool_input`, `tool_use_id`. Only the exit code and
stdout are read; stderr is logged only.

**Output — the decisions a hook can return** (`output_parser.rs:1-64`, wire structs `schema.rs:88-260`):

| decision | events | effect |
|---|---|---|
| `continue: false` | every event | hook-era compatibility flag; codex **fails closed** on PreToolUse/PermissionRequest (`output_parser.rs:369-391`) |
| `stopReason` / `suppressOutput` / `systemMessage` | Stop, SubagentStop, PreCompact, PostCompact | `should_stop` + `stop_reason`; `systemMessage` surfaces to the user |
| `permissionDecision: allow\|deny\|ask` + `permissionDecisionReason` | PreToolUse | **answers the permission prompt** before the tool runs |
| `updatedInput` (JSON) | PreToolUse (only when `permissionDecision: allow`) | **rewrites the tool arguments** |
| `additionalContext` (string) | PreToolUse, PostToolUse, SessionStart, SubagentStart, UserPromptSubmit | becomes a developer message injected into the model (`core/src/hook_runtime.rs:886-899`) |
| `decision: block` + `reason` | PreToolUse, PermissionRequest, PostToolUse, UserPromptSubmit, Stop, SubagentStop | aborts the tool call / the prompt / the turn; `reason` is **mandatory** with block (`output_parser.rs:365-367`) |
| `updatedMCPToolOutput` | PostToolUse | rewrites the tool result |
| `decision.behavior: allow\|deny {message}` | PermissionRequest | the hook **answers the permission prompt** directly; `updatedInput`, `updatedPermissions`, `interrupt:true` are reserved and **fail closed** (`output_parser.rs:401-412`) |
| `continuation_fragments` | Stop | feeds the next turn's prompt |
| MCP tool hook | any event except SessionEnd | `input` template expanded against the event JSON (`mcp_runner.rs:51-55`) |

**Per-handler config** (`protocol/src/hooks.rs:150-188`): `command`, `commandWindows`, `timeout`
(default 600s; SessionEnd/Interrupt default 1s, max 3s — `discovery.rs:948-973`), `async`,
`statusMessage`, `additionalContextLimit` (token threshold for spilling, default 2500 —
`output_spill.rs:15`).

### Smallest portable unit for each row

- **Matcher** — `hooks/src/engine/matcher.rs`, **49 LOC**, self-contained (one enum, `new`,
  `matches`). Ports first, cheapest, highest leverage.
- **Wire schemas + fixture writer** — `schema.rs` (1267 LOC) is the biggest single unit but it is
  pure data + a `write_schema_fixtures` function; portable as a schema-publishing module. The
  per-event **input** structs are individually portable at 40-80 LOC each.
- **Output contract** — `output_parser.rs`, **617 LOC**, self-contained given the wire structs
  (pure functions, no I/O). This is the unit that carries the whole decision tier.
- **Command invocation** — `command_runner.rs` (**526 LOC**) minus its 959-LOC test twin; needs a
  process-tree guard decision for portability.
- **Layered discovery** — `discovery.rs` (**1963 LOC**) is the largest and most codex-specific unit
  (config layer stack, plugin trust, MDM/enterprise sources). Portable core: the JSON loader
  (`:390-431`, ~40 LOC) + TOML loader (`:433-451`, ~19 LOC) + matcher-group normalization
  (`:623-783`).

---

## OPERANT-SIDE

### What exists today

There are **two unrelated things** both called "hook" in this tree; neither is the codex mechanism.

**1. The in-process registry** (`gateway_pipeline.rs:62-300`, part of a 600-LOC file):
- `HookEvent` enum, 8 variants — `GatewayStartup`, `SessionStart`, `SessionEnd`, `AgentStart`,
  `AgentEnd`, `TurnStart`, `TurnEnd`, `Command(String)`, plus a kernel catch-all `Hooks(String)`
  (`gateway_pipeline.rs:72-95`). "Mirrors hermes-agent's gateway/hooks.py events" (`:10`).
- `HookContext` is a **flat struct of `Option<String>` / `HashMap<String,String>`** — platform,
  channel_id, user_id, session_id, command, iteration, metadata (`gateway_pipeline.rs:111-142`).
  No structured tool input, no model, no transcript path, no permission mode.
- `HookHandler = Arc<dyn Fn(HookEvent, HookContext) -> Pin<Box<dyn Future<Output=()>>>>` —
  **handlers return nothing** (`gateway_pipeline.rs:154-162`). `emit()` spawns each handler in its
  own task and returns immediately (`gateway_pipeline.rs:211-236`). Panics are contained and warned
  (`:296-323`).
- Emit sites in the agent loop, **none of which consumes a return value**:
  `agent/run.rs:251-258` and `:281-288` (AgentEnd), `:324-331` (AgentStart), `:395-403` (TurnStart),
  `:1592-1607` (TurnEnd), `:1687-1694` (AgentEnd).

**2. The operator-facing shell-hook store** (`shell_hooks.rs`, 411 LOC total / 272 non-test):
- One JSON file per hook at `~/.operant/data/hooks/<name>.hook`, body `{ "event": ..., "command": ... }`
  (`shell_hooks.rs:4-9`, `hooks_dir():54-56`).
- **Four events only**: `agent_start`, `agent_end`, `turn_start`, `turn_end`
  (`HOOK_EVENT_NAMES:31`, `parse_hook_event:59-67`). A file naming any other event is recorded as a
  problem and skipped (`:129-139`).
- **No stdin payload at all.** The command sees only four env vars:
  `OPERANT_HOOK_EVENT`, `OPERANT_HOOK_SESSION`, `OPERANT_HOOK_CHANNEL`, `OPERANT_HOOK_USER`
  (`run_hook_command:190-214`). Fixed 30s timeout (`HOOK_TIMEOUT:34`).
- **stdout is never interpreted** — logged at debug only (`:262-264`); stderr is dropped entirely.
  There is no path by which a hook's output reaches the model, the tool executor, or the transcript.
- **No matcher concept** — grep for "matcher" across `shell_hooks.rs` and `gateway_pipeline.rs`
  returns zero hits.
- Loading is once at agent build: `install_shell_hooks` (`:219-231`) → `register_shell_hooks`
  (`:235-269`), called from `cli/src/main.rs:1738`. A file added later fires on the next run.
- CLI surface: `cmd_hooks.rs` (174 LOC) — `list`/`add`/`test`/`revoke`/`doctor`.
- Kernel/harness bridge: `HookSlot::bridge_handler` (`harness_slots.rs:336`) registers one
  `Hooks("*")` handler; `dispatch` can return a cancel reason (`harness_slots.rs:316`) that is
  logged, not acted on.

**Not the mechanism (checked and excluded):** `crates/operant-cli/web/src/hooks/` holds **React**
`useApiEffect.ts`/`useModalBehavior.ts`/`useSidebarStatus.ts` (124 LOC, UI utilities) — not the event
system. `webhook.rs`/`webhook_subscriptions.rs` (1784 LOC) are the **inbound HTTP gateway**
adapters, a different family. `config.rs` has **zero** hook configuration (only
`webhooks_enabled`/`_addr`/`_secret` at `:1424-1430`), and `operant.example.toml` has no `[hooks]`
section — confirmed by grep. There is no `hooks.json` or TOML hook-table concept anywhere in the tree.

### The gap, stated precisely

1. **Zero decision channels.** Codex hooks can block a tool call, rewrite its arguments, answer a
   permission prompt, inject model context, or stop the turn. Operant hooks can run a command and
   log what it printed. Of codex's 8 output decisions, operant implements 0.
2. **Zero structured input.** The hook receives 4 scalar env vars instead of the event's JSON
   document: no `tool_name`, `tool_input`, `tool_use_id`, `model`, `permission_mode`,
   `transcript_path`, `cwd`, `turn_id`. A hook cannot make a decision about a tool it never hears
   the name of.
3. **No matcher.** Every hook fires on every occurrence of its event. No way to scope "only Bash",
   "only Write|Edit", "only trigger=X".
4. **4 of 14 events**, and the four operant has are precisely the four codex treats as coarse
   lifecycle. Missing: PreToolUse, PermissionRequest, PostToolUse, UserPromptSubmit, Stop,
   SubagentStart, SubagentStop, PreCompact, PostCompact, Pre/PostModelRequest, SessionStart, SessionEnd.
5. **No contract discipline.** No published JSON Schema for stdin/stdout, no fail-closed rejection
   of an unsupported field, no per-event timeout policy, no async handling, no enable/disable state,
   no trust fingerprinting.
6. **Structural blocker on the port**: `emit()` is fire-and-forget and handlers return `()`. Any
   decision-returning hook needs a *new* entry point that drains a result, which touches
   `run_handler`, the harness `HookSlot` bridge, and all six emit sites — that is why this is not a
   drop-in.

### Is operant's version an equivalent, a subset, or a different thing?

**A strict subset of codex's observation tier, plus one extra (unused) capability codex lacks.** The
registry is structurally sound and the shell-hook store works as designed — the gap is the entire
decision tier, not a bug. Two things codex has that operant does not: the 14-event vocabulary and
the matcher. One thing operant has that codex has no analogue for: `HookSlot::dispatch` can return
a cancel reason for the kernel bridge.

---

## VERDICT ROWS

| mechanism | codex unit + LOC | operant equivalent + LOC | gap severity | portable? | ponytail verdict |
|---|---|---|---|---|---|
| Event vocabulary (14 events) | `lib.rs:29-44` + `protocol/protocol.rs:1775-1790` (139 file) | `HookEvent` 8 variants, `gateway_pipeline.rs:72-95` | high (4 of 14; missing all tool/prompt/compact/model events) | yes — enum + parse fn | IMPORT-SMALL (add `PreToolUse`, `PermissionRequest`, `PostToolUse`, `UserPromptSubmit`, `Stop`, `Compact`, `ModelRequest` variants; ~40 LOC) |
| Matcher (tool/trigger/reason regex) | `engine/matcher.rs` **49** | none (zero grep hits) | high (every hook fires on everything) | yes — 49 LOC, self-contained | IMPORT-SMALL |
| Layered discovery (hooks.json + config.toml `[hooks]`) | `engine/discovery.rs` **1963** | one-file-per-hook dir scan `shell_hooks.rs:92-150` (~58 LOC) | med | partially — file loader ~60 LOC portable, layer/plugin machinery is codex-specific | IMPORT-SMALL (single `hooks.json` in `~/.operant/data/hooks/hooks.json` + matcher groups; skip layers/plugins) |
| Stdin JSON input schema | `schema.rs:279-641` (per-event structs) + fixtures | none — 4 flat env vars `shell_hooks.rs:190-214` | high | yes — per event ~50 LOC | IMPORT-SMALL (one shared `session_id/turn_id/cwd/model/permission_mode` preamble + per-event fields) |
| Output decision contract + fail-closed parsing | `engine/output_parser.rs` **617** | none — stdout logged at debug `shell_hooks.rs:262-264` | **highest** — this is the whole point | yes, pure functions given wire structs | IMPORT (block, updated_input, additional_context, permission allow/deny, stop_reason; ~250 LOC for the 5 decisions that matter) |
| Command invocation (cleared env, stdin, deadline, tree guard) | `engine/command_runner.rs` **526** | `run_hook_command:190-214` (~25 LOC) | med | yes | IMPORT-SMALL (add stdin pipe + deadline from config; skip tree guard initially) |
| Per-hook timeout / async / status message / spill limit | `discovery.rs:948-973`, `output_spill.rs` **135** | fixed 30s `shell_hooks.rs:34`; spawn-per-handler already async by accident | med | yes | DO-NOT (one timeout constant + the existing spawn is enough; spill limit only matters once additional_context exists) |
| Enable/disable + trust fingerprint state | `config_rules.rs` **269** + `plugin_trust.rs` **115** + `plugin_fingerprint.rs` **91** | none | med | partially | DO-NOT (trust/plugins are a codex-plugin-stack artifact; the enable bit could ride the hook file) |
| Hook listing / observability API | `registry.rs:388-405` `list_hooks` + `HookListEntry` | `cmd_hooks.rs list/doctor` (174 LOC) | low | yes | ALREADY-HAVE (operant's list/doctor covers the same operator need) |
| Kernel/harness hook bridge | — | `harness_slots.rs:316-360` `HookSlot::dispatch`/`bridge_handler` | none (operant-only) | n/a | ALREADY-HAVE |
| Web UI for hooks | — | `web/src/hooks/` = React utilities, **not** this system; TUI says "managed with `operant hooks`" | low | n/a | ALREADY-HAVE (CLI is the surface by design) |

**Cheapest useful port order**: matcher (49) → stdin payload (shared preamble + 3 events) → output
contract (block + updated_input + additional_context, the 3 that change the run) → discover
`hooks.json`. That is roughly 400 LOC for a mechanism that turns operant hooks from loggers into
policy. The `emit() -> ()` signature is the one thing that must change first, and it should gain a
*separate* drainable path rather than alter the existing fire-and-forget one the kernel bridge
depends on.

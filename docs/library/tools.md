# The tool system

Owns: the mechanism of agent-callable tools — the registry, the builtin
families with their registered names and failure modes, the AFT and
sourcehound bridges, guardrails, approval, and tool state.
Not for: the security taxonomy behind approval
([security.md](security.md)), the kernel tools' sidecar
([kernel.md](kernel.md)), MCP ([mcp.md](mcp.md)), the slash-command
surface ([cli-surface.md](cli-surface.md)).

Source: `crates/operant-core/src/tools.rs` (registry + `OperantTool`
trait), `crates/operant-core/src/tools/` (the families),
`crates/operant-core/src/tool_guardrails.rs`, `crates/operant-core/src/tools/tool_state.rs`,
`crates/operant-core/src/tools/tool_search.rs` +
`tools/tool_search/name_recovery.rs`, `crates/operant-core/src/aft_bridge.rs` +
`crates/operant-core/src/tools/aft_tools.rs`,
`crates/operant-core/src/binary_updater.rs`,
`crates/operant-core/src/schema.rs` (JSON Schema generation),
`crates/operant-cli/src/main.rs` (the conditional registrations the CLI
adds on top of `builtin.rs`).

Counts on this page are as of commit `826a4568`.

## Contents

- [The registry](#the-registry)
- [The interception seam (iter-1722)](#the-interception-seam-iter-1722)
- [The builtin families (`tools/`, registered in `builtin.rs`)](#the-builtin-families-tools-registered-in-builtinrs)
- [The runtime CLI-admission capability system](#the-runtime-cli-admission-capability-system)
- [AFT — 20 IDE-grade tools over a subprocess](#aft-20-ide-grade-tools-over-a-subprocess)
- [The guardrail ladder (`tool_guardrails.rs`)](#the-guardrail-ladder-toolguardrailsrs)
- [Tool schema discipline](#tool-schema-discipline)
- [Failure modes — what the operator sees](#failure-modes-what-the-operator-sees)
- [Intention](#intention)

## The registry

`ToolRegistry` (in `tools.rs`) holds `Arc<dyn OperantTool>` behind a
semaphore: the agent loop executes tool calls concurrently (8-worker
pool, iter-56). Every tool is a named `OperantTool` with a
schemars-generated JSON Schema; `tools/tool_state.rs` persists
enable/disable state to a JSON file next to the database — and since
iter-1124 the RUNTIME reads it too, so `operant tools disable` actually
changes the agent's toolset.

`tools/tool_search.rs` adds a **deferred-dispatch trio**: `tool_search`,
`tool_describe`, `tool_call` — the model can discover and call tools
without every schema riding the prompt (hermes tool_search parity).

### The registration path, in order

1. `register_builtin_tools` (`crates/operant-core/src/tools/builtin.rs`)
   registers the unconditional builtins. `builtin_tool_names()` in the
   same file is the NAME SURFACE `operant tools list` reports and
   `operant tools disable` matches; it is `builtin.rs`'s registration
   minus `delegate_task` and `mcp_management`, plus the three bridge
   tools, plus `sourcehound_composites::composite_names()`. That identity
   is asserted by `builtin.rs`'s own `test_register_all_builtin_tools`.
2. `register_builtin_tools_with_sub_agent` adds `delegate_task` with the
   parent's disabled sets and approval allowlist threaded in, so a child
   can never regain a tool the parent banned.
3. `crates/operant-cli/src/main.rs` then adds what depends on live state:
   `register_ctx_tools` always (2 tools), `register_lcm_tools` when
   `agent.context_engine = "lcm"` (8 tools, 7 without an embedder),
   `register_harness_tools` when `[harness] enabled`, the plugin tools,
   and `memory_provider_tools::register_provider_tools` for an active
   provider. `tools/kernel/mod.rs::register` runs inside `builtin.rs`
   and installs `kernel_exec` / `kernel_state` / `kernel_refine` plus
   the phase-2.5 tool bridge when `[tools.kernel] enabled`.

So the model-visible count is not a constant: it is the builtin set plus
the engine-conditional sets, and every one of those gates is a config key
or a provider capability, never a compile-time list.

### Name recovery for a hallucinated name

`tools/tool_search/name_recovery.rs` turns a `ToolNotFound` into one of
three outcomes instead of a dead end the model retries: recover onto a
single near-miss, refuse because two candidates are equally close, or
refuse because nothing is close enough. The threshold is `RECOVERY_THRESHOLD`
= 2 edits, case-insensitive, and recovery requires distance PLUS
uniqueness — `resolve` reads `ToolRegistry::get_schemas()`, so a disabled
or unavailable tool can never be resurrected. Ties are named in the error
so the model can choose.

## The interception seam (iter-1722)

`ToolRegistry::register_tool_interceptor` is the VETO-capable path
around `execute`: a `pre-execute` handler can stop a tool before it
runs, and a `post-execute` handler can replace a result with an error —
deepseek-harness `tools/pre-execute` / `tools/post-execute` waterfall
parity, reusing the vestigial `PipelineAction` verdicts. The emit-only
HookRegistry cannot veto; only this seam can. Consumers: WASM observer
plugins (see [plugins.md](plugins.md)) — resident module instances since
iter-1725 (`runtime::PluginPool`: one WASM compile per module per process,
fail-open on any load/call/pool-wait failure), each event envelope carrying
the plugin manifest's `[config]` table. An empty interceptor list is the
default and stays allocation-free; first Block wins; a veto is a typed
error RESULT to the model (`blocked by interceptor: <reason>`), not a
loop error.

## The builtin families (`tools/`, registered in `builtin.rs`)

Each row names the REGISTERED tool names (the strings the registry keys
on and the model sees), the module that implements them, and what the
tool returns when it cannot do its job. "Error result" means a
`ToolResult::error(name, msg)` — a typed value the model reads and can
react to — as opposed to a loop-level failure that ends the turn.

### Files, patches and state

| Tool names | Module | When it fails |
|---|---|---|
| `file_read`, `file_write`, `file_search`, `file_list` | `tools/file_tools.rs` | Error result naming the tool (`file_read`, `file_write`, `file_search`, `file_list`). Path-safety rejection comes from the shared `validate_path` before any I/O. |
| `patch` | `tools/patch_tool.rs` | Exact-match miss falls back to a whitespace-normalizing fuzzy match; an ambiguous `find` is REFUSED with the occurrence count, never guessed (`occurrence` names one). Uses `file_tools::atomic_write` and `holon_write_fence`. |
| `file_state` | `tools/file_state.rs` | Watches are keyed by `(session_id, path)` (RT-538) so two gateway chats editing one file never see each other's diff. A path with no snapshot is a fresh watch, not an error. |
| `working_diff` | `tools/working_diff_tool.rs` | `working` mode runs `git diff` plus an untracked list; `staged` runs `git diff --cached`. Caps: 50 untracked files, ~100 KB of diff. A non-git or missing-git failure is an error result carrying git's own stderr. |

`tools/patch/` is a directory module beside `patch_tool.rs`; the tool
itself is the 1,359-line `patch_tool.rs`.

### Execution: terminal, processes, code

| Tool names | Module | When it fails |
|---|---|---|
| `terminal` | `tools/terminal_tool.rs`, backends in `tools/terminal_backend.rs` | Backend is `local` / `docker` / `ssh` (`config.terminal_backend`, default `local`). Output is capped by `tools.terminal.max_output_bytes` (default 1,000,000) and the cut is reported by the backend's own marker (`[output truncated]` or `...[truncated]`), which the tool sniffs because `CommandOutput` does not carry a flag. With `[compression] shell = "rtk"` and rtk missing, the command still runs UNCHANGED and the label says `rtk not found — commands were not compressed`. |
| `process` | `tools/process_tool.rs` over `crate::process_registry::ProcessRegistry` | Actions are `list`, `spawn`, `poll`, `wait`, `kill`, `get_output`. An unknown action or a missing `session_id` is an error result naming the action it wanted. |
| `code_execution` | `tools/code_execution.rs` | python / js / rust / shell on the LOCAL HOST with a wall-clock timeout and process-group teardown (grandchildren included). Explicitly NOT a sandbox. Capture is capped at 50,000 bytes stdout / 10,000 bytes stderr with head/tail truncation. A timeout kills the group and returns the partial output plus the timeout notice. |
| `tool_backend` | `tools/tool_backend_helpers.rs` | An introspection tool, not an executor: it reports which backends exist for a capability family (`web_search` → `sourcehound`; `code_execution` → `local`, `docker`; `terminal` → `local`, `docker`, `ssh`; `vision` → `openai`, `anthropic`). An unknown family is an error result listing the ones it knows. |

### Web, browser and the sourcehound composites

| Tool names | Module | When it fails |
|---|---|---|
| `web_search`, `web_fetch` | `tools/web_tools.rs`, providers in `tools/web_providers/` | Search is sourcehound-only (`build_search_candidates` returns exactly one provider). `web_fetch` runs the SSRF verdict (`security::ssrf_verdict`) before any request. Timeouts come from `tools.web.search_timeout_secs` (15) and `tools.web.fetch_timeout_secs` (30). |
| `web_scrape`, `web_extract`, `web_crawl` | `tools/sourcehound.rs` | Each self-disables via `is_available()` when the `sourcehound` binary is absent — the tool is registered but not advertised, so the model is never offered a tool that cannot run. |
| 23 composites: `papers_search`, `papers_get`, `news_fetch`, `news_search`, `social_search`, `social_feed`, `video_transcript`, `data_query`, `gov_search`, `patents_search`, `company_lookup`, `security_vuln`, `recon_target`, `archive`, `catalog_list`, `data_catalog`, `index_query`, `results_get`, `workflow_run`, `search`, `read`, `crawl`, `cloakctl_call` | `tools/sourcehound_composites.rs` | One proxy struct registered 23 times from a static `CompositeSpec` table; each declares `ToolExposure::Deferred`, so they cost ~760 catalog tokens on the turn that researches rather than ~4,738 on every request. Every call sets `format: "json"` itself (RT-188/189). Each self-disables when the engine binary is missing. A duplicate name at registration is skipped rather than silently overwriting. |
| `browser` (actions include `accessibility_tree`), `browser_dialog`, `browser_cdp` | `tools/browser_tool.rs`, `tools/browser_dialog_tool.rs`, `tools/browser_cdp_tool.rs`, shared transport in `tools/cdp_utils.rs` | See [browser.md](browser.md). `cdp_utils::send_cdp_command` maps a socket failure to `Error::ToolExecution { name: "cdp" }` so the model sees a tool error, not a panic. |

### Memory, skills, SOPs and the org work

| Tool names | Module | When it fails |
|---|---|---|
| `memory_store`, `memory_search` | `tools/memory_tools.rs` | Retired names (`memory_recall`, `memory_save`, `memory_smart_search`) resolve through the registry's ALIAS layer at zero schema cost. The provider degrades to a logged miss, never a dead turn. See [memory.md](memory.md). |
| `skills_list`, `skill_view`, `skill_manage` | `tools/skills_tool.rs` | Vendor roots come from `crates/operant-cli/src/skill_roots.rs` (ONE list). `skill_manage` stays pool-only, so a vendor skill can never be written in place. See [skills.md](skills.md). |
| `sop` | `tools/sop_tool.rs` | Reads `~/.operant/sops/*.toml`; no name lists them, a name reads one. An unreadable or unknown SOP is an error result naming what it looked for. |
| `cron` | `tools/cron_tool.rs` over `cronjobs::db::CronDb` | Cron expressions are calendar rules in UTC; `every N<unit>` phrases are fixed durations. Actions: `create`, `list`, `get`, `update`, `delete`, `pause`, `resume`. Delivery origin is captured from the live surface into the turn's `ToolContext` (RT-534), so a model cannot invent a delivery target. |
| `kanban` | `tools/kanban_tool.rs` over `kanban::KanbanDb` | Action-keyed (`action` + optional fields). An unknown action or a missing required field is an error result naming it. |
| `todo` | `tools/todo_tool.rs` | FILE-BACKED per session (`<operant_home>/todos/<hash>.json`) since iter-1234; a fresh process rehydrates from disk. Content and item count are capped, duplicates collapse, and `merge: true` updates by id. Active items are re-injected after compaction. |
| `learning_manage` | `tools/learning_mutation_tool.rs` | `action = "delete"` or `"edit"` on a learning-graph node id. An unknown node or a malformed id is an error result from `learning_graph`. |
| `config_manage` | `tools/config_tool.rs` | `get` / `set` / `show` / `path` / `reload` against the process-global runtime config. A `set` is validated by round-tripping `AppConfig`; an invalid change is REJECTED, never silently applied. Changes are runtime-only by design — persistence is `operant config set` or a file edit. |
| `checkpoint` | `tools/checkpoint_tool.rs` | Shadow-git snapshots; see [persistence.md](persistence.md). |
| `session_search`, `session_insights`, `verify_task`, `evidence_recall` | `tools/session_search_tool.rs`, `tools/insights_tool.rs`, `tools/verification_tool.rs`, `tools/evidence_recall.rs` | `evidence_recall` resolves the key a compression marker named (default 4,096 chars, ceiling so one recovery cannot spend more context than the compression saved). An unknown key is an error result. |

### Comms and notifications

| Tool names | Module | When it fails |
|---|---|---|
| `send_message` | `tools/send_message_tool.rs` | Telegram / Discord / Slack / generic webhook with platform-aware chunking (Telegram 4096, Discord 2000, Slack 40000; webhooks unbounded). A missing token or an unreachable API is an error result; a text that exceeds the platform limit is SPLIT, not refused. |
| `react_to_message` | `tools/reaction_tool.rs` | Telegram / Discord / Slack. Slack needs an emoji NAME, so a glyph is mapped through `GLYPH_TO_SLACK_NAME`; an unmapped glyph on Slack errors with a hint instead of failing silently. |
| `discord`, `discord_admin` | `tools/discord_tool.rs` | REST API with `DISCORD_BOT_TOKEN`, 15 s timeout. `discord` reads and participates; `discord_admin` manages the server. Missing token is an error result. |
| `homeassistant` | `tools/home_assistant_tool.rs` | REST API with `HASS_URL` (default `http://homeassistant.local:8123`) and `HASS_TOKEN`. Entity ids are validated against `^[a-z_][a-z0-9_]*\.[a-z0-9_]+$`, and dangerous domains (`shell_command`, `python_script`) are blocked outright. |
| `notification` | `tools/notification_tool.rs` | Delivers a user-facing notification (optional title). The registered name in the tool's own impl is `notify` — see the stale-claim note under Failure modes. |
| `approval_request` | `tools/notification_tool.rs` | The interactive approval prompt. It and `clarify` are granted a 300 s tool timeout by `main.rs` because they block on a human, not on compute. |

### Ops, datetime and environment

| Tool names | Module | When it fails |
|---|---|---|
| `http_request` | `tools/http_tool.rs` | Full header control. Timeout from `tools.http.timeout_secs` (default 30). A non-2xx is returned as the response, not as a tool failure, so the model can read the status. |
| `datetime`, `timestamp` | `tools/datetime_tool.rs` | `datetime` takes an optional timezone (offset or name) and a strftime format; `timestamp` returns epoch seconds. An unparseable timezone is an error result naming the value. |
| `env_probe` | `tools/env_probe_tool.rs` | Audits which environment variables LOOK secret and are exposed to this process. Observation-only: reports NAMES, never values. An optional `dotenv_path` audits a `.env` file without loading it. |
| `debug_env`, `echo`, `debug_system` | `tools/debug_helpers.rs` | `debug_env` lists variables, redacting any whose name contains KEY / TOKEN / SECRET / PASSWORD / PASS / CREDENTIAL. `debug_system` reports platform facts. |
| `osv_check` | `tools/osv_check.rs` | Vulnerability lookup by package/version. An unknown ecosystem or an unreachable OSV API is an error result. |
| `image_generate`, `text_to_speech`, `transcribe_audio`, `video_analyze`, `vision_analyze` | `tools/image_generation_tool.rs`, `tools/tts_tool.rs`, `tools/transcription_tool.rs`, `tools/video_analysis_tool.rs`, `tools/vision_tool.rs` | See [voice.md](voice.md) and [vision.md](vision.md). Each has its own provider config; a missing key is an error result, not a silent empty result. |

Note: `text_to_speech` is the registered name of `TtsTool` (the file is
`tts_tool.rs`), so it is not a typo when a config or a doc says `tts`.

### Spotify

`tools/spotify_tool.rs` registers SEVEN tools: `spotify_playback`,
`spotify_devices`, `spotify_queue`, `spotify_search`,
`spotify_playlists`, `spotify_albums`, `spotify_library`.

- **Auth**: `HERMES_SPOTIFY_ACCESS_TOKEN` alone, or `SPOTIFY_CLIENT_ID` +
  `SPOTIFY_CLIENT_SECRET`. Every tool self-disables through
  `is_available()` when neither pair is present, so an unconfigured
  install registers the names but advertises none of them.
- **Failure**: a non-2xx from `https://api.spotify.com/v1` becomes an
  error result carrying the API's body; a 401 means the token expired and
  the tool says so rather than retrying.
- **Args** are per-tool structs (`PlaybackArgs` carries an `action` enum:
  get / start_resume / pause / next / previous / seek / set_repeat /
  set_shuffle / set_volume / transfer).

### Delegation

`sub_agent` (`tools/sub_agent_tool.rs`, registered name `delegate_task`)
is documented in [sub-agents.md](sub-agents.md). Two pieces of its
machinery live in `tools/` and are worth naming here:

- `tools/delegation_output_schema.rs` — an optional per-task
  `output_schema` (a JSON Schema object). The child is told about the
  contract in an OUTPUT CONTRACT block, the parent validates the child's
  final answer with a deliberately small JSON-Schema subset validator
  (root `type`, per-property `type`, `required`, nested `properties`,
  `items`), and on failure sends exactly ONE bounded retry carrying the
  validation errors verbatim (`MAX_SCHEMA_RETRIES = 1`, at most 10 errors
  rendered). More retries made frontier models DROP fields that were right
  the first time.
- `tools/foreground_swarm.rs` — the live membership registry a foreground
  `delegate_task(tasks=[…])` writes and `App::swarm_gallery_members`
  reads. It is a PROCESS fact: it dies with the process, and the durable
  account is the transcript's swarm card. Before it, a running swarm was
  invisible while it ran.

### Prime Kernel and Harness (opt-in)

`tools/kernel/` registers `kernel_exec` (`kernel_tool.rs`),
`kernel_state` + `kernel_refine` (`kernel/harness_tools.rs`) and the
phase-2.5 tool bridge (`kernel/tool_bridge.rs`, allowlist deny-by-default,
64 calls/exec, 60 s per call), all gated on `[tools.kernel] enabled`
(default false). `tools/harness_tools.rs` registers `harness_dump`,
`harness_mount`, `harness_unmount`, gated on `[harness] enabled`;
mount and unmount additionally require `ToolContext.metadata["approval"]`.
See [kernel.md](kernel.md) and [harness.md](harness.md).

### LCM context tools (opt-in)

`tools/lcm_tools.rs` registers `lcm_recall`, `lcm_stats`, `lcm_assert`,
`lcm_recall_round`, `lcm_recent`, `lcm_doctor`, `lcm_load_session`, and
`lcm_vector_recall` only when an embedder is configured — 8 tools with
one, 7 without. They load ONLY when `[agent] context_engine = "lcm"`;
`lcm_assert` is granted a 180 s timeout because its `action = "extract"`
arm runs a reasoning model.

### Agent-facing context tools (always on)

`tools/ctx_tools.rs` registers `ctx_status` and `ctx_search` for EVERY
engine, from `main.rs` right after the builtins. They are READ-ONLY ON
PURPOSE: the write half (mark / restore / pin / sweep) needs a state model
that could corrupt a live request, so it is a separate iteration.

- `ctx_status` answers "what is in my window right now": the provider's
  own token count for the last request, the engine's estimate, the split
  between tool output and conversation, the budget, and the session's
  totals.
- `ctx_search` is token-efficient recall of the agent's OWN session
  context, scoped to the session it is in, with a hard token budget on the
  evidence it returns.

Both resolve the session as: the `session` argument, then the
`session_id` the tool context carries, then the newest session in the
database.

## The runtime CLI-admission capability system

This is a documented feature with three modules and no compile-time
catalogue. The requirement it serves is the operator's: **"nothing of
CLI/MCP may be compile time."**

- `tools/cli_roster.rs` — the roster. It holds `RosterEntry` values
  (name, `version_args`, `CliCategory`, parsed `capabilities`,
  `admitted_at`) in `<config dir>/cli_roster.toml`, resolved through
  `roster_path_from_config` so `HERMES_HOME` and the active profile are
  honoured. A missing file is an EMPTY roster; a corrupt file is also an
  empty roster, not an error. `seed_entries()` names 16 binaries
  (git, python, python3, node, npm, pip, pip3, docker, cargo, make,
  kubectl, rustc, claude, gemini, kilo, gws) as the catalogue shown
  BEFORE anything has been discovered. Two files, two jobs: the roster is
  what EXISTS on this machine and what it yields; `[cli.tools]` in
  `operant.toml` is what the operator CHOSE to grant.
- `tools/cli_help_parser.rs` — turns one `--help` into N capabilities.
  The grammar is deliberately narrow: a line whose first token matches
  `[A-Za-z0-9][A-Za-z0-9_-]*`, followed by two spaces, a comma, or
  end-of-line. A leading `-` is a FLAG, not a command. Output is DATA:
  `MAX_DESCRIPTION_CHARS = 200`, `MAX_CAPABILITIES = 500`,
  `MAX_HELP_LINES = 500`, so a hostile binary cannot turn a `--help` into
  a context bomb. A binary whose help yields nothing parseable still
  contributes ONE capability (the binary itself) via
  `whole_binary_capability`.
- `tools/cli_discovery.rs` — the PATH scan. `which`/`where` with a 2 s
  timeout, `<tool> --version` with a 3 s timeout, `CliCategory` labels
  (VersionControl, Language, PackageManager, Container, Build, Cloud,
  AiAgent, Productivity). Every execution of an operator-named binary
  goes through its one spawn primitive, which closes stdin and applies a
  hard timeout.

The MCP parity is exact: one MCP server yields N tools over `tools/list`;
one binary yields N subcommand capabilities over `--help`. Discover at
runtime, persist the result, advertise names cheaply, resolve schemas on
demand.

The operator drives it with `operant context cli list|add|remove|toggle`
(`crates/operant-cli/src/cmd_context.rs`, `CliSubcommand`). `add` ADMITS
a binary that is on PATH and refuses one that is not; `--description`,
`--global-args` and `--allow-subcommands` narrow the grant.

STATUS NOTE, and it matters: `[cli] enabled` defaults to `false`, and the
agent-facing half (`cli_exec` plus the `<cli_reference>` prompt block) is
Phase C of `docs/plans/plan-2026-10-28-native-capability-extension-mcp-cli-runtime-admission.md`
and has NOT landed. A grant is recorded and COSTED, not invocable.
`[cli] max_help_lines` is accepted but not applied (RT-278) for the same
reason.

## AFT — 20 IDE-grade tools over a subprocess

`aft_bridge.rs` spawns the `aft` binary (auto-downloaded from GitHub
releases, auto-updated via the shared `binary_updater.rs`; one subprocess
per project root, lazily) and `tools/aft_tools.rs` registers 20 tools:
`aft_read`, `aft_write`, `aft_edit`, `aft_bash`, `aft_grep`, `aft_glob`,
`aft_search`, `aft_outline`, `aft_zoom`, `aft_inspect`, `aft_ast_search`,
`aft_ast_replace`, `aft_apply_patch`, `aft_callers`, `aft_checkpoint_paths`,
`aft_checkpoint`, `aft_list_checkpoints`, `aft_restore_checkpoint`,
`aft_undo`, `aft_status`. NDJSON protocol; params flat at top level
except `bash` (nested + async).
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
and a side-effecting tool can mutate state repeatedly. The ladder is a
pure per-turn tracker (`ToolGuardrailTracker`) whose decisions are
`Allow` / `Warn` / `Skip` / `Halt(String)`. The rungs, with the
constants read from the source:

| Rung | Fires at | Effect |
|---|---|---|
| Identical-args repeat | `REPEAT_WARN_THRESHOLD = 3` warn; `REPEAT_SKIP_THRESHOLD_EFFECT = 3` (side-effecting) / `REPEAT_SKIP_THRESHOLD_NO_EFFECT = 4` | Warn, then a synthetic skip result |
| Ping-pong (two tools alternating) | `PING_PONG_MIN_CYCLES = 4` warn; `PING_PONG_SKIP_CYCLES = 5` | Warn, then skip; window `PATTERN_WINDOW_SIZE = 20` |
| No-progress (identical results) | `NO_PROGRESS_MIN_CALLS = 5` warn; `NO_PROGRESS_SKIP_CALLS = 6` | Warn, then skip regardless of args |
| Per-tool failure streak | `FAILURE_WARN_THRESHOLD = 8` warn; `FAILURE_HALT_THRESHOLD = 12` | Warn, then HALT the turn with a root-cause summary |
| Hard policy rejection | `HARD_REJECT_HALT = 2` | HALT — a blocked call re-issued unchanged can never succeed (`HARD_REJECT_PREFIX = "Blocked by security policy"`) |
| Identical assistant output | `OUTPUT_REPEAT_WARN = 4`; `OUTPUT_REPEAT_SKIP = 5` | Warn, then skip — never halt (repeated SUCCESSES are corrected, not punished) |
| Claim-without-action | `PROGRESS_TOKEN_WARN = 4`; `PROGRESS_TOKEN_SKIP = 5` | Warn, then skip; catches a model that rewords its "next I'll…" every turn |

No-effect tools (`NO_EFFECT_TOOL_NAMES`: `file_search`, `file_read`,
`session_search`, `skill_view`, `skills_list`, `web_search`, `web_fetch`,
`web_extract`, `vision_analyze`, `env_probe`, `browser_snapshot`,
`browser_get_images`, `browser_console`, `datetime`) are safe to re-run
and safe to discard; everything else is presumed to mutate state.
`[agent] guardrail_exempt_tools` (exact names or `*`/`?` globs) removes a
tool from every rung. See [security.md](security.md) for the approval
layer that sits above it.

## Tool schema discipline

`schema.rs` generates OpenAI-compatible JSON Schema from Rust structs;
`crates/operant-api/src/schema.rs` cleans them per provider.
The RT-192 lesson lives here too: doc comments serialize into schemas and
cost wire tokens per request — model-facing text only.

## Failure modes — what the operator sees

1. **A missing subprocess.** sourcehound-backed tools (`web_scrape`,
   `web_extract`, `web_crawl`, the 23 composites) and the Spotify tools
   self-disable through `is_available()`: they stay registered so
   `operant tools list` shows them, but they are not advertised, so the
   model is never offered a tool that cannot run.
2. **AFT missing or broken.** `aft_enabled = true` does not hide the
   native file/terminal tools until the CLI has verified the bridge is
   live. A missing binary degrades to the builtins.
3. **rtk configured but absent.** `[compression] shell = "rtk"` with no
   rtk on PATH runs the ORIGINAL command unchanged (fail open) and labels
   the result `rtk not found — commands were not compressed`.
4. **A hallucinated tool name.** `name_recovery` either recovers onto one
   near-miss (≤2 edits, unique) or returns an error naming the candidates.
   It never runs an unrelated tool.
5. **A crash-looping tool.** The guardrail ladder converts it into a
   warning, a synthetic skip, or a turn halt with a root-cause summary,
   depending on the rung.
6. **A blocked tool.** A pre-execute interceptor veto, an approval denial,
   or the blast-radius gate returns `blocked by interceptor: <reason>` /
   `Blocked by security policy` as a typed error RESULT. The model reads
   it and can choose another path; the loop does not die.

### A stale claim found while writing this page

`builtin_tool_names()` carries the string `"notification"`, but the tool
in `tools/notification_tool.rs` registers itself as `"notify"` (its own
`name()` returns `"notify"`, and its schema test asserts
`schema.name == "notify"`). `crates/operant-core/tests/integration_test.rs`
also asserts `registry.get("notify")` is present. The two name surfaces
therefore disagree, and `"notification"` appears in exactly three places
(tree-wide grep): the name list, `agent/evidence/classify.rs`'s tool
classification, and the TUI's tool-icon table. This is recorded here
rather than fixed because a rename is its own iteration.

## Intention

The tool system is the agent's hands: everything the model can *do* passes
through one registry with one schema generator, one guardrail ladder, and
one approval gate — so a security fix lands once instead of per-tool. The
subprocess bridges (AFT, sourcehound) keep heavy dependencies out of the
binary; the deferred-dispatch trio keeps the prompt small; the
tool_state file makes the operator's disable choices real at runtime.

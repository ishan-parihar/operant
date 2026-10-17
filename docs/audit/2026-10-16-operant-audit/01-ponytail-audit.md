# Ponytail audit — operant @ 0ee25739 (2026-10-16)

**What this repo does:** operant is a Rust coding agent and personal agent
runtime. One binary (`operant`) gives you a ratatui TUI, a one-shot CLI, an
autonomous TODO.md mode, and a gateway daemon. The gateway connects Telegram,
Discord, Slack and other chats, runs cron jobs, and hosts the Holon
"organization" organs (`operant-os`). The agent loop lives in `operant-core`.
It talks to one OpenAI-compatible endpoint (here, the local omp gateway at
`localhost:20129`). Memory is memory-wire, in-process. Web and browser go
through sourcehound, and code tooling through aft.

**Load I assumed:** one owner on one workstation. That means one or two TUI
sessions, plus one always-on gateway (systemd user unit) serving the owner's
chats from several platforms and channels, about 10 cron jobs, and, in org
mode, a few Holon bodies at once. So "many users" means a few chats or seats
at the same time on the gateway. That is enough for races to happen every day.

Findings are ordered most important first. Each one has a plan file with test-first
steps (named in the Fix bullet).

## Must fix

**1. Gateway chats run at the same time on one agent with one conversation slot — `crates/operant-cli/src/gateway_runner.rs:784,936,1019-1026,3694-3707`, `crates/operant-core/src/gateway/mod.rs:255-256`, `crates/operant-core/src/agent/events.rs:713`**
- What this is: every gateway message is a `tokio::spawn`. The turn lease
  blocks only a second turn in the *same* chat. All chats share one
  `Arc<OperantAgent>`, and `set_session_id` swaps the agent's single hot
  conversation.
- Problem: Telegram chat A is mid-turn, then Discord chat B arrives.
  B calls `set_session_id(B)`, which saves A's half-finished turn and loads B's.
  A's next `add_message` then lands in B's history. Model overrides also leak
  across chats (`set_model` is per agent). `/stop` in A also stops B, and B's
  start calls `stop_flag.reset()`, which drops A's stop. BUGS W3-1 fixed only
  *sequential* switching.
- Fix: one `tokio::sync::Mutex<()>` field on `GatewayMessageHandler`, held
  from `set_session_id` to the end of `agent.run`. Busy chats wait their turn.
  See `06` part A.
- If we skip it: one person's messages and memory show up in another chat's
  replies, and stopping one chat stops or un-stops another.

**2. `/stop`, `/clear`, `/new` mark the turn idle while the agent keeps running — `crates/operant-cli/src/tui/app/commands.rs:555-575,618-632,637-655`, `crates/operant-cli/src/tui/adapter_types/tui_app.rs:412-450`**
- What this is: these arms set `is_streaming = false` but never trigger the
  interrupt flag (only `/cancel` → `cancel_streaming_turn` does).
- Problem: type `/stop` mid-answer, then a new prompt. The composer path sees
  `is_streaming == false` and spawns a second `agent.run` on the same agent
  while the first is still streaming. You get two writers on one conversation,
  and events from the old run paint into the new turn.
- Fix: route `stop` to the same code as `cancel`. In `/clear` and `/new`,
  refuse while streaming ("Cancel first") or cancel first. Guard the spawn on
  `agent_task_handle.is_finished()`. See `03` §1.
- If we skip it: corrupted context and duplicated or mixed answers, with no error shown.

**3. `/clear`, `/new`, `/undo`, `/retry` change only the screen; the model keeps everything — `crates/operant-cli/src/tui/app/commands.rs:555-718`, `crates/operant-cli/src/tui/adapter_types/tui_app.rs:875-1032`**
- What this is: the arms edit `App.messages` (display only).
  `run_pending_agent_actions` handles repair, branch, rewind, effort and compact,
  but none of these four. The agent already has `clear_history()`,
  `set_session_id()` and `rewind_to_turn()`.
- Problem: `/clear` prints "Conversation cleared.", and the next answer still
  quotes the old chat. `/retry` removes the last turn from the screen, then
  sends it again, so the model sees the question twice. `/undo` says "Last turn
  undone" but the model still has it.
- Fix: add `pending_clear` / `pending_new` / `pending_undo` and drain them in
  `run_pending_agent_actions` with the existing agent APIs. `/undo` and
  `/retry` reuse `rewind_to_turn` (rewind forks, nothing is lost). Then rebuild
  `App.messages` from `agent.conversation()`, the way the rewind branch already
  does. See `03` §2.
- If we skip it: users trust a "cleared" context that is not cleared. That is a
  privacy and correctness lie.

**4. `/model` never switches the running agent, and lists the wrong catalogue — `crates/operant-cli/src/tui/app/providers.rs:10-100,255-267`, `crates/operant-cli/src/tui/app/key_handling.rs:678-718`, `crates/operant-cli/src/tui/model_picker/render.rs:136`**
- What this is: `App::set_model` updates the status bar, the cost tracker and
  the TOML file. It never calls `agent.set_model`. The picker lists models.dev
  entries for a provider *guessed from the model name* (default `"anthropic"`),
  not the endpoint the agent really uses (`config.client.base_url`, here the
  omp gateway).
- Problem: you pick a model, the status bar changes, and every request still
  goes to the old model until a restart. The list shows `gpt-4o` three times and
  `gpt-4o-mini` four times (dated ids rendered by display name). The
  gateway's own routes (`small-stack`, …) are never offered. Alt+P cycling
  (`providers.rs:222-252`) has the same bug.
- Fix: the picker reads `GET {client.base_url}/models` plus `[agent] model` and
  `fallback_models`, and shows ids. Confirm sets `pending_model`, and
  `run_pending_agent_actions` calls `agent.set_model`. The current marker comes
  from `agent.model()`. See `05` part A.
- If we skip it: the main "switch model" feature does nothing visible to the model.

**5. Permission mode is a status badge only — `crates/operant-cli/src/tui/app/channel_drains.rs:272-333`, `crates/operant-cli/src/tui/render/utils.rs:123-138`, `crates/operant-cli/src/tui/app/commands.rs:601-615,1097-1123`**
- What this is: `/permissions`, `/plan` and `/yolo` set
  `settings.permission_mode`. Its only readers are the status-line renderer
  and tests. `drain_permission_requests` never looks at it.
- Problem: "Plan mode ON — Operant will plan before acting" still runs every
  write tool after an ordinary prompt. "YOLO mode armed" still prompts for
  everything. `/yolo` also writes `BypassPermissions` to `settings.json` with no
  confirmation, skipping the dialog `/permissions` insists on.
- Fix: in `drain_permission_requests`, `BypassPermissions` answers
  `AllowSession` and `Plan` answers `Deny` with "plan mode: read-only".
  `Default` shows the dialog. The core hardline and danger checks still run
  first in `stream.rs`. Make `/yolo` open the same confirm dialog. See `03` §3.
- If we skip it: the user thinks Plan mode protects them. It doesn't.

**6. "Always allow" for bash trusts the first word, so `cd x && rm -rf ~` passes — `crates/operant-cli/src/tui/app/dialog_routing.rs:226-243`, `crates/operant-cli/src/tui/app/channel_drains.rs:297-311`**
- What this is: the session allowlist stores `command.split_whitespace().next()`
  and auto-approves any later command that starts with it.
- Problem: allowing `git status` once allows `git push --force origin main`.
  Allowing `cd` allows `cd /tmp && rm -rf ~`.
- Fix: refuse prefix auto-approve when the command contains `;` `&&` `||` `|`
  `` ` `` `$(` `>` `<` or a newline. Store the first *two* tokens. Run
  `operant_core::approval` `check_dangerous` on the full command and never
  auto-approve a dangerous one. See `03` §4.
- If we skip it: one careless "always allow" turns into arbitrary command execution.

**7. The TUI rewrites your whole `operant.toml` on every model or provider change — `crates/operant-cli/src/tui/app/providers.rs:131-166`**
- What this is: `sync_model_to_toml` serializes the *runtime* config (after CLI
  flag and env overrides) with `toml::to_string_pretty`, then writes it to
  `~/.operant/operant.toml`.
- Problem: it drops every comment. It writes `~/.operant/operant.toml` even
  when you launched with `-c other.toml`. It forces `client.base_url` to the
  provider's public URL (breaking a `localhost:20129` gateway setup). It writes
  `OPENAI_API_KEY` and any `--api-key`/env tokens into the file as plain text,
  and it brings back every removed default key (that is why the user's file
  still has `agentmemory_auto_spawn`).
- Fix: change only `agent.model` with `toml_edit` (already in `Cargo.lock` via
  `operant-config`) in the config file that was actually loaded. Never touch
  `client.*`. See `09`.
- If we skip it: silent config loss and secrets on disk, every time someone uses `/model`.

**8. Ten test targets don't compile, so the gate can't be met — `crates/operant-cli/src/gateway_commands.rs:2611-2940`, `crates/operant-cli/src/gateway_runner.rs:4558`, `crates/operant-core/tests/{org_wave2_shared_file,governance_escalation,seat_policy_run_path,cron_session_isolation,org_identity_gate,org_notice_board,org_worklog_framework_only,org_employee_registry,integration_test}.rs`**
- What this is: the tests still import retired org modules (`authority`,
  `employee_db`, `notice`, `worklog_db`, `write_barrier`, …) and the purged
  `CamofoxStateTool`.
- Problem: `cargo test -p operant-cli --bin operant` (gate 2) fails with 11
  errors, so the iter-850 budget test and the rewind-selector tests have never
  run. `cargo test --workspace` fails in 9 core targets.
- Fix: delete the tests of retired features (the production `grant` command
  now only prints a Holon redirect, `gateway_commands.rs:2254`). Rewire the
  `/permissions` test to `seat_policy_db`. Drop the Camofox import. See `02`.
- If we skip it: every "gates green" claim stays unverifiable, and regressions land unseen.

**9. Native scroll hides or loses rows after `/clear`, `/new`, rewind, resume and resize — `crates/operant-cli/src/tui/app/scroll_emit.rs:17-20,50-58,85`, `crates/operant-cli/src/tui/operant_ui/ui_viewport.rs:423`**
- What this is: the emitted-rows watermark is a row count. No code resets it
  when the transcript shrinks. On resize it jumps to the new count without
  emitting anything.
- Problem: after `/clear` in native-scroll mode the watermark is still, say,
  500. New rows are not printed to history until the transcript passes 500
  rows, and the viewport floor (`scroll.max(scroll_emitted_rows)`) hides them
  from the live view too. A message that settles in the same frame as a resize
  is never emitted. The code comment says it "never re-emits on resize", which
  goes against the user's decision ("always replay at the new width").
- Fix: a message-index watermark, plus `replay()` (clear screen and scrollback,
  re-emit) on clear, new, rewind, session switch and debounced resize. See `07`.
- If we skip it: native scroll, the mode the user asked to make the default, loses content.

## Should fix

**10. Memory is still split between memory-wire and the old MEMORY.md files — `crates/operant-cli/src/cmd_memory.rs:107`, `crates/operant-cli/src/cmd_doctor/checks_config.rs:356-383`, `crates/operant-cli/src/tui/app/commands.rs:1580-1625`, `crates/operant-cli/src/tui/journey_view.rs:95`, `crates/operant-core/src/config.rs:1505-1592`**
- What this is: since 11C3 the agent reads and writes memory-wire only. But
  `operant memory` (13 subcommands), doctor, `/whoami` and `/journey` still use
  `MemoryManager` and `MemoryStore` over MEMORY.md/USER.md.
  `MemorySettings::default()` is `provider="agentmemory"`,
  `agentmemory_auto_spawn=Some(true)`.
- Problem: `operant memory store x` writes a file the agent never reads.
  `/whoami` says "I don't know much about you" while memory-wire has
  memories. Every run prints the agentmemory warning, even with the clean
  example config (seen in the simulator's stderr).
- Fix: point those surfaces at `MemoryWireProvider`, delete what has no
  memory-wire equivalent, delete the `agentmemory_*` fields and set the
  default to `memory-wire`. See `08`.
- If we skip it: two memory stores that never agree. That is the fragmentation the owner asked to remove.

**11. Two command tables disagree; autocomplete offers broken commands and hides working ones — `crates/operant-cli/src/tui/app/helpers.rs:17-23`, `crates/operant-cli/src/tui/operant_app/app.rs:242-336`, `crates/operant-cli/src/commands.rs:474-920`, `crates/operant-cli/src/tui/app/commands.rs:276-1648`**
- What this is: typeahead and help come from the Hermes `COMMAND_REGISTRY` (90
  non-gateway entries). The TUI has its own `REGISTERED_COMMANDS` (57), plus a
  106-arm `match`, plus 24 registry handlers.
- Problem: 18 suggested commands answer "Command /cron is not yet wired to a
  handler" (`cron`, `kanban`, `curator`, `suggestions`, `profile`,
  `snapshot`, `history`, `title`, …). `/rewind`, `/permissions`, `/effort`,
  `/cls` and `/thinking-display` never autocomplete (checked: `/rew` shows
  nothing). `/split` and `/mcp add x` are sent to the model as prompts
  (checked: `/split` reached the provider).
- Fix: one table, the TUI one, and every row must have a handler, enforced by
  the existing `no_registered_tui_command_is_unwired` test. Keep
  `COMMAND_REGISTRY` for the gateway only. See `04`.
- If we skip it: "slash commands don't work" stays the top user complaint.

**12. Handlers that report success for things they didn't do — `crates/operant-cli/src/tui/app/key_handling.rs:739,750,761`, `crates/operant-cli/src/tui/app/commands.rs:1127-1138,1247-1254,1359-1364,1388-1394,1431-1437`**
- What this is: the branch browser prints "Switched to branch", "Created
  branch" and "Deleted branch" and does none of them. `/rollback` (registry:
  "restore filesystem checkpoints") opens a diff viewer. `/busy` toggles
  auto-compact. `/browser` talks about Camofox (purged in 11C2). `/mouse`
  always says "enabled". `/update`'s help says "background update and auto
  reload" but it prints a hint. `/reload`'s help says "reload into newest
  binary" but it reloads settings.
- Problem: the user acts on false status lines.
- Fix: wire branch switch to `agent.set_session_id(branch.id)` plus a display
  rebuild (create = the existing rewind fork, delete = DB delete), or remove
  those keys. Delete or honestly re-describe the rest. See `04` §3.
- If we skip it: users stop trusting any status line.

**13. The jcode visual port is half-wired; legacy overlays do the work — `crates/operant-cli/src/tui/app/tui_state_impl.rs:970-978`, `crates/operant-cli/src/tui/operant_ui/mod.rs:82,3387-3395`, `crates/operant-cli/src/tui/render/operant_overlays.rs:480-650`**
- What this is: the jcode inline picker renderer (`ui_inline_interactive.rs`,
  1.5k lines) and the session picker (`operant_ui/session_picker*`, about 9.8k
  lines with tests) are compiled, but `inline_interactive_state()` returns
  `None` and nothing opens `session_picker`. Meanwhile 20+ legacy modal
  overlays (`model_picker/`, `session_browser`, `effort_picker`,
  `connect_dialog`, …) paint on top. There are 213 `[port-decision]` stubs.
- Problem: the user sees the old modals, not jcode's. Two implementations of
  every picker exist, and only the older one is reachable.
- Fix: wire the ported state (one `Option<InlineInteractiveState>` on `App`)
  for model, effort, login and logout, and open `session_picker` for
  `/resume`. Delete each legacy twin in the same commit that replaces it.
  See `05` part B.
- If we skip it: twice the UI code, and still not jcode's look.

**14. Role-specific memory is not wired; every gateway chat and seat shares one bank — `crates/operant-core/src/memory_wire.rs:85-95,281`**
- What this is: `bank_for` / `MemoryWireProvider::scoped` (11E) exist, but
  only tests use them.
- Problem: in personal-mode gateway use, a memory retained in one seat's or
  chat's turn is recalled into every other seat's turns.
- Fix: after finding 1's lock, set the provider's bank per turn from
  `seat_id()` (`emp:<id>`), or the default bank for the owner. Holon runs never
  write (O-22). See `06` part B.
- If we skip it: no role memory, which is the core of the organism design.

**15. Typed prompts don't cancel the idle recap; the submit code is copy-pasted — `crates/operant-cli/src/tui/adapter_types/tui_app.rs:412-454` vs `486-526`**
- What this is: `submit_user_message` calls `recap_activity.notify()`. The main
  composer path (a near-copy) doesn't.
- Problem: the user sends a new prompt within the idle window and the recap
  still fires during the next turn, which also costs an extra LLM call. Two
  copies will keep drifting.
- Fix: make the composer path call `submit_user_message` (keep its
  steer-queue branch first). That deletes about 25 lines. See `03` §5.
- If we skip it: stray `※ recap:` lines mid-turn.

**16. The headless simulator calls the real provider — `crates/operant-cli/src/cmd_tui_debug.rs` (`tui debug simulate` without `--agent-script`)**
- What this is: with no agent script the sim builds a real agent from env
  (`OPENAI_BASE_URL`).
- Problem: `tui debug simulate --keys '/split<enter>' --api-key sk-test`
  produced "HTTP 400 … provider 'kilocode'" from the live omp gateway.
  Command sweeps and corpora can spend money and flake on the network.
- Fix: when `--agent-script` is absent, use an empty mock script unless
  `--live` is passed.
- If we skip it: flaky corpus runs and surprise API usage.

## Nice to have

**17. About 176k lines live in crates the binary never calls — `Cargo.toml:3-21`, `crates/operant-cli/Cargo.toml:29-35`, `crates/operant-cli/src/gateway_runner.rs:4076`, `crates/operant-core/src/doctor.rs:259,426`**
- What this is: the CLI never names `operant_gateway`, `operant_channels` or
  `operant_hardware`; channels run from `operant-core/src/gateway/`.
  `operant_runtime` is used for one function (`core_observer_from_config`).
  `operant-tool-call-parser` and `operant-tool-planning` are used only by
  `operant-runtime`. Core uses `operant-tools` and `operant-providers` once
  each, in `doctor.rs`.
- Problem: long builds and a big audit surface. Two channel stacks (21 channels
  in `operant-channels`, 8 in core) confuse every reader.
- Fix: plan 11D with the evidence in `10`. Move the one observer factory into
  core, then delete gateway, hardware, channels, runtime, parser and planning,
  one crate per commit with build and gateway-smoke proof.
- If we skip it: slow builds and a quarter of the code to read for nothing.

**18. No-op persistence commands and duplicate helpers — `crates/operant-cli/src/cmd_model.rs:129-150`, `crates/operant-cli/src/cmd_config.rs:69-88`, `crates/operant-core/src/tools/config_tool.rs:178-198`, `crates/operant-cli/src/tui/provider.rs:3` / `crates/operant-cli/src/cmd_tui_debug.rs:780`, `crates/operant-os/src/tools.rs:73-83` / `crates/operant-os/src/run.rs:43-50`**
- What this is: `operant model set` and `operant config set` edit the in-memory
  config of a process that exits right away. `set_nested` exists twice,
  `infer_provider_from_model` exists twice, and two Holon launcher
  resolvers repeat the same `[holon] bin` → `$HOLON_BIN` → `holon` fallback.
- Problem: "Model set to: X" is printed and nothing changes. The copies drift.
- Fix: both CLI setters write through the `09` surgical writer. Delete the
  copies, and share the launcher fallback as one fn. The body-only `$HOLON` rule
  stays in `tools.rs`.
- If we skip it: confusing CLI, and drift between the copies.

**19. Hermes/Nous leftovers and stub dialogs — `crates/operant-cli/src/commands.rs` (`credits`, `billing`, `gquota`, `codex-runtime`, `pet`, `heapdump`, `replay`, `replay-diff`, `background`, `footer`), `crates/operant-cli/src/tui/adapter_types/tui_app.rs:297-314`, `crates/operant-cli/src/cmd_peripheral.rs`, `cmd_hardware.rs`**
- What this is: commands for services operant doesn't use, plus "planned"
  stubs. Device auth and MCP panel auth say "not yet wired — restart". There are
  STM32/Arduino flashing subcommands in a coding-agent CLI.
- Problem: noise in `/help`, and broken flows (Copilot and Codex device login
  never finishes).
- Fix: delete them, or finish device-flow polling if Copilot or Codex login is
  wanted (ask the owner). Hardware goes with `operant-hardware` (17).
- If we skip it: clutter, and one broken login path.

**20. `operant_home()` trusts a relative or placeholder `HERMES_HOME` — `crates/operant-core/src/platform.rs:97-103`**
- What this is: the audit item O-06 was fixed on the Holon side only. operant
  still accepts `HERMES_HOME="{holon_home}"` and writes sessions and stores
  under `./{holon_home}` in the work directory.
- Problem: one bad driver file breaks run isolation (invariant 4) again.
- Fix: if the value is not absolute or contains `{`, log a warning and fall back
  to `~/.operant`. Add one unit test.
- If we skip it: a quiet isolation hole that is easy to reintroduce.

**Verdict:** The core loop is in good shape. The surfaces around it are not
honest. Fix the red test gate first. Next, make the TUI and the gateway drive
the agent's real state (conversation, model, permissions, session) and stop
keeping their own copies. Then delete the unreachable crates and the legacy
twins of the jcode port.

**Lean: about -180,000 lines and -6 workspace crates possible** (gateway,
hardware, channels, runtime, tool-call-parser, tool-planning, about 176k lines).
Add the legacy model picker, session browser, fake branch browser, Hermes-only
commands, duplicate helpers and stale tests, about 4–6k more. Count the
third-party dependencies removed with `cargo tree -e normal --prefix none | sort -u | wc -l`
before and after (not counted here).

**Not checked:** internals of `operant-providers`, `operant-tools`,
`operant-config`, `operant-channels` and `operant-runtime` (reachability only);
the web dashboard; the wasm plugin host; the ACP server; cron scheduler
concurrency on its own agent (D-6 still open); cost and pricing; Windows paths;
Holon's side; the sourcehound and aft binaries themselves; webhook endpoint
security beyond the O-items; kanban, curator and skill marketplace. I also did
not check the live-provider verdicts (plan 01 S1/S2, plan 04 Esc during
`sleep 20`, plan 05 3-turn rewind).

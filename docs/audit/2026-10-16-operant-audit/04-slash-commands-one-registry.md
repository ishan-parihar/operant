# 04 — One slash-command table; no dead or lying commands

**Size:** M. **Findings:** 11, 12, 19 in `01`. Do it after `03`, because `03`
changes several arms.

## How dispatch works today (three tables and a fallback)

1. **Typeahead, command palette and help** read `crate::commands::COMMAND_REGISTRY`
   (Hermes-port table, `crates/operant-cli/src/commands.rs:474-920`, 90
   non-gateway entries) via `tui/app/helpers.rs:17-23` (`tui_slash_command_data`,
   `help_overlay_entries`).
2. **The TUI registry** `REGISTERED_COMMANDS`
   (`tui/operant_app/app.rs:242-336`, 57 entries) feeds the `/help` "uncovered"
   list (`operant_ui/ui_overlays.rs:318`) and the "registered but not wired" catch-all
   (`tui_app.rs:399-409`).
3. **The dispatcher** is a 106-arm `match` in
   `tui/app/commands.rs:276-1648`. Its `_` arm falls back to
   `CommandRegistry::execute` (24 handlers, registered at
   `tui_app.rs` `register_tui_command_handlers` / `register_agent_flow_handlers`).
   If the name is in `COMMAND_REGISTRY` with no handler, the fallback prints
   "Command /X is not yet wired to a handler".

Measured on 0ee25739 (a script over the three sources, plus simulator checks):

**A. Suggested by autocomplete, but they print "not yet wired" (18):**
`history, title, handoff, snapshot, env, codex-runtime, profile, footer, toolsets,
cron, suggestions, blueprint, curator, kanban, platforms, paste, image, gquota`.
Seen: `/cron<enter>` → "Command /cron is not yet wired to a handler."

**B. Work, but never autocomplete or appear in `/help`'s registry list (42 in core
defs only + 16 in neither):** `background, billing, branch, browser, busy, copy,
credits, doctor, exit, export, goal, heapdump, indicator, init, journey, mem,
mouse, new, personality, pet, plugins, queue, redraw, reload-mcp, reload-skills,
replay, replay-diff, retry, rollback, setup, skill, skin, status, statusbar,
steer, stop, subgoal, tasks, undo, verbose, whoami, yolo` and `bundle, changes,
connect, cost, find, fresh, hooks, import-config, mcp, output-style, search,
settings, split, split-view, theme, vim`.

**C. TUI-only commands that never autocomplete, because typeahead reads the
*other* table:** `/rewind, /permissions, /effort, /thinking-display, /cls,
/keys, /hotkeys, /diff, /transcript, /changelog, /todos, /goals, /context,
/refresh-model-list, /stats, /terminal-setup, /triage, /review, /judge, /auth,
/info, /improve, /refactor, /fix, /compact, /plan, /fast, /save, /rename,
/quit, /login, /logout`. Seen: typing `/rew` and `/perm` shows no suggestion.

**D. Sent to the model as a prompt:**
- `/split`, `/split-view` → the arm recurses to `"splitview"`, which has no
  arm (`commands.rs:296-298`). Seen: the simulator sent `/split` to the live provider.
- `/mcp <anything>` → `handle_tui_command` returns `false` for `mcp` with args
  (`commands.rs:15-17`), and `mcp` is not in either table.

## Target design

**One table: `REGISTERED_COMMANDS`** in `tui/operant_app/app.rs`, extended
with `aliases: &'static [&'static str]` and `args_hint`. It is the single source
for typeahead, the palette, `/help` and the catch-all.
`COMMAND_REGISTRY` stays only for the **gateway** (`gateway_commands.rs`
reads it). Delete `tui_slash_command_data` and `help_overlay_entries`'s use of
it. Remove every non-gateway row from `COMMAND_REGISTRY` that has no gateway
handler.

**Every row has a handler.** The existing test
`no_registered_tui_command_is_unwired` (plan 02) must iterate the merged table
**including aliases** and call the dispatcher. The test fails if the result is
`false` or if the output contains "not yet wired". That makes A and D
impossible to reintroduce. Delete the "registered but not wired" runtime
catch-all (`tui_app.rs:392-409`) once the test enforces it, so a gap is a
build failure and not a runtime notice.

## Decision per command (apply in this order)

**Wire to the real CLI subcommand (the owner asked for subcommand parity).**
Simplest working rung: these subcommands already print text, so run them
in-process and print the output as a system message. **Don't re-implement
them in the TUI.** Pattern: the `/setup` arm's `pending_shell_command`
(`commands.rs:1566-1577`) suspends the TUI. For non-interactive output,
prefer a `CommandResult::Message` handler that calls the same function the
subcommand calls (`cmd_cron::list(...)` etc.) and returns its text.

| Slash | Runs | Notes |
|---|---|---|
| `/cron [list\|add\|rm …]` | `operant cron …` | |
| `/kanban …` | `operant kanban …` | |
| `/curator …` | `operant curator …` | |
| `/suggestions …` | `operant suggestions …` | |
| `/profile` | `operant profile show` | |
| `/checkpoints [list\|restore N]` | `operant checkpoints …` | replaces the lying `/rollback` arm (`commands.rs:1247-1254`); keep `/rollback` as an alias |
| `/pause`, `/unpause` | `operant pause` / `operant resume` | `operant resume` = e-stop resume, which collides with TUI `/resume` = sessions. Name the TUI one `/unpause`. |
| `/logs [n]` | `operant logs` tail | |
| `/org …` | `operant org …` views (roster, board, approvals) | read-only; O-31 deferred the TUI panels, so text output is the lazy version |
| `/mcp [list\|add\|rm …]` | bare `/mcp` → the panel; with args → `operant mcp …` | fixes D |
| `/memory` | memory-wire status, recall search (see `08`) | today it opens an AGENTS.md (user/project/local) file selector with help text "Toggle memory feature". Rename that selector to `/context-files` |

**Make honest or delete:**

| Command | Today | Do |
|---|---|---|
| `/branch` + Ctrl+B browser | Enter/n/d print "Switched/Created/Deleted branch" and do nothing (`key_handling.rs:739,750,761`); `/branch` opens an **empty** list (`commands.rs:489-492` passes `vec![]` and never sets `pending_branch_list`) | Enter → `agent.set_session_id(branch.id)` + display rebuild (`03` pattern). `n` → the rewind fork at the current point. `d` → `database().delete_session(id)` (check the name) with confirm. `/branch` sets `pending_branch_list = true`. Or delete the browser and point to `/resume`. Ask the owner, default wire. |
| `/split`, `/split-view` | sent to the model | delete the arm (no split view exists) |
| `/busy` | toggles **auto-compact** (`commands.rs:1127-1138`) | delete; registry text "Control what Enter does while busy" describes a feature that doesn't exist |
| `/indicator`, `/statusbar` | both toggle `terminal_progress_bar` | keep `/statusbar`, delete `/indicator` |
| `/browser` | says "operant uses Camofox" (purged 11C2) | delete (sourcehound is a tool, not a TUI surface) |
| `/mouse` | always says "enabled", even with `--no-mouse` | read the real flag (`TuiApp.no_mouse`, pass it into `App`) or delete |
| `/update` | help "Background update and auto reload"; prints a hint | help → "How to update (runs `operant update`)", or run it via `pending_shell_command` |
| `/reload` | help "Reload into newest available binary"; reloads settings | help → "Reload settings from disk" |
| `/billing`, `/credits`, `/gquota`, `/codex-runtime` | Nous/Gemini/Codex leftovers | delete |
| `/pet`, `/heapdump`, `/mem` | easter egg / a duplicate of `/stats` | delete |
| `/replay`, `/replay-diff` | "planned" stub | delete (YAGNI) |
| `/background` | prints a shell line | delete; `/bg` is a jcode-style feature for later |
| `/history`, `/toolsets`, `/env`, `/title`, `/handoff`, `/snapshot`, `/footer`, `/blueprint`, `/platforms`, `/paste`, `/image` | not wired | `/title` = alias of `/rename`; `/platforms` → `operant gateway status`; `/image <path>` and `/paste` → wire to the existing image-attach path if it exists (`tui/image_paste.rs`), else delete; the rest delete |
| `/memory` help | "Toggle memory feature" | see table above |
| `/output-style`, `/verbose` | cycles a string nothing renders? | check with `rtk grep output_style`; delete if no renderer reads it |
| `/doctor` | arm returns `false`, then the registry handler | fine, but list it in the table |

**Stub flows in the run loop** (`tui_app.rs:297-314`): MCP panel auth and device
auth both print "not yet wired — restart operant". Either finish device-flow
polling for `github-copilot` / `openai-codex` (only if the owner uses them; ask),
or remove those providers from the connect dialog so the dead end is
unreachable. Default: remove.

## Tests first

1. Extend `no_registered_tui_command_is_unwired` as described. It is RED today
   for the 18 in A plus `/split`.
2. A typeahead test: typing `/rew` → suggestions contain `/rewind`. RED today.
3. A corpus scenario `slash-typeahead.scenario.json`: `/rew` screen golden.
4. One test per rewired subcommand: `/cron list` with an empty cron DB → the
   output contains the same header `operant cron list` prints.

## Acceptance

- `python3 crates/operant-cli/tests/tui_scenarios/command_sweep.py` (with a
  **mock** agent, see `01` finding 16) reports 0 fall-throughs and 0
  "not yet wired".
- `/help` lists exactly the merged table and nothing else.
- README's command list is regenerated from the table (one source).
- CHANGELOG lists every deleted command.

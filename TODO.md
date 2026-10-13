# Operant task ledger

Autonomous mode (`operant autonomous`, `operant run --autonomous`) reads this
file (`[autonomous] todo_path`): it works the `## Pending` list, runs
`[autonomous] test_command`, commits to `[autonomous] git_branch`, and records
progress in `autonomous-status.toml`. Only the `## Implemented` and
`## Pending` sections are parsed. Anything that needs an owner decision belongs
in `## Review`, which autonomous mode never touches.

## Implemented

- TUI rows render once while tools run (iter-797)
- Synchronous, viewport-independent drag copy (iter-797, iter-801)
- Reasoning display: jcode Current semantics by default (iter-797)
- Streams outlive `timeout_secs`; stream drops always earn retries (iter-799)
- Large pastes reach the model, not their placeholder (iter-800)
- Copy is clean document text: logical lines, no gutters, no tool rows (iter-801)
- Copy carries no chrome: system cards rejoin; code, quote, table as text (iter-802)
- Tool-loop guardrails warn the model and stop the turn cleanly (iter-805)
- Selection highlights only the text a copy carries (iter-806)
- oh-my-pi shortcuts, conflict-checked; `[tui] external_editor` (iter-807)
- Essential commands wired (`/compact`, `/cancel`, `/transcript`, `/changelog`, `/refresh-model-list`, `/auth`, `/goals`); jcode-only commands removed (iter-808)

## Pending

- Rebuild `/settings` over `operant.toml` (toml_edit, comments kept), tabs per config section, live-apply where supported; retire the legacy settings.json screen
- `[keybindings]` user overrides in `operant.toml`; `/keys` shows effective bindings and conflicts
- `▸ thought for Xs` collapsed row when a Current-mode reasoning trace hides
- Unify the gateway's `agent.show_reasoning` with the TUI reasoning display mode
- BUGS.md K-4: gate the `operant_runtime` observer call so `--no-default-features` builds
- BUGS.md K-3: key the terminal-scroll watermark on message boundaries, not rows

## Review

Owner decisions — not autonomous work:

- Commands kept registered but not wired: `/commit`, `/commit-and-push`, `/commit-push-release`, `/cut-release`, `/git`, `/test`, `/swarm`, `/subagent`, `/subagent-model`, `/ssh`, `/remote` — implement or remove each
- Reasoning effort: the effort picker is display-only (no provider request field); wire `reasoning_effort` (some providers reject it) or remove the picker
- Esc / `/cancel` clears ALL tool rows of the session (`tool_use_blocks.clear()`), not just the cancelled turn's
- Context management (magic-context review): agent-driven reduction (`ctx_reduce`) and a background historian instead of the synchronous summarize rung — larger core changes
- BUGS.md R42-8: two live Telegram bot tokens unredacted in gateway.log

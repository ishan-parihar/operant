# TUI lifecycle coverage — operant vs codex / oh-my-pi

**Goal from the operator:** the TUI must be the *entire* interface for the entire
lifecycle of running this agentic harness.

## operant's TUI surface today (repo-verified, 50 modules under `operant-cli/src/tui`)

`context_viz` · `background_tasks` · `agents_view` (multi-agent roster) · `bypass_permissions_dialog` ·
`dialog_select` · `diff_viewer` (parse + render) · `plugins_hub` · `mcp_view` · `skills_view` ·
`memory_file_selector` · `settings_screen` · `theme_screen` · `usage_overlay` · `stats_dialog` ·
`substitution_cost` · `journey_view` · `rewind_selector` · `session_switch` · `session_hub` ·
`export_dialog` · `permissions_picker` · `keybindings` (defaults + registry) · `free_mode_dialog` ·
`custom_provider_dialog` · `import_config_dialog` + ~24 more.

## Panes that EXIST vs the reference TUIs

| Lifecycle surface | operant | codex / oh-my-pi | Verdict |
|---|---|---|---|
| Context / budget meter | `context_viz` | same | **parity** |
| Tool trace | `ui_tools` + `background_tasks` | same | **parity** |
| Session fork / branch / export | `session_hub`, `export_dialog` | same | **parity** |
| Approval queue | `permissions_picker`, bypass dialog | same | **parity** |
| Diff review / rewind | `diff_viewer`, `rewind_selector` | `app_backtrack`, `backtrack_summary` | **parity** |
| MCP / plugins / skills | `mcp_view`, `plugins_hub`, `skills_view` | `mcp_tools.rs` | **parity** |
| Keybindings | `keybindings/registry.rs` | `slash_command.rs` (67 cmds) | **parity** |
| **Cron / scheduler pane** | **`/cron` command only, NO pane** | codex `Daemon` slash + `periodic.md` | **GAP → G-010** |
| **Channel parity (all 72 TUI commands reachable from Telegram/Discord/webhook)** | absent | n/a | **GAP → G-009 (canonical RT-179)** |

## The two actions that close the lifecycle

1. **G-010** — add a cron overlay to `App::overlay_flags()`. The wheel-in-panel bug
   (RT-251, closed iter-1802) was the same class: a hand-written list of overlays
   drifted from the catalogue. Register the cron pane in the catalogue, not in a list.
2. **G-009 / RT-179** — diff `RegisteredCommand` (TUI) against `gateway_commands.rs`
   and register the missing surface so the TUI is genuinely the *whole* interface.

## What this costs the operator today

Scheduling is a **CLI-only** capability: `operant cron …`. A user sitting in the TUI must
shell out to see or change a job, while every peer surface (memory, skills, MCP, diff,
sessions, agents) is one keystroke. That is the single largest lifecycle asymmetry left.

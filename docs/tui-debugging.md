# TUI Debugging & Headless Simulation

Operant's TUI has a debugging layer that lets you drive the interface
headlessly, observe its state and rendered output, and assert on both — the
foundation for autonomous debug/refactor loops and CI regression tests.

## `operant tui debug` — read-only overlay inspection

Each subcommand runs the same data-loading path the matching TUI overlay uses,
and prints plain text/JSON (it does **not** render the TUI):

```
operant tui debug skills          # /skills overlay data
operant tui debug plugins         # /plugins overlay data
operant tui debug journey         # /journey overlay data
operant tui debug mcp             # /mcp overlay data
operant tui debug stats           # /stats overlay data
operant tui debug context         # /context overlay data
operant tui debug sessions        # /resume overlay data
operant tui debug banner          # ASCII banner
operant tui debug slash-commands  # every intercepted slash command
operant tui debug state           # persistent state (settings.json + masked auth.json)
operant tui debug cost            # cost / token / turn summary
```

Exit codes: `0` success, `1` data-load failure, `2` argument error.

## `operant tui debug simulate` — headless simulator

Drives the **real** `App::run` loop against a `ratatui` `TestBackend` (no
terminal, no drift from production), replaying a key sequence and optionally
injecting mock agent events, then asserts on the final state and rendered
screen.

```
operant tui debug simulate --keys <sequence> [flags]
```

| Flag | Purpose |
|------|---------|
| `--keys <seq>` | Keystroke sequence to replay (required). |
| `--assert <clauses>` | State assertions against `App::debug_snapshot()` (comma-separated). |
| `--assert-screen <clauses>` | Screen-content assertions (comma-separated). |
| `--dump-screen <path>` | Write the final rendered screen (one text row per line). |
| `--output <path>` | Write the event log as pretty JSON. |
| `--agent-script <path>` | Inject mock agent events from a JSON file instead of a real network agent (deterministic, offline). |
| `--size <WxH>` | Terminal size (default `120x40`). Reproduce layout/wrapping bugs. |
| `--max-frames <N>` | Frame cap before force-exit (default `100000`). Guards against runaway streams. |

Exit is nonzero if the event log contains an error, an assertion fails, or a
clause is malformed.

### Key sequence vocabulary

Literal characters are typed as-is. Escapes: `\n` (Enter), `\t` (Tab), `\\`.
Named keys (case-insensitive tokens):

```
<enter> <esc> <tab> <shift+tab>
<up> <down> <left> <right> <backspace>
<ctrl+a> <ctrl+c> <ctrl+t> <ctrl+r>
```

Unknown `<...>` tokens are typed literally.

Example: `--keys "/model<enter><down><down><enter>"`.

### State assertions (`--assert`)

Each clause is `path OP value`, comma-separated. `OP` is `==`, `!=`, or
`contains`. `path` is a dot-path into the state snapshot:

- Top-level: `should_exit`, `is_streaming`, `is_simulating`, `plan_mode`,
  `show_help`, `show_reasoning`, `fast_mode`, `messages` (count), `model`,
  `provider`, `focus`, `token_count`, `any_modal_open`, `status_message`.
- Overlays: `overlays.<name>` — e.g. `overlays.model_picker`,
  `overlays.help_overlay`, `overlays.settings_screen`, `overlays.mcp_view`,
  `overlays.permission_request`, … (all 35 dialog/overlay visibilities).
- Legacy alias: `<name>.visible` maps to `overlays.<name>`.

Values match booleans, numbers, and strings. Examples:

```
--assert "overlays.model_picker == true,is_streaming == false"
--assert "messages == 3"
--assert "model contains gpt"
```

Unknown paths fail loudly.

### Screen assertions (`--assert-screen`)

Each clause is `contains:TEXT` or `not-contains:TEXT`, matched against the full
rendered screen text:

```
--assert-screen "contains:Shortcuts,not-contains:Error"
```

### Mock agent script (`--agent-script`)

A JSON array of tagged events injected through the real `agent_event_rx`
channel (no network). The run stays alive until the events drain, then exits.

```json
[
  {"type": "thinking",  "content": "reasoning..."},
  {"type": "content",   "text": "Hello "},
  {"type": "content",   "text": "world"},
  {"type": "tool_start",    "id": "t1", "name": "read_file", "arguments": "{}"},
  {"type": "tool_complete", "id": "t1", "name": "read_file", "output": "..."},
  {"type": "usage",     "input_tokens": 12, "output_tokens": 8},
  {"type": "done",      "text": "Hello world"}
]
```

Event types: `thinking`, `reasoning`, `content`, `tool_start`,
`tool_complete`, `tool_error`, `usage`, `done`, `error`.

### Example: verify a dialog opens and renders

```bash
operant tui debug simulate \
  --keys "/help<enter>" \
  --assert "overlays.help_overlay == true,any_modal_open == true" \
  --assert-screen "contains:Shortcuts" \
  --dump-screen /tmp/help.txt
```

## Runtime event bus + F12 overlay

When `OPERANT_TUI_DEBUG=1` (or F12 in an interactive session), the TUI records
a ring buffer of typed events (`TuiEvent`) — keys, agent events, slash
commands, permission/user-question/model-fetch/session events, frames, and
errors. `--output` dumps this log as JSON; each entry has a `kind` tag and a
timestamp. Set `OPERANT_TUI_EVENT_LOG=<path>` to dump the ring on exit.

## Scenario regression tests

The dialog open/close regression pack lives in
`crates/operant-cli/src/tui/app.rs` (`test_dialog_open_close_scenarios`). It
drives each slash-openable overlay headlessly and asserts it opens and closes,
guarding the dialog-unification refactor.

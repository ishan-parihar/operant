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
| `--baseline <path>` | Diff the rendered screen against a committed golden file. See [Baseline diffing](#baseline-diffing---baseline). |
| `--accept-baseline` | Rewrite `--baseline` in place from the current render. Explicit; never implicit. |
| `--capture-frames <list>` | Capture per-frame text (comma-separated 0-based indices). See [Per-frame capture](#per-frame-capture---capture-frames). |
| `--capture-dir <path>` | Output directory for `--capture-frames` files (default `tui-frames`). |

Exit is nonzero if the event log contains an error, an assertion fails, a
clause is malformed, or `--baseline` detects drift.

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

### Baseline diffing (`--baseline`)

`--dump-screen` records a snapshot but nothing compares it, so drift between
runs is invisible. `--baseline <path>` closes that gap: the rendered screen is
diffed against a committed golden file and the process **exits non-zero** on
drift.

```bash
# 1. First run bootstraps the golden (file did not exist → created, exit 0).
operant tui debug simulate --keys "/help<enter>" \
  --baseline tests/tui_scenarios/help.golden

# 2. Later runs gate: match → "✓ baseline matches …", exit 0.
operant tui debug simulate --keys "/help<enter>" \
  --baseline tests/tui_scenarios/help.golden

# 3. Deliberate change → accept it explicitly.
operant tui debug simulate --keys "/help<enter>" \
  --baseline tests/tui_scenarios/help.golden --accept-baseline
```

Behaviour, precisely:

| Situation | Behaviour | Exit |
|-----------|-----------|------|
| `--accept-baseline` passed | Rewrites the golden from this render, prints the path + size. | 0 |
| Golden file missing | Bootstraps it, prints `Baseline created at <path> … Commit it; the next run will diff against it`. | 0 |
| Golden matches | Prints `✓ baseline matches <path>`. | 0 |
| Golden differs | Prints a **unified diff to stderr**, then errors. | non-zero |

The diff is line-based with `@@` hunk headers and 3 lines of context (`diff -u`
shape), rendered from the `similar` crate. Only the changed hunks are printed
— the whole screen is never dumped.

#### Baseline file format

Deliberately the dullest format that works: **one trimmed screen row per line,
newline-terminated**, i.e. byte-identical to `--dump-screen` output plus a final
newline. No header, no metadata, no escape sequences (TestBackend text capture
drops fg/bg/modifier — style comparison lives in `scripts/tui-capture.sh`, see
below).

Both sides of the comparison are canonicalised before comparing: CRLF→LF,
per-line trailing-whitespace strip, exactly one trailing newline. An editor
that pads lines or rewrites line endings therefore cannot manufacture a
phantom diff.

### Per-frame capture (`--capture-frames`)

`--dump-screen` reaches exactly one screen: the final state. Intermediate
states — streaming progression, animation frames, mid-sequence scroll
position — were unreachable. `--capture-frames` reaches them.

```bash
operant tui debug simulate --keys "/help<enter>" \
  --capture-frames 0,1,2 \
  --capture-dir /tmp/frames
```

- **Indices are 0-based over painted frames.** `0` is the first frame the run
  loop draws, before any key has been handled.
- **Naming**: `<capture-dir>/frame-<NNNN>.txt`, zero-padded to 4 digits so a
  lexical sort equals a numeric sort. `--capture-dir` defaults to `tui-frames`
  in the working directory and is created if missing.
- The payload is exactly the baseline file format (trimmed rows), so a
  captured frame can be compared the same way a final screen is.
- Requesting a frame the run never painted is an **error**, not a no-op: the
  harness prints which indices were captured and which were missing, and exits
  non-zero. Silently writing fewer files than requested would make a broken
  scenario look green.

#### Why capture happens inside the loop, and what that costs

`App::run`'s exit check fires at the *top* of the loop, before the draw, so the
final state is never painted by the loop — `run_headless` does one extra
`terminal.draw` after the loop purely to make the last state visible. That
extra draw is the only post-exit paint, and by then every intermediate buffer
is gone. A finished buffer **cannot be rewound**: re-rendering it after exit
yields the final screen, N times over, which is why the flag is not implemented
that way.

So capture is armed into the debug hub and fires from the same place the OSC-8
URL scan already uses — the *just-rendered* `CompletedFrame`, immediately after
`record_frame`:

```rust
let completed = terminal.draw(|f| render::render_app(f, self))?;
self.debug_hub.record_frame(render_ms);
self.debug_hub.capture_frame(completed.buffer);   // inert unless armed
```

Cost of this choice, honestly:

- One added call line in the production run loop. Nothing is restructured;
  the loop's shape, exit conditions, and return type are untouched.
- `capture_frame` is a single `Mutex::lock` + `Option` check per frame. When
  capture is not armed (every real session, and every test) it is a locked
  read of `None` and an early return.
- The payload is **not** attached to the `TuiEvent::FrameRendered` event, so
  the 1,000-entry event ring cannot balloon and the `--output` JSON shape is
  unchanged. Capture outcome is reported on stdout and through the return value
  instead.

The rejected alternative (replay the scenario once per requested frame with a
truncated key sequence) would need no production change at all, but it is
multiplicative in frames and cannot see frames produced by streaming timing
rather than by key count.

## Real-terminal capture (`scripts/tui-capture.sh`)

`TestBackend` deliberately bypasses the interactive-only paths: raw mode,
alt-screen, mouse capture, real terminal-mode reassertion on
`Event::FocusGained`, OSC-8 hyperlinks, and pinned terminal images.
`scripts/tui-capture.sh` covers those with a real pty:

```bash
scripts/tui-capture.sh --help

# Capture the landing screen at a pinned size.
scripts/tui-capture.sh --out baseline/chat.landing.txt

# Drive it: type /help, submit, wait for the overlay, capture, and gate.
scripts/tui-capture.sh \
  --keys '/help' --enter \
  --wait-text 'Shortcuts' \
  --out baseline/chat.help.txt \
  --assert-contains 'Shortcuts'
```

It starts `operant chat` in a detached tmux session at a fixed size (default
`120x40`, `--size WxH`), optionally `send-keys` a literal key sequence, waits
deterministically, then writes `tmux capture-pane -p -e` — `-e` preserves ANSI
escapes, which is what makes a *style* comparison possible (the `TestBackend`
text capture drops fg/bg/modifier).

Determinism controls, so a developer's local theme can never leak into a
committed baseline:

- `tmux -f /dev/null` — your `~/.tmux.conf` cannot alter the pane.
- Pane size pinned via `new-session -x/-y`.
- `HOME` is pointed at a `mktemp -d` holding a fixed minimal `operant.toml`
  (explicit theme, unroutable endpoint, no API key), and `--config` points at
  it. `--config-mode inherit` opts out; `--keep-home` retains the temp dir.
- `TERM`/`COLORTERM` pinned; `OPERANT_TUI_DEBUG`, `OPERANT_TUI_EVENT_LOG` and
  `NO_COLOR` are unset for the child.

Waiting is never a blind sleep: `--wait-text TEXT` polls the pane until the
substring appears; the default polls until two consecutive dumps are identical
and non-empty. `--timeout SECONDS` (default 30) bounds both.

The tmux session is killed by a `trap` on `EXIT`, `INT` and `TERM`, so a
failure never leaves a stray session behind. Exit codes: `0` captured and all
`--assert-contains` held, `1` capture or assertion failure, `2` usage error.

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

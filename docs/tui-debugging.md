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
| `--dump-style <path>` | Write the per-cell **style** grid (fg/bg/mods per cell). Catches colour regressions the text dump cannot see. See [Style goldens](#style-goldens---style-baseline). |
| `--style-baseline <path>` | Gate the style grid against a committed golden, naming the first differing `(row, col)`. Shares `--accept-baseline` with `--baseline`. |
| `--bypass-permissions` | Start with the bypass-permissions confirmation dialog already open, the way the real CLI flag does. It is a flag rather than a script event because `TuiApp::enter` raises that dialog before any key or event is processed. |

Exit is nonzero if the event log contains an error, an assertion fails, a
clause is malformed, or `--baseline` detects drift.

### Key sequence vocabulary

Literal characters are typed as-is. Escapes: `\n` (Enter), `\t` (Tab), `\\`.
All tokens are case-insensitive.

Named keys:

```
<enter> <esc> <escape> <tab> <shift+tab>
<up> <down> <left> <right> <backspace> <bs>
<ctrl+a> <ctrl+c> <ctrl+t> <ctrl+r>
```

Function keys: `<f1>` … `<f12>`. Digits only, so `<f0>` and `<f13>` are
deliberately *not* tokens.

Modifier chords — any `+`-joined combination of `ctrl`/`control`,
`alt`/`meta`/`option` and `shift`, ending in a single character, with at least
one modifier:

```
<ctrl+k> <ctrl+j> <alt+v> <ctrl+shift+m> <ctrl+alt+p>
```

**Unknown `<...>` tokens are typed literally.** This is intentional and load-
bearing: a scenario that names a binding the parser does not know shows up as
typed text on screen, never as a silent no-op. So `<f13>` types the six
characters `<f13>` rather than pressing F13 — if you see angle brackets in a
render, you have a typo, not a rendering bug.

Not tokens today: `<home>`, `<end>`, `<pageup>`, `<pagedown>`, `<delete>`,
`<insert>`, `<space>`, a bare `<shift+<letter>>` with no ctrl/alt, and
multi-character `<ctrl+key>` forms like `<ctrl+backspace>`.

Examples: `--keys "/model<enter><down><down><enter>"`, `--keys "<f8>"`,
`--keys "/voice<enter><ctrl+shift+m>"`.

### State assertions (`--assert`)

Each clause is `path OP value`, comma-separated. `OP` is `==`, `!=`, or
`contains`. `path` is a dot-path into the state snapshot:

- Top-level: `should_exit`, `is_streaming`, `is_simulating`, `plan_mode`,
  `show_help`, `show_reasoning`, `fast_mode`, `messages` (count), `model`,
  `provider`, `focus`, `token_count`, `any_modal_open`, `status_message`.
- Overlays: `overlays.<name>` — e.g. `overlays.model_picker`,
  `overlays.help_overlay`, `overlays.settings_screen`, `overlays.mcp_view`,
  `overlays.permission_request`, … (all 34 entries of `App::overlay_flags()`).
- Non-modal panels: `usage_overlay`, `debug_overlay` (top level, **not** under
  `overlays`). Both render below every modal without gating input, and
  `any_modal_open()` is derived from `overlay_flags()`, so listing them there
  would make `any_modal_open` claim a modal is up when none is.
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

**A needle may not contain a comma** — `,` is the clause separator, so
`contains:Yes, allow once` silently becomes the two clauses `contains:Yes` and
`allow once`. Stop the needle at the last comma-free word.

Prefer asserting the surface's own chrome over a state proxy. `any_modal_open
== false` says nothing about whether a surface rendered; a
`contains:⚠ Error` on the error modal does.

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

Agent events (pushed onto `agent_event_rx`): `thinking`, `reasoning`,
`content`, `tool_start`, `tool_complete`, `tool_error`, `usage`, `done`,
`error`.

Four more variants do **not** map to an `AgentEvent` — the TUI receives them on
channels, so `run_headless` dispatches them onto the same plumbing the live
agent uses:

| Type | Goes to | Raises |
|------|---------|--------|
| `user` | `app.messages` | The transcript turn `build_transcript_turns` anchors on. |
| `permission_request` | `permission_tx` | `overlays.permission_request` |
| `user_question` | the user-question channel | `overlays.ask_user_dialog` |
| `background_task` | operant-core's `async_delegation` registry | The background-task rows |

```json
[
  {"type": "user",              "text": "Read the repo header."},
  {"type": "tool_start",        "id": "t1", "name": "read_file"},
  {"type": "tool_complete",     "id": "t1", "name": "read_file", "output": "# operant"},
  {"type": "done",              "text": "done"}
]
```

Three rules the corpus learned by getting them wrong:

1. **A script with `tool_*` events must lead with a `user` event.** Tool blocks
   are partitioned onto a transcript turn, and a turn only exists once
   `build_transcript_turns` has seen a `Role::User` message. Without it the tool
   state populates and nothing paints.
2. **Never put `done` after `error`.** `handle_agent_event` auto-dismisses error
   notifications on the next `content`/`thinking`/`reasoning`/`done`, and the
   real agent never emits `Done` after `Error` (both `AgentEvent::Error` sites
   in `operant-core/src/agent/run.rs` return `Err` immediately). An
   error-then-done script swallows the error modal before it is ever painted.
3. **`background_task.status`** is `pending` (default), `completed` or
   `failed`. The rendered row is derived from the real registry record, never
   injected into TUI state.

### Host-dependent startup state

On a host with a working audio device, `TuiApp::enter` calls
`voice_mode_notice.show_if_available(..)`, so `voice_mode_notice` is visible
from frame 0 and `any_modal_open` is `true` before any key is pressed. A
scenario that needs a clean start must lead with `/voice<enter>`, which
replaces the notice with a fresh invisible state and is idempotent on a silent
host.

`<esc>` also dismisses the notice, **except** when an `Error` notification is
current: Esc then hits `dismiss_error_notifications()` and takes the modal down
with it. The `error-modal` scenario therefore sends no keys at all and does not
assert `any_modal_open`.

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
newline. No header, no metadata, no escape sequences.

The text dump drops fg/bg/modifier, so it is blind to colour. That is what
`--style-baseline` is for — see [Style goldens](#style-goldens---style-baseline)
below. `scripts/tui-capture.sh` remains the only way to see what a **real
terminal** does with those cells (it captures with ANSI intact via
`tmux capture-pane -e`); it is complementary, not a substitute.

Both sides of the comparison are canonicalised before comparing: CRLF→LF,
per-line trailing-whitespace strip, exactly one trailing newline. An editor
that pads lines or rewrites line endings therefore cannot manufacture a
phantom diff.

### Style goldens (`--style-baseline`)

Two surfaces can render identical text with inverted palettes, and the text
golden cannot tell them apart. `--dump-style <path>` writes a per-cell grid of
`<fg>/<bg>/<mods>@<symbol>` tokens that can; `--style-baseline <path>` gates it
the same way `--baseline` gates text, and names the first differing cell:

```
DRIFT  help-overlay: Rendered style grid differs from baseline
       ".../help-overlay.style.txt" at (row 0, col 42):
         expected rgb:CC9B1F/rgb:0A0A0E/-@X, got rgb:CC9B1F/rgb:0A0A0E/-@_
```

Both flags can be passed together; `--accept-baseline` then rewrites whichever
of the two goldens were supplied.

The format is fixed by `tests/tui_scenarios/schema.json` → `baseline_formats.style`
and is a **verbatim projection of the buffer** — the writer normalises nothing,
because post-processing would hide exactly the regressions the file exists to
catch. Header line `operant-style-v1 <width>x<height>`, then exactly `height`
grid lines of exactly `width` space-separated tokens. Wide (CJK/emoji) glyphs are
written once in the cell where they start, with continuation cells as
`~/<fg>/<bg>/-`. Space becomes `_` so a token can never contain whitespace;
other control characters become `?`.

> **Portability.** A style golden is only meaningful if every colour in it is
> encoded at the *same* colour depth. operant has two depth pipelines —
> `color_depth::detect()` (honours `OPERANT_COLOR_DEPTH`) and the vendored
> `style::color::color_capability()` (does not). If they disagree, a frame comes
> out hybrid — mostly `rgb:` with one run of `indexed:` — and the golden silently
> records the generating terminal rather than the render. Both are now pinned to
> truecolor; `verify` fails loudly if that ever stops being true. See
> `color_depth.rs`.

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

## The visual-regression scenario corpus

`crates/operant-cli/tests/tui_scenarios/` is the headless visual-regression
gate for the TUI's render ladder. One `*.scenario.json` per renderable
surface, executed against the **real binary** via `tui debug simulate`.

```
crates/operant-cli/tests/tui_scenarios/
├── *.scenario.json        one per surface, 50 files
├── schema.json            the field/key/assertion contract
├── excluded.json          surfaces tracked but NOT executed (see below)
├── agent_scripts/         mock-agent fixtures referenced by `agent_script`
├── baselines/             intentionally EMPTY — goldens land on first run
├── _validate.py           static checker; encodes the harness ground truth
└── _run.py                executes every scenario + variant
```

`baselines/` is empty on purpose. Until a baseline exists, a scenario is gated
on its `--assert` / `--assert-screen` clauses alone; the first
`--baseline <path>` run bootstraps the golden and every later run diffs against
it, so the gate can start failing on visual drift the moment goldens land.

```bash
cd crates/operant-cli/tests/tui_scenarios
python3 _validate.py     # static: tokens, assert paths, needles, completeness
python3 _run.py          # dynamic: every scenario + variant against the binary
python3 _run.py <name>   # one scenario, by name
```

Set `OPERANT_BIN` to point `_run.py` at a specific binary. Use a throwaway
config so nothing depends on the developer's real `~/.operant`:

```toml
[client]
base_url = "http://127.0.0.1:9/v1"   # unroutable: any real network call fails
[agent]
model = "offline-capture"
```

### Writing a scenario that is worth having

- **Assert the surface's own text, not a proxy.** `any_modal_open == false`
  proves nothing about whether anything rendered.
- **Read the actual screen before writing a needle.** Several needles in the
  corpus were mis-cased (`Files` vs `0 files`, `Usage` vs ` F8 · usage `) and
  could never have matched. `--dump-screen` is the ground truth.
- **Never add a comma to a needle.** `,` is the clause separator.
- **Pin the default, not just the presence.** The permission dialog's
  `contains:► [y] Yes` exists so a flip from Allow to Deny shows up as a text
  diff.
- **A `not-contains:` clause is often the strongest one.** The error modal's
  `not-contains:Response complete` is what catches the loss of its early
  return, because that toast is drawn by the code path the return skips.

### `excluded.json` — surfaces the corpus does not execute

A scenario file that cannot pass is worse than no scenario: it trains people to
ignore red, and it silently removes the surface from the regression net. So a
surface that cannot be reached does not live in a `*.scenario.json` at all. It
lives in `excluded.json`, which records for each one the surface, the baseline
filenames a future run should create, one `reason`, and an `unblock_when`.

`_run.py` prints the excluded count on every run so the net can never quietly
shrink, and `_validate.py` cross-checks `excluded.json` against its own
`SURFACES` ledger — an entry that drifts out of sync with the render ladder is
a validation error, and a scenario file that reappears for a parked surface is
too.

### `deterministic: false` — surfaces that execute but cannot be goldened

`excluded.json` is for surfaces operant cannot *reach*. A different problem is a
surface that runs perfectly well yet renders something a committed file cannot
capture: a wall clock, a timestamped id, or the checkout's own absolute path.
Those **keep** their `*.scenario.json` — the assertions and `assert_screen`
checks still run and still catch logic regressions — and opt out of golden
comparison with two extra fields:

```json
"deterministic": false,
"nondeterminism": {
  "reason": "…why two renders of the same scenario disagree…",
  "varying_region": "rows 20-21, the Project/Local value columns",
  "unblock_when": "what would make it capturable"
}
```

`_run.py verify` reports these separately as `assertions-only (nondeterministic)`
so the count is always visible and the net cannot shrink quietly.

Current entries (5): `background-task-rows`, `debug-overlay`, `tool-block`
(a real clock is read in each), `diff-viewer-git` (renders the working tree's own
`git diff`), `memory-file-selector` (renders the checkout's absolute path).

> **The trap worth naming.** `memory-file-selector` passes a same-machine
> two-run determinism proof and is still uncapturable, because a path is stable
> within one checkout and different in every other. *Determinism and portability
> are different properties, and a same-machine proof only establishes the
> first.* When judging a candidate golden, ask what would differ on someone
> else's machine, not merely what differs between two runs here.

Current `excluded.json` entries (3 of 53 surfaces):

| Surface | Why |
|---------|-----|
| `mcp-approval` | **Product gap.** `McpApprovalDialogState::show` (`tui/dialogs/mcp_approval.rs:88`) has no call site anywhere in the workspace and is `#[allow(dead_code)]`. The dialog is fully built — render, key handling, Z-order branch, `dialog_priority` entry, `overlay_flags` entry — but nothing in operant ever opens it, so there is no real event sequence a scenario could reproduce. |
| `mcp-approval-dialog` | The same product gap, kept as a separate entry so the 1:1 surface mapping stays complete: `render_app` has its own `if app.mcp_approval.visible` branch and this is that branch. |
| `selection-highlight` | Two independent blockers. `apply_selection_highlight` returns unless both `selection_anchor` and `selection_focus` are set, and the only writer is the mouse handler — `App::run` drives a `Vec<KeyEvent>`, so this needs a new mouse source in the event pump. But even with that, the pass is style-only by design: the text is unchanged, so no `--assert` or `--assert-screen` clause can detect it. Only the style baseline can, and that is out of scope while `baselines/` is empty. |

`reachability: "blocked"` on a scenario file is a *temporary* marker for
something being unblocked in the current iteration — the runner prints its
`blocked_by` and counts it as SKIPPED. The corpus currently has zero blocked
entries; `_validate.py` warns if any appear.

# 10 — Shortcut and overlay coherence: one source of truth, no advertised lies

## Symptom (user, 2026-10-14)

"There are several other dissonances in the shortcuts and what they do, and in the
overlays that pop up when certain commands are pressed."

## Evidence

### A. The help overlay's keyboard section is jcode's, hand-written

`tui/operant_ui/ui_overlays.rs:388-560` (`Navigation`, `Diagrams & Diffs`,
`Input & Editing`) advertises chords that do something else, or nothing, in operant:

| Advertised | Help says | Operant actually does (`app/key_handling.rs`) |
|---|---|---|
| Ctrl+A | copy visible viewport | opens the model picker (`:1242`) |
| Ctrl+K | delete to end of input / jump to previous prompt | opens the command palette (`:1323`) |
| Ctrl+J | jump to next prompt | inserts a newline |
| Ctrl+P | toggle auto-poke | global search (`:1256`) |
| Ctrl+T / Ctrl+Tab | toggle queue mode | copy mode (`:1433`) |
| Ctrl+X | cut input line | recall queued message (`:1427`) |
| Ctrl+B / Ctrl+F | word left / right | Ctrl+B opens an **empty** session-branch browser (`:1266`); Ctrl+F is word right |
| Shift+Tab | cycle favorited models | cycles permission mode, reaching **Bypass** in 2 presses (`:1542-1558`) |
| Ctrl+O | set default model (in picker) | expand the latest tool output (globally) |
| Alt+G | cycle diff mode | scroll bookmark (iter-807) |
| Up / Down | scroll history | prompt history |
| Alt+A, Alt+S, Alt+X, Alt+T, Alt+Shift+I, Ctrl+1..9, Ctrl+H/L focus, `[ ]`, `+ -` diagram keys | jcode features | unbound or nonexistent |

### B. The tips widget advertises jcode features

`operant_app/info_widget/info_widget_tips.rs:15-26`: "Ctrl+G to bookmark", which is now
Alt+G; Ctrl+J/K prompt jumps; swarms; ambient mode with emailed summaries; Alt+B to
background a tool; "Shift+Tab cycles favorited models". None of these is true in
operant.

### C. Overlays and commands that pop up the wrong thing

From a headless sweep of all registered commands on iter-811:

- `/keys` opens **scrolled to the bottom** (`↑172`, on "Indent selection VimIndent"). It
  is a system message containing all ~170 registry rows, ~120 of them vim bindings.
  `/hotkeys` is the same catalogue (`app/commands.rs:1060-1073`).
- `/keybindings` opens `keybindings.json` in an external editor. **Nothing reads that
  file** (`load_keybindings_from_config` is a stub that returns the defaults), and on
  failure it calls `eprintln!`, writing onto the TUI screen (`commands.rs:1047-1056`).
- `/log` opens the **Connect a provider** dialog.
- `/info` prints its line **twice** (as a status notice and as a system message).
- `/usage` says "use /stats". It should simply be `/stats`.
- `/permissions` isn't a command, so it falls through to the model as a prompt.
- `/btw`, `/voice`, `/fork`, `/restart`, `/observe`: dead (removed by plan 02).
- `/config` is the legacy settings screen ("Max Tokens" edits `context_window`; it writes
  `settings.json`). The rebuild over `operant.toml` is a TODO.md Pending item; out of scope
  here, but don't advertise it as complete.

### D. Safety

Shift+Tab silently arms *Bypass permissions* in two keypresses while composing.

## Design

### 1. The registry is the only source of shortcut text

`tui/keybindings/defaults.rs` already holds every binding with its context and
description (iter-807 added the composer chords and the `the_default_keymap_has_no_conflicts`
test). Extend each `KeyBinding` with:
- `group: HelpGroup` (Session, Composer, Transcript, Display, Overlays, Vim);
- `advertise: bool` (vim internals and dialog-local keys are `false` in the help overlay;
  they appear only in `/keys --all`).

Generate from it:
- the help overlay's keyboard sections (delete every hand-written `key_entry(...)` in
  `ui_overlays.rs:388-560`; keep the slash-command and skills parts);
- `/keys`, as a proper scrollable overlay (not a system message), **opened at the top**,
  grouped, with `advertise=false` rows behind a `v` toggle ("show vim / dialog keys");
- the tips widget: rewrite the tips as templates that reference action ids
  (`"{key:thinking.toggle} hides the model's thinking"`), so the key labels always come
  from the registry. Delete tips about features operant lacks;
- the footer/placeholder hints (`ui_input.rs`, status line), from the same lookup.

### 2. The keymap (final; decisions 1–3 in INDEX)

Changed from today: **bold**. Everything else stays where it is.

| Key | Action |
|---|---|
| Enter / Shift+Enter, Alt+Enter | submit / newline |
| Esc | interrupt the streaming turn (plan 04); close an overlay; **double-Esc on an empty composer opens the rewind selector (plan 05)** |
| Ctrl+C | copy selection; or interrupt (plan 04, same path as Esc); or clear the composer; twice on empty = exit |
| Ctrl+D | exit (empty composer, twice) |
| **Ctrl+T** | **toggle thinking (plan 08)** |
| **Ctrl+Shift+O / Alt+O** | **show/hide tool activity (plan 08)** |
| Ctrl+O | expand/collapse the latest tool output |
| **Shift+Tab** | **cycle reasoning effort (plan 03)** |
| Ctrl+G | external editor |
| Alt+G | scroll bookmark (alt-screen mode only; hidden in native-scroll mode) |
| Alt+P | cycle configured models |
| Ctrl+A | model picker (kept; not readline line-start, documented) |
| F5 / Alt+R | retry |
| Ctrl+R | prompt history search |
| Up/Down | prompt history (empty composer, or first/last line) |
| Ctrl+P | global search |
| Ctrl+K | command palette (kept; documented) |
| Ctrl+B | **session branches (now populated, plan 05)** |
| Ctrl+L / Alt+L | clear view / **redraw history (native mode replay, plan 09)** |
| Ctrl+S / Ctrl+X / Ctrl+Z / Ctrl+Y | stash / recall queued / undo / yank |
| Ctrl+U / Ctrl+W / Alt+Backspace / Alt+D / Alt+B / Alt+F / Ctrl+E / Ctrl+F | readline editing |
| **Alt+Y** | **copy mode (was Ctrl+T); yank-pop dropped** |
| F1 / `?` on empty | help |
| F8 | usage overlay |
| F12 | debug overlay |

Permission mode leaves Shift+Tab. It is set through a new **`/permissions`** picker
(Default, Accept edits, Plan, Bypass). Choosing Bypass asks for confirmation, reusing the
`bypass_permissions_dialog`. `/yolo` stays as the explicit one-shot arm.

### 3. Overlay fixes

- `/keys` and `/hotkeys`: one scrollable overlay (`/hotkeys` = `/keys` sorted by your
  usage), opened at the top.
- `/keybindings`: remove it, along with the `keybindings.json` writer, until user
  overrides are actually implemented (TODO.md Pending: `[keybindings]` table in TOML).
  Remove the `eprintln!`.
- `/log`: open the log file path in a pager overlay (tail of `~/.operant/logs/operant.log`,
  read-only), or unregister it. Don't open the Connect dialog.
- `/info`: show it once (status notice only).
- `/usage`: alias of `/stats`.
- `/permissions`: new picker (above).
- `/config`: keep it, with its title suffixed "(legacy — edits settings.json)" until the
  rebuild lands, so it doesn't pretend to edit `operant.toml`.

## Tests (RED first)

1. `help_overlay_advertises_only_registry_chords`: render the help overlay to text,
   extract every chord-looking token (`(Ctrl|Alt|Shift|Cmd|Super|F\d+)(\+\S+)*`), and
   assert each one parses to a registry binding with `advertise = true`. RED today (Ctrl+J,
   Alt+S, …).
2. `tips_reference_registry_actions_only`: every tip template's `{key:…}` resolves, and no
   tip contains a raw chord string.
3. `every_advertised_binding_dispatches`: table-driven. For each `advertise=true` binding
   in the Global/Prompt contexts, feed the key to a fresh `App` (with the minimal state
   it needs) and assert the registered action's observable effect: an overlay flag, the
   composer change, the agent action queued, or the notice text. This is the guard that
   keeps "what it says" == "what it does".
4. `shift_tab_never_changes_permission_mode` and `permissions_picker_requires_confirm_for_bypass`.
5. `keys_overlay_opens_at_top`, `info_prints_once`, `log_does_not_open_connect_dialog`.
6. The existing `the_default_keymap_has_no_conflicts` stays green.

## Steps

1. Registry fields + generator functions (`keybindings/report.rs` already formats
   reports; extend it).
2. Help overlay, `/keys` overlay, tips, footer from the registry. Delete the hand-written
   lists.
3. Key changes from the table (coordinate with plans 03, 05, 08; whichever lands first
   adds its row, and the dispatch test covers it).
4. Overlay fixes.
5. Corpus: `help-overlay` (×2), `footer-bar`, `status-row`, `effort-picker` goldens are
   re-proved, with every diff reviewed. New scenarios: `keys-overlay`,
   `permissions-picker`.
6. README shortcuts table regenerated from the registry. Add a `cargo test` that fails
   if README's table is stale, by comparing it with the generated text.

## Acceptance

Every chord shown anywhere in the TUI does what it says, as proved by test 3. Shift+Tab
can't arm bypass. `/keys` is readable and starts at the top. No command opens an unrelated
overlay.

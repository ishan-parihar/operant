# Plan: TUI Visual Overhaul → jcode parity

> Status: proposed. Scope decided by user: **local pty + tmux capture** for the feedback loop
> (telegram delivery dropped), **phased port — foundations first, then surfaces in order**.
> Reference: `parent-projects/jcode` (`jcode-tui*`, 15 crates, 376 files / 259,323 LOC).
> Subject: `crates/operant-cli/src/tui/` (156 files / 57,186 LOC).

---

## 0. The finding that reframes this

jcode is **not** a design-token temple. Grepped clean, it has:

- **no spacing scale** — no `Padding` constant, no `Margin`, `ratatui::layout::Margin` imported
  exactly once (`session_picker.rs:16`), and no `theme::space` of any kind.
- **no typography scale** — `Modifier::BOLD` applied inline per title (~20 sites), no size/weight
  abstraction, three grey constants standing in for text tiers.

So "make it look like jcode" is **not** "add design tokens". jcode's quality comes from four
things it *does* have, and operant lacks all four:

| # | jcode has | operant lacks |
|---|-----------|---------------|
| 1 | 22 **semantic roles** + one buffer-level substitution choke point (`jcode-tui-style` 5,019 LOC; `palette.rs:30-80`, `adapt_buffer_for_display` at `ui.rs:2669`) | 18 palette fields but **4 competing accent sources** and ~300 hardcoded colors that bypass the palette |
| 2 | **one** modal/box idiom — `render_rounded_box` (`jcode-tui-render/src/lib.rs:9-14`), every overlay gets title + bottom hint + dim border (`ui_overlays.rs:71-83`) | **16** modal sizing formulas and **5** title idioms |
| 3 | **frame hygiene** — full clear per frame (`ui.rs:2694-2700`) + `BeginSynchronizedUpdate` (`run_shell.rs:477,:516`) + `SoftRepaint` sentinel (`run_shell.rs:189-222`) | no clear, no sync, `Color::Black` hardcoded as the base fill (`render/mod.rs:103`) |
| 4 | **narrow-terminal floors + graceful collapse** (`MIN_CHAT_WIDTH=20`, `MIN_DIAGRAM_WIDTH=24`, …, `ui.rs:2835-2836`), a 10-step footer compaction ladder (`ui_input.rs:2027-2038`) | none; insets disagree `-4`×12 / `-2`×2 / unclamped×3 |

Consequence for the plan: phases 0–1 and 3.1–3.2 are where the win is. Phases 4–5 are the
difference between "looks tidy" and "feels designed". I am deliberately adding one thing jcode
lacks — a **spacing scale** (§1.4) — because 16 divergent modal formulas is a *correctness*
smell, not a taste question.

---

## 1. The feedback loop already exists — and works

`operant tui debug simulate` drives the **real** `App::run` loop against a ratatui
`TestBackend`, replays scripted keys, injects deterministic mock agent events, dumps the final
screen, and asserts on both state JSON and screen text. Verified live, exit 0.

`App::run<B: ratatui::backend::Backend>` (`tui/app/mod.rs:776-782`) is **generic over the backend**,
so production and headless share one code path with zero shimming. This is the right seam and we
keep it.

```
operant tui debug simulate \
  --keys "/help<enter>" \
  --agent-script ag.json \
  --assert "overlays.help_overlay == true,any_modal_open == true" \
  --assert-screen "contains:Shortcuts" \
  --dump-screen out.txt --size 120x40
```

**Three real gaps, all additive:**

| Gap | Evidence | Consequence |
|-----|----------|-------------|
| **No baseline diff** | `--dump-screen` writes text (`cmd_tui_debug.rs:1357-1360`); `--assert-screen` is substring-only (`:1383-1412`) | visual drift between runs is undetectable; you can only assert a string is *present* |
| **Final frame only** | one explicit draw at `tui_app.rs:922` | streaming progression, animation state, mid-sequence scroll never captured. `FrameRendered{…}` event exists (`event_bus.rs:96-100`) but records *timing only* |
| **Text-only dump** | `tui_app.rs:933` extracts `c.symbol()` and nothing else | `fg`/`bg`/`modifier`/`underline_color` are dropped — **a palette or contrast regression is structurally invisible** |

Also: 349 TUI tests, ~0 visual. The flagship `test_dialog_open_close_scenarios`
(`tui/app/tests.rs:1454-1507`) drives 13 overlays and asserts **state JSON only** — it would pass
with a completely broken render. No `insta`/`expectrl`/`vt100` anywhere; `ratatui 0.30.2` ships
`TestBackend` in-tree so no new dependency is needed.

And a doc bug: `cmd_tui_debug.rs:13-15` still claims the subcommand "does NOT render anything".

---

## 2. Why it looks ugly — ranked, measured, with the jcode counterpart

### 2.1 No spacing system *(the single biggest offender)*
`Padding::` — **1 use** in 57,186 LOC (`Padding::new(1,0,1,0)`). `Margin::` — **0 uses**. No spacing
constants. Only three layout constants exist under `render/`: `WELCOME_BOX_HEIGHT`, `STATUS_THINKING`,
`STATUS_THINKING_ELLIPSIS` (`render/mod.rs:93-95`). Chat chrome is raw `Span::raw("  ")` at six
sites (`render/footer.rs:311,322,336,361,372,384`) and `" {} "` pills at seven more.

Modal vertical inset: `-4` in 12 surfaces, `-2` in 2 (`ask_user_dialog.rs:209`,
`free_mode_dialog.rs:220`), **unclamped** in 3 (`custom_provider_dialog.rs:125`,
`key_input_dialog.rs:104`, `memory_file_selector.rs:129`). Widths span **44–90** with no scale —
`effort_picker` is 44 wide, `journey_view` is 90.

A correct clamp already exists and is used by only 2 of 16 sites:
`overlays/layout.rs:111-112` (`begin_modal_frame` `:137`, `begin_modal_buf` `:152`).

### 2.2 Five incompatible title conventions across 23 sites
| Format | Sites |
|---|---|
| plain `&str`, no colour, no BOLD | 6 — `session_browser.rs:388`, `history_search.rs:579`, `message_selector.rs:196`, `rewind_flow.rs:150`, `mcp_view.rs:618`, `global_search.rs:162` |
| `Span::styled` + BOLD + themed fg | 4 — `mcp_approval.rs:291`, `debug/overlay.rs:39`, `bypass_permissions_dialog.rs:85`, `mcp_view.rs:333` |
| `Span::styled` + BOLD, **no fg** | 4 — `effort_picker.rs:73`, `journey_view.rs:195`, `plugins_hub.rs:165`, `skills_view.rs:172` |
| multi-span | 2 — `welcome.rs:136`, `bypass_permissions_dialog.rs:85` |
| `title_alignment(Center)` | 1 — `session_browser.rs:389` (the only centred title in the app) |

Plus a 6th idiom in `overlays/layout.rs:158` used by `theme_screen.rs` and `settings_screen` and
by no `Block`-titled dialog. Key-hint text has 4 spellings of the same fact: `"Esc to cancel"` ×3,
`"Esc close"`, `"Esc: close"`, `"Esc to close."`.

### 2.3 Four competing accent sources for one role
Four structurally identical management overlays differ only in which colour they reach for:
`skills_view.rs:169` `accent()` · `plugins_hub.rs:163` `success()` · `journey_view.rs:194`
hardcoded `Color::Magenta` · `effort_picker.rs:72` hardcoded `Color::Magenta`.

Worse, the 4th source is `app.accent_color` = `ACCENT_BUILD = Rgb(255,191,0)` (`app/enums.rs:314`),
which is **byte-identical to the default theme's `emphasis` and `selection_bg`**
(`theme_colors.rs:89,94`) but is a separate literal — so **`/theme` cannot repaint it** on 7 of 8
themes. `banner.rs:275-280,345` documents this as a known pending migration.

Also: `Color::DarkGray` hardcoded at 22 sites in `stats_dialog/render.rs` alone, 16 in
`journey_view.rs`, 7 in `dialogs/mcp_approval.rs` — while `DIALOG_DIM` (`theme_colors.rs:403`) and
`DIALOG_MUTED` (`:406`) exist for exactly this role and are used at only 6 and 5 sites.

### 2.4 The palette is real but leaky
625 `theme_colors::` call sites across 60 files — the majority path is correct. Surviving beside it:
**50 `Color::Rgb` literals in 18 files** and **250 named-ANSI `Color::X` uses in 39 files**
(268 total − 16 in `theme_colors.rs` − 2 in `color_depth.rs`). `Color::Black` is the hardcoded
frame background at `render/mod.rs:103`, `debug/overlay.rs:46,120`, `prompt_input/render.rs:201,203,349`
— the base fill ignores `panel_bg`/`overlay_bg`.

`stats_dialog/render.rs` is the worst cluster: 9 RGB literals + 22 `Color::DarkGray` alongside 24
palette calls, and the 5-stop heat ramp **declared twice** (`:311-317` inline, `:395-403` as a match
arm — and `Rgb(0,150,0)` at `:399` has no counterpart in the inline version).

Duplicated values: `Rgb(200,200,200)` ×9, `Rgb(0,150,200)` ×7, `Rgb(255,191,0)` ×5,
`Rgb(100,181,246)` ×5 (dark theme makes info = action = accent = emphasis = selection_bg).

### 2.5 Two render-pass idioms force duplicated helpers
Some surfaces call `frame.render_widget`, others take `frame.buffer_mut()`. Consequence:
`overlays/layout.rs` ships **every shared helper twice** — `render_dark_overlay`/`_buf` (`:63`/`:68`),
`render_dialog_bg`/`_buf` (`:79`/`:84`), `render_modal_title_frame`/`_buf` (`:158`/`:183`).
`centered_rect` is implemented twice: `overlays/layout.rs:27` and `dialogs/permission.rs:339`.

Ratatui's own `List`/`ListItem` are effectively unused (`List::new` ×3, `ListItem::new` ×2 against
`Paragraph::new` ×134) — which is why selection/highlight is re-implemented per view.

### 2.6 3 rounded surfaces against 22 square
`BorderType::Rounded` at exactly 3 sites: `render/selection.rs:168`, `render/welcome.rs:134`,
`render/utils.rs:82`. The other 22 `Borders::ALL` are square. The entire 3,307-LOC `messages/` layer
and `render/messages.rs` (553 LOC) are **borderless**. Only 2 divider idioms exist: the 1-row blank
`chunks[1]` (`render/mod.rs:231`) and `Borders::TOP` (`debug/overlay.rs:115`).

### 2.7 The event loop is a 545-line flat `loop`
`app/mod.rs:776-1321`: **9 inline `try_recv` drains**, a `tokio::spawn` embedded in the loop body
(`:857-880`), and the draw→poll→dispatch triad never extracted. `render_app` is a 399-line ladder
with **35 overlay `if app.X.visible` branches** (`render/mod.rs:96`).

`dialog_priority()` — a 34-arm priority chain (`app/dialog_routing.rs:6-101`) — is **computed and
discarded** at `key_handling.rs:45-47` (`let _priority = self.dialog_priority();`, under a comment
that says outright "we assert the current handler matches that priority for debugging"). It is a
debug assertion, not the router. Real routing is ~30 `if state.visible { match key.code }` gates across
31 `match` sites in a 2,088-line file.

---

## 3. Phase 0 — Make the ugliness visible and regression-proof

**Prerequisite for everything.** Until style is in the snapshot, no visual change is verifiable.

| # | Task | Files | Done when |
|---|------|-------|-----------|
| 0.1 | **Style-aware dump.** Add `--dump-style` to `simulate`: serialise `symbol` + `fg` + `bg` + `modifier` per cell (TOON or a compact `fg:bg:sym` grid), not `c.symbol()` alone. | `tui_app.rs:924-939`, `cmd_tui_debug.rs` | a theme change produces a **different** dump; a palette regression is detectable |
| 0.2 | **Baseline diff.** Add `--baseline <path>`: compare rendered screen to a committed golden file, print a unified diff, exit non-zero on drift. Baselines in `crates/operant-cli/tests/tui_baselines/`. | `cmd_tui_debug.rs` | deliberately changing one colour fails the gate with a readable diff |
| 0.3 | **Per-frame capture.** Extend `FrameRendered` to optionally carry buffer content; add `--capture-frames 0,3,7` to snapshot at chosen frames so streaming / animation / mid-scroll states are covered. | `event_bus.rs:96-100`, `debug_hub.rs:74-84`, `app/mod.rs:1151` | a mid-stream frame is capturable and diffable |
| 0.4 | **Scenario corpus.** One scenario file per surface — all **43** `render_*` entry points plus the 13-overlay regression pack. Schema: `{ keys, agent_script, size, assert, assert_screen, baseline }`. Runner walks the directory. | new `tui_scenarios/` + runner | `operant tui debug simulate --suite` runs all surfaces green; a new surface without a scenario fails CI |
| 0.5 | **Real-terminal capture.** `tmux`-based: launch the real binary in a detached pane at a fixed size, `send-keys` the scenario, `capture-pane -p -e` (ANSI preserved). Covers the interactive-only paths `TestBackend` deliberately bypasses: raw mode, alt-screen, mouse capture, real `Event::FocusGained` reassertion, OSC 8, pinned images. | new `scripts/tui-capture.sh` | a real `operant chat` pane is captured and diffed the same way as the headless dumps |
| 0.6 | **Fix the stale header.** `cmd_tui_debug.rs:13-15` says the subcommand "does NOT render anything". | 1 line | docs match reality |

`tmux` and `script` are on PATH; `vhs`, `asciinema`, `expect`, `ttyd`, headless chromium are not.
No new crate needed.

---

## 4. Phase 1 — Design foundation

The jcode-tui-style analogue. Wrap `theme_colors.rs`, do not rewrite it.

| # | Task | Port from | Done when |
|---|------|-----------|-----------|
| 1.1 | **`Role` enum + frozen default table.** 22 named roles over the existing 18 fields (`User, Ai, Tool, FileLink, Dim, Accent, System, Queued, Pending, UserText, UserBg, AiText, HeaderIcon, HeaderName, HeaderSession, Success, Warning, Error, Info, Border, SelectionBg, …`). Defaults byte-frozen in a redundant `HAND_TUNED` table so tooling can't drift them. | `palette.rs:30-80`, `palette.rs:148-173`, `palette.rs:776-799` | `Role::default_rgb()` is the single source; changing it is a deliberate edit |
| 1.2 | **Buffer-level substitution choke point.** One `adapt_buffer_for_display(&mut Buffer)` after every draw: apply theme overrides → adapt unconfigured → contrast-repair. Role accessors return the **default**; substitution happens once per frame, so no cell is remapped twice. | `ui.rs:2659-2680`, `palette.rs:362-373`, `palette.rs:618-699` | `/theme` repaints **every** surface, including `app.accent_color`; an unconfigured palette is a byte-identical no-op |
| 1.3 | **Kill the competing accents.** `app.accent_color`/`ACCENT_BUILD` (`app/enums.rs:314`) becomes a *role*, not a colour. `journey_view.rs:194` and `effort_picker.rs:72` hardcoded `Magenta` → roles. | — | exactly one accent source |
| 1.4 | **Spacing scale** *(deliberate deviation — jcode has none)*. A small `space` const set (`XXS…XXL`) + `pad(h)`/`pad_v(v)` helpers. Chrome stops using `Span::raw("  ")`. | — | no raw-space padding remains in chrome |
| 1.5 | **One modal idiom.** `modal_frame(title, hint) -> Rect` → rounded border, themed bold title, `title_bottom` hint line, dim border, shared clamp. Replaces all 16 sizing formulas and all 6 title idioms. One `Esc to close · …` hint spelling. | `render_rounded_box` `jcode-tui-render/src/lib.rs:9-14`; `ui_overlays.rs:71-83`; `chrome.rs:31-36` | every modal is sized by one function; grep finds one title idiom |
| 1.6 | **Frame hygiene.** Full clear per frame; wrap the draw in `BeginSynchronizedUpdate`/`EndSynchronizedUpdate`; add `SoftRepaint` sentinel invalidation. Kill hardcoded `Color::Black` as the base fill. | `ui.rs:2694-2700`, `run_shell.rs:189-222`, `run_shell.rs:477,:516` | no stale cells after a layout change; no flicker on repaint |
| 1.7 | **Contrast + light/dark repair.** HSL lightness inversion with a WCAG binary-search repair on **foregrounds only**, target contrast 7.0. | `theme_mode.rs:108-220` | light theme and 256-colour terminals stay legible |
| 1.8 | *(deferred)* **Oklab harmony scoring** — `harmony.rs` is 2,357 LOC in jcode. Worth it only if palette work continues past parity. | `jcode-tui-style/src/harmony/` | — |

---

## 5. Phase 2 — Loop architecture

| # | Task | Port from | Done when |
|---|------|-----------|-----------|
| 2.1 | **`needs_redraw: bool` discipline.** Every branch returns a bool OR, not a repaint-everything loop. Idle costs nothing. | `run_shell.rs:654-753` | idle CPU drops; every repaint has a cause |
| 2.2 | **Named redraw reasons.** `live_activity_redraw_reason() -> Option<&'static str>` so "why is this a full frame" is answerable without bisecting. | `redraw_schedule.rs:630-662` (reason table `:160-186`) | the F12 overlay reports the exact predicate |
| 2.3 | **Extract the 9 inline channel drains** into named handlers; move the embedded `tokio::spawn` out of the loop body. | — | `app/mod.rs:776` shrinks to a readable loop |
| 2.4 | **Event coalescing.** Drain up to N extra events after a focus/key event so a held key coalesces into one frame. | `app/local.rs:141-158` | held-key bursts cost one frame, not N |
| 2.5 | **Actually route by priority.** Make `dialog_priority()` the router instead of a discarded assertion. | `dialog_routing.rs:6-101` | one `if` chain, not 30 scattered gates |
| 2.6 | **Reassert terminal modes on `FocusGained`.** Terminals and multiplexers clear modes behind your back. | `tui/mod.rs:187-216` | returning to the pane keeps bracketed paste / mouse / focus events |

---

## 6. Phase 3 — Surfaces in order

Ordered by user-visible frequency. Each step reuses the Phase 1 foundation and regenerates that
surface's baselines.

| # | Surface group | Current | Target shape |
|---|---------------|---------|---------------|
| 3.1 | **Main conversation** — transcript, tool blocks, reasoning | `render/messages.rs` (553), `render/tools.rs`, `messages/` (3,307) | borderless, structure from role colour + glyph prefixes (jcode's approach). The `▼ Thinking` shimmer stays. Tool rows: model intent by default, technical detail behind a toggle. |
| 3.2 | **Chrome** — header, status, composer, footer | no header exists; `render/footer.rs` (751 LOC) | jcode `ui_header.rs` + `ui_input.rs:774/2444`. Add a real header. Replace the half-used `Length(2)` footer with jcode's **10-step compaction ladder** (`ui_input.rs:2027-2038`) so it always answers where-am-I / what's-running / how-full-is-context. |
| 3.3 | **Management panels** — MCP, Skills, Plugins, Hooks, Journey, Agents, Context, Usage | 8 surfaces, 4 differing border colours, `mcp_view.rs` at 50 palette sites | one idiom, `Percentage` splits replaced with ratio-as-upper-bound + min floors |
| 3.4 | **Pickers** — `dialog_select` (one renderer, 4 surfaces), effort, model, session browser, memory selector | widths 44–60, hardcoded `Color::White` highlight at `dialog_select.rs:200,:316` | one `picker()` idiom; selection uses `Role::SelectionBg` + `on_selection()` |
| 3.5 | **Overlays** — help, history/global search, rewind, settings, theme, stats, diff | 13 overlays, 2 render-pass idioms | one box idiom; scroll % folded into the title; kill the frame/buf helper duplication |

**Cross-cutting:** `McpViewState`, `SettingsScreen`, `PromptInputState` are already separate structs —
good. Leave `VirtualList<T: VirtualItem>` alone; it is sound.

---

## 7. Phase 4 — Affordances operant does not have at all

None of these exist today. This is the "feels designed" layer.

| # | Affordance | jcode reference |
|---|-----------|-----------------|
| 4.1 | **Narrow-terminal floors + graceful collapse.** `MIN_CHAT_WIDTH`, `MIN_*` per surface; if a floor can't be met, don't split the pane out at all. | `ui.rs:2835-2836`, `:2946-2950` |
| 4.2 | **Empty states** for every panel and picker — "type to search" vs "no matches" are different strings. | `ui_input.rs:157-162`, `navigation.rs:1102,1119` |
| 4.3 | **Focus indication** — the focused pane's border takes the focus colour; explicit focus flags threaded to renderers. | `chrome.rs:33-36`, `ui.rs:3374-3386` |
| 4.4 | **Overflow labelling** — `↑N` above, `+N more` below. Never a silent truncation. | `ui_input.rs:303-314` |
| 4.5 | **One hint line** — priority-ordered, mode-aware, suppressed when an overlay owns the rows. Replaces 4 `Esc` spellings. | `ui_input.rs:2483-2513` |
| 4.6 | **Truncation primitives** — clip / ellipsis / suffix-preserving, all `unicode_width`-aware. Suffix-preserving matters for paths and branch names. | `jcode-tui-render/src/lib.rs:71-195` |
| 4.7 | **Actionable errors** — a failure names the recovery command and restores your input. | `tui_lifecycle.rs:337-351` |
| 4.8 | **Resize preserves reading position** instead of teleporting to the new bottom. | `app.rs:890-894`, `local.rs:90` |

---

## 8. Sequencing and honest sizing

This is **many iterations**, not one. `AGENTS.md` requires one commit + push per iteration, and
every phase above is independently shippable and reviewable.

| Iteration | Content | Risk |
|-----------|---------|------|
| 1 | Phase 0 (0.1–0.6) | Low — additive, no visual change yet. Unblocks everything. |
| 2 | 1.1–1.3 (roles, substitution, kill competing accents) | Medium — touches every render site. Baselines will churn; that is the point. |
| 3 | 1.4–1.5 (spacing scale, one modal idiom) | Medium — 16 modal call sites + 23 title sites. Biggest single visual win. |
| 4 | 1.6–1.7 (frame hygiene, contrast repair) | Low-medium — self-contained in `draw`. |
| 5 | Phase 2 (loop architecture) | Medium — behaviour-preserving but touches the hot loop. |
| 6+ | Phase 3, one surface group per iteration | Low each, cumulative |
| n | Phase 4 | Low — additive affordances |

**Expected win:** phases 0–1 + 3.1–3.2 address causes 2.1–2.7, which the audit ranks as the
dominant contributors. Claiming parity is not honest until baselines from those steps are reviewed
side-by-side against jcode at the same terminal size.

**Not doing (YAGNI unless asked):** Oklab harmony scoring (1.8, 2,357 LOC upstream), jcode's 3D idle
donut, the 15-widget HUD, inline interactive pickers, copy-mode with edge autoscroll, workspace map.

---

## 9. Verification gate (every iteration)

Per `AGENTS.md`, scoped and local. GitHub Actions runs are banned — read-only `gh run` inspection only.

```bash
source scripts/dev-env.sh
cargo fmt --all
./scripts/check.sh check -p operant-cli --bin operant
./scripts/check.sh test  -p operant-cli --bin operant -- tui          # state + buffer assertions
./scripts/check.sh test  -p operant-cli --test tui_scenarios          # the corpus
./scripts/tui-capture.sh --all                                       # tmux real-terminal spot check
bash scripts/clippy-warning-gate.sh                                  # no NEW warnings vs allowlist
```

Plus, for a visual iteration:

```bash
# regenerate baselines, then read the diff — never auto-commit a baseline
./scripts/tui-baselines.sh accept
git diff --stat crates/operant-cli/tests/tui_baselines/
```

A baseline regeneration that touches more files than the iteration changed is a signal the change
was too broad. Split it.

### Working-tree constraint (live)

Peer WIP is **uncommitted and must not be staged or reformatted**:
`crates/operant-core/tests/stream_interrupt.rs`, `crates/operant-core/src/agent/stream_retry_budget.rs`,
`crates/operant-cli/src/gateway_commands.rs`, `gateway_runner.rs`, `BUGS.md`, `Cargo.toml`,
`Cargo.lock`, `docs/plan-2026-09-29-telegram-loop.md`, `plans/telegram-ux-fixes.md`,
`docs/ORGANISM-OS-UPGRADE-OUTLINE.md`, plus the dirty `crates/operant-core/src/agent/*` files.

`cargo fmt --all` will touch `gateway_commands.rs:517` and dirty the tree. Use
`rustfmt --check --edition 2024 <changed files>` for this work, and stage explicit paths only.
`git pull --ff-only` is currently blocked by this WIP — do not stash.

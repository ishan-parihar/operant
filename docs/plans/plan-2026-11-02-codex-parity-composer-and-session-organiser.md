# Codex-parity composer and the command-center session organiser

Dated-frozen: published `2026-11-02`. Registered in `genome.toml`
`core.living_plans`.

## The request

The operator asked for two things in one breath:

1. Make operant's input box look like `../external-repos/codex`'s — "must more
   premium and mature" — keeping operant's footer exactly as it is.
2. Investigate codex's session-picker / agent-command-center and its
   "segregated session-management system", because operant's session hub left
   pane is "immature in terms of its organising capacity" while the right pane
   (live session preview) is already better than codex's.

This plan records the investigation and the design for both. It supersedes
nothing and is the visual sibling of the already-landed
`2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md` and
`plan-2026-11-02-session-resumability-and-gateway-session-navigation.md`.

---

## Part 1 — What codex's composer actually is

Read in full this session: `tui/src/bottom_pane/chat_composer.rs` (12,810 lines)
and `chat_composer/` (`composer_layout.rs`, `status_surface.rs`,
`footer_state.rs`, `popup_state.rs`, `sparkle.rs`); `textarea.rs` (4,760) and
`textarea/`; `{popup_consts.rs,picker_style.rs,composer_gap.rs,voice_strip.rs}`;
`ui_consts.rs`, `style.rs`, `styles.md`; and every composer snapshot under
`snapshots/` and `chat_composer/snapshots/` (~60 files), which are the ground
truth for the drawn pixels.

### The measured design

Codex's composer is **not** a bordered box. It is a **filled surface**:

```text
                                                                 
› Explore the night sky
                                                                 
                                                                 
                                                                 
                                                                 
                                                                 
                                                                 
                                                                 
  › is for shortcuts                                   100% context left
```

What makes it look expensive, in order of contribution:

1. **A filled background over the whole band** —
   `Block::default().style(user_message_style()).render(composer_rect, buf)` at
   `chat_composer.rs:4889`. `user_message_bg_rgb` (`style.rs:261-268`) blends
   white at alpha **0.12** over the terminal background (black at 0.04 on light
   themes). This one line is most of the perceived quality.
2. **A single-glyph prompt in a dedicated 2-column gutter** —
   `LIVE_PREFIX_COLS = 2` (`ui_consts.rs:10`), shared with the user-history
   lines and the status rows, so the whole left edge of the transcript aligns on
   one gutter. codex never prints a prompt number.
3. **A dimmed placeholder in the empty state** — `"Ask Codex to do anything"`
   (`chat_composer.rs:4976` replaces the need for a "type here" hint row).
4. **A blank row above and below the text** — the layout's
   `desired_height_with_options` carries a trailing `+2` and a
   `Constraint::Min(3 + voice_rows)` floor.
5. **A separate persistent status row below the band** (`status_surface.rs`):
   `MODEL · ~/project · Context 20% used` left, `Plan mode` right. Distinct
   from the hint row so transient hints never displace persistent context.
6. **A hint footer** — `› is for shortcuts` left, `100% context left` right,
   indented by `FOOTER_INDENT_COLS = 2` so it aligns with the gutter.
7. **Popups float above the composer, never inline** —
   `ActivePopup::is_above_composer()`, and the popup's height is reserved from
   the space *above*, so opening `@` or `/` never moves the input band.

What it deliberately does not have: no box-drawing border, no rounded frame, no
per-side colour. The premium feel is entirely fill + whitespace + one bold glyph
+ a disciplined footer.

### The prompt glyph is mode-aware (`chat_composer.rs:4923-4945`)

| state | glyph |
|---|---|
| normal | `›` bold |
| input disabled | `›` dim |
| shell mode (leading `!`) | `!` light-red bold |
| Luna Reserve | `›` gold `(246,197,67)` bold |
| effort tier | `tier.prompt(charge)` — an igniting gradient glyph |

### The geometry (`composer_layout.rs`)

```text
area
├── status row            (1 row, only when separate_status_line)
│   ├── [popup above]         (reserved from above the composer)
│   └── composer rect         -- fill --
│       ├── voice strip (0..n)
│       ├── textarea rect     (inset tlbr(1 + voice, LIVE_PREFIX_COLS, 1, 1))
│       └── remote images     (n rows above the textarea)
└── footer hint row       (1 row, inset by FOOTER_INDENT_COLS)
```

---

## Part 2 — operant's composer before this change

Measured with the headless sim at 100x30 (`operant tui debug simulate --keys
hello --size 100x30 --dump-screen`):

```text
-- INSERT --
1> hello
                                    ~/?/operant API key OpenAI GPT-4
```

`draw_input` (`ui_input.rs`) was a bare `Paragraph` in chunk 7 with no fill, no
gutter discipline, no placeholder, and a numbered prompt `1> `. Above it sat the
vim badge row; below it the one-row overscroll footer. The gap against Part 1 is
precisely: no fill, no `›`-glyph gutter, no placeholder, no padding rows, and a
prompt index codex does not have.

**The operator's explicit constraint: the footer stays as it is.** Operant's
footer is the overscroll row plus the hint rows (shell-mode hint,
`Ctrl/Cmd+Enter to queue`, the vim badge, the send-mode icon). This iteration
adds above the footer and restyles inside the input band only.

---

## Part 3 — What landed (`feat(iter-1637)`)

New module `crates/operant-cli/src/tui/operant_ui/composer.rs` (257 lines, 10
unit tests) plus four surgical edits to `ui_input.rs` and one to
`operant_ui/mod.rs`. The code is on `origin/main` at `d310c2e1` / `33552dfd`;
the peer agent's `docs(iter-1637)` commit `bb4a6dc1` swept the tree mid-edit and
carried these three files up with it, which is why they are labelled iter-1637
rather than their own commit. Content is unmodified — `git diff bb4a6dc1 HEAD`
over the three paths is empty.

### 3.1 The fill

`composer_fill_style()` reproduces codex's alpha blend. operant cannot read the
terminal background — `theme_detect.rs` gates the OSC 11 query behind a
pending-dependency `#[cfg(any())]` and falls back to dark — so the blend runs
over `theme_colors::panel_bg`, the surface the active theme already declares.
Rec. 709 luma picks the lift direction, the same `is_light` decision codex makes
at the same threshold. A non-RGB surface (a quantized 256 palette) degrades to
an empty style, exactly as codex degrades when `default_bg()` answers `None`.

Painted with `Block::default().style(style).render(area, buf)` before the
paragraph, mirroring `chat_composer.rs:4889`.

### 3.2 The gutter and the `›` glyph

`input_gutter_cols()` returns `GUTTER_COLS = 2` and `debug_assert`s that every
mode glyph `input_prompt` can return is two columns, so the constant cannot
silently drift from the painted glyph. The default prompt glyph changed from
`> ` to `› ` (codex's `"›".bold()` branch at `chat_composer.rs:4945`);
`$ `, `› ` and `› ` keep their mode meanings.

`wrap_input_text` gained a gutter policy: `wrap_input_text_gutter(.., None, ..)`
paints only the glyph, and the continuation-row indent
(`" ".repeat(prompt_len)`) carries the alignment, so a wrapped first row and its
continuations share one left edge. The reserved WIDTH is still `prompt_len`, so
`draw_input`'s copy-selection margins and cursor maths are unchanged between
policies.

**The prompt index is no longer painted.** Trade-off stated rather than hidden:
codex has no index and the operator asked for codex parity; the index remains
meaningful to the footer status row and the call site's height derivation, so
`draw_input`'s `next_prompt` parameter stays documented as `_next_prompt` rather
than being removed and rippling into the call site.

`input_prompt_len` became dead (no remaining caller) and was removed rather than
left as dead code.

### 3.3 The placeholder

`"Ask operant to do anything"`, dimmed and italic, painted into the composer's
inset textarea rect when the draft is empty. Two offsets are required and both
are documented at the call site: the ROW offset (`suggestions_offset`) so it
never paints over a hint/vim badge row, and the COLUMN offset (`prompt_len`) so
it never paints over the glyph. Both were hit during implementation and measured
on the drawn buffer, not inferred.

### 3.4 The padding rows and band height

`input_padding_height = 2` added to `input_height` in `operant_ui/mod.rs`, and
`draw_input` measures and paints against `composer::textarea_rect(area)` =
`tlbr(1, GUTTER_COLS, 1, 1)` — the same inset codex computes at
`composer_layout.rs:105-110`. The band owns the two rows: chunk 7 grows by 2 and
the transcript gives them up, the same trade codex makes. Copy-selection margins,
the copy-snapshot rect, the cursor position and the centered alignment all moved
to the padded rect together, so hit-testing and copy still exclude the gutter.

### 3.5 What is deliberately NOT ported

- **`status_surface.rs`'s persistent status row.** operant already renders a
  status block above the composer (the brand header plus the overscroll row);
  a second status surface would violate the one-renderer rule in AGENTS.md and
  the operator's "keep the footer as it is" constraint.
- `Sparkle`, `VoiceStrip`, the effort-ignition glyph gradients, remote-image
  rows, `composer_gap` separators, warning notices — each is a subsystem with
  its own frame scheduling, not a restyle.
- `TextArea` (4,760 lines: vim registers, search, grapheme editing, paste-burst
  detection). operant's `wrap_input_segments` / `visual_line_move` is a working
  subset; the parity gap there is real but out of scope.

### 3.6 Measured result (headless sim, 100x30)

Typed state:

```text
-- INSERT --
› hello
                                     ~/?/operant API key OpenAI GPT-4
```

Empty state:

```text
-- INSERT --
› Ask operant to do anything
```

On the style grid (`--dump-style`): the fill `rgb:353547` covers every cell of
the band and nothing else; `›` sits at column 2 in the accent colour; the text
starts at column 4; the band carries a blank top row and a blank bottom row.
The vim badge and the overscroll footer are byte-identical to before.

### 3.7 Verification

- 10 unit tests in `composer.rs` (gutter width per mode, blend endpoints and
  clamping, both lift directions, the inset arithmetic including the
  narrower-than-the-gutter case, the placeholder's single span, the fill's
  degrade path).
- `cargo check -p operant-cli --bin operant` clean; `cargo fmt --all` clean.
- 217 existing `input`-filtered tests pass; the composer-filtered set passes.
- A visual before/after capture at 100x30 on both the typed and the empty state.

The workspace clippy gate was NOT run as part of this iteration: a concurrent
agent's in-flight WIP leaves `crates/operant-cli/src/tui/app/tests.rs`
mid-`vec!`-truncation, so the workspace test target does not compile. That
failure is none of this iteration's files — `cargo check -p operant-cli --bin
operant --tests` reports zero errors under `operant_ui/` — and per AGENTS.md
fleet rule 7 a peer's red subsystem is not this slice's to fix.

---

## Part 4 — codex's session-management system (the investigation)

There are TWO separate surfaces in codex, and the operator asked for both.

| surface | size | what it is |
|---|---|---|
| **Resume picker** | `resume_picker.rs` 7,906 lines + 5 children ~980 | a full-screen alt-screen picker for `codex resume`, with a toolbar, pagination and per-row archive |
| **Agent command center** | `agents_overview*.rs` ~3,500 lines + `agent_center/` | an in-TUI full-frame dashboard of background tasks: status tabs, grouping, search, rename, a metadata details pane, live notifications |

Operant's session hub is a full-frame overlay with the 40/60 split
(list | preview), which is architecturally the *picker* (the vendored jcode
geometry). It is missing everything that makes codex's picker a picker rather
than a list.

### 4.1 The resume picker's design

`resume_picker/layout.rs` — the vertical stack with collapsing decorative gaps:

```
FIXED_CHROME_ROWS = 7, MIN_LIST_ROWS = 6
gaps = clamp(height - 13, 0, 3)

header     1 row     "Resume a session" (bold)
gap        gaps>=3
toolbar    1 row     Filter: Cwd Repo All   Sort: Updated Created   Status: Active Archived
gap        gaps>=2
search     1 row     "Search: <query>" / "Type to search"
gap        gaps>=1
list       Min(...)  rows, viewport width = list - PICKER_LIST_HORIZONTAL_INSET
footer     4 rows    separator + 3 priority-degraded hint lines
```

The **toolbar** is the piece operant has nothing like. Three controls
(`Filter`, `Sort`, `Status`), each a set of pill values; Tab moves focus between
controls, `</>` moves within a control's values; the focused value gets
`active_tab_style()` (a real background fill) and merely-active values a
secondary fill. Values degrade to a compact form (`Sort: Updated`) when width
runs out, and to the single focused control's value when even that does not
(`toolbar_for_width`, `:2375-2424`).

The **row** has two shapes behind one enum, `SessionListDensity::Comfortable |
Dense` (`:298`): comfortable is marker + title then a meta line (created,
updated, branch, cwd — the cwd only when the row sits outside the filter's
directory); dense is one line, `date | title`, with a fixed date column. Both
paint a full-width row background: zebra at white-alpha 0.055, selected at 0.12
(`dense_row_background_style`, `:3227-3244`) via `apply_line_background`, which
pads the row to the viewport width and resolves each span's foreground against
the painted background with `readable_color_on`. **That contrast resolution is
the single visual upgrade operant's left pane needs most**: today every row is
a transparent `ListItem` in a bordered `Block` and the selection is a bare `bg`
fill with no contrast resolution, which is why it is hard to read on some
themes.

**`↑ more` / `↓ more`** page affordances appear at the list edges, with the
loading variant swapped in (`↓ loading more`). The semantics come from
`catalog.rs`'s per-source cursor state — one pending page per source, stale
replies discarded by request token, and a slow or exhausted source never blocks
another.

**Archive** (`archive.rs`) is a three-state machine `Idle | Pending | Restoring`
with `archive_shortcut_available()` checking the keymap does not already bind
`Ctrl+A`, a guard against archiving the current thread, and a row-level inline
error surface. Archiving re-lists rather than mutating in place.

**The picker runs on its own alt screen with an RAII `AltScreenGuard`** and a
background-source fan-out with cancellation-on-close (`sources.rs`).

### 4.2 The command center's design

`agents_overview_view.rs` + `agent_center/{render,rows,navigation,input,hints}.rs`:

```text
  Agent command center  Group: Project  g
   All 10   Needs you 0   Working 0   Ready 0   Inactive 10
  ────────────────────────────────────────────────────────────
      Tasks                                            Status         Updated
  › ○ Task 12                                          Inactive [age]
    ○ Task 11                                          Inactive [age]
      Show more
  ? help  esc back  ↑/↓ move  enter open  n new
```

The organising capacity the operator is asking for, item by item:

1. **A header naming the view and its grouping mode** — `Agent command center
   Group: Project  g`. One line tells you where you are and which key changes
   the organisation.
2. **A status tab bar with live counts** — `render_filled_tab_bar` over
   `TASK_FILTERS` (`agent_center/mod.rs`), each label carrying its count.
   Operant has a `HubFilterMode` enum but renders it as a static box title
   (`Sessions [all]`) with no counts and no tab affordance.
3. **Column headers** — `Tasks | Status | Updated`, dimmed, so the eye knows
   what the right-hand columns mean.
4. **Status-derived ordering** — `AgentsOverviewGroup::{NeedsYou, Working,
   Ready, Finished}`, rows keeping their relative order inside a group across
   metadata refreshes.
5. **Four grouping modes** — `Project` (git repo identity: `common_dir` +
   `relative_cwd`, so worktrees of one repo group together), `Directory`,
   `Status`, `Model` — cycled with `g`, each rendering a group heading with a
   count and sorting by `Reverse(updated_at)` inside it. Config-persisted.
6. **A status dot per row plus a `›` selection marker on the same line** — two
   glyphs, two jobs, no emoji.
7. **A metadata details pane** (`render_details`, `:448-563`): `Task details`,
   the title, `Model:`, `Reasoning:`, `Project`, `Branch`, usage lines,
   `Last message`, and a 2-line markdown preview of the prompt. Exactly the
   separation codex makes: the list is for choosing, the details pane carries
   what the row could not show without becoming a wall of text.
8. **Rename inline** — a `Rename > ` editor row replaces the search row.
9. **Removal is graceful** — `selection_after_removal`, `refill_count` and
   `hidden_threads` so deleting a row never leaves the view pointing at
   nothing.

### 4.3 Operant's left pane as it stands

`session_hub/render.rs::render_list`: a bordered `Block` titled `Sessions
[all]`; one two-line `ListItem` per row (title with glyph + optional `🏛`
employee, then `#id`, `N msg`, `k tok`, delegation status, age, `$cost`);
selection is a bare `bg` fill; no tab bar, no counts, no column headers, no
grouping, no sort control, no zebra, no per-row background, no `›`/dot
distinction, no archive action. Search is a single `🔍` row above the body
with per-char highlight.

That is a competent **list**. It is not an **organiser**: against a live DB with
3,226 sessions, nothing answers "what needs me", "what is still running",
"group this by repo", or "sort oldest-first" from the keyboard.

### 4.4 The staged upgrade design (not yet built)

- **Stage A — header + tab bar + rule.** Replaces the `Sessions [all]` title
  with three rows: a bold header naming the view and the grouping mode with its
  key; a filter tab row with live counts derived from `visible_rows()`; a dim
  rule. Reuses `HubFilterMode` verbatim rather than inventing a second filter
  enum.
- **Stage B — column headers + the two-glyph row.** A dim
  `Tasks | Status | Updated` header, and a row that draws `›`/space for
  selection and `○` for identity separately, so a highlighted row never loses
  its selection glyph. Status becomes a right-aligned column; age stops being
  the last thing on a wrapping line.
- **Stage C — grouping with a `g` cycle.** Four modes over the existing row
  set: `Auto` (today's order: pinned → live → interrupted → recent),
  `Directory` (`row.cwd`), `Employee` (`row.employee_id`), `Status`
  (`RowStatus`), each rendering `group heading  N` in dim and sorting by
  `updated_at` descending inside the group. Codex's four are `Project |
  Directory | Status | Model`; operant's axes are its own data's axes, so
  `Directory | Employee | Status | Auto` is the honest port — `Project` would
  need git-identity resolution operant does not yet do, and `Model` would
  render an empty group for every operant row.
- **Stage D — contrast-resolved row backgrounds.** Zebra at a low-alpha lift,
  selection at a higher one, each span's foreground resolved against the
  painted background. Operant has `theme_colors::selection_bg` but never
  resolves the text colour against it.
- **Stage E — sort and archive.** `o` cycles sort (`Updated | Created | Cost |
  Messages`) with the key shown in the header; `a` archives the selected row
  behind an explicit confirm.

### 4.5 Out of scope for the organiser

- codex's multi-backend source fan-out (`sources.rs`'s `SelectAll` over N
  `BackendId`s) and per-source cursor `catalog.rs`. Operant has one backend
  (SQLite): the pagination machinery would be ceremony with no second source to
  fan out to. The `↑/↓ more` affordance is ported in spirit (a `Show more`
  row), not the machinery.
- The alt-screen picker. Operant's hub is an in-TUI overlay, which is the better
  interaction for a session you are already inside, and the operator's
  right-pane preference confirms the overlay choice.
- Live notification feeds, inline rename, worktree creation.

---

## Part 5 — Findings worth recording beyond the code

1. **codex's premium feel is restraint, not chrome.** No border, no shadow, no
   colour on the input surface itself — a 12%-alpha fill, a 2-column gutter, one
   bold glyph, two rows of air. Any port that adds a box has missed the point.
2. **codex's composer and its status surface are separate surfaces.** operant
   already has a status block; adding a second would violate the one-renderer
   rule. The correct port is fill + gutter + placeholder, not the status row.
3. **codex's session organiser is a tab bar, a grouping key and a status
   column** — not a bigger list. Operant's 40/60 geometry is already right; what
   is missing is those three affordances plus contrast-resolved row backgrounds.
4. **operant's right pane is genuinely ahead of codex's.** codex's picker shows
   no session preview in the list (only an expandable transcript preview on the
   selected row in some modes) and its command-center details pane is
   metadata-only. Operant's hub previews the session's own messages. Do not
   trade that away for parity.

---

## Resume here

**Part 3 (the composer) is done and on `origin/main`.** Do not re-plan it. The
remaining work is Part 4's staged upgrade of the session hub's left pane, in
this order — each stage lands as its own iteration:

| stage | change | files | key |
|---|---|---|---|
| **A** | header + live-count tab bar + dim rule, replacing the `Sessions [all]` box title | `session_hub/render.rs`, `session_hub/mod.rs` (reuse `HubFilterMode`) | status row only |
| **B** | column headers (`Tasks / Status / Updated`) + the two-glyph row (`›` selection, `○` identity) | `session_hub/render.rs` | none |
| **C** | grouping with a `g` cycle: `Auto / Directory / Employee / Status` | `session_hub/{mod,render,key_handling}.rs` | `g` |
| **D** | contrast-resolved row backgrounds (zebra + selection), foregrounds resolved against the painted background | `session_hub/render.rs`; reuse `composer.rs`'s blend helper | none |
| **E** | sort (`o`) and archive (`a` behind an explicit confirm) | `session_hub/{mod,render,key_handling}.rs` | `o`, `a` |

Constraints that must survive every stage: the right pane (live session
preview) is preserved untouched, the hub's existing keybinds keep working, and
no stage introduces a second filter enum — `HubFilterMode` is the one.

Verify each stage with the headless sim (`operant tui debug simulate --size
110x40 --dump-screen`) with the session hub open, and check the drawn screen for
the header, the counts, the columns and the selected row.

---

## Stage A — what landed (`feat(iter-1644)`) and its two findings

One shared predicate, `row_matches_filter(mode, row, branch_root, cwd)`, is now
the single definition of what each filter means: `rebuild_visible` uses it for
the visible set and `filter_counts()` uses it for the counts. Two copies of
"what does `active` mean" would have drifted, and a count that disagrees with
the list is worse than no count at all. `filter_counts()` walks `next()` to
derive the tab bar's order, so it cannot disagree with the order `s` cycles in.

The chrome renders as the first two `ListItem`s INSIDE the bordered block, not
above it. That ordering is the finding: drawing them above produced a left pane
whose top rows carried no border glyph at all, which looked broken AND shifted
every border-parity analysis in the tree (the preview-pane tests locate the
pane split by counting the second `│` per row). Both symptoms were caught by
the test suite before this was committed, which is why the border-parity test
exists now — it is the regression test for a mistake that never shipped.

`state.selected` indexes the BODY set, so the rendered selection is
`CHROME_ROWS + selected`. Two chrome rows above the rows is exactly the kind of
offset bug that highlights a different row than the operator is on.

The tab bar degrades by width, as codex's `toolbar_for_width` does: full labels
("all 12  current dir 3  …") while they fit, compact ones ("all 12 cur 3 emp 0
live 0 br 5 arch 2") when the pane is 40% of 120 columns, and the single active
filter when even that does not. The measured defect this fixes: at 48 columns
the six long labels pushed `archived` clean off the edge, and the count the
operator most needed — the filter they had not tried yet — was the silent
casualty. `HubFilterMode::short_label` exists for exactly this ladder and its
forms are all unambiguous at the lead.

Still open, deliberately: Stages C–E. Stage B did not add grouping or sort,
and the header's `Group:` clause is shaped for but not yet wired.


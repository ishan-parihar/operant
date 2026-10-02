# TUI Visual Port — Next-Phase Outline

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `9e54c511` (iter-566)
**Parent**: `.omo/plans/reaudit-tui-faithful-jcode-port.md` (local-only — `.omo/` is
gitignored at `.gitignore:121`, so it is not in history and the next agent must
find it on disk rather than via `git log`)

Every number below was measured at the stated baseline, not carried forward. The
source kit is present at `parent-projects/jcode` @ `0a9dc7805` and the parent's
line citations were spot-checked against it (the 10-band chrome stack at
`ui.rs:3182-3200` and `clear_area` at `chrome.rs:4-10` both match verbatim), so
the parent plan's authority holds.

---

## 0. Where the port actually stands

| Tier-1 item (load-bearing — omission looks BROKEN) | State |
|---|---|
| 5. Selection = 58% accent blend | **done** (iter-566) |
| 4. Code-fence `┌─ lang` / `│ ` / `└─` | not started |
| 1. Composer glyph mode switch (4 states) | not started |
| 2. 10-band chrome stack | not started |
| 3. User bubble bg + rainbow ordinal | not started |

Phases 1, 2 and 4 are untouched. Phase 3 is one slice of four (iter-549 fixed
`modal_frame_buf`). **1 of 5 phases.**

**Correction to the parent's size estimate.** It budgets `jcode-tui-render` at 293
LOC. Measured: the crate is 5,590, of which `chrome.rs` 91 + `layout.rs` 64 +
`lib.rs` 202 = **357** are the generic files. The remaining 4,297 is
`swarm_gallery` / `swarm_tiles` / `memory_tiles` — the swarm visualizer the parent
already says to skip. The estimate was right about what to port and wrong about
the crate.

---

## 1. Phase 1 — Delete the self-inflicted layer *(cheapest; do first)*

`vendor/render_core`, `vendor/workspace` and `vendor/anim` have **zero consumers**
outside `vendor/` itself. The only reference anywhere is a doc comment in
`copy_targets.rs:60`. They are 6,794 LOC of dead code carrying 92 tests that test
nothing operant uses.

| Delete | LOC | Tests inside |
|---|---|---|
| `vendor/render_core/` | 4,608 | 68 |
| `vendor/workspace/` | 1,246 | 24 |
| `vendor/anim/` | 940 | 0 |

`vendor/style/` **stays** — 44 consumer files, and it is jcode's actual palette.

Then remove the bridge that exists only because we adapted instead of ported:
`pin_truecolor_for_tests` (7 sites) and the `color_depth::detect` ↔
`color::color_capability` reconciliation (18 sites).

**Done when**: `cargo check -p operant-cli` clean, and the surviving
`vendor::style` tests still pass. Losing 92 tests is correct — they covered code
nothing calls — but say so in the commit body so a reviewer does not read it as a
regression.

---

## 2. Phase 2 — Port the renderers *(the bulk; blocked on sequencing, not on access)*

The source is available and the deps are already in `Cargo.toml`
(`pulldown-cmark`, `syntect`, `unicode-width`). Nothing here is blocked on
prerequisites.

Land in dependency order, each slice mutation-proven:

| # | Slice | Source LOC | Unblocks |
|---|---|---|---|
| 2.1 | `jcode-tui-render` generic files — `clear_area`, `render_rounded_box`, the two truncate fns, `line_plain_text` | 357 | every overlay |
| 2.2 | `markdown_render_full.rs` + `markdown_render_support.rs` — code fences `┌─ {lang}` / `│ ` / `└─` | 1,585 | **Tier 1 #4** |
| 2.3 | `ui_tools.rs` — `get_tool_summary_with_budget`, 6 truncation variants | 1,687 | Tier 2 #11 |
| 2.4 | `ui_prepare.rs` — role dispatch, user gutter + rainbow ordinal, system force-recolor | 2,721 | **Tier 1 #3** |
| 2.5 | `ui_messages.rs` — `render_assistant_message`, tool rows | 4,480 | Tier 1 #4 consumer |
| 2.6 | `ui_input.rs` composer glyph modes | — | **Tier 1 #1** |

**2.2 before 2.5, and that order is forced.** The `│ ` gutter *is* the copy-target
detection mechanism (`markdown_render_support.rs:42-72`), and `copy_targets.rs`
already depends on that shape. Changing the fence char before the detector is
ported would silently break copy.

**Do not port**: `swarm_gallery` (3,099), `swarm_tiles` (611), `memory_tiles` (587),
`markdown_incremental` (311), `markdown_latex_image` (1,294 — operant has its own
`latex.rs`). That is 5,902 LOC the parent already excluded.

---

## 3. Phase 3 — Composition *(finish what iter-549 started)*

| # | Item | State |
|---|---|---|
| 3.1 | Overlay erase → `Borders::ALL` → content | **done** (iter-549) |
| 3.2 | 10-band chrome stack, replacing `render/mod.rs:136 let reserved: u16 = 1u16` | open |
| 3.3 | Kill the second hint row — `render_status_row` + `render_footer` both draw; jcode has one status band | open |
| 3.4 | `help.rs` routes through the framework | done (iter-549) |

---

## 4. Phase 4 — Collapse the parallel render trees

Four live trees: `render/` (9 files), `messages/` (9), `dialogs/` (4),
`overlays/` (8). Target is one path through the ported layer. **17** files outside
`vendor/` still touch `Block::`/`Borders::` without going through `modal_frame` —
they are chrome, panes and diff chrome rather than modals, so most are legitimately
out of scope; each needs a decision, not a blanket rewrite.

This phase is mechanical but large, and it is the one most likely to produce a
merge conflict if parallelized. Sequence it after Phase 3, not alongside it.

---

## 5. Phase 5 — Verification

The golden corpus is healthy and can carry this: 53 surfaces enumerated, 50 live,
3 parked, 0 missing, 0 extra, 120 baselines (60 text + 45 style + coverage),
`_validate.py` PASSED.

Two gaps to close first:

- **`selection-highlight` is parked**, so the 58% blend shipped in iter-566 has no
  golden coverage. Its stated unblock is a mouse-injection seam on the simulated
  event pump plus a style baseline; neither exists. This is the highest-value gate
  work in the plan, because it is the one place a regression would be invisible.
- **Rebaseline once, deliberately, in its own commit** with a rationale body. The
  current goldens encode operant's layout, not jcode's.

---

## 6. Sequencing

Phase 1 is independent and unblocks nothing else — it only removes. Phase 2 slices
are sequential in the order above (2.2 gates 2.5). Phase 3 depends on 2.1. Phase 4
depends on Phase 3. Phase 5's rebaseline depends on all of it.

**Suggested next iteration: Phase 1.** It is pure deletion, provably safe (zero
consumers, verified), and it removes the incoherence that the parent plan
identifies as the root cause — before adding 15k more lines on top of it.

---

## 7. Standing rules for this port

1. **Every new renderer is mutation-proven.** iter-566 shipped a test that derived
   its own expectation from the constant under test; changing `0.58` → `0.30` kept
   it green. That is the failure mode to design against.
2. **Zero consumers means delete, not keep.** `vendor/` grew to 6,794 LOC of
   speculative lift because nothing forced the question.
3. **Cite the source line.** The parent plan's citations verified exactly; that is
   why its conclusions could be trusted without re-deriving them.
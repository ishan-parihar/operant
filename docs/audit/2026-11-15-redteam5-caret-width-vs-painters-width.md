# Red team 5 — the caret's width vs the painter's width

**Date:** 2026-11-15
**Operator report:** "Using up/dn arrows, only moves the prompt history, not the
cursor in the input box. The mechanics is straightforward, the up/down arrow
navigates the cursor within the lines, until it touches the top/bottom."
**Base:** `origin/main` at `40380253`.
**Method:** live tmux first, then a probe for whatever the live run could not
isolate. The live run is what found this; the earlier two passes were unit-only
and both missed it.

---

## What the live run showed

I drove the real binary in tmux (`--no-mouse`, `display-message` for the caret)
across: a plain multi-line draft, a wrapped single line, a recalled multi-line
entry, a trailing newline, the operator's own `operant.toml`, and both my build
and the deployed one.

**The caret walked correctly in every one of those cases.** So the operator's
report was not reproducible with the shapes I guessed at — which is why this
report leads with the measurement rather than the fix.

What the live run *did* find is a real defect one level down, and it is the kind
that only appears at a specific width:

## Finding — the arrows and the painter used different widths

The caret's visual-row model needs the width the text is **wrapped and painted**
at. The two disagreed:

| | formula | at 80 columns, no send-mode icon |
|---|---|---|
| the painter | `textarea_rect(area).width - send_mode_reserved_width` = `area.width - 2 - 0` | **78** |
| the arrow keys | `area.width - 4` | **76** |

So for any line longer than 76 and at most 78 columns, the painter drew **two**
rows and the arrows believed there was **one**. A caret on the second painted row
had no row above it by the arrows' model, so Up fell straight through to the
history ladder — which is the operator's sentence exactly: "only moves the prompt
history, not the cursor in the input box".

Measured across sizes (`/tmp/wprobe2`, 40 combinations of area width × reserved):
the old formula disagreed with the painter in **30 of 40** cases; the new one
disagrees in **0**.

The same seam explains why my earlier passes missed it. Every unit test I wrote
set `last_input_area` by hand and then asserted against `cursor_visual_pos` with
a width I also chose, so the two sides of the disagreement were never in the same
test. That is the "test that agrees with itself" failure mode, and the live run is
what exposed it.

## Fix

One function owns the number:

```rust
pub(crate) fn composer_text_width(app: &dyn TuiState, area_width: u16) -> usize
```

It is derived from **the same rect and the same reservation the painter uses**, so
"wrapped to width" and "painted at width" are the same number by construction and
cannot drift again. `App::composer_width()` delegates to it, and the two visual
walks (Up and Down) plus the three Shift+arrow selection arms all call the helper
— six call sites, one formula.

## Also fixed in this pass

`App::composer_width` was private, so the tests could not reach it and had to
re-derive the width themselves. That is precisely how the disagreement survived
two audits: the test could not call the thing it was meant to pin. It is
`pub(crate)` now, and the width test drives it directly.

## Regression tests (3)

| test | pins |
|---|---|
| `the_arrows_and_the_painter_agree_on_the_composers_width` | the helper equals the painter's arithmetic at 9 widths |
| `up_walks_a_row_that_only_exists_at_the_painters_width` | the 79-column case: two painted rows, and Up walks |
| `the_render_path_publishes_a_usable_input_area_width` | the render path really publishes a non-zero rect (the other way the walk silently dies) |

Mutation-proven: restoring `area.width - 4` fails **both** width tests
(`left: 16, right: 18` at area 20 — the exact shape of the bug).

## Live verification

tmux, 80×24, real binary, caret read with `tmux display-message`:

| case | result |
|---|---|
| 79-column line, 2 painted rows | Down: no-op (already last row). Up: walks 19→18. **correct** |
| 4-line draft with 3 prompts of history | Up walks 21→20→19→18→17, and the 5th Up recalls history from the top row. **correct** |
| 3 visual rows (79-char line + a second line) | three Ups walk three rows. **correct** |

## What this pass does not claim

- **The report's literal symptom was not reproduced** on any shape I built. The
  width defect above is real and matches the description, but I cannot prove it
  is the one the operator hit. If it persists after this, the next thing to
  measure is the operator's actual terminal width and draft, live.
- **`/queue`, `/steer` and the Enter chords were not touched** — they were
  verified correct in the two previous passes and the live run did not contradict
  that.
- **No change to the browsing-live precedence.** A recalled multi-line entry
  still steps the ladder rather than being walked; that is the 2026-11-12
  decision, still pinned, still deliberate.

## Gates

`cargo test -p operant-cli --bin operant`: **zero** new failures against a
baseline taken in this worktree (6 pre-existing, all peer-file: `cmd_channel`,
`dashboard_server`, `gateway_runner`, `advisor_card` ×2, `commands`).
`cargo clippy -p operant-cli --bin operant`: **zero** warnings in the touched
files.
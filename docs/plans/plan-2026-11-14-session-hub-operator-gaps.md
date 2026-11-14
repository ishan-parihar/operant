# Plan — close the session-hub gaps the operator reported

**Date:** 2026-11-14 (revision 2 — supersedes the first outline)
**Base:** `origin/main` @ `6c59b3b0`
**Status:** outline. Nothing here is executed by writing this file.

## Status against the operator's seven items

| # | Item | State |
|---|---|---|
| — | Filter tags garbled (`n 30 w 0 r 3292`) | **closed** — `iter-2317` (three-way counts, no-counts rung) |
| 1 | Preview filters: full / user-only / user+end-turn (default) | **closed** — `iter-2328` + `iter-2338` |
| 2 | Panes must be non-redundant; list shows only what identifies a session | open |
| 3 | Date and runtime in the data | open |
| 4 | Left list segregated by path/project, codex-style | open |
| 5 | Preview always shows the BOTTOM of the session | **closed** — `iter-1906b4db` |
| 6 | Left/right layout rebalance | open |
| **new** | Preview needs a fixed, non-scrolling header strip | **open — found while executing item 1** |

The new row is not scope creep. It is a defect the closed items exposed, and two
open items depend on it. See iteration 1 below.

## The fact that reorders this plan

Measured at `render.rs:1314-1641`: **the preview renders one `Paragraph` that
holds the header AND the transcript, and `.scroll((scroll, 0))` moves the whole
thing.**

    let lines: Vec<Line> = match (...) { ... };     // header lines + transcript
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);

So there is no fixed region in the preview pane. Every line the pane shows —
id, employee, project path, cost, and the `first:` prompt line — scrolls away
with the transcript.

That single fact explains a bug I shipped and had to record as open in
`iter-2338`: the `first:` line was the first line of the scrolling transcript,
and once the pane began following the BOTTOM (item 5), that line became
unreachable on any transcript longer than the viewport. Its purpose is to
surface a prompt that scrolled off the head; it was scrolled off with
everything else.

It also means **item 3 cannot be done as specified.** A runtime that must
advance while a session is live has to live in a region that is always on
screen. Put it in the scrolling transcript and it is a fact the operator sees
only by scrolling to it.

## Iterations, in execution order

### Iteration 1 — a fixed header strip in the preview pane

**Why first:** it is the defect the closed items exposed, and it is the
substrate item 3 needs. It is small, and it is the only item that makes two
others possible.

**Shape:** split the preview's inner area vertically before rendering —
`Layout::default().direction(Vertical).constraints([Length(header), Min(1)])` —
render the header as its own `Paragraph` into the first chunk, and the
transcript as a scrolling `Paragraph` into the second. The header must NOT
scroll.

**What moves into it:** the header lines the pane already builds (title, id,
employee, `project:`, cost, message count), plus the `first:` prompt line and
the active `PreviewFilter` label. Nothing new is invented; the lines already
exist, they are just rendered into the wrong region.

**Measured constraint:** the header currently occupies roughly 7-8 lines of a
pane that is `Min(8)` tall in the body plus chrome. At the smallest realistic
terminal the body must keep at least 3-4 transcript lines or the pane stops
being a transcript. Measure the header height at 100 columns before fixing the
`Length`, and cap the header — a session with a long `project:` path must not
consume the transcript.

**Pin:** a 400-message transcript renders with the header still visible at
scroll = max (this is the exact case `iter-2338` could not express); the header
line count is identical at scroll 0 and at max scroll; and at 100 columns the
body retains at least 3 lines.

### Iteration 2 — deduplicate the two panes

**Operator's words:** "the data on the right and the left panes must be
non-redundant. Currently, there are several duplicates. The list must only show
those items that must help the session to be identified."

**The duplication, measured:**

| Field | List row | Preview header |
|---|---|---|
| id | `#{short_id}` | `id: {full}` |
| age | `just now` | `{relative_time}` |
| cost | `$12.3400` | `${:.4}` |
| messages | `120 msg` | `{} msg` |

**The decision the operator still has to make.** My recommendation, unchanged:

- **List keeps:** title, directory qualifier, id, employee, status, age — what
  lets you pick a row out of 3,000.
- **List drops:** message count, token count, cost. They do not identify a
  session, and they are what push the row past its budget — the `metrics_width`
  ladder at `render.rs:996` exists solely to make room for them.
- **Preview keeps:** cost, messages, tokens — the session's weight, read after
  picking.
- **Preview stops repeating:** the id. The list already carries it.

**Sequence:** after iteration 1. Dropping three fields is what pays for the
header strip's height at narrow widths, so doing this first makes iteration 1
cheaper; but iteration 1 is the smaller, more certain change and should not wait
on a product decision.

**Pin:** the list row's meta line contains no `$` and no `msg`; the preview
header contains both; and the row still fits 46 cells with the longest field
combination, measured by the ladder's own arithmetic.

### Iteration 3 — date and runtime

**Operator's words:** "no date, runtime (the time the session has spent
executing) in the data."

**What exists:** `sessions.started_at` and `sessions.ended_at` are both columns.
The row shows only a relative age, so a session from three weeks ago and one
from three days ago read alike.

**Shape:**

- **List row:** the absolute date, once iteration 2 frees the width. Format as a
  date, not an instant — the row is scanned, not read.
- **Preview header:** the runtime — `ended_at - started_at`, or `now -
  started_at` while live. Same magnitude shape as `tokens_suffix`
  (`1h 12m`, `45s`, `3d 4h`).
- **A live runtime must be derived at the read site from the clock.** A duration
  written once at load is frozen at its first value; that is the stored-animation
  pitfall, and it is why this item needs the fixed header from iteration 1
  rather than a line in the transcript.

**Data caveat, measured earlier this campaign:** 3,308 of 3,314 live rows have
no `started_at`. The honest rendering for those rows is an absent field, not a
zero and not a guessed duration. Say so in the header rather than printing
`0s`.

**Pin:** a known `started_at`/`ended_at` pair renders the exact duration; a live
session's duration differs between two frames; a NULL `started_at` renders
nothing.

### Iteration 4 — project sections in the left list

**Operator's words:** "the left-list must also be segregated based on
path/project just like codex."

**What exists:** `HubGroupMode { Auto, Directory, Employee, Status }` already
computes a directory grouping. Codex's picker goes further: it **sections** —
a heading per project with a count, and the rows beneath it do not repeat the
project name.

**Shape:**

- `Directory` renders as a sectioned list rather than a flat run with a heading
  row.
- The heading carries the project **basename** and its count; rows beneath do not
  repeat the basename — the same redundancy as iteration 2, one level up.
- `Auto` selects project sections when the loaded rows span more than one
  directory. That is the codex behaviour and the answer to "the aggregate of all
  sessions is confusing to navigate".

**Risk:** this is the change most likely to overflow the 46-cell pane, because it
adds an indent level. Measure the indent cost before writing the render.

**Not included:** collapse/expand per section. Not requested; leave it out until
asked.

**Pin:** three directories produce three headings in order; a row under a
heading does not repeat the basename; each heading's count equals the rows
beneath it; the heading plus the longest row fits 100 columns.

### Iteration 5 — rebalance the panes

**Operator's words:** "left/right side needs rebalancing of layout."

**What exists:** `SPLIT = (Percentage(40), Percentage(60))` at `render.rs:24`.

**Why last:** the split follows the content, so it cannot be chosen until the
other four land. Iteration 4 adds width pressure to the left (indent,
headings); iteration 1 adds a header strip to the right; iteration 2 removes
three fields from the left row.

**Recommendation:** 45/55, then verify.

**Mandatory check:** the tab-bar ladder derives its rungs from the width, so
changing the split changes which rung is chosen at every terminal width. Re-run
the `iter-2317` narrow-terminal test at 100, 110 and 120 columns after the
change. Do not assume the ladder adapts — that assumption is what produced both
layout regressions this campaign (`iter-2207` collapsed the bar to one pill,
`iter-2252` fitted the fix to the wrong quantity).

## Standing rules for every iteration

From `rust-tui-debugging`, the clauses that have already bitten this campaign:

1. **Take the width budget from the layout call, not a constant.** The list pane
   is `SPLIT.0` = 40%, so inner cells = `terminal * 0.4 - 2`: 46 at 120 columns,
   **38 at 100**.
2. **A second report of the same visual defect means the first fix was fitted to
   the wrong quantity.** Restart from the cell budget and the reported width.
   This is exactly what happened between `iter-2252` and `iter-2317`.
3. **Reproduce constrained geometry explicitly** — the smallest realistic
   terminal, including borders and the sibling pane's share. A test at 160
   columns passes over a defect that fires at 100.
4. **Repair a red gate as its own iteration.** The bin test target currently
   compiles for the hub suite but not whole: a peer's uncommitted
   `gateway_runner/tests.rs` (initializers missing `live_sessions` and
   `running`) breaks it. Any item verified only by `rustfmt` is not verified.

## What this plan deliberately does not do

- No change to the page cursor's RFC3339 text comparison (latent: every
  session-timestamp writer uses `to_rfc3339()` over `Utc::now()`; all 3,314 live
  rows are `+00:00`, zero `Z`-suffixed, zero space-separated).
- No change to the cwd stamp's last-turn-wins semantics — a deliberate
  trade-off recorded in `iter-2273`, with the one-line alternative documented
  there.
- No section collapse/expand.
# Plan — close the session-hub gaps the operator reported

**Date:** 2026-11-14 (revision 3 — supersedes revisions 1 and 2)
**Base:** `origin/main` @ `4ec61e0e`
**Status:** outline. Writing this file executes nothing.

## Status against the operator's seven items

| # | Item | State |
|---|---|---|
| — | Filter tags garbled (`n 30 w 0 r 3292`) | **closed** — `iter-2317` |
| — | Preview opens at the BOTTOM of the session | **closed** — `iter-1906b4db` |
| 1 | Preview filters: full / user-only / user+end-turn (default) | **closed** — `iter-2328` + `iter-2338` |
| — | Preview header pinned (found while executing 1) | **closed** — `iter-2341` |
| 2 | Panes non-redundant; list shows only what identifies a session | **closed** — `iter-2343` |
| 3 | Date and runtime in the data | open — **split into 3a and 3b, see below** |
| 4 | Left list segregated by path/project, codex-style | open |
| 5 | Left/right layout rebalance | open |

Three items remain. One of them (3) has been measured into two halves that are
no longer the same size of job, and that is the main change in this revision.

## What iteration 2 changed about the budget

The row's meta line is now `#{id}  {employee}  {status}  {age}`. Measured against
the 46 inner cells a 120-column terminal gives the list pane:

| employee | status | age | cells |
|---|---|---|---|
| — | Idle | 3d ago | 30 |
| — | Interrupted | just now | 39 |
| premiere | Idle | just now | 40 |
| premiere | Interrupted | just now | **47 — over** |

So after shedding three fields the row is **still one cell over** in its worst
case, and it has no slack for anything new. That is not a problem — it is what
the `id_budget` ladder is for, and it already shrinks the id from 12 toward 3 to
pay for the line — but it does mean **no new field can be added to the row
without the ladder absorbing it**. Any iteration that adds to the row must state
what pays.

## Iteration 3a — the absolute date

**Operator's words:** "no date, runtime (the time the session has spent
executing) in the data."

**Measured: this half is nearly free.** `SessionEntryV2.updated_at` is already a
parsed `DateTime<Utc>` (`listing.rs:84`); the picker merely formats it as a
relative phrase. The date is a format change, not a data change. No SQL, no new
field, no migration.

**The decision this forces:** relative or absolute on the row. The operator's
complaint is that relative time is too vague — a session from three weeks ago
and one from three days ago read alike. So the row shows the **absolute date**
(`2026-11-11`), and the preview header keeps the relative phrase beside it, where
there is width. Swapping rather than adding costs +3 cells in the worst case
(`2026-11-11` is 10 cells against `just now`'s 7), which the id ladder absorbs.

**Why the date belongs on the row at all:** it is identity, not weight. Two
sessions with the same title are told apart by when they ran. That is the same
test iteration 2 applied to cost and message count, and the date passes it.

**Pin:** a row whose `updated_at` is three weeks old shows the date, not "3w
ago"; the preview header shows both; and the worst-case row still fits 46 cells
with the id ladder's help, asserted at the ladder's own arithmetic.

## Iteration 3b — the runtime

**Measured: this half is NOT free, and the plan was wrong to treat it as one
item.**

`SessionEntryV2` has **no `started_at`** (`listing.rs:75-99`). It has
`updated_at`, documented as "parsed `ended_at` (falls back to `started_at` when
the session never ended)". So the struct cannot answer "how long did this run" —
it can only answer "when did it last change".

The runtime therefore needs:

1. `started_at: Option<DateTime<Utc>>` on `SessionEntryV2`.
2. The column in the `SELECT` of `list_sessions_rich_filtered_page` (and the
   `RichSessionRow` it maps through), which is the one SQL string every surface
   shares.
3. `ended_at` too, or a derived duration — `updated_at` is already
   `COALESCE(ended_at, started_at)`, so `ended_at` and `started_at` together are
   what `updated_at` is standing in for. Naming the two real columns is clearer
   than deriving from the coalesce.
4. A live row (`ended_at IS NULL`) computes `now - started_at` **at the read
   site**. A duration written once at load is frozen at its first value.

**Where it renders:** the pinned header (`iter-2341`), not the row. The row is
already one cell over in its worst case and a duration is the widest field in the
set. The header is the right home for a fact that changes while you watch it.

**Data caveat, measured earlier this campaign:** 3,308 of 3,314 live rows have no
`started_at`. The honest rendering is an absent field — not `0s`, and not a
duration guessed from `updated_at`. Say so in the header.

**Pin:** a known `started_at`/`ended_at` pair renders the exact duration; a live
row's duration differs between two frames; a NULL `started_at` renders nothing at
all; and the header's line count is unchanged for a row with no runtime.

## Iteration 4 — project sections in the left list

**Operator's words:** "the left-list must also be segregated based on
path/project just like codex."

**What exists:** `HubGroupMode { Auto, Directory, Employee, Status }` already
computes a directory grouping. Codex's picker goes further: it **sections** — a
heading per project carrying its count, and the rows beneath it do not repeat the
project name.

**Shape:**

- `Directory` renders as a sectioned list rather than a flat run with a heading
  row.
- The heading carries the project **basename** and its count; rows beneath do not
  repeat the basename. That is iteration 2's redundancy rule one level up, applied
  before the fact rather than after.
- `Auto` selects project sections when the loaded rows span more than one
  directory. That is the codex behaviour and the direct answer to "the aggregate
  of all sessions is confusing to navigate".

**The budget question, and it is now sharper:** the heading adds an indent level
to every row beneath it. Measured, the worst-case row is already 47 cells against
46. An indent of two cells puts it at 49 before the id ladder runs, so the ladder
will shrink the id to 3 on those rows. That is acceptable but it must be
**measured, not assumed** — assert the worst case with the indent applied.

**Not included:** collapse/expand per section. Not requested; leave it out until
asked.

**Pin:** three directories produce three headings in order; a row under a heading
does not repeat the basename; each heading's count equals the rows beneath it; the
worst-case row with the indent applied still fits 46 cells.

## Iteration 5 — rebalance the panes

**Operator's words:** "left/right side needs rebalancing of layout."

**What exists:** `SPLIT = (Percentage(40), Percentage(60))` at `render.rs:24`.

**Why last:** the split follows the content, and the content is still moving.
Iteration 3b adds a line to the right pane's header; iteration 4 adds an indent
to the left pane's rows.

**Recommendation:** 45/55.

**Mandatory check, and the reason it is mandatory:** the tab-bar ladder derives
its rungs from the width, so changing the split changes which rung is chosen at
every terminal width. Re-run the `iter-2317` narrow-terminal test at 100, 110 and
120 columns after the change and assert all seven filter names remain readable.
Assuming the ladder adapts is exactly what produced `iter-2207` (the bar collapsed
to one pill) and `iter-2252` (the fix fitted to the wrong quantity).

## Standing rules for every iteration

From `rust-tui-debugging`, the clauses that have already bitten this campaign:

1. **Take the width budget from the layout call, not a constant.** The list pane
   is `SPLIT.0` = 40%, so inner cells = `terminal * 0.4 - 2`: 46 at 120 columns,
   **38 at 100**.
2. **A second report of the same visual defect means the first fix was fitted to
   the wrong quantity.** Restart from the cell budget and the reported width.
3. **Reproduce constrained geometry explicitly** — the smallest realistic
   terminal, including borders and the sibling pane's share.
4. **Repair a red gate as its own iteration.** The bin test target does not
   currently compile whole: a peer's uncommitted `operant_model`/`ui_tools.rs`
   work imports a `tool_display` module that does not exist yet. `iter-2343` is
   committed on rustfmt alone for exactly this reason. Any item verified only by
   rustfmt is not verified.

## What this plan deliberately does not do

- No change to the page cursor's RFC3339 text comparison (latent: every
  session-timestamp writer uses `to_rfc3339()` over `Utc::now()`; all 3,314 live
  rows are `+00:00`, zero `Z`-suffixed, zero space-separated).
- No change to the cwd stamp's last-turn-wins semantics — a deliberate trade-off
  recorded in `iter-2273`, with the one-line alternative documented there.
- No section collapse/expand.
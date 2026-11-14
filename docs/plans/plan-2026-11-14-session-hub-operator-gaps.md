# Plan — close the session-hub gaps the operator reported

**Date:** 2026-11-14
**Base:** `origin/main` @ `1906b4db`
**Status:** outline — nothing here is executed yet.

## Why this plan exists

The operator reported seven gaps after the codex command-center port landed.
Two are closed (`iter-2317` readable filter tags, `iter-1906b4db` preview opens
at the bottom). Five remain. This plan orders them, names the seam each one
touches, and states how each is pinned — because two of the five are layout
changes, and layout is where this campaign has already shipped two regressions
(`iter-2207` collapsed the tab bar; `iter-2252` fitted the fix to the wrong
width).

## The rule this plan runs under

From `rust-tui-debugging`, the three clauses that have already bitten this
campaign:

1. **Take the width budget from the layout call, not from a constant.** The list
   pane is `SPLIT.0` = `Percentage(40)`, so inner cells = `terminal * 0.4 - 2`.
   Every strip that lives in that pane is measured at that number.
2. **A second report of the same visual defect means the first fix was fitted to
   the wrong quantity.** Restart from the cell budget and the reported width.
3. **Reproduce constrained geometry explicitly** — the smallest realistic
   terminal, including borders and the sibling pane's share. A test at 160
   columns passes over a defect that fires at 100.

## Measured facts this plan stands on

| Fact | Value | How measured |
|---|---|---|
| List pane split | `Percentage(40)` of the frame | `render.rs:24` |
| Inner cells at 120 cols | 46 | `120*0.4-2` |
| Inner cells at 100 cols | 38 | `100*0.4-2` |
| Group modes that exist | `Auto, Directory, Employee, Status` | `mod.rs` enum |
| Preview filter state that exists | **none** | grep, 0 hits |
| Row meta fields | `#{id}  🪪 emp  N msg • Nk  status  age  $cost` | `render.rs:1068-1090` |
| Preview header fields | `id: … · age · $cost · N msg` | `render.rs:1376` |

The last two rows are the duplication the operator is complaining about: **id,
age, cost and message count are printed in both panes**, and the preview repeats
the row's identity rather than adding anything.

---

## Iteration 1 — preview filters (the largest gap)

**Operator's words:** "the right session preview must also have the filters for
`[full-session (shows all tool calls), user-messages, user-messages+end-turn-messages
(set this as default)]`"

**What exists:** nothing. `preview_filter` has zero occurrences in
`session_hub/mod.rs`.

**Shape:**

- Add `PreviewFilter { Full, UserOnly, UserAndTurnEnd }` to `session_hub/mod.rs`,
  defaulting to `UserAndTurnEnd`.
- `Full` is today's behaviour and is also what `hide_tool_calls = false` means —
  so the existing `hide_tool_calls` flag becomes a *derived* consequence of
  `Full` rather than a second axis. Two axes for one question is how the pane
  started contradicting itself (`iter-2230`); do not repeat that.
- "end of turn" is the existing turn-boundary concept, not a new one — the
  preview already renders an end-of-turn marker, so reuse that predicate rather
  than inventing a second definition of where a turn ends.
- The filter is per-session, not global: the operator filters one transcript
  they are reading, and a global filter would silently re-scope the next session
  they open.
- Bind a key in the preview pane's focus context only, and paint the active
  filter in the preview header so it is visible rather than remembered.

**Pin:** render a transcript containing tool calls, tool results, assistant
turns and end-of-turn markers; assert for each of the three filters which lines
appear. Assert the default is `UserAndTurnEnd` on a freshly loaded preview, and
that switching sessions resets to the default.

**Risk:** the filter must not change the scroll anchor. The preview now follows
the bottom (`iter-1906b4db`); a filter that changes the line count while
`preview_follow_bottom` is set must re-clamp, or the operator lands mid-transcript.

---

## Iteration 2 — deduplicate the two panes

**Operator's words:** "the data on the right and the left panes must be
non-redundant. Currently, there are several duplicates. The list must only show
those items that must help the session to be identified"

**The duplication, measured:**

| Field | List row | Preview header |
|---|---|---|
| id | `#{short_id}` | `id: {full}` |
| age | `just now` | `{relative_time}` |
| cost | `$12.3400` | `${:.4}` |
| messages | `120 msg` | `{} msg` |

**The decision the operator has to make, and my recommendation:** the list row
answers *which session is this*, the preview answers *what happened in it*.
Identity belongs to the list; detail belongs to the preview.

- **Keep in the list:** title, directory qualifier, id, employee, status, age.
  Those are what let the operator pick a row out of 3,000.
- **Drop from the list:** message count, token count and cost — they do not help
  identify a session, and they are what push the row past its width budget (the
  `metrics_width` ladder at `render.rs:998` exists only to make room for them).
- **Keep in the preview header:** cost, message count, tokens — the session's
  weight, which is what you read once you have picked it.
- **Add to the preview header:** nothing that the list already says. The preview
  should stop repeating the id and instead show what the list cannot.

**Pin:** assert the list row's meta line does not contain `$` or `msg`; assert the
preview header does. Assert the row still fits 46 cells with the longest field
combination, using the ladder's own measurement.

**Note:** this is the only item that needs a product decision before code. If the
operator wants cost in the list, the ladder has to drop something else — say
which.

---

## Iteration 3 — date and runtime

**Operator's words:** "no date, runtime (the time the session has spent
executing) in the data"

**What exists:** `sessions.started_at` and `sessions.ended_at` are both columns;
the row shows only a relative age (`just now`), which answers "when" so vaguely
that a session from three weeks ago and one from three days ago read alike.

**Shape:**

- List row: keep the relative age (it is scannable) and put the **absolute date**
  where it cannot be confused — the row's title line already carries the
  directory qualifier, so the date belongs on the meta line only if iteration 2
  frees the width. **Sequence this after iteration 2**, or it will not fit.
- Preview header: **runtime** — `ended_at - started_at`, or `now - started_at`
  while the session is live. Format as `1h 12m` / `45s` / `3d 4h`, the same
  shape `tokens_suffix` uses for its magnitudes.
- A live session's runtime must advance. That means deriving it at the read site
  from the clock, not storing a formatted string — the `rust-tui-debugging`
  pitfall "a stored animation frame is a producer you have to keep alive" applies
  exactly: a runtime written once at load is frozen at its first value.

**Pin:** a session with a known `started_at`/`ended_at` pair renders the exact
duration; a live session (no `ended_at`) renders a duration that advances across
two frames; a session with a NULL `started_at` (legacy rows — 3,308 of 3,314 are
in that state today) renders nothing rather than a wrong number.

**Data caveat:** only rows with `started_at` set can show a runtime. Measure how
many live rows have it before promising the field; the preview's own
`first_prompt` column shows the same legacy problem, and the honest answer for
old rows is an absent field, not a zero.

---

## Iteration 4 — project sections in the left list

**Operator's words:** "the left-list must also be segregated based on
path/project just like codex"

**What exists:** `HubGroupMode::Directory` groups by directory, and codex's own
picker goes further — it **sections** the list, with a heading per project and
the project name on each row, so the operator reads a small number of projects
rather than a flat run of 3,000 sessions.

**Shape:**

- `Directory` already computes the grouping; the gap is that it renders as a
  flat list with a heading row rather than as a sectioned list.
- The section heading needs the project **name** (the basename) and its count,
  and the rows under it should not repeat the basename — that is the same
  redundancy the operator flagged in item 2, one level up.
- `Auto` should pick project sections when the loaded rows span more than one
  directory, which is the codex behaviour and the answer to "the aggregate of
  all sessions is confusing to navigate".
- Collapsing a section is the obvious next affordance, but it is not requested —
  leave it out until asked (YAGNI).

**Pin:** rows spanning three directories render three headings in order; a row
under a heading does not repeat the heading's basename; the count on each heading
equals the rows beneath it; and at 100 columns the heading plus the longest row
still fits the pane.

**Risk:** this is the change most likely to overflow the 46-cell pane, because it
adds a level of indentation. Measure the indent cost before writing the render.

---

## Iteration 5 — rebalance the panes

**Operator's words:** "left/right side needs rebalancing of layout"

**What exists:** `SPLIT = (Percentage(40), Percentage(60))`.

**Shape:** the right split is a judgement about what the operator reads most, so
it should be measured rather than guessed. Two facts decide it:

- After iteration 4 the left pane carries sections, so it needs *width* for the
  indent and the project name.
- After iteration 1 the right pane carries a filter strip and a bottom-anchored
  transcript, which reads better narrow (transcripts wrap).

**Recommendation:** `Percentage(45)` / `Percentage(55)`, then verify at 100, 120
and 160 columns that (a) the tab bar still finds a readable rung — the budget
changes with the split, so the whole ladder must be re-measured, not assumed —
and (b) the section headings do not truncate.

**Pin:** re-run the iteration-2317 narrow-terminal test at the new split. The
ladder is derived from the width, so it should adapt, but that is exactly the
assumption that produced two regressions — prove it.

**Do this last.** Every other iteration changes what is in each pane, so the
right split cannot be chosen until they land.

---

## Order and why

1. **Preview filters** — independent of layout, largest operator value.
2. **Pane deduplication** — frees the width that items 3 and 4 need.
3. **Date and runtime** — depends on 2 for room.
4. **Project sections** — depends on 2 (no repeated basename) and 3 (the date has
   to live somewhere).
5. **Rebalance** — depends on all four, because the split follows the content.

## Cross-cutting: the gate is currently red

Every iteration above needs the bin test target, and it does not compile: a
peer's uncommitted `cmd_continue.rs` (a `warn!` with no import) and
`gateway_runner/tests.rs` (a field removed from `GatewaySessionHandler`) break it.
Per `rust-tui-debugging` item 10, **repair the red gate first, as its own
iteration**, or every behavioural item below is verified against a suite that
cannot run.

## What this plan deliberately does not do

- No change to the page cursor's RFC3339 text comparison (latent; every writer
  is UTC, all 3,314 live rows are `+00:00`).
- No change to the cwd stamp's last-turn-wins semantics (a deliberate trade-off
  recorded in `iter-2273`; the one-line change is documented there).
- No section collapse/expand (not requested).
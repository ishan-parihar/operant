# Red team 4 + ponytail audit — attacking iter-2353's own execution

**Date:** 2026-11-15
**Attacked:** `fix(iter-2353): Up/Down walked no lines, and the queue refused
without an agent` (`9e31dbf7`), its audit
(`2026-11-14-updown-line-first-and-enter-chords.md`) and its four ledger rows.
**Base:** `origin/main` at `9b1032ee`.
**Method:** every finding below is a probe run against the shipped code. No
finding is a reading.

---

## Why this pass

The operator asked for the same treatment iter-2353 gave its own subject: red
team the *execution* of the plan, then a ponytail audit for feature/operational
gaps and redundancies. So the subject here is my own work — are the tests real,
are the claims true, and what did I not build.

---

## What survived the attack (verified, no change)

| claim | verdict | evidence |
|---|---|---|
| All 15 claimed tests exist and pass | **true** | enumerated by name in the tree |
| Up walks a 5-line paste 4 rows at width 100 | **true** | `up_walks_a_paste_four_rows_at_a_realistic_width` |
| Down is no longer a dead key on a wrapped draft | **true** | `down_walks_a_wrapped_single_line_draft` |
| Up recall → top-start, Down recall → bottom-end, on **every** step | **true** | `a_recall_going_forward_puts_the_caret_at_the_bottom_end` walks two steps |
| The queue works with no agent **and is deliverable** | **true** | `a_queue_made_without_an_agent_is_delivered_exactly_once` |
| `/queue` fallback works with no agent | **true** | `the_slash_queue_fallback_works_without_an_agent` |
| The composer width is `area.width - 4` | **true** | `up_at_the_wrap_boundary_walks_then_recalls` fails under mutation |
| Vim Normal mode obeys the same rule | **true** | `up_walks_a_line_in_vim_normal_mode_too` |
| The unenhanced hint does not promise Shift+Enter | **true** | `the_unenhanced_hint_says_nothing_about_a_chord_it_cannot_deliver` |

## Finding 1 — the ledger row overstated what shipped (fixed)

**Confirmed, my own defect.** RT-573's closure said *"both arms try
`move_visual_up`/`move_visual_down` first and reach the ladder only when the walk
cannot move"*.

That is **false during a browse.** `history_is_live()` is checked *before* the
visual walk in both arms, so a recalled multi-line entry steps the ladder on the
very first Up — the walk is never reached. I read the code after writing the row
and did not re-verify the sentence against it.

The row now states the exception, cites the 2026-11-12 decision that made it, and
names the test that pins which rule wins.

## Finding 2 — a recalled multi-line entry cannot be walked at all (gap, documented not fixed)

**Confirmed and unchanged, but now pinned.** iter-2353 put the recall caret at
the top-start; RT-560 makes an edit end the browse; the browsing-live branch
precedes the visual walk. Together:

- Recall a multi-line entry → caret at top-start → Up steps further back. The
  entry's own lines are unreachable.
- Edit the recalled entry → browse ends → the next Up starts a **fresh** browse
  from the newest entry. Still unreachable.

So the one case the operator cannot escape is a multi-line entry they want to
read before sending. This is the **2026-11-12 decision** (a browse must keep
stepping so the operator can walk back out), deliberately kept, and it was
already pinned by `a_recalled_multiline_entry_still_steps_the_history_ladder`.

What was missing is that the interaction was never **measured**. Two new tests
pin both halves: `a_recalled_multiline_entry_steps_the_ladder_instead_of_being_walked`
and `an_edited_recall_survives_the_fresh_browse_it_triggers`.

**Not fixed**, because "which rule wins" is a product decision, not a bug:
making the walk win would strand the operator mid-ladder with no way back out.
The honest resolution is a modifier (Alt+Up to walk a recalled entry), which the
operator did not ask for and which would add a key they would have to learn.

## Finding 3 — redundancy: the recall writers repeated themselves three times (fixed)

**Confirmed.** `history_up` and both `history_down` branches each carried their
own 6-line tail: drop the store, set the text, set the caret, clear
`preferred_col`, set `history_recall`, estimate tokens, suppress the popup, clear
the suggestions. Three copies, differing only in the caret.

`install_recall(&str, RecallEdge)` is now the one owner, and the three invariants
it enforces are stated once instead of three times. `RecallEdge` is an enum
rather than a `bool` so the call site reads as the gesture (`RecallEdge::Top` vs
`RecallEdge::Bottom`) rather than as a flag.

## Finding 4 — redundancy: six copies of the composer width (fixed)

**Confirmed.** `let area = self.last_input_area.get(); let width =
area.width.saturating_sub(4) as usize;` appeared **six times** across the two
arms (three in Up, three in Down). One `composer_width()` helper now owns the
deduction, with the reason for each part documented (the `4` is the composer's
chrome; a zero width means never laid out and must skip the walk, not walk zero
columns).

## Finding 5 — the ledger's "nine regression tests" undercounted the audit's own output (fixed)

**Confirmed, cosmetic.** The audit's table lists nine tests and the red-team
section lists six attacks, but the body text says "nine regression tests"
without noting that the six attacks are also kept as tests. Clarified.

---

## Audited and found sound, deliberately not changed

- **`Ctrl+N`** is a ladder step guarded by `history_pos.is_some()`, so it cannot
  start a browse. It is a readline alias, not a fourth Up/Down. Correct as-is.
- **`Ctrl+P`** is already honestly de-catalogued (iter-429).
- **`/queue` and `/steer` ARE registered** (`app.rs:380`, `:394`). I initially
  believed they were missing from `REGISTERED_COMMANDS` and wrote a probe to that
  effect; the probe failed, the registration is real, and the probe was removed.
  Recording the correction here because the wrong version was in my working notes.
- **The unenhanced-terminal hint** is honest: it offers `/queue <text>` and says
  nothing about Shift+Enter, which that terminal also cannot deliver.
- **Up with no history and one row** is a silent no-op. Standard readline; pinned
  so it is a decision.

## Not a gap (checked)

The plan claimed the Enter chords were "already right" and did not change them.
That claim held under attack: all six combinations (three chords × two streaming
states) are correct, and the explanation for the operator's report remains
terminal capability, not dispatch.

---

## Regression tests added (12)

| test | pins |
|---|---|
| `a_recalled_multiline_entry_steps_the_ladder_instead_of_being_walked` | finding 2, first half |
| `a_recalled_entry_can_be_revisited_and_the_draft_always_returns` | the gap costs reading, not data |
| `an_edited_recall_survives_the_fresh_browse_it_triggers` | RT-560 + iter-2353 interaction |
| `up_walks_a_paste_four_rows_at_a_realistic_width` | the operator's report, at width 100 |
| `the_width_deduction_is_the_composer_chrome` | the `-4`, pinned via the wrap model |
| `a_never_laid_out_composer_skips_the_walk_entirely` | the `width > 0` guard |
| `up_walks_a_line_in_vim_normal_mode_too` | the rule is mode-independent |
| `a_queue_made_without_an_agent_is_delivered_exactly_once` | iter-2353's queue fix, end to end |
| `the_slash_queue_fallback_works_without_an_agent` | the hint's recommended path |
| `up_with_no_history_and_one_row_is_silently_inert` | the ordinary readline no-op |
| `down_out_of_the_ladder_returns_to_the_drafts_end` | the exit edge |
| `the_unenhanced_hint_says_nothing_about_a_chord_it_cannot_deliver` | the hint is honest |

Mutation-proven: reverting `RecallEdge::Top` to the bottom fails two tests;
removing the `-4` deduction fails the wrap-boundary test.

## Gates

`cargo test -p operant-cli --bin operant`: **zero** new failures against a
baseline taken in this worktree (9 pre-existing, all peer-file: `cmd_channel`,
`dashboard_server`, `gateway_runner`, `advisor_card`, `commands`; 7 after, the
two that moved are pre-existing flakes).
`cargo clippy -p operant-cli --bin operant`: **zero** warnings in the touched
files. `scripts/audit-ledger-index.py`: only the pre-existing RT-553/554.

## What this pass did not do

- **No live session.** The operator's report is about a running TUI; these are
  unit-level.
- **No new key for walking a recalled entry.** The fix would be a product
  decision (a modifier, or a mode), not a repair.
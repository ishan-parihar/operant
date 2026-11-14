# Operator report 2026-11-14: Up/Down line-first, recall cursor, Enter chords

**Date:** 2026-11-14
**Operator report (verbatim):**
> The up/dn arrow must originally navigate the input-area lines, rather than
> directly navigate the prompt history.
>
> The prompt-history navigation must only work when the cursor is at the
> top/bottom. When navigating the prompt-history in forward/backward, the cursor
> must always stay at the top-start, or bottom-end for effective navigation of
> prompt history.
>
> The queue/steer is still not working effectively. CTRL+ENTER must queue the
> agent, ENTER must steer the agent, and SHIFT+ENTER must be a line break.

**Audited:** `origin/main` at `4ec61e0e` (worktree `/home/ishanp/rt2`).
**Method:** every finding below is a **failing probe**, run against the shipped
code, not a reading. Probes were removed after the audit; each became a named
regression test.

---

## Finding 1 — Up recalled history instead of walking a wrapped line

**Confirmed.** `key_handling.rs` split the Up/Down arms on
`prompt_input.text.contains('\n')`. A single-line draft jumped straight to the
history ladder, so the `move_visual_up` branch was only reachable for a draft
with a HARD newline.

A single line **wraps**. A 61-character draft in a 20-column composer is four
visual rows. Probe `probe_up_walks_a_wrapped_single_line_draft`: the caret sat on
visual row 3 and Up recalled an old prompt instead of walking to row 2 — throwing
away the draft the operator was editing.

The newline test was measuring the wrong thing. What matters is whether there is
a visual row **above the caret**, and `move_visual_up` already answers that for a
wrapped line exactly as for a hard break.

- **Fix:** both arms now try the visual walk first and reach history only when it
  cannot move (`!walked`). One rule, no newline test.
- **Also fixed, in passing:** the Down arm's old `!text.contains('\n')` guard made
  Down a **dead key** on any single-line draft, wrapped or not. Probe
  `probe_down_walks_a_wrapped_single_line_draft` showed the caret on row 0 of a
  4-row wrapped draft not moving at all.

## Finding 2 — the recall cursor sat at the wrong end

**Confirmed.** Both ladder methods set `self.cursor = self.text.len()` on every
step, so an Up recall landed with the caret at the END of the recalled entry.
Probe `probe_recall_puts_the_cursor_at_the_top_start`: `cursor=36 len=36` after an
Up recall.

That is the opposite of the gesture: Up reaches **back**, and an entry opening
with the caret at its end reads as "this is the newest line". It also means the
next Up walks the entry's last row before it can step further back.

- **Fix:** `history_up` → `cursor = 0` (top-start), `history_down` → `cursor =
  text.len()` (bottom-end), including the exit back to the draft. All three
  writers also clear `preferred_col`, so a stale saved column from a previous
  vertical walk cannot move the caret sideways on the next recall.
- **Cost, stated rather than hidden:** after a recall the caret is at the top, so
  typing inserts **above** the recalled text. Pinned by
  `typing_after_a_recall_inserts_at_the_top_start`. The operator asked for
  top-start navigation; this is what it costs on the first keystroke.

## Finding 3 — the queue refused when no agent was attached

**Confirmed, and the one that explains "not working effectively".**
`queue_for_next_turn` (`commands.rs:281`) opened with:

```rust
if self.steer_queue_handle.is_none() {
    return "Queueing is unavailable (no live agent to deliver to).".to_string();
}
```

The next-turn queue is the **App's own `Vec`** (`next_turn_queue`), and it is
drained by `take_pending_queued_messages` at the next submission — it never
touches `steer_queue_handle`. The gate refused Ctrl+Enter in exactly the two
windows the operator reaches for it: **before the first turn of a session**, and
**after an agent that failed to attach**.

- **Fix:** the gate is gone. `steer_now` keeps its gate, because a steer really
  does need somewhere to be injected. The asymmetry is deliberate and pinned by
  `the_queue_works_before_any_agent_is_attached` +
  `a_steer_without_an_agent_still_refuses`.

## Finding 4 — the Enter chords were already right

**Not a bug.** All three arms are correct, in both streaming states. This is
pinned by `enter_chords_keep_their_meaning_in_both_states`, which drives the real
key events:

| chord | idle | streaming |
|---|---|---|
| `Shift+Enter` | newline | newline |
| `Ctrl+Enter` | queue | queue |
| `Enter` | submit | **steer** |

So the operator's third complaint was not a dispatch defect. Two things explain
the report without excusing it:

1. **Terminal capability.** `Shift+Enter` and `Ctrl+Enter` are only distinguishable
   if the terminal grants the kitty keyboard protocol
   (`keyboard_enhancement.rs`; a bare `\r` is all an unenhanced terminal sends).
   `perf.rs:178` turns the request **off** for `is_wsl_windows_terminal`. On such a
   terminal every Enter chord arrives as plain Enter, and Ctrl+Enter becomes a
   steer — the opposite of the advertised action. The composer hint is already
   honest about this (`ui_input.rs:101` offers `/queue <text>` instead), and
   `/keys` flags the chord.
2. **The queue's only feedback was a 3-second notice** — until this audit
   confirmed the `⏳` queued band renders `next_turn_queue`
   (`ui_input.rs:631`, `tui_state_impl.rs:243`). The queue **is** visible; the
   operator's "not working" is the Finding-3 refusal, not invisibility.

## Finding 5 — the catalogue described Up/Down as the history ladder

**Confirmed, documentation defect.** `keybindings/defaults.rs` described `Up` as
`"History previous"` and `Down` as `"History next"` — true only at the boundary,
and false for the common case after Finding 1. `/keys` and the help overlay were
telling the operator the keys do the thing they were complaining about.

- **Fix:** `"Line up, then history previous at the top row"` and the mirror. The
  pin test `the_help_overlay_descriptions_match_what_the_arms_do` was updated in
  the same commit.

## Audited and found sound

- **`Ctrl+N` ("History next")** is catalogued and implemented as a ladder step
  guarded by `history_pos.is_some()`, so it cannot start a browse and cannot
  strand one. It is a readline-style convenience, not a fourth Up/Down; the
  new rule governs Up/Down only. Pinned by
  `ctrl_n_cannot_start_a_browse_from_a_fresh_draft`.
- **`Ctrl+P`** is already honestly de-catalogued (iter-429 comment): the arm opens
  global search, and the catalogue no longer claims a history meaning.
- **The `⏳` queued band** renders `next_turn_queue`, so a queued message is
  visible after the notice expires.
- **The steer transcript row** (`commands.rs:249`) and its mirror still work; the
  queue has no transcript row by design (a queued message is not yet a message).

---

## Regression tests added

| test | pins |
|---|---|
| `up_walks_the_draft_lines_before_reaching_for_history` | Up moves the caret on a multi-line draft |
| `up_walks_a_wrapped_single_line_draft_before_reaching_for_history` | the wrapped-single-line case (finding 1) |
| `down_walks_a_wrapped_single_line_draft` | Down is not a dead key on a wrapped draft |
| `a_recall_going_back_puts_the_caret_at_the_top_start` | finding 2 |
| `a_recall_going_forward_puts_the_caret_at_the_bottom_end` | finding 2, both branches |
| `up_recalls_history_once_the_caret_is_on_the_top_row` | the boundary still reaches history |
| `the_queue_works_before_any_agent_is_attached` | finding 3 |
| `a_steer_without_an_agent_still_refuses` | the deliberate asymmetry |
| `enter_chords_keep_their_meaning_in_both_states` | finding 4, all six combinations |

**Red-team pass on my own change** (six attacks, all green, all kept):
`up_recalls_history_when_the_composer_was_never_laid_out` (the `width == 0` case
the change could have broken), `up_recalls_on_a_short_draft_that_does_not_wrap`
(the hermes-agent single-line behavior the old code had),
`up_recalls_on_the_next_press_after_reaching_the_top_row` (no two-press boundary),
`typing_after_a_recall_inserts_at_the_top_start` (the cost of the rule),
`down_at_the_bottom_row_does_not_step_into_the_future`,
`up_at_the_wrap_boundary_walks_then_recalls`.

Mutation-proven, five mutations / five red: `held()` → `text` (both key-arm
tests), the Up visual walk removed (three tests), the Down walk removed (one),
the recall cursor reverted (one), the queue gate restored (one).

## Gates

`cargo test -p operant-cli --bin operant`: **zero** new failures against the
no-fix baseline taken in this worktree (11 pre-existing, from a peer's
`session_hub` / `advisor_card` breakage that is red on `origin/main` itself; one
`gateway_runner` flake that appears in one run and not the next).
`cargo clippy -p operant-cli --bin operant`: **zero** warnings in the touched
files.

## Not done, deliberately

- **The Enter chords were not "fixed"** because they are not broken. The tests
  exist so the next agent can verify the claim rather than re-read it. If the
  operator's terminal is WSL+Windows Terminal, the honest answer is
  `/terminal-setup`, not a code change.
- **`Ctrl+N` was not made to walk lines.** It is a deliberate readline alias with
  its own guard; changing it would be scope creep the operator did not ask for.
- **No live session verification.** The operator's report is about a running TUI;
  these are unit-level. `up_at_the_wrap_boundary_walks_then_recalls` is the
  closest proxy, but a live paste-and-arrow pass is the acceptance step this
  audit did not run.
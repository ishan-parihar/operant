# TUI input area — red team and ponytail audit

Dated-frozen: audited `2026-11-12`; findings mark verified vs asserted.

Base commit: `a3ee4d07` (`feat(iter-2197)`). Every finding below was verified by reading the
cited lines in this tree; the fixes landed in iter-2209 (code), and iter-2210/iter-2211 carry this
campaign's fleet-coordination fix and records.

## Contents

- [Scope](#scope)
- [Method](#method)
- [What was ALREADY there — and the first inventory that got it wrong](#what-was-already-there--and-the-first-inventory-that-got-it-wrong)
- [The operator's four asks, measured](#the-operators-four-asks-measured)
- [Findings](#findings)
- [The fragmentation, re-measured](#the-fragmentation-re-measured)
- [Ponytail verdicts](#ponytail-verdicts)
- [What was NOT done](#what-was-not-done)
- [Verdict](#verdict)

## Scope

The **chat composer's input area** in the operant TUI: the text buffer, the caret, the completion
popups, the prompt-history ladder, the inline ghost-text predictor, and the keys that drive them.

The operator's four asks are the acceptance criteria:

1. "when using tab with slash command, all text in front of it must not be deleted"
2. "the up/dn keys must seamlessly scroll between the previous prompt history rather than getting
   blocked due to incorrect cursor position which must be at top/bottom when navigating"
3. "embedding a dictionary with the pre-existing suggestions, so that the prediction-model can act
   as the self-learning system of what the user is about to type"
4. "various other input area ease of access features and bugs"

Reference implementations read: `external-repos/codex` (OpenAI Codex's ratatui bottom pane) and
`external-repos/oh-my-pi` (OMP's `pi-predict`).

## Method

Direct reads of the composer state, the render layer, the key dispatcher and the predictor, plus a
read of codex's `completion_target.rs` / `slash_input.rs` and OMP's `pi-predict` n-gram crate. Each
finding below was reproduced as a failing test on the base commit before any code changed.

## What was ALREADY there — and the first inventory that got it wrong

**This audit's first pass was wrong, and the correction matters.** A file-level inventory of
`crates/operant-cli/src/tui/` (which enumerated only the TOP level of each directory) reported that
operant had **no prediction model** and **one** history owner. Both claims were false:

| Claimed absent | Reality on the base commit |
|---|---|
| "no prediction model" | `prompt_input/prediction/` — `model.rs` (457 LOC), `query.rs` (334), `snapshot.rs` (127), `mod.rs` (332): an interpolated absolute-discounting word **trigram** over the operator's own submitted prompts, with an atomic snapshot (`ngram.snapshot`), an async engine thread, and ghost-text projection (`prompt_input/ghost.rs`). It is OMP's `pi-predict` port, and it was **off by default** only in the sense that the *dictionary* was missing (see RT-557). |
| "one history owner" | `prompt_input/history.rs` (the in-session ladder), `tui/input_history.rs` (the `history.jsonl` persistence), `app/prompt.rs` + `operant_app/prompt.rs` (the submit path), `adapter_types/history.rs` (**session titles**, not prompt history). Four files, four jobs — **not** four copies of one thing. |
| "the composer's caret model is a frozen string" | `prompt_input/visual.rs` already had a 2-D visual-row model (`cursor_visual_pos`, `move_visual_up/down`) — but it was an APPROXIMATION of the painter's wrap (RT-558). |

The lesson is the one this repo already writes down: **enumerate the tree, not the directory.**
A depth-1 `find` cannot see `prompt_input/prediction/`, and a module count is not a capability
count.

Two things were therefore NOT defects and are recorded here so the next agent does not "fix" them:
the prediction model exists and is self-learning; the history ladder, the persistence and the submit
path are three different responsibilities.

## The operator's four asks, measured

| Ask | State on the base commit | Verdict |
|---|---|---|
| 1. Tab + slash must not delete the text in front | The slash arm of `accept_suggestion` did `self.text = s.text.clone()` — the **whole buffer** was replaced, and the popup only opened when the text **started** with `/`, so "look at /rev" completed to nothing at all | **DEFECT** → RT-555 |
| 2. Up/Down must scroll the history, cursor at top/bottom | The ladder existed, but the browsing branch was gated on `!text.contains('\n')`, so a recalled **multi-line** entry fed the caret instead of the ladder; and the multi-line branch discarded `move_visual_up`'s "did I move?" answer, so Up on the top row did nothing | **DEFECT** → RT-556 |
| 3. A dictionary + a self-learning predictor | The self-learning half existed (the n-gram). The **dictionary did not**: `model.rs` says so in its own header — "no embedded Norvig web prior (the prior is uniform)". A fresh install predicted nothing until the operator had submitted the same word twice (`MIN_PROMPTS = 2`) | **DEFECT** → RT-557 |
| 4. Other ease-of-access features | Codex's `TextArea` (vim registers, search, paste-burst, hyperlinks) is ~4,760 lines; operant's composer already carries vim (`prompt_input/vim.rs`), a kill ring, a typeahead, ghost text, file refs and `$skill` mentions | **MOSTLY PRESENT** — the gaps were in the caret's row model (RT-558) |

## Findings

### RT-555 — Tab on a slash command deleted the whole draft

`prompt_input/suggestions.rs`, `accept_suggestion`, the `SlashCommand` arm:

```rust
TypeaheadSource::SlashCommand => {
    // Replace entire text; discard anything after cursor too.
    self.text = s.text.clone();
    self.text.len()
}
```

The `@` arm beside it has always spliced (`accept_suggestion_file_ref_after_text_preserves_prefix`
pins it); the slash arm never did. A multi-line draft whose first line was a command lost every
line below it on Tab. And because `compute_typeahead` only offered slash commands when the text
**started** with `/`, the operator could not even write "look at /rev" and complete it.

**Fixed** in iter-2209: one `slash_token_range` helper resolves the whitespace-delimited `/…` word
under the caret (a word with a second `/` is a path and never opens the popup), the popup opens on
that token wherever it sits, and the accept splices the token — the text in front and the tail
after the caret both survive. The five places that each had their own "does the text start with `/`"
rule (`typeahead.rs`, `active_file_query`, `ghost.rs`, the Up/Down gate, the popup's highlight
needle) now read the same helper.

**Pinned** by `accept_suggestion_slash_command_preserves_the_text_in_front`,
`accept_suggestion_slash_command_preserves_the_draft_tail`,
`accepting_a_slash_command_with_no_token_never_clobbers_the_buffer`,
`a_slash_command_after_prose_opens_the_popup`, `a_path_token_never_opens_the_command_popup` and
`the_command_popup_closes_after_the_command_name`.

### RT-556 — Up/Down could not reach the history from a multi-line draft

`app/key_handling.rs`, the Up/Down arms. Two gates had to be removed:

- the browsing branch was `history_pos.is_some() && !text.contains('\n')` — a recalled **multi-line**
  entry made the caret walk a line instead of stepping the ladder;
- the multi-line branch called `move_visual_up(width)` and **discarded its `bool`** — the helper
  already reported "there was no row above", so Up on the top row of a multi-line draft did
  nothing at all.

**Fixed** in iter-2209: browsing wins for any recalled entry, and Up falls through to the ladder
when the caret is already on the top visual row (with a zero-width composer the visual-row model is
undefined, so nothing is recalled).

**Pinned** by `up_walks_the_draft_rows_then_recalls_the_history_at_the_top_row` and
`a_recalled_multiline_entry_still_steps_the_history_ladder`.

### RT-557 — the predictor had no cold start

`prompt_input/prediction/model.rs`, header comment: "no embedded Norvig web prior (the prior is
uniform; OMP ships a 2.6 MB data file — the show threshold compensates on a thin corpus)".

So the ghost was silent until the operator had submitted the same word in two distinct prompts.
OMP closes that gap with a 2.6 MB compressed Norvig prior blended into the trigram.

**Fixed** in iter-2209 with the minimum that buys the same cold start: a 5,000-word seed dictionary
(`prediction/dictionary.tsv`, 60 KB, the most frequent lowercase English prose forms with their
frequency ranks) consulted **only when the learned vocabulary knows no word under the typed prefix
at all**. It fills a vocabulary gap; it never competes with the learned model, never revives a word
the `MIN_PROMPTS` hygiene gate rejected, and never extends a word the operator has already finished
("the" must not become "their"). The operator's show threshold governs it too — the dictionary's
frequency rank is mapped onto the same `0.0..=1.0` scale the learned posterior uses.

**Pinned** by `the_seed_dictionary_answers_a_prefix_the_learned_model_never_saw`,
`the_learned_model_outranks_the_seed_dictionary`,
`the_seed_dictionary_does_not_revive_a_word_the_hygiene_gate_rejected`, and the dictionary's own
four tests.

### RT-558 — the caret and the painter disagreed about which row a line was on

The painter lays the composer's rows out with `wrap_input_segments` (in `operant_ui/ui_input.rs`),
and resolves the caret's row from that map. The **state** layer had its own approximation:
`cursor_visual_pos` computed the row as `display_col / width`, which assumes every row is exactly
`width` cells wide — a row holding five CJK characters is ten cells wide and five characters wide,
and the division puts the caret one row too far. `set_cursor_at_visual` made the same assumption
when it turned a row/column pair back into a byte offset (`intra_chunk * width` CHARACTERS), so Up
from the second painted row of a CJK draft landed past the end of the first.

**Fixed** in iter-2209: one wrap model (`prompt_input::wrap_input_rows` / `VisualRow`) is used by
the painter AND by the caret. The renderer's private copy was deleted; the caret's row and column
now come from the painted rows, and the column is a DISPLAY column.

**Pinned** by `the_caret_row_is_the_row_the_painter_draws`,
`the_caret_column_is_a_display_column_on_a_cjk_row` and `moving_up_lands_on_the_painted_row`.

## The fragmentation, re-measured

The first pass claimed operant carried four copies of the prompt history and two key interpreters.
Re-measured on the base commit:

| Claim | Measurement |
|---|---|
| "four history owners" | **False.** `prompt_input/history.rs` is the in-session ladder; `tui/input_history.rs` is the `history.jsonl` persistence; `app/prompt.rs` is the submit path; `adapter_types/history.rs` is **session titles**. Three jobs, three owners. Nothing is duplicated. |
| "`operant_app/` is a second App" | **False.** `operant_app/mod.rs` says it outright: a vendored jcode port of the renderer-facing leaf surface, re-rooted, with the App struct and every App-bound fn deliberately NOT ported. It is a dependency of the render layer, not a rival to `app/`. |
| "two keystroke interpreters" | **False.** `app/key_handling.rs` is the only dispatcher; `keybindings/defaults.rs` is its catalogue. The real duplication was narrower: the **slash-token grammar** was re-derived at five sites, each with its own rule — that is what RT-555 collapsed into one helper. |
| "three row-budget computations" | **Half false.** `ui_frame_metrics.rs` and `ui_viewport.rs` do not both own the composer's rows; the duplication was between the painter's wrap and the caret's wrap (RT-558), which is now one function. |

## Ponytail verdicts

| Candidate | Verdict | Why |
|---|---|---|
| Port codex's `TextArea` (4,760 lines: vim registers, search, paste-burst, hyperlinks) | **SKIP** | operant already has vim, a kill ring, a typeahead, ghost text and file refs. The parity gap is real but it is a restyle, not a fix for any reported defect. |
| Port OMP's 2.6 MB web prior verbatim | **SKIP** | The cold start is bought by a 60 KB seed dictionary that fires only in a vocabulary gap. The learned model stays the self-learning system, which is what the operator asked for. |
| A new "composer" abstraction to unify `prompt_input/` and `operant_ui/` | **SKIP** | The two layers have different jobs (state vs paint) and the repo's one-renderer rule already fixes the boundary. The duplication that existed was two WRAP algorithms and five slash-token rules — both now single. |
| Rework the keybindings catalogue | **SKIP** | `keybindings/defaults.rs` is the catalogue and `app/key_handling.rs` is the dispatcher; the catalogue is generated into `/keys`, so there is one source. |
| A "prediction settings" screen for the dictionary | **SKIP** | The dictionary is a cold-start seed, not a preference. The existing `word_completion` / `word_completion_threshold` settings already govern the predictor. |

## What was NOT done

- **No vim-surface parity work.** The catalogue rows for vim chords were verified, not extended.
- **No mouse-selection work.** `app/mouse.rs` already places the caret and drags a selection.
- **The dashboard test stays red on this host.** `the_dashboard_reports_the_daemon_across_processes`
  asserts "no PID file ⇒ not running", but the check also consults the systemd user unit, and this
  host runs a live `operant-gateway` — so the assertion is false here. It fails identically on the
  base commit with this campaign's changes reverted (measured, iter-2209). It is a peer's RT-518
  test whose isolation is missing, not an input-area defect.
- **The `Enter` catalogue pin was corrected, not the row.** iter-2172 changed the row to "Submit
  prompt (steers the running turn while it streams)" — which is what the dispatch really does — and
  left the test's pin at the pre-2172 short string. The row is right; the pin was stale.

## Verdict

The operator's four asks are now met, and the input area's remaining fragmentation is two functions
that are now one. The four defects were real, small, and each is pinned by a test that fails on the
base commit. The ponytail verdict is that operant's composer already carries most of what codex
does; what it lacked was **agreement** — between the caret and the painter, and between the popup
and the buffer — not features.

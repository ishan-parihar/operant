# codex input-area quality-of-life: paste collapse/expand, history navigation gate, dictionary hygiene

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## The request, verbatim

> Investigate `../external-repos/codex/` to understand the TUI input-area features, like
> various shortcuts, and quality of life features, like ["when copying large-text, single
> paste shows truncated text like `pasted NN lines`, and pasting it again expands that,
> with a tooltip when pasted a single time", "when using tab with slash command, all text
> in front of it must not be deleted", "the up/dn keys must seamlessly scroll between the
> previous prompt history rather than getting blocked due to incorrect cursor position.
> It would be done by standardizing the prompt change by cursor location at top/bottom of
> the input-text when navigating. It must not conflict with the up/dn key for navigating
> the input text across multiple lines", "embedding an actual dictionary (like dictionary2
> crate) with the pre-existing suggestions, so that the prediction-model can act as the
> self-learning system of what the user is about to type, without any incorrect words or
> typos, while learning relevant words that are used multiple times"]

## Status of the four asks (measured, not assumed)

Three of the four shipped in iter-2209 (`971bfb5b`, RT-553/554/555/556/557/558 in the
ledger). This plan records the investigation and the ONE that is still open, plus the
three defects the comparison turned up in the shipped halves.

| ask | codex mechanism | operant today | verdict |
|---|---|---|---|
| paste collapses to `pasted NN lines`; pasting again expands; tooltip on the single paste | `[Pasted Content N chars]` + `pending_pastes` + `TextElement` atomic ranges (`chat_composer.rs:2002`, `apply_paste`, `expand_pending_pastes`) | `[Pasted ~N lines #K]` + `paste_contents` map; collapse only. NO expand, NO tooltip, and the payload leaks (RT-559) | **partially open** |
| Tab on a slash command keeps the text in front | `slash_input.rs:426` — the completion REPLACES `0..replace_end` and re-appends the tail after the cursor as inline args | `slash_token_range` splice (`suggestions.rs`, iter-2209) | **closed** |
| Up/Dn scroll the prompt history, never blocked by the caret, never fighting multi-line cursor motion | `should_handle_navigation`: empty text, OR text == last recalled entry AND caret at a line boundary (0 / len) (`chat_composer_history.rs:394`) | browsing wins for any recalled entry; Up falls through to the ladder at the top visual row; `set_cursor_at_visual` zeroes `history_pos` | **closed, with two residuals (RT-560 the recall/edit gate, RT-561 the saved column)** |
| a real dictionary behind the ghost, without incorrect words or typos | **codex has no word predictor at all** (no ghost, no dictionary; grep for `ghost|predictive|autocomplete` hits only popups) — the reference for this ask is oh-my-pi `pi-predict`, which operant already ports | 5,000-word seed dictionary, consulted ONLY on a learned-vocabulary gap; `MIN_PROMPTS = 2` hygiene | **closed (iter-2209)** |

## Contents

- [Status of the four asks](#status-of-the-four-asks-measured-not-assumed)
- [What codex's input area actually is](#what-codexs-input-area-actually-is-measured-this-session)
- [The three defects this comparison found](#the-three-defects-this-comparison-turned-up-in-operant-new)
- [Gates / success criteria](#gates--success-criteria)
- [Cost](#cost)

## What codex's input area actually is (measured this session)

Files read: `tui/src/bottom_pane/chat_composer.rs` (12.8k), `chat_composer/` (34 files),
`textarea.rs` (4.8k) + `textarea/` (9 files), `paste_burst.rs` (610), `keymap.rs` (4.6k) +
`keymap/bindings.rs`, `command_popup.rs`, `shortcut_overlay.rs`, `completion_target.rs`.

### 1. Large-paste collapse is ATOMIC, not a string swap

Codex's placeholder is a **`TextElement`** — a byte range the textarea refuses to edit
piecemeal (`prev_atomic_boundary` / `next_atomic_boundary`, `textarea.rs:1974/2000`):

- `apply_paste` (`chat_composer/paste_input.rs:208`) inserts the placeholder element at the
  edit target and pushes `(placeholder, full_text)` onto `draft.pending_pastes`.
- The label is derived from the CONTENT, not a counter: `[Pasted Content {chars} chars]`,
  with ` #2`, ` #3` suffixes computed from the placeholders still live
  (`next_large_paste_placeholder`, `chat_composer.rs:2001`). Deleting the placeholder drops
  the payload (`reconcile_deleted_elements`), so a removed paste can never be submitted and
  the numbering is reclaimed.
- Submission expands: `expand_pending_pastes` walks the element list in byte order and
  rebuilds text + element spans in one pass, so a paste that is half-quoted (codex's
  `> `-blockquote paste, `apply_paste`'s `is_blockquote`) still expands to the exact text.
- Threshold is **1000 characters** (`LARGE_PASTE_CHAR_THRESHOLD`), not a line count.

**There is no "paste again to expand" in codex.** The operator's phrasing describes
jcode's model, which IS in this tree (`../external-repos/jcode`): `handle_text_paste`
(`jcode-tui/src/tui/app/input.rs:539`) collapses a ≥5-line paste to `[pasted N lines]`
and stores the text in a `Vec<String>`, and `expand_matching_paste` re-pastes the SAME
text by matching `rposition` on the stored content, then swaps the placeholder occurrence
that belongs to that content (skipping newer same-shaped placeholders so equal line counts
cannot expand the wrong one). jcode's tooltip is the absence of one: nothing announces the
expand affordance, which is why the operator asked for it.

### 2. Slash completion: codex has the SAME split operant had

`handle_key_event_with_slash_popup`'s Tab arm (`chat_composer/slash_input.rs:279`) is
three-tier:

1. a command that dispatches immediately (e.g. `/clear`) replaces the buffer and returns;
2. `complete_selected_slash_command_preserving_existing_draft_tail_as_inline_args` splices
   `0..replace_end` to `/name` and keeps `text[replace_end..]` as inline args — the tail
   survives, and the cursor lands at the END of the whole buffer;
3. otherwise `selected_command_completion` returns `"/name "` for the WHOLE first line.

The popup's filter is the first line only (`command_under_cursor`: the line must start with
`/`, the filter is the token under the caret). Operant's iter-2209 splice is the same
contract with the prefix preserved (codex requires byte 0; operant allows prose before the
token because `slash_token_range` is whitespace-anchored, which is strictly more general).

### 3. History navigation: codex's gate is the "last recalled text" rule

`ChatComposerHistory::should_handle_navigation` (`chat_composer_history.rs:394`):

```text
empty text                                        -> navigate
cursor is not at 0 and not at len                 -> DO NOT navigate (move the caret)
otherwise                                         -> navigate IFF text == last_history_text
```

`last_history_text` is recorded on every recall (`record_recalled_text`). The effect: the
ladder is walkable from a recalled multi-line entry, but the moment the operator EDITS that
entry the ladder stops claiming Up/Down (otherwise Up could never move the caret inside a
draft the operator is editing). `history_navigation_cursor` maps vim-normal's end-of-line
cursor to `len` so vim users are not stranded. Popup sync is suppressed while browsing
(`sync_popups` returns early), which is the same precedence iter-2209 gave operant.

Two more codex details operant lacks:

- **`preferred_col`** (`textarea.rs:1509`): Up/Down through wrapped rows keep a SAVED column
  when the target row is shorter, and clear it on any non-vertical edit. Operant recomputes
  the column from the current row every time, so `move_visual_up` from column 60 onto a
  20-column row and back lands at column 20, not 60.
- **Ctrl+A at BOL / Ctrl+E at EOL** (`VimAction::MoveLineStart { move_up_at_bol }`): readline's
  `^A`/`^E` step a line when they are already at the boundary. Operant's Ctrl+A is the model
  picker (deliberate, `iter-429`) and its Ctrl+E is line-end with no wrap.

### 4. The editor keymap is DATA in codex, code in operant

`EditorKeymap` (`keymap.rs:217`) is 17 named actions, each a `Vec<KeyBinding>` resolved from
`[editor]` TOML with a validated conflict pass against `AppKeymap`, and `/keymap` edits it
live. The defaults (`keymap.rs:1895-1975`) are readline plus vim-insert:

| action | codex default | operant |
|---|---|---|
| `insert_newline` | Ctrl+J, Ctrl+M, Enter, Shift+Enter, Alt+Enter | Shift/Alt+Enter, Ctrl+J (no Ctrl+M) |
| `move_left/right` | arrows, Ctrl+B, Ctrl+F | arrows (Ctrl+B is the branch hub, Ctrl+F is word-forward) |
| `move_up/down` | arrows, Ctrl+P, Ctrl+N | arrows, Ctrl+N (history next); Ctrl+P is global search |
| `move_word_left/right` | Alt+B/F, Alt/Ctrl+arrows | Alt+B/F (no Alt/Ctrl+arrow) |
| `move_line_start/end` | Home/End, Ctrl+A/E, Super+arrows | Home/End (line-scoped), Ctrl+E; **no Super+arrows, no Ctrl+A** |
| `delete_backward` | Backspace, Shift+Backspace, Ctrl+H | Backspace (Ctrl+H is word-delete) |
| `delete_forward` | Delete, Shift+Delete, Ctrl+D | Delete (Ctrl+D is exit-confirm on empty) |
| `delete_backward_word` | Alt/Ctrl+Backspace, Ctrl+W, Ctrl+Alt+H | Alt/Ctrl+Backspace, Ctrl+W, Ctrl+H |
| `delete_forward_word` | Alt/Ctrl+Delete, Alt+D | Alt+Delete, Alt+D |
| `kill_line_start` | Ctrl+U | Ctrl+U (into the kill ring) |
| `kill_line_end` | **Ctrl+K** | **MISSING** (Ctrl+K is the command palette) |
| `kill_whole_line` | unbound | MISSING |
| `yank` | Ctrl+Y | Ctrl+Y (kill ring) |

Operant's chords are hardcoded arms in `app/key_handling.rs`; `keybindings/defaults.rs` is a
catalogue that does not dispatch (its own header says so). `/keymap` has no operant
equivalent.

### 5. Paste-burst detection: both trees have it, differently

- codex: `PasteBurst` (`paste_burst.rs`, 610 lines + tests) — a pure state machine with
  char-interval and idle timeouts, flicker suppression (hold the first char), retro-capture
  of already-inserted prefixes, an Enter-suppression window so a multi-line paste is not
  submitted by its own newlines, and an explicit "do not accept a completion on Tab" rule.
- operant: `collect_paste_burst` (`app/mouse.rs:803`) — a zero-timeout drain with a
  two-char threshold, an Enter-ambiguity rule (more clipboard behind it → newline; queue
  empty → replay the submit key) and the Ctrl+J-as-newline fix. Simpler, and it covers the
  case that actually bites on Linux/tmux.

### 6. Transient hints are a first-class surface in codex

`FooterFlash` (`chat_composer/footer_state.rs:408`): one line + an expiry, painted in the
footer row in place of the hint strip, cleared on activity. Codex uses it for "Message too
long" and warnings. This is the mechanism the operator's "tooltip when pasted a single
time" should use — operant's `set_status_notice` is the same shape (3 s TTL,
`live_status_notice`), so the affordance has a home without new architecture.

## The three defects this comparison turned up in operant (new)

- **RT-559 (S, PASTE PAYLOAD LEAKS AND CANNOT BE TAKEN BACK).** `paste()` inserts the
  placeholder then stores `paste_contents[counter] = content`; the only reader is `take()`.
  `clear()` (Ctrl+C, `/clear`, a submit) never clears the map, so a discarded draft's
  clipboard content stays resident for the process lifetime. Worse, the key is the counter
  and the label carries `#{id}`, so an undo that removes the placeholder leaves a payload
  nothing can ever expand — the model gets the text only if the placeholder survives to
  `take()`. codex's element model makes both impossible (delete the element, drop the
  payload; `reconcile_deleted_elements`).
- **RT-560 (S, RECALL SURVIVES AN EDIT).** `history_up` sets `history_pos` and nothing
  clears it when the operator types: `insert_char` does not, and `refresh_prompt_input`
  only recomputes suggestions. So Up-after-edit keeps walking the ladder instead of moving
  the caret through the draft — codex's `last_history_text` rule is exactly this guard.
- **RT-561 (S, NO SAVED COLUMN).** `move_visual_up/down` recompute the column from the row
  being left instead of keeping a `preferred_col`, so vertical motion through wrapped rows
  drifts to the shortest row's length and never comes back.

Plus the open ask itself:

- **RT-562 (M, NO COLLAPSE FEEDBACK, NO EXPAND).** A collapse is silent and irreversible.
  Needs three things codex/jcode split between them: the placeholder is ATOMIC (so a
  Backspace removes it whole and drops the payload), a transient notice on the collapse
  ("Pasted 412 lines — paste again to expand", the `FooterFlash`/`set_status_notice` seam),
  and a re-paste that finds the placeholder belonging to that content and swaps the text
  in (jcode's `expand_matching_paste`, including its same-shape disambiguation).

## Gates / success criteria

(fixed before the run)

1. RT-559 — a `clear()` empties `paste_contents`; a Backspace that removes a placeholder
   drops its payload; submitting sends the pasted text, not the token (the last is already
   pinned by `submitting_a_large_paste_sends_the_pasted_text_not_the_placeholder`).
2. RT-560 — after an edit, Up moves the caret (not the ladder); on an unedited recall it
   still walks the ladder (both directions asserted).
3. RT-561 — Up/Down through rows of unequal length keeps the saved column, and any
   horizontal edit clears it.
4. RT-562 — collapse shows the transient notice; re-pasting the same content expands THAT
   placeholder (two same-shaped pastes, the older one expanded); deleting a placeholder
   makes a later re-paste of that content collapse again rather than resurrecting it.
5. Full gate: `./scripts/check.sh check -p operant-cli --bin operant`,
   `cargo test -p operant-cli --bin operant`, `bash scripts/clippy-warning-gate.sh`, and the
   ledger index (`python3 scripts/audit-ledger-index.py`) green in the same iteration.

## Cost

Three small defects and one medium feature in two files (`prompt_input/{mod,history,state}.rs`
and the paste path's callers), plus one render/status call site. No new dependency, no new
config key (the notice is unconditional; the expand gesture is a paste, not a chord). The
`dictionary2` crate the request names is NOT adopted: operant's seed dictionary is a
frequency-ranked word list, and a spell-check dictionary answers "is this word correct",
which is the wrong question for a completion engine — the hygiene gate (`MIN_PROMPTS`) is
what keeps typos out of the learned model, and a dictionary suggestion can only ever fill a
gap the learned vocabulary left (RT-557, iter-2209).

## Outcome (iter-2255, 2026-11-14)

All four gates met. One paste owner now exists (`crates/operant-cli/src/tui/prompt_input/paste.rs`)
and the composer's one refresh choke point enforces the store invariant, so no mutation
site has to remember it.

| row | what shipped | pinning test(s) |
|---|---|---|
| RT-559 | `pending_pastes: Vec<(placeholder, content)>` (label-keyed, no counter); `clear()` empties it; `drop_missing_paste_placeholders` at the choke point; label numbered from the LIVE set so a reclaimed number is reused | `deleting_a_placeholder_drops_its_payload`, `a_numbered_placeholder_does_not_keep_the_bare_one_alive`, `keeping_a_placeholder_keeps_its_payload`, `deleting_a_placeholder_reclaims_its_number` |
| RT-560 | `history_recall` + `history_is_live()` gate used by both Up/Down arms and both ladder methods; no edit-path hook needed (an edit changes the text, so the gate opens by construction) | `editing_a_recalled_entry_ends_browsing`, `an_edited_recall_is_not_replaced_by_the_ladder` |
| RT-561 | `preferred_col: Option<usize>` saved by the first vertical move, reused by later ones, cleared by every non-vertical primitive in `editing.rs` | `a_vertical_walk_keeps_the_column_it_started_from`, `a_horizontal_change_restarts_the_saved_column` |
| RT-562 | re-paste expands THAT placeholder (spliced, so a second collapsed paste survives); the expand reports the line count through the status notice; `suppress_paste_expand` stops the choke point re-collapsing it | `repasting_the_same_content_expands_the_placeholder`, `expanding_reports_the_size_of_what_it_unwrapped`, `two_pastes_of_the_same_shape_are_not_confused`, `the_expand_target_is_the_placeholder_belonging_to_that_content`, `a_deleted_placeholder_is_not_resurrected_by_a_later_paste` |

Two defects surfaced DURING the run and were fixed rather than recorded:

- **`expand_target` keyed the shape off the wrong argument.** It called
  `paste_placeholder(candidate, &[])` where `candidate` is already a placeholder string, so
  the label was rebuilt from the placeholder's own line count (`[Pasted ~1 lines]`) and never
  matched a real placeholder: the expand always returned `None`. Now the shape is read off the
  placeholder and compared as an exact string.
- **The expand replaced the whole buffer.** `replace_text(content)` on one placeholder wiped
  every other collapsed paste in the composer. The expand now splices the payload in where the
  placeholder was, leaves the caret at the end of what it inserted, and puts the payload back
  if the text has no placeholder at the recorded offset (nothing is silently swallowed).
  Pinned by `two_pastes_of_the_same_shape_are_not_confused`, which failed against the first
  implementation.

One pre-existing break was repaired in passing because it blocked the iteration's own gate:
`crates/operant-cli/src/gateway_commands.rs:3767` carried the truncated call
`telegr[REDACTED](&vendor)` (committed by iter-2217; `origin/main` itself does not compile its
test target). The intended call is `telegram_bot_commands(&vendor)`. Fleet rule 7 says not to
fix a peer's in-flight subsystem; this is not in-flight — it is published, it breaks
`cargo test -p operant-cli` for every agent, and the fix is the one line the truncated token
was meant to be.

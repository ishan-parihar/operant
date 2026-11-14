# Red team + ponytail audit — the paste collapse / expand / history slice (iter-2266)

**Date:** 2026-11-14
**Audited:** the slice shipped as `fix(iter-2266): the paste store owns its payload,
and the ladder lets go` plus `docs(iter-2271)`, as they exist on `origin/main`
(`651bc708`).
**Auditor:** one agent, no peer review, no runtime session (the gateway is running
the shipped binary but the findings below are all reproducible from unit probes).

## What this code does

The composer (`PromptInputState`) is the one-line-to-many-line text box the
operator types a prompt into. When a paste is big (three lines or 150 characters)
the composer shows a short token such as `[Pasted ~412 lines]` and keeps the real
text in a side store, so a huge paste does not push the operator's own words off
screen. Submitting swaps the token back for the text, so the model still receives
the whole paste. The slice under audit added four things: one owner for the store
(`prompt_input/paste.rs`), a rule that drops a payload when its token leaves the
composer (RT-559), a rule that the history ladder only answers while the composer
still holds what was recalled (RT-560), and a saved column so Up/Down through
wrapped rows do not drift (RT-561), plus a re-paste that expands a collapsed paste
(RT-562).

**Load assumed:** one operator at a keyboard, one composer, pastes up to a few
megabytes. Not multi-user, not concurrent.

## Method

Every finding below is a **failing probe test**, not a reading. Each probe was
written against the shipped code, run, and its failure output recorded. The probe
file was removed afterwards (`git checkout -- …/tests.rs`), so the tree is clean.
Line numbers are from `origin/main` at `651bc708`.

---

## Must fix

### 1. An expand puts raw clipboard bytes into the composer (reintroduces RT-539)

- **What this is:** `expand_paste_placeholder` (`prompt_input/history.rs:126-148`)
  splices the stored payload straight into the buffer with
  `text.replace_range(..)`, with no sanitising step.
- **Problem:** the store holds the **raw** clipboard text — `paste()` stores
  `content` before sanitising (`history.rs:103`), so `pending_pastes` keeps the
  escape bytes. A collapse is safe (the buffer only ever gets the token), but an
  **expand** writes the raw payload into the buffer. A clipboard holding
  `a\x1b[31mRED\x1b[0m\nb\nc` collapses clean and expands to
  `"a\u{1b}[31mRED\u{1b}[0m\nb\nc"`. A terminal executes `ESC`, so the frame
  smears and the cursor can move anywhere — this is the exact defect iter-2159
  fixed for the paste path (RT-539), reopened by the expand.
- **Fix:** sanitise at the boundary. Either store `sanitize_pasted_text(content)`
  as the payload (one line, and then the store matches what the buffer holds —
  which also fixes finding 4), or run the payload through `sanitize_pasted_text`
  inside `expand_paste_placeholder` before the splice.
- **If we skip it:** the operator pastes a large coloured block, pastes it again
  to see it, and the terminal is corrupted until redraw.

*Probe:* `probe_expand_splices_unsafe_text_into_the_buffer` → `PROBE text="a\u{1b}[31mRED\u{1b}[0m\nb\nc"`.

### 2. `Ctrl+S` stash → restore sends the model a literal `[Pasted ~5 lines]`

- **What this is:** `stash_input()` (`state.rs:204`) calls `clear()`, which empties
  `pending_pastes`; `restore_stash()` (`state.rs:210`) puts the text back without
  the store.
- **Problem:** the restored text contains the token, the store is empty, so
  `take()` (`state.rs:128`) expands nothing and the model receives the token.
  Verified: `stash_input()` → `restore_stash()` → `take()` yields
  `"[Pasted ~5 lines]"`. This is RT-559's "restore text without the map" case,
  which the slice's own ledger row claimed to have closed.
- **Fix:** stash the store with the text. `stash: Option<(String, Vec<(String,String)>)>`
  moved out with `mem::take` like the text already is — no clone, one field.
- **If we skip it:** a very common key sequence (park a draft, paste, park again,
  bring it back) silently replaces a 5 000-word paste with a 19-character token.

*Probe:* `probe_stash_drops_the_payload_the_restored_text_needs` → `PROBE1 sent="[Pasted ~5 lines]"`.

### 3. Undo across a paste leaves an orphan payload in the store

- **What this is:** `restore_snapshot()` (`state.rs:285`) restores `text` and
  `cursor` only.
- **Problem:** after a paste and an undo back across it, the text is the pre-paste
  draft while the store still holds the payload
  (`PROBE2 undid=true text="draft" store=[("[Pasted ~5 lines]", "1\n2\n3\n4\n5\n")]`).
  It is dropped by the next refresh, so it is bounded, not permanent — but the
  invariant "the store only holds payloads whose token is in the text" is false
  between the undo and the next refresh, and `take()` runs before that refresh.
- **Fix:** call `drop_missing_paste_placeholders()` at the end of
  `restore_snapshot`. Two lines, and it makes the invariant true at every exit.
- **If we skip it:** the window is narrow but real; the same shape is what finding
  2 falls into permanently.

*Probe:* `probe_undo_across_a_paste_leaves_an_orphan_placeholder` (passes only
because it asserts the weaker "if the placeholder is present" branch; the printed
state shows the orphan).

### 4. The store is keyed by the raw text, the buffer holds sanitised text

- **What this is:** `paste()` (`history.rs:95-108`) matches `expand_target(content, …)`
  and `PasteOutcome::of(content, …)` against the **raw** `content`, but the buffer
  receives `sanitize_pasted_text(content)`.
- **Problem:** a paste containing a tab is stored as `"a\tb\nc\nd"` while the
  buffer holds `"a    b\nc\nd"`. Two consequences: (a) a re-paste of the same
  clipboard content matches by raw bytes so the expand works, but (b) the payload
  the store holds is **not** what the buffer would have held, so an expand changes
  the text on disk (`a\tb` → `a    b`) even though nothing was re-sanitised. The
  store and the buffer disagree about the same fact — a one-source-of-truth break.
- **Fix:** sanitise once, at the top of `paste()`, and use the sanitised value for
  the match, the store and the insert. Then finding 1's expand-splice fix is the
  same value and the two can never disagree.
- **If we skip it:** every fix for finding 1 has to be applied twice, and the
  store's copy of a fact still differs from the buffer's.

*Probe:* `probe_expand_splices_a_raw_tab` → `PROBE2 text="a\tb\nc\nd"`.

### 5. `take()` can hand the model a raw control byte

- **What this is:** `take()` (`state.rs:128`) expands the store into the outgoing
  text with no sanitising.
- **Problem:** pre-existing, not introduced by this slice (the old `paste()` had
  the same shape), but the slice's own module doc says "the model must get the
  paste" and the composer's invariant is that it never holds a control byte. A
  clipboard with an escape yields `sent="a\u{1b}b\nc\nd"`.
- **Fix:** sanitise the expanded result in `take()` — one call, and the composer's
  invariant holds on the way out as well as in.
- **If we skip it:** an escape byte reaches the provider's JSON; most clients strip
  or reject it, but "most" is not an invariant.

*Probe:* `probe_take_can_hand_the_model_a_raw_control_byte` → `PROBE sent="a\u{1b}b\nc\nd"`.

### 6. The collapse notice the operator asked for does not exist

- **What this is:** the operator's ask, verbatim: *"single paste shows truncated
  text like `pasted NN lines`, and pasting it again expands that, **with a tooltip
  when pasted a single time**"*.
- **Problem:** the slice implemented the **expand** notice (`paste_expanded` →
  `set_status_notice`, `history.rs:145`) but nothing sets a notice on the
  **collapse**. `grep` for a status notice in the paste paths returns nothing. So
  the collapse is still silent, which is half of what was asked for and half of
  the gate as written in the plan ("collapse shows the transient notice"). The
  ledger row RT-562 is marked CLOSED on the strength of the expand half.
- **Fix:** set the notice at the collapse site — the operator pasted 412 lines and
  should be told, and told how to get them back:
  `"Pasted 412 lines — paste again to expand."` The seam exists and is already
  used two lines away.
- **If we skip it:** the feature the operator asked for reads as half-built: the
  token appears with no explanation, and the only way to learn that a re-paste
  expands it is to guess.

*Evidence:* `grep -rn "set_status_notice" … | grep -i paste` → no collapse site.

---

## Should fix

### 7. `clear()` leaves the expand one-shot and the expand notice armed

- **What this is:** `clear()` (`state.rs:106-124`) resets `history_recall`,
  `preferred_col` and `pending_pastes`, but not `suppress_paste_expand` or
  `paste_expanded`.
- **Problem:** expand (arms both), then submit / `Ctrl+C` → the one-shot is still
  true, so the **next** paste's payload is dropped by a stale flag
  (`PROBE after clear suppress=true expanded=Some(...)`). The notice also survives
  the clear and pops up over an empty composer.
- **Fix:** two lines in `clear()`.
- **If we skip it:** paste, expand, submit, paste again → the second paste
  collapses and its payload is silently discarded.

*Probe:* `probe_clear_does_not_reset_the_expand_flag`.

### 8. A paste does not clear the saved column (RT-561's own rule)

- **What this is:** every non-vertical primitive in `editing.rs` clears
  `preferred_col`, but `insert_pasted_text` (`history.rs:154`) and the expand
  splice set the caret without clearing it.
- **Problem:** `PROBE_D preferred_col=Some(8)` after a paste that inserted `"XY"`
  mid-run. The contract written in the RT-561 ledger row is "every non-vertical
  edit clears it"; a paste is a non-vertical edit.
- **Fix:** one line in `insert_pasted_text` (the expand already clears it).
- **If we skip it:** the next Up jumps to the pre-paste column instead of the
  column the operator can see.

*Probe:* `probe_d_preferred_col_after_a_paste`.

### 9. Two different boundary rules decide whether a token is live

- **What this is:** the drop rule (`placeholder_is_live`, `state.rs:302`) rejects a
  token followed by `#` or a digit; the expand rule (`replace_placeholders`,
  `paste.rs:100`) sorts by label length and does a plain `contains`.
- **Problem:** for a hand-typed `[Pasted ~3 lines] #2` the two disagree in
  principle (drop: the bare token is dead because ` #2` follows; expand: the bare
  token is a live substring and gets rewritten). The probe shows the combination is
  currently unreachable from a normal paste, so this is a latent inconsistency, not
  a live bug.
- **Fix:** one predicate, used by both. `paste.rs` should expose
  `is_live_placeholder(text, placeholder)` and `state.rs` should call it.
- **If we skip it:** the next edit to either rule silently changes the other's
  meaning.

*Probe:* `probe_the_two_boundary_rules_agree` (passes; printed state shows both
rules evaluating the same text differently).

### 10. `replace_placeholders` / `expand_paste_placeholders` is a one-line wrapper

- **What this is:** `expand_paste_placeholders` (`paste.rs:138`) calls
  `replace_placeholders` and nothing else. `replace_placeholders` is private and
  has one caller.
- **Fix:** delete `replace_placeholders` and put its body in
  `expand_paste_placeholders`. −8 lines, one name instead of two for one idea.
- **If we skip it:** a reader has to jump to learn that the public name is the
  whole implementation.

### 11. The audit's own gate was marked met on a claim the code does not support

- **What this is:** the plan's outcome table says RT-562 "reports the line count it
  unwrapped through the status notice", and the ledger row says `CLOSED iter-2266`.
  The expand notice is real; the **collapse** notice (finding 6) is not, and the
  plan's own gate list says "collapse shows the transient notice".
- **Fix:** reopen RT-562's gate or narrow its wording, in the same iteration as
  finding 6.
- **If we skip it:** the ledger records a gate as met that a reader can disprove in
  one grep.

---

## Nice to have

### 12. The expand notice's wording is wrong about what happened

- **What this is:** `"Expanded the pasted 5 lines (the placeholder is back in your
  clipboard's text)."` (`history.rs:145`). The parenthetical is nonsense — the
  clipboard is not involved, and "back in your clipboard's text" describes nothing
  the operator can observe.
- **Fix:** `"Pasted 5 lines expanded."`
- **If we skip it:** the one piece of feedback the feature has is confusing.

---

## Counts

| | count |
|---|---|
| findings | 12 (6 must, 5 should, 1 nice) |
| probes written | 12 |
| probes that failed against shipped code | 8 |
| probes removed after the audit | 12 (tree clean) |
| lines in the slice under audit | +842 / −140 across 9 files |
| new public surface added | 4 fields, 5 functions, 1 enum, 1 enum impl |
| of those with a production caller | 5 functions, 1 enum, 1 impl (the 4 fields are all read) |

## Verdict

The architecture the slice chose is right — one owner for the store, the invariant
enforced at one choke point, the ladder gated on the recalled text. The
**implementation is not done**: three of the six must-fix findings are reachable
from key sequences an operator performs daily (`Ctrl+S` twice, expand, undo), and
one of them (finding 2) sends the model a token instead of the paste, which is the
single defect the whole feature exists to prevent. Finding 1 reopens a defect the
project fixed two iterations before this slice.

**Fix first:** findings 1, 2 and 5 together — they are one change (sanitise at the
store boundary) plus one field (stash the store with the text).

## Not checked

- No live TUI session: no paste was driven through a real terminal, so the render
  path and the tmux behaviour are unverified in this audit.
- The gateway's paste path (a channel paste, not a composer paste) was not traced.
- The `detect_pasted_path` branch of `handle_paste_data` was not probed.
- Cross-platform behaviour (Windows `Ctrl+V` burst path) was not tested.

## Applied (iter-2323)

All twelve findings were fixed in iter-2323. The four mutations run to prove the
new tests are load-bearing:

| mutation removed | test that caught it |
|---|---|
| the paste-boundary sanitise | `the_store_holds_the_sanitised_text_not_the_raw_clipboard`, `a_paste_drops_terminal_escapes_but_keeps_newlines_and_indentation` |
| the expand splice's sanitise guard | `expanding_a_paste_never_puts_a_control_byte_in_the_composer` |
| the stash restore's store assignment | `stashing_and_restoring_keeps_the_payloads` |
| the undo exit's drop invariant | `undoing_across_a_paste_leaves_no_orphan_payload` |

The two findings not fixed by code change: finding 11 was closed by reopening the
ledger rows (done — RT-559/561/562 now name what the first pass missed), and
finding 12's wording was replaced.

Net: +342 / −57 across 7 files, of which +217 is tests.

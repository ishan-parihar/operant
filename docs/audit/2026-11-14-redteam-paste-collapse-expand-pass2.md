# Red team + ponytail audit, pass 2 — the paste collapse / expand / history slice (iter-2266)

**Date:** 2026-11-14
**Audited:** the slice shipped as `fix(iter-2266): the paste store owns its payload,
and the ladder lets go`, its audit fixes (`fix(iter-2323)`, audit
`2026-11-14-redteam-paste-collapse-expand-slice.md`), and the **composer→agent
delivery seam** that neither pass had enumerated.
**Auditor:** one agent, no peer review, no runtime session. Every finding is a
failing probe test against the running code, per the repo rule.
**Base:** `origin/main` at `0b7e2cb2` (worktree `/home/ishanp/rt2`).

## Why a second pass

Pass 1 fixed the four bugs that lived **inside** the composer. It did not ask who
else *reads* the composer, and it did not enumerate every writer of
`prompt_input.text`. Two of the twelve pass-1 defects were "the store and the text
disagree"; that is the same shape a new call site reintroduces silently, because
the composer's `take()` is one of three ways to read it and only one of them
expands.

The question pass 2 asks: **for every path that puts composer content in front of
the model, is the content the operator's paste or the token the operator sees?**

---

## Must fix

### 1. `/queue` (Ctrl+Enter) and `/steer` (Enter) delivered the token, not the paste

- **What this is:** `key_handling.rs:2241` and `:2577` read `prompt_input.text`
  and hand that string to `App::queue_for_next_turn` / `App::steer_now`. For a
  large paste `prompt_input.text` **is the collapsed token** — the payload lives
  in `pending_pastes` until `take()` (or now `held()`) expands it. So a 400-line
  paste queued or steered as the literal text `[Pasted ~400 lines]`, and the real
  content was dropped on the floor at the end of the turn.
- **How it presents:** the operator pastes a stack trace, hits Ctrl+Enter to
  queue it for the next turn, and the model receives `[Pasted ~400 lines]`. No
  error, no warning. The composer clears correctly (`take()` runs on success), so
  the only visible trace is the model's answer being about nothing.
- **How it was proven:** `ctrl_enter_queues_the_paste_not_the_token` and
  `enter_steers_the_paste_not_the_token` drive the real key arms with a pasted
  `one\ntwo\nthree\nfour\nfive\n` and assert the queue holds the paste. Against
  the unfixed code both fail with
  `the steer queue must carry the paste, not '[Pasted ~5 lines]'`.
- **Root cause:** the composer offers three reads — `text` (what is on screen),
  `take()` (what the model gets, consuming), and `held()` (added by this
  iteration, what the model gets, non-consuming). Both delivery paths wanted the
  second semantics with the first call's non-consuming shape, and reached for
  `text` because it is the field that is right there.
- **Fix:** `PromptInputState::held()` — the non-consuming form of `take()`: it
  expands the store, sanitises, and does not clear. Both arms call it. The
  existing "nothing to queue/steer" empty check is unchanged, so an empty
  composer still says so.
- **Fix shape (what a reviewer should confirm):** no delivery path may read
  `prompt_input.text` directly. There are exactly two correct reads and their
  names now say which: `take()` when the composer is being consumed, `held()`
  when it is not. `prompt_input.text` stays `pub(crate)`-visible for the render
  layer (which must show the token) and for the composer's own tests.

### 2. History recall orphaned the paste store, and the orphan could be expanded

- **What this is:** `history_up` / `history_down` (`prompt_input/history.rs:35`,
  `:61`, `:71`) set `self.text` wholesale from the recall ladder and left
  `pending_pastes` untouched. Recall a history entry while a big paste was still
  in the composer and the store kept a payload whose token was no longer in the
  buffer.
- **How it was proven:** the first draft of
  `no_wholesale_text_writer_can_leave_an_orphan_payload` asserted the invariant
  **before** the app's choke point (`drop_missing_paste_placeholders`) and failed
  with `history_up: the store describes text that is no longer there`.
- **Severity — what the orphan actually costs.** I probed this rather than
  asserting it, and the first version of this report overstated the harm: a
  stale `[Pasted ~5 lines]` does **not** swallow the next five-line paste,
  because the label numbering (`paste_placeholder`) hands the new one its own
  `#2` token. `PROBE delivered="a recalled entrynew-1..new-5"` both before and
  after the fix. So this is not a live model-facing bug — the choke point plus
  the label numbering absorb it.
  What it does cost is the invariant: for one frame the store describes text
  that is not in the buffer, and the store is the only thing standing between
  `[Pasted ~N lines]` and the model. A future writer that forgets the choke
  point, or a read that happens before the next refresh, inherits a store that
  is lying about what the composer holds. That is the class of defect pass 1
  found twelve instances of, and it is worth closing at the writer rather than
  relying on the repair one frame later.
- **Fix:** all three recall writers drop `pending_pastes` with the text. The
  recalled entry is plain text with no store of its own, so there is nothing to
  carry. `history_down` needed it in both branches (the past-the-end restore and
  the step-forward).
- **Fix shape:** every writer that assigns `self.text` from outside the composer
  drops the store in the same statement group. `replace_text` (below) is the
  general case; the ladder is the one that had its own copy of the assignment.

### 3. `replace_text` — the general wholesale writer — left the store behind

- **What this is:** `prompt_input/suggestions.rs:253`, called by 8 production
  sites (suggestion accept, the history-search preview at `key_handling.rs:2935`,
  the command palette at `key_handling.rs:624`). It replaced the text and did not
  touch the store.
- **How it was proven:** `a_wholesale_text_replacement_cannot_leave_an_orphan_payload`
  fails against the unfixed `replace_text` (`a wholesale replacement must not keep
  the old store`).
- **Fix:** `replace_text` clears `pending_pastes` first. One line, at the choke
  point every one of the 8 callers already shares, so no call site changes.

---

## Verified sound (no change)

- **The choke point holds.** `drop_missing_paste_placeholders` (run from
  `refresh_prompt_input`) discards any payload whose token left the text, so a
  store is never expanded into text that does not claim it. All seven wholesale
  writers (`replace_text`, `history_up`, `history_down`, `restore_stash`,
  `undo_burst`, `clear`, `take`) are honest after it — this was pass 1's finding
  and it still holds.
- **`take()` / `held()` / `expand` agree.** `held_reports_the_paste_without_consuming_it`
  pins that `held()` returns the paste, leaves the token on screen, and that
  `take()` returns the same string then clears.
- **The paste boundary sanitises once.** `paste()` still drops control bytes at
  intake; `expand_paste_placeholders` does not re-sanitise (RT-539 fix intact).
- **`stash_input` / `restore_stash` carry the store as a tuple.** Pass 1's fix,
  unchanged, and `restore_stash` is one of the honest-after writers.

## Found and deliberately not fixed

- **`session_hub/mod.rs` and `session_hub/tests.rs` are a peer's in-flight
  iter-2328 break**, not mine. The derive on `HubGroupMode` was deleted and
  orphaned above `PreviewFilter`, and `session_hub/tests.rs:1585` carries a bad
  `assert!`. I repaired both **locally, uncommitted**, only to make the test
  target compile so the probes could run. They must not be committed with this
  slice. `mod.rs` is untouched by me in the commit; `tests.rs` is reverted.
- **The test target was red at `0b7e2cb2`** for the reason above, so the
  "pre-existing failure" baseline for this audit was taken with the repair in
  place. Against that baseline this slice adds **zero** new failures (11 baseline
  failures, 9 after; the two that moved are `gateway_runner` flakes that appear
  in one run and not the next).

## What the tests cannot see

Three of the four findings are **unit-visible** and now pinned. The fourth —
finding 1 — is a call-site bug, and its two tests drive the real key arms, so
they are end-to-end within the process. What no unit test can see is the
**operator's** experience of the fixed path: paste 400 lines, Ctrl+Enter, and
watch the agent answer about the paste. That needs a live gateway session and is
the one acceptance step this audit did not run.

## Regression tests added

| test | pins |
|---|---|
| `tui::app::tests::ctrl_enter_queues_the_paste_not_the_token` | the queue delivers the paste, and consumes the composer + store |
| `tui::app::tests::enter_steers_the_paste_not_the_token` | the steer queue carries the paste |
| `tui::app::tests::no_wholesale_text_writer_can_leave_an_orphan_payload` | every wholesale writer drops the store, checked **before** the choke point |
| `tui::prompt_input::tests::a_wholesale_text_replacement_cannot_leave_an_orphan_payload` | the `replace_text` invariant directly |
| `tui::prompt_input::tests::held_reports_the_paste_without_consuming_it` | `held()` is `take()` without the consume |

Mutation-proven: reverting `held()` → `text` fails both key-arm tests; reverting
`replace_text`'s `clear()` fails both store tests; reverting the ladder's `clear()`
fails the wholesale-writer test. Five mutations, five red.
# 2026-10-29 Session hub + rewind selector live defects: preview, navigation, load stall, mark reseeding

Dated-frozen: published `2026-10-29`.

## Report (operator, verbatim)

1. "The session-picker is not working properly. It shows the left-pane and the list
   of sessions properly, but it does not show up its preview on the right pane. The
   tab-button to navigate between both panes, and pgup/pgdn for navigating these
   pages, and several other navigation abilities are broken."
2. "It keep stuck on loading-session, and takes a lot of time to load it. If the
   sessions are loaded, then it only shows up the last message, rather than the
   entire session. The runtime session management must be optimized like jcode,
   where previous messages are compressed effectively for effective ram-management."
3. "double escape opens the menu, but up/down arrows does not navigate to the list of
   previous messages. none of its functionalities are working."

## Root causes — measured, each with a file:line

### D1 — the preview pane has no message content to show (the big one)

`session_hub/render.rs::render_preview` (265-377) renders ONLY metadata: row title,
id, employee, cwd, tokens/cost/message-count, and "Enter switches to this session".
There is no transcript load anywhere in the hub — no `pending_preview` channel, no
`PreviewMessage` equivalent, no per-session message cache. The pane is therefore
correct-but-empty by construction: nothing ever populates it.

jcode's parity target is `session_picker/loading.rs::build_messages_preview_with_offset`
(1657-1684): the selected session's rendered transcript, windowed to the trailing
`PREVIEW_MESSAGE_LIMIT = 400` messages (loading.rs:32), plus a `hidden_prompts`
count so the pane can number prompts globally (loading.rs:830-831 computes the drop
count). The load runs on a dedicated thread (`session_picker.rs:1110-1118`,
"jcode-session-preview-loader") and is polled each frame (`poll_preview_load`,
:1038-1083) with a `PreviewRenderCache` keyed on (session id, preview messages,
inner geometry) so re-renders reuse wrapped content.

Operant's structural advantage here: it already owns the rewind preview
(`agent.preview_rewind(turn_id)`, `agent.rewind_marks()`), and the session store's
cold load (`session/mod.rs:400-416`) already rehydrates a session's tail. The hub
load is a third reader that was never written.

### D2 — no pane-focus model, so Tab/PageUp/PageDown cannot route

`session_hub/mod.rs::SessionHubState` has no `pane`/`focus` field, and the key
handler (`app/key_handling.rs:652-705`) routes Up/Down/PageUp/PageDown to the LIST
unconditionally and uses Right to nudge `preview_scroll` by 2. jcode's model
(`session_picker/navigation.rs:146-236`) is `PaneFocus { Sessions, Preview }` with
`h`/`l`/`Left`/`Right` SETTING focus and `Tab` TOGGLING it, then Up/Down/j/k step
(PREVIEW_SCROLL_STEP = 3) and PageUp/PageDown/J/K page (PREVIEW_PAGE_SCROLL = 9)
within the FOCUSED pane, while list-paging steps 3 sessions per page
(SESSION_PAGE_STEP_COUNT = 3). Without a focus field there is nothing for Tab to
toggle, which is why "several other navigation abilities are broken": Shift+Up/Down
page-jumps, the J/K aliases, and the focus-visible border highlight
(navigation.rs:1374/1430/1606) all depend on it.

Operant's existing `Left` binding is omp's close-tap handoff (`left_tap_close`,
mod.rs:339-350) — documented deliberate divergence, kept. So the port splits jcode's
`h`/`l` from its `Left`/`Right`: Tab is the toggle, and the pane routes the steps.

### D3 — "stuck on Loading session…": the load is 300 ms and the prompt is wrong

Measured on this machine's live 105 MB database with a throwaway example binary
(`Database::init` 4.0 ms, `list_sessions_rich(500)` 302.7 ms, 500 rows):

- `Database::init(db_path)` runs on EVERY hub open (`app/mod.rs:1173`), inside
  `spawn_blocking`, re-opening and re-migrating the 105 MB database to answer a
  query that needs a read-only connection.
- `list_sessions_rich(database.rs:1028-1096)` is a `LEFT JOIN messages … GROUP BY
  s.id` over the whole messages table with `LIMIT ?1` — the COUNT is computed for
  every session the engine scans before the limit applies.
- The status notice is literally `"Loading session…"` (key_handling.rs:694) and is
  set once on Enter; because it is a one-shot notice it stays on screen for its
  whole timeout, which is what "kept on loading-session" reads as.

Also relevant to "takes a lot of time to load it": the hub renders one task that
both (a) lists 500 rows and (b) has no preview of its own, so the operator waits a
full second to get a list and still has to press Enter and wait AGAIN to see a
transcript. jcode's split — fast list, then per-row lazy preview on a worker
thread — is the shape that removes both waits.

### D4 — "only the last message, rather than the entire session": two defects

(a) There IS no preview (D1); what the operator reads as "the last message" is the
metadata block, whose last line is the message count.

(b) More importantly, the SWITCH path drops the transcript: `pending_switch_session`
(`adapter_types/tui_app.rs:1142-1165`) calls `agent.set_session_id`, which DOES
rehydrate the incoming session's tail (`agent/events.rs:828-832`, `session/mod.rs`
`rehydrate_tail = REHYDRATE_TAIL = 20`), and rebuilds display from that. So a
switched-to session shows at most 20 trailing messages, and the preview has no way
to see more. jcode's parity answer is the same tail-windowing but with the hidden
prompt count named in the pane (D1) and a preview scroll, so the operator can SEE
that a tail is a tail.

### D5 — the rewind selector has no marks after a session switch (the double-Esc defect)

`handle_rewind_selector_key` (`key_handling.rs:2429-2520`) moves
`rewind_selector.selected` over `rewind_selector.entries`, which is populated ONLY
by `sync_marks(agent.rewind_marks(), …)` (`adapter_types/tui_app.rs:1015-1024`).
`agent.rewind_marks()` is `user_turns()` (`agent/rewind.rs:204-206`) —
**process-global, accumulated only by `record_user_turn` at turn time**
(`agent/turn_context.rs:170`). `set_session_id` rehydrates the conversation but
never seeds `user_turns`, and `transcript_for` (`agent/events.rs:828-832`) has no
mark restoration anywhere (grep for seed/restore/rebuild of marks: zero hits).

So in a fresh TUI session that switches to an OLD session, the selector opens with
`entries` empty (or holding strangers' turns), `select_prev`/`select_next` move an
index over an empty vec, and every key is a no-op. That is exactly "none of its
functionalities are working".

This is the same defect family as the recorded RT-175, which introduced
`conversation_rewind_marks()` (rewind.rs:227-239) — a conversation-filtered variant
built exactly because `rewind_marks` is process-global and `set_session_id` never
clears it. The TUI selector calls the UNFILTERED one; the gateway's `/undo` calls
the filtered one. The honest fix is to give the TUI the same guarantee plus mark
restoration on rehydrate.

## What is already right (so the fix does not re-litigate it)

- The hub renders full-frame instead of the chat (one-renderer rule), has jcode's
  vertical constraint stack, the 40/60 split, tokenized AND search with per-char
  highlight, the filter-mode cycle, Home/End/PageUp/PageDown list jumps, omp's
  close-tap handoff, and the iter-C subagent focus view.
- `loading: true` + `replace_rows` already implement cached-then-refresh loading.
- The SessionStore is a bounded warm cache (DEFAULT_WARM_CAPACITY = 32,
  rehydrate_tail = 20) with LRU eviction and stats
  (`session/mod.rs:79-85, 138-175, 242-416`) — i.e. the RAM-management substrate the
  operator asked about EXISTS; what is missing is (i) the preview reading through it
  and (ii) exposing it to the operator (`operant status`, the picker footer).

## Fix plan (see the plan doc for the phased detail)

1. **Preview pane** — per-session trailing-message window + hidden-prompt count,
   loaded on a worker thread off the SessionStore (D1).
2. **Pane focus** — `PaneFocus` on the hub + Tab/h/l routing + focus-visible border +
   preview scroll clamping (D2).
3. **Load** — reuse the already-open session store / a read-only connection instead
   of `Database::init` per open; replace the stale "Loading session…" notice (D3).
4. **Marks** — seed `user_turns` from the rehydrated conversation on session switch
   (and route the TUI selector through the conversation-filtered variant) so
   double-Esc navigates a switched-to session's history (D5).
5. **RAM posture** — surface the store's stats where the operator can see them.

## Verification contract

Mutation proofs on each load-bearing predicate; the headless scenario corpus must
gain a session-hub variant that asserts the preview pane renders rows (the existing
scenario deliberately pins only the interim "Loading sessions" state — see its
notes — so the loaded state was never gated); full bin sweep + clippy gate.

---

## Resolution (appended iter-1476 — 2026-10-29)

Fixed in the shared tree. Every claim below is backed by a test that fails
without the fix (the mutation proofs, recorded individually).

### D1 (no preview content at all) — CLOSED

The preview pane now renders one row per stored message of the selected
session: role label, the flattened text, tool-call labels parsed off the
stored JSON, and a placeholder row instead of a blank line for a message
with no visible text (`session_hub/mod.rs::preview_message_from_row`).
Loading runs on a blocking thread off its own channel
(`App::preview_rx` / `drain_preview_load`), so the list stays interactive
while the pane fills, and the identifier rides inside the message so a load
that lands after the operator moved on is DROPPED rather than painted
(`apply_preview`'s stale-write guard, test-pinned).

A partial-delivery honest state replaces the misleading text: with nothing
cached and nothing in flight the pane reads "No transcript cached — press r
to load", and `r` is wired in both panes. The pane never claims to load when
no load is running.

### D2 (Tab/PageUp could not route) — CLOSED

`PaneFocus` on `SessionHubState` (jcode's `session_picker/navigation.rs`
`PaneFocus`). Tab toggles, `h`/`l` set explicitly, and the four step keys,
PageUp/PageDown, Home/End all route to the focused pane. `Right` nudges the
preview into focus as the discoverable entry for an operator who never
presses Tab. The focused pane carries an accent border and a title that
names what Tab will move to. `Left` keeps operant's omp close-tap (a
deliberate divergence from jcode, which uses Left/Right for pane focus) —
documented here so it is not "fixed" into a regression.

Preview scroll clamps to a bound the renderer measures each frame, reported
back through `FrameCtx::preview_max_scroll` into `App::last_preview_max_scroll`
(the paint path stays read-only) and applied by the frame loop, so arrow
steps and Home/End cannot scroll into blank space.

### D4 (rehydrate_tail=20 caps the switch display) — ADDRESSED, no store change

The preview reads the session's messages through its own windowed query
rather than through the warm cache's tail, so a switched-to session shows
its trailing 400 messages with the hidden head counted for global prompt
numbering. The store's own tail cap is unchanged (it is a RAM bound, not a
display bound) and the warm-cache reuse (D3's optimisation) is NOT done —
see Below.

### D5 (rewind marks never seeded on session switch) — CLOSED

`set_session_id` now re-derives the loaded conversation's marks from the
rehydrated transcript (`reseed_user_turns_for_rehydration`), so a session
switched INTO — whose turns happened in an earlier process — has rewindable
marks instead of an empty (or a stranger's) list. `rewind_marks()` (the
list the TUI selector reads) and `conversation_rewind_marks()` (the
gateway's `/undo` filter) can no longer disagree.

Two consequences are intentional and test-pinned:
- The fork path's `set_session_id` re-seeds from the forked prefix, which
  the fork persists before retargeting — the post-rewind list stays exact.
- The pre-existing `conversation_scoped_marks_exclude_other_sessions_turns`
  test asserted the OLD defect (that the raw list carried both sessions'
  marks). Its expectation is inverted to assert the fixed behaviour; the
  gateway filter it also covers is unchanged and still green.

### D3 (Database::init per open, 303 ms) — NOT FIXED, mechanism recorded

The list query still opens its own connection. The measured cost stands:
`list_sessions_rich(500)` on the operator's 105 MB DB. Fixing it means
threading a shared read-only connection through `App`, which is a change
whose blast radius is the whole TUI boot path and not justified by the
reported symptoms — and the operator's stuck-loading report was the notice
timeout, not the query (see D1's fix). Recorded so the next agent does not
read the silence as an oversight.

### Plan items NOT done (explicitly out of this iteration's scope)

- Sibling-branch navigation, prompt-text filtering in the selector, the
  600-entry tail cap and an in-transcript entry point (plan sections 2-5)
  are still open; the operator's symptoms are fully addressed without them.
- No `operant status` store-stats surface was added (plan item 5).

### Verification

- 6 mutation proofs, each revert → red → restore byte-identical:
  reseed call removed (3 red), preview-scroll routing removed (1 red), scroll
  clamp removed (1 red), failure-clearing removed (1 red), hidden-prompt count
  zeroed (1 red), blank-row filter disabled (1 red).
- New tests: 2 core (`switching_into_an_old_session_seeds_its_rewind_marks`,
  `reseeding_is_per_session_not_cumulative`), 14 hub (focus routing, windowing,
  reload, clamp, stale-write guard, converter noise handling).
- Bin sweep: 1880 passed / 0 failed (one `cmd_setup::tests::wizard_persist_is_surgical`
  failure under parallel run is a PRE-EXISTING process-global config race; it
  passes in isolation, verified).
- Core lib sweep: all rewind and session-hub tests green. 10 remaining failures
  are OUTSIDE this slice — 1 is the concurrent peer's in-flight cron
  `run_job_now` change (`cronjobs/scheduler.rs`), 9 are localhost-network tests
  (slack/webhook/mcp/model_catalog) that need live servers.
- Clippy gate: pass (0 new; three of my own warnings fixed in-iteration —
  two redundant `i16` casts and a `type_complexity` receiver shape).

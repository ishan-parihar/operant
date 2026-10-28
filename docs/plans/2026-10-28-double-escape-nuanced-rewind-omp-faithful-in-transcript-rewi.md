# 2026-10-28 Double-Escape nuanced rewind: the omp-faithful in-transcript rewind selector upgrade

Dated-frozen: published `2026-10-28`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Operator ask

"Also check for the double-escape to scroll to any previous message, like nuanced
rewind for tui. Investigate its implementation on omp source code, and create a plan
to implement its upgrade."

The double-Esc → rewind selector path EXISTS in operant
(`key_handling.rs:1195` double-tap → `open_rewind_selector()`,
`[tui] double_escape_action` = "rewind" default, `tree`/`none` also wired). So this
plan is not "make double-Esc work" — it is "make double-Esc navigable to ANY previous
message, the way omp's is", which is an upgrade of the selector's MODEL and its
keyboard reach, not the entry chord.

## Mechanism — omp source, measured (not inferred)

Source: `external-repos/oh-my-pi`.

### 1. The entry contract (input-controller.ts:570-655)

Omp's Esc handler is a ladder, and double-Esc is its LAST rung:

1. streaming → `abortStreamingTurn()` + `pauseLoop()` + drop the captured prompt so
   the 800 ms auto-resubmit never fires.
2. focused subagent → clear typed text if any, else `unfocusSession()`.
   Comment: "// double-escape backtrack (/tree, /branch) stays main-only".
3. collab guest → ask the host to interrupt.
4. loader running + `cancelPendingSubmission()` → done.
5. `awaitingAsyncWork` → `restoreQueuedMessagesToEditor({ abort: true })`.
6. bash / eval / bash-mode / python-mode → abort or clear.
7. streaming (again, the standing turn) → abort.
8. **draft present** → `this.ctx.lastEscapeTime = 0` ("Esc must not destroy an
   in-progress draft") — the double-tap clock is RESET, not armed.
9. **empty draft** → the double-tap window: `now - lastEscapeTime < 500` runs
   `cfgDoubleEscapeAction.get(settings)`:
   - `"tree"` → `showTreeSelector()`
   - `"rewind"` (DEFAULT) → `showUserMessageSelector()`
   - `"none"` → nothing
   After acting: `ui.requestRender(true)` (a FORCED viewport repaint, with the
   comment explaining why `resetDisplay()` is wrong here — it replays the whole
   committed transcript and blocks on PTY backpressure for tens of seconds on long
   sessions, so the selector opens invisibly and double-Esc reads as dead), then
   `lastEscapeTime = 0`.
   Otherwise the first press only stamps `lastEscapeTime = now`.

Operant's current ladder has the same shape (streaming cancel, rewind confirm
cancel, double-tap → selector, `double_escape_action` config) — the ENTRY is parity.
What follows is what is not.

### 2. What the selector is — a transcript browser, not a list

`RewindSelectorComponent` (`packages/tui/src/overlays/rewind-selector.ts`, 1114
lines) is NOT a list of prompt strings. It is:

- `#builder: ChatTranscriptBuilder` + `#browser: TranscriptBrowser` — the SAME
  transcript renderer the live chat uses, replayed inside a fullscreen
  alternate-screen overlay (`showOverlay(..., { fullscreen: true, margin: 0,
  width: "100%", maxHeight: "100%" })`).
- `#targets: OutlineTarget[]` — the rewindable points, from
  `chat/transcript-outline.ts`. The selector INITIALISES ON THE LAST TARGET
  (`#selected = Math.max(0, targets.length - 1)`), so the selector opens at the
  bottom of your own history.
- Outline gutter (`prepareOutline`) marking which rows are selectable vs
  non-selectable, with per-frame `#mainVisible` bookkeeping ("renders at least one
  non-blank row") so invisible targets never win the selection.
- `#selected` turn is OUTLINED; everything after it is dimmed (the preview of what
  a rewind drops). Operant already has this dimming (`draw_rewind_selector`),
  which is why the visual gap is smaller than the behavioural one.
- **Branch strip** (`#variantCache: SiblingColumn[]`, `#activeVariant`, `#slide`
  camera animation): selecting a turn that has SIBLING branches adds a column
  strip (`#siblingBranchPaths`) — every other child of that turn's parent, each
  unrolled along its most-recent descendant chain, selectable with ←/→ and a
  fractional-slide animation between positions. This is "what if at that question
  you had said something else" navigation.
- **Filter that searches the RENDERED transcript** (`#filterInput`, `#filterPatterns`,
  `#nativeTexts`, `#mainRows` + a `#rowText` WeakMap over cached row arrays): the
  filter matches what is ON SCREEN, with a match-count readout
  (`${at >= 0 ? at + 1 : "-"}/${matches.length}`) and step-through navigation.
- **Tail replay** (`#entries` vs `recentTranscriptEntries(entries, limit = 600)`):
  the whole branch may be replayed at construction, trimming to the last 600
  entries sliced at a USER-REQUEST boundary (`transcript-entry.ts:79-85`), with
  `#truncated = tail.length < entries.length` driving a "more history exists but is
  not replayed" state.
- Keys (from the action bar): up/down, left/right (user turns), the action bar's
  `Show all` (`cancel`, key: escape) and the rewind button. `handleNativeEvent`
  routes the SAME actions: `rewind` → `#selectFiltered()` while filtering else
  `#selectOutlined()`; `cancel` → `#closeFilter()` while filtering else
  `deps.onCancel()`.
- `targetCount` gates the whole surface: hosts skip mounting when zero
  (`selector-controller.ts:1341-1346`).

### 3. What a rewind DOES (selector-controller.ts:1400-1443)

`#rewindFromTranscript(entryId, done)`:

- `isUserRequestEntry(entry)` decides how the target is treated.
- Selecting the CURRENT LEAF when it is not a user request → "Already at this
  point" (no-op, no work).
- `#treeRewindBoundary(entryId, realLeafId)` computes a FAST path: after
  `navigateTree`, if the leaf landed on the expected id, a `truncateTranscriptFromMessage`
  is enough and the full replay (`renderInitialMessages({ clearTerminalHistory: true })`)
  is skipped.
- `navigateTree(entryId, { summarize: false })` — the REAL rewind. A user-request
  target rewinds PAST ITSELF (leaf → parent) and its draft text replaces the editor
  text (`result.editorText`), so it is a real move even when it IS the current leaf;
  every other target lands the leaf ON the entry.
- On success the prompt comes back to the composer as an editable draft
  (`setDraft(result.editorText, result.editorImages)`) when the target was a user
  request or the editor is empty.
- Status lines: "Rewound to selected point", "No messages to branch from",
  "Navigation cancelled", or the error message.

### 4. Entry points into the selector (three, not one)

1. double-Esc with an empty draft (§1, default).
2. the transcript action handler (`interactive-mode.ts:2397`): a per-message
   toolbar action `{ act: "rewind" }` on a USER MESSAGE row — a clickable/keyed
   affordance in the transcript itself, same handler the retry / switch-model /
   resume / copy row actions use.
3. the slash command path (`slash-commands/builtin-session.ts` — `/rewind`).

So the reach is: any message in the transcript can become a rewind, from the
transcript you are already reading.

## Where operant stands now (measured, honest)

ALREADY PARITY:
- double-Esc entry + `[tui] double_escape_action` ("rewind" | "tree" | "none").
- fullscreen selector over the REAL transcript lines
  (`prepare_body_for_overlay`), selected turn outlined, everything after dimmed,
  filter over prompt texts (`f`), Enter rewinds, Esc cancels, header/footer.
- `sync_marks` keeps the cursor on the same turn id as marks change while open,
  and falls back to the last turn — the same "opens on your latest" posture.
- The rewind engine forks a new session with `parent_session_id = old`
  (`agent/rewind.rs:101+`), rebuilds display from the agent's forked store, and the
  rewound prompt returns to the composer ready to edit and resend (with a
  "previous path kept as session {short}" notice — the operator's "did I lose it?"
  question answered in the status row).
- preview stats: `preview_rewind(turn_id)` → (kept messages, tokens) in the footer.
- compacted turns are marked and refused with an inline notice.

NOT PARITY — the real gaps, in descending operator value:

1. **Only user prompts are reachable.** `RewindEntry` is one-per-user-turn
   (`UserTurnMark`). Omp's `OutlineTarget` set is broader: its outline marks
   selectable points across the branch, and its rewind of a non-user target lands
   the leaf ON that entry (mid-assistant, tool boundary) instead of a whole turn.
   For the operator ask ("scroll to ANY previous message") this is THE gap: the
   work answer is that assistant/tool turns can be rewind points too.

2. **No ←/→ branch navigation.** Omp's sibling-branch strip lets you, at a chosen
   turn, walk the alternate things-you-said at that point and rewind into one.
   Operant's sessions carry `parent_session_id` (so the data model EXISTS —
   `iter-1461`'s sibling work is the same family) but the selector never reads
   sibling paths. This is the "nuanced" half of "nuanced rewind".

3. **Filter matches prompt text, not the rendered transcript.** Operant's filter is
   a substring over `RewindEntry.text`; the entry text is the PROMPT, so nothing in
   the assistant's answer is searchable, and there is no match-count/step-through.
   Omp's filter matches rendered rows (plus a native text fallback), shows
   `n/total`, and steps matches.

4. **No truncation model for long sessions.** Omp replays a 600-entry tail sliced
   at a user boundary and names the unreplayed history. Operant renders the whole
   transcript every frame in the overlay with no tail cap — on a long session the
   overlay's first frame is where the cost is.

5. **No in-transcript entry point.** Omp's user-message toolbar action
   (`{ act: "rewind" }`) lets the operator pick the message in the transcript
   itself. Operant's is `/rewind <n>` + double-Esc. The slash syntax counts back
   from the NEWEST, which is a documented deliberate divergence from jcode's
   absolute-index reading — but the toolbar affordance is a different feature.

6. **Renderer is a second renderer.** Omp replays the live `ChatTranscriptBuilder`
   (one implementation, two hosts). Operant's overlay re-derives wrapped lines via
   `prepare_body_for_overlay` and re-implements outline/dim/highlight on top of it.
   That works, and the seam is real, but every renderer upgrade must be landed
   twice. (Deliberate-scope decision, not a defect.)

7. **No keybinding-course awareness of the selector's own keys.** Omp's selector
   publishes its keys as an action bar the transcript actions share, so one
   action vocabulary drives mouse/toolbar/keys. Operant's selector keys are
   hardcoded in the key_handling arm.

## Plan — the upgrade, phased

Each phase is one iteration with its own commit, push and (for code phases) deploy.
Ordered so that every phase is shippable and useful alone.

### Phase 1 — reach ANY message, not just user prompts (highest value; the operator's ask)

- Extend the mark model: the agent already tracks turns; add a per-message rewind
  target list (`RewindTarget` = message index + kind + label) instead of
  user-turn-only marks, keeping `turn_id` as the user-turn convenience view for
  `/rewind <n>` and the existing tests.
- `rewind_cut_point` gains a message-index cut so a mid-assistant/tool-boundary
  rewind truncates correctly (the existing user-turn path becomes the
  "user-target rewinds past itself" special case of one rule).
- Selector gains an outline column marking which rows are targets, so non-target
  rows cannot win the selection, and `<`/`>` (or ←/→) step target-to-target
  rather than whole-turn-to-whole-turn.
- Entry: `/rewind` with no `n` opens the selector (parity with action 3), and
  `/rewind <n>` keeps its newest-first count for user turns (documented divergence
  stays).
- Regression tests: (a) mid-assistant rewind truncates at that message and the
  composer gets the following user prompt's draft (omp's
  rewind-past-itself semantics), (b) selecting the current leaf mid-assistant is
  "Already at this point", (c) non-target rows never become the selection while
  stepping.
- Mutation proof on the cut-point predicate (revert → the mid-turn test fails).

### Phase 2 — the filter searches the transcript you can read

- Replace the prompt-substring filter with a search over the SELECTOR's OWN
  rendered rows (the overlay already materialises `prepared.wrapped_lines`), with
  omp's `n/total` readout and step-through (`Enter` steps matches, `/` re-opens
  the filter), while keeping the existing substring behaviour as the fallback when
  no frame rows exist.
- The match stepping moves the selection, so a filtered rewind lands where the
  operator is looking.
- Tests: filter matches an assistant message's words (today it cannot), the count
  readout, stepping wraps/clamps, rewind-from-filtered-selection rewinds the target
  under the match.

### Phase 3 — sibling branches at the chosen turn (the "nuanced" half)

- Model: session tree children of the target's parent (operant has
  `parent_session_id` + the new hub's row loader as precedent; the sibling reading
  is a child/parent query, not new persistence).
- UI: a branch strip beside the outline when the selected turn has siblings; ←/→
  (or `[`/`]`, now backed by iter-1461's prompt-jump precedent for bracket keys)
  switch between the current path and each sibling; the selector previews the
  sibling's continuation; Enter rewinds into it.
- The viewer is READ-ONLY over sibling history (no replay cost): reuse the
  existing loader, cap the preview depth, and name the sibling's session id in the
  footer (the "previous path kept as {short}" convention already exists).
- Tests: sibling strip appears only when siblings exist, selection survives a
  sibling switch, rewind-into-sibling lands the expected session.

### Phase 4 — long-session tail replay + the in-transcript entry point

- Tail cap: replay the last N messages sliced at a message boundary when the
  session exceeds it, with a "older history not replayed" marker in the header
  (omp's `#truncated`), so the overlay's first frame cost is bounded on long
  sessions. The user-turn fast path (whole turns, no slicing) stays the common
  case.
- In-transcript affordance: user-message rows carry a rewind affordance opening
  the selector seeded at that message (parity with omp's `{ act: "rewind" }`
  transcript action) — mouse AND a key on the focused row, sharing ONE action
  handler with `/rewind` and double-Esc.
- Tests: tail marker appears only past the cap, the seeded selector opens on the
  message the operator clicked, the action handler is shared by all three entries.

### Phase 5 — optional, only if Phase 1-4 measured boring

- Fold the overlay renderer onto the live body renderer (kill the second
  implementation) — a refactor with a behaviour contract, so it needs its own
  mutation-proven tests, and it is genuinely optional.

## Explicitly OUT of scope

- The `"tree"` double-Esc action (session tree selector): operant landed the
  session hub in iters 1458/1459/1460; the tree selector is a different surface and
  the hub covers its job. Not re-litigated here.
- `[tui] double_escape_action = "none"`: already wired, no change.
- The `estimate`/`summarize` rewind variants (`navigateTree(..., { summarize })`):
  operant's rewind is a fork-then-truncate, and summarization is a context-policy
  question, not a selector question.

## Verification contract (every phase)

- `cargo fmt --all`; `./scripts/check.sh check -p operant-cli --bin operant`;
  `bash scripts/clippy-warning-gate.sh`; full `cargo test -p operant-cli --bin operant`.
- Each load-bearing predicate mutation-proved RED→restored, cited in the commit body.
- Shared-tree discipline: `key_handling.rs` and `tests.rs` are contended — hunk
  ranges checked disjoint before staging, path-limited `git add`, never a bare
  `git add -A`, never stashing/rebasing over peer WIP.
- Deploy for code phases: provenance-gated release build, `~/.local/bin`, gateway
  restart, `/proc/<pid>/exe` md5 three-way match. (`/usr/local/bin` needs an
  interactive sudo that this environment does not provide — documented, not
  silently skipped.)

## Sequencing note

Phase 1 is the operator's literal ask and is independently useful. Phases 2-4 each
add one of omp's layers in descending value. Nothing here is blocked on a peer
except the shared-file discipline above; the entry chord already works today.

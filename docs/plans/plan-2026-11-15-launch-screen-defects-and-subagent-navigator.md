# Plan — launch-screen defects, the subagent dock, and the session-scoped navigator

**Date:** 2026-11-15 (revision 1)
**Base:** `origin/main` @ `30b79753` (NOT this checkout's local `main` — see §0)
**Status:** outline. Writing this file executes nothing.

Read with its parent: [plan-2026-11-15-session-hub-surfaces-and-agent-identity.md](plan-2026-11-15-session-hub-surfaces-and-agent-identity.md).
Its T5/T6/T7 already specify the subagent navigator; this plan does not restate
that design, it supplies the two defects that block it and the tranche order that
lands everything without a half-wired surface.

---

## 0. Base revision — read this before any code

This checkout's local `main` is **not** the published revision.

| Fact | Measurement |
|---|---|
| Local `main` HEAD | `10357edc` "the employee axis told a lie of omission" |
| `origin/main` HEAD | `30b79753` "a model is the session's, not the operator's every conversation" |
| Local-only commits | 7 — every one a **duplicate title** of a commit already on `origin/main` under a different hash (rebased or cherry-picked, never pushed) |
| Remote-only commits | 45 |
| Working tree | 53 paths dirty, including another writer's substantive in-flight work (`gateway_runner.rs` +536, `cmd_gateway.rs` +264, `tui_app.rs` +224 — not formatting; `git diff --ignore-all-space` still shows 2,246 changed lines) |

Two of the four operator asks are **already implemented on `origin/main`** and
absent here:

- `3a7cc279` "the session title is the header, and the picker can rename" — closes
  ask #2 (session title as the TUI header) and ask #3 (rename from the picker).
- `b021d13d`, `30b79753` — the identity/model work that lands beside it.

**Consequence for execution.** Land nothing on `10357edc`. Reconcile to
`origin/main` first (the dirty tree belongs to a peer; it is not mine to commit,
stash or revert). Every "done when" below is written against `origin/main`, so a
fix verified against the stale tree would prove nothing about what ships.

---

## 1. What the operator reported, and what each one actually is

### 1.1 The "dlg-hash-number" overlay — CONFIRMED, root-caused, threshold computed

> "There is an overlay of `dlg-hash-number` as the delegated stale log, behind
> which the text is hidden and selectable is: `/model to switch · Large Stack /
> v0.3.0 / /login to add provider / ● openai(key) / ● custom`. This means that the
> subagent swarm preview on operant has bugs."

**It is not a dialog.** `dlg-<epoch>-<n>` is a **delegation id**, and the rows are
the **background-task dock** — `background_tasks::render_rows`, painted from
`render/mod.rs:108-122` into `rows_area(message_area(), rows.len())`.

**Mechanism, read from the code and then corrected against a failed repro.**

1. `BackgroundTaskRegistry::sync_from` (`background_tasks.rs:191-237`) **appends a
   row for every record it has ever seen and never removes one.** There is no
   `retain`, no TTL, no cap. `release_all` clears only on session teardown.
2. `render_rows` (`background_tasks.rs:288-297`) caps only by the container:
   `rows[start..]` where `start = rows.len().saturating_sub(area.height)`. With
   more rows than the band is tall, **every row renders and the band is full**.
3. `rows_area` (`:308-326`) docks to the bottom of `message_area()`.
4. The dock paints **after** `operant_ui::draw` (which owns the header —
   `render/mod.rs:65` then `:116`), so it covers the header rather than sitting
   under it.

**Where the rows come from — and the correction.** The rows are records in the
process-global registry (`async_delegation.rs:125-128`), read by
`list_records()`. That registry is filled two ways:

- **In-process**, at dispatch (`sub_agent_tool.rs:1204`,
  `try_create_record`) — the ordinary case.
- **From the ledger on boot**, via `enable_persistence`
  (`async_delegation.rs:155-160`), called from
  `create_model_client_with_fallback` (`main.rs:1281`). The real TUI reaches
  that; `tui debug simulate` does **not**, which is why seeding
  `delegations.json` and running the simulator renders an **empty** dock.

**Recorded because it changed the repro, not the defect.** My first attempt
seeded `delegations.json` with 14 records and ran the simulator; the dock stayed
empty, because the simulator never calls the ledger loader. That does not weaken
the finding — it sharpens it: the accumulation is driven by **in-process
records**, and the ledger adds a second, restart-surviving source on the real
path. The deterministic repro is the registry itself:
`BackgroundTaskRegistry::sync_from(&records, now)` with 14 records, then
`rows_area(band, 14)` — which is exactly the seam the module's existing tests
already use (`background_tasks.rs:375-440`, `pending()` / `settled()` builders,
no process-global, no disk).

**The exact threshold, computed from `rows_area` + `render_rows`.** The dock's
top row is `band_height - min(row_count, band_height) - 1`. It reaches the top of
the band (and therefore the header) exactly when
**`row_count >= band_height - 1`**:

| band height | rows | dock top row | covers the header? |
|---|---|---|---|
| 30 | 14 | 15 | no |
| 30 | 5 | 24 | no |
| 14 | 14 | 0 | **yes** |
| 10 | 14 | 0 | **yes** |
| 30 | 3 | 26 | no |

So the defect needs **two** conditions, and both are ordinary: enough accumulated
records, and a terminal short enough that the band is under that count. A
maximized 50-row terminal needs ~49 records; a split-pane or ssh session at 14
rows needs 13. That is why it presents as intermittent — and why the operator
hit it on a launch screen, where the transcript is empty and the band is at its
shortest.

**Why it presents on the launch screen specifically.** On a fresh session the
transcript is empty, so the layout takes the **packed** branch
(`use_packed`, `mod.rs:2941-2942`) and `chunks[0]` is
`content_height.max(1)` — the band is exactly as tall as the content, i.e. about
the header. A band that short is filled by a **handful** of rows, not fifty. So
the launch screen is the *worst case* for this defect, which is precisely where
the operator met it. As the transcript grows the band grows and the same record
count stops overlapping — which is why it looks like a launch-screen-only bug and
is not one.

**This is one defect, not two.** The operator's "swarm preview has bugs" and the
"delegated stale log" are the same rows seen at two different record counts.

**Second-order finding, out of scope here.** The `lively-*` goals in the live
ledger (`lively-fail-0`, `lively-settled-3`, `lively-pruned-4`) are **test
fixtures** from `session_hub/tests.rs::refresh_liveness_tests`
(`unique("follow")`, `unique("fail")`, `unique("narrow")`, `unique("settled")`,
`unique("pruned")`). A test run wrote into the operator's real home. That is a
test-isolation defect, not a UI one, and until it is fixed any screenshot of this
dock is partly a screenshot of test data.

### 1.2 The grey line above the input bar — CONFIRMED, root-caused

> "The input-area bar has an extra grey-line coming on top of it, which would come
> above the persistent last-user-message."

**Measured:** in the style grid at 80x24, row 15 is `rgb:161A21` across all 80
columns and **carries no text**. It is the **status row** (`chunks[2]`), not a
border and not the composer.

**Mechanism.** `draw_status` (`ui_input.rs:914`) builds its line from
`app.status()`. On an idle session that arm is `ProcessingStatus::Idle =>
Line::from("")` (`ui_input.rs:984`) — an **empty** line. The caller then applies
`status_line::bar::fill_style()` to the whole row whenever
`status_row_is_real && area.height > 0` (`ui_input.rs:1271-1277`). An empty line
with a fill painted across it **is** a solid grey bar.

So the bar is not extra chrome — it is the status row rendering as a filled
rectangle because it has nothing to say. The row is reserved unconditionally
(`Constraint::Length(1)`, `operant_ui/mod.rs:2956`) and filled unconditionally.

**Reproduction:** any idle session at any height with credentials configured.
Row index varies with content height; the filled-and-empty row does not.

**Why the operator sees it "above the last-user-message".** With the transcript
bottom-anchored, the status row sits immediately below the last message and
immediately above the composer band — exactly the reported position.

### 1.3 Session title as the TUI header — ALREADY LANDED upstream

`3a7cc279` on `origin/main`. Not in this checkout. **Verify, do not re-implement.**

### 1.4 Rename the session from the picker — ALREADY LANDED upstream

`3a7cc279` (`n` opens an inline editor; Enter writes that row's title through
`update_session_title_preserving_timestamps`; Esc discards; empty refused).
**Verify, do not re-implement.**

### 1.5 `←` opens the subagent panel on an active session — PARTIALLY THERE

> "in active session, `<-` left arrow key must open the subagent panel just like
> `../external-repos/oh-my-pi`."

**Operant already has the opener, and it is the wrong scope.** A **double**-tap on
an empty composer opens the **global session hub**
(`key_handling.rs:2036-2067`, window `[40ms, 500ms)`, with the `armed_close_tap`
handoff). What is missing is the **second surface**: a session-scoped subagent
navigator, plus a **single** `←` in the state where it has no cursor meaning.

**oh-my-pi reference, read directly** (`external-repos/oh-my-pi`):

| Fact | Evidence |
|---|---|
| `←` in an empty composer opens the **Agent Hub** (session-scoped roster), double-tap only, window `[40ms, 500ms)`, 3rd tap = burst and never fires | `input-controller.ts:804-813`, `:1003-1020`; constants `:275-276` |
| The hub's scope is **the current session's subagents**, main agent excluded — "its conversation is the ambient session view" | `agent-hub.md:3`; `#sessionRefs()` filters `ref.id !== MAIN_AGENT_ID` (`agent-hub.ts:1656-1658`) |
| While a subagent is **focused**, double-tap `←` returns to the main session (`unfocusSession`) | `input-controller.ts:849-853`, listener `:376-385` |
| Inside the hub, `←` first closes a narrow detail pane; a **second** `←` within 500 ms closes the hub | `agent-hub.ts:2569-2579` |
| `armCloseTap()` seeds the close detector so the gesture that **opened** the hub also arms its close | `agent-hub.ts:666-677` |
| Refuses to mount on an **empty** roster (`requireContent`) | `selector-controller.ts:2683-2699` |
| `t` toggles flat roster ↔ parent/child tree | `agent-hub.md:36` |

**The real gap, stated plainly.** Operant has **no composer-level focused-agent
concept at all** — `grep` for `focused_agent|unfocus_session|focus_agent` over
`crates/operant-cli/src/tui/` returns nothing outside the hub's own
`focused_subagent` row state (`key_handling.rs:717-719`). So "← returns to the main
session" has nothing to return from. That is a prerequisite, not a keybinding.

**Operant's hazard, which the reference does not have.** The delegation registry is
one process-global map and the ledger is one file per home, so an unscoped
navigator would list **every session's** children. jcode solves exactly this with
`filter_inline_swarm_subtree` (`tui_state.rs:2754-2790`): build a parent→children
index and walk outward from `self_id`, cycle-safe, excluding self — with the written
rationale that other sessions share a repo without this session having spawned them.
Operant must copy the walk, not just the key.

---

## 2. Tranche order

Each tranche is independently landable, compiles on its own, and has its own gate.
Nothing here depends on the peer's in-flight work.

### B0 — Reconcile the base (blocking; not a code change)

1. Confirm with the operator that the 7 local-only commits are the duplicates they
   appear to be (identical titles to published commits).
2. Reconcile local `main` to `origin/main` **without touching the 53 dirty paths** —
   they are a peer's, and they have no stash and no reflog.
3. Re-run the reproduction in §1.1 and §1.2 against the reconciled tree, because a
   count taken on `10357edc` is not evidence about `30b79753`.

**done when.** `git log --oneline -1` reads `30b79753` or later, the peer's dirty
paths are untouched, and both bugs still reproduce.

### B1 — The dock stops eating the screen (closes §1.1)

**Shape the operator chose:** suppress when stale, cap the recent rows, expand into
a `+N more` row, and make the band **collapsible/expandable** — the same idiom the
todo card already uses.

**Change, in `background_tasks.rs`:**

1. **Suppress stale.** A terminal row older than a threshold stops rendering. The
   threshold is a constant with a stated reason, next to `GOAL_PREVIEW` — not a
   config key, because there is no operator question here, only a hygiene rule.
2. **Cap the recent.** Render at most N rows (N small — 3 is the number the todo
   card and the info-widget dock already use) and fold the rest into one
   `+N more` row, exactly as `render_background_widget` does with
   `frame::more(hidden)` (`info_widget_swarm_background.rs:65`).
3. **Collapse/expand.** A toggle on the dock band expands it to the full recent
   list and contracts it back. Reuse the todo card's collapse state rather than
   adding a second collapse mechanism — one idiom, one key, one test shape.
4. **Never overlap the chrome — and this is the one that actually closes the
   bug.** Cap the dock by what is **free**, not by the container height.

   **Two wrong turns, both measured, both recorded so they are not re-tried.**

   *Wrong turn 1 — key the reservation on where the header currently is.* The
   header is a transcript section (`PreparedSectionKind::Header`,
   `ui_prepare.rs:1095`), so it **scrolls away**: at `scroll > 0` the top of the
   band is ordinary content and there is no header there to avoid. A rule keyed on
   the header's *position* makes the dock's correctness depend on a scroll value
   that changes every frame.

   *Wrong turn 2 — a plain cap with a constant floor.* I tested the obvious fix
   before writing it down, and it fails on the case that matters: with a budget of
   N+1 rows the dock still reaches the top of the band whenever the band is
   shorter than N+2. On the launch screen the band **is** about the header, so a
   3+1 budget at band 7-10 still covers the header (measured: band=10, budget 3+1
   → top row 5, header ends at 6). Capping is necessary and **not sufficient**.

   The rule that does hold, verified by enumeration:

   ```
   usable = band_height - header_height - BOTTOM_INSET
   if usable == 0: render nothing
   dock_height = min(min(recent_rows, CAP) + 1, usable)   // the +1 is the "+N more" row
   ```

   The floor is the **header's own height**, not a constant and not a
   scroll-derived free region. The distinction is the whole point:

   - The header's height **is** a frame property, not a viewport property. It is
     the wrapped-line count of the `PreparedSectionKind::Header` section
     (`prepared.rs:147-152`, `sections`), published on the frame the dock can
     already read (`operant_ui::last_chat_frame()`, `mod.rs:1532`). It does not
     change with scroll.
   - What changes with scroll is the header's **position**. I first rejected this
     approach for that reason, which conflated the two: reserving the header's
     *height* is correct at every scroll position, because at `scroll > 0` the
     header is simply off-screen and reserving its height costs a few blank rows
     rather than overlapping anything.

   Enumerated and confirmed: band 7 with a 7-row header renders nothing; band 10
   renders a 2-row dock at top row 7 (no overlap); band 30 renders 4 rows at row
   25; zero rows renders nothing. **A constant floor does not work** — band 7 and
   band 30 have the same header and need different answers.

   **Collapsed transcript still renders nothing.** When the transcript is
   collapsed (`terminal_clear_collapsed` or a full-screen side panel,
   `mod.rs:3080-3096`) the band's geometry is not authoritative; the dock renders
   nothing there.

**Guard.** A test that seeds 14 records and asserts (a) at most CAP rows render,
(b) a `+N more` row appears, (c) at a band height equal to the header the dock
renders **nothing** (the launch-screen case — this is the assertion that fails
against a cap-only fix), (d) at a band of header+4 the dock renders at most 3 rows
and none above the header, and (e) the collapse toggle changes the rendered row
count and nothing else. Each assertion must fail when its own mechanism is removed.

**done when.** The §1.1 reproduction renders a capped dock with a `+N more` row and
a fully legible header at every band height from the header height upward; a
collapsed transcript renders no dock at all; the collapse toggle works.

### B2 — The status row stops painting an empty bar (closes §1.2)

**Change, in `ui_input.rs`:** when the resolved status line is empty, do not apply
the bar fill. The site is `ui_input.rs:1271-1277`:

```rust
let paragraph = Paragraph::new(line);
let paragraph = if status_row_is_real && area.height > 0 {
```

`line` is **moved** into `Paragraph::new` on the line above, so the condition
cannot read it after the move. Bind the emptiness first
(`let line_is_empty = line.width() == 0;` — `Line::width` is ratatui's own,
`ratatui-0.29.0/src/text/line.rs:440`) and key the fill on
`status_row_is_real && area.height > 0 && !line_is_empty`.

Key it on **the line actually being empty**, not on `is_processing`: a processing
session whose status is `Idle` produces the same empty line and the same phantom
bar.

**Do not** remove the row's `Constraint::Length(1)`. The row is the composer's
breathing space and other arms paint into it; the defect is the fill, not the
reservation.

**Guard.** A test that renders an idle session and asserts no row between the last
message and the composer band carries a non-reset background; and a companion test
that a *processing* session still gets its fill. Both must fail when the condition
is inverted.

**done when.** The style grid at 80x24 shows no filled-and-empty row above the
composer, and a processing session still shows its filled status row.

### B3 — Verify the two upstream asks (closes §1.3, §1.4 by verification)

Confirm on the reconciled tree: line 1 is the session label; `n` opens the rename
editor; Enter writes only that row's title; Esc discards; an empty title is
refused; the frame-cache key carries the label (so a rename repaints); and `n` on
an archive confirm still means "no". These are `3a7cc279`'s own guards — re-run
them rather than trusting the commit message.

**done when.** The four rename tests and the two header tests pass on the
reconciled tree, and a manual `simulate` dump shows the title on line 1.

### B4 — The subagent navigator (closes §1.5)

Per the parent plan's T5/T6/T7, which this plan adopts unchanged. The ordered
subset:

1. **T5 first** — a carrier for subagent lineage (parent edge + depth). Without it
   the navigator has nothing to scope on. Operant has the shallow version already:
   `s.id = ?5 OR s.parent_session_id = ?5`, one level
   (`database.rs`, `list_sessions_rich_filtered_page`).
2. **Single `←`** on an empty composer with cursor at 0 — the state where `←` is
   already a no-op. The gate *is* the state, so no config flag is needed (jcode
   gates behind `display.active_sessions_manager`; codex guards on "no overlay, no
   modal, composer empty"). Keep the double-tap as the universal opener.
3. **The navigator surface**, session-scoped, refusing to mount on an empty roster,
   with jcode's subtree walk, `t` for flat/tree, and frozen row order.
4. **Focused-agent return.** Introduce the composer-level focused-agent state and
   bind double-tap `←` to return to the main session (omp
   `input-controller.ts:849-853`). This is the piece with no operant precedent and
   it must land **after** (3), or the gesture has nothing to return from.

**done when.** On a session with one live delegation and one ledger record, `←`
opens a navigator listing exactly those two; a child of a *different* session does
not appear; a session with no children never mounts; and double-tap `←` on a
focused child returns to the main session.

---

## 3. What this plan deliberately does not do

- **No test-isolation fix for the `lively-*` ledger pollution** (§1.1, second-order).
  It is a real defect and it is a separate change; folding it in here would mix a
  test-harness fix into a UI tranche.
- **No change to the delegation ledger's retention policy.** B1 hides stale rows
  from the screen; whether the ledger itself should prune is a data-lifecycle
  question with its own blast radius.
- **No new config keys.** Both fixes are hygiene rules with one right answer, not
  operator preferences.
- **No work on the peer's 53 dirty paths.**

## 4. Risks

| Risk | Mitigation |
|---|---|
| The peer's in-flight work collides with B1/B2 (both touch TUI files) | B0 reconciles first; re-read each target file immediately before editing it |
| B1's cap hides a row the operator needed | The `+N more` row names the count, and the expand toggle reaches every recent row |
| B2's condition is keyed on a proxy and misses the `is_processing && Idle` shape | The guard is written against the *line being empty*, and the companion test pins the processing case |
| B4 lands a navigator with no focused-agent state to return from | Ordered 4 after 3; the return gesture is its own step with its own guard |
---

## 5. Execution record (2026-11-15)

Executed in a temporary worktree at `origin/main`, not in the shared tree: 21 of
the 45 files in the incoming range were dirty with a peer's uncommitted work, so
rule 4's path-disjointness check FAILED and the ff-merge would have been refused.
The peer's tree was left untouched; a `fleet-hold-launch-defects-wip` branch was
created at the stale local HEAD first (rule 2), and the 7 local-only commits were
confirmed as duplicate titles of published commits before anything moved.

### B0 — base reconciled

Local `main` (`10357edc`) was a stale duplicate; all 7 local-only commits mapped
to published commits on `origin/main`. Work proceeded at `origin/main`, advancing
as peers pushed (01b26ae7 → d256c7d2 → 40853d7c).

**Two AGENTS traps hit and worked around, both as documented:**
- **Rule 6 trap 2 (espeak path length).** A worktree at 73 chars failed exactly
  as recorded — `Failed to open: .../phsource/vwl_en_us_nyc/a_rai` then
  `Compiled phonemes`. **The threshold is tighter than the doc's 76:** 68 chars
  builds, 73 does not. Use the shortest path the workspace allows.
- **Rule 6 trap 1 (missing `local/`).** Symlinked `local/` in; `dev-env.sh` itself
  cannot run under this sandbox (no user bus — "Failed to connect to user scope bus
  via local transport"), so the env vars were exported directly and `/usr/bin/cargo`
  called past the `~/.local/bin/cargo` systemd-run wrapper.

### B1 — the dock (iter-2409)

Shipped as specified, with **one correction to the plan's own design**.

- **Suppression, cap and `+N more`:** as planned (`STALE_AFTER_SECS = 600`,
  `DOCK_MAX_ROWS = 3`, matching the todos widget).
- **The expand/contract toggle was dropped, deliberately.** The plan said to copy
  the todos idiom. Investigation found the expand half of that idiom is
  **unreachable in this build**: `App::side_panel()` returns a permanently empty
  `EMPTY_SIDE_PANEL` (`tui_state_impl.rs:1043`), `focused_page_id` has no
  production writer, and `render_todos_expanded` is therefore dead outside tests.
  Copying it would have shipped a toggle that can never fire. The dock follows the
  **cap + `+N more`** half, which works.
- **The reservation rule was corrected during implementation.** The plan's final
  rule was right, but two earlier drafts (reject-header-position, then cap-with-
  constant-floor) were each disproved by measurement before the code was written:
  a 3+1 budget at band 10 still lands on row 5 with the header ending at row 6.
  `rows_area` takes the header's height; a band that cannot spare room renders
  nothing. Header height came from a new `PreparedChatFrame::header_height()`
  (the wrapped line count of the `Header` section) — a frame property, so it does
  not move with scroll.
- **A pre-existing bug was corrected while there.** `BOTTOM_INSET`'s doc says the
  rows never sit flush against the separator, but the old clamp let the rect take
  the full band height, putting the "inset" outside the container. Two assertions
  that pinned that behaviour now assert the documented contract.

**Measured, not inferred:** the live ledger held **50 records, all terminal**. The
dock claimed the entire band at every height tested (7, 9, 12, 20, 30, 50) — not
just on the launch screen.

### B2 — the phantom grey bar (iter-2403)

Shipped as planned. The fill decision was extracted into a pure
`status_row_fills(status_row_is_real, height, line_width)` because the arm cannot
be driven from a test where it sits: `draw_status` reads the process-global
build-progress file and the configured tip, so a render-level guard hung past 60s
under the read-only home. Mutation-proven.

### B3 — verified, not re-implemented

`3a7cc279` is on `origin/main` and its tests pass: `persistent_header_first_line_
carries_the_session_title_when_renamed` and the rename set (10 passed; the one
failure is the sandbox blocking a `/var/tmp` database path).

### B4 — the navigator opener (iter-2411)

**The plan's T7 was largely overtaken by peers while this pass ran.** The navigator
itself landed as iter-2399 and its grandchild walk was fixed in iter-2407. The one
thing still missing was the plan's T6/T7 opener — the exact thing the operator
asked for — and that shipped: `←` on an active session opens the navigator, with
the fresh-session `←` to the hub untouched, and a refused empty roster falling
through.

### Verification standard applied

Every guard was **mutation-proven**: the fix was removed, the test was run, and it
had to fail on the defect before the fix was restored.

| Guard | Mutation | Result |
|---|---|---|
| `an_empty_status_line_is_not_filled` | drop `line_width > 0` | FAILED as required |
| `the_dock_never_paints_over_the_header` | `usable = area.height` | FAILED as required |
| `a_short_band_renders_no_dock_rather_than_a_stale_log` | cap-only | FAILED as required |
| `stale_rows_are_suppressed_and_live_ones_never_are` | drop staleness filter | FAILED as required |
| `the_recent_rows_are_capped_and_the_rest_named` | drop the cap | FAILED as required |
| `one_left_on_an_active_session_opens_the_subagent_navigator` | short-circuit arm | FAILED as required |

**Zero regressions, proven by set comparison.** The full suite has 241 failures
both with these changes and on the untouched base at `01b26ae7`, and the failing
*test-name sets are identical in both directions* (`comm` empty each way). All 241
are sandbox `ReadOnlyFilesystem`/`tempdir` errors from a read-only `$HOME` and
`/var/tmp` — an environmental limit of this run, not a property of the code. The
pass count rose from 2449 to 2459 with the new tests.

`cargo clippy -p operant-cli --bin operant --all-targets`: 0 errors. `cargo fmt`:
only the files touched here (the tree is unformatted upstream, so a bare
`cargo fmt --all` churns ~14 unrelated files — those were reverted, per the
"attribute the working tree before you run anything" rule).

### What this execution did NOT do

- **The test-isolation defect** (§1.1, second-order): `session_hub/tests.rs`
  writes its `lively-*` fixtures into the operator's real ledger. Real, out of
  scope, and the reason any screenshot of that dock is partly test data.
- **The ledger's own retention policy.** B1 hides stale rows from the screen; it
  does not prune the ledger. (A peer's iter-2406 separately fixed a real bug in
  `merge_records`, which truncated running delegations.)

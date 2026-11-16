# RT-2 red-team: TUI red-team (Z23)

**Status (iter-2495):** CLOSED. All eight Z23 rows resolved.

- **Z23-B (HIGH, mouse click resolves to the WRONG message)** -- FIXED in iter-2495.
  The resolver read a `display_messages()` row index as an `App::messages` index.
  The two index spaces differ: tool rows, reasoning rows, settled tool cards and
  annotations each take a display row without being an app message, and one app
  message can expand to several rows. Fix: `display_messages_with_app_index()`
  returns each row's app index from the same walk, cached beside the rows it
  describes; `TuiState::display_message_app_index` exposes the map; the builders
  use the mapped call so the map cannot be skipped by accident. The gap was
  widest for a tool message and its rows, which is what the fleet's change made
  common.
- **Z23-E (LOW)** -- dead `App::thinking_expanded` field REMOVED (iter-2495).
- **Z23-A/C/D/F/G/H** -- CLOSED as recorded: A's regression test proves the Z11
  reload survives stream resumption; C is asserted by measurement (offset held
  at 0 after reanchor and after a `+2` adjust); D is registered and
  per-bullet bounded by the scope gate, with the load itself reported as
  blocked-by-design rather than as a sweep; F is guarded by an explicit panic
  before `resolve_content_pos` truncates; G and H need no code change (Z23-G
  says RETAIN, not remove; Z23-H's failure is an existing blocking gate).

> **STATUS (updated 2026-11-16, commit `d7ecc58f` / iter-2492).** `RT-Z23-B` is
> **CLOSED**. `message_index_at_screen_row` now returns an `App::messages` index,
> bridged by a `display_messages_with_app_index` map built in the SAME pass that
> emits the display rows, cached beside them, and published per message boundary
> in `PreparedMessages::message_app_index`. The regression test uses the ordinary
> agent shape (user, assistant, tool row, assistant) and is mutation-proven:
> restoring the pre-fix resolver fails it with
> `got=Some(Message { message_index: 3 })`, exactly the wrong message measured
> below. `RT-Z23-E` is **CLOSED**: `App::thinking_expanded` and its initialiser
> are deleted. `RT-Z23-F`'s two existing tests still pass unchanged.
> `RT-Z22-F` (an observation, no action) is unchanged.


**Audited:** 2026-11-16
**Pinned revision:** `ae9a27cf` — `fix(iter-2490): six-lane remediation of the 2026-11-16 red-team (Z15-Z31)`
**Lane:** RT-2 (`Z22`, `Z23` — the TUI half)
**Source findings:** `docs/audit/2026-11-16-operant-architecture-red-team/H-tui.md` (`H5`), `Z-coordinator-verification.md` (`Z22`, `Z23`)
**Method:** read the diff, re-derive the mechanism each fix is supposed to change, then attack the
neighbouring cases the fix's own test never asks about. Every claim below is backed by a run command;
the probes were built in a throwaway worktree pinned at `ae9a27cf` and are NOT checked in.
`cargo test -p operant-cli --lib` reports 0 tests (the suite is the **bin** target) — everything
here was run as `cargo test -p operant-cli --bin operant -- <filter>`.

## Verdict summary

| finding | verdict | note |
|---|---|---|
| **Z22** `/model roles` opened an unpainted panel that owned the keyboard and could write config | **CONFIRMED FIXED** | panel deleted, opener repointed at a painted surface, catalogue/parity guards hold |
| **Z23** two dead hit-test maps | **PARTIALLY FIXED** | the deletion and re-resolution are correct, but the new resolver returns an index in the WRONG index space, so the menu it opens names the wrong message |

**Z22 is a clean closure.** Nothing of the old panel survives outside comments; `/model roles`
reaches a surface that is on `overlay_flags()`, in the `OVERLAYS` paint table, and provably paints
its rows; the regression test fails when the fix is mutated back.

**Z23 closed the dead-code half and opened a correctness hole in the live half.** The hit test now
resolves against the painted frame, which is the structurally right move, but it returns an index
into `display_messages()` and both callers feed it straight to a consumer that reads `self.messages`.
Those two vectors differ by exactly one row per tool call / system annotation / background card
between the target and the end of the transcript, so the operator right-clicks a message and `Copy`
yields a different message — or nothing at all. Measured, not inferred. This is the same defect the
peer contribution records as `RT-Z23-A`; this lane re-measured it against a realistic transcript
shape, established that the **ordinary agent transcript** (a tool call in the middle) triggers it
rather than only the annotation case, and found the same index-space confusion already present in
the selection chrome filter, which the peer note does not cover.

---

## Z22 — the invisible panel

### Z22-a. Does the fix close the original finding? **Yes.**

The original defect had four parts. All four are gone.

1. **The panel is deleted.** `git show ae9a27cf --stat` shows `crates/operant-cli/src/tui/models_panel/mod.rs | 391 ----------`.
   The directory does not exist: `ls crates/operant-cli/src/tui/models_panel` → `No such file or directory`.
2. **Every reference is gone, not just commented out.** `grep -rn "models_panel\|ModelsPanel\|handle_models_panel_key"` across
   `*.rs`/`*.md`/`*.toml` returns hits in exactly three places: four *comments* that record the deletion
   (`tui/mod.rs:60`, `app/init.rs:246`, `app/key_handling.rs:1243`, `app/mod.rs:696`, `app/commands.rs:770`),
   one *test doc comment* (`app/tests.rs:4644`), and the **pre-existing audit documents** that describe the
   old defect. There is no declaration, no field, no initialiser, no router, no `open()`, no `close()`,
   no `move_selection()`, no `rows()`, no `clear_slot()`.
3. **`/model roles` is reachable and paints.** `app/commands.rs:775-778`:
   `open_model_picker_inline("")` then `apply_model_scope(ModelScope::Roles)` then `return true`.
   `open_model_picker_inline` (`app/providers.rs:212`) builds the picker and assigns
   `self.inline_interactive = Some(picker)`; the picker prepends `slot_role_rows()` (`providers.rs:258-260`,
   body at `:621`), so the Roles scope has real rows: `ModelScope::Roles` selects
   `matches!(entry.action, PickerAction::ModelSlot(_))` (`model_scope.rs:73`), and `slot_role_rows`
   emits one `PickerAction::ModelSlot` per non-conversation slot (`providers.rs:627-649`).
   The renderer half exists and is driven from the `OVERLAYS` table:
   `("inline_interactive", ...)` is the `overlay_flags()` row (`app/commands.rs:114`), and the
   painted surface for the picker is `paint_command_palette` (`render/operant_overlays.rs:536`),
   which the `command_palette` `OVERLAYS` row reaches. (The picker and the palette share one paint
   entry — `paint_command_palette` paints the palette, then `subagent_navigator`, then
   `agent_picker` — so the `inline_interactive` flag has no row of its own. That is deliberate: one
   surface, one flag, one painter. It is asserted, not inferred, by the test below, which reads the
   real frame rather than the table.)
   Measured: `/model roles` with a fallback model assigned leaves `picker.filtered` non-empty with at
   least one `ModelSlot` row, and `screen_rows()` contains every role row's name.
4. **`any_modal_open()` can no longer report false while the surface owns the keyboard.** It is derived
   from `overlay_flags()` (`app/commands.rs:2396-2405`), whose 29-name array now contains
   `inline_interactive` and no longer contains anything panel-shaped.

### Z22-b. Systematic sweep: is there another "opener but no renderer" panel?

The charter's real question was not "is `models_panel` gone" but "was it the only one". Two lists have
to be compared, and both now agree:

- **`overlay_flags()`** — 29 rows, `app/commands.rs:93-135`. Every one maps to a field with a `.visible`
  or an `is_some()`.
- **`OVERLAYS`** — the paint table, `render/operant_overlays.rs:213-...`, 31 rows, with a
  hand-written `EXPECTED` mirror at `:667-697` and two structural guards
  (`dispatch_rows_are_the_registered_surfaces_in_paint_order` at `:711`, `dispatch_declares_each_surface_exactly_once`
  at `:742`).

Result of checking each `overlay_flags()` name against the paint table: **every one is present as an
`OVERLAYS` row with a real paint fn**. The two `overlay_flags()` names that are not table rows are
`permission_request` (a payload, painted by the `permission_dialog` row through
`visible_permission_dialog`) and `inline_interactive` (painted by the `command_palette` row's
`paint_command_palette`, which also paints `subagent_navigator` and `agent_picker`,
`operant_overlays.rs:536-547`). `usage_overlay` is the reverse asymmetry: it is an `OVERLAYS` row but
not an `overlay_flags()` row — deliberate and correct, because it is a passive metrics panel with no
input capture; it is toggled by one key handler (`key_handling.rs:2013`).

**No second invisible panel exists.** The `subagent_navigator`/`agent_picker` shape the charter pointed
at as the "done it right" precedent really is done right: each has a paint fn, a key router
(`key_handling.rs:590`, `:604`), and an opener (`commands.rs:1435` → `open_subagent_navigator`).

The `.visible = true` sweep agrees. 31 production writers, each in a surface's own `open()`; every
receiver is either in `overlay_flags()` (and therefore in the paint table) or is one of the two
non-catalogue exceptions above.

### Z22-c. Is `persist_config_key` safe from an invisible panel now?

**Yes, in the specific sense the finding named, with one honest caveat.**

The unreachable write is gone: the only caller that could fire with nothing on screen was
`handle_models_panel_key`'s `x` arm (`persist_config_key(key_path, "")`), deleted with the panel.
The remaining `persist_config_key` calls in the TUI all sit behind a *painted* surface:

- `app/providers.rs:186, 420, 463, 748, 756` — the inline picker (painted) and its favourite/default
  toggles (rows in a painted picker).
- `app/key_handling.rs:3639` — the picker's own slot-assignment Enter, which additionally gates on
  `slot_role_rows()` and a capability check before writing.
- `settings_screen/state.rs:183, 255` — the settings screen, which is in `overlay_flags()` and the paint table.

So the *invisible-panel-writes-config* class is closed. **The caveat, stated plainly:** the fix does not
make `persist_config_key` itself refuse a caller — it is still a free function any future code can call.
What closed the finding is that the one surface which could reach it while painting nothing no longer
exists, and the catalogue/paint-table parity guards are what keep the next panel from re-creating the
shape. That is a structural fix, not a type-level one; if a future lane wants the stronger guarantee it
would have to make the write require a visible-surface token. Not a defect in this remediation.

### Z22-d. Did the fix leave dead keybinding docs or help text?

No. `Ctrl+Shift+M` is still a live binding (`key_handling.rs:1966-1971`) and still does something
observable (see Z23). No keybinding-registry row or `/help` entry named the deleted panel:
`grep "context menu\|right-click" overlays/help.rs` and `keybindings/defaults.rs` both return nothing
for it. The only stale mentions of the old maps are two comments that cite them as a *past* mistake
(`app/tests.rs:4730`, `:15183`) — accurate as history, not as documentation of live behaviour.

### Z22-e. Mutation test

**Fails when mutated.** Two mutations, run in a scratch worktree at `ae9a27cf`:

| mutation | result |
|---|---|
| `/model roles` arm → `open_model_picker_inline("")` only, dropping `apply_model_scope(Roles)` | `model_roles_opens_the_rendered_picker_on_its_roles_scope` **FAILED** at `tests.rs:4679` ("and on its Roles scope") |
| delete the whole `roles` arm (no picker opened at all) | same test fails at its first assertion |

The test asserts on state *and* on the painted frame (`screen_rows` must contain every role row's
name), so a repoint at a state-only surface cannot pass.

---

## Z23 — the dead hit-test maps

### Z23-a. The deletion and re-resolution are correct

`thinking_row_map` and `message_row_map` are gone from the struct (`app/mod.rs:995-1004` is the
replacement comment), from the initialiser (`app/init.rs:416`), and from their readers
(`app/mouse.rs:118-120` `message_index_at_row` deleted; the two lookup sites now call
`operant_ui::message_index_at_screen_row`, `mouse.rs:554` and `:1274`). `grep -rn "thinking_row_map\|message_row_map"`
returns only comments and audit documents. `message_index_at_screen_row`
(`operant_ui/ui_viewport.rs:1775-1782`) resolves through one shared subtraction
(`wrapped_line_at_screen_row`, `:1747`) and then through
`PreparedChatFrame::message_index_at_line` (`operant_model/prepared.rs:424-435`), which is the same
frame the paint used. Pointer and paint are on one geometry. The `wrapped_line_at_screen_row`
negative case is right: a row above `content_top` returns `None` rather than clamping to line 0, and
`a_row_above_the_transcript_is_not_a_message` (`tests.rs:4888`) pins it.

**Geometry probes (all measured, 120x40 TestBackend, real `render_app`):**

| case | result |
|---|---|
| fresh frame, tail of a 60-message transcript | correct — click `SCROLL-059` → `messages[59]`, `SCROLL-055` → `[55]`, `SCROLL-050` → `[50]` |
| PageUp ×3 then click first/middle/last visible row | correct at every step (`resolved_scroll` 72 → 39 → 6 → 0, all hits exact) |
| resize 120x40 → 60x20 between frames | `content_area` and `resolved_scroll` both update; a stale-geometry hit afterwards returns `None`, not a wrong index |
| empty transcript | `content_area` published, every row returns `None` |
| multi-line wrapped message | one message spanning several rows still names itself on each of its rows |

So the scrolled-viewport question the charter asked ("verify the new function against `render` for the
case of a scrolled-up viewport") comes back clean. The frame/scroll pairing is right.

### Z23-b. The new resolver returns an index in the wrong index space — **NEW GAP, HIGH**

**The resolver's return value is documented correctly and used incorrectly.**
`message_index_at_screen_row` says so itself (`ui_viewport.rs:1769-1774`):

> The returned index is into `App::display_messages()`, which is the vector the Body section was built
> from ... A caller that wants an `App::messages` index has to map across, and the two are NOT
> interchangeable.

Neither caller maps across.

- `mouse.rs:1274-1284` (right-click) takes the value and stores it as
  `ContextMenuKind::Message { message_index }` after only a bounds check
  `message_index < self.messages.len()`.
- `mouse.rs:554-565` (`Ctrl+Shift+M`) does the same with the same check at `:556`.

The consumer is `handle_context_menu_action` (`mouse.rs:590-595`), which reads
`self.messages.get(message_index)`. `self.messages` holds only `User`/`Assistant`/`System`;
`display_messages()` (`operant_model/adapter.rs:73-185`) additionally splices in tool rows
(`:100`, `:172-174`), system annotations (`:136-138`, `:175-177`), background cards and collapsed tool
runs (`:108-123`). Every such row between the target and the head of the transcript shifts the index by one.

**Measured.** Right-clicks driven through the real `handle_mouse_event`, resolved against the published
content rect, reporting what `Copy` would read:

| transcript shape | click on | Copy returns | verdict |
|---|---|---|---|
| 3 plain messages (the fix's own fixture shape) | `AAA-A` | `messages[1]` = "AAA-A one" | CORRECT |
| same, one settled tool row after message 2 (the **ordinary** agent transcript) | `BBB-B` | `messages[3]` = **"BBB-C three"** | **WRONG MESSAGE** |
| same | `BBB-C` (last) | menu dismissed — index 4 ≥ `messages.len()` = 4 | **NO MENU** |
| 2 messages + 6 tool rows + a trailing message | `CCC-Z` (last) | menu dismissed — display index 8 vs 3 messages | **NO MENU** |

`display_messages().len()` = 5 vs `messages.len()` = 4 in the tool-row case; 9 vs 3 in the many-rows case.
The divergence is one row per intervening tool row / annotation, so it grows without bound as the
session gets longer — by the time a session has twenty tool calls, every right-click below the first of
them either names a message twenty positions off or produces no menu at all.

**Why the fix's own test passes.** `right_click_over_a_transcript_row_opens_a_message_context_menu`
(`tests.rs:4738`) builds `add_message` ×2 and nothing else — no tool rows, no annotations — so
`display_messages().len() == messages.len()` and the two index spaces coincide. The test never
exercises the case where they diverge, which is every real transcript.

**The `Ctrl+Shift+M` walk-down makes this worse, not better.** `mouse.rs:545-560` probes downward from
the top visible row "to the first row that does name a message". Under an index-space skew, the first
row that resolves is very likely to be a *tool row* (which resolves fine — it is a display row), so the
chord now opens a menu naming whatever message the tool row's display index happens to land on, rather
than the message under the reader. It also re-introduces a smaller version of the original bug's shape:
a chord that silently targets something the operator did not point at.

**Severity: HIGH.** Not "MED" as the peer note has it. The remediation's own severity doctrine
(this repo's RT docs) calls a case HIGH when "the operator is misled in a way that changes an
outcome". `Copy` is the only item on the message menu (`mouse.rs:113`), so the outcome is: the operator
right-clicks a message, picks Copy, and the clipboard receives a *different message's* text with no
error and no notice. Silent wrong-data is worse than the silent no-op the peer describes, and the
no-op is only the last-row special case. The two behaviours are the same root cause and the same fix.

**What to delete, not rename:** nothing needs deleting here — the fix's structure is right. What is
missing is the mapping. Add a `Vec<usize>` of display-index → app-message-index (or
`Option<usize>` for display rows that are not app messages) built once per frame next to
`message_boundaries` in `prepare_body` (`operant_ui/ui_prepare.rs:2097` is the loop that has both the
display index and the app index in scope), publish it the way `tool_row_regions` is published
(`ui_viewport.rs:445`), and have both callers resolve through it. Then the `message_index <
self.messages.len()` guard becomes a real bounds check rather than the thing standing in for the
mapping. Alternatively change `ContextMenuKind::Message` to carry the `DisplayMessage` itself and drop
the index entirely — larger, but it deletes the possibility instead of papering over it.

### Z23-c. The same index-space confusion is already in the selection filter

`render/selection.rs:78-88` (`is_ui_row`) does:

```rust
frame.wrapped_line_map(line).is_none()
    || frame.message_index_at_line(line)
        .and_then(|i| messages.get(i))     // messages = app.display_messages()
        .is_some_and(|m| UI_ROLES.contains(&m.role.as_str()))
```

This one is **correct** — `messages` here *is* `app.display_messages()` (`selection.rs:101`), so the
index space matches. It is worth recording because it is the in-tree precedent for the right shape:
the selection path already resolves frame lines against `display_messages()` and never against
`App::messages`. The Z23 handlers are the only place that mixes the two. Citing this in the fix
would make the intended invariant obvious to the next reader.

### Z23-d. The `Ctrl+Shift+M` fallback still exists, and is now reachable

`open_context_menu_at_cursor` (`mouse.rs:528-581`) ends with an unconditional
`show_context_menu(..., ContextMenuKind::Selection)` when nothing resolves. That branch is correct
behaviour (a selection-only menu is honest when there is no message), and the remediation's own
`ctrl_shift_m_over_the_transcript_opens_a_message_context_menu` (`tests.rs:4833`) proves the message
branch is now reachable. But note the interaction with Z23-b: because the message branch resolves a
*wrong* index rather than failing, the fallback is now less likely to fire than it should be. Fixing
Z23-b restores the fallback to its honest role.

### Z23-e. `thinking_expanded` is now a field with no reader — leftover state

The remediation removed the field's only writer (`mouse.rs:1372-1390` comment) and correctly did NOT
invent a replacement collapse state. But the field itself survives: declared at `app/mod.rs:986`
("Set of thinking block content hashes that are expanded"), initialised at `app/init.rs:411`, and read
by **nothing**. `grep -rn "thinking_expanded"` returns exactly those two plus the two comments.

The commit message says "DELETED, with the declaration, the field, its initialiser" about
`models_panel`, and for Z23 it says the maps and "their dead handlers" were deleted. The maps went;
the *state the dead handler was the only writer of* stayed. Its own doc comment on `app/mod.rs:1000-1002`
concedes it: "`thinking_expanded` could never be non-empty, since its only writer was that dead branch".
A field that is provably always empty and provably never read is the same class of defect the finding
was about, one layer down.

**Say what to DELETE:** `App::thinking_expanded` (`app/mod.rs:986`) and its initialiser
(`app/init.rs:411`). Nothing reads it; the collapse feature it was for is now honestly provided by
`ReasoningDisplayMode::Current` + `/thinking-display`, and the comment at `mouse.rs:1382-1390` already
says so. Deleting it removes the last artefact that would let a future agent believe a
click-to-collapse path exists.

### Z23-f. Mutation test

**Fails when mutated.** Replacing the body of `message_index_at_screen_row` with the pre-fix
behaviour (`let _ = (screen_row, content_top, scroll); None`, i.e. the map that was never written):

| test | result |
|---|---|
| `right_click_over_a_transcript_row_opens_a_message_context_menu` | **FAILED** at `tests.rs:4792` — "must open the MESSAGE menu for index 0, got None" |
| `ctrl_shift_m_over_the_transcript_opens_a_message_context_menu` | **FAILED** at `tests.rs:4869` |

Both are mutation-shaped, so the deletion half of Z23 is genuinely pinned. What is *not* pinned is
correctness of the returned index — no test asserts that the menu names the message the operator
pointed at in a transcript where the two index spaces diverge. That is the shape the fix needs next.

---

## What I ran

All in a throwaway worktree pinned at `ae9a27cf`, never in the shared tree (a peer was writing
`tui/app/tests.rs` there — see "Note on the working tree"). Cleaned up afterwards.

```
cargo test -p operant-cli --bin operant -- tui::app::tests::model_roles_opens_the_rendered_picker_on_its_roles_scope   # ok
                                                                          ...::right_click_over_a_transcript_row_...         # ok
                                                                          ...::ctrl_shift_m_over_the_transcript_...          # ok
                                                                          ...::a_row_above_the_transcript_is_not_a_message   # ok
cargo test -p operant-cli --bin operant -- tui::app::tests                     # 463 passed, 4 failed
cargo test -p operant-cli --bin operant -- <mutation>                          # the 3 failures quoted above
```

The 4 failures in the full app suite (`a_delegation_with_no_assigned_todos_has_no_counter`,
`a_settled_delegation_leaves_the_strip_but_keeps_its_card`, `test_dialog_open_close_scenarios`,
`the_help_overlay_descriptions_match_what_the_arms_do`) are the **same four** `ae9a27cf`'s own commit
body reports as pre-existing, with the same 463/4 split. This lane's changes do not touch them and I
did not investigate them (out of scope, and the commit body's claim was not re-verified here — marked
**ASSERTED**).

## Note on the working tree (for whoever reads this next)

Twice while this audit was running, the shared tree was reset or checked out from under me:
`HEAD` went `ae9a27cf` → `c00f1962` (pre-remediation) mid-session, then back to `e8e8f021`. The
second move discarded an uncommitted peer probe in `tui/app/tests.rs` that referenced
`DisplayMessage::Message(...)` and `adapter_types::SystemMessageStyle` — both paths that do not exist
in this tree, which is why `cargo test -p operant-cli --bin operant` briefly failed to compile with
3 errors. That is a **peer's in-flight WIP, not a defect in `ae9a27cf`**; the pinned revision compiles
its test target clean. Flagging it because the next agent will otherwise read a red test build and
blame the remediation. All citations above are to blobs at `ae9a27cf`/`e8e8f021` and are stable
regardless of where `HEAD` moves.

## Findings, by severity

**HIGH — `RT-Z23-B` (new, this lane): the message context menu names the wrong message.**
`message_index_at_screen_row` returns a `display_messages()` index; both callers pass it to a consumer
that reads `self.messages`. Measured: right-clicking the third message of a four-message transcript
with one tool row copies the *fourth* message's text; right-clicking the last message of a transcript
with six tool rows produces no menu at all. Grows with session length. Fix: publish a
display→app index map per frame (or carry the `DisplayMessage` in `ContextMenuKind`). The fix's own
test passes because its fixture has no tool rows or annotations. Overlaps the peer's `RT-Z23-A`; this
lane's contribution is the measurement on the ordinary tool-call shape, the HIGH rating with the
silent-wrong-data reasoning, and the `is_ui_row` precedent at `render/selection.rs:78-88` showing the
correct resolution style already in-tree.

**LOW — `RT-Z23-E` (new, this lane): `App::thinking_expanded` is dead state with a live doc comment.**
Field at `app/mod.rs:986`, initialiser at `app/init.rs:411`, zero readers, zero writers. Delete both.
The commit's own comment concedes the field can never be non-empty.

**LOW — `RT-Z22-F` (observation, no action): the catalogue/paint-table parity is guarded by tests, not types.**
`overlay_flags()` (29) and `OVERLAYS` (31) are kept in agreement by
`dispatch_rows_are_the_registered_surfaces_in_paint_order` and `dialog_covers_every_gated_surface`,
which is the right mechanism and does hold today (verified: every catalogue name has a paint row).
The two intentional asymmetries (`usage_overlay` paint-only; `permission_request`/`inline_interactive`
as payload/multi-surface rows) are correct but are not written down anywhere a new agent would read
before "fixing" the mismatch. Worth a comment on the `OVERLAYS` table, not a code change.

**Not a finding, recorded so it is not re-litigated:** the malformed double header at
`operant_overlays.rs:213-214` ("base chrome" immediately followed by "the overlay block"). Cosmetic,
pre-existing, and the `EXPECTED` mirror makes it harmless. Mention only because it reads as a
half-finished edit at exactly the spot a reviewer of Z22 will look.
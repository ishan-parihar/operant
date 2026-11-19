# Close the TUI audit round's verified findings

Date: 2026-11-20
Status: ready-for-execution
Source: `research/audits/context-pack-2026-11-19-tui/` — a full context-pack round
(AI Studio, `gemini-4-hydrogen/high`, 11 findings) with
`VERIFICATION.md` recording each claim's outcome against the tree.

## Claim rule

Never take a number from a header. `git fetch origin`, read the top label from
`origin/main`, add one. This plan owns the TUI-audit cleanup question.

## Relation to `2026-11-19-close-the-measured-residuals-*`

That plan and this one **share one checkbox**: RT-543 / F9 / T10 is the same
stale-checkbox question. Everything else here is disjoint from it — that plan
is doc-drift, dead code, library-doc drift and a clippy gate; this one is
TUI correctness. If both are in flight, land the residuals plan first (its
T10 touches `BUGS.md`, and two agents editing one checkbox table is the
collision).

## What the round actually produced

11 findings. Verified at tip `687ae9b6` (iter-2693):

| id | verdict | evidence |
|---|---|---|
| F4 contrast seam | **REAL** | `adapt_buffer_for_display` at `operant_ui/mod.rs:2550` runs inside `draw`; `draw_operant_overlays` runs at `render/mod.rs:140`, 8 call sites later |
| F5 routing lists | **REAL** | `overlay_flags` = 29 (the only machine-checkable list), `dialog_priority` = 30 (a second list), inline `if self.X.visible` gates in `key_handling.rs` = 34 (a third, and the one nothing can enumerate). `agent_picker` is in `commands.rs` + 15 inline sites, and 0 of `dialog_routing.rs` — the failure mode, not a hypothetical |
| F6 `[keybindings]` refusal | **REAL** | `config.rs:3141-3143` — a single `eprintln!`; survives a GUI launch, `/quit` and a slot handover |
| F8 OUTLINE crate graph | REAL | already owned by the residuals plan T1 |
| F9 excluded.json | **REAL** | 4 entries, 2 name surfaces whose files are gone (`context_menu.rs`, `hooks_config_menu.rs`); the third (`diff-viewer-git`) has no render fn, only `render_diff_dialog` |
| F10 workspace extensions | REAL | already owned by the residuals plan T3 |
| F11 clippy gate | **REAL, corrected** | round claimed 151 bindings / 46 scenario files; MEASURED: **150** `DefaultBinding` across **10** contexts, **68** scenario files. The finding survives on its own numbers |
| F7 renderer fan-out | **NOT REAL** | premise refuted: 7 call sites are `no_run` probes, 3 are test reachability, 1 is the panic-message scan |
| RT-171 "19 settings / 4 dropped" | **STALE** | live screen offers 8, applies all 8 |
| RT-389 `discard` durability | **STALE** | corrected at `687ae9b6` |
| `notify` delivery ack | **NOT A FINDING** | `notify_result.rs:144` returns `delivered: true`; lost at iter-2665, restored before the round began |

Two B-DELTA arithmetic errors were caught by a reproducibility skeptic and
corrected in place: the binding total (151 → 150) and the session_hub ratio
(4.6 → 3.64).

## T1 — Contrast-repair the surfaces `draw` never sees (overlays, background rows, error modal)

**Files:** `crates/operant-cli/src/tui/render/mod.rs`,
`crates/operant-cli/src/tui/operant_ui/mod.rs`.

Today `operant_ui::draw` runs the contrast pass at its own line 2550, then
`render/mod.rs` paints `background_tasks::render_rows` (:132) and
`draw_operant_overlays` (:140) — 8 call sites later. Everything those two
draw is never contrast-repaired on `light` or `deuteranopia`.

The contradiction is sharper than "some rows are unrepaired":
`background_tasks.rs:568` carries a test, `module_has_no_raw_colour_literals`,
whose stated purpose is that no new literal may reappear because "iter-414
migrated 294 named terminal colours to the theme palette" — a gate that
asserts palette *conformance* while the module's cells are the ones the
contrast pass never reaches. The gate passes; the rows still render
unrepaired on a light theme. That is the finding in one sentence.

**Falsifier:** on `light` and `deuteranopia`, paint a background-task row
(during a live tool) and an overlay (`/settings`), and sample the drawn
cells: they carry raw literals while the panels behind them are repaired.

**Fix:** call `adapt_buffer_for_display` + `adapt_buffer_for_emoji_preference`
once, in `render/mod.rs` immediately before the `draw_operant_overlays` call
(:140) — and REMOVE the pair from `operant_ui/mod.rs:2550-2551` so it is not
applied twice (a second pass over already-repaired cells is idempotent for
contrast but not for emoji preference, which must run on the final text).

**Test:** a render test that draws a background-task row and an overlay on
`light`, then asserts a contrast-repaired cell where the panel is repaired.
Extend, do not duplicate, the existing `render::tests` theme cases.

**The call site is the whole risk.** `render/mod.rs` is not a straight line:

- :141 `if draw_operant_overlays(...) { return; }` — the error modal is the
  frame's only early exit, and the four post-passes below it (pinned
  graphics, `osc8::scan_buffer_for_urls`) paint over it. Putting the pass
  BEFORE :141 means the error modal, the single surface an operator most
  needs to read, is the one surface left unrepaired — on a light theme that
  is exactly the failure this tranche exists to fix.
- Putting it AFTER the post-passes means `osc8::scan_buffer_for_urls`
  reads pre-repair URLs (it scans for text, so that is likely fine — but
  verify, because it is also a **buffer-mutating** pass and repairing after
  it would double-process anything it writes).

MEASURED, and it settles the ordering: `pinned_images::prepare_strip`
(`render/mod.rs:154`) calls `blank(buf, rect)` on every registered
attachment rect. A blanked cell has no foreground to repair, so a pass placed
before :140 repairs cells that a pinned image then erases — wasted work, and
worse, it repairs a colour that is never displayed. `osc8::scan_buffer_for_urls`
takes `&Buffer` and only reads (`osc8.rs:86`).

**Therefore:** place the pair at the END of `render_frame`, after the OSC 8
pass, and ALSO accept the early-return consequence: the error modal returns at
:141 before the post-passes, so a pass at the end never runs on the modal
frame. Fix that by moving the `return` — repair, then return — rather than by
putting the pass before :141 where pinned images would undo it. Assert in the
test that the modal's cells are repaired AND that a pinned-image rect is not
repaired before being blanked.

If moving the early return is judged too invasive, the fallback is to call the
pair in BOTH places and make it idempotent by construction (contrast repair
is; emoji preference is not) — do not take that fallback without saying so in
the commit body, because it re-introduces the double-pass the tranche exists
to remove.

**Constraint:** do not place the emoji pass anywhere that a later surface
writes text. Verify by reading `:141` through the end of `render_frame`, not
by grepping for `buf.set_string` alone.

## T2 — One catalogue for every dialog surface

**Files:** `crates/operant-cli/src/tui/app/commands.rs`,
`crates/operant-cli/src/tui/app/dialog_routing.rs`.

`overlay_flags()` (29) is the only machine-checkable list. `dialog_priority()`
(30) and the inline `if self.X.visible` gates in `key_handling.rs` (34) are
sibling lists nobody can compare. `agent_picker` proves the failure mode: it
is in the catalogue and 15 inline sites, and absent from the routing table.

**Fix:** one function returns every surface's `(name, visible, priority, block_on_focus)`;
the three call sites consume it. Then a test asserts
`overlay_flags() ⊆ catalogue` and `catalogue == routing table keys`.

**Constraint:** the inline gates that precede the `dialog_priority` loop stay
inline (they are not in the loop's order), but they must be RECORDED in the
catalogue or the test cannot see them. Recording is the fix; moving them into
the loop is a separate change and not required.

**Do not** attempt a total ordering across all 34 — several surfaces are
deliberately independent (`settings_screen` runs its own chain per the
comment at `key_handling.rs:91-93`). The invariant is *catalogued*, not
*ordered*.

## T3 — One key dialect, and a guard that keeps it

**Files:** `crates/operant-cli/src/tui/app/key_handling.rs` (the 12 move-only
arms), `crates/operant-cli/src/tui/app/dialog_key_tests.rs` (the assertions).
NOT touched: `theme_screen.rs` and `settings_screen/keys.rs` — both are
text-entry and stay literal. If the fix ends up needing a file from the
text-entry set, the fix has grown a text-input gate and belongs in its own
iteration.

MEASURED across the 29 catalogued surfaces: vim `j`/`k` on **7**, `q`-to-close
on **3** (and one of those is conditional — `mcp_view` closes on `q` only from
`ServerList`), and **22 have neither**. For a keyboard-first TUI that is the
largest single inconsistency the round found.

**The gate is the whole task, and the text-entry classification is what makes
it tractable.** A surface that accepts typed characters cannot take a bare
`Char('k')` nav arm: `k` would move the cursor instead of being typed.
`session_hub` shows the only correct shape (`key_handling.rs:804-820`): an
inner-mode gate (`search_active`) that swallows `Char(c)` and returns BEFORE
the nav match, so the two arms can never both see one keypress.

MEASURED, the 29 catalogued surfaces fall out like this:

| class | surfaces | `j`/`k` today |
|---|---|---|
| **text entry, ungated** — `Char` reaches a query/`insert_char`/`push_char` | `settings_screen`, `theme_screen`, `ask_user_dialog` (:239), `key_input_dialog` (:325), `custom_provider_dialog` (:416), `free_mode_dialog` (:528), `mcp_view`, `global_search`, `history_search_overlay` | none — and must stay none |
| **text entry, gated correctly** | `session_hub` (`search_active`) | yes |
| **move-only** | `help_overlay`, `agents_menu`, `diff_viewer`, `memory_file_selector`, `inline_interactive`, `export_dialog`, `context_viz`, `import_config_picker`, `import_config_dialog`, `command_palette`, `permission_request`, `stats_dialog`, `rewind_selector`*, `bypass_permissions_dialog`*, `permissions_picker`* | some (`*` = already have it) |

The move-only list is the actionable set: **12 surfaces** have no `j`/`k` and
no text input to break — `help_overlay`, `agents_menu`, `diff_viewer`,
`memory_file_selector`, `inline_interactive`, `export_dialog`, `context_viz`,
`import_config_picker`, `import_config_dialog`, `command_palette`,
`permission_request`, `stats_dialog`. Count them in the table; if the count
and the sentence ever disagree, the table wins.

**Per-surface verification is not optional.** The `Char(c) =>` and
`push_char` / `insert_char` / `push_search_char` greps above are what decided
this table, and a surface can pass the grep in one module while its key arm
lives in another (that is exactly why `theme_screen` looks move-only from its
own file and is not). Write the grep result per surface into the commit body:
a reviewer who cannot see the classification cannot check the arm.

**Fix:** add `Char('j')`/`Char('k')` to the move arms of the move-only
surfaces only, matching the existing `KeyCode::Up | KeyCode::Char('k')`
shape at `key_handling.rs:848`. Add `q`-to-close to the surfaces whose whole
job is a list, matching the 3 that have it. Leave the text-entry surfaces
alone and say so in the plan output.

**Test:** extend `app/dialog_key_tests.rs` (27 tests today) with per-surface
assertions. Then add the parity guard from T4 — without it, the bindings
screen and the dispatch loops drift again the next time a key is added.

## T4 — `/keys` must show what dispatch actually does

**Files:** a new test in `crates/operant-cli/src/tui/keybindings/tests.rs`
or `app/dialog_key_tests.rs`.

There is no test pinning `DefaultBinding` (150 entries, 10 contexts) against
what `key_handling.rs` / `commands.rs` do. That is how the `q`-mismatch
survived: `/keys` shows a binding, and no evidence checks the handler.

**Fix:** a test that, for every catalogued surface (T2's catalogue), reads
the binding list and asserts that the surface's dispatch arm exists and
matches. Minimum viable form: assert every `BindingAction` named in
`DefaultBinding` is reachable from a dispatch arm — a name-coverage test,
not a full semantic one. Say which of the two it is in the commit body.

**Why this one matters most:** it is the only item here that prevents
recurrence rather than fixing an instance.

## T5 — Make the `[keybindings]` refusal durable

**File:** `crates/operant-core/src/config.rs:3140-3143` (+ the reason
`keybindings` is parsed-but-ignored).

The operator's contract at `config.rs:60-63` says the table is "parsed so
the file still loads, then ignored". The surface only `eprintln!`s, from
inside the config load — so it is invisible on a GUI launch, gone after
`/quit`, and the operator cannot tell a stale `[keybindings]` block from a
working one.

**Fix:** return the refusal as data from the load path and let the TUI show
it (an invalid-config card, which already exists), rather than printing and
forgetting. If that is too large a seam, the minimum acceptable version is
that the warning also appears in `operant doctor` and in the startup banner
that `/keys` renders — decide which at implementation time and say which
was done.

**Do not** implement `[keybindings]` parsing. The documented contract is
refusal; make the refusal honest.

## T6 — `excluded.json` describes surfaces that exist, and only those

**Files:** `crates/operant-cli/tests/tui_scenarios/excluded.json`.

MEASURED: 4 entries. `context-menu` and `hooks-config-menu` name surfaces
whose files are gone (`context_menu.rs`, `hooks_config_menu.rs`) and whose
render functions no longer exist — they are dead entries that inflate
`_run.py`'s `excluded_count()` (printed twice as a coverage number).
`diff-viewer-git` and `diff-viewer-turn` name two panes of a screen whose
only render fn is `render_diff_dialog`; they are arguably one surface.

**Fix:** remove the two dead entries; keep or merge the diff-viewer pair
with a comment explaining that the screen renders both panes from one fn.
Assert the total is 2 and that every entry names a renderable surface.

**Constraint:** `_run.py` only counts the list, so the count change is
cosmetic to the runner — the fix is honesty of the exclusion record, not a
coverage gain. Do not delete the file.

## T7 — Route the RT-543 checkbox to a verdict

**File:** `docs/audit/2026-06-19-BUGS.md` only.

RT-543 says the gateway "never connects" the session socket. The gateway
**does**: `gateway_runner.rs:6501` calls `run_session_socket(...)`
unconditionally, binding `session_client::default_socket_path()`
(4,811 chars of socket code, 22 call sites). What is true is narrower —
the socket is not in a health check, and nothing proves a dial succeeds.

**Fix:** flip the checkbox with the corrected premise in the same cell, and
point at the residual (a dial probe) as its own row if the owner wants it.
One `edit` call. Shared with the residuals plan's T10 — land that first.

## Explicitly NOT in scope

- **F7 renderer fan-out** — premise refuted; the 7 sites are `no_run` probes
  and 3 are test reachability. Deleting `render.rs` would strand 3 tests.
- **RT-171, RT-389, the `notify` ack** — all three claims were stale/false at
  the tip; recorded in `VERIFICATION.md` so the next round does not re-derive
  them.
- **A 4th settings surface** — none exists; the 3 legacies (`LlmProvider`,
  `Bootstrap`, `DisplayMode`) are render-variants of the one screen.
- **Implementing `[keybindings]` parsing** — the documented contract is
  refusal (T5 makes it honest, not functional).
- **RT-190 / RT-280** — owner-gated, documented as not scheduled.
- **The 28 pre-existing lib-test failures** — proven pre-existing at
  `d250e538`.
- **Palette consolidation** (33 production literal sites, down from 221 in
  iter-2516) — a design migration, not a defect. Do not fold it in.

## Order and dependency

T2 → T3 → T4 (T4's guard needs T2's catalogue and T3's dialect).
T1 and T5 and T6 are independent of those three and of each other.
T7 is a one-line docs edit, after the residuals plan.

**T1 first if only one lands:** it is the only item here that is wrong on the
operator's screen today, and it needs no schema, no new module and no
decision.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 contrast seam | `render/mod.rs`, `operant_ui/mod.rs` | ~12 | none | medium (render order; must verify no text paints after the new call site) |
| T2 one catalogue | `commands.rs`, `dialog_routing.rs` | ~60 | none | low (measuring change, not behavioural) |
| T3 key dialect | `key_handling.rs`, `dialog_key_tests.rs` | ~60 | none | medium (touches working screens' keys; the text-entry split is the trap, and `theme_screen` is on the WRONG side of it from its own file) |
| T4 parity guard | one test module | ~70 | none | low (test-only) |
| T5 refusal channel | `config.rs` (+ one caller) | ~35 | none | low |
| T6 excluded.json | `excluded.json` | ~6 | none | none |
| T7 RT-543 | `BUGS.md` | ~4 | none | none (docs) |

Seven tranches. Zero new dependencies. Zero schema changes. One shared
checkbox with the residuals plan.

# Plan: subagent preview and management upgrade

**Date:** 2026-10-30
**Kind:** plan (dated-frozen — this file is the record of what was decided; do
not edit after the iteration that implements it, write a new plan instead)
**Audit it implements:**
`docs/audit/2026-10-30-subagent-preview-and-management-surface-jcode-and-oh-my-pi-c.md`
**Boundary:** the code changes shipped in this iteration are Wave 1 only. Waves
2-3 are scoped, ranked and estimated below with the reason each is deferred.

---

## 0. What the audit changed about the direction

The instruction was: audit jcode's subagent-previews, compare against omp, plan
an upgrade. The audit's first finding inverts the premise:

- omp owns the rich surface (`docs/agent-hub.md`: roster, inspector, activity
  log, steer/revive/kill, pinned live preview, persisted discovery).
- jcode has **none** — `PickerItem` is `{ServerHeader, Session, OrphanHeader,
  SavedHeader}`, zero `subagent`/`delegation` hits in the 108 KB picker.
- operant's `RowKind::Subagent` is already closer to omp than to jcode.

So this plan ports **from omp** and touches jcode only where operant already
vendored its geometry.

---

## Wave 1 (SHIPPED in this iteration)

### 1.1 The session hub stops colliding with the idle animation

**Defect (red-team findings 1+2):** the previous session added
`SESSION_HUB_VISIBLE: AtomicBool` to `ui_animations.rs` with a reader and **no
writer** — `cargo check --all-targets` emitted `dead_code` for
`set_session_hub_visible`, and the claimed "hub open sets the flag" call did not
exist. The premise was also wrong: `render_app` paints the chrome and THEN the
overlay ladder paints the hub over the whole frame, so the animation was never
visible under the hub — the cost was a 14-row layout reservation and a full 3D
sample per frame, not a visual collision.

**Fix:**

- `TuiState::session_hub_visible()` (trait, default `false`), implemented on
  `App` as `self.session_hub.visible` — `tui_state.rs:304-320`,
  `tui_state_impl.rs:556-563`.
- `should_reserve_idle_animation(state)` — the one place the hub gate lives,
  named so it is assertable; `operant_ui/mod.rs:3185-3208`, used at `:2589`.
- The `AtomicBool` orbital and its guard inside `draw_idle_animation` deleted,
  with a comment recording why a draw-side guard cannot work.

**Why a named function, not the inline `&& !app.session_hub_visible()`:** the
inline form reads identically to a reviewer and passes identically when
deleted. An earlier version of the test asserted only the trait impl and
survived exactly that deletion — which is how the mutation test below was
chosen.

**Tests:**

- `the_hub_suppresses_the_idle_animation_band` — asserts the gate is false with
  the hub open AND equal to the vendored policy with it closed (so the
  suppression cannot leak). MUTATION-PROVEN: deleting the gate turns it red.
- `app_reports_the_session_hub_state_to_the_render_layer` — the open/close
  lifecycle through the trait.
- `a_closed_session_hub_leaves_the_animation_band_reserved` — pins the
  `14 / 0` reservation shape at `ui_animations.rs`.

### 1.2 The subagent badge draws once

**Defect (red-team finding 3):** `row_title()` already prefixes a subagent row
with `🤖 ` (`render.rs:52`), and the previous session added a second `🤖` span
in the list renderer. Runtime-rendered row: `⚡ 🤖 🤖 do research`.

**Fix:** the duplicate span is gone; what the row was actually missing is
colour, so a subagent row's title now takes the accent colour.

**Tests** (`session_hub/tests.rs`):

- `a_subagent_row_draws_exactly_one_badge` — counts glyphs in the LIST pane's
  column slice only. The first version of this test filtered whole screen
  lines and failed on a hub whose list and preview both badge correctly, which
  is why the 40/60 pane boundary (`LIST_PANE_END = 47` for a 120-wide frame) is
  measured, not assumed. MUTATION-PROVEN: re-adding the span gives
  `left: 2, right: 1` with the drawn row quoted.
- `an_ordinary_session_row_draws_no_badge` — the badge is the subagent's and
  must not spread.

### 1.3 The published debug instrumentation is removed

**Defect (red-team finding 5):** a ~90-line `mod redteam_repro` of `eprintln!`
probes was swept into commit `078e8e52` by a concurrent agent's `git add` and
reached `origin/main`. Per the append-only doctrine this is removed by commit,
never by force-push.

---

## Wave 2 (scoped, next iteration)

### 2.1 Subagent rows stop fabricating usage (audit Gaps B + C)

`$0.0000 · 0 msg · 0.000k tok` on a subagent row asserts facts operant does not
have. omp prints `usage —` (`agent-hub.md:36`) rather than estimating.

Change `render.rs`'s meta line so a `RowKind::Subagent` prints `usage —` for
cost/messages/tokens and the **delegation status** from
`RowKind::Subagent.status` (Pending / Completed / Failed) — currently visible
only in the focused card. ~40 lines, no core change.

Tests: a subagent row's meta contains `usage —`; a session row keeps its real
numbers; a `Failed` and a `Completed` row differ in the list without focusing.

### 2.2 The gateway gets a real subagent command (audit §5)

`/agents` lists **platform sessions** (`gateway_commands.rs:1476-1494` calls
`get_session_store()`), never the delegation registry. The gateway operator has
no command for the right surface.

Add `/subagents` reading `async_delegation::list_records()` (module-level,
`async_delegation.rs:171`) and formatting through the existing Markdown layer,
so all seven platforms get it at once. `/agents` keeps its current meaning and
its description is corrected to name it ("Show active chat sessions").

Tests: `/subagents` renders goal/status/age per record; an empty registry says
so rather than printing a header with nothing under it; `/agents` output is
byte-unchanged.

---

## Wave 3 (scoped, deferred with reasons)

### 3.1 A live tail on the subagent row (audit Gap A — the biggest gap)

omp's `subagentLivePreview` second line (current tool call, one-line detail,
elapsed past 5s) is what makes a roster a window. operant's rows carry no event
stream.

**Deferred because it is core-side, not hub-side:** `RowKind::Subagent` has no
feed, and nothing keys `AgentEvent`s by delegation id. The cheap half —
re-rendering the row each time `async_delegation` transitions a record, plus
age from `dispatch_time` — needs one new field on the registry read path and is
the only part with a bounded diff. Full tool-call streaming needs a new event
key and its own plan.

### 3.2 Lineage and a parent/child tree (audit Gap D)

Needs a parent field on `RowKind::Subagent`; blocked until 3.1 lands so the two
do not collide on the same row struct.

### 3.3 Control surface: steer / revive / kill (audit Gap E)

Each needs a core API that does not exist. Feature work, not bug work, and the
instruction's plan-shaped request makes it a plan row rather than an
implementation.

---

## Explicitly rejected

- **Porting jcode's picker model.** Its `PickerItem` has no subagent variant;
  adopting it would delete `RowKind::Subagent`. jcode is the geometry baseline
  operant already vendored, not the subagent reference.
- **Hiding the whole chat frame when the hub is open.** The hub overpaints it
  correctly; skipping the chrome would desync the published band rects that
  mouse hit-testing reads (`render/mod.rs:57-95`).

---

## Verification recorded for Wave 1

- `cargo fmt --all` clean
- `cargo clippy -p operant-core -p operant-cli --all-targets`: **0 warnings**
  (the `derivable_impls` on `CapabilityManager` and the `let_unit_value` from
  the removed probe both cleared; `derivable_impls` fixed by deriving `Default`)
- `session_hub::tests`, `app_reports_the_session_hub`,
  `the_hub_suppresses_the_idle_animation_band`, `operant_ui::animations`:
  **46 passed, 0 failed**, both mutations re-run red before revert
- The workspace clippy gate is currently RED for `crates/operant-core/src/
  model_roles.rs` (`E0382`) — an untracked, in-flight file from a concurrent
  agent. Per fleet rule 7 it is not touched; this plan's delta adds no warnings.

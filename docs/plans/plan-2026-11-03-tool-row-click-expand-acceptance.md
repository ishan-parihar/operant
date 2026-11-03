# Click-to-expand acceptance: make the headless harness able to see it

Dated-frozen: published `2026-11-03`. Registered in `genome.toml`
`core.living_plans`.

## The request

Close the one gap that the RT-144/145/146 validation round (iters 1668 and
1672) could not close from the agent side.

The work itself is done and proven. RT-144 removed the vim badge, RT-145
retired the idle animation, RT-146 added click-to-expand. Eight mutation tests
were caught across four guards, both input paths were traced executing live in
a running binary, and the full `operant-cli` suite is 2090 passed / 1 failed at
the tip of that round — the one failure proven pre-existing on three axes (it
failed identically at iter-1668, its file was last touched by iter-910 and
iter-686, and none of the four RT commits touch that file).

What is missing is **observability**, not behaviour:

- `App::debug_snapshot()` (`tui/app/commands.rs:96`) exposes `selection_anchor`,
  `selection_focus`, `terminal_scroll_mode`, `scroll_emitted_through` and
  friends precisely so a headless scenario can assert on state instead of
  goldenning an unpainted screen. It exposes **nothing** about tool rows:
  `expanded_tool_id` is unreachable, so no scenario can assert an expansion
  happened.
- The corpus has no scenario that clicks a tool row. The only two files that
  carry a `<left,x,y>` mouse event are `selection-highlight.scenario.json` and
  `selection-highlight-chrome.scenario.json`, both prose-only
  (`assistant-simple.json`) and both asserting selection state.
- `debug_snapshot` also omits `tool_full_outputs` length, so a scenario cannot
  distinguish "the click was inert because output was evicted" from "the click
  expanded".

Consequence: the harness cannot see the feature it exists to protect. A future
regression that silently breaks click-to-expand would leave every scenario
green.

This plan supersedes nothing. It is the acceptance sibling of the landed
`plan-2026-11-02-codex-parity-composer-and-session-organiser.md`.

---

## Scope

Two additions and two scenarios. No behaviour change, no new dependency, no new
config key.

| # | Change | File |
|---|---|---|
| 1 | Expose tool-row state in `debug_snapshot` | `crates/operant-cli/src/tui/app/commands.rs` |
| 2 | Regression unit test pinning the new keys | `crates/operant-cli/src/tui/app/tests.rs` |
| 3 | A corpus scenario that clicks a tool row and asserts expansion | `crates/operant-cli/tests/tui_scenarios/tool-row-click-expands.scenario.json` |
| 4 | Its collapse-half sibling | `crates/operant-cli/tests/tui_scenarios/tool-row-click-collapses.scenario.json` |

No change is needed in `cmd_tui_debug.rs`: `evaluate_assertions`
(`cmd_tui_debug.rs:1355`) navigates the snapshot by dot-path generically, so a
new key is assertable the moment it exists.

### What this deliberately does NOT do

- It does not change `--frame-per-event` to also capture the frame *after* the
  final event. That is a deeper harness change (the run loop must paint once
  more before it exits) and it would move the golden baseline of every existing
  scenario. Out of scope; the state assertion makes it unnecessary here.
- It does not fix the pre-existing mermaid failure. That is already recorded in
  `docs/audit/2026-06-19-BUGS.md` as a fleet-rule-7 "recorded, not fixed" item,
  and fixing it is a different slice.
- It does not turn `show_bash_output` on. See Part 3b — that is the one lever
  that would make a *screen* oracle possible, and it is a behaviour change, not
  a test-harness change.

---

## Part 1 — Why a state assertion, not a golden screen

The screen is the wrong oracle here for a specific, measured reason: the
headless run loop drives all simulated keys before all simulated mouse events
(`tui/app/mod.rs:2055-2064`), and it exits when both queues drain
(`tui/app/mod.rs:1599-1602`). An expansion triggered by the *last* event
therefore has no subsequent frame to paint into. This was measured directly:
266 captured frames, 0 containing the expanded output, while a trace on the
same run proved the toggle fired and the adapter produced the 30-line body.

A state assertion sidesteps that entirely: `--assert` is evaluated against the
final `App` after the loop ends (`cmd_tui_debug.rs:1769-1772`), exactly where
the selection scenario already gets its `selection_anchor != nil` proof.

The precedent is the important part. `selection_anchor` / `selection_focus`
were added for the identical reason, with the identical comment in the source:
*"the scenario corpus asserts on these to prove a simulated drag actually
anchored/focused a range instead of goldenning an unpainted screen"*. This plan
extends the same discipline to tool rows.

---

## Part 2 — The snapshot keys

Three keys, chosen so a scenario can tell the three outcomes apart:

| key | type | why |
|---|---|---|
| `expanded_tool_id` | `String` or `null` | the contract itself. `null` collapsed, `Some(id)` expanded |
| `tool_full_outputs` | number | distinguishes "inert: output evicted" from "collapsed by choice". Also catches a `KEEP_CALLS` regression |
| `tool_calls_seen` | number | how many tool blocks the transcript holds, so a scenario can assert it seeded what it intended |

`expanded_tool_id` is the valuable one; the other two are cheap and make a
failure diagnosable rather than merely red.

`expanded_tool_id` is already a plain `Option<String>` field on `App`
(`tui/app/mod.rs:188`), so the snapshot is a one-line `serde_json` add. No
borrow gymnastics are needed — unlike `selection_text`, which goes through a
`RefCell`.

### The placement rule

The keys go beside `selection_*` in the `json!` literal, **not** into the
`overlays` object. `overlays` is derived from `overlay_flags()` and feeds
`any_modal_open()`; putting a non-overlay key there would change real
behaviour (it gates error-modal precedence, toast suppression, and key
routing). The source comment at `commands.rs:127-135` states the rule; the new
keys belong under it, not against it.

---

## Part 3 — The scenario

Shape follows `selection-highlight.scenario.json` exactly, including its
`variants` and `notes` fields.

```json
{
  "name": "tool-row-click-expands",
  "surface": "crates/operant-cli/src/tui/app/mouse.rs::handle_mouse_event::tool_row_region_at",
  "opens_with": "(agent script seeds a tool row, then a left click on it expands its full output)",
  "keys": "/voice<enter>",
  "mouse": "<left,10,23>",
  "size": "120x40",
  "agent_script": "tool-run-completed.json",
  "assert": [
    "expanded_tool_id == \"call_1\"",
    "tool_full_outputs >= 1",
    "tool_calls_seen == 1"
  ],
  "notes": "The region starts one row ABOVE the rendered summary glyph: measured start_line 23, end_line 24 in a 120x40 frame, so screen row 23 hits and row 24 does not. `keys` must clear the boot connect dialog first — it swallows the first Left-Down. Asserted on state, not on a golden, because the harness never paints the frame that follows the final simulated event."
}
```

Four details that are load-bearing, each learned by measurement:

1. **`keys: "/voice<enter>"` is mandatory.** The boot connect dialog (no
   credentials in the pinned config) swallows the first Left-Down —
   `selection-highlight.scenario.json`'s own `notes` field records this, and my
   live tmux run hit it as a swallowed click. Without the `/voice` prefix the
   click never reaches the transcript.
2. **The tool row is at content line 23, screen row 23** in the measured
   120x40 layout with `content_top = 0`. The published region was
   `start_line: 23, end_line: 24`, so `abs_line = 23` is the row that hits.
   This is exactly the off-by-one that cost the most time during validation:
   the *rendered* summary glyph sits at row 24, but the clickable region starts
   one row earlier.
3. **`tool-run-completed.json` is the right script to start from** — its output
   is `# operant\n\nterminal-native AI agent` (3 lines), which is *identical* to
   the collapsed preview's last-3-lines. So nothing visibly changes on expand,
   and a screen oracle is useless here no matter what. Verify with
   `tool_calls_seen == 1` that the script seeded the one row you intended.
4. **`assert` uses the exact string the tool carries**, `"call_1"`, which the
   scenario pins. If a future change re-keys tool ids, the assertion fails
   loudly rather than silently passing on a `null` mismatch.

A second scenario pins the collapse half:

```json
{
  "name": "tool-row-click-collapses",
  "keys": "/voice<enter>",
  "mouse": "<left,10,23><left,10,23>",
  "agent_script": "tool-run-completed.json",
  "assert": ["expanded_tool_id == null"],
  "size": "120x40"
}
```

Two clicks on the same row must return to `null`. This is the mutation I broke
first during validation (`self.expanded_tool_id = Some(...)` unconditionally),
so it is the one with a proven regression history.

The one-at-a-time contract ("click row B while row A is open closes A") is
already pinned by the unit test `clicking_a_tool_row_toggles_that_call_only`
at `app/tests.rs:11034`, so it does not need a scenario.

---

## Part 3b — The measurement that settles the oracle choice

This finding makes a state assertion not just convenient but **necessary**, and
it should be re-run before any code is written, because it has already changed
this plan once.

`show_bash_output` defaults to **`false`**
(`operant_app/config_shim.rs:345`; `operant.example.toml` ships
`show_bash_output=false`). In the headless sim — which is `#[cfg(not(test))]`,
so `ui_tools.rs:57` reads the real config rather than the
`tests_show_bash_output_override` a unit test can flip — **no tool row renders
an output body at all**, the same as a `read_file` row.

Consequences that must not be discovered mid-implementation:

- **The test-only override cannot help here.**
  `tests_show_bash_output_override::set(true)` is `#[cfg(test)]`-only, so a
  headless *simulation* — a separate process running the debug binary — cannot
  set it. Only in-crate unit tests can.
- **No existing agent script renders a body.** Every tool script under
  `tests/tui_scenarios/agent_scripts/` uses `read_file`, `aft_read`, or
  `shell`, and none has a tool output longer than 3 lines (measured across all
  of them).
- **Therefore a screen oracle cannot work for this feature at all** in the
  current config. The two honest ways to get one — turning `show_bash_output`
  on in the pinned sim config, or adding a body-rendering path — are both
  behaviour changes and out of scope here.

So: assert state. If a future slice wants a *visible* proof, the honest route is
flipping `show_bash_output` in the pinned sim config, which would then let a
golden work. That is explicitly **not** this plan.

---

## Part 4 — The regression unit test

One test in `app/tests.rs`, next to the existing RT-146 tests, pinning that the
snapshot keys exist and are honest:

```rust
#[test]
fn debug_snapshot_reports_tool_row_expansion_state() {
    let mut app = app_with_three_clickable_tools();
    let snap = app.debug_snapshot();
    assert_eq!(snap["expanded_tool_id"], serde_json::Value::Null);
    app.toggle_tool_expansion_by_id("call-0");
    let snap = app.debug_snapshot();
    assert_eq!(snap["expanded_tool_id"], "call-0");
    assert_eq!(snap["tool_full_outputs"].as_u64(), Some(3));
    assert_eq!(snap["tool_calls_seen"].as_u64(), Some(3));
}
```

`app_with_three_clickable_tools()` is the existing fixture at
`app/tests.rs:10988`, already used by
`published_tool_regions_start_at_the_prepared_frames_ranges` and all three
click tests. **The ids are `call-0`, `call-1`, `call-2`** (verified by reading
the fixture, not assumed) and it seeds three `bash` tool blocks, each with a
retained 3-line full output.

Two fixture details that will bite an implementer who assumes otherwise:

1. **The fixture uses `bash`, not `read_file`, deliberately.** The fixture's own
   comment records why: `read_file` rows render a single summary line with no
   output body at all, so "expand" would have nothing to show.
2. **`tests_show_bash_output_override::set(true)` is required** for those rows
   to render the body — and the fixture sets it. This is what makes the *unit
   test* able to see an expansion while the *simulation* cannot (Part 3b).

No new fixture is needed.

The test pins the *seam* (the snapshot must expose the field) which is the part
a future refactor would silently drop — the same reason
`the_resize_anchor_seam_is_wired_at_both_ends` pins `record_reader_anchor(` in
the source text.

---

## Verification

```bash
source scripts/dev-env.sh
cargo fmt --all
./scripts/check.sh check -p operant-cli --bin operant

# the fast half of the proof — do this BEFORE the scenarios
cargo test -p operant-cli --bin operant debug_snapshot_reports_tool_row
cargo test -p operant-cli --bin operant tool_row_hit_test tool_row_regions clicking

# the corpus. OPERANT_BIN must point at a FRESH debug build: a stale binary
# silently reports the pre-change state and the scenario lies.
cargo build -p operant-cli --bin operant
OPERANT_BIN=$PWD/target/debug/operant \
  python3 crates/operant-cli/tests/tui_scenarios/_run.py tool-row-click-expands
OPERANT_BIN=$PWD/target/debug/operant \
  python3 crates/operant-cli/tests/tui_scenarios/_run.py tool-row-click-collapses
```

Both must PASS. Then mutation-prove the guard bites:

```bash
# remove `expanded_tool_id` from the snapshot, then re-run the scenario.
# It must FAIL with "Unknown assertion path: 'expanded_tool_id'" — a loud
# failure, not a silent skip — and the unit test must FAIL too.
# Restore it and confirm both green again.
```

Then the full sweep before push:

```bash
bash scripts/clippy-warning-gate.sh      # no NEW warnings
cargo test -p operant-cli --bin operant  # re-derive the count; at the last
                                         # measurement 2090 pass + the 1
                                         # mermaid pre-existing failure
```

Do **not** add a `baseline` to either scenario. A golden of a collapsed-versus-
expanded screen is exactly the trap Part 1 argues against — and per Part 3b it
could not even detect the difference.

---

## Risks

| risk | mitigation |
|---|---|
| Adding snapshot keys changes a golden somewhere | keys are additive; the corpus asserts via `--assert`, not `--dump-screen`, so no golden moves. Verified by running the full corpus after |
| The click row is wrong at another terminal size | ship 120x40 only. Add an 80x24 `variants` entry only after measuring the real row there — the selection scenario needed a *different* row per size (24 vs 16) |
| `--assert` string comparison on the id is brittle | deliberately brittle: a silent re-key of tool ids should fail, not pass |
| A peer is mid-flight in `commands.rs`, `mouse.rs`, or `tests.rs` | `git status` before staging; stage only the four files above |
| A peer's fetch/checkout discards the untracked plan or scenarios mid-flight | commit the plan + scenarios early in the same iteration rather than leaving them untracked |

---

## Resume here

Nothing is started at time of publishing; only this plan exists.

**Start at Part 3b, not Part 1.** The measurement that settles the oracle
(`show_bash_output` defaults to false, so no tool row renders a body in the
headless sim) is the finding that determines whether the rest is even shaped
right. Re-run it before writing any code — it is three commands and it has
already changed this plan once.

Then, in order, one commit:

1. `commands.rs` — add the three keys beside `selection_*`, **not** in
   `overlays` (the placement rule is in the source at `commands.rs:127-135`;
   putting a non-overlay key in `overlays` changes real behaviour).
2. `app/tests.rs` — `debug_snapshot_reports_tool_row_expansion_state`, using
   `app_with_three_clickable_tools()` whose ids are `call-0/1/2`. Green this
   **before** the scenarios: it is the fast half of the proof.
3. The two `*.scenario.json` files, driven with `OPERANT_BIN` pointed at a
   freshly built debug binary.
4. Mutation-prove the guard: remove `expanded_tool_id` from the snapshot,
   confirm the scenario fails with `Unknown assertion path`, restore it.

**Expected numbers to re-derive, not copy:** the last full-suite measurement
was 2090 passed / 1 failed, the one failure being the mermaid side-panel test
already recorded in `docs/audit/2026-06-19-BUGS.md` as a fleet-rule-7 item. The
fleet has advanced since (iter-1684–1686 landed during the validation round),
so re-run it.

**Do not** add a `baseline` to either scenario. That is the golden-screen
oracle Part 1 argues against, and per Part 3b it could not even see the
difference.

---

## Definition of done

- [ ] `debug_snapshot` exposes `expanded_tool_id`, `tool_full_outputs`, `tool_calls_seen`
- [ ] `debug_snapshot_reports_tool_row_expansion_state` unit test green
- [ ] `tool-row-click-expands.scenario.json` PASSES
- [ ] `tool-row-click-collapses.scenario.json` PASSES
- [ ] Removing the snapshot key makes the scenario FAIL loudly (mutation-proven)
- [ ] Full corpus green with no golden drift
- [ ] `clippy-warning-gate.sh` reports no NEW warnings
- [ ] One commit, type `feat(iter-NNNN)`, pushed, release built and deployed

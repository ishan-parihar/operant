# Audit — dead-code suppression triage: 242 to 19, and what each remaining suppression is for

**Date:** 2026-10-21
**Agent:** jcode
**Supersedes:** the "242 suppressed warnings" figure in
`docs/RUST_BEST_PRACTICES_PLAN.md:25` (dated 2026-08-02, sourced from
`docs/DEAD_CODE_GAP_ANALYSIS.md`). Both files are dated-frozen and are left
untouched; this is the live measurement.

## What was measured

`grep -rn "allow(dead_code)\|allow(unused" crates/ --include=*.rs` across the
workspace after iter-1163:

| class | count |
|---|---|
| `#[allow(dead_code)]` | **19** |
| `#[allow(unused_*)]` | **6** |
| `#[expect(dead_code)]` | **5** |
| combined remaining suppressions | **25** |

The plan's figure of **242** is off by nearly an order of magnitude, and
`docs/DEAD_CODE_GAP_ANALYSIS.md`'s own breakdown (197 `dead_code` + 45
`unused_*`) with it. Whatever that document measured on 2026-08-02 has since
been retired wholesale — the config derive machinery, the parallel config
aggregate, the retired provider surface and the three deleted crates carried
most of it. The plan's target ("triage to 0") is not met: **25 remain**, and
this audit records what each one is, so the remaining triage is informed.

## The 19 `allow(dead_code)`, by reason

**Genuinely removable (0 left after iter-1163's neighbours).** The two
`session_sqlite.rs` and `skills_tool.rs` sites were write-only fields and an
orphaned static; both are now gone or converted to `#[expect]`.

**Feature-conflicting (must stay `allow`).**

- `operant-providers/src/gemini.rs:221` — its own comment is correct and was
  verified: the type is used by the default-feature response path and unused
  under `--all-features`, so `#[expect]` goes unfulfilled under one of the two
  feature sets. `allow` is the only correct choice.
- `operant-core/src/agent/background_review.rs:614` — a
  `#[cfg_attr(not(test), allow(dead_code))]`, documented as exercised only by
  unit tests today. The comment already explains why `expect` cannot work
  (the lint fires only outside `cfg(test)`).

**Macro-generated uniform schema (must stay `allow`).**

- `operant-cli/src/tui/operant_app/todo.rs` (2 sites, on `as_str` and
  `legacy_score` in the `semantic_state!` macro). Verified by experiment:
  swapping them to `#[expect]` produces exactly two "this lint expectation is
  unfulfilled" warnings, because `ConfidenceState` really does consume both
  (`info_widget_todos.rs` aggregates numerically through `legacy_score`). A
  macro generates one impl for eleven enums with differing consumers, and
  `#[expect]` requires the lint to fire on EVERY expansion. The measurement is
  in the comment at the attribute.

**Intentional excision notes (documented, safe).**
`tui/banner.rs` and `tui/rustle.rs` (both: the module-level suppression was
removed in iter-392, and the mention left is a note about that, not an
attribute), `tui/operant_model/adapter.rs`,
`tui/app/redraw_reason.rs` (whose test asserts the allow's absence),
`core/tools/kernel/sidecar.rs` (`bridge_rx`, "consumed by tool_bridge drain
loop (phase 2.5 wiring)"), `providers/bedrock.rs:575` (an enum variant that
holds a parsed payload for a shape only some responses use), three
`operant-harness/src/row.rs` / `pool_provider.rs` fields documented as Phase-3
stub carriage, `core/examples/simple_agent.rs` (2, example code), and two
`operant-harness/tests/` sites.

## The 6 `allow(unused_*)`, all documented

`ui_viewport.rs` x3 (`consumed by the gated inline-image block until W6`),
`operant-config/src/policy.rs` `unused_mut` (platform-dependent vec mutation),
`core/agent/run.rs` `unused_assignments` (a collect-before-match pattern with
the reason inline), `core/mcp_oauth.rs` `unused_variables` (a poisoning-recovery
path that keeps the signature).

## What this audit does and does not claim

It claims the COUNT and the REASON for each of the 25 remaining. It does not
claim the count is stable: new work adds suppressions, and the right gate is a
script rather than a prose number. The measurement is one grep, so the next
agent can re-run it rather than re-derive it.

**Recommended next step:** a `scripts/audit-lint-suppressions.sh` that counts
`allow(dead_code)` / `allow(unused_*)` / `expect(dead_code)` and fails only on
an entry outside a baseline carrying its reason — the same shape
`scripts/audit-config-surface.py` took in iter-1153 (RT-159), which turned a
permanently-red check into one that can express "new".

---

## Addendum (iter-1164 to iter-1169, same session)

**The recommended next step above was built, and it immediately found that it
did not work.** `scripts/audit-lint-suppressions.py` landed in iter-1164
modelled on `audit-config-surface.py`, and when it was first run against the
annotations this sweep was adding it governed **30 of the repo's 80
suppressions** -- because its regex matched only the bare `#[allow(...)]` /
`#[expect(...)]` spelling. Every `#[cfg_attr(not(test), allow(...))]` and every
rustfmt-split multi-line attribute was invisible to it, including ones carrying
their own `reason` string. A gate that cannot see a suppression cannot govern
it: the same defect class RG-013 records for the doc gate, recreated in the tool
meant to close it. Four defects, all measured:

1. `RE_ATTR` could not align the cfg_attr form -- in `#[cfg_attr(test,
   allow(...))]` the `allow` sits INSIDE cfg_attr's parens, not after them, so
   the optional-prefix pattern returned `False` for every cfg_attr line.
2. a per-line scan cannot see an attribute rustfmt split over four lines; the
   scan now joins each unclosed `#[...]` through its matching paren.
3. `item_after()` walked from the attribute's first line and read the cfg
   predicate (`test,`) as the item name, so every cfg_attr site was keyed `test`.
4. `item_after` was called with a 1-based line number as a 0-based index.

**The dead-code sweep itself: 54 warnings to 0, all test-profile.** A sweep of
`cargo clippy -p operant-cli --bin operant --all-targets` measured 54 warnings,
every one `dead_code`, with the PRODUCTION profile clean at 0 throughout
(re-checked after each file, never assumed). Each was one of: a wire-format
field the tests parse but no live row reads; render state the live panel never
enters; a picker action or result vocabulary no live picker returns; a
config-shim field kept so the shape matches upstream. Each was annotated with
`#[cfg_attr(test, allow(dead_code, reason = ...))]` -- the TEST-profile arm,
because an `#[expect]` would be unfulfilled in production where the lint does
not fire. The idiom is the one `operant_model/adapter.rs` already documents
(name the dead variants in the module head, keep the explicit arms, so the first
live construction is a visible decision point).

**Final state:** 119 suppressions across 61 files, 119 baseline entries, 0
unexplained. `cargo clippy -p operant-cli --bin operant --all-targets` emits
ZERO warnings, verified after forcing a rebuild of every touched file.
operant-cli stays 1638 passed / 0 failed.

**Two gate defects found by the gate itself while widening it**, both fixed:
28 duplicate `BASELINE` keys entered in iter-1168 (one collided so two different
`row.rs` `config` fields shared a single reason), and the second field's
`(2)`-suffixed key was among the deleted.

**Mutation-checked twice, and the second run corrected the first.** Adding an
unexplained `#[allow(dead_code)]` on an item that already carries a suppression
matches an existing baseline key and is correctly NOT a new site -- the gate was
right and my mutation was wrong. The valid mutation (a suppression on an
unsuppressed item) makes the gate name the file and line and exit 1.

**What is still open, with the reason:** RT-168's two `CopyTargetKind` enums are
annotated (the suppressions now point at the row) but NOT collapsed, for the
three reasons its own row records. RT-169's doc gate blind spot is recorded with
a measurement and deliberately not fixed. The `usage.rs` report path is ported
and has exactly one constructor -- its own module's test -- so it is
ported-but-unwired, which is now stated in the type's doc rather than left for a
reader to discover.

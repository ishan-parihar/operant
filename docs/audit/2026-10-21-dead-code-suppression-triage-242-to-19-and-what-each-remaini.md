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

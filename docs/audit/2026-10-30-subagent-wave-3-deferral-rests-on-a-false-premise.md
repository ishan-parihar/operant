# Audit: the 2026-10-30 subagent plan's Wave-3 deferral rests on a false premise

**Date:** 2026-10-30
**Kind:** audit (dated-frozen)
**Amends:** `docs/plans/2026-10-30-subagent-preview-and-management-upgrade.md` § 3.1
**Prior art:** `docs/audit/2026-10-28-session-management-red-team-audit-all-surfaces-vs-jcode-oh-m.md` § 1.5 ("Subagents: exist, but invisible and second-class") and the CHANGELOG entry "A usage overlay (F8) and background task rows showing live delegation state, both fed by data that already existed but was never surfaced"

## The claim this audit corrects

The plan defers Wave 3.1 — "a live tail on the subagent row (audit Gap A — the biggest gap)" — with this reason:

> **Deferred because it is core-side, not hub-side:** `RowKind::Subagent` has no feed, and nothing keys `AgentEvent`s by delegation id.

The first clause is narrowly true and the conclusion built on it is false. **The live tail already exists and already renders in production.** The survey looked in `session_hub/` and did not look in `background_tasks.rs`.

## What exists, verified by reading the live call site

`crates/operant-cli/src/tui/background_tasks.rs` is a per-frame, delegation-keyed live roster:

- `BackgroundTaskRegistry::refresh()` re-reads the process-wide registry on **every frame** — called from `render_app` (`tui/render/mod.rs:100`), which is the production render entry point driven by `terminal.draw` in the main loop (`tui/render/mod.rs:169`), not a test harness.
- `sync_from` keys each row by `rec.delegation_id` (`background_tasks.rs:194`) — so "nothing keys AgentEvents by delegation id" is true of `AgentEvent` specifically and irrelevant, because the tail is driven by the **registry read**, not by `AgentEvent`.
- `render_rows` → `row_spans` prints, per row: a state glyph, the **delegation id**, the state label, a **live elapsed seconds** (`row.elapsed_secs(now)`), and a clipped goal. `rows_area` docks them bottom-left of the messages band, above the transcript and below the overlays.

Against the plan's own description of omp's `subagentLivePreview` ("current tool call, one-line detail, elapsed past 5s"), operant already ships the id + status + live-elapsed + goal. It does not ship the current tool call.

The pin test the module's own comment describes exists precisely because "this wiring is invisible when it breaks: the module still compiles, still passes its tests, and quietly shows the user nothing" (`tui/render/mod.rs:97-99`). That is the opposite of the plan's premise that no feed exists.

## Where a real gap does exist

The two surfaces are separate, and neither substitutes for the other:

| Surface | Fed by | Live? | Shows elapsed? |
|---|---|---|---|
| `background_tasks` rows (transcript band) | `BackgroundTaskRegistry::refresh()` per frame | yes, per frame | yes, in seconds |
| session-hub `RowKind::Subagent` | `async_delegation::records_snapshot(), loaded async when `session_list_pending` is set (`app/mod.rs:1332-1352`) | on open only | no; `dispatch_time` becomes an absolute timestamp in the preview only (`session_hub/render.rs:457`) |

I first wrote this row as "a stale snapshot at open time" and then checked it: the hub re-reads on **every** open, not once. `session_list_pending = true` is set at five sites, including the double-left gesture (`key_handling.rs:1720`) and `/resume` (`commands.rs:529/540`). So the defect is not a frozen list — it is that the list is frozen **while the hub stays open**, which is exactly when the operator is watching it.

So the honest defects are:

1. **The hub's subagent rows do not age or change state while it is open.** Open the hub, and a row that reads `pending` keeps reading `pending` until the hub is closed and reopened, no matter how long the operator watches it. The live tail sits one band away and is correct throughout.
2. **`tui_state_impl.rs:203-207` returns an empty slice** for `background_task_rows()` with a `[port-decision]` comment ("wire when the registry grows a …"). One consumer of the live tail is stubbed off, so the two rosters are not even shareable today.
3. `AgentEvent` genuinely is not keyed by delegation id, so a *tool-call* tail (the part omp has and operant lacks) is still missing — that is the real remainder, and it is far smaller than the plan's "biggest gap" framing.

## What this means for the plan's ranking

Wave 3.1 as written should be **withdrawn, not deferred**, because its stated blocker is false. Its useful residue is reframed: make the session-hub read the live registry (or refresh on frame while open) so its status and age cannot go stale, and decide whether the two rosters should merge. Waves 3.2 (lineage, blocked "until 3.1 lands") and 3.3 (control surface) inherit that blocker and should be re-scoped against the corrected premise rather than against the dead one.

## Method note

Every claim here was checked at the call site, not inferred from the module name: `render_app`'s production status was established by finding its `terminal.draw` caller, and "the tail renders" was established by reading `row_spans`, not by the module's tests. The failure that produced the plan's error is the usual one — a survey that greps for `AgentEvent` and concludes absence, when the mechanism was never `AgentEvent`-based.

---
ID: "AD-009"
Title: "A gateway budget is metered per seat turn, and a hard cap fails closed"
Status: "Active"
Date: "2026-10-19"
Description: "Spend is recorded per turn against the start-time seat and the window reads that ledger; a hard cap whose usage cannot be read refuses"
---

# AD-009: A gateway budget is metered per seat turn, and a hard cap fails closed

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The Wave-4 budget window summed `gateway_sessions.total_tokens` over the rows whose *current* `employee_id` matched the seat (GW-1). `total_tokens` is a lifetime total per chat, so a chat's whole history counted in every window it was touched in, and a rebind moved the whole history to the new seat. Live check recorded in BUGS GW-1: chief-of-staff had 8,232,833 tokens across 3 rows updated since 2026-10-14, of which 4,111,430 were on rows created before 2026-10-14.

The gate also treated a failed usage read as "no budget" (GW-8). The bylaws invariant, "metering must never refuse a run because the meter could not be read", is written for telemetry writes. A hard cap is a limit, and a limit whose spend cannot be read is not known to hold.

## Decision

1. Each turn writes one row to `seat_spend` (`employee_id`, `recorded_at`, `tokens`, `cost_usd`), charged to the seat the turn started under: `bound_employee` at turn start for a chat turn, the run's seat for a cron run. The window sums that ledger where `recorded_at >= since`. The `gateway_sessions` totals remain for display and are no longer read by the budget.
2. The ledger starts empty on upgrade. Pre-upgrade spend is not carried into any window, because carrying the lifetime totals over would reproduce GW-1.
3. A hard cap whose usage cannot be read refuses the turn (chat) or the run (cron), with a message that names the read failure. Soft mode runs the turn and loses only the injected remainder. A missing store stays ungoverned, as before.
4. The seat-override read (`seat_budgets.get`) still degrades to the default budget on error. This is a residual: a hard override that cannot be read is governed by the default budget.

## Consequences

- A turn is charged on the window in which it finishes. A turn that spans a window boundary counts in the later window, and its spend is not split.
- A chat's spend stays with the seat it was earned under after a rebind, so the new seat starts from its own window.
- A locked or missing usage table now refuses a hard-capped seat's turns until the read works. That is the intended trade for a hard cap.
- Tests: `budget_gate_refuses_a_hard_cap_whose_spend_cannot_be_read`, `budget_gate_soft_mode_still_runs_when_the_meter_is_unreadable` (cron), `a_rebind_does_not_move_spend_and_the_window_rolls` (store), `wave4_employee_window_usage_rolls_up_the_accumulator` (store), `wave4_turn_usage_drains_into_the_budget_accumulator` (runner drain). The runner's own hard refusal has no unit seam; it shares the rule above and was checked by reading.

## Orthogonality

AD-004 covers which employee ids a session may bind. AD-010 covers which transcript a seat runs on.

## Revisions

- 2026-10-19: created

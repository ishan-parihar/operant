# 11 — Answers to the 2026-10-15 re-audit brief

Source: `docs/audit/2026-10-15-batch-remaining-gaps.md` §5 (8 questions).
Short answers, then what to do with each remaining batch plan.

## §5 checklist

**1. Direction: is the remaining batch still the right next work?**
No, not first. This audit found correctness bugs in work the batch marked
**done**, and they come before any remaining phase:

| Batch plan marked done | What's still wrong | Fix in |
|---|---|---|
| 02 dead commands | 18 autocompleted commands still print "not yet wired"; `/split` and `/mcp <args>` go to the model | `04` |
| 10 shortcut/overlay coherence | typeahead reads a different table than dispatch; `/rewind`, `/permissions`, `/effort` never autocomplete | `04` |
| 03 reasoning effort | fine (it reaches the agent). But the model it applies to is not the model the picker shows | `05` A |
| 04 cancel preserves context | `/cancel` and Esc are right; `/stop`, `/clear`, `/new` bypass it and allow a second concurrent run | `03` §1 |
| 05 rewind | fine. `/undo` and `/retry` should reuse it and don't | `03` §2 |
| 09 step 1 (K-3) | still a row watermark; no reset on shrink; resize drops rows | `07` |
| 11C3 memory-wire only | agent yes; `operant memory`, doctor, `/whoami`, `/journey` no; the defaults warn | `08` |
| 11E core seam | seam exists and nothing uses it; the gateway has one bank | `06` B |

Superseded plan sections (delete them or mark them superseded in the batch files):
- **06-context-management P2** (§N§ tags over `context_injection`): superseded for
  org mode. Holon messages carry identity. Keep only if personal-mode
  channel context needs it, and nothing in this audit says it does. Mark it
  "deferred, no consumer".
- **06 P3/P4 historian → memory-wire**: personal-mode only (same pattern as
  `seat_authority`). A Holon run never writes (O-22). Re-scope the plan text to
  say so. It writes to the seat's `emp:` bank via `06` B's `set_scope`.
- **11E (a)** "synthesis stops opening its own store": moot (synthesis.rs is retired).
- **11E (c)** charter postscript: Holon owns org identity. Personal-mode seats
  only.

**2. Boundary statement (O-22 vs 11E vs 06.P4):** written in `06` part B. In short:
personal mode = memory-wire banks (`default` owner, `emp:<seat>` per seat, no
cross-reads). Holon runs = never write any operant bank, and learnings go to `result.json`.

**3. 06.P0 cache stability vs the usage/refusals sidecar in `prompting.rs`:** still
valid, but it is an optimisation. Run it after `02`–`09`. Re-derive the RED test
against current `build_messages` (`crates/operant-core/src/agent/run.rs`) at
that time. Line numbers will have moved.

**4. 07.2 shared services:** **fold into the thin-client decision; don't
start.** `docs/MEMORY_BUDGET.md` says idle is already at target (gateway 26.7 MB,
TUI 24–25.5 MB). A services socket inside the organ-hosting gateway adds a
protocol and a failure mode for no measured need. Note: `06` A puts a turn
lock in the same daemon, so measure queue wait before any thin-client design.

**5. `seed_p3` fixture:** prescribed in `02`. Delete the grant/revoke tests (that
feature is Holon's now; production prints a redirect, `gateway_commands.rs:2254`).
Rewire only `permissions_shows_posture_and_gates_other_seats` to `SeatPolicyDb`.
Owner: whoever runs `02`, which is first in order. The W-plan zone is quiet now (its last
commits were iter-856–858).

**6. Deferred tests:** run order is in `02` step 4, right after the gate compiles.

**7. LIVE items:** keep the brief's provider recovery criterion (≥3 stable probe
turns). One correction: don't challenge the "S1/S2 can't be judged headlessly"
oracle until `07` step 1′'s recording backend exists. That wrapper is what
would make emitted-row checks headless. Until then the oracle is right.
Separately, the simulator's live-provider default (`01` finding 16) must be
fixed before any sweep runs near a live gateway.

**8. Ledger hygiene:** the duplicate labels (iter-838 ×2, iter-850 ×2, and also
iter-851 ×2, iter-856 ×2, iter-857 ×2, iter-858 ×2 in the last 45 commits)
are append-only. **Don't renumber.** Fix the race at the source: the
protocol's `git fetch` + label + push is not atomic across peers. The lazy
fix is a push-time check. After `git push`, if
`git log origin/main --oneline | grep -c "iter-N"` > 1, amend the *next*
commit's subject with `(dup-label iter-N)`. Don't rewrite history.

## Remaining batch plans, re-ranked

| Plan | Now |
|---|---|
| 07.2 shared services | fold (see 4) |
| 06 P0 | after this audit's 02–09 |
| 06 P1 BustPermission / ContextExhausted at 95% | still valid, after P0 |
| 06 P2 | deferred, no consumer |
| 06 P3/P4 | re-scoped to personal mode + `emp:` banks |
| 06 P5 | unchanged |
| 11D | replaced by `10` (now with evidence) |
| 11E remainder | replaced by `06` B |
| LIVE 01/04/05/09 | after `03`, `05` A and `07` 1′–2 land (they change exactly what LIVE 04/05/09 exercise) |

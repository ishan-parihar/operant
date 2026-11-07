# Verify — the harness that re-tests every finding

Nothing in this KB re-runs itself today. That is the gap. This is the harness.

## Run it

```bash
cd agentic-harness/operant/operant-testing
bash red-team/docs/verify.sh
```

Exit 0 = every finding still holds. Exit 1 = a finding changed; re-reconcile before working it.

## What each check proves

| Check | Proves | Expected at iter-1818 |
|---|---|---|
| `mcp::` unit tests | the drain-until-match fix and the iter-187 guard | 27 passed, 0 failed |
| `auto_poke` tests | upstream's bounded continuation after an early exit | pass |
| `sub_agent` named-failure test | a child that produced nothing is a named failure | pass |
| source anchors resolve | every `file:line` in `gap-register.md` still points at the same code | all resolve |
| cron rows are scheduled | G-008 is not silently re-broken | `OPEN`/`PASS`, never a regression |
| no junk dirs | G-013 | `OPEN`/`PASS`, never a regression |
| TUI overlay catalogue | G-010 — every pane registered in ONE list | `OPEN` while the cron pane is missing |

## PASS / OPEN / FAIL

- **PASS** — a *closed* finding is still closed.
- **OPEN** — a row that is still open by design. It is reported, not hidden, and it does
  NOT fail the harness: an open gap is the register's job, not a regression.
- **FAIL** — a *closed* finding regressed, or the harness itself broke. **This is the
  only thing that fails the run.**

## Why the anchor check matters

The tree moves under you. A `file:line` in the gap register is a claim about a commit; the
anchor check resolves it against the pinned commit (`758dd4c5`) and reports any row whose
anchor has drifted. Drift is not automatically a bug — it is a signal to re-read the code
before editing it.

## Adding a check

One function per check in `verify.sh`, each printing `PASS`/`FAIL` and a one-line reason.
A check that cannot fail is not a check.

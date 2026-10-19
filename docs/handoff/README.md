# Handoff — what a fresh agent reads, in order

| # | file | why |
|---|---|---|
| 1 | `AGENTS.md` | the rules, the iteration protocol, and the documentation routing table |
| 2 | `docs/handoff/STATE.md` | where the project stands right now |
| 3 | `TODO.md` | the register: autonomous mode's `Implemented` / `Pending` / `Review` |
| 4 | `docs/audit/2026-06-19-BUGS.md` (root) | open audit findings and their fix state — the repo-native issue ledger; the open queue is its `OPEN — Red-team: Holon seam, agentic-core seat and TUI (2026-10-19)` section |
| 5 | `docs/RUNBOOK.md` | exact commands, environment traps, fleet procedure |
| 6 | `docs/kb/CLOSED.md` | what was tried and rejected — do not re-litigate |
| 7 | `docs/plans/` | current pre-registered plans (dated, frozen); `docs/plans/2026-10-14-batch/00-INDEX.md` indexes the live batch |
| 8 | `docs/kb/WORKLOG.md` | what happened, unit of work by unit of work |

Legacy one-off design/parity docs (`docs/HERMES_*`, `docs/ORGANISM-*`,
`docs/*PLAN*`, `docs/*AUDIT*`, 2026-08 era) are a frozen reference shelf — cite,
never edit. Superseded docs live in `docs/archive/` with tombstones at their
original paths.

**On wake as a parallel agent:** `./scripts/genome feed --to <your-agent-id>` first —
other agents may have left you blockers, answers, or handoffs — then
`./scripts/genome doctor` for live claims and staleness. Claim before you work
(`./scripts/genome claim <item>` on a workflow board, or `./scripts/genome claim-register <row>`).

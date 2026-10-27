# Handoff — what a fresh agent reads, in order

| # | file | why |
|---|---|---|
| 1 | `AGENTS.md` | the rules, the iteration protocol, and the documentation routing table |
| 2 | `docs/handoff/STATE.md` | where the project stands right now |
| 3 | `TODO.md` | the register: autonomous mode's `Implemented` / `Pending` / `Review` |
| 4 | `docs/audit/2026-06-19-BUGS.md` (root) | open audit findings and their fix state — the repo-native issue ledger. **The ledger is organised by AUDIT BATCH, not by open/closed state, so a section heading is not a status: several `## OPEN —` sections hold no open rows at all.** The open queue is the unchecked boxes, and nothing else: `rg -n '^- \[ \] \*\*' docs/audit/2026-06-19-BUGS.md` (count: `rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md`) |
| 5 | `docs/RUNBOOK.md` | exact commands, environment traps, fleet procedure |
| 6 | `docs/kb/CLOSED.md` | what was tried and rejected — do not re-litigate |
| 7 | `docs/plans/` | current pre-registered plans (dated, frozen). **There is no single live batch** — the directory holds dated plan sets side by side, and most of their subjects are already implemented. The newest are `plan-2026-10-19-plugin-ecosystem.md`, `plan-2026-10-17-remaining-implementation-gaps.md` and `plan-2026-10-17-redteam-audit-of-remaining-gaps.md`; `docs/plans/2026-10-14-batch/` is a 2026-10-14 set whose subjects are largely done. Files under 600 bytes are TOMBSTONES, not plans — read them for the archive pointer |
| 8 | `docs/kb/WORKLOG.md` | what happened, unit of work by unit of work |
| 9 | `docs/library/README.md` | the mechanism library: one doc per architectural element (mechanism + intention) — read the agentic core group first, then the domain you came for |

Legacy one-off design/parity docs (`docs/HERMES_*`, `docs/ORGANISM-*`,
`docs/*PLAN*`, `docs/*AUDIT*`, 2026-08 era) are a frozen reference shelf — cite,
never edit. Superseded docs live in `docs/archive/` with tombstones at their
original paths.

**On wake as a parallel agent:** `./scripts/genome feed --to <your-agent-id>` first —
other agents may have left you blockers, answers, or handoffs — then
`./scripts/genome doctor` for live claims and staleness. Claim before you work
(`./scripts/genome claim <item>` on a workflow board, or `./scripts/genome claim-register <row>`).

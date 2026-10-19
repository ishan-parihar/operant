# Ownership map — every active doc, the question it owns, its status

Status: LIVE (current truth) · EVIDENCE (dated measured verdict) · PLAN-ACTIVE.

This is the map the genome's `ownership_map` role points at: for every
question about the project's documentation, exactly one file answers it.
Two files answering the same question is a defect, and this table is how
that is prevented.

| File | Status | Owns | Not for |
|---|---|---|---|
| `AGENTS.md` | LIVE | the rules, the iteration protocol, the documentation routing table | current state (that is `docs/handoff/STATE.md`) |
| `docs/handoff/STATE.md` | LIVE | where the project stands right now | what to read first (that is `docs/handoff/README.md`) |
| `docs/handoff/README.md` | LIVE | what a fresh agent reads, in order | the rules themselves |
| `TODO.md` | LIVE | the register: `Implemented` / `Pending` / `Review` | open defects (that is `BUGS.md`) |
| `BUGS.md` (root) | LIVE | open findings and their fix state | rejected proposals (that is `docs/kb/CLOSED.md`) |
| `docs/RUNBOOK.md` | LIVE | exact commands, environment traps, fleet procedure | the rules (that is `AGENTS.md`) |
| `docs/kb/CLOSED.md` | APPEND-ONLY | what was tried and rejected — do not re-litigate | live findings |
| `docs/kb/WORKLOG.md` | APPEND-ONLY | what happened, unit of work by unit of work | current state |
| `docs/kb/feed.md` | APPEND-ONLY | cross-agent messages, claims, mentions | the workflow boards |
| `docs/kb/workflows/` | LIVE | in-flight work for ONE workflow, claim-guarded | cross-cutting state |
| `docs/decisions/` | IMMUTABLE | why X is the way it is (`AD-NNN-*.md`) | what to do next |
| `docs/regressions/` | IMMUTABLE | what bit us, and the law it produced (`RG-NNN-*.md`) | decisions |
| `docs/plans/` | DATED-FROZEN | current pre-registered plans | findings (that is `docs/audit/`) |
| `docs/audit/` | DATED-FROZEN | what was audited, what was found | live findings (that is `BUGS.md`) |
| `docs/archive/` | TOMBSTONE | superseded history, with a tombstone at each original path | anything current |
| `docs/CHANGELOG.md` | APPEND-ONLY | release history, dated | the current state |
| `docs/INDEX.md` | LIVE | this map | anything else |
| `docs/RECORDS.md` | GENERATED | the decision/regression index | the records themselves |

## The frozen reference shelf

The 2026-08 one-off design and parity documents (`docs/HERMES_*`,
`docs/ORGANISM-*`, `docs/*PLAN*`, `docs/*AUDIT*`) are a historical shelf:
cite them, never edit them, never let them answer a current question.
They are the *evidence* behind decisions, not the decisions.

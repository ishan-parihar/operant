---
ID: "RG-016"
Title: "The owner ledger's own index table went stale in both directions, and its own count prose named five closed rows as open"
Status: "Active"
Date: "2026-10-29"
Description: "docs/audit/2026-06-19-BUGS.md's 'What is OPEN right now' table is the index the file itself calls 'the authority on what stands'. On 2026-10-29 it named RT-135 as an 'accepted stub' after iter-1474 closed it, RT-180 as 'OPEN (low)' after its 2026-10-26 fix, and omitted RT-136, RT-190 and RT-197 entirely - the three rows that actually were open. The count prose above it said 'nine' and then listed five rows (RT-135, RT-153, RT-154, RT-168, RT-170) as open, every one of which is closed. The checkboxes in the same file were correct the whole time: 285 closed, 4 open."
Severity: "High"
---

# RG-016: The owner ledger's own index table went stale in both directions

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

`docs/audit/2026-06-19-BUGS.md` is the repo's fix-state owner (AGENTS.md's
routing table: "open audit findings and fix state (repo-native)"; its mutability
class is "rows update, verdicts append"). Its own header names the index as the
place to look:

> The table below is the index and its State column is the authority on what
> stands.

On 2026-10-29, while updating `docs/handoff/STATE.md` under the operator's KB
directive, the table was found wrong in **both directions at once**:

| index row | said | truth |
|---|---|---|
| RT-135 | "**accepted stub** — `operant cron run <id>` marks the job and does not run it" | **CLOSED iter-1474.** `CronScheduler::run_job_now` runs it |
| RT-180 | "**OPEN (low)** — verify dies with `KeyError: 'baseline'`" | **CLOSED 2026-10-26** at its cause in `_run.py` |
| RT-136 | *(absent from the table)* | OPEN — confirmed-not-a-defect, carried as a record |
| RT-190 | *(absent)* | OPEN — owner-ruled option (c) done iter-1373, option (b) pending |
| RT-197 | *(absent)* | OPEN — the one actionable fix |

Two closed rows wearing an OPEN state, and the three genuinely-open rows not in
the table at all. The count prose immediately above the table was a third,
independent error of the same family RG-008 describes: it said "**Do not read the
word 'Nine' as the current count**", named the command that produces the count,
and then in the same sentence listed **five** rows as open — RT-135, RT-153,
RT-154, RT-168, RT-170 — **every one of which was closed** (iters 1474, 1215,
1185, 1184 and 1176 respectively).

The checkboxes in the same file were correct throughout: `rg -c '^- \[x\]'`
measured 285 and `rg -c '^- \[ \]'` measured 4.

## Root cause

The ledger's state lives in **three places per row** and only one of them is
maintained by the act of closing a defect.

1. The **checkbox** (`- [x] NotImplemented ...`) — flipped when the fix lands,
   because that is the line the fixing iteration edits.
2. The **index table's State column** — a paraphrase, written when the row was
   filed and not re-derived when the row closed.
3. The **count prose** — a third paraphrase with its own number, its own date,
   and its own list of ids.

Nothing walks back to (2) or (3) when (1) flips. `genome lint` does not catch it:
an index row naming a valid id with a `CLOSED iter-N` string is schema-valid, and
a row naming a **wrong** state is still a well-formed claim. `genome doctor`
does not catch it: the table is internally consistent with itself. It was found
only because the operator asked for the KB to be brought up to date and a human
read the table against the checkboxes.

The same shape is on the workflow boards: `remaining-gaps-1364-001` was
`in-progress`, claimed by `agent-omarchy` since 2026-10-27, for RT-179 work whose
own ledger row records the functional core as now fully reachable — a stale claim
on a board that is the collision guard, which is RG-001 exactly.

## Law

**The state of a ledger row is a claim with three copies, and closing a row
updates one of them. A paraphrase is a claim: a table column that restates a row
is supposed to be re-derived, or it is a guess with a table around it.**

Two corollaries:

- **An index that is not derived will contradict its own source.** Where the
  state can be computed from the checkboxes, compute it. Where it cannot, the
  paraphrase must carry the iteration that would invalidate it, so the next
  reader knows what to re-check.
- **Absence is a claim too.** Three of the three open rows were *missing* from a
  table whose job is to list the open rows. A reader who trusts the index
  concludes the open set is smaller than it is; a reader who counts checkboxes
  concludes the two disjoint sets disagree and must then decide which is
  authoritative, which the document has to answer for them.

## Detection

The checkboxes are the machine-readable surface and they were right:

```bash
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   # closed
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   # open  (a bare `grep -c '^- \[ \]'` also
                                                  #        counts D-5b/D-6, which are
                                                  #        headings; `rg '^- \[ \] \*\*RT'`
                                                  #        is the RT-only count)
rg -n '^- \[ \] \*\*(RT|S|D)-' docs/audit/2026-06-19-BUGS.md   # the open set, by id
```

The comparison is then a **set difference in both directions**, which is what the
2026-10-29 correction did and what a future agent should do before acting on any
row in that table:

- every index row whose State does not begin `CLOSED` must name an id in the open
  set;
- every id in the open set must have an index row.

A row in neither set, or an id in one set and not the other, is the defect. This
is deliberately runnable by hand: the failure mode is a human reading a table.

## Orthogonality

- **RG-009 / RT-163** own the **section heading** claim (a `## OPEN —` section
  holding no open row, 110 rows wearing an OPEN heading). This record is the
  **index table's State column** contradicting the checkboxes in the same file.
  Different surfaces: the heading can be right and the table wrong, which is
  exactly what was found (the nine fully-closed sections had already been
  retitled by RT-163, while the table still said "accepted stub" for RT-135).
- **RG-008** owns a **count** asserted without its derivation. Here the count
  prose *did* carry its command and *still* listed five wrong ids — a list is not
  a count, and RG-008's law does not cover a paraphrase list. Fixed here by
  making both the count and the list derived in the same place.
- **RG-010** owns a **code-surface list** going stale in both directions against
  the code. Here the source is a ledger's own checkboxes, not the code, and the
  consumer is an agent choosing work rather than a reader learning the CLI.
- **RG-001** owns a **workflow board's** status being a claim. `-001` on
  `remaining-gaps-1364` is a RG-001 instance and is cited here as a co-finding,
  not owned.

Found 2026-10-29 (iter-1480 era) while updating `docs/handoff/STATE.md` under the
operator's KB directive; corrected in the same iteration as the ledger itself.
Recon R10291944240e2.

## Revisions

- 2026-10-29: created

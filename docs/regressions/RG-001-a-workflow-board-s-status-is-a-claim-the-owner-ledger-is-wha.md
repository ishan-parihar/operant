---
ID: "RG-001"
Title: "A workflow board's status is a claim; the owner ledger is what decides the fix state"
Status: "Active"
Date: "2026-10-20"
Description: "The four workflow boards under docs/kb/workflows/ listed as open work that docs/audit/2026-06-19-BUGS.md had already recorded as fixed, and genome doctor did not catch it because the boards were internally consistent."
Severity: "Medium"
---

# RG-001: A workflow board's status is a claim; the owner ledger is what decides the fix state

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

On 2026-10-20 an agent woke to work the genome and found all four workflow boards
full of `open` rows. 39 rows across `holon-seat-redteam` (17), `tui-redteam`
(12), `org-tui-foundation` (2) and `knowledge-base-redteam` (20 findings in
prose) described defects that `docs/audit/2026-06-19-BUGS.md` had already
recorded as **fixed**, between iter-958 and iter-973 — the day before.

`./scripts/genome doctor` reported one finding, and it was not this. It reported
a stale `STATE.md` (a sha mismatch), which is real and separate. The boards
passed lint, and `wf list` showed healthy row counts. Nothing in the tooling
flagged the contradiction.

## Root cause

The board is written when a defect is *found*, and the row is only closed by an
explicit `genome done`. The fixes landed through a DIFFERENT path — the defect
sweep worked `BUGS.md` directly and pushed — so no agent ever returned to the
boards. A board row is therefore frozen at the state of the world on the day it
was minted, and it is structurally incapable of noticing that its answer changed.

The boards are also the collision guard, which makes the staleness expensive
rather than merely untidy: `genome claim` refuses a double-claim, so a fresh
claim on an already-fixed row would have looked legitimate and synchronised two
agents on the same remediation of finished work.

The closest existing check could not have caught it: the genome validates a
board's SCHEMA (valid ids, valid statuses, `--evidence` that names a file that
exists), never a row's status against the ledger that owns the truth.

## Law

**A workflow board row is a claim, not a record. The ledger that owns the fix
state — `docs/audit/2026-06-19-BUGS.md` — decides. A board may say `open` only
when the ledger agrees.**

Consequences a future session must follow:

1. Before claiming a board row, read the BUGS.md row it names and confirm that
   row is `[ ]`. If the ledger says `[x]`, the row is stale — close it, do not
   work it.
2. A fix closes its board row in the same iteration it closes its ledger row.
   A commit that ticks a BUGS.md box and leaves a board row open has left a
   claim behind that the next agent will act on.
3. When reconciling, close with `--evidence` pointing at the ledger row and a
   note naming the fixing iteration, so the board records WHY it closed rather
   than merely that it did.
4. Two rows on these boards were decisions, not fixes (the seat default for new
   org chats, and the G-1 defect). A decision closes a row too — with the AD
   that ruled, not with an iteration.

## Detection

Not automated yet, and the gap is honest: a `genome doctor` cross-check would
have to parse the item text of every open board row, extract its `BUGS` id
(`OS-1`, `GW-9`, `TUI-11`, `KB-8`), and compare it against that row's checkbox
in the ledger. The ids are present in both, so the check is writeable — a
board row whose ledger row is `[x]` while the board says `open` is a reportable
stale row. Until it exists, the manual gate is: **on wake, `genome wf list`,
then for every board with open rows, read the named ledger rows.**

Do not trust a green `doctor` on this. It checks staleness and append-only
violations, not cross-document agreement, and this defect is exactly the kind
it cannot see: internally consistent, and wrong.

## Orthogonality

No AD or RG covers a stale documentation claim, and the two closest owners solve a different problem: AD-041 corrects one overstated decompression claim in code, not a status table that never got walked back; the CLOSED ledger records rejected PROPOSALS, not a lesson about the ledger's own staleness. This is a regression law about the boards. (recon R1020184442b86: stale workflow board status open already fixed)

## Revisions

- 2026-10-20: created

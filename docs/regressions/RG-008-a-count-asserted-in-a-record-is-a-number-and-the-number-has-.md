---
ID: "RG-008"
Title: "A count asserted in a record is a number, and the number has to be re-derived from the source"
Status: "Active"
Date: "2026-10-20"
Description: "RG-001, STATE.md and the worklog all said 39 stale board rows. The true count is 49: 17 holon-seat + 12 tui-redteam + 2 org-tui-foundation + 18 knowledge-base prose rows. The error was KB-1 and KB-2, which were already fixed before the board was written, counted as reconciled."
Severity: "Low"
---

# RG-008: A count asserted in a record is a number, and the number has to be re-derived from the source

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-001 states:

> 39 rows across `holon-seat-redteam` (17), `tui-redteam` (12),
> `org-tui-foundation` (2) and `knowledge-base-redteam` (20 findings in
> prose) described defects that `docs/audit/2026-06-19-BUGS.md` had already
> recorded as **fixed**

The per-board numbers in that sentence are 17, 12, 2 and 20, and they sum to
**51**, not 39. So the sentence did not even agree with itself: it listed four
numbers and then reported a fifth.

The true count of rows that were open and needed reconciling is **49**:

| board | open rows before the reconciliation | source of truth |
|---|---|---|
| `holon-seat-redteam-2026-10-19` | 17 | `git show 697550af:…` → `rg -c '\| open \|'` |
| `tui-redteam-2026-10-19` | 12 | same |
| `org-tui-foundation` | 2 | same |
| `knowledge-base-redteam-2026-10-19` | **18** | same, counting the `\| open` rows of the prose findings table |
| **total** | **49** | |

The KB board is where the error came from. It has 20 rows in its findings
table, and I counted all 20 as reconciled. But **KB-1 and KB-2 were already
fixed (iter-958) when that board was written** — their status cells read "fixed
iter-958" even before the reconciliation. So 18 of the 20 were open and needed
closing; 2 were not mine to close.

The wrong number reached four surfaces and three commit bodies before anyone
re-derived it:

- RG-001's Symptom section (immutable once numbered — cannot be edited)
- `docs/handoff/STATE.md`, which restated it in the genome-reconciliation section
- `docs/kb/WORKLOG.md`, in the entry for the reconciliation iteration
- the commit bodies of iters 1056, 1062 and 1067, which each carried "39" or
  "39 stale board rows" as the headline figure

## Root cause

The number was **assembled rather than counted**, and the assembly step is where
the error entered.

The session had already established — with a great deal of care — that each row
had to be VERIFIED against its ledger before being closed. That verification was
per-row and per-board, and it was genuinely done: every one of the closures
names the fixing iteration and cites the BUGS.md row or the AD. The *individual*
claims were all sound.

What was never done is the step that comes after: **re-derive the total from
the source**. "49 rows were reconciled" is a different claim from "these 49
rows were each reconciled", and only the first one is arithmetically checkable
without re-reading the ledger. The figure was produced by summing per-board
numbers remembered from the work, one of which was wrong by two.

Two compounding factors:

1. **The four numbers in the sentence did not sum to the number in the
   sentence.** 17 + 12 + 2 + 20 = 51. A reader could have caught the error by
   adding up the sentence's own list. That is a check that costs nothing and
   requires no external source — and it is exactly the check that was skipped
   because the writer already "knew" the total.
2. **The figure was repeated rather than re-derived.** Once "39" existed, it was
   copied into the state doc, the worklog and three commit messages, each time
   as a given. RG-007's law — a claim asserted in one place and propagated — is
   about causes, but the same shape applies to a number: a copied figure carries
   the authority of its first appearance and none of its checking.

The irony is not lost: the session's own first record, RG-001, says "a count is
not a fact until it has been derived from the code", and the very count in its
own headline was never derived.

## Law

**A number in a record is a number, and the number is re-derived from the
source before it is written — and re-checked against its own parts before it is
copied.**

1. **Derive, do not assemble.** The count comes from the source (here: the
   pre-reconciliation board state in git), not from a sum of remembered
   per-board figures. `git show <base>:docs/kb/workflows/<board>` and count is
   two commands and cannot drift.
2. **Check the total against the parts.** If a sentence lists 17, 12, 2 and 20
   and then says 39, it is wrong on its face. Adding up the list is the cheapest
   possible check and it catches the class of error that per-item verification
   cannot: every item can be right and the total still be wrong.
3. **Count what you changed, not what you touched.** 20 KB rows were EDITED by
   the reconciliation; 18 were OPEN and needed it. "All 20 rows are closed" is
   true and is a statement about the board's end state; "20 rows were reconciled
   by me" was false. The two are different claims and only the second is a
   measure of the work.
4. **A repeated figure is not re-checked.** Every surface that carries a number
   copied from another surface should either cite the source or re-derive it.
   Three commit bodies and two documents repeating "39" is one fact repeated
   five times, not five confirmations.

## Detection

```bash
# the true reconciled count, from the pre-reconciliation state
for b in holon-seat-redteam-2026-10-19 tui-redteam-2026-10-19 org-tui-foundation; do
  printf '%s ' "$(git show 697550af:docs/kb/workflows/$b.md | rg -c '\| open \|')"
done
git show 697550af:docs/kb/workflows/knowledge-base-redteam-2026-10-19.md \
  | rg -c '^\| KB-[0-9]+ \|.*\| open'
# → 17 12 2 18  →  49
```

And the check that needs no source at all: **add up the numbers in your own
sentence.** 17 + 12 + 2 + 20 = 51, and the sentence said 39, so the sentence
was wrong before any reader was involved.

The generalisable detector, and the one this session's records earned the hard
way: **a figure that appears in more than one place should be derived in one
place and cited elsewhere.** Where a number is repeated across documents and
commit messages, the place that DERIVES it is the only one that should be
trusted, and the others should say where it came from.

## Orthogonality

RG-001 states the board-row count wrong (39 for 49) and is immutable, so it cannot be corrected in place. No existing record covers the arithmetic itself: RG-002 is about bypassing the immutability gate, RG-003 about a detector that overfits, RG-006 about asserting a tool limit from memory, RG-007 about asserting a cause without testing it. This record is about a NUMBER being wrong and the fact that nobody re-derived it across four documents and three commit bodies. (recon R1020234202ae3: arithmetic error count understated figure wrong by ten)

## Revisions

- 2026-10-20: created

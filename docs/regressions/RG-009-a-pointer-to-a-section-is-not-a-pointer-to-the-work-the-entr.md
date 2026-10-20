---
ID: "RG-009"
Title: "A pointer to a section is not a pointer to the work: the entry contract named an open-queue section with nothing open in it"
Status: "Active"
Date: "2026-10-20"
Description: "docs/handoff/README.md — the entry contract, the file AGENTS.md names as what a fresh agent reads — pointed at the 'OPEN — Red-team: Holon seam, agentic-core seat and TUI (2026-10-19)' section of BUGS.md as the open queue. That section holds zero open rows. All four open rows live at lines 703-719 inside a section named after a different defect entirely."
Severity: "High"
---

# RG-009: A pointer to a section is not a pointer to the work: the entry contract named an open-queue section with nothing open in it

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

`docs/handoff/README.md` is the file AGENTS.md names as **"what a fresh agent
reads, in order"** — the entry contract. Its row 4 read:

> `docs/audit/2026-06-19-BUGS.md` (root) | open audit findings and their fix
> state — the repo-native issue ledger; **the open queue is its `OPEN —
> Red-team: Holon seam, agentic-core seat and TUI (2026-10-19)` section**

That section spans lines 176–215. It contains **zero open rows** — every row in
it is `[x]`. The four open rows in the entire ledger live at lines 703–719, in
a section headed:

> `## OPEN — Duplicate tool rows mid-turn (2026-10-14 batch, plan 01)`

a heading about a completely unrelated defect, because the ledger is organised
by **audit batch**, not by open/closed state. Its own `## Index of items` lists
section names, and several sections headed `## OPEN —` contain no open rows at
all.

So the first thing a fresh agent is told to read for the open work points at an
empty section, and the section it would need to find the real work in is named
after a defect nobody is currently chasing.

The same row had a second, quieter failure: it named a **location** where the
reliable thing is a **query**. The open queue is not a place in this ledger. It
is the unchecked boxes, and the only way to see them is
`rg -n '^- \[ \] \*\*' docs/audit/2026-06-19-BUGS.md`.

Row 7 of the same table had the matching defect: it said
`docs/plans/2026-10-14-batch/00-INDEX.md` "indexes the live batch". That batch
has no supersession marker, but its subjects are largely implemented (reasoning
effort, rewind, context management, visibility toggles are all in
`TODO.md`'s `Implemented`), and there are newer plan sets — 2026-10-17 and
2026-10-19. There is no single live batch. A third file, `docs/kb/CLOSED.md`,
still cited the dead `kb status` command.

## Root cause

**A heading is a label; a pointer to a heading is a claim about what is inside
it. Neither is a status, and both were treated as one.**

The ledger grew by appending audit batches, each with a heading describing the
batch. Early headings are still `## OPEN —` from the day they were written;
closing a row inside one leaves the heading lying. Over time the headings and
the row states drifted apart, and the drift is invisible to every tool here:
`genome doctor` counts rows, `genome lint` validates the docs' schema, and
nothing checks that a named section still contains what it was named for.

The entry contract is where the two met. It was written when
`OPEN — Red-team: Holon seam, agentic-core seat and TUI (2026-10-19)` DID hold
the open queue — that section was the red-team's output and the sweep had not
moved past it. As the sweep closed those rows one by one, the section emptied,
and the pointer quietly became wrong. No event marked the moment it broke, so
no commit says "the open queue moved".

That is the same failure RG-001 opened this session with, one abstraction up:
**a status that was true when written, kept in a place that outlived it, and
read as current.** RG-001's boards are the same shape — a table that says `open`
because the row was written when it was open.

The deeper cause is that both documents answer a question the underlying
artefact does not answer in that form. The ledger does not have an open queue;
it has a sequence of batches with mixed states. The plans directory does not
have a live batch; it has dated sets side by side. Pointing at either is a
guess dressed as a citation, and the guess was made once, long ago, and never
re-checked.

## Law

**Point at a query, not at a location, whenever the thing moves.** A location is
a claim that its contents are still there; a query is a claim that the contents
are wherever they are.

1. **When a document names a section as "the" place to look, that is a claim
   about its CONTENTS.** Before relying on it, check the contents: for a ledger,
   `rg -c '^- \[ \]' <file>` inside the named section. If the number is zero, the
   pointer is stale and the work is somewhere else.
2. **A section heading is not a status.** In a ledger organised by batch,
   `## OPEN —` means "this batch was opened", not "these rows are open". The
   row's checkbox is the status; the heading is a label.
3. **Prefer a command to a path in every entry contract.** The entry contract is
   the one document a fresh agent follows literally, so a path there is
   followed literally and a wrong path costs the agent its whole first hour.
   `rg -n '^- \[ \] \*\*' docs/audit/2026-06-19-BUGS.md` costs the reader
   nothing and cannot go stale.
4. **When a document's summary of a moving thing is found wrong, check the OTHER
   summaries in the same document.** One stale pointer in the entry contract
   almost never travels alone: row 7 named a "live batch" that is not live, and
   a third file cited a dead command. They accumulate the same way — written
   once, re-read as current.
5. **The entry contract is the highest-leverage file in the repo and is treated
   as the least.** It is 8 rows. It should be the most carefully verified
   document there, because it is the only one a reader is guaranteed to follow.

## Detection

Cheap, and it is the check this session's records keep earning:

```bash
# does the section the entry contract names still hold open rows?
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md          # 4 total
sed -n '176,215p' docs/audit/2026-06-19-BUGS.md | rg -c '^- \[ \]'   # 0 in the named section
```

The generalisable form, and the check that would have caught all three defects
in one pass: **for every "the X is at <path>" claim in an entry document, open
the path and confirm X is there.** Not that the path exists — that X is there.
`docs/plans/2026-10-14-batch/00-INDEX.md` exists, is well-formed, and indexes a
batch whose work is done. `docs/kb/CLOSED.md` exists and cites a command that
does not exist. Existence was never the thing to check.

## Orthogonality

RG-008 is about a NUMBER being wrong; this is about a POINTER being wrong — a document naming a location that no longer holds what it names. RG-001 is stale board statuses, RG-002 the immutability bypass, RG-003 an overfit detector, RG-006 a tool limit asserted from memory, RG-007 a cause asserted without testing. None covers the navigation failure: the file a fresh agent reads FIRST sent it to a section with zero open rows, in a ledger organised by audit batch where a heading is not a status. (recon R102023493407f: entry contract points at a section open queue misdirected section heading is not a status)

## Revisions

- 2026-10-20: created

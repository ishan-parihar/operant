---
ID: "RG-004"
Title: "A numbered record that is half wrong has no sanctioned correction: supersede applies to the whole record or nothing"
Status: "Retired"
Date: "2026-10-20"
Description: "RG-003 said it 'supersedes RG-002's Detection section only'. That is not a real operation: RG-002 is still Active with its Detection text byte-identical, and genome archive --section works only for living plans (W011), not numbered records. The genome can supersede a whole record or nothing."
Severity: "Medium"
---

# RG-004: A numbered record that is half wrong has no sanctioned correction: supersede applies to the whole record or nothing

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-003 states, in its Law section:

> This record supersedes **RG-002's Detection section only**. RG-002's Law — that
> numbered records are immutable once numbered, that the CLI enforces it, and that
> `rec supersede` is the only sanctioned correction — is correct and stands.

That sentence describes an operation that **does not exist**.

- RG-002 is still `Status: "Active"` in its front matter.
- RG-002's Detection text is byte-identical to what it was when RG-003 was
  minted.
- `genome rec supersede` takes a record `id`, sets `Status:` to `Superseded` and
  writes a `SupersededBy` field. It has no `--section` argument, and no concept of
  a partially retired record.
- `genome archive --section` does exist — and applies only to **living plans**
  (it is the remedy for the W011 lint finding, "section '{h}' is superseded"). It
  refuses a numbered record's path outright.

So the genome can supersede a whole record, or nothing. "Section only" was a
prose gesture at an operation the tool does not have, written by the same agent
that had just finished writing a record about not asserting operations that do
not happen.

The underlying situation is genuinely awkward, and that is the part worth
recording: **RG-002 is half wrong.** Its Law — immutable once numbered, the CLI
enforces it, `rec supersede` is the only sanctioned correction — is correct and
load-bearing. Its Detection section is overbroad and would fire on the genome's
own supersession flow. There is no sanctioned way to fix the one without
retiring the other.

## Root cause

The doctrine models a record as a single atomic claim with a single verdict. That
is right for a decision ("we do X") and mostly right for a regression law, but a
record is a document with sections, and **a document's sections can have different
fates**. The immutability rule was written for the claim, and the moment a record
carries two claims of unequal standing, the rule has no answer.

The pragmatic failure is that the rule's ONLY tool is all-or-nothing, so the
operator of the rule is forced into a worse option by the available set: leave
known-wrong text standing, or destroy correct law alongside it. Both are worse
than a partial correction, and neither is sanctioned.

RG-003 chose the first option in prose (calling it a supersession) instead of in
fact. That is the specific defect: it named an outcome rather than achieving one,
which is the whole class of error this session's records exist to stop.

## Law

**Supersession is whole-record. There is no section-level correction for a
numbered record, and a sentence claiming one is a claim that no tool backed.**

Consequences an operator must follow:

1. **Never write that a record's section is superseded.** Either supersede the
   whole record (`genome rec supersede <id> --by <new> --reason`) or state plainly
   that the section is wrong and remains uncorrected. Prose cannot retract.
2. When a record is half right, **supersede the whole record and carry the still-
   correct parts into the new record's body**, quoting them and naming RG-002's
   Law as still in force. The new record then holds both claims, one corrected and
   one reaffirmed, and the audit trail is real.
3. Say what actually happened. If the correct option costs too much to take and
   the wrong text is left standing, the honest record is the one that names the
   wrong text, why it was not corrected, and which reader should distrust it. A
   document that says the problem is solved when it is not is the defect RG-001
   opened with.
4. **A new record is the only correction.** If you want a section of a numbered
   record changed, mint a record that says so and link both ways; do not edit the
   old record, and do not describe an edit that did not occur.

## Detection

The check is cheap and it is the one that found this:

```bash
genome rec show <the record you claim to have superseded> | rg 'Status|SupersededBy'
```

If the record still reads `Active` with no `SupersededBy`, the supersession did
not happen, whatever prose says. Pair it with the RG-003 gate — a record whose
content changes exceed its dated revision rows — to separate a real transition
from a claimed one.

The generalisable form, and the one this session needed four passes to reach:
**a sentence describing an effect is not the effect.** Before writing "X is now
superseded/corrected/fixed", run the command that would show it. RG-003 asserted
a correction; `rec show RG-002` printed `Active`. That is the whole difference
between a record and a claim, which was RG-001's law on its first day.

## Orthogonality

RG-001 is about stale board statuses, RG-002 about violating record immutability, and RG-003 about a detector that overfits. None covers the structural gap this record names: when PART of a numbered record is wrong, the genome's only sanctioned correction (rec supersede) retires the entire record including the parts that are right, so the operator must choose between leaving wrong text standing and destroying correct law. The recon confirms it: the closest sections are PARTIALLY LANDED findings about code, not about the record lifecycle. (recon R1020205643336: record section partially wrong cannot supersede)

## Revisions

- 2026-10-20: created
- 2026-10-20: retired — Its central claim is false and its title and front-matter Description assert it: RG-004 said no section-level supersession exists for a numbered record and that the genome can supersede a whole record or nothing. That is wrong.  works on numbered records, removes the section, preserves it in docs/archive/ with provenance, and writes the INDEX row. The claim was disproved by running the very command it declared impossible, which also had the side effect of performing the correct correction. Superseded by RG-005.

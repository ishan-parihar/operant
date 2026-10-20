---
ID: "RG-006"
Title: "A tool limit asserted from memory rather than from the CLI is a claim, and the experiment that tests it may be the fix"
Status: "Active"
Date: "2026-10-20"
Description: "RG-004 declared that genome archive --section works only for living plans and that a numbered record can be superseded whole or not at all. Running the command proved the opposite and performed RG-003's correction as a side effect. RG-004 was retired; the section-level mechanism is real and is the sanctioned way to correct one wrong section."
Severity: "Medium"
---

# RG-006: A tool limit asserted from memory rather than from the CLI is a claim, and the experiment that tests it may be the fix

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-004 was minted with this claim in its title, its front-matter `Description`,
and its body:

> "…has no sanctioned correction: supersede applies to the whole record or
> nothing" — `genome archive --section` "works only for living plans (W011), not
> numbered records" and "refuses a numbered record's path outright".

That claim was written from **memory and inference**, not from the CLI. It was
wrong, and it was wrong about a tool whose help text was one command away:

```bash
genome archive --help
# usage: genome archive [-h] --reason REASON [--section SECTION] ...
```

The `--section` flag is right there, with no statement restricting it to plans.

Then the claim was "tested" — by running the command it declared impossible:

```bash
genome archive docs/regressions/RG-002-....md --section Detection --reason "<why>"
```

It **worked**. It removed RG-002's overbroad Detection section, preserved it in
`docs/archive/RG-002-....--detection.md` with a provenance header naming the
source file, the line range and the reason, and appended the INDEX row.

Two things follow, and both matter more than the wrong claim:

1. **The experiment performed RG-003's correction.** RG-003 had correctly said
   "this record supersedes RG-002's Detection section only". RG-004 doubted it,
   "tested" it, and the test did it. RG-003 was right all along.
2. **The experiment was run against the live numbered record.** It had a real,
   irreversible-in-place side effect. The result happened to be the desired one,
   so the outcome is good — but nothing guaranteed that, and a wrong claim tested
   on the only copy of the subject can destroy the thing it set out to check.

RG-004 was retired through the sanctioned path (`genome rec retire RG-004
--reason`), so the wrong claim is out of force and its removal is on the record.

## Root cause

Three separable failures, one of which is the generalisable one:

1. **A capability was asserted without being read.** The agent reasoned about what
   the tool "surely" did from one observed warning message (W011, which names
   living plans) and generalised a restriction that was never there. The
   restriction was invented to explain why an adjacent design felt wrong, not
   observed.
2. **A test was run on the live subject instead of a copy.** The claim was about
   the genome's section-archival of a numbered record. There was a numbered
   record available — RG-002 — and the natural "test subject" was it. That turns
   a read-only check into a mutation of the record under discussion.
3. **The record series had no shrink-wrap, and the last record inherited the
   failure.** RG-001 opened by saying "a claim in a document is worth exactly what
   the search behind it covered". Four records later the same class recurred,
   because the law was stated about documents and not applied to the agent
   asserting tool capabilities.

## Law

**Read `--help` before you assert what a tool cannot do.** A capability stated
from memory is a claim, and RG-004 is what one looks like when it is wrong.

- **Never assert a tool's limit without having read its help or its source** in
  this session. If you have not, say "I believe" and then check — the check is
  seconds.
- **Test destructive claims on a copy.** `cp` the subject to a scratch path, run
  the command there, and only apply it to the real record when the behaviour is
  confirmed. A command that mutates the subject it tests is an experiment with no
  undo.
- **`genome archive <record> --section <Section> --reason <why>` IS the
  sanctioned way to correct one wrong section of a record whose other sections
  are correct.** It removes the section, preserves it in `docs/archive/` with
  provenance, and writes the INDEX row. The lint then requires the archived
  *filename* to appear in the INDEX text (W007 is a substring match on the
  filename, not on the source path — name the file, not just its origin).
- **Section-level correction is a real third option**, and it is the right one far
  more often than whole-record supersession: a record whose Law is correct and
  whose Detection is wrong does not need to be retired. RG-002 stands with its
  Law intact and its Detection archived.
- **An experiment that succeeds changes the world.** Before running a command to
  check a claim, ask what it does if the claim is false. If the answer is "it
  performs the thing", you are not testing — you are deciding.

## Detection

The check that would have prevented this one:

```bash
genome archive --help | rg -n 'section'
```

Costs one second and would have shown `[--section SECTION]`. The general form,
and the one this session needed five records to reach: **a reason for believing
a limit is not evidence of the limit.** RG-004's reason was that W011 mentions
plans; W011 is a lint finding about plans, not a scoping statement about
`archive`. The evidence and the conclusion did not connect, and nothing checked
the join.

## Orthogonality

RG-001 is stale board statuses, RG-002 the immutability bypass, RG-003 an overfit detector, RG-004 (retired, its claim false) asserted a nonexistent limit. None covers the operational failure that produced RG-004: asserting a CLI limitation without reading the CLI, then 'testing' it against a live numbered record where the test itself performs the correction. That is a distinct defect with a distinct fix — read --help, and copy before you experiment. (recon R1020211916083: experiment on live record side effect asserted tool limit)

## Revisions

- 2026-10-20: created

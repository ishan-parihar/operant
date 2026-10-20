---
ID: "RG-002"
Title: "A numbered record is immutable once numbered: the CLI enforces it and a raw file edit is the way past"
Status: "Active"
Date: "2026-10-20"
Description: "RG-001 was amended in place with a raw file edit two commits after it was minted, bypassing the genome CLI's records.mode=immutable gate. The genome refused a later 'rec amend'; only 'rec supersede' is sanctioned. lint cannot detect an in-place edit because E103/E106 check supersession, not mutation."
Severity: "High"
---

# RG-002: A numbered record is immutable once numbered: the CLI enforces it and a raw file edit is the way past

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-001 was minted in iter-1056 with the header line every numbered record in this
repo carries:

> Immutable once numbered: a correction is a new record that supersedes this one.

Two commits later (iter-1062) the same agent edited RG-001 **in place** with a
raw text edit, adding a new symptom sub-section and a fifth law. No revision row,
no reason, no date. Nothing in `git log` distinguishes the amended text from the
original unless you already suspect it.

The violation was found by an independent validation pass that grepped the
record's git history — `git log --follow -- docs/regressions/RG-001-*.md` —
rather than by the genome tooling.

`./scripts/genome lint` was clean, and reported clean again afterwards, with
`--advisory` as well. `./scripts/genome rec amend RG-001 ...` was later run and
**refused**: `genome: records.mode is immutable — correct a record with
`genome rec supersede`, or set mode = "amendable"`. So the gate was working. It
had already been walked around.

## Root cause

Three things were true at once, and each was individually reasonable:

1. The genome module's default `records.mode` is `immutable` and this repo's
   `genome.toml` never sets it, so the gate is on by default — nobody chose it
   here, and nobody named it in AGENTS.md either. The routing table says
   "immutable once numbered" without saying that the CLI enforces it and how.
2. The agent authoring the correction was focused on the CONTENT being right
   (the citation defect was real and worth recording) and treated the file as a
   document to update rather than a numbered record to supersede.
3. The tooling cannot see an in-place edit at all. `lint`'s E103 flags a record
   "that is {status} but still active" and E106 flags two-sided supersession
   problems. Both fire on SUPERSESSION, never on MUTATION, so an edited record
   looks exactly like a clean one.

The compounding detail, and the reason this is High rather than Medium: the
subject of RG-001 is that *a claim in a document can be wrong while every
automated check passes.* Its author then committed that exact failure mode
against RG-001 itself, and the same checks passed again.

## Law

**A record under `docs/decisions/` or `docs/regressions/` is immutable once
numbered. The genome CLI is the authority on correcting one, and it refuses you
by design.**

- Correct a record with `genome rec supersede <id> --by <new-id> --reason`, and
  write the correction as the new record.
- `genome rec amend` is refused while `records.mode` is `immutable`, which it is
  by default and which this repo does not override. The refusal is correct.
- **Never** `edit`, `replace` or `write` a file under `docs/decisions/` or
  `docs/regressions/`. If you have just reached for one of those tools on a
  numbered record, that impulse is the signal: you are correcting a numbered
  claim, and the claim belongs in a new numbered record. This record exists
  because the author did not read the signal.
- The precedent is already in the repo: AD-041 carries a section headed
  "Why this is a new record and not an edit to AD-039", and states the reason a
  correction is recorded rather than applied — "the honest thing is to record
  the correction rather than to quietly make the old record true by editing
  it". Follow that shape.
- **An author's familiarity is not an exemption.** The agent that wrote RG-001
  wrote the rule, then broke it within two commits. The rule needs a gate, and
  the gate exists — it just has to be used instead of bypassed.

## Detection

Cheap and effective, and it was the finder:

```bash
git log --follow --oneline -- docs/regressions/ docs/decisions/
```

A record touched by more than one commit was amended in place. Every numbered
record should appear exactly once, in the commit that minted it.

`genome lint` cannot do this today (E103/E106 are supersession checks, not
mutation checks), and that gap is the same shape as RG-001's: internally
consistent output that cannot see its own subject. Until the CLI grows a
mutation check, the manual gate above is the only one, which means the
bypass is detectable — but only by someone who thinks to look.

This record is the correction RG-001 needed: it is a new record, minted through
the sanctioned path, superseding nothing because RG-001's IDENTITY stands and
RG-001's law was sound. The two additions RG-001 needed (the citation defect and
the fifth law) are recorded in the iter-1062 body and stay in RG-001's amended
text, with this record naming that they were added without a revision trail.

## Orthogonality

RG-001 covers a stale board STATUS and closes by superseding a claim about a doc. This record is about the numbered-record class itself — that the genome CLI's records.mode=immutable is the real gate, that it can be bypassed with any text editor, and that its own author bypassed it. AD-041 is the precedent proving the rule (it records WHY it is a new record and not an edit) but it is not a law, and no RG records the bypass. (recon R1020200044884: immutable record amended in place violation raw edit)

## Revisions

- 2026-10-20: created

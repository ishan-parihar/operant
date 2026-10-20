---
ID: "RG-003"
Title: "A record touched twice is not evidence of a bypass: the discriminator is the revision trail, not the commit count"
Status: "Active"
Date: "2026-10-20"
Description: "RG-002's detection law said a record touched by more than one commit was amended in place. That is false: AD-037 is touched by three commits through the genome's sanctioned supersession flow. The discriminator is the dated Revisions row, not the commit count."
Severity: "Medium"
---

# RG-003: A record touched twice is not evidence of a bypass: the discriminator is the revision trail, not the commit count

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

RG-002 minted a detection law of the form:

> A record touched by more than one commit was amended in place. Every numbered
> record should appear exactly once, in the commit that minted it.

The very next validation pass ran that law across the whole records corpus and
found four records touched more than once. Applying RG-002's rule literally,
all four are bypasses. **They are not.**

| record | commits | dated revision rows? | what actually happened |
|---|---|---|---|
| AD-037 | 3 | **no — none at all** | superseded by AD-038 through the genome tooling: it carries the `# SUPERSEDED by AD-038` banner and `Status:` / `SupersededBy:` front-matter, but **no `## Revisions` section**. The sanitised transition, but no revision trail |
| AD-027 | 2 | yes — two dated rows | an edit whose reason is stated in a dated row. Partial: it has the trail, but no `## Amendment` heading, so the reason is recorded and the mechanism is not |
| AD-040 | 2 | **no — none at all** | a raw content edit: no banner, no front-matter change, no revisions. The same bypass class as RG-001 |
| RG-001 | 2 | **no — none at all** | the bypass this whole series records |

**The gate's first run got the AD-037 row wrong, which is the finding worth
keeping.** The table above first claimed AD-037 had "three dated rows" — read
off the supersession banner, which *looks* like a recorded transition, without
checking whether the row existed. Running the gate showed `commits=3
revision-rows=0`, and the claim was corrected against the gate rather than
against the impression.

The banner is tool output, not author diligence. `genome rec supersede` writes
both the banner and a revision row; AD-037 has the first and not the second, so
even a sanctioned transition can arrive without a trail. **Check the row, not
the banner.**

So the law's answer to "was this a bypass?" is "more than one commit", and the
truth is that the commit count carries almost no signal. A gate this shape does
not just miss its target — it reports the genome tooling's own correct behaviour
as a defect, which is how a real detector gets switched off.

## Root cause

The law was generalised from ONE example (RG-001, touched twice, no revision
row) without a second case to test the boundary against. The distinguishing
feature that was visible in that single example — the absent revision row — was
never used; the incidental feature (the commit count) was promoted to the rule
instead. That is a single-example overfit, and it produces a rule that is
confidently wrong on the first case outside the training set.

The genome CLI had the discriminator available the whole time, in its own
source: `rec amend` calls `add_revision()`, and `rec supersede/merge/retire`
calls `add_revision()` with the transition line. Every sanctioned touch writes
a dated `## Revisions` row. A bypass does not, because a bypass is an edit.

## Law

**The discriminator is the revision trail, not the commit count.**

A numbered record touched by more than one commit is **not** evidence of a
bypass. To find a bypass:

1. `git log --oneline -- <the record>` — find records touched more than once.
   This is a CANDIDATE LIST, nothing more.
2. `rg '^## Revisions' <the record>` and read the rows. **Every row
dated, and one per content change.**
3. A content change with **no corresponding dated revision row** is the finding.
   RG-001's two commits and one row ("created") is the finding; AD-027's two
   commits and two dated rows is not.
4. Read the `# SUPERSEDED by <id>` banner and the `Status:` / `SupersededBy:`
   front-matter as a SECOND-level signal only. The banner is tool output, not
   author diligence, and AD-037 proves a sanctioned transition can arrive with
   the banner and no revision row at all. A banner without a row is a
   supersession that left no trail — worth reporting, not worth trusting.

**A count is never a law.** One positive example does not establish a rule; it
establishes a hypothesis that needs a negative case to separate the signal from
the incidental feature. This record exists because the rule was written from a
single example and the first out-of-sample case broke it.

This record supersedes **RG-002's Detection section only**. RG-002's Law — that
numbered records are immutable once numbered, that the CLI enforces it, and that
`rec supersede` is the only sanctioned correction — is correct and stands.

## Detection

The corrected gate, run over the whole corpus:

```bash
for f in docs/decisions/AD-*.md docs/regressions/RG-*.md; do
  n=$(git log --oneline -- "$f" | wc -l)
  rows=$(rg -c '^- 20[0-9]{2}-[0-9]{2}-[0-9]{2}:' "$f" 2>/dev/null || echo 0)
  if [ "$n" -gt 1 ]; then echo "$f  commits=$n  revision-rows=$rows"; fi
done
```

A record whose content changes exceed its dated revision rows is the finding.
Run before relying on RG-002, and treat any law stated as a bare count with
suspicion — including this one, until it is checked against a case where the
answer is no.

## Orthogonality

RG-002 states the rule correctly (immutable once numbered, the CLI enforces it) but its DETECTION law is overbroad and would fire on the genome's own sanctioned supersession flow. RG-001 is about stale board statuses and RG-002 about the bypass itself; neither covers a detector that over-fires, which is a different defect: a gate that cries wolf on legitimate behaviour gets switched off. (recon R1020202816c42: detection law overbroad false positive sanctioned supersession)

## Revisions

- 2026-10-20: created

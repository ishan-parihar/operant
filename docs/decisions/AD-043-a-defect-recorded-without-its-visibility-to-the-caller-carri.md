---
ID: "AD-043"
Title: "A defect recorded without its visibility to the caller carries the wrong severity, and the fix that ships is the fix for the severity that was written"
Status: "Active"
Date: "2026-10-31"
Description: "A defect recorded without its visibility to the caller carries the wrong severity, and the fix that ships is the fix for the severity that was written"
---

# AD-043: A defect recorded without its visibility to the caller carries the wrong severity, and the fix that ships is the fix for the severity that was written

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The 2026-10-31 codex deepen audit recorded defect D1 on `tools/patch_tool.rs`: a `find` string matching
more than once is rewritten in every position, and the audit called it "silent" — a model asking for
one edit can rewrite five. That framing drove the plan's fix: refuse a multi-occurrence find.

Re-measuring before execution showed the behaviour is **not silent**. `execute` computes
`exact_count = content.matches(&args.find).count()` and returns it to the caller as
`"replacements"` in the tool result (`patch_tool.rs:172-176`, `:199-207`). The model sees
`replacements: 3` and can tell it rewrote three things.

That does not make the behaviour *correct* — codex's contract is a refusal, and a model that asked
for one edit and got three should be told, not told after the fact. But it changes what the defect
**is**: not "silent corruption" but "a contract with no occurrence guard, whose outcome is
reported".

## Decision

When a defect is recorded, the **visibility of its consequence to the party who would act** is part
of the record, and it is derived from the call site rather than from the shape of the code. A write
that lands in five places is a high-severity defect when nothing reports it and a medium-severity
design gap when the tool result already carries the count.

The fix that ships is derived from the severity that is *recorded*. An audit that overstates
visibility produces an urgent fix; one that understates it produces a silent bug. Both come from the
same line of the record, which is why the derivation belongs next to the claim.

D1 keeps its fix (refuse the ambiguous find, take an explicit occurrence index) and keeps its
`med` severity in the audit, but the audit's wording now carries a headnote pointing at this
record, and the regression test asserts the *refusal message names the count* rather than asserting
the old behaviour was silent corruption.

## Consequences

The plan's D1 row is a design-contract row, not a corruption row. Its acceptance criterion changed
from "must not rewrite every occurrence" to "must refuse an ambiguous find and name the count",
which is a smaller, better-targeted test. No other defect in the audit is affected — D2 through D5
were each re-derived at their call sites during the same pass and stand.

## Orthogonality

AD-022 covers degraded-memory reporting and AD-018 covers tool-availability reporting; both are about a feature that reports. This is about the audit step: a defect whose consequence the caller already observes is a different row from one the caller cannot see, and the audit carried the higher severity into the plan without re-deriving it from the call site. (recon R1031204029cbb: hook event count correction plan measurement)

## Revisions

- 2026-10-31: created

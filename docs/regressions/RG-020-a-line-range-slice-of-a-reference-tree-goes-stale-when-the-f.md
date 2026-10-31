---
ID: "RG-020"
Title: "A line-range slice of a reference tree goes stale when the file grows, and the count in the heading is the number that gets quoted"
Status: "Active"
Date: "2026-10-31"
Description: "A line-range slice of a reference tree goes stale when the file grows, and the count in the heading is the number that gets quoted"
Severity: "High"
---

# RG-020: A line-range slice of a reference tree goes stale when the file grows, and the count in the heading is the number that gets quoted

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

The frozen plan `docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md`
asserted that codex's hook discovery was a "~60 LOC" unit at
`hooks/src/engine/discovery.rs:390-451`. Re-measuring with `wc -l` on the same path in the same
reference tree returned **1,963 lines**. The plan also asserted "4 of 14 hook events" where the
reference's `HookEventNameWire` enum carries **11** variants.

Both numbers were produced from an outline/structural view of a file in a tree that is not this repo.
Nothing re-checks them: the reference tree is a sibling checkout, it is not in any test, and no gate
in this repo reads it.

## Root cause

A line range is a *snapshot*. A `wc -l` is a *derivation*. The plan recorded the snapshot and quoted
it as if it were derived. The specific failure is that the numbers enter a plan as inputs to a cost
estimate ("~60 LOC", "Row 7 is S-sized"), and a cost estimate built on a snapshot carries no
signal when the file moves.

## Law

**A LOC figure for a file in a reference tree is a derivation or it is not a number.** It is
derived with `wc -l <path>` in that tree at the commit named in the audit header, and the command
and its output are recorded, not just the result. A line range quoted without the total is a
snapshot and must be labelled as one. An enum variant count is derived by reading the enum, never
recalled from the shape of a previous conversation.

This is the third direction of the same family: RG-008 covers counts asserted in operant's own
docs; RG-010 covers surface lists that go stale in both directions; this covers the reference tree,
where nothing forces a re-derivation at all.

## Detection

Before a number enters a plan: `wc -l <named path in the reference tree>` and, for any count, read
the enum or the table directly. If the reference tree is not available, say the figure is an
estimate rather than a measurement — the plan's own honesty section already carries this convention
and it was not applied to the two rows that needed it.

## Orthogonality

RG-008 governs counts asserted in operant's own docs and RG-010 governs surface lists. This is the third direction: a LOC figure for a REFERENCE tree's file, taken from an outline view rather than wc -l on the file, which had no staleness signal at all because the reference tree is not this repo and nothing re-checks it. (recon R1031204029cbb: hook event count correction plan measurement)

## Revisions

- 2026-10-31: created

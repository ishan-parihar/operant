---
ID: "AD-029"
Title: "Context management stays on the synchronous summarize rung; agent-driven reduction is not built"
Status: "Active"
Date: "2026-10-19"
Description: "The magic-context review's agent-driven reduction and background historian are not built now; the synchronous summarize rung stays"
---

# AD-029: Context management stays on the synchronous summarize rung; agent-driven reduction is not built

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The TODO Review list carried the magic-context review: an agent-driven reduction tool (`ctx_reduce`) and a background historian in place of the synchronous summarize rung. The review named these as larger core changes.

## Decision

Not built in this iteration. No open BUGS row, and no observed failure, depends on them. The synchronous summarize rung stays as it is. A future change would start as a new record with a design, so the choice can be reversed on evidence.

This is the one review item the iteration declined to build. It is flagged in the final report so the owner can overrule it.

## Consequences

- Long sessions still summarize synchronously when the threshold is reached. The behaviour is unchanged.
- There is no background historian and no agent-driven reduction tool.

## Orthogonality

The context-bust permission work (plan 06 P1, closed in BUGS) is a separate change and is not affected.

## Revisions

- 2026-10-19: created


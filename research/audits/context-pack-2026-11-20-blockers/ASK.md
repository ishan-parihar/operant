# Ask — the open decision (2026-11-20)

## The question for this round

**Round 2 returned 12 findings; all 12 are closed. The residual cleanup (dead code, doc
drift, stale checkboxes, clippy) is also closed. What is the NEXT blocker?**

The operator has asked for a third round because the project has reached a local
plateau: the architecture review closed its findings, the dead-code audit found no dead
production code, the clippy gate is green, and the 7-crate workspace is stable. The
question is whether the NEXT phase's blockers are:

1. **Bottlenecks** — what is the binding constraint on the iteration model? (compute,
   deploy, verification, peer coordination)
2. **Redundancies** — what is duplicated across the 7 crates that should be unified?
3. **Fragmentations** — where does the system have two halves that should be one?
4. **Integration gaps** — what is wired but not reachable, or reachable but not wired?

## What the operator will do with the answer

The operator (the user) tests from `origin/main` after every push. A finding that
names a concrete blocker with a `file:line` and a proposed fix is actionable in the
next session. A finding that is a judgment call without evidence is not.

## What is NOT the question

- The architecture review (rounds 1 and 2) already covered agent-loop, gateway, memory,
  context, config, tools, skills, plugins, autonomous, TUI, org, security. Do not
  re-litigate those surfaces.
- The dead-code audit (iter-2704) already enumerated every `dead_code` site. Do not
  re-enumerate them.
- TheHydrogen campaign (iters 2671-2692) already closed F1-F12. Do not re-open them.

## What would make this round a delta, not a re-derivation

A finding is a DELTA finding only if it names something that:
- landed since round 2's pack was cut (`df1ea4c0..origin/main`), OR
- was not covered by rounds 1 or 2's surfaces, OR
- contradicts a claim in the STATE section (stale, wrong, or unfalsifiable).

If the reviewer finds that the system is in a steady state with no new blockers, say
so — that is a valid finding. The operator will then shift from architecture work to
feature work.
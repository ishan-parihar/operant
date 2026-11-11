# Intent Table — agentic-core operationality repair (round 22)

| Field | Content |
|---|---|
| Deliverable | A verified, deployable agentic-core repair that removes the live context-window incident and closes confirmed high-severity cross-feature gaps. |
| Verbs | investigate; fix; test; deploy; red-team; document. |
| Explicit constraints | C1: investigate all implementation and feature gaps in `agentic-core`. C2: specifically fix `The conversation no longer fits ... Run /compact`. C3: use `$ponytail` and `/home/ishanp/.agents/skills/rust-best-practices/SKILL.md`. C4: autonomous recursive iterative loop. C5: comprehensive realtime testing across the full feature surface. C6: use architecture documentation to map testing/refinement for every agentic-architecture feature. |
| Implicit constraints | C7: follow `AGENTS.md` iteration, staging, push, release and deployment rules. C8: preserve peer WIP. C9: smallest complete root-cause fix. C10: typed errors, no production unwrap/expect, strict clippy. C11: architecture docs must match shipped behavior. |
| Scope | IN: `operant-core`, the TUI/gateway adapters that expose its contracts, focused CLI/core regressions, deployment and runtime verification. OUT: unrelated peer WIP; speculative product features; dormant public APIs already explicitly ruled out by RT-275. |
| Conflicts | C1 vs C9: breadth wins only for confirmed defects with a bounded root-cause fix; unresolved product decisions are named, not guessed. C5 vs C7: safe local fake-provider/live-loop tests satisfy realtime coverage; credentialed production calls are separate final evidence. |
| Assumptions | A1: `agent.context_window = 4242` in the live file came from the operant-cli test suite; verified by exact fixture value, test timeline, file mtime, and the concurrent global config-path race. A2: the intended live window is 1,000,000; recorded in the prior kernel round and prior gateway logs. A3: the protected prefix floor must be detected dynamically because it varies with skills/capabilities; a static floor only rejects algebraically impossible values. |

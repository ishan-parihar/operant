# Workflow: remaining-gaps-1364

`genome wf add remaining-gaps-1364 "item"`, `genome claim <id>`, `genome done <id> done --evidence <path>`.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| remaining-gaps-1364-001 | in-progress | RT-179: wire the TUI functional core (/plan /context /memory /effort) into the gateway via run_session_scoped_command (blocker RT-175 CLOSED iter-1298) | claimed-by `agent-omarchy` since 2026-10-27T06:27 | [2026-10-27] iter-1379 shipped /effort + /plan; iter-1386 shipped /context (status half, read-only, same seam). REMAINING in value order: /memory, /skill-invoke, /export, /search — row stays open |
| remaining-gaps-1364-002 | done | RT-195(b): aft search-lane warmup/diagnostic in the bridge, or surface aft setup in the tool error | claimed-by `agent-omarchy` since 2026-10-27T05:12 | [2026-10-27] crates/operant-core/src/aft_bridge.rs — WARMUP_AREAS="callgraph,search", one spawn per root, contract test mutation-proven |
| remaining-gaps-1364-003 | done | RT-196: second smoke pass over the 22 unexercised tools (side-effect tools behind an explicit consent pass) | claimed-by `agent-omarchy` since 2026-10-29T06:53 | [2026-10-29] crates/operant-core/tests/tool_smoke_second_pass.rs |
| remaining-gaps-1364-004 | done | RT-194: extend is_deferrable to sourcehound composites (~4781 tok saved/request) — after RT-188/189 verified in production | claimed-by `agent-omarchy` since 2026-10-27T03:55 | [2026-10-27] crates/operant-core/src/tools/sourcehound_composites.rs |
| remaining-gaps-1364-005 | open | RT-190: memory bank topology — owner ruling (c) document-boundary now, (b) bridge later | — | — |
| remaining-gaps-1364-006 | open | S2: live wide-terminal (emoji) glyph probe — close or split the row | — | — |
| remaining-gaps-1364-007 | done | E303 x9: docs/library/ citations of nonexistent paths (peer-owned, do not fix from another slice) | — | [2026-10-27] docs/library/ |

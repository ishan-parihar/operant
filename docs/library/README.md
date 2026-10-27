# The operant mechanism library

One document per architectural element: what the mechanism IS, how it works,
and why it is that way. This is the library a new agent (or operator) reads
to understand the harness before touching it, and the place existing
mechanisms point at instead of re-explaining themselves.

Owns: the mechanism + intention behind each element of the harness.
Not for: rules and iteration protocol (`AGENTS.md`), where the project stands
(`docs/handoff/STATE.md`), commands and environment traps (`docs/RUNBOOK.md`),
audit findings (`docs/audit/2026-06-19-BUGS.md`), or frozen history (the
`docs/` reference shelf, `docs/PAGE-INDEX.md`).

## How to read it

Each doc is self-contained and cites its source modules. Read order for a
new agent: **agentic core** → the domain you came for. The docs describe the
mechanisms as they exist in the code; frozen plans and audits
(`docs/plans/`, `docs/audit/`) carry the history of how they got here, and
`docs/decisions/` carries the why-decisions that outlive the code shape.

## Contents

### The agentic core

- [agent-loop.md](agent-loop.md) — the ReAct loop: turn structure, streaming, retries, fallback, iteration budget, turn-end seam
- [context-management.md](context-management.md) — tiered eviction, decay curve, the compaction ladder (decay → micro-compact → semantic → LLM summarize), prompt-cache frozen prefix
- [memory.md](memory.md) — memory-wire: the one memory engine, retain/recall/sync_turn, hybrid retrieval, import, degradation contract
- [context-injection.md](context-injection.md) — every modality that injects content into the model's turn: memory recall, skills, MoA, advisor, recap, ponytail, context_files, org context injection
- [providers.md](providers.md) — the model path: OpenAI-compatible client, Anthropic, fallback chains, rate limiting, reasoning timeouts, model catalog
- [tools.md](tools.md) — the tool system: registry, 60+ builtins, AFT, sourcehound, kernel tools, guardrails, approval, blast radius
- [sub-agents.md](sub-agents.md) — delegation: sub_agent tool, swarm mode, async delegation
- [background-review.md](background-review.md) — the post-turn reviewer that learns skills and memories
- [verification.md](verification.md) — the verification harness: recipe detection, phases, evidence ledger

### Capacities

- [voice.md](voice.md) — voice mode: push-to-talk recording, 6 STT providers, 11 TTS providers, VAD, hallucination filter, state machine
- [vision.md](vision.md) — vision: the auxiliary-LLM contract, video analysis, image generation
- [browser.md](browser.md) — the sourcehound browser engine: cloakctl verbs, CDP access, cookie import, accessibility tree
- [web-tools.md](web-tools.md) — sourcehound web intelligence: search/scrape/extract/crawl, updates
- [kernel.md](kernel.md) — the persistent Python kernel: kernel_exec/state/refine, sidecar protocol, tool bridge
- [harness.md](harness.md) — the composable provider runtime: claims, seams, effects, swaps, composition-as-data

### Surfaces

- [tui.md](tui.md) — the terminal UI: renderer pipeline, model/view layering, composer, overlays, debug tooling
- [gateway.md](gateway.md) — the channel gateway: 7 adapters, session store, dispatch breaker, admin commands, subscriptions
- [sessions.md](sessions.md) — the session substrate: per-session conversation, branching, rewind, autocompact, session search/recap
- [skills.md](skills.md) — skills: discovery, guard, usage, marketplace, bundles, SOPs
- [mcp.md](mcp.md) — MCP both ways: client (HTTP/stdio/SSE, OAuth) and server (`operant mcp serve`)
- [plugins.md](plugins.md) — WASM plugins: manifest, capabilities, host, install
- [autonomous.md](autonomous.md) — autonomous mode: the TODO.md loop, workspace snapshot, publish path
- [org.md](org.md) — the org layer: Holon boundary, seat policy/budgets/authority, kanban, curator, cron

### Foundations

- [config.md](config.md) — the one config system: precedence, sections, one-home resolver, profiles
- [security.md](security.md) — the security stack: approval, blast radius, justification, redaction, credential registry, path safety, SSRF
- [persistence.md](persistence.md) — SQLite layer: state database, migrations, declarative reconciliation, trajectory, checkpoints
- [observability.md](observability.md) — observer events, runtime metrics, logging, insights, dashboard
- [operability.md](operability.md) — doctor, setup, integrations updater, service install, backup/import, MCP-serve

## Maintenance law

- A new mechanism doc lands here when a subsystem's *mechanism and intention*
  needs explaining to someone who has not read its source; it is listed in
  `docs/INDEX.md` the same iteration it lands.
- Each doc cites its source modules by path so a reader can jump from claim
  to code; counts and lists are derived from the code, never copied from
  another doc (RG-008's law: derive, do not copy).
- These docs are LIVE: when a mechanism changes, its library page changes in
  the same iteration (the "keeping this current" contract the agentic-os
  README set for its own pages).
- A library page does not duplicate the frozen reference shelf: history and
  rationale live in `docs/decisions/`, dated plans, and audits, and are
  linked, not restated.

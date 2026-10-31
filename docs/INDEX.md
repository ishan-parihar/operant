# Ownership map — every active doc, the question it owns, its status

Status: LIVE (current truth) · EVIDENCE (dated measured verdict) · PLAN-ACTIVE.

This is the map the genome's `ownership_map` role points at: for every
question about the project's documentation, exactly one file answers it.
Two files answering the same question is a defect, and this table is how
that is prevented. `genome lint` (E301) enforces it mechanically: every
doc in an owner location — the mechanism library included — must appear
here or the gate is red.

| File | Status | Owns | Not for |
|---|---|---|---|
| `AGENTS.md` | LIVE | the rules, the iteration protocol, the documentation routing table | current state (that is `docs/handoff/STATE.md`) |
| `docs/handoff/STATE.md` | LIVE | where the project stands right now | what to read first (that is `docs/handoff/README.md`) |
| `docs/handoff/README.md` | LIVE | what a fresh agent reads, in order | the rules themselves |
| `TODO.md` | LIVE | the register: `Implemented` / `Pending` / `Review` | open defects (that is `docs/audit/2026-06-19-BUGS.md`) |
| `docs/audit/2026-06-19-BUGS.md` (root) | LIVE | open findings and their fix state | rejected proposals (that is `docs/kb/CLOSED.md`) |
| `docs/RUNBOOK.md` | LIVE | exact commands, environment traps, fleet procedure | the rules (that is `AGENTS.md`) |
| `docs/kb/CLOSED.md` | APPEND-ONLY | what was tried and rejected — do not re-litigate | live findings |
| `docs/kb/WORKLOG.md` | APPEND-ONLY | what happened, unit of work by unit of work | current state |
| `docs/kb/feed.md` | APPEND-ONLY | cross-agent messages, claims, mentions | the workflow boards |
| `docs/kb/ASKS.md` | APPEND-ONLY | what the operator was asked and what they ruled | decisions (`docs/decisions/`) |
| `docs/kb/workflows/` | LIVE | in-flight work for ONE workflow, claim-guarded | cross-cutting state |
| `docs/decisions/` | IMMUTABLE | why X is the way it is (`AD-NNN-*.md`) | what to do next |
| `docs/regressions/` | IMMUTABLE | what bit us, and the law it produced (`RG-NNN-*.md`) | decisions |
| `docs/plans/` | DATED-FROZEN | current pre-registered plans | findings (that is `docs/audit/`) |
| `docs/plans/plan-2026-10-28-native-capability-extension-mcp-cli-runtime-admission.md` | DATED-FROZEN | native capability extension: MCP stays config-declared/discovered via `tools/list`, CLI capabilities move OFF the compile-time `KNOWN_CLIS` const to runtime admission via `--help`, phase-activated disclosure mirroring `<available_skills>` (measured 61x), the missing `cli_exec` invoke path, and a `cap_manage` verb set mirroring `skill_manage`; written for post-implementation red-team audit with the A0 attack-surface table named before the code | the measured cost table inside the plan itself (`operant mcp serve` wire capture: 94 tools = 75,016 B ≈ 18,754 tok/request vs names-only 306 tok) |
| `docs/plans/2026-10-28-kb-management-protocol-v2-sqlite-state-gates-one-active-per-.md` | DATED-FROZEN | the AMENDED KB protocol proposal — SQLite write-model for WORKLOG/BUGS/TODO/WORKFLOW, the plan/audit admission gate (E017), one-active-per-agent, the genome ponytail-audit; supersedes the 2026-10-27 v1 | the survey numbers (that is `docs/audit/2026-10-27-kb-surface-management-the-operant-survey-and-its-long-horizo.md`) |
| `docs/plans/2026-10-27-kb-management-protocol-consolidation-lifecycle-compaction-an.md` | DATED-FROZEN (SUPERSEDED by the 2026-10-28 v2) | the first KB protocol proposal — kept for the record | the amended design (that is `docs/plans/2026-10-28-kb-management-protocol-v2-sqlite-state-gates-one-active-per-.md`) |
| `docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md` | DATED-FROZEN | the codex import build list: 12 rows, ~2,900 LOC, zero new deps, each with its acceptance + mutation guard | the audit measurements behind it (that is `docs/audit/2026-10-31-codex-architecture-deepen/00-INDEX.md`) |
| `docs/audit/` | DATED-FROZEN | what was audited, what was found | live findings (that is `docs/audit/2026-06-19-BUGS.md`) |
| `docs/audit/2026-10-31-codex-architecture-deepen/00-INDEX.md` | DATED-FROZEN | the codex-rs deep-dive: the eight unmeasured families (hooks, command safety, skills, memory, file editing, mermaid, process hardening, turn attribution), the ranked ponytail shortlist, the four rows where operant is AHEAD, and defects D1–D5 | the build list (that is `docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md`) and the 2026-10-30 portability ledger's four closed families |
| `docs/plans/2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md` | DATED-FROZEN | the unified session-management re-architecture v2: jcode picker pane + omp unified session/subagent hub + employee tags + the ponytail-scoped session-server phase (gateway-daemon route group); supersedes the 2026-10-28 v1 | the audit evidence (that is `docs/audit/2026-10-28-session-management-red-team-audit-v2-corrections-the-server-.md`) |
| `docs/plans/2026-10-28-unified-session-management-jcode-style-session-picker-pane-o.md` (tombstone; content at `docs/archive/2026-10-28-unified-session-management-jcode-style-session-picker-pane-o.md`) | DATED-FROZEN (SUPERSEDED by the 2026-10-28 v2) | the first session-management plan — kept for the record | the corrected design (that is `docs/plans/2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md`) |
| `docs/audit/2026-10-28-session-management-red-team-audit-v2-corrections-the-server-.md` | DATED-FROZEN | the session-management red-team audit v2: the three remediation corrections (seat_id exists, reconcile_columns is the migration path, get_session_messages exists) + the server question answered (gateway-daemon route group); supersedes the 2026-10-28 v1 | the plan it feeds (that is `docs/plans/2026-10-28-unified-session-management-v2-jcode-picker-pane-omp-hub-empl.md`) |
| `docs/audit/2026-10-28-session-management-red-team-audit-all-surfaces-vs-jcode-oh-m.md` | DATED-FROZEN (SUPERSEDED by the 2026-10-28 v2) | the first session-management audit — kept for the record | the corrected audit (that is `docs/audit/2026-10-28-session-management-red-team-audit-v2-corrections-the-server-.md`) |
| `docs/audit/2026-10-27-kb-surface-management-the-operant-survey-and-its-long-horizo.md` | DATED-FROZEN | the 2026-10-27 KB surface survey: measured state + the L1–L6 long-horizon limitations | the protocol that answers them (that is `docs/plans/2026-10-27-kb-management-protocol-consolidation-lifecycle-compaction-an.md`) |
| `docs/archive/` | TOMBSTONE | superseded history, with a tombstone at each original path | anything current |
| `docs/CHANGELOG.md` | APPEND-ONLY | release history, dated | the current state |
| `docs/INDEX.md` | LIVE | this map | anything else |
| `docs/RECORDS.md` | GENERATED | the decision/regression index | the records themselves |
| `docs/PAGE-INDEX.md` | LIVE | the frozen reference shelf's table of contents (the 2026-08 one-off design/parity/audit pages) | current mechanisms (that is `docs/library/`) |

## The mechanism library (`docs/library/`)

One doc per architectural element: the mechanism AND the intention.
These are LIVE pages — they change in the same iteration as the code
they describe. The front door is [docs/library/README.md](library/README.md); the
`Source:` header of each page cites its modules.

### The agentic core

| File | Owns | Not for |
|---|---|---|
| `docs/library/README.md` | the library's front door and maintenance law | any single mechanism |
| `docs/library/agent-loop.md` | the ReAct loop: turn structure, streaming, retries, fallback, iteration budget, turn-end seam | context eviction (`context-management.md`) |
| `docs/library/context-management.md` | tiered eviction, decay curve, the 4-rung compaction ladder, the frozen-prefix law | injection of NEW content into a turn (`context-injection.md`) |
| `docs/library/memory.md` | the memory-wire engine: lifecycle, retrieval, the consolidated tool pair, import, degradation contract | the session transcript (`sessions.md`), session search (`sessions.md`) |
| `docs/library/context-injection.md` | every modality that injects content into the model's turn (memory recall, skills, MoA, advisor, recap, ponytail, context_files, org) | eviction/compaction (`context-management.md`) |
| `docs/library/providers.md` | the model path: OpenAI-compatible client, Anthropic, fallback, rate limits, model catalog | which models cost what (`docs/library/observability.md` §insights) |
| `docs/library/tools.md` | the tool system: registry, builtins, guardrails, dedup | any single integration's own page |
| `docs/library/sub-agents.md` | delegation: sub_agent tool, swarm mode, async delegation | the org layer's seats (`org.md`) |
| `docs/library/background-review.md` | the post-turn reviewer that learns skills and memories | foreground verification (`verification.md`) |
| `docs/library/verification.md` | the verification harness: recipe detection, phases, evidence ledger, ESTOP | the post-turn review (`background-review.md`) |

### Capacities

| File | Owns | Not for |
|---|---|---|
| `docs/library/voice.md` | voice mode: push-to-talk, STT/TTS providers, VAD, hallucination filter | vision (`vision.md`) |
| `docs/library/vision.md` | vision: the auxiliary-LLM contract, video analysis, image generation | voice (`voice.md`) |
| `docs/library/browser.md` | the sourcehound browser engine: cloakctl verbs, CDP, cookies, accessibility tree | web search/scrape (`web-tools.md`) |
| `docs/library/web-tools.md` | sourcehound web intelligence: search/scrape/extract/crawl | the browser (`browser.md`) |
| `docs/library/kernel.md` | the persistent Python kernel: kernel_exec/state/refine, sidecar protocol, tool bridge | the harness (`harness.md`), MCP (`mcp.md`) |
| `docs/library/harness.md` | the composable provider runtime: claims, seams, effects, swaps | the kernel sidecar (`kernel.md`) |

### Surfaces

| File | Owns | Not for |
|---|---|---|
| `docs/library/tui.md` | the terminal UI: renderer pipeline, model/view layering, composer, overlays | TUI debugging tooling (`tui-debugging.md`) |
| `docs/library/tui-debugging.md` | TUI debug + live-repro tooling: headless simulate, scenario corpus, tmux recipes | the TUI's architecture (`tui.md`) |
| `docs/library/gateway.md` | the channel gateway: 7 adapters, session store, dispatch breaker, subscriptions | the dashboard (`observability.md`), platform org seats (`org.md`) |
| `docs/library/sessions.md` | the session substrate: per-session conversation, branching, rewind, checkpoints, search/recap | persistence mechanics (`persistence.md`) |
| `docs/library/skills.md` | skills: discovery, guard, usage, marketplace, bundles, SOPs | ponytail packs (`context-injection.md` §ponytail) |
| `docs/library/mcp.md` | MCP both ways: client (HTTP/stdio/SSE, OAuth) and server | plugins (WASM) (`plugins.md`) |
| `docs/library/plugins.md` | WASM plugins: manifest, capabilities, host, install | MCP servers (`mcp.md`) |
| `docs/library/autonomous.md` | autonomous mode: the TODO.md loop, workspace snapshot gate, publish path | the iteration protocol (`AGENTS.md`) |
| `docs/library/org.md` | the org layer: Holon boundary, seat policy/budgets/authority, kanban, curator, cron | context injection quotas (`context-injection.md` §10) |

### Foundations

| File | Owns | Not for |
|---|---|---|
| `docs/library/config.md` | the one config system: precedence, one-home resolver, profiles | subsystem-specific keys (each library page's own sections) |
| `docs/library/security.md` | the guard stack: approval, blast radius, justification, redaction, credentials, path safety, SSRF, staging | skill content scanning (`skills.md`) |
| `docs/library/persistence.md` | the SQLite substrate: state DB, schema reconciliation, migrations, trajectory, RAM budget | session semantics (`sessions.md`), the memory store (`memory.md`) |
| `docs/library/observability.md` | observer events, runtime metrics, logging, insights, the dashboard | the doctor (`operability.md`) |
| `docs/library/operability.md` | doctor, setup, integrations updater, service install, backup/import, MCP-serve | the gateway daemon (`gateway.md`) |

## The frozen reference shelf

The 2026-08 one-off design and parity documents (`docs/HERMES_*`,
`docs/ORGANISM-*`, `docs/*PLAN*`, `docs/*AUDIT*`) are a historical shelf:
cite them, never edit them, never let them answer a current question.
They are the *evidence* behind decisions, not the decisions.
`docs/PAGE-INDEX.md` is that shelf's table of contents. Three shelf
pages were re-homed into the library — the original paths now hold
tombstones pointing at the archived copies, and the library pages are
the live answers:
`docs/kernel.md` → `docs/library/kernel.md`
(archive: `docs/archive/kernel.md`),
`docs/harness-kernel.md` → `docs/library/harness.md`
(archive: `docs/archive/harness-kernel.md`),
`docs/tui-debugging.md` → `docs/library/tui-debugging.md`
(archive: `docs/archive/tui-debugging.md`).

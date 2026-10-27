# Context injection — every modality that shapes the model's turn

Owns: the complete inventory of mechanisms that put content in front of the
model — where each one lands (frozen prefix vs turn tail), when it fires,
and what it costs.
Not for: the context *budget* mechanics ([context-management.md](context-management.md)),
the memory engine itself ([memory.md](memory.md)).

Source: `crates/operant-core/src/agent/builders.rs` (assembly),
`crates/operant-core/src/moa.rs`, `advisor.rs`, `recap.rs`,
`session_recap.rs`, `ponytail.rs`, `context_files.rs`,
`context_references.rs`, `crates/operant-core/src/org/context_injection.rs`,
`crates/operant-core/src/skills.rs` + `agent/skill_preprocessing.rs`,
`crates/operant-core/src/distillation.rs`.

## The placement law

Every injection lands in one of exactly two places:

| Zone | Content | Cache effect |
|---|---|---|
| **Frozen prefix** (system prompt) | base persona + skills guidance + ponytail `always_on` ruleset | byte-stable between turns; ANY change invalidates the provider prompt cache |
| **Turn tail** (`<turn_context>` block on the latest user message) | memory recall, workspace facts, MoA advice, harness state, recaps | volatile; free to change every turn (iter-911) |

A modality that changes every turn must never ride the prefix — that is
the whole prompt-cache stability contract. A modality that is effectively
constant (skills guidance, the ponytail ruleset) rides the prefix but pays
the invalidation cost on change, which is documented, not free.

## The inventory

### 1. Memory recall (turn tail)

`memory.rs::prefetch` ranks and formats the most relevant memories for the
current turn and queues them into the next turn's `<turn_context>`.
Fires: every turn, after the previous one (`queue_prefetch` in
`agent/turn_finalizer.rs`). Cost: retrieval only (no LLM). See
[memory.md](memory.md).

### 2. Skills guidance (frozen prefix, on load)

`skills.rs` + `agent/skill_preprocessing.rs` — loaded skills' guidance
sections ride the system prompt; invocations load full SKILL.md content
into a user message. The SkillManager also exposes **extra roots** (the
operator pool, then integration pools — ponytail skills) with the
operator's pool winning name collisions. `clarify`-style progressive
disclosure: tier 1 = list metadata, tier 2-3 = full content on view. See
[skills.md](skills.md).

### 3. MoA — Mixture-of-Agents (turn tail)

`moa.rs` — hermes moa_loop.py parity (the Python reference project is
a machine-local checkout, not a repo path). A **turn amplifier**:
explicit fan-out per turn. Reference (advisor) models receive a flattened
text-only view of the conversation; an aggregator model synthesizes their
advice into a concise guidance block injected before the acting agent
answers. The normal agent loop still owns tool calling — MoA is context
synthesis, not execution. Config: `[moa] enabled`, `references[]`,
`aggregator`. Cost: N reference calls + 1 aggregation call per turn.

### 4. Advisor (post-turn watchdog, feeds the next turn)

`advisor.rs` — omp `coding-agent/src/advisor/` parity (P0 #1 of the
agentic-core upgrade plan). Where MoA is a per-turn amplifier, the advisor
is an **always-on transcript watchdog**: a second LLM receives an
INCREMENTAL delta after each completed turn (via the `TurnEndBus`, see
[agent-loop.md](agent-loop.md)) and streams back short notes. The notes
ride the next turn's tail. Off by default: `[advisor] enabled = false`,
with optional `[advisor.model]` override and `timeout_secs` (default 120s,
Default-impl bug fixed iter-1328). Cost: one side-channel call per turn.

### 5. Recap — idle "where the session stands" (side channel; never in context)

`recap.rs` — omp parity (`recap.*`, `recap-user.md`, `RecapNotice`).
After a completed turn, when no further activity arrives for
`recap_idle_seconds`, ONE ephemeral side-channel LLM call renders a
~40-word status line over a SNAPSHOT of the transcript. The live agent
context is **never touched** — the snapshot is built through
`crate::moa`'s flattener, and the recap renders to the operator surface
(TUI status, gateway) only. `[recap.model]` selects the side-channel
model (empty = primary, iter-1328). Cost: one small call per idle period.

### 6. Session recap (`session_recap.rs`) — re-orientation across sessions

Pure LOCAL computation over the persisted message history (hermes
`session_recap.py` parity, Claude Code `/recap` parity): no LLM call, no
auxiliary model, no prompt-cache invalidation. Summarizes what happened
recently in a session so a user juggling multiple sessions re-orients
instantly. Cost: zero.

### 7. Ponytail (frozen prefix, when `always_on`; user message, when invoked)

`ponytail.rs` + `crates/operant-core/src/skills_guard.rs` (the
install guard) + the SkillManager extra roots. The fourth self-updating
integration and the first CONTENT one (iters 1306/1307/1311/1314): upstream
releases carry no assets, so the tag tarball is the payload; installs land
under versioned `~/.operant/integrations/ponytail-vX.Y.Z/` (keep=2), NEVER
in the operator's `~/.operant/skills/`. Two injection modes:

- `inject = "on_invoke"` (default): zero prompt-byte change — the skills
  are invocable (`/skill ponytail-review`) like any skill.
- `inject = "always_on"`: the active level's ruleset rides the frozen
  prefix as a `<ponytail>` block after the skills guidance (lite/full/ultra
  suffix), **never on Holon runs**. Changing the level invalidates the
  prompt cache — documented consequence.

`[agent.ponytail]` defaults ALL OFF (`enabled=false`,
`default_level="full"`, `inject="on_invoke"`) — the posture changes what
the model is told every turn and must never silently enable. `/ponytail
[lite|full|ultra|off]` is a process-local runtime override (config
untouched). Design record:
`docs/plans/2026-10-26-ponytail-as-a-self-updating-content-integration.md`.

### 8. Context files (frozen prefix, at boot)

`context_files.rs` — workspace-level guidance (`AGENTS.md`, `CLAUDE.md`):
loaded, scanned for obvious prompt-injection text, truncated if oversized,
injected into the system prompt as durable project context. This mirrors
upstream operant's context-file behavior at smaller scope.

### 9. `@`-references (`context_references.rs`)

The `@kind:` grammar (`@file:`, `@folder:`, `@skill:`…) — accepted file
references expand into the message at submit time via
`REFERENCE_PATTERN`; the TUI typeahead inserts `@file:<path>` /
`@folder:<path>/` tokens (see [tui.md](tui.md)). Bare `@path` is not the
grammar and does not expand.

### 10. DM/feed org context injection (turn tail, gateway)

`org/context_injection.rs` — phase 1 (the cron seam), approved with the
owner's rulings: per-aspect character quotas (the MEMORY.md budget model
applied to four aspect classes) and re-ranking across the classes
(recency + lexical affinity + author authority). Sources are org-internal
only. Config: `[context_injection] enabled = true`, `total_char_cap`,
`dm_quota`, `feed_quota`, `global_quota`, `dept_quota`.

### 11. Distillation (`distillation.rs`)

State distillation for durable long-term memory — the extraction step
that decides what a turn's content means for the *durable* store, feeding
memory retain. See [memory.md](memory.md).

### 12. Turn summary (display only, never context)

`turn_summary.rs` — per-turn accounting line. Pure formatter + collector
over the tool event feed; explicitly NOT an injection (display-only,
listed here so nobody mistakes it for one).

## Cost discipline

| Modality | LLM calls | Lands in | Fires |
|---|---|---|---|
| memory recall | 0 | tail | every turn |
| skills guidance | 0 | prefix | on load/change |
| MoA | N+1 | tail | per turn when enabled |
| advisor | 1 | tail (next turn) | per turn when enabled |
| recap (idle) | 1 | nowhere (operator surface) | after idle |
| session recap | 0 | nowhere (operator surface) | on demand |
| ponytail | 0 | prefix or user message | per posture |
| context files | 0 | prefix | boot |
| @references | 0 | message | per reference |
| org context injection | 0 | tail | gateway turns |
| distillation | 0 | → memory store | turn end |

## Intention

The tail-vs-prefix law exists because the prompt cache is the single
biggest cost lever: a frozen prefix makes long sessions affordable, and
one careless injection into the prefix re-bills the whole context every
turn. The always-on vs on-demand split (advisor/MoA off by default,
ponytail OFF by default) exists because every always-on modality changes
what the model is told *every* turn — that is an operator decision, never
a default. The omp-parity ports (advisor, recap) deliberately use side
channels that never touch the live context, so observability never costs
cache.

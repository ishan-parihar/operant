# Sub-agents — delegation, swarm, async

Owns: the mechanisms by which one agent spawns others — the `sub_agent`
tool (single / swarm), and the async delegation registry.
Not for: the advisor/MoA context modalities
([context-injection.md](context-injection.md)) — those shape context, they
do not execute.

Source: `crates/operant-core/src/tools/sub_agent_tool.rs`,
`crates/operant-core/src/tools/async_delegation.rs`,
`crates/operant-core/src/daemon_pool.rs`.

## `sub_agent` — one code path, three modes

A parent agent delegates focused analysis to isolated child agents
without changing its own ReAct loop:

- **single** — one child, synchronous.
- **swarm** (`tasks` argument) — N real worker sessions, each with its OWN
  agent loop, message history, and tool scope, fanned out behind the same
  semaphore + budget machinery.
- **async** (`background=true`) — dispatch and continue: the child runs on
  a spawned tokio task, the parent gets a handle immediately, and the
  user + model keep working while it runs.

## The async delegation registry (`async_delegation.rs`)

Backs the async mode: the record transitions to complete/fail when the
child finishes, and the result is retrievable by handle. Fire-and-forget
semantics with observable completion.

## Daemon pool (`daemon_pool.rs`)

The helper for background/best-effort work generally: a daemon task must
not block shutdown on `.join()`, is fire-and-forget by default (errors
logged at WARN, never propagated), and is observable via a static handle
table so tests can wait for completion. `daemon_pool::spawn("name", …)`.

## What a child inherits, and what it does not

Measured 2026-11-14, and corrected here because this page used to claim the
budgeting and approval machinery "applies uniformly". Budgeting does — one
`SpawnPermit` claim covers all three modes, so a swarm cannot get a looser
admission rule than a single child. Approval did not:

| inherited from the parent | how |
|---|---|
| model, depth, toolsets, disabled tools/toolsets | constructor arguments |
| tool-approval allowlist and the file it persists to | `with_parent_approval`, at every hop (iter-2329) |
| nothing else — a child is a fresh `AgentConfig` | `AgentConfig::default()` |

So a child is **headless**: it has no interactive approver, and a tool the
smart gate flags is refused unless the operator already allowed it. That is
the intended posture, but it only matches the parent's if the allowlist is
handed down — which is what iter-2329 fixed.

## What survives a restart

Background delegations persist to `<operant home>/delegations.json`
(iter-2331). A record that was pending when the process died is recovered as
**failed** with the reason, not left reading as running. Foreground workers
live in the process only: the durable account of a finished swarm is the
transcript card, which carries each worker's report.

A worker's answer carries its own handles: the section heading names the
worker id and the session it ran in, and a truncated report can be read back
with `session_search(session_id="…")` — which reads a session's tail from the
id alone (iter-2356). The heading alone was not enough before that: the id
travelled from iter-2320, but no tool would open a session by id.

What the parent receives is a **preview** per worker, not the full report
(iter-2357). A 6-worker swarm pushes ~4.6 KB rather than ~25 KB; the rest of a
report is one `session_search` call away. The preview is a deliberate middle
between "pay for every word" and "make the parent guess at dispatch time which
workers it will need" — it is enough to decide which reports matter.

A swarm returns at its own deadline (`SWARM_RESULT_DEADLINE_SECS = 120`), not
when its slowest worker does (iter-2361). Finished sections are delivered as
they land; workers still running at the deadline are reported `[cut off]` and
cancelled. They are not detached: a detached worker would free the spawn
reservation while a model stream was still live.

A re-dispatched background delegation with the same `request_key` returns the
existing handle instead of starting a second child (iter-2360). Without a key,
losing the handle and re-dispatching ran the work twice.

## Intention

Delegation exists because one context cannot hold ten explorations: the
parent stays small and steers; children burn their own context and report
back. The single/swarm/async trio shares one code path so the budgeting
machinery applies uniformly — a swarm does not get a looser permission model
than a single child.

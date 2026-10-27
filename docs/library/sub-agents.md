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

## Intention

Delegation exists because one context cannot hold ten explorations: the
parent stays small and steers; children burn their own context and report
back. The single/swarm/async trio shares one code path so the budgeting
and approval machinery applies uniformly — a swarm does not get a
looser permission model than a single child.

# Two-tier memory — the FS-seat / memory-wire-org hybrid (2026-10-07)

Owner decision this implements: **both, not either/or** — memory-wire is the
global memory (the org tier, native through MCP and its hooks), and a
file-system memory managed by the seat itself is the seat-specific tier.
Investigated against the live hermes-agent deployment
(`~/Documents/github/my-projects/agentic-harness/parent-projects/hermes-agent`)
whose cron employees already run this shape:
`~/.hermes/cron/employee-memory/emp-<id>.json` — per-employee files, keyed by
the §3.1.1-derived id, holding the run-continuity events that give a
cold-start cron employee an identity thread (see the
`cron-employee-identity-contract` skill: "carries memory of prior runs").

## Verdict: the hybrid is the better call. Adopted.

It replaces the earlier emp-bank-in-memory-wire sketch (plan §4 of
`plan-2026-10-07-organism-operational-memory.md`): the org tier stays
exactly as designed there; only the seat tier changes substrate.

**Why the FS tier wins for seats:**

1. **A seat needs its whole memory, not top-k recall.** A cron seat starts
   cold every cycle; injecting the full curated file is the correct shape.
   Semantic recall is the wrong tool at seat scale and adds a query round
   every run.
2. **Manually managed = the seat's own curation.** The seat reads/writes its
   memory with its file tools deliberately — the exact hermes precedent, and
   the mechanism the owner asked for. No new tool surface: the file tools
   exist.
3. **Transparent and operator-editable.** Markdown on disk; the owner can
   read, fix, or seed a seat's memory directly. Git-able, diffable,
   auditable — org memory in a DB row is none of these by hand.
4. **memory-wire stays where it is strong.** Cross-seat semantic recall,
   retention policies, native MCP + hooks (`prefetch`/`sync_turn`) — all
   already wired in-process. The org bank rides the existing `bank` field
   (`memory_wire.rs:115`), no new dependency.
5. **BuiltinProvider precedent.** operant already ships the file-backed
   memory pattern (MEMORY.md/USER.md) — the seat tier is that pattern,
   scoped per seat directory.

## The architecture

| Tier | Substrate | Path / bank | Writers | Readers |
|---|---|---|---|---|
| Org | memory-wire | bank `org` | governance acts (decision accept, grant mint, directive), synthesis | every seat's prefetch; the owner |
| Seat | FS, markdown | `~/.operant/org/employees/<seat>/MEMORY.md` | the seat itself, via file tools, at its own discretion | that seat (injected at cycle start); manager audit |

Enforcement of the transferable/non-transferable boundary is unchanged from
the plan: **by writer** — governance acts are the only org-bank writers.

## Wiring (the implementation slice)

1. **Injection at cycle start** (`run_agent_job`): after `set_session_id`,
   read the seat's `MEMORY.md` (if present) and prepend it to the job prompt
   under a fixed delimiter, e.g.
   `## Your seat memory (curated by you in prior cycles)\n<file>\n---\n`.
   Cold start = no file = byte-identical to today (dark-mergeable).
2. **Charter prompt guidance** (rides the F2 fix): each seat's charter gains
   one line — *"Maintain your seat memory at
   `~/.operant/org/employees/<seat>/MEMORY.md`; read it at cycle start and
   update it with what changed"* — so the seat owns the file's growth.
3. **Org tier**: gateway/agent memory provider pinned to bank `org`
   (config-level or at provider construction); `sync_turn` retains there.
   Per-Seat 7 of the old plan (merged prefetch, bank routing by session) is
   now unnecessary for the seat tier — the file IS the seat memory; only
   the org bank routes through memory-wire.

## What this makes true

- **Identity without bleed** (the owner's fear): seats diverge via their
  own files; nothing seat-private ever enters a shared engine.
- **Cold start that remembers**: the FS file is the continuity thread the
  hermes identity contract demands ("carries memory of prior runs").
- **The socialization feature gets a substrate**: seats exchange
  intelligence in DM sessions and write what they learned into their own
  MEMORY.md — densification is file growth, visible to the owner.
- **The org converges separately**: synthesis (Slice 9) reads seats'
  outputs from worklog/board and retains org-level facts to the `org` bank.

## Non-goals

- No per-seat memory-wire banks (superseded by this doc).
- No inbound platform reads here (separate wave, owner decision pending).
- No charter-level auto-mutation: evolution stays evidence → proposal →
  decision → amendment (plan §5).

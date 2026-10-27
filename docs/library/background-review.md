# Background review — the post-turn learner

Owns: the background agent that forks after a turn to decide whether a
skill or memory should be saved or updated.
Not for: the memory engine ([memory.md](memory.md)), the skills system
([skills.md](skills.md)), the write-approval gate that constrains its
writes ([security.md](security.md)).

Source: `crates/operant-core/src/agent/background_review.rs`,
`crates/operant-core/src/write_origin.rs`,
`crates/operant-core/src/write_approval.rs`,
`crates/operant-core/src/curator/` (the lifecycle manager it feeds).

## The mechanism

After every turn, `OperantAgent::run()` may trigger a background review:
a fork of the agent that evaluates whether any skill or memory should be
saved or updated from what the turn did. Writes go straight to the memory
+ skill stores; **the main conversation and prompt cache are never
touched** — the review runs on its own context, so it costs no cache and
cannot corrupt the live transcript.

## Model routing

The review agent runs on the MAIN model by default ("auto"); an auxiliary
route can be configured. A failed review is a log line, not a failed turn.

## Why it is gated

The review is a *writer* from a non-interactive origin, so it passes
through the write-provenance machinery:

- `write_origin.rs` — task-local + scoped origin tracking
  (`TASK_ORIGIN` → scope). The review sets origin `"background_review"`
  so the skill manager can enforce write guards (no editing
  protected/hub skills, no creating skills outside review scope). The
  process-lifetime WRITE_ORIGIN poisoning bug (iter-332/338) is the
  recorded reason this is task-local today.
- `write_approval.rs` — staged write approval (plan 012): writes from
  non-interactive contexts (background review, cron, gateway) are STAGED
  (`GateDecision::Stage { pending_id }`) for the user to approve/discard
  later; interactive (user) origin bypasses the gate.

## What it feeds

- The **curator** (`curator/`) — skill lifecycle management: state
  transitions (active → stale → archived), pinning, usage tracking,
  backup/rollback snapshots, LLM-driven consolidation of overlapping
  skills.
- **memory retain** — the durable facts the turn produced
  (sync_turn is the automatic path; the review is the judgment path).
- The **skill nudge counter** in the agent loop — nudges toward creating
  a skill when a repeated pattern is detected.

## Intention

Self-improvement has to be post-turn and off the hot path: doing it inside
the turn would double every bill and risk the live context. The review is
deliberately a fork with a scoped origin and a staged-approval gate — the
learning loop writes, but never unattended. The full design history is
in the hermes parity plans (`docs/plans/`, frozen) and
`docs/CHANGELOG.md`.

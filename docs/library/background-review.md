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
saved or updated from what the turn did. The skill lane may update the
skill store under its scope/read-before-modify guard; the memory lane is
unattended and therefore refuses memory writes, reporting candidates for a
foreground turn instead. **The main conversation and prompt cache are never
touched** — the review runs on its own context, so it cannot corrupt the
live transcript.

## Model routing

The review agent runs on the MAIN model by default; an auxiliary route can
be configured through `[auxiliary_models.memory]`. Model, `base_url`, and
`api_key` form one route contract: a model-only slot rides the primary
client, while a pinned endpoint or credential gets a dedicated client.
A different model, endpoint, or credential is a cold route and uses digest
replay. A failed review is a log line, not a failed turn.

## Why it is gated

The review is a *writer* from a non-interactive origin, so it passes
through the write-provenance machinery:

- `write_origin.rs` — task-local + scoped origin tracking
  (`TASK_ORIGIN` → scope). The review sets origin `"background_review"`
  so the skill manager can enforce write guards (no editing
  protected/hub skills, no creating skills outside review scope). The
  process-lifetime WRITE_ORIGIN poisoning bug (iter-332/338) is the
  recorded reason this is task-local today.
- `write_approval.rs` — the foreground knowledge-write approval seam.
  Background reviews do not stage durable writes: under AD-020 the memory
  lane refuses writes, and the skill lane applies its scope and
  read-before-modify guard directly.

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
deliberately a fork with a scoped origin and explicit write guards — the
skill lane may write only what it read and may create only non-protected
skills; the memory lane cannot write. The full design history is
in the hermes parity plans (`docs/plans/`, frozen) and
`docs/CHANGELOG.md`.

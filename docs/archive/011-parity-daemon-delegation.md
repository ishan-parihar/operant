# 011 — Parity: daemon pool + delegation live-log (hermes `daemon_pool.py`, `delegation_live_log.py`)

Stamped: `d394c136`. Priority: **P1** (core — user-designated "daemon/delegation").

## Why

Operant has a daemon (`crates/operant-runtime/src/daemon/mod.rs`) and delegation
(`DelegateTool` in `crates/operant-runtime/src/tools/delegate.rs`, `SubAgentTool` in
`crates/operant-core/src/tools/sub_agent_tool.rs`), but two hermes behaviors are
missing:
1. **Daemon-pool semantics** (`daemon_pool.py`, 64 LOC): background/best-effort work
   must not keep the process alive (daemon threads; pool skips idle-timeout join). Rust
   equivalent: spawned tasks must not block shutdown on join, and background task
   sites (background review, curator ticks) should share one helper.
2. **Delegation live-log** (`delegation_live_log.py`, 424 LOC): every delegation
   writes an append-only, human-readable transcript under
   `<home>/cache/delegation/live/<delegation_id>/task-<n>.log` — attached immediately,
   one line per child event (assistant text, tool calls, final result), with per-line
   truncation budgets and credential redaction; mounted so tools can read it back.

## Files in scope

- New `crates/operant-runtime/src/tools/delegation_live_log.rs` (or
  `crates/operant-core/src/delegation_live_log.rs` — put it where both
  `DelegateTool` (runtime) and `SubAgentTool` (core) can import; prefer core if
  possible, else runtime and have core's SubAgentTool not depend on it).
- `crates/operant-runtime/src/tools/delegate.rs` (write live-log entries on events)
- `crates/operant-core/src/tools/sub_agent_tool.rs` (write live-log entries if it has
  its own event stream; coordinate with delegate.rs to avoid double-wiring)
- New `crates/operant-runtime/src/tasks/daemon_pool.rs` or extend
  `crates/operant-runtime/src/daemon/mod.rs` (daemon spawn helper)
- Background task sites: `crates/operant-core/src/agent/background_review.rs`,
  curator tick sites in `crates/operant-core/src/curator/mod.rs`

## Files out of scope

- Sub-agent budget semantics (`delegate.rs:1235` "TODO thread from parent") — note but
  don't implement here.
- Channel/gateway delivery.

## Hermes reference (key semantics)

- `new_live_delegation_id()` — unique id per delegation; log path
  `cache/delegation/live/<id>/task-<n>.log`; `_one_line(text, limit)` truncation
  budgets per line; `_redact(text)` — redact credentials before writing (the dir is
  mounted for tool read-back, so secrets must never land in it).
- Writer attaches immediately (before the child produces output) and streams one line
  per child event; writer failure degrades to debug logging (never fails the task).
- Daemon pool: best-effort work must not join-block shutdown.

## Steps

1. **Live-log module**: `new_live_delegation_id()`, `live_transcript_root(home)`,
   `open_task_log(home, id, task_n) -> io::Result<impl Write>` (create dirs 0700),
   `one_line(text, limit)`, `redact(text)` (reuse the redaction approach from
   `security.rs`/`debug_helpers.rs` if one exists; else a conservative pattern list:
   api keys, bearer tokens, `sk-`/`ghp_` patterns). File perms 0600.
2. **Wire into `DelegateTool`**: on dispatch, create `<id>/task-0.log`; on each child
   event (assistant text / tool call / result) append a one-line entry. Failures to
   write → `tracing::debug` only. Add the transcript path to the tool result so the
   parent agent can read it back.
3. **Daemon-pool helper**: `spawn_daemon_task(fut)` that detaches the task and
   guarantees no shutdown join (track in a best-effort set; on shutdown, log-and-drop).
   Route `spawn_background_review` and curator background ticks through it.
4. Tests (below); update `BUGS.md`.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-runtime --all-targets -- -D warnings && cargo clippy -p operant-core --all-targets -- -D warnings
cargo test -p operant-runtime --lib live_log && cargo test -p operant-runtime --lib daemon && cargo test -p operant-core --lib background_review
cargo test --workspace --all-features --lib          # final gate
```
Live smoke: a `run` that delegates writes `~/.operant/cache/delegation/live/<id>/task-0.log`
containing per-event lines; transcript contains no credential material.

## Test plan

- `live_log_creates_task_file`: dispatch → file exists, dir 0700, file 0600.
- `live_log_appends_per_event_lines`: scripted child events → one line each, in order.
- `live_log_redacts_credentials`: a line containing a fake `sk-...` token → redacted.
- `live_log_writer_failure_degrades`: read-only parent dir → task still succeeds,
  debug-logged.
- `daemon_pool_task_does_not_block_shutdown`: spawn a daemon task, drop the runtime
  within a short timeout → no hang.

## Maintenance note

- The live-log dir is agent-visible (mounted like hermes) — it is an output surface;
  redaction must be maintained when new secret shapes appear.
- Background tasks must never grow unbounded: cap open live-log ids (prune older than
  N days on open — pick N=7 and document).

## Escape hatches

- If `SubAgentTool` (core) can't import the runtime crate (dependency direction), the
  live-log wiring stays in `DelegateTool` only, and `SubAgentTool` gets a stub note —
  do not invert the dependency direction.

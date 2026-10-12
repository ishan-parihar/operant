# Packet C — Scheduled-run autocompaction with a compaction floor (Step C)

## Goal

Per §4.5, a session compacts **after scheduled execution**, with a **compaction floor** so
short sessions are not reduced to a lossy summary-of-a-summary.

The session substrate already landed in iter-548 and is your foundation. Read
`crates/operant-core/src/session/mod.rs` before you write anything.

## Owner rules — read these first

**You own exactly these files. Do not create, edit, or delete anything else.**

- `crates/operant-core/src/session/autocompact.rs` (you create this)

**Do NOT touch:** `session/mod.rs`, `org/mod.rs`, `cmd_org.rs`, `turn_end.rs`, `employee.rs`,
`authority.rs`, `tools.rs`, or any file under `crates/operant-cli/`. Other agents own those
right now and are editing them concurrently. The coordinator registers your module and wires
it into the scheduler.

**If you believe you need to change a file you do not own, do not change it — report it in
your final message instead.**

## Foundation you build on (landed and tested in iter-548)

`crates/operant-core/src/session/mod.rs` provides `SessionStore`:

- `acquire(&SessionKey) -> usize` — make resident, rehydrating from disk on a miss
- `release(&SessionKey)` — hand back; **this is not a wipe**
- `evict(&SessionKey) -> bool` — drop from memory, keep on disk
- `discard(&SessionKey) -> bool` — destructive; drops the resident copy
- `peek(&SessionKey) -> Vec<Message>`
- `persist(&SessionKey, &[Message])` — hand a transcript back to the store
- `is_warm`, `stats`
- `SessionStoreConfig { warm_capacity, rehydrate_tail }`, default `warm_capacity = 32`
- `SessionKey`, `DEFAULT_WARM_CAPACITY`, `REHYDRATE_TAIL`

`OperantAgent` exposes `session_store() -> &Arc<SessionStore>` and `session_stats()`.

## Two existing compression mechanisms — do NOT conflate them

1. `context_management.rs` — tiered eviction + decay, runs when a prompt approaches the
   context window. Reactive, per-request.
2. `agent::llm_compressor` — `compress_context_overflow`. Reactive, on provider overflow.

Your autocompactor is a **third, different thing**: a post-scheduled-execution, end-of-session
compaction that runs *after* the run is already finished, to keep the stored transcript small
and useful across jobs. Read `context_management.rs` and `agent/compress.rs` to understand the
existing machinery and to reuse its token estimation rather than writing a competing one.

**The outline explicitly says: one Autocompactor design pass exists; do not write a competing
one.** If you find an existing autocompactor design in the repo, build on it.

## The compaction floor — the point of this packet

Without a floor, a short session gets compacted into a lossy summary of a summary, and the
session loses information it never needed to lose. The floor is the minimum transcript size
below which compaction is **skipped**.

You must:

1. Expose the floor as a named, documented constant or config field with a defensible default.
2. **Justify the default from measurement or from existing constants in the repo** — do not
   pick a round number and call it done. `config.rs` has `context_window` (default 128_000)
   and tool-result budget fields; use them.
3. Make it observable: report whether compaction ran, was skipped by the floor, or was skipped
   for another reason. Silence is a lie.
4. Ensure compaction **preserves** what matters. At minimum keep the system prompt boundary
   and the most recent turns. Document exactly what is dropped.

## Semantics that are NOT negotiable

1. **Compaction happens AFTER scheduled execution, inside the persistent session.** It is not a
   pre-request compression and must not run before a job's work is done.
2. **The session must survive compaction.** After compaction the session id is the same, and a
   later acquire returns a usable transcript — not an empty one.
3. **Compaction must not silently lose the employee's turns without a record.** If you drop
   turns, leave a marker/summary that says so.
4. **Never claim `PRAGMA user_version`.** That counter belongs to the kanban family alone.

## Tests you must write

`crates/operant-core/tests/session_autocompact.rs` (you own this new test file).

Required, each with a real assertion:
- a session below the floor is **not** compacted (this is the headline case)
- a session above the floor **is** compacted
- the session id and a usable transcript survive compaction
- compaction is idempotent — compacting twice does not degrade further
- the floor default is asserted, so a silent change to it fails a test

**Mutation-prove your tests.** After they pass, break your implementation on purpose, confirm
the tests FAIL, then restore. Report the counts. A test that cannot fail proves nothing.

## Verification (run these yourself)

```bash
cd /home/ishanp/Documents/github/my-projects/agentic-harness/operant
source scripts/dev-env.sh
cargo fmt --all
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh check -p operant-core --lib
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --test session_autocompact
MEMORY_WIRE_NO_SYNC=1 ./scripts/check.sh test  -p operant-core --lib -- session::
MEMORY_WIRE_NO_SYNC=1 cargo clippy -p operant-core --lib --all-features
```

**You must not regress the 2199 passing `operant-core` lib tests or the existing
`agent_session_isolation` suite.** Run both and report the counts.

Do NOT run `cargo check --workspace` — the box is loaded and it takes 10+ minutes.

## Hard constraints

- **No `.unwrap()` or `.expect()` in production code.** The clippy gate denies both.
- Match the surrounding code style. Do not reformat files you do not own.
- Do not add features beyond what is asked.

## Final message format

Report: files created/changed, the public API the coordinator needs to wire compaction into
the scheduler, your compaction floor value and the evidence for it, test counts, mutation
results (N of M failed when broken), the confirmation that `agent_session_isolation` and the
core lib suite still pass, and the verification output tail. If you could not complete
something, say so plainly rather than describing partial work as done.
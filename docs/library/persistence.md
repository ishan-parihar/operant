# Persistence — the SQLite substrate

Owns: operant's persistence mechanisms — the state database, schema
reconciliation, migrations, trajectory export, and the RAM budget.
Not for: session semantics ([sessions.md](sessions.md)), the memory
store ([memory.md](memory.md)), the LCM DAG
([context-management.md](context-management.md)).

Source: `crates/operant-core/src/database.rs`,
`crates/operant-core/src/migrations.rs`,
`crates/operant-core/src/trajectory.rs`,
`crates/operant-core/src/tools/checkpoint_tool.rs` (shadow git),
`crates/operant-cli/src/cmd_trajectory.rs`,
`docs/MEMORY_BUDGET.md` (the measured record).

## The state database (`database.rs`)

SQLite (rusqlite, bundled) under the operant home, holding session
storage, message history, checkpoint metadata, cron/kanban/suggestion
rows, verification events, insights inputs. Schema management follows
the declarative reconciliation pattern (Python operant-state parity):
`DESIRED_SCHEMA_SQL` is the single source of truth; on existing
databases `reconcile_columns()` diffs live columns against it and adds
what's missing — no hand-written upgrade scripts. FTS5 backs transcript
search, with special-character escaping (iter-46 — a search silently
missing rows was the defect).

## The migration runner (`migrations.rs`)

Plan 004: append-only migrations keyed by `PRAGMA user_version`. The
runner walks the `MIGRATIONS` array applying any entry whose index >=
current version; each migration runs in its own transaction; on error
the version stays at the last successful index. The `MIGRATIONS` array
is append-only — never edit a shipped entry. One family per DB file is
the documented invariant (iter-333).

## Trajectory export (`trajectory.rs`)

Conversation trajectories saved for RL training data
(`~/.operant/trajectories/`, bounded by
`agent.max_trajectory_files` — 0 keeps all, and `operant trajectory
export` collects these files as prompt-completion pairs for
fine-tuning, so a bound deletes the corpus that command is for; the
operator opts in).

## Checkpoints (shadow git)

Filesystem snapshots through an isolated shadow-git store (see
[sessions.md](sessions.md) §checkpoints) — persistence for
*filesystem* state, deliberately separate from the SQLite substrate.

## The RAM budget (`docs/MEMORY_BUDGET.md`)

The measured footprint record: gateway daemon ~26.7 MB idle, TUI
~24-25.5 MB, with the caps table enforced in code (retained tool
outputs 32×128 KB, aft stderr ring 64 lines, drained child pipes, …).
`operant debug memory` prints the live numbers; daemon-side management
landed with the arena cap + debounced malloc_trim at turn end
(iter-1336/1337).

## Checkpoints and rewind

Filesystem checkpoints are the working-directory safety net: they are
on when `[checkpoints] enabled` is true, the store lives under
`[checkpoints] base_dir` (default `~/.operant/checkpoints`), and
`[checkpoints] max_snapshots`, `max_total_size_mb` and
`max_file_size_mb` bound what is captured. `operant checkpoints
status` reports the store and the current directory's count,
`list` prints the hashes, `prune [keep]` keeps the N newest
(default 10), and `clear` drops this directory's set. The
`checkpoint` tool (`ensure`, `list`, `restore`, `diff`) lets the agent
take and restore one mid-mutation. Conversation rewinding is a
different mechanism: `/rewind` opens the selector, `/rewind <n>`
targets the Nth prompt back, and the rewind is a FORK into a new
session, so the old transcript stays listed under `/session`.

## Intention

One storage engine (SQLite, bundled, no external service) with
declarative schema drift-repair instead of versioned upgrade scripts —
the same philosophy as the migration runner's append-only array:
state on disk is always forward-repairable by the running binary.
The deliberate non-goals: the transcript is NOT the memory store, the
DAG is NOT the session store — each subsystem owns its own file family
(one family per DB file), so a corrupt store never takes down two
subsystems at once.

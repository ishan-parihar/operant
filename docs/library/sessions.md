# Sessions — the conversation substrate

Owns: the per-session conversation mechanism — the retargetable session
id, branching, rewind-as-fork, scheduled autocompact, session search and
recap, checkpoints.
Not for: the gateway's channel-scoped session store
([gateway.md](gateway.md)), the memory engine ([memory.md](memory.md)).

Source: `crates/operant-core/src/session/` (mod, autocompact),
`crates/operant-core/src/session_interrupted.rs` (the exit marker),
`crates/operant-core/src/agent/rewind.rs`,
`crates/operant-core/src/active_sessions.rs`,
`crates/operant-core/src/tools/session_search_tool.rs`,
`crates/operant-core/src/session_recap.rs`,
`crates/operant-core/src/tools/checkpoint_tool.rs`,
`crates/operant-cli/src/cmd_sessions.rs`,
`crates/operant-cli/src/cmd_checkpoints.rs`,
`crates/operant-cli/src/tui/session_hub/` (mod.rs + render.rs — the
full-frame picker that replaced the 70x20 `session_browser` modal).

## The retargetable conversation

`OperantAgent` originally kept exactly one conversation in memory
(`conversation: Arc<RwLock<Vec<Message>>>`) while already carrying a
retargetable `session_id`. Every persistence site resolves its id through
`OperantAgent::session_id`, so a host can retarget the agent to another
session's history without rebuilding the agent — the session substrate
(`session/mod.rs`) made that explicit instead of accidental.

## Branching

Each branch is an independent copy of the conversation from a chosen
message point forward (`agent/rewind.rs`) — forking happens through `/rewind`,
which is also what the gateway's session store uses for switch/resume/branch on
the same substrate. The hub shows the resulting tree as `parent_id` +
`RowKind::Branch` (branch rows render with the ↳ prefix), and `/branch` (or
Ctrl+B) opens the hub on the `CurrentBranch` filter — the live session's root
and its forks (iter-1467 retired the old `session_branching` overlay and
`cmd_gateway.rs` prints those rows as bindings, not as a second browser).

## Rewind is a FORK, not a deletion (`agent/rewind.rs`)

The prefix before the chosen user prompt becomes a NEW session (with
`parent_session_id` pointing back at the old one); the old session stays
intact in `/sessions`, and the agent retargets to the fork. The old path
is never lost — plan 05's deliberate contract.

## Concurrency cap (`active_sessions.rs`)

A `max_concurrent_sessions` cap on NEW gateway turns: each distinct
session holds a lock file under `<data_dir>/sessions/active/`, refreshed
on every message, so a busy session can never be evicted by a newcomer;
when the cap is reached, new sessions are refused while existing
holders keep their slots.

## Scheduled autocompact — removed

The scheduled-run compactor was deleted in iter-2144 (it had no production
caller). See [context-management.md](context-management.md) §3.

## Session search (`session_search_tool.rs`)

Three modes over the persisted history (hermes parity): browse (recent
sessions, zero LLM cost), discovery (FTS5 search, deduped per session
lineage, with the matching message window + session bookends), scroll
(session_id + around_message navigation). The `session_recap.rs`
companion summarizes a session's recent history with PURE local
computation (see [context-injection.md](context-injection.md) §6).

## Checkpoints (`tools/checkpoint_tool.rs`)

Filesystem snapshots via an isolated shadow-git store: every
file-mutating tool run can be preceded by a snapshot, committed to a
private store under `~/.operant/checkpoints/`; explicit
`checkpoint ensure/list/restore/diff` for the model and
`operant checkpoints status/list/prune/clear` for the operator.

## The interrupted-session signal

Two records, one generation each, and neither is a cursor.

**The exit marker** (`session_interrupted.rs`) is a small JSON file under
the sessions directory holding the session id, a timestamp, a title, and a
version. The TUI's teardown writes it on an interrupted exit; a deliberate
`/exit` deletes it first, so the marker means "this was cut off" and never
"you chose to stop". `consume_marker` reads and deletes in one act, so the
resume offer survives being acknowledged exactly once — a notice that
cannot be dismissed stops being a notice. A corrupt marker reads as
nothing: the operator's session list is the durable source and a bad
sidecar file must never stop a launch. An empty session id is refused
outright, because it would match nothing.

**The DB flag** rides the existing `end_reason` column as
`Database::INTERRUPTED_END_REASON` — no schema change and no migration.
`list_sessions_rich` derives `RowStatus::Interrupted` from it, ranking below
`Archived` (a parked row is a deliberate state) and above `Idle` (an
interrupted row is a resumable accident, which is the more useful answer).
`clear_session_interrupted` is scoped to the interrupted value rather than
nulling the column, so a real `"completed"` survives. A session's next turn
clears it after `save_session`, which is what stops every relaunch
re-claiming every old session as interrupted.

**The notice never auto-attaches.** Both the TUI and the gateway name the
interrupted session and stop there. Ctrl+C is how an operator escapes a
turn that is going badly; silently reattaching them to the session they
abandoned would be the opposite of a rescue.

**Liveness.** `active_sessions.rs` gained a cross-process presence
registry (`claim_session_here` / `session_is_live` / `prune_dead_presence`)
distinct from the file-based concurrency cap it already held, which tracks
sessions one *process* owns and says nothing about other processes.
`claim_session_here` is the single entry point a live process calls: it
prunes other processes' dead records, then claims for its own pid. Without
it the notice cannot tell a marked session whose process died from one
still streaming in another window — and offering to resume the latter is
an invitation to race on the same rows. `process_is_running` uses
`fs::metadata` on `/proc/<pid>` rather than `libc::kill(pid, 0)` because
`libc` is not an `operant-core` dependency; the tradeoff (EPERM and ENOENT
are indistinguishable) is written down in-code and fails toward the
redundant notice, never toward a wedged self-registration.

## Intention

Sessions are the unit of *user* continuity (memory is the unit of
cross-session continuity). The substrate exists so history is never a
singleton: branching, rewind, gateway channel sessions, and the
autonomous loop all retarget one agent instead of forking processes.
Rewind-as-fork and the eviction-proof concurrency cap both come from the
same law: never destroy a conversation the user may still want.

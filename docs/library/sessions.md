# Sessions — the conversation substrate

Owns: the per-session conversation mechanism — the retargetable session
id, branching, rewind-as-fork, scheduled autocompact, session search and
recap, checkpoints.
Not for: the gateway's channel-scoped session store
([gateway.md](gateway.md)), the memory engine ([memory.md](memory.md)).

Source: `crates/operant-core/src/session/` (mod, autocompact),
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

## Scheduled autocompact (`session/autocompact.rs`)

The third compaction mechanism — see
[context-management.md](context-management.md) §3 for the mechanism and
why it is a third one and not a second.

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

## Intention

Sessions are the unit of *user* continuity (memory is the unit of
cross-session continuity). The substrate exists so history is never a
singleton: branching, rewind, gateway channel sessions, and the
autonomous loop all retarget one agent instead of forking processes.
Rewind-as-fork and the eviction-proof concurrency cap both come from the
same law: never destroy a conversation the user may still want.

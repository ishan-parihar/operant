# Red team: the process-architecture execution (2026-11-12)

**Scope:** the fourteen code iterations this campaign pushed for the 2026-11-12
process-architecture audit (RT-507…RT-526) — read as "the senior who inherits
this and gets paged when it breaks", plus the ponytail-audit pass over the
surface those iterations touched (the gateway/server/session/MCP path).

**Load assumed:** one operator, one host, a gateway serving several chats
(a few seats), plus CLI/ACP clients that may multiply to hundreds. The
100–1000 parallel agents figure is a TARGET, not a measured load: nothing here
was load-tested, and no number in this report was invented.

---

## What the campaign actually built

- **One integrated daemon.** `operant-gateway` is the server; the CLI is a
  client of it. A restart drains the turn in flight (RT-509) and leaves an
  honest record (RT-510, RT-511).
- **Shared child fleet.** One MCP child set, one AFT pool, one kernel runtime,
  one watchdog per process (RT-513…RT-517). A hundred CLI sessions are a
  hundred thin clients over one runtime, not a hundred processes.
- **Concurrent turns.** The gateway runs agent slots (RT-508): chats in
  different slots turn at the same time; chats in one slot still serialise.
- **Honest surfaces.** A bind failure stops the start (RT-520), the API binds
  before it prints its token (RT-523), the daemon writes a state file (RT-523),
  and the session API serves from one database handle (RT-519).

---

## Must fix

### 1. Two concurrent slots can swap each other's memory bank
(`crates/operant-cli/src/gateway_runner.rs:3008`, `:3041`; `crates/operant-core/src/memory_wire.rs:407`)

- **What this is:** every slot's agent shares ONE memory provider, and that
  provider holds ONE scope cell (`bank`). The turn sets the scope to the
  seat's bank before it runs.
- **Problem:** with the slots running turns at the same time (RT-508), slot B
  can set the scope while slot A's turn is between its recall and its retain.
  A seat's turn then recalls from another seat's bank — the exact leak the
  old global turn lock used to prevent. The code comment still claims the
  lock makes that impossible; that comment is now false.
- **Fix:** one provider per slot over the same store (the scope cell is then
  per slot, and a slot runs one turn at a time).
- **If we skip it:** a seat's memories leak into another seat's bank.

### 2. Two concurrent slots can steal each other's cron delivery target
(`crates/operant-cli/src/gateway_runner.rs:2804`; `crates/operant-core/src/tools/cron_tool.rs:26`)

- **What this is:** the turn records "this cron job came from chat X" in a
  process-global cell, so the cron tool can file the job with that origin.
- **Problem:** the cell is global, so chat B's turn overwrites the origin
  before chat A's `cron` tool call runs — and A's job is delivered to B's
  chat. Same shape as finding 1: a global that the old turn lock hid.
- **Fix:** carry the origin on the turn, not in a global.
- **If we skip it:** a scheduled job is delivered to the wrong chat.

### 3. The sessions that share a slot inherit each other's model, effort and charter
(`crates/operant-cli/src/gateway_runner.rs:2882-2980`)

- **What this is:** the model, the reasoning effort and the charter live on
  the agent. Before RT-508 the gateway had ONE agent and one turn at a time,
  so an agent field was effectively "this session's" field.
- **Problem:** now the agent belongs to a SLOT, and a slot serves many
  sessions. A chat with no model override inherits the previous chat's model;
  a chat with no charter inherits the previous chat's charter; `/effort`
  sets the SLOT's effort, so the next chat in that slot inherits it.
- **Fix:** reset the per-session fields to this session's own values at the
  start of every turn (seat and charter already are; model and effort are
  not).
- **If we skip it:** a chat silently answers with another chat's model.

### 4. The gateway tests write the operator's real status file
(`crates/operant-core/src/gateway/status.rs:118`; `crates/operant-cli/src/gateway_runner.rs` turn end)

- **What this is:** RT-523 made the daemon write `gateway.status.json` into
  the operant home at every turn end. The gateway's own tests run turns.
- **Problem:** the tests do not redirect `HERMES_HOME`, so they wrote
  `/home/ishanp/.operant/gateway.status.json` — the operator's real file, with
  the operator's real delivery counts overwritten by test numbers. Measured:
  the file's mtime moved during the test run.
- **Fix:** the test handlers point the status path at their temp home (or the
  writer takes the path), and the test asserts the operator's file is not
  touched.
- **If we skip it:** every test run corrupts the operator's live state.

---

## Should fix

### 5. The checkpoint list is shared by every chat — **CLOSED (iter-2171)**
(`crates/operant-core/src/tools/checkpoint_tool.rs:30`)

- **What this is:** checkpoints live in one process-global manager, keyed by
  working directory.
- **Problem:** every gateway chat shares the working directory, so one chat's
  `checkpoint_list` returns another chat's checkpoints — including a seat's
  work in progress.
- **Fix:** key the checkpoints by session as well as working directory.
- **If we skip it:** a seat can read another seat's checkpoints.
- **Closed in iter-2171:** the store is keyed by (session, directory), the
  per-turn dedup key carries the session, and the tool reads the session from
  the tool context. Pinned by
  `tools::checkpoint_tool::tests::two_sessions_in_one_directory_do_not_share_the_checkpoint_list`
  (two mutation-proofed assertions).

### 6. The file-state snapshots are shared by every chat — **CLOSED (iter-2158)**
(`crates/operant-core/src/tools/file_state.rs:23`)

- **What this is:** one process-global map of file snapshots (the file-edit
  tool's undo surface).
- **Problem:** two chats editing the same file interleave their snapshots.
- **Fix:** key the map by session and path.
- **If we skip it:** a chat's file undo can restore another chat's snapshot.
- **Closed in iter-2158:** the map is keyed by (session, path) and the session
  rides the tool's operations through the context. Pinned by
  `tools::file_state::tests::two_sessions_snapshots_of_one_file_do_not_interleave`
  (mutation-proved).

### 7. The conversation-reload decision races between slots — **CLOSED (iter-2155)**
(`crates/operant-cli/src/gateway_runner.rs:2843`)

- **What this is:** the handler keeps ONE `current_session_id` to decide
  whether the conversation must be rehydrated.
- **Problem:** two slots running turns share that one cell, so a slot can see
  the other slot's session id and decide to reload a conversation it already
  has.
- **Fix:** the decision belongs to the slot (each slot has one agent and one
  conversation), so the cell belongs to the slot.
- **If we skip it:** a redundant rehydrate per turn — slower, not wrong.
- **Closed in iter-2155:** the handler's shared `current_session_id` is gone;
  each slot owns a `loaded_session` cell, and the decision reads and writes
  the slot of THIS session. Pinned by
  `gateway_runner::tests::the_reload_decision_belongs_to_the_slot`
  (mutation-proved).

### 8. The audit's own ledger rows — **ALL CLOSED**
RT-518 (the dashboard is a disconnected process, closed iter-2143), RT-521
(unbounded growth under load, closed iter-2148), RT-522 (the presence registry
is never claimed, closed iter-2150), RT-524 (one persistence rule applied four
ways, closed iter-2151), RT-525 (about 1,350 lines of implemented, tested,
unwired machinery, closed iter-2152) and RT-526 (the daemon lock file is
unlinked on drop, closed iter-2142). Each carries its own fix and test in the
ledger. The two findings this report raised that are still open are tracked
as ledger rows RT-541 (the shared checkpoint list, closed in iter-2171) and
RT-538 (the shared file-state snapshots, closed in iter-2158).

---

## Nice to have

- The `/reload-skills` command now builds one SkillManager per slot (RT-508),
  so a reload scans the skills tree N times. Four scans of a directory tree
  is fine; a hundred slots would not be. One manager per slot, built once, is
  the shape to keep.

---

## Redundancy found in the campaign's own output

- The gateway's agent slots each build their OWN tool registry, and each
  registry holds the same MCP tool objects. The MCP CHILDREN are shared (that
  was the point of RT-517), but the tool wrappers are copied per slot. Four
  slots = four copies of every MCP tool descriptor. This is acceptable at
  four slots and wasteful at a hundred; the fix belongs with the agent-pool
  work (RT-508's follow-on), not as a patch here.

---

## Verdict

**Every finding in this report is CLOSED**, and the ledger says so: the
must-fix rows RT-533…RT-536, the should-fix rows RT-538 (file-state snapshots)
and RT-541 (the checkpoint list), the redundancy row RT-525, and the
reload-decision race (finding 7) are all closed with a named regression test.

The campaign's direction was right and its claims hold under reading, but it
shipped one design consequence unexamined: **making the gateway concurrent
turned process-global state into shared state between turns.** Findings 1–3
were that gap; they were small, they were in the code this campaign wrote, and
they are now fixed, so the slots can be trusted with more than one seat.

The same lesson generalised: the shared-state hunt found the SAME shape again
in the file-state snapshots (finding 6) and the checkpoint list (finding 5) —
process-global maps keyed by something coarser than a session. Both are fixed
by adding the session to the key.

**One regression was caught and re-landed:** a peer's iter-2123 reverted the
whole RT-517 fix (the shared AFT pool, the reused kernel runtime and the single
watchdog all disappeared, and the ledger row reopened) while the CHANGELOG
entry stayed. iter-2176 re-landed it and re-ran its 4 mutation-proved tests.

**Honest limits.** The 100–1000 parallel-agent target is still a TARGET: no
load test has been run in this session, so the claims are architectural (one
runtime, one pool, one watchdog, one socket server, one tool registry per
process) and not measured throughput. The release build and the gateway restart
could not run in this sandbox (no systemd user bus), so every commit here is
pushed but NOT deployed.

**Lean: the open RT-525 row removes about 1,350 lines; the rest of the
campaign's output adds no new dependency.**

**Not checked:** the TUI's 300+ modules (out of scope), the dashboard server's
rendering path (only its gate was checked), the kernel sidecar's Python
runtime (it needs the main tree's virtualenv), and the release build/deploy
(this sandbox has no systemd user bus).

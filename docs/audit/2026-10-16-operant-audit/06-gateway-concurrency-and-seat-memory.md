# 06 — Gateway: one turn at a time on the shared agent; role-specific memory banks

**Size:** M. **Findings:** 1, 14 in `01`. Part A is urgent, because the
workstation gateway runs this code every day.

## Part A — the shared-agent race

### Evidence

- One agent for every chat: `GatewayMessageHandler { agent: Arc<OperantAgent>, … }`
  (`crates/operant-cli/src/gateway_runner.rs:783-784`).
- Every incoming message is its own task: `tokio::spawn(async move { … gw.route_message(msg) … })`
  (`gateway_runner.rs:3707`, inside the dispatch loop).
- The only serialization is **per chat**:
  `self.turn_lease.try_acquire(session_key)` (`crates/operant-core/src/gateway/mod.rs:255-266`).
  Two different chats run at the same time.
- Inside `handle`, the agent is retargeted with no lock held across the run:
  `self.agent.set_session_id(session_id)` (`gateway_runner.rs:936`), then the
  model override `self.agent.set_model(...)` (`:1019-1026`), then
  `self.agent.run(query)` (`:1275-1278`). `current_session_id` is locked
  only briefly (`:961-975`).
- `set_session_id` **swaps the single hot conversation**: it saves the outgoing
  one and loads the incoming one (`crates/operant-core/src/agent/events.rs:713-760`).
  `add_message` writes to that one slot (`events.rs:188-191`).
- Stop: `let stop_flag = handler.agent.interrupt_flag(); stop_flag.reset(); turn_inflight.insert(session_key, stop_flag);`
  (`gateway_runner.rs:3694-3697`). That is one flag for all chats.
- Process-global per-turn state, also raced: `set_cron_origin(...)` (`gateway_runner.rs:3910-3916`).
- BUGS.md W3-1 ("FIXED iter-548") fixed *sequential* switching only.

### Failure case (concrete)

1. Telegram DM (session A) sends a prompt. The turn runs for 40 s with tools.
2. At t=5 s a Discord message (session B) arrives. Its task calls
   `set_session_id(B)`. A's in-flight messages go back to the store, and B's
   history is loaded into the hot slot.
3. A's loop appends its tool results and answer **into B's conversation** and
   persists them under B's id (`session_id()` now returns B).
4. B's `stop_flag.reset()` clears a `/stop` A had just sent. A `/stop` in B
   kills A's turn too.
5. If B has a `/model` override, A's remaining iterations use B's model.

### Fix (smallest correct rung)

One field on `GatewayMessageHandler`: `turn_lock: tokio::sync::Mutex<()>`.
At the top of `handle`, take `let _turn = self.turn_lock.lock().await;`
before `set_session_id` and hold it until `handle` returns. Then put the
cron-origin set inside the lock too: move the `set_cron_origin` call from the
dispatch closure into `handle`, after the lock.

```rust
// ponytail: one gateway turn at a time on the shared agent. Upgrade path:
// an agent per seat/session (create_runtime_agent per key) if a busy chat
// starves others — measure queue wait first.
```

The per-chat lease stays (it gives the polite "still working" reply for the
*same* chat). Other chats now **queue** instead of corrupting each other. Show
the queue: if `try_lock` fails, send the platform "typing…" indicator. Most
adapters already have one; grep `send_typing`.

Stop flag: with the lock, only one turn runs at a time, so the shared flag is
correct again. Keep `reset()` but move it *inside* the lock, at the turn start.

Cron runs on its own agent (BUGS D-3), so it's not affected. **Check** whether
the cron scheduler can run two jobs at once on *its* agent (`crates/operant-core/src/cronjobs/scheduler.rs:503`
calls `set_session_id`). If it can, take the same lock pattern there. I did not
check this (D-6 is open in BUGS.md).

### Tests first

In `operant-core/src/gateway/mod.rs` tests (the shape of
`route_message_serializes_concurrent_turns_via_lease`, `:1272`), or in
`gateway_runner.rs` tests with a mock agent client:
- Two sessions, both sent at once. The mock model's first response sleeps
  200 ms. Assert that each session's stored conversation contains **only its
  own** user message and answer. RED today.
- Session A in flight, `/stop` arrives for B → A completes normally. RED today.

### Acceptance

- Both tests green.
- Live: two chats (for example Telegram DM + a Telegram group), prompts sent
  within a second → each answer matches its own prompt, and `operant sessions show`
  of each contains only its own rows.

## Part B — role-specific memory banks (11E remainder, re-scoped)

### Boundary statement (answers the 2026-10-15 brief, checklist item 2)

> **Personal mode:** memory is memory-wire, one store, banks by identity.
> The owner's own turns (TUI, owner DMs) use the default bank. A seat
> (`seat_id()` set: a cast employee or bound gateway seat) uses `emp:<id>`
> and never reads or writes another seat's bank. The default bank is never
> visible to a seat unless the seat is the owner. **Org (Holon) mode:**
> a Holon run never writes any operant memory bank (O-22). Learnings travel as
> `result.json` annotations. No `emp:` bank is consulted in a Holon run, because
> Holon owns org memory.

### Evidence

- `bank_for` / `MemoryScope` / `MemoryWireProvider::scoped` exist
  (`crates/operant-core/src/memory_wire.rs:85-95,274-283`). The only callers
  are tests (`memory_wire.rs:713-769`).
- `scoped()` calls `Self::new(storage_dir)`, which opens a **new store per
  provider**. That works for a one-off, but it is the wrong shape for a per-turn switch.
- The agent gets one provider at build (`with_memory_provider`,
  `agent/builders.rs`). The gateway agent therefore recalls and retains in one
  bank for every chat and seat.

### Fix

1. `MemoryWireProvider.bank: String` → `bank: std::sync::RwLock<String>`, plus
   `pub fn set_scope(&self, scope: &MemoryScope)`. `scoped()` becomes
   `new` + `set_scope` (one store; the bank is only a key).
2. The agent exposes `memory_provider()` (it exists, `builders.rs`). In
   `GatewayMessageHandler::handle`, **inside Part A's lock**, right after
   the seat binding is applied (`gateway_runner.rs` ~1035-1060, "Wave 2 …
   employee binding"), call
   `set_scope(&MemoryScope::Seat { identity })` when the turn runs as a
   non-owner seat, else `&MemoryScope::Personal`. The provider is behind
   `dyn MemoryProvider`, so add `fn set_scope(&self, _: &MemoryScope) {}` as a
   default trait method. One trait method, no new type.
3. Holon runs: assert in `run_env` setup that retain is disabled (O-22 already
   says "never write"). Add a test that a Holon-run agent's provider rejects
   or no-ops `retain`. If W6/W7 already pins it, cite that test and skip.
4. Personal-mode seat memory files (the 11E plan's item b): the one-time
   importer that moved MEMORY.md (11C3) runs for the default bank only. If seat
   memory files still exist on disk (`~/.operant/org/...`, check), import each
   into its `emp:<id>` bank once with the same importer, else drop the item.

### Tests first

- Gateway turn as seat `emp-a` retains "A-fact". The next turn as seat `emp-b`
  recalls with the query "A-fact" → nothing. A personal turn → nothing. RED
  today (one bank).
- The existing `solo-never-sees-emp:*` test stays green.

### Acceptance

`operant debug memory` (the plan 07.1 surface) is extended to print per-bank
counts (it may not do so today, so check), and the live gateway shows `emp:*` banks
growing per seat.

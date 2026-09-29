# Defect & Gap Plan — 2026-09-29

Everything here is backed by a measured observation. Nothing is speculative.

## Tier 1 — data loss / undebuggable

### D1. Telegram startup probe discards pending updates
`crates/operant-core/src/gateway/telegram.rs:692-707`

```rust
if let Some(updates) = data["result"].as_array() {
    for update in updates {
        if let Some(update_id) = update["update_id"].as_i64() {
            offset = update_id + 1;   // reads the message, then drops it
        }
    }
}
```

The probe exists to advance the offset past updates it cannot process, so
returning `[]` is the *safe* fix. The alternative — dispatching them — would
need a re-entrancy guarantee.

**Severity:** silent data loss. Anything queued while the gateway is down is
consumed unread.

**Cost:** one line. Serve `[]` from the probe and let the real poll loop (which
carries the offset) pick the updates up on its next iteration.

### D2. Gateway logs 5 lines and no tracing subscriber
`operant gateway run` → 2 banner lines, then nothing. `RUST_LOG=debug` has no
effect because no subscriber is initialised on this path. Every hop had to be
recovered from the transport's own view instead of the log.

**Cost:** small, but it's what made the other three defects slow to find.

## Tier 2 — the loop has no pre/post-message pipeline

Not bugs — **absent functionality**. The gateway receives, gates, runs one agent
turn, replies. There is no post-turn processing of any kind.

| Expected | Actual |
|---|---|
| pre-instruction | 0 files — does not exist |
| post-message | 0 files — does not exist |
| skill / systemic revision | 0 files — does not exist |
| curator | exists, **0 calls from `gateway_runner.rs`**; manual `/curator` only |
| `sync_turn` | runs every turn (`agent/mod.rs:360`) |

**This is the answer to "are the sub-agent processes running effectively":
they are not running, because they were never built.**

## Tier 3 — memory persists nothing

`sync_turn` → `add_message` → appends to `session_messages`
(`memory_provider.rs:363`). Only `write_memories` / `save_to_disk` touch disk.
`build_memory_provider`'s builtin branch *does* pass `with_storage_dir`
(`memory_provider.rs:432`), so the storage dir is real — the write path just
never runs.

**Consequence:** every gateway turn's memory is discarded on exit. A long-lived
bot has no recall.

**Note:** `~/.operant/operant.toml` has `provider = "builtin"`. AGENTS.md names
agentmemory the default; that divergence is itself worth resolving.

## Tier 4 — security follow-up

### D3. Bot token exposed
The Telegram bot token appeared in shell output during the mock work and in
this session's transcript.

**Action:** rotate in @BotFather. Not code — operational.

### D4. Fail-open allowlist remains fail-open
`gateway_runner.rs:1846` — `if !admins.is_empty() && !admins.contains(...)`.
An empty `admins` blocks nobody. iter-436 made it *loud* (startup warning); it
did not make it fail-closed, because inverting would silently break every
install that never set `admins`.

A fresh install re-opens the exposure each time. The structural fix is a
`dm_policy` (admins_only | allow_list | open) defaulting to `admins_only` — a
migration and a user decision, not a patch.

### D5. No per-platform id scoping
The gate compares `msg.user_id` with no platform scoping, so a Discord id never
matches a Telegram user. A shared allowlist silently locks one platform out.

## Not yet tested

- **Permission-gate path in the live loop.** Both real turns used `aft_bash`
  and auto-approved. The 60s auto-deny at `telegram.rs:1666` and the
  `/approve` / `/deny` text paths are read from code, not observed.
- **`setMyCommands` at boot** (`gateway_runner.rs:958`) hits real
  api.telegram.org with 5 retries + backoff and bypasses `telegram_api_base`.
  Untested under a slow network.
- **`sendVoice` at `telegram.rs:934`** hardcodes api.telegram.org.

## Suggested order

1. D1 — one line, silent data loss
2. D2 — unblocks diagnosing everything else
3. D3 — rotate now, costs nothing
4. D4 + D5 — needs your decision
5. Tier 2 / 3 — feature work, not defects

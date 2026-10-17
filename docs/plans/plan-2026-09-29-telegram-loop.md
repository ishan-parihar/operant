# Defect & Gap Plan — 2026-09-29

Everything here is backed by a measured observation. Nothing is speculative.

## Tier 1 — data loss

### D1. Telegram startup probe discards pending updates — fix in 47f318a4, POST-FIX UNVERIFIED
`crates/operant-core/src/gateway/telegram.rs:692`

The probe posted `{offset:0, timeout:0}`, read the result, and advanced
`offset = update_id + 1`. The real poll loop is the sole dispatcher, so the
first real poll after a boot would skip exactly the updates the probe had seen.

**Mechanism confirmed in code** (`telegram.rs:874`): the offset file is persisted
only `if had_updates`. The probe raises the offset, the real poll then receives
an empty result, so nothing is ever written.

**Evidence status — read this before citing it.** The defect is confirmed in the
mock, where the probe demonstrably consumed an update the poll never saw. It is
**not** confirmed against production, and neither of the two lost test messages
is evidence for it:

- 11:15:51 — received, allowlisted, `Starting agent run`, then killed 5.2s later
  with the process. Never skipped; see D2.
- 11:26:06 — the log has **zero lines between 11:16 and 11:33**, so no gateway
  was running. There was no probe to consume it. The message was simply
  unreachable, then picked up on a later boot.

Attributing either message to D1 was wrong. The only remaining production
reproduction is the `OFFLINE_QUEUED_TEST_441` run, which was ambiguous (the
"1 message stored" count was `tg history`'s fetch total, not proof of silence).

**Outstanding:** the fix is committed but not built or deployed — the installed
binary is pre-fix. A post-fix stop → queue → start run is required before D1 can
be called verified.

### D2. In-flight turns are lost when the process dies — **the real cause of the 11:15 stuck turn**
`~/.operant/logs/gateway.log` is the ground truth (45 MB, 867,909 lines, full
structured tracing with span fields). An earlier note in this doc claimed the
gateway initialised no tracing subscriber; that was wrong — it was reading a
foreground stdout redirect, not the log.

```
11:15:51.552  Starting agent run
11:15:56.793  Stopping platform adapter, platform: telegram
11:15:56.796  Telegram adapter stopped
11:34:25.005  WARN Detected interrupted turn from previous session
```

The turn ran 5.2s and was terminated with the process. The gateway *detects*
this on the next boot and warns (`gateway_runner.rs:3012`), but **it never tells
the user** — the handler pushes `(channel_id, timestamp)` into a list and
returns; nothing sends a message to that channel. So the user sees silence for
18 minutes with no explanation anywhere except the server log.

That is the concrete fix: on detecting an interrupted turn, notify the channel
("your previous turn was interrupted when the gateway restarted").

```rust
if let Some(updates) = data["result"].as_array() {
    for update in updates {
        if let Some(update_id) = update["update_id"].as_i64() {
            offset = update_id + 1;   // reads the message, then drops it
        }
    }
}
```

The probe exists to report whether anything is pending. The real poll loop
below it — same URL, same `offset` — is the **sole dispatcher**: it iterates
`result` and sends each update to `message_tx` (`telegram.rs:789`).

So the probe's `offset = update_id + 1` is the entire bug. It made the first
real poll skip precisely the updates the probe had just seen.

**Fix:** delete the offset assignment. The probe then logs, and the next poll
re-fetches at the unchanged offset and dispatches. This is client-side only —
returning `[]` server-side would do nothing on its own, because the client is
what advances the offset.

**Severity:** silent data loss. Anything queued while the gateway is down is
consumed unread.

**Cost:** one assignment removed.

### D3. Gateway logs 5 lines in the foreground
`operant gateway run` prints a two-line banner to stdout, which looks like an
unloggable binary. It is not: structured logs go to `~/.operant/logs/gateway.log`
with span fields (`in run with user_query:`, `in execute with tool_name:`).
Reading the banner instead of that file is what made several findings here
initially wrong.

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

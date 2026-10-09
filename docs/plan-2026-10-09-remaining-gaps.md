# Remaining implementation gaps — 2026-10-09

> Supersedes the execution queue in `plan-2026-10-08-remaining-implementation-gaps.md`
> (all its items are done/decided; only its "Item 7" survives here, re-scoped).
> Owner supplied a live Telegram bot token (bot **8916661121 @ip_zeroclaw_bot**,
> "Zeroclaw") on 2026-10-09; this outline reflects what that changed and what
> live verification on the box actually showed. Every claim below was measured,
> not inferred.

## 0. Live state as of this writing (verified)

- **Delivery hop CLOSED.** `getMe=200` (Zeroclaw), `getChat(5297486612)=200`
  (admin private chat), `sendMessage` DM delivered (message_id 40). The old
  "stale channel ref / empty token" blocker is retired. Token installed in
  `~/.operant/.env` (old Operant Testing Bot token kept as commented fallback;
  `.env.bak-20261009` snapshot).
- **Inbound DM pipeline works end-to-end.** Test DM (sent via the owner's `tg`
  MTProto CLI as 5297486612) → gateway long-poll consumed it → agent turn ran
  (small-stack) → 730-char reply delivered to the admin (message_id 46). Log:
  `~/.operant/logs/gateway.log` @ 22:35Z.
- **Outbound is healthy; inbound *capture* is not** — two bugs below.

## 1. Telegram offset store is not keyed by bot (found live; S) — FIXED iter-704 `0373bb15`

- **Operational patch (applied)**: reset `~/.operant/telegram_offset.txt` to 0
  and restarted the daemon (`Loaded saved offset: 0` in the log).
- **Code fix (deployed 2026-10-09)**: `get_offset_path_for(bot_id)` keys the
  cursor `telegram_offset.<bot_id>.txt`; unit test
  `offset_paths_are_keyed_per_bot` pins distinct bots to distinct paths.
  Live-verified: the daemon writes `telegram_offset.8916661121.txt`.

## 2. Gateway session entry is never created for a Telegram DM → DM tap and metering both miss (found live; S–M) — FIXED iter-704 `0373bb15`

- **Root cause (sharper than first filed)**: key-namespace split. The store
  files every session under `build_session_key` (`agent:main:telegram:dm:…`),
  while the turn path looked rows up by its turn-lease key
  (`telegram:5297486612:5297486612`) — structurally never a match, so the
  Wave-2 employee binding, the iter-684 DM tap, and Wave-4 metering were
  ALL dead for platform chats from the start. Live evidence: agent replied
  fine but `context_items` stayed 0 and metering logged `Session not found`.
- **Fix (deployed, live-verified)**: the turn path resolves through
  `entry_for_source` — the same origin-triple seam `/session` uses — and
  carries the canonical key to the metering drain. Regression pin:
  `wave2_store_files_dm_sessions_not_under_the_turn_lease_key`.
- **Live proof 2026-10-09**: two test DMs → two `dm` rows, `seat_hint=premiere`,
  author `ishan_parihar`; the metering warn is gone post-restart; the
  `premiere|dm` watermark advanced.

## 3. Feed class: channel/group posts → `context_items` — EXECUTION PLAN (M)

**State change since first filing**: inbound is live end-to-end
(iter-704). The gateway's poll is the single `getUpdates` consumer —
the feed class rides it; NO second poller (a separate one 409s against
the daemon, observed live 2026-10-09).

**Seam ruling (review-verified)**: capture goes at the TOP of
`route_message` (gateway/mod.rs:345), BEFORE the admin check at :412 —
group/channel posts come from arbitrary users, and capturing inside
`MessageHandler::handle` would only ever see the operator's own posts
plus draw "You are not authorized" replies into channels. A new
`MessageHandler::record_feed(&IncomingMessage)` trait method with a
NO-OP DEFAULT (contract NOT widened; existing `handle` untouched).

### Slice A — the feed aspect in the injector (pure core, no gateway)

1. `ContextClass::Feed` + `as_str()="feed"` + `quota_for` arm
   (`context_injection.rs:73-90`).
2. `feed_seat_map: HashMap<String,String>` + `feed_quota: usize`
   (default 500) in `ContextInjectionSettings` (`config.rs:188`);
   `operant.example.toml` gains both keys. Unmapped chat → `premiere`
   (consistent with the DM default binding).
3. `collect_class` Feed arm: `SELECT author, ts, text FROM context_items
   WHERE class='feed' AND seat_hint=? AND ts>watermark` (mirror the Dm
   arm at :430).
4. `record_feed_item(seat_id, author, text)` mirroring `record_inbound_dm`
   (:389) — class 'feed', truncate 800, fail-open.
5. Render label: `ContextClass::Feed => "Channel feed"` (pattern: Dept →
   "Department feed", :690).
6. **Tests**: feed quota fill + roll-over into pool; record → collect →
   watermark advance round-trip; unmapped-default-premiere mapping.

### Slice B — capture at route time (adapter + routing + handler)

1. `IncomingMessage.is_channel_post: bool` (default false) +
   `.with_channel_post()` builder (`gateway/types.rs:100`).
2. `parse_update` (`telegram.rs:1444`): accept `channel_post` as an
   alternative top-level update key (same body minus `from`; author =
   channel title from `chat.title`); mark `is_channel_post=true`.
   Group/supergroup messages keep existing behavior — they ALREADY
   parse with `is_group_chat=true`.
3. `route_message` top (mod.rs:345, before the admin check):
   - `is_channel_post` → `handler.record_feed(&msg)` then `Ok(None)`
     (no turn, no reply — a channel post never spawns an agent turn
     and never answers into the channel).
   - `is_group_chat` → `handler.record_feed(&msg)` then FALL THROUGH
     to normal routing (the operator's group-command surface keeps
     working; feed capture is additive).
4. `GatewayMessageHandler::record_feed` override (gateway_runner.rs):
   seat = `settings.feed_seat_map.get(channel_id)` else `premiere`;
   `injector.record_feed_item(...)`; fail-open on every error.
5. **Tests**: `parse_update` channel_post → `is_channel_post=true` +
   author=title; route_message feed-branch unit (stub handler counting
   `record_feed` calls — channel post records + returns None, no
   "not authorized" outgoing); group message records AND still turns.

### Slice C — live verification (owner action prerequisite)

Add @ip_zeroclaw_bot to a channel (or group) and post. Verify on the box:
`context_items` gains a `feed` row with the mapped seat,
`context_watermarks` gains `(seat,'feed')`, and the next turn prologue
of that seat renders the Channel feed section. Until the owner adds
the bot to a chat, Slices A+B are proven by unit tests + deploy only —
state that plainly in the iteration report.

### Slice D — docs

CHANGELOG entry + example.toml keys + this row → EXECUTED. The stale
`Channel: chat_636bbfc5f7ee` preamble label is RETIRED (not a bug):
`build_session_context` routes channel ids through
`pii::redact_chat_id` (gateway_runner.rs:1705) — `chat_…` is the
redactor's stable alias, by design.

**Order**: A → B → deploy → C (owner) → D. One code iteration (A+B),
the live verify gated on the owner, the docs row close-out rides the
same commit.

## 4. Discord/Slack read adapters — still blocked on owner credentials

- `discord_enabled=false`, `slack_enabled=false`, both tokens unset
  (re-verified 2026-10-09). Same rule as before: no adapters against dead
  credentials. Blocked, not scoped.

## 5. Small disclosed items (not implementation)

- Wave-4 metering fail-open warning shares root cause with item 2 — fixed by
  it; do not patch separately.
- Peer's two clippy `expect()` deny sites in `agent/stream.rs:524/:544` —
  theirs (rule 7); annotate only if still present after their TUI work
  settles.
- Owner flips pending: `[socialization] enabled=true` (arms the completed
  09:30 sessions); `~/.operant/backups/packet-e-wt-wip-20261007.tar.gz`
  deletion sign-off.
- Dispatcher standing duty: the audit artifact's `telegram_status` header
  should now record the Zeroclaw hop as live (getMe/sendMessage 200) at the
  next cycle; the old "stale channel ref" blocker line is obsolete.

## Execution order

3 (slices A→B→deploy, C gated on the owner) → 4 waits on the owner.
Items 1+2 are DONE (iter-704, live-verified). Each code slice ships as
one iteration: fix + test + deploy + live-verify per AGENTS.md.

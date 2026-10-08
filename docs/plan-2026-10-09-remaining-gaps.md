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

## 1. Telegram offset store is not keyed by bot (found live; S)

- **Bug**: `get_offset_path()` (`gateway/telegram.rs:410`) returns
  `telegram_offset.txt` in the daemon's cwd — one global offset, no bot id.
  Swapping the token made the new bot inherit the old bot's offset
  (`505536029`); a fresh bot's update ids start near 1, so
  `getUpdates?offset=505536029` **silently skipped every inbound DM**. This is
  why the gateway saw nothing for ~8 minutes today.
- **Operational patch (applied)**: reset `~/.operant/telegram_offset.txt` to 0
  and restarted the daemon (`Loaded saved offset: 0` in the log).
- **Code fix owed**: derive the offset filename from the bot id (token segment
  before `:`), e.g. `telegram_offset.<bot_id>.txt`. One seam change + a unit
  test asserting distinct tokens map to distinct paths. Also documents the
  operational hazard: swapping bots without resetting the offset file drops
  all inbound until offset catches up (never, for a fresh bot).
- **Effort**: S. Files: `crates/operant-core/src/gateway/telegram.rs`.

## 2. Gateway session entry is never created for a Telegram DM → DM tap and metering both miss (found live; S–M)

- **Bug**: the iter-684 capture-at-inbound tap (`gateway_runner.rs:1069-1073`)
  fires only when the persistent session store has an entry for the session
  key (`telegram:5297486612:5297486612`) so `bound_employee` resolves. Live
  test: two DMs → agent replied to both → `context_items` stayed at **0**.
  The Wave-4 metering write failed with `Session not found:
  telegram:5297486612:5297486612` — proof no entry exists even *after* a
  completed turn.
- **Contract conflict**: the `/session` command tells the user "it will bind
  to `premiere` on first message" (`gateway_commands.rs:830`), but no code
  path creates the entry on a first turn. The `entry_for_source` predicate
  (`gateway_session.rs:1505`) requires `origin.user_id`/`chat_id` — the
  turn-end `save_session` call (`gateway_runner.rs:935`) does not carry the
  platform origin, so DM sessions never land in `entries`.
- **Fix shape** (pick one, cheap): (a) persist the session entry with platform
  origin at gateway turn end, making the `/session` promise true and feeding
  the tap + metering; or (b) have the tap default-bind to `premiere` when no
  entry exists (matching the documented default) and let the first turn
  create the entry. (a) is the root-cause fix; (b) is the smaller diff.
- **Acceptance**: an E2E test — inbound DM on a fresh session → second DM →
  `context_items` gains a `Dm` row with `seat_hint=premiere` (or the bound
  seat); metering warning gone.
- **Effort**: S–M. Files: `gateway_runner.rs`, `gateway_session.rs`.

## 3. Feed class: channel/group posts → `context_items` (phase-2 remainder; M)

- The `context_items` schema already reserves non-DM classes
  (`context_injection.rs:60` — "class dm (future: feed sources)"); the
  watermark table is keyed `(seat_id, class)`.
- **Scope now**: Telegram only. The bot (`can_join_groups=true`,
  `has_topics_enabled=true`) can be added to a channel/group; the gateway's
  poll already receives `channel_post`/`message` updates from groups —
  classify them as `feed` rows through the same injector, per-seat by topic
  or chat→seat mapping. **No second poller** — the gateway owns `getUpdates`;
  a separate read adapter would 409-conflict with it (observed today: my own
  probe and the daemon's long-poll collided into a 409 restart loop until the
  probe stopped).
- **Acceptance**: a group message lands as a `feed` row for the mapped seat;
  DM quota/classes unaffected; `org synthesize`/turn prologue renders the
  feed class under its own quota.
- **Effort**: M. Files: `gateway_runner.rs` (classification),
  `context_injection.rs` (feed class constant + query), tests.

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

1 → 2 → 3 (1 and 2 are independent bug fixes; both small; do 1 first — it
blocks all inbound on any future token swap). 4 waits on the owner. Each item
is one iteration: fix + test + deploy + live-verify per AGENTS.md.

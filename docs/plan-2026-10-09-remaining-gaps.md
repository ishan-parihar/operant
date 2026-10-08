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

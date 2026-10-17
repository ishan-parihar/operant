# Remaining implementation gaps — 2026-10-09 (rev 2, post-arm)

> Supersedes both the execution queue in
> `plan-2026-10-08-remaining-implementation-gaps.md` (all items done/decided)
> and rev 1 of this file (items 1+2 fixed iter-704, item 3 slices A/B/D
> executed iter-711 — history preserved in git; this rev tracks only what
> still REMAINS). Every claim below was measured on the box, not inferred.
> Live bot: **8916661121 @ip_zeroclaw_bot** ("Zeroclaw"), token in
> `~/.operant/.env`; delivery hop closed and live-verified 2026-10-09.

## 0. State as of 05:20Z 2026-10-09 (verified)

- Inbound DM pipeline live end-to-end (iter-704: per-bot offset cursor +
  session-key namespace fix; `dm` rows land, metering warn gone).
- Feed class **deployed** (iter-711): `ContextClass::Feed`, capture at
  `route_message` top before the admin gate, channel posts feed-ONLY,
  group posts feed + turn. Unit-proven; live E2E gated (item 1).
- `context_items` retention **deployed** (iter-713): 7-day write-through
  prune, pinned by `capture_prunes_rows_past_the_retention_window`.
- Socialization **armed and fired** (config + iter-715): first run
  2026-10-09T02:01:34Z, "pairs: 7", stamp-first held; every turn failed
  on the environment's provider 503 class (same flake as cron_runs
  breaker data). Next full attempt: **09:30Z daily**, now on the 715
  binary with the empty-tick gate fixed.
- Daemon healthy: 57 MB RSS, unit peak 188 MB (the earlier 14 GB RSS
  concern does not reproduce on the gateway unit). Disk 43 G free.

## 1. Slice C (feed live E2E) — owner-gated, empirically blocked (S)

Two posts to the owner's channel `-1002220508783` (via `tg`) produced no
`Sent message to gateway handler` log line, no offset advance, no pending
updates: **the bot receives no `channel_post` updates at all**.
`getChat` ok:true is disputed evidence (public-channel lookup vs bot
membership) — do not resolve by argument; resolve by act:

**Owner action**: add @ip_zeroclaw_bot as ADMIN to the channel (or any
group). Then re-test: `tg send ishaan_parihar "<text>"` → expect a
`context_items` row `class='feed'`, default seat `premiere`
(`feed_seat_map` empty live), a `(premiere,'feed')` watermark row, and
the seat's next turn prologue rendering "Channel feed". Test posts sent
so far (msg_id 7, 8) are deleted; no residue.

## 2. Socialization outcome check — watch item, no code (S)

Check after 09:30Z: `grep socialization ~/.operant/logs/gateway.log`
(gate fix means the fire need not ride the hourly jobs), the
`socialization_state` stamp, seat MEMORY.md densification, and whether
provider 503s cleared. If turns still 503 → provider-side; automatic
retry next day. A crashed pair is a missed pair, never a doubled one
(stamp-first, at-most-once — pinned).

## 3. Three small code debts — FIXED iter-720 `3429363f` (deployed, md5-verified)

1. **`context_items(ts)` index** — `CONTEXT_ITEMS_SCHEMA` has none; the
   per-capture `DELETE WHERE ts < ?` is a full scan (quadratic on an
   active channel). One line in the existing schema batch:
   `CREATE INDEX IF NOT EXISTS idx_context_items_ts ON context_items(ts)`.
2. **`GatewayMessageHandler::record_feed` CLI-level test missing** — the
   seat-map → `record_feed_item` link is only stub-tested. Real injector
   + temp db: channel_post → `premiere` row; then with a
   `feed_seat_map` entry → mapped seat.
3. **Prune comment overclaims** — the watermark does NOT protect an
   offline seat's queued items (8-day-offline seat loses rows before its
   watermark advances). Either per-class retention (longer for `dm`)
   or state the loss in the comment instead of "only ever see items
   behind its watermark anyway".

## 4. Telegram 409 self-race — separate finding, candidate fix, time-boxed (M)

Pre-existing (observed before any of today's changes). With exactly one
gateway process on the box, bursts (~70s) correlate with agent-cycle
windows; no second process was ever caught. Candidate (NOT shipped —
needs poll-lifecycle testing): on a 409 the inner loop backs off 35s and
the `'restart:` epoch re-enters via a cold-start probe
(`getUpdates offset=0 timeout=0`) before re-polling; probe and first
long-poll can self-409. Fix shape: skip the restart-probe on 409 backoff
(probe is cold-start-only). Ship only with a unit test over the epoch
state machine. Do not block this plan on it.

## 5. Discord/Slack read adapters — blocked on owner credentials (unchanged)

`discord_enabled=false`, `slack_enabled=false`, tokens unset (re-verified).
No adapters against dead credentials. Blocked, not scoped.

## 6. Owner-side pending (no agent action)

- [ ] Add @ip_zeroclaw_bot as channel/group admin (unblocks item 1).
- [ ] Discord/Slack tokens or explicit deferral (item 5).
- [ ] `~/.operant/backups/packet-e-wt-wip-20261007.tar.gz` deletion sign-off.
- Peer's TUI gate debt (rule 7) stays theirs until their wave settles.

## 7. Dispatcher standing duty (rides existing cycle)

The audit artifact's `telegram_status` header records the Zeroclaw hop as
live (getMe/sendMessage 200); the old "stale channel ref" blocker line is
obsolete. Probes stay getMe/getChat only — the daemon's poll owns
`getUpdates`; any other process probing it triggers the 409 loops (item 4).

## Execution order

Item 3 landed (iter-720, deployed) → item 1 the moment the owner adds
the bot → item 2 check after 09:30Z → item 4 only if 409s recur and
block inbound. Items 5+6 wait on the owner.

## Owner-gated blockers (explicit)

1. **@ip_zeroclaw_bot must be added as channel/group ADMIN** — without
   it the bot receives no `channel_post` updates at all (two test posts:
   no handler line, no offset advance). Unblocks item 1.
2. **Discord/Slack bot tokens or an explicit deferral** — item 5.
3. **`packet-e-wt-wip-20261007.tar.gz` deletion sign-off**.
4. **Socialization spend authorization** — the daily sessions are
   armed and will consume up to 7 pairs × 3 turns of provider tokens
   per run once the 503s clear; pause with `enabled=false` if unwanted.
5. **Provider capacity** — every socialization turn on 2026-10-09
   failed on `resource_pressure` 503s from the configured endpoint;
   until capacity clears, sessions and crons will keep flaking.

# Gateway — the channel surface

Owns: the multi-platform messaging gateway — the 7-adapter registry, the
session store, the dispatch breaker, admin commands, and webhook
subscriptions.
Not for: the agent loop behind a gateway turn
([agent-loop.md](agent-loop.md)), platform API bases
([providers.md](providers.md) §platform-bases), the daemon lifecycle
([operability.md](operability.md)).

Source: `crates/operant-core/src/gateway/` (mod, telegram, discord,
slack, whatsapp, email, sms, webhook, webhook_subscriptions, admin,
lifecycle, dispatch_breaker, types),
`crates/operant-core/src/gateway_session.rs`,
`crates/operant-core/src/gateway_pipeline.rs` (hooks),
`crates/operant-core/src/gateway_markdown.rs`,
`crates/operant-cli/src/gateway_runner.rs` (+ `gateway_runner/` split:
turn_state, session handling) and `gateway_commands.rs`,
`gateway_platforms.rs`, `crates/operant-cli/src/cmd_gateway.rs`.

## The registry: 7 platforms, all real

`platform_registry()` (in `gateway_runner.rs`) is the factory that
replaced the if/elif chain (iter-66). Every entry has a real adapter in
`operant-core/src/gateway/`:

| Platform | Direction | Mechanism |
|---|---|---|
| telegram | in + out | Bot API long-poll |
| discord | in + out | Gateway WebSocket + REST |
| slack | **outbound only** | no Socket Mode WebSocket exists — `start()` logs and returns; `handle_update` is dead code (RT-86) |
| whatsapp | out + webhook in | Cloud API + webhook |
| email_smtp | out + webhook in | SMTP + inbound webhook |
| sms_twilio | out + webhook in | Twilio API + webhook |
| webhooks | **inbound only** | axum HTTP server, HMAC-SHA256 validation (incl. Slack `v0:` variant); `send_message` deliberately a no-op |

The registry IS the whole channel surface (the 2026-10-17 purge deleted
the never-called `operant-channels`/`operant-gateway` crates; 20 phantom
platforms were purged earlier, iter-50). Do not re-add platforms without
a real adapter.

## The daemon (`gateway_runner.rs`)

A singleton gateway instance persists across CLI commands in one
process; `operant gateway run` starts the daemon (systemd unit
`operant-gateway` on the operator's box). A second `gateway run` refuses
to start (flock, iter-1335). Status surfaces include the daemon's
RSS/HWM (`gateway status --memory`, iter-1337). Turn handling lives in
the `gateway_runner/` submodule split (turn_state etc.), which applies
per-channel policy, the dispatch breaker, and session scoping before
the agent runs.

## Session store (`gateway_session.rs`)

SQLite-backed, hermes `SessionStore` parity: deterministic session keys
(one owner: `crates/operant-api/src/session_keys.rs` — the same id is the
JSONL filename, the history-cache HashMap key, AND the memory
`session_id` filter), session TTL + idle timeout with daily reset,
transcript persistence with FTS5 search, session switching/resume/branch.
`clear_history` only on session change (iter-49) — the session-caching
fix.

## The dispatch breaker (`dispatch_breaker.rs`)

Per-channel circuit breaker (hermes `#526` parity): after
`OPEN_THRESHOLD` consecutive send failures the channel's dispatch opens
and cools down instead of hammering a platform that already told us to
stop. It exists because a dead Telegram token produced hours of
401→400→503→404 loops.

## Pipeline + hooks (`gateway_pipeline.rs`)

Message filtering + lifecycle event hooks (`gateway_pipeline.rs::
HookEvent`: `GatewayStartup`, `SessionStart`, … — hermes parity with
the Python reference's gateway hooks). The `HookRegistry` is created
unconditionally; the harness bridge registers on it when the harness
is on (iter-1125).

## Markdown dialects (`gateway_markdown.rs`)

One public function `markdown_to_telegram_html` converting model
markdown to Telegram-compatible HTML; the WhatsApp asterisk dialect is
shared (`asterisk_dialect`, iter-341) — one dialect owner per platform
family instead of per-adapter hand-rolls.

## Admin commands + subscriptions

- `gateway_commands.rs` — the canonical Telegram bot command registry +
  admin gating + `setMyCommands` payloads.
- `webhook_subscriptions.rs` + `cmd_webhook.rs` — a subscription records
  an INBOUND route: the path under `/webhook/`, the prompt/skills that
  shape handling, and the delivery target (`deliver` / `deliver_chat_id`
  — consumed at the gateway runner, RT-140 closed iter-1132). A
  subscription with its own secret is verified against that secret.
  `operant webhook test` POSTs a signed ping for real.

## Intention

The gateway is how an unattended agent meets humans: every platform
quirk (chunking limits, HTML dialects, token deaths) is absorbed here so
the agent loop stays platform-agnostic. The honest direction table
(inbound-only / outbound-only) exists because the docs once claimed
capabilities the adapters never had — the RT-45..50 claim audit. Slack
inbound is a documented gap (RT-86), not a defect to quietly "fix" by
adding a WebSocket client nobody asked for.

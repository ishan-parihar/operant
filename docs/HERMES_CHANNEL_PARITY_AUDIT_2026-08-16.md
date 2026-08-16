# Hermes-Agent Channel Parity Audit — Telegram & Discord

**Date:** August 16, 2026
**Scope:** Feature-by-feature parity of the **Telegram** and **Discord** platform implementations
between `hermes-agent` (reference, Python) and `operant` (Rust port).
**Method:** Method-list + symbol-level comparison of
`hermes-agent/plugins/platforms/{telegram,discord}/adapter.py` (10,241 / 10,150 lines)
against operant's two gateway stacks. Every gap below was verified by grep — 0 matches
in operant for the feature's core symbol.

---

## 0. Critical architectural finding — TWO parallel gateway stacks

Operant has **two** independent gateway implementations, and the production
`operant gateway run` path uses the **thin** one, not the rich one:

| Stack | Location | Used by | Telegram impl | Discord impl |
|-------|----------|---------|---------------|--------------|
| **Thin** `PlatformAdapter` stack | `operant-core/src/gateway/mod.rs` | `gateway_runner.rs` → **`gateway run`** (systemd unit, live bring-up) | `TelegramAdapter` (~700 lines, `send`/`edit`/`delete`/`parse_update`, HTML-only, no callbacks, no drafts) | `DiscordAdapter` (~250 lines, `send` (chunked) /`typing`/REST-only, websocket **receive-only**) |
| **Rich** `Channel` trait stack | `operant-channels/src/` | daemon's injected `start_channels` (via `channels_start`), `channel send` CLI | `TelegramChannel` (6,032 lines: drafts, approvals, ack reactions, pairing, TTS) | `DiscordChannel` (3,227 lines: markers, reactions, stall watchdog) + `DiscordHistoryChannel` |

**Consequence:** most hermes-parity features that *are* already implemented in
`operant-channels` (approvals, draft streaming, ack reactions, pairing, transcription)
are **NOT live in `gateway run`** — the production gateway only gets the thin adapters.
The two implementations drift independently; a feature can be "implemented" (channels
crate) yet absent from the shipped gateway path.

---

## 1. Telegram parity

Reference: `hermes-agent/plugins/platforms/telegram/adapter.py` (10,241 lines).
Operant: thin `TelegramAdapter` (`operant-core/src/gateway/mod.rs:1124-1885`) +
rich `TelegramChannel` (`operant-channels/src/telegram.rs`, 6,032 lines).

### 1.1 Already at parity ✓

| Feature | operant |
|---------|---------|
| Message chunking (4096 UTF-16 limit) | `split_message_for_telegram` + `send_text_chunks` (both stacks) |
| HTML markdown conversion + plain-text fallback | `markdown_to_telegram_html` + 400-retry-as-plain (thin), same in channels |
| Media send (document/photo/video/audio/voice by bytes **and** URL) | `send_document/photo/video/audio/voice[_bytes|_by_url]` |
| Incoming media download + transcription | `download_file`, `parse_attachment_metadata`, transcription feature |
| Draft streaming (send/update/finalize/cancel) | `send_draft`/`update_draft`/`finalize_draft`/`cancel_draft` with rate limiting |
| Approval via inline keyboard (approve/deny/always) + callback routing | `request_approval` + `listen()` `callback_query` handling |
| Ack reactions | `build_telegram_ack_reaction_request`, `random_telegram_ack_reaction` |
| Typing indicator | `start_typing`/`stop_typing` |
| Pairing (`/bind`) + allowed-users allowlist | `TELEGRAM_BIND_COMMAND`, `persist_allowed_identity`, `is_user_allowed` |
| Bot command registration (`setMyCommands`) | `register_bot_commands` (100/32/100 limits) |
| Voice replies with TTS | `try_queue_voice_reply`, `synthesize_and_send_voice` |
| Thread/topic replies (forum `message_thread_id`) | `extract_update_message_target`, `parse_reply_target`, thread kwargs |
| Startup polling probe + 409 conflict handling | probe loop + 35s conflict backoff |
| Mention gating in groups | `contains_bot_mention`, `normalize_incoming_content` |

### 1.2 Confirmed gaps (operant missing) — Telegram

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| **T1** | **DM topics (forum topics per chat)** — create/load/rename/persist `thread_id` back to config, handoff threads | `_setup_dm_topics`, `_create_dm_topic`, `ensure_dm_topic`, `rename_dm_topic`, `_persist_dm_topic_thread_id`, `create_handoff_thread` (3263-3566) | `telegram_dm_topics_enabled` **config flag exists but is dead**: `TelegramAdapter::with_config` accepts it as `_dm_topics_enabled` (underscore = ignored); `telegram.rs` has **0** `dm_topic` references; no `createForumTopic` call anywhere | 🟡 Medium — multi-topic DM organization, hermes flagship feature |
| **T2** | **Interactive keyboards beyond approval** — model picker (2-step provider→model drill-down, pagination), choice picker, clarify buttons, exec approval, slash confirm | `send_model_picker`, `send_choice_picker`, `send_clarify`, `send_exec_approval`, `send_slash_confirm`, `_handle_callback_query`, `_handle_model_picker_callback`, `_build_provider_keyboard`, `_build_model_keyboard` (5535-6403) | Only `request_approval` (approve/deny/always). No model/choice/clarify/exec keyboards. `request_choice` trait method exists (default `None`) but is never overridden by telegram | 🟡 Medium — interactive model switching, clarification UX |
| **T3** | **Rich rendering pipeline with capability detection** — rich HTML rendering, `_is_rich_capability_error`, rich draft, overflow split, Telegram-Desktop crash-shape guard (math/CJK) | `_try_send_rich`, `_try_edit_rich`, `_rich_message_payload`, `_is_rich_capability_error`, `_is_rich_fallback_error`, `_needs_rich_rendering`, `_edit_overflow_split`, `_has_telegram_desktop_details_math_crash_shape`, `_has_telegram_desktop_cjk_rich_garble_shape` (1758-2102, 5108) | Single-shot HTML send + plain fallback; no progressive rich upgrade, no capability probing, no crash-shape guards | 🟢 Low-Med — formatting fidelity on long/streamed messages |
| **T4** | **Polling resilience suite** — heartbeat, pending-update probing, reconnect verification, PTB retry-loop disarm, fallback IPs, general-request drain | `_polling_heartbeat_loop`, `_probe_pending_updates`, `_verify_polling_after_reconnect`, `_disarm_ptb_retry_loop`, `_fallback_ips`, `_drain_general_connections_after_pool_timeout`, `_instrument_polling_request` (2694-3262) | Only startup probe + 409 backoff (35s). No heartbeat, no pending-probe, no reconnect verification, no fallback IPs | 🟢 Low-Med — resilience on flaky networks/ISP |
| **T5** | **Typing cooldown/backoff** | `_record_typing_cooldown`, `_typing_in_cooldown`, `_is_transient_typing_error` (7575-7617) | Typing is unconditional; no cooldown after transient errors (rate-limited typing spam) | 🟢 Low — chat hygiene |
| **T6** | **Config depth: allowed chats / topics / ignored threads / free-response / guest mode / exclusive mentions / bot identity refresh** | `_telegram_allowed_chats`, `_telegram_allowed_topics`, `_telegram_ignored_threads`, `_telegram_free_response_chats/topics`, `_telegram_guest_mode`, `_telegram_exclusive_bot_mentions`, `_bot_identity_refresh_loop` (7867-8404) | Only `allowed_users` + `mention_only`. No chat/topic-level ACLs, no guest mode, no free-response rooms, no periodic username refresh (fetched once) | 🟡 Medium — multi-group deployments, guest access |
| **T7** | **Link preview control** | `_link_preview_kwargs` (1582) | Not implemented | 🟢 Low |
| **T8** | **Multiple images / animation send** | `send_multiple_images`, `send_animation` (7089, 7526) | Single photo; no album, no animation | 🟢 Low-Med — rich media UX |
| **T9** | **Adapter-level error redaction** | `_redact_telegram_error_text` (28) | Operant has core `redaction.rs` (added earlier) but the **gateway adapters do not route their errors through it** — token leakage risk in gateway logs | 🟠 **Medium-High — secret hygiene** |

---

## 2. Discord parity

Reference: `hermes-agent/plugins/platforms/discord/adapter.py` (10,150 lines).
Operant: thin `DiscordAdapter` (`operant-core/src/gateway/mod.rs:1886-2335`) +
rich `DiscordChannel` (`operant-channels/src/discord.rs`, 3,227 lines) +
`DiscordHistoryChannel` (554 lines).

### 2.1 Already at parity ✓

| Feature | operant |
|---------|---------|
| Message chunking (2000) | `chunk_text` (thin), `split_message_for_discord` (channels) |
| Attachment markers + inline URLs | `parse_attachment_markers`, `classify_outgoing_attachments` |
| Ack + failure reactions | `DISCORD_ACK_REACTIONS`, `apply_failure_reactions`, `add_reaction`/`remove_reaction` |
| Typing indicator | `start_typing`/`stop_typing` |
| Thread channel detection | `is_thread_channel`, thread lookup with timeout |
| WebSocket with heartbeat + stall watchdog + reconnect | heartbeat task, `stall watchdog fired — … triggering reconnect` (1359) |
| Allowed-users allowlist | `is_user_allowed` |
| Audio attachment transcription | `DISCORD_AUDIO_EXTENSIONS`, transcription manager |
| Attachment download → workspace | `process_attachments`, `download_attachment_bytes` |
| History persistence (all messages → discord.db) | `DiscordHistoryChannel` |
| Multi-message streaming + draft updates | `supports_draft_updates`, `supports_multi_message_streaming` |

### 2.2 Confirmed gaps (operant missing) — Discord

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| **D1** | **Voice — full stack** (join/leave VC, play audio, receive + SSRC mapping, silence detection, PCM→WAV, voice mixer, TTS playback, voice ack, ambient PCM, timeouts) | `VoiceReceiver`, `voice_mixer.py`, `join_voice_channel`, `play_in_voice_channel`, `get_user_voice_channel`, `play_tts`, `play_ack_in_voice`, `_voice_listen_loop`, `_process_voice_input`, `_install_voice_mixer` (525-908, 3760-4557) | **0 voice in `discord.rs`** — only REST/WS text. (`voice_call.rs` = PSTN calls, `voice_wake.rs` = wake-word; neither touches Discord) | 🟠 **High — hermes' largest Discord feature** |
| **D2** | **Slash-command registration + sync** — `/new`, `/reset`, `/model`, `/reasoning`, app-command fingerprint sync, rate-limit handling (`Retry-After`), unknown-interaction guard | `_register_slash_commands`, `_safe_sync_slash_commands`, `_desired_command_sync_fingerprint`, `_extract_discord_retry_after`, `_is_discord_rate_limit`, `_is_discord_unknown_interaction`, `_evaluate_slash_authorization`, `_reject_slash` (4755-5479) | **0 slash/interaction handling** in `discord.rs` or thin adapter (grep `INTERACTION\|application_command\|slash` → 0) | 🟡 Medium — Discord-native controls, permission gating |
| **D3** | **Missed-message backfill + SQLite recovery ledger** — per-channel cursors, message-seen ledger, processing claims, recovery scans, backfill window/limit/max-dispatches | `_run_missed_message_backfill`, `_discord_recovery_cursor`, `_record_discord_message_seen`, `_record_discord_processing_start/complete`, `_discord_message_has_active_claim`, `_record_recovery_scan_start/complete` (2107-2758) | `DiscordHistoryChannel` **stores** messages but there is **no cursor/ledger/backfill** — no at-least-once recovery of missed messages after gateway downtime | 🟡 Medium — message loss on restart |
| **D4** | **Forum-channel posting** | `_send_to_forum`, `_forum_post_file` (3193-3333) | Thread *detection* only; no `POST /channels/{id}/threads` forum creation | 🟢 Low-Med — community servers |
| **D5** | **Allowed-mentions safety** — deny `@everyone`/`@here`/role pings by default (config + env overridable) | `_build_allowed_mentions` (476-510) | Operant sends `{"content": …}` with **no `allowed_mentions`** → any `@everyone` in LLM output or echoed user content pings the whole server | 🟠 **High — mention-bomb/security** |
| **D6** | **Liveness probe / websocket health** (start → probe → fatal-notify) | `_start_liveness_probe`, `_read_websocket_health`, `_liveness_loop`, `_notify_liveness_fatal_error` (1532-1716) | Stall watchdog exists (partial); no proactive bot-presence/health probe | 🟢 Low-Med |
| **D7** | **Reaction lifecycle on processing** (emoji ack on start, completion/outcome reactions) | `on_processing_start`, `on_processing_complete` (2979-3009) | Static ack/failure reactions only; no event-driven lifecycle | 🟢 Low |
| **D8** | **SSRF-guarded remote image fetch** (redirect re-validation per hop, blocked private addresses) | `_read_url_image_with_redirect_guard`, `_ssrf_redirect_guard`, `tools/url_safety.py` (171-207, base.py:670) | Operant sends by URL without SSRF re-check on redirects; downloads attachments only from Discord CDN | 🟠 **Medium — SSRF on URL sends** |
| **D9** | **Non-conversational message tracking** (persisted tracker to skip bots/announcements) | `_DiscordNonConversationalMessageTracker` (289-343) | Not present | 🟢 Low |
| **D10** | **Text-batch flush on shutdown** (flush pending batches before disconnect) | `_text_batch_flush_deadline_seconds`, shutdown flush (1756-1782) | Multi-message mode exists; no explicit shutdown flush | 🟢 Low |

---

## 3. Recommended priority

### Tier 1 (do first — security + biggest parity wins)
1. **D5 — `allowed_mentions` on all Discord sends** (deny everyone/roles by default). Small, prevents real mention-bombing.
2. **T9 / D8 — route gateway adapter errors and URL fetches through the existing redaction + SSRF guards.** Secret hygiene + SSRF.
3. **T1 — wire `telegram_dm_topics_enabled`** (implement `createForumTopic`/persist in `telegram.rs`) or **remove the dead flag** — currently misleading.
4. **Close the two-stack gap** — either route `gateway run` through `start_channels` (channels crate) or port the channels-crate features (approvals, drafts, ack reactions, pairing) into the thin adapters. Without this, already-built parity features never ship.

### Tier 2 (feature parity)
5. **D1 — Discord voice** (largest effort; consider via a `voice_call`-style sub-module + lavalink/gateway voice WS).
6. **D2 — Discord slash-command registration + sync** (fingerprint + rate-limit state).
7. **T2 — Telegram interactive keyboards** (model picker, choice picker, exec approval) via the existing `request_choice` trait seam.
8. **D3 — Discord recovery ledger/backfill** (cursor + claim on top of `DiscordHistoryChannel`).

### Tier 3 (polish)
9. T3 rich rendering pipeline, T4 polling resilience, T5 typing cooldown, T6 config depth, T7 link previews, T8 media albums, D4 forums, D6 liveness probe, D7 reaction lifecycle, D9 non-conversational tracker, D10 shutdown flush.

---

## 4. Implementation status (updated Aug 16, 2026)

Tier-1 security + dead-code items, the Tier-2 feature-parity items, and the
Tier-3 polish items are all implemented, committed, and pushed to `main`:

| # | Item | Commit(s) | Notes |
|---|------|-----------|-------|
| D5 | `allowed_mentions` (deny everyone/roles by default) | `803402ff` | `DiscordAllowedMentions` policy on all send/edit helpers; patched in `discord.rs`, `discord_history.rs`, and thin `DiscordAdapter`; `with_allowed_mentions` builder |
| T9 | Adapter error redaction | `803402ff` | `Error::Network` variant now wraps redacting `RedactedReqwestError` (token-bearing URLs masked crate-wide); telegram regex fixed to match `/bot<TOKEN>` URLs; `gateway_runner` log lines routed through `redact_err` |
| D8 | SSRF guard on remote fetches | `212d8d1d` | `ssrf_guard` in channels `util.rs` (blocked loopback/private/link-local + scheme allowlist); applied to Discord attachment fetches and `link_enricher` redirect chain |
| T1 | Telegram DM topics | `79924226` | `createForumTopic` + persisted `chat_id → thread_id` state file, DM routing into topic; wired via `with_dm_topics` from `[channels.telegram]` config; 3 tests |
| Two-stack | Inline-button approvals on thin gateway path | `e149e597` | `send_approval_prompt` trait method + Telegram inline-keyboard override; `callback_query` taps synthesize `/approve` `/deny` through the shared resolver; `Gateway::adapter_for`; 4 tests |
| T2 | Telegram choice picker | `f0296880` | `request_choice` override with inline keyboard, index-based callback data, `pending_choices` map, last-chat addressing; 4 tests |
| D2 | Discord slash commands | `9fcadf51` | `register_slash_commands` (guild/global REST PUT) + `INTERACTION_CREATE` handling with auth gate + ephemeral ACK; pure `interaction_to_command` parser; 4 tests |
| D3 | Discord recovery ledger + backfill | `c00decc1` | SQLite `discord_recovery_cursors` + `backfill_missed_messages` (REST scan after cursor, mention re-dispatch, timestamp preservation); live-loop cursor advance; 3 tests |
| T5/T7/D6 | Typing cooldown, link previews, liveness probe | `866a5c94` | `typing_cooldown_seconds` per-chat backoff; `disable_link_previews` → `link_preview_options`; heartbeat-ACK liveness counter (3-miss → reconnect); config + example TOML + tests |

**Remaining (not yet implemented):** D1 Discord voice (largest effort), T3 rich
rendering pipeline, T4 polling resilience suite, T6 config depth (chat/topic
ACLs, guest mode, identity refresh), T8 media albums, D4 forum posting, D7
reaction lifecycle, D9 non-conversational tracker, D10 shutdown flush.

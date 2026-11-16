# The configuration reference

Owns: every config key this binary accepts — its section, its type, its
default, and one line on what it changes.
Not for: the config MECHANISM (precedence, the one-home resolver,
profiles) — that is [config.md](config.md); per-subsystem config stories
([tools.md](tools.md), [gateway.md](gateway.md),
[memory.md](memory.md), [security.md](security.md)); exact commands and
environment traps (`docs/RUNBOOK.md`).

Source: `crates/operant-core/src/config.rs` (`AppConfig` and every
settings struct), `crates/operant-core/src/config_registry.rs` (the
GENERATED key table), `crates/operant-config/src/` (the shared section
types `AppConfig` composes), `scripts/gen-config-registry.py` (the
generator), `operant.example.toml` (the template).

Counts on this page are as of commit `826a4568` and were derived from
`crates/operant-core/src/config_registry.rs`.

## Contents

- [How to read this page](#how-to-read-this-page)
- [The bare top-level keys](#the-bare-top-level-keys)
- [`[client]` — the model transport (13 accepted, 10 describable)](#client-the-model-transport-13-accepted-10-describable)
- [`[agent]` — the behaviour surface (66 accepted, 57 describable)](#agent-the-behaviour-surface-66-accepted-57-describable)
- [`[gateway]` — channels and the daemon (30 accepted, 11 describable)](#gateway-channels-and-the-daemon-30-accepted-11-describable)
- [`[tools]` — the tool surface (59 accepted, 32 describable)](#tools-the-tool-surface-59-accepted-32-describable)
- [`[mcp]` — external servers (4 accepted, 2 describable)](#mcp-external-servers-4-accepted-2-describable)
- [`[memory]`, `[browser]`, `[vision]` — providers](#memory-browser-vision-providers)
- [`[skills]` (10 keys)](#skills-10-keys)
- [`[plugins]` and `[harness]`](#plugins-and-harness)
- [The org sections: `[bylaws]`, `[holon]`, `[socialization]`, `[integrations]`](#the-org-sections-bylaws-holon-socialization-integrations)
- [`[context_injection]` (13 accepted, 11 describable)](#contextinjection-13-accepted-11-describable)
- [The background-model sections](#the-background-model-sections)
- [The remaining sections](#the-remaining-sections)
- [Failure modes — what the operator sees](#failure-modes-what-the-operator-sees)
- [Intention](#intention)

## How to read this page

- **Two key counts exist and they are different numbers on purpose.**
  `KNOWN_KEYS` is what the LOADER accepts, and it is the list that decides
  whether an unknown key gets parked or refused: **385 dotted keys across 33
  top-level sections** (a section is a leading path segment; the bare
  `terminal_backend` counts as its own, because it is a bare key).
  `CONFIG_KEYS` is what `operant config describe` and `config notes` can
  DESCRIBE (dotted key, default, note): **302 keys**. The 83-key difference
  is real and unevenly spread, not a footnote: `tools` is 59 accepted versus
  32 describable, `gateway` 30 versus 11, `client` 13 versus 10, `mcp` 4
  versus 2, and `[logging]` is 7 accepted and **0** describable. A key in the
  first list and not the second is LOADED and its default applies, but
  `operant config describe` cannot show it.
- **The 33 sections**, with what the loader accepts per section: `advisor` 11,
  `agent` 66, `autonomous` 9, `auxiliary_models` 45, `browser` 1, `bylaws` 9,
  `checkpoints` 5, `cli` 5, `client` 13, `compression` 4,
  `context_injection` 13, `credential_pool` 3, `gateway` 30, `harness` 4,
  `holon` 8, `integrations` 2, `logging` 7, `mcp` 4, `memory` 8, `moa` 6,
  `observability` 7, `pk` 4, `plugins` 4, `providers` 5, `proxy` 7,
  `recap` 7, `skills` 10, `socialization` 4, `terminal_backend` 1 (bare),
  `tools` 59, `tts` 3, `tui` 17, `vision` 4.
- **`deny_unknown_fields` is on every settings struct.** A key that is not
  in `KNOWN_KEYS` cannot be loaded at all, so this page's key list is not
  advisory — it is the set of names the binary will accept.
- Every `crates/…` path cited here exists in the tree at the commit above.

The two names that look wrong and are not:

- `crates/operant-core/src/config.rs` carries a `[tts] provider` default of
  `"kokoro"` in the WORKING TREE and `"phonon"` in `HEAD` (commit
  `ec443d0c`, iter-2513, replaced Kokoro with Phonon as the local TTS
  default). The committed default is **`"phonon"`**, and the generated
  registry agrees with the commit, not with the working tree. Read the
  committed source before believing a local checkout.
- `agent.context_semantic_max_groups` and `agent.context_historian*` exist
  in `HEAD`'s `BehaviorSettings` and in `KNOWN_KEYS`; they are absent from
  an older working tree. Same rule.

## The bare top-level keys

These sit directly on `AppConfig`, not in a section.

| Key | Type | Default | What it changes |
|---|---|---|---|
| `terminal_backend` | string | `"local"` | Which terminal backend `terminal` uses: `local`, `docker`, `ssh`. |
| `database_path` | path | `<operant home>/database.db` | The SQLite file. It lives under the operant home so a PROFILE gets its own sessions, cron jobs, budgets and tool state. Not a config key an operator normally sets. |
| `command_allowlist` | array of globs | `[]` | Tool names (exact or `*`/`?` globs) that skip the permission prompt in every session. Interactive "always allow" choices are appended to `~/.operant/approval_allowlist.json`, not here. |
| `version` | int | `2` | Config format version. `None` means legacy v0. Rarely hand-set. |
| `keybindings` | table | unset | Parsed so the file still loads, then IGNORED: the TUI dispatch does not read overrides yet (RT-3). `load` warns once when set. |
| `config_path` | path | unset | PROVENANCE, not configuration. `#[serde(skip)]`, never serialized. Records which file this config was loaded from so a handler that edits config writes back to the file the operator invoked. |

## `[client]` — the model transport (13 accepted, 10 describable)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `client.base_url` | string | `"https://api.openai.com/v1"` | The OpenAI-compatible endpoint. This is the key a self-hosted gateway (LiteLLM, vLLM, Ollama) or a regional endpoint changes. |
| `client.api_key` | string? | `None` | The provider key. PLAINTEXT in `operant.toml` (the file is 0600); prefer `OPENAI_API_KEY` in the environment or `.env`. |
| `client.additional_api_keys` | array | `[]` | Extra keys for the credential pool / multi-key rotation. |
| `client.timeout_secs` | int | `60` | Transport-level header wait. Superseded per-model by `model_request_timeouts`. |
| `client.model_request_timeouts` | map | `{}` | Explicit per-model request budgets in seconds. An entry here WINS over `timeout_secs` and over the built-in reasoning floor, and bounds BOTH the transport wait and the loop's ceiling. The only lever that covers a local alias no floor table knows. |
| `client.rate_limit.max_retries` | int | `3` | Retry attempts after a 429 or transient error. |
| `client.rate_limit.base_delay_secs` | int | `5` | Base delay for exponential backoff. |
| `client.rate_limit.max_delay_secs` | int | `60` | Backoff ceiling. |
| `client.rate_limit.bucket_capacity` | int | `60` | Token-bucket burst capacity (max requests in a burst). |
| `client.rate_limit.bucket_refill_rate` | float | `1.0` | Tokens refilled per second. |
| `client.prompt_cache_ttl` | `"5m"` \| `"1h"` | `"5m"` | TTL for provider prompt-cache `cache_control` breakpoints (OpenAI/OpenRouter and the Anthropic adapter). |
| `client.max_context_length` | int | `128_000` | **DEPRECATED and IGNORED.** The context window is `agent.context_window`. Kept only so old files parse; it had two writers and no reader. |

## `[agent]` — the behaviour surface (66 accepted, 57 describable)

The model, the budget, the retry policy, the compaction thresholds, and
the ponytail posture.

### The turn

| Key | Type | Default | What it changes |
|---|---|---|---|
| `agent.model` | string | `"gpt-4"` | The model this session sends. Also settable per-session with `operant model set`. |
| `agent.max_iterations` | int | `90` | Hard cap on tool-loop iterations per turn. The budget grace call turns a hit into a summarize request rather than a dead stop. |
| `agent.tool_timeout_secs` | int | `30` | Per-tool execution cap. Interactive tools (`clarify`, `approval_request`) and `lcm_assert` are granted longer windows in code. |
| `agent.request_timeout_secs` | int | `120` | The agent's per-request ceiling. |
| `agent.max_healing_attempts` | int | `3` | How many times a malformed provider response is repaired before the turn fails. |
| `agent.stream` | bool | `true` | Stream assistant output. |
| `agent.show_reasoning` | bool | `true` | Maps to the TUI reasoning display mode: `true` = Current (traces commit in place), `false` = Off. `/thinking-display full` keeps every trace. |
| `agent.tool_progress` | `per_step`\|`final_only`\|`streaming`\|`auto` | `auto` | How tool progress is reported to the user. |
| `agent.system_prompt` | string? | `None` | A full replacement system prompt. Rarely set; the frozen prefix is assembled from base + skills. |
| `agent.reasoning_effort` | `""`\|`minimal`\|`low`\|`medium`\|`high`\|`xhigh` | `""` (don't send) | The effort field sent on every main-turn request. An invalid value FAILS config load with the allowed set named. |
| `agent.reasoning_effort_param` | `auto`\|`reasoning_effort`\|`reasoning`\|`anthropic_thinking`\|`none` | `auto` | The WIRE SHAPE of the effort field, chosen per endpoint family by default. |
| `agent.fallback_models` | array | `[]` | Ordered same-provider fallback models tried on a retryable provider error (5xx, 429, network). |
| `agent.fallback_on_errors` | bool | `true` | Master switch for the fallback list. |
| `agent.max_consecutive_tool_only` | int | `90` | Consecutive tool-only iterations before tools are withheld to force a textual answer. 0 disables. |
| `agent.session_reset` | `never`\|`on_system_prompt_change`\|`on_tool_change`\|`always` | `never` | Accepted but NOT ENFORCED (RT-263): no code path reads it. |
| `agent.guardrail_exempt_tools` | array of globs | `[]` | Tool names removed from every guardrail rung. |
| `agent.guardian_llm` | bool | `false` | Config-file default for the Guardian-LLM approval arm; the `OPERANT_GUARDIAN_LLM` env var still wins when set. |
| `agent.micro_compaction` | bool | `false` | Config-file default for micro-compaction; the env var still wins when set. |
| `agent.loop_detection_enabled` | bool | `true` | The turn's loop detector. |
| `agent.show_turn_attribution` | bool | `true` | Whether the turn's four exclusive spans are aggregated and emitted on the post-turn `TurnEnd` seam. |
| `agent.recap_idle_seconds` | int | `240` | Idle seconds after a completed turn before the ephemeral recap call fires. 0 disables. |
| `agent.max_trajectories` | int | `0` (keep all) | Cap on `~/.operant/trajectories`. 0 keeps every file, because `trajectory export` exists to collect them as a fine-tuning corpus. |
| `agent.memory_nudge_interval` | int | `10` | Turns between memory reviews. 0 disables. |
| `agent.creation_nudge_interval` | int | `10` | Iterations between skill nudges. 0 disables. |

### The window and the compaction ladder

| Key | Type | Default | What it changes |
|---|---|---|---|
| `agent.context_window` | int | `0` (AUTOMATIC) | Tokens budgeted per request. 0 resolves from the provider's `/models`, then models.dev, then a 128k fallback. A value > 0 is an explicit operator decision that wins over every discovered source. |
| `agent.context_budget_tokens` | int | `0` (unset) | Target total tokens the tool SURFACES (builtin schemas + each MCP server + each granted CLI) may add per request. 0 means `context budget` reports but `context optimize` proposes nothing. |
| `agent.context_engine` | `"compact"` \| `"lcm"` | `"compact"` | Which engine assembles the per-call message list. `lcm` is the lossless DAG (opt-in). |
| `agent.context_compression` | bool | `false` | The LLM summarize rung of the preflight ladder. |
| `agent.context_compression_threshold` | float | `0.5` | Fraction of the window at which the ladder starts. |
| `agent.context_soft_cap_tokens` | int | `0` (disabled) | Absolute soft-compaction limit. Whichever of this and `context_soft_cap_percent` is SMALLER binds. 0 disables the absolute axis. |
| `agent.context_soft_cap_percent` | float | `0.80` | Soft-compaction limit as a fraction of the window. |
| `agent.context_hard_cap_tokens` | int | `0` (disabled) | Absolute hard-compaction limit. |
| `agent.context_hard_cap_percent` | float | `0.95` | Hard-compaction limit as a fraction of the window. |
| `agent.context_compression_summary_max_chars` | int | `u32::MAX` | Char ceiling for one LLM compaction summary. `usize::MAX` here would overflow TOML's i64 and make the config unwritable. |
| `agent.context_semantic_compaction` | bool | `false` | Goal-similarity GROUP SELECTION as a preflight rung between decay and summarize. Needs an embedder to do anything. |
| `agent.context_semantic_max_groups` | int | `64` | Cap on the groups the semantic rung may cut. |
| `agent.context_historian` | bool | `false` | The context historian pass. |
| `agent.context_historian_max_span` | int | `0` | Its span cap. 0 disables. |
| `agent.context_reference_soft_share` | float | `0.0` | Share of the window @-references may take softly. |
| `agent.context_reference_hard_share` | float | `0.0` | Hard ceiling for the same. |
| `agent.max_tool_result_share` | float | `0.05` | Share of the window ONE tool result may take before its bulk is withheld. |
| `agent.max_tool_result_total_share` | float | `0.40` | The same bound in AGGREGATE across all tool results. |
| `agent.advertised_skills_token_budget` | int | `4_000` | Ceiling on the advertised-skills block. |
| `agent.memory_injection_budget_tokens` | int | `4_000` | Ceiling on the recalled-memory block. 0 = the built-in default. |

### The LCM (lossless context) keys

Only read when `context_engine = "lcm"`.

| Key | Type | Default | What it changes |
|---|---|---|---|
| `agent.context_lcm_db` | path? | `None` (`~/.operant/lcm.db`) | SQLite path for the lossless DAG. |
| `agent.context_lcm_tail_tokens` | int | `12_000` | Fresh-tail budget kept verbatim; older messages compact into the DAG and stay recallable. |
| `agent.context_lcm_auto_recall` | bool | `true` | Run one bounded retrieval round per assemble and inject the hits as pre-answer evidence. |
| `agent.context_lcm_auto_recall_limit` | int | `3` | Max nodes auto-recalled per assemble. |
| `agent.context_lcm_auto_recall_max_chars` | int | `4_000` | Hard cap on the injected evidence block. |
| `agent.context_lcm_rollups_inject` | bool | `true` | Inject stored day/week/month rollups into compaction instead of a bare placeholder. |
| `agent.context_lcm_rollup_interval_minutes` | int | `0` (disabled) | Background rollup-maintenance interval. |
| `agent.context_lcm_embedding_model` | string? | `None` | Embedding model for `lcm_vector_recall`. `"local:hash"` uses the zero-dependency built-in; anything else needs an OpenAI-compatible `/embeddings` endpoint. Empty disables the vector tool. |
| `agent.context_lcm_embedding_base_url` | string? | `None` | Embeddings endpoint when it differs from the chat provider. |
| `agent.context_lcm_assertion_extraction` | bool | `false` | LLM-driven (subject, predicate, object) extraction into the assertion store. Costs a reasoning-model call. |
| `agent.context_lcm_ignore_session_patterns` | array of globs | `[]` | Sessions skipped in GLOBAL DAG recall. |
| `agent.context_lcm_readonly_sessions` | array | `[]` | Sessions that must never be mutated; ingest is a no-op for them. |

### The retry policy (`[agent.retry]`)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `agent.retry.small_span_retries` | int | `3` | Immediate re-issues with no sleep before the large span starts. 0 disables the small span. |
| `agent.retry.large_span_secs` | int | `300` | The large span: once the small span is spent, retry this often. Must be > 0. |
| `agent.retry.max_span_secs` | int | `0` (INFINITE) | Opt-in ceiling on the TOTAL elapsed retry span. 0 retries the large span forever. |
| `agent.retry.announce_large_span` | bool | `true` | Announce every large-span retry (heartbeat + log line). An unannounced infinite retry looks exactly like a hang. |

### Ponytail (`[agent.ponytail]`)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `agent.ponytail.enabled` | bool | `false` | The POSTURE switch. OFF means the ruleset is not injected. The installed skills stay invocable either way. Never silently enable. |
| `agent.ponytail.default_level` | `lite`\|`full`\|`ultra` | `full` | Which ruleset level the posture uses. |
| `agent.ponytail.inject` | `on_invoke`\|`always_on` | `on_invoke` | Whether the rules ride only on invocation or the frozen prefix as a `<ponytail>` block. `always_on` changes what the model is told every turn and invalidates the prompt cache. |
| `agent.ponytail.update` | bool | `true` | The CONTENT refresh (check upstream tags at startup). Independent of `enabled`: a content integration that stops updating when its posture is off is a broken integration. |

## `[gateway]` — channels and the daemon (30 accepted, 11 describable)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `gateway.telegram_enabled` | bool | `false` | Enable the Telegram long-poll adapter. |
| `gateway.telegram_token` | string? | `None` | The bot token. The gateway RECEIVES on Telegram. |
| `gateway.telegram_api_base` | string | `"https://api.telegram.org"` | Override for a proxy or a regional endpoint. |
| `gateway.telegram_proxy` | string? | `None` | HTTP/SOCKS5 proxy URL for Telegram API requests. |
| `gateway.telegram_bot_username` | string? | `None` | Bot username for @mention detection in groups. |
| `gateway.telegram_dm_topics_enabled` | bool | `false` | DM topic creation for private chats (Bot API 9.4+). |
| `gateway.discord_enabled` | bool | `false` | Enable the Discord Gateway adapter. |
| `gateway.discord_token` | string? | `None` | The bot token. |
| `gateway.discord_api_base` | string | `"https://discord.com/api/v10"` | API base override. |
| `gateway.slack_enabled` | bool | `false` | **OUTBOUND-ONLY**: operant can post to Slack, but there is no Socket Mode WebSocket in this tree, so Slack messages never reach the agent. |
| `gateway.slack_token` | string? | `None` | The bot token. |
| `gateway.slack_api_base` | string | `"https://slack.com/api"` | API base override. |
| `gateway.whatsapp_enabled` | bool | `false` | Cloud API adapter (outbound; inbound via webhook). |
| `gateway.whatsapp_token` | string? | `None` | The Graph API token. |
| `gateway.whatsapp_api_base` | string | `"https://graph.facebook.com"` | Graph API base. |
| `gateway.whatsapp_phone_number_id` | string? | `None` | REQUIRED for outbound sends: without it the Graph API URL has no `phone_number_id` segment and returns 404. |
| `gateway.email_enabled` | bool | `false` | SMTP outbound adapter. |
| `gateway.email_smtp_host` / `_user` / `_pass` | string? | `None` | SMTP connection details. |
| `gateway.webhooks_enabled` | bool | `false` | The axum inbound webhook server (INBOUND-ONLY; its `send_message` is a deliberate no-op). |
| `gateway.webhooks_addr` | string? | `None` | Bind address for that server. |
| `gateway.webhooks_secret` | string? | `None` | Shared secret for HMAC-SHA256 signature verification (GitHub `x-hub-signature-256`, Stripe, Slack, or the custom `x-webhook-signature` header). When set, unsigned or mismatched requests are rejected. |
| `gateway.sms_twilio_enabled` | bool | `false` | Twilio SMS (outbound). |
| `gateway.admins` | array | `[]` | Platform user ids allowed to run admin commands. |
| `gateway.streaming_transport` | string | `"auto"` | How partial output reaches the channel. |
| `gateway.max_concurrent_sessions` | int? | `Some(32)` | Cap on concurrent gateway sessions. NOT reachable as `None` from TOML (TOML has no null, and the serde default materialises `Some(32)`), so an absent key is 32, not unlimited. `None` means no cap only from programmatic config. |
| `gateway.session_retention_days` | int | `90` | How long a gateway session is kept after its last activity. 0 = keep forever. |
| `gateway.api_enabled` | bool | `true` | Serve the read-only session API (`GET /api/sessions`, `/api/sessions/:id/messages`, `/api/subagents`) on the daemon. |
| `gateway.api_bind` | string | `"127.0.0.1:8731"` | Bind address for that listener. Loopback is defence in depth; the token is the gate. |

## `[tools]` — the tool surface (59 accepted, 32 describable)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `tools.aft_enabled` | bool | `true` | Register the 20 `aft_*` IDE tools and, once the bridge is verified live, auto-disable the basic file/terminal tools. A missing binary degrades to the builtins. |
| `tools.registry_timeout_secs` | int | `30` | Registry-level tool timeout. |
| `tools.event_channel_size` | int | `100` | Depth of the tool-event channel. **ACCEPTED BUT UNREAD**: a set value is reported at load. |
| `tools.disabled_tools` | array | `[]` | Tool names removed from the agent's toolset (the TOML half of the disable state). |
| `tools.disabled_toolsets` | array | `[]` | Toolset-level bans inherited by delegated children. |
| `tools.deferred_tools` | array | `[]` | Tool names deferred behind the `tool_search` bridge: registered and callable, schemas not on every request. |
| `tools.browser_binary_path` | path? | `None` | **ACCEPTED BUT UNREAD**: the only browser backend resolves its binary from `SOURCEHOUND_BINARY` / `PATH`. A set value is reported at load. |
| `tools.web.search_timeout_secs` | int | `15` | `web_search` timeout. |
| `tools.web.fetch_timeout_secs` | int | `30` | `web_fetch` timeout. |
| `tools.web.default_results` | int | `10` | Default result count. |
| `tools.web.max_results` | int | `20` | Result ceiling. |
| `tools.web.user_agent` | string | `"Mozilla/5.0 (compatible; OperantAgent/0.1)"` | Outbound UA for the web tools. |
| `tools.web.preferred_provider` / `search_url` / `tavily_api_key` / `exa_api_key` / `searxng_base_url` | various | `None` | **RETIRED, ignored, warned about at load.** Web search is sourcehound-only (plan 11C1). |
| `tools.http.timeout_secs` | int | `30` | `http_request` timeout. |
| `tools.terminal.max_timeout_secs` | int | `300` | Hard ceiling on a `terminal` call. |
| `tools.terminal.max_output_bytes` | int | `1_000_000` | Output cap; the cut is reported by the backend's own marker. |
| `tools.terminal.docker.image` | string? | `"nikolaik/python-nodejs:python3.11-nodejs20"` | Docker backend image. |
| `tools.terminal.docker.volumes` | array | `[]` | Bind mounts for the Docker backend. |
| `tools.terminal.docker.cpu` / `memory_mb` | float / int | `1.0` / `5120` | Docker resource limits. |
| `tools.terminal.ssh.host` / `user` / `port` / `key_path` | string / string / int / string | `None` / `None` / `22` / `None` | SSH backend connection. |
| `tools.code_execution.default_timeout_secs` | int | `60` | Default wall-clock cap for `code_execution`. |
| `tools.code_execution.max_timeout_secs` | int | `300` | Hard ceiling for it. |
| `tools.stt.openai_model` | string | `"whisper-1"` | Transcription model for the OpenAI STT provider. |
| `tools.stt.groq_model` | string | `"whisper-large-v3-turbo"` | Transcription model for the Groq STT provider. |
| `tools.stt.openai_api_key` / `groq_api_key` | string? | `None` | STT credentials. |
| `tools.tool_search.enabled` | `auto`\|`on`\|`off` | `auto` | Whether the deferred-dispatch bridge is active. `auto` activates only when deferrable (MCP) tools exist. |
| `tools.tool_search.threshold_pct` | float | `5.0` | Listing budget as a percentage of the context window before disclosure degrades. |
| `tools.tool_search.listing` | `auto`\|`on`\|`off` | `auto` | Whether the bridge description embeds a catalog listing of every deferred tool. |
| `tools.tool_search.listing_max_tokens` | int | `4000` | Absolute cap on that listing. |
| `tools.tool_search.search_default_limit` | int | `10` | Default `limit` for `tool_search`. **ACCEPTED BUT UNREAD**: a set value is reported at load. |
| `tools.tool_search.max_search_limit` | int | `25` | Hard cap on a single `tool_search` `limit`. **ACCEPTED BUT UNREAD**: a set value is reported at load. |

### The kernel (`[tools.kernel]`, opt-in)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `tools.kernel.enabled` | bool | `false` | The single gate for the persistent Python kernel path. Dark-merged. |
| `tools.kernel.python` | path? | `None` (find then PATH) | Python >= 3.11 for the sidecar. |
| `tools.kernel.vendor_dir` | path? | `None` (vendored submodule) | Root of the prime-agent runtime. |
| `tools.kernel.state_dir` | path? | `None` (`~/.local/share/operant/kernel`) | Harness store root. |
| `tools.kernel.sidecar_idle_secs` | int | `1800` | Sidecar idle timeout. |
| `tools.kernel.request_timeout_secs` | int | `120` | Per-request timeout to the sidecar. |
| `tools.kernel.max_output_bytes` | int | `200_000` | Output cap per kernel call. |
| `tools.kernel.session_gc_ttl_hours` | int | `168` | Prune per-session harness dirs older than this. 0 disables. |
| `tools.kernel.route_python_to_kernel` | bool | `false` | **Deprecated**: no longer gates anything; kept so old configs parse. |
| `tools.kernel.harness_auto_learn` | bool | `false` | Let the background review emit prompt/subagent harness edits. |
| `tools.kernel.tool_bridge.enabled` | bool | `false` | Master switch for `operant_tool()` inside kernel cells. |
| `tools.kernel.tool_bridge.allowlist` | array | 8 read-only names (`file_list`, `file_read`, `file_search`, `web_search`, `web_scrape`, `http_request`, `datetime`, `session_insights`) | Deny-by-default allowlist of callable tools. |
| `tools.kernel.tool_bridge.max_calls_per_exec` | int | `64` | Cap on bridged calls per executed cell. |
| `tools.kernel.tool_bridge.per_call_timeout_secs` | int | `60` | Per-bridged-call timeout. |
| `tools.kernel.pyskill.enabled` | bool | `false` | Executable-skill binding from SKILL.toml `[reference]`. |
| `tools.kernel.pyskill.allowed_imports` | array of globs | `[]` (deny all) | Import prefixes callable from a skill. |

## `[mcp]` — external servers (4 accepted, 2 describable)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `mcp.deferred_loading` | bool | `true` | Progressive tool disclosure for MCP tools. `false` loads every MCP schema eagerly and forces `tools.tool_search.enabled = "off"`. |
| `mcp.watchdog_interval_secs` | int | `30` | Stdio MCP watchdog sweep interval. 0 disables. It auto-reconnects a child that has exited. |
| `mcp.autoload` | bool | `true` | Autoload configured servers at startup. |
| `mcp.servers` | array of tables | `[]` | The server list. Each entry has `name`, `transport` (`http` / `stdio` / `streamable-http`), `url`, `command`, `args`, `env`, `auth_token`. |

## `[memory]`, `[browser]`, `[vision]` — providers

| Key | Type | Default | What it changes |
|---|---|---|---|
| `memory.provider` | string | `"memory-wire"` | The one memory engine. Any other name warns at load and is ignored. |
| `memory.enabled` | bool | `true` | Master switch for long-term memory. |
| `memory.injection_budget_tokens` | int | `4_000` | Ceiling on the recalled-memory block. |
| `memory.agentmemory_url` / `_secret` / `_auto_spawn` / `_version` | various | `None` | **RETIRED** agentmemory keys: accepted, ignored, and warned about when set. |
| `memory.audit_enabled` / `response_cache_enabled` | bool? | `None` | **RETIRED**, accepted and ignored. |
| `browser.provider` | string | `"sourcehound"` | Accepted for compatibility and IGNORED: sourcehound is the only engine. A non-default value warns once at load. |
| `vision.provider` / `model` / `base_url` / `api_key` | string? | `None` | The vision auxiliary-model slot. |

## `[skills]` (10 keys)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `skills.root_dir` | path | `<operant home>/skills` | The primary skill pool. |
| `skills.memory_dir` | path | sibling `memory/` | Where memory files live. |
| `skills.autoload` | bool | `true` | Load and advertise skills at agent construction. `false` builds an EMPTY skill manager AND leaves `skills_list` / `skill_view` / `skill_manage` unregistered: the operator's off switch for the whole skill surface. |
| `skills.template_name` | string | `"new-skill"` | Scaffolded skill name. |
| `skills.template_description` | string | `"Describe what this skill does."` | Scaffolded description. |
| `skills.template_vars` | bool | `true` | Substitute `${OPERANT_SKILL_DIR}` / `${OPERANT_SESSION_ID}` in SKILL.md before injection. |
| `skills.inline_shell` | bool | `false` | Expand `` !`cmd` `` snippets in SKILL.md at load time. Off by default. |
| `skills.inline_shell_timeout` | int | `10` | Timeout per inline-shell snippet. |
| `skills.extra_roots` | array of paths | `[]` | Additional operator-named roots, in precedence order after `root_dir` and before the project root. Missing/unreadable/credential-store entries are SKIPPED and reported by `operant doctor`. |
| `skills.project_root` | `"auto"`\|`"off"`\|path | `"auto"` | Project-level discovery: `auto` resolves `<workspace>/skills`. `"off"` is the escape hatch. |

## `[plugins]` and `[harness]`

| Key | Type | Default | What it changes |
|---|---|---|---|
| `plugins.plugin_dirs` | array of paths | `[<operant home>/plugins]` | Directories scanned for plugin manifests. |
| `plugins.disabled` | array | `[]` | Plugin names skipped at discovery. |
| `plugins.signature_mode` | `disabled`\|`permissive`\|`strict` | `disabled` | Signature enforcement for the tool bridge AND the kernel swap watcher, so the two cannot disagree. |
| `plugins.trusted_publisher_keys` | array | `[]` | Keys trusted under `strict`. |
| `harness.enabled` | bool | `false` | Construct the harness; when false no provider is mounted and the live loop is unaffected. |
| `harness.architecture_toml` | path? | `None` | The `architecture.toml` to load; relative paths resolve under `~/.operant/`. |
| `harness.max_active_providers` | int | `64` | Cap on simultaneously active providers. |
| `harness.watch_wasm` | bool | `false` | Watch the plugin dir and hot-swap verified WASM providers. Requires `strict` + trusted keys. |

## The org sections: `[bylaws]`, `[holon]`, `[socialization]`, `[integrations]`

| Key | Type | Default | What it changes |
|---|---|---|---|
| `bylaws.unrestricted_default` | `yolo`\|`standard`\|`scoped`\|`lockdown` | `"yolo"` | The mode a newly-seated policy row carries when the operator does not name one. The runtime default for a seat WITHOUT a policy row stays ungoverned. |
| `bylaws.unattended_posture` | seat-mode spelling | `"yolo"` | The posture an UNGOVERNED unattended run takes on a dangerous tool when nobody can answer. `"yolo"` = legacy auto-approve. A typo'd spelling fails CLOSED. |
| `bylaws.grant_ttl_days` | int | `7` | **RETIRED** with the grant ledger (RT-136): there is no mint any more. Accepted so old configs parse; `unused_setting_warnings` says so. |
| `bylaws.session_summary_ttl_minutes` | int | `30` | Minutes a session's rolling summary stays a valid continuation entry point. |
| `bylaws.budget.cap` | float | `0.0` | The org-wide default budget cap. 0 = ungoverned (no budget consulted at all). |
| `bylaws.budget.basis` | `tokens`\|`usd` | `"tokens"` | What the cap counts. |
| `bylaws.budget.window` | `daily`\|`weekly`\|`monthly` | `"daily"` | Rollup window. |
| `bylaws.budget.mode` | `soft`\|`hard` | `"soft"` | Warn-and-continue vs refuse at the cap. |
| `holon.enabled` | bool | `false` | Master switch for every Holon-backed surface. Off = personal mode, byte-identical (invariant 5). |
| `holon.home` | path | `""` | `$HOLON_HOME`, the instance operant reads. |
| `holon.bin` | string? | `None` (PATH `holon`) | Pin the kernel launcher. |
| `holon.notion_script` | path? | `None` | The kernel's notion adapter script. |
| `holon.supervise` | bool | `false` | Let the resident supervisor claim, spawn, heartbeat and hand in for `supervise_employees`. |
| `holon.supervise_employees` | array | `[]` | Required when `supervise` is on. |
| `holon.max_concurrent_runs` | int | `2` | Bodies the supervisor runs at once. |
| `holon.clock` | bool | `false` | Fire `holon tick` on the no-agent cron instead of a systemd timer. |
| `socialization.enabled` | bool | `false` | Master switch for the daily seat-pairing sessions. Phase 1 has NOT landed (RT-136): `enabled = true` changes nothing and says so at load. |
| `socialization.schedule` | cron | `"0 30 9 * * *"` | Six-field UTC cron. |
| `socialization.turn_budget` | int | `3` | Shared turn budget per pair per day. |
| `socialization.pairs` | array of `[senior, junior]` | `[]` | Ordered pairs; the senior initiates. |
| `integrations.herdr.enabled` | bool | `true` | Report this operant's lifecycle state to the Herdr pane that owns it. On by default, because without it a Herdr pane running operant looks like a plain terminal. |

## `[context_injection]` (13 accepted, 11 describable)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `context_injection.enabled` | bool | `true` | Master switch. `false` makes every prompt byte-identical. |
| `context_injection.total_char_cap` | int | `2000` | Hard ceiling for the whole rendered section. |
| `context_injection.dm_quota` | int | `800` | Per-aspect char quota for DMs. |
| `context_injection.feed_quota` | int | `500` | Per-seat char quota for feeds. |
| `context_injection.self_quota` | int | `300` | **NOT APPLIED** (RT-289): the Self class was retired to Holon. The loader reports it when set. |
| `context_injection.global_quota` | int | `500` | **NOT APPLIED** (RT-289): same. |
| `context_injection.dept_quota` | int | `400` | **NOT APPLIED** (RT-289): same. |
| `context_injection.feed_seat_map` | map | `{}` | Platform chat id -> the seat whose feed it is. Unmapped chats default to `premiere`. |
| `context_injection.recency_halflife_hours_feed` | float | `24.0` | Feed recency half-life. Feeds go stale faster than DMs. |
| `context_injection.recency_halflife_hours_dm` | float | `168.0` | DM recency half-life. |
| `context_injection.weight_recency` | float | `1.0` | Ranking weight for recency. |
| `context_injection.weight_lexical` | float | `1.0` | Ranking weight for lexical affinity. |
| `context_injection.weight_authority` | float | `0.5` | Ranking weight for author authority. |

## The background-model sections

`[advisor]`, `[recap]`, `[moa]`, `[auxiliary_models]`, `[credential_pool]`
and `[providers]` all speak the same auxiliary-model vocabulary, so they are
documented once here.

Each `AuxiliaryModelConfig` slot has four keys, all optional and all
defaulting to unset (fall back to the primary model / provider):

| Sub-key | Type | Default | What it changes |
|---|---|---|---|
| `<slot>.model` | string? | `None` | The model id. Empty means the primary model. |
| `<slot>.provider` | string? | `None` | Which provider profile serves it. |
| `<slot>.base_url` | string? | `None` | Endpoint override. |
| `<slot>.api_key` | string? | `None` | Credential override. |

| Section | Slots | Default | What it changes |
|---|---|---|---|
| `[advisor]` | `enabled`, `model` (plus its four slot sub-keys), `instructions`, `continue_severity`, `suppress_streak`, `max_notes_per_update`, `timeout_secs` | engine OFF | The always-on transcript watchdog: a side-channel reviewer that notes concerns after each completed turn WITHOUT joining the acting agent's context. `timeout_secs` = 120, `max_notes_per_update` = 4, `suppress_streak` = 3, `continue_severity` = `concern`. |
| `[recap]` | `model` (plus its four slot sub-keys), `instructions` | idle window 240 s | The idle-recap engine: an ephemeral side call that renders a ~40-word "where the session stands" over a SNAPSHOT of the transcript. It has **no `enabled` key**: the on/off switch is `agent.recap_idle_seconds`, where `0` disables and the default `240` enables. |
| `[moa]` | `references` (array of slots), `aggregator`, `max_tokens`, `temperature`, `timeout_secs` | `enabled = false` | Mixture-of-Agents: N reference models advise, one aggregator synthesises. `timeout_secs` = 60. Inert when `references` is empty. |
| `[auxiliary_models]` | `vision`, `compression`, `web_extract` (INERT), `image_gen`, `embeddings`, `search`, `memory`, `code_execution`, `reasoning` | all `None` | Per-task model slots for the named subsystem. `web_extract` is inert because the IGS-backed extractor returns raw markdown with no LLM step. |
| `[credential_pool]` | `enabled`, `strategy`, `strategies` | `enabled = false` | Multi-key rotation. `strategy` is `fill_first` (default) / `round_robin` / `random` / `least_used`; `strategies` overrides per provider. |
| `[providers]` | `models`, `model_routes`, `fallback`, `fallback_chain`, `embedding_routes` | all empty | Provider profiles and routing. `models` is the named profile map; `fallback_chain` is the ordered cross-provider chain (unknown profile keys are skipped with a warning); `fallback` is the profile used when no route matches. |

## The remaining sections

| Section | Key | Type | Default | What it changes |
|---|---|---|---|---|
| `[autonomous]` | `interval_secs` | int | `300` | Loop tick interval. |
| | `todo_path` | path | `"TODO.md"` | The task ledger the loop reads. |
| | `status_path` | path | `"autonomous-status.toml"` | The repo-local state file. |
| | `test_command` | string | `"cargo test --workspace"` | The verification command. |
| | `git_remote` / `git_branch` | string | `"origin"` / `"agent-dev"` | Where the loop publishes. |
| | `commit_message` | string | `"Auto-commit by operant-rs"` | Commit trailer. |
| | `command_timeout_secs` | int | `900` | Cap on the test command. **LOADED, NOT DESCRIBABLE**. |
| | `max_failures_per_state` | int | `3` | Repeated-failure pause threshold, reloaded across restarts. **LOADED, NOT DESCRIBABLE**. |
| `[logging]` | `level` | string | `"info"` | Log level. `--verbose` overrides to debug. |
| | `format` | string | `"pretty"` | Log format. |
| | `log_file` | string? | `None` | A configured log file. Rotated at 32 MB, 2 generations. An empty string means unset. |
| | `with_target` / `with_thread_ids` / `with_file` / `with_line_number` | bool | `false` | tracing-subscriber decorations. |

**All 7 `[logging]` keys are LOADED but NOT DESCRIBABLE** — `logging.*` is
in `KNOWN_KEYS` and absent from `CONFIG_KEYS`, so `operant config describe`
and `config notes` cannot render any of them. Their types and defaults above
come from `LoggingSettings` (`crates/operant-core/src/config.rs:1313`).

### `[compression]` — the evidence layer (4 keys)

| Key | Type | Default | What it changes |
|---|---|---|---|
| `compression.enabled` | bool | `false` | Master switch. While false the evidence layer is a NO-OP at the tool-result chokepoint, so the section is behaviour-neutral. |
| `compression.min_size` | int | `512` | Results smaller than this are left alone. |
| `compression.shell` | `aft` \| `native` \| `rtk` | `aft` | WHO compresses shell output: `aft` leaves AFT's own tool alone and compresses the shell tools operant owns; `native` compresses every payload itself; `rtk` leaves it to the `rtk` binary. |
| `compression.families` | map | `{}` | Per-subject-family chain overrides, keyed by subject name. |
| `[proxy]` | `enabled` | bool | `false` | Enable proxy support at all. |
| | `http_proxy` / `https_proxy` / `all_proxy` | string? | `None` | Proxy URLs (http, https, socks5, socks5h). |
| | `no_proxy` | array | `[]` | Bypass list, same format as `NO_PROXY`. |
| | `scope` | `zeroclaw`\|… | `zeroclaw` | Proxy application scope. |
| | `services` | array | `[]` | Service selectors used when `scope = "services"`. |
| `[observability]` | `backend` | `none`\|`log`\|`verbose`\|`prometheus`\|`otel` | `"none"` | Which observer the gateway mounts on the cron scheduler and delivery loop. Default is dark: no observer, byte-identical. |
| | `otel_endpoint` / `otel_service_name` / `otel_headers` | string? / string? / map? | `None` | **NOT APPLIED** (RT-274): accepted, ignored. |
| | `runtime_trace_mode` / `_path` / `_max_entries` | string / string / int | `"none"` / `"state/runtime-trace.jsonl"` / `200` | **NOT APPLIED** (RT-274). |
| `[cli]` | `enabled` | bool | `false` | Master switch for the runtime CLI-admission surface. Unlike MCP, opt-in, because a PATH is full of binaries the agent has no business running. |
| | `inject_help` | `usage`\|`description`\|`off` | `"usage"` | What the `<cli_reference>` block carries per opted-in CLI. Raw full `--help` is deliberately NOT offered (3,643 tok/request for a 15-CLI roster). |
| | `default_timeout_secs` | int | `30` | Global per-call timeout; a tool entry's own `timeout_secs` overrides. |
| | `max_help_lines` | int | `60` | **ACCEPTED BUT NOT APPLIED** (RT-278): the surface that would serve help is the unlanded Phase-C `cli_exec`. |
| | `tools` | array of tables | `[]` | The opted-in binaries. Each entry has `name`, `enabled` (default true), `description`, `global_args`, `timeout_secs`, `allow_subcommands`. |
| `[checkpoints]` | `enabled` | bool | `true` | Filesystem checkpoints (shadow git). |
| | `base_dir` | path? | `None` (`~/.operant/checkpoints`) | Shadow store location. |
| | `max_snapshots` | int | `20` | Snapshots kept per working directory. |
| | `max_total_size_mb` | int | `500` | A working tree over this is skipped: a checkpoint is a safety net for a mutation, not a backup. |
| | `max_file_size_mb` | int | `10` | A tree containing a larger single file is skipped. |
| `[tts]` | `provider` | string | `"phonon"` (committed) | The TTS provider. See the note above about the working-tree `"kokoro"`. |
| | `voice` | string? | `None` | Provider-specific default voice. |
| | `enabled` | bool | `false` | Master switch for TTS. |
| `[pk]` | `enabled` | bool | `false` | **NOTHING READS THIS TABLE** (RT-136): the live kernel surface is `[tools.kernel]`. Kept so older configs parse. |
| | `sidecar_script` / `ndjson_socket` / `exec_timeout_secs` | various | `None` / `None` / `30` | Same: accepted, unread. |
| `[tui]` | `theme` | string | `"default"` | Theme name. |
| | `rich_output` | bool | `true` | Route logs to the sink so they do not corrupt the frame. |
| | `terminal_scroll_mode` | bool | `false` | Inline viewport, no alternate screen, mouse capture OFF: the terminal's native scrollback owns the history. |
| | `show_tool_calls` | bool | `true` | Render tool rows. Read at init (`crates/operant-cli/src/tui/app/init.rs:110`) and toggled live by `/tools`. |
| | `show_iterations` | bool | `true` | **ACCEPTED BUT UNREAD** (RT-136): the render layer consults none of them. |
| | `landing_title` | string | `"HERMES"` | **ACCEPTED BUT UNREAD** (RT-136). |
| | `prompt_placeholder` | string | `"Ask anything... \"Fix a TODO in the codebase\""` | **ACCEPTED BUT UNREAD** (RT-136). |
| | `refresh_rate_ms` | int | `80` | **ACCEPTED BUT UNREAD**: frames are paced adaptively (`tui::redraw::redraw_interval`). |
| | `compact_width` / `medium_width` | int | `96` / `120` | **ACCEPTED BUT UNREAD** (RT-136). |
| | `external_editor` | string | `""` | Command Ctrl+G opens (empty = `$VISUAL`, `$EDITOR`, `vi`). |
| | `double_escape_action` | `rewind`\|`none` | `"rewind"` | What a double Esc on an empty composer does. |
| | `word_completion` | `ngram`\|`off` | `"ngram"` | The ghost-text engine. |
| | `word_completion_threshold` | float? | `None` (τ 0.15) | Show threshold for ghost text. |
| | `model_favorites` | array | `[]` | The picker's starred models. |
| | `model_default` | string? | `None` | Where a FRESH session's picker lands. |
| | `colors` | map | `{}` | Per-role TUI colour overrides (`role = "#rrggbb"`), applied on top of the active theme. |

## Failure modes — what the operator sees

1. **An unknown key.** `deny_unknown_fields` would refuse the whole file, so
   the loader takes the upgrade-parking path: `config_heal::split_unknown`
   re-walks the file against `KNOWN_KEYS`, parks what a NEWER binary owns,
   and `heal_and_persist` writes the healed document back ONCE, atomically,
   with one `warn!` per row naming key, old value, new value and why. An
   idempotency test pins that a second load heals nothing.
2. **A key with a value the loader considers out of range.** Clamped, named,
   persisted. `Healed::clamped` is the row type.
3. **A retired key.** Accepted and warned about (the `[memory] agentmemory_*`
   family, `[tools.web] tavily_api_key`, `[browser] provider` when it is not
   `sourcehound`, `[agent] session_reset`). The warning fires only for a value
   the FILE carries, so a clean start is quiet.
4. **`reasoning_effort` with a typo.** FAILS config load, naming the allowed
   set. This is the one validator that refuses rather than heals.
5. **An out-of-range TOML integer.** TOML integers are i64, so a `usize::MAX`
   anywhere in the config makes the whole file unwritable. That is why
   `context_compression_summary_max_chars` is a `u32`.

## Intention

The config is the operator's contract with the harness, so the keys are
derived rather than remembered: `scripts/gen-config-registry.py` walks the
settings structs and emits `config_registry.rs`, and `operant config describe`
renders THAT, so the list an agent reads cannot drift from the list the binary
accepts. The two-count split (385 accepted, 302 describable) is an honest
artefact of that generator's reach rather than a design goal, and closing it
is a generator change, not a doc change. Everything dark-merged is dark for
the same reason: a personal agent that changed behaviour because a key gained
a default would be a release nobody could safely install.

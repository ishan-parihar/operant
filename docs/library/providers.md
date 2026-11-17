# Providers — the model path

Owns: how operant talks to LLM providers — clients, provider catalog,
model discovery, credentials, rate limiting, reasoning-model handling.
Not for: the loop that calls them ([agent-loop.md](agent-loop.md)),
fallback policy ([agent-loop.md](agent-loop.md) §fallback), OAuth account
machinery (here, §OAuth), the `[client]` config keys
([config.md](config.md)).

Source: `crates/operant-core/src/client.rs` (OpenAI-compatible + SSE),
`crates/operant-core/src/agent/clients/` (Anthropic adapter;
`clients/openai.rs` is the loop's own adapter onto that client),
`provider_catalog.rs`, `model_catalog.rs`, `models_dev.rs`,
`credential_pool.rs`, `credential_files.rs`, `rate_limiter.rs`,
`agent/pooled_client.rs` (per-provider rotation),
`reasoning_timeouts.rs`, `oauth/` (mod, pkce, singleflight, device,
loopback, http, store) + `oauth_refresh.rs`,
`crates/operant-cli/src/provider.rs` (CLI registry),
`crates/operant-core/src/tools/platform_bases.rs`.

## The clients

Two wire clients, selected by provider name:

- **`client.rs`** — a lightweight custom OpenAI-compatible client (reqwest
  + serde) with SSE streaming and `reasoning_content` support for
  extended-thinking models. No compiled-in model list: a provider's models
  come from its `/models` endpoint, then the models.dev catalog, then
  configured models — `model_catalog.rs` carries the chosen source on
  every row.
- **`agent/clients/`** — the Anthropic adapter (messages API,
  `cache_control` breakpoints on the system prompt, thinking blocks).

Behind those two wire clients sit four modules that name a route rather than
carry bytes. `agent/model_client.rs` is the provider-agnostic trait itself:
`ModelClient` plus the `ChatRequest` and `StreamChunk` types every adapter
implements. `agent/chat_provider.rs` is the DECLARATION side — one trait
implementation per vendor plus one registration line, naming that vendor's
alternate spellings and model prefixes and its models.dev id for pricing;
adding a provider is that pair of edits, and the live call stays in the
fallback chain. `agent/runtime_key.rs` is the structural identity that
replaces free-form display strings: a `RuntimeKey` is `{ vendor, model,
revision }`, so two spellings of one runtime produce one key, and cache
entries, cooldown bookkeeping and failover decisions key off something
structural. `agent/provider_registry.rs` is the cross-provider fallback
registry — a chain entry is turned into a `RuntimeKey` and the anti-thrash
check runs over it, so an Anthropic → OpenAI chain does not oscillate.

Tool schemas are normalized per provider by `schema.rs` +
`crates/operant-api/src/schema.rs` (JSON Schema cleaning: unsupported
keywords stripped, `$ref` resolved, so one tool definition serves every
provider).

## The provider catalog

`provider_catalog.rs` is the static name-recognition surface (157 names)
`operant doctor` validates configuration against; it replaced the deleted
`operant-providers` crate's entire role (RT-153, iter-1215). The runtime's
model path is `client.rs` — the factory the doctor once called is gone.
`crates/operant-cli/src/provider.rs` holds the CLI-side provider registry
(`ProviderDef` + `PROVIDERS`) with each provider's `/models` wire family;
model lists are never compiled in.

`models_dev.rs` is the offline-first model catalog with capability
metadata (model lookup, capability queries, agentic-model filtering),
ported from operant-agent's models_dev.py (the Python reference is a
machine-local checkout, not a repo path).

## Credentials

Two layers:

- **`credential_pool.rs`** — multi-credential pool for same-provider
  failover and load balancing: thread-safe, configurable selection
  (fill-first, round-robin, random, least-used), seeding from
  env/config/OAuth refresh (iter-66).
- **`agent/pooled_client.rs`** — the rotation LOOP, inside the client rather
  than above it. `PooledModelClient` wraps a `ModelClient` with that
  provider's `CredentialPool` and runs hermes's
  `mark_exhausted_and_rotate` on a classified credential error: the key that
  failed is marked exhausted with an error-class cooldown (401 → 5 min,
  429 → 1 hour, a sole-credential transient → 60 s) and the next available
  key is tried, bounded by the attempt budget. An error class with no
  cooldown is not retried against a fresh key, because that is a request
  the provider rejected rather than a credential that ran out.
- **`credential_files.rs`** — the registry of protected credential files
  the agent must never read or send (`tools/file_tools.rs`'s hard-deny
  list is the built-in layer; this is the registerable layer on top).
- The account side: `[oauth]` (authorization-code + PKCE, device-code,
  single-flight refresh; `mcp_oauth.rs` is the separate MCP-server OAuth
  path), `oauth_refresh.rs` (background refresh for provider-account
  tokens — Anthropic, Codex, xAI, Nous).
- The flows under `oauth/`: `pkce.rs` mints the RFC 7636
  `code_verifier` / `code_challenge` pair and the RFC 6749 §10.12 `state`
  parameter; `singleflight.rs` (`RefreshCoordinator`) collapses N
  concurrent callers that discover the same expiry into ONE
  rotating-refresh-token spend per account; `device.rs` is the RFC 8628
  device grant for a box with no browser, with the SERVER owning the
  polling cadence; `loopback.rs` is the redirect listener that binds an
  ephemeral port instead of a fixed one (a hardcoded 8080 collides, can be
  held by a stale process, and can be pre-bound by a hostile local process
  that then receives the authorization code); `http.rs` is the injectable
  `HttpEndpoint` boundary every flow depends on instead of `reqwest`, so a
  test drives the code, device and refresh paths with canned responses;
  `store.rs` is the account INDEX — keys and ordering, never tokens — whose
  `TokenRef` serializes a truncated SHA-256 fingerprint, enforced
  structurally rather than by convention.
- API keys live in `~/.operant/.env` (0600), never in the config file.

## Rate limiting (`rate_limiter.rs`)

Token bucket per model/provider, consumed on each request, refilled at a
fixed rate. On a 429 the bucket is drained and the caller waits
Streaming requests use the same 429 contract: the provider's
`Retry-After` header is honored when present, otherwise the configured
exponential backoff is used before the request is retried within the
configured retry budget.

Usage and cost events name the route that served the request, not the
model configured at startup. When a provider has already reported usage
and the stream then fails, that known usage is still emitted before the
failure is surfaced.

`Retry-After` + exponential backoff. This is the *proactive* half; the
*reactive* half is the error classifier's RateLimit/UpstreamRateLimit
classes (see [agent-loop.md](agent-loop.md)).

## Reasoning models (`reasoning_timeouts.rs`)

Reasoning models that think 120s+ before their first content token get
idle-killed by proxies tuned for chat models. This module carries
stale-timeout floors + thinking-timeout guidance (hermes
`reasoning_timeouts.py` parity) so reasoning routes get the timeouts they
need, and the classifier carries the provider quirks
(`ThinkingSignature`, `ReasoningParamRejected`, `LongContextTier`).

## Platform bases

`tools/platform_bases.rs` — ONE owner of each messaging platform's API
base. Both the gateway adapters and the agent's tools read from it; the
tools used to hard-code public URLs while the adapters read config
directly, so a `TELEGRAM_API_BASE` proxy could split-brain (RT-1xx
family). One module, one truth.

## Switching model or provider

In the TUI, bare `/model` opens the inline picker over the live
catalogue; `/model <selector>` takes the selector grammar
`@role[:level]`, `role[:level]` or `[provider/]modelId[:level]`.
Switching and persisting are separate acts: a switch applies to the
session, and `/model set-default` writes that choice into
`agent.model`. `/refresh-model-list` drops the cached
`~/.operant/cache/models.json` and refetches. From the shell,
`operant model` shows the live model, provider, base URL, streaming,
context window and iteration cap (`--json` for the same as data), and
`operant model set <name>` persists `agent.model`. The underlying keys
are `[client] base_url` and `[client] api_key` for the route,
`[agent] model` for the model, `[agent] fallback_models` plus
`fallback_on_errors` for same-provider fallback, and the
`[providers.models.<id>]` profiles with
`[[providers.fallback_chain]]` for cross-provider failover.

## Intention

The provider surface is deliberately thin: two wire clients instead of a
provider-adapter zoo (the deleted `operant-providers` crate was a second
surface production never called — its role collapsed into the static
catalog the doctor needs). Every capability claim a provider makes is
checked against `models_dev` metadata rather than assumed — the
"agentic model filtering" exists because a non-agentic model behind an
agentic config produces loops that fail in confusing ways.

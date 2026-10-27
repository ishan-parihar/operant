# Providers — the model path

Owns: how operant talks to LLM providers — clients, provider catalog,
model discovery, credentials, rate limiting, reasoning-model handling.
Not for: the loop that calls them ([agent-loop.md](agent-loop.md)),
fallback policy ([agent-loop.md](agent-loop.md) §fallback), OAuth account
machinery (here, §OAuth), the `[client]` config keys
([config.md](config.md)).

Source: `crates/operant-core/src/client.rs` (OpenAI-compatible + SSE),
`crates/operant-core/src/agent/clients/` (Anthropic adapter),
`provider_catalog.rs`, `model_catalog.rs`, `models_dev.rs`,
`credential_pool.rs`, `credential_files.rs`, `rate_limiter.rs`,
`reasoning_timeouts.rs`, `oauth/` + `oauth_refresh.rs`,
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
- **`credential_files.rs`** — the registry of protected credential files
  the agent must never read or send (`tools/file_tools.rs`'s hard-deny
  list is the built-in layer; this is the registerable layer on top).
- The account side: `[oauth]` (authorization-code + PKCE, device-code,
  single-flight refresh; `mcp_oauth.rs` is the separate MCP-server OAuth
  path), `oauth_refresh.rs` (background refresh for provider-account
  tokens — Anthropic, Codex, xAI, Nous). API keys live in
  `~/.operant/.env` (0600), never in the config file.

## Rate limiting (`rate_limiter.rs`)

Token bucket per model/provider, consumed on each request, refilled at a
fixed rate. On a 429 the bucket is drained and the caller waits
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

## Intention

The provider surface is deliberately thin: two wire clients instead of a
provider-adapter zoo (the deleted `operant-providers` crate was a second
surface production never called — its role collapsed into the static
catalog the doctor needs). Every capability claim a provider makes is
checked against `models_dev` metadata rather than assumed — the
"agentic model filtering" exists because a non-agentic model behind an
agentic config produces loops that fail in confusing ways.

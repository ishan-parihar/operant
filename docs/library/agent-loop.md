# The agentic core — the ReAct loop

Owns: the mechanism of operant's agent loop — turn structure, streaming,
retry, fallback, budgets, and the turn-end seam.
Not for: memory ([memory.md](memory.md)), context compaction
([context-management.md](context-management.md)), the provider clients
themselves ([providers.md](providers.md)).

Source: `crates/operant-core/src/agent/` (mod, run, builders, prompting,
stream, turn_finalizer, turn_retry_state, turn_rules, turn_context,
error_classifier, fallback, iteration_budget, stream_retry_budget,
turn_end_heuristics, message_safety, runtime_key, pooled_client,
provider_registry, chat_provider, model_client).

## The loop in one paragraph

`OperantAgent::run()` implements the ReAct pattern: build the prompt
(system + context), call the model, parse tool calls from the stream,
execute them (concurrently, behind a semaphore), append results, repeat —
until the model produces a final answer with no tool calls, the iteration
budget is exhausted, or an unrecoverable error is classified. Every failure
along the way is classified into a 24-class `FailoverReason` taxonomy
(`agent/error_classifier.rs`) that decides retry, fallback, compression, or
abort — the loop self-heals instead of dying.

## The turn, in order

1. **Turn context assembly** (`agent/turn_context.rs`,
   `agent/builders.rs`) — the system prompt is a **frozen prefix**
   (base + skills guidance): everything before the latest user message is
   byte-stable between turns, so the provider prompt cache stays valid. The
   volatile per-turn context (memory recall, workspace, harness, MoA advice)
   rides the TAIL as a `<turn_context>` prefix on the latest user message —
   never inside the frozen prefix. See
   [context-injection.md](context-injection.md).
2. **The model call** — streamed (`agent/stream.rs`), with a tolerant XML /
   tool-call parser that fires callbacks the moment a tool block is detected
   (`parser.rs`). The parser distinguishes native `tool_calls` from
   prose-heuristic invocations: since iter-1280 the prose heuristic is armed
   only inside explicit `tool_call` blocks, because armed-on-all-text it
   fabricated 136 phantom `process` calls from a streamed markdown table.
3. **Tool execution** (`agent/run.rs` → `ToolRegistry::execute`) —
   concurrent behind an 8-worker pool + semaphore (iter-56). Each call passes
   the guardrail ladder (see [tools.md](tools.md)).
4. **Turn finalization** (`agent/turn_finalizer.rs`) — memory
   `sync_turn` auto write-back, queued prefetch for the next turn, the
   `TurnEnd` event (below), turn summary accounting.
5. **Next iteration or stop** — `turn_end_heuristics.rs` detects cut-off /
   truncated responses and requests continuation instead of surfacing a
   partial answer as final (hermes `_has_natural_response_ending` parity).

## Failure handling: classify, then act

Every provider error is mapped to one of 24 `FailoverReason` classes
(`agent/error_classifier.rs`): Auth, AuthPermanent, Billing, RateLimit,
UpstreamRateLimit, Overloaded, ServerError, Timeout, SslCertVerification,
ContextOverflow, PayloadTooLarge, ImageTooLarge, ModelNotFound,
ProviderPolicyBlocked, ContentPolicyBlocked, FormatError,
InvalidEncryptedContent, MultimodalToolContentUnsupported, ThinkingSignature,
ReasoningParamRejected, LongContextTier, OauthLongContextBetaForbidden,
LlamaCppGrammarPattern, Unknown. Each class answers three questions:
should we retry, should we fall back to another model, should we compress
the context? `ContextOverflow` triggers auto-compression via
[context-management.md](context-management.md) and a retry.

## The retry ladder (three independent budgets)

- **Turn retries** (`agent/turn_retry_state.rs`) — a dropped or failed
  response stream retries up to `DEFAULT_MAX_RETRIES` (3) times.
- **Wall-clock retry budget** (`agent/stream_retry_budget.rs`) — an
  attempt count says nothing about how long a person waits; this bounds the
  whole ladder by wall-clock so a dead-but-accepting endpoint cannot hold a
  turn hostage across retries.
- **Iteration budget** (`agent/iteration_budget.rs`) — a thread-safe
  consume/refund counter; the parent cap comes from `max_iterations`
  (default 90). Compression calls refund their iterations (non-productive
  calls don't burn budget). At the limit the loop does a **grace call**:
  one final summarize-and-stop turn instead of a hard stop (iter-57).

## Model fallback (`agent/fallback.rs`)

`FallbackModelClient` wraps the primary model with a fallback chain: on a
retryable error (5xx, 429, network) the next model in the chain is tried.
Fallback is **per-request** — every call starts again at the primary.
Two config sources: `agent.fallback_models` (same-client, same-provider)
and cross-provider chains via `[providers]`. Reasoning-model quirks are
classed per provider (`ThinkingSignature`, `ReasoningParamRejected`,
`LlamaCppGrammarPattern`) so fallback targets a model that accepts the
current request shape.

## The turn-end seam (`turn_end.rs`)

Exactly one `TurnEnd` event is emitted per completed turn, after memory
`sync_turn` / prefetch. Post-turn features subscribe to a `TurnEndBus`
instead of wrapping the agent loop — the attachment point for reflection,
the advisor, recap, and dreaming. See
[context-injection.md](context-injection.md) for what subscribes.

## Self-healing summary

| Failure | Mechanism | Where |
|---|---|---|
| Stream drop mid-response | retry ladder + wall-clock bound | `stream_retry_budget.rs` |
| 429 / rate limit | token-bucket pre-throttle + backoff | `rate_limiter.rs` |
| Context overflow | classify → compress → retry | `error_classifier.rs` + `context_management.rs` |
| Bad / hallucinated model | per-request fallback chain | `fallback.rs` |
| Truncated final answer | continuation prompt | `turn_end_heuristics.rs` |
| Repeated identical tool calls | guardrail ladder | `tool_guardrails.rs` |
| Reasoning model slow-start | timeout floors | `reasoning_timeouts.rs` |

## Intention

The loop is hermes-agent's Python loop rebuilt in Rust with the same
behavioral contract (classify-then-act, degrade-don't-die) but without the
runtime: single binary, no GC pauses mid-stream, memory-safe persistence.
The heavy audit trail for how each piece landed is in the frozen 2026-08
parity audits under `docs/audit/` (see `docs/PAGE-INDEX.md` for the
shelf; the core-loop audit itself is `docs/AUDIT_2026-08-02.md`)
and the BUGS.md ledger; the live defect-fix state lives in
`docs/audit/2026-06-19-BUGS.md`.

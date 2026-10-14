# 03 — Wire reasoning effort end to end

## Evidence

- The TUI has an effort state, `App::effort_level` (`tui/app/mod.rs:212`). It is set by the
  `/effort` picker (`tui/effort_picker.rs`; keys at `app/key_handling.rs:112-125`) and by
  Left/Right in the model picker (`key_handling.rs:595-613`), and seeded from the legacy
  `settings.json` (`app/init.rs:19`). **Nothing reads it to build a request.**
- Core request type `ChatRequest` (`operant-core/src/agent/model_client.rs:20-27`) has no
  effort field. Every main-turn request is built by `fitted_request`
  (`agent/compress.rs:443-469`).
- The OpenAI-compatible path (`agent/clients/openai.rs:61-80` →
  `client.rs::build_chat_request` `:626-700`) sends `model`, `messages`, `stream`,
  `stream_options`, `max_tokens`, `temperature` and `tools`. No reasoning field.
- The native Anthropic client (`agent/clients/anthropic.rs`, body at `:119`) never sends
  `thinking`.
- `operant-core/src/config.rs` (`AgentSettings`, near `show_reasoning` `:417`) has no
  effort setting. `operant-config/src/schema/agent_cfg.rs:13` has a validator,
  `normalize_reasoning_effort` (`minimal|low|medium|high|xhigh`). The unused
  `operant-providers/src/compatible.rs:557` (`reasoning_effort_for_model`) is a reference
  for model gating.
- The user's provider is an OpenAI-compatible local proxy (`[client] base_url =
  http://localhost:20129/v1`, model `small-stack`).

## Design

One effort value is owned by the agent and sent on every main-turn request, through the
shape each provider family accepts. If a provider rejects the field, the request is retried
once without it and the effort is marked unsupported for that model for the rest of the
session. That fallback is what makes wiring it safe ("some providers reject that setting").

```
[agent]
reasoning_effort = ""        # "" = don't send; minimal|low|medium|high|xhigh
reasoning_effort_param = "auto"  # auto | reasoning_effort | reasoning_object | anthropic_thinking | none
```

`auto` picks the shape:

| Endpoint | Shape |
|---|---|
| base_url contains `openrouter` or `kilo` | `"reasoning": {"effort": "<e>"}` |
| `api.openai.com`, or model id `o1*/o3*/o4*/gpt-5*/codex` | `"reasoning_effort": "<e>"` |
| native Anthropic client | `"thinking": {"type": "enabled", "budget_tokens": B}`, with B from {minimal: 1024, low: 2048, medium: 8192, high: 16384, xhigh: 32000}, clamped below `max_tokens`, and temperature dropped (Anthropic requires that when thinking is on) |
| anything else (proxies like the user's) | `"reasoning_effort": "<e>"`. LiteLLM-style proxies and most OpenAI-compatible gateways accept it, and the rejection fallback covers the rest |

`minimal` maps to `low` for shapes that don't know `minimal`; `xhigh` maps to `high` for
the same reason.

## Steps

1. **Config** (`operant-core/src/config.rs` `AgentSettings` + `Default`): add
   `reasoning_effort: String` (empty default, validated with the same set as
   `normalize_reasoning_effort`; an invalid value is a config error naming the allowed
   values) and `reasoning_effort_param: ReasoningEffortParam` (enum, default `Auto`).
   Update `operant.example.toml` with comments.
2. **Request type:** add `reasoning_effort: Option<ReasoningEffort>` to `ChatRequest`, with
   a `with_reasoning_effort` builder. Every existing constructor stays `None`, so auxiliary
   calls (guardian, compressor, recap, background review) send nothing. Only main turns
   carry effort.
3. **Agent state:** `OperantAgent` gets `reasoning_effort: Arc<RwLock<Option<ReasoningEffort>>>`,
   initialized from config, plus `pub fn set_reasoning_effort(&self, e: Option<ReasoningEffort>)`
   and a getter. `fitted_request` attaches it. Also keep
   `unsupported_effort_models: Mutex<HashSet<String>>` and skip attaching for those.
4. **Serialization:**
   - Core OpenAI-compatible client: thread the effort through
     `OpenAIClient::chat_streaming` / `chat` → `build_chat_request` (new parameter, or pass
     the whole `ChatRequest`, which is the cleaner change and the one to prefer), applying
     the `auto` table.
   - Anthropic client: add `thinking`, and drop `temperature` when it is present.
   - Pooled and fallback clients pass the request through unchanged; check
     `pooled_client.rs:191` and `fallback.rs`.
5. **Rejection fallback:** in the stream/request error path, classify HTTP 400/422 whose
   body mentions `reasoning`, `reasoning_effort`, `thinking`, `effort` or
   `unrecognized`/`unknown` + `parameter`. `agent/error_classifier.rs` already
   pattern-matches bodies (`:381` checks "thinking"), so add a `ReasoningParamRejected`
   class there. On that class: add the model to `unsupported_effort_models`, emit
   `AgentEvent::Notice { "Reasoning effort isn't supported by <model>; continuing without it" }`,
   and retry the same request once without the field. This retry must not count against
   the stream retry budget.
6. **TUI:**
   - `App::effort_level` becomes a view of the agent's effort. Picker confirm, model-picker
     Left/Right and **Shift+Tab** (decision 1 in INDEX; the key change itself is plan 10)
     call `agent.set_reasoning_effort(...)` through `run_pending_agent_actions`, the same
     channel `/compact` uses (`TuiApp::run_pending_agent_actions`).
   - Seed from the TOML value, not from `settings.json`. Remove the `settings.json`
     `effort_level` read.
   - Effort changes are session-scoped (`/effort high` lasts until exit). Persist only
     through `/settings` (TODO.md Pending: settings rebuild).
   - Show it in the status line next to the model (`small-stack · effort high`), and show
     `effort n/a` when the model was marked unsupported.
7. **Gateway:** the same `[agent] reasoning_effort` applies, because the gateway uses
   `OperantAgent`. No new surface.

## Tests (RED first)

- `build_chat_request_includes_reasoning_effort_for_auto_openai_compatible`: with
  base_url `http://localhost:20129/v1` and effort `high`, the body has
  `"reasoning_effort":"high"`.
- `..._openrouter_uses_reasoning_object` and `..._omits_when_none`.
- Anthropic: `thinking.budget_tokens` is clamped below `max_tokens`, and `temperature` is
  absent.
- Agent: with a mock client that returns 400 `{"error":"Unrecognized request argument
  supplied: reasoning_effort"}` once, the turn succeeds on the retry, a Notice is emitted,
  and the next request omits the field.
- Auxiliary requests (guardian, compressor) never carry effort.
- TUI: Shift+Tab cycles `off → low → medium → high → xhigh → off`, and the agent setter
  receives each value. The status line shows the effort.

## Acceptance

Live against the user's proxy: with `/effort high`, the request body (log at `debug` with
`target: "llm_request"`, or capture with a local echo proxy) contains the field. Turns
still succeed if the proxy rejects it. `operant.example.toml` and `README` document the
two config fields and the shape table.

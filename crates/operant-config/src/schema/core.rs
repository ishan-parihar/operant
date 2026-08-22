//! `core` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use crate::autonomy::AutonomyLevel;
use crate::domain_matcher::DomainMatcher;
use crate::provider_aliases::{is_glm_alias, is_zai_alias};
use crate::traits::{ChannelConfig, HasPropKind, PropKind};
use crate::validation_bail;
use anyhow::{Context, Result};
use directories::UserDirs;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(unix)]
use tokio::fs::File;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use super::*;

/// Multi-client workspace isolation configuration.
///
/// When enabled, each client engagement gets an isolated workspace with
/// separate memory, audit, secrets, and tool restrictions.
#[allow(clippy::struct_excessive_bools)]
/// Opaque state the `operant onboard` flow writes so it can tell, on a
/// re-run, which sections the user has already walked through at least
/// once — which lets it offer "Reconfigure? [y/N]" skip gates instead of
/// forcing users through every field again.
///
/// This is meta-state about the onboard process, not user-facing config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "onboard_state"]
pub struct OnboardStateConfig {
    /// Section keys the user has completed at least once via onboard.
    /// Values are the lowercased Section variant names
    /// (`"workspace"`, `"providers"`, …).
    #[serde(default)]
    pub completed_sections: Vec<String>,
}

/// Named provider profile definition.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "providers.models"]
pub struct ModelProviderConfig {
    /// Secret API token for this provider — grab it from the provider's dashboard (OpenAI platform, Anthropic console, OpenRouter keys page, etc.). Stored via the OS keyring when possible; never commit it to config.toml directly.
    #[secret]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Additional API keys for this provider's credential pool (hermes
    /// multi-key-per-provider parity). The primary `api_key` is tried first;
    /// these rotate in on 401/429/billing exhaustion, each with its own
    /// error-class bench (hermes `load_pool(provider)` seeds every key a
    /// provider owns). Empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<String>,
    /// Override the provider type label. Rarely needed — only useful when you run two profiles against the same provider type (e.g. two different OpenAI-compatible gateways) and want to tell them apart in logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// HTTPS endpoint the client hits. Override when pointing at a self-hosted gateway (LiteLLM, vLLM, Ollama), a regional endpoint, or a proxy; leave unset to use the provider's public endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Path suffix appended to the base URL. Almost no one needs this — only touch it for custom reverse-proxy routing where your gateway mounts the API under a non-standard prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_path: Option<String>,
    /// Model identifier to send with each request — the ID string from the provider's catalog (e.g. `gpt-4o`, `claude-sonnet-4-5`, `llama-3.3-70b`). Must match a model the provider actually serves on this account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Sampling temperature passed to the model. Lower values (0.0–0.3) give
    /// deterministic, near-verbatim output — fits code, routing, summarization.
    /// Higher values (0.7–1.2) give more varied output — fits open-ended chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// HTTP request timeout in seconds. Bump this for slow local providers (Ollama on CPU, big local models) or high-latency networks; leave unset otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Extra HTTP headers sent with every request. Niche — used for auth bridges, corporate proxies, or custom gateways that demand a tracing header. Most users never touch this; edit `config.toml` directly if you need it.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra_headers: HashMap<String, String>,
    /// Wire protocol flavor: `"responses"` for OpenAI's Codex/Responses API, `"chat_completions"` for everything else (OpenAI chat, Anthropic, OpenRouter, Groq, local gateways). Auto-selected per provider — only override if you're forcing an unusual combination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_api: Option<String>,
    /// When true, the client pulls credentials from `OPENAI_API_KEY` or `~/.codex/auth.json` instead of the `api_key` field above. Turn on only for the OpenAI Codex provider; leave off for standard API-key providers.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_openai_auth: bool,
    /// Azure OpenAI resource name (the `<resource>` part of `<resource>.openai.azure.com`). Azure-only; ignore for OpenAI, Anthropic, and everything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_resource: Option<String>,
    /// Azure OpenAI deployment name — the deployment you created in Azure AI Studio that wraps a specific model. Azure-only; ignore for other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_deployment: Option<String>,
    /// Azure OpenAI API version string (e.g. `2024-10-21`). Azure-only; must match a version your resource supports. Ignore for other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub azure_openai_api_version: Option<String>,
    /// Hard cap on response length in tokens. Most models enforce sensible built-in limits already — leave unset unless you specifically need to clip long outputs for cost or latency reasons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Provider-specific quirk: fold the system prompt into the first user message instead of sending a separate system role. Only needed for models that reject (or mishandle) a standalone system role — e.g. certain older Mistral variants.
    #[serde(default, skip_serializing_if = "is_false")]
    pub merge_system_into_user: bool,
    /// Extra JSON parameters to include in API requests.
    /// Merged at the top level of the request body, allowing provider-specific
    /// features (routing, transforms, etc.) without code changes.
    /// Example: `provider_extra = { provider = { only = ["Anthropic"] } }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_extra: Option<serde_json::Value>,
    /// Per-provider input/output token pricing (USD per 1M tokens). When set,
    /// merged into the cost-tracking lookup at `<provider_id>/<model>` so the
    /// budget surface attributes spend correctly even when the same model is
    /// served by different providers at different rates. Top-level
    /// `[cost.prices.<key>]` entries continue to take precedence on conflict;
    /// this field is purely additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    /// Override the provider's default for native tool calling.
    /// `None` (default) honors the provider's built-in choice. `Some(true)`
    /// forces native tool calls on, `Some(false)` forces text-fallback.
    /// Currently consulted only by the Groq factory, which defaults to
    /// text-fallback because llama-family Groq models reject native tool
    /// calls with HTTP 400. Setting `native_tools = true` re-enables native
    /// tool calling for Groq models that support it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_tools: Option<bool>,
    /// Enable or disable chain-of-thought thinking for models that support it
    /// (e.g. Qwen3, GLM-4). `true` turns thinking on, `false` turns it off.
    /// `None` (default) lets the model decide. Forwarded as `enable_thinking`
    /// in the request body; mirrors the Ollama provider's `think` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think: Option<bool>,
    /// Arbitrary key/value pairs forwarded verbatim as `chat_template_kwargs`
    /// in the request body (llama.cpp-specific). Use this to pass model-family
    /// template variables that control behaviour not exposed by other fields.
    /// Example (Qwen3 thinking suppression):
    ///   `chat_template_kwargs = { enable_thinking = false }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<serde_json::Value>,
    /// Override the Ollama `num_ctx` (context window, in tokens) sent on
    /// every `/api/chat` request. Only consulted when this profile resolves
    /// to the `ollama` provider. Defaults to the framework constant
    /// (`OLLAMA_DEFAULT_NUM_CTX`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_num_ctx: Option<u32>,
    /// Override the Ollama `num_predict` (max output tokens) sent on every
    /// `/api/chat` request. Only consulted when this profile resolves to
    /// the `ollama` provider. Defaults to the framework constant
    /// (`OLLAMA_DEFAULT_NUM_PREDICT`) when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_num_predict: Option<i32>,
    /// Force every Ollama `/api/chat` request to use this temperature,
    /// overriding the per-call value passed through
    /// `Provider::chat_with_system(.., temperature)`. When unset
    /// (`None`, the default), the per-call temperature wins — full
    /// backward compatibility. Only consulted when this profile
    /// resolves to the `ollama` provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_temperature_override: Option<f64>,
}

/// Valid temperature range for all paths (config, CLI, env override).
pub const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = 0.0..=2.0;

/// Defaults to 0 so configs without an explicit `schema_version` are recognized
/// as pre-versioning and get migrated.
pub(crate) fn default_schema_version() -> u32 {
    0
}

/// Verifiable Intent (VI) credential verification and issuance (`[verifiable_intent]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "verifiable-intent"]
pub struct VerifiableIntentConfig {
    /// Enable VI credential verification on commerce tool calls (default: false).
    #[serde(default)]
    pub enabled: bool,

    /// Strictness mode for constraint evaluation: "strict" (fail-closed on unknown
    /// constraint types) or "permissive" (skip unknown types with a warning).
    /// Default: "strict".
    #[serde(default = "default_vi_strictness")]
    pub strictness: String,
}

impl Default for VerifiableIntentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strictness: default_vi_strictness(),
        }
    }
}

// ── Nodes (Dynamic Node Discovery) ───────────────────────────────

/// Configuration for the dynamic node discovery system (`[nodes]`).
///
/// When enabled, external processes/devices can connect via WebSocket
/// at `/ws/nodes` and advertise their capabilities at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "nodes"]
pub struct NodesConfig {
    /// Enable dynamic node discovery endpoint.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of concurrent node connections.
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
    /// Optional bearer token for node authentication.
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Default for NodesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_nodes: default_max_nodes(),
            auth_token: None,
        }
    }
}

/// Determines when a `ToolFilterGroup` is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToolFilterGroupMode {
    /// Tools in this group are always included in every turn.
    Always,
    /// Tools in this group are included only when the user message contains
    /// at least one of the configured `keywords` (case-insensitive substring match).
    #[default]
    Dynamic,
}

/// A named group of MCP tool patterns with an activation mode.
///
/// Each group lists glob patterns for MCP tool names (prefix `mcp_`) and an
/// optional set of keywords that trigger inclusion in `dynamic` mode.
/// Built-in (non-MCP) tools always pass through and are never affected by
/// `tool_filter_groups`.
///
/// # Example
/// ```toml
/// [[agent.tool_filter_groups]]
/// mode = "always"
/// tools = ["mcp_filesystem_*"]
/// keywords = []
///
/// [[agent.tool_filter_groups]]
/// mode = "dynamic"
/// tools = ["mcp_browser_*"]
/// keywords = ["browse", "website", "url", "search"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ToolFilterGroup {
    /// Activation mode: `"always"` or `"dynamic"`.
    #[serde(default)]
    pub mode: ToolFilterGroupMode,
    /// Glob patterns matching MCP tool names (single `*` wildcard supported).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Keywords that activate this group in `dynamic` mode (case-insensitive substring).
    /// Ignored when `mode = "always"`.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// When true, also filter built-in tools (not just MCP tools).
    #[serde(default)]
    pub filter_builtins: bool,
}

/// HMAC tool execution receipt configuration (`[agent.tool_receipts]`).
///
/// Receipts are short HMAC-SHA256 tags appended to tool results so the model
/// cannot claim it ran a tool that never actually executed. See
/// `docs/book/src/security/tool-receipts.md`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent.tool_receipts"]
pub struct ToolReceiptsConfig {
    /// Generate HMAC receipts on every tool execution. Default: `false`.
    /// When false, the entire receipt subsystem is inert (no key, no
    /// generation, no append, no system-prompt addendum).
    #[serde(default)]
    pub enabled: bool,
    /// Append a trailing `Tool receipts:` block to user-visible replies so
    /// receipts are auditable from the channel surface, not just the
    /// internal history. Default: `false`.
    #[serde(default)]
    pub show_in_response: bool,
    /// Inject the receipt-echo instruction into the system prompt so the
    /// model carries receipts verbatim into its response. Default: `true`.
    /// No effect when `enabled = false`.
    #[serde(default = "default_inject_system_prompt")]
    pub inject_system_prompt: bool,
}

impl Default for ToolReceiptsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            show_in_response: false,
            inject_system_prompt: default_inject_system_prompt(),
        }
    }
}

/// Agent orchestration configuration (`[agent]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "agent"]
pub struct AgentConfig {
    /// When true: bootstrap_max_chars=6000, rag_chunk_limit=2. Use for 13B or smaller models.
    #[serde(default)]
    pub compact_context: bool,
    /// Maximum tool-call loop turns per user message. Default: `10`.
    /// Setting to `0` falls back to the safe default of `10`.
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: usize,
    /// Maximum conversation history messages retained per session. Default: `50`.
    #[serde(default = "default_agent_max_history_messages")]
    pub max_history_messages: usize,
    /// Maximum estimated tokens for conversation history before compaction triggers.
    /// Uses ~4 chars/token heuristic. When this threshold is exceeded, older messages
    /// are summarized to preserve context while staying within budget. Default: `32000`.
    #[serde(default = "default_agent_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Enable parallel tool execution within a single iteration. Default: `false`.
    #[serde(default)]
    pub parallel_tools: bool,
    /// Tool dispatch strategy (e.g. `"auto"`). Default: `"auto"`.
    #[serde(default = "default_agent_tool_dispatcher")]
    pub tool_dispatcher: String,
    /// Tools exempt from the within-turn duplicate-call dedup check. Default: `[]`.
    #[serde(default)]
    pub tool_call_dedup_exempt: Vec<String>,
    /// Per-turn MCP tool schema filtering groups.
    ///
    /// When non-empty, only MCP tools matched by an active group are included in the
    /// tool schema sent to the LLM for that turn. Built-in tools always pass through.
    /// Default: `[]` (no filtering — all tools included).
    #[serde(default)]
    pub tool_filter_groups: Vec<ToolFilterGroup>,
    /// Maximum characters for the assembled system prompt. When `> 0`, the prompt
    /// is truncated to this limit after assembly (keeping the top portion which
    /// contains identity and safety instructions). `0` means unlimited.
    /// Useful for small-context models (e.g. glm-4.5-air ~8K tokens → set to 8000).
    #[serde(default = "default_max_system_prompt_chars")]
    pub max_system_prompt_chars: usize,
    /// Thinking/reasoning level control. Configures how deeply the model reasons
    /// per message. Users can override per-message with `/think:<level>` directives.
    #[nested]
    #[serde(default)]
    pub thinking: crate::scattered_types::ThinkingConfig,

    /// History pruning configuration for token efficiency.
    #[nested]
    #[serde(default)]
    pub history_pruning: crate::scattered_types::HistoryPrunerConfig,

    /// Enable context-aware tool filtering (only surface relevant tools per iteration).
    #[serde(default)]
    pub context_aware_tools: bool,

    /// Post-response quality evaluator configuration.
    #[nested]
    #[serde(default)]
    pub eval: crate::scattered_types::EvalConfig,

    /// Automatic complexity-based classification fallback.
    #[nested]
    #[serde(default)]
    pub auto_classify: Option<crate::scattered_types::AutoClassifyConfig>,

    /// Context compression configuration for automatic conversation compaction.
    #[nested]
    #[serde(default)]
    pub context_compression: crate::scattered_types::ContextCompressionConfig,

    /// Channel reply-intent precheck configuration (model override, timeout).
    #[nested]
    #[serde(default)]
    pub precheck: crate::scattered_types::ChannelPrecheckConfig,

    /// Maximum characters for a single tool result before truncation.
    /// Head (2/3) and tail (1/3) are preserved with a truncation marker in the
    /// middle. Set to `0` to disable truncation. Default: `50000`.
    #[serde(default = "default_max_tool_result_chars")]
    pub max_tool_result_chars: usize,

    /// Number of most recent conversation turns whose full tool-call/result
    /// messages are preserved in channel conversation history. Older turns
    /// keep only the final assistant text. Set to `0` to disable (previous
    /// behavior). Default: `2`.
    #[serde(default = "default_keep_tool_context_turns")]
    pub keep_tool_context_turns: usize,

    /// Self-evolution: completed turns between memory-review nudges.
    ///
    /// At each interval boundary the agent runs a lightweight LLM memory
    /// review of the recent conversation and persists the extracted durable
    /// facts as long-term (`core`) memory entries, mirroring the streaming
    /// agent path in hermes-agent-ultra (`turns_since_memory`). `0` disables
    /// the trigger. Default: `10`.
    #[serde(default = "default_memory_nudge_interval")]
    pub memory_nudge_interval: usize,

    /// Self-evolution: completed turns between skill-creation nudges.
    ///
    /// At each interval boundary an `evolution_nudge` observer event
    /// (`kind = "skill"`) is emitted so UIs can prompt the user/agent to
    /// create or upgrade a skill. `0` disables the trigger. Default: `10`.
    #[serde(default = "default_creation_nudge_interval")]
    pub creation_nudge_interval: usize,

    /// HMAC tool execution receipt configuration.
    #[nested]
    #[serde(default)]
    pub tool_receipts: ToolReceiptsConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            compact_context: true,
            max_tool_iterations: default_agent_max_tool_iterations(),
            max_history_messages: default_agent_max_history_messages(),
            max_context_tokens: default_agent_max_context_tokens(),
            parallel_tools: false,
            tool_dispatcher: default_agent_tool_dispatcher(),
            tool_call_dedup_exempt: Vec::new(),
            tool_filter_groups: Vec::new(),
            max_system_prompt_chars: default_max_system_prompt_chars(),
            thinking: crate::scattered_types::ThinkingConfig::default(),
            history_pruning: crate::scattered_types::HistoryPrunerConfig::default(),
            context_aware_tools: false,
            eval: crate::scattered_types::EvalConfig::default(),
            auto_classify: None,
            context_compression: crate::scattered_types::ContextCompressionConfig::default(),
            precheck: crate::scattered_types::ChannelPrecheckConfig::default(),
            max_tool_result_chars: default_max_tool_result_chars(),
            keep_tool_context_turns: default_keep_tool_context_turns(),
            memory_nudge_interval: default_memory_nudge_interval(),
            creation_nudge_interval: default_creation_nudge_interval(),
            tool_receipts: ToolReceiptsConfig::default(),
        }
    }
}

/// Pipeline tool configuration (`[pipeline]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "pipeline"]
pub struct PipelineConfig {
    /// Enable the `execute_pipeline` meta-tool.
    /// Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of steps allowed in a single pipeline invocation.
    /// Default: `20`.
    #[serde(default = "default_pipeline_max_steps")]
    pub max_steps: usize,
    /// Tools allowed in pipeline steps. Steps referencing tools not on this
    /// list are rejected before execution.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_steps: 20,
            allowed_tools: Vec::new(),
        }
    }
}

// ── Media Pipeline ──────────────────────────────────────────────

/// Automatic media understanding pipeline configuration (`[media_pipeline]`).
///
/// When enabled, inbound channel messages with media attachments are
/// pre-processed before reaching the agent: audio is transcribed, images are
/// annotated, and videos are summarised.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "media-pipeline"]
pub struct MediaPipelineConfig {
    /// Master toggle for the media pipeline (default: false).
    #[serde(default)]
    pub enabled: bool,

    /// Transcribe audio attachments using the configured transcription provider.
    #[serde(default = "default_true")]
    pub transcribe_audio: bool,

    /// Add image descriptions when a vision-capable model is active.
    #[serde(default = "default_true")]
    pub describe_images: bool,

    /// Summarize video attachments (placeholder — requires external API).
    #[serde(default = "default_true")]
    pub summarize_video: bool,
}

impl Default for MediaPipelineConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transcribe_audio: true,
            describe_images: true,
            summarize_video: true,
        }
    }
}

// ── Identity (AIEOS / OpenClaw format) ──────────────────────────

/// Identity format configuration (`[identity]` section).
///
/// Supports `"openclaw"` (default) or `"aieos"` identity documents.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "identity"]
pub struct IdentityConfig {
    /// Identity format: "openclaw" (default) or "aieos"
    #[serde(default = "default_identity_format")]
    pub format: String,
    /// Path to AIEOS JSON file (relative to workspace)
    #[serde(default)]
    pub aieos_path: Option<String>,
    /// Inline AIEOS JSON (alternative to file path)
    #[serde(default)]
    pub aieos_inline: Option<String>,
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            format: default_identity_format(),
            aieos_path: None,
            aieos_inline: None,
        }
    }
}

/// Secure transport configuration for inter-node communication (`[node_transport]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "node-transport"]
pub struct NodeTransportConfig {
    /// Enable the secure transport layer.
    #[serde(default = "default_node_transport_enabled")]
    pub enabled: bool,
    /// Shared secret for HMAC authentication between nodes.
    #[serde(default)]
    pub shared_secret: String,
    /// Maximum age of signed requests in seconds (replay protection).
    #[serde(default = "default_max_request_age")]
    pub max_request_age_secs: i64,
    /// Require HTTPS for all node communication.
    #[serde(default = "default_require_https")]
    pub require_https: bool,
    /// Allow specific node IPs/CIDRs.
    #[serde(default)]
    pub allowed_peers: Vec<String>,
    /// Path to TLS certificate file.
    #[serde(default)]
    pub tls_cert_path: Option<String>,
    /// Path to TLS private key file.
    #[serde(default)]
    pub tls_key_path: Option<String>,
    /// Require client certificates (mutual TLS).
    #[serde(default)]
    pub mutual_tls: bool,
    /// Maximum number of connections per peer.
    #[serde(default = "default_connection_pool_size")]
    pub connection_pool_size: usize,
}

impl Default for NodeTransportConfig {
    fn default() -> Self {
        Self {
            enabled: default_node_transport_enabled(),
            shared_secret: String::new(),
            max_request_age_secs: default_max_request_age(),
            require_https: default_require_https(),
            allowed_peers: Vec::new(),
            tls_cert_path: None,
            tls_key_path: None,
            mutual_tls: false,
            connection_pool_size: default_connection_pool_size(),
        }
    }
}

// ── Composio (managed tool surface) ─────────────────────────────

/// Composio managed OAuth tools integration (`[composio]` section).
///
/// Provides access to 1000+ OAuth-connected tools via the Composio platform.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "composio"]
pub struct ComposioConfig {
    /// Enable Composio integration for 1000+ OAuth tools
    #[serde(default, alias = "enable")]
    pub enabled: bool,
    /// Composio API key (stored encrypted when secrets.encrypt = true)
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Default entity ID for multi-user setups
    #[serde(default = "default_entity_id")]
    pub entity_id: String,
}

impl Default for ComposioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: None,
            entity_id: default_entity_id(),
        }
    }
}

// ── Microsoft 365 (Graph API integration) ───────────────────────

/// Microsoft 365 integration via Microsoft Graph API (`[microsoft365]` section).
///
/// Provides access to Outlook mail, Teams messages, Calendar events,
/// OneDrive files, and SharePoint search.
#[derive(Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "ms365"]
pub struct Microsoft365Config {
    /// Enable Microsoft 365 integration
    #[serde(default, alias = "enable")]
    pub enabled: bool,
    /// Azure AD tenant ID
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Azure AD application (client) ID
    #[serde(default)]
    pub client_id: Option<String>,
    /// Azure AD client secret (stored encrypted when secrets.encrypt = true)
    #[serde(default)]
    #[secret]
    pub client_secret: Option<String>,
    /// Authentication flow: "client_credentials" or "device_code"
    #[serde(default = "default_ms365_auth_flow")]
    pub auth_flow: String,
    /// OAuth scopes to request
    #[serde(default = "default_ms365_scopes")]
    pub scopes: Vec<String>,
    /// Encrypt the token cache file on disk
    #[serde(default = "default_true")]
    pub token_cache_encrypted: bool,
    /// User principal name or "me" (for delegated flows)
    #[serde(default)]
    pub user_id: Option<String>,
}

impl std::fmt::Debug for Microsoft365Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Microsoft365Config")
            .field("enabled", &self.enabled)
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .field("auth_flow", &self.auth_flow)
            .field("scopes", &self.scopes)
            .field("token_cache_encrypted", &self.token_cache_encrypted)
            .field("user_id", &self.user_id)
            .finish()
    }
}

impl Default for Microsoft365Config {
    fn default() -> Self {
        Self {
            enabled: false,
            tenant_id: None,
            client_id: None,
            client_secret: None,
            auth_flow: default_ms365_auth_flow(),
            scopes: default_ms365_scopes(),
            token_cache_encrypted: true,
            user_id: None,
        }
    }
}

// ── Secrets (encrypted credential store) ────────────────────────

/// Secrets encryption configuration (`[secrets]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "secrets"]
pub struct SecretsConfig {
    /// Enable encryption for API keys and tokens in config.toml
    #[serde(default = "default_true")]
    pub encrypt: bool,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self { encrypt: true }
    }
}

// ── Browser (friendly-service browsing only) ───────────────────

/// Computer-use sidecar configuration (`[browser.computer_use]` section).
///
/// Delegates OS-level mouse, keyboard, and screenshot actions to a local sidecar.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "browser.computer-use"]
pub struct BrowserComputerUseConfig {
    /// Sidecar endpoint for computer-use actions (OS-level mouse/keyboard/screenshot)
    #[serde(default = "default_browser_computer_use_endpoint")]
    pub endpoint: String,
    /// Optional bearer token for computer-use sidecar
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Per-action request timeout in milliseconds
    #[serde(default = "default_browser_computer_use_timeout_ms")]
    pub timeout_ms: u64,
    /// Allow remote/public endpoint for computer-use sidecar (default: false)
    #[serde(default)]
    pub allow_remote_endpoint: bool,
    /// Optional window title/process allowlist forwarded to sidecar policy
    #[serde(default)]
    pub window_allowlist: Vec<String>,
    /// Optional X-axis boundary for coordinate-based actions
    #[serde(default)]
    pub max_coordinate_x: Option<i64>,
    /// Optional Y-axis boundary for coordinate-based actions
    #[serde(default)]
    pub max_coordinate_y: Option<i64>,
}

impl Default for BrowserComputerUseConfig {
    fn default() -> Self {
        Self {
            endpoint: default_browser_computer_use_endpoint(),
            api_key: None,
            timeout_ms: default_browser_computer_use_timeout_ms(),
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

/// Browser automation configuration (`[browser]` section).
///
/// Controls the `browser_open` tool and browser automation backends.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "browser"]
#[integration(
    category = "ToolsAutomation",
    display_name = "Browser",
    description = "Chrome/Chromium control",
    status_field = "enabled"
)]
pub struct BrowserConfig {
    /// Enable `browser_open` tool (opens URLs in the system browser without scraping)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Allowed domains for `browser_open` (exact or subdomain match)
    #[serde(default = "default_browser_allowed_domains")]
    pub allowed_domains: Vec<String>,
    /// Browser session name (for agent-browser automation)
    #[serde(default)]
    pub session_name: Option<String>,
    /// Browser automation backend: "agent_browser" | "rust_native" | "computer_use" | "auto"
    #[serde(default = "default_browser_backend")]
    pub backend: String,
    /// Headless mode for rust-native backend
    #[serde(default = "default_true")]
    pub native_headless: bool,
    /// WebDriver endpoint URL for rust-native backend (e.g. `http://127.0.0.1:9515`)
    #[serde(default = "default_browser_webdriver_url")]
    pub native_webdriver_url: String,
    /// Optional Chrome/Chromium executable path for rust-native backend
    #[serde(default)]
    pub native_chrome_path: Option<String>,
    /// Computer-use sidecar configuration
    #[serde(default)]
    #[nested]
    pub computer_use: BrowserComputerUseConfig,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_domains: vec!["*".into()],
            session_name: None,
            backend: default_browser_backend(),
            native_headless: default_true(),
            native_webdriver_url: default_browser_webdriver_url(),
            native_chrome_path: None,
            computer_use: BrowserComputerUseConfig::default(),
        }
    }
}

// ── HTTP request tool ───────────────────────────────────────────

/// HTTP request tool configuration (`[http_request]` section).
///
/// Domain filtering: `allowed_domains` controls which hosts are reachable (use `["*"]`
/// for all public hosts, which is the default). If `allowed_domains` is empty, all
/// requests are rejected.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "http-request"]
pub struct HttpRequestConfig {
    /// Enable `http_request` tool for API interactions
    #[serde(default)]
    pub enabled: bool,
    /// Allowed domains for HTTP requests (exact or subdomain match)
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Maximum response size in bytes (default: 1MB, 0 = unlimited)
    #[serde(default = "default_http_max_response_size")]
    pub max_response_size: usize,
    /// Request timeout in seconds (default: 30)
    #[serde(default = "default_http_timeout_secs")]
    pub timeout_secs: u64,
    /// Allow requests to private/LAN hosts (RFC 1918, loopback, link-local, .local).
    /// Default: false (deny private hosts for SSRF protection).
    #[serde(default)]
    pub allow_private_hosts: bool,
}

impl Default for HttpRequestConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_domains: vec!["*".into()],
            max_response_size: default_http_max_response_size(),
            timeout_secs: default_http_timeout_secs(),
            allow_private_hosts: false,
        }
    }
}

// ── Web fetch ────────────────────────────────────────────────────

/// Web fetch tool configuration (`[web_fetch]` section).
///
/// Fetches web pages and converts HTML to plain text for LLM consumption.
/// Domain filtering: `allowed_domains` controls which hosts are reachable (use `["*"]`
/// for all public hosts). `blocked_domains` takes priority over `allowed_domains`.
/// If `allowed_domains` is empty, all requests are rejected (deny-by-default).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "web-fetch"]
pub struct WebFetchConfig {
    /// Enable `web_fetch` tool for fetching web page content
    #[serde(default)]
    pub enabled: bool,
    /// Allowed domains for web fetch (exact or subdomain match; `["*"]` = all public hosts)
    #[serde(default = "default_web_fetch_allowed_domains")]
    pub allowed_domains: Vec<String>,
    /// Blocked domains (exact or subdomain match; always takes priority over allowed_domains)
    #[serde(default)]
    pub blocked_domains: Vec<String>,
    /// Private/internal hosts allowed to bypass SSRF protection (e.g. `["192.168.1.10", "internal.local"]`)
    #[serde(default)]
    pub allowed_private_hosts: Vec<String>,
    /// Maximum response size in bytes (default: 500KB, plain text is much smaller than raw HTML)
    #[serde(default = "default_web_fetch_max_response_size")]
    pub max_response_size: usize,
    /// Request timeout in seconds (default: 30)
    #[serde(default = "default_web_fetch_timeout_secs")]
    pub timeout_secs: u64,
    /// Firecrawl fallback configuration (`[web_fetch.firecrawl]`)
    #[serde(default)]
    #[nested]
    pub firecrawl: FirecrawlConfig,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allowed_domains: vec!["*".into()],
            blocked_domains: vec![],
            allowed_private_hosts: vec![],
            max_response_size: default_web_fetch_max_response_size(),
            timeout_secs: default_web_fetch_timeout_secs(),
            firecrawl: FirecrawlConfig::default(),
        }
    }
}

// ── Link enricher ─────────────────────────────────────────────────

/// Automatic link understanding for inbound channel messages (`[link_enricher]`).
///
/// When enabled, URLs in incoming messages are automatically fetched and
/// summarised. The summary is prepended to the message before the agent
/// processes it, giving the LLM context about linked pages without an
/// explicit tool call.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "link-enricher"]
pub struct LinkEnricherConfig {
    /// Enable the link enricher pipeline stage (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Maximum number of links to fetch per message (default: 3)
    #[serde(default = "default_link_enricher_max_links")]
    pub max_links: usize,
    /// Per-link fetch timeout in seconds (default: 10)
    #[serde(default = "default_link_enricher_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for LinkEnricherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_links: default_link_enricher_max_links(),
            timeout_secs: default_link_enricher_timeout_secs(),
        }
    }
}

// ── Text browser ─────────────────────────────────────────────────

/// Text browser tool configuration (`[text_browser]` section).
///
/// Uses text-based browsers (lynx, links, w3m) to render web pages as plain
/// text. Designed for headless/SSH environments without graphical browsers.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "text-browser"]
pub struct TextBrowserConfig {
    /// Enable `text_browser` tool
    #[serde(default)]
    pub enabled: bool,
    /// Preferred text browser ("lynx", "links", or "w3m"). If unset, auto-detects.
    #[serde(default)]
    pub preferred_browser: Option<String>,
    /// Request timeout in seconds (default: 30)
    #[serde(default = "default_text_browser_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for TextBrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preferred_browser: None,
            timeout_secs: default_text_browser_timeout_secs(),
        }
    }
}

// ── Shell tool ───────────────────────────────────────────────────

/// Shell tool configuration (`[shell_tool]` section).
///
/// Controls the behaviour of the `shell` execution tool. The main
/// tunable is `timeout_secs` — the maximum wall-clock time a single
/// shell command may run before it is killed.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "shell-tool"]
pub struct ShellToolConfig {
    /// Maximum shell command execution time in seconds (default: 60).
    #[serde(default = "default_shell_tool_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for ShellToolConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_shell_tool_timeout_secs(),
        }
    }
}

// ── Escalation routing ───────────────────────────────────────────

/// Escalation routing configuration (`[escalation]` section).
///
/// Controls which channels receive alert notifications when
/// `escalate_to_human` is called with high or critical urgency.
/// Channels are identified by name (e.g. `"telegram"`, `"slack"`).
/// Alerts are sent best-effort and do not block the escalation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "escalation"]
pub struct EscalationConfig {
    /// Channel names to alert on high/critical escalations (default: empty).
    ///
    /// Each name must match a configured channel. Unrecognised names are
    /// logged at WARN level and skipped.
    #[serde(default)]
    pub alert_channels: Vec<String>,
}

// ── Web search ───────────────────────────────────────────────────

/// Web search tool configuration (`[web_search]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "web-search"]
pub struct WebSearchConfig {
    /// Enable `web_search_tool` for web searches
    #[serde(default)]
    pub enabled: bool,
    /// Search provider: "duckduckgo" (free), "brave" (requires API key), "tavily" (requires API key), or "searxng" (self-hosted)
    #[serde(default = "default_web_search_provider")]
    pub provider: String,
    /// Brave Search API key (required if provider is "brave")
    #[serde(default)]
    #[secret]
    pub brave_api_key: Option<String>,
    /// Tavily Search API key (required if provider is "tavily")
    #[serde(default)]
    #[secret]
    pub tavily_api_key: Option<String>,
    /// SearXNG instance URL (required if provider is `"searxng"`), e.g. `"https://searx.example.com"`.
    #[serde(default)]
    pub searxng_instance_url: Option<String>,
    /// Maximum results per search (1-10)
    #[serde(default = "default_web_search_max_results")]
    pub max_results: usize,
    /// Request timeout in seconds
    #[serde(default = "default_web_search_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_web_search_provider(),
            brave_api_key: None,
            tavily_api_key: None,
            searxng_instance_url: None,
            max_results: default_web_search_max_results(),
            timeout_secs: default_web_search_timeout_secs(),
        }
    }
}

// ── Project Intelligence ────────────────────────────────────────

/// Project delivery intelligence configuration (`[project_intel]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "project-intel"]
pub struct ProjectIntelConfig {
    /// Enable the project_intel tool. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Default report language (en, de, fr, it). Default: "en".
    #[serde(default = "default_project_intel_language")]
    pub default_language: String,
    /// Output directory for generated reports.
    #[serde(default = "default_project_intel_report_dir")]
    pub report_output_dir: String,
    /// Optional custom templates directory.
    #[serde(default)]
    pub templates_dir: Option<String>,
    /// Risk detection sensitivity: low, medium, high. Default: "medium".
    #[serde(default = "default_project_intel_risk_sensitivity")]
    pub risk_sensitivity: String,
    /// Include git log data in reports. Default: true.
    #[serde(default = "default_true")]
    pub include_git_data: bool,
    /// Include Jira data in reports. Default: false.
    #[serde(default)]
    pub include_jira_data: bool,
    /// Jira instance base URL (required if include_jira_data is true).
    #[serde(default)]
    pub jira_base_url: Option<String>,
}

impl Default for ProjectIntelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_language: default_project_intel_language(),
            report_output_dir: default_project_intel_report_dir(),
            templates_dir: None,
            risk_sensitivity: default_project_intel_risk_sensitivity(),
            include_git_data: true,
            include_jira_data: false,
            jira_base_url: None,
        }
    }
}

// ── Data Retention ──────────────────────────────────────────────

/// Data retention and purge configuration (`[data_retention]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "data-retention"]
pub struct DataRetentionConfig {
    /// Enable the `data_management` tool.
    #[serde(default)]
    pub enabled: bool,
    /// Days of data to retain before purge eligibility.
    #[serde(default = "default_retention_days")]
    pub retention_days: u64,
    /// Preview what would be deleted without actually removing anything.
    #[serde(default)]
    pub dry_run: bool,
    /// Limit retention enforcement to specific data categories (empty = all).
    #[serde(default)]
    pub categories: Vec<String>,
}

impl Default for DataRetentionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: default_retention_days(),
            dry_run: false,
            categories: Vec::new(),
        }
    }
}

/// Google Workspace CLI (`gws`) tool configuration (`[google_workspace]` section).
///
/// ## Defaults
/// - `enabled`: `false` (tool is not registered unless explicitly opted-in).
/// - `allowed_services`: empty vector, which grants access to the full default
///   service set: `drive`, `sheets`, `gmail`, `calendar`, `docs`, `slides`,
///   `tasks`, `people`, `chat`, `classroom`, `forms`, `keep`, `meet`, `events`.
/// - `allowed_operations`: empty vector, which preserves the legacy behavior of
///   allowing any resource/method under the allowed service set.
/// - `credentials_path`: `None` (uses default `gws` credential discovery).
/// - `default_account`: `None` (uses the `gws` active account).
/// - `rate_limit_per_minute`: `60`.
/// - `timeout_secs`: `30`.
/// - `audit_log`: `false`.
///
/// ## Compatibility
/// Configs that omit the `[google_workspace]` section entirely are treated as
/// `GoogleWorkspaceConfig::default()` (disabled, all defaults allowed). Adding
/// the section is purely opt-in and does not affect other config sections.
///
/// ## Rollback / Migration
/// To revert, remove the `[google_workspace]` section from the config file (or
/// set `enabled = false`). No data migration is required; the tool simply stops
/// being registered.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "google-workspace"]
#[integration(
    category = "ToolsAutomation",
    display_name = "Google Workspace",
    description = "Drive, Gmail, Calendar, Sheets, Docs via gws CLI",
    status_field = "enabled"
)]
pub struct GoogleWorkspaceConfig {
    /// Enable the `google_workspace` tool. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Restrict which Google Workspace services the agent can access.
    ///
    /// When empty (the default), the full default service set is allowed (see
    /// struct-level docs). When non-empty, only the listed service IDs are
    /// permitted. Each entry must be non-empty, lowercase alphanumeric with
    /// optional underscores/hyphens, and unique.
    #[serde(default)]
    pub allowed_services: Vec<String>,
    /// Restrict which resource/method combinations the agent can access.
    ///
    /// When empty (the default), all methods under `allowed_services` remain
    /// available for backward compatibility. When non-empty, the runtime denies
    /// any `(service, resource, sub_resource, method)` combination that is not
    /// explicitly listed. `sub_resource` is optional per entry: an entry without
    /// it matches only 3-segment `gws` calls; an entry with it matches only calls
    /// that supply that exact sub_resource value.
    ///
    /// Each entry's `service` must appear in `allowed_services` when that list is
    /// non-empty; config validation rejects entries that would never match at
    /// runtime.
    #[serde(default)]
    pub allowed_operations: Vec<GoogleWorkspaceAllowedOperation>,
    /// Path to service account JSON or OAuth client credentials file.
    ///
    /// When `None`, the tool relies on the default `gws` credential discovery
    /// (`gws auth login`). Set this to point at a service-account key or an
    /// OAuth client-secrets JSON for headless / CI environments.
    #[serde(default)]
    pub credentials_path: Option<String>,
    /// Default Google account email to pass to `gws --account`.
    ///
    /// When `None`, the currently active `gws` account is used.
    #[serde(default)]
    pub default_account: Option<String>,
    /// Maximum number of `gws` API calls allowed per minute. Default: `60`.
    #[serde(default = "default_gws_rate_limit")]
    pub rate_limit_per_minute: u32,
    /// Command execution timeout in seconds. Default: `30`.
    #[serde(default = "default_gws_timeout_secs")]
    pub timeout_secs: u64,
    /// Enable audit logging of every `gws` invocation (service, resource,
    /// method, timestamp). Default: `false`.
    #[serde(default)]
    pub audit_log: bool,
}

// ── Knowledge ───────────────────────────────────────────────────

/// Knowledge graph configuration for capturing and reusing expertise.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "knowledge"]
pub struct KnowledgeConfig {
    /// Enable the knowledge graph tool. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Path to the knowledge graph SQLite database.
    #[serde(default = "default_knowledge_db_path")]
    pub db_path: String,
    /// Maximum number of knowledge nodes. Default: 100000.
    #[serde(default = "default_knowledge_max_nodes")]
    pub max_nodes: usize,
    /// Automatically capture knowledge from conversations. Default: false.
    #[serde(default)]
    pub auto_capture: bool,
    /// Proactively suggest relevant knowledge on queries. Default: true.
    #[serde(default = "default_true")]
    pub suggest_on_query: bool,
    /// Allow searching across workspaces (disabled by default for client data isolation).
    #[serde(default)]
    pub cross_workspace_search: bool,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: default_knowledge_db_path(),
            max_nodes: default_knowledge_max_nodes(),
            auto_capture: false,
            suggest_on_query: true,
            cross_workspace_search: false,
        }
    }
}

/// Plugin system configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "plugins"]
pub struct PluginsConfig {
    /// Enable the plugin system (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Directory where plugins are stored
    #[serde(default = "default_plugins_dir")]
    pub plugins_dir: String,
    /// Auto-discover and load plugins on startup
    #[serde(default)]
    pub auto_discover: bool,
    /// Maximum number of plugins that can be loaded
    #[serde(default = "default_max_plugins")]
    pub max_plugins: usize,
    /// Plugin signature verification security settings
    #[serde(default)]
    #[nested]
    pub security: PluginSecurityConfig,
}

/// Plugin signature verification configuration (`[plugins.security]`).
///
/// Controls Ed25519 signature verification for plugin manifests.
/// In `strict` mode, only plugins signed by a trusted publisher key are loaded.
/// In `permissive` mode, unsigned or untrusted plugins produce warnings but are
/// still loaded. In `disabled` mode (the default), no signature checking occurs.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "plugins.security"]
pub struct PluginSecurityConfig {
    /// Signature enforcement mode: "disabled", "permissive", or "strict".
    #[serde(default = "default_signature_mode")]
    pub signature_mode: String,
    /// Hex-encoded Ed25519 public keys of trusted plugin publishers.
    #[serde(default)]
    pub trusted_publisher_keys: Vec<String>,
}

impl Default for PluginSecurityConfig {
    fn default() -> Self {
        Self {
            signature_mode: default_signature_mode(),
            trusted_publisher_keys: Vec::new(),
        }
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            plugins_dir: default_plugins_dir(),
            auto_discover: false,
            max_plugins: default_max_plugins(),
            security: PluginSecurityConfig::default(),
        }
    }
}

/// Stability AI image generation settings (`[linkedin.image.stability]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.image.stability"]
pub struct ImageProviderStabilityConfig {
    /// Environment variable name holding the API key.
    #[serde(default = "default_stability_api_key_env")]
    pub api_key_env: String,
    /// Stability model identifier.
    #[serde(default = "default_stability_model")]
    pub model: String,
}

impl Default for ImageProviderStabilityConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_stability_api_key_env(),
            model: default_stability_model(),
        }
    }
}

/// Flux (fal.ai) image generation settings (`[linkedin.image.flux]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.image.flux"]
pub struct ImageProviderFluxConfig {
    /// Environment variable name holding the fal.ai API key.
    #[serde(default = "default_flux_api_key_env")]
    pub api_key_env: String,
    /// Flux model identifier.
    #[serde(default = "default_flux_model")]
    pub model: String,
}

impl Default for ImageProviderFluxConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_flux_api_key_env(),
            model: default_flux_model(),
        }
    }
}

// ── Standalone Image Generation ─────────────────────────────────

/// Standalone image generation tool configuration (`[image_gen]`).
///
/// When enabled, registers an `image_gen` tool that generates images via
/// fal.ai's synchronous API (Flux / Nano Banana models) and saves them
/// to the workspace `images/` directory.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "image-gen"]
pub struct ImageGenConfig {
    /// Enable the standalone image generation tool. Default: false.
    #[serde(default)]
    pub enabled: bool,

    /// Default fal.ai model identifier.
    #[serde(default = "default_image_gen_model")]
    pub default_model: String,

    /// Environment variable name holding the fal.ai API key.
    #[serde(default = "default_image_gen_api_key_env")]
    pub api_key_env: String,
}

impl Default for ImageGenConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_model: default_image_gen_model(),
            api_key_env: default_image_gen_api_key_env(),
        }
    }
}

// ── Claude Code ─────────────────────────────────────────────────

/// Claude Code CLI tool configuration (`[claude_code]` section).
///
/// Delegates coding tasks to the `claude -p` CLI. Authentication uses the
/// binary's own OAuth session (Max subscription) by default — no API key
/// needed unless `env_passthrough` includes `ANTHROPIC_API_KEY`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "claude-code"]
pub struct ClaudeCodeConfig {
    /// Enable the `claude_code` tool
    #[serde(default)]
    pub enabled: bool,
    /// Maximum execution time in seconds (coding tasks can be long)
    #[serde(default = "default_claude_code_timeout_secs")]
    pub timeout_secs: u64,
    /// Claude Code tools the subprocess is allowed to use
    #[serde(default = "default_claude_code_allowed_tools")]
    pub allowed_tools: Vec<String>,
    /// Optional system prompt appended to Claude Code invocations
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Maximum output size in bytes (2MB default)
    #[serde(default = "default_claude_code_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Extra env vars passed to the claude subprocess (e.g. ANTHROPIC_API_KEY for API-key billing)
    #[serde(default)]
    pub env_passthrough: Vec<String>,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: default_claude_code_timeout_secs(),
            allowed_tools: default_claude_code_allowed_tools(),
            system_prompt: None,
            max_output_bytes: default_claude_code_max_output_bytes(),
            env_passthrough: Vec::new(),
        }
    }
}

// ── Claude Code Runner ──────────────────────────────────────────

/// Claude Code task runner configuration (`[claude_code_runner]` section).
///
/// Spawns Claude Code in a tmux session with HTTP hooks that POST tool
/// execution events back to Operant's gateway, updating a Slack message
/// in-place with progress plus an SSH handoff link.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "claude-code-runner"]
pub struct ClaudeCodeRunnerConfig {
    /// Enable the `claude_code_runner` tool
    #[serde(default)]
    pub enabled: bool,
    /// SSH host for session handoff links (e.g. "myhost.example.com")
    #[serde(default)]
    pub ssh_host: Option<String>,
    /// Prefix for tmux session names (default: "zc-claude-")
    #[serde(default = "default_claude_code_runner_tmux_prefix")]
    pub tmux_prefix: String,
    /// Session time-to-live in seconds before auto-cleanup (default: 3600)
    #[serde(default = "default_claude_code_runner_session_ttl")]
    pub session_ttl: u64,
}

impl Default for ClaudeCodeRunnerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ssh_host: None,
            tmux_prefix: default_claude_code_runner_tmux_prefix(),
            session_ttl: default_claude_code_runner_session_ttl(),
        }
    }
}

// ── Codex CLI ───────────────────────────────────────────────────

/// Codex CLI tool configuration (`[codex_cli]` section).
///
/// Delegates coding tasks to the `codex -q` CLI. Authentication uses the
/// binary's own session by default — no API key needed unless
/// `env_passthrough` includes `OPENAI_API_KEY`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "codex-cli"]
pub struct CodexCliConfig {
    /// Enable the `codex_cli` tool
    #[serde(default)]
    pub enabled: bool,
    /// Maximum execution time in seconds (coding tasks can be long)
    #[serde(default = "default_codex_cli_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum output size in bytes (2MB default)
    #[serde(default = "default_codex_cli_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Extra env vars passed to the codex subprocess (e.g. OPENAI_API_KEY)
    #[serde(default)]
    pub env_passthrough: Vec<String>,
}

impl Default for CodexCliConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: default_codex_cli_timeout_secs(),
            max_output_bytes: default_codex_cli_max_output_bytes(),
            env_passthrough: Vec::new(),
        }
    }
}

// ── Gemini CLI ──────────────────────────────────────────────────

/// Gemini CLI tool configuration (`[gemini_cli]` section).
///
/// Delegates coding tasks to the `gemini -p` CLI. Authentication uses the
/// binary's own session by default — no API key needed unless
/// `env_passthrough` includes `GOOGLE_API_KEY`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "gemini-cli"]
pub struct GeminiCliConfig {
    /// Enable the `gemini_cli` tool
    #[serde(default)]
    pub enabled: bool,
    /// Maximum execution time in seconds (coding tasks can be long)
    #[serde(default = "default_gemini_cli_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum output size in bytes (2MB default)
    #[serde(default = "default_gemini_cli_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Extra env vars passed to the gemini subprocess (e.g. GOOGLE_API_KEY)
    #[serde(default)]
    pub env_passthrough: Vec<String>,
}

impl Default for GeminiCliConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: default_gemini_cli_timeout_secs(),
            max_output_bytes: default_gemini_cli_max_output_bytes(),
            env_passthrough: Vec::new(),
        }
    }
}

// ── OpenCode CLI ───────────────────────────────────────────────

/// OpenCode CLI tool configuration (`[opencode_cli]` section).
///
/// Delegates coding tasks to the `opencode run` CLI. Authentication uses the
/// binary's own session by default — no API key needed unless
/// `env_passthrough` includes provider-specific keys.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "opencode-cli"]
pub struct OpenCodeCliConfig {
    /// Enable the `opencode_cli` tool
    #[serde(default)]
    pub enabled: bool,
    /// Maximum execution time in seconds (coding tasks can be long)
    #[serde(default = "default_opencode_cli_timeout_secs")]
    pub timeout_secs: u64,
    /// Maximum output size in bytes (2MB default)
    #[serde(default = "default_opencode_cli_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Extra env vars passed to the opencode subprocess
    #[serde(default)]
    pub env_passthrough: Vec<String>,
}

impl Default for OpenCodeCliConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_secs: default_opencode_cli_timeout_secs(),
            max_output_bytes: default_opencode_cli_max_output_bytes(),
            env_passthrough: Vec::new(),
        }
    }
}

pub(crate) fn normalize_service_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = normalize_comma_values(values)
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

// ── Proxy-aware WebSocket connect ────────────────────────────────
//
// `tokio_tungstenite::connect_async` does not honour proxy settings.
// The helpers below resolve the effective proxy URL for a given service
// key and, when a proxy is active, establish a tunnelled TCP connection
// (HTTP CONNECT for http/https proxies, SOCKS5 for socks5/socks5h)
// before handing the stream to `tokio_tungstenite` for the WebSocket
// handshake.

/// Combined async IO trait for boxed WebSocket transport streams.
pub(crate) trait AsyncReadWrite:
    tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send
{
}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

/// A boxed async IO stream used when a WebSocket connection is tunnelled
/// through a proxy.  The concrete type varies depending on the proxy
/// kind (HTTP CONNECT vs SOCKS5) and the target scheme (ws vs wss).
///
/// We wrap in a newtype so we can implement `AsyncRead` and `AsyncWrite`
/// via delegation, since Rust trait objects cannot combine multiple
/// non-auto traits.
pub struct BoxedIo(pub(crate) Box<dyn AsyncReadWrite>);

impl tokio::io::AsyncRead for BoxedIo {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for BoxedIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut *self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.0).poll_shutdown(cx)
    }
}

impl Unpin for BoxedIo {}

/// Convenience alias for the WebSocket stream returned by the proxy-aware
/// connect helpers.
pub type ProxiedWsStream = tokio_tungstenite::WebSocketStream<BoxedIo>;

/// Resolve the effective proxy URL for a WebSocket connection to the
/// given `ws_url`, taking into account the per-channel `proxy_url`
/// override, the runtime proxy config, scope and no_proxy list.
pub(crate) fn resolve_ws_proxy_url(
    service_key: &str,
    ws_url: &str,
    channel_proxy_url: Option<&str>,
) -> Option<String> {
    // 1. Explicit per-channel proxy always wins.
    if let Some(url) = normalize_proxy_url_option(channel_proxy_url) {
        return Some(url);
    }

    // 2. Consult the runtime proxy config.
    let cfg = runtime_proxy_config();
    if !cfg.should_apply_to_service(service_key) {
        return None;
    }

    // Check the no_proxy list against the WebSocket target host.
    if let Ok(parsed) = reqwest::Url::parse(ws_url)
        && let Some(host) = parsed.host_str()
    {
        let no_proxy_entries = cfg.normalized_no_proxy();
        if !no_proxy_entries.is_empty() {
            let host_lower = host.to_ascii_lowercase();
            let matches_no_proxy = no_proxy_entries.iter().any(|entry| {
                let entry = entry.trim().to_ascii_lowercase();
                if entry == "*" {
                    return true;
                }
                if host_lower == entry {
                    return true;
                }
                // Support ".example.com" matching "foo.example.com"
                if let Some(suffix) = entry.strip_prefix('.') {
                    return host_lower.ends_with(suffix) || host_lower == suffix;
                }
                // Support "example.com" also matching "foo.example.com"
                host_lower.ends_with(&format!(".{entry}"))
            });
            if matches_no_proxy {
                return None;
            }
        }
    }

    // For wss:// prefer https_proxy, for ws:// prefer http_proxy, fall
    // back to all_proxy in both cases.
    let is_secure = ws_url.starts_with("wss://") || ws_url.starts_with("wss:");
    let preferred = if is_secure {
        normalize_proxy_url_option(cfg.https_proxy.as_deref())
    } else {
        normalize_proxy_url_option(cfg.http_proxy.as_deref())
    };
    preferred.or_else(|| normalize_proxy_url_option(cfg.all_proxy.as_deref()))
}

/// Find the `\r\n\r\n` boundary marking the end of HTTP headers.
pub(crate) fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// PostgreSQL memory backend configuration (`[memory.postgres]` section).
///
/// Used when `[memory].backend = "postgres"`. Connection parameters
/// (`db_url`, `schema`, `table`, `connect_timeout_secs`) live under
/// `[storage.provider.config]`; this struct only holds vector-search settings.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "memory.postgres"]
pub struct PostgresMemoryConfig {
    /// Enable pgvector extension for hybrid vector+keyword recall.
    #[serde(default)]
    pub vector_enabled: bool,

    /// Vector dimensions for pgvector embeddings (default: 1536).
    #[serde(default = "default_pgvector_dimensions")]
    pub vector_dimensions: usize,
}

impl Default for PostgresMemoryConfig {
    fn default() -> Self {
        Self {
            vector_enabled: false,
            vector_dimensions: default_pgvector_dimensions(),
        }
    }
}

/// Search strategy for memory recall.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Pure keyword search (FTS5 BM25)
    Bm25,
    /// Pure vector/semantic search
    Embedding,
    /// Weighted combination of keyword + vector (default)
    #[default]
    Hybrid,
}

/// Memory backend configuration (`[memory]` section).
///
/// Controls conversation memory storage, embeddings, hybrid search, response
/// caching, and memory snapshot/hydration. Backend-specific sub-tables
/// (`[memory.qdrant]`, `[memory.postgres]`) live alongside.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "memory"]
#[allow(clippy::struct_excessive_bools)]
pub struct MemoryConfig {
    /// Where conversations, notes, and memories live. `agentmemory` (default) = semantic memory backend (injects the native `agentmemory` MCP server into `[mcp.servers]` as a deferred server; the runtime memory layer uses the `agentmemory` REST backend via `AGENTMEMORY_URL`/`AGENTMEMORY_SECRET`); `sqlite` = embedded DB with optional vector + keyword hybrid search (fast, self-contained); `markdown` = plain-text files you can read and edit by hand (portable but no vector search); `lucid` = sync with the external `lucid-memory` CLI; `qdrant` = dedicated vector DB via `[memory.qdrant]` or `QDRANT_URL` env var; `none` = disable memory entirely.
    pub backend: String,
    /// Auto-save what *you* tell Operant into memory as conversation history — the agent's own replies are not saved. Turn off if you want memory to only hold things you explicitly record via the memory tool.
    pub auto_save: bool,
    /// Run the periodic hygiene pass that archives stale daily/session files and enforces retention windows. Leave on unless you want to manage cleanup yourself.
    #[serde(default = "default_hygiene_enabled")]
    pub hygiene_enabled: bool,
    /// Move daily/session files to the archive directory after this many days. Keeps the hot working set small without deleting history.
    #[serde(default = "default_archive_after_days")]
    pub archive_after_days: u32,
    /// Delete archived files permanently after this many days. Set high if you need long-term history; set low for privacy / disk-space reasons.
    #[serde(default = "default_purge_after_days")]
    pub purge_after_days: u32,
    /// For the sqlite backend only — drop conversation rows older than this many days to keep the DB lean. Doesn't touch core memories or notes.
    #[serde(default = "default_conversation_retention_days")]
    pub conversation_retention_days: u32,
    /// Source of embedding vectors for semantic search. `none` = keyword-only retrieval (no API calls, no vector cost); `openai` = OpenAI's embedding API; `custom:URL` = any OpenAI-compatible embedding endpoint (LiteLLM, local gateway, etc.).
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    /// Embedding model identifier — must match a model your chosen embedding
    /// provider serves (e.g. `text-embedding-3-small` for OpenAI). Empty
    /// (default) = no external embedding model is assumed; the default
    /// `embedding_provider = "none"` then yields keyword-only retrieval with
    /// zero external dependencies. Changing this invalidates existing
    /// embeddings; you'll need to re-index.
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Vector width produced by the embedding model — must match the model's
    /// native dimension or vectors won't store correctly. `0` (default)
    /// means "unconfigured" and pairs with the empty default model. Look up
    /// the number on the provider's model page.
    #[serde(default = "default_embedding_dims")]
    pub embedding_dimensions: usize,
    /// How heavily vector (semantic) similarity counts when `search_mode = hybrid`. Raise toward 1.0 to favor meaning-based matches; lower it to lean on keyword overlap instead.
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// How heavily BM25 (keyword) overlap counts when `search_mode = hybrid`. Raise toward 1.0 for exact-term matching; lower it when paraphrases should still score well.
    #[serde(default = "default_keyword_weight")]
    pub keyword_weight: f64,
    /// How memories are retrieved: `bm25` = keyword-only (no embeddings, cheapest); `embedding` = vector similarity only (needs an embedding provider); `hybrid` = blended keyword + vector score using the weights above (most robust).
    #[serde(default)]
    pub search_mode: SearchMode,
    /// Minimum hybrid score (0.0–1.0) for a memory to be included in context.
    /// Memories scoring below this threshold are dropped to prevent irrelevant
    /// context from bleeding into conversations. Default: 0.4
    #[serde(default = "default_min_relevance_score")]
    pub min_relevance_score: f64,
    /// Max embedding cache entries before LRU eviction
    #[serde(default = "default_cache_size")]
    pub embedding_cache_size: usize,
    /// Max tokens per chunk for document splitting
    #[serde(default = "default_chunk_size")]
    pub chunk_max_tokens: usize,

    // ── Response Cache (saves tokens on repeated prompts) ──────
    /// Enable LLM response caching to avoid paying for duplicate prompts
    #[serde(default)]
    pub response_cache_enabled: bool,
    /// TTL in minutes for cached responses (default: 60)
    #[serde(default = "default_response_cache_ttl")]
    pub response_cache_ttl_minutes: u32,
    /// Max number of cached responses before LRU eviction (default: 5000)
    #[serde(default = "default_response_cache_max")]
    pub response_cache_max_entries: usize,
    /// Max in-memory hot cache entries for the two-tier response cache (default: 256)
    #[serde(default = "default_response_cache_hot_entries")]
    pub response_cache_hot_entries: usize,

    // ── Memory Snapshot (soul backup to Markdown) ─────────────
    /// Enable periodic export of core memories to MEMORY_SNAPSHOT.md
    #[serde(default)]
    pub snapshot_enabled: bool,
    /// Run snapshot during hygiene passes (heartbeat-driven)
    #[serde(default)]
    pub snapshot_on_hygiene: bool,
    /// Auto-hydrate from MEMORY_SNAPSHOT.md when brain.db is missing
    #[serde(default = "default_true")]
    pub auto_hydrate: bool,

    // ── Retrieval Pipeline ─────────────────────────────────────
    /// Retrieval stages to execute in order. Valid: "cache", "fts", "vector".
    #[serde(default = "default_retrieval_stages")]
    pub retrieval_stages: Vec<String>,
    /// Enable LLM reranking when candidate count exceeds threshold.
    #[serde(default)]
    pub rerank_enabled: bool,
    /// Minimum candidate count to trigger reranking.
    #[serde(default = "default_rerank_threshold")]
    pub rerank_threshold: usize,
    /// FTS score above which to early-return without vector search (0.0–1.0).
    #[serde(default = "default_fts_early_return_score")]
    pub fts_early_return_score: f64,

    // ── Namespace Isolation ─────────────────────────────────────
    /// Default namespace for memory entries.
    #[serde(default = "default_namespace")]
    pub default_namespace: String,

    // ── Conflict Resolution ─────────────────────────────────────
    /// Cosine similarity threshold for conflict detection (0.0–1.0).
    #[serde(default = "default_conflict_threshold")]
    pub conflict_threshold: f64,

    // ── Audit Trail ─────────────────────────────────────────────
    /// Enable audit logging of memory operations.
    #[serde(default)]
    pub audit_enabled: bool,
    /// Retention period for audit entries in days (default: 30).
    #[serde(default = "default_audit_retention_days")]
    pub audit_retention_days: u32,

    // ── Policy Engine ───────────────────────────────────────────
    /// Memory policy configuration.
    #[serde(default)]
    #[nested]
    pub policy: MemoryPolicyConfig,

    // ── SQLite backend options ─────────────────────────────────
    /// For sqlite backend: max seconds to wait when opening the DB (e.g. file locked).
    /// None = wait indefinitely (default). Recommended max: 300.
    #[serde(default)]
    pub sqlite_open_timeout_secs: Option<u64>,

    // ── Qdrant backend options ─────────────────────────────────
    /// Configuration for Qdrant vector database backend.
    /// Only used when `backend = "qdrant"`.
    #[serde(default)]
    #[nested]
    pub qdrant: QdrantConfig,

    // ── PostgreSQL backend options ─────────────────────────────
    /// Configuration for PostgreSQL memory backend (`[memory.postgres]`).
    /// Only used when `backend = "postgres"`.
    #[serde(default)]
    #[nested]
    pub postgres: PostgresMemoryConfig,
}

/// Memory policy configuration (`[memory.policy]` section).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "memory.policy"]
pub struct MemoryPolicyConfig {
    /// Maximum entries per namespace (0 = unlimited).
    #[serde(default)]
    pub max_entries_per_namespace: usize,
    /// Maximum entries per category (0 = unlimited).
    #[serde(default)]
    pub max_entries_per_category: usize,
    /// Retention days by category (overrides global). Keys: "core", "daily", "conversation".
    #[serde(default)]
    pub retention_days_by_category: std::collections::HashMap<String, u32>,
    /// Namespaces that are read-only (writes are rejected).
    #[serde(default)]
    pub read_only_namespaces: Vec<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "agentmemory".into(),
            auto_save: true,
            hygiene_enabled: default_hygiene_enabled(),
            archive_after_days: default_archive_after_days(),
            purge_after_days: default_purge_after_days(),
            conversation_retention_days: default_conversation_retention_days(),
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            vector_weight: default_vector_weight(),
            keyword_weight: default_keyword_weight(),
            search_mode: SearchMode::default(),
            min_relevance_score: default_min_relevance_score(),
            embedding_cache_size: default_cache_size(),
            chunk_max_tokens: default_chunk_size(),
            response_cache_enabled: false,
            response_cache_ttl_minutes: default_response_cache_ttl(),
            response_cache_max_entries: default_response_cache_max(),
            response_cache_hot_entries: default_response_cache_hot_entries(),
            snapshot_enabled: false,
            snapshot_on_hygiene: false,
            auto_hydrate: true,
            retrieval_stages: default_retrieval_stages(),
            rerank_enabled: false,
            rerank_threshold: default_rerank_threshold(),
            fts_early_return_score: default_fts_early_return_score(),
            default_namespace: default_namespace(),
            conflict_threshold: default_conflict_threshold(),
            audit_enabled: false,
            audit_retention_days: default_audit_retention_days(),
            policy: MemoryPolicyConfig::default(),
            sqlite_open_timeout_secs: None,
            qdrant: QdrantConfig::default(),
            postgres: PostgresMemoryConfig::default(),
        }
    }
}

// ── Observability ─────────────────────────────────────────────────

/// Observability backend configuration (`[observability]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "observability"]
pub struct ObservabilityConfig {
    /// "none" | "log" | "verbose" | "prometheus" | "otel"
    pub backend: String,

    /// OTLP endpoint (e.g. `"http://localhost:4318"`). Only used when backend = `"otel"`.
    #[serde(default)]
    pub otel_endpoint: Option<String>,

    /// Service name reported to the OTel collector. Defaults to "operant".
    #[serde(default)]
    pub otel_service_name: Option<String>,

    /// Optional HTTP headers sent with every OTLP export request (e.g. authorization).
    /// Specified as key-value pairs in TOML:
    /// ```toml
    /// [observability.otel_headers]
    /// Authorization = "Bearer sk-..."
    /// ```
    #[serde(default)]
    pub otel_headers: Option<std::collections::HashMap<String, String>>,

    /// Runtime trace storage mode: "none" | "rolling" | "full".
    /// Controls whether model replies and tool-call diagnostics are persisted.
    #[serde(default = "default_runtime_trace_mode")]
    pub runtime_trace_mode: String,

    /// Runtime trace file path. Relative paths are resolved under workspace_dir.
    #[serde(default = "default_runtime_trace_path")]
    pub runtime_trace_path: String,

    /// Maximum entries retained when runtime_trace_mode = "rolling".
    #[serde(default = "default_runtime_trace_max_entries")]
    pub runtime_trace_max_entries: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            backend: "none".into(),
            otel_endpoint: None,
            otel_service_name: None,
            otel_headers: None,
            runtime_trace_mode: default_runtime_trace_mode(),
            runtime_trace_path: default_runtime_trace_path(),
            runtime_trace_max_entries: default_runtime_trace_max_entries(),
        }
    }
}

// ── Hooks ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "hooks"]
/// Lifecycle hook configuration for tool-invocation side effects.
pub struct HooksConfig {
    /// Enable lifecycle hook execution.
    ///
    /// Hooks run in-process with the same privileges as the main runtime.
    /// Keep enabled hook handlers narrowly scoped and auditable.
    pub enabled: bool,
    #[serde(default)]
    #[nested]
    /// Builtin hook handlers (command logger, webhook audit, ...).
    pub builtin: BuiltinHooksConfig,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            builtin: BuiltinHooksConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "hooks.builtin"]
/// Toggles for the builtin hook handlers.
pub struct BuiltinHooksConfig {
    /// Enable the command-logger hook (logs tool calls for auditing).
    pub command_logger: bool,
    /// Configuration for the webhook-audit hook.
    ///
    /// When enabled, POSTs a JSON payload to `url` for every tool invocation
    /// that matches one of `tool_patterns`.
    #[serde(default)]
    #[nested]
    pub webhook_audit: WebhookAuditConfig,
}

/// Configuration for the webhook-audit builtin hook.
///
/// Sends an HTTP POST with a JSON body to an external endpoint each time
/// a tool call matches one of the configured patterns. Useful for
/// centralised audit logging, SIEM ingestion, or compliance pipelines.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "hooks.builtin.webhook-audit"]
pub struct WebhookAuditConfig {
    /// Enable the webhook-audit hook. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Target URL that will receive the audit POST requests.
    #[serde(default)]
    pub url: String,
    /// Glob patterns for tool names to audit (e.g. `["Bash", "Write"]`).
    /// An empty list means **no** tools are audited.
    #[serde(default)]
    pub tool_patterns: Vec<String>,
    /// Include tool call arguments in the audit payload. Default: `false`.
    ///
    /// Be mindful of sensitive data — arguments may contain secrets or PII.
    #[serde(default)]
    pub include_args: bool,
    /// Maximum size (in bytes) of serialised arguments included in a single
    /// audit payload. Arguments exceeding this limit are truncated.
    /// Default: `4096`.
    #[serde(default = "default_max_args_bytes")]
    pub max_args_bytes: u64,
}

impl Default for WebhookAuditConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            tool_patterns: Vec::new(),
            include_args: false,
            max_args_bytes: default_max_args_bytes(),
        }
    }
}

pub(crate) fn is_valid_env_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

// ── Runtime ──────────────────────────────────────────────────────

/// Runtime adapter configuration (`[runtime]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "runtime"]
pub struct RuntimeConfig {
    /// Runtime kind (`native` | `docker`).
    #[serde(default = "default_runtime_kind")]
    pub kind: String,

    /// Docker runtime settings (used when `kind = "docker"`).
    #[serde(default)]
    #[nested]
    pub docker: DockerRuntimeConfig,

    /// Global reasoning override for providers that expose explicit controls.
    /// - `None`: provider default behavior
    /// - `Some(true)`: request reasoning/thinking when supported
    /// - `Some(false)`: disable reasoning/thinking when supported
    #[serde(default)]
    pub reasoning_enabled: Option<bool>,
    /// Optional reasoning effort for providers that expose a level control.
    #[serde(default, deserialize_with = "deserialize_reasoning_effort_opt")]
    pub reasoning_effort: Option<String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            kind: default_runtime_kind(),
            docker: DockerRuntimeConfig::default(),
            reasoning_enabled: None,
            reasoning_effort: None,
        }
    }
}

// ── Reliability / supervision ────────────────────────────────────

/// Reliability and supervision configuration (`[reliability]` section).
///
/// Controls provider retries, fallback chains, API key rotation, and channel restart backoff.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "reliability"]
pub struct ReliabilityConfig {
    /// Retries per provider before failing over.
    #[serde(default = "default_provider_retries")]
    pub provider_retries: u32,
    /// Base backoff (ms) for provider retry delay.
    #[serde(default = "default_provider_backoff_ms")]
    pub provider_backoff_ms: u64,
    /// Fallback provider chain (e.g. `["anthropic", "openai"]`).
    #[serde(default)]
    pub fallback_providers: Vec<String>,
    /// Additional API keys for round-robin rotation on rate-limit (429) errors.
    /// The primary `api_key` is always tried first; these are extras.
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Per-model fallback chains. When a model fails, try these alternatives in order.
    /// Example: `{ "claude-opus-4-20250514" = ["claude-sonnet-4-20250514", "gpt-4o"] }`
    #[serde(default)]
    pub model_fallbacks: std::collections::HashMap<String, Vec<String>>,
    /// Initial backoff for channel/daemon restarts.
    #[serde(default = "default_channel_backoff_secs")]
    pub channel_initial_backoff_secs: u64,
    /// Max backoff for channel/daemon restarts.
    #[serde(default = "default_channel_backoff_max_secs")]
    pub channel_max_backoff_secs: u64,
    /// Scheduler polling cadence in seconds.
    #[serde(default = "default_scheduler_poll_secs")]
    pub scheduler_poll_secs: u64,
    /// Max retries for cron job execution attempts.
    #[serde(default = "default_scheduler_retries")]
    pub scheduler_retries: u32,
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            provider_retries: default_provider_retries(),
            provider_backoff_ms: default_provider_backoff_ms(),
            fallback_providers: Vec::new(),
            api_keys: Vec::new(),
            model_fallbacks: std::collections::HashMap::new(),
            channel_initial_backoff_secs: default_channel_backoff_secs(),
            channel_max_backoff_secs: default_channel_backoff_max_secs(),
            scheduler_poll_secs: default_scheduler_poll_secs(),
            scheduler_retries: default_scheduler_retries(),
        }
    }
}

// ── Model routing ────────────────────────────────────────────────

/// Route a task hint to a specific provider + model.
///
/// ```toml
/// [[model_routes]]
/// hint = "reasoning"
/// provider = "openrouter"
/// model = "anthropic/claude-opus-4-20250514"
///
/// [[model_routes]]
/// hint = "fast"
/// provider = "groq"
/// model = "llama-3.3-70b-versatile"
/// ```
///
/// Usage: pass `hint:reasoning` as the model parameter to route the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ModelRouteConfig {
    /// Task hint name (e.g. "reasoning", "fast", "code", "summarize")
    pub hint: String,
    /// Provider to route to (must match a known provider name)
    pub provider: String,
    /// Model to use with that provider
    pub model: String,
    /// Optional API key override for this route's provider
    #[serde(default)]
    pub api_key: Option<String>,
}

// ── Cross-provider fallback chain ──────────────────────────────

/// One entry in the ordered cross-provider fallback chain (hermes
/// `fallback_providers` parity).
///
/// `provider` names a profile key from `[providers.models]`; `model` is the
/// model identifier to try with that provider when the primary fails with an
/// auth/billing error (cross-provider switch) — or, when `provider` matches
/// the primary provider, the model is folded into `agent.fallback_models`
/// (same-client model swap on 5xx/429/network errors).
///
/// ```toml
/// [[providers.fallback_chain]]
/// provider = "opencode-zen"
/// model = "deepseek-v4-flash-free"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct FallbackProviderConfig {
    /// Provider profile name — must match a key in `[providers.models]`.
    pub provider: String,
    /// Model identifier to send with that provider.
    pub model: String,
}

// ── Query Classification ─────────────────────────────────────────

/// Automatic query classification — classifies user messages by keyword/pattern
/// and routes to the appropriate model hint. Disabled by default.
#[derive(Debug, Clone, Serialize, Deserialize, Default, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "query-classification"]
pub struct QueryClassificationConfig {
    /// Enable automatic query classification. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Classification rules evaluated in priority order.
    #[serde(default)]
    pub rules: Vec<ClassificationRule>,
}

/// A single classification rule mapping message patterns to a model hint.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct ClassificationRule {
    /// Must match a `[[model_routes]]` hint value.
    pub hint: String,
    /// Case-insensitive substring matches.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Case-sensitive literal matches (for "```", "fn ", etc.).
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Only match if message length >= N chars.
    #[serde(default)]
    pub min_length: Option<usize>,
    /// Only match if message length <= N chars.
    #[serde(default)]
    pub max_length: Option<usize>,
    /// Higher priority rules are checked first.
    #[serde(default)]
    pub priority: i32,
}

// ── Cron ────────────────────────────────────────────────────────

/// Cron job configuration (`[cron]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "cron"]
#[integration(
    category = "ToolsAutomation",
    display_name = "Cron",
    description = "Scheduled tasks",
    status_field = "enabled"
)]
pub struct CronConfig {
    /// Enable the cron subsystem. Default: `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Run all overdue jobs at scheduler startup. Default: `true`.
    ///
    /// When the machine boots late or the daemon restarts, jobs whose
    /// `next_run` is in the past are considered "missed". With this
    /// option enabled the scheduler fires them once before entering
    /// the normal polling loop. Disable if you prefer missed jobs to
    /// simply wait for their next scheduled occurrence.
    #[serde(default = "default_true")]
    pub catch_up_on_startup: bool,
    /// Maximum number of historical cron run records to retain. Default: `50`.
    #[serde(default = "default_max_run_history")]
    pub max_run_history: u32,
    /// Declarative cron job definitions (`[[cron.jobs]]`).
    ///
    /// Jobs declared here are synced into the database at scheduler startup.
    /// They use `source = "declarative"` to distinguish them from jobs
    /// created imperatively via CLI or API. Declarative config takes
    /// precedence on each sync: if the config changes, the DB is updated
    /// to match. Imperative jobs are never deleted by the sync process.
    #[serde(default)]
    pub jobs: Vec<CronJobDecl>,
}

/// Delivery configuration for declarative cron jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct DeliveryConfigDecl {
    /// Delivery mode: `"none"` or `"announce"`.
    #[serde(default = "default_delivery_mode")]
    pub mode: String,
    /// Channel name (e.g. `"telegram"`, `"discord"`).
    #[serde(default)]
    pub channel: Option<String>,
    /// Target/recipient identifier.
    #[serde(default)]
    pub to: Option<String>,
    /// Optional thread/conversation identifier carried into the outbound send.
    /// Required by channels that route on a separate `thread_id` field (e.g.
    /// webhook callbacks bridging into agent-chat platforms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Best-effort delivery. Default: `true`.
    #[serde(default = "default_true")]
    pub best_effort: bool,
}

// ── Tunnel ──────────────────────────────────────────────────────

/// Tunnel configuration for exposing the gateway publicly (`[tunnel]` section).
///
/// Supported providers: `"none"` (default), `"cloudflare"`, `"tailscale"`, `"ngrok"`, `"openvpn"`, `"pinggy"`, `"custom"`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel"]
pub struct TunnelConfig {
    /// How the gateway gets exposed to the public internet so webhooks (Telegram, Slack, etc.) can reach it. `none` = keep it local, no tunnel; `cloudflare` = Cloudflare Tunnel via cloudflared (needs a Zero Trust account and token); `tailscale` = Tailscale Funnel/Serve (tailnet-only or public, no account beyond tailscale); `ngrok` = ngrok agent with auth token; `openvpn` = bring-your-own OpenVPN egress; `pinggy` = Pinggy SSH tunnels (quick one-shot URLs); `custom` = run an arbitrary command you define under `[tunnel.custom]`.
    pub provider: String,

    /// Cloudflare Tunnel configuration (used when `provider = "cloudflare"`).
    #[serde(default)]
    #[nested]
    pub cloudflare: Option<CloudflareTunnelConfig>,

    /// Tailscale Funnel/Serve configuration (used when `provider = "tailscale"`).
    #[serde(default)]
    #[nested]
    pub tailscale: Option<TailscaleTunnelConfig>,

    /// ngrok tunnel configuration (used when `provider = "ngrok"`).
    #[serde(default)]
    #[nested]
    pub ngrok: Option<NgrokTunnelConfig>,

    /// OpenVPN tunnel configuration (used when `provider = "openvpn"`).
    #[serde(default)]
    #[nested]
    pub openvpn: Option<OpenVpnTunnelConfig>,

    /// Custom tunnel command configuration (used when `provider = "custom"`).
    #[serde(default)]
    #[nested]
    pub custom: Option<CustomTunnelConfig>,

    /// Pinggy tunnel configuration (used when `provider = "pinggy"`).
    #[serde(default)]
    #[nested]
    pub pinggy: Option<PinggyTunnelConfig>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            cloudflare: None,
            tailscale: None,
            ngrok: None,
            openvpn: None,
            custom: None,
            pinggy: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.cloudflare"]
/// Cloudflare Tunnel configuration (quick tunnel via `cloudflared`).
pub struct CloudflareTunnelConfig {
    /// Cloudflare Tunnel token (from Zero Trust dashboard)
    #[serde(default)]
    #[secret]
    pub token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.tailscale"]
/// Tailscale Serve/Funnel tunnel configuration.
pub struct TailscaleTunnelConfig {
    /// Use Tailscale Funnel (public internet) vs Serve (tailnet only)
    #[serde(default)]
    pub funnel: bool,
    /// Optional hostname override
    #[serde(default)]
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.ngrok"]
/// ngrok tunnel configuration.
pub struct NgrokTunnelConfig {
    /// ngrok auth token
    #[serde(default)]
    #[secret]
    pub auth_token: String,
    /// Optional custom domain
    #[serde(default)]
    pub domain: Option<String>,
}

/// OpenVPN tunnel configuration (`[tunnel.openvpn]`).
///
/// Required when `tunnel.provider = "openvpn"`. Omitting this section entirely
/// preserves previous behavior. Setting `tunnel.provider = "none"` (or removing
/// the `[tunnel.openvpn]` block) cleanly reverts to no-tunnel mode.
///
/// Defaults: `connect_timeout_secs = 30`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.openvpn"]
pub struct OpenVpnTunnelConfig {
    /// Path to `.ovpn` configuration file (must not be empty).
    pub config_file: String,
    /// Optional path to auth credentials file (`--auth-user-pass`).
    #[serde(default)]
    pub auth_file: Option<String>,
    /// Advertised address once VPN is connected (e.g., `"10.8.0.2:42617"`).
    /// When omitted the tunnel falls back to `http://{local_host}:{local_port}`.
    #[serde(default)]
    pub advertise_address: Option<String>,
    /// Connection timeout in seconds (default: 30, must be > 0).
    #[serde(default = "default_openvpn_timeout")]
    pub connect_timeout_secs: u64,
    /// Extra openvpn CLI arguments forwarded verbatim.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

impl Default for OpenVpnTunnelConfig {
    fn default() -> Self {
        Self {
            config_file: String::new(),
            auth_file: None,
            advertise_address: None,
            connect_timeout_secs: default_openvpn_timeout(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.pinggy"]
/// Pinggy tunnel configuration.
pub struct PinggyTunnelConfig {
    /// Pinggy access token (optional — free tier works without one).
    #[serde(default)]
    #[secret]
    pub token: Option<String>,
    /// Server region: `"us"` (USA), `"eu"` (Europe), `"ap"` (Asia), `"br"` (South America), `"au"` (Australia), or omit for auto.
    #[serde(default)]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "tunnel.custom"]
/// Custom command-driven tunnel configuration.
pub struct CustomTunnelConfig {
    /// Command template to start the tunnel. Use {port} and {host} placeholders.
    /// Example: "bore local {port} --to bore.pub"
    #[serde(default)]
    pub start_command: String,
    /// Optional URL to check tunnel health
    #[serde(default)]
    pub health_url: Option<String>,
    /// Optional regex to extract public URL from command stdout
    #[serde(default)]
    pub url_pattern: Option<String>,
}

// ── Channels ─────────────────────────────────────────────────────

pub(crate) struct ConfigWrapper<T: ChannelConfig>(std::marker::PhantomData<T>);

impl<T: ChannelConfig> ConfigWrapper<T> {
    pub(crate) fn new(_: Option<&T>) -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ChannelConfig> crate::traits::ConfigHandle for ConfigWrapper<T> {
    fn name(&self) -> &'static str {
        T::name()
    }
    fn desc(&self) -> &'static str {
        T::desc()
    }
}

/// Streaming mode for channels that support progressive message updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    /// No streaming -- send the complete response as a single message (default).
    #[default]
    Off,
    /// Update a draft message with every flush interval.
    Partial,
    /// Send the response as multiple separate messages at paragraph boundaries.
    #[serde(rename = "multi_message")]
    MultiMessage,
}

/// Mattermost bot channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.mattermost"]
pub struct MattermostConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Mattermost server URL (e.g. `"https://mattermost.example.com"`).
    pub url: String,
    /// Mattermost bot access token.
    #[secret]
    pub bot_token: String,
    /// Optional channel ID to restrict the bot to a single channel.
    pub channel_id: Option<String>,
    /// Allowed Mattermost user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true (default), replies thread on the original post.
    /// When false, replies go to the channel root.
    #[serde(default)]
    pub thread_replies: Option<bool>,
    /// When true, only respond to messages that @-mention the bot.
    /// Other messages in the channel are silently ignored.
    #[serde(default)]
    pub mention_only: Option<bool>,
    /// When true, a newer Mattermost message from the same sender in the same channel
    /// cancels the in-flight request and starts a fresh response with preserved history.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for MattermostConfig {
    fn name() -> &'static str {
        "Mattermost"
    }
    fn desc() -> &'static str {
        "connect to your bot"
    }
}

/// Webhook channel configuration.
///
/// Receives messages via HTTP POST and sends replies to a configurable outbound URL.
/// This is the "universal adapter" for any system that supports webhooks.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.webhook"]
pub struct WebhookConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Port to listen on for incoming webhooks.
    pub port: u16,
    /// URL path to listen on (default: `/webhook`).
    #[serde(default)]
    pub listen_path: Option<String>,
    /// URL to POST/PUT outbound messages to.
    #[serde(default)]
    pub send_url: Option<String>,
    /// HTTP method for outbound messages (`POST` or `PUT`). Default: `POST`.
    #[serde(default)]
    pub send_method: Option<String>,
    /// Optional `Authorization` header value for outbound requests.
    #[serde(default)]
    pub auth_header: Option<String>,
    /// Optional shared secret for webhook signature verification (HMAC-SHA256).
    #[secret]
    pub secret: Option<String>,
}

impl ChannelConfig for WebhookConfig {
    fn name() -> &'static str {
        "Webhook"
    }
    fn desc() -> &'static str {
        "HTTP endpoint"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.linq"]
/// Linq SMS channel configuration.
pub struct LinqConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Linq Partner API token (Bearer auth)
    #[secret]
    pub api_token: String,
    /// Phone number to send from (E.164 format)
    pub from_phone: String,
    /// Webhook signing secret for signature verification
    #[serde(default)]
    #[secret]
    pub signing_secret: Option<String>,
    /// Allowed sender handles (phone numbers) or "*" for all
    #[serde(default)]
    pub allowed_senders: Vec<String>,
}

impl ChannelConfig for LinqConfig {
    fn name() -> &'static str {
        "Linq"
    }
    fn desc() -> &'static str {
        "iMessage/RCS/SMS via Linq API"
    }
}

/// Nextcloud Talk bot configuration (webhook receive + OCS send API).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.nextcloud-talk"]
pub struct NextcloudTalkConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Nextcloud base URL (e.g. `"https://cloud.example.com"`).
    pub base_url: String,
    /// Bot app token used for OCS API bearer auth.
    #[secret]
    pub app_token: String,
    /// Shared secret for webhook signature verification.
    ///
    /// Can also be set via `OPERANT_NEXTCLOUD_TALK_WEBHOOK_SECRET`.
    #[serde(default)]
    #[secret]
    pub webhook_secret: Option<String>,
    /// Allowed Nextcloud actor IDs (`[]` = deny all, `"*"` = allow all).
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Display name of the bot in Nextcloud Talk (e.g. "operant").
    /// Used to filter out the bot's own messages and prevent feedback loops.
    /// If not set, defaults to an empty string (no self-message filtering by name).
    #[serde(default)]
    pub bot_name: Option<String>,
    /// Controls whether and how streaming draft updates are delivered.
    ///
    /// - `"off"` (default) — responses are sent as a single final message.
    /// - `"partial"` — a placeholder is posted first and edited incrementally
    ///   as tokens arrive, making long responses visible in real time.
    #[serde(default)]
    pub stream_mode: StreamMode,
    /// Minimum interval in milliseconds between consecutive OCS edit calls per
    /// room when `stream_mode = "partial"`. Default: 1000 ms.
    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
}

impl ChannelConfig for NextcloudTalkConfig {
    fn name() -> &'static str {
        "NextCloud Talk"
    }
    fn desc() -> &'static str {
        "NextCloud Talk platform"
    }
}

/// How Operant receives events from Feishu / Lark.
///
/// - `websocket` (default) — persistent WSS long-connection; no public URL required.
/// - `webhook`             — HTTP callback server; requires a public HTTPS endpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum LarkReceiveMode {
    #[default]
    /// Persistent WebSocket long-connection; no public URL required.
    Websocket,
    /// HTTP callback server; requires a public HTTPS endpoint.
    Webhook,
}

/// Lark/Feishu configuration for messaging integration.
/// Lark is the international version; Feishu is the Chinese version.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.lark"]
pub struct LarkConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// App ID from Lark/Feishu developer console
    pub app_id: String,
    /// App Secret from Lark/Feishu developer console
    #[secret]
    pub app_secret: String,
    /// Encrypt key for webhook message decryption (optional)
    #[serde(default)]
    #[secret]
    pub encrypt_key: Option<String>,
    /// Verification token for webhook validation (optional)
    #[serde(default)]
    #[secret]
    pub verification_token: Option<String>,
    /// Allowed user IDs or union IDs (empty = deny all, "*" = allow all)
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true, only respond to messages that @-mention the bot in groups.
    /// Direct messages are always processed.
    #[serde(default)]
    pub mention_only: bool,
    /// Whether to use the Feishu (Chinese) endpoint instead of Lark (International)
    #[serde(default)]
    pub use_feishu: bool,
    /// Event receive mode: "websocket" (default) or "webhook"
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,
    /// HTTP port for webhook mode only. Must be set when receive_mode = "webhook".
    /// Not required (and ignored) for websocket mode.
    #[serde(default)]
    pub port: Option<u16>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for LarkConfig {
    fn name() -> &'static str {
        "Lark"
    }
    fn desc() -> &'static str {
        "Lark Bot"
    }
}

/// DM (1:1 chat) access policy for the LINE channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum LineDmPolicy {
    /// Respond to every DM regardless of who sent it.
    Open,
    /// Require a one-time `/bind <code>` handshake before responding (default).
    /// Operant prints the bind code on startup; send it once to unlock access.
    #[default]
    Pairing,
    /// Respond only to LINE user IDs listed in `allowed_users`.
    Allowlist,
}

/// Group / multi-person chat policy for the LINE channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum LineGroupPolicy {
    /// Respond to every message in group/room chats.
    Open,
    /// Respond only when the bot is @mentioned (default).
    #[default]
    Mention,
    /// Ignore all messages in group/room chats.
    Disabled,
}

/// LINE Messaging API channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.line"]
pub struct LineConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Long-lived channel access token (from LINE Developers Console).
    /// Used for both the Reply API and the Push API fallback.
    /// Falls back to the `LINE_CHANNEL_ACCESS_TOKEN` environment variable if empty.
    #[serde(default)]
    #[secret]
    pub channel_access_token: String,
    /// Channel secret (from LINE Developers Console).
    /// Used to verify the `X-Line-Signature` header on incoming webhooks.
    /// Falls back to the `LINE_CHANNEL_SECRET` environment variable if empty.
    #[serde(default)]
    #[secret]
    pub channel_secret: String,
    /// DM (1:1 chat) access policy. Default: `pairing`.
    ///
    /// - `open`      — respond to everyone
    /// - `pairing`   — require one-time `/bind <code>` handshake on first contact
    /// - `allowlist` — respond only to user IDs listed in `allowed_users`
    #[serde(default)]
    pub dm_policy: LineDmPolicy,
    /// Group / multi-person chat policy. Default: `mention`.
    ///
    /// - `open`     — respond to every message
    /// - `mention`  — respond only when @mentioned
    /// - `disabled` — ignore all group messages
    #[serde(default)]
    pub group_policy: LineGroupPolicy,
    /// LINE user IDs that are allowed to interact with the bot.
    /// Used when `dm_policy = allowlist`. `["*"]` accepts everyone.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// TCP port the embedded webhook server listens on. Default: `8443`.
    #[serde(default = "default_line_webhook_port")]
    pub webhook_port: u16,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for LineConfig {
    fn name() -> &'static str {
        "LINE"
    }
    fn desc() -> &'static str {
        "connect your LINE bot"
    }
}

// ── Security Config ─────────────────────────────────────────────────

/// Security configuration for sandboxing, resource limits, and audit logging
#[derive(Debug, Clone, Serialize, Deserialize, Default, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security"]
pub struct SecurityConfig {
    /// Sandbox configuration
    #[serde(default)]
    #[nested]
    pub sandbox: SandboxConfig,

    /// Resource limits
    #[serde(default)]
    #[nested]
    pub resources: ResourceLimitsConfig,

    /// Audit logging configuration
    #[serde(default)]
    #[nested]
    pub audit: AuditConfig,

    /// OTP gating configuration for sensitive actions/domains.
    #[serde(default)]
    #[nested]
    pub otp: OtpConfig,

    /// Emergency-stop state machine configuration.
    #[serde(default)]
    #[nested]
    pub estop: EstopConfig,

    /// Nevis IAM integration for SSO/MFA authentication and role-based access.
    #[serde(default)]
    #[nested]
    pub nevis: NevisConfig,

    /// WebAuthn / FIDO2 hardware key authentication configuration.
    #[serde(default)]
    #[nested]
    pub webauthn: WebAuthnConfig,
}

/// OTP validation strategy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum OtpMethod {
    /// Time-based one-time password (RFC 6238).
    #[default]
    Totp,
    /// Future method for paired-device confirmations.
    Pairing,
    /// Future method for local CLI challenge prompts.
    CliPrompt,
}

/// Security OTP configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security.otp"]
#[serde(deny_unknown_fields)]
pub struct OtpConfig {
    /// Enable OTP gating. Defaults to disabled for backward compatibility.
    #[serde(default)]
    pub enabled: bool,

    /// OTP method.
    #[serde(default)]
    pub method: OtpMethod,

    /// TOTP time-step in seconds.
    #[serde(default = "default_otp_token_ttl_secs")]
    pub token_ttl_secs: u64,

    /// Reuse window for recently validated OTP codes.
    #[serde(default = "default_otp_cache_valid_secs")]
    pub cache_valid_secs: u64,

    /// Tool/action names gated by OTP.
    #[serde(default = "default_otp_gated_actions")]
    pub gated_actions: Vec<String>,

    /// Explicit domain patterns gated by OTP.
    #[serde(default)]
    pub gated_domains: Vec<String>,

    /// Domain-category presets expanded into `gated_domains`.
    #[serde(default)]
    pub gated_domain_categories: Vec<String>,

    /// Maximum number of OTP challenge attempts before lockout.
    #[serde(default = "default_otp_challenge_max_attempts")]
    pub challenge_max_attempts: u32,
}

impl Default for OtpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            method: OtpMethod::Totp,
            token_ttl_secs: default_otp_token_ttl_secs(),
            cache_valid_secs: default_otp_cache_valid_secs(),
            gated_actions: default_otp_gated_actions(),
            gated_domains: Vec::new(),
            gated_domain_categories: Vec::new(),
            challenge_max_attempts: default_otp_challenge_max_attempts(),
        }
    }
}

/// Sandbox configuration for OS-level isolation
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security.sandbox"]
pub struct SandboxConfig {
    /// Enable sandboxing (None = auto-detect, Some = explicit)
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Sandbox backend to use
    #[serde(default)]
    pub backend: SandboxBackend,

    /// Custom Firejail arguments (when backend = firejail)
    #[serde(default)]
    pub firejail_args: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: None, // Auto-detect
            backend: SandboxBackend::Auto,
            firejail_args: Vec::new(),
        }
    }
}

/// Sandbox backend selection
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    /// Auto-detect best available (default)
    #[default]
    Auto,
    /// Landlock (Linux kernel LSM, native)
    Landlock,
    /// Firejail (user-space sandbox)
    Firejail,
    /// Bubblewrap (user namespaces)
    Bubblewrap,
    /// Docker container isolation
    Docker,
    /// macOS sandbox-exec (Seatbelt)
    #[serde(alias = "sandbox-exec")]
    SandboxExec,
    /// No sandboxing (application-layer only)
    None,
}

/// Resource limits for command execution
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security.resources"]
pub struct ResourceLimitsConfig {
    /// Maximum memory in MB per command
    #[serde(default = "default_max_memory_mb")]
    pub max_memory_mb: u32,

    /// Maximum CPU time in seconds per command
    #[serde(default = "default_max_cpu_time_seconds")]
    pub max_cpu_time_seconds: u64,

    /// Maximum number of subprocesses
    #[serde(default = "default_max_subprocesses")]
    pub max_subprocesses: u32,

    /// Enable memory monitoring
    #[serde(default = "default_memory_monitoring_enabled")]
    pub memory_monitoring: bool,
}

impl Default for ResourceLimitsConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: default_max_memory_mb(),
            max_cpu_time_seconds: default_max_cpu_time_seconds(),
            max_subprocesses: default_max_subprocesses(),
            memory_monitoring: default_memory_monitoring_enabled(),
        }
    }
}

/// Audit logging configuration
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security.audit"]
pub struct AuditConfig {
    /// Enable audit logging
    #[serde(default = "default_audit_enabled")]
    pub enabled: bool,

    /// Path to audit log file (relative to operant dir)
    #[serde(default = "default_audit_log_path")]
    pub log_path: String,

    /// Maximum log size in MB before rotation
    #[serde(default = "default_audit_max_size_mb")]
    pub max_size_mb: u32,

    /// Sign events with HMAC for tamper evidence
    #[serde(default)]
    pub sign_events: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: default_audit_enabled(),
            log_path: default_audit_log_path(),
            max_size_mb: default_audit_max_size_mb(),
            sign_events: false,
        }
    }
}

/// WeCom (WeChat Enterprise) Bot Webhook configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.wecom"]
pub struct WeComConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Webhook key from WeCom Bot configuration
    #[secret]
    pub webhook_key: String,
    /// Allowed user IDs. Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

impl ChannelConfig for WeComConfig {
    fn name() -> &'static str {
        "WeCom"
    }
    fn desc() -> &'static str {
        "WeCom Bot Webhook"
    }
}

/// WeChat personal iLink Bot channel configuration.
///
/// Uses the iLink Bot API (`ilinkai.weixin.qq.com`) with QR-code login.
/// The bot token is obtained by scanning a QR code and persisted to disk
/// so subsequent restarts do not require re-scanning.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.wechat"]
pub struct WeChatConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed WeChat user IDs (e.g. `"xxx@im.wechat"`).
    /// `"*"` = allow all. Empty = require pairing (`/bind <code>` from WeChat);
    /// the QR-login user is auto-added at first connect.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Override the iLink API base URL. Default: `https://ilinkai.weixin.qq.com`.
    #[serde(default)]
    pub api_base_url: Option<String>,
    /// Override the CDN base URL. Default: `https://novac2c.cdn.weixin.qq.com/c2c`.
    #[serde(default)]
    pub cdn_base_url: Option<String>,
    /// Directory to persist bot token and sync cursor.
    /// Default: `~/.operant/wechat/`.
    #[serde(default)]
    pub state_dir: Option<String>,
}

impl ChannelConfig for WeChatConfig {
    fn name() -> &'static str {
        "WeChat"
    }
    fn desc() -> &'static str {
        "WeChat iLink Bot"
    }
}

/// X/Twitter channel configuration (Twitter API v2)
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.twitter"]
pub struct TwitterConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Twitter API v2 Bearer Token (OAuth 2.0)
    #[secret]
    pub bearer_token: String,
    /// Allowed usernames or user IDs. Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
}

impl ChannelConfig for TwitterConfig {
    fn name() -> &'static str {
        "X/Twitter"
    }
    fn desc() -> &'static str {
        "X/Twitter Bot via API v2"
    }
}

/// Reddit channel configuration (OAuth2 bot).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.reddit"]
pub struct RedditConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Reddit OAuth2 client ID.
    pub client_id: String,
    /// Reddit OAuth2 client secret.
    #[secret]
    pub client_secret: String,
    /// Reddit OAuth2 refresh token for persistent access.
    #[secret]
    pub refresh_token: String,
    /// Reddit bot username (without `u/` prefix).
    pub username: String,
    /// Optional subreddit to filter messages (without `r/` prefix).
    /// When set, only messages from this subreddit are processed.
    #[serde(default)]
    pub subreddit: Option<String>,
}

impl ChannelConfig for RedditConfig {
    fn name() -> &'static str {
        "Reddit"
    }
    fn desc() -> &'static str {
        "Reddit bot (OAuth2)"
    }
}

/// Bluesky channel configuration (AT Protocol).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.bluesky"]
pub struct BlueskyConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Bluesky handle (e.g. `"mybot.bsky.social"`).
    pub handle: String,
    /// App-specific password (from Bluesky settings).
    #[secret]
    pub app_password: String,
}

impl ChannelConfig for BlueskyConfig {
    fn name() -> &'static str {
        "Bluesky"
    }
    fn desc() -> &'static str {
        "AT Protocol"
    }
}

/// Voice duplex configuration (`[channels.voice_duplex]`).
///
/// Enables full-duplex voice event handling over WebSocket.
/// When disabled (default), voice events are rejected as unknown types.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable, Default)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct VoiceDuplexConfig {
    /// Enable full-duplex voice event handling over WebSocket.
    /// Default: false. When false, voice events are rejected as unknown types.
    #[serde(default)]
    pub enabled: bool,
}

/// Voice wake word detection channel configuration.
///
/// Listens on the default microphone for a configurable wake word,
/// then captures the following utterance and transcribes it via the
/// existing transcription API.
#[cfg(feature = "voice-wake")]
#[derive(Debug, Clone, Serialize, Deserialize, operant_macros::Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "voice-wake"]
pub struct VoiceWakeConfig {
    /// Wake word phrase to listen for (case-insensitive substring match).
    /// Default: `"hey operant"`.
    #[serde(default = "default_voice_wake_word")]
    pub wake_word: String,
    /// Silence timeout in milliseconds — how long to wait after the last
    /// energy spike before finalizing a capture window. Default: `2000`.
    #[serde(default = "default_voice_wake_silence_timeout_ms")]
    pub silence_timeout_ms: u32,
    /// RMS energy threshold for voice activity detection. Samples below
    /// this level are treated as silence. Default: `0.01`.
    #[serde(default = "default_voice_wake_energy_threshold")]
    pub energy_threshold: f32,
    /// Maximum capture duration in seconds before forcing transcription.
    /// Default: `30`.
    #[serde(default = "default_voice_wake_max_capture_secs")]
    pub max_capture_secs: u32,
}

#[cfg(feature = "voice-wake")]
impl Default for VoiceWakeConfig {
    fn default() -> Self {
        Self {
            wake_word: default_voice_wake_word(),
            silence_timeout_ms: default_voice_wake_silence_timeout_ms(),
            energy_threshold: default_voice_wake_energy_threshold(),
            max_capture_secs: default_voice_wake_max_capture_secs(),
        }
    }
}

#[cfg(feature = "voice-wake")]
impl ChannelConfig for VoiceWakeConfig {
    fn name() -> &'static str {
        "VoiceWake"
    }
    fn desc() -> &'static str {
        "voice wake word detection"
    }
}

/// Nostr channel configuration (NIP-04 + NIP-17 private messages)
#[cfg(feature = "channel-nostr")]
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.nostr"]
pub struct NostrConfig {
    /// Whether this channel is active. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Private key in hex or nsec bech32 format
    #[secret]
    pub private_key: String,
    /// Relay URLs (wss://). Defaults to popular public relays if omitted.
    #[serde(default = "default_nostr_relays")]
    pub relays: Vec<String>,
    /// Allowed sender public keys (hex or npub). Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_pubkeys: Vec<String>,
}

#[cfg(feature = "channel-nostr")]
impl ChannelConfig for NostrConfig {
    fn name() -> &'static str {
        "Nostr"
    }
    fn desc() -> &'static str {
        "Nostr DMs"
    }
}

///
/// Controls the read-only cloud transformation analysis tools:
/// IaC review, migration assessment, cost analysis, and architecture review.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "cloud-ops"]
pub struct CloudOpsConfig {
    /// Enable cloud operations tools. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Default cloud provider for analysis context. Default: "aws".
    #[serde(default = "default_cloud_ops_cloud")]
    pub default_cloud: String,
    /// Supported cloud providers. Default: [`aws`, `azure`, `gcp`].
    #[serde(default = "default_cloud_ops_supported_clouds")]
    pub supported_clouds: Vec<String>,
    /// Supported IaC tools for review. Default: \[`terraform`\].
    #[serde(default = "default_cloud_ops_iac_tools")]
    pub iac_tools: Vec<String>,
    /// Monthly USD threshold to flag cost items. Default: 100.0.
    #[serde(default = "default_cloud_ops_cost_threshold")]
    pub cost_threshold_monthly_usd: f64,
    /// Well-Architected Frameworks to check against. Default: \[`aws-waf`\].
    #[serde(default = "default_cloud_ops_waf")]
    pub well_architected_frameworks: Vec<String>,
}

impl Default for CloudOpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_cloud: default_cloud_ops_cloud(),
            supported_clouds: default_cloud_ops_supported_clouds(),
            iac_tools: default_cloud_ops_iac_tools(),
            cost_threshold_monthly_usd: default_cloud_ops_cost_threshold(),
            well_architected_frameworks: default_cloud_ops_waf(),
        }
    }
}

impl CloudOpsConfig {
    /// Validate the cloud-ops config, bailing on empty required fields when enabled.
    pub fn validate(&self) -> Result<()> {
        if self.enabled {
            if self.default_cloud.trim().is_empty() {
                anyhow::bail!(
                    "cloud_ops.default_cloud must not be empty when cloud_ops is enabled"
                );
            }
            if self.supported_clouds.is_empty() {
                anyhow::bail!(
                    "cloud_ops.supported_clouds must not be empty when cloud_ops is enabled"
                );
            }
            for (i, cloud) in self.supported_clouds.iter().enumerate() {
                if cloud.trim().is_empty() {
                    validation_bail!(
                        RequiredFieldEmpty,
                        format!("cloud_ops.supported_clouds[{i}]"),
                        "cloud_ops.supported_clouds[{i}] must not be empty"
                    );
                }
            }
            if !self.supported_clouds.contains(&self.default_cloud) {
                anyhow::bail!(
                    "cloud_ops.default_cloud '{}' is not in cloud_ops.supported_clouds {:?}",
                    self.default_cloud,
                    self.supported_clouds
                );
            }
            if self.cost_threshold_monthly_usd < 0.0 {
                anyhow::bail!(
                    "cloud_ops.cost_threshold_monthly_usd must be non-negative, got {}",
                    self.cost_threshold_monthly_usd
                );
            }
            if self.iac_tools.is_empty() {
                anyhow::bail!("cloud_ops.iac_tools must not be empty when cloud_ops is enabled");
            }
        }
        Ok(())
    }
}

/// Conversational AI agent builder configuration (`[conversational_ai]` section).
///
/// **Status: Reserved for future use.** This configuration is parsed but not yet
/// consumed by the runtime. Setting `enabled = true` will produce a startup warning.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "conversational-ai"]
pub struct ConversationalAiConfig {
    /// Enable conversational AI features. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Default language for conversations (BCP-47 tag). Default: "en".
    #[serde(default = "default_conversational_ai_language")]
    pub default_language: String,
    /// Supported languages for conversations. Default: [`en`, `de`, `fr`, `it`].
    #[serde(default = "default_conversational_ai_supported_languages")]
    pub supported_languages: Vec<String>,
    /// Automatically detect user language from message content. Default: true.
    #[serde(default = "default_true")]
    pub auto_detect_language: bool,
    /// Intent confidence below this threshold triggers escalation. Default: 0.3.
    #[serde(default = "default_conversational_ai_escalation_threshold")]
    pub escalation_confidence_threshold: f64,
    /// Maximum conversation turns before auto-ending. Default: 50.
    #[serde(default = "default_conversational_ai_max_turns")]
    pub max_conversation_turns: usize,
    /// Conversation timeout in seconds (inactivity). Default: 1800.
    #[serde(default = "default_conversational_ai_timeout_secs")]
    pub conversation_timeout_secs: u64,
    /// Enable conversation analytics tracking. Default: false (privacy-by-default).
    #[serde(default)]
    pub analytics_enabled: bool,
    /// Optional tool name for RAG-based knowledge base lookup during conversations.
    #[serde(default)]
    pub knowledge_base_tool: Option<String>,
}

impl ConversationalAiConfig {
    /// Returns `true` when the feature is disabled (the default).
    ///
    /// Used by `#[serde(skip_serializing_if)]` to omit the entire
    /// `[conversational_ai]` section from newly-generated config files,
    /// avoiding user confusion over an undocumented / experimental section.
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

impl Default for ConversationalAiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_language: default_conversational_ai_language(),
            supported_languages: default_conversational_ai_supported_languages(),
            auto_detect_language: true,
            escalation_confidence_threshold: default_conversational_ai_escalation_threshold(),
            max_conversation_turns: default_conversational_ai_max_turns(),
            conversation_timeout_secs: default_conversational_ai_timeout_secs(),
            analytics_enabled: false,
            knowledge_base_tool: None,
        }
    }
}

// ── Security ops config ─────────────────────────────────────────

/// Managed Cybersecurity Service (MCSS) dashboard agent configuration (`[security_ops]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security-ops"]
pub struct SecurityOpsConfig {
    /// Enable security operations tools.
    #[serde(default)]
    pub enabled: bool,
    /// Directory containing incident response playbook definitions (JSON).
    #[serde(default = "default_playbooks_dir")]
    pub playbooks_dir: String,
    /// Automatically triage incoming alerts without user prompt.
    #[serde(default)]
    pub auto_triage: bool,
    /// Require human approval before executing playbook actions.
    #[serde(default = "default_require_approval")]
    pub require_approval_for_actions: bool,
    /// Maximum severity level that can be auto-remediated without approval.
    /// One of: "low", "medium", "high", "critical". Default: "low".
    #[serde(default = "default_max_auto_severity")]
    pub max_auto_severity: String,
    /// Directory for generated security reports.
    #[serde(default = "default_report_output_dir")]
    pub report_output_dir: String,
    /// Optional SIEM webhook URL for alert ingestion.
    #[serde(default)]
    pub siem_integration: Option<String>,
}

impl Default for SecurityOpsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            playbooks_dir: default_playbooks_dir(),
            auto_triage: false,
            require_approval_for_actions: true,
            max_auto_severity: default_max_auto_severity(),
            report_output_dir: default_report_output_dir(),
            siem_integration: None,
        }
    }
}

// ── Config impl ──────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        let home =
            UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf());
        let operant_dir = home.join(".operant");

        Self {
            workspace_dir: operant_dir.join("workspace"),
            config_path: operant_dir.join("config.toml"),
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            providers: crate::providers::ProvidersConfig::default(),
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            trust: crate::scattered_types::TrustConfig::default(),
            backup: BackupConfig::default(),
            data_retention: DataRetentionConfig::default(),
            cloud_ops: CloudOpsConfig::default(),
            conversational_ai: ConversationalAiConfig::default(),
            security: SecurityConfig::default(),
            security_ops: SecurityOpsConfig::default(),
            runtime: RuntimeConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            agent: AgentConfig::default(),
            pacing: PacingConfig::default(),
            skills: SkillsConfig::default(),
            pipeline: PipelineConfig::default(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            channels: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            microsoft365: Microsoft365Config::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            browser_delegate: crate::scattered_types::BrowserDelegateConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            media_pipeline: MediaPipelineConfig::default(),
            web_fetch: WebFetchConfig::default(),
            link_enricher: LinkEnricherConfig::default(),
            text_browser: TextBrowserConfig::default(),
            web_search: WebSearchConfig::default(),
            project_intel: ProjectIntelConfig::default(),
            google_workspace: GoogleWorkspaceConfig::default(),
            proxy: ProxyConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            delegate: DelegateToolConfig::default(),
            agents: HashMap::new(),
            swarms: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            transcription: TranscriptionConfig::default(),
            tts: TtsConfig::default(),
            mcp: McpConfig::default(),
            nodes: NodesConfig::default(),
            workspace: WorkspaceConfig::default(),
            onboard_state: OnboardStateConfig::default(),
            notion: NotionConfig::default(),
            jira: JiraConfig::default(),
            node_transport: NodeTransportConfig::default(),
            knowledge: KnowledgeConfig::default(),
            linkedin: LinkedInConfig::default(),
            image_gen: ImageGenConfig::default(),
            plugins: PluginsConfig::default(),
            locale: None,
            verifiable_intent: VerifiableIntentConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            claude_code_runner: ClaudeCodeRunnerConfig::default(),
            codex_cli: CodexCliConfig::default(),
            gemini_cli: GeminiCliConfig::default(),
            opencode_cli: OpenCodeCliConfig::default(),
            sop: SopConfig::default(),
            shell_tool: ShellToolConfig::default(),
            escalation: EscalationConfig::default(),
        }
    }
}

/// Resolve the current runtime config/workspace directories for onboarding flows.
///
/// This mirrors the same precedence used by `Config::load_or_init()`:
/// `OPERANT_CONFIG_DIR` > `OPERANT_WORKSPACE` > active workspace marker > defaults.
pub async fn resolve_runtime_dirs_for_onboarding() -> Result<(PathBuf, PathBuf)> {
    let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
    let (config_dir, workspace_dir, _) =
        resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;
    Ok((config_dir, workspace_dir))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigResolutionSource {
    EnvConfigDir,
    EnvWorkspace,
    ActiveWorkspaceMarker,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::EnvConfigDir => "OPERANT_CONFIG_DIR",
            Self::EnvWorkspace => "OPERANT_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

/// Parse the `OPERANT_EXTRA_HEADERS` environment variable value.
///
/// Format: `Key:Value,Key2:Value2`
///
/// Entries without a colon or with an empty key are silently skipped.
/// Leading/trailing whitespace on both key and value is trimmed.
pub fn parse_extra_headers_env(raw: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((key, value)) = entry.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                tracing::warn!("Ignoring extra header with empty name in OPERANT_EXTRA_HEADERS");
                continue;
            }
            result.push((key.to_string(), value.to_string()));
        } else {
            tracing::warn!("Ignoring malformed extra header entry (missing ':'): {entry}");
        }
    }
    result
}

pub(crate) fn read_codex_openai_api_key() -> Option<String> {
    let home = UserDirs::new()?.home_dir().to_path_buf();
    let auth_path = home.join(".codex").join("auth.json");
    let raw = std::fs::read_to_string(auth_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;

    parsed
        .get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Ensure that essential bootstrap files exist in the workspace directory.
///
/// When the workspace is created outside of `operant onboard` (e.g., non-tty
/// daemon/cron sessions), these files would otherwise be missing. This function
/// creates sensible defaults that allow the agent to operate with a basic identity.
pub(crate) async fn ensure_bootstrap_files(workspace_dir: &Path) -> Result<()> {
    let defaults: &[(&str, &str)] = &[
        (
            "IDENTITY.md",
            "# IDENTITY.md — Who Am I?\n\n\
             I am Operant, an autonomous AI agent.\n\n\
             ## Traits\n\
             - Helpful, precise, and safety-conscious\n\
             - I prioritize clarity and correctness\n",
        ),
        (
            "SOUL.md",
            "# SOUL.md — Who You Are\n\n\
             You are Operant, an autonomous AI agent.\n\n\
             ## Core Principles\n\
             - Be helpful and accurate\n\
             - Respect user intent and boundaries\n\
             - Ask before taking destructive actions\n\
             - Prefer safe, reversible operations\n",
        ),
    ];

    for (filename, content) in defaults {
        let path = workspace_dir.join(filename);
        if !path.exists() {
            fs::write(&path, content)
                .await
                .with_context(|| format!("Failed to create default {filename} in workspace"))?;
        }
    }

    Ok(())
}

impl Config {
    /// Collect the `IntegrationDescriptor` from every nested config that
    /// declares one via `#[integration(...)]`. Adding a new toggleable
    /// integration is one struct-level attribute on the new config + one
    /// row in this method. The integrations registry consumes the result
    /// without per-vendor branches.
    pub fn integration_descriptors(&self) -> Vec<crate::config::IntegrationDescriptor> {
        vec![
            self.browser.integration_descriptor(),
            self.cron.integration_descriptor(),
            self.google_workspace.integration_descriptor(),
        ]
    }

    /// Combine top-level `[cost.prices.<key>]` entries with any per-provider
    /// `pricing` entries declared on `[providers.models.<id>]`. Per-provider
    /// pricing is keyed as `<provider_id>/<model>` to align with the lookup
    /// pattern in `record_tool_loop_cost_usage` (qualified `<provider>/<model>`
    /// → bare `<model>` → suffix-after-last-slash). The qualified-first lookup
    /// order is what makes per-provider disambiguation actually take effect:
    /// an operator who sets `[providers.models.openai.pricing]` for `gpt-4o`
    /// gets that rate even if a generic `[cost.prices.gpt-4o]` is also set.
    /// Top-level entries still win on exact-key conflict so existing operator
    /// overrides keyed as `<provider>/<model>` are never silently shadowed.
    pub fn combined_pricing(&self) -> std::collections::HashMap<String, ModelPricing> {
        let mut combined = self.cost.prices.clone();
        for (provider_id, provider) in &self.providers.models {
            let (Some(pricing), Some(model)) = (&provider.pricing, &provider.model) else {
                continue;
            };
            if model.is_empty() {
                continue;
            }
            combined
                .entry(format!("{provider_id}/{model}"))
                .or_insert_with(|| pricing.clone());
        }
        combined
    }

    /// Return top-level TOML keys in `raw_toml` that Config does not recognise.
    ///
    /// Keys present in `Config::default()` serialization pass immediately.
    /// Remaining keys are probed: the key is deserialized in isolation and
    /// the result compared to the default — a changed output means serde
    /// consumed it (covers `Option<T>` fields and `#[serde(alias)]` names).
    /// V1 legacy keys (consumed by migration) are also accepted.
    pub fn unknown_keys(raw_toml: &str) -> Vec<String> {
        let raw: toml::Table = match raw_toml.parse() {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        static DEFAULTS: OnceLock<toml::Table> = OnceLock::new();
        let defaults = DEFAULTS.get_or_init(|| {
            toml::to_string(&Config::default())
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default()
        });
        raw.keys()
            .filter(|key| {
                if defaults.contains_key(key.as_str()) {
                    return false;
                }
                if crate::migration::V1_LEGACY_KEYS.contains(&key.as_str()) {
                    return false;
                }
                let mut t = toml::Table::new();
                t.insert((*key).clone(), raw[key.as_str()].clone());
                let consumed = toml::to_string(&t)
                    .ok()
                    .and_then(|s| toml::from_str::<Config>(&s).ok())
                    .and_then(|c| toml::to_string(&c).ok())
                    .and_then(|s| s.parse::<toml::Table>().ok())
                    .is_some_and(|t| t != *defaults);
                !consumed
            })
            .cloned()
            .collect()
    }

    /// Load the on-disk config, or initialize a fresh default config when none exists.
    pub async fn load_or_init() -> Result<Self> {
        let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;

        let (operant_dir, workspace_dir, resolution_source) =
            resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;

        let config_path = operant_dir.join("config.toml");

        fs::create_dir_all(&operant_dir)
            .await
            .with_context(|| config_dir_creation_error(&operant_dir))?;
        fs::create_dir_all(&workspace_dir)
            .await
            .context("Failed to create workspace directory")?;

        ensure_bootstrap_files(&workspace_dir).await?;

        if config_path.exists() {
            // Warn if config file is world-readable (may contain API keys)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(&config_path).await
                    && meta.permissions().mode() & 0o004 != 0
                {
                    tracing::warn!(
                        "Config file {:?} is world-readable (mode {:o}). \
                             Consider restricting with: chmod 600 {:?}",
                        config_path,
                        meta.permissions().mode() & 0o777,
                        config_path,
                    );
                }
            }

            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;

            // Deserialize the config with the standard TOML parser.
            //
            // Previously this used `serde_ignored::deserialize` for both
            // deserialization and unknown-key detection.  However,
            // `serde_ignored` silently drops field values inside nested
            // structs that carry `#[serde(default)]` (e.g. the entire
            // `[autonomy]` table), causing user-supplied values to be
            // replaced by defaults.  See #4171.
            //
            // We now deserialize with `toml::from_str` (which is correct)
            // and run `serde_ignored` separately just for diagnostics.
            //
            // Before deserialization, run `prepare_table` to handle nested
            // field migrations (e.g. room_id → allowed_rooms in matrix)
            // that `#[serde(flatten)]` cannot capture.
            let mut table: toml::Table =
                toml::from_str(&contents).context("Failed to parse config as TOML table")?;
            crate::migration::prepare_table(&mut table);
            let table_str =
                toml::to_string(&table).context("Failed to re-serialize prepared table")?;
            let compat: crate::migration::V1Compat =
                toml::from_str(&table_str).context("Failed to deserialize config file")?;
            let mut config: Config = compat.into_config();

            // Ensure the built-in default auto_approve entries are always
            // present.  When a user specifies `auto_approve` in their TOML
            // (e.g. to add a custom tool), serde replaces the default list
            // instead of merging.  This caused default-safe tools like
            // `weather` or `calculator` to lose their auto-approve status
            // and get silently denied in non-interactive channel runs.
            // See #4247.
            //
            // Users who want to require approval for a default tool can
            // add it to `always_ask`, which takes precedence over
            // `auto_approve` in the approval decision (see approval/mod.rs).
            config.autonomy.ensure_default_auto_approve();

            // Backward-compatible `enabled` backfill: if a channel section
            // exists in the TOML but has no explicit `enabled` key, the user
            // configured it before `enabled` was introduced — treat it as
            // enabled so existing setups don't silently break.
            config.channels.backfill_enabled(&contents);

            // Detect unknown top-level config keys by comparing the raw
            // TOML table keys against what Config actually deserializes.
            // This replaces the previous serde_ignored-based approach which
            // had false-positive issues with #[serde(default)] nested structs.
            for key in Self::unknown_keys(&contents) {
                tracing::warn!(
                    "Unknown config key ignored: \"{key}\". Check config.toml for typos or deprecated options.",
                );
            }
            // Set computed paths that are skipped during serialization
            config.config_path = config_path.clone();
            config.workspace_dir = workspace_dir;
            let store = crate::secrets::SecretStore::new(&operant_dir, config.secrets.encrypt);
            // Decrypt all #[secret]-annotated fields via Configurable derive
            config.decrypt_secrets(&store)?;

            config.apply_env_overrides();
            config.ensure_default_mcp_servers();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        } else {
            let mut config = Config {
                config_path: config_path.clone(),
                workspace_dir,
                ..Config::default()
            };
            config.save().await?;

            // Restrict permissions on newly created config file (may contain API keys)
            #[cfg(unix)]
            {
                use std::{fs::Permissions, os::unix::fs::PermissionsExt};
                let _ = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await;
            }

            config.apply_env_overrides();
            config.ensure_default_mcp_servers();
            config.validate()?;
            tracing::info!(
                path = %config.config_path.display(),
                workspace = %config.workspace_dir.display(),
                source = resolution_source.as_str(),
                initialized = true,
                "Config loaded"
            );
            Ok(config)
        }
    }

    /// Inject the agentmemory MCP server into `config.mcp.servers` when MCP
    /// is enabled (`mcp.enabled`), the memory backend selects agentmemory
    /// (`memory.backend == "agentmemory"`), and no `agentmemory` server is
    /// already configured.
    ///
    /// Mirrors `operant_core::config::ensure_default_mcp_servers` (the CLI/
    /// `AppConfig` path) so the runtime daemon / gateway / channels-orchestrator
    /// path — which loads `operant_config::schema::Config` — exposes the same
    /// native-MCP agentmemory surface. The schema's `mcp.deferred_loading`
    /// flag is global and defaults to `true`, so an appended server
    /// automatically joins the deferred toolset (`DeferredMcpToolSet` +
    /// `tool_search`) and never spawns `npx @agentmemory/mcp` at boot.
    ///
    /// The schema `[memory]` section has no agentmemory-specific URL/secret
    /// fields; the server reads `AGENTMEMORY_URL` from the environment and
    /// falls back to the agentmemory default (`http://localhost:3111`),
    /// mirroring the AppConfig default.
    pub fn ensure_default_mcp_servers(&mut self) {
        if !self.mcp.enabled {
            return;
        }
        // Schema-world equivalent of the AppConfig `provider == "agentmemory"`
        // trigger: the `[memory] backend` key selects agentmemory.
        if self.memory.backend != "agentmemory" {
            return;
        }
        if self.mcp.servers.iter().any(|s| s.name == "agentmemory") {
            return;
        }

        let url = std::env::var("AGENTMEMORY_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| "http://localhost:3111".to_string());
        let mut env = std::collections::HashMap::new();
        env.insert("AGENTMEMORY_URL".to_string(), url);
        if let Some(secret) = std::env::var("AGENTMEMORY_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())
        {
            env.insert("AGENTMEMORY_SECRET".to_string(), secret);
        }

        self.mcp.servers.push(McpServerConfig {
            name: "agentmemory".to_string(),
            transport: McpTransport::Stdio,
            url: None,
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@agentmemory/mcp".to_string()],
            env,
            headers: std::collections::HashMap::new(),
            tool_timeout_secs: None,
        });
    }

    fn lookup_model_provider_profile(
        &self,
        provider_name: &str,
    ) -> Option<(String, ModelProviderConfig)> {
        let needle = provider_name.trim();
        if needle.is_empty() {
            return None;
        }

        self.providers
            .models
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(needle))
            .map(|(name, profile)| (name.clone(), profile.clone()))
    }

    /// Apply Codex-app-server compatibility shims to the resolved fallback provider entry.
    ///
    /// Historically this method mutated `self.providers.fallback` to a "canonical" key
    /// derived from the profile's `name` field, `wire_api`, or `base_url`. That mutation
    /// caused two problems:
    ///
    /// 1. **CLI get/set divergence.** `Config::load_or_init` calls `apply_env_overrides`,
    ///    which calls this function. After load, `providers.fallback` no longer matched
    ///    what was on disk, so `operant config get providers.fallback` returned the
    ///    rewritten value while the file still had the user's literal value. The next
    ///    `save()` would then persist the rewrite, silently changing the user's config.
    /// 2. **Orphaned references.** When the rewrite pointed at a key that did not exist
    ///    in `providers.models` (e.g. profile had `name = "gemini"` but no
    ///    `[providers.models.gemini]` entry), runtime `fallback_provider()` lookups
    ///    returned `None` and downstream code fell through to a hardcoded default model.
    ///
    /// The fix: keep `self.providers.fallback` as the literal user-supplied key.
    /// Propagate the profile's `base_url` / `api_path` / `max_tokens` / `api_key` onto the
    /// resolved entry as before, and mirror the entry under any canonical alias keys so
    /// runtime lookups by either name still resolve. The user's `[providers] fallback`
    /// value is preserved end-to-end through load → save → load.
    fn apply_named_model_provider_profile(&mut self) {
        let Some(current_provider) = self.providers.fallback.clone() else {
            return;
        };

        let Some((profile_key, profile)) = self.lookup_model_provider_profile(&current_provider)
        else {
            return;
        };

        let base_url = profile
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);

        {
            let fallback_provider = self.providers.fallback_provider();
            let current_url = fallback_provider
                .and_then(|e| e.base_url.as_deref())
                .map(str::trim);
            if current_url.is_none_or(|value| value.is_empty())
                && let Some(base_url) = base_url.as_ref()
                && let Some(entry) = self.providers.fallback_provider_mut()
            {
                entry.base_url = Some(base_url.clone());
            }
        }

        // Propagate api_path from the profile when not already set on fallback entry.
        {
            let has_api_path = self
                .providers
                .fallback_provider()
                .and_then(|e| e.api_path.as_ref())
                .is_some();
            if !has_api_path && let Some(ref path) = profile.api_path {
                let trimmed = path.trim();
                if !trimmed.is_empty()
                    && let Some(entry) = self.providers.fallback_provider_mut()
                {
                    entry.api_path = Some(trimmed.to_string());
                }
            }
        }

        // Propagate max_tokens from the profile when not already set on fallback entry.
        {
            let has_max_tokens = self
                .providers
                .fallback_provider()
                .and_then(|e| e.max_tokens)
                .is_some();
            if !has_max_tokens
                && let Some(max_tokens) = profile.max_tokens
                && let Some(entry) = self.providers.fallback_provider_mut()
            {
                entry.max_tokens = Some(max_tokens);
            }
        }

        if profile.requires_openai_auth {
            let needs_key = self
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref())
                .map(str::trim)
                .is_none_or(|value| value.is_empty());
            if needs_key {
                let codex_key = std::env::var("OPENAI_API_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .or_else(read_codex_openai_api_key);
                if let Some(codex_key) = codex_key
                    && let Some(entry) = self.providers.fallback_provider_mut()
                {
                    entry.api_key = Some(codex_key);
                }
            }
        }

        let normalized_wire_api = profile.wire_api.as_deref().and_then(normalize_wire_api);
        let profile_name = profile
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        // Mirror the resolved entry under any canonical alias keys so that runtime
        // lookups by the profile-implied name (e.g. wire_api → "openai-codex",
        // explicit `name = ...`, or `custom:<base_url>`) also resolve. We do NOT
        // rewrite `providers.fallback` itself: that is the user's literal config
        // value and must round-trip cleanly through CLI get/set/save.
        let mut alias_keys: Vec<String> = Vec::new();
        if normalized_wire_api == Some("responses") {
            alias_keys.push("openai-codex".to_string());
        }
        if let Some(profile_name) = profile_name
            && !profile_name.eq_ignore_ascii_case(&profile_key)
        {
            alias_keys.push(profile_name.to_string());
        }
        if let Some(ref base_url) = base_url {
            alias_keys.push(format!("custom:{base_url}"));
        }

        for alias in alias_keys {
            if !self.providers.models.contains_key(&alias)
                && let Some(entry) = self.providers.models.get(&profile_key).cloned()
            {
                self.providers.models.insert(alias, entry);
            }
        }
    }

    /// Collect non-fatal validation warnings — config that loads and
    /// validates successfully (`validate()` returns `Ok(())`) but will fail
    /// at runtime because of a logical inconsistency the schema cannot
    /// enforce structurally.
    ///
    /// Called by `validate()` (which emits each warning via `tracing::warn!`
    /// for log visibility) and by the gateway HTTP API (which returns the
    /// structured list in `PropResponse` / `PatchResponse` so dashboard
    /// callers see the same signal the CLI sees on stderr).
    ///
    /// Adding a new warning: append a check here, pick a stable `code`,
    /// and document the code in `validation_warnings.rs`.
    pub fn collect_warnings(&self) -> Vec<crate::validation_warnings::ValidationWarning> {
        use crate::validation_warnings::ValidationWarning;
        let mut warnings = Vec::new();

        // providers.fallback references a key not present in providers.models
        if let Some(ref fallback_key) = self.providers.fallback
            && !self.providers.models.contains_key(fallback_key)
        {
            warnings.push(ValidationWarning::new(
                "dangling_provider_fallback",
                format!(
                    "providers.fallback references '{fallback_key}' which does not exist in providers.models; provider resolution will fail at runtime"
                ),
                "providers.fallback",
            ));
        }

        warnings
    }

    /// Validate configuration values that would cause runtime failures.
    ///
    /// Called after TOML deserialization and env-override application to catch
    /// obviously invalid values early instead of failing at arbitrary runtime points.
    pub fn validate(&self) -> Result<()> {
        // Tunnel — OpenVPN
        if self.tunnel.provider.trim() == "openvpn" {
            let openvpn = self.tunnel.openvpn.as_ref().ok_or_else(|| {
                anyhow::anyhow!("tunnel.provider='openvpn' requires [tunnel.openvpn]")
            })?;

            if openvpn.config_file.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "tunnel.openvpn.config_file",
                    "tunnel.openvpn.config_file must not be empty"
                );
            }
            if openvpn.connect_timeout_secs == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "tunnel.openvpn.connect_timeout_secs",
                    "tunnel.openvpn.connect_timeout_secs must be greater than 0"
                );
            }
        }

        // Gateway
        if self.gateway.host.trim().is_empty() {
            validation_bail!(
                RequiredFieldEmpty,
                "gateway.host",
                "gateway.host must not be empty"
            );
        }
        if let Some(ref prefix) = self.gateway.path_prefix {
            // Validate the raw value — no silent trimming so the stored
            // value is exactly what was validated.
            if !prefix.is_empty() {
                if !prefix.starts_with('/') {
                    validation_bail!(
                        InvalidFormat,
                        "gateway.path_prefix",
                        "gateway.path_prefix must start with '/'"
                    );
                }
                if prefix.ends_with('/') {
                    validation_bail!(
                        InvalidFormat,
                        "gateway.path_prefix",
                        "gateway.path_prefix must not end with '/' (including bare '/')"
                    );
                }
                // Reject characters unsafe for URL paths or HTML/JS injection.
                // Whitespace is intentionally excluded from the allowed set.
                if let Some(bad) = prefix.chars().find(|c| {
                    !matches!(c, '/' | '-' | '_' | '.' | '~'
                        | 'a'..='z' | 'A'..='Z' | '0'..='9'
                        | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
                        | ':' | '@')
                }) {
                    anyhow::bail!(
                        "gateway.path_prefix contains invalid character '{bad}'; \
                         only unreserved and sub-delim URI characters are allowed"
                    );
                }
            }
        }

        // Autonomy
        if self.autonomy.max_actions_per_hour == 0 {
            validation_bail!(
                InvalidNumericRange,
                "autonomy.max_actions_per_hour",
                "autonomy.max_actions_per_hour must be greater than 0"
            );
        }
        for (i, env_name) in self.autonomy.shell_env_passthrough.iter().enumerate() {
            if !is_valid_env_var_name(env_name) {
                anyhow::bail!(
                    "autonomy.shell_env_passthrough[{i}] is invalid ({env_name}); expected [A-Za-z_][A-Za-z0-9_]*"
                );
            }
        }

        // Security OTP / estop
        if self.security.otp.challenge_max_attempts == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.challenge_max_attempts",
                "security.otp.challenge_max_attempts must be greater than 0"
            );
        }
        if self.security.otp.token_ttl_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.token_ttl_secs",
                "security.otp.token_ttl_secs must be greater than 0"
            );
        }
        if self.security.otp.cache_valid_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.cache_valid_secs",
                "security.otp.cache_valid_secs must be greater than 0"
            );
        }
        if self.security.otp.cache_valid_secs < self.security.otp.token_ttl_secs {
            anyhow::bail!(
                "security.otp.cache_valid_secs must be greater than or equal to security.otp.token_ttl_secs"
            );
        }
        if self.security.otp.challenge_max_attempts == 0 {
            validation_bail!(
                InvalidNumericRange,
                "security.otp.challenge_max_attempts",
                "security.otp.challenge_max_attempts must be greater than 0"
            );
        }
        for (i, action) in self.security.otp.gated_actions.iter().enumerate() {
            let normalized = action.trim();
            if normalized.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("security.otp.gated_actions[{i}]"),
                    "security.otp.gated_actions[{i}] must not be empty"
                );
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "security.otp.gated_actions[{i}] contains invalid characters: {normalized}"
                );
            }
        }
        DomainMatcher::new(
            &self.security.otp.gated_domains,
            &self.security.otp.gated_domain_categories,
        )
        .with_context(
            || "Invalid security.otp.gated_domains or security.otp.gated_domain_categories",
        )?;
        if self.security.estop.state_file.trim().is_empty() {
            validation_bail!(
                RequiredFieldEmpty,
                "security.estop.state_file",
                "security.estop.state_file must not be empty"
            );
        }

        // Scheduler
        if self.scheduler.max_concurrent == 0 {
            validation_bail!(
                InvalidNumericRange,
                "scheduler.max_concurrent",
                "scheduler.max_concurrent must be greater than 0"
            );
        }
        if self.scheduler.max_tasks == 0 {
            validation_bail!(
                InvalidNumericRange,
                "scheduler.max_tasks",
                "scheduler.max_tasks must be greater than 0"
            );
        }

        // Model routes
        for (i, route) in self.providers.model_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].hint"),
                    "model_routes[{i}].hint must not be empty"
                );
            }
            if route.provider.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].provider"),
                    "model_routes[{i}].provider must not be empty"
                );
            }
            if route.model.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("model_routes[{i}].model"),
                    "model_routes[{i}].model must not be empty"
                );
            }
        }

        // Embedding routes
        for (i, route) in self.providers.embedding_routes.iter().enumerate() {
            if route.hint.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].hint"),
                    "embedding_routes[{i}].hint must not be empty"
                );
            }
            if route.provider.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].provider"),
                    "embedding_routes[{i}].provider must not be empty"
                );
            }
            if route.model.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("embedding_routes[{i}].model"),
                    "embedding_routes[{i}].model must not be empty"
                );
            }
        }

        for (profile_key, profile) in &self.providers.models {
            let profile_name = profile_key.trim();
            if profile_name.is_empty() {
                anyhow::bail!("model_providers contains an empty profile name");
            }

            let has_name = profile
                .name
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            let has_base_url = profile
                .base_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());

            // Entries created by migration from top-level fields use the provider
            // name as the map key and may not have explicit `name` or `base_url`
            // (the provider factory resolves known names). An entry with no
            // identifying information at all is almost always an in-progress
            // onboarding state — the user picked the provider but hasn't filled
            // anything in yet. Warn but don't bail; the runtime falls back to
            // provider-trait defaults at use time, and a chat against the
            // unconfigured provider fails with a clear error then.
            let has_api_key = profile
                .api_key
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty());
            let has_model = profile
                .model
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty());
            if !has_name && !has_base_url && !has_api_key && !has_model {
                tracing::warn!(
                    provider = %profile_name,
                    "providers.models.{profile_name} is empty (no name / base_url / api_key / model). \
                     Skipping at runtime; finish onboarding via the dashboard or `operant onboard` \
                     to make this provider usable.",
                );
                continue;
            }

            if let Some(base_url) = profile.base_url.as_deref().map(str::trim)
                && !base_url.is_empty()
            {
                let parsed = reqwest::Url::parse(base_url).with_context(|| {
                    format!("model_providers.{profile_name}.base_url is not a valid URL")
                })?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    anyhow::bail!("model_providers.{profile_name}.base_url must use http/https");
                }
            }

            if let Some(wire_api) = profile.wire_api.as_deref().map(str::trim)
                && !wire_api.is_empty()
                && normalize_wire_api(wire_api).is_none()
            {
                anyhow::bail!(
                    "model_providers.{profile_name}.wire_api must be one of: responses, chat_completions"
                );
            }

            if let Some(temp) = profile.temperature {
                validate_temperature(temp).map_err(|e| {
                    anyhow::anyhow!("providers.models.{profile_name}.temperature: {e}")
                })?;
            }
        }

        // Non-fatal validation warnings: surfaced both via tracing (CLI sees
        // on stderr) and via Config::collect_warnings (gateway HTTP returns
        // structured to dashboard callers). Single source of truth lives in
        // collect_warnings; emit each one to tracing here so the existing
        // log behavior is preserved.
        for w in self.collect_warnings() {
            tracing::warn!(path = %w.path, code = %w.code, "{}", w.message);
        }

        // Ollama cloud-routing safety checks
        if self
            .providers
            .fallback
            .as_deref()
            .is_some_and(|provider| provider.trim().eq_ignore_ascii_case("ollama"))
            && self
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref())
                .is_some_and(|model| model.trim().ends_with(":cloud"))
        {
            if is_local_ollama_endpoint(
                self.providers
                    .fallback_provider()
                    .and_then(|e| e.base_url.as_deref()),
            ) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset. Set api_url to a remote Ollama endpoint (for example https://ollama.com)."
                );
            }

            if !has_ollama_cloud_credential(
                self.providers
                    .fallback_provider()
                    .and_then(|e| e.api_key.as_deref()),
            ) {
                anyhow::bail!(
                    "default_model uses ':cloud' with provider 'ollama', but no API key is configured. Set api_key or OLLAMA_API_KEY."
                );
            }
        }

        // Microsoft 365
        if self.microsoft365.enabled {
            let tenant = self
                .microsoft365
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if tenant.is_none() {
                anyhow::bail!(
                    "microsoft365.tenant_id must not be empty when microsoft365 is enabled"
                );
            }
            let client = self
                .microsoft365
                .client_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if client.is_none() {
                anyhow::bail!(
                    "microsoft365.client_id must not be empty when microsoft365 is enabled"
                );
            }
            let flow = self.microsoft365.auth_flow.trim();
            if flow != "client_credentials" && flow != "device_code" {
                anyhow::bail!(
                    "microsoft365.auth_flow must be 'client_credentials' or 'device_code'"
                );
            }
            if flow == "client_credentials"
                && self
                    .microsoft365
                    .client_secret
                    .as_deref()
                    .is_none_or(|s| s.trim().is_empty())
            {
                anyhow::bail!(
                    "microsoft365.client_secret must not be empty when auth_flow is 'client_credentials'"
                );
            }
        }

        // Microsoft 365
        if self.microsoft365.enabled {
            let tenant = self
                .microsoft365
                .tenant_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if tenant.is_none() {
                anyhow::bail!(
                    "microsoft365.tenant_id must not be empty when microsoft365 is enabled"
                );
            }
            let client = self
                .microsoft365
                .client_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if client.is_none() {
                anyhow::bail!(
                    "microsoft365.client_id must not be empty when microsoft365 is enabled"
                );
            }
            let flow = self.microsoft365.auth_flow.trim();
            if flow != "client_credentials" && flow != "device_code" {
                anyhow::bail!("microsoft365.auth_flow must be client_credentials or device_code");
            }
            if flow == "client_credentials"
                && self
                    .microsoft365
                    .client_secret
                    .as_deref()
                    .is_none_or(|s| s.trim().is_empty())
            {
                anyhow::bail!(
                    "microsoft365.client_secret must not be empty when auth_flow is client_credentials"
                );
            }
        }

        // MCP
        if self.mcp.enabled {
            validate_mcp_config(&self.mcp)?;
        }

        // Knowledge graph
        if self.knowledge.enabled {
            if self.knowledge.max_nodes == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "knowledge.max_nodes",
                    "knowledge.max_nodes must be greater than 0"
                );
            }
            if self.knowledge.db_path.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "knowledge.db_path",
                    "knowledge.db_path must not be empty"
                );
            }
        }

        // Google Workspace allowed_services validation
        let mut seen_gws_services = std::collections::HashSet::new();
        for (i, service) in self.google_workspace.allowed_services.iter().enumerate() {
            let normalized = service.trim();
            if normalized.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_services[{i}]"),
                    "google_workspace.allowed_services[{i}] must not be empty"
                );
            }
            if !normalized
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_services[{i}] contains invalid characters: {normalized}"
                );
            }
            if !seen_gws_services.insert(normalized.to_string()) {
                anyhow::bail!(
                    "google_workspace.allowed_services contains duplicate entry: {normalized}"
                );
            }
        }

        // Build the effective allowed-services set for cross-validation.
        // When the operator leaves allowed_services empty the tool falls back to
        // DEFAULT_GWS_SERVICES; use the same constant here so validation is
        // consistent in both cases.
        let effective_services: std::collections::HashSet<&str> =
            if self.google_workspace.allowed_services.is_empty() {
                DEFAULT_GWS_SERVICES.iter().copied().collect()
            } else {
                self.google_workspace
                    .allowed_services
                    .iter()
                    .map(|s| s.trim())
                    .collect()
            };

        let mut seen_gws_operations = std::collections::HashSet::new();
        for (i, operation) in self.google_workspace.allowed_operations.iter().enumerate() {
            let service = operation.service.trim();
            let resource = operation.resource.trim();

            if service.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_operations[{i}].service"),
                    "google_workspace.allowed_operations[{i}].service must not be empty"
                );
            }
            if resource.is_empty() {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].resource must not be empty"
                );
            }

            if !effective_services.contains(service) {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].service '{service}' is not in the \
                     effective allowed_services; this entry can never match at runtime"
                );
            }
            if !service
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].service contains invalid characters: {service}"
                );
            }
            if !resource
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            {
                anyhow::bail!(
                    "google_workspace.allowed_operations[{i}].resource contains invalid characters: {resource}"
                );
            }

            if let Some(ref sub_resource) = operation.sub_resource {
                let sub = sub_resource.trim();
                if sub.is_empty() {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].sub_resource must not be empty when present"
                    );
                }
                if !sub
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
                {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].sub_resource contains invalid characters: {sub}"
                    );
                }
            }

            if operation.methods.is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    format!("google_workspace.allowed_operations[{i}].methods"),
                    "google_workspace.allowed_operations[{i}].methods must not be empty"
                );
            }

            let mut seen_methods = std::collections::HashSet::new();
            for (j, method) in operation.methods.iter().enumerate() {
                let normalized = method.trim();
                if normalized.is_empty() {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods[{j}] must not be empty"
                    );
                }
                if !normalized
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
                {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods[{j}] contains invalid characters: {normalized}"
                    );
                }
                if !seen_methods.insert(normalized.to_string()) {
                    anyhow::bail!(
                        "google_workspace.allowed_operations[{i}].methods contains duplicate entry: {normalized}"
                    );
                }
            }

            let sub_key = operation
                .sub_resource
                .as_deref()
                .map(str::trim)
                .unwrap_or("");
            let operation_key = format!("{service}:{resource}:{sub_key}");
            if !seen_gws_operations.insert(operation_key.clone()) {
                anyhow::bail!(
                    "google_workspace.allowed_operations contains duplicate service/resource/sub_resource entry: {operation_key}"
                );
            }
        }

        // Project intelligence
        if self.project_intel.enabled {
            let lang = &self.project_intel.default_language;
            if !["en", "de", "fr", "it"].contains(&lang.as_str()) {
                anyhow::bail!(
                    "project_intel.default_language must be one of: en, de, fr, it (got '{lang}')"
                );
            }
            let sens = &self.project_intel.risk_sensitivity;
            if !["low", "medium", "high"].contains(&sens.as_str()) {
                anyhow::bail!(
                    "project_intel.risk_sensitivity must be one of: low, medium, high (got '{sens}')"
                );
            }
            if let Some(ref tpl_dir) = self.project_intel.templates_dir
                && !std::path::Path::new(tpl_dir).exists()
            {
                anyhow::bail!("project_intel.templates_dir path does not exist: {tpl_dir}");
            }
        }

        // Proxy (delegate to existing validation)
        self.proxy.validate()?;
        self.cloud_ops.validate()?;

        // Notion
        if self.notion.enabled {
            if self.notion.database_id.trim().is_empty() {
                anyhow::bail!("notion.database_id must not be empty when notion.enabled = true");
            }
            if self.notion.poll_interval_secs == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "notion.poll_interval_secs",
                    "notion.poll_interval_secs must be greater than 0"
                );
            }
            if self.notion.max_concurrent == 0 {
                validation_bail!(
                    InvalidNumericRange,
                    "notion.max_concurrent",
                    "notion.max_concurrent must be greater than 0"
                );
            }
            if self.notion.status_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.status_property",
                    "notion.status_property must not be empty"
                );
            }
            if self.notion.input_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.input_property",
                    "notion.input_property must not be empty"
                );
            }
            if self.notion.result_property.trim().is_empty() {
                validation_bail!(
                    RequiredFieldEmpty,
                    "notion.result_property",
                    "notion.result_property must not be empty"
                );
            }
        }

        // Pinggy tunnel region — validate allowed values (case-insensitive, auto-lowercased at runtime).
        if let Some(ref pinggy) = self.tunnel.pinggy
            && let Some(ref region) = pinggy.region
        {
            let r = region.trim().to_ascii_lowercase();
            if !r.is_empty() && !matches!(r.as_str(), "us" | "eu" | "ap" | "br" | "au") {
                anyhow::bail!(
                    "tunnel.pinggy.region must be one of: us, eu, ap, br, au (or omitted for auto)"
                );
            }
        }

        // Jira
        if self.jira.enabled {
            if self.jira.base_url.trim().is_empty() {
                anyhow::bail!("jira.base_url must not be empty when jira.enabled = true");
            }
            if self.jira.api_token.trim().is_empty()
                && std::env::var("JIRA_API_TOKEN")
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
            {
                anyhow::bail!(
                    "jira.api_token must be set (or JIRA_API_TOKEN env var) when jira.enabled = true"
                );
            }
            let valid_actions = ["get_ticket", "search_tickets", "comment_ticket"];
            for action in &self.jira.allowed_actions {
                if !valid_actions.contains(&action.as_str()) {
                    anyhow::bail!(
                        "jira.allowed_actions contains unknown action: '{}'. \
                         Valid: get_ticket, search_tickets, comment_ticket",
                        action
                    );
                }
            }
        }

        // Nevis IAM — delegate to NevisConfig::validate() for field-level checks
        if let Err(msg) = self.security.nevis.validate() {
            anyhow::bail!("security.nevis: {msg}");
        }

        // Delegate agent timeouts
        const MAX_DELEGATE_TIMEOUT_SECS: u64 = 3600;
        for (name, agent) in &self.agents {
            if let Some(timeout) = agent.timeout_secs {
                if timeout == 0 {
                    validation_bail!(
                        InvalidNumericRange,
                        format!("agents.{name}.timeout_secs"),
                        "agents.{name}.timeout_secs must be greater than 0"
                    );
                }
                if timeout > MAX_DELEGATE_TIMEOUT_SECS {
                    anyhow::bail!(
                        "agents.{name}.timeout_secs exceeds max {MAX_DELEGATE_TIMEOUT_SECS}"
                    );
                }
            }
            if let Some(timeout) = agent.agentic_timeout_secs {
                if timeout == 0 {
                    validation_bail!(
                        InvalidNumericRange,
                        format!("agents.{name}.agentic_timeout_secs"),
                        "agents.{name}.agentic_timeout_secs must be greater than 0"
                    );
                }
                if timeout > MAX_DELEGATE_TIMEOUT_SECS {
                    anyhow::bail!(
                        "agents.{name}.agentic_timeout_secs exceeds max {MAX_DELEGATE_TIMEOUT_SECS}"
                    );
                }
            }
        }

        // Transcription
        {
            let dp = self.transcription.default_provider.trim();
            match dp {
                "groq" | "openai" | "deepgram" | "assemblyai" | "google" | "local_whisper" => {}
                other => {
                    anyhow::bail!(
                        "transcription.default_provider must be one of: groq, openai, deepgram, assemblyai, google, local_whisper (got '{other}')"
                    );
                }
            }
        }

        // Delegate tool global defaults
        if self.delegate.timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "delegate.timeout_secs",
                "delegate.timeout_secs must be greater than 0"
            );
        }
        if self.delegate.agentic_timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "delegate.agentic_timeout_secs",
                "delegate.agentic_timeout_secs must be greater than 0"
            );
        }

        // Per-agent delegate timeout overrides
        for (name, agent) in &self.agents {
            if let Some(t) = agent.timeout_secs
                && t == 0
            {
                validation_bail!(
                    InvalidNumericRange,
                    format!("agents.{name}.timeout_secs"),
                    "agents.{name}.timeout_secs must be greater than 0"
                );
            }
            if let Some(t) = agent.agentic_timeout_secs
                && t == 0
            {
                validation_bail!(
                    InvalidNumericRange,
                    format!("agents.{name}.agentic_timeout_secs"),
                    "agents.{name}.agentic_timeout_secs must be greater than 0"
                );
            }
        }

        // Channel reply-intent precheck. Zero timeout or empty/whitespace model would
        // silently fail open to REPLY and quietly disable the group-chat noise filter
        // — reject the typo cases explicitly. Use `enabled = false` to disable instead.
        if self.agent.precheck.timeout_secs == 0 {
            validation_bail!(
                InvalidNumericRange,
                "agent.precheck.timeout_secs",
                "agent.precheck.timeout_secs must be greater than 0 (use agent.precheck.enabled = false to disable the precheck)"
            );
        }
        if let Some(ref model) = self.agent.precheck.model
            && model.trim().is_empty()
        {
            validation_bail!(
                RequiredFieldEmpty,
                "agent.precheck.model",
                "agent.precheck.model must not be empty or whitespace; omit the key to fall back to the route model"
            );
        }

        Ok(())
    }

    /// Ensure the fallback provider entry exists, creating it if necessary.
    pub fn ensure_fallback_provider(&mut self) -> &mut ModelProviderConfig {
        let fallback = self
            .providers
            .fallback
            .clone()
            .unwrap_or_else(|| "default".into());
        if self.providers.fallback.is_none() {
            self.providers.fallback = Some(fallback.clone());
        }
        self.providers.models.entry(fallback).or_default()
    }

    /// Apply environment variable overrides to config
    pub fn apply_env_overrides(&mut self) {
        // API Key: OPERANT_API_KEY or API_KEY (generic)
        if let Ok(key) = std::env::var("OPERANT_API_KEY").or_else(|_| std::env::var("API_KEY"))
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }
        // API Key: GLM_API_KEY overrides when provider is a GLM/Zhipu variant.
        if self.providers.fallback.as_deref().is_some_and(is_glm_alias)
            && let Ok(key) = std::env::var("GLM_API_KEY")
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }

        // API Key: ZAI_API_KEY overrides when provider is a Z.AI variant.
        if self.providers.fallback.as_deref().is_some_and(is_zai_alias)
            && let Ok(key) = std::env::var("ZAI_API_KEY")
            && !key.is_empty()
        {
            self.ensure_fallback_provider().api_key = Some(key);
        }

        // Provider override precedence:
        // 1) OPERANT_PROVIDER always wins when set.
        // 2) OPERANT_MODEL_PROVIDER/MODEL_PROVIDER (Codex app-server style).
        // 3) Legacy PROVIDER is honored only when config still uses default provider.
        if let Ok(provider) = std::env::var("OPERANT_PROVIDER")
            && !provider.is_empty()
        {
            self.providers.fallback = Some(provider);
        } else if let Ok(provider) =
            std::env::var("OPERANT_MODEL_PROVIDER").or_else(|_| std::env::var("MODEL_PROVIDER"))
            && !provider.is_empty()
        {
            self.providers.fallback = Some(provider);
        } else if let Ok(provider) = std::env::var("PROVIDER") {
            let should_apply_legacy_provider = self
                .providers
                .fallback
                .as_deref()
                .is_none_or(|configured| configured.trim().eq_ignore_ascii_case("openrouter"));
            if should_apply_legacy_provider && !provider.is_empty() {
                self.providers.fallback = Some(provider);
            }
        }

        // Model: OPERANT_MODEL or MODEL
        if let Ok(model) = std::env::var("OPERANT_MODEL").or_else(|_| std::env::var("MODEL"))
            && !model.is_empty()
        {
            self.ensure_fallback_provider().model = Some(model);
        }

        // Provider HTTP timeout: OPERANT_PROVIDER_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_PROVIDER_TIMEOUT_SECS")
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.ensure_fallback_provider().timeout_secs = Some(timeout_secs);
        }

        // Extra provider headers: OPERANT_EXTRA_HEADERS
        // Format: "Key:Value,Key2:Value2"
        // Env var headers override config file headers with the same name.
        if let Ok(raw) = std::env::var("OPERANT_EXTRA_HEADERS") {
            let entry = self.ensure_fallback_provider();
            for header in parse_extra_headers_env(&raw) {
                entry.extra_headers.insert(header.0, header.1);
            }
        }

        // Apply named provider profile remapping (Codex app-server compatibility).
        self.apply_named_model_provider_profile();

        // Workspace directory: OPERANT_WORKSPACE
        if let Ok(workspace) = std::env::var("OPERANT_WORKSPACE")
            && !workspace.is_empty()
        {
            let expanded = expand_tilde_path(&workspace);
            let (_, workspace_dir) = resolve_config_dir_for_workspace(&expanded);
            self.workspace_dir = workspace_dir;
        }

        // Open-skills opt-in flag: OPERANT_OPEN_SKILLS_ENABLED
        if let Ok(flag) = std::env::var("OPERANT_OPEN_SKILLS_ENABLED")
            && !flag.trim().is_empty()
        {
            match flag.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.skills.open_skills_enabled = true,
                "0" | "false" | "no" | "off" => self.skills.open_skills_enabled = false,
                _ => tracing::warn!(
                    "Ignoring invalid OPERANT_OPEN_SKILLS_ENABLED (valid: 1|0|true|false|yes|no|on|off)"
                ),
            }
        }

        // Open-skills directory override: OPERANT_OPEN_SKILLS_DIR
        if let Ok(path) = std::env::var("OPERANT_OPEN_SKILLS_DIR") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.skills.open_skills_dir = Some(trimmed.to_string());
            }
        }

        // Skills script-file audit override: OPERANT_SKILLS_ALLOW_SCRIPTS
        if let Ok(flag) = std::env::var("OPERANT_SKILLS_ALLOW_SCRIPTS")
            && !flag.trim().is_empty()
        {
            match flag.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => self.skills.allow_scripts = true,
                "0" | "false" | "no" | "off" => self.skills.allow_scripts = false,
                _ => tracing::warn!(
                    "Ignoring invalid OPERANT_SKILLS_ALLOW_SCRIPTS (valid: 1|0|true|false|yes|no|on|off)"
                ),
            }
        }

        // Skills prompt mode override: OPERANT_SKILLS_PROMPT_MODE
        if let Ok(mode) = std::env::var("OPERANT_SKILLS_PROMPT_MODE")
            && !mode.trim().is_empty()
        {
            if let Some(parsed) = parse_skills_prompt_injection_mode(&mode) {
                self.skills.prompt_injection_mode = parsed;
            } else {
                tracing::warn!("Ignoring invalid OPERANT_SKILLS_PROMPT_MODE (valid: full|compact)");
            }
        }

        // Gateway port: OPERANT_GATEWAY_PORT or PORT
        if let Ok(port_str) =
            std::env::var("OPERANT_GATEWAY_PORT").or_else(|_| std::env::var("PORT"))
            && let Ok(port) = port_str.parse::<u16>()
        {
            self.gateway.port = port;
        }

        // Gateway host: OPERANT_GATEWAY_HOST or HOST
        if let Ok(host) = std::env::var("OPERANT_GATEWAY_HOST").or_else(|_| std::env::var("HOST"))
            && !host.is_empty()
        {
            self.gateway.host = host;
        }

        // Allow public bind: OPERANT_ALLOW_PUBLIC_BIND
        if let Ok(val) = std::env::var("OPERANT_ALLOW_PUBLIC_BIND") {
            self.gateway.allow_public_bind = val == "1" || val.eq_ignore_ascii_case("true");
        }

        // Require pairing: OPERANT_REQUIRE_PAIRING
        if let Ok(val) = std::env::var("OPERANT_REQUIRE_PAIRING") {
            self.gateway.require_pairing = val == "1" || val.eq_ignore_ascii_case("true");
        }

        // Web dist dir: OPERANT_WEB_DIST_DIR
        if let Ok(path) = std::env::var("OPERANT_WEB_DIST_DIR") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                self.gateway.web_dist_dir = Some(trimmed.to_string());
            }
        }

        // Temperature: OPERANT_TEMPERATURE
        if let Ok(temp_str) = std::env::var("OPERANT_TEMPERATURE") {
            match temp_str.parse::<f64>() {
                Ok(temp) if TEMPERATURE_RANGE.contains(&temp) => {
                    self.ensure_fallback_provider().temperature = Some(temp);
                }
                Ok(temp) => {
                    tracing::warn!(
                        "Ignoring OPERANT_TEMPERATURE={temp}: \
                         value out of range (expected {}..={})",
                        TEMPERATURE_RANGE.start(),
                        TEMPERATURE_RANGE.end()
                    );
                }
                Err(_) => {
                    tracing::warn!("Ignoring OPERANT_TEMPERATURE={temp_str:?}: not a valid number");
                }
            }
        }

        // Reasoning override: OPERANT_REASONING_ENABLED or REASONING_ENABLED
        if let Ok(flag) = std::env::var("OPERANT_REASONING_ENABLED")
            .or_else(|_| std::env::var("REASONING_ENABLED"))
        {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.runtime.reasoning_enabled = Some(true),
                "0" | "false" | "no" | "off" => self.runtime.reasoning_enabled = Some(false),
                _ => {}
            }
        }

        if let Ok(raw) = std::env::var("OPERANT_REASONING_EFFORT")
            .or_else(|_| std::env::var("REASONING_EFFORT"))
            .or_else(|_| std::env::var("OPERANT_CODEX_REASONING_EFFORT"))
        {
            match normalize_reasoning_effort(&raw) {
                Ok(effort) => self.runtime.reasoning_effort = Some(effort),
                Err(message) => tracing::warn!("Ignoring reasoning effort env override: {message}"),
            }
        }

        // Web search enabled: OPERANT_WEB_SEARCH_ENABLED or WEB_SEARCH_ENABLED
        if let Ok(enabled) = std::env::var("OPERANT_WEB_SEARCH_ENABLED")
            .or_else(|_| std::env::var("WEB_SEARCH_ENABLED"))
        {
            self.web_search.enabled = enabled == "1" || enabled.eq_ignore_ascii_case("true");
        }

        // Web search provider: OPERANT_WEB_SEARCH_PROVIDER or WEB_SEARCH_PROVIDER
        if let Ok(provider) = std::env::var("OPERANT_WEB_SEARCH_PROVIDER")
            .or_else(|_| std::env::var("WEB_SEARCH_PROVIDER"))
        {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.web_search.provider = provider.to_string();
            }
        }

        // Brave API key: OPERANT_BRAVE_API_KEY or BRAVE_API_KEY
        if let Ok(api_key) =
            std::env::var("OPERANT_BRAVE_API_KEY").or_else(|_| std::env::var("BRAVE_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.brave_api_key = Some(api_key.to_string());
            }
        }

        // Tavily API key: OPERANT_TAVILY_API_KEY or TAVILY_API_KEY
        if let Ok(api_key) =
            std::env::var("OPERANT_TAVILY_API_KEY").or_else(|_| std::env::var("TAVILY_API_KEY"))
        {
            let api_key = api_key.trim();
            if !api_key.is_empty() {
                self.web_search.tavily_api_key = Some(api_key.to_string());
            }
        }

        // SearXNG instance URL: OPERANT_SEARXNG_INSTANCE_URL or SEARXNG_INSTANCE_URL
        if let Ok(instance_url) = std::env::var("OPERANT_SEARXNG_INSTANCE_URL")
            .or_else(|_| std::env::var("SEARXNG_INSTANCE_URL"))
        {
            let instance_url = instance_url.trim();
            if !instance_url.is_empty() {
                self.web_search.searxng_instance_url = Some(instance_url.to_string());
            }
        }

        // Web search max results: OPERANT_WEB_SEARCH_MAX_RESULTS or WEB_SEARCH_MAX_RESULTS
        if let Ok(max_results) = std::env::var("OPERANT_WEB_SEARCH_MAX_RESULTS")
            .or_else(|_| std::env::var("WEB_SEARCH_MAX_RESULTS"))
            && let Ok(max_results) = max_results.parse::<usize>()
            && (1..=10).contains(&max_results)
        {
            self.web_search.max_results = max_results;
        }

        // Web search timeout: OPERANT_WEB_SEARCH_TIMEOUT_SECS or WEB_SEARCH_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_WEB_SEARCH_TIMEOUT_SECS")
            .or_else(|_| std::env::var("WEB_SEARCH_TIMEOUT_SECS"))
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.web_search.timeout_secs = timeout_secs;
        }

        // Storage provider key (optional backend override): OPERANT_STORAGE_PROVIDER
        if let Ok(provider) = std::env::var("OPERANT_STORAGE_PROVIDER") {
            let provider = provider.trim();
            if !provider.is_empty() {
                self.storage.provider.config.provider = provider.to_string();
            }
        }

        // Storage connection URL (for remote backends): OPERANT_STORAGE_DB_URL
        if let Ok(db_url) = std::env::var("OPERANT_STORAGE_DB_URL") {
            let db_url = db_url.trim();
            if !db_url.is_empty() {
                self.storage.provider.config.db_url = Some(db_url.to_string());
            }
        }

        // Storage connect timeout: OPERANT_STORAGE_CONNECT_TIMEOUT_SECS
        if let Ok(timeout_secs) = std::env::var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS")
            && let Ok(timeout_secs) = timeout_secs.parse::<u64>()
            && timeout_secs > 0
        {
            self.storage.provider.config.connect_timeout_secs = Some(timeout_secs);
        }
        // Proxy enabled flag: OPERANT_PROXY_ENABLED
        let explicit_proxy_enabled = std::env::var("OPERANT_PROXY_ENABLED")
            .ok()
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
        }

        // Proxy URLs: OPERANT_* wins, then generic *PROXY vars.
        let mut proxy_url_overridden = false;
        if let Ok(proxy_url) =
            std::env::var("OPERANT_HTTP_PROXY").or_else(|_| std::env::var("HTTP_PROXY"))
        {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPERANT_HTTPS_PROXY").or_else(|_| std::env::var("HTTPS_PROXY"))
        {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(proxy_url) =
            std::env::var("OPERANT_ALL_PROXY").or_else(|_| std::env::var("ALL_PROXY"))
        {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            proxy_url_overridden = true;
        }
        if let Ok(no_proxy) =
            std::env::var("OPERANT_NO_PROXY").or_else(|_| std::env::var("NO_PROXY"))
        {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
        }

        // Proxy scope and service selectors.
        if let Ok(scope_raw) = std::env::var("OPERANT_PROXY_SCOPE") {
            if let Some(scope) = parse_proxy_scope(&scope_raw) {
                self.proxy.scope = scope;
            } else {
                tracing::warn!(
                    scope = %scope_raw,
                    "Ignoring invalid OPERANT_PROXY_SCOPE (valid: environment|operant|services)"
                );
            }
        }

        if let Ok(services_raw) = std::env::var("OPERANT_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());

        if self.conversational_ai.enabled {
            tracing::warn!(
                "conversational_ai.enabled = true but conversational AI features are not yet \
                 implemented; this section is reserved for future use and will be ignored"
            );
        }

        // Slack channel-token env-var fallbacks. Resolved here (after the
        // file is parsed and all other overrides applied) so a config that
        // omits `bot_token` entirely still deserializes — channel
        // construction picks up the env value at startup. See #6237.
        // OPERANT_-prefixed variants take precedence over the bare names.
        if let Some(ref mut sl) = self.channels.slack {
            if sl.bot_token.as_deref().is_none_or(str::is_empty)
                && let Ok(v) = std::env::var("OPERANT_SLACK_BOT_TOKEN")
                    .or_else(|_| std::env::var("SLACK_BOT_TOKEN"))
                && !v.is_empty()
            {
                sl.bot_token = Some(v);
            }
            if sl.app_token.as_deref().is_none_or(str::is_empty)
                && let Ok(v) = std::env::var("OPERANT_SLACK_APP_TOKEN")
                    .or_else(|_| std::env::var("SLACK_APP_TOKEN"))
                && !v.is_empty()
            {
                sl.app_token = Some(v);
            }
        }
    }

    async fn resolve_config_path_for_save(&self) -> Result<PathBuf> {
        if self
            .config_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
        {
            return Ok(self.config_path.clone());
        }

        let (default_operant_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
        let (operant_dir, _workspace_dir, source) =
            resolve_runtime_config_dirs(&default_operant_dir, &default_workspace_dir).await?;
        let file_name = self
            .config_path
            .file_name()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| std::ffi::OsStr::new("config.toml"));
        let resolved = operant_dir.join(file_name);
        tracing::warn!(
            path = %self.config_path.display(),
            resolved = %resolved.display(),
            source = source.as_str(),
            "Config path missing parent directory; resolving from runtime environment"
        );
        Ok(resolved)
    }

    /// Persist this config to disk, encrypting secrets and syncing to disk.
    pub async fn save(&self) -> Result<()> {
        // Encrypt secrets before serialization
        let mut config_to_save = self.clone();
        let config_path = self.resolve_config_path_for_save().await?;
        let operant_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;
        let store = crate::secrets::SecretStore::new(operant_dir, self.secrets.encrypt);

        // Encrypt all #[secret]-annotated fields via Configurable derive
        config_to_save.encrypt_secrets(&store)?;

        let new_toml =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        // If an existing config file is present, sync the new values onto it
        // to preserve comments and formatting. Otherwise, use the fresh serialization.
        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let new_table: toml::Table =
                    toml::from_str(&new_toml).context("Failed to round-trip serialized config")?;
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("Failed to parse existing config for comment preservation")?;
                crate::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        let parent_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        fs::create_dir_all(parent_dir).await.with_context(|| {
            format!(
                "Failed to create config directory: {}",
                parent_dir.display()
            )
        })?;

        let file_name = config_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("config.toml");
        let temp_path = parent_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
        let backup_path = parent_dir.join(format!("{file_name}.bak"));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| {
                format!(
                    "Failed to create temporary config file: {}",
                    temp_path.display()
                )
            })?;
        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to fsync temporary config file")?;
        drop(temp_file);

        let had_existing_config = config_path.exists();
        if had_existing_config {
            fs::copy(&config_path, &backup_path)
                .await
                .with_context(|| {
                    format!(
                        "Failed to create config backup before atomic replace: {}",
                        backup_path.display()
                    )
                })?;
        }

        if let Err(e) = fs::rename(&temp_path, &config_path).await {
            let _ = fs::remove_file(&temp_path).await;
            if had_existing_config && backup_path.exists() {
                fs::copy(&backup_path, &config_path)
                    .await
                    .context("Failed to restore config backup")?;
            }
            anyhow::bail!("Failed to atomically replace config file: {e}");
        }

        #[cfg(unix)]
        {
            use std::{fs::Permissions, os::unix::fs::PermissionsExt};
            if let Err(err) = fs::set_permissions(&config_path, Permissions::from_mode(0o600)).await
            {
                tracing::warn!(
                    "Failed to harden config permissions to 0600 at {}: {}",
                    config_path.display(),
                    err
                );
            }
        }

        sync_directory(parent_dir).await?;

        if had_existing_config {
            let _ = fs::remove_file(&backup_path).await;
        }

        Ok(())
    }
}

#[allow(clippy::unused_async)] // async needed on unix for tokio File I/O; no-op on other platforms
pub(crate) async fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let dir = File::open(path)
            .await
            .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
        dir.sync_all()
            .await
            .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
        // FlushFileBuffers on directory handles returns ERROR_ACCESS_DENIED on
        // Windows (OS Error 5). This is expected — NTFS does not support
        // flushing directory metadata the same way Unix does. The individual
        // files have already been synced, so it is safe to ignore this error.
        if let Err(e) = dir.sync_all() {
            if e.raw_os_error() == Some(5) {
                tracing::trace!(
                    "Ignoring expected ACCESS_DENIED when fsyncing directory on Windows: {}",
                    path.display()
                );
            } else {
                return Err(e).with_context(|| {
                    format!("Failed to fsync directory metadata: {}", path.display())
                });
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

// ── HasPropKind impls for config enums ──
// Scalars (bool, String, integers, floats) are covered by impl_prop_kind! in traits.rs.
// Config enums serialize as TOML strings and are classified as PropKind::Enum.
macro_rules! impl_enum_prop_kind {
    ($($ty:ty),+ $(,)?) => {
        $(impl HasPropKind for $ty { const PROP_KIND: PropKind = PropKind::Enum; })+
    };
}

impl_enum_prop_kind!(
    SwarmStrategy,
    HardwareTransport,
    McpTransport,
    ToolFilterGroupMode,
    SkillsPromptInjectionMode,
    FirecrawlMode,
    ProxyScope,
    SearchMode,
    CronScheduleDecl,
    StreamMode,
    WhatsAppWebMode,
    WhatsAppChatPolicy,
    LineDmPolicy,
    LineGroupPolicy,
    LarkReceiveMode,
    OtpMethod,
    SandboxBackend,
    AutonomyLevel,
);

impl HasPropKind for ModelPricing {
    // ModelPricing is a 2-field struct (`input`, `output`). Wire form is a
    // JSON object (e.g. `{"input": 1.0, "output": 2.5}`); the dashboard
    // renders a sub-form for the inner fields. `PropKind::Object` (vs
    // `String`) is what makes the round-trip through `Config::set_prop`
    // succeed — `parse_prop_value` parses the JSON into a TOML table so
    // serde deserializes it back into the typed `ModelPricing` instead
    // of failing on a TOML string (#6357 review).
    const PROP_KIND: PropKind = PropKind::Object;
}

impl HasPropKind for serde_json::Value {
    // `serde_json::Value` is an arbitrary JSON document, not an enum.
    // Classifying it as `Enum` previously made `enum_variants_for::<Value>()`
    // hand back the literal placeholder `"(unknown variants)"`, and the
    // dashboard form rendered fields like `providers.models.<key>.provider_extra`
    // as a single-option dropdown. `String` is the closest scalar kind —
    // the form renders a text input where the user pastes raw JSON.
    // Round-trip via `set_prop` stays correct: serde deserializes the TOML
    // string back into `Value::String(...)`. Power users editing complex
    // objects still use `operant config set --json` or hand-edit the
    // `config.toml`.
    const PROP_KIND: PropKind = PropKind::String;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use tempfile::TempDir;
    use tokio::sync::{Mutex, MutexGuard};
    use tokio::test;

    // ── Tilde expansion ───────────────────────────────────────

    #[test]
    async fn expand_tilde_path_handles_absolute_path() {
        let path = expand_tilde_path("/absolute/path");
        assert_eq!(path, PathBuf::from("/absolute/path"));
    }

    #[test]
    async fn expand_tilde_path_handles_relative_path() {
        let path = expand_tilde_path("relative/path");
        assert_eq!(path, PathBuf::from("relative/path"));
    }

    #[test]
    async fn expand_tilde_path_expands_tilde_when_home_set() {
        // This test verifies that tilde expansion works when HOME is set.
        // In normal environments, HOME is set, so ~ should expand.
        let path = expand_tilde_path("~/.operant");
        // The path should not literally start with '~' if HOME is set
        // (it should be expanded to the actual home directory)
        if std::env::var("HOME").is_ok() {
            assert!(
                !path.to_string_lossy().starts_with('~'),
                "Tilde should be expanded when HOME is set"
            );
        }
    }

    // ── Defaults ─────────────────────────────────────────────

    fn has_test_table(raw: &str, table: &str) -> bool {
        let exact = format!("[{table}]");
        let nested = format!("[{table}.");
        raw.lines()
            .map(str::trim)
            .any(|line| line == exact || line.starts_with(&nested))
    }

    fn parse_test_config(raw: &str) -> Config {
        let mut merged = raw.trim().to_string();
        for table in [
            "data_retention",
            "cloud_ops",
            "conversational_ai",
            "security",
            "security_ops",
        ] {
            if has_test_table(&merged, table) {
                continue;
            }
            if !merged.is_empty() {
                merged.push_str("\n\n");
            }
            merged.push('[');
            merged.push_str(table);
            merged.push(']');
        }
        merged.push('\n');
        // Deserialize through V1Compat to handle legacy top-level fields.
        let compat: crate::migration::V1Compat = toml::from_str(&merged).unwrap();
        let mut config = compat.into_config();
        config.autonomy.ensure_default_auto_approve();
        config
    }

    #[test]
    async fn http_request_config_default_has_correct_values() {
        let cfg = HttpRequestConfig::default();
        assert_eq!(cfg.timeout_secs, 30);
        assert_eq!(cfg.max_response_size, 1_000_000);
        assert!(cfg.enabled);
        assert_eq!(cfg.allowed_domains, vec!["*".to_string()]);
    }

    #[test]
    async fn config_default_has_sane_values() {
        let c = Config::default();
        // V2: no fallback provider by default — set during onboarding.
        assert!(c.providers.fallback.is_none());
        assert!(c.providers.fallback_provider().is_none());
        assert!(!c.skills.open_skills_enabled);
        assert!(!c.skills.allow_scripts);
        assert!(!c.skills.install_suggestions.enabled);
        assert_eq!(
            c.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Full
        );
        assert!(c.workspace_dir.to_string_lossy().contains("workspace"));
        assert!(c.config_path.to_string_lossy().contains("config.toml"));
    }

    #[test]
    async fn skills_install_suggestions_config_deserializes_enabled() {
        let c = parse_test_config(
            r#"
[skills.install_suggestions]
enabled = true
"#,
        );

        assert!(c.skills.install_suggestions.enabled);
    }

    #[test]
    async fn skills_install_suggestions_config_accepts_hyphen_alias() {
        let c = parse_test_config(
            r#"
[skills.install-suggestions]
enabled = true
"#,
        );

        assert!(c.skills.install_suggestions.enabled);
    }

    #[derive(Clone, Default)]
    struct SharedLogBuffer(Arc<StdMutex<Vec<u8>>>);

    struct SharedLogWriter(Arc<StdMutex<Vec<u8>>>);

    impl SharedLogBuffer {
        fn captured(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogBuffer {
        type Writer = SharedLogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriter(self.0.clone())
        }
    }

    impl io::Write for SharedLogWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    async fn config_dir_creation_error_mentions_openrc_and_path() {
        let msg = config_dir_creation_error(Path::new("/etc/operant"));
        assert!(msg.contains("/etc/operant"));
        assert!(msg.contains("OpenRC"));
        assert!(msg.contains("operant"));
    }

    #[test]
    async fn config_schema_export_contains_expected_contract_shape() {
        #[cfg(feature = "schema-export")]
        let schema = schemars::schema_for!(Config);
        let schema_json = serde_json::to_value(&schema).expect("schema should serialize to json");

        // schemars 0.8 uses draft-07; the exact URL format may vary across versions
        let schema_version = schema_json
            .get("$schema")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert!(
            schema_version.contains("draft-07"),
            "schema should use JSON Schema draft-07, got: {schema_version}"
        );

        let properties = schema_json
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema should expose top-level properties");

        assert!(properties.contains_key("providers"));
        assert!(properties.contains_key("skills"));
        assert!(properties.contains_key("gateway"));
        assert!(properties.contains_key("channels"));
        assert!(!properties.contains_key("workspace_dir"));
        assert!(!properties.contains_key("config_path"));
        // These fields are now #[serde(skip)] cache fields, not in schema.
        assert!(!properties.contains_key("default_provider"));
        assert!(!properties.contains_key("api_key"));
        assert!(!properties.contains_key("default_model"));

        // schemars 0.8 (draft-07) uses `definitions`; draft-2020-12 uses `$defs`
        assert!(
            schema_json
                .get("definitions")
                .or_else(|| schema_json.get("$defs"))
                .and_then(serde_json::Value::as_object)
                .is_some(),
            "schema should include reusable type definitions"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn save_sets_config_permissions_on_new_file() {
        let temp = TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");

        let config = Config {
            config_path: config_path.clone(),
            workspace_dir,
            ..Default::default()
        };

        config.save().await.expect("save config");

        let mode = std::fs::metadata(&config_path)
            .expect("config metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    async fn observability_config_default() {
        let o = ObservabilityConfig::default();
        assert_eq!(o.backend, "none");
        assert_eq!(o.runtime_trace_mode, "none");
        assert_eq!(o.runtime_trace_path, "state/runtime-trace.jsonl");
        assert_eq!(o.runtime_trace_max_entries, 200);
    }

    #[test]
    async fn autonomy_config_default() {
        let a = AutonomyConfig::default();
        assert_eq!(a.level, AutonomyLevel::Supervised);
        assert!(a.workspace_only);
        assert!(a.allowed_commands.contains(&"git".to_string()));
        assert!(a.allowed_commands.contains(&"cargo".to_string()));
        assert!(a.forbidden_paths.contains(&"/etc".to_string()));
        assert_eq!(a.max_actions_per_hour, 20);
        assert_eq!(a.max_cost_per_day_cents, 500);
        assert!(a.require_approval_for_medium_risk);
        assert!(a.block_high_risk_commands);
        assert!(a.shell_env_passthrough.is_empty());
    }

    #[test]
    async fn runtime_config_default() {
        let r = RuntimeConfig::default();
        assert_eq!(r.kind, "native");
        assert_eq!(r.docker.image, "alpine:3.20");
        assert_eq!(r.docker.network, "none");
        assert_eq!(r.docker.memory_limit_mb, Some(512));
        assert_eq!(r.docker.cpu_limit, Some(1.0));
        assert!(r.docker.read_only_rootfs);
        assert!(r.docker.mount_workspace);
    }

    #[test]
    async fn heartbeat_config_default() {
        let h = HeartbeatConfig::default();
        assert!(h.enabled);
        assert_eq!(h.interval_minutes, 30);
        assert!(h.message.is_none());
        assert!(h.target.is_none());
        assert!(h.to.is_none());
    }

    #[test]
    async fn heartbeat_config_parses_delivery_aliases() {
        let raw = r#"
enabled = true
interval_minutes = 10
message = "Ping"
channel = "telegram"
recipient = "42"
"#;
        let parsed: HeartbeatConfig = toml::from_str(raw).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.interval_minutes, 10);
        assert_eq!(parsed.message.as_deref(), Some("Ping"));
        assert_eq!(parsed.target.as_deref(), Some("telegram"));
        assert_eq!(parsed.to.as_deref(), Some("42"));
    }

    #[test]
    async fn cron_config_default() {
        let c = CronConfig::default();
        assert!(c.enabled);
        assert_eq!(c.max_run_history, 50);
    }

    #[test]
    async fn cron_config_serde_roundtrip() {
        let c = CronConfig {
            enabled: false,
            catch_up_on_startup: false,
            max_run_history: 100,
            jobs: Vec::new(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let parsed: CronConfig = serde_json::from_str(&json).unwrap();
        assert!(!parsed.enabled);
        assert!(!parsed.catch_up_on_startup);
        assert_eq!(parsed.max_run_history, 100);
    }

    #[test]
    async fn config_defaults_cron_when_section_missing() {
        let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;

        let parsed = parse_test_config(toml_str);
        assert!(parsed.cron.enabled);
        assert!(parsed.cron.catch_up_on_startup);
        assert_eq!(parsed.cron.max_run_history, 50);
    }

    #[test]
    async fn memory_config_default_hygiene_settings() {
        let m = MemoryConfig::default();
        assert_eq!(m.backend, "agentmemory");
        assert!(m.auto_save);
        assert!(m.hygiene_enabled);
        assert_eq!(m.archive_after_days, 7);
        assert_eq!(m.purge_after_days, 30);
        assert_eq!(m.conversation_retention_days, 30);
        assert!(m.sqlite_open_timeout_secs.is_none());
        assert_eq!(m.search_mode, SearchMode::Hybrid);
    }

    #[test]
    async fn search_mode_config_deserialization() {
        let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "bm25"
"#;
        let parsed = parse_test_config(toml_str);
        assert_eq!(parsed.memory.search_mode, SearchMode::Bm25);

        let toml_str_embedding = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "embedding"
"#;
        let parsed = parse_test_config(toml_str_embedding);
        assert_eq!(parsed.memory.search_mode, SearchMode::Embedding);

        let toml_str_hybrid = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
search_mode = "hybrid"
"#;
        let parsed = parse_test_config(toml_str_hybrid);
        assert_eq!(parsed.memory.search_mode, SearchMode::Hybrid);
    }

    #[test]
    async fn search_mode_defaults_to_hybrid_when_omitted() {
        let toml_str = r#"
workspace_dir = "/tmp/workspace"
config_path = "/tmp/config.toml"
default_temperature = 0.7

[memory]
backend = "sqlite"
auto_save = true
"#;
        let parsed = parse_test_config(toml_str);
        assert_eq!(parsed.memory.search_mode, SearchMode::Hybrid);
    }

    #[test]
    async fn search_mode_serde_roundtrip() {
        let json_bm25 = serde_json::to_string(&SearchMode::Bm25).unwrap();
        assert_eq!(json_bm25, "\"bm25\"");
        let parsed: SearchMode = serde_json::from_str(&json_bm25).unwrap();
        assert_eq!(parsed, SearchMode::Bm25);

        let json_embedding = serde_json::to_string(&SearchMode::Embedding).unwrap();
        assert_eq!(json_embedding, "\"embedding\"");
        let parsed: SearchMode = serde_json::from_str(&json_embedding).unwrap();
        assert_eq!(parsed, SearchMode::Embedding);

        let json_hybrid = serde_json::to_string(&SearchMode::Hybrid).unwrap();
        assert_eq!(json_hybrid, "\"hybrid\"");
        let parsed: SearchMode = serde_json::from_str(&json_hybrid).unwrap();
        assert_eq!(parsed, SearchMode::Hybrid);
    }

    #[test]
    async fn storage_provider_config_defaults() {
        let storage = StorageConfig::default();
        assert!(storage.provider.config.provider.is_empty());
        assert!(storage.provider.config.db_url.is_none());
        assert_eq!(storage.provider.config.schema, "public");
        assert_eq!(storage.provider.config.table, "memories");
        assert!(storage.provider.config.connect_timeout_secs.is_none());
    }

    #[test]
    async fn memory_config_pgvector_defaults() {
        let memory = MemoryConfig::default();
        assert!(!memory.postgres.vector_enabled);
        assert_eq!(memory.postgres.vector_dimensions, 1536);
    }

    #[test]
    async fn memory_config_pgvector_roundtrip() {
        // `auto_save` is required on MemoryConfig and unrelated to the pgvector
        // fields these tests exercise. Including it keeps the fixture parseable
        // without coupling the test to schema-default behavior on auto_save.
        let toml = r#"
            backend = "postgres"
            auto_save = true
            [postgres]
            vector_enabled = true
            vector_dimensions = 768
        "#;
        let parsed: MemoryConfig = toml::from_str(toml).unwrap();
        assert!(parsed.postgres.vector_enabled);
        assert_eq!(parsed.postgres.vector_dimensions, 768);

        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: MemoryConfig = toml::from_str(&serialized).unwrap();
        assert!(reparsed.postgres.vector_enabled);
        assert_eq!(reparsed.postgres.vector_dimensions, 768);
    }

    #[test]
    async fn memory_config_pgvector_defaults_when_omitted() {
        let toml = r#"
            backend = "postgres"
            auto_save = true
        "#;
        let parsed: MemoryConfig = toml::from_str(toml).unwrap();
        assert!(!parsed.postgres.vector_enabled);
        assert_eq!(parsed.postgres.vector_dimensions, 1536);
    }

    #[test]
    async fn model_provider_config_ollama_tuning_fields_roundtrip() {
        let toml = r#"
            ollama_num_ctx = 16384
            ollama_num_predict = 4096
            ollama_temperature_override = 0.5
        "#;
        let parsed: ModelProviderConfig = toml::from_str(toml).unwrap();
        assert_eq!(parsed.ollama_num_ctx, Some(16384));
        assert_eq!(parsed.ollama_num_predict, Some(4096));
        assert_eq!(parsed.ollama_temperature_override, Some(0.5));

        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: ModelProviderConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.ollama_num_ctx, Some(16384));
        assert_eq!(reparsed.ollama_num_predict, Some(4096));
        assert_eq!(reparsed.ollama_temperature_override, Some(0.5));
    }

    #[test]
    async fn model_provider_config_ollama_tuning_fields_default_to_none() {
        let toml = r#"
            api_key = "sk-test"
        "#;
        let parsed: ModelProviderConfig = toml::from_str(toml).unwrap();
        assert!(parsed.ollama_num_ctx.is_none());
        assert!(parsed.ollama_num_predict.is_none());
        assert!(parsed.ollama_temperature_override.is_none());
    }

    #[test]
    async fn channels_default() {
        let c = ChannelsConfig::default();
        assert!(c.cli);
        assert!(c.telegram.is_none());
        assert!(c.discord.is_none());
        assert!(!c.show_tool_calls);
    }

    // ── Serde round-trip ─────────────────────────────────────

    #[test]
    async fn config_toml_roundtrip() {
        let config = Config {
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            providers: crate::providers::ProvidersConfig {
                fallback: Some("openrouter".into()),
                fallback_chain: Vec::new(),
                models: {
                    let mut m = HashMap::new();
                    m.insert(
                        "openrouter".into(),
                        ModelProviderConfig {
                            api_key: Some("sk-test-key".into()),
                            model: Some("gpt-4o".into()),
                            temperature: Some(0.5),
                            timeout_secs: Some(120),
                            ..Default::default()
                        },
                    );
                    m
                },
                model_routes: Vec::new(),
                embedding_routes: Vec::new(),
            },
            workspace_dir: PathBuf::from("/tmp/test/workspace"),
            config_path: PathBuf::from("/tmp/test/config.toml"),
            observability: ObservabilityConfig {
                backend: "log".into(),
                ..ObservabilityConfig::default()
            },
            autonomy: AutonomyConfig {
                level: AutonomyLevel::Full,
                workspace_only: false,
                allowed_commands: vec!["docker".into()],
                forbidden_paths: vec!["/secret".into()],
                max_actions_per_hour: 50,
                max_cost_per_day_cents: 1000,
                require_approval_for_medium_risk: false,
                block_high_risk_commands: true,
                shell_env_passthrough: vec!["DATABASE_URL".into()],
                auto_approve: vec!["file_read".into()],
                always_ask: vec![],
                allowed_roots: vec![],
                non_cli_excluded_tools: vec![],
                shell_timeout_secs: default_shell_timeout_secs(),
            },
            trust: crate::scattered_types::TrustConfig::default(),
            backup: BackupConfig::default(),
            data_retention: DataRetentionConfig::default(),
            cloud_ops: CloudOpsConfig::default(),
            conversational_ai: ConversationalAiConfig::default(),
            security: SecurityConfig::default(),
            security_ops: SecurityOpsConfig::default(),
            runtime: RuntimeConfig {
                kind: "docker".into(),
                ..RuntimeConfig::default()
            },
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            skills: SkillsConfig::default(),
            pipeline: PipelineConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            heartbeat: HeartbeatConfig {
                enabled: true,
                interval_minutes: 15,
                two_phase: true,
                message: Some("Check London time".into()),
                target: Some("telegram".into()),
                to: Some("123456".into()),
                ..HeartbeatConfig::default()
            },
            cron: CronConfig::default(),
            channels: ChannelsConfig {
                cli: true,
                telegram: Some(TelegramConfig {
                    enabled: true,
                    dm_topics_enabled: false,
                    dm_topic_name: default_telegram_dm_topic_name(),
                    bot_token: "123:ABC".into(),
                    allowed_users: vec!["user1".into()],
                    stream_mode: StreamMode::default(),
                    draft_update_interval_ms: default_draft_update_interval_ms(),
                    interrupt_on_new_message: false,
                    mention_only: false,
                    ack_reactions: None,
                    proxy_url: None,
                    approval_timeout_secs: default_telegram_approval_timeout_secs(),
                    disable_link_previews: false,
                    typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
                    fallback_ips: vec![],
                }),
                discord: None,
                discord_history: None,
                slack: None,
                mattermost: None,
                webhook: None,
                imessage: None,
                matrix: None,
                signal: None,
                whatsapp: None,
                linq: None,
                wati: None,
                nextcloud_talk: None,
                email: None,
                gmail_push: None,
                irc: None,
                lark: None,
                line: None,
                feishu: None,
                dingtalk: None,
                wecom: None,
                wechat: None,
                qq: None,
                twitter: None,
                mochat: None,
                #[cfg(feature = "channel-nostr")]
                nostr: None,
                clawdtalk: None,
                reddit: None,
                bluesky: None,
                voice_call: None,
                voice_duplex: None,
                #[cfg(feature = "voice-wake")]
                voice_wake: None,
                mqtt: None,
                message_timeout_secs: 300,
                ack_reactions: true,
                show_tool_calls: true,
                session_persistence: true,
                session_backend: default_session_backend(),
                session_ttl_hours: 0,
                debounce_ms: 0,
            },
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            microsoft365: Microsoft365Config::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            browser_delegate: crate::scattered_types::BrowserDelegateConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            media_pipeline: MediaPipelineConfig::default(),
            web_fetch: WebFetchConfig::default(),
            link_enricher: LinkEnricherConfig::default(),
            text_browser: TextBrowserConfig::default(),
            web_search: WebSearchConfig::default(),
            project_intel: ProjectIntelConfig::default(),
            google_workspace: GoogleWorkspaceConfig::default(),
            proxy: ProxyConfig::default(),
            agent: AgentConfig::default(),
            pacing: PacingConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            delegate: DelegateToolConfig::default(),
            agents: HashMap::new(),
            swarms: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            transcription: TranscriptionConfig::default(),
            tts: TtsConfig::default(),
            mcp: McpConfig::default(),
            nodes: NodesConfig::default(),
            workspace: WorkspaceConfig::default(),
            onboard_state: OnboardStateConfig::default(),
            notion: NotionConfig::default(),
            jira: JiraConfig::default(),
            node_transport: NodeTransportConfig::default(),
            knowledge: KnowledgeConfig::default(),
            linkedin: LinkedInConfig::default(),
            image_gen: ImageGenConfig::default(),
            plugins: PluginsConfig::default(),
            locale: None,
            verifiable_intent: VerifiableIntentConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            claude_code_runner: ClaudeCodeRunnerConfig::default(),
            codex_cli: CodexCliConfig::default(),
            gemini_cli: GeminiCliConfig::default(),
            opencode_cli: OpenCodeCliConfig::default(),
            sop: SopConfig::default(),
            shell_tool: ShellToolConfig::default(),
            escalation: EscalationConfig::default(),
        };
        // Provider fields are now resolved directly — no cache needed.

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed = parse_test_config(&toml_str);

        assert_eq!(parsed.providers.fallback, config.providers.fallback);
        assert_eq!(parsed.observability.backend, "log");
        assert_eq!(parsed.observability.runtime_trace_mode, "none");
        assert_eq!(parsed.autonomy.level, AutonomyLevel::Full);
        assert!(!parsed.autonomy.workspace_only);
        assert_eq!(parsed.runtime.kind, "docker");
        assert!(parsed.heartbeat.enabled);
        assert_eq!(parsed.heartbeat.interval_minutes, 15);
        assert_eq!(
            parsed.heartbeat.message.as_deref(),
            Some("Check London time")
        );
        assert_eq!(parsed.heartbeat.target.as_deref(), Some("telegram"));
        assert_eq!(parsed.heartbeat.to.as_deref(), Some("123456"));
        assert!(parsed.channels.telegram.is_some());
        assert_eq!(parsed.channels.telegram.unwrap().bot_token, "123:ABC");
    }

    #[test]
    async fn config_minimal_toml_uses_defaults() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(minimal);
        assert!(
            parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref())
                .is_none()
        );
        assert_eq!(parsed.observability.backend, "none");
        assert_eq!(parsed.observability.runtime_trace_mode, "none");
        assert_eq!(parsed.autonomy.level, AutonomyLevel::Supervised);
        assert_eq!(parsed.runtime.kind, "native");
        assert!(parsed.heartbeat.enabled);
        assert!(parsed.channels.cli);
        assert!(parsed.memory.hygiene_enabled);
        assert_eq!(parsed.memory.archive_after_days, 7);
        assert_eq!(parsed.memory.purge_after_days, 30);
        assert_eq!(parsed.memory.conversation_retention_days, 30);
        // Temperature migrated to the fallback provider entry
        assert!(
            (parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.temperature)
                .unwrap_or(0.7)
                - 0.7)
                .abs()
                < f64::EPSILON
        );
        assert_eq!(
            parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.timeout_secs)
                .unwrap_or(120),
            DEFAULT_DELEGATE_TIMEOUT_SECS
        );
    }

    /// Regression test for #4171: the `[autonomy]` section must not be
    /// silently dropped when parsing config TOML.
    #[test]
    async fn autonomy_section_is_not_silently_ignored() {
        let raw = r#"
default_temperature = 0.7

[autonomy]
level = "full"
max_actions_per_hour = 99
auto_approve = ["file_read", "memory_recall", "http_request"]
"#;
        let parsed = parse_test_config(raw);
        assert_eq!(
            parsed.autonomy.level,
            AutonomyLevel::Full,
            "autonomy.level must be parsed from config (was silently defaulting to Supervised)"
        );
        assert_eq!(
            parsed.autonomy.max_actions_per_hour, 99,
            "autonomy.max_actions_per_hour must be parsed from config"
        );
        assert!(
            parsed
                .autonomy
                .auto_approve
                .contains(&"http_request".to_string()),
            "autonomy.auto_approve must include http_request from config"
        );
    }

    /// Regression test for #4247: when a user provides a custom auto_approve
    /// list, the built-in defaults must still be present.
    #[test]
    async fn auto_approve_merges_user_entries_with_defaults() {
        let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = ["my_custom_tool", "another_tool"]
"#;
        let parsed = parse_test_config(raw);
        // User entries are preserved
        assert!(
            parsed
                .autonomy
                .auto_approve
                .contains(&"my_custom_tool".to_string()),
            "user-supplied tool must remain in auto_approve"
        );
        assert!(
            parsed
                .autonomy
                .auto_approve
                .contains(&"another_tool".to_string()),
            "user-supplied tool must remain in auto_approve"
        );
        // Defaults are merged in
        for default_tool in &[
            "file_read",
            "memory_recall",
            "weather",
            "calculator",
            "web_fetch",
        ] {
            assert!(
                parsed
                    .autonomy
                    .auto_approve
                    .contains(&String::from(*default_tool)),
                "default tool '{default_tool}' must be present in auto_approve even when user provides custom list"
            );
        }
    }

    /// Regression test: empty auto_approve still gets defaults merged.
    #[test]
    async fn auto_approve_empty_list_gets_defaults() {
        let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = []
"#;
        let parsed = parse_test_config(raw);
        let defaults = default_auto_approve();
        for tool in &defaults {
            assert!(
                parsed.autonomy.auto_approve.contains(tool),
                "default tool '{tool}' must be present even when user sets auto_approve = []"
            );
        }
    }

    /// When no autonomy section is provided, defaults are applied normally.
    #[test]
    async fn auto_approve_defaults_when_no_autonomy_section() {
        let raw = r#"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(raw);
        let defaults = default_auto_approve();
        for tool in &defaults {
            assert!(
                parsed.autonomy.auto_approve.contains(tool),
                "default tool '{tool}' must be present when no [autonomy] section"
            );
        }
    }

    /// Duplicates are not introduced when ensure_default_auto_approve runs
    /// on a list that already contains the defaults.
    #[test]
    async fn auto_approve_no_duplicates() {
        let raw = r#"
default_temperature = 0.7

[autonomy]
auto_approve = ["weather", "file_read"]
"#;
        let parsed = parse_test_config(raw);
        let weather_count = parsed
            .autonomy
            .auto_approve
            .iter()
            .filter(|t| *t == "weather")
            .count();
        assert_eq!(weather_count, 1, "weather must not be duplicated");
        let file_read_count = parsed
            .autonomy
            .auto_approve
            .iter()
            .filter(|t| *t == "file_read")
            .count();
        assert_eq!(file_read_count, 1, "file_read must not be duplicated");
    }

    #[test]
    async fn provider_timeout_secs_parses_from_toml() {
        let raw = r#"
default_temperature = 0.7
provider_timeout_secs = 300
"#;
        let parsed = parse_test_config(raw);
        assert_eq!(
            parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.timeout_secs)
                .unwrap_or(120),
            300
        );
    }

    #[test]
    async fn parse_extra_headers_env_basic() {
        let headers = parse_extra_headers_env("User-Agent:MyApp/1.0,X-Title:operant");
        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers[0],
            ("User-Agent".to_string(), "MyApp/1.0".to_string())
        );
        assert_eq!(headers[1], ("X-Title".to_string(), "operant".to_string()));
    }

    #[test]
    async fn parse_extra_headers_env_with_url_value() {
        let headers =
            parse_extra_headers_env("HTTP-Referer:https://github.com/zeroclaw-labs/operant");
        assert_eq!(headers.len(), 1);
        // Only splits on first colon, preserving URL colons in value
        assert_eq!(headers[0].0, "HTTP-Referer");
        assert_eq!(headers[0].1, "https://github.com/zeroclaw-labs/operant");
    }

    #[test]
    async fn parse_extra_headers_env_empty_string() {
        let headers = parse_extra_headers_env("");
        assert!(headers.is_empty());
    }

    #[test]
    async fn parse_extra_headers_env_whitespace_trimming() {
        let headers = parse_extra_headers_env("  X-Title : operant , User-Agent : cli/1.0 ");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("X-Title".to_string(), "operant".to_string()));
        assert_eq!(
            headers[1],
            ("User-Agent".to_string(), "cli/1.0".to_string())
        );
    }

    #[test]
    async fn parse_extra_headers_env_skips_malformed() {
        let headers = parse_extra_headers_env("X-Valid:value,no-colon-here,Another:ok");
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], ("X-Valid".to_string(), "value".to_string()));
        assert_eq!(headers[1], ("Another".to_string(), "ok".to_string()));
    }

    #[test]
    async fn parse_extra_headers_env_skips_empty_key() {
        let headers = parse_extra_headers_env(":value,X-Valid:ok");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0], ("X-Valid".to_string(), "ok".to_string()));
    }

    #[test]
    async fn parse_extra_headers_env_allows_empty_value() {
        let headers = parse_extra_headers_env("X-Empty:");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0], ("X-Empty".to_string(), String::new()));
    }

    #[test]
    async fn parse_extra_headers_env_trailing_comma() {
        let headers = parse_extra_headers_env("X-Title:operant,");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0], ("X-Title".to_string(), "operant".to_string()));
    }

    #[test]
    async fn extra_headers_parses_from_toml() {
        let raw = r#"
default_temperature = 0.7

[extra_headers]
User-Agent = "MyApp/1.0"
X-Title = "operant"
"#;
        let parsed = parse_test_config(raw);
        let headers = &parsed
            .providers
            .fallback_provider()
            .expect("fallback provider")
            .extra_headers;
        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("User-Agent").unwrap(), "MyApp/1.0");
        assert_eq!(headers.get("X-Title").unwrap(), "operant");
    }

    #[test]
    async fn extra_headers_defaults_to_empty() {
        let raw = r#"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(raw);
        assert!(
            parsed
                .providers
                .fallback_provider()
                .map(|e| e.extra_headers.is_empty())
                .unwrap_or(true)
        );
    }

    #[test]
    async fn storage_provider_dburl_alias_deserializes() {
        let raw = r#"
default_temperature = 0.7

[storage.provider.config]
provider = "qdrant"
dbURL = "http://localhost:6333"
schema = "public"
table = "memories"
connect_timeout_secs = 12
"#;

        let parsed = parse_test_config(raw);
        assert_eq!(parsed.storage.provider.config.provider, "qdrant");
        assert_eq!(
            parsed.storage.provider.config.db_url.as_deref(),
            Some("http://localhost:6333")
        );
        assert_eq!(parsed.storage.provider.config.schema, "public");
        assert_eq!(parsed.storage.provider.config.table, "memories");
        assert_eq!(
            parsed.storage.provider.config.connect_timeout_secs,
            Some(12)
        );
    }

    #[test]
    async fn runtime_reasoning_enabled_deserializes() {
        let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_enabled = false
"#;

        let parsed = parse_test_config(raw);
        assert_eq!(parsed.runtime.reasoning_enabled, Some(false));
    }

    #[test]
    async fn runtime_reasoning_effort_deserializes() {
        let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_effort = "HIGH"
"#;

        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.runtime.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    async fn runtime_reasoning_effort_rejects_invalid_values() {
        let raw = r#"
default_temperature = 0.7

[runtime]
reasoning_effort = "turbo"
"#;

        let error = toml::from_str::<Config>(raw).expect_err("invalid value should fail");
        assert!(error.to_string().contains("reasoning_effort"));
    }

    #[test]
    async fn agent_config_defaults() {
        let cfg = AgentConfig::default();
        assert!(cfg.compact_context);
        assert_eq!(cfg.max_tool_iterations, 10);
        assert_eq!(cfg.max_history_messages, 50);
        assert!(!cfg.parallel_tools);
        assert_eq!(cfg.tool_dispatcher, "auto");
    }

    #[test]
    async fn agent_config_deserializes() {
        let raw = r#"
default_temperature = 0.7
[agent]
compact_context = true
max_tool_iterations = 20
max_history_messages = 80
parallel_tools = true
tool_dispatcher = "xml"
"#;
        let parsed = parse_test_config(raw);
        assert!(parsed.agent.compact_context);
        assert_eq!(parsed.agent.max_tool_iterations, 20);
        assert_eq!(parsed.agent.max_history_messages, 80);
        assert!(parsed.agent.parallel_tools);
        assert_eq!(parsed.agent.tool_dispatcher, "xml");
    }

    #[test]
    async fn agent_config_evolution_nudge_intervals_deserialize() {
        // R24: the gateway path (runtime Agent) reads these from `[agent]`;
        // they drive the per-turn memory-review and skill-nudge triggers.
        let raw = r#"
default_temperature = 0.7
[agent]
memory_nudge_interval = 3
creation_nudge_interval = 7
"#;
        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.agent.memory_nudge_interval, 3);
        assert_eq!(parsed.agent.creation_nudge_interval, 7);
    }

    #[test]
    async fn agent_config_evolution_nudge_intervals_default_to_ten() {
        let parsed: Config = toml::from_str(
            r#"default_temperature = 0.7
[agent]
"#,
        )
        .unwrap();
        assert_eq!(parsed.agent.memory_nudge_interval, 10);
        assert_eq!(parsed.agent.creation_nudge_interval, 10);
    }

    #[test]
    async fn pacing_config_defaults_are_all_none_or_empty() {
        let cfg = PacingConfig::default();
        assert!(cfg.step_timeout_secs.is_none());
        assert!(cfg.loop_detection_min_elapsed_secs.is_none());
        assert!(cfg.loop_ignore_tools.is_empty());
        assert!(cfg.message_timeout_scale_max.is_none());
    }

    #[test]
    async fn pacing_config_deserializes_from_toml() {
        let raw = r#"
default_temperature = 0.7
[pacing]
step_timeout_secs = 120
loop_detection_min_elapsed_secs = 60
loop_ignore_tools = ["browser_screenshot", "browser_navigate"]
message_timeout_scale_max = 8
"#;
        let parsed: Config = toml::from_str(raw).unwrap();
        assert_eq!(parsed.pacing.step_timeout_secs, Some(120));
        assert_eq!(parsed.pacing.loop_detection_min_elapsed_secs, Some(60));
        assert_eq!(
            parsed.pacing.loop_ignore_tools,
            vec!["browser_screenshot", "browser_navigate"]
        );
        assert_eq!(parsed.pacing.message_timeout_scale_max, Some(8));
    }

    #[test]
    async fn pacing_config_absent_preserves_defaults() {
        let raw = r#"
default_temperature = 0.7
"#;
        let parsed: Config = toml::from_str(raw).unwrap();
        assert!(parsed.pacing.step_timeout_secs.is_none());
        assert!(parsed.pacing.loop_detection_min_elapsed_secs.is_none());
        assert!(parsed.pacing.loop_ignore_tools.is_empty());
        assert!(parsed.pacing.message_timeout_scale_max.is_none());
    }

    #[tokio::test]
    async fn sync_directory_handles_existing_directory() {
        let dir = std::env::temp_dir().join(format!(
            "operant_test_sync_directory_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        sync_directory(&dir).await.unwrap();

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_and_load_tmpdir() {
        let dir = std::env::temp_dir().join("operant_test_config");
        let _ = fs::remove_dir_all(&dir).await;
        fs::create_dir_all(&dir).await.unwrap();

        let config_path = dir.join("config.toml");
        let mut providers = crate::providers::ProvidersConfig {
            fallback: Some("openrouter".into()),
            ..Default::default()
        };
        providers.models.insert(
            "openrouter".into(),
            ModelProviderConfig {
                api_key: Some("sk-roundtrip".into()),
                model: Some("test-model".into()),
                temperature: Some(0.9),
                timeout_secs: Some(120),
                ..Default::default()
            },
        );
        let config = Config {
            schema_version: crate::migration::CURRENT_SCHEMA_VERSION,
            providers,
            workspace_dir: dir.join("workspace"),
            config_path: config_path.clone(),
            observability: ObservabilityConfig::default(),
            autonomy: AutonomyConfig::default(),
            trust: crate::scattered_types::TrustConfig::default(),
            backup: BackupConfig::default(),
            data_retention: DataRetentionConfig::default(),
            cloud_ops: CloudOpsConfig::default(),
            conversational_ai: ConversationalAiConfig::default(),
            security: SecurityConfig::default(),
            security_ops: SecurityOpsConfig::default(),
            runtime: RuntimeConfig::default(),
            reliability: ReliabilityConfig::default(),
            scheduler: SchedulerConfig::default(),
            skills: SkillsConfig::default(),
            pipeline: PipelineConfig::default(),
            query_classification: QueryClassificationConfig::default(),
            heartbeat: HeartbeatConfig::default(),
            cron: CronConfig::default(),
            channels: ChannelsConfig::default(),
            memory: MemoryConfig::default(),
            storage: StorageConfig::default(),
            tunnel: TunnelConfig::default(),
            gateway: GatewayConfig::default(),
            composio: ComposioConfig::default(),
            microsoft365: Microsoft365Config::default(),
            secrets: SecretsConfig::default(),
            browser: BrowserConfig::default(),
            browser_delegate: crate::scattered_types::BrowserDelegateConfig::default(),
            http_request: HttpRequestConfig::default(),
            multimodal: MultimodalConfig::default(),
            media_pipeline: MediaPipelineConfig::default(),
            web_fetch: WebFetchConfig::default(),
            link_enricher: LinkEnricherConfig::default(),
            text_browser: TextBrowserConfig::default(),
            web_search: WebSearchConfig::default(),
            project_intel: ProjectIntelConfig::default(),
            google_workspace: GoogleWorkspaceConfig::default(),
            proxy: ProxyConfig::default(),
            agent: AgentConfig::default(),
            pacing: PacingConfig::default(),
            identity: IdentityConfig::default(),
            cost: CostConfig::default(),
            peripherals: PeripheralsConfig::default(),
            delegate: DelegateToolConfig::default(),
            agents: HashMap::new(),
            swarms: HashMap::new(),
            hooks: HooksConfig::default(),
            hardware: HardwareConfig::default(),
            transcription: TranscriptionConfig::default(),
            tts: TtsConfig::default(),
            mcp: McpConfig::default(),
            nodes: NodesConfig::default(),
            workspace: WorkspaceConfig::default(),
            onboard_state: OnboardStateConfig::default(),
            notion: NotionConfig::default(),
            jira: JiraConfig::default(),
            node_transport: NodeTransportConfig::default(),
            knowledge: KnowledgeConfig::default(),
            linkedin: LinkedInConfig::default(),
            image_gen: ImageGenConfig::default(),
            plugins: PluginsConfig::default(),
            locale: None,
            verifiable_intent: VerifiableIntentConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            claude_code_runner: ClaudeCodeRunnerConfig::default(),
            codex_cli: CodexCliConfig::default(),
            gemini_cli: GeminiCliConfig::default(),
            opencode_cli: OpenCodeCliConfig::default(),
            sop: SopConfig::default(),
            shell_tool: ShellToolConfig::default(),
            escalation: EscalationConfig::default(),
        };

        // Provider fields are now resolved directly — no cache needed.
        config.save().await.unwrap();
        assert!(config_path.exists());

        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        let compat: crate::migration::V1Compat = toml::from_str(&contents).unwrap();
        let loaded = compat.into_config();
        let entry = &loaded.providers.models["openrouter"];
        assert!(
            entry
                .api_key
                .as_deref()
                .is_some_and(crate::secrets::SecretStore::is_encrypted)
        );
        let store = crate::secrets::SecretStore::new(&dir, true);
        let decrypted = store.decrypt(entry.api_key.as_deref().unwrap()).unwrap();
        assert_eq!(decrypted, "sk-roundtrip");
        assert_eq!(entry.model.as_deref(), Some("test-model"));
        assert!(
            entry
                .temperature
                .is_some_and(|t| (t - 0.9).abs() < f64::EPSILON)
        );

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_encrypts_nested_credentials() {
        let dir = std::env::temp_dir().join(format!(
            "operant_test_nested_credentials_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        let mut config = Config {
            workspace_dir: dir.join("workspace"),
            config_path: dir.join("config.toml"),
            ..Default::default()
        };
        config.providers.fallback = Some("default".into());
        config.providers.models.insert(
            "default".into(),
            ModelProviderConfig {
                api_key: Some("root-credential".into()),
                ..Default::default()
            },
        );
        // Provider fields are now resolved directly — no cache needed.
        config.composio.api_key = Some("composio-credential".into());
        config.browser.computer_use.api_key = Some("browser-credential".into());
        config.web_search.brave_api_key = Some("brave-credential".into());
        config.web_search.tavily_api_key = Some("tavily-credential".into());
        config.storage.provider.config.db_url = Some("postgres://user:pw@host/db".into());
        config.channels.feishu = Some(FeishuConfig {
            enabled: true,
            app_id: "cli_feishu_123".into(),
            app_secret: "feishu-secret".into(),
            encrypt_key: Some("feishu-encrypt".into()),
            verification_token: Some("feishu-verify".into()),
            allowed_users: vec!["*".into()],
            mention_only: false,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            proxy_url: None,
        });

        config.agents.insert(
            "worker".into(),
            DelegateAgentConfig {
                provider: "openrouter".into(),
                model: "model-test".into(),
                system_prompt: None,
                api_key: Some("agent-credential".into()),
                temperature: None,
                max_depth: 3,
                agentic: false,
                allowed_tools: Vec::new(),
                max_iterations: 10,
                timeout_secs: None,
                agentic_timeout_secs: None,
                skills_directory: None,
                memory_namespace: None,
            },
        );

        config.save().await.unwrap();

        let contents = tokio::fs::read_to_string(config.config_path.clone())
            .await
            .unwrap();
        let stored: Config = toml::from_str::<crate::migration::V1Compat>(&contents)
            .unwrap()
            .into_config();
        let store = crate::secrets::SecretStore::new(&dir, true);

        let root_encrypted = stored
            .providers
            .models
            .get("default")
            .and_then(|m| m.api_key.as_deref())
            .unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(root_encrypted));
        assert_eq!(store.decrypt(root_encrypted).unwrap(), "root-credential");

        let composio_encrypted = stored.composio.api_key.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(
            composio_encrypted
        ));
        assert_eq!(
            store.decrypt(composio_encrypted).unwrap(),
            "composio-credential"
        );

        let browser_encrypted = stored.browser.computer_use.api_key.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(browser_encrypted));
        assert_eq!(
            store.decrypt(browser_encrypted).unwrap(),
            "browser-credential"
        );

        let web_search_encrypted = stored.web_search.brave_api_key.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(
            web_search_encrypted
        ));
        assert_eq!(
            store.decrypt(web_search_encrypted).unwrap(),
            "brave-credential"
        );

        let tavily_encrypted = stored.web_search.tavily_api_key.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(tavily_encrypted));
        assert_eq!(
            store.decrypt(tavily_encrypted).unwrap(),
            "tavily-credential"
        );

        let worker = stored.agents.get("worker").unwrap();
        let worker_encrypted = worker.api_key.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(worker_encrypted));
        assert_eq!(store.decrypt(worker_encrypted).unwrap(), "agent-credential");

        let storage_db_url = stored.storage.provider.config.db_url.as_deref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(storage_db_url));
        assert_eq!(
            store.decrypt(storage_db_url).unwrap(),
            "postgres://user:pw@host/db"
        );

        let feishu = stored.channels.feishu.as_ref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(
            &feishu.app_secret
        ));
        assert_eq!(store.decrypt(&feishu.app_secret).unwrap(), "feishu-secret");
        assert!(
            feishu
                .encrypt_key
                .as_deref()
                .is_some_and(crate::secrets::SecretStore::is_encrypted)
        );
        assert_eq!(
            store
                .decrypt(feishu.encrypt_key.as_deref().unwrap())
                .unwrap(),
            "feishu-encrypt"
        );
        assert!(
            feishu
                .verification_token
                .as_deref()
                .is_some_and(crate::secrets::SecretStore::is_encrypted)
        );
        assert_eq!(
            store
                .decrypt(feishu.verification_token.as_deref().unwrap())
                .unwrap(),
            "feishu-verify"
        );

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn config_save_atomic_cleanup() {
        let dir =
            std::env::temp_dir().join(format!("operant_test_config_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();

        let config_path = dir.join("config.toml");
        let mut config = Config {
            workspace_dir: dir.join("workspace"),
            config_path: config_path.clone(),
            ..Default::default()
        };
        config.providers.fallback = Some("test".into());
        config.providers.models.insert(
            "test".into(),
            ModelProviderConfig {
                model: Some("model-a".into()),
                ..Default::default()
            },
        );
        config.save().await.unwrap();
        assert!(config_path.exists());

        config.providers.models.get_mut("test").unwrap().model = Some("model-b".into());
        config.save().await.unwrap();

        let contents = tokio::fs::read_to_string(&config_path).await.unwrap();
        assert!(contents.contains("model-b"));

        let mut names: Vec<String> = Vec::new();
        let mut read_dir = fs::read_dir(&dir).await.unwrap();
        while let Some(entry) = read_dir.next_entry().await.unwrap() {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        assert!(!names.iter().any(|name| name.contains(".tmp-")));
        assert!(!names.iter().any(|name| name.ends_with(".bak")));

        let _ = fs::remove_dir_all(&dir).await;
    }

    // ── Telegram / Discord config ────────────────────────────

    #[test]
    async fn telegram_config_serde() {
        let tc = TelegramConfig {
            enabled: true,
            bot_token: "123:XYZ".into(),
            allowed_users: vec!["alice".into(), "bob".into()],
            stream_mode: StreamMode::Partial,
            draft_update_interval_ms: 500,
            interrupt_on_new_message: true,
            mention_only: false,
            ack_reactions: None,
            proxy_url: None,
            approval_timeout_secs: 120,
            dm_topics_enabled: false,
            dm_topic_name: default_telegram_dm_topic_name(),
            disable_link_previews: false,
            typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
            fallback_ips: vec![],
        };
        let json = serde_json::to_string(&tc).unwrap();
        let parsed: TelegramConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bot_token, "123:XYZ");
        assert_eq!(parsed.allowed_users.len(), 2);
        assert_eq!(parsed.stream_mode, StreamMode::Partial);
        assert_eq!(parsed.draft_update_interval_ms, 500);
        assert!(parsed.interrupt_on_new_message);
    }

    #[test]
    async fn telegram_config_defaults_stream_off() {
        let json = r#"{"bot_token":"tok","allowed_users":[]}"#;
        let parsed: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.stream_mode, StreamMode::Off);
        assert_eq!(parsed.draft_update_interval_ms, 1000);
        assert!(!parsed.interrupt_on_new_message);
    }

    #[test]
    async fn discord_config_serde() {
        let dc = DiscordConfig {
            enabled: true,
            bot_token: "discord-token".into(),
            guild_id: Some("12345".into()),
            allowed_users: vec![],
            listen_to_bots: false,
            interrupt_on_new_message: false,
            mention_only: false,
            proxy_url: None,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1000,
            multi_message_delay_ms: 800,
            stall_timeout_secs: 0,
            approval_timeout_secs: 300,
        };
        let json = serde_json::to_string(&dc).unwrap();
        let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bot_token, "discord-token");
        assert_eq!(parsed.guild_id.as_deref(), Some("12345"));
    }

    #[test]
    async fn discord_config_optional_guild() {
        let dc = DiscordConfig {
            enabled: true,
            bot_token: "tok".into(),
            guild_id: None,
            allowed_users: vec![],
            listen_to_bots: false,
            interrupt_on_new_message: false,
            mention_only: false,
            proxy_url: None,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1000,
            multi_message_delay_ms: 800,
            stall_timeout_secs: 0,
            approval_timeout_secs: 300,
        };
        let json = serde_json::to_string(&dc).unwrap();
        let parsed: DiscordConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.guild_id.is_none());
    }

    // ── iMessage / Matrix config ────────────────────────────

    #[test]
    async fn imessage_config_serde() {
        let ic = IMessageConfig {
            enabled: true,
            allowed_contacts: vec!["+1234567890".into(), "user@icloud.com".into()],
        };
        let json = serde_json::to_string(&ic).unwrap();
        let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.allowed_contacts.len(), 2);
        assert_eq!(parsed.allowed_contacts[0], "+1234567890");
    }

    #[test]
    async fn imessage_config_empty_contacts() {
        let ic = IMessageConfig {
            enabled: true,
            allowed_contacts: vec![],
        };
        let json = serde_json::to_string(&ic).unwrap();
        let parsed: IMessageConfig = serde_json::from_str(&json).unwrap();
        assert!(parsed.allowed_contacts.is_empty());
    }

    #[test]
    async fn imessage_config_wildcard() {
        let ic = IMessageConfig {
            enabled: true,
            allowed_contacts: vec!["*".into()],
        };
        let toml_str = toml::to_string(&ic).unwrap();
        let parsed: IMessageConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.allowed_contacts, vec!["*"]);
    }

    #[test]
    async fn matrix_config_serde() {
        let mc = MatrixConfig {
            enabled: true,
            homeserver: "https://matrix.org".into(),
            access_token: "syt_token_abc".into(),
            user_id: Some("@bot:matrix.org".into()),
            device_id: Some("DEVICE123".into()),
            allowed_users: vec!["@user:matrix.org".into()],
            allowed_rooms: vec!["!room123:matrix.org".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        let json = serde_json::to_string(&mc).unwrap();
        let parsed: MatrixConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.homeserver, "https://matrix.org");
        assert_eq!(parsed.access_token, "syt_token_abc");
        assert_eq!(parsed.user_id.as_deref(), Some("@bot:matrix.org"));
        assert_eq!(parsed.device_id.as_deref(), Some("DEVICE123"));
        assert_eq!(
            parsed.allowed_rooms.first().map(|s| s.as_str()),
            Some("!room123:matrix.org")
        );
        assert_eq!(parsed.allowed_users.len(), 1);
    }

    #[test]
    async fn matrix_config_toml_roundtrip() {
        let mc = MatrixConfig {
            enabled: true,
            homeserver: "https://synapse.local:8448".into(),
            access_token: "tok".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec!["@admin:synapse.local".into(), "*".into()],
            allowed_rooms: vec!["!abc:synapse.local".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        let toml_str = toml::to_string(&mc).unwrap();
        let parsed: MatrixConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.homeserver, "https://synapse.local:8448");
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    async fn matrix_config_backward_compatible_without_session_hints() {
        // room_id in TOML is now migrated by prepare_table at the top level;
        // a bare MatrixConfig parse just ignores unknown keys.
        let toml = r#"
homeserver = "https://matrix.org"
access_token = "tok"
allowed_users = ["@ops:matrix.org"]
allowed_rooms = ["!ops:matrix.org"]
"#;

        let parsed: MatrixConfig = toml::from_str(toml).unwrap();
        assert_eq!(parsed.homeserver, "https://matrix.org");
        assert!(parsed.user_id.is_none());
        assert!(parsed.device_id.is_none());
        assert_eq!(parsed.allowed_rooms, vec!["!ops:matrix.org"]);
    }

    #[test]
    async fn matrix_config_reply_in_thread_defaults_to_true() {
        let toml = r#"
homeserver = "https://matrix.org"
access_token = "tok"
allowed_users = ["@u:matrix.org"]
"#;
        let parsed: MatrixConfig = toml::from_str(toml).unwrap();
        assert!(parsed.reply_in_thread);
    }

    #[test]
    async fn signal_config_serde() {
        let sc = SignalConfig {
            enabled: true,
            http_url: "http://127.0.0.1:8686".into(),
            account: "+1234567890".into(),
            group_id: Some("group123".into()),
            allowed_from: vec!["+1111111111".into()],
            ignore_attachments: true,
            ignore_stories: false,
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        let json = serde_json::to_string(&sc).unwrap();
        let parsed: SignalConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.http_url, "http://127.0.0.1:8686");
        assert_eq!(parsed.account, "+1234567890");
        assert_eq!(parsed.group_id.as_deref(), Some("group123"));
        assert_eq!(parsed.allowed_from.len(), 1);
        assert!(parsed.ignore_attachments);
        assert!(!parsed.ignore_stories);
    }

    #[test]
    async fn signal_config_toml_roundtrip() {
        let sc = SignalConfig {
            enabled: true,
            http_url: "http://localhost:8080".into(),
            account: "+9876543210".into(),
            group_id: None,
            allowed_from: vec!["*".into()],
            ignore_attachments: false,
            ignore_stories: true,
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        let toml_str = toml::to_string(&sc).unwrap();
        let parsed: SignalConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.http_url, "http://localhost:8080");
        assert_eq!(parsed.account, "+9876543210");
        assert!(parsed.group_id.is_none());
        assert!(parsed.ignore_stories);
    }

    #[test]
    async fn signal_config_defaults() {
        let json = r#"{"http_url":"http://127.0.0.1:8686","account":"+1234567890"}"#;
        let parsed: SignalConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.group_id.is_none());
        assert!(parsed.allowed_from.is_empty());
        assert!(!parsed.ignore_attachments);
        assert!(!parsed.ignore_stories);
    }

    #[test]
    async fn channels_with_imessage_and_matrix() {
        let c = ChannelsConfig {
            cli: true,
            telegram: None,
            discord: None,
            discord_history: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: Some(IMessageConfig {
                enabled: true,
                allowed_contacts: vec!["+1".into()],
            }),
            matrix: Some(MatrixConfig {
                enabled: true,
                homeserver: "https://m.org".into(),
                access_token: "tok".into(),
                user_id: None,
                device_id: None,
                allowed_users: vec!["@u:m".into()],
                allowed_rooms: vec!["!r:m".into()],
                interrupt_on_new_message: false,
                stream_mode: StreamMode::default(),
                draft_update_interval_ms: 1500,
                multi_message_delay_ms: 800,
                recovery_key: None,
                mention_only: false,
                password: None,
                approval_timeout_secs: 300,
                reply_in_thread: true,
                ack_reactions: true,
            }),
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            gmail_push: None,
            irc: None,
            lark: None,
            line: None,
            feishu: None,
            dingtalk: None,
            wecom: None,
            wechat: None,
            qq: None,
            twitter: None,
            mochat: None,
            #[cfg(feature = "channel-nostr")]
            nostr: None,
            clawdtalk: None,
            reddit: None,
            bluesky: None,
            voice_call: None,
            voice_duplex: None,
            #[cfg(feature = "voice-wake")]
            voice_wake: None,
            mqtt: None,
            message_timeout_secs: 300,
            ack_reactions: true,
            show_tool_calls: true,
            session_persistence: true,
            session_backend: default_session_backend(),
            session_ttl_hours: 0,
            debounce_ms: 0,
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.imessage.is_some());
        assert!(parsed.matrix.is_some());
        assert_eq!(parsed.imessage.unwrap().allowed_contacts, vec!["+1"]);
        assert_eq!(parsed.matrix.unwrap().homeserver, "https://m.org");
    }

    #[test]
    async fn channels_default_has_no_imessage_matrix() {
        let c = ChannelsConfig::default();
        assert!(c.imessage.is_none());
        assert!(c.matrix.is_none());
    }

    // ── Edge cases: serde(default) for allowed_users ─────────

    #[test]
    async fn discord_config_deserializes_without_allowed_users() {
        // Old configs won't have allowed_users — serde(default) should fill vec![]
        let json = r#"{"bot_token":"tok","guild_id":"123"}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.allowed_users.is_empty());
    }

    #[test]
    async fn discord_config_deserializes_with_allowed_users() {
        let json = r#"{"bot_token":"tok","guild_id":"123","allowed_users":["111","222"]}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.allowed_users, vec!["111", "222"]);
    }

    #[test]
    async fn slack_config_deserializes_without_allowed_users() {
        let json = r#"{"bot_token":"xoxb-tok"}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.channel_ids.is_empty());
        assert!(parsed.allowed_users.is_empty());
        assert!(!parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn slack_config_deserializes_with_allowed_users() {
        let json = r#"{"bot_token":"xoxb-tok","allowed_users":["U111"]}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.channel_ids.is_empty());
        assert_eq!(parsed.allowed_users, vec!["U111"]);
        assert!(!parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn slack_config_deserializes_with_channel_ids() {
        let json = r#"{"bot_token":"xoxb-tok","channel_ids":["C111","D222"]}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.channel_ids, vec!["C111", "D222"]);
        assert!(parsed.allowed_users.is_empty());
        assert!(!parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn slack_config_deserializes_with_mention_only() {
        let json = r#"{"bot_token":"xoxb-tok","mention_only":true}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.mention_only);
        assert!(!parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
    }

    #[test]
    async fn slack_config_deserializes_interrupt_on_new_message() {
        let json = r#"{"bot_token":"xoxb-tok","interrupt_on_new_message":true}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn slack_config_deserializes_thread_replies() {
        let json = r#"{"bot_token":"xoxb-tok","thread_replies":false}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.thread_replies, Some(false));
        assert!(!parsed.interrupt_on_new_message);
        assert!(!parsed.mention_only);
    }

    /// Regression test for #6237 — before the fix, omitting `bot_token`
    /// from `[channels.slack]` made the entire config file fail to
    /// deserialize with `missing field 'bot_token'`, blocking startup
    /// even when `SLACK_BOT_TOKEN` was provided via the environment
    /// (the env-fallback never ran because deserialization aborted first).
    #[test]
    async fn slack_config_deserializes_without_bot_token() {
        let json = r#"{}"#;
        let parsed: SlackConfig = serde_json::from_str(json).expect(
            "SlackConfig must deserialize without bot_token so the env-var \
             fallback in apply_env_overrides has a chance to populate it",
        );
        assert!(parsed.bot_token.is_none());
        assert!(parsed.app_token.is_none());
    }

    #[test]
    async fn slack_config_deserializes_explicit_bot_token() {
        let json = r#"{"bot_token":"xoxb-from-toml"}"#;
        let parsed: SlackConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.bot_token.as_deref(), Some("xoxb-from-toml"));
    }

    /// `apply_env_overrides` populates `bot_token` from `SLACK_BOT_TOKEN`
    /// when the config field is `None`. This is the path that #6237
    /// reporters were trying to use — they had `SLACK_BOT_TOKEN` set in
    /// their environment but the schema rejected the config before the
    /// override could fire.
    #[test]
    async fn slack_apply_env_overrides_populates_bot_token_from_env() {
        let _env_guard = env_override_lock().await;
        // SAFETY: test-only, single-threaded test runner.
        unsafe {
            std::env::remove_var("OPERANT_SLACK_BOT_TOKEN");
            std::env::remove_var("OPERANT_SLACK_APP_TOKEN");
            std::env::remove_var("SLACK_APP_TOKEN");
            std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env");
        }

        let mut config = Config::default();
        config.channels.slack = Some(SlackConfig {
            bot_token: None,
            ..Default::default()
        });
        config.apply_env_overrides();

        assert_eq!(
            config
                .channels
                .slack
                .as_ref()
                .and_then(|s| s.bot_token.as_deref()),
            Some("xoxb-from-env")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("SLACK_BOT_TOKEN") };
    }

    /// The `OPERANT_SLACK_BOT_TOKEN` variant takes precedence over
    /// `SLACK_BOT_TOKEN` so workspace-scoped envs can override a
    /// generic one set on the host.
    #[test]
    async fn slack_apply_env_overrides_prefers_operant_prefix() {
        let _env_guard = env_override_lock().await;
        // SAFETY: test-only, single-threaded test runner.
        unsafe {
            std::env::set_var("SLACK_BOT_TOKEN", "xoxb-generic");
            std::env::set_var("OPERANT_SLACK_BOT_TOKEN", "xoxb-operant-prefix");
        }

        let mut config = Config::default();
        config.channels.slack = Some(SlackConfig {
            bot_token: None,
            ..Default::default()
        });
        config.apply_env_overrides();

        assert_eq!(
            config
                .channels
                .slack
                .as_ref()
                .and_then(|s| s.bot_token.as_deref()),
            Some("xoxb-operant-prefix")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe {
            std::env::remove_var("SLACK_BOT_TOKEN");
            std::env::remove_var("OPERANT_SLACK_BOT_TOKEN");
        }
    }

    /// `apply_env_overrides` must NOT clobber a config-supplied bot_token,
    /// otherwise users who set the value in `config.toml` would silently
    /// have it replaced by an env var they didn't intend to be authoritative.
    #[test]
    async fn slack_apply_env_overrides_preserves_config_supplied_bot_token() {
        let _env_guard = env_override_lock().await;
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("SLACK_BOT_TOKEN", "xoxb-from-env") };

        let mut config = Config::default();
        config.channels.slack = Some(SlackConfig {
            bot_token: Some("xoxb-from-toml".to_string()),
            ..Default::default()
        });
        config.apply_env_overrides();

        assert_eq!(
            config
                .channels
                .slack
                .as_ref()
                .and_then(|s| s.bot_token.as_deref()),
            Some("xoxb-from-toml"),
            "config-supplied bot_token must win over env var"
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("SLACK_BOT_TOKEN") };
    }

    #[test]
    async fn discord_config_default_interrupt_on_new_message_is_false() {
        let json = r#"{"bot_token":"tok"}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert!(!parsed.interrupt_on_new_message);
    }

    #[test]
    async fn discord_config_deserializes_interrupt_on_new_message_true() {
        let json = r#"{"bot_token":"tok","interrupt_on_new_message":true}"#;
        let parsed: DiscordConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.interrupt_on_new_message);
    }

    #[test]
    async fn discord_config_toml_backward_compat() {
        let toml_str = r#"
bot_token = "tok"
guild_id = "123"
"#;
        let parsed: DiscordConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.bot_token, "tok");
    }

    #[test]
    async fn slack_config_toml_with_channel_ids() {
        let toml_str = r#"
bot_token = "xoxb-tok"
channel_ids = ["C123", "D456"]
"#;
        let parsed: SlackConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.channel_ids, vec!["C123", "D456"]);
        assert!(parsed.allowed_users.is_empty());
        assert!(!parsed.interrupt_on_new_message);
        assert_eq!(parsed.thread_replies, None);
        assert!(!parsed.mention_only);
    }

    #[test]
    async fn slack_config_toml_without_channel_ids_defaults_empty() {
        let toml_str = r#"
bot_token = "xoxb-tok"
"#;
        let parsed: SlackConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.channel_ids.is_empty());
    }

    #[test]
    async fn mattermost_config_default_interrupt_on_new_message_is_false() {
        let json = r#"{"url":"https://mm.example.com","bot_token":"tok"}"#;
        let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
        assert!(!parsed.interrupt_on_new_message);
    }

    #[test]
    async fn mattermost_config_deserializes_interrupt_on_new_message_true() {
        let json =
            r#"{"url":"https://mm.example.com","bot_token":"tok","interrupt_on_new_message":true}"#;
        let parsed: MattermostConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.interrupt_on_new_message);
    }

    #[test]
    async fn webhook_config_with_secret() {
        let json = r#"{"port":8080,"secret":"my-secret-key"}"#;
        let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.secret.as_deref(), Some("my-secret-key"));
    }

    #[test]
    async fn webhook_config_without_secret() {
        let json = r#"{"port":8080}"#;
        let parsed: WebhookConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.secret.is_none());
        assert_eq!(parsed.port, 8080);
    }

    // ── WhatsApp config ──────────────────────────────────────

    #[test]
    async fn whatsapp_config_serde() {
        let wc = WhatsAppConfig {
            enabled: true,
            access_token: Some("EAABx...".into()),
            phone_number_id: Some("123456789".into()),
            verify_token: Some("my-verify-token".into()),
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1234567890".into(), "+9876543210".into()],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        let json = serde_json::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, Some("EAABx...".into()));
        assert_eq!(parsed.phone_number_id, Some("123456789".into()));
        assert_eq!(parsed.verify_token, Some("my-verify-token".into()));
        assert_eq!(parsed.allowed_numbers.len(), 2);
    }

    #[test]
    async fn whatsapp_config_toml_roundtrip() {
        let wc = WhatsAppConfig {
            enabled: true,
            access_token: Some("tok".into()),
            phone_number_id: Some("12345".into()),
            verify_token: Some("verify".into()),
            app_secret: Some("secret123".into()),
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1".into()],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        let toml_str = toml::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.phone_number_id, Some("12345".into()));
        assert_eq!(parsed.allowed_numbers, vec!["+1"]);
    }

    #[test]
    async fn whatsapp_config_deserializes_without_allowed_numbers() {
        let json = r#"{"access_token":"tok","phone_number_id":"123","verify_token":"ver"}"#;
        let parsed: WhatsAppConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.allowed_numbers.is_empty());
    }

    #[test]
    async fn whatsapp_config_wildcard_allowed() {
        let wc = WhatsAppConfig {
            enabled: true,
            access_token: Some("tok".into()),
            phone_number_id: Some("123".into()),
            verify_token: Some("ver".into()),
            app_secret: None,
            session_path: None,
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["*".into()],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        let toml_str = toml::to_string(&wc).unwrap();
        let parsed: WhatsAppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.allowed_numbers, vec!["*"]);
    }

    #[test]
    async fn whatsapp_config_backend_type_cloud_precedence_when_ambiguous() {
        let wc = WhatsAppConfig {
            enabled: true,
            access_token: Some("tok".into()),
            phone_number_id: Some("123".into()),
            verify_token: Some("ver".into()),
            app_secret: None,
            session_path: Some("~/.operant/state/whatsapp-web/session.db".into()),
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec!["+1".into()],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        assert!(wc.is_ambiguous_config());
        assert_eq!(wc.backend_type(), "cloud");
    }

    #[test]
    async fn whatsapp_config_backend_type_web() {
        let wc = WhatsAppConfig {
            enabled: true,
            access_token: None,
            phone_number_id: None,
            verify_token: None,
            app_secret: None,
            session_path: Some("~/.operant/state/whatsapp-web/session.db".into()),
            pair_phone: None,
            pair_code: None,
            allowed_numbers: vec![],
            mention_only: false,
            mode: WhatsAppWebMode::default(),
            dm_policy: WhatsAppChatPolicy::default(),
            group_policy: WhatsAppChatPolicy::default(),
            self_chat_mode: false,
            dm_mention_patterns: vec![],
            group_mention_patterns: vec![],
            proxy_url: None,
            approval_timeout_secs: 300,
        };
        assert!(!wc.is_ambiguous_config());
        assert_eq!(wc.backend_type(), "web");
    }

    #[test]
    async fn channels_with_whatsapp() {
        let c = ChannelsConfig {
            cli: true,
            telegram: None,
            discord: None,
            discord_history: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: Some(WhatsAppConfig {
                enabled: true,
                access_token: Some("tok".into()),
                phone_number_id: Some("123".into()),
                verify_token: Some("ver".into()),
                app_secret: None,
                session_path: None,
                pair_phone: None,
                pair_code: None,
                allowed_numbers: vec!["+1".into()],
                mention_only: false,
                mode: WhatsAppWebMode::default(),
                dm_policy: WhatsAppChatPolicy::default(),
                group_policy: WhatsAppChatPolicy::default(),
                self_chat_mode: false,
                dm_mention_patterns: vec![],
                group_mention_patterns: vec![],
                proxy_url: None,
                approval_timeout_secs: 300,
            }),
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            gmail_push: None,
            irc: None,
            lark: None,
            line: None,
            feishu: None,
            dingtalk: None,
            wecom: None,
            wechat: None,
            qq: None,
            twitter: None,
            mochat: None,
            #[cfg(feature = "channel-nostr")]
            nostr: None,
            clawdtalk: None,
            reddit: None,
            bluesky: None,
            voice_call: None,
            voice_duplex: None,
            #[cfg(feature = "voice-wake")]
            voice_wake: None,
            mqtt: None,
            message_timeout_secs: 300,
            ack_reactions: true,
            show_tool_calls: true,
            session_persistence: true,
            session_backend: default_session_backend(),
            session_ttl_hours: 0,
            debounce_ms: 0,
        };
        let toml_str = toml::to_string_pretty(&c).unwrap();
        let parsed: ChannelsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.whatsapp.is_some());
        let wa = parsed.whatsapp.unwrap();
        assert_eq!(wa.phone_number_id, Some("123".into()));
        assert_eq!(wa.allowed_numbers, vec!["+1"]);
    }

    #[test]
    async fn channels_default_has_no_whatsapp() {
        let c = ChannelsConfig::default();
        assert!(c.whatsapp.is_none());
    }

    #[test]
    async fn channels_default_has_no_nextcloud_talk() {
        let c = ChannelsConfig::default();
        assert!(c.nextcloud_talk.is_none());
    }

    // ══════════════════════════════════════════════════════════
    // SECURITY CHECKLIST TESTS — Gateway config
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn checklist_gateway_default_requires_pairing() {
        let g = GatewayConfig::default();
        assert!(g.require_pairing, "Pairing must be required by default");
    }

    #[test]
    async fn checklist_gateway_default_blocks_public_bind() {
        let g = GatewayConfig::default();
        assert!(
            !g.allow_public_bind,
            "Public bind must be blocked by default"
        );
    }

    #[test]
    async fn checklist_gateway_default_no_tokens() {
        let g = GatewayConfig::default();
        assert!(
            g.paired_tokens.is_empty(),
            "No pre-paired tokens by default"
        );
        assert_eq!(g.pair_rate_limit_per_minute, 10);
        assert_eq!(g.webhook_rate_limit_per_minute, 60);
        assert!(!g.trust_forwarded_headers);
        assert_eq!(g.rate_limit_max_keys, 10_000);
        assert_eq!(g.idempotency_ttl_secs, 300);
        assert_eq!(g.idempotency_max_keys, 10_000);
    }

    #[test]
    async fn checklist_gateway_cli_default_host_is_localhost() {
        // The CLI default for --host is 127.0.0.1 (checked in main.rs)
        // Here we verify the config default matches
        let c = Config::default();
        assert!(
            c.gateway.require_pairing,
            "Config default must require pairing"
        );
        assert!(
            !c.gateway.allow_public_bind,
            "Config default must block public bind"
        );
    }

    #[test]
    async fn checklist_gateway_serde_roundtrip() {
        let g = GatewayConfig {
            port: 42617,
            host: "127.0.0.1".into(),
            require_pairing: true,
            allow_public_bind: false,
            paired_tokens: vec!["zc_test_token".into()],
            pair_rate_limit_per_minute: 12,
            webhook_rate_limit_per_minute: 80,
            trust_forwarded_headers: true,
            path_prefix: Some("/operant".into()),
            rate_limit_max_keys: 2048,
            idempotency_ttl_secs: 600,
            idempotency_max_keys: 4096,
            session_persistence: true,
            session_ttl_hours: 0,
            pairing_dashboard: PairingDashboardConfig::default(),
            web_dist_dir: None,
            tls: None,
            platform_toolsets: HashMap::new(),
        };
        let toml_str = toml::to_string(&g).unwrap();
        let parsed: GatewayConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.require_pairing);
        assert!(parsed.session_persistence);
        assert_eq!(parsed.session_ttl_hours, 0);
        assert!(!parsed.allow_public_bind);
        assert_eq!(parsed.paired_tokens, vec!["zc_test_token"]);
        assert_eq!(parsed.pair_rate_limit_per_minute, 12);
        assert_eq!(parsed.webhook_rate_limit_per_minute, 80);
        assert!(parsed.trust_forwarded_headers);
        assert_eq!(parsed.path_prefix.as_deref(), Some("/operant"));
        assert_eq!(parsed.rate_limit_max_keys, 2048);
        assert_eq!(parsed.idempotency_ttl_secs, 600);
        assert_eq!(parsed.idempotency_max_keys, 4096);
    }

    #[test]
    async fn checklist_gateway_backward_compat_no_gateway_section() {
        // Old configs without [gateway] should get secure defaults
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(minimal);
        assert!(
            parsed.gateway.require_pairing,
            "Missing [gateway] must default to require_pairing=true"
        );
        assert!(
            !parsed.gateway.allow_public_bind,
            "Missing [gateway] must default to allow_public_bind=false"
        );
    }

    #[test]
    async fn checklist_autonomy_default_is_workspace_scoped() {
        let a = AutonomyConfig::default();
        assert!(a.workspace_only, "Default autonomy must be workspace_only");
        assert!(
            a.forbidden_paths.contains(&"/etc".to_string()),
            "Must block /etc"
        );
        assert!(
            a.forbidden_paths.contains(&"/proc".to_string()),
            "Must block /proc"
        );
        assert!(
            a.forbidden_paths.contains(&"~/.ssh".to_string()),
            "Must block ~/.ssh"
        );
    }

    // ══════════════════════════════════════════════════════════
    // COMPOSIO CONFIG TESTS
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn composio_config_default_disabled() {
        let c = ComposioConfig::default();
        assert!(!c.enabled, "Composio must be disabled by default");
        assert!(c.api_key.is_none(), "No API key by default");
        assert_eq!(c.entity_id, "default");
    }

    #[test]
    async fn composio_config_serde_roundtrip() {
        let c = ComposioConfig {
            enabled: true,
            api_key: Some("comp-key-123".into()),
            entity_id: "user42".into(),
        };
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: ComposioConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.api_key.as_deref(), Some("comp-key-123"));
        assert_eq!(parsed.entity_id, "user42");
    }

    #[test]
    async fn composio_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(minimal);
        assert!(
            !parsed.composio.enabled,
            "Missing [composio] must default to disabled"
        );
        assert!(parsed.composio.api_key.is_none());
    }

    #[test]
    async fn composio_config_partial_toml() {
        let toml_str = r"
enabled = true
";
        let parsed: ComposioConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.api_key.is_none());
        assert_eq!(parsed.entity_id, "default");
    }

    #[test]
    async fn composio_config_enable_alias_supported() {
        let toml_str = r"
enable = true
";
        let parsed: ComposioConfig = toml::from_str(toml_str).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.api_key.is_none());
        assert_eq!(parsed.entity_id, "default");
    }

    // ══════════════════════════════════════════════════════════
    // SECRETS CONFIG TESTS
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn secrets_config_default_encrypts() {
        let s = SecretsConfig::default();
        assert!(s.encrypt, "Encryption must be enabled by default");
    }

    #[test]
    async fn secrets_config_serde_roundtrip() {
        let s = SecretsConfig { encrypt: false };
        let toml_str = toml::to_string(&s).unwrap();
        let parsed: SecretsConfig = toml::from_str(&toml_str).unwrap();
        assert!(!parsed.encrypt);
    }

    #[test]
    async fn secrets_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(minimal);
        assert!(
            parsed.secrets.encrypt,
            "Missing [secrets] must default to encrypt=true"
        );
    }

    #[test]
    async fn config_default_has_composio_and_secrets() {
        let c = Config::default();
        assert!(!c.composio.enabled);
        assert!(c.composio.api_key.is_none());
        assert!(c.secrets.encrypt);
        assert!(c.browser.enabled);
        assert_eq!(c.browser.allowed_domains, vec!["*".to_string()]);
    }

    #[test]
    async fn browser_config_default_enabled() {
        let b = BrowserConfig::default();
        assert!(b.enabled);
        assert_eq!(b.allowed_domains, vec!["*".to_string()]);
        assert_eq!(b.backend, "agent_browser");
        assert!(b.native_headless);
        assert_eq!(b.native_webdriver_url, "http://127.0.0.1:9515");
        assert!(b.native_chrome_path.is_none());
        assert_eq!(b.computer_use.endpoint, "http://127.0.0.1:8787/v1/actions");
        assert_eq!(b.computer_use.timeout_ms, 15_000);
        assert!(!b.computer_use.allow_remote_endpoint);
        assert!(b.computer_use.window_allowlist.is_empty());
        assert!(b.computer_use.max_coordinate_x.is_none());
        assert!(b.computer_use.max_coordinate_y.is_none());
    }

    #[test]
    async fn browser_config_serde_roundtrip() {
        let b = BrowserConfig {
            enabled: true,
            allowed_domains: vec!["example.com".into(), "docs.example.com".into()],
            session_name: None,
            backend: "auto".into(),
            native_headless: false,
            native_webdriver_url: "http://localhost:4444".into(),
            native_chrome_path: Some("/usr/bin/chromium".into()),
            computer_use: BrowserComputerUseConfig {
                endpoint: "https://computer-use.example.com/v1/actions".into(),
                api_key: Some("test-token".into()),
                timeout_ms: 8_000,
                allow_remote_endpoint: true,
                window_allowlist: vec!["Chrome".into(), "Visual Studio Code".into()],
                max_coordinate_x: Some(3840),
                max_coordinate_y: Some(2160),
            },
        };
        let toml_str = toml::to_string(&b).unwrap();
        let parsed: BrowserConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.allowed_domains.len(), 2);
        assert_eq!(parsed.allowed_domains[0], "example.com");
        assert_eq!(parsed.backend, "auto");
        assert!(!parsed.native_headless);
        assert_eq!(parsed.native_webdriver_url, "http://localhost:4444");
        assert_eq!(
            parsed.native_chrome_path.as_deref(),
            Some("/usr/bin/chromium")
        );
        assert_eq!(
            parsed.computer_use.endpoint,
            "https://computer-use.example.com/v1/actions"
        );
        assert_eq!(parsed.computer_use.api_key.as_deref(), Some("test-token"));
        assert_eq!(parsed.computer_use.timeout_ms, 8_000);
        assert!(parsed.computer_use.allow_remote_endpoint);
        assert_eq!(parsed.computer_use.window_allowlist.len(), 2);
        assert_eq!(parsed.computer_use.max_coordinate_x, Some(3840));
        assert_eq!(parsed.computer_use.max_coordinate_y, Some(2160));
    }

    #[test]
    async fn browser_config_backward_compat_missing_section() {
        let minimal = r#"
workspace_dir = "/tmp/ws"
config_path = "/tmp/config.toml"
default_temperature = 0.7
"#;
        let parsed = parse_test_config(minimal);
        assert!(parsed.browser.enabled);
        assert_eq!(parsed.browser.allowed_domains, vec!["*".to_string()]);
    }

    // ── Environment variable overrides (Docker support) ─────────

    async fn env_override_lock() -> MutexGuard<'static, ()> {
        static ENV_OVERRIDE_TEST_LOCK: Mutex<()> = Mutex::const_new(());
        ENV_OVERRIDE_TEST_LOCK.lock().await
    }

    fn clear_proxy_env_test_vars() {
        for key in [
            "OPERANT_PROXY_ENABLED",
            "OPERANT_HTTP_PROXY",
            "OPERANT_HTTPS_PROXY",
            "OPERANT_ALL_PROXY",
            "OPERANT_NO_PROXY",
            "OPERANT_PROXY_SCOPE",
            "OPERANT_PROXY_SERVICES",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    async fn env_override_api_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_ref())
                .is_none()
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_API_KEY", "sk-test-env-key") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref()),
            Some("sk-test-env-key")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_API_KEY") };
    }

    #[test]
    async fn env_override_api_key_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_API_KEY") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("API_KEY", "sk-fallback-key") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref()),
            Some("sk-fallback-key")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("API_KEY") };
    }

    #[test]
    async fn env_override_provider() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROVIDER", "anthropic") };
        config.apply_env_overrides();
        assert_eq!(config.providers.fallback.as_deref(), Some("anthropic"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    }

    #[test]
    async fn env_override_model_provider_alias() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_MODEL_PROVIDER", "openai-codex") };
        config.apply_env_overrides();
        assert_eq!(config.providers.fallback.as_deref(), Some("openai-codex"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_MODEL_PROVIDER") };
    }

    #[test]
    async fn toml_supports_model_provider_and_model_alias_fields() {
        let raw = r#"
default_temperature = 0.7
model_provider = "sub2api"
model = "gpt-5.3-codex"

[model_providers.sub2api]
name = "sub2api"
base_url = "https://api.tonsof.blue/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

        let parsed = parse_test_config(raw);
        assert_eq!(parsed.providers.fallback.as_deref(), Some("sub2api"));
        assert_eq!(
            parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("gpt-5.3-codex")
        );
        let profile = parsed
            .providers
            .models
            .get("sub2api")
            .expect("profile should exist");
        assert_eq!(profile.wire_api.as_deref(), Some("responses"));
        assert!(profile.requires_openai_auth);
    }

    #[test]
    async fn env_override_open_skills_enabled_and_dir() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert!(!config.skills.open_skills_enabled);
        assert!(config.skills.open_skills_dir.is_none());
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Full
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_ENABLED", "true") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_DIR", "/tmp/open-skills") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_SKILLS_ALLOW_SCRIPTS", "yes") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_SKILLS_PROMPT_MODE", "compact") };
        config.apply_env_overrides();

        assert!(config.skills.open_skills_enabled);
        assert!(config.skills.allow_scripts);
        assert_eq!(
            config.skills.open_skills_dir.as_deref(),
            Some("/tmp/open-skills")
        );
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_ENABLED") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_DIR") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_SKILLS_ALLOW_SCRIPTS") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_SKILLS_PROMPT_MODE") };
    }

    #[test]
    async fn env_override_open_skills_enabled_invalid_value_keeps_existing_value() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.skills.open_skills_enabled = true;
        config.skills.allow_scripts = true;
        config.skills.prompt_injection_mode = SkillsPromptInjectionMode::Compact;

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_OPEN_SKILLS_ENABLED", "maybe") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_SKILLS_ALLOW_SCRIPTS", "maybe") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_SKILLS_PROMPT_MODE", "invalid") };
        config.apply_env_overrides();

        assert!(config.skills.open_skills_enabled);
        assert!(config.skills.allow_scripts);
        assert_eq!(
            config.skills.prompt_injection_mode,
            SkillsPromptInjectionMode::Compact
        );
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_OPEN_SKILLS_ENABLED") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_SKILLS_ALLOW_SCRIPTS") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_SKILLS_PROMPT_MODE") };
    }

    #[test]
    async fn env_override_provider_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("PROVIDER", "openai") };
        config.apply_env_overrides();
        assert_eq!(config.providers.fallback.as_deref(), Some("openai"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("PROVIDER") };
    }

    #[test]
    async fn env_override_provider_fallback_does_not_replace_non_default_provider() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("custom:https://proxy.example.com/v1".to_string());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("PROVIDER", "openrouter") };
        config.apply_env_overrides();
        assert_eq!(
            config.providers.fallback.as_deref(),
            Some("custom:https://proxy.example.com/v1")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("PROVIDER") };
    }

    #[test]
    async fn env_override_zero_claw_provider_overrides_non_default_provider() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("custom:https://proxy.example.com/v1".to_string());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROVIDER", "openrouter") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("PROVIDER", "anthropic") };
        config.apply_env_overrides();
        assert_eq!(config.providers.fallback.as_deref(), Some("openrouter"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("PROVIDER") };
    }

    #[test]
    async fn env_override_glm_api_key_for_regional_aliases() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("glm-cn".to_string());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("GLM_API_KEY", "glm-regional-key") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref()),
            Some("glm-regional-key")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("GLM_API_KEY") };
    }

    #[test]
    async fn env_override_zai_api_key_for_regional_aliases() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("zai-cn".to_string());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("ZAI_API_KEY", "zai-regional-key") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.api_key.as_deref()),
            Some("zai-regional-key")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("ZAI_API_KEY") };
    }

    #[test]
    async fn env_override_model() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_MODEL", "gpt-4o") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("gpt-4o")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_MODEL") };
    }

    #[test]
    async fn model_provider_profile_maps_to_custom_endpoint() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("sub2api".to_string());
        config.providers.models.insert(
            "sub2api".to_string(),
            ModelProviderConfig {
                name: Some("sub2api".to_string()),
                base_url: Some("https://api.tonsof.blue/v1".to_string()),
                wire_api: None,
                requires_openai_auth: false,
                azure_openai_resource: None,
                azure_openai_deployment: None,
                azure_openai_api_version: None,
                api_path: None,
                max_tokens: None,
                ..Default::default()
            },
        );

        config.apply_env_overrides();
        // The user's literal fallback key is preserved; we no longer rewrite it
        // to a canonical alias. This is the round-trip-safe contract that the
        // `operant config get/set` CLI relies on.
        assert_eq!(config.providers.fallback.as_deref(), Some("sub2api"));
        // The original entry is still stored under its config key.
        assert_eq!(
            config
                .providers
                .models
                .get("sub2api")
                .and_then(|e| e.base_url.as_deref()),
            Some("https://api.tonsof.blue/v1")
        );
        // The entry is also mirrored under the canonical alias key so runtime
        // lookups by `custom:<base_url>` still resolve even though the user's
        // fallback string is the original profile key.
        assert_eq!(
            config
                .providers
                .models
                .get("custom:https://api.tonsof.blue/v1")
                .and_then(|e| e.base_url.as_deref()),
            Some("https://api.tonsof.blue/v1")
        );
        assert!(config.providers.fallback_provider().is_some());
    }

    #[test]
    async fn model_provider_profile_responses_uses_openai_codex_and_openai_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("sub2api".to_string());
        config.providers.models.insert(
            "sub2api".to_string(),
            ModelProviderConfig {
                name: Some("sub2api".to_string()),
                base_url: Some("https://api.tonsof.blue".to_string()),
                wire_api: Some("responses".to_string()),
                requires_openai_auth: true,
                azure_openai_resource: None,
                azure_openai_deployment: None,
                azure_openai_api_version: None,
                api_path: None,
                max_tokens: None,
                ..Default::default()
            },
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPENAI_API_KEY", "sk-test-codex-key") };
        config.apply_env_overrides();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPENAI_API_KEY") };

        // The user's literal fallback key is preserved; we no longer rewrite it
        // to "openai-codex". The Codex-app-server compatibility shim instead
        // mirrors the resolved entry under that alias key for runtime lookups.
        assert_eq!(config.providers.fallback.as_deref(), Some("sub2api"));
        // The original entry is still stored under its config key.
        let entry = config
            .providers
            .models
            .get("sub2api")
            .expect("sub2api entry");
        assert_eq!(entry.base_url.as_deref(), Some("https://api.tonsof.blue"));
        assert_eq!(entry.api_key.as_deref(), Some("sk-test-codex-key"));
        // The entry is mirrored under the "openai-codex" alias so any code
        // path that looks providers up by that canonical key still finds it.
        let aliased = config
            .providers
            .models
            .get("openai-codex")
            .expect("openai-codex alias entry");
        assert_eq!(aliased.base_url.as_deref(), Some("https://api.tonsof.blue"));
        assert_eq!(aliased.api_key.as_deref(), Some("sk-test-codex-key"));
    }

    /// Regression test for the config CLI get/set divergence bug.
    ///
    /// Before the fix, `apply_named_model_provider_profile` rewrote
    /// `self.providers.fallback` to the profile's `name` field whenever they
    /// differed. That meant:
    ///
    /// - `operant config get providers.fallback` returned the rewritten value
    ///   even though the on-disk TOML still held the user's literal key.
    /// - `operant config set providers.fallback <new>` would persist `<new>`
    ///   to disk, but the next load mutated it back in memory, so a subsequent
    ///   `get` reported a stale value and the daemon's resolver looked up a
    ///   provider key that did not exist in `[providers.models.*]`.
    ///
    /// The fix preserves the literal fallback key end-to-end. The named-profile
    /// shim now only mirrors the resolved entry under canonical alias keys for
    /// runtime lookup convenience.
    #[test]
    async fn apply_env_overrides_preserves_user_supplied_fallback_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        // User configures fallback = "primary" with a profile whose `name` field
        // differs from the key. This is the exact shape that triggered the bug.
        config.providers.fallback = Some("primary".to_string());
        config.providers.models.insert(
            "primary".to_string(),
            ModelProviderConfig {
                name: Some("alias-name".to_string()),
                base_url: Some("https://example.invalid/v1".to_string()),
                model: Some("primary-model".to_string()),
                ..Default::default()
            },
        );

        config.apply_env_overrides();

        // The literal user key must survive. This is what `config get` returns
        // and what `config set` persists.
        assert_eq!(
            config.providers.fallback.as_deref(),
            Some("primary"),
            "providers.fallback must preserve the user's literal key after \
             apply_env_overrides; got {:?}",
            config.providers.fallback,
        );
        // Runtime resolution must still find the entry under the original key.
        assert!(
            config.providers.fallback_provider().is_some(),
            "fallback_provider() must still resolve via the user's literal key",
        );
    }

    /// Round-trip test for the config CLI: a TOML file with the user's value
    /// must deserialize, apply env overrides, and serialize back to the same
    /// `providers.fallback`. This is the full path that backed the user-visible
    /// `config set` -> `config get` divergence.
    #[test]
    async fn fallback_round_trips_through_load_apply_serialize() {
        let _env_guard = env_override_lock().await;
        let toml_in = r#"
schema_version = 1

[providers]
fallback = "primary"

[providers.models.primary]
name = "alias-name"
base_url = "https://example.invalid/v1"
model = "primary-model"
"#;

        let mut config: Config = toml::from_str(toml_in).expect("parse toml");
        config.apply_env_overrides();

        // What `config get providers.fallback` returns post-load.
        assert_eq!(
            config.get_prop("providers.fallback").unwrap(),
            "primary",
            "config get providers.fallback must return the user's literal value",
        );

        // What `config save` would write back to disk.
        let toml_out = toml::to_string(&config).expect("serialize toml");
        assert!(
            toml_out.contains(r#"fallback = "primary""#),
            "serialized config must keep fallback = \"primary\"; got:\n{toml_out}",
        );
    }

    /// `set_prop` followed by `get_prop` must return the value that was set,
    /// even when the surrounding profile shape would historically have caused
    /// the in-memory value to be rewritten.
    #[test]
    async fn set_prop_then_get_prop_round_trips_for_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.models.insert(
            "primary".to_string(),
            ModelProviderConfig {
                name: Some("alias-name".to_string()),
                model: Some("primary-model".to_string()),
                ..Default::default()
            },
        );
        // Simulate the daemon's load path before the user runs `config set`.
        config.apply_env_overrides();

        config.set_prop("providers.fallback", "primary").unwrap();
        // Mimic any post-set normalization a future codepath might add.
        config.apply_env_overrides();

        assert_eq!(config.get_prop("providers.fallback").unwrap(), "primary");
    }

    /// `resolve_default_model` returns the fallback provider's model when set,
    /// and falls through to the first available `models.*` entry otherwise.
    /// Returning `None` is reserved for "no provider has any model configured",
    /// which callers must surface as a configuration error rather than silently
    /// substituting a vendor default.
    #[test]
    async fn resolve_default_model_prefers_fallback_then_first_available() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        // Empty config: no model anywhere -> None (caller errors loudly).
        assert_eq!(config.providers.resolve_default_model(), None);

        // Add an entry without a model -> still None.
        config
            .providers
            .models
            .insert("secondary".to_string(), ModelProviderConfig::default());
        assert_eq!(config.providers.resolve_default_model(), None);

        // Add an entry with a model -> first-available wins when no fallback.
        config.providers.models.insert(
            "tertiary".to_string(),
            ModelProviderConfig {
                model: Some("tertiary-model".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(
            config.providers.resolve_default_model().as_deref(),
            Some("tertiary-model"),
        );

        // Set fallback to a provider with its own model -> fallback wins.
        config.providers.fallback = Some("primary".to_string());
        config.providers.models.insert(
            "primary".to_string(),
            ModelProviderConfig {
                model: Some("primary-model".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(
            config.providers.resolve_default_model().as_deref(),
            Some("primary-model"),
        );
    }

    #[test]
    async fn save_repairs_bare_config_filename_using_runtime_resolution() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("workspace");
        let resolved_config_path = temp_home.join(".operant").join("config.toml");

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

        let mut config = Config {
            workspace_dir,
            config_path: PathBuf::from("config.toml"),
            ..Default::default()
        };
        config.providers.fallback = Some("default".into());
        config.providers.models.insert(
            "default".into(),
            ModelProviderConfig {
                temperature: Some(0.5),
                ..Default::default()
            },
        );
        // Provider fields are now resolved directly — no cache needed.
        config.save().await.unwrap();

        assert!(resolved_config_path.exists());
        let saved = tokio::fs::read_to_string(&resolved_config_path)
            .await
            .unwrap();
        let parsed = parse_test_config(&saved);
        assert!(
            (parsed
                .providers
                .fallback_provider()
                .and_then(|e| e.temperature)
                .unwrap_or(0.7)
                - 0.5)
                .abs()
                < f64::EPSILON
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = tokio::fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn validate_ollama_cloud_model_requires_remote_api_url() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("ollama".to_string());
        config.providers.models.insert(
            "ollama".to_string(),
            ModelProviderConfig {
                model: Some("glm-5:cloud".to_string()),
                base_url: None,
                api_key: Some("ollama-key".to_string()),
                ..Default::default()
            },
        );

        let error = config.validate().expect_err("expected validation to fail");
        assert!(error.to_string().contains(
            "default_model uses ':cloud' with provider 'ollama', but api_url is local or unset"
        ));
    }

    #[test]
    async fn validate_ollama_cloud_model_accepts_remote_endpoint_and_env_key() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("ollama".to_string());
        config.providers.models.insert(
            "ollama".to_string(),
            ModelProviderConfig {
                model: Some("glm-5:cloud".to_string()),
                base_url: Some("https://ollama.com/api".to_string()),
                api_key: None,
                ..Default::default()
            },
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OLLAMA_API_KEY", "ollama-env-key") };
        let result = config.validate();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OLLAMA_API_KEY") };

        assert!(result.is_ok(), "expected validation to pass: {result:?}");
    }

    #[test]
    async fn validate_rejects_unknown_model_provider_wire_api() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.providers.fallback = Some("sub2api".to_string());
        config.providers.models.insert(
            "sub2api".to_string(),
            ModelProviderConfig {
                name: Some("sub2api".to_string()),
                base_url: Some("https://api.tonsof.blue/v1".to_string()),
                wire_api: Some("ws".to_string()),
                requires_openai_auth: false,
                azure_openai_resource: None,
                azure_openai_deployment: None,
                azure_openai_api_version: None,
                api_path: None,
                max_tokens: None,
                ..Default::default()
            },
        );

        let error = config.validate().expect_err("expected validation failure");
        assert!(
            error
                .to_string()
                .contains("wire_api must be one of: responses, chat_completions")
        );
    }

    #[test]
    async fn env_override_model_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_MODEL") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("MODEL", "anthropic/claude-3.5-sonnet") };
        config.apply_env_overrides();
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("anthropic/claude-3.5-sonnet")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("MODEL") };
    }

    #[test]
    async fn env_override_workspace() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", "/custom/workspace") };
        config.apply_env_overrides();
        assert_eq!(config.workspace_dir, PathBuf::from("/custom/workspace"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_env_workspace_first() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let workspace_dir = default_config_dir.join("profile-a");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };
        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::EnvWorkspace);
        assert_eq!(config_dir, workspace_dir);
        assert_eq!(resolved_workspace_dir, workspace_dir.join("workspace"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_env_config_dir_first() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let explicit_config_dir = default_config_dir.join("explicit-config");
        let marker_config_dir = default_config_dir.join("profiles").join("alpha");
        let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        fs::create_dir_all(&default_config_dir).await.unwrap();
        let state = ActiveWorkspaceState {
            config_dir: marker_config_dir.to_string_lossy().into_owned(),
        };
        fs::write(&state_path, toml::to_string(&state).unwrap())
            .await
            .unwrap();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_CONFIG_DIR", &explicit_config_dir) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::EnvConfigDir);
        assert_eq!(config_dir, explicit_config_dir);
        assert_eq!(
            resolved_workspace_dir,
            explicit_config_dir.join("workspace")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };
        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_uses_active_workspace_marker() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");
        let marker_config_dir = default_config_dir.join("profiles").join("alpha");
        let state_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        fs::create_dir_all(&default_config_dir).await.unwrap();
        let state = ActiveWorkspaceState {
            config_dir: marker_config_dir.to_string_lossy().into_owned(),
        };
        fs::write(&state_path, toml::to_string(&state).unwrap())
            .await
            .unwrap();

        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::ActiveWorkspaceMarker);
        assert_eq!(config_dir, marker_config_dir);
        assert_eq!(resolved_workspace_dir, marker_config_dir.join("workspace"));

        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn resolve_runtime_config_dirs_falls_back_to_default_layout() {
        let _env_guard = env_override_lock().await;
        let default_config_dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let default_workspace_dir = default_config_dir.join("workspace");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        let (config_dir, resolved_workspace_dir, source) =
            resolve_runtime_config_dirs(&default_config_dir, &default_workspace_dir)
                .await
                .unwrap();

        assert_eq!(source, ConfigResolutionSource::DefaultConfigDir);
        assert_eq!(config_dir, default_config_dir);
        assert_eq!(resolved_workspace_dir, default_workspace_dir);

        let _ = fs::remove_dir_all(default_config_dir).await;
    }

    #[test]
    async fn default_path_under_config_dir_respects_operant_config_dir() {
        let _env_guard = env_override_lock().await;
        let custom_dir = std::env::temp_dir().join("operant-test-profile");
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_CONFIG_DIR", &custom_dir) };

        let result = default_path_under_config_dir("knowledge.db");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_CONFIG_DIR") };

        assert_eq!(
            result,
            custom_dir.join("knowledge.db").to_string_lossy().as_ref(),
            "expected path under OPERANT_CONFIG_DIR, got: {result}"
        );
    }

    #[test]
    async fn load_or_init_workspace_override_uses_workspace_root_for_config() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("profile-a");

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir.join("workspace"));
        assert_eq!(config.config_path, workspace_dir.join("config.toml"));
        assert!(workspace_dir.join("config.toml").exists());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_workspace_suffix_uses_legacy_config_layout() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("workspace");
        let legacy_config_path = temp_home.join(".operant").join("config.toml");

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, legacy_config_path);
        assert!(config.config_path.exists());

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_workspace_override_keeps_existing_legacy_config() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("custom-workspace");
        let legacy_config_dir = temp_home.join(".operant");
        let legacy_config_path = legacy_config_dir.join("config.toml");

        fs::create_dir_all(&legacy_config_dir).await.unwrap();
        fs::write(
            &legacy_config_path,
            r#"default_temperature = 0.7
default_model = "legacy-model"
"#,
        )
        .await
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        assert_eq!(config.workspace_dir, workspace_dir);
        assert_eq!(config.config_path, legacy_config_path);
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("legacy-model")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_decrypts_feishu_channel_secrets() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let config_dir = temp_home.join(".operant");
        let config_path = config_dir.join("config.toml");

        fs::create_dir_all(&config_dir).await.unwrap();

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

        let mut config = Config {
            config_path: config_path.clone(),
            workspace_dir: config_dir.join("workspace"),
            ..Default::default()
        };
        config.secrets.encrypt = true;
        config.channels.feishu = Some(FeishuConfig {
            enabled: true,
            app_id: "cli_feishu_123".into(),
            app_secret: "feishu-secret".into(),
            encrypt_key: Some("feishu-encrypt".into()),
            verification_token: Some("feishu-verify".into()),
            allowed_users: vec!["*".into()],
            mention_only: false,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            proxy_url: None,
        });
        config.save().await.unwrap();

        let loaded = Box::pin(Config::load_or_init()).await.unwrap();
        let feishu = loaded.channels.feishu.as_ref().unwrap();
        assert_eq!(feishu.app_secret, "feishu-secret");
        assert_eq!(feishu.encrypt_key.as_deref(), Some("feishu-encrypt"));
        assert_eq!(feishu.verification_token.as_deref(), Some("feishu-verify"));

        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_uses_persisted_active_workspace_marker() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let temp_default_dir = temp_home.join(".operant");
        let custom_config_dir = temp_home.join("profiles").join("agent-alpha");

        fs::create_dir_all(&custom_config_dir).await.unwrap();
        // Pre-create the default dir so is_temp_directory() can canonicalize
        // the path on macOS (where /var → /private/var symlink requires
        // the directory to exist for canonicalize to resolve correctly).
        fs::create_dir_all(&temp_default_dir).await.unwrap();
        fs::write(
            custom_config_dir.join("config.toml"),
            "default_temperature = 0.7\ndefault_model = \"persisted-profile\"\n",
        )
        .await
        .unwrap();

        // Write the marker using the explicit default dir (no HOME manipulation
        // needed for the persist call itself).
        persist_active_workspace_config_dir_in(&custom_config_dir, &temp_default_dir)
            .await
            .unwrap();

        // Config::load_or_init still reads HOME to find the marker, so we
        // must override HOME here. The persist above already wrote to the
        // correct temp location, so no stale marker can leak.
        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        assert_eq!(config.config_path, custom_config_dir.join("config.toml"));
        assert_eq!(config.workspace_dir, custom_config_dir.join("workspace"));
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("persisted-profile")
        );

        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn load_or_init_env_workspace_override_takes_priority_over_marker() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let temp_default_dir = temp_home.join(".operant");
        let marker_config_dir = temp_home.join("profiles").join("persisted-profile");
        let env_workspace_dir = temp_home.join("env-workspace");

        fs::create_dir_all(&marker_config_dir).await.unwrap();
        fs::write(
            marker_config_dir.join("config.toml"),
            "default_temperature = 0.7\ndefault_model = \"marker-model\"\n",
        )
        .await
        .unwrap();

        // Write marker via explicit default dir, then set HOME for load_or_init.
        persist_active_workspace_config_dir_in(&marker_config_dir, &temp_default_dir)
            .await
            .unwrap();

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &env_workspace_dir) };

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        assert_eq!(config.workspace_dir, env_workspace_dir.join("workspace"));
        assert_eq!(config.config_path, env_workspace_dir.join("config.toml"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn persist_active_workspace_marker_is_cleared_for_default_config_dir() {
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let default_config_dir = temp_home.join(".operant");
        let custom_config_dir = temp_home.join("profiles").join("custom-profile");
        let marker_path = default_config_dir.join(ACTIVE_WORKSPACE_STATE_FILE);

        // Use the _in variant directly -- no HOME manipulation needed since
        // this test only exercises persist/clear logic, not Config::load_or_init.
        persist_active_workspace_config_dir_in(&custom_config_dir, &default_config_dir)
            .await
            .unwrap();
        assert!(marker_path.exists());

        persist_active_workspace_config_dir_in(&default_config_dir, &default_config_dir)
            .await
            .unwrap();
        assert!(!marker_path.exists());

        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    #[allow(clippy::large_futures)]
    async fn load_or_init_logs_existing_config_as_initialized() {
        let _env_guard = env_override_lock().await;
        let temp_home =
            std::env::temp_dir().join(format!("operant_test_home_{}", uuid::Uuid::new_v4()));
        let workspace_dir = temp_home.join("profile-a");
        let config_path = workspace_dir.join("config.toml");

        fs::create_dir_all(&workspace_dir).await.unwrap();
        fs::write(
            &config_path,
            r#"default_temperature = 0.7
default_model = "persisted-profile"
"#,
        )
        .await
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOME", &temp_home) };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_WORKSPACE", &workspace_dir) };

        let capture = SharedLogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_writer(capture.clone())
            .finish();
        let dispatch = tracing::Dispatch::new(subscriber);
        let guard = tracing::dispatcher::set_default(&dispatch);

        let config = Box::pin(Config::load_or_init()).await.unwrap();

        drop(guard);
        let logs = capture.captured();

        assert_eq!(config.workspace_dir, workspace_dir.join("workspace"));
        assert_eq!(config.config_path, config_path);
        assert_eq!(
            config
                .providers
                .fallback_provider()
                .and_then(|e| e.model.as_deref()),
            Some("persisted-profile")
        );
        assert!(logs.contains("Config loaded"), "{logs}");
        assert!(logs.contains("initialized=true"), "{logs}");
        assert!(!logs.contains("initialized=false"), "{logs}");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_WORKSPACE") };
        if let Some(home) = original_home {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::set_var("HOME", home) };
        } else {
            // SAFETY: test-only, single-threaded test runner.
            unsafe { std::env::remove_var("HOME") };
        }
        let _ = fs::remove_dir_all(temp_home).await;
    }

    #[test]
    async fn env_override_empty_values_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_provider = config.providers.fallback.clone();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROVIDER", "") };
        config.apply_env_overrides();
        assert_eq!(config.providers.fallback, original_provider);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_PROVIDER") };
    }

    #[test]
    async fn env_override_gateway_port() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.gateway.port, 42617);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_GATEWAY_PORT", "8080") };
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, 8080);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_PORT") };
    }

    #[test]
    async fn env_override_port_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_PORT") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("PORT", "9000") };
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, 9000);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("PORT") };
    }

    #[test]
    async fn env_override_gateway_host() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.gateway.host, "127.0.0.1");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_GATEWAY_HOST", "0.0.0.0") };
        config.apply_env_overrides();
        assert_eq!(config.gateway.host, "0.0.0.0");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_HOST") };
    }

    #[test]
    async fn env_override_host_fallback() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_GATEWAY_HOST") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("HOST", "0.0.0.0") };
        config.apply_env_overrides();
        assert_eq!(config.gateway.host, "0.0.0.0");

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("HOST") };
    }

    #[test]
    async fn env_override_require_pairing() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert!(config.gateway.require_pairing);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REQUIRE_PAIRING", "false") };
        config.apply_env_overrides();
        assert!(!config.gateway.require_pairing);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REQUIRE_PAIRING", "true") };
        config.apply_env_overrides();
        assert!(config.gateway.require_pairing);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_REQUIRE_PAIRING") };
    }

    #[test]
    async fn env_override_temperature() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_TEMPERATURE", "0.5") };
        config.apply_env_overrides();
        assert!(
            (config
                .providers
                .fallback_provider()
                .and_then(|e| e.temperature)
                .unwrap_or(0.7)
                - 0.5)
                .abs()
                < f64::EPSILON
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };
    }

    #[test]
    async fn env_override_temperature_out_of_range_ignored() {
        let _env_guard = env_override_lock().await;
        // Clean up any leftover env vars from other tests
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };

        let mut config = Config::default();
        let original_temp = config
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7);

        // Temperature > 2.0 should be ignored
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_TEMPERATURE", "3.0") };
        config.apply_env_overrides();
        assert!(
            (config
                .providers
                .fallback_provider()
                .and_then(|e| e.temperature)
                .unwrap_or(0.7)
                - original_temp)
                .abs()
                < f64::EPSILON,
            "Temperature 3.0 should be ignored (out of range)"
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_TEMPERATURE") };
    }

    #[test]
    async fn validate_rejects_out_of_range_temperature() {
        let mut config = Config::default();
        config.providers.fallback = Some("test".into());
        config.providers.models.insert(
            "test".into(),
            ModelProviderConfig {
                name: Some("test-provider".into()),
                temperature: Some(99.0),
                ..Default::default()
            },
        );
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("temperature"),
            "expected temperature validation error, got: {err}"
        );
    }

    #[test]
    async fn validate_rejects_negative_temperature() {
        let mut config = Config::default();
        config.providers.fallback = Some("test".into());
        config.providers.models.insert(
            "test".into(),
            ModelProviderConfig {
                name: Some("test-provider".into()),
                temperature: Some(-0.5),
                ..Default::default()
            },
        );
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("temperature"),
            "expected temperature validation error, got: {err}"
        );
    }

    #[test]
    async fn validate_accepts_valid_temperature() {
        let mut config = Config::default();
        config.providers.fallback = Some("test".into());
        config.providers.models.insert(
            "test".into(),
            ModelProviderConfig {
                name: Some("test-provider".into()),
                temperature: Some(0.7),
                ..Default::default()
            },
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    async fn validate_rejects_precheck_timeout_zero() {
        let mut config = Config::default();
        config.agent.precheck.timeout_secs = 0;
        let err = config.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("agent.precheck.timeout_secs") && msg.contains("greater than 0"),
            "expected precheck timeout validation error, got: {msg}"
        );
    }

    #[test]
    async fn validate_rejects_precheck_empty_model() {
        let mut config = Config::default();
        config.agent.precheck.model = Some("   ".into());
        let err = config.validate().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("agent.precheck.model"),
            "expected precheck model validation error, got: {msg}"
        );
    }

    #[test]
    async fn validate_accepts_default_precheck() {
        let config = Config::default();
        assert!(
            config.validate().is_ok(),
            "default ChannelPrecheckConfig must pass validation"
        );
    }

    #[test]
    async fn validate_accepts_precheck_model_override() {
        let mut config = Config::default();
        config.agent.precheck.model = Some("fast-classifier".into());
        config.agent.precheck.timeout_secs = 3;
        assert!(config.validate().is_ok());
    }

    #[test]
    async fn validate_rejects_unpublished_jira_actions() {
        for action in ["list_projects", "myself"] {
            let mut config = Config::default();
            config.jira.enabled = true;
            config.jira.base_url = "https://jira.example.test".into();
            config.jira.api_token = "token".into();
            config.jira.allowed_actions = vec![action.into()];

            let err = config
                .validate()
                .expect_err("unpublished Jira action should be rejected")
                .to_string();
            assert!(
                err.contains("jira.allowed_actions contains unknown action"),
                "expected Jira allowed action error for {action}, got: {err}"
            );
        }
    }

    #[test]
    async fn env_override_reasoning_enabled() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.runtime.reasoning_enabled, None);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "false") };
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(false));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "true") };
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(true));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_REASONING_ENABLED") };
    }

    #[test]
    async fn env_override_reasoning_invalid_value_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        config.runtime.reasoning_enabled = Some(false);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REASONING_ENABLED", "maybe") };
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_enabled, Some(false));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_REASONING_ENABLED") };
    }

    #[test]
    async fn env_override_reasoning_effort() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        assert_eq!(config.runtime.reasoning_effort, None);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_REASONING_EFFORT", "HIGH") };
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_effort.as_deref(), Some("high"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_REASONING_EFFORT") };
    }

    #[test]
    async fn env_override_reasoning_effort_legacy_codex_env() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_CODEX_REASONING_EFFORT", "minimal") };
        config.apply_env_overrides();
        assert_eq!(config.runtime.reasoning_effort.as_deref(), Some("minimal"));

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_CODEX_REASONING_EFFORT") };
    }

    #[test]
    async fn env_override_invalid_port_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_port = config.gateway.port;

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("PORT", "not_a_number") };
        config.apply_env_overrides();
        assert_eq!(config.gateway.port, original_port);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("PORT") };
    }

    #[test]
    async fn env_override_web_search_config() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_ENABLED", "false") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_PROVIDER", "brave") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_MAX_RESULTS", "7") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_TIMEOUT_SECS", "20") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("BRAVE_API_KEY", "brave-test-key") };

        config.apply_env_overrides();

        assert!(!config.web_search.enabled);
        assert_eq!(config.web_search.provider, "brave");
        assert_eq!(config.web_search.max_results, 7);
        assert_eq!(config.web_search.timeout_secs, 20);
        assert_eq!(
            config.web_search.brave_api_key.as_deref(),
            Some("brave-test-key")
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_ENABLED") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_MAX_RESULTS") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_TIMEOUT_SECS") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("BRAVE_API_KEY") };
    }

    #[test]
    async fn env_override_web_search_invalid_values_ignored() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();
        let original_max_results = config.web_search.max_results;
        let original_timeout = config.web_search.timeout_secs;

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_MAX_RESULTS", "99") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("WEB_SEARCH_TIMEOUT_SECS", "0") };

        config.apply_env_overrides();

        assert_eq!(config.web_search.max_results, original_max_results);
        assert_eq!(config.web_search.timeout_secs, original_timeout);

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_MAX_RESULTS") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("WEB_SEARCH_TIMEOUT_SECS") };
    }

    #[test]
    async fn env_override_storage_provider_config() {
        let _env_guard = env_override_lock().await;
        let mut config = Config::default();

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_STORAGE_PROVIDER", "qdrant") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_STORAGE_DB_URL", "http://localhost:6333") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS", "15") };

        config.apply_env_overrides();

        assert_eq!(config.storage.provider.config.provider, "qdrant");
        assert_eq!(
            config.storage.provider.config.db_url.as_deref(),
            Some("http://localhost:6333")
        );
        assert_eq!(
            config.storage.provider.config.connect_timeout_secs,
            Some(15)
        );

        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_STORAGE_PROVIDER") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_STORAGE_DB_URL") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::remove_var("OPERANT_STORAGE_CONNECT_TIMEOUT_SECS") };
    }

    #[test]
    async fn proxy_config_scope_services_requires_entries_when_enabled() {
        let proxy = ProxyConfig {
            enabled: true,
            http_proxy: Some("http://127.0.0.1:7890".into()),
            https_proxy: None,
            all_proxy: None,
            no_proxy: Vec::new(),
            scope: ProxyScope::Services,
            services: Vec::new(),
        };

        let error = proxy.validate().unwrap_err().to_string();
        assert!(error.contains("proxy.scope='services'"));
    }

    #[test]
    async fn env_override_proxy_scope_services() {
        let _env_guard = env_override_lock().await;
        clear_proxy_env_test_vars();

        let mut config = Config::default();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROXY_ENABLED", "true") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_HTTP_PROXY", "http://127.0.0.1:7890") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe {
            std::env::set_var(
                "OPERANT_PROXY_SERVICES",
                "provider.openai, tool.http_request",
            );
        }
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROXY_SCOPE", "services") };

        config.apply_env_overrides();

        assert!(config.proxy.enabled);
        assert_eq!(config.proxy.scope, ProxyScope::Services);
        assert_eq!(
            config.proxy.http_proxy.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert!(config.proxy.should_apply_to_service("provider.openai"));
        assert!(config.proxy.should_apply_to_service("tool.http_request"));
        assert!(!config.proxy.should_apply_to_service("provider.anthropic"));

        clear_proxy_env_test_vars();
    }

    #[test]
    async fn env_override_proxy_scope_environment_applies_process_env() {
        let _env_guard = env_override_lock().await;
        clear_proxy_env_test_vars();

        let mut config = Config::default();
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROXY_ENABLED", "true") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_PROXY_SCOPE", "environment") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_HTTP_PROXY", "http://127.0.0.1:7890") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_HTTPS_PROXY", "http://127.0.0.1:7891") };
        // SAFETY: test-only, single-threaded test runner.
        unsafe { std::env::set_var("OPERANT_NO_PROXY", "localhost,127.0.0.1") };

        config.apply_env_overrides();

        assert_eq!(config.proxy.scope, ProxyScope::Environment);
        assert_eq!(
            std::env::var("HTTP_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7891")
        );
        assert!(
            std::env::var("NO_PROXY")
                .ok()
                .is_some_and(|value| value.contains("localhost"))
        );

        clear_proxy_env_test_vars();
    }

    #[test]
    async fn google_workspace_allowed_operations_require_methods() {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: Vec::new(),
        }];

        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("google_workspace.allowed_operations[0].methods"));
    }

    #[test]
    async fn google_workspace_allowed_operations_reject_duplicate_service_resource_sub_resource_entries()
     {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("drafts".into()),
                methods: vec!["create".into()],
            },
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("drafts".into()),
                methods: vec!["update".into()],
            },
        ];

        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate service/resource/sub_resource entry"));
    }

    #[test]
    async fn google_workspace_allowed_operations_allow_same_resource_different_sub_resource() {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("messages".into()),
                methods: vec!["list".into(), "get".into()],
            },
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("drafts".into()),
                methods: vec!["create".into(), "update".into()],
            },
        ];

        assert!(config.validate().is_ok());
    }

    #[test]
    async fn google_workspace_allowed_operations_reject_duplicate_methods_within_entry() {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["create".into(), "create".into()],
        }];

        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("duplicate entry"),
            "expected duplicate entry error, got: {err}"
        );
    }

    #[test]
    async fn google_workspace_allowed_operations_accept_valid_entries() {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("messages".into()),
                methods: vec!["list".into(), "get".into()],
            },
            GoogleWorkspaceAllowedOperation {
                service: "drive".into(),
                resource: "files".into(),
                sub_resource: None,
                methods: vec!["list".into(), "get".into()],
            },
        ];

        assert!(config.validate().is_ok());
    }

    #[test]
    async fn google_workspace_allowed_operations_reject_invalid_sub_resource_characters() {
        let mut config = Config::default();
        config.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("bad resource!".into()),
            methods: vec!["list".into()],
        }];

        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("sub_resource contains invalid characters"));
    }

    fn runtime_proxy_cache_contains(cache_key: &str) -> bool {
        match runtime_proxy_client_cache().read() {
            Ok(guard) => guard.contains_key(cache_key),
            Err(poisoned) => poisoned.into_inner().contains_key(cache_key),
        }
    }

    #[test]
    async fn runtime_proxy_client_cache_reuses_default_profile_key() {
        let service_key = format!(
            "provider.cache_test.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let cache_key = runtime_proxy_cache_key(&service_key, None, None);

        clear_runtime_proxy_client_cache();
        assert!(!runtime_proxy_cache_contains(&cache_key));

        let _ = build_runtime_proxy_client(&service_key);
        assert!(runtime_proxy_cache_contains(&cache_key));

        let _ = build_runtime_proxy_client(&service_key);
        assert!(runtime_proxy_cache_contains(&cache_key));
    }

    #[test]
    async fn set_runtime_proxy_config_clears_runtime_proxy_client_cache() {
        let service_key = format!(
            "provider.cache_timeout_test.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        );
        let cache_key = runtime_proxy_cache_key(&service_key, Some(30), Some(5));

        clear_runtime_proxy_client_cache();
        let _ = build_runtime_proxy_client_with_timeouts(&service_key, 30, 5);
        assert!(runtime_proxy_cache_contains(&cache_key));

        set_runtime_proxy_config(ProxyConfig::default());
        assert!(!runtime_proxy_cache_contains(&cache_key));
    }

    #[test]
    async fn gateway_config_default_values() {
        let g = GatewayConfig::default();
        assert_eq!(g.port, 42617);
        assert_eq!(g.host, "127.0.0.1");
        assert!(g.require_pairing);
        assert!(!g.allow_public_bind);
        assert!(g.paired_tokens.is_empty());
        assert!(!g.trust_forwarded_headers);
        assert_eq!(g.rate_limit_max_keys, 10_000);
        assert_eq!(g.idempotency_max_keys, 10_000);
    }

    // ── Peripherals config ───────────────────────────────────────

    #[test]
    async fn peripherals_config_default_disabled() {
        let p = PeripheralsConfig::default();
        assert!(!p.enabled);
        assert!(p.boards.is_empty());
    }

    #[test]
    async fn peripheral_board_config_defaults() {
        let b = PeripheralBoardConfig::default();
        assert!(b.board.is_empty());
        assert_eq!(b.transport, "serial");
        assert!(b.path.is_none());
        assert_eq!(b.baud, 115_200);
    }

    #[test]
    async fn peripherals_config_toml_roundtrip() {
        let p = PeripheralsConfig {
            enabled: true,
            boards: vec![PeripheralBoardConfig {
                board: "nucleo-f401re".into(),
                transport: "serial".into(),
                path: Some("/dev/ttyACM0".into()),
                baud: 115_200,
            }],
            datasheet_dir: None,
        };
        let toml_str = toml::to_string(&p).unwrap();
        let parsed: PeripheralsConfig = toml::from_str(&toml_str).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.boards.len(), 1);
        assert_eq!(parsed.boards[0].board, "nucleo-f401re");
        assert_eq!(parsed.boards[0].path.as_deref(), Some("/dev/ttyACM0"));
    }

    #[test]
    async fn lark_config_serde() {
        let lc = LarkConfig {
            enabled: true,
            app_id: "cli_123456".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["user_123".into(), "user_456".into()],
            mention_only: false,
            use_feishu: true,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            proxy_url: None,
        };
        let json = serde_json::to_string(&lc).unwrap();
        let parsed: LarkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.app_id, "cli_123456");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
        assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
        assert_eq!(parsed.allowed_users.len(), 2);
        assert!(parsed.use_feishu);
    }

    #[test]
    async fn lark_config_toml_roundtrip() {
        let lc = LarkConfig {
            enabled: true,
            app_id: "cli_123456".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["*".into()],
            mention_only: false,
            use_feishu: false,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
            proxy_url: None,
        };
        let toml_str = toml::to_string(&lc).unwrap();
        let parsed: LarkConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.app_id, "cli_123456");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert!(!parsed.use_feishu);
    }

    #[test]
    async fn lark_config_deserializes_without_optional_fields() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.encrypt_key.is_none());
        assert!(parsed.verification_token.is_none());
        assert!(parsed.allowed_users.is_empty());
        assert!(!parsed.mention_only);
        assert!(!parsed.use_feishu);
    }

    #[test]
    async fn lark_config_defaults_to_lark_endpoint() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert!(
            !parsed.use_feishu,
            "use_feishu should default to false (Lark)"
        );
    }

    #[test]
    async fn lark_config_with_wildcard_allowed_users() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret","allowed_users":["*"]}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.allowed_users, vec!["*"]);
    }

    #[test]
    async fn feishu_config_serde() {
        let fc = FeishuConfig {
            enabled: true,
            app_id: "cli_feishu_123".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["user_123".into(), "user_456".into()],
            mention_only: false,
            receive_mode: LarkReceiveMode::Websocket,
            port: None,
            proxy_url: None,
        };
        let json = serde_json::to_string(&fc).unwrap();
        let parsed: FeishuConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.app_id, "cli_feishu_123");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.encrypt_key.as_deref(), Some("encrypt_key"));
        assert_eq!(parsed.verification_token.as_deref(), Some("verify_token"));
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    async fn feishu_config_toml_roundtrip() {
        let fc = FeishuConfig {
            enabled: true,
            app_id: "cli_feishu_123".into(),
            app_secret: "secret_abc".into(),
            encrypt_key: Some("encrypt_key".into()),
            verification_token: Some("verify_token".into()),
            allowed_users: vec!["*".into()],
            mention_only: false,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
            proxy_url: None,
        };
        let toml_str = toml::to_string(&fc).unwrap();
        let parsed: FeishuConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.app_id, "cli_feishu_123");
        assert_eq!(parsed.app_secret, "secret_abc");
        assert_eq!(parsed.receive_mode, LarkReceiveMode::Webhook);
        assert_eq!(parsed.port, Some(9898));
    }

    #[test]
    async fn feishu_config_deserializes_without_optional_fields() {
        let json = r#"{"app_id":"cli_123","app_secret":"secret"}"#;
        let parsed: FeishuConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.encrypt_key.is_none());
        assert!(parsed.verification_token.is_none());
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.receive_mode, LarkReceiveMode::Websocket);
        assert!(parsed.port.is_none());
    }

    // ── LINE ──────────────────────────────────────────────────

    #[test]
    async fn line_config_toml_roundtrip() {
        // Full [channels.line] TOML block — covers every user-facing field.
        //
        // channel_access_token and channel_secret can be omitted here and
        // supplied via LINE_CHANNEL_ACCESS_TOKEN / LINE_CHANNEL_SECRET env vars
        // instead; both fields default to "" when absent.
        let toml = r#"
[channels_config.line]
enabled = true
channel_access_token = "ChannelAccessToken=="
channel_secret = "abc123secret"
dm_policy = "pairing"
group_policy = "mention"
allowed_users = []
webhook_port = 8443
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ln = config.channels.line.as_ref().unwrap();
        assert!(ln.enabled);
        assert_eq!(ln.channel_access_token, "ChannelAccessToken==");
        assert_eq!(ln.channel_secret, "abc123secret");
        assert_eq!(ln.dm_policy, LineDmPolicy::Pairing);
        assert_eq!(ln.group_policy, LineGroupPolicy::Mention);
        assert_eq!(ln.webhook_port, 8443);
        assert!(ln.proxy_url.is_none());
    }

    #[test]
    async fn line_config_defaults() {
        // Minimal config — only the required secret fields are provided.
        // All optional fields should resolve to documented defaults.
        let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ln = config.channels.line.as_ref().unwrap();
        assert!(!ln.enabled, "enabled should default to false");
        assert_eq!(
            ln.dm_policy,
            LineDmPolicy::Pairing,
            "dm_policy default is pairing"
        );
        assert_eq!(
            ln.group_policy,
            LineGroupPolicy::Mention,
            "group_policy default is mention"
        );
        assert_eq!(ln.webhook_port, 8443, "webhook_port default is 8443");
        assert!(ln.allowed_users.is_empty());
        assert!(ln.proxy_url.is_none());
    }

    #[test]
    async fn line_config_allowlist_policy() {
        // dm_policy = allowlist with an explicit user ID list.
        let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
dm_policy = "allowlist"
allowed_users = ["Uabc123", "Udef456"]
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ln = config.channels.line.as_ref().unwrap();
        assert_eq!(ln.dm_policy, LineDmPolicy::Allowlist);
        assert_eq!(ln.allowed_users, vec!["Uabc123", "Udef456"]);
    }

    #[test]
    async fn line_config_open_policies() {
        // dm_policy = open + group_policy = open — most permissive combination.
        let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
dm_policy = "open"
group_policy = "open"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ln = config.channels.line.as_ref().unwrap();
        assert_eq!(ln.dm_policy, LineDmPolicy::Open);
        assert_eq!(ln.group_policy, LineGroupPolicy::Open);
    }

    #[test]
    async fn line_config_group_disabled() {
        // group_policy = disabled — bot ignores all group/room messages.
        let toml = r#"
[channels_config.line]
channel_access_token = "tok"
channel_secret = "sec"
group_policy = "disabled"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ln = config.channels.line.as_ref().unwrap();
        assert_eq!(ln.group_policy, LineGroupPolicy::Disabled);
    }

    #[test]
    async fn nextcloud_talk_config_serde() {
        let nc = NextcloudTalkConfig {
            enabled: true,
            base_url: "https://cloud.example.com".into(),
            app_token: "app-token".into(),
            webhook_secret: Some("webhook-secret".into()),
            allowed_users: vec!["user_a".into(), "*".into()],
            proxy_url: None,
            bot_name: None,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1000,
        };

        let json = serde_json::to_string(&nc).unwrap();
        let parsed: NextcloudTalkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.base_url, "https://cloud.example.com");
        assert_eq!(parsed.app_token, "app-token");
        assert_eq!(parsed.webhook_secret.as_deref(), Some("webhook-secret"));
        assert_eq!(parsed.allowed_users, vec!["user_a", "*"]);
    }

    #[test]
    async fn nextcloud_talk_config_defaults_optional_fields() {
        let json = r#"{"base_url":"https://cloud.example.com","app_token":"app-token"}"#;
        let parsed: NextcloudTalkConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.webhook_secret.is_none());
        assert!(parsed.allowed_users.is_empty());
    }

    // ── Config file permission hardening (Unix only) ───────────────

    #[cfg(unix)]
    #[test]
    async fn new_config_file_has_restricted_permissions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Create a config and save it
        let config = Config {
            config_path: config_path.clone(),
            ..Default::default()
        };
        config.save().await.unwrap();

        let meta = fs::metadata(&config_path).await.unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "New config file should be owner-only (0600), got {mode:o}"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn save_restricts_existing_world_readable_config_to_owner_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        let mut config = Config {
            config_path: config_path.clone(),
            ..Default::default()
        };
        config.save().await.unwrap();

        // Simulate the regression state observed in issue #1345.
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let loose_mode = std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            loose_mode, 0o644,
            "test setup requires world-readable config"
        );

        if let Some(entry) = config.providers.fallback_provider_mut() {
            entry.temperature = Some(0.6);
        }
        config.save().await.unwrap();

        let hardened_mode = std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            hardened_mode, 0o600,
            "Saving config should restore owner-only permissions (0600)"
        );
    }

    #[cfg(unix)]
    #[test]
    async fn world_readable_config_is_detectable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");

        // Create a config file with intentionally loose permissions
        std::fs::write(&config_path, "# test config").unwrap();
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let meta = std::fs::metadata(&config_path).unwrap();
        let mode = meta.permissions().mode();
        assert!(
            mode & 0o004 != 0,
            "Test setup: file should be world-readable (mode {mode:o})"
        );
    }

    #[test]
    async fn transcription_config_defaults() {
        let tc = TranscriptionConfig::default();
        assert!(!tc.enabled);
        assert!(tc.api_url.contains("groq.com"));
        assert_eq!(tc.model, "whisper-large-v3-turbo");
        assert!(tc.language.is_none());
        assert_eq!(tc.max_duration_secs, 120);
        assert!(!tc.transcribe_non_ptt_audio);
    }

    #[test]
    async fn config_roundtrip_with_transcription() {
        let mut config = Config::default();
        config.transcription.enabled = true;
        config.transcription.language = Some("en".into());

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed = parse_test_config(&toml_str);

        assert!(parsed.transcription.enabled);
        assert_eq!(parsed.transcription.language.as_deref(), Some("en"));
        assert_eq!(parsed.transcription.model, "whisper-large-v3-turbo");
    }

    #[test]
    async fn config_without_transcription_uses_defaults() {
        let toml_str = r#"
            default_provider = "openrouter"
            default_model = "test-model"
            default_temperature = 0.7
        "#;
        let parsed = parse_test_config(toml_str);
        assert!(!parsed.transcription.enabled);
        assert_eq!(parsed.transcription.max_duration_secs, 120);
    }

    #[test]
    async fn security_defaults_are_backward_compatible() {
        let parsed = parse_test_config(
            r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7
"#,
        );

        assert!(!parsed.security.otp.enabled);
        assert_eq!(parsed.security.otp.method, OtpMethod::Totp);
        assert!(!parsed.security.estop.enabled);
        assert!(parsed.security.estop.require_otp_to_resume);
    }

    #[test]
    async fn security_toml_parses_otp_and_estop_sections() {
        let parsed = parse_test_config(
            r#"
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4.6"
default_temperature = 0.7

[security.otp]
enabled = true
method = "totp"
token_ttl_secs = 30
cache_valid_secs = 120
gated_actions = ["shell", "browser_open"]
gated_domains = ["*.chase.com", "accounts.google.com"]
gated_domain_categories = ["banking"]

[security.estop]
enabled = true
state_file = "~/.operant/estop-state.json"
require_otp_to_resume = true
"#,
        );

        assert!(parsed.security.otp.enabled);
        assert!(parsed.security.estop.enabled);
        assert_eq!(parsed.security.otp.gated_actions.len(), 2);
        assert_eq!(parsed.security.otp.gated_domains.len(), 2);
        parsed.validate().unwrap();
    }

    #[test]
    async fn security_validation_rejects_invalid_domain_glob() {
        let mut config = Config::default();
        config.security.otp.gated_domains = vec!["bad domain.com".into()];

        let err = config.validate().expect_err("expected invalid domain glob");
        assert!(err.to_string().contains("gated_domains"));
    }

    #[test]
    async fn validate_accepts_local_whisper_as_transcription_default_provider() {
        let mut config = Config::default();
        config.transcription.default_provider = "local_whisper".to_string();

        config.validate().expect(
            "local_whisper must be accepted by the transcription.default_provider allowlist",
        );
    }

    #[test]
    async fn validate_rejects_unknown_transcription_default_provider() {
        let mut config = Config::default();
        config.transcription.default_provider = "unknown_stt".to_string();

        let err = config
            .validate()
            .expect_err("expected validation to reject unknown transcription provider");
        assert!(
            err.to_string().contains("transcription.default_provider"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn channel_secret_telegram_bot_token_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "operant_test_tg_bot_token_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        let plaintext_token = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";

        let mut config = Config {
            workspace_dir: dir.join("workspace"),
            config_path: dir.join("config.toml"),
            ..Default::default()
        };
        config.channels.telegram = Some(TelegramConfig {
            enabled: true,
            bot_token: plaintext_token.into(),
            allowed_users: vec!["user1".into()],
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: default_draft_update_interval_ms(),
            interrupt_on_new_message: false,
            mention_only: false,
            ack_reactions: None,
            proxy_url: None,
            approval_timeout_secs: default_telegram_approval_timeout_secs(),
            dm_topics_enabled: false,
            dm_topic_name: default_telegram_dm_topic_name(),
            disable_link_previews: false,
            typing_cooldown_seconds: default_telegram_typing_cooldown_secs(),
            fallback_ips: vec![],
        });

        // Save (triggers encryption)
        config.save().await.unwrap();

        // Read raw TOML and verify plaintext token is NOT present
        let raw_toml = tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap();
        assert!(
            !raw_toml.contains(plaintext_token),
            "Saved TOML must not contain the plaintext bot_token"
        );

        // Parse stored TOML and verify the value is encrypted
        let stored: Config = toml::from_str(&raw_toml).unwrap();
        let stored_token = &stored.channels.telegram.as_ref().unwrap().bot_token;
        assert!(
            crate::secrets::SecretStore::is_encrypted(stored_token),
            "Stored bot_token must be marked as encrypted"
        );

        // Decrypt and verify it matches the original plaintext
        let store = crate::secrets::SecretStore::new(&dir, true);
        assert_eq!(store.decrypt(stored_token).unwrap(), plaintext_token);

        // Simulate a full load: deserialize then decrypt (mirrors load_or_init logic)
        let mut loaded: Config = toml::from_str(&raw_toml).unwrap();
        loaded.config_path = dir.join("config.toml");
        let load_store = crate::secrets::SecretStore::new(&dir, loaded.secrets.encrypt);
        loaded.decrypt_secrets(&load_store).unwrap();
        assert_eq!(
            loaded.channels.telegram.as_ref().unwrap().bot_token,
            plaintext_token,
            "Loaded bot_token must match the original plaintext after decryption"
        );

        let _ = fs::remove_dir_all(&dir).await;
    }

    #[test]
    async fn security_validation_rejects_unknown_domain_category() {
        let mut config = Config::default();
        config.security.otp.gated_domain_categories = vec!["not_real".into()];

        let err = config
            .validate()
            .expect_err("expected unknown domain category");
        assert!(err.to_string().contains("gated_domain_categories"));
    }

    #[test]
    async fn security_validation_rejects_zero_token_ttl() {
        let mut config = Config::default();
        config.security.otp.token_ttl_secs = 0;

        let err = config
            .validate()
            .expect_err("expected ttl validation failure");
        assert!(err.to_string().contains("token_ttl_secs"));
    }

    // ── MCP config validation ─────────────────────────────────────────────

    fn stdio_server(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransport::Stdio,
            command: command.to_string(),
            ..Default::default()
        }
    }

    fn http_server(name: &str, url: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransport::Http,
            url: Some(url.to_string()),
            ..Default::default()
        }
    }

    fn sse_server(name: &str, url: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransport::Sse,
            url: Some(url.to_string()),
            ..Default::default()
        }
    }

    #[test]
    async fn validate_mcp_config_empty_servers_ok() {
        let cfg = McpConfig::default();
        assert!(validate_mcp_config(&cfg).is_ok());
    }

    #[test]
    async fn validate_mcp_config_valid_stdio_ok() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![stdio_server("fs", "/usr/bin/mcp-fs")],
            ..Default::default()
        };
        assert!(validate_mcp_config(&cfg).is_ok());
    }

    #[test]
    async fn validate_mcp_config_valid_http_ok() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![http_server("svc", "http://localhost:8080/mcp")],
            ..Default::default()
        };
        assert!(validate_mcp_config(&cfg).is_ok());
    }

    #[test]
    async fn validate_mcp_config_valid_sse_ok() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![sse_server("svc", "https://example.com/events")],
            ..Default::default()
        };
        assert!(validate_mcp_config(&cfg).is_ok());
    }

    #[test]
    async fn validate_mcp_config_rejects_empty_name() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![stdio_server("", "/usr/bin/tool")],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("empty name should fail");
        assert!(
            err.to_string().contains("name must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    async fn validate_mcp_config_rejects_whitespace_name() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![stdio_server("   ", "/usr/bin/tool")],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("whitespace name should fail");
        assert!(
            err.to_string().contains("name must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    async fn validate_mcp_config_rejects_duplicate_names() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![
                stdio_server("fs", "/usr/bin/mcp-a"),
                stdio_server("fs", "/usr/bin/mcp-b"),
            ],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("duplicate name should fail");
        assert!(err.to_string().contains("duplicate name"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_rejects_zero_timeout() {
        let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
        server.tool_timeout_secs = Some(0);
        let cfg = McpConfig {
            enabled: true,
            servers: vec![server],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("zero timeout should fail");
        assert!(err.to_string().contains("greater than 0"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_rejects_timeout_exceeding_max() {
        let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
        server.tool_timeout_secs = Some(MCP_MAX_TOOL_TIMEOUT_SECS + 1);
        let cfg = McpConfig {
            enabled: true,
            servers: vec![server],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("oversized timeout should fail");
        assert!(err.to_string().contains("exceeds max"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_allows_max_timeout_exactly() {
        let mut server = stdio_server("fs", "/usr/bin/mcp-fs");
        server.tool_timeout_secs = Some(MCP_MAX_TOOL_TIMEOUT_SECS);
        let cfg = McpConfig {
            enabled: true,
            servers: vec![server],
            ..Default::default()
        };
        assert!(validate_mcp_config(&cfg).is_ok());
    }

    #[test]
    async fn validate_mcp_config_rejects_stdio_with_empty_command() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![stdio_server("fs", "")],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("empty command should fail");
        assert!(
            err.to_string().contains("requires non-empty command"),
            "got: {err}"
        );
    }

    #[test]
    async fn validate_mcp_config_rejects_http_without_url() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "svc".to_string(),
                transport: McpTransport::Http,
                url: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("http without url should fail");
        assert!(err.to_string().contains("requires url"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_rejects_sse_without_url() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![McpServerConfig {
                name: "svc".to_string(),
                transport: McpTransport::Sse,
                url: None,
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("sse without url should fail");
        assert!(err.to_string().contains("requires url"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_rejects_non_http_scheme() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![http_server("svc", "ftp://example.com/mcp")],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("non-http scheme should fail");
        assert!(err.to_string().contains("http/https"), "got: {err}");
    }

    #[test]
    async fn validate_mcp_config_rejects_invalid_url() {
        let cfg = McpConfig {
            enabled: true,
            servers: vec![http_server("svc", "not a url at all !!!")],
            ..Default::default()
        };
        let err = validate_mcp_config(&cfg).expect_err("invalid url should fail");
        assert!(err.to_string().contains("valid URL"), "got: {err}");
    }

    #[test]
    async fn mcp_config_default_disabled_with_empty_servers() {
        let cfg = McpConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.servers.is_empty());
    }

    #[test]
    async fn mcp_transport_serde_roundtrip_lowercase() {
        let cases = [
            (McpTransport::Stdio, "\"stdio\""),
            (McpTransport::Http, "\"http\""),
            (McpTransport::Sse, "\"sse\""),
        ];
        for (variant, expected_json) in &cases {
            let serialized = serde_json::to_string(variant).expect("serialize");
            assert_eq!(&serialized, expected_json, "variant: {variant:?}");
            let deserialized: McpTransport =
                serde_json::from_str(expected_json).expect("deserialize");
            assert_eq!(&deserialized, variant);
        }
    }

    #[test]
    async fn swarm_strategy_roundtrip() {
        let cases = vec![
            (SwarmStrategy::Sequential, "\"sequential\""),
            (SwarmStrategy::Parallel, "\"parallel\""),
            (SwarmStrategy::Router, "\"router\""),
        ];
        for (variant, expected_json) in &cases {
            let serialized = serde_json::to_string(variant).expect("serialize");
            assert_eq!(&serialized, expected_json, "variant: {variant:?}");
            let deserialized: SwarmStrategy =
                serde_json::from_str(expected_json).expect("deserialize");
            assert_eq!(&deserialized, variant);
        }
    }

    #[test]
    async fn swarm_config_deserializes_with_defaults() {
        let toml_str = r#"
            agents = ["researcher", "writer"]
            strategy = "sequential"
        "#;
        let config: SwarmConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(config.agents, vec!["researcher", "writer"]);
        assert_eq!(config.strategy, SwarmStrategy::Sequential);
        assert!(config.router_prompt.is_none());
        assert!(config.description.is_none());
        assert_eq!(config.timeout_secs, 300);
    }

    #[test]
    async fn swarm_config_deserializes_full() {
        let toml_str = r#"
            agents = ["a", "b", "c"]
            strategy = "router"
            router_prompt = "Pick the best."
            description = "Multi-agent router"
            timeout_secs = 120
        "#;
        let config: SwarmConfig = toml::from_str(toml_str).expect("deserialize");
        assert_eq!(config.agents.len(), 3);
        assert_eq!(config.strategy, SwarmStrategy::Router);
        assert_eq!(config.router_prompt.as_deref(), Some("Pick the best."));
        assert_eq!(config.description.as_deref(), Some("Multi-agent router"));
        assert_eq!(config.timeout_secs, 120);
    }

    #[test]
    async fn config_with_swarms_section_deserializes() {
        let toml_str = r#"
            [agents.researcher]
            provider = "ollama"
            model = "llama3"

            [agents.writer]
            provider = "openrouter"
            model = "claude-sonnet"

            [swarms.pipeline]
            agents = ["researcher", "writer"]
            strategy = "sequential"
        "#;
        let config = parse_test_config(toml_str);
        assert_eq!(config.agents.len(), 2);
        assert_eq!(config.swarms.len(), 1);
        assert!(config.swarms.contains_key("pipeline"));
    }

    #[tokio::test]
    async fn nevis_client_secret_encrypt_decrypt_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "operant_test_nevis_secret_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).await.unwrap();

        let plaintext_secret = "nevis-test-client-secret-value";

        let mut config = Config {
            workspace_dir: dir.join("workspace"),
            config_path: dir.join("config.toml"),
            ..Default::default()
        };
        config.security.nevis.client_secret = Some(plaintext_secret.into());

        // Save (triggers encryption)
        config.save().await.unwrap();

        // Read raw TOML and verify plaintext secret is NOT present
        let raw_toml = tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap();
        assert!(
            !raw_toml.contains(plaintext_secret),
            "Saved TOML must not contain the plaintext client_secret"
        );

        // Parse stored TOML and verify the value is encrypted
        let stored: Config = toml::from_str(&raw_toml).unwrap();
        let stored_secret = stored.security.nevis.client_secret.as_ref().unwrap();
        assert!(
            crate::secrets::SecretStore::is_encrypted(stored_secret),
            "Stored client_secret must be marked as encrypted"
        );

        // Decrypt and verify it matches the original plaintext
        let store = crate::secrets::SecretStore::new(&dir, true);
        assert_eq!(store.decrypt(stored_secret).unwrap(), plaintext_secret);

        // Simulate a full load: deserialize then decrypt (mirrors load_or_init logic)
        let mut loaded: Config = toml::from_str(&raw_toml).unwrap();
        loaded.config_path = dir.join("config.toml");
        let load_store = crate::secrets::SecretStore::new(&dir, loaded.secrets.encrypt);
        loaded.decrypt_secrets(&load_store).unwrap();
        assert_eq!(
            loaded.security.nevis.client_secret.as_deref().unwrap(),
            plaintext_secret,
            "Loaded client_secret must match the original plaintext after decryption"
        );

        let _ = fs::remove_dir_all(&dir).await;
    }

    // ══════════════════════════════════════════════════════════
    // Nevis config validation tests
    // ══════════════════════════════════════════════════════════

    #[test]
    async fn nevis_config_validate_disabled_accepts_empty_fields() {
        let cfg = NevisConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    async fn nevis_config_validate_rejects_empty_instance_url() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: String::new(),
            client_id: "test-client".into(),
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("instance_url"));
    }

    #[test]
    async fn nevis_config_validate_rejects_empty_client_id() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            client_id: String::new(),
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("client_id"));
    }

    #[test]
    async fn nevis_config_validate_rejects_empty_realm() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            client_id: "test-client".into(),
            realm: String::new(),
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("realm"));
    }

    #[test]
    async fn nevis_config_validate_rejects_local_without_jwks() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            client_id: "test-client".into(),
            token_validation: "local".into(),
            jwks_url: None,
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("jwks_url"));
    }

    #[test]
    async fn nevis_config_validate_rejects_zero_session_timeout() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            client_id: "test-client".into(),
            token_validation: "remote".into(),
            session_timeout_secs: 0,
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("session_timeout_secs"));
    }

    #[test]
    async fn nevis_config_validate_accepts_valid_enabled_config() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            realm: "master".into(),
            client_id: "test-client".into(),
            token_validation: "remote".into(),
            session_timeout_secs: 3600,
            ..NevisConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    async fn nevis_config_validate_rejects_invalid_token_validation() {
        let cfg = NevisConfig {
            enabled: true,
            instance_url: "https://nevis.example.com".into(),
            realm: "master".into(),
            client_id: "test-client".into(),
            token_validation: "invalid_mode".into(),
            session_timeout_secs: 3600,
            ..NevisConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("invalid value 'invalid_mode'"),
            "Expected invalid token_validation error, got: {err}"
        );
    }

    #[test]
    async fn nevis_config_debug_redacts_client_secret() {
        let cfg = NevisConfig {
            client_secret: Some("super-secret".into()),
            ..NevisConfig::default()
        };
        let debug_output = format!("{:?}", cfg);
        assert!(
            !debug_output.contains("super-secret"),
            "Debug output must not contain the raw client_secret"
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output must show [REDACTED] for client_secret"
        );
    }

    #[test]
    async fn telegram_config_ack_reactions_false_deserializes() {
        let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = false
        "#;
        let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.ack_reactions, Some(false));
    }

    #[test]
    async fn telegram_config_ack_reactions_true_deserializes() {
        let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = true
        "#;
        let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.ack_reactions, Some(true));
    }

    #[test]
    async fn telegram_config_ack_reactions_missing_defaults_to_none() {
        let toml_str = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
        "#;
        let cfg: TelegramConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.ack_reactions, None);
    }

    #[test]
    async fn telegram_config_ack_reactions_channel_overrides_top_level() {
        let tg_toml = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
            ack_reactions = false
        "#;
        let tg: TelegramConfig = toml::from_str(tg_toml).unwrap();
        let top_level_ack = true;
        let effective = tg.ack_reactions.unwrap_or(top_level_ack);
        assert!(
            !effective,
            "channel-level false must override top-level true"
        );
    }

    #[test]
    async fn telegram_config_ack_reactions_falls_back_to_top_level() {
        let tg_toml = r#"
            bot_token = "123:ABC"
            allowed_users = ["alice"]
        "#;
        let tg: TelegramConfig = toml::from_str(tg_toml).unwrap();
        let top_level_ack = false;
        let effective = tg.ack_reactions.unwrap_or(top_level_ack);
        assert!(
            !effective,
            "must fall back to top-level false when channel omits field"
        );
    }

    #[test]
    async fn google_workspace_allowed_operations_deserialize_from_toml() {
        let toml_str = r#"
            enabled = true

            [[allowed_operations]]
            service = "gmail"
            resource = "users"
            sub_resource = "drafts"
            methods = ["create", "update"]
        "#;

        let cfg: GoogleWorkspaceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.allowed_operations.len(), 1);
        assert_eq!(cfg.allowed_operations[0].service, "gmail");
        assert_eq!(cfg.allowed_operations[0].resource, "users");
        assert_eq!(
            cfg.allowed_operations[0].sub_resource.as_deref(),
            Some("drafts")
        );
        assert_eq!(
            cfg.allowed_operations[0].methods,
            vec!["create".to_string(), "update".to_string()]
        );
    }

    #[test]
    async fn google_workspace_allowed_operations_deserialize_without_sub_resource() {
        let toml_str = r#"
            enabled = true

            [[allowed_operations]]
            service = "drive"
            resource = "files"
            methods = ["list", "get"]
        "#;

        let cfg: GoogleWorkspaceConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.allowed_operations[0].sub_resource, None);
    }

    #[test]
    async fn config_validate_accepts_google_workspace_allowed_operations() {
        let mut cfg = Config::default();
        cfg.google_workspace.enabled = true;
        cfg.google_workspace.allowed_services = vec!["gmail".into()];
        cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "gmail".into(),
            resource: "users".into(),
            sub_resource: Some("drafts".into()),
            methods: vec!["create".into(), "update".into()],
        }];

        cfg.validate().unwrap();
    }

    #[test]
    async fn config_validate_rejects_duplicate_google_workspace_allowed_operations() {
        let mut cfg = Config::default();
        cfg.google_workspace.enabled = true;
        cfg.google_workspace.allowed_services = vec!["gmail".into()];
        cfg.google_workspace.allowed_operations = vec![
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("drafts".into()),
                methods: vec!["create".into()],
            },
            GoogleWorkspaceAllowedOperation {
                service: "gmail".into(),
                resource: "users".into(),
                sub_resource: Some("drafts".into()),
                methods: vec!["update".into()],
            },
        ];

        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("duplicate service/resource/sub_resource entry"));
    }

    #[test]
    async fn config_validate_rejects_operation_service_not_in_allowed_services() {
        let mut cfg = Config::default();
        cfg.google_workspace.enabled = true;
        cfg.google_workspace.allowed_services = vec!["gmail".into()];
        cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "drive".into(), // drive is not in allowed_services
            resource: "files".into(),
            sub_resource: None,
            methods: vec!["list".into()],
        }];

        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("not in the effective allowed_services"),
            "expected not-in-allowed_services error, got: {err}"
        );
    }

    #[test]
    async fn config_validate_accepts_default_service_when_allowed_services_empty() {
        // When allowed_services is empty the validator uses DEFAULT_GWS_SERVICES.
        // A known default service must pass.
        let mut cfg = Config::default();
        cfg.google_workspace.enabled = true;
        // allowed_services deliberately left empty (falls back to defaults)
        cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "drive".into(),
            resource: "files".into(),
            sub_resource: None,
            methods: vec!["list".into()],
        }];

        assert!(cfg.validate().is_ok());
    }

    #[test]
    async fn config_validate_rejects_unknown_service_when_allowed_services_empty() {
        // Even with allowed_services empty (using defaults), an operation whose
        // service is not in DEFAULT_GWS_SERVICES must fail validation — not silently
        // pass through to be rejected at runtime.
        let mut cfg = Config::default();
        cfg.google_workspace.enabled = true;
        // allowed_services deliberately left empty
        cfg.google_workspace.allowed_operations = vec![GoogleWorkspaceAllowedOperation {
            service: "not_a_real_service".into(),
            resource: "files".into(),
            sub_resource: None,
            methods: vec!["list".into()],
        }];

        let err = cfg.validate().unwrap_err().to_string();
        assert!(
            err.contains("not in the effective allowed_services"),
            "expected effective-allowed_services error, got: {err}"
        );
    }

    // ── Bootstrap files ─────────────────────────────────────

    #[tokio::test]
    async fn ensure_bootstrap_files_creates_missing_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        let _: () = tokio::fs::create_dir_all(&ws).await.unwrap();

        ensure_bootstrap_files(&ws).await.unwrap();

        let soul: String = tokio::fs::read_to_string(ws.join("SOUL.md")).await.unwrap();
        let identity: String = tokio::fs::read_to_string(ws.join("IDENTITY.md"))
            .await
            .unwrap();
        assert!(soul.contains("SOUL.md"));
        assert!(identity.contains("IDENTITY.md"));
    }

    #[tokio::test]
    async fn ensure_bootstrap_files_does_not_overwrite_existing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        let _: () = tokio::fs::create_dir_all(&ws).await.unwrap();

        let custom = "# My custom SOUL";
        let _: () = tokio::fs::write(ws.join("SOUL.md"), custom).await.unwrap();

        ensure_bootstrap_files(&ws).await.unwrap();

        let soul: String = tokio::fs::read_to_string(ws.join("SOUL.md")).await.unwrap();
        assert_eq!(
            soul, custom,
            "ensure_bootstrap_files must not overwrite existing files"
        );

        // IDENTITY.md should still be created since it was missing
        let identity: String = tokio::fs::read_to_string(ws.join("IDENTITY.md"))
            .await
            .unwrap();
        assert!(identity.contains("IDENTITY.md"));
    }

    // ── PacingConfig serde defaults ─────────────────────────────

    #[test]
    async fn pacing_config_serde_defaults_match_manual_default() {
        // Deserialise an empty TOML table and verify the loop-detection
        // fields receive the same defaults as `PacingConfig::default()`.
        let from_toml: PacingConfig = toml::from_str("").unwrap();
        let manual = PacingConfig::default();

        assert_eq!(
            from_toml.loop_detection_enabled,
            manual.loop_detection_enabled
        );
        assert_eq!(
            from_toml.loop_detection_window_size,
            manual.loop_detection_window_size
        );
        assert_eq!(
            from_toml.loop_detection_max_repeats,
            manual.loop_detection_max_repeats
        );

        // Verify concrete values so a silent change to the defaults is caught.
        assert!(from_toml.loop_detection_enabled, "default should be true");
        assert_eq!(from_toml.loop_detection_window_size, 20);
        assert_eq!(from_toml.loop_detection_max_repeats, 3);
    }

    // ── Docker baked config template ────────────────────────────

    /// The TOML template baked into Docker images (Dockerfile + Dockerfile.debian).
    /// Kept here so changes to the Dockerfiles can be validated by `cargo test`.
    const DOCKER_CONFIG_TEMPLATE: &str = r#"
workspace_dir = "/operant-data/workspace"
config_path = "/operant-data/.operant/config.toml"
api_key = ""
default_provider = "openrouter"
default_model = "anthropic/claude-sonnet-4-20250514"
default_temperature = 0.7

[gateway]
port = 42617
host = "[::]"
allow_public_bind = true

[autonomy]
level = "supervised"
auto_approve = ["file_read", "file_write", "file_edit", "memory_recall", "memory_store", "web_search_tool", "web_fetch", "calculator", "glob_search", "content_search", "image_info", "weather", "git_operations"]
"#;

    #[test]
    async fn docker_config_template_is_parseable() {
        let cfg: Config = toml::from_str(DOCKER_CONFIG_TEMPLATE)
            .expect("Docker baked config.toml must be valid TOML that deserialises into Config");

        // The [autonomy] section must be present and contain the expected tools.
        let auto = &cfg.autonomy.auto_approve;
        for tool in &[
            "file_read",
            "file_write",
            "file_edit",
            "memory_recall",
            "memory_store",
            "web_search_tool",
            "web_fetch",
            "calculator",
            "glob_search",
            "content_search",
            "image_info",
            "weather",
            "git_operations",
        ] {
            assert!(
                auto.iter().any(|t| t == tool),
                "Docker config auto_approve missing expected tool: {tool}"
            );
        }
    }

    #[test]
    async fn cost_enforcement_config_defaults() {
        let config = CostEnforcementConfig::default();
        assert_eq!(config.mode, "warn");
        assert_eq!(config.route_down_model, None);
        assert_eq!(config.reserve_percent, 10);
    }

    #[test]
    async fn cost_config_includes_enforcement() {
        let config = CostConfig::default();
        assert_eq!(config.enforcement.mode, "warn");
        assert_eq!(config.enforcement.reserve_percent, 10);
    }

    // ── Configurable macro tests ──

    #[test]
    async fn matrix_secret_fields_discovered() {
        let mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "tok".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        let fields = mx.secret_fields();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "channels.matrix.access-token");
        assert_eq!(fields[0].category, "Channels");
        assert!(fields[0].is_set);
        assert_eq!(fields[1].name, "channels.matrix.recovery-key");
        assert!(!fields[1].is_set);
        assert_eq!(fields[2].name, "channels.matrix.password");
        assert!(!fields[2].is_set);
    }

    #[test]
    async fn matrix_secret_fields_empty_not_set() {
        let mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: String::new(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        let fields = mx.secret_fields();
        assert!(!fields[0].is_set);
    }

    #[test]
    async fn set_secret_updates_field() {
        let mut mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "old".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        mx.set_secret("channels.matrix.access-token", "new-token".into())
            .unwrap();
        assert_eq!(mx.access_token, "new-token");
    }

    #[test]
    async fn set_secret_unknown_name_fails() {
        let mut mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "tok".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };
        assert!(
            mx.set_secret("channels.matrix.nonexistent", "val".into())
                .is_err()
        );
    }

    #[test]
    async fn config_tree_traversal_discovers_nested_secrets() {
        let mut config = Config::default();
        // Set api_key on fallback provider
        if let Some(name) = config.providers.fallback.clone() {
            if let Some(entry) = config.providers.models.get_mut(&name) {
                entry.api_key = Some("test-key".into());
            }
        } else {
            config.providers.fallback = Some("default".into());
            config.providers.models.insert(
                "default".into(),
                ModelProviderConfig {
                    api_key: Some("test-key".into()),
                    ..Default::default()
                },
            );
        }
        config.channels.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "mx-tok".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        });

        let fields = config.secret_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert!(names.contains(&"channels.matrix.access-token"));
        assert!(names.contains(&"channels.matrix.recovery-key"));
    }

    #[test]
    async fn config_set_secret_dispatches_to_child() {
        let mut config = Config::default();
        config.channels.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "old".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        });

        config
            .set_secret("channels.matrix.access-token", "new".into())
            .unwrap();
        assert_eq!(config.channels.matrix.as_ref().unwrap().access_token, "new");
    }

    #[test]
    async fn config_set_secret_dispatches_to_matrix_child() {
        let mut config = Config::default();
        config.channels.matrix = Some(MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "old".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            mention_only: false,
            recovery_key: None,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        });
        config
            .set_secret("channels.matrix.access-token", "sk-test".into())
            .unwrap();
        assert_eq!(
            config.channels.matrix.as_ref().unwrap().access_token,
            "sk-test"
        );
    }

    #[test]
    async fn config_set_secret_unknown_fails() {
        let mut config = Config::default();
        assert!(
            config
                .set_secret("nonexistent.field", "val".into())
                .is_err()
        );
    }

    #[test]
    async fn encrypt_decrypt_roundtrip_via_macro() {
        let dir = TempDir::new().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path(), true);

        let mut mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "plaintext-token".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };

        // Encrypt
        mx.encrypt_secrets(&store).unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(&mx.access_token));
        assert_ne!(mx.access_token, "plaintext-token");

        // Decrypt
        mx.decrypt_secrets(&store).unwrap();
        assert_eq!(mx.access_token, "plaintext-token");
    }

    #[test]
    async fn encrypt_skips_already_encrypted() {
        let dir = TempDir::new().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path(), true);

        let mut mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "plaintext-token".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };

        mx.encrypt_secrets(&store).unwrap();
        let first_encrypted = mx.access_token.clone();

        // Encrypt again — should be idempotent
        mx.encrypt_secrets(&store).unwrap();
        assert_eq!(mx.access_token, first_encrypted);
    }

    #[test]
    async fn encrypt_no_op_on_disabled_store() {
        let dir = TempDir::new().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path(), false);

        let mut mx = MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "plaintext-token".into(),
            user_id: None,
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        };

        mx.encrypt_secrets(&store).unwrap();
        // With encryption disabled, value should stay plaintext
        assert_eq!(mx.access_token, "plaintext-token");
    }

    // ── Property method tests ──

    fn test_matrix_config() -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver: "https://m.org".into(),
            access_token: "tok".into(),
            user_id: Some("@bot:m.org".into()),
            device_id: None,
            allowed_users: vec![],
            allowed_rooms: vec!["!r:m".into()],
            interrupt_on_new_message: false,
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 1500,
            multi_message_delay_ms: 800,
            recovery_key: None,
            mention_only: false,
            password: None,
            approval_timeout_secs: 300,
            reply_in_thread: true,
            ack_reactions: true,
        }
    }

    #[test]
    async fn prop_fields_returns_typed_entries() {
        let mx = test_matrix_config();
        let fields = mx.prop_fields();
        let by_name: std::collections::HashMap<&str, &crate::traits::PropFieldInfo> =
            fields.iter().map(|f| (f.name.as_str(), f)).collect();

        // Bool field
        let enabled = by_name["channels.matrix.enabled"];
        assert_eq!(enabled.type_hint, "bool");
        assert_eq!(enabled.display_value, "true");
        assert!(!enabled.is_secret);
        assert!(!enabled.is_enum());

        // String field
        let homeserver = by_name["channels.matrix.homeserver"];
        assert_eq!(homeserver.type_hint, "String");
        assert_eq!(homeserver.display_value, "https://m.org");

        // Option<String> — set
        let user_id = by_name["channels.matrix.user-id"];
        assert_eq!(user_id.type_hint, "Option<String>");
        assert_eq!(user_id.display_value, "@bot:m.org");

        // Option<String> — unset
        let device_id = by_name["channels.matrix.device-id"];
        assert_eq!(device_id.display_value, "<unset>");

        // u64 field
        let interval = by_name["channels.matrix.draft-update-interval-ms"];
        assert_eq!(interval.type_hint, "u64");
        assert_eq!(interval.display_value, "1500");

        // Enum field
        let stream = by_name["channels.matrix.stream-mode"];
        assert!(stream.is_enum());
        assert!(stream.enum_variants.is_some());

        // Secret field — masked
        let token = by_name["channels.matrix.access-token"];
        assert!(token.is_secret);
        assert_eq!(token.display_value, "****");

        // All fields have correct category
        for field in &fields {
            assert_eq!(field.category, "Channels");
        }
    }

    #[test]
    async fn get_prop_returns_values_by_path() {
        let mx = test_matrix_config();

        assert_eq!(
            mx.get_prop("channels.matrix.homeserver").unwrap(),
            "https://m.org"
        );
        assert_eq!(mx.get_prop("channels.matrix.enabled").unwrap(), "true");
        assert_eq!(
            mx.get_prop("channels.matrix.draft-update-interval-ms")
                .unwrap(),
            "1500"
        );
        assert_eq!(
            mx.get_prop("channels.matrix.user-id").unwrap(),
            "@bot:m.org"
        );
        assert_eq!(mx.get_prop("channels.matrix.device-id").unwrap(), "<unset>");
        // Secrets return masked value
        assert_eq!(
            mx.get_prop("channels.matrix.access-token").unwrap(),
            "**** (encrypted)"
        );
    }

    #[test]
    async fn get_prop_unknown_path_fails() {
        let mx = test_matrix_config();
        assert!(mx.get_prop("channels.matrix.nonexistent").is_err());
    }

    #[test]
    async fn set_prop_string() {
        let mut mx = test_matrix_config();
        mx.set_prop("channels.matrix.homeserver", "https://new.org")
            .unwrap();
        assert_eq!(mx.homeserver, "https://new.org");
    }

    #[test]
    async fn set_prop_bool() {
        let mut mx = test_matrix_config();
        mx.set_prop("channels.matrix.interrupt-on-new-message", "true")
            .unwrap();
        assert!(mx.interrupt_on_new_message);
    }

    #[test]
    async fn set_prop_bool_rejects_invalid() {
        let mut mx = test_matrix_config();
        let err = mx.set_prop("channels.matrix.enabled", "yes").unwrap_err();
        assert!(err.to_string().contains("bool"));
    }

    #[test]
    async fn set_prop_u64() {
        let mut mx = test_matrix_config();
        mx.set_prop("channels.matrix.draft-update-interval-ms", "3000")
            .unwrap();
        assert_eq!(mx.draft_update_interval_ms, 3000);
    }

    #[test]
    async fn set_prop_u64_rejects_invalid() {
        let mut mx = test_matrix_config();
        assert!(
            mx.set_prop("channels.matrix.draft-update-interval-ms", "abc")
                .is_err()
        );
    }

    #[test]
    async fn set_prop_option_string_set_and_clear() {
        let mut mx = test_matrix_config();
        mx.set_prop("channels.matrix.user-id", "@new:m.org")
            .unwrap();
        assert_eq!(mx.user_id.as_deref(), Some("@new:m.org"));

        // Empty string clears Option
        mx.set_prop("channels.matrix.user-id", "").unwrap();
        assert!(mx.user_id.is_none());
    }

    #[test]
    async fn set_prop_enum() {
        let mut mx = test_matrix_config();
        mx.set_prop("channels.matrix.stream-mode", "partial")
            .unwrap();
        assert_eq!(mx.stream_mode, StreamMode::Partial);

        mx.set_prop("channels.matrix.stream-mode", "multi_message")
            .unwrap();
        assert_eq!(mx.stream_mode, StreamMode::MultiMessage);
    }

    #[test]
    async fn set_prop_enum_rejects_invalid() {
        let mut mx = test_matrix_config();
        let err = mx
            .set_prop("channels.matrix.stream-mode", "invalid")
            .unwrap_err();
        assert!(err.to_string().contains("expected one of"));
    }

    #[test]
    async fn set_prop_unknown_path_fails() {
        let mut mx = test_matrix_config();
        assert!(mx.set_prop("channels.matrix.nonexistent", "val").is_err());
    }

    #[test]
    async fn prop_is_secret_static_check() {
        assert!(MatrixConfig::prop_is_secret("channels.matrix.access-token"));
        assert!(MatrixConfig::prop_is_secret("channels.matrix.recovery-key"));
        assert!(!MatrixConfig::prop_is_secret("channels.matrix.homeserver"));
        assert!(!MatrixConfig::prop_is_secret("channels.matrix.enabled"));
    }

    #[test]
    async fn prop_is_secret_routes_through_hashmap_keyed_paths() {
        // Regression: the macro's HashMap<String, T> arm previously passed the
        // full materialised path (e.g. `providers.models.openrouter.api-key`)
        // straight to the inner type's `prop_is_secret`, which then matched on
        // its own configurable_prefix and returned false. Result: the CLI's
        // `config set --json` and the gateway's PropResponse both took the
        // non-secret branch and emitted `{value}` instead of `{populated}` for
        // any secret on a map-keyed nested type.
        assert!(Config::prop_is_secret(
            "providers.models.openrouter.api-key"
        ));
        assert!(Config::prop_is_secret("providers.models.default.api-key"));
        assert!(!Config::prop_is_secret(
            "providers.models.openrouter.endpoint"
        ));
        assert!(!Config::prop_is_secret(
            "providers.models.openrouter.context-window"
        ));
    }

    #[test]
    async fn hashmap_property_paths_preserve_url_like_keys() {
        let dir = TempDir::new().unwrap();
        let mut config = Config {
            config_path: dir.path().join("config.toml"),
            workspace_dir: dir.path().join("workspace"),
            ..Default::default()
        };
        let provider_key = "custom:https://api.example.invalid/v1";
        config
            .providers
            .models
            .insert(provider_key.to_string(), ModelProviderConfig::default());

        let api_key_path = format!("providers.models.{provider_key}.api-key");
        let base_url_path = format!("providers.models.{provider_key}.base-url");
        let model_path = format!("providers.models.{provider_key}.model");
        let temperature_path = format!("providers.models.{provider_key}.temperature");

        assert!(
            Config::prop_is_secret(&api_key_path),
            "url-like provider keys must still route secret metadata"
        );

        config.set_prop(&api_key_path, "sk-test-custom").unwrap();
        config
            .set_prop(&base_url_path, "https://api.example.invalid/v1")
            .unwrap();
        config.set_prop(&model_path, "local-large").unwrap();
        config.set_prop(&temperature_path, "0.2").unwrap();

        let provider = config
            .providers
            .models
            .get(provider_key)
            .expect("custom provider key should be preserved exactly");
        assert_eq!(provider.api_key.as_deref(), Some("sk-test-custom"));
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://api.example.invalid/v1")
        );
        assert_eq!(provider.model.as_deref(), Some("local-large"));
        assert_eq!(provider.temperature, Some(0.2));

        assert_eq!(config.get_prop(&api_key_path).unwrap(), "**** (encrypted)");
        assert_eq!(
            config.get_prop(&base_url_path).unwrap(),
            "https://api.example.invalid/v1"
        );

        config.save().await.unwrap();
        let raw_toml = tokio::fs::read_to_string(&config.config_path)
            .await
            .unwrap();
        assert!(
            raw_toml.contains(provider_key),
            "saved TOML should preserve the exact URL-like provider key"
        );
        assert!(
            !raw_toml.contains("sk-test-custom"),
            "saved TOML must not contain the plaintext custom provider API key"
        );

        let mut loaded: Config = toml::from_str::<crate::migration::V1Compat>(&raw_toml)
            .unwrap()
            .into_config();
        loaded.config_path = config.config_path.clone();
        loaded.workspace_dir = config.workspace_dir.clone();
        let store = crate::secrets::SecretStore::new(dir.path(), loaded.secrets.encrypt);
        loaded.decrypt_secrets(&store).unwrap();
        let loaded_provider = loaded
            .providers
            .models
            .get(provider_key)
            .expect("saved custom provider key should reload exactly");
        assert_eq!(
            loaded.providers.fallback.as_deref(),
            None,
            "property round-trip should not invent a fallback provider"
        );
        assert_eq!(loaded_provider.api_key.as_deref(), Some("sk-test-custom"));
        assert_eq!(
            loaded_provider.base_url.as_deref(),
            Some("https://api.example.invalid/v1")
        );
        assert_eq!(loaded_provider.model.as_deref(), Some("local-large"));
        assert_eq!(loaded_provider.temperature, Some(0.2));
    }

    #[test]
    async fn enum_variants_callback_returns_values() {
        let mx = test_matrix_config();
        let fields = mx.prop_fields();
        let stream_field = fields
            .iter()
            .find(|f| f.name == "channels.matrix.stream-mode")
            .unwrap();
        let variants = (stream_field.enum_variants.unwrap())();
        assert!(variants.contains(&"off".to_string()));
        assert!(variants.contains(&"partial".to_string()));
        assert!(variants.contains(&"multi_message".to_string()));
    }

    #[test]
    async fn map_key_sections_discovers_providers_models() {
        // The Configurable derive walks #[nested] HashMap<String, T> fields
        // and exposes them via map_key_sections(). Without this enumeration,
        // the dashboard has no way to know `providers.models.<name>` is an
        // addable shape — it only sees fields that already exist.
        let sections = Config::map_key_sections();
        let providers_models = sections
            .iter()
            .find(|s| s.path == "providers.models")
            .expect("providers.models must be discoverable as a map-keyed section");
        assert_eq!(providers_models.kind, crate::traits::MapKeyKind::Map);
        assert_eq!(providers_models.value_type, "ModelProviderConfig");

        // agents is also #[nested] HashMap on root Config.
        assert!(
            sections.iter().any(|s| s.path == "agents"),
            "agents map should be discoverable"
        );

        // mcp.servers is a Vec<McpServerConfig> with #[nested] — should
        // surface as a List-kind section so the dashboard's "+ Add MCP
        // server" affordance picks it up. Without this, dashboard users
        // hit a silent dead-end and have to hand-edit config.toml. Pinned
        // here so a regression that drops the #[nested] annotation or the
        // Configurable derive on McpServerConfig fails CI.
        let mcp_servers = sections
            .iter()
            .find(|s| s.path == "mcp.servers")
            .expect("mcp.servers must be discoverable as a list-shaped section");
        assert_eq!(mcp_servers.kind, crate::traits::MapKeyKind::List);
        assert_eq!(mcp_servers.value_type, "McpServerConfig");
    }

    #[test]
    async fn create_map_key_inserts_default_mcp_server() {
        // Round-trip: `POST /api/config/map-key?path=mcp.servers&key=github`.
        // The new entry's `name` field is initialized to the supplied key
        // by the macro's List-kind insertion logic.
        let mut config = Config::default();
        assert!(config.mcp.servers.is_empty());

        let created = config
            .create_map_key("mcp.servers", "github")
            .expect("mcp.servers should accept new list entries");
        assert!(created, "first add should report created=true");
        assert_eq!(config.mcp.servers.len(), 1);
        assert_eq!(
            config.mcp.servers[0].name, "github",
            "new entry must carry the supplied key as its name field"
        );
    }

    #[test]
    async fn create_map_key_inserts_default_provider() {
        // Round-trip: `+ Add anthropic provider` from the dashboard.
        let mut config = Config::default();
        assert!(!config.providers.models.contains_key("anthropic"));

        let created = config
            .create_map_key("providers.models", "anthropic")
            .expect("providers.models should accept new map keys");
        assert!(created, "first add should report created=true");
        assert!(config.providers.models.contains_key("anthropic"));

        // Idempotent: second add returns false, doesn't error.
        let again = config
            .create_map_key("providers.models", "anthropic")
            .expect("second add still resolves the section");
        assert!(!again, "duplicate add should report created=false");
    }

    #[test]
    async fn create_map_key_rejects_unknown_section() {
        let mut config = Config::default();
        let err = config
            .create_map_key("not.a.real.section", "anything")
            .expect_err("unknown section path should error");
        assert!(err.contains("not.a.real.section"));
    }

    #[test]
    async fn init_defaults_instantiates_none_sections() {
        let mut config = Config::default();
        assert!(config.channels.matrix.is_none());

        let initialized = config.init_defaults(Some("channels.matrix"));
        assert!(initialized.contains(&"channels.matrix"));
        assert!(config.channels.matrix.is_some());
    }

    #[test]
    async fn deserialized_matrix_set_prop_round_trips_vec_string() {
        // Mirror the real-world daemon flow: config loaded from disk where
        // [channels.matrix] is present (possibly with all default fields),
        // then a PATCH from the dashboard hits set_prop.
        let toml_src = r#"
schema_version = 2

[channels.matrix]
enabled = false
homeserver = ""
access_token = ""
allowed_rooms = []
allowed_users = []
"#;
        let mut config: Config = toml::from_str(toml_src).expect("parse toml");
        assert!(
            config.channels.matrix.is_some(),
            "matrix must be Some after deserialize"
        );

        config
            .set_prop("channels.matrix.allowed-rooms", r#"["alice","bob"]"#)
            .expect("set_prop should succeed against deserialized matrix");
        assert_eq!(
            config.channels.matrix.as_ref().unwrap().allowed_rooms,
            vec!["alice".to_string(), "bob".to_string()],
        );
    }

    #[test]
    async fn init_defaults_then_set_prop_round_trips_vec_string() {
        // Regression for #6175 Channels picker → form → save:
        // 1. init_defaults creates channels.matrix = Some(MatrixConfig::default())
        // 2. set_prop on channels.matrix.allowed-rooms must accept a JSON-array
        //    string (the shape coerce_for_set_prop emits for Vec<String>).
        // 3. get_prop reads it back.
        let mut config = Config::default();
        let initialized = config.init_defaults(Some("channels.matrix"));
        assert!(initialized.contains(&"channels.matrix"));
        assert!(config.channels.matrix.is_some());

        // prop_fields must surface the kebab path so the form can render it.
        let has_field = config
            .prop_fields()
            .iter()
            .any(|f| f.name == "channels.matrix.allowed-rooms");
        assert!(
            has_field,
            "channels.matrix.allowed-rooms must appear in prop_fields after init"
        );

        // set_prop with the JSON-array string the gateway PATCH path produces.
        config
            .set_prop("channels.matrix.allowed-rooms", r#"["alice","bob"]"#)
            .expect("set_prop should accept JSON-array string for Vec<String>");
        assert_eq!(
            config.channels.matrix.as_ref().unwrap().allowed_rooms,
            vec!["alice".to_string(), "bob".to_string()],
        );
    }

    #[test]
    async fn mcp_servers_addable_via_create_map_key_and_per_entry_props() {
        // `mcp.servers` is a `Vec<McpServerConfig>` with `#[nested]`, so the
        // `Configurable` derive surfaces it as a List section (not an
        // ObjectArray prop) — operators add servers via
        // `POST /api/config/map-key?path=mcp.servers&key=<name>` and edit
        // each server's fields via per-prop GET/PUT.
        //
        // This replaces the prior model where the entire Vec round-tripped
        // through set_prop("mcp.servers", "<json-array>"). The List model
        // matches the rest of the schema (`providers.models`, `agents`,
        // etc.) and gives the dashboard a per-field editor instead of a
        // monolithic JSON blob.
        let mut config = Config::default();

        // The List section is discoverable.
        let sections = Config::map_key_sections();
        assert!(
            sections
                .iter()
                .any(|s| s.path == "mcp.servers" && s.kind == crate::traits::MapKeyKind::List),
            "mcp.servers should surface as a List section in map_key_sections()"
        );

        // create_map_key inserts a default-valued entry and seeds its
        // `name` field from the supplied key.
        config
            .create_map_key("mcp.servers", "fs")
            .expect("mcp.servers should accept new list entries via create_map_key");
        assert_eq!(config.mcp.servers.len(), 1);
        assert_eq!(config.mcp.servers[0].name, "fs");

        // Per-entry fields are mutated via standard set_prop on the inner
        // path (the same call site the per-prop PUT handler uses); the
        // McpServerConfig schema's `#[prefix = "mcp.servers"]` makes the
        // path resolution work without hand-table dispatch.
        // (Wider per-entry path routing through Vec<T> requires a
        // future generalization of route_hashmap_path-equivalent for
        // List sections; tracked as future work.)
    }

    #[test]
    async fn ensure_default_mcp_servers_injects_agentmemory_when_backend_selects_it() {
        // Mirrors operant_core::config::ensure_default_mcp_servers (AppConfig
        // path) for the schema Config used by the runtime daemon/gateway.
        let mut config = Config::default();
        config.mcp.enabled = true;
        config.memory.backend = "agentmemory".to_string();

        config.ensure_default_mcp_servers();

        assert_eq!(config.mcp.servers.len(), 1);
        let server = &config.mcp.servers[0];
        assert_eq!(server.name, "agentmemory");
        assert_eq!(server.transport, McpTransport::Stdio);
        assert_eq!(server.command, "npx");
        assert_eq!(
            server.args,
            vec!["-y".to_string(), "@agentmemory/mcp".to_string()]
        );
        assert_eq!(
            server.env.get("AGENTMEMORY_URL").map(String::as_str),
            Some("http://localhost:3111")
        );

        // Idempotent: a second call must not duplicate the server.
        config.ensure_default_mcp_servers();
        assert_eq!(config.mcp.servers.len(), 1);
    }

    #[test]
    async fn ensure_default_mcp_servers_noop_without_agentmemory_backend() {
        let mut config = Config::default();
        config.mcp.enabled = true;
        config.memory.backend = "sqlite".to_string();

        config.ensure_default_mcp_servers();
        assert!(config.mcp.servers.is_empty());

        // Disabled MCP also prevents injection even with the backend set.
        config.memory.backend = "agentmemory".to_string();
        config.mcp.enabled = false;
        config.ensure_default_mcp_servers();
        assert!(config.mcp.servers.is_empty());
    }

    #[test]
    async fn ensure_default_mcp_servers_skips_when_user_configured() {
        let mut config = Config::default();
        config.mcp.enabled = true;
        config.memory.backend = "agentmemory".to_string();
        config.mcp.servers.push(McpServerConfig {
            name: "agentmemory".to_string(),
            transport: McpTransport::Stdio,
            url: None,
            command: "custom".to_string(),
            args: vec![],
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            tool_timeout_secs: None,
        });

        config.ensure_default_mcp_servers();

        // User's own agentmemory server is preserved untouched.
        assert_eq!(config.mcp.servers.len(), 1);
        assert_eq!(config.mcp.servers[0].command, "custom");
    }

    #[test]
    async fn init_defaults_skips_already_set() {
        let mut config = Config::default();
        config.channels.matrix = Some(test_matrix_config());

        let initialized = config.init_defaults(Some("channels.matrix"));
        // Already set — should not re-initialize
        assert!(!initialized.contains(&"channels.matrix"));
        // Original value preserved
        assert_eq!(
            config.channels.matrix.as_ref().unwrap().homeserver,
            "https://m.org"
        );
    }

    #[test]
    async fn nested_get_set_prop_traverses_config_tree() {
        let mut config = Config::default();
        config.channels.matrix = Some(test_matrix_config());

        // get_prop traverses Config → ChannelsConfig → MatrixConfig
        assert_eq!(
            config.get_prop("channels.matrix.homeserver").unwrap(),
            "https://m.org"
        );

        // set_prop traverses the same path
        config
            .set_prop("channels.matrix.homeserver", "https://new.org")
            .unwrap();
        assert_eq!(
            config.channels.matrix.as_ref().unwrap().homeserver,
            "https://new.org"
        );
    }

    #[test]
    async fn hashmap_nested_encrypt_decrypt_traverses_values() {
        let dir = TempDir::new().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path(), true);

        let mut config = Config::default();
        config.agents.insert(
            "test-agent".into(),
            DelegateAgentConfig {
                api_key: Some("secret-key".into()),
                ..Default::default()
            },
        );

        config.encrypt_secrets(&store).unwrap();
        let encrypted_key = config.agents["test-agent"].api_key.as_ref().unwrap();
        assert!(crate::secrets::SecretStore::is_encrypted(encrypted_key));

        config.decrypt_secrets(&store).unwrap();
        assert_eq!(
            config.agents["test-agent"].api_key.as_deref(),
            Some("secret-key")
        );
    }

    #[test]
    async fn vec_secret_encrypt_decrypt_traverses_elements() {
        let dir = TempDir::new().unwrap();
        let store = crate::secrets::SecretStore::new(dir.path(), true);

        let mut config = Config::default();
        config.gateway.paired_tokens = vec!["token-a".into(), "token-b".into()];

        config.encrypt_secrets(&store).unwrap();
        for token in &config.gateway.paired_tokens {
            assert!(crate::secrets::SecretStore::is_encrypted(token));
        }

        config.decrypt_secrets(&store).unwrap();
        assert_eq!(config.gateway.paired_tokens, vec!["token-a", "token-b"]);
    }

    /// Walk every property on a default Config: get_prop must succeed,
    /// and set_prop must round-trip for non-secret, non-enum scalar fields.
    #[test]
    async fn every_prop_is_gettable_and_settable() {
        let mut config = Config::default();
        // Initialize all Option<T> sections so their fields are reachable
        config.init_defaults(None);

        let fields = config.prop_fields();
        assert!(
            fields.len() > 50,
            "Expected 50+ props, got {} — macro may be skipping fields",
            fields.len()
        );

        for field in &fields {
            // get_prop must not panic or error
            let get_result = config.get_prop(&field.name);
            assert!(
                get_result.is_ok(),
                "get_prop failed for '{}': {}",
                field.name,
                get_result.unwrap_err()
            );

            // set_prop: round-trip the display value back through set_prop.
            // Skip secrets (masked), enums (need valid variant), and <unset> Options.
            if field.is_secret || field.is_enum() || field.display_value == "<unset>" {
                continue;
            }

            let set_result = config.set_prop(&field.name, &field.display_value);
            assert!(
                set_result.is_ok(),
                "set_prop failed for '{}' with value '{}': {}",
                field.name,
                field.display_value,
                set_result.unwrap_err()
            );

            // Value should survive the round-trip
            let after = config.get_prop(&field.name).unwrap();
            assert_eq!(
                after, field.display_value,
                "round-trip mismatch for '{}': set '{}', got '{}'",
                field.name, field.display_value, after
            );
        }
    }

    /// Every enum field must have a working enum_variants callback, and
    /// set_prop must accept each variant it advertises.
    #[test]
    async fn every_enum_variant_is_settable() {
        let mut config = Config::default();
        config.init_defaults(None);

        for field in config.prop_fields() {
            if !field.is_enum() {
                continue;
            }
            let get_variants = field.enum_variants.unwrap_or_else(|| {
                panic!("enum field '{}' has no enum_variants callback", field.name)
            });
            let variants = get_variants();
            assert!(
                !variants.is_empty(),
                "enum field '{}' returned no variants",
                field.name
            );

            for variant in &variants {
                let result = config.set_prop(&field.name, variant);
                assert!(
                    result.is_ok(),
                    "set_prop('{}', '{}') failed: {}",
                    field.name,
                    variant,
                    result.unwrap_err()
                );
            }
        }
    }

    #[test]
    async fn backfill_enabled_activates_channel_without_explicit_enabled() {
        let toml = r#"
[channels.matrix]
homeserver = "https://matrix.org"
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
"#;
        let mut config: Config = toml::from_str(toml).unwrap();
        assert!(!config.channels.matrix.as_ref().unwrap().enabled);

        config.channels.backfill_enabled(toml);
        assert!(config.channels.matrix.as_ref().unwrap().enabled);
    }

    #[test]
    async fn backfill_enabled_respects_explicit_false() {
        let toml = r#"
[channels.matrix]
homeserver = "https://matrix.org"
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
enabled = false
"#;
        let mut config: Config = toml::from_str(toml).unwrap();
        config.channels.backfill_enabled(toml);
        assert!(
            !config.channels.matrix.as_ref().unwrap().enabled,
            "explicit enabled=false must not be overwritten"
        );
    }

    #[test]
    async fn backfill_enabled_no_op_when_section_absent() {
        let toml = r#"
api_key = "sk-test"
"#;
        let mut config: Config = toml::from_str(toml).unwrap();
        config.channels.backfill_enabled(toml);
        assert!(config.channels.telegram.is_none());
    }

    #[test]
    async fn backfill_enabled_works_with_toml_comments() {
        let toml = r#"
# My matrix setup
[channels.matrix]
homeserver = "https://matrix.org"  # production server
access_token = "tok"
allowed_rooms = ["!r:m"]
allowed_users = ["@u:m"]
# enabled intentionally omitted
"#;
        let mut config: Config = toml::from_str(toml).unwrap();
        assert!(!config.channels.matrix.as_ref().unwrap().enabled);

        config.channels.backfill_enabled(toml);
        assert!(
            config.channels.matrix.as_ref().unwrap().enabled,
            "backfill should activate channel even when config has comments"
        );
    }

    #[test]
    async fn channel_approval_timeout_secs_defaults_to_300() {
        // Omitting approval_timeout_secs from each config should deserialize to 300
        let discord: DiscordConfig =
            serde_json::from_str(r#"{"bot_token":"tok","enabled":true}"#).unwrap();
        assert_eq!(discord.approval_timeout_secs, 300);

        let slack: SlackConfig =
            serde_json::from_str(r#"{"bot_token":"tok","enabled":true}"#).unwrap();
        assert_eq!(slack.approval_timeout_secs, 300);

        let signal: SignalConfig = serde_json::from_str(
            r#"{"http_url":"http://localhost","account":"+1","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(signal.approval_timeout_secs, 300);

        let matrix: MatrixConfig = serde_json::from_str(
            r#"{"homeserver":"https://matrix.org","access_token":"tok","enabled":true,"allowed_users":[]}"#,
        )
        .unwrap();
        assert_eq!(matrix.approval_timeout_secs, 300);

        let whatsapp: WhatsAppConfig = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(whatsapp.approval_timeout_secs, 300);
    }

    #[test]
    async fn channel_approval_timeout_secs_explicit_override() {
        let discord: DiscordConfig = serde_json::from_str(
            r#"{"bot_token":"tok","enabled":true,"approval_timeout_secs":60}"#,
        )
        .unwrap();
        assert_eq!(discord.approval_timeout_secs, 60);

        let slack: SlackConfig = serde_json::from_str(
            r#"{"bot_token":"tok","enabled":true,"approval_timeout_secs":120}"#,
        )
        .unwrap();
        assert_eq!(slack.approval_timeout_secs, 120);

        let signal: SignalConfig = serde_json::from_str(
            r#"{"http_url":"http://localhost","account":"+1","enabled":true,"approval_timeout_secs":90}"#,
        )
        .unwrap();
        assert_eq!(signal.approval_timeout_secs, 90);

        let matrix: MatrixConfig = serde_json::from_str(
            r#"{"homeserver":"https://matrix.org","access_token":"tok","enabled":true,"allowed_users":[],"approval_timeout_secs":45}"#,
        )
        .unwrap();
        assert_eq!(matrix.approval_timeout_secs, 45);

        let whatsapp: WhatsAppConfig =
            serde_json::from_str(r#"{"enabled":true,"approval_timeout_secs":180}"#).unwrap();
        assert_eq!(whatsapp.approval_timeout_secs, 180);
    }

    // ── combined_pricing: per-provider + top-level merge (#6251) ──────

    fn pricing(input: f64, output: f64) -> ModelPricing {
        ModelPricing { input, output }
    }

    fn config_with_provider(
        provider_id: &str,
        model: Option<&str>,
        per_provider_pricing: Option<ModelPricing>,
    ) -> Config {
        let mut config = Config::default();
        // Start clean: Config::default() seeds CostConfig::default() which calls
        // get_default_pricing(); for these tests we want a deterministic baseline.
        config.cost.prices.clear();
        config.providers.models.insert(
            provider_id.to_string(),
            ModelProviderConfig {
                model: model.map(ToString::to_string),
                pricing: per_provider_pricing,
                ..ModelProviderConfig::default()
            },
        );
        config
    }

    #[test]
    async fn combined_pricing_passes_through_when_no_per_provider_pricing() {
        let mut config = config_with_provider("openai", Some("gpt-4o"), None);
        config
            .cost
            .prices
            .insert("openai/gpt-4o".into(), pricing(2.5, 10.0));

        let combined = config.combined_pricing();
        assert_eq!(combined.len(), 1);
        let entry = combined.get("openai/gpt-4o").expect("top-level entry");
        assert_eq!(entry.input, 2.5);
        assert_eq!(entry.output, 10.0);
    }

    #[test]
    async fn combined_pricing_merges_per_provider_into_provider_slash_model_key() {
        let config = config_with_provider(
            "anthropic",
            Some("claude-sonnet-4-5"),
            Some(pricing(3.0, 15.0)),
        );

        let combined = config.combined_pricing();
        let entry = combined
            .get("anthropic/claude-sonnet-4-5")
            .expect("per-provider pricing keyed as <provider_id>/<model>");
        assert_eq!(entry.input, 3.0);
        assert_eq!(entry.output, 15.0);
    }

    #[test]
    async fn combined_pricing_top_level_wins_on_conflict() {
        let mut config = config_with_provider(
            "anthropic",
            Some("claude-sonnet-4-5"),
            Some(pricing(3.0, 15.0)),
        );
        // Operator pinned a different rate at the top level — must survive.
        config
            .cost
            .prices
            .insert("anthropic/claude-sonnet-4-5".into(), pricing(2.0, 8.0));

        let combined = config.combined_pricing();
        let entry = combined.get("anthropic/claude-sonnet-4-5").unwrap();
        assert_eq!(
            entry.input, 2.0,
            "top-level [cost.prices] override must not be silently shadowed by per-provider pricing"
        );
        assert_eq!(entry.output, 8.0);
    }

    #[test]
    async fn combined_pricing_skips_provider_with_no_model() {
        // Provider has pricing but no `model` set — we cannot synthesize the
        // <provider_id>/<model> key, so the entry must be skipped (not crash,
        // not produce a malformed key).
        let config = config_with_provider("openrouter", None, Some(pricing(1.0, 2.0)));

        let combined = config.combined_pricing();
        assert!(
            combined.is_empty(),
            "per-provider pricing without `model` is silently skipped, got {combined:?}"
        );
    }

    #[test]
    async fn combined_pricing_skips_provider_with_empty_model() {
        let config = config_with_provider("openrouter", Some(""), Some(pricing(1.0, 2.0)));

        let combined = config.combined_pricing();
        assert!(
            combined.is_empty(),
            "empty model string must be treated the same as missing, got {combined:?}"
        );
    }

    // ── set_prop round-trip for `pricing` (#6357 review) ──────────────
    //
    // Dashboard / JSON-patch callers send `pricing` as a JSON object
    // (`{"input": 1.0, "output": 2.5}`). Before #6357 review, `pricing` was
    // classified as `PropKind::String`, which meant `parse_prop_value`
    // inserted the JSON text as a TOML string and serde failed to
    // deserialize it back into `Option<ModelPricing>`. The fix classifies
    // `ModelPricing` as `PropKind::Object` and routes through `json_to_toml`
    // so the value lands as a typed inline table. These tests pin that
    // contract end-to-end through `Config::set_prop` and the wire-form
    // coercion at `coerce_for_set_prop`.

    #[test]
    async fn set_prop_round_trips_per_provider_pricing_object() {
        let mut config = config_with_provider("openai", Some("gpt-4o"), None);

        // Caller hits `Config::set_prop` directly with the JSON-stringified
        // object that the dashboard / CLI hand off after type coercion.
        config
            .set_prop(
                "providers.models.openai.pricing",
                r#"{"input": 1.5, "output": 6.0}"#,
            )
            .expect("set_prop must accept a JSON object for pricing");

        // Round-trip 1: typed access on the struct must reflect the write.
        let pricing = config
            .providers
            .models
            .get("openai")
            .and_then(|m| m.pricing.clone())
            .expect("pricing must round-trip back into a typed ModelPricing, not a string");
        assert_eq!(pricing.input, 1.5);
        assert_eq!(pricing.output, 6.0);

        // Round-trip 2: the merged pricing map (the runtime/channel cost
        // contexts consume this) must show the new values keyed under
        // `<provider_id>/<model>`.
        let combined = config.combined_pricing();
        let entry = combined
            .get("openai/gpt-4o")
            .expect("set_prop write must surface in combined_pricing");
        assert_eq!(entry.input, 1.5);
        assert_eq!(entry.output, 6.0);
    }

    #[test]
    async fn set_prop_pricing_rejects_non_object_string_payload() {
        // Sanity: a quoted JSON string (which is what the buggy
        // `PropKind::String` path would have silently accepted by writing
        // the raw text into a TOML string field) must NOT round-trip into
        // a typed `ModelPricing`. The fix is `PropKind::Object`, which
        // requires a JSON object; anything else fails set_prop and leaves
        // the field unchanged. We assert the failure is observable AND
        // that no garbage state was written.
        let mut config = config_with_provider("openai", Some("gpt-4o"), None);

        let result = config.set_prop(
            "providers.models.openai.pricing",
            "\"{\\\"input\\\":1.5,\\\"output\\\":6.0}\"",
        );
        assert!(
            result.is_err(),
            "a JSON string in place of an object must be rejected by set_prop"
        );
        // Pricing must still be `None` — no partial / garbage write.
        let pricing_after = config
            .providers
            .models
            .get("openai")
            .and_then(|m| m.pricing.clone());
        assert!(
            pricing_after.is_none(),
            "rejected set_prop must not mutate pricing, got {pricing_after:?}"
        );
    }

    #[test]
    async fn coerce_for_set_prop_object_round_trips_pricing_payload() {
        use crate::traits::PropKind;
        use crate::typed_value::coerce_for_set_prop;

        // Dashboard / CLI sends a real JSON object; the coercion layer
        // must hand it back as a JSON-stringified object (the wire form
        // that `parse_prop_value`'s `Object` arm consumes).
        let coerced = coerce_for_set_prop(
            &serde_json::json!({"input": 1.5, "output": 6.0}),
            Some(PropKind::Object),
        )
        .expect("JSON object payload must coerce successfully for an Object field");
        let parsed: serde_json::Value =
            serde_json::from_str(&coerced).expect("coerced output must remain valid JSON");
        assert_eq!(parsed, serde_json::json!({"input": 1.5, "output": 6.0}));
    }

    #[test]
    async fn coerce_for_set_prop_object_rejects_non_object() {
        use crate::traits::PropKind;
        use crate::typed_value::coerce_for_set_prop;

        let err = coerce_for_set_prop(
            &serde_json::json!("{\"input\":1.5}"),
            Some(PropKind::Object),
        )
        .expect_err("a JSON string is not a JSON object — must be rejected");
        assert!(
            err.message.contains("object"),
            "rejection message must name the object requirement, got: {}",
            err.message
        );
    }
}

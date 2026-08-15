use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;

// Re-export the `[providers]` section types so consumers (CLI run path) can
// name provider profiles without depending on operant-config directly.
pub use operant_config::providers::ProvidersConfig;
pub use operant_config::schema::{FallbackProviderConfig, ModelProviderConfig};

static RUNTIME_CONFIG: OnceLock<RwLock<AppConfig>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Configuration format version. None = v0 (legacy, no version field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub client: ClientSettings,
    pub agent: BehaviorSettings,
    pub autonomous: AutonomousSettings,
    pub logging: LoggingSettings,
    pub tui: TuiSettings,
    pub mcp: McpSettings,
    pub skills: SkillsSettings,
    pub gateway: GatewaySettings,
    pub plugins: PluginSettings,
    pub tools: ToolSettings,
    pub tts: TtsSettings,
    pub memory: MemorySettings,
    pub browser: BrowserSettings,
    pub vision: VisionSettings,
    pub credential_pool: CredentialPoolSettings,
    pub terminal_backend: TerminalBackend,
    pub auxiliary_models: AuxiliaryModels,
    pub moa: MoaSettings,
    pub checkpoints: CheckpointsSettings,
    pub database_path: PathBuf,
    /// Provider profiles + ordered cross-provider fallback chain
    /// (`[providers]`, hermes `fallback_providers` parity). Consumed by the
    /// run path to build a [`crate::agent::provider_registry::ProviderRegistry`]
    /// so auth/billing failures can switch providers; same-provider model
    /// fallback is driven by `agent.fallback_models`.
    #[serde(default)]
    pub providers: operant_config::providers::ProvidersConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let database_path = dirs::home_dir()
            .map(|h| h.join(".operant").join("database.db"))
            .unwrap_or_else(|| PathBuf::from("database.db"));

        Self {
            version: Some(2),
            client: ClientSettings::default(),
            agent: BehaviorSettings::default(),
            autonomous: AutonomousSettings::default(),
            logging: LoggingSettings::default(),
            tui: TuiSettings::default(),
            mcp: McpSettings::default(),
            skills: SkillsSettings::default(),
            gateway: GatewaySettings::default(),
            plugins: PluginSettings::default(),
            tools: ToolSettings::default(),
            tts: TtsSettings::default(),
            memory: MemorySettings::default(),
            browser: BrowserSettings::default(),
            vision: VisionSettings::default(),
            credential_pool: CredentialPoolSettings::default(),
            terminal_backend: TerminalBackend::Local,
            checkpoints: CheckpointsSettings::default(),
            auxiliary_models: AuxiliaryModels::default(),
            moa: MoaSettings::default(),
            database_path,
            providers: operant_config::providers::ProvidersConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientSettings {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Additional API keys for credential pool / multi-key rotation.
    #[serde(default)]
    pub additional_api_keys: Vec<String>,
    pub timeout_secs: u64,
    pub max_context_length: usize,
    /// Rate limit configuration for outbound API requests.
    pub rate_limit: RateLimitSettings,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: None,
            additional_api_keys: Vec::new(),
            timeout_secs: 60,
            max_context_length: 128_000,
            rate_limit: RateLimitSettings::default(),
        }
    }
}

/// Settings for token-bucket rate limiting of outbound API requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimitSettings {
    /// Maximum number of retry attempts after a 429 or transient error.
    pub max_retries: u32,
    /// Base delay (seconds) for exponential backoff.
    pub base_delay_secs: u64,
    /// Maximum delay (seconds) for exponential backoff.
    pub max_delay_secs: u64,
    /// Default token-bucket capacity (max requests in a burst).
    pub bucket_capacity: u32,
    /// Default token refill rate (tokens per second).
    pub bucket_refill_rate: f64,
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_secs: 5,
            max_delay_secs: 60,
            bucket_capacity: 60,
            bucket_refill_rate: 1.0,
        }
    }
}

/// How tool execution progress is reported to the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ToolProgressMode {
    PerStep,
    FinalOnly,
    Streaming,
    #[default]
    Auto,
}

/// When the conversation session should be automatically reset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionResetMode {
    #[default]
    Never,
    OnSystemPromptChange,
    OnToolChange,
    Always,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BehaviorSettings {
    pub model: String,
    pub max_iterations: usize,
    pub tool_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub system_prompt: Option<String>,
    pub stream: bool,
    pub context_window: usize,
    pub max_healing_attempts: usize,
    pub show_reasoning: bool,
    pub tool_progress: ToolProgressMode,
    pub session_reset: SessionResetMode,
    pub context_compression: bool,
    pub context_compression_threshold: f64,
    /// Max consecutive tool-only iterations before force-answer kicks in.
    /// When the LLM calls tools N times in a row without producing text,
    /// the agent omits tools from the request to force a textual response.
    /// Set to 0 to disable (use max_iterations as the only limit).
    pub max_consecutive_tool_only: usize,

    /// Ordered list of fallback models to try when the primary model fails
    /// with a retryable provider error (5xx, 429, network error).
    /// The models are tried in order until one succeeds or all are exhausted.
    /// Fallback is per-request — the next request starts again with the primary model.
    #[serde(default)]
    pub fallback_models: Vec<String>,

    /// Whether to automatically fall back to `fallback_models` on provider errors.
    /// Default: `true`. Set to `false` to disable fallback behavior entirely.
    #[serde(default = "default_fallback_on_errors")]
    pub fallback_on_errors: bool,

    /// Turns between memory reviews (0 = disabled). Mirrors hermes-agent's
    /// `memory_nudge_interval` (default 10).
    #[serde(default = "default_memory_nudge_interval")]
    pub memory_nudge_interval: usize,

    /// Iterations between skill nudges (0 = disabled). Mirrors hermes-agent's
    /// `creation_nudge_interval` (default 10).
    #[serde(default = "default_creation_nudge_interval")]
    pub creation_nudge_interval: usize,

    /// Context engine used to assemble the per-call message list.
    ///   - `"compact"` (default): deterministic decay + `evict_to_budget`
    ///     (current behavior — lossy when over budget).
    ///   - `"lcm"`: lossless DAG + fresh-tail assembly (hermes-lcm parity;
    ///     see docs/HERMES_LCM_INTEGRATION.md). Opt-in — the default is
    ///     unchanged until rollups ship in P1.
    #[serde(default = "default_context_engine")]
    pub context_engine: String,
    /// SQLite path for the LCM lossless DAG (default `~/.operant/lcm.db`).
    #[serde(default)]
    pub context_lcm_db: Option<PathBuf>,
    /// Fresh-tail (D0) token budget kept verbatim by the LCM engine.
    /// Older messages are compacted into the DAG and recallable verbatim.
    #[serde(default = "default_context_lcm_tail_tokens")]
    pub context_lcm_tail_tokens: usize,
    /// P3 adaptive auto-recall: when the LCM engine assembles the context,
    /// run one bounded retrieval round against the latest user message and
    /// inject the top hits as a system "pre-answer evidence" block (hermes
    /// `adaptive_retrieval.py` / pre-answer evidence parity). Default on —
    /// costs one FTS query per turn.
    #[serde(default = "default_true")]
    pub context_lcm_auto_recall: bool,
    /// Max nodes auto-recalled and injected per assemble (default 3).
    #[serde(default = "default_lcm_auto_recall_limit")]
    pub context_lcm_auto_recall_limit: usize,
    /// Hard cap on the injected evidence block, in characters (default 4000).
    #[serde(default = "default_lcm_auto_recall_max_chars")]
    pub context_lcm_auto_recall_max_chars: usize,
    /// P1 rollup-in-compaction: when the LCM engine compacts an over-budget
    /// context, inject stored day/week/month rollup summaries instead of a
    /// bare placeholder marker. Default on — rollups are only injected when
    /// they already exist (built via `operant context rollup`).
    #[serde(default = "default_true")]
    pub context_lcm_rollups_inject: bool,
    /// P1 background rollup maintenance: interval in minutes between
    /// automatic maintenance passes that build missing day/week/month
    /// rollups for all DAG sessions (hermes `_RollupMaintenanceScheduler`
    /// parity). `0` (default) disables the background task — run
    /// `operant context rollup-maintenance` manually instead.
    #[serde(default = "default_lcm_rollup_interval_minutes")]
    pub context_lcm_rollup_interval_minutes: u64,
    /// P3 vector recall: embedding model for `lcm_vector_recall` (hermes
    /// `embedding_provider.py` parity). Empty (default) disables the vector
    /// tool — the provider must expose the OpenAI-compatible `/embeddings`
    /// endpoint.
    #[serde(default)]
    pub context_lcm_embedding_model: Option<String>,
    /// Base URL for the embeddings endpoint when it differs from the chat
    /// provider (e.g. a local Ollama at `http://localhost:11434/v1`). Empty
    /// (default) reuses the chat provider's base URL and key.
    #[serde(default)]
    pub context_lcm_embedding_base_url: Option<String>,
    /// P3 LLM-driven assertion extraction (hermes `assertion_extraction.py`
    /// `ModelAssertionExtractor` parity, opt-in): when true, the
    /// `lcm_assert` tool's `action = "extract"` mines durable
    /// (subject, predicate, object) facts out of the most recent DAG nodes
    /// and persists them to the assertion store. Off by default — costs one
    /// LLM call per extract invocation.
    #[serde(default)]
    pub context_lcm_assertion_extraction: bool,
    /// hermes-lcm `ignore_session_patterns` parity: glob patterns (fnmatch
    /// `*`/`?`/`[...]`) of sessions to skip in global DAG recall (FTS,
    /// vector, recent). Explicit per-session recall is unaffected. Empty
    /// (default) recalls across every session.
    #[serde(default)]
    pub context_lcm_ignore_session_patterns: Vec<String>,
    /// hermes-lcm `read_only` session scopes parity: session ids that must
    /// never be mutated — ingest is a no-op so archived transcripts stay
    /// byte-for-byte stable. Empty (default) allows all sessions to write.
    #[serde(default)]
    pub context_lcm_readonly_sessions: Vec<String>,
}

fn default_fallback_on_errors() -> bool {
    true
}

fn default_memory_nudge_interval() -> usize {
    10
}

fn default_creation_nudge_interval() -> usize {
    10
}

fn default_context_engine() -> String {
    "compact".to_string()
}

fn default_context_lcm_tail_tokens() -> usize {
    12_000
}

fn default_lcm_auto_recall_limit() -> usize {
    3
}

fn default_lcm_auto_recall_max_chars() -> usize {
    4_000
}

fn default_lcm_rollup_interval_minutes() -> u64 {
    0
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            model: "gpt-4".to_string(),
            max_iterations: 90,
            tool_timeout_secs: 30,
            request_timeout_secs: 120,
            system_prompt: None,
            stream: true,
            context_window: 128_000,
            max_healing_attempts: 3,
            show_reasoning: true,
            tool_progress: ToolProgressMode::Auto,
            session_reset: SessionResetMode::Never,
            context_compression: false,
            context_compression_threshold: 0.5,
            max_consecutive_tool_only: 90,
            fallback_models: Vec::new(),
            fallback_on_errors: true,
            memory_nudge_interval: 10,
            creation_nudge_interval: 10,
            context_engine: "compact".to_string(),
            context_lcm_db: None,
            context_lcm_tail_tokens: 12_000,
            context_lcm_auto_recall: true,
            context_lcm_auto_recall_limit: 3,
            context_lcm_auto_recall_max_chars: 4_000,
            context_lcm_rollups_inject: true,
            context_lcm_rollup_interval_minutes: 0,
            context_lcm_embedding_model: None,
            context_lcm_embedding_base_url: None,
            context_lcm_assertion_extraction: false,
            context_lcm_ignore_session_patterns: Vec::new(),
            context_lcm_readonly_sessions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutonomousSettings {
    pub interval_secs: u64,
    pub todo_path: PathBuf,
    pub status_path: PathBuf,
    pub test_command: String,
    pub git_remote: String,
    pub git_branch: String,
    pub commit_message: String,
    pub command_timeout_secs: u64,
    pub max_failures_per_state: usize,
}

impl Default for AutonomousSettings {
    fn default() -> Self {
        Self {
            interval_secs: 300,
            todo_path: PathBuf::from("TODO.md"),
            status_path: PathBuf::from("autonomous-status.toml"),
            test_command: "cargo test --workspace".to_string(),
            git_remote: "origin".to_string(),
            git_branch: "agent-dev".to_string(),
            commit_message: "Auto-commit by operant-rs".to_string(),
            command_timeout_secs: 900,
            max_failures_per_state: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingSettings {
    pub level: String,
    pub format: String,
    pub log_file: Option<String>,
    pub with_target: bool,
    pub with_thread_ids: bool,
    pub with_file: bool,
    pub with_line_number: bool,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
            log_file: None,
            with_target: false,
            with_thread_ids: false,
            with_file: false,
            with_line_number: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TuiSettings {
    pub theme: String,
    pub rich_output: bool,
    pub show_tool_calls: bool,
    pub show_iterations: bool,
    pub landing_title: String,
    pub prompt_placeholder: String,
    pub refresh_rate_ms: u64,
    pub compact_width: u16,
    pub medium_width: u16,
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            theme: "opencode".to_string(),
            rich_output: true,
            show_tool_calls: true,
            show_iterations: true,
            landing_title: "HERMES".to_string(),
            prompt_placeholder: "Ask anything... \"Fix a TODO in the codebase\"".to_string(),
            refresh_rate_ms: 80,
            compact_width: 96,
            medium_width: 120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    #[default]
    Http,
    Stdio,
    /// Streamable-HTTP transport (MCP spec 2025-06-18). Uses HTTP POST
    /// for client→server requests and SSE (Server-Sent Events) for
    /// server→client streaming. This is the modern recommended HTTP
    /// transport, superseding the old plain-POST HTTP + legacy SSE.
    /// (iter-137 — closes ponytail-audit gap B2.)
    StreamableHttp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportKind,
    pub url: Option<String>,
    pub auth_token: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub enabled: bool,
    /// Connect lazily: skip the eager autoload connect at agent startup.
    /// The server stays available on demand via `operant mcp` / the MCP
    /// tooling. Used for the injected agentmemory server so an operant
    /// invocation never spawns `npx @agentmemory/mcp` unless the user
    /// actually connects it (hermes lazy-MCP parity).
    pub deferred: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransportKind::Http,
            url: None,
            auth_token: None,
            command: None,
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            enabled: true,
            deferred: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    pub autoload: bool,
    /// Master switch for progressive tool disclosure of MCP tools (hermes
    /// `mcp.deferred_loading` parity). When `false`, MCP tool schemas are
    /// always loaded eagerly and `tools.tool_search.enabled` is forced to
    /// `"off"`. Defaults to `true` (defer behind the tool_search bridge
    /// when MCP tools are present).
    #[serde(default = "default_true")]
    pub deferred_loading: bool,
    /// Stdio MCP watchdog sweep interval in seconds (hermes
    /// `mcp_stdio_watchdog.py` parity). A background task periodically
    /// checks every connected stdio MCP server's child process and
    /// auto-reconnects one that has exited (crash), re-syncing its tools
    /// into the registry. `0` disables the watchdog. Default 30s.
    #[serde(default = "default_mcp_watchdog_interval")]
    pub watchdog_interval_secs: u64,
    pub servers: Vec<McpServerConfig>,
}

fn default_mcp_watchdog_interval() -> u64 {
    30
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            autoload: true,
            deferred_loading: true,
            watchdog_interval_secs: default_mcp_watchdog_interval(),
            servers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkillsSettings {
    pub root_dir: PathBuf,
    pub memory_dir: PathBuf,
    pub autoload: bool,
    pub template_name: String,
    pub template_description: String,
    /// Substitute `${OPERANT_SKILL_DIR}` / `${OPERANT_SESSION_ID}` in
    /// SKILL.md before injection (hermes `template_vars`, default true).
    #[serde(default = "default_skill_template_vars")]
    pub template_vars: bool,
    /// Expand `` !`cmd` `` inline-shell snippets in SKILL.md at load time
    /// (hermes `inline_shell`, default false — off unless opted in).
    #[serde(default)]
    pub inline_shell: bool,
    /// Timeout (seconds) for each inline-shell snippet.
    #[serde(default = "default_skill_inline_shell_timeout")]
    pub inline_shell_timeout: u64,
}

fn default_skill_template_vars() -> bool {
    true
}
fn default_skill_inline_shell_timeout() -> u64 {
    10
}

impl Default for SkillsSettings {
    fn default() -> Self {
        let root_dir = platform::operant_skills_dir();
        let memory_dir = root_dir.parent().unwrap_or(&root_dir).join("memory");
        Self {
            root_dir,
            memory_dir,
            autoload: true,
            template_name: "new-skill".to_string(),
            template_description: "Describe what this skill does.".to_string(),
            template_vars: true,
            inline_shell: false,
            inline_shell_timeout: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GatewaySettings {
    pub telegram_enabled: bool,
    pub telegram_token: Option<String>,
    pub telegram_api_base: String,
    pub discord_enabled: bool,
    pub discord_token: Option<String>,
    pub discord_api_base: String,
    pub slack_enabled: bool,
    pub slack_token: Option<String>,
    pub slack_api_base: String,
    pub whatsapp_enabled: bool,
    pub whatsapp_token: Option<String>,
    /// WhatsApp Cloud API phone number ID (Meta Business Manager).
    /// Required for outbound sends — without it the Graph API URL has no
    /// `phone_number_id` segment and returns 404. (R22)
    pub whatsapp_phone_number_id: Option<String>,
    pub email_enabled: bool,
    pub email_smtp_host: Option<String>,
    pub email_smtp_user: Option<String>,
    pub email_smtp_pass: Option<String>,
    pub webhooks_enabled: bool,
    pub webhooks_addr: Option<String>,
    /// Shared secret for HMAC-SHA256 verification of inbound webhook
    /// signatures (GitHub `x-hub-signature-256`, Stripe `Stripe-Signature`,
    /// Slack `x-slack-signature`, or the custom `x-webhook-signature` header).
    /// When set, unsigned or mismatched webhook requests are rejected.
    pub webhooks_secret: Option<String>,
    pub sms_twilio_enabled: bool,
    pub admins: Vec<String>,
    pub streaming_transport: String,
    /// HTTP/SOCKS5 proxy URL for Telegram API requests (e.g. "socks5://127.0.0.1:1080")
    pub telegram_proxy: Option<String>,
    /// Bot username for @mention detection in groups
    pub telegram_bot_username: Option<String>,
    /// Enable DM topic creation for private chats (Bot API 9.4+)
    pub telegram_dm_topics_enabled: bool,
    /// Cap on concurrent gateway sessions (hermes `max_concurrent_sessions`
    /// parity). When reached, new sessions get a refusal reply while existing
    /// holders keep their slots. `None` = unlimited.
    pub max_concurrent_sessions: Option<usize>,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            telegram_enabled: false,
            telegram_token: None,
            telegram_api_base: "https://api.telegram.org".to_string(),
            discord_enabled: false,
            discord_token: None,
            discord_api_base: "https://discord.com/api/v10".to_string(),
            slack_enabled: false,
            slack_token: None,
            slack_api_base: "https://slack.com/api".to_string(),
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            sms_twilio_enabled: false,
            admins: Vec::new(),
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            max_concurrent_sessions: None,
        }
    }
}

/// Plugin system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginSettings {
    /// Directories to scan for plugin manifests (`plugin.toml` / `plugin.yaml`).
    pub plugin_dirs: Vec<PathBuf>,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            plugin_dirs: vec![platform::operant_home().join("plugins")],
        }
    }
}

/// Checkpoint (filesystem snapshot) settings.
///
/// Checkpoints are opt-in: when `enabled`, the agent snapshots working
/// directories into an isolated shadow git store under `base_dir` before
/// mutating operations, and the `checkpoint` tool becomes functional. Hermes
/// parity: hermes gates checkpoints behind a `checkpoints` config flag and
/// stores snapshots in an isolated git store so no git state leaks into the
/// user's project repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointsSettings {
    /// Whether filesystem checkpoints are enabled.
    pub enabled: bool,
    /// Base directory for the checkpoint shadow store.
    /// Defaults to `~/.operant/checkpoints`.
    pub base_dir: Option<PathBuf>,
    /// Maximum snapshots kept per working directory.
    pub max_snapshots: usize,
}

impl Default for CheckpointsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            base_dir: None,
            max_snapshots: 20,
        }
    }
}

/// Terminal execution backends.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TerminalBackend {
    #[default]
    Local,
    Docker,
    Modal,
    Ssh,
    Daytona,
    VercelSandbox,
    Singularity,
}

impl std::fmt::Display for TerminalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Docker => write!(f, "docker"),
            Self::Modal => write!(f, "modal"),
            Self::Ssh => write!(f, "ssh"),
            Self::Daytona => write!(f, "daytona"),
            Self::VercelSandbox => write!(f, "vercel_sandbox"),
            Self::Singularity => write!(f, "singularity"),
        }
    }
}

impl std::str::FromStr for TerminalBackend {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "docker" => Ok(Self::Docker),
            "modal" => Ok(Self::Modal),
            "ssh" => Ok(Self::Ssh),
            "daytona" => Ok(Self::Daytona),
            "vercel_sandbox" => Ok(Self::VercelSandbox),
            "singularity" => Ok(Self::Singularity),
            _ => Err(format!("Unknown terminal backend: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolSettings {
    pub registry_timeout_secs: u64,
    pub event_channel_size: usize,
    pub browser_binary_path: Option<PathBuf>,
    pub web: WebToolSettings,
    pub http: HttpToolSettings,
    pub terminal: TerminalSettings,
    pub code_execution: CodeExecutionSettings,
    pub stt: SttSettings,
    pub disabled_tools: Vec<String>,
    pub disabled_toolsets: Vec<String>,
    /// Whether to enable AFT (Agent File Tools) IDE-grade coding tools.
    /// When true, operant registers 18 aft_* tools that communicate with
    /// an aft subprocess (auto-downloaded from GitHub releases when not
    /// installed). Defaults to true; the CLI verifies the bridge is live
    /// before hiding the native file/terminal tools, so a broken/missing
    /// aft always falls back to the built-in tools.
    #[serde(default = "default_true")]
    pub aft_enabled: bool,
    /// Whether to register the IGS-backed web tools (web_scrape,
    /// web_extract) and the `igs` browser provider. Requires the `igs`
    /// binary (see igs.rs IGS_INSTALL_HINT). Defaults to true.
    #[serde(default = "default_true")]
    pub igs_enabled: bool,
    /// Optional explicit path to the `igs` binary (default: PATH lookup).
    #[serde(default)]
    pub igs_binary_path: Option<PathBuf>,
    /// Optional explicit path to the Obscura browser binary used by the
    /// `obscura` browser provider. When unset, operant reuses the binary the
    /// IGS integration manages (`$IGS_CONFIG_DIR/bin/obscura` or
    /// `~/.config/igs-mcp/bin/obscura`), then falls back to its own copy at
    /// `~/.operant/bin/obscura`. Point this at the same binary IGS uses to
    /// guarantee a single shared Obscura across browser + web tools.
    #[serde(default)]
    pub obscura_binary_path: Option<PathBuf>,
    /// Whether the `obscura` browser provider runs in stealth mode: prefers
    /// the `-stealth` release build when downloading and passes `--stealth` to
    /// `obscura serve` (anti-detection: browser TLS fingerprinting, tracker
    /// blocking, `navigator.webdriver` masking). Defaults to true. Set to
    /// false only if your Obscura binary predates `--stealth` support.
    #[serde(default = "default_true")]
    pub obscura_stealth: bool,
    /// Timeout (seconds) for a single `igs` invocation (5..=600).
    #[serde(default = "default_igs_timeout")]
    pub igs_timeout_secs: u64,
    #[serde(default)]
    pub lifeos_enabled: bool,
    /// Progressive tool disclosure (hermes `tools.tool_search` parity).
    ///
    /// When active, MCP server tools (`mcp_*` names) are replaced in the
    /// model-visible tools array by three bridge tools — `tool_search`,
    /// `tool_describe`, `tool_call` — and surfaced on demand. Native
    /// builtin tools never defer; Tier 0 (no MCP tools) is a pure
    /// passthrough. See `tools/tool_search.rs` for the full design.
    #[serde(default)]
    pub tool_search: ToolSearchSettings,
}

fn default_true() -> bool {
    true
}

fn default_igs_timeout() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToolSearchSettings {
    /// `"auto"` | `"on"` | `"off"`. `"auto"` (default) activates the
    /// bridge only when deferrable (MCP) tools are present — a pure
    /// passthrough otherwise, so zero-behavior-change for native-only
    /// configs. `"on"` forces the bridge; `"off"` disables it even when
    /// MCP tools exist.
    #[serde(default = "default_tool_search_enabled")]
    pub enabled: String,
    /// Listing budget as a percentage of the model's context window that
    /// the embedded catalog listing may consume before disclosure degrades
    /// (full listing -> names-only -> bare bridge). Default 5.0.
    #[serde(default = "default_tool_search_threshold_pct")]
    pub threshold_pct: f64,
    /// `"auto"` | `"on"` | `"off"` — whether the `tool_search` bridge
    /// description embeds a skills-style catalog listing of every deferred
    /// tool (name + short description). `"auto"` = include when it fits
    /// the listing budget, else names-only, else none (bare bridge).
    #[serde(default = "default_tool_search_listing")]
    pub listing: String,
    /// Absolute cap on the embedded listing, regardless of context size.
    /// Effective budget = min(listing_max_tokens, threshold_pct% of
    /// context). Default 4000.
    #[serde(default = "default_tool_search_listing_max_tokens")]
    pub listing_max_tokens: usize,
    /// Default `limit` for `tool_search` when the model omits it. Default 10.
    #[serde(default = "default_tool_search_default_limit")]
    pub search_default_limit: usize,
    /// Hard cap on a single `tool_search` `limit`. Default 25.
    #[serde(default = "default_tool_search_max_limit")]
    pub max_search_limit: usize,
}

fn default_tool_search_enabled() -> String {
    "auto".to_string()
}

fn default_tool_search_threshold_pct() -> f64 {
    5.0
}

fn default_tool_search_listing() -> String {
    "auto".to_string()
}

fn default_tool_search_listing_max_tokens() -> usize {
    4000
}

fn default_tool_search_default_limit() -> usize {
    10
}

fn default_tool_search_max_limit() -> usize {
    25
}

impl Default for ToolSearchSettings {
    fn default() -> Self {
        Self {
            enabled: default_tool_search_enabled(),
            threshold_pct: default_tool_search_threshold_pct(),
            listing: default_tool_search_listing(),
            listing_max_tokens: default_tool_search_listing_max_tokens(),
            search_default_limit: default_tool_search_default_limit(),
            max_search_limit: default_tool_search_max_limit(),
        }
    }
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            registry_timeout_secs: 30,
            event_channel_size: 100,
            browser_binary_path: None,
            web: WebToolSettings::default(),
            http: HttpToolSettings::default(),
            terminal: TerminalSettings::default(),
            code_execution: CodeExecutionSettings::default(),
            stt: SttSettings::default(),
            disabled_tools: Vec::new(),
            disabled_toolsets: Vec::new(),
            aft_enabled: true,
            igs_enabled: true,
            igs_binary_path: None,
            obscura_binary_path: None,
            obscura_stealth: true,
            igs_timeout_secs: 60,
            lifeos_enabled: false,
            tool_search: ToolSearchSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SttSettings {
    pub groq_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub groq_model: String,
    pub openai_model: String,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            groq_api_key: None,
            openai_api_key: None,
            groq_model: "whisper-large-v3-turbo".to_string(),
            openai_model: "whisper-1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TtsSettings {
    /// TTS provider: kokoro|edge|elevenlabs|openai|minimax|mistral|gemini|xai|neutts|kittentts|piper
    pub provider: String,
    pub enabled: bool,
    /// Default voice for the selected provider (provider-specific identifier)
    pub voice: Option<String>,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            provider: "kokoro".to_string(),
            enabled: false,
            voice: None,
        }
    }
}

/// Memory provider configuration.
///
/// `provider` selects the long-term memory backend:
/// - `"agentmemory"` (default) — hybrid semantic memory via the agentmemory
///   server (BM25 + vector + graph). Operant auto-spawns
///   `npx @agentmemory/agentmemory` on :3111 when needed and registers its
///   MCP tools (memory_smart_search, memory_save, ...).
/// - `"builtin"` — file-backed MEMORY.md/USER.md in the operant home directory
/// - Any other string falls back to `"builtin"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemorySettings {
    /// Memory provider: agentmemory|builtin
    pub provider: String,
    /// Whether long-term memory is enabled at all
    pub enabled: bool,
    /// agentmemory server base URL (default: http://localhost:3111)
    pub agentmemory_url: Option<String>,
    /// Optional shared secret for the agentmemory REST API
    pub agentmemory_secret: Option<String>,
    /// Auto-spawn `npx @agentmemory/agentmemory` when the server is unreachable
    pub agentmemory_auto_spawn: Option<bool>,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            provider: "agentmemory".to_string(),
            enabled: true,
            agentmemory_url: None,
            agentmemory_secret: None,
            agentmemory_auto_spawn: Some(true),
        }
    }
}

/// Browser provider configuration.
///
/// `provider` selects the browser backend:
/// - `"obscura"` (default) — local Obscura binary shared with IGS; CDP-driven,
///   stealth by default (reliable multi-step automation)
/// - `"igs"` — IGS CLI (`igs web scrape` / stateless browser CLI)
/// - `"lightpanda"` — local binary, auto-downloaded from GitHub Releases
/// - `"camofox"` — local anti-detection browser REST API (`CAMOFOX_URL`)
/// - `"browserbase"` — Browserbase cloud (`BROWSERBASE_API_KEY` + `BROWSERBASE_PROJECT_ID`)
/// - `"browser-use"` — Browser Use cloud agent (`BROWSER_USE_API_KEY`)
/// - `"firecrawl"` — Firecrawl scrape API (`FIRECRAWL_API_KEY`)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserSettings {
    /// Browser provider name
    pub provider: String,
}

impl Default for BrowserSettings {
    fn default() -> Self {
        Self {
            provider: "obscura".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct VisionSettings {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct CredentialPoolSettings {
    pub strategy: Option<String>,
    pub enabled: bool,
    pub strategies: HashMap<String, String>,
}

/// Configuration for an auxiliary model assigned to a specific task slot.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuxiliaryModelConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

/// Mixture-of-Agents (MoA) configuration (`[moa]` section, hermes
/// `hermes_cli/moa_config.py` + `agent/moa_loop.py` parity).
///
/// When `enabled`, a MoA turn fans out `references` (advisory models) over
/// the flattened conversation, then an `aggregator` model synthesizes their
/// advice into guidance that is injected into the acting agent's context
/// before it answers. Off by default — MoA costs N+1 LLM calls per turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct MoaSettings {
    /// Master switch. When false (default), MoA turns are a no-op.
    pub enabled: bool,
    /// Reference (advisor) models. Each is called with the flattened
    /// conversation plus an advisory system prompt; their outputs feed the
    /// aggregator. Empty (default) → MoA is inert even when enabled.
    pub references: Vec<AuxiliaryModelConfig>,
    /// Aggregator model that synthesizes the reference advice into concise
    /// guidance for the acting agent. When unset, the main agent model is
    /// used with the main provider client.
    pub aggregator: Option<AuxiliaryModelConfig>,
    /// Optional per-reference output cap (tokens). None → the model's own
    /// maximum. The aggregator synthesis is NEVER capped (hermes parity).
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Optional temperature for reference and aggregator calls. None → the
    /// provider default applies.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Per-call timeout in seconds for reference/aggregator calls
    /// (default 60).
    #[serde(default = "default_moa_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_moa_timeout_secs() -> u64 {
    60
}

/// Auxiliary model routing for specialized task slots.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuxiliaryModels {
    pub vision: Option<AuxiliaryModelConfig>,
    pub compression: Option<AuxiliaryModelConfig>,
    /// LLM post-processing slot for `web_extract`. Currently **inert**: the
    /// IGS-backed `web_extract` tool returns raw markdown directly (no LLM
    /// summarization step), so this slot only takes effect once an
    /// LLM-backed extractor is wired to it.
    pub web_extract: Option<AuxiliaryModelConfig>,
    pub image_gen: Option<AuxiliaryModelConfig>,
    pub embeddings: Option<AuxiliaryModelConfig>,
    pub search: Option<AuxiliaryModelConfig>,
    pub memory: Option<AuxiliaryModelConfig>,
    pub code_execution: Option<AuxiliaryModelConfig>,
    pub reasoning: Option<AuxiliaryModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebToolSettings {
    pub search_url: String,
    pub search_timeout_secs: u64,
    pub fetch_timeout_secs: u64,
    pub user_agent: String,
    pub default_results: usize,
    pub max_results: usize,
    /// Preferred web search provider: igs (default — requires the `igs`
    /// binary, falls back to duckduckgo) | tavily | exa | searxng | duckduckgo
    pub preferred_provider: String,
    pub tavily_api_key: Option<String>,
    pub exa_api_key: Option<String>,
    pub searxng_base_url: Option<String>,
}

impl Default for WebToolSettings {
    fn default() -> Self {
        Self {
            search_url: "https://lite.duckduckgo.com/lite/?q={query}".to_string(),
            search_timeout_secs: 15,
            fetch_timeout_secs: 30,
            user_agent: "Mozilla/5.0 (compatible; OperantAgent/0.1)".to_string(),
            default_results: 10,
            max_results: 20,
            preferred_provider: "igs".to_string(),
            tavily_api_key: None,
            exa_api_key: None,
            searxng_base_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpToolSettings {
    pub timeout_secs: u64,
}

impl Default for HttpToolSettings {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalSettings {
    pub max_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub docker: DockerTerminalSettings,
    pub ssh: SshTerminalSettings,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            max_timeout_secs: 300,
            max_output_bytes: 1_000_000,
            docker: DockerTerminalSettings::default(),
            ssh: SshTerminalSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerTerminalSettings {
    pub image: Option<String>,
    pub volumes: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub cpu: f64,
    pub memory_mb: u64,
}

impl Default for DockerTerminalSettings {
    fn default() -> Self {
        Self {
            image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            volumes: Vec::new(),
            env: std::collections::HashMap::new(),
            cpu: 1.0,
            memory_mb: 5120,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshTerminalSettings {
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: u16,
    pub key_path: Option<String>,
}

impl Default for SshTerminalSettings {
    fn default() -> Self {
        Self {
            host: None,
            user: None,
            port: 22,
            key_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodeExecutionSettings {
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

impl Default for CodeExecutionSettings {
    fn default() -> Self {
        Self {
            default_timeout_secs: 60,
            max_timeout_secs: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub source: Option<PathBuf>,
}

pub fn install_runtime_config(config: AppConfig) {
    let store = RUNTIME_CONFIG.get_or_init(|| RwLock::new(AppConfig::default()));
    if let Ok(mut current) = store.write() {
        *current = config;
    }
}

pub fn runtime_config() -> AppConfig {
    let store = RUNTIME_CONFIG.get_or_init(|| RwLock::new(AppConfig::default()));
    store
        .read()
        .map(|config| config.clone())
        .unwrap_or_else(|_| AppConfig::default())
}

pub fn load_app_config(explicit: Option<&Path>) -> Result<LoadedConfig> {
    let (mut config, source) = if let Some(path) = explicit {
        if !path.exists() {
            return Err(Error::Config(format!(
                "Config file '{}' was not found. Pass a valid --config path or create operant.toml.",
                path.display()
            )));
        }
        (parse_config_file(path)?, Some(path.to_path_buf()))
    } else {
        let mut found = None;
        for path in default_config_paths() {
            if path.exists() {
                found = Some(path);
                break;
            }
        }
        match found {
            Some(path) => (parse_config_file(&path)?, Some(path)),
            None => {
                // Nothing exists yet: fall back to defaults, but when
                // OPERANT_CONFIG_DIR is set, still claim that isolated path
                // as the config source so the `config_manage` tool and any
                // persistence (e.g. `operant channel add`) target the
                // isolated config home instead of the real `~/.operant`
                // config. A later explicit persist creates the file there.
                let source = operant_config_dir_override().map(|dir| dir.join("operant.toml"));
                (AppConfig::default(), source)
            }
        }
    };

    // Native-MCP registration: when the agentmemory memory provider is
    // active, ensure an `agentmemory` stdio MCP server exists in the config
    // so EVERY agent-construction path (CLI registry, runtime-agent
    // connect_all, gateway orchestrator) exposes the full 53-tool memory
    // surface. Users who configured their own server (or deliberately
    // disabled it) keep their entry untouched.
    ensure_default_mcp_servers(&mut config);

    Ok(LoadedConfig { config, source })
}

/// Inject the agentmemory MCP server into `config.mcp.servers` when the
/// agentmemory memory provider is active and no `agentmemory` server is
/// already configured. This makes the memory tools a native, config-driven
/// MCP server instead of a CLI-only special case (hermes-agent plugin
/// parity: agentmemory registers itself as an MCP server the same way the
/// hermes plugin's docs configure `@agentmemory/mcp`).
pub fn ensure_default_mcp_servers(config: &mut AppConfig) {
    if !(config.memory.enabled && config.memory.provider == "agentmemory") {
        return;
    }
    if config.mcp.servers.iter().any(|s| s.name == "agentmemory") {
        return;
    }
    let server_url = config
        .memory
        .agentmemory_url
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "http://localhost:3111".to_string());
    let mut env = HashMap::new();
    env.insert("AGENTMEMORY_URL".to_string(), server_url);
    // Propagate the optional shared secret so the MCP tools authenticate
    // exactly like the REST hooks (parity: the plugin's MCP server reads
    // AGENTMEMORY_SECRET from its environment).
    if let Some(secret) = config
        .memory
        .agentmemory_secret
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        env.insert("AGENTMEMORY_SECRET".to_string(), secret);
    }
    config.mcp.servers.push(McpServerConfig {
        name: "agentmemory".to_string(),
        transport: McpTransportKind::Stdio,
        url: None,
        auth_token: None,
        command: Some("npx".to_string()),
        args: vec!["-y".to_string(), "@agentmemory/mcp".to_string()],
        env,
        enabled: true,
        // Deferred (lazy): the provider's own memory tools are registered
        // directly, so we never want to spawn npx on every agent startup.
        deferred: true,
    });
}

/// The `OPERANT_CONFIG_DIR` override (if set and non-empty), tilde-expanded.
///
/// Mirrors the schema layer's `default_config_dir()` so EVERY load path —
/// CLI `run`/`gateway`/`cron`, the `config_manage` tool, and the gateway
/// service — resolves to the same isolated config the user asked for
/// instead of silently falling back to `~/.operant/operant.toml`.
pub(crate) fn operant_config_dir_override() -> Option<PathBuf> {
    let custom = std::env::var("OPERANT_CONFIG_DIR").ok()?;
    let custom = custom.trim();
    if custom.is_empty() {
        return None;
    }
    // Expand a leading `~` (bare, `~/`, or `~\`) to $HOME; leave
    // `~otheruser/...` untouched. Mirrors the schema layer's tilde handling
    // without pulling in shellexpand here.
    if let Some(rest) = custom.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        && let Ok(home) = std::env::var("HOME")
    {
        return Some(PathBuf::from(home).join(rest.trim_start_matches(['/', '\\'])));
    }
    Some(PathBuf::from(custom))
}

pub fn default_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // OPERANT_CONFIG_DIR is the highest-precedence override after an
    // explicit `--config`: `<dir>/operant.toml`. Checked BEFORE the cwd
    // candidates so a stray local `operant.toml` can never shadow an
    // explicitly requested isolated config home (isolation guarantee).
    if let Some(dir) = operant_config_dir_override() {
        paths.push(dir.join("operant.toml"));
    }

    paths.push(PathBuf::from("operant.toml"));
    paths.push(PathBuf::from(".operant.toml"));

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("operant").join("config.toml"));
    }

    if let Some(home_dir) = dirs::home_dir() {
        paths.push(home_dir.join(".operant").join("operant.toml"));
    }

    paths
}

pub fn parse_config_file(path: &Path) -> Result<AppConfig> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        Error::Config(format!(
            "Failed to read config file '{}': {}",
            path.display(),
            error
        ))
    })?;

    parse_config_str(&raw, path)
}

pub fn parse_config_str(raw: &str, source: &Path) -> Result<AppConfig> {
    toml::from_str(raw).map_err(|error| {
        let message = match error.span() {
            Some(span) => format!(
                "Invalid TOML in '{}': {} (bytes {}..{})",
                source.display(),
                error,
                span.start,
                span.end
            ),
            None => format!("Invalid TOML in '{}': {}", source.display(), error),
        };

        Error::Config(message)
    })
}

impl AppConfig {
    pub fn apply_env_overrides(&mut self) -> Result<()> {
        apply_string_option_override("OPENAI_API_KEY", &mut self.client.api_key)?;
        apply_string_value_override("OPENAI_BASE_URL", &mut self.client.base_url);
        apply_string_value_override("HERMES_MODEL", &mut self.agent.model);
        apply_usize_override("HERMES_MAX_ITERATIONS", &mut self.agent.max_iterations)?;
        apply_u64_override("HERMES_TOOL_TIMEOUT", &mut self.agent.tool_timeout_secs)?;
        apply_u64_override(
            "HERMES_REQUEST_TIMEOUT",
            &mut self.agent.request_timeout_secs,
        )?;
        apply_usize_override("HERMES_CONTEXT_WINDOW", &mut self.agent.context_window)?;
        apply_usize_override(
            "HERMES_MAX_HEALING_ATTEMPTS",
            &mut self.agent.max_healing_attempts,
        )?;
        apply_bool_override("HERMES_STREAM", &mut self.agent.stream)?;
        apply_string_option_override("HERMES_SYSTEM_PROMPT", &mut self.agent.system_prompt)?;
        apply_u64_override(
            "HERMES_AUTONOMOUS_INTERVAL",
            &mut self.autonomous.interval_secs,
        )?;
        apply_path_override("HERMES_AUTONOMOUS_TODO", &mut self.autonomous.todo_path)?;
        apply_path_override("HERMES_AUTONOMOUS_STATUS", &mut self.autonomous.status_path)?;
        apply_string_value_override(
            "HERMES_AUTONOMOUS_TEST_COMMAND",
            &mut self.autonomous.test_command,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_GIT_REMOTE",
            &mut self.autonomous.git_remote,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_GIT_BRANCH",
            &mut self.autonomous.git_branch,
        );
        apply_string_value_override(
            "HERMES_AUTONOMOUS_COMMIT_MESSAGE",
            &mut self.autonomous.commit_message,
        );
        apply_u64_override(
            "HERMES_AUTONOMOUS_COMMAND_TIMEOUT",
            &mut self.autonomous.command_timeout_secs,
        )?;
        apply_usize_override(
            "HERMES_AUTONOMOUS_MAX_FAILURES",
            &mut self.autonomous.max_failures_per_state,
        )?;
        apply_string_value_override("HERMES_LOG_LEVEL", &mut self.logging.level);
        apply_path_override("HERMES_SKILLS_DIR", &mut self.skills.root_dir)?;

        // Gateway env var overrides
        apply_string_option_override("TELEGRAM_BOT_TOKEN", &mut self.gateway.telegram_token)?;
        apply_bool_override(
            "HERMES_TELEGRAM_ENABLED",
            &mut self.gateway.telegram_enabled,
        )?;
        apply_string_option_override("DISCORD_BOT_TOKEN", &mut self.gateway.discord_token)?;
        apply_bool_override("HERMES_DISCORD_ENABLED", &mut self.gateway.discord_enabled)?;
        apply_string_option_override("SLACK_BOT_TOKEN", &mut self.gateway.slack_token)?;
        apply_bool_override("HERMES_SLACK_ENABLED", &mut self.gateway.slack_enabled)?;
        apply_string_option_override(
            "OPERANT_WHATSAPP_PHONE_NUMBER_ID",
            &mut self.gateway.whatsapp_phone_number_id,
        )?;

        Ok(())
    }
}

fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn apply_string_option_override(name: &str, target: &mut Option<String>) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = Some(value);
    }
    Ok(())
}

fn apply_string_value_override(name: &str, target: &mut String) {
    if let Some(value) = read_env(name) {
        *target = value;
    }
}

fn apply_u64_override(name: &str, target: &mut u64) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = value.parse().map_err(|_| {
            Error::Config(format!(
                "Environment variable '{}' must be an unsigned integer.",
                name
            ))
        })?;
    }
    Ok(())
}

fn apply_usize_override(name: &str, target: &mut usize) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = value.parse().map_err(|_| {
            Error::Config(format!(
                "Environment variable '{}' must be an unsigned integer.",
                name
            ))
        })?;
    }
    Ok(())
}

fn apply_bool_override(name: &str, target: &mut bool) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => {
                return Err(Error::Config(format!(
                    "Environment variable '{}' must be a boolean.",
                    name
                )));
            }
        };
    }
    Ok(())
}

fn apply_path_override(name: &str, target: &mut PathBuf) -> Result<()> {
    if let Some(value) = read_env(name) {
        *target = PathBuf::from(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn gateway_toml_roundtrips_whatsapp_phone_number_id() {
        // R22: proves the headline fix path — a user setting
        // `whatsapp_phone_number_id` under [gateway] in operant.toml actually
        // reaches GatewaySettings (pure serde via parse_config_str).
        let raw = r#"
[gateway]
whatsapp_enabled = true
whatsapp_token = "wa-token"
whatsapp_phone_number_id = "123456789"
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("test.toml")).expect("valid TOML");
        assert!(parsed.gateway.whatsapp_enabled);
        assert_eq!(parsed.gateway.whatsapp_token.as_deref(), Some("wa-token"));
        assert_eq!(
            parsed.gateway.whatsapp_phone_number_id.as_deref(),
            Some("123456789")
        );

        // And an old TOML without the key still parses (backward compat).
        let old = r#"
[gateway]
whatsapp_enabled = true
whatsapp_token = "wa-token"
"#;
        let parsed = parse_config_str(old, std::path::Path::new("old.toml")).expect("valid TOML");
        assert_eq!(parsed.gateway.whatsapp_phone_number_id, None);
    }

    #[test]
    fn tools_toml_parses_obscura_binary_path() {
        // The shared-Obscura knob: `tools.obscura_binary_path` must reach
        // ToolSettings so ObscuraProvider can reuse the IGS-managed binary.
        let raw = r#"
[tools]
igs_enabled = true
obscura_binary_path = "/home/dev/.config/igs-mcp/bin/obscura"
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("test.toml")).expect("valid TOML");
        assert!(parsed.tools.igs_enabled);
        assert_eq!(
            parsed.tools.obscura_binary_path.as_deref(),
            Some(std::path::Path::new(
                "/home/dev/.config/igs-mcp/bin/obscura"
            ))
        );

        // Old TOML without the key still parses (backward compat → None).
        let old = r#"
[tools]
igs_enabled = true
"#;
        let parsed = parse_config_str(old, std::path::Path::new("old.toml")).expect("valid TOML");
        assert_eq!(parsed.tools.obscura_binary_path, None);
    }

    #[test]
    fn tools_toml_parses_obscura_stealth_default_true() {
        // Stealth is on by default; explicitly disabling must round-trip.
        let raw = r#"
[tools]
obscura_stealth = false
"#;
        let parsed = parse_config_str(raw, std::path::Path::new("test.toml")).expect("valid TOML");
        assert!(!parsed.tools.obscura_stealth);

        let default = AppConfig::default();
        assert!(default.tools.obscura_stealth);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "operant_config_test_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn with_current_dir<T>(path: &Path, f: impl FnOnce() -> T) -> T {
        let current = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        let result = f();
        std::env::set_current_dir(current).unwrap();
        result
    }

    fn set_env(name: &str, value: &str) -> Option<OsString> {
        let previous = std::env::var_os(name);
        // SAFETY: test-only env mutation under exclusive lock
        unsafe { std::env::set_var(name, value) };
        previous
    }

    fn restore_env(name: &str, previous: Option<OsString>) {
        if let Some(value) = previous {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::set_var(name, value) };
        } else {
            // SAFETY: test-only env mutation under exclusive lock
            unsafe { std::env::remove_var(name) };
        }
    }

    #[test]
    fn example_toml_parses() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("operant.example.toml");
        let raw = std::fs::read_to_string(&root).unwrap();
        let config = parse_config_str(&raw, &root).unwrap();
        assert_eq!(config.agent.model, "gpt-4");
        assert!(config.tui.rich_output);
        assert_eq!(config.autonomous.git_branch, "agent-dev");
        assert_eq!(
            config.autonomous.status_path,
            PathBuf::from("autonomous-status.toml")
        );
    }

    #[test]
    fn default_path_discovery_prefers_local_operant_toml() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("default_path");
        std::fs::write(
            dir.join("operant.toml"),
            "[agent]\nmodel = \"gpt-4.1-mini\"\n",
        )
        .unwrap();

        let loaded = with_current_dir(&dir, || load_app_config(None)).unwrap();
        assert_eq!(loaded.source.unwrap().file_name().unwrap(), "operant.toml");
        assert_eq!(loaded.config.agent.model, "gpt-4.1-mini");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn explicit_path_overrides_defaults() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("explicit_path");
        let explicit = dir.join("custom.toml");
        std::fs::write(dir.join("operant.toml"), "[agent]\nmodel = \"wrong\"\n").unwrap();
        std::fs::write(&explicit, "[agent]\nmodel = \"right\"\n").unwrap();

        let loaded = with_current_dir(&dir, || load_app_config(Some(&explicit))).unwrap();
        assert_eq!(loaded.config.agent.model, "right");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_toml_returns_field_aware_error() {
        let path = PathBuf::from("broken.toml");
        let error = parse_config_str("[agent]\nmax_iterations = \"many\"\n", &path).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("Invalid TOML"));
        assert!(text.contains("expected"));
    }

    #[test]
    fn ensure_default_mcp_servers_injects_agentmemory_when_provider_active() {
        // Default config has provider=agentmemory → the agentmemory stdio
        // MCP server is injected natively so all agent paths expose the
        // 53-tool memory surface.
        let mut config = AppConfig::default();
        assert!(config.memory.enabled);
        assert_eq!(config.memory.provider, "agentmemory");
        ensure_default_mcp_servers(&mut config);

        let server = config
            .mcp
            .servers
            .iter()
            .find(|s| s.name == "agentmemory")
            .expect("agentmemory server should be injected");
        assert!(server.enabled);
        assert_eq!(server.transport, McpTransportKind::Stdio);
        assert_eq!(server.command.as_deref(), Some("npx"));
        assert!(server.args.iter().any(|a| a == "@agentmemory/mcp"));
        // Deferred (lazy): never spawn npx on agent startup — the provider's
        // own memory tools are registered directly instead.
        assert!(server.deferred);
        assert_eq!(
            server.env.get("AGENTMEMORY_URL").map(String::as_str),
            Some("http://localhost:3111")
        );
    }

    #[test]
    fn ensure_default_mcp_servers_respects_custom_url_and_existing_server() {
        // Custom agentmemory_url + secret flow into the injected server env.
        let mut config = AppConfig::default();
        config.memory.agentmemory_url = Some("http://127.0.0.1:9999".to_string());
        config.memory.agentmemory_secret = Some("s3cret".to_string());
        ensure_default_mcp_servers(&mut config);
        let server = config
            .mcp
            .servers
            .iter()
            .find(|s| s.name == "agentmemory")
            .expect("agentmemory server should be injected");
        assert_eq!(
            server.env.get("AGENTMEMORY_URL").map(String::as_str),
            Some("http://127.0.0.1:9999")
        );
        assert_eq!(
            server.env.get("AGENTMEMORY_SECRET").map(String::as_str),
            Some("s3cret"),
            "configured secret must reach the injected MCP server env"
        );

        // A user-configured agentmemory server is never duplicated, even if
        // they deliberately disabled it.
        let mut config = AppConfig::default();
        config.mcp.servers.push(McpServerConfig {
            name: "agentmemory".to_string(),
            enabled: false,
            ..Default::default()
        });
        ensure_default_mcp_servers(&mut config);
        assert_eq!(
            config
                .mcp
                .servers
                .iter()
                .filter(|s| s.name == "agentmemory")
                .count(),
            1,
            "existing agentmemory server must not be duplicated"
        );
    }

    #[test]
    fn ensure_default_mcp_servers_noop_for_builtin_provider() {
        let mut config = AppConfig::default();
        config.memory.provider = "builtin".to_string();
        ensure_default_mcp_servers(&mut config);
        assert!(config.mcp.servers.is_empty());
    }

    #[test]
    fn env_overrides_apply_after_file_values() {
        let _guard = env_lock().lock().unwrap();
        let previous_model = set_env("HERMES_MODEL", "gpt-4.1");
        let previous_stream = set_env("HERMES_STREAM", "false");
        let previous_interval = set_env("HERMES_AUTONOMOUS_INTERVAL", "120");
        let previous_status = set_env("HERMES_AUTONOMOUS_STATUS", "runtime/autonomous-status.toml");

        let mut config = parse_config_str(
            "[agent]\nmodel = \"gpt-4o-mini\"\nstream = true\n",
            Path::new("env.toml"),
        )
        .unwrap();
        config.apply_env_overrides().unwrap();

        assert_eq!(config.agent.model, "gpt-4.1");
        assert!(!config.agent.stream);
        assert_eq!(config.autonomous.interval_secs, 120);
        assert_eq!(
            config.autonomous.status_path,
            PathBuf::from("runtime/autonomous-status.toml")
        );

        restore_env("HERMES_MODEL", previous_model);
        restore_env("HERMES_STREAM", previous_stream);
        restore_env("HERMES_AUTONOMOUS_INTERVAL", previous_interval);
        restore_env("HERMES_AUTONOMOUS_STATUS", previous_status);
    }

    #[test]
    fn providers_section_roundtrips_into_app_config() {
        // Hermes `fallback_providers` parity: the run path must be able to
        // read `[providers]` profiles + the ordered fallback chain from
        // operant.toml (this was previously only loaded by the channels/ACP
        // config path, never by AppConfig).
        let raw = r#"
[client]
base_url = "https://zen.example/v1"
api_key = "sk-test"

[agent]
model = "laguna-s-2.1-free"
fallback_models = ["deepseek-v4-flash-free"]

[providers.models.opencode-zen]
base_url = "https://zen.example/v1"
model = "deepseek-v4-flash-free"

[[providers.fallback_chain]]
provider = "opencode-zen"
model = "deepseek-v4-flash-free"
"#;
        let parsed =
            parse_config_str(raw, std::path::Path::new("providers.toml")).expect("valid TOML");

        // Profiles load.
        assert!(parsed.providers.models.contains_key("opencode-zen"));
        assert_eq!(
            parsed.providers.models["opencode-zen"].model.as_deref(),
            Some("deepseek-v4-flash-free")
        );
        // Ordered chain loads in order.
        assert_eq!(parsed.providers.fallback_chain.len(), 1);
        assert_eq!(parsed.providers.fallback_chain[0].provider, "opencode-zen");
        assert_eq!(
            parsed.providers.fallback_chain[0].model,
            "deepseek-v4-flash-free"
        );
        // Same-provider model fallback loads.
        assert_eq!(parsed.agent.fallback_models, vec!["deepseek-v4-flash-free"]);
        // Absent section defaults to empty — never blocks boot.
        let minimal = parse_config_str("[agent]\nmodel = \"x\"\n", Path::new("min.toml")).unwrap();
        assert!(minimal.providers.models.is_empty());
        assert!(minimal.providers.fallback_chain.is_empty());
    }
}

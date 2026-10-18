//! CLI configuration layer for Operant-RS.
//!
//! Bridges the Python dict-based YAML config (config.py, ~5,141 LOC) to Rust types.
//! Provides a dual-file system (config.yaml + .env), deep merge, env expansion,
//! migration system, validation, and config diff.
//!
//! Loading order (Python's deep-merge pattern):
//! 1. Default config values in code
//! 2. config.yaml (base config) from HERMES_HOME/config.yaml or HERMES_CONFIG env
//! 3. config.local.yaml (local overrides, gitignored)
//! 4. .env file from HERMES_HOME/.env or parent directory
//! 5. Environment variables (HERMES_* prefix)

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

// =============================================================================
// Top-Level CLI Config
// =============================================================================

/// CLI-specific configuration that extends the core `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    // Config file paths
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub local_config_file: PathBuf,
    pub env_file: PathBuf,

    // Config version for migration tracking
    pub config_version: Option<String>,

    // Top-level config sections — mirror Python's DEFAULT_CONFIG structure
    pub operant: OperantConfig,
    pub api: ApiConfig,
    pub agent: AgentConfigV2,
    pub terminal: TerminalConfig,
    pub web: WebConfigV2,
    pub browser: BrowserConfig,
    pub checkpoints: CheckpointsConfig,
    pub compression: CompressionConfig,
    pub prompt_caching: PromptCachingConfig,
    pub bedrock: BedrockConfig,
    pub auxiliary: AuxiliaryConfig,
    pub display: DisplayConfig,
    pub dashboard: DashboardConfig,
    pub tts: TtsConfig,
    pub stt: SttConfigV2,
    pub memory: MemoryConfigV2,
    pub delegation: DelegationConfig,
    pub goals: GoalsConfig,
    pub skills: SkillsConfigV2,
    pub curator: CuratorConfig,
    pub approvals: ApprovalsConfig,
    pub security: SecurityConfig,
    pub code_execution: CodeExecutionConfigV2,
    #[serde(alias = "prime_kernel")]
    pub kernel: KernelConfigV2,
    pub logging: LoggingConfigV2,
    pub model_catalog: ModelCatalogConfig,
    pub sessions: SessionsConfig,
    pub tools: ToolsConfigV2,
    pub gateways: GatewaysConfig,
    pub environments: EnvironmentsConfig,
    pub integrations: IntegrationsConfig,
    pub plugins: PluginsConfig,
    pub context: ContextConfigV2,
    pub hooks: HooksConfig,
    pub onboarding: OnboardingConfig,
}

impl Default for CliConfig {
    fn default() -> Self {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".operant"))
            .unwrap_or_else(|| PathBuf::from(".operant"));

        // Respect HERMES_HOME env var if set
        let config_dir = std::env::var("HERMES_HOME")
            .map(PathBuf::from)
            .unwrap_or(config_dir);

        Self {
            config_dir: config_dir.clone(),
            config_file: config_dir.join("config.yaml"),
            local_config_file: config_dir.join("config.local.yaml"),
            env_file: config_dir.join(".env"),
            config_version: Some("1.0.0".to_string()),
            operant: OperantConfig::default(),
            api: ApiConfig::default(),
            agent: AgentConfigV2::default(),
            terminal: TerminalConfig::default(),
            web: WebConfigV2::default(),
            browser: BrowserConfig::default(),
            checkpoints: CheckpointsConfig::default(),
            compression: CompressionConfig::default(),
            prompt_caching: PromptCachingConfig::default(),
            bedrock: BedrockConfig::default(),
            auxiliary: AuxiliaryConfig::default(),
            display: DisplayConfig::default(),
            dashboard: DashboardConfig::default(),
            tts: TtsConfig::default(),
            stt: SttConfigV2::default(),
            memory: MemoryConfigV2::default(),
            delegation: DelegationConfig::default(),
            goals: GoalsConfig::default(),
            skills: SkillsConfigV2::default(),
            curator: CuratorConfig::default(),
            approvals: ApprovalsConfig::default(),
            security: SecurityConfig::default(),
            code_execution: CodeExecutionConfigV2::default(),
            kernel: KernelConfigV2::default(),
            logging: LoggingConfigV2::default(),
            model_catalog: ModelCatalogConfig::default(),
            sessions: SessionsConfig::default(),
            tools: ToolsConfigV2::default(),
            gateways: GatewaysConfig::default(),
            environments: EnvironmentsConfig::default(),
            integrations: IntegrationsConfig::default(),
            plugins: PluginsConfig::default(),
            context: ContextConfigV2::default(),
            hooks: HooksConfig::default(),
            onboarding: OnboardingConfig::default(),
        }
    }
}

// =============================================================================
// Config Section Structs — mirroring Python DEFAULT_CONFIG
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OperantConfig {
    pub name: Option<String>,
    pub workspace: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub skills_dir: Option<PathBuf>,
    pub plugins_dir: Option<PathBuf>,
}

impl Default for OperantConfig {
    fn default() -> Self {
        Self {
            name: Some("operant-rs".to_string()),
            workspace: None,
            data_dir: None,
            locale: Some("en".to_string()),
            timezone: Some(String::new()),
            skills_dir: None,
            plugins_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiConfig {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_version: Option<String>,
    pub models: Vec<ModelConfig>,
    pub default_model: Option<String>,
    pub max_retries: Option<u32>,
    pub timeout_seconds: Option<u64>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub fallback_providers: Vec<String>,
    pub credential_pool_strategies: HashMap<String, String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            provider: None,
            base_url: None,
            api_key: None,
            api_version: None,
            models: Vec::new(),
            default_model: None,
            max_retries: Some(3),
            timeout_seconds: Some(120),
            max_tokens: Some(128_000),
            temperature: Some(0.7),
            fallback_providers: Vec::new(),
            credential_pool_strategies: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub id: String,
    pub name: Option<String>,
    pub provider: Option<String>,
    pub max_tokens: Option<u32>,
    pub context_length: Option<u32>,
    pub supports_streaming: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_tools: Option<bool>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: None,
            provider: None,
            max_tokens: None,
            context_length: None,
            supports_streaming: Some(true),
            supports_vision: Some(false),
            supports_tools: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfigV2 {
    pub max_turns: Option<u32>,
    pub api_max_retries: Option<u32>,
    pub service_tier: Option<String>,
    pub disabled_toolsets: Vec<String>,
}

impl Default for AgentConfigV2 {
    fn default() -> Self {
        Self {
            max_turns: Some(90),
            api_max_retries: Some(3),
            service_tier: Some(String::new()),
            disabled_toolsets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub backend: Option<String>,
    pub cwd: Option<String>,
    pub timeout: Option<u64>,
    pub env_passthrough: Vec<String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            backend: Some("local".to_string()),
            cwd: Some(".".to_string()),
            timeout: Some(180),
            env_passthrough: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfigV2 {
    pub backend: Option<String>,
}

impl Default for WebConfigV2 {
    fn default() -> Self {
        Self {
            backend: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    pub command_timeout: Option<u64>,
    pub engine: Option<String>,
    pub cdp_url: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            command_timeout: Some(30),
            engine: Some("auto".to_string()),
            cdp_url: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointsConfig {
    pub enabled: Option<bool>,
    pub max_snapshots: Option<u32>,
    pub retention_days: Option<u32>,
}

impl Default for CheckpointsConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            max_snapshots: Some(20),
            retention_days: Some(7),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    pub enabled: Option<bool>,
    pub threshold: Option<f64>,
    pub protect_last_n: Option<u32>,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            threshold: Some(0.50),
            protect_last_n: Some(20),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptCachingConfig {
    pub cache_ttl: Option<String>,
}

impl Default for PromptCachingConfig {
    fn default() -> Self {
        Self {
            cache_ttl: Some("5m".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockConfig {
    pub region: Option<String>,
    pub discovery: BedrockDiscoveryConfig,
    pub guardrail: BedrockGuardrailConfig,
}

impl Default for BedrockConfig {
    fn default() -> Self {
        Self {
            region: Some(String::new()),
            discovery: BedrockDiscoveryConfig::default(),
            guardrail: BedrockGuardrailConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockDiscoveryConfig {
    pub enabled: Option<bool>,
    pub provider_filter: Vec<String>,
}

impl Default for BedrockDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            provider_filter: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockGuardrailConfig {
    pub trace: Option<String>,
}

impl Default for BedrockGuardrailConfig {
    fn default() -> Self {
        Self {
            trace: Some("disabled".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxTaskConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub timeout: Option<u64>,
    pub extra_body: HashMap<String, Value>,
    pub max_concurrency: Option<u32>,
}

impl Default for AuxTaskConfig {
    fn default() -> Self {
        Self {
            provider: Some("auto".to_string()),
            model: Some(String::new()),
            base_url: Some(String::new()),
            api_key: Some(String::new()),
            timeout: Some(30),
            extra_body: HashMap::new(),
            max_concurrency: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxiliaryConfig {
    pub vision: AuxTaskConfig,
    pub web_extract: AuxTaskConfig,
    pub compression: AuxTaskConfig,
    pub session_search: AuxTaskConfig,
    pub approval: AuxTaskConfig,
    pub mcp: AuxTaskConfig,
    pub curator: AuxTaskConfig,
}

impl Default for AuxiliaryConfig {
    fn default() -> Self {
        Self {
            vision: AuxTaskConfig {
                timeout: Some(120),
                ..Default::default()
            },
            web_extract: AuxTaskConfig {
                timeout: Some(360),
                ..Default::default()
            },
            compression: AuxTaskConfig {
                timeout: Some(120),
                ..Default::default()
            },
            session_search: AuxTaskConfig {
                timeout: Some(30),
                max_concurrency: Some(3),
                ..Default::default()
            },
            approval: AuxTaskConfig {
                timeout: Some(30),
                ..Default::default()
            },
            mcp: AuxTaskConfig {
                timeout: Some(30),
                ..Default::default()
            },
            curator: AuxTaskConfig {
                timeout: Some(600),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub compact: Option<bool>,
    pub personality: Option<String>,
    pub streaming: Option<bool>,
    pub skin: Option<String>,
    pub language: Option<String>,
    pub tool_progress_command: Option<bool>,
    pub platforms: HashMap<String, HashMap<String, Value>>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            compact: Some(false),
            personality: Some("kawaii".to_string()),
            streaming: Some(false),
            skin: Some("default".to_string()),
            language: Some("en".to_string()),
            tool_progress_command: Some(false),
            platforms: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DashboardConfig {
    pub theme: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            theme: Some("default".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct TtsProviderConfig {
    pub voice: Option<String>,
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub voice_id: Option<String>,
    pub language: Option<String>,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsConfig {
    pub provider: Option<String>,
    pub edge: TtsProviderConfig,
    pub elevenlabs: TtsProviderConfig,
    pub openai: TtsProviderConfig,
    pub xai: TtsProviderConfig,
    pub mistral: TtsProviderConfig,
    pub neutts: TtsProviderConfig,
    pub piper: TtsProviderConfig,
}

impl Default for TtsConfig {
    fn default() -> Self {
        let edge = TtsProviderConfig {
            voice: Some("en-US-AriaNeural".to_string()),
            ..Default::default()
        };
        let elevenlabs = TtsProviderConfig {
            voice_id: Some("pNInz6obpgDQGcFmaJgB".to_string()),
            model_id: Some("eleven_multilingual_v2".to_string()),
            ..Default::default()
        };
        let openai_tts = TtsProviderConfig {
            model: Some("gpt-4o-mini-tts".to_string()),
            voice: Some("alloy".to_string()),
            ..Default::default()
        };
        let xai = TtsProviderConfig {
            voice_id: Some("eve".to_string()),
            language: Some("en".to_string()),
            ..Default::default()
        };
        let mistral = TtsProviderConfig {
            model: Some("voxtral-mini-tts-2603".to_string()),
            voice_id: Some("c69964a6-ab8b-4f8a-9465-ec0925096ec8".to_string()),
            ..Default::default()
        };
        let neutts = TtsProviderConfig {
            model: Some("neuphonic/neutts-air-q4-gguf".to_string()),
            device: Some("cpu".to_string()),
            ..Default::default()
        };
        let piper = TtsProviderConfig {
            voice: Some("en_US-lessac-medium".to_string()),
            ..Default::default()
        };

        Self {
            provider: Some("edge".to_string()),
            edge,
            elevenlabs,
            openai: openai_tts,
            xai,
            mistral,
            neutts,
            piper,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttProviderConfig {
    pub model: Option<String>,
    pub language: Option<String>,
}

impl Default for SttProviderConfig {
    fn default() -> Self {
        Self {
            model: Some("base".to_string()),
            language: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SttConfigV2 {
    pub enabled: Option<bool>,
    pub provider: Option<String>,
    pub local: SttProviderConfig,
    pub openai: SttProviderConfig,
    pub mistral: SttProviderConfig,
}

impl Default for SttConfigV2 {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            provider: Some("local".to_string()),
            local: SttProviderConfig::default(),
            openai: SttProviderConfig {
                model: Some("whisper-1".to_string()),
                ..Default::default()
            },
            mistral: SttProviderConfig {
                model: Some("voxtral-mini-latest".to_string()),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfigV2 {
    pub provider: Option<String>,
}

impl Default for MemoryConfigV2 {
    fn default() -> Self {
        Self {
            provider: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DelegationConfig {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub max_iterations: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub max_concurrent_children: Option<u32>,
    pub max_spawn_depth: Option<u32>,
    pub orchestrator_enabled: Option<bool>,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            model: Some(String::new()),
            provider: Some(String::new()),
            base_url: Some(String::new()),
            api_key: Some(String::new()),
            max_iterations: Some(90),
            reasoning_effort: Some(String::new()),
            max_concurrent_children: Some(3),
            max_spawn_depth: Some(2),
            orchestrator_enabled: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalsConfig {
    pub max_turns: Option<u32>,
}

impl Default for GoalsConfig {
    fn default() -> Self {
        Self {
            max_turns: Some(20),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillsConfigV2 {
    pub template_vars: Option<bool>,
    pub inline_shell: Option<bool>,
    pub inline_shell_timeout: Option<u64>,
}

impl Default for SkillsConfigV2 {
    fn default() -> Self {
        Self {
            template_vars: Some(true),
            inline_shell: Some(false),
            inline_shell_timeout: Some(10),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CuratorConfig {
    pub enabled: Option<bool>,
    pub interval_hours: Option<u64>,
    pub stale_after_days: Option<u64>,
    pub archive_after_days: Option<u64>,
    pub backup: CuratorBackupConfig,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            interval_hours: Some(168), // 24 * 7
            stale_after_days: Some(30),
            archive_after_days: Some(90),
            backup: CuratorBackupConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CuratorBackupConfig {
    pub enabled: Option<bool>,
    pub keep: Option<u32>,
}

impl Default for CuratorBackupConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            keep: Some(5),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalsConfig {
    pub mode: Option<String>,
    pub timeout: Option<u64>,
}

impl Default for ApprovalsConfig {
    fn default() -> Self {
        Self {
            mode: Some("manual".to_string()),
            timeout: Some(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub redact_secrets: Option<bool>,
    // NOTE: `tirith_*` keys used to live here. They were removed with no
    // replacement: no tirith module ever existed in the tree (only a doc
    // comment advertising one), so the four fields were defined, defaulted
    // (fail-open), and never read. This struct has `#[serde(default)]` and
    // no `deny_unknown_fields`, so existing configs carrying `tirith_*`
    // keys still parse — the keys are simply ignored, as they always were.
    pub command_allowlist: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            redact_secrets: Some(true),
            command_allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CodeExecutionConfigV2 {
    pub mode: Option<String>,
}

impl Default for CodeExecutionConfigV2 {
    fn default() -> Self {
        Self {
            mode: Some("project".to_string()),
        }
    }
}

/// Plan 015 mirror of core `KernelSettings` (Option-wrapped V2 form).
/// Kept field-identical to the core struct; a round-trip test asserts both
/// parse one TOML snippet consistently so the copies cannot drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KernelConfigV2 {
    pub enabled: Option<bool>,
    pub python: Option<String>,
    pub vendor_dir: Option<String>,
    pub state_dir: Option<String>,
    pub sidecar_idle_secs: Option<u64>,
    pub request_timeout_secs: Option<u64>,
    pub max_output_bytes: Option<usize>,
    pub route_python_to_kernel: Option<bool>,
    pub session_gc_ttl_hours: Option<u64>,
}

impl Default for KernelConfigV2 {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            python: None,
            vendor_dir: None,
            state_dir: None,
            sidecar_idle_secs: Some(1800),
            request_timeout_secs: Some(120),
            max_output_bytes: Some(200_000),
            route_python_to_kernel: Some(false),
            session_gc_ttl_hours: Some(168),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfigV2 {
    pub level: Option<String>,
}

impl Default for LoggingConfigV2 {
    fn default() -> Self {
        Self {
            level: Some("INFO".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelCatalogConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub ttl_hours: Option<u64>,
    pub providers: HashMap<String, HashMap<String, String>>,
}

impl Default for ModelCatalogConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            url: Some(
                "https://operant-agent.nousresearch.com/docs/api/model-catalog.json".to_string(),
            ),
            ttl_hours: Some(24),
            providers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionsConfig {
    pub retention_days: Option<u32>,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            retention_days: Some(90),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolOutputLimitsConfig {
    pub max_bytes: Option<u32>,
    pub max_lines: Option<u32>,
}

impl Default for ToolOutputLimitsConfig {
    fn default() -> Self {
        Self {
            max_bytes: Some(50_000),
            max_lines: Some(2000),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfigV2 {
    pub tool_output: ToolOutputLimitsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct GatewaysConfig {
    pub telegram: GatewayPlatformConfig,
    pub discord: DiscordGatewayConfig,
    pub slack: SlackGatewayConfig,
    pub mattermost: MattermostGatewayConfig,
    pub matrix: MatrixGatewayConfig,
    pub whatsapp: HashMap<String, Value>,
    pub webhooks: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayPlatformConfig {
    pub enabled: Option<bool>,
    pub token: Option<String>,
    pub allowed_users: Option<String>,
    pub reactions: Option<bool>,
    pub proxy: Option<String>,
}

impl Default for GatewayPlatformConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            token: None,
            allowed_users: None,
            reactions: Some(false),
            proxy: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordGatewayConfig {
    pub enabled: Option<bool>,
    pub token: Option<String>,
    pub allowed_users: Option<String>,
    pub reactions: Option<bool>,
}

impl Default for DiscordGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            token: None,
            allowed_users: None,
            reactions: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackGatewayConfig {
    pub enabled: Option<bool>,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
}

impl Default for SlackGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            bot_token: None,
            app_token: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MattermostGatewayConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub allowed_users: Option<String>,
}

impl Default for MattermostGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            url: None,
            token: None,
            allowed_users: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MatrixGatewayConfig {
    pub enabled: Option<bool>,
    pub homeserver: Option<String>,
    pub access_token: Option<String>,
    pub user_id: Option<String>,
    pub password: Option<String>,
    pub encryption: Option<bool>,
    pub device_id: Option<String>,
    pub allowed_rooms: Option<String>,
    pub allowed_users: Option<String>,
}

impl Default for MatrixGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            homeserver: None,
            access_token: None,
            user_id: None,
            password: None,
            encryption: Some(false),
            device_id: None,
            allowed_rooms: Some(String::new()),
            allowed_users: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvironmentsConfig {
    pub docker: DockerEnvConfig,
    pub ssh: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct DockerEnvConfig {
    pub image: Option<String>,
    pub network: Option<String>,
    pub volumes: Vec<String>,
    pub working_dir: Option<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct IntegrationsConfig {
    pub enabled: Vec<String>,
    pub configs: HashMap<String, HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct PluginsConfig {
    pub enabled: Vec<String>,
    pub configs: HashMap<String, HashMap<String, Value>>,
    pub dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfigV2 {
    pub engine: Option<String>,
}

impl Default for ContextConfigV2 {
    fn default() -> Self {
        Self {
            engine: Some("compressor".to_string()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    pub scripts: HashMap<String, Vec<HookScriptConfig>>,
    pub personalities: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HookScriptConfig {
    pub matcher: Option<String>,
    pub command: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct OnboardingConfig {
    pub seen: HashMap<String, bool>,
}

/// Errors specific to CLI configuration loading.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    #[allow(dead_code)]
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "Config IO error: {}", e),
            ConfigError::Parse(msg) => write!(f, "Config parse error: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

/// Type alias for config results.
pub type ConfigResult<T> = std::result::Result<T, ConfigError>;

impl CliConfig {
    /// Load config from default locations.
    ///
    /// `config.yaml`/`config.local.yaml` are deprecated — `operant.toml`
    /// (loaded separately via `operant-core`) is the sole config source now.
    /// This only loads the `.env` file and applies `HERMES_*` env overrides;
    /// the resolved (but unread) legacy paths are kept on the struct solely
    /// for `operant doctor`'s legacy-file detection.
    ///
    /// Loading order:
    /// 1. Default config values in code
    /// 2. .env file from HERMES_HOME
    /// 3. HERMES_* environment variable overrides
    pub fn load() -> ConfigResult<Self> {
        let mut config = Self::default();
        let config_dir = config.config_dir.clone();

        config.config_file = std::env::var("HERMES_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| config_dir.join("config.yaml"));
        config.local_config_file = config_dir.join("config.local.yaml");

        // Load .env file
        if config.env_file.exists() {
            load_dotenv_file(&config.env_file)?;
        }

        // Apply HERMES_* env var overrides
        config.apply_operant_env_overrides();

        Ok(config)
    }

    /// Apply HERMES_* environment variable overrides to the config.
    fn apply_operant_env_overrides(&mut self) {
        // HERMES_MODEL → agent default model
        if let Ok(val) = std::env::var("HERMES_MODEL")
            && !val.is_empty()
        {
            self.api.default_model = Some(val);
        }

        // HERMES_LOG_LEVEL
        if let Ok(val) = std::env::var("HERMES_LOG_LEVEL")
            && !val.is_empty()
        {
            self.logging.level = Some(val);
        }

        // HERMES_MAX_ITERATIONS
        if let Ok(val) = std::env::var("HERMES_MAX_ITERATIONS")
            && let Ok(n) = val.parse::<u32>()
        {
            self.agent.max_turns = Some(n);
        }

        // HERMES_TOOL_TIMEOUT
        if let Ok(val) = std::env::var("HERMES_TOOL_TIMEOUT")
            && let Ok(n) = val.parse::<u64>()
        {
            self.terminal.timeout = Some(n);
        }

        // HERMES_REQUEST_TIMEOUT
        if let Ok(val) = std::env::var("HERMES_REQUEST_TIMEOUT")
            && let Ok(n) = val.parse::<u64>()
        {
            self.api.timeout_seconds = Some(n);
        }

        // HERMES_CONTEXT_WINDOW
        if let Ok(val) = std::env::var("HERMES_CONTEXT_WINDOW")
            && let Ok(n) = val.parse::<u32>()
        {
            self.api.max_tokens = Some(n);
        }

        // HERMES_STREAM
        if let Ok(val) = std::env::var("HERMES_STREAM") {
            self.display.streaming = Some(matches!(
                val.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ));
        }

        // HERMES_SKILLS_DIR
        if let Ok(val) = std::env::var("HERMES_SKILLS_DIR")
            && !val.is_empty()
        {
            self.operant.skills_dir = Some(PathBuf::from(val));
        }

        // HERMES_HOME — already handled in paths
        // HERMES_CONFIG — already handled in load path selection
    }
}

// =============================================================================
// Env Var Expansion
// =============================================================================

/// Expand `${VAR_NAME}` and `$VAR_NAME` patterns in a string value.
///
/// Supports:
/// - `${VAR}` — replaced with env var value (or empty string if unset)
/// - `$VAR` — same as above
/// - `${VAR:-default}` — uses `default` if VAR is unset or empty
/// - Recursive expansion: values of env vars can themselves contain `${OTHER}`
pub fn expand_env_vars(value: &str) -> String {
    if !value.contains('$') {
        return value.to_string();
    }

    let mut result = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            match chars.peek() {
                Some('{') => {
                    // ${VAR} or ${VAR:-default}
                    chars.next(); // consume '{'
                    let mut var_name = String::new();
                    let mut default_value = String::new();
                    let mut has_default = false;

                    while let Some(&c) = chars.peek() {
                        if c == ':' {
                            chars.next(); // consume ':'
                            if chars.peek() == Some(&'-') {
                                chars.next(); // consume '-'
                                has_default = true;
                                // Collect default value until '}'
                                while let Some(&c) = chars.peek() {
                                    if c == '}' {
                                        break;
                                    }
                                    default_value.push(c);
                                    chars.next();
                                }
                                break;
                            }
                        }
                        if c == '}' {
                            break;
                        }
                        var_name.push(c);
                        chars.next();
                    }
                    chars.next(); // consume '}'

                    let expanded = resolve_env_var(&var_name, &default_value, has_default);
                    result.push_str(&expanded);
                }
                _ => {
                    // $VAR — collect alphanumeric characters
                    let mut var_name = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            var_name.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let expanded = resolve_env_var(&var_name, "", false);
                    result.push_str(&expanded);
                }
            }
        } else {
            result.push(ch);
        }
    }

    // Recursive expansion: check if result still contains $ patterns
    if result.contains('$') && result != value {
        expand_env_vars(&result)
    } else {
        result
    }
}

/// Resolve a single env var reference with optional default.
fn resolve_env_var(name: &str, default: &str, has_default: bool) -> String {
    match std::env::var(name) {
        Ok(val) if !val.is_empty() => val,
        _ if has_default => default.to_string(),
        _ => String::new(),
    }
}

// =============================================================================
// Dotenv Loading
// =============================================================================

/// Load a .env file into the process environment.
/// Lines are "KEY=VALUE" format. Comments start with #. Empty lines are skipped.
pub fn load_dotenv_file(path: &Path) -> ConfigResult<()> {
    let contents = std::fs::read_to_string(path)?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            let value = trimmed[eq_pos + 1..].trim();

            let value = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };

            let expanded = expand_env_vars(value);

            if std::env::var(key).is_err() {
                // SAFETY: no other thread can read the environment here.
                // `load_dotenv_file` is reached only from `CliConfig::load()`,
                // and `main` (main.rs) calls that before it builds the tokio
                // runtime, so no worker thread exists yet. This is enforced by
                // the order in `main`, not by a comment alone: moving the load
                // after the runtime would break it. The `is_err()` guard keeps
                // a variable the user exported deliberately from being replaced.
                unsafe {
                    std::env::set_var(key, expanded);
                }
            }
        }
    }
    Ok(())
}

// =============================================================================
// Conversion to core AppConfig
// =============================================================================

impl CliConfig {
    /// Convert the CLI config into a core `AppConfig`, merging CLI-level
    /// settings with the existing core structure.
    pub fn to_app_config(&self) -> operant_core::config::AppConfig {
        let mut app = operant_core::config::AppConfig::default();

        // Map API settings
        if let Some(base_url) = &self.api.base_url {
            app.client.base_url = base_url.clone();
        }
        if let Some(api_key) = &self.api.api_key {
            app.client.api_key = Some(api_key.clone());
        }
        if let Some(timeout) = self.api.timeout_seconds {
            app.client.timeout_secs = timeout;
        }
        if let Some(max_tokens) = self.api.max_tokens {
            app.client.max_context_length = max_tokens as usize;
        }

        // Map agent settings
        if let Some(model) = &self.api.default_model {
            app.agent.model = model.clone();
        }
        if let Some(max_turns) = self.agent.max_turns {
            app.agent.max_iterations = max_turns as usize;
        }
        if let Some(streaming) = self.display.streaming {
            app.agent.stream = streaming;
        }
        if let Some(window) = self.api.max_tokens {
            app.agent.context_window = window as usize;
        }

        // Map logging
        if let Some(level) = &self.logging.level {
            app.logging.level = level.clone();
        }

        // Map terminal timeout to tool timeout
        if let Some(timeout) = self.terminal.timeout {
            app.agent.tool_timeout_secs = timeout;
        }
        if let Some(timeout) = self.api.timeout_seconds {
            app.agent.request_timeout_secs = timeout;
        }

        // Map skills directory
        if let Some(skills_dir) = &self.operant.skills_dir {
            app.skills.root_dir = skills_dir.clone();
        }

        // Map MCP/gateway settings from CLI gateways
        if self.gateways.telegram.enabled.unwrap_or(false) {
            app.gateway.telegram_enabled = true;
            if let Some(token) = &self.gateways.telegram.token {
                app.gateway.telegram_token = Some(token.clone());
            }
        }
        if self.gateways.discord.enabled.unwrap_or(false) {
            app.gateway.discord_enabled = true;
            if let Some(token) = &self.gateways.discord.token {
                app.gateway.discord_token = Some(token.clone());
            }
        }
        if self.gateways.slack.enabled.unwrap_or(false) {
            app.gateway.slack_enabled = true;
            if let Some(token) = &self.gateways.slack.bot_token {
                app.gateway.slack_token = Some(token.clone());
            }
        }

        app
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::OnceLock;

    /// Plan 015 drift-guard: one TOML snippet must parse consistently into the
    /// CLI V2 mirror and core KernelSettings (values + defaults agree).
    #[test]
    fn kernel_v2_and_core_parse_same_toml() {
        let toml = r#"
            enabled = true
            route_python_to_kernel = false
            sidecar_idle_secs = 900
            request_timeout_secs = 60
        "#;
        let v2: KernelConfigV2 = toml::from_str(toml).expect("v2 parse");
        assert_eq!(v2.enabled, Some(true));
        assert_eq!(v2.sidecar_idle_secs, Some(900));

        let core: operant_core::config::KernelSettings = toml::from_str(toml).expect("core parse");
        assert!(core.enabled);
        assert_eq!(core.sidecar_idle_secs, 900);
        assert_eq!(core.request_timeout_secs, 60);
        // Default agreement on untouched fields:
        let default_core = operant_core::config::KernelSettings::default();
        assert_eq!(default_core.max_output_bytes, 200_000);
    }

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env(name: &str, value: &str) -> Option<std::ffi::OsString> {
        let prev = std::env::var_os(name);
        // SAFETY: test-only env mutation
        unsafe { std::env::set_var(name, value) };
        prev
    }

    fn restore_env(name: &str, prev: Option<std::ffi::OsString>) {
        match prev {
            Some(val) => unsafe { std::env::set_var(name, val) },
            None => unsafe { std::env::remove_var(name) },
        }
    }

    // ================================================================
    // Default Config Tests
    // ================================================================

    #[test]
    fn test_default_config_has_all_sections() {
        let config = CliConfig::default();
        assert_eq!(config.operant.name.as_deref(), Some("operant-rs"));
        assert!(config.api.max_retries.is_some());
        assert!(config.terminal.timeout.is_some());
        assert!(config.compression.enabled.unwrap_or(false));
        assert_eq!(config.display.language.as_deref(), Some("en"));
    }

    #[test]
    fn test_default_config_version_is_set() {
        let config = CliConfig::default();
        assert_eq!(config.config_version.as_deref(), Some("1.0.0"));
    }

    // ================================================================
    // Env Var Expansion Tests
    // ================================================================

    #[test]
    fn test_expand_simple_var() {
        let _guard = env_lock().lock().unwrap();
        let prev = set_env("TEST_HERMES_VAR", "hello");
        assert_eq!(expand_env_vars("$TEST_HERMES_VAR"), "hello");
        assert_eq!(expand_env_vars("${TEST_HERMES_VAR}"), "hello");
        restore_env("TEST_HERMES_VAR", prev);
    }

    #[test]
    fn test_expand_with_default() {
        assert_eq!(expand_env_vars("${UNSET_VAR:-fallback}"), "fallback");
    }

    #[test]
    fn test_expand_unset_var_empty() {
        assert_eq!(expand_env_vars("$MISSING_VAR_XYZ"), "");
    }

    #[test]
    fn test_expand_no_vars() {
        assert_eq!(expand_env_vars("hello world"), "hello world");
    }

    #[test]
    fn test_expand_mixed_content() {
        let _guard = env_lock().lock().unwrap();
        let prev = set_env("TEST_NAME", "world");
        assert_eq!(expand_env_vars("hello $TEST_NAME!"), "hello world!");
        restore_env("TEST_NAME", prev);
    }

    #[test]
    fn test_expand_recursive() {
        let _guard = env_lock().lock().unwrap();
        let prev_a = set_env("OUTER", "hello ${INNER}");
        let prev_b = set_env("INNER", "world");
        assert_eq!(expand_env_vars("${OUTER}"), "hello world");
        restore_env("OUTER", prev_a);
        restore_env("INNER", prev_b);
    }

    #[test]
    fn test_expand_multiple_vars() {
        let _guard = env_lock().lock().unwrap();
        let prev_a = set_env("A", "1");
        let prev_b = set_env("B", "2");
        assert_eq!(expand_env_vars("${A} + ${B} = 3"), "1 + 2 = 3");
        restore_env("A", prev_a);
        restore_env("B", prev_b);
    }

    // ================================================================
    // Dotenv Loading Tests
    // ================================================================

    #[test]
    fn test_load_dotenv_file() {
        let _guard = env_lock().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("operant_dotenv_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let env_path = dir.join("test.env");

        std::fs::write(&env_path, "# Comment\nKEY=value\nNUMBER=42\nEMPTY=\n").unwrap();

        // Clear env vars first
        // SAFETY: test-only env mutation
        unsafe { std::env::remove_var("KEY") };
        // SAFETY: test-only env mutation
        unsafe { std::env::remove_var("NUMBER") };

        load_dotenv_file(&env_path).unwrap();

        assert_eq!(std::env::var("KEY").unwrap(), "value");
        assert_eq!(std::env::var("NUMBER").unwrap(), "42");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_dotenv_quoted_values() {
        let _guard = env_lock().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("operant_dotenv_quote_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let env_path = dir.join("quoted.env");

        std::fs::write(
            &env_path,
            "DOUBLE=\"hello world\"\nSINGLE='single quoted'\n",
        )
        .unwrap();

        // SAFETY: test-only env mutation
        unsafe { std::env::remove_var("DOUBLE") };
        // SAFETY: test-only env mutation
        unsafe { std::env::remove_var("SINGLE") };

        load_dotenv_file(&env_path).unwrap();

        assert_eq!(std::env::var("DOUBLE").unwrap(), "hello world");
        assert_eq!(std::env::var("SINGLE").unwrap(), "single quoted");

        let _ = std::fs::remove_dir_all(dir);
    }

    // ================================================================
    // HERMES_* Env Override Tests
    // ================================================================

    #[test]
    fn test_operant_env_overrides_model() {
        let _guard = env_lock().lock().unwrap();
        let prev = set_env("HERMES_MODEL", "gpt-5");

        let mut config = CliConfig::default();
        config.apply_operant_env_overrides();
        assert_eq!(config.api.default_model.as_deref(), Some("gpt-5"));

        restore_env("HERMES_MODEL", prev);
    }

    #[test]
    fn test_operant_env_overrides_log_level() {
        let _guard = env_lock().lock().unwrap();
        let prev = set_env("HERMES_LOG_LEVEL", "DEBUG");

        let mut config = CliConfig::default();
        config.apply_operant_env_overrides();
        assert_eq!(config.logging.level.as_deref(), Some("DEBUG"));

        restore_env("HERMES_LOG_LEVEL", prev);
    }

    // ================================================================
    // To AppConfig Conversion Tests
    // ================================================================

    #[test]
    fn test_to_app_config_maps_fields() {
        let mut cli_config = CliConfig::default();
        cli_config.api.base_url = Some("https://custom.api.com/v1".to_string());
        cli_config.api.default_model = Some("gpt-5".to_string());
        cli_config.api.timeout_seconds = Some(300);
        cli_config.display.streaming = Some(false);
        cli_config.logging.level = Some("WARN".to_string());

        let app_config = cli_config.to_app_config();

        assert_eq!(app_config.client.base_url, "https://custom.api.com/v1");
        assert_eq!(app_config.client.timeout_secs, 300);
        assert_eq!(app_config.agent.model, "gpt-5");
        assert!(!app_config.agent.stream);
        assert_eq!(app_config.logging.level, "WARN");
    }
}

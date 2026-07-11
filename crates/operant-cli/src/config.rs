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
    pub openrouter: OpenRouterConfig,
    pub bedrock: BedrockConfig,
    pub auxiliary: AuxiliaryConfig,
    pub display: DisplayConfig,
    pub dashboard: DashboardConfig,
    pub privacy: PrivacyConfig,
    pub tts: TtsConfig,
    pub stt: SttConfigV2,
    pub voice: VoiceConfig,
    pub memory: MemoryConfigV2,
    pub delegation: DelegationConfig,
    pub goals: GoalsConfig,
    pub skills: SkillsConfigV2,
    pub curator: CuratorConfig,
    pub approvals: ApprovalsConfig,
    pub security: SecurityConfig,
    pub cron: CronConfigV2,
    pub kanban: KanbanConfig,
    pub code_execution: CodeExecutionConfigV2,
    pub logging: LoggingConfigV2,
    pub model_catalog: ModelCatalogConfig,
    pub sessions: SessionsConfig,
    pub updates: UpdatesConfig,
    pub network: NetworkConfig,
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
            openrouter: OpenRouterConfig::default(),
            bedrock: BedrockConfig::default(),
            auxiliary: AuxiliaryConfig::default(),
            display: DisplayConfig::default(),
            dashboard: DashboardConfig::default(),
            privacy: PrivacyConfig::default(),
            tts: TtsConfig::default(),
            stt: SttConfigV2::default(),
            voice: VoiceConfig::default(),
            memory: MemoryConfigV2::default(),
            delegation: DelegationConfig::default(),
            goals: GoalsConfig::default(),
            skills: SkillsConfigV2::default(),
            curator: CuratorConfig::default(),
            approvals: ApprovalsConfig::default(),
            security: SecurityConfig::default(),
            cron: CronConfigV2::default(),
            kanban: KanbanConfig::default(),
            code_execution: CodeExecutionConfigV2::default(),
            logging: LoggingConfigV2::default(),
            model_catalog: ModelCatalogConfig::default(),
            sessions: SessionsConfig::default(),
            updates: UpdatesConfig::default(),
            network: NetworkConfig::default(),
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
    pub session_dir: Option<PathBuf>,
    pub assistant_prelude: Option<String>,
    pub auto_summarize: Option<bool>,
    pub auto_distill: Option<bool>,
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
            session_dir: None,
            assistant_prelude: None,
            auto_summarize: Some(true),
            auto_distill: Some(false),
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
    pub api_type: Option<String>,
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
            api_type: Some("chat_completions".to_string()),
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
    pub supports_system_prompt: Option<bool>,
    pub input_price_per_1k: Option<f64>,
    pub output_price_per_1k: Option<f64>,
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
            supports_system_prompt: Some(true),
            input_price_per_1k: None,
            output_price_per_1k: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfigV2 {
    pub max_turns: Option<u32>,
    pub gateway_timeout: Option<u64>,
    pub restart_drain_timeout: Option<u64>,
    pub api_max_retries: Option<u32>,
    pub service_tier: Option<String>,
    pub tool_use_enforcement: Option<String>,
    pub gateway_timeout_warning: Option<u64>,
    pub gateway_notify_interval: Option<u64>,
    pub gateway_auto_continue_freshness: Option<u64>,
    pub image_input_mode: Option<String>,
    pub disabled_toolsets: Vec<String>,
}

impl Default for AgentConfigV2 {
    fn default() -> Self {
        Self {
            max_turns: Some(90),
            gateway_timeout: Some(1800),
            restart_drain_timeout: Some(180),
            api_max_retries: Some(3),
            service_tier: Some(String::new()),
            tool_use_enforcement: Some("auto".to_string()),
            gateway_timeout_warning: Some(900),
            gateway_notify_interval: Some(180),
            gateway_auto_continue_freshness: Some(3600),
            image_input_mode: Some("auto".to_string()),
            disabled_toolsets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub backend: Option<String>,
    pub modal_mode: Option<String>,
    pub cwd: Option<String>,
    pub timeout: Option<u64>,
    pub env_passthrough: Vec<String>,
    pub shell_init_files: Vec<String>,
    pub auto_source_bashrc: Option<bool>,
    pub docker_image: Option<String>,
    pub docker_forward_env: Vec<String>,
    pub docker_env: HashMap<String, String>,
    pub docker_volumes: Vec<String>,
    pub docker_mount_cwd_to_workspace: Option<bool>,
    pub docker_run_as_host_user: Option<bool>,
    pub singularity_image: Option<String>,
    pub modal_image: Option<String>,
    pub daytona_image: Option<String>,
    pub vercel_runtime: Option<String>,
    pub container_cpu: Option<u32>,
    pub container_memory: Option<u32>,
    pub container_disk: Option<u32>,
    pub container_persistent: Option<bool>,
    pub persistent_shell: Option<bool>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            backend: Some("local".to_string()),
            modal_mode: Some("auto".to_string()),
            cwd: Some(".".to_string()),
            timeout: Some(180),
            env_passthrough: Vec::new(),
            shell_init_files: Vec::new(),
            auto_source_bashrc: Some(true),
            docker_image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            docker_forward_env: Vec::new(),
            docker_env: HashMap::new(),
            docker_volumes: Vec::new(),
            docker_mount_cwd_to_workspace: Some(false),
            docker_run_as_host_user: Some(false),
            singularity_image: Some(
                "docker://nikolaik/python-nodejs:python3.11-nodejs20".to_string(),
            ),
            modal_image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            daytona_image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            vercel_runtime: Some("node24".to_string()),
            container_cpu: Some(1),
            container_memory: Some(5120),
            container_disk: Some(51200),
            container_persistent: Some(true),
            persistent_shell: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfigV2 {
    pub backend: Option<String>,
    pub search_backend: Option<String>,
    pub extract_backend: Option<String>,
}

impl Default for WebConfigV2 {
    fn default() -> Self {
        Self {
            backend: Some(String::new()),
            search_backend: Some(String::new()),
            extract_backend: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    pub inactivity_timeout: Option<u64>,
    pub command_timeout: Option<u64>,
    pub record_sessions: Option<bool>,
    pub allow_private_urls: Option<bool>,
    pub engine: Option<String>,
    pub auto_local_for_private_urls: Option<bool>,
    pub cdp_url: Option<String>,
    pub dialog_policy: Option<String>,
    pub dialog_timeout_s: Option<u64>,
    pub camofox: CamofoxConfig,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            inactivity_timeout: Some(120),
            command_timeout: Some(30),
            record_sessions: Some(false),
            allow_private_urls: Some(false),
            engine: Some("auto".to_string()),
            auto_local_for_private_urls: Some(true),
            cdp_url: Some(String::new()),
            dialog_policy: Some("must_respond".to_string()),
            dialog_timeout_s: Some(300),
            camofox: CamofoxConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CamofoxConfig {
    pub managed_persistence: Option<bool>,
}

impl Default for CamofoxConfig {
    fn default() -> Self {
        Self {
            managed_persistence: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CheckpointsConfig {
    pub enabled: Option<bool>,
    pub max_snapshots: Option<u32>,
    pub max_total_size_mb: Option<u32>,
    pub max_file_size_mb: Option<u32>,
    pub auto_prune: Option<bool>,
    pub retention_days: Option<u32>,
    pub delete_orphans: Option<bool>,
    pub min_interval_hours: Option<u32>,
}

impl Default for CheckpointsConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            max_snapshots: Some(20),
            max_total_size_mb: Some(500),
            max_file_size_mb: Some(10),
            auto_prune: Some(true),
            retention_days: Some(7),
            delete_orphans: Some(true),
            min_interval_hours: Some(24),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    pub enabled: Option<bool>,
    pub threshold: Option<f64>,
    pub target_ratio: Option<f64>,
    pub protect_last_n: Option<u32>,
    pub hygiene_hard_message_limit: Option<u32>,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            threshold: Some(0.50),
            target_ratio: Some(0.20),
            protect_last_n: Some(20),
            hygiene_hard_message_limit: Some(400),
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
pub struct OpenRouterConfig {
    pub response_cache: Option<bool>,
    pub response_cache_ttl: Option<u64>,
    pub min_coding_score: Option<f64>,
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            response_cache: Some(true),
            response_cache_ttl: Some(300),
            min_coding_score: Some(0.65),
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
    pub refresh_interval: Option<u64>,
}

impl Default for BedrockDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            provider_filter: Vec::new(),
            refresh_interval: Some(3600),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BedrockGuardrailConfig {
    pub guardrail_identifier: Option<String>,
    pub guardrail_version: Option<String>,
    pub stream_processing_mode: Option<String>,
    pub trace: Option<String>,
}

impl Default for BedrockGuardrailConfig {
    fn default() -> Self {
        Self {
            guardrail_identifier: Some(String::new()),
            guardrail_version: Some(String::new()),
            stream_processing_mode: Some("async".to_string()),
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
    pub download_timeout: Option<u64>,
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
            download_timeout: None,
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
    pub skills_hub: AuxTaskConfig,
    pub approval: AuxTaskConfig,
    pub mcp: AuxTaskConfig,
    pub title_generation: AuxTaskConfig,
    pub triage_specifier: AuxTaskConfig,
    pub curator: AuxTaskConfig,
}

impl Default for AuxiliaryConfig {
    fn default() -> Self {
        Self {
            vision: AuxTaskConfig {
                timeout: Some(120),
                download_timeout: Some(30),
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
            skills_hub: AuxTaskConfig {
                timeout: Some(30),
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
            title_generation: AuxTaskConfig {
                timeout: Some(30),
                ..Default::default()
            },
            triage_specifier: AuxTaskConfig {
                timeout: Some(120),
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
    pub resume_display: Option<String>,
    pub busy_input_mode: Option<String>,
    pub tui_auto_resume_recent: Option<bool>,
    pub bell_on_complete: Option<bool>,
    pub show_reasoning: Option<bool>,
    pub streaming: Option<bool>,
    pub final_response_markdown: Option<String>,
    pub persistent_output: Option<bool>,
    pub persistent_output_max_lines: Option<u32>,
    pub inline_diffs: Option<bool>,
    pub show_cost: Option<bool>,
    pub skin: Option<String>,
    pub language: Option<String>,
    pub tui_status_indicator: Option<String>,
    pub user_message_preview: UserMessagePreviewConfig,
    pub interim_assistant_messages: Option<bool>,
    pub tool_progress_command: Option<bool>,
    pub tool_preview_length: Option<u32>,
    pub ephemeral_system_ttl: Option<u64>,
    pub platforms: HashMap<String, HashMap<String, Value>>,
    pub runtime_footer: RuntimeFooterConfig,
    pub copy_shortcut: Option<String>,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            compact: Some(false),
            personality: Some("kawaii".to_string()),
            resume_display: Some("full".to_string()),
            busy_input_mode: Some("interrupt".to_string()),
            tui_auto_resume_recent: Some(false),
            bell_on_complete: Some(false),
            show_reasoning: Some(false),
            streaming: Some(false),
            final_response_markdown: Some("strip".to_string()),
            persistent_output: Some(true),
            persistent_output_max_lines: Some(200),
            inline_diffs: Some(true),
            show_cost: Some(false),
            skin: Some("default".to_string()),
            language: Some("en".to_string()),
            tui_status_indicator: Some("kaomoji".to_string()),
            user_message_preview: UserMessagePreviewConfig::default(),
            interim_assistant_messages: Some(true),
            tool_progress_command: Some(false),
            tool_preview_length: Some(0),
            ephemeral_system_ttl: Some(0),
            platforms: HashMap::new(),
            runtime_footer: RuntimeFooterConfig::default(),
            copy_shortcut: Some("auto".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserMessagePreviewConfig {
    pub first_lines: Option<u32>,
    pub last_lines: Option<u32>,
}

impl Default for UserMessagePreviewConfig {
    fn default() -> Self {
        Self {
            first_lines: Some(2),
            last_lines: Some(2),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeFooterConfig {
    pub enabled: Option<bool>,
    pub fields: Vec<String>,
}

impl Default for RuntimeFooterConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            fields: vec![
                "model".to_string(),
                "context_pct".to_string(),
                "cwd".to_string(),
            ],
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
pub struct PrivacyConfig {
    pub redact_pii: Option<bool>,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            redact_pii: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TtsProviderConfig {
    pub voice: Option<String>,
    pub model: Option<String>,
    pub model_id: Option<String>,
    pub voice_id: Option<String>,
    pub language: Option<String>,
    pub sample_rate: Option<u32>,
    pub bit_rate: Option<u32>,
    pub ref_audio: Option<String>,
    pub ref_text: Option<String>,
    pub device: Option<String>,
}

impl Default for TtsProviderConfig {
    fn default() -> Self {
        Self {
            voice: None,
            model: None,
            model_id: None,
            voice_id: None,
            language: None,
            sample_rate: None,
            bit_rate: None,
            ref_audio: None,
            ref_text: None,
            device: None,
        }
    }
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
    pub max_text_length: Option<u32>,
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
            sample_rate: Some(24000),
            bit_rate: Some(128000),
            ..Default::default()
        };
        let mistral = TtsProviderConfig {
            model: Some("voxtral-mini-tts-2603".to_string()),
            voice_id: Some("c69964a6-ab8b-4f8a-9465-ec0925096ec8".to_string()),
            ..Default::default()
        };
        let neutts = TtsProviderConfig {
            ref_audio: Some(String::new()),
            ref_text: Some(String::new()),
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
            max_text_length: None,
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
pub struct VoiceConfig {
    pub record_key: Option<String>,
    pub max_recording_seconds: Option<u32>,
    pub auto_tts: Option<bool>,
    pub beep_enabled: Option<bool>,
    pub silence_threshold: Option<u32>,
    pub silence_duration: Option<f64>,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            record_key: Some("ctrl+b".to_string()),
            max_recording_seconds: Some(120),
            auto_tts: Some(false),
            beep_enabled: Some(true),
            silence_threshold: Some(200),
            silence_duration: Some(3.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfigV2 {
    pub memory_enabled: Option<bool>,
    pub user_profile_enabled: Option<bool>,
    pub memory_char_limit: Option<u32>,
    pub user_char_limit: Option<u32>,
    pub provider: Option<String>,
}

impl Default for MemoryConfigV2 {
    fn default() -> Self {
        Self {
            memory_enabled: Some(true),
            user_profile_enabled: Some(true),
            memory_char_limit: Some(2200),
            user_char_limit: Some(1375),
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
    pub inherit_mcp_toolsets: Option<bool>,
    pub max_iterations: Option<u32>,
    pub child_timeout_seconds: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub max_concurrent_children: Option<u32>,
    pub max_spawn_depth: Option<u32>,
    pub orchestrator_enabled: Option<bool>,
    pub subagent_auto_approve: Option<bool>,
}

impl Default for DelegationConfig {
    fn default() -> Self {
        Self {
            model: Some(String::new()),
            provider: Some(String::new()),
            base_url: Some(String::new()),
            api_key: Some(String::new()),
            inherit_mcp_toolsets: Some(true),
            max_iterations: Some(90),
            child_timeout_seconds: Some(600),
            reasoning_effort: Some(String::new()),
            max_concurrent_children: Some(3),
            max_spawn_depth: Some(2),
            orchestrator_enabled: Some(true),
            subagent_auto_approve: Some(false),
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
    pub external_dirs: Vec<PathBuf>,
    pub template_vars: Option<bool>,
    pub inline_shell: Option<bool>,
    pub inline_shell_timeout: Option<u64>,
    pub guard_agent_created: Option<bool>,
}

impl Default for SkillsConfigV2 {
    fn default() -> Self {
        Self {
            external_dirs: Vec::new(),
            template_vars: Some(true),
            inline_shell: Some(false),
            inline_shell_timeout: Some(10),
            guard_agent_created: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CuratorConfig {
    pub enabled: Option<bool>,
    pub interval_hours: Option<u64>,
    pub min_idle_hours: Option<u64>,
    pub stale_after_days: Option<u64>,
    pub archive_after_days: Option<u64>,
    pub backup: CuratorBackupConfig,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            enabled: Some(true),
            interval_hours: Some(168), // 24 * 7
            min_idle_hours: Some(2),
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
    pub cron_mode: Option<String>,
    pub mcp_reload_confirm: Option<bool>,
}

impl Default for ApprovalsConfig {
    fn default() -> Self {
        Self {
            mode: Some("manual".to_string()),
            timeout: Some(60),
            cron_mode: Some("deny".to_string()),
            mcp_reload_confirm: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    pub allow_private_urls: Option<bool>,
    pub redact_secrets: Option<bool>,
    pub tirith_enabled: Option<bool>,
    pub tirith_path: Option<String>,
    pub tirith_timeout: Option<u64>,
    pub tirith_fail_open: Option<bool>,
    pub website_blocklist: WebsiteBlocklistConfig,
    pub command_allowlist: Vec<String>,
    pub quick_commands: HashMap<String, String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allow_private_urls: Some(false),
            redact_secrets: Some(true),
            tirith_enabled: Some(true),
            tirith_path: Some("tirith".to_string()),
            tirith_timeout: Some(5),
            tirith_fail_open: Some(true),
            website_blocklist: WebsiteBlocklistConfig::default(),
            command_allowlist: Vec::new(),
            quick_commands: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebsiteBlocklistConfig {
    pub enabled: Option<bool>,
    pub domains: Vec<String>,
    pub shared_files: Vec<PathBuf>,
}

impl Default for WebsiteBlocklistConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            domains: Vec::new(),
            shared_files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CronConfigV2 {
    pub wrap_response: Option<bool>,
    pub max_parallel_jobs: Option<u32>,
}

impl Default for CronConfigV2 {
    fn default() -> Self {
        Self {
            wrap_response: Some(true),
            max_parallel_jobs: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KanbanConfig {
    pub dispatch_in_gateway: Option<bool>,
    pub dispatch_interval_seconds: Option<u64>,
    pub failure_limit: Option<u32>,
}

impl Default for KanbanConfig {
    fn default() -> Self {
        Self {
            dispatch_in_gateway: Some(true),
            dispatch_interval_seconds: Some(60),
            failure_limit: Some(2),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfigV2 {
    pub level: Option<String>,
    pub max_size_mb: Option<u32>,
    pub backup_count: Option<u32>,
}

impl Default for LoggingConfigV2 {
    fn default() -> Self {
        Self {
            level: Some("INFO".to_string()),
            max_size_mb: Some(5),
            backup_count: Some(3),
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
    pub auto_prune: Option<bool>,
    pub retention_days: Option<u32>,
    pub vacuum_after_prune: Option<bool>,
    pub min_interval_hours: Option<u32>,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            auto_prune: Some(false),
            retention_days: Some(90),
            vacuum_after_prune: Some(true),
            min_interval_hours: Some(24),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatesConfig {
    pub pre_update_backup: Option<bool>,
    pub backup_keep: Option<u32>,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            pre_update_backup: Some(false),
            backup_keep: Some(5),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub force_ipv4: Option<bool>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            force_ipv4: Some(false),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolOutputLimitsConfig {
    pub max_bytes: Option<u32>,
    pub max_lines: Option<u32>,
    pub max_line_length: Option<u32>,
}

impl Default for ToolOutputLimitsConfig {
    fn default() -> Self {
        Self {
            max_bytes: Some(50_000),
            max_lines: Some(2000),
            max_line_length: Some(2000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolLoopGuardrailsConfig {
    pub warnings_enabled: Option<bool>,
    pub hard_stop_enabled: Option<bool>,
    pub warn_after: ToolLoopThresholds,
    pub hard_stop_after: ToolLoopThresholds,
}

impl Default for ToolLoopGuardrailsConfig {
    fn default() -> Self {
        Self {
            warnings_enabled: Some(true),
            hard_stop_enabled: Some(false),
            warn_after: ToolLoopThresholds {
                exact_failure: Some(2),
                same_tool_failure: Some(3),
                idempotent_no_progress: Some(2),
            },
            hard_stop_after: ToolLoopThresholds {
                exact_failure: Some(5),
                same_tool_failure: Some(8),
                idempotent_no_progress: Some(5),
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolLoopThresholds {
    pub exact_failure: Option<u32>,
    pub same_tool_failure: Option<u32>,
    pub idempotent_no_progress: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfigV2 {
    pub tool_output: ToolOutputLimitsConfig,
    pub tool_loop_guardrails: ToolLoopGuardrailsConfig,
    pub file_read_max_chars: Option<u32>,
}

impl Default for ToolsConfigV2 {
    fn default() -> Self {
        Self {
            tool_output: ToolOutputLimitsConfig::default(),
            tool_loop_guardrails: ToolLoopGuardrailsConfig::default(),
            file_read_max_chars: Some(100_000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewaysConfig {
    pub telegram: GatewayPlatformConfig,
    pub discord: DiscordGatewayConfig,
    pub slack: SlackGatewayConfig,
    pub mattermost: MattermostGatewayConfig,
    pub matrix: MatrixGatewayConfig,
    pub whatsapp: HashMap<String, Value>,
    pub webhooks: HashMap<String, Value>,
}

impl Default for GatewaysConfig {
    fn default() -> Self {
        Self {
            telegram: GatewayPlatformConfig::default(),
            discord: DiscordGatewayConfig::default(),
            slack: SlackGatewayConfig::default(),
            mattermost: MattermostGatewayConfig::default(),
            matrix: MatrixGatewayConfig::default(),
            whatsapp: HashMap::new(),
            webhooks: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayPlatformConfig {
    pub enabled: Option<bool>,
    pub token: Option<String>,
    pub allowed_users: Option<String>,
    pub allowed_chats: Option<String>,
    pub channel_prompts: HashMap<String, String>,
    pub reactions: Option<bool>,
    pub proxy: Option<String>,
}

impl Default for GatewayPlatformConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            token: None,
            allowed_users: None,
            allowed_chats: None,
            channel_prompts: HashMap::new(),
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
    pub require_mention: Option<bool>,
    pub free_response_channels: Option<String>,
    pub allowed_channels: Option<String>,
    pub allowed_users: Option<String>,
    pub auto_thread: Option<bool>,
    pub reactions: Option<bool>,
    pub channel_prompts: HashMap<String, String>,
    pub dm_role_auth_guild: Option<String>,
    pub server_actions: Option<String>,
}

impl Default for DiscordGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            token: None,
            require_mention: Some(true),
            free_response_channels: Some(String::new()),
            allowed_channels: Some(String::new()),
            allowed_users: None,
            auto_thread: Some(true),
            reactions: Some(true),
            channel_prompts: HashMap::new(),
            dm_role_auth_guild: Some(String::new()),
            server_actions: Some(String::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SlackGatewayConfig {
    pub enabled: Option<bool>,
    pub bot_token: Option<String>,
    pub app_token: Option<String>,
    pub require_mention: Option<bool>,
    pub free_response_channels: Option<String>,
    pub allowed_channels: Option<String>,
    pub channel_prompts: HashMap<String, String>,
}

impl Default for SlackGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            bot_token: None,
            app_token: None,
            require_mention: Some(true),
            free_response_channels: Some(String::new()),
            allowed_channels: Some(String::new()),
            channel_prompts: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MattermostGatewayConfig {
    pub enabled: Option<bool>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub require_mention: Option<bool>,
    pub free_response_channels: Option<String>,
    pub allowed_channels: Option<String>,
    pub allowed_users: Option<String>,
    pub channel_prompts: HashMap<String, String>,
}

impl Default for MattermostGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: Some(false),
            url: None,
            token: None,
            require_mention: Some(true),
            free_response_channels: Some(String::new()),
            allowed_channels: Some(String::new()),
            allowed_users: None,
            channel_prompts: HashMap::new(),
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
    pub recovery_key: Option<String>,
    pub require_mention: Option<bool>,
    pub free_response_rooms: Option<String>,
    pub allowed_rooms: Option<String>,
    pub allowed_users: Option<String>,
    pub auto_thread: Option<bool>,
    pub dm_auto_thread: Option<bool>,
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
            recovery_key: None,
            require_mention: Some(true),
            free_response_rooms: Some(String::new()),
            allowed_rooms: Some(String::new()),
            allowed_users: None,
            auto_thread: Some(true),
            dm_auto_thread: Some(false),
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
pub struct DockerEnvConfig {
    pub image: Option<String>,
    pub network: Option<String>,
    pub extra_hosts: Vec<String>,
    pub volumes: Vec<String>,
    pub working_dir: Option<String>,
    pub env: HashMap<String, String>,
}

impl Default for DockerEnvConfig {
    fn default() -> Self {
        Self {
            image: None,
            network: None,
            extra_hosts: Vec::new(),
            volumes: Vec::new(),
            working_dir: None,
            env: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationsConfig {
    pub enabled: Vec<String>,
    pub configs: HashMap<String, HashMap<String, Value>>,
}

impl Default for IntegrationsConfig {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            configs: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub enabled: Vec<String>,
    pub configs: HashMap<String, HashMap<String, Value>>,
    pub dirs: Vec<PathBuf>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            configs: HashMap::new(),
            dirs: Vec::new(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    pub scripts: HashMap<String, Vec<HookScriptConfig>>,
    pub auto_accept: Option<bool>,
    pub personalities: HashMap<String, Value>,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            scripts: HashMap::new(),
            auto_accept: Some(false),
            personalities: HashMap::new(),
        }
    }
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
pub struct OnboardingConfig {
    pub seen: HashMap<String, bool>,
}

impl Default for OnboardingConfig {
    fn default() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }
}

/// Errors specific to CLI configuration loading.
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
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
    /// Load config from default locations, merging layers.
    ///
    /// Loading order:
    /// 1. Default config values in code
    /// 2. config.yaml from HERMES_HOME (or HERMES_CONFIG env var)
    /// 3. config.local.yaml from HERMES_HOME
    /// 4. .env file from HERMES_HOME
    /// 5. HERMES_* environment variable overrides
    pub fn load() -> ConfigResult<Self> {
        let default_self = Self::default();
        let config_dir = default_self.config_dir.clone();

        // Start with an empty YAML value and merge layers
        let mut merged = Value::Mapping(serde_yaml::Mapping::new());
        let mut config_version: Option<String> = None;

        // Layer 1: Try loading config.yaml
        let config_path = std::env::var("HERMES_CONFIG")
            .map(PathBuf::from)
            .ok()
            .unwrap_or_else(|| config_dir.join("config.yaml"));

        if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)?;
            if let Ok(value) = serde_yaml::from_str::<Value>(&raw) {
                // Check version for migration
                if let Some(ver) = value.get("config_version").and_then(|v| v.as_str()) {
                    config_version = Some(ver.to_string());
                }
                // Expand env vars in the loaded YAML
                let expanded = expand_env_vars_in_value(&value);
                deep_merge(&mut merged, &expanded);
            }
        }

        // Layer 2: Try loading config.local.yaml (local overrides)
        let local_config_path = config_dir.join("config.local.yaml");
        if local_config_path.exists() {
            let raw = std::fs::read_to_string(&local_config_path)?;
            if let Ok(value) = serde_yaml::from_str::<Value>(&raw) {
                let expanded = expand_env_vars_in_value(&value);
                deep_merge(&mut merged, &expanded);
            }
        }

        // Layer 3: Parse merged YAML into CliConfig
        let mut config: CliConfig =
            serde_yaml::from_value(merged).map_err(|e| ConfigError::Parse(e.to_string()))?;

        // Restore file paths (they come from defaults, not YAML)
        config.config_dir = config_dir;
        config.config_file = config_path;
        config.local_config_file = local_config_path;
        config.env_file = default_self.env_file;
        if let Some(ver) = config_version {
            config.config_version = Some(ver);
        }

        // Layer 4: Load .env file
        if config.env_file.exists() {
            load_dotenv_file(&config.env_file)?;
        }

        // Layer 5: Apply HERMES_* env var overrides
        config.apply_operant_env_overrides();

        Ok(config)
    }

    /// Apply HERMES_* environment variable overrides to the config.
    fn apply_operant_env_overrides(&mut self) {
        // HERMES_MODEL → agent default model
        if let Ok(val) = std::env::var("HERMES_MODEL") {
            if !val.is_empty() {
                self.api.default_model = Some(val);
            }
        }

        // HERMES_LOG_LEVEL
        if let Ok(val) = std::env::var("HERMES_LOG_LEVEL") {
            if !val.is_empty() {
                self.logging.level = Some(val);
            }
        }

        // HERMES_MAX_ITERATIONS
        if let Ok(val) = std::env::var("HERMES_MAX_ITERATIONS") {
            if let Ok(n) = val.parse::<u32>() {
                self.agent.max_turns = Some(n);
            }
        }

        // HERMES_TOOL_TIMEOUT
        if let Ok(val) = std::env::var("HERMES_TOOL_TIMEOUT") {
            if let Ok(n) = val.parse::<u64>() {
                self.terminal.timeout = Some(n);
            }
        }

        // HERMES_REQUEST_TIMEOUT
        if let Ok(val) = std::env::var("HERMES_REQUEST_TIMEOUT") {
            if let Ok(n) = val.parse::<u64>() {
                self.api.timeout_seconds = Some(n);
            }
        }

        // HERMES_CONTEXT_WINDOW
        if let Ok(val) = std::env::var("HERMES_CONTEXT_WINDOW") {
            if let Ok(n) = val.parse::<u32>() {
                self.api.max_tokens = Some(n);
            }
        }

        // HERMES_STREAM
        if let Ok(val) = std::env::var("HERMES_STREAM") {
            self.display.streaming = Some(matches!(
                val.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ));
        }

        // HERMES_SKILLS_DIR
        if let Ok(val) = std::env::var("HERMES_SKILLS_DIR") {
            if !val.is_empty() {
                self.operant.skills_dir = Some(PathBuf::from(val));
            }
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

/// Recursively expand env vars in all string values of a serde_yaml Value.
fn expand_env_vars_in_value(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(expand_env_vars(s)),
        Value::Mapping(map) => {
            let mut new_map = serde_yaml::Mapping::new();
            for (k, v) in map {
                let new_key = expand_env_vars_in_value(k);
                let new_val = expand_env_vars_in_value(v);
                new_map.insert(new_key, new_val);
            }
            Value::Mapping(new_map)
        }
        Value::Sequence(seq) => {
            let new_seq: Vec<Value> = seq.iter().map(|v| expand_env_vars_in_value(v)).collect();
            Value::Sequence(new_seq)
        }
        other => other.clone(),
    }
}

// =============================================================================
// Deep Merge
// =============================================================================

/// Deep-merge `overlay` into `base` recursively.
///
/// Rules:
/// - For objects (Mapping): merge keys recursively
/// - For arrays (Sequence): concatenate (overlay appended to base)
/// - For primitives: overlay wins
/// - Null values in overlay remove the key from base
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Mapping(base_map), Value::Mapping(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                if overlay_val.is_null() {
                    base_map.remove(key);
                    continue;
                }
                match base_map.get_mut(key) {
                    Some(base_val) => {
                        deep_merge(base_val, overlay_val);
                    }
                    None => {
                        base_map.insert(key.clone(), overlay_val.clone());
                    }
                }
            }
        }
        (Value::Sequence(base_seq), Value::Sequence(overlay_seq)) => {
            // Arrays: overlay items appended to base
            base_seq.extend(overlay_seq.clone());
        }
        (base_val, _) => {
            // Primitives or type mismatch: overlay wins
            *base_val = overlay.clone();
        }
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
                std::env::set_var(key, expanded);
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
        if let Some(show_reasoning) = self.display.show_reasoning {
            app.agent.show_reasoning = show_reasoning;
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

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env(name: &str, value: &str) -> Option<std::ffi::OsString> {
        let prev = std::env::var_os(name);
        std::env::set_var(name, value);
        prev
    }

    fn restore_env(name: &str, prev: Option<std::ffi::OsString>) {
        match prev {
            Some(val) => std::env::set_var(name, val),
            None => std::env::remove_var(name),
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
    // Deep Merge Tests
    // ================================================================

    #[test]
    fn test_deep_merge_primitives_overlay_wins() {
        let mut base = serde_yaml::from_str("key: old").unwrap();
        let overlay = serde_yaml::from_str("key: new").unwrap();
        deep_merge(&mut base, &overlay);
        assert_eq!(base.get("key").and_then(|v| v.as_str()), Some("new"));
    }

    #[test]
    fn test_deep_merge_nested_objects() {
        let mut base: Value = serde_yaml::from_str(
            r#"
            outer:
                inner: old
                keep: preserved
            "#,
        )
        .unwrap();
        let overlay: Value = serde_yaml::from_str(
            r#"
            outer:
                inner: new
            "#,
        )
        .unwrap();
        deep_merge(&mut base, &overlay);

        let outer = base.get("outer").and_then(|v| v.as_mapping()).unwrap();
        assert_eq!(
            outer
                .get(&Value::String("inner".into()))
                .and_then(|v| v.as_str()),
            Some("new")
        );
        assert_eq!(
            outer
                .get(&Value::String("keep".into()))
                .and_then(|v| v.as_str()),
            Some("preserved")
        );
    }

    #[test]
    fn test_deep_merge_arrays_concatenate() {
        let mut base: Value = serde_yaml::from_str("items: [1, 2]").unwrap();
        let overlay: Value = serde_yaml::from_str("items: [3, 4]").unwrap();
        deep_merge(&mut base, &overlay);
        let items: Vec<i64> = base["items"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        assert_eq!(items, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_deep_merge_null_removes_key() {
        let mut base: Value = serde_yaml::from_str("key: value").unwrap();
        let overlay: Value = serde_yaml::from_str("key: ~").unwrap();
        deep_merge(&mut base, &overlay);
        assert!(base
            .as_mapping()
            .unwrap()
            .get(&Value::String("key".into()))
            .is_none());
    }

    #[test]
    fn test_deep_merge_new_keys_added() {
        let mut base: Value = serde_yaml::from_str("existing: old").unwrap();
        let overlay: Value = serde_yaml::from_str("new_key: new_val").unwrap();
        deep_merge(&mut base, &overlay);
        assert_eq!(
            base.get("new_key").and_then(|v| v.as_str()),
            Some("new_val")
        );
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
        std::env::remove_var("KEY");
        std::env::remove_var("NUMBER");

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

        std::env::remove_var("DOUBLE");
        std::env::remove_var("SINGLE");

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

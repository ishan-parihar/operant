//! # Operant-RS Core Library
//!
//! A high-performance Rust implementation of the Operant-Agent orchestration loop.
//! Supports asynchronous tool execution, streaming-first architecture, and
//! dynamic JSON-schema generation.
//!
//! ## Key Features
//!
//! - **Streaming-First**: Detect and execute tool calls incrementally from partial LLM outputs
//! - **Tool System**: 17+ built-in tools for file ops, terminal, web, code execution, memory, and more
//! - **Self-Healing**: Re-prompt LLM with error context on tool execution failures
//! - **Context Compression**: Automatic compression of long conversations to fit context window
//! - **Memory System**: Persistent file-backed memory with MEMORY.md/USER.md storage
//! - **Trajectory Saving**: Export conversation trajectories for RL training
//! - **Multi-Platform Gateway**: Support for Telegram, Discord, Slack, and more
//! - **MCP Client**: Model Context Protocol client (HTTP + stdio) for extended capabilities
//! - **Skills System**: Skill discovery, loading, and management from SKILL.md directories
//! - **Cross-Platform**: Windows (PowerShell/cmd), macOS, Linux with automatic shell detection
//! - **Structured Logging**: Comprehensive observability via the `tracing` crate
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ OperantAgent │
//! │ ┌─────────────┐ ┌──────────────┐ ┌────────────────────┐ │
//! │ │ OpenAI │ │ XMLParser │ │ ToolRegistry │ │
//! │ │ Client │ │ (Tolerant) │ │ & 17+ Tools │ │
//! │ └─────────────┘ └──────────────┘ └────────────────────┘ │
//! │ ┌─────────────────────────────────────────────────────────┐│
//! │ │ Orchestration Loop (ReAct) ││
//! │ │ Think → Plan → Execute Tools → Observe → Respond ││
//! │ └─────────────────────────────────────────────────────────┘│
//! │ ┌───────────────┐ ┌──────────────┐ ┌────────────────────┐│
//! │ │ Context Mgr │ │ Memory Mgr │ │ Trajectory Mgr ││
//! │ └───────────────┘ └──────────────┘ └────────────────────┘│
//! └─────────────────────────────────────────────────────────────┘
//! │                     Gateway & MCP Support                  │
//! └─────────────────────────────────────────────────────────────┘
//! ```

pub mod accessibility;
pub mod acp;
pub mod agent;
pub mod ansi_strip;
pub mod approval;
pub mod browser_camofox;
pub mod browser_provider;
pub mod browser_supervisor;
pub mod budget_config;
pub mod client;
pub mod config;
pub mod context;
pub mod context_files;
pub mod context_management;
pub mod credential_files;
pub mod credential_pool;
pub mod cronjobs;
pub mod curator;
pub mod database;
pub mod distillation;
pub mod env_passthrough;
pub mod environments;
pub mod error;
pub mod error_classifier;
pub mod fuzzy_match;
pub mod gateway;
pub mod gateway_markdown;
pub mod gateway_pipeline;
pub mod gateway_session;
pub mod hooks;
pub mod interrupt;
pub mod kanban;
pub mod managed_tool_gateway;
pub mod mcp;
pub mod mcp_oauth;
pub mod memory;
pub mod memory_provider;
pub mod models_dev;
pub mod ms_graph;
pub mod nous_rate_guard;
pub mod oauth_refresh;
pub mod parser;
pub mod pii;
pub mod platform;
pub mod plugins;
pub mod process_registry;
pub mod profile;
pub mod rate_limit_tracker;
pub mod rate_limiter;
pub mod retry;
pub mod rl_training;
pub mod schema;
pub mod schema_sanitizer;
pub mod security;
pub mod skill_usage;
pub mod skills;
pub mod skills_guard;
pub mod skills_hub;
pub mod skills_sync;
pub mod tool_result_storage;
pub mod tools;
pub mod trajectory;
pub mod voice;
pub mod website_policy;
pub mod yuanbao;

pub use acp::{server, AcpHandler, AgentState, RpcRequest, RpcResponse};
pub use agent::{
    cache::{
        new_shared_cache, new_shared_cache_with_config, AgentCache, AgentCacheConfig,
        AgentCacheEntry, SharedAgentCache,
    },
    AgentConfig, AgentEvent, FallbackModelClient, OperantAgent,
};
pub use approval::{
    check_tool_approval, prompt_user_for_approval, ApprovalContext, ApprovalGuard, ApprovalMode,
    ApprovalVerdict, RiskLevel,
};
pub use browser_provider::{
    build_browser_provider, BrowserProvider, BrowserUseProvider, BrowserbaseProvider,
    CamofoxProvider, FirecrawlProvider, LightpandaProvider,
};
pub use browser_supervisor::{
    BrowserSession, BrowserSupervisorTool, CDPSupervisor, CdpNavigateTool, CloudProvider,
    CloudProviderClient, CloudProviderConfig, DialogBridgeTool, SessionStatus,
};
pub use client::{Message, OpenAIClient};
pub use config::{
    install_runtime_config, load_app_config, runtime_config, AppConfig, AutonomousSettings,
    BehaviorSettings, ClientSettings, CodeExecutionSettings, GatewaySettings, HttpToolSettings,
    LoadedConfig, LoggingSettings, McpServerConfig, McpSettings, MemorySettings, RateLimitSettings,
    SkillsSettings, SttSettings, TerminalSettings, ToolSettings, TuiSettings, WebToolSettings,
};
pub use context::{estimate_tokens, ContextConfig, ContextManager};
pub use context_files::{
    load_context_dir, load_default_context_files, load_workspace_context, scan_context_content,
};
pub use credential_pool::{AuthType, CredentialPool, PoolStrategy, PooledCredential};
pub use curator::{archiver, backup, review, CuratorEngine, CuratorReport, CuratorState};
pub use distillation::distill_session_to_memory;
pub use environments::{
    daytona::DaytonaEnvironment, docker::DockerEnvironment, local::LocalEnvironment,
    modal::ModalEnvironment, singularity::SingularityEnvironment, ssh::SshEnvironment,
    vercel::VercelSandboxEnvironment, Environment, EnvironmentConfig, EnvironmentPool,
    EnvironmentResult, EnvironmentType,
};
pub use error::{Error, Result};
pub use gateway::{
    format_startup_message, handle_admin_command, ChannelDirectory, ChannelInfo, ChannelType,
    Gateway, GatewayConfig, GatewayStats, IncomingMessage, OutgoingMessage, PlatformAdapter,
    PlatformRegistry, PlatformSession, SessionStore, TelegramAdapter, TelegramPoller, UserInfo,
    WebhookAdapter,
};
pub use gateway_session::{
    build_session_key, hash_chat_id, hash_sender_id, is_shared_multi_user_session,
    PersistentSessionStore, ResetMode, SessionEntry, SessionResetPolicy, SessionSource,
    SessionStoreConfig,
};
pub use hooks::{
    emit_hook, emit_hook_collect, global_hook_registry, register_hook, HookAction, HookEvent,
    HookHandler as LifecycleHookHandler, HookRegistry,
};
pub use managed_tool_gateway::{
    GatewayConfig as ManagedGatewayConfig, ManagedToolGateway, UrlPattern,
};
pub use mcp::{McpClient, McpNamespacedTool, McpStdioClient, McpTool, McpTransport};
pub use memory::{MemoryBlock, MemoryManager, Session, UserProfile};
pub use memory_provider::{build_memory_provider, BuiltinProvider, MemoryProvider, TdgMemoryProvider};
pub use ms_graph::{
    CachedAccessToken, GraphCredentials, MicrosoftGraphClient, MicrosoftGraphError,
    MicrosoftGraphTokenProvider,
};
pub use parser::ToolCallParser;
pub use platform::PlatformInfo;
pub use plugins::{
    discover_plugins, get_plugin_commands, handle_plugin_command, is_plugin_command,
    register_plugin_command, resolve_plugin_command, PluginCommand, PluginHandler, PluginManifest,
};
pub use process_registry::{ProcessRegistry, ProcessSession, ProcessStatus};
pub use profile::{
    clone_profile, create_profile, delete_profile, get_active_profile, get_operant_home,
    get_profile_dir, get_profiles_root, list_profiles, normalize_profile_name, profile_exists,
    set_active_profile, set_operant_home_override, use_profile, validate_profile_name,
    OperantHomeToken, ProfileInfo,
};
pub use rl_training::{
    check_rl_env_vars, check_tinker_atropos, list_available_environments, ActionValue,
    EpisodeResult, QStateEntry, QTable, RlState, RlTrainer, StepResult, TrainingSummary,
};
pub use skill_usage::{LifecycleState, SkillUsageTracker, UsageRecord, UsageTelemetry};
pub use skills::{Skill, SkillManager};
pub use skills_guard::{
    content_hash, format_scan_report, scan_skill, should_allow_install, GuardScanner, ScanResult,
    ScanVerdict, SecurityFinding, Severity, TrustLevel, Verdict,
};
pub use tools::{
    register_builtin_tools, register_builtin_tools_with_sub_agent, OperantTool, ToolRegistry,
    ToolResult,
};
pub use trajectory::{Trajectory, TrajectoryBuilder, TrajectoryExporter};

pub use error_classifier::{classify_api_error, ClassifiedError, FailoverReason};
pub use models_dev::{
    fetch_models_dev, get_model_capabilities, list_agentic_models, lookup_models_dev_context,
    provider_to_models_dev, ModelCapabilities,
};
pub use nous_rate_guard::{
    clear_nous_rate_limit, is_genuine_nous_rate_limit, nous_rate_limit_remaining,
    record_nous_rate_limit, NousRateLimitState,
};
pub use oauth_refresh::{
    auth_store_path, load_auth_store, save_auth_store, AuthStore, OAuthRefresher,
    OAuthTokenResponse, ProviderState,
};
pub use rate_limit_tracker::{RateLimitBucket, RateLimitState};
pub use rate_limiter::{
    exponential_backoff_secs, parse_retry_after_header, RateLimitError, RateLimitStatus,
    RateLimiter, TokenBucket,
};
pub use retry::{default_backoff, jittered_backoff};

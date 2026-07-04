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
pub mod aft_bridge;
pub mod agent;
pub mod approval;
pub mod browser_provider;
pub mod client;
pub mod config;
pub mod context_files;
pub mod context_management;
pub mod credential_pool;
pub mod cronjobs;
pub mod curator;
pub mod database;
pub mod distillation;
pub mod env_passthrough;
pub mod error;
pub mod gateway;
pub mod gateway_markdown;
pub mod gateway_pipeline;
pub use gateway_pipeline::{
    HookContext, HookEvent, HookRegistry,
    MessageFilter, MessagePipeline, PipelineAction,
};
pub mod gateway_session;
pub mod interrupt;
pub mod kanban;
pub mod mcp;
pub mod mcp_oauth;
pub mod memory;
pub mod memory_provider;
pub mod models_dev;
pub mod oauth_refresh;
pub mod parser;
pub mod pii;
pub mod platform;
pub mod plugins;
pub mod process_registry;
pub mod profile;
pub mod rate_limiter;
pub mod schema;
pub mod security;
pub mod skill_usage;
pub mod skills;
pub mod skills_guard;
pub mod tools;
pub mod trajectory;
pub mod voice;

pub use acp::{server, AcpHandler, AgentState, RpcRequest, RpcResponse};
pub use agent::{
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
pub use client::{Message, OpenAIClient};
pub use config::{
    install_runtime_config, load_app_config, runtime_config, AppConfig, AutonomousSettings,
    BehaviorSettings, ClientSettings, CodeExecutionSettings, GatewaySettings, HttpToolSettings,
    LoadedConfig, LoggingSettings, McpServerConfig, McpSettings, MemorySettings, RateLimitSettings,
    SkillsSettings, SttSettings, TerminalSettings, ToolSettings, TuiSettings, WebToolSettings,
};
pub use context_files::{
    load_context_dir, load_default_context_files, load_workspace_context, scan_context_content,
};
pub use credential_pool::{AuthType, CredentialPool, PooledCredential, PoolStrategy};
pub use curator::{archiver, backup, review, CuratorEngine, CuratorReport, CuratorState};
pub use distillation::distill_session_to_memory;
pub use error::{Error, Result};
pub use gateway::{
    format_startup_message, handle_admin_command, ChannelDirectory, ChannelInfo, ChannelType,
    Gateway, GatewayConfig, GatewayStats, IncomingMessage, OutgoingMessage, PlatformAdapter,
    PlatformSession, SessionStore, TelegramAdapter, TelegramPoller, UserInfo,
    WebhookAdapter, WhatsAppAdapter, EmailAdapter, SmsAdapter,
};
pub use gateway_session::{
    build_session_key, hash_chat_id, hash_sender_id, is_shared_multi_user_session,
    PersistentSessionStore, ResetMode, SessionEntry, SessionResetPolicy, SessionSource,
    SessionStoreConfig,
};
pub use mcp::{McpClient, McpNamespacedTool, McpStdioClient, McpTool, McpTransport};
pub use memory::{MemoryBlock, MemoryManager, Session, UserProfile};
pub use memory_provider::{build_memory_provider, BuiltinProvider, MemoryProvider, TdgMemoryProvider};
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

pub use models_dev::{
    fetch_models_dev, get_model_capabilities, list_agentic_models, lookup_models_dev_context,
    provider_to_models_dev, ModelCapabilities,
};
pub use oauth_refresh::{
    auth_store_path, load_auth_store, save_auth_store, AuthStore, OAuthRefresher,
    OAuthTokenResponse, ProviderState,
};
pub use rate_limiter::{
    exponential_backoff_secs, parse_retry_after_header, RateLimitError, RateLimitStatus,
    RateLimiter, TokenBucket,
};

//! # Hermes-RS Core Library
//!
//! A high-performance Rust implementation of the Hermes-Agent orchestration loop.
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
//! │ HermesAgent │
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

pub mod acp;
pub mod agent;
pub mod ansi_strip;
pub mod approval;
pub mod browser_camofox;
pub mod browser_supervisor;
pub mod budget_config;
pub mod client;
pub mod config;
pub mod context;
pub mod context_files;
pub mod credential_files;
pub mod credential_pool;
pub mod cronjobs;
pub mod curator;
pub mod database;
pub mod distillation;
pub mod env_passthrough;
pub mod environments;
pub mod error;
pub mod fuzzy_match;
pub mod gateway;
pub mod gateway_hooks;
pub mod gateway_markdown;
pub mod gateway_pipeline;
pub mod gateway_session;
pub mod interrupt;
pub mod kanban;
pub mod managed_tool_gateway;
pub mod mcp;
pub mod mcp_oauth;
pub mod memory;
pub mod ms_graph;
pub mod parser;
pub mod platform;
pub mod process_registry;
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
pub use agent::{AgentConfig, AgentEvent, HermesAgent};
pub use approval::{
    check_tool_approval, prompt_user_for_approval, ApprovalContext, ApprovalGuard, ApprovalMode,
    ApprovalVerdict, RiskLevel,
};
pub use browser_camofox::CamofoxBrowser;
pub use browser_supervisor::{
    BrowserSession, BrowserSupervisorTool, CDPSupervisor, CdpNavigateTool, CloudProvider,
    CloudProviderClient, CloudProviderConfig, DialogBridgeTool, SessionStatus,
};
pub use client::{Message, OpenAIClient};
pub use config::{
    install_runtime_config, load_app_config, runtime_config, AppConfig, AutonomousSettings,
    BehaviorSettings, ClientSettings, CodeExecutionSettings, GatewaySettings, HttpToolSettings,
    LoadedConfig, LoggingSettings, McpServerConfig, McpSettings, SkillsSettings, SttSettings,
    TerminalSettings, ToolSettings, TuiSettings, WebToolSettings,
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
pub use gateway_session::PersistentSessionStore;
pub use managed_tool_gateway::{
    GatewayConfig as ManagedGatewayConfig, ManagedToolGateway, UrlPattern,
};
pub use mcp::{McpClient, McpNamespacedTool, McpStdioClient, McpTool, McpTransport};
pub use memory::{MemoryBlock, MemoryManager, Session, UserProfile};
pub use ms_graph::{
    CachedAccessToken, GraphCredentials, MicrosoftGraphClient, MicrosoftGraphError,
    MicrosoftGraphTokenProvider,
};
pub use parser::ToolCallParser;
pub use platform::PlatformInfo;
pub use process_registry::{ProcessRegistry, ProcessSession, ProcessStatus};
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
    register_builtin_tools, register_builtin_tools_with_sub_agent, HermesTool, ToolRegistry,
    ToolResult,
};
pub use trajectory::{Trajectory, TrajectoryBuilder, TrajectoryExporter};

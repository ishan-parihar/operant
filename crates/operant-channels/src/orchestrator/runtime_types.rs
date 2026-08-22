//! `runtime_types` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use operant_memory::{self, Memory};
use operant_providers::{self, ChatMessage, Provider};
use operant_runtime::approval::ApprovalManager;
use operant_runtime::observability::Observer;
use operant_runtime::security::AutonomyLevel;
use operant_runtime::tools::Tool;
use portable_atomic::Ordering;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;

use super::*;

/// Per-sender conversation history for channel messages.
/// Bounded by `MAX_CONVERSATION_SENDERS` — oldest-accessed senders are evicted.
pub(crate) type ConversationHistoryMap = Arc<Mutex<lru::LruCache<String, Vec<ChatMessage>>>>;
/// Senders that requested `/new` and must force a fresh prompt on their next message.
pub(crate) type PendingNewSessionSet = Arc<Mutex<HashSet<String>>>;

pub(crate) type ProviderCacheMap = Arc<Mutex<HashMap<String, Arc<dyn Provider>>>>;
pub(crate) type RouteSelectionMap = Arc<Mutex<HashMap<String, ChannelRouteSelection>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelRouteSelection {
    pub(crate) provider: String,
    pub(crate) model: String,
    /// Route-specific API key override. When set, this takes precedence over
    /// the global `api_key` in [`ChannelRuntimeContext`] when creating the
    /// provider for this route.
    pub(crate) api_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelRuntimeCommand {
    ShowProviders,
    SetProvider(String),
    ShowModel,
    SetModel(String),
    ShowConfig,
    NewSession,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ModelCacheState {
    pub(crate) entries: Vec<ModelCacheEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ModelCacheEntry {
    pub(crate) provider: String,
    pub(crate) models: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelRuntimeDefaults {
    pub(crate) default_provider: String,
    pub(crate) model: String,
    pub(crate) temperature: f64,
    pub(crate) api_key: Option<String>,
    pub(crate) api_url: Option<String>,
    pub(crate) reliability: operant_config::schema::ReliabilityConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigFileStamp {
    pub(crate) modified: SystemTime,
    pub(crate) len: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfigState {
    pub(crate) defaults: ChannelRuntimeDefaults,
    pub(crate) last_applied_stamp: Option<ConfigFileStamp>,
}

pub(crate) fn runtime_config_store() -> &'static Mutex<HashMap<PathBuf, RuntimeConfigState>> {
    static STORE: OnceLock<Mutex<HashMap<PathBuf, RuntimeConfigState>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct InterruptOnNewMessageConfig {
    pub(crate) telegram: bool,
    pub(crate) slack: bool,
    pub(crate) discord: bool,
    pub(crate) mattermost: bool,
    pub(crate) matrix: bool,
}

impl InterruptOnNewMessageConfig {
    pub(crate) fn enabled_for_channel(self, channel: &str) -> bool {
        match channel {
            "telegram" => self.telegram,
            "slack" => self.slack,
            "discord" => self.discord,
            "mattermost" => self.mattermost,
            "matrix" => self.matrix,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChannelCostTrackingState {
    pub(crate) tracker: Arc<operant_runtime::cost::CostTracker>,
    pub(crate) prices: Arc<HashMap<String, operant_config::schema::ModelPricing>>,
}

#[derive(Clone)]
pub(crate) struct ChannelRuntimeContext {
    pub(crate) channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>,
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) default_provider: Arc<String>,
    pub(crate) prompt_config: Arc<operant_config::schema::Config>,
    pub(crate) memory: Arc<dyn Memory>,
    pub(crate) tools_registry: Arc<Vec<Box<dyn Tool>>>,
    pub(crate) observer: Arc<dyn Observer>,
    pub(crate) system_prompt: Arc<String>,
    pub(crate) model: Arc<String>,
    pub(crate) temperature: f64,
    pub(crate) auto_save_memory: bool,
    pub(crate) max_tool_iterations: usize,
    pub(crate) min_relevance_score: f64,
    pub(crate) conversation_histories: ConversationHistoryMap,
    pub(crate) pending_new_sessions: PendingNewSessionSet,
    pub(crate) provider_cache: ProviderCacheMap,
    pub(crate) route_overrides: RouteSelectionMap,
    pub(crate) api_key: Option<String>,
    pub(crate) api_url: Option<String>,
    pub(crate) reliability: Arc<operant_config::schema::ReliabilityConfig>,
    pub(crate) provider_runtime_options: operant_providers::ProviderRuntimeOptions,
    pub(crate) workspace_dir: Arc<PathBuf>,
    pub(crate) message_timeout_secs: u64,
    pub(crate) interrupt_on_new_message: InterruptOnNewMessageConfig,
    pub(crate) multimodal: operant_config::schema::MultimodalConfig,
    pub(crate) media_pipeline: operant_config::schema::MediaPipelineConfig,
    pub(crate) transcription_config: operant_config::schema::TranscriptionConfig,
    pub(crate) hooks: Option<Arc<operant_runtime::hooks::HookRunner>>,
    pub(crate) non_cli_excluded_tools: Arc<Vec<String>>,
    pub(crate) autonomy_level: AutonomyLevel,
    pub(crate) tool_call_dedup_exempt: Arc<Vec<String>>,
    pub(crate) model_routes: Arc<Vec<operant_config::schema::ModelRouteConfig>>,
    pub(crate) query_classification: operant_config::schema::QueryClassificationConfig,
    pub(crate) ack_reactions: bool,
    pub(crate) show_tool_calls: bool,
    pub(crate) session_store: Option<Arc<dyn operant_infra::session_backend::SessionBackend>>,
    /// Non-interactive approval manager for channel-driven runs.
    /// Enforces `auto_approve` / `always_ask` / supervised policy from
    /// `[autonomy]` config; auto-denies tools that would need interactive
    /// approval since no operator is present on channel runs.
    pub(crate) approval_manager: Arc<ApprovalManager>,
    pub(crate) activated_tools:
        Option<std::sync::Arc<std::sync::Mutex<operant_runtime::tools::ActivatedToolSet>>>,
    pub(crate) cost_tracking: Option<ChannelCostTrackingState>,
    pub(crate) pacing: operant_config::schema::PacingConfig,
    pub(crate) max_tool_result_chars: usize,
    pub(crate) context_token_budget: usize,
    pub(crate) debouncer: Arc<operant_infra::debounce::MessageDebouncer>,
    /// HMAC receipt generator. `Some` when `[agent.tool_receipts] enabled = true`.
    /// Threaded into `run_tool_call_loop` so `tool_execution::execute_one_tool`
    /// can sign each result.
    pub(crate) receipt_generator: Option<operant_runtime::agent::tool_receipts::ReceiptGenerator>,
    /// Mirror of `[agent.tool_receipts] show_in_response`. When true,
    /// `process_channel_message` renders the per-turn collector as a trailing
    /// `Tool receipts:` block sent after the main reply.
    pub(crate) show_receipts_in_response: bool,
}

#[derive(Clone)]
pub(crate) struct InFlightSenderTaskState {
    pub(crate) task_id: u64,
    pub(crate) cancellation: CancellationToken,
    pub(crate) completion: Arc<InFlightTaskCompletion>,
}

pub(crate) struct InFlightTaskCompletion {
    pub(crate) done: AtomicBool,
    pub(crate) notify: tokio::sync::Notify,
}

impl InFlightTaskCompletion {
    pub(crate) fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn mark_done(&self) {
        self.done.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    pub(crate) async fn wait(&self) {
        if self.done.load(Ordering::Acquire) {
            return;
        }
        self.notify.notified().await;
    }
}

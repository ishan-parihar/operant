/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
// Split modules (dedup pass) — re-exported so `loop_::` paths hold.
pub mod context;
pub mod messages;
pub mod run;
pub mod streaming;
pub mod tool_loop;
pub mod turn;
pub use context::*;
pub use messages::*;
pub use run::*;
pub use streaming::*;
pub use tool_loop::*;
pub use turn::*;

pub static CLI_CHANNEL_FN: std::sync::OnceLock<
    Box<dyn Fn() -> Box<dyn operant_api::channel::Channel> + Send + Sync>,
> = std::sync::OnceLock::new();

/// Register the CLI channel factory. Called once at startup by the binary.
pub fn register_cli_channel_fn(
    f: Box<dyn Fn() -> Box<dyn operant_api::channel::Channel> + Send + Sync>,
) {
    let _ = CLI_CHANNEL_FN.set(f);
}

/// Peripheral tools factory type — takes owned config so the returned future is 'static.
pub type PeripheralToolsFn = Box<
    dyn Fn(
            operant_config::schema::PeripheralsConfig,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<Vec<Box<dyn Tool>>>> + Send>,
        > + Send
        + Sync,
>;

/// Peripheral tools factory, injected by the binary when hardware feature is on.
static PERIPHERAL_TOOLS_FN: std::sync::OnceLock<PeripheralToolsFn> = std::sync::OnceLock::new();

/// Register the peripheral tools factory. Called once at startup by the binary.
pub fn register_peripheral_tools_fn(f: PeripheralToolsFn) {
    let _ = PERIPHERAL_TOOLS_FN.set(f);
}
use crate::cost::types::BudgetCheck;
use crate::observability::{self, Observer, ObserverEvent, runtime_trace};
use crate::platform;
use crate::security::{AutonomyLevel, SecurityPolicy};
use crate::tools::{self, Tool};
use crate::util::truncate_with_ellipsis;
use anyhow::Result;
use futures_util::StreamExt;
use operant_api::channel::Channel;
use operant_api::provider::StreamEvent;
use operant_config::schema::Config;
use operant_memory::{
    self, MEMORY_CONTEXT_CLOSE, MEMORY_CONTEXT_OPEN, Memory, MemoryCategory, decay,
};
use operant_providers::multimodal;
use operant_providers::{
    self, ChatMessage, ChatRequest, Provider, ProviderCapabilityError, ToolCall,
};
use std::collections::HashSet;
use std::fmt::Write;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// Cost tracking moved to `super::cost`.
pub use super::cost::{
    TOOL_LOOP_COST_TRACKING_CONTEXT, ToolLoopCostTrackingContext, TurnUsage,
    check_tool_loop_budget, record_tool_loop_cost_usage,
};

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
const STREAM_CHUNK_MIN_CHARS: usize = 80;
/// Rolling window size for detecting streamed tool-call payload markers.
const STREAM_TOOL_MARKER_WINDOW_CHARS: usize = 512;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// How many times to retry an empty assistant response (no text, no reasoning,
/// no tool calls) before giving up and returning it. Mirrors OperantAgent's
/// R4 `empty_content_retries` ladder and hermes's conversation_loop.py.
const EMPTY_RESPONSE_MAX_RETRIES: usize = 3;

// History management moved to `super::history`.
pub use super::history::{
    append_or_merge_system_message, canonicalize_tool_result_media_markers, emergency_history_trim,
    estimate_history_tokens, fast_trim_tool_results, load_interactive_session_history,
    normalize_system_messages, save_interactive_session_history, trim_history,
    truncate_tool_result,
};

/// Minimum user-message length (in chars) for auto-save to memory.
/// Matches the channel-side constant in `channels/mod.rs`.
const AUTOSAVE_MIN_MESSAGE_CHARS: usize = 20;

/// Callback type for checking if model has been switched during tool execution.
/// Returns Some((provider, model)) if a switch was requested, None otherwise.
pub type ModelSwitchCallback = Arc<Mutex<Option<(String, String)>>>;

/// Global model switch request state - used for runtime model switching via model_switch tool.
/// This is set by the model_switch tool and checked by the agent loop.
#[allow(clippy::type_complexity)]
static MODEL_SWITCH_REQUEST: LazyLock<Arc<Mutex<Option<(String, String)>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(None)));

/// Get the global model switch request state
pub fn get_model_switch_state() -> ModelSwitchCallback {
    Arc::clone(&MODEL_SWITCH_REQUEST)
}

/// Clear any pending model switch request
pub fn clear_model_switch_request() {
    if let Ok(guard) = MODEL_SWITCH_REQUEST.lock() {
        let mut guard = guard;
        *guard = None;
    }
}

// Re-export from operant-types for backwards compatibility.
pub use operant_api::TOOL_LOOP_SESSION_KEY;
pub use operant_api::TOOL_LOOP_THREAD_ID;

// Stateless support layer extracted to loop_support (dedup pass);
// re-exported here so `loop_::` import paths keep working.
pub(crate) use super::loop_support::compute_excluded_mcp_tools;
pub use super::loop_support::{
    build_tool_instructions, build_tool_instructions_for_names, filter_by_allowed_tools,
    filter_tool_specs_for_turn, scope_session_key, scope_thread_id, scrub_credentials,
};

pub fn is_tool_loop_cancelled(err: &anyhow::Error) -> bool {
    err.chain().any(|source| source.is::<ToolLoopCancelled>())
}

#[derive(Debug)]
pub struct ModelSwitchRequested {
    pub provider: String,
    pub model: String,
}

impl std::fmt::Display for ModelSwitchRequested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "model switch requested to {} {}",
            self.provider, self.model
        )
    }
}

impl std::error::Error for ModelSwitchRequested {}

pub fn is_model_switch_requested(err: &anyhow::Error) -> Option<(String, String)> {
    err.chain()
        .filter_map(|source| source.downcast_ref::<ModelSwitchRequested>())
        .map(|e| (e.provider.clone(), e.model.clone()))
        .next()
}

#[derive(Debug, Default)]
pub struct StreamedChatOutcome {
    response_text: String,
    /// Accumulated reasoning/thinking content from streaming deltas.
    ///
    /// Captured separately from `response_text` so it can be threaded into
    /// `ChatResponse.reasoning_content` and ultimately persisted on the
    /// `AssistantToolCalls` history entry. Required for providers like
    /// DeepSeek V4 that reject follow-up requests when the assistant's
    /// prior `reasoning_content` is missing from replayed tool-call turns
    /// (see issue #6059).
    reasoning_content: String,
    tool_calls: Vec<ToolCall>,
    forwarded_live_deltas: bool,
    usage: Option<operant_providers::traits::TokenUsage>,
}

/// Optional overrides for the agent `run` entry point.
///
/// Groups the 8 customization parameters that were previously passed
/// individually, reducing `run`'s argument count from 10 to 3.
#[derive(Clone, Default)]
pub struct RunOverrides {
    /// Override the LLM provider (e.g. "openrouter", "anthropic").
    pub provider_override: Option<String>,
    /// Override the model name (e.g. "anthropic/claude-sonnet-4").
    pub model_override: Option<String>,
    /// Sampling temperature (0.0–2.0).  Defaults to 0.7.
    pub temperature: f64,
    /// Extra peripheral tool names to register (hardware, robot-kit).
    pub peripheral_overrides: Vec<String>,
    /// Whether to enter interactive REPL mode after the first response.
    pub interactive: bool,
    /// Optional path for persisting session state across restarts.
    pub session_state_file: Option<PathBuf>,
    /// Whitelist of tool names the agent may call (None = all allowed).
    pub allowed_tools: Option<Vec<String>>,
    /// Optional observer for recording agent events (metrics, tracing).
    pub observer: Option<Arc<dyn Observer>>,
}

impl std::fmt::Debug for RunOverrides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunOverrides")
            .field("provider_override", &self.provider_override)
            .field("model_override", &self.model_override)
            .field("temperature", &self.temperature)
            .field("peripheral_overrides", &self.peripheral_overrides)
            .field("interactive", &self.interactive)
            .field("session_state_file", &self.session_state_file)
            .field("allowed_tools", &self.allowed_tools)
            .field(
                "observer",
                &self.observer.as_ref().map(|_| "<dyn Observer>"),
            )
            .finish()
    }
}

// ── CLI Entrypoint ───────────────────────────────────────────────────────
// Wires up all subsystems (observer, runtime, security, memory, tools,
// provider, hardware RAG, peripherals) and enters either single-shot or
// interactive REPL mode. The interactive loop manages history compaction
// and hard trimming to keep the context window bounded.

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
#[allow(clippy::too_many_lines)]
#[cfg(test)]
#[cfg(test)]
mod tests;

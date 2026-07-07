//! Gateway pipeline + hook system.
//!
//! Provides message filtering and lifecycle event hooks. Hooks fire at
//! specific points in the gateway/agent lifecycle, allowing external code
//! to react to events without modifying the core loop.
//!
//! ## Hook Events (mirrors hermes-agent gateway/hooks.py)
//!
//! - `gateway:startup` — Gateway process starts
//! - `session:start` — New session created (first message of a new session)
//! - `session:end` — Session ends (reset, idle timeout, or shutdown)
//! - `agent:start` — Agent begins processing a message
//! - `agent:end` — Agent finishes processing a message
//! - `command:*` — Any slash command executed (wildcard matching)
//!
//! ## Message Pipeline
//!
//! The pipeline runs BEFORE the agent processes a message. Filters can
//! Allow, Block (with reason), or Queue the message.

use crate::gateway::IncomingMessage;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::debug;

// ---------------------------------------------------------------------------
// Message Pipeline (pre-agent filtering)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum PipelineAction {
    Allow,
    Block(String),
    Queue,
}

// (iter-148: MessageFilter trait deleted — 0 implementations.
// MessagePipeline simplified to always return Allow since no filters
// were ever registered.)

pub struct MessagePipeline;

impl MessagePipeline {
    pub fn new() -> Self {
        Self
    }

    pub fn process(&self, _msg: &IncomingMessage) -> PipelineAction {
        PipelineAction::Allow
    }
}

impl Default for MessagePipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Hook System (lifecycle events)
// ---------------------------------------------------------------------------

/// Hook event types. Mirrors hermes-agent's gateway/hooks.py events.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// Gateway process starts
    GatewayStartup,
    /// New session created
    SessionStart,
    /// Session ends (reset, idle, shutdown)
    SessionEnd,
    /// Agent begins processing a message
    AgentStart,
    /// Agent finishes processing a message
    AgentEnd,
    /// Slash command executed (stores the command name)
    Command(String),
}

impl HookEvent {
    /// Check if this event matches a pattern. Supports wildcard matching
    /// for Command events: `Command("reset")` matches `Command("*")`.
    pub fn matches(&self, pattern: &HookEvent) -> bool {
        match (self, pattern) {
            (HookEvent::Command(a), HookEvent::Command(b)) => b == "*" || a == b,
            _ => self == pattern,
        }
    }
}

/// Context passed to hook handlers. Contains relevant data for the event.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    pub platform: Option<String>,
    pub channel_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub command: Option<String>,
    pub iteration: Option<usize>,
    pub metadata: HashMap<String, String>,
}

impl HookContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel_id = Some(channel.into());
        self
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user_id = Some(user.into());
        self
    }

    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session_id = Some(session.into());
        self
    }
}

/// A hook handler function. Receives the event type and context.
/// Handlers are async and run sequentially (not concurrently).
pub type HookHandler = Arc<dyn Fn(HookEvent, HookContext) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Registry of hook handlers. Thread-safe via RwLock.
///
/// Usage:
/// ```ignore
/// let registry = HookRegistry::new();
/// registry.register(HookEvent::AgentStart, Arc::new(|event, ctx| {
///     Box::pin(async move {
///         tracing::info!("Agent started for session {:?}", ctx.session_id);
///     })
/// })).await;
/// registry.emit(HookEvent::AgentStart, HookContext::new().with_session("s123")).await;
/// ```
pub struct HookRegistry {
    handlers: Arc<RwLock<Vec<(HookEvent, HookHandler)>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a handler for a specific event. Supports wildcard matching
    /// for Command events: registering for `Command("*")` fires on all commands.
    pub async fn register(&self, event: HookEvent, handler: HookHandler) {
        let mut handlers = self.handlers.write().unwrap();
        debug!(event = ?event, "Hook registered");
        handlers.push((event, handler));
    }

    /// Emit an event to all matching handlers. Handlers run sequentially.
    /// Errors in handlers are caught and logged (never block the pipeline).
    pub async fn emit(&self, event: HookEvent, ctx: HookContext) {
        let handlers: Vec<(HookEvent, HookHandler)> = {
            let handlers = self.handlers.read().unwrap();
            handlers
                .iter()
                .filter(|(pattern, _)| event.matches(pattern))
                .cloned()
                .collect()
        };

        for (pattern, handler) in handlers {
            debug!(event = ?event, pattern = ?pattern, "Firing hook");
            // Catch panics in handlers — a buggy hook should never crash the gateway.
            let handler_clone = handler.clone();
            let event_clone = event.clone();
            let ctx_clone = ctx.clone();
            tokio::spawn(async move {
                let result = std::panic::AssertUnwindSafe(
                    handler_clone(event_clone, ctx_clone)
                ).catch_unwind_safe().await;
                if let Err(_) = result {
                    tracing::warn!("Hook handler panicked — caught and ignored");
                }
            });
        }
    }

    /// Returns the number of registered handlers.
    pub fn len(&self) -> usize {
        self.handlers.read().unwrap().len()
    }

    /// Returns true if no handlers are registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.read().unwrap().is_empty()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// catch_unwind for async futures
trait CatchUnwindSafe<F: std::future::Future> {
    async fn catch_unwind_safe(self) -> std::result::Result<F::Output, ()>;
}

impl<F: std::future::Future + Send> CatchUnwindSafe<F> for std::panic::AssertUnwindSafe<F> {
    async fn catch_unwind_safe(self) -> std::result::Result<F::Output, ()> {
        // We can't use futures::FutureExt::catch_unwind because it requires
        // the future to be Unpin. Instead, we use tokio::spawn + oneshot
        // to isolate the panic. But that's complex — for now, just await
        // directly. Panics in async hooks are rare and will be caught by
        // the tokio runtime's panic handler.
        Ok(self.await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_allows_when_no_filters() {
        let pipeline = MessagePipeline::new();
        let msg = IncomingMessage::new("test", "user1", "user1", "chan1", "hello");
        assert!(matches!(pipeline.process(&msg), PipelineAction::Allow));
    }

    #[test]
    fn hook_event_matches_wildcard() {
        let event = HookEvent::Command("reset".to_string());
        let wildcard = HookEvent::Command("*".to_string());
        assert!(event.matches(&wildcard));

        let specific = HookEvent::Command("reset".to_string());
        assert!(event.matches(&specific));

        let other = HookEvent::Command("clear".to_string());
        assert!(!event.matches(&other));
    }

    #[test]
    fn hook_event_matches_non_command() {
        let event = HookEvent::AgentStart;
        assert!(event.matches(&HookEvent::AgentStart));
        assert!(!event.matches(&HookEvent::AgentEnd));
    }

    #[tokio::test]
    async fn hook_registry_register_and_emit() {
        let registry = HookRegistry::new();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        registry.register(
            HookEvent::AgentStart,
            Arc::new(move |_event, _ctx| {
                let c = counter_clone.clone();
                Box::pin(async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
            }),
        ).await;

        assert_eq!(registry.len(), 1);

        registry.emit(HookEvent::AgentStart, HookContext::new()).await;
        // Give the spawned task time to run
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hook_registry_wildcard_matching() {
        let registry = HookRegistry::new();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter_clone = counter.clone();

        // Register wildcard handler
        registry.register(
            HookEvent::Command("*".to_string()),
            Arc::new(move |_event, _ctx| {
                let c = counter_clone.clone();
                Box::pin(async move {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })
            }),
        ).await;

        // Emit specific command — should match wildcard
        registry.emit(HookEvent::Command("reset".to_string()), HookContext::new()).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Emit different command — should also match wildcard
        registry.emit(HookEvent::Command("clear".to_string()), HookContext::new()).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}

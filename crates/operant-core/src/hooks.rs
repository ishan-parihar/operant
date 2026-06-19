//! Lifecycle hooks system for Operant-RS
//!
//! Provides a type-safe, async event system for intercepting key lifecycle
//! points in the agent loop. Mirrors the Python hook system with events for
//! pre/post tool calls, pre/post LLM calls, and session start/end.
//!
//! # Architecture
//!
//! A global [`HookRegistry`] holds all registered handlers, backed by
//! [`tokio::sync::RwLock`] for async concurrent read access. Handlers are
//! trait objects implementing [`HookHandler`] with priority-based ordering.
//!
//! # Events
//!
//! - [`HookEvent::PreToolCall`] — Before tool execution
//! - [`HookEvent::PostToolCall`] — After tool execution
//! - [`HookEvent::PreLlmCall`] — Before LLM API call
//! - [`HookEvent::PostLlmCall`] — After LLM API call
//! - [`HookEvent::SessionStart`] — Session initialized
//! - [`HookEvent::SessionEnd`] — Session terminated
//!
//! # Error Handling
//!
//! Hook errors are caught and logged but never block the main pipeline.
//! Individual handler failures do not abort remaining handlers.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::client::{ChatResponse, Message};
use crate::tools::ToolResult;

// ---------------------------------------------------------------------------
// Hook Events
// ---------------------------------------------------------------------------

/// Lifecycle events that handlers can intercept.
///
/// Each variant carries the data relevant to that lifecycle point.
/// Use references in the event to avoid unnecessary cloning.
pub enum HookEvent<'a> {
    /// Before a tool call is executed.
    PreToolCall {
        /// Name of the tool being called.
        tool_name: &'a str,
        /// Arguments to the tool call.
        args: &'a Value,
    },
    /// After a tool call completes.
    PostToolCall {
        /// Name of the tool that was called.
        tool_name: &'a str,
        /// Result of the tool execution.
        result: &'a ToolResult,
    },
    /// Before an LLM API call is made.
    PreLlmCall {
        /// Messages being sent to the LLM.
        messages: &'a [Message],
    },
    /// After an LLM API call completes.
    PostLlmCall {
        /// Response from the LLM.
        response: &'a ChatResponse,
    },
    /// A new session has started.
    SessionStart {
        /// Unique session identifier.
        session_id: &'a str,
    },
    /// A session has ended.
    SessionEnd {
        /// Unique session identifier.
        session_id: &'a str,
    },
}

impl<'a> HookEvent<'a> {
    /// Returns the event type name for logging.
    pub fn event_type(&self) -> &'static str {
        match self {
            HookEvent::PreToolCall { .. } => "pre_tool_call",
            HookEvent::PostToolCall { .. } => "post_tool_call",
            HookEvent::PreLlmCall { .. } => "pre_llm_call",
            HookEvent::PostLlmCall { .. } => "post_llm_call",
            HookEvent::SessionStart { .. } => "session_start",
            HookEvent::SessionEnd { .. } => "session_end",
        }
    }
}

impl<'a> fmt::Debug for HookEvent<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookEvent::PreToolCall { tool_name, .. } => {
                write!(f, "PreToolCall({})", tool_name)
            }
            HookEvent::PostToolCall { tool_name, .. } => {
                write!(f, "PostToolCall({})", tool_name)
            }
            HookEvent::PreLlmCall { .. } => write!(f, "PreLlmCall"),
            HookEvent::PostLlmCall { .. } => write!(f, "PostLlmCall"),
            HookEvent::SessionStart { session_id } => {
                write!(f, "SessionStart({})", session_id)
            }
            HookEvent::SessionEnd { session_id } => {
                write!(f, "SessionEnd({})", session_id)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hook Actions
// ---------------------------------------------------------------------------

/// Action returned by a hook handler to control pipeline flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookAction {
    /// Continue normal execution (default).
    Continue,
    /// Cancel the operation (for pre-hooks).
    Cancel,
    /// Return modified data (encoded as JSON Value).
    Modify(Value),
}

impl Default for HookAction {
    fn default() -> Self {
        Self::Continue
    }
}

// ---------------------------------------------------------------------------
// Hook Handler Trait
// ---------------------------------------------------------------------------

/// Trait for lifecycle hook handlers.
///
/// Implement this trait to intercept agent lifecycle events.
/// Handlers are sorted by priority (lower = higher priority, runs first).
#[async_trait]
pub trait HookHandler: Send + Sync + 'static {
    /// Handle a lifecycle event.
    ///
    /// Returns a [`HookAction`] to control pipeline flow.
    /// Errors are caught and logged but do not abort the pipeline.
    async fn handle(&self, event: &HookEvent<'_>) -> crate::error::Result<HookAction>;

    /// Priority for execution ordering (lower = runs first, default 100).
    fn priority(&self) -> i32 {
        100
    }
}

// ---------------------------------------------------------------------------
// Registered Handler Entry
// ---------------------------------------------------------------------------

/// A registered handler with metadata.
struct HandlerEntry {
    handler: Arc<dyn HookHandler>,
    name: String,
}

// ---------------------------------------------------------------------------
// Hook Registry
// ---------------------------------------------------------------------------

/// Thread-safe registry of lifecycle hook handlers.
///
/// Handlers are registered for specific event types and invoked in priority
/// order when events are emitted.
pub struct HookRegistry {
    handlers: RwLock<HashMap<&'static str, Vec<HandlerEntry>>>,
}

impl HookRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a handler for a specific event type.
    ///
    /// The event type is derived from the variant name (e.g., `"pre_tool_call"`).
    /// Handlers with lower priority values execute first.
    pub async fn register(
        &self,
        event_type: &'static str,
        name: impl Into<String>,
        handler: Arc<dyn HookHandler>,
    ) {
        let mut handlers = self.handlers.write().await;
        let entries = handlers.entry(event_type).or_default();

        let name = name.into();
        info!(
            "Registered hook '{}' for event '{}' (priority: {})",
            name,
            event_type,
            handler.priority()
        );

        entries.push(HandlerEntry { handler, name });

        // Sort by priority (lower = first)
        entries.sort_by_key(|e| e.handler.priority());
    }

    /// Remove all handlers for a given event type.
    pub async fn unregister_all(&self, event_type: &str) {
        let mut handlers = self.handlers.write().await;
        handlers.remove(event_type);
    }

    /// Remove a specific handler by name from an event type.
    pub async fn unregister(&self, event_type: &str, name: &str) {
        let mut handlers = self.handlers.write().await;
        if let Some(entries) = handlers.get_mut(event_type) {
            entries.retain(|e| e.name != name);
        }
    }

    /// Emit an event to all registered handlers.
    ///
    /// Handler errors are logged but do not abort remaining handlers.
    /// Returns the first non-Continue action, or Continue if all handlers pass.
    pub async fn emit(&self, event: &HookEvent<'_>) -> HookAction {
        let event_type = event.event_type();
        let handlers = self.handlers.read().await;

        if let Some(entries) = handlers.get(event_type) {
            for entry in entries {
                match entry.handler.handle(event).await {
                    Ok(action) => {
                        if action != HookAction::Continue {
                            info!(
                                "Hook '{}' returned {:?} for event '{}'",
                                entry.name, action, event_type
                            );
                            return action;
                        }
                    }
                    Err(e) => {
                        error!(
                            "Hook '{}' failed for event '{}': {}",
                            entry.name, event_type, e
                        );
                    }
                }
            }
        }

        HookAction::Continue
    }

    /// Emit an event and collect all handler return values.
    ///
    /// Like [`emit`](Self::emit) but collects non-Continue actions from all handlers.
    /// Useful for decision-style hooks that need responses from multiple handlers.
    pub async fn emit_collect(&self, event: &HookEvent<'_>) -> Vec<HookAction> {
        let event_type = event.event_type();
        let handlers = self.handlers.read().await;
        let mut results = Vec::new();

        if let Some(entries) = handlers.get(event_type) {
            for entry in entries {
                match entry.handler.handle(event).await {
                    Ok(action) => {
                        if action != HookAction::Continue {
                            results.push(action);
                        }
                    }
                    Err(e) => {
                        error!(
                            "Hook '{}' failed for event '{}': {}",
                            entry.name, event_type, e
                        );
                    }
                }
            }
        }

        results
    }

    /// List all registered event types.
    pub async fn list_event_types(&self) -> Vec<&'static str> {
        let handlers = self.handlers.read().await;
        handlers.keys().copied().collect()
    }

    /// List all handlers for a given event type.
    pub async fn list_handlers(&self, event_type: &str) -> Vec<String> {
        let handlers = self.handlers.read().await;
        handlers
            .get(event_type)
            .map(|entries| entries.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global Registry
// ---------------------------------------------------------------------------

use std::sync::OnceLock;

static GLOBAL_HOOK_REGISTRY: OnceLock<HookRegistry> = OnceLock::new();

/// Get the global hook registry.
pub fn global_hook_registry() -> &'static HookRegistry {
    GLOBAL_HOOK_REGISTRY.get_or_init(HookRegistry::new)
}

// ---------------------------------------------------------------------------
// Convenience API
// ---------------------------------------------------------------------------

/// Register a handler for a specific event type in the global registry.
pub async fn register_hook(
    event_type: &'static str,
    name: impl Into<String>,
    handler: Arc<dyn HookHandler>,
) {
    global_hook_registry()
        .register(event_type, name, handler)
        .await;
}

/// Emit an event to the global registry.
pub async fn emit_hook(event: &HookEvent<'_>) -> HookAction {
    global_hook_registry().emit(event).await
}

/// Emit an event and collect all handler responses.
pub async fn emit_hook_collect(event: &HookEvent<'_>) -> Vec<HookAction> {
    global_hook_registry().emit_collect(event).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

    /// Simple counter hook for testing.
    struct CounterHook {
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HookHandler for CounterHook {
        async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(HookAction::Continue)
        }
    }

    /// Hook that cancels operations.
    struct CancelHook;

    #[async_trait]
    impl HookHandler for CancelHook {
        async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
            Ok(HookAction::Cancel)
        }
    }

    /// Hook that returns an error.
    struct FailingHook;

    #[async_trait]
    impl HookHandler for FailingHook {
        async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
            Err(crate::error::Error::Agent("test failure".into()))
        }
    }

    /// Hook with custom priority.
    struct PriorityHook {
        priority: i32,
        order: Arc<AtomicUsize>,
        call_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl HookHandler for PriorityHook {
        async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.order.fetch_add(1, Ordering::SeqCst);
            Ok(HookAction::Continue)
        }

        fn priority(&self) -> i32 {
            self.priority
        }
    }

    #[tokio::test]
    async fn test_registry_creation() {
        let registry = HookRegistry::new();
        let types = registry.list_event_types().await;
        assert!(types.is_empty());
    }

    #[tokio::test]
    async fn test_register_and_emit() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(CounterHook {
            call_count: counter.clone(),
        });

        registry
            .register("pre_tool_call", "test_counter", hook)
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        let action = registry.emit(&event).await;
        assert_eq!(action, HookAction::Continue);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_cancel_action() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));

        // Register cancel hook first (lower priority = runs first)
        registry
            .register(
                "pre_tool_call",
                "cancel",
                Arc::new(CancelHook) as Arc<dyn HookHandler>,
            )
            .await;

        // This hook should NOT run because cancel returns early
        registry
            .register(
                "pre_tool_call",
                "counter",
                Arc::new(CounterHook {
                    call_count: counter.clone(),
                }),
            )
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        let action = registry.emit(&event).await;
        assert_eq!(action, HookAction::Cancel);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_failing_hook_does_not_abort() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));

        // Register failing hook first
        registry
            .register(
                "pre_tool_call",
                "failing",
                Arc::new(FailingHook) as Arc<dyn HookHandler>,
            )
            .await;

        // This hook should still run
        registry
            .register(
                "pre_tool_call",
                "counter",
                Arc::new(CounterHook {
                    call_count: counter.clone(),
                }),
            )
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        let action = registry.emit(&event).await;
        assert_eq!(action, HookAction::Continue);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_priority_ordering() {
        let registry = HookRegistry::new();
        let order = Arc::new(AtomicUsize::new(0));
        let counter1 = Arc::new(AtomicUsize::new(0));
        let counter2 = Arc::new(AtomicUsize::new(0));

        // Register high priority (10) first
        registry
            .register(
                "pre_tool_call",
                "high_priority",
                Arc::new(PriorityHook {
                    priority: 10,
                    order: order.clone(),
                    call_count: counter1.clone(),
                }),
            )
            .await;

        // Register low priority (200) second
        registry
            .register(
                "pre_tool_call",
                "low_priority",
                Arc::new(PriorityHook {
                    priority: 200,
                    order: order.clone(),
                    call_count: counter2.clone(),
                }),
            )
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        registry.emit(&event).await;

        // Both should have been called
        assert_eq!(counter1.load(Ordering::SeqCst), 1);
        assert_eq!(counter2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_emit_collect() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));

        registry
            .register(
                "pre_tool_call",
                "counter",
                Arc::new(CounterHook {
                    call_count: counter.clone(),
                }),
            )
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        let results = registry.emit_collect(&event).await;
        assert!(results.is_empty()); // All Continue actions are filtered
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(CounterHook {
            call_count: counter.clone(),
        });

        registry
            .register("pre_tool_call", "test_counter", hook)
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        // Emit should work
        registry.emit(&event).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Unregister
        registry.unregister("pre_tool_call", "test_counter").await;

        // Emit should not call handler
        registry.emit(&event).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1); // No change
    }

    #[tokio::test]
    async fn test_unregister_all() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(CounterHook {
            call_count: counter.clone(),
        });

        registry
            .register("pre_tool_call", "test_counter", hook)
            .await;

        let event = HookEvent::PreToolCall {
            tool_name: "test_tool",
            args: &serde_json::json!({}),
        };

        registry.emit(&event).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Unregister all
        registry.unregister_all("pre_tool_call").await;

        // Emit should not call handler
        registry.emit(&event).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1); // No change
    }

    #[tokio::test]
    async fn test_list_handlers() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(CounterHook {
            call_count: counter.clone(),
        });

        registry
            .register("pre_tool_call", "test_counter", hook)
            .await;

        let handlers = registry.list_handlers("pre_tool_call").await;
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0], "test_counter");

        let empty = registry.list_handlers("unknown_event").await;
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn test_event_debug_format() {
        let event = HookEvent::PreToolCall {
            tool_name: "my_tool",
            args: &serde_json::json!({"key": "value"}),
        };

        let debug = format!("{:?}", event);
        assert!(debug.contains("PreToolCall"));
        assert!(debug.contains("my_tool"));
    }

    #[tokio::test]
    async fn test_session_events() {
        let registry = HookRegistry::new();
        let start_counter = Arc::new(AtomicUsize::new(0));
        let end_counter = Arc::new(AtomicUsize::new(0));

        let start_hook = Arc::new(CounterHook {
            call_count: start_counter.clone(),
        });
        let end_hook = Arc::new(CounterHook {
            call_count: end_counter.clone(),
        });

        registry
            .register("session_start", "start_counter", start_hook)
            .await;
        registry
            .register("session_end", "end_counter", end_hook)
            .await;

        let start_event = HookEvent::SessionStart {
            session_id: "test-session",
        };
        let end_event = HookEvent::SessionEnd {
            session_id: "test-session",
        };

        registry.emit(&start_event).await;
        assert_eq!(start_counter.load(Ordering::SeqCst), 1);
        assert_eq!(end_counter.load(Ordering::SeqCst), 0);

        registry.emit(&end_event).await;
        assert_eq!(start_counter.load(Ordering::SeqCst), 1);
        assert_eq!(end_counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_modify_action() {
        let registry = HookRegistry::new();
        let modified = Arc::new(AtomicBool::new(false));
        let m = modified.clone();
        struct ModifyHandler(Arc<AtomicBool>);
        #[async_trait]
        impl HookHandler for ModifyHandler {
            async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
                self.0.store(true, Ordering::SeqCst);
                Ok(HookAction::Modify(serde_json::json!({"key": "value"})))
            }
            fn priority(&self) -> i32 {
                0
            }
        }
        registry
            .register("pre_tool_call", "modify_test", Arc::new(ModifyHandler(m)))
            .await;
        let action = registry
            .emit(&HookEvent::PreToolCall {
                tool_name: "t",
                args: &serde_json::json!({}),
            })
            .await;
        assert!(modified.load(Ordering::SeqCst));
        assert!(matches!(action, HookAction::Modify(_)));
    }

    #[tokio::test]
    async fn test_emit_no_handlers_returns_continue() {
        let registry = HookRegistry::new();
        let action = registry
            .emit(&HookEvent::PreToolCall {
                tool_name: "t",
                args: &serde_json::json!({}),
            })
            .await;
        assert!(matches!(action, HookAction::Continue));
    }

    #[tokio::test]
    async fn test_emit_collect_multiple_actions() {
        let registry = HookRegistry::new();
        for i in 0..5 {
            struct ModifyHandler(i32);
            #[async_trait]
            impl HookHandler for ModifyHandler {
                async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
                    Ok(HookAction::Modify(serde_json::json!({"i": self.0})))
                }
                fn priority(&self) -> i32 {
                    self.0
                }
            }
            registry
                .register(
                    "pre_tool_call",
                    &format!("h{}", i),
                    Arc::new(ModifyHandler(i)),
                )
                .await;
        }
        let actions = registry
            .emit_collect(&HookEvent::PreToolCall {
                tool_name: "t",
                args: &serde_json::json!({}),
            })
            .await;
        assert_eq!(actions.len(), 5);
    }

    #[tokio::test]
    async fn test_list_event_types() {
        let registry = HookRegistry::new();
        struct NoopHandler;
        #[async_trait]
        impl HookHandler for NoopHandler {
            async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
                Ok(HookAction::Continue)
            }
            fn priority(&self) -> i32 {
                0
            }
        }
        registry
            .register("pre_tool_call", "n1", Arc::new(NoopHandler))
            .await;
        registry
            .register("session_start", "n2", Arc::new(NoopHandler))
            .await;
        let types = registry.list_event_types().await;
        assert!(types.contains(&"pre_tool_call"));
        assert!(types.contains(&"session_start"));
    }

    #[tokio::test]
    async fn test_reregister_same_name_adds() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicI32::new(0));
        let c = counter.clone();
        struct CountHandler(Arc<AtomicI32>);
        #[async_trait]
        impl HookHandler for CountHandler {
            async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(HookAction::Continue)
            }
            fn priority(&self) -> i32 {
                0
            }
        }
        registry
            .register(
                "pre_tool_call",
                "handler_a",
                Arc::new(CountHandler(c.clone())),
            )
            .await;
        registry
            .register(
                "pre_tool_call",
                "handler_b",
                Arc::new(CountHandler(c.clone())),
            )
            .await;
        registry
            .emit(&HookEvent::PreToolCall {
                tool_name: "t",
                args: &serde_json::json!({}),
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_unregister_one_of_many() {
        let registry = HookRegistry::new();
        let counter = Arc::new(AtomicI32::new(0));
        struct CountHandler(Arc<AtomicI32>);
        #[async_trait]
        impl HookHandler for CountHandler {
            async fn handle(&self, _event: &HookEvent<'_>) -> crate::error::Result<HookAction> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(HookAction::Continue)
            }
            fn priority(&self) -> i32 {
                0
            }
        }
        registry
            .register(
                "pre_tool_call",
                "keep",
                Arc::new(CountHandler(counter.clone())),
            )
            .await;
        registry
            .register(
                "pre_tool_call",
                "remove_me",
                Arc::new(CountHandler(counter.clone())),
            )
            .await;
        registry.unregister("pre_tool_call", "remove_me").await;
        registry
            .emit(&HookEvent::PreToolCall {
                tool_name: "t",
                args: &serde_json::json!({}),
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hook_debug_format() {
        let event = HookEvent::PreToolCall {
            tool_name: "bash",
            args: &serde_json::json!({}),
        };
        let debug = format!("{:?}", event);
        assert!(debug.contains("PreToolCall"));
    }
}

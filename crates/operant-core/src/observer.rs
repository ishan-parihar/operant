//! Observer/Telemetry trait for structured agent runtime observability.
//!
//! Modeled after zeroclaw's `observability_traits.rs`. Provides structured
//! events and metrics that observers can record, aggregate, or forward to
//! external monitoring systems (structured logging, Prometheus, OpenTelemetry).

use std::time::Duration;

/// Discrete events emitted by the agent runtime for observability.
///
/// Each variant represents a lifecycle event that observers can record,
/// aggregate, or forward to external monitoring systems. Events carry
/// just enough context for tracing and diagnostics without exposing
/// sensitive prompt or response content.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ObserverEvent {
    /// The agent orchestration loop has started a new session.
    AgentStart { provider: String, model: String },
    /// A request is about to be sent to an LLM provider.
    LlmRequest {
        provider: String,
        model: String,
        messages_count: usize,
    },
    /// Result of a single LLM provider call.
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    /// The agent session has finished.
    AgentEnd {
        provider: String,
        model: String,
        duration: Duration,
        tokens_used: Option<u64>,
        cost_usd: Option<f64>,
    },
    /// A tool call is about to be executed.
    ToolCallStart {
        tool: String,
        arguments: Option<String>,
    },
    /// A tool call has completed with a success/failure outcome.
    ToolCall {
        tool: String,
        duration: Duration,
        success: bool,
    },
    /// The agent produced a final answer for the current user message.
    TurnComplete,
    /// A message was sent or received through a channel.
    ChannelMessage {
        channel: String,
        direction: String,
    },
    /// Periodic heartbeat tick from the runtime keep-alive loop.
    HeartbeatTick,
    /// Response cache hit — an LLM call was avoided.
    CacheHit {
        cache_type: String,
        tokens_saved: u64,
    },
    /// Response cache miss — the prompt was not found in cache.
    CacheMiss { cache_type: String },
    /// An error occurred in a named component.
    Error {
        component: String,
        message: String,
    },
    /// A hand (sub-agent or specialized task) has started execution.
    HandStarted { hand_name: String },
    /// A hand has completed execution successfully.
    HandCompleted {
        hand_name: String,
        duration_ms: u64,
        findings_count: usize,
    },
    /// A hand has failed during execution.
    HandFailed {
        hand_name: String,
        error: String,
        duration_ms: u64,
    },
}

/// Numeric metrics emitted by the agent runtime.
///
/// Observers can aggregate these into dashboards, alerts, or structured logs.
/// Each variant carries a single scalar value with implicit units.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ObserverMetric {
    /// Time elapsed for a single LLM or tool request.
    RequestLatency(Duration),
    /// Number of tokens consumed by an LLM call.
    TokensUsed(u64),
    /// Current number of active concurrent sessions.
    ActiveSessions(u64),
    /// Current depth of the inbound message queue.
    QueueDepth(u64),
    /// Duration of a single hand run.
    HandRunDuration {
        hand_name: String,
        duration: Duration,
    },
    /// Number of findings produced by a hand run.
    HandFindingsCount { hand_name: String, count: u64 },
    /// Records a hand run outcome for success-rate tracking.
    HandSuccessRate { hand_name: String, success: bool },
}

/// Core observability trait for recording agent runtime telemetry.
///
/// Implement this trait to integrate with any monitoring backend (structured
/// logging, Prometheus, OpenTelemetry, etc.). The agent runtime holds one or
/// more `Observer` instances and calls [`record_event`](Observer::record_event)
/// and [`record_metric`](Observer::record_metric) at key lifecycle points.
///
/// Implementations must be `Send + Sync` because the observer is
/// shared across async tasks via `Arc`.
pub trait Observer: Send + Sync {
    /// Record a discrete lifecycle event.
    ///
    /// Called synchronously on the hot path; implementations should avoid
    /// blocking I/O. Buffer events internally and flush asynchronously
    /// when possible.
    fn record_event(&self, event: &ObserverEvent);

    /// Record a numeric metric sample.
    ///
    /// Called synchronously; same non-blocking guidance as
    /// [`record_event`](Observer::record_event).
    fn record_metric(&self, metric: &ObserverMetric);

    /// Flush any buffered telemetry data to the backend.
    ///
    /// The runtime calls this during graceful shutdown. The default
    /// implementation is a no-op.
    ///
    /// **Note**: This is synchronous by design — async backends (e.g.,
    /// Prometheus push gateway) should buffer internally and flush on
    /// a background task triggered by this call.
    fn flush(&self) {}

    /// Return the human-readable name of this observer backend.
    ///
    /// Used in logs and diagnostics (e.g., `"console"`, `"prometheus"`,
    /// `"opentelemetry"`).
    fn name(&self) -> &str;
}

/// Blanket implementation: `Arc<T>` delegates all `Observer` methods to `T`.
impl<T: Observer + ?Sized> Observer for std::sync::Arc<T> {
    fn record_event(&self, event: &ObserverEvent) {
        self.as_ref().record_event(event);
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.as_ref().record_metric(metric);
    }

    fn flush(&self) {
        self.as_ref().flush();
    }

    fn name(&self) -> &str {
        self.as_ref().name()
    }
}

/// A simple console observer that logs events and metrics to stderr via `tracing`.
///
/// This is a reference implementation. Production deployments should use
/// a dedicated observer backend (e.g., Prometheus, OpenTelemetry).
pub struct ConsoleObserver;

impl Observer for ConsoleObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                tracing::info!(provider, model, "agent session started");
            }
            ObserverEvent::AgentEnd {
                provider,
                model,
                duration,
                tokens_used,
                cost_usd,
            } => {
                tracing::info!(
                    provider,
                    model,
                    duration_ms = duration.as_millis() as u64,
                    tokens_used,
                    cost_usd,
                    "agent session ended"
                );
            }
            ObserverEvent::LlmRequest {
                provider,
                model,
                messages_count,
            } => {
                tracing::debug!(provider, model, messages_count, "llm request");
            }
            ObserverEvent::LlmResponse {
                provider,
                model,
                duration,
                success,
                error_message,
                input_tokens,
                output_tokens,
            } => {
                tracing::debug!(
                    provider,
                    model,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    error_message,
                    input_tokens,
                    output_tokens,
                    "llm response"
                );
            }
            ObserverEvent::ToolCallStart { tool, arguments } => {
                tracing::debug!(tool, arguments, "tool call started");
            }
            ObserverEvent::ToolCall {
                tool,
                duration,
                success,
            } => {
                tracing::debug!(
                    tool,
                    duration_ms = duration.as_millis() as u64,
                    success,
                    "tool call completed"
                );
            }
            ObserverEvent::TurnComplete => {
                tracing::debug!("turn complete");
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                tracing::debug!(channel, direction, "channel message");
            }
            ObserverEvent::HeartbeatTick => {
                tracing::trace!("heartbeat tick");
            }
            ObserverEvent::CacheHit {
                cache_type,
                tokens_saved,
            } => {
                tracing::debug!(cache_type, tokens_saved, "cache hit");
            }
            ObserverEvent::CacheMiss { cache_type } => {
                tracing::debug!(cache_type, "cache miss");
            }
            ObserverEvent::Error { component, message } => {
                tracing::error!(component, message, "runtime error");
            }
            ObserverEvent::HandStarted { hand_name } => {
                tracing::info!(hand_name, "hand started");
            }
            ObserverEvent::HandCompleted {
                hand_name,
                duration_ms,
                findings_count,
            } => {
                tracing::info!(hand_name, duration_ms, findings_count, "hand completed");
            }
            ObserverEvent::HandFailed {
                hand_name,
                error,
                duration_ms,
            } => {
                tracing::error!(hand_name, error, duration_ms, "hand failed");
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(duration) => {
                tracing::debug!(duration_ms = duration.as_millis() as u64, "request latency");
            }
            ObserverMetric::TokensUsed(count) => {
                tracing::debug!(tokens = count, "tokens used");
            }
            ObserverMetric::ActiveSessions(count) => {
                tracing::debug!(sessions = count, "active sessions");
            }
            ObserverMetric::QueueDepth(depth) => {
                tracing::debug!(depth, "queue depth");
            }
            ObserverMetric::HandRunDuration {
                hand_name,
                duration,
            } => {
                tracing::debug!(hand_name, duration_ms = duration.as_millis() as u64, "hand run duration");
            }
            ObserverMetric::HandFindingsCount {
                hand_name,
                count,
            } => {
                tracing::debug!(hand_name, count, "hand findings count");
            }
            ObserverMetric::HandSuccessRate {
                hand_name,
                success,
            } => {
                tracing::debug!(hand_name, success, "hand success rate");
            }
        }
    }

    fn name(&self) -> &str {
        "console"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestObserver {
        events: AtomicU64,
        metrics: AtomicU64,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                events: AtomicU64::new(0),
                metrics: AtomicU64::new(0),
            }
        }
    }

    impl Observer for TestObserver {
        fn record_event(&self, _event: &ObserverEvent) {
            self.events.fetch_add(1, Ordering::SeqCst);
        }

        fn record_metric(&self, _metric: &ObserverMetric) {
            self.metrics.fetch_add(1, Ordering::SeqCst);
        }

        fn name(&self) -> &str {
            "test"
        }
    }

    #[test]
    fn observer_records_events_and_metrics() {
        let observer = TestObserver::new();

        observer.record_event(&ObserverEvent::HeartbeatTick);
        observer.record_event(&ObserverEvent::Error {
            component: "test".into(),
            message: "boom".into(),
        });
        observer.record_metric(&ObserverMetric::TokensUsed(42));

        assert_eq!(observer.events.load(Ordering::SeqCst), 2);
        assert_eq!(observer.metrics.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn observer_default_flush_works() {
        let observer = TestObserver::new();
        observer.flush(); // no-op should not panic
        assert_eq!(observer.name(), "test");
    }

    #[test]
    fn observer_arc_delegates() {
        let observer = std::sync::Arc::new(TestObserver::new());
        observer.record_event(&ObserverEvent::TurnComplete);
        assert_eq!(observer.events.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn console_observer_name() {
        let observer = ConsoleObserver;
        assert_eq!(observer.name(), "console");
    }

    #[test]
    fn observer_event_is_clone_and_debug() {
        let event = ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: true,
        };
        let cloned = event.clone();
        assert!(format!("{:?}", cloned).contains("shell"));
    }

    #[test]
    fn observer_metric_is_clone_and_debug() {
        let metric = ObserverMetric::RequestLatency(Duration::from_millis(8));
        let cloned = metric.clone();
        let debug = format!("{:?}", cloned);
        assert!(debug.contains("RequestLatency"));
    }

    #[test]
    fn hand_events_recordable() {
        let observer = TestObserver::new();

        observer.record_event(&ObserverEvent::HandStarted {
            hand_name: "review".into(),
        });
        observer.record_event(&ObserverEvent::HandCompleted {
            hand_name: "review".into(),
            duration_ms: 1500,
            findings_count: 3,
        });
        observer.record_event(&ObserverEvent::HandFailed {
            hand_name: "review".into(),
            error: "timeout".into(),
            duration_ms: 5000,
        });

        assert_eq!(observer.events.load(Ordering::SeqCst), 3);
    }
}

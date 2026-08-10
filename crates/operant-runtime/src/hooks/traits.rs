use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use operant_api::channel::ChannelMessage;
use operant_api::provider::{ChatMessage, ChatResponse};
use operant_api::tool::ToolResult;

/// Result of a modifying hook — continue with (possibly modified) data, or cancel.
#[derive(Debug, Clone)]
pub enum HookResult<T> {
    Continue(T),
    Cancel(String),
}

impl<T> HookResult<T> {
    pub fn is_cancel(&self) -> bool {
        matches!(self, HookResult::Cancel(_))
    }
}

/// Trait for hook handlers. All methods have default no-op implementations.
/// Implement only the events you care about.
#[async_trait]
pub trait HookHandler: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 {
        0
    }

    // --- Void hooks (parallel, fire-and-forget) ---
    async fn on_gateway_start(&self, _host: &str, _port: u16) {}
    async fn on_gateway_stop(&self) {}
    async fn on_session_start(&self, _session_id: &str, _channel: &str) {}
    async fn on_session_end(&self, _session_id: &str, _channel: &str) {}
    async fn on_llm_input(&self, _messages: &[ChatMessage], _model: &str) {}
    async fn on_llm_output(&self, _response: &ChatResponse) {}
    async fn on_after_tool_call(&self, _tool: &str, _result: &ToolResult, _duration: Duration) {}
    async fn on_message_sent(&self, _channel: &str, _recipient: &str, _content: &str) {}
    async fn on_heartbeat_tick(&self) {}

    // --- Hermes-parity lifecycle hooks (all default no-ops) ---
    // Ported 1:1 from hermes-agent `VALID_HOOKS` so a hermes plugin can
    // implement every callback it registers without adaptation.

    /// Successful skill lifecycle facts (skill invoked / tool ran).
    async fn on_skill_lifecycle(&self, _skill: &str, _action: &str, _ok: bool) {}
    /// Fired when a delegate sub-agent starts a run.
    async fn subagent_start(&self, _agent: &str, _depth: u32, _prompt: &str) {}
    /// Fired when a delegate sub-agent finishes (or fails) a run.
    async fn subagent_stop(&self, _agent: &str, _depth: u32, _ok: bool) {}
    /// Fired before an approval prompt is raised (observers only; cannot veto).
    async fn pre_approval_request(&self, _tool: &str, _summary: &str, _surface: &str) {}
    /// Fired after an approval decision was recorded.
    async fn post_approval_response(&self, _tool: &str, _decision: &str) {}
    /// Fired when a session is reset (not started/ended — a reset).
    async fn on_session_reset(&self, _session_id: &str, _channel: &str) {}

    // --- Modifying hooks (sequential by priority, can cancel) ---
    async fn before_model_resolve(
        &self,
        provider: String,
        model: String,
    ) -> HookResult<(String, String)> {
        HookResult::Continue((provider, model))
    }

    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
        HookResult::Continue(prompt)
    }

    async fn before_llm_call(
        &self,
        messages: Vec<ChatMessage>,
        model: String,
    ) -> HookResult<(Vec<ChatMessage>, String)> {
        HookResult::Continue((messages, model))
    }

    async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> {
        HookResult::Continue((name, args))
    }

    async fn on_message_received(&self, message: ChannelMessage) -> HookResult<ChannelMessage> {
        HookResult::Continue(message)
    }

    async fn on_message_sending(
        &self,
        channel: String,
        recipient: String,
        content: String,
    ) -> HookResult<(String, String, String)> {
        HookResult::Continue((channel, recipient, content))
    }

    /// Transform the assistant response text before it is returned to the
    /// user. Mirrors hermes `transform_llm_output`: a handler returns a string
    /// to replace the text, or `None` to leave unchanged. First non-`None`
    /// result wins across handlers.
    async fn transform_llm_output(&self, _text: String) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHook {
        name: String,
        priority: i32,
    }

    impl TestHook {
        fn new(name: &str, priority: i32) -> Self {
            Self {
                name: name.to_string(),
                priority,
            }
        }
    }

    #[async_trait]
    impl HookHandler for TestHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
    }

    #[test]
    fn hook_result_is_cancel() {
        let ok: HookResult<String> = HookResult::Continue("hi".into());
        assert!(!ok.is_cancel());
        let cancel: HookResult<String> = HookResult::Cancel("blocked".into());
        assert!(cancel.is_cancel());
    }

    #[test]
    fn default_priority_is_zero() {
        struct MinimalHook;
        #[async_trait]
        impl HookHandler for MinimalHook {
            fn name(&self) -> &str {
                "minimal"
            }
        }
        assert_eq!(MinimalHook.priority(), 0);
    }

    #[tokio::test]
    async fn default_modifying_hooks_pass_through() {
        let hook = TestHook::new("test", 0);
        match hook
            .before_tool_call("shell".into(), serde_json::json!({"cmd": "ls"}))
            .await
        {
            HookResult::Continue((name, _args)) => assert_eq!(name, "shell"),
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }
}

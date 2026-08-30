//! Dynamic hook slot — kernel providers register/unregister hooks at runtime
//! without mutating HookRunner's static dispatch list.
//!
//! Design (plan 016 "kernel wraps, never replaces"): a single [`DynamicHooks`]
//! object implements [`HookHandler`] itself and is registered with the runner
//! like any static handler. Providers then add/remove child handlers through
//! its shared lock; every dispatch fans out to the current children snapshot
//! in insertion order. The hot path pays one extra handler entry — nothing
//! else changes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use operant_api::channel::ChannelMessage;
use operant_api::provider::{ChatMessage, ChatResponse};
use operant_api::tool::ToolResult;

use super::traits::{HookHandler, HookResult};

/// Shared, mutable hook slot. Cheap to clone via [`Arc`].
#[derive(Default)]
pub struct DynamicHooks {
    entries: RwLock<Vec<(String, Arc<dyn HookHandler>)>>,
}

impl DynamicHooks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a handler under a stable id.
    pub async fn add(&self, id: impl Into<String>, handler: Arc<dyn HookHandler>) {
        let id = id.into();
        let mut entries = self.entries.write().await;
        if let Some(slot) = entries.iter_mut().find(|(eid, _)| eid == &id) {
            slot.1 = handler;
            return;
        }
        entries.push((id, handler));
    }

    /// Remove a handler by id. Returns true when it existed.
    pub async fn remove(&self, id: &str) -> bool {
        let mut entries = self.entries.write().await;
        let len_before = entries.len();
        entries.retain(|(eid, _)| eid != id);
        entries.len() != len_before
    }

    /// Current child count (introspection).
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }

    /// Snapshot children in insertion order.
    async fn children(&self) -> Vec<Arc<dyn HookHandler>> {
        self.entries
            .read()
            .await
            .iter()
            .map(|(_, h)| Arc::clone(h))
            .collect()
    }
}

#[async_trait]
impl HookHandler for DynamicHooks {
    fn name(&self) -> &str {
        "dynamic-hooks"
    }

    // --- Void hooks: fan out in insertion order ---

    async fn on_gateway_start(&self, host: &str, port: u16) {
        for h in self.children().await {
            h.on_gateway_start(host, port).await;
        }
    }
    async fn on_gateway_stop(&self) {
        for h in self.children().await {
            h.on_gateway_stop().await;
        }
    }
    async fn on_session_start(&self, session_id: &str, channel: &str) {
        for h in self.children().await {
            h.on_session_start(session_id, channel).await;
        }
    }
    async fn on_session_end(&self, session_id: &str, channel: &str) {
        for h in self.children().await {
            h.on_session_end(session_id, channel).await;
        }
    }
    async fn on_llm_input(&self, messages: &[ChatMessage], model: &str) {
        for h in self.children().await {
            h.on_llm_input(messages, model).await;
        }
    }
    async fn on_llm_output(&self, response: &ChatResponse) {
        for h in self.children().await {
            h.on_llm_output(response).await;
        }
    }
    async fn on_after_tool_call(&self, tool: &str, result: &ToolResult, duration: Duration) {
        for h in self.children().await {
            h.on_after_tool_call(tool, result, duration).await;
        }
    }
    async fn on_message_sent(&self, channel: &str, recipient: &str, content: &str) {
        for h in self.children().await {
            h.on_message_sent(channel, recipient, content).await;
        }
    }
    async fn on_heartbeat_tick(&self) {
        for h in self.children().await {
            h.on_heartbeat_tick().await;
        }
    }
    async fn on_skill_lifecycle(&self, skill: &str, action: &str, ok: bool) {
        for h in self.children().await {
            h.on_skill_lifecycle(skill, action, ok).await;
        }
    }
    async fn subagent_start(&self, agent: &str, depth: u32, prompt: &str) {
        for h in self.children().await {
            h.subagent_start(agent, depth, prompt).await;
        }
    }
    async fn subagent_stop(&self, agent: &str, depth: u32, ok: bool) {
        for h in self.children().await {
            h.subagent_stop(agent, depth, ok).await;
        }
    }
    async fn pre_approval_request(&self, tool: &str, summary: &str, surface: &str) {
        for h in self.children().await {
            h.pre_approval_request(tool, summary, surface).await;
        }
    }
    async fn post_approval_response(&self, tool: &str, decision: &str) {
        for h in self.children().await {
            h.post_approval_response(tool, decision).await;
        }
    }
    async fn on_session_reset(&self, session_id: &str, channel: &str) {
        for h in self.children().await {
            h.on_session_reset(session_id, channel).await;
        }
    }

    // --- Modifying hooks: fold through children; Cancel short-circuits ---

    async fn before_model_resolve(
        &self,
        provider: String,
        model: String,
    ) -> HookResult<(String, String)> {
        let mut provider = provider;
        let mut model = model;
        for h in self.children().await {
            match h.before_model_resolve(provider, model).await {
                HookResult::Continue((p, m)) => {
                    provider = p;
                    model = m;
                }
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue((provider, model))
    }

    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
        let mut prompt = prompt;
        for h in self.children().await {
            match h.before_prompt_build(prompt).await {
                HookResult::Continue(p) => prompt = p,
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue(prompt)
    }

    async fn before_llm_call(
        &self,
        messages: Vec<ChatMessage>,
        model: String,
    ) -> HookResult<(Vec<ChatMessage>, String)> {
        let mut messages = messages;
        let mut model = model;
        for h in self.children().await {
            match h.before_llm_call(messages, model).await {
                HookResult::Continue((m, mo)) => {
                    messages = m;
                    model = mo;
                }
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue((messages, model))
    }

    async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> {
        let mut name = name;
        let mut args = args;
        for h in self.children().await {
            match h.before_tool_call(name, args).await {
                HookResult::Continue((n, a)) => {
                    name = n;
                    args = a;
                }
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue((name, args))
    }

    async fn on_message_received(&self, message: ChannelMessage) -> HookResult<ChannelMessage> {
        let mut message = message;
        for h in self.children().await {
            match h.on_message_received(message).await {
                HookResult::Continue(m) => message = m,
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue(message)
    }

    async fn on_message_sending(
        &self,
        channel: String,
        recipient: String,
        content: String,
    ) -> HookResult<(String, String, String)> {
        let mut channel = channel;
        let mut recipient = recipient;
        let mut content = content;
        for h in self.children().await {
            match h.on_message_sending(channel, recipient, content).await {
                HookResult::Continue((c, r, ct)) => {
                    channel = c;
                    recipient = r;
                    content = ct;
                }
                HookResult::Cancel(reason) => return HookResult::Cancel(reason),
            }
        }
        HookResult::Continue((channel, recipient, content))
    }

    /// First non-`None` transform wins across children (hermes parity).
    async fn transform_llm_output(&self, text: String) -> Option<String> {
        for h in self.children().await {
            if let Some(replacement) = h.transform_llm_output(text.clone()).await {
                return Some(replacement);
            }
        }
        None
    }
}

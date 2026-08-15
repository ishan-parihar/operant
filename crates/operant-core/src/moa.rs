//! Mixture-of-Agents (MoA) context synthesis — hermes `agent/moa_loop.py`
//! parity.
//!
//! A MoA turn fans out a set of *reference* (advisor) models over a flattened,
//! text-only view of the conversation, then an *aggregator* model synthesizes
//! their advice into concise guidance. The guidance block is injected into the
//! acting agent's context before it answers — the normal agent loop still owns
//! tool calling and turn termination (hermes: "the slash command is
//! deliberately not a model tool").
//!
//! Design notes mirrored from hermes:
//! - References never act: each advisor call carries an advisory system
//!   prompt that reframes the model as an analyst, not the acting agent.
//! - The advisory view contains ZERO `tool`-role messages and ZERO
//!   `tool_calls` arrays — tool calls are rendered inline as text and tool
//!   results folded (head+tail preview) into the preceding assistant turn, so
//!   strict providers never reject unproduced tool payloads.
//! - The advisory view always ends on a `user` turn (append a synthetic
//!   judge-the-state marker when needed) so Anthropic-style providers don't
//!   treat a trailing assistant turn as a prefill.
//! - A failed reference becomes a labelled note; the aggregator runs over the
//!   surviving references. If EVERY reference failed, the aggregator call is
//!   skipped and the turn degrades to single-model mode with a notice.
//! - The aggregator synthesis is never capped (no `max_tokens`), matching
//!   hermes (`reference_max_tokens` applies only to the reference fan-out).

use std::time::Duration;

use futures::future::join_all;

use crate::client::{ChatResponse, ClientConfig, Message, OpenAIClient, Role};
use crate::config::AuxiliaryModelConfig;
use crate::error::Result;

/// System prompt prepended to every reference-model call. References are
/// advisory — they do NOT act, call tools, or own the task (hermes
/// `_REFERENCE_SYSTEM_PROMPT`).
pub const ADVISORY_SYSTEM_PROMPT: &str = "You are a reference advisor in a Mixture of Agents (MoA) process. You are \
NOT the acting agent and you do NOT execute anything: you cannot call tools, run commands, browse, or access files, \
repositories, or URLs, and you should not try to or apologize for being unable to. A separate aggregator/orchestrator \
model holds those capabilities and will take the actual actions.

CRITICAL: You must NEVER claim or imply that you have executed a command, downloaded a file, accessed a URL, or \
performed any action. You can only analyze and advise based on the conversation context.

The conversation below is the current state of a task handled by that acting agent. Your job is to give your most \
intelligent analysis of that state: understand the goal, reason about the problem, and advise on what to do next. \
Surface the best approach, concrete next steps and tool-use strategy, likely pitfalls and risks, and anything the \
acting agent may have missed or gotten wrong. Assume any referenced files, URLs, or systems exist and reason about \
them from the context given rather than asking for access.

Respond with your advice directly — no preamble, no disclaimers about tools or access. Your response is private \
guidance handed to the aggregator, not an answer shown to the user. NEVER claim to have executed anything.";

/// Synthetic user marker appended when the advisory view would otherwise end
/// on an assistant turn (hermes `_ADVISORY_INSTRUCTION`).
pub const ADVISORY_INSTRUCTION: &str = "[The conversation above is the current state of the task. Give your most \
intelligent judgement: what is going on, what should happen next, what risks or mistakes you see, and how the acting \
agent should proceed.]";

/// Per-tool-result character budget for the advisory view (hermes
/// `_REFERENCE_TOOL_RESULT_BUDGET`). The acting agent always gets the full
/// transcript; this only shapes the disposable advisory copy.
const ADVISORY_TOOL_RESULT_BUDGET: usize = 4000;

/// Head+tail preview of a tool result for the advisory view — keeps the first
/// and last halves of the budget with an omitted-count marker between them.
fn truncate_tool_result(text: &str) -> String {
    if text.len() <= ADVISORY_TOOL_RESULT_BUDGET {
        return text.to_string();
    }
    let half = ADVISORY_TOOL_RESULT_BUDGET / 2;
    let omitted = text.len() - 2 * half;
    let mut out = String::with_capacity(ADVISORY_TOOL_RESULT_BUDGET + 32);
    out.push_str(&text[..half]);
    out.push_str(&format!("\n[... {omitted} chars omitted ...]\n"));
    out.push_str(&text[text.len() - half..]);
    out
}

/// Render one assistant turn's `tool_calls` as readable text lines.
fn render_tool_calls(tool_calls: &[crate::client::ToolCall]) -> String {
    let mut lines = Vec::new();
    for tc in tool_calls {
        let name = &tc.function.name;
        let args = tc.function.arguments.as_str();
        if args.is_empty() {
            lines.push(format!("[called tool: {name}]"));
        } else {
            lines.push(format!("[called tool: {name}({args})]"));
        }
    }
    lines.join("\n")
}

/// Build a flattened, text-only advisory view of the conversation for
/// reference models (hermes `_reference_messages`).
///
/// - system prompt: dropped (not advisory signal).
/// - assistant turns: kept; `tool_calls` rendered inline as
///   `[called tool: name(args)]` text.
/// - `tool`-role results: folded (head+tail preview) into the preceding
///   assistant turn as `[tool result: ...]`.
/// - The view always ends on a `user` turn (append the synthetic advisory
///   marker when needed) so Anthropic-style providers never see a trailing
///   assistant prefill.
pub fn build_advisory_messages(messages: &[Message]) -> Vec<Message> {
    let mut rendered: Vec<Message> = Vec::new();
    let mut last_user_text: Option<String> = None;

    for msg in messages {
        match msg.role {
            Role::System => continue,
            Role::User => {
                let text = msg.content.trim();
                if text.is_empty() {
                    continue;
                }
                last_user_text = Some(msg.content.clone());
                rendered.push(Message::user(&msg.content));
            }
            Role::Assistant => {
                let mut parts: Vec<String> = Vec::new();
                if !msg.content.trim().is_empty() {
                    parts.push(msg.content.trim().to_string());
                }
                if let Some(ref tool_calls) = msg.tool_calls
                    && !tool_calls.is_empty()
                {
                    let calls = render_tool_calls(tool_calls);
                    if !calls.is_empty() {
                        parts.push(calls);
                    }
                }
                if !parts.is_empty() {
                    rendered.push(Message::assistant(parts.join("\n")));
                }
            }
            Role::Tool => {
                // Fold the tool result into the preceding assistant turn so
                // the reference sees what came back without a tool-role
                // message it never produced.
                let block = format!("[tool result: {}]", truncate_tool_result(&msg.content));
                if let Some(last) = rendered.last_mut()
                    && last.role == Role::Assistant
                {
                    last.content.push('\n');
                    last.content.push_str(&block);
                } else {
                    rendered.push(Message::assistant(block));
                }
            }
        }
    }

    // End on a user turn: append the synthetic advisory request rather than
    // deleting the agent's latest assistant context.
    if rendered.last().is_some_and(|m| m.role == Role::Assistant) {
        rendered.push(Message::user(ADVISORY_INSTRUCTION));
    }

    if rendered.is_empty()
        && let Some(fallback) = last_user_text
    {
        return vec![Message::user(fallback)];
    }
    rendered
}

/// One reference-model output: a label plus the advisor's text.
#[derive(Debug, Clone)]
pub struct ReferenceOutput {
    pub label: String,
    pub text: String,
}

impl ReferenceOutput {
    /// Whether this output is an internal failure/skip sentinel — not real
    /// advice, so it never reaches the aggregator prompt.
    pub fn is_failed(&self) -> bool {
        let sentinel = self.text.trim_start().to_ascii_lowercase();
        sentinel.starts_with("[failed:") || sentinel.starts_with("[skipped:")
    }
}

/// Join successful reference outputs as labelled blocks (hermes `joined`).
pub fn format_reference_blocks(outputs: &[ReferenceOutput]) -> String {
    let mut blocks = Vec::new();
    for (idx, out) in outputs.iter().enumerate() {
        blocks.push(format!(
            "Reference {} — {}:\n{}",
            idx + 1,
            out.label,
            out.text
        ));
    }
    blocks.join("\n\n")
}

/// Build the final guidance block injected into the acting agent's context
/// (hermes `aggregate_moa_context` return shape).
pub fn build_guidance_block(
    aggregator_label: &str,
    reference_labels: &[String],
    synthesis: &str,
) -> String {
    let refs = if reference_labels.is_empty() {
        "(none)".to_string()
    } else {
        reference_labels.join(", ")
    };
    format!(
        "[Mixture of Agents context — use this as private guidance for the normal agent loop. \
         You may call tools, continue reasoning, or finish normally.]\n\
         Aggregator: {aggregator_label}\n\
         References: {refs}\n\n\
         {synthesis}"
    )
}

/// Label for a MoA slot (provider:model, mirroring hermes `_slot_label`).
fn slot_label(slot: &AuxiliaryModelConfig) -> String {
    let provider = slot.provider.as_deref().unwrap_or("").trim();
    let model = slot.model.as_deref().unwrap_or("").trim();
    if provider.is_empty() {
        model.to_string()
    } else if model.is_empty() {
        provider.to_string()
    } else {
        format!("{provider}:{model}")
    }
}

/// Build a client for one MoA slot: seed from the main client config, then
/// override base_url/api_key when the slot pins them.
fn slot_client(base: &ClientConfig, slot: &AuxiliaryModelConfig) -> OpenAIClient {
    let mut cfg = base.clone();
    if let Some(base_url) = &slot.base_url
        && !base_url.trim().is_empty()
    {
        cfg.base_url = base_url.clone();
    }
    if let Some(api_key) = &slot.api_key
        && !api_key.trim().is_empty()
    {
        cfg.api_key = Some(api_key.clone());
    }
    OpenAIClient::new(cfg)
}

/// Call ONE reference model. Never raises: a failed reference becomes a
/// labelled `[failed: ...]` note so the aggregator can still act with partial
/// context (hermes `_run_reference`).
async fn run_reference(
    slot: &AuxiliaryModelConfig,
    advisory: &[Message],
    base: &ClientConfig,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    timeout: Duration,
) -> ReferenceOutput {
    let label = slot_label(slot);
    let model = slot.model.as_deref().unwrap_or("").trim().to_string();
    if model.is_empty() {
        return ReferenceOutput {
            label,
            text: "[failed: no model configured for reference slot]".to_string(),
        };
    }
    let client = slot_client(base, slot);
    let mut msgs = vec![Message::system(ADVISORY_SYSTEM_PROMPT)];
    msgs.extend_from_slice(advisory);

    let result = tokio::time::timeout(
        timeout,
        client.chat(&model, &msgs, None, max_tokens, temperature),
    )
    .await;
    match result {
        Ok(Ok(resp)) => {
            let text = extract_text(&resp);
            if text.is_empty() {
                ReferenceOutput {
                    label,
                    text: "[failed: empty reference response]".to_string(),
                }
            } else {
                ReferenceOutput { label, text }
            }
        }
        Ok(Err(e)) => ReferenceOutput {
            label,
            text: format!("[failed: {e}]"),
        },
        Err(_) => ReferenceOutput {
            label,
            text: format!("[failed: reference timed out after {}s]", timeout.as_secs()),
        },
    }
}

/// Extract the text of a chat completion response.
pub fn extract_text(resp: &ChatResponse) -> String {
    resp.choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Run a full MoA turn: fan out the references in parallel over the advisory
/// view, then synthesize with the aggregator (hermes `aggregate_moa_context`).
///
/// Returns `Ok(None)` when no references are configured (MoA inert). The
/// returned guidance block is injected into the acting agent's context; when
/// every reference failed, the block carries a degraded notice instead of
/// aggregator output (no wasted synthesis call over zero real advice).
#[expect(
    clippy::too_many_arguments,
    reason = "one knob per hermes aggregate_moa_context parameter; callers pass a config-derived set"
)]
pub async fn aggregate_moa_context(
    user_prompt: &str,
    api_messages: &[Message],
    references: &[AuxiliaryModelConfig],
    aggregator: Option<&AuxiliaryModelConfig>,
    main_model: &str,
    base: &ClientConfig,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    timeout: Duration,
) -> Result<Option<String>> {
    let enabled_refs: Vec<&AuxiliaryModelConfig> = references.iter().collect();
    if enabled_refs.is_empty() {
        return Ok(None);
    }

    let advisory = build_advisory_messages(api_messages);
    let labels: Vec<String> = enabled_refs.iter().map(|s| slot_label(s)).collect();

    // Fan out all references in parallel (independent advisory calls).
    let futures = enabled_refs
        .iter()
        .map(|slot| run_reference(slot, &advisory, base, max_tokens, temperature, timeout));
    let outputs: Vec<ReferenceOutput> = join_all(futures).await;

    let successful: Vec<ReferenceOutput> =
        outputs.iter().filter(|o| !o.is_failed()).cloned().collect();
    let failed_labels: Vec<String> = outputs
        .iter()
        .filter(|o| o.is_failed())
        .map(|o| o.label.clone())
        .collect();
    let degraded = if failed_labels.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n[Reference models unavailable: {}]",
            failed_labels.join(", ")
        )
    };

    // Skip the aggregator call when every reference failed — synthesising
    // over zero real advice wastes tokens (hermes parity).
    if successful.is_empty() {
        let notice = if failed_labels.is_empty() {
            "[Reference models unavailable]".to_string()
        } else {
            format!(
                "References: {}\n\n[Reference models unavailable: {}]",
                labels.join(", "),
                failed_labels.join(", ")
            )
        };
        return Ok(Some(build_guidance_block(
            "(aggregator skipped)",
            &labels,
            &notice,
        )));
    }

    let joined = format_reference_blocks(&successful);
    let joined = format!("{joined}{degraded}");

    // Aggregator synthesis — never capped on output tokens (hermes parity).
    let (agg_client, agg_model, agg_label) = match aggregator {
        Some(slot) => {
            let model = slot.model.as_deref().unwrap_or("").trim().to_string();
            if model.is_empty() {
                (
                    slot_client(base, slot),
                    main_model.to_string(),
                    slot_label(slot),
                )
            } else {
                (slot_client(base, slot), model, slot_label(slot))
            }
        }
        None => (
            OpenAIClient::new(base.clone()),
            main_model.to_string(),
            main_model.to_string(),
        ),
    };

    let synth_prompt = format!(
        "You are the aggregator in a Mixture of Agents process. Synthesize the reference \
         responses into concise, actionable guidance for the main agent. Focus on next steps, \
         tool-use strategy, risks, and any disagreements. Do not answer the user directly unless \
         that is all that is needed; produce context the main agent should use in its normal loop.\n\n\
         Original user prompt:\n{user_prompt}\n\n\
         Reference responses:\n{joined}"
    );

    let synthesis = match tokio::time::timeout(
        timeout,
        agg_client.chat(
            &agg_model,
            &[Message::user(synth_prompt)],
            None,
            None,
            temperature,
        ),
    )
    .await
    {
        Ok(Ok(resp)) => extract_text(&resp),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "MoA aggregator call failed — using joined references");
            joined.clone()
        }
        Err(_) => {
            tracing::warn!("MoA aggregator call timed out — using joined references");
            joined.clone()
        }
    };

    Ok(Some(build_guidance_block(&agg_label, &labels, &synthesis)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ToolCall, ToolCallFunction};

    fn msgs_with_tool_flow() -> Vec<Message> {
        let tc = ToolCall {
            id: "call_1".to_string(),
            function: ToolCallFunction {
                name: "file_search".to_string(),
                arguments: r#"{"query":"moa"}"#.to_string(),
            },
        };
        vec![
            Message::system("You are a helpful assistant."),
            Message::user("Find the MoA docs."),
            Message {
                role: Role::Assistant,
                content: "I'll search.".to_string(),
                reasoning: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![tc]),
                extra_content: None,
            },
            Message::tool("call_1", "no results found"),
        ]
    }

    #[test]
    fn advisory_view_flattens_tool_calls_and_results() {
        let view = build_advisory_messages(&msgs_with_tool_flow());
        // System dropped, tool-role dropped — only user/assistant remain.
        assert!(
            view.iter()
                .all(|m| m.role == Role::User || m.role == Role::Assistant),
            "advisory view must contain only user/assistant turns"
        );
        let text: String = view
            .iter()
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("[called tool: file_search"),
            "tool call rendered inline"
        );
        assert!(
            text.contains("[tool result: no results found]"),
            "tool result folded in"
        );
        assert_eq!(
            view.last().unwrap().role,
            Role::User,
            "view ends on a user turn"
        );
    }

    #[test]
    fn advisory_view_appends_synthetic_user_when_ending_assistant() {
        let msgs = vec![Message::user("hi"), Message::assistant("hello")];
        let view = build_advisory_messages(&msgs);
        assert_eq!(view.len(), 3);
        assert_eq!(view.last().unwrap().role, Role::User);
        assert!(
            view.last()
                .unwrap()
                .content
                .contains("intelligent judgement")
        );
    }

    #[test]
    fn advisory_view_drops_system_and_empty_turns() {
        let msgs = vec![
            Message::system("sys"),
            Message::user("   "),
            Message::user("real"),
        ];
        let view = build_advisory_messages(&msgs);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].content, "real");
    }

    #[test]
    fn advisory_view_truncates_huge_tool_results() {
        let big = "x".repeat(10_000);
        let msgs = vec![
            Message::user("q"),
            Message::assistant("a"),
            Message::tool("c1", &big),
        ];
        let view = build_advisory_messages(&msgs);
        let text = &view[1].content;
        assert!(
            text.contains("chars omitted"),
            "big results must be head+tail previewed"
        );
        assert!(text.len() < big.len());
    }

    #[test]
    fn reference_block_formatting_and_failure_filtering() {
        let outputs = [
            ReferenceOutput {
                label: "a:one".to_string(),
                text: "good advice".to_string(),
            },
            ReferenceOutput {
                label: "b:two".to_string(),
                text: "[failed: boom]".to_string(),
            },
        ];
        assert!(outputs[1].is_failed());
        let joined = format_reference_blocks(&[outputs[0].clone()]);
        assert!(joined.contains("Reference 1 — a:one:"));
        assert!(joined.contains("good advice"));
    }

    #[test]
    fn guidance_block_shape() {
        let block = build_guidance_block(
            "main",
            &["a:one".to_string(), "b:two".to_string()],
            "Do X next.",
        );
        assert!(block.starts_with("[Mixture of Agents context"));
        assert!(block.contains("Aggregator: main"));
        assert!(block.contains("References: a:one, b:two"));
        assert!(block.ends_with("Do X next."));
    }

    #[test]
    fn slot_label_handles_partial_configs() {
        let full = AuxiliaryModelConfig {
            provider: Some("openai".to_string()),
            model: Some("gpt-4o-mini".to_string()),
            ..Default::default()
        };
        assert_eq!(slot_label(&full), "openai:gpt-4o-mini");
        let model_only = AuxiliaryModelConfig {
            model: Some("gpt-4o-mini".to_string()),
            ..Default::default()
        };
        assert_eq!(slot_label(&model_only), "gpt-4o-mini");
        let empty = AuxiliaryModelConfig::default();
        assert!(slot_label(&empty).is_empty());
    }

    #[test]
    fn all_failed_references_skip_aggregator_with_notice() {
        // build_guidance_block with an empty synthesis + failed labels:
        // the degraded-notice path is pure string shaping, exercised here.
        let block = build_guidance_block(
            "(aggregator skipped)",
            &["a:one".to_string()],
            "References: a:one\n\n[Reference models unavailable: a:one]",
        );
        assert!(block.contains("(aggregator skipped)"));
        assert!(block.contains("Reference models unavailable"));
    }
}

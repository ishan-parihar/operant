//! `streaming` — extracted verbatim from the former loop_.rs monolith.
//! Re-exported from `loop_` so every import path is unchanged.

/// CLI channel factory, injected by the binary. Returns a `Box<dyn Channel>` for interactive mode.
use super::*;

pub async fn consume_provider_streaming_response(
    provider: &dyn Provider,
    messages: &[ChatMessage],
    request_tools: Option<&[crate::tools::ToolSpec]>,
    model: &str,
    temperature: f64,
    cancellation_token: Option<&CancellationToken>,
    on_delta: Option<&tokio::sync::mpsc::Sender<DraftEvent>>,
) -> Result<StreamedChatOutcome> {
    let mut provider_stream = provider.stream_chat(
        ChatRequest {
            messages,
            tools: request_tools,
        },
        model,
        Some(temperature),
        operant_providers::traits::StreamOptions::new(true),
    );
    let mut outcome = StreamedChatOutcome::default();
    let mut delta_sender = on_delta;
    let mut suppress_forwarding = false;
    let mut marker_window = String::new();

    loop {
        let next_chunk = if let Some(token) = cancellation_token {
            tokio::select! {
                () = token.cancelled() => return Err(ToolLoopCancelled.into()),
                chunk = provider_stream.next() => chunk,
            }
        } else {
            provider_stream.next().await
        };

        let Some(event_result) = next_chunk else {
            break;
        };

        let event = event_result.map_err(|err| anyhow::anyhow!("provider stream error: {err}"))?;
        match event {
            StreamEvent::Final => break,
            StreamEvent::Usage(usage) => {
                outcome.usage = Some(usage);
            }
            StreamEvent::ToolCall(tool_call) => {
                outcome.tool_calls.push(tool_call);
                suppress_forwarding = true;
            }
            StreamEvent::PreExecutedToolCall { .. } | StreamEvent::PreExecutedToolResult { .. } => {
                // Pre-executed tool events are for observability only.
                // They are forwarded to the gateway via turn_streamed but
                // do not affect the agent's tool dispatch loop.
            }
            StreamEvent::TextDelta(chunk) => {
                // Reasoning/thinking deltas arrive on the same `TextDelta`
                // event as plain text but populate `chunk.reasoning` instead
                // of `chunk.delta`. They must be captured into the outcome
                // even when `chunk.delta` is empty — otherwise providers
                // that require reasoning to round-trip on subsequent turns
                // (DeepSeek V4 thinking mode; see #6059) reject the next
                // request with a 400. Reasoning is never forwarded as a
                // visible response delta — it is the model's internal
                // monologue, kept for replay only.
                if let Some(reasoning) = chunk.reasoning.as_deref()
                    && !reasoning.is_empty()
                {
                    outcome.reasoning_content.push_str(reasoning);
                }

                if chunk.delta.is_empty() {
                    continue;
                }

                outcome.response_text.push_str(&chunk.delta);
                marker_window.push_str(&chunk.delta);

                if marker_window.len() > STREAM_TOOL_MARKER_WINDOW_CHARS {
                    let keep_from = marker_window.len() - STREAM_TOOL_MARKER_WINDOW_CHARS;
                    let boundary = marker_window
                        .char_indices()
                        .find(|(idx, _)| *idx >= keep_from)
                        .map_or(0, |(idx, _)| idx);
                    marker_window.drain(..boundary);
                }

                if !suppress_forwarding && {
                    let lowered = marker_window.to_ascii_lowercase();
                    lowered.contains("<tool_call")
                        || lowered.contains("<toolcall")
                        || lowered.contains("\"tool_calls\"")
                } {
                    suppress_forwarding = true;
                }

                if suppress_forwarding {
                    continue;
                }

                if let Some(tx) = delta_sender {
                    outcome.forwarded_live_deltas = true;
                    if tx.send(StreamDelta::Text(chunk.delta)).await.is_err() {
                        delta_sender = None;
                    }
                }
            }
        }
    }

    Ok(outcome)
}

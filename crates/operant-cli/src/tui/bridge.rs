use tokio::sync::mpsc;
use crate::tui::adapter_types::query::{QueryEvent, StreamEvent, UsageInfo};
use operant_core::agent::AgentEvent;

pub fn spawn_bridge() -> (
    mpsc::Sender<AgentEvent>,
    mpsc::Receiver<QueryEvent>,
) {
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(256);
    let (query_tx, query_rx) = mpsc::channel::<QueryEvent>(256);

    tokio::spawn(async move {
        let mut pending_usage: Option<UsageInfo> = None;

        while let Some(event) = agent_rx.recv().await {
            let query_event = match event {
                AgentEvent::Thinking { content } => {
                    Some(QueryEvent::Stream(StreamEvent::ContentBlockDelta {
                        delta: format!("[thinking] {}", content),
                    }))
                }
                AgentEvent::Reasoning { text } => {
                    Some(QueryEvent::Stream(StreamEvent::ContentBlockDelta {
                        delta: format!("[reasoning] {}", text),
                    }))
                }
                AgentEvent::Content { text } => {
                    Some(QueryEvent::Stream(StreamEvent::ContentBlockDelta {
                        delta: text,
                    }))
                }
                AgentEvent::ToolStart { name, arguments } => {
                    let tool_id = format!("tool_{}_{}", name, chrono::Utc::now().timestamp_millis());
                    Some(QueryEvent::ToolStart {
                        tool_name: name,
                        tool_id,
                        input_json: arguments,
                    })
                }
                AgentEvent::ToolComplete { result } => {
                    let content = if result.success {
                        result.content.clone()
                    } else {
                        result.error.clone().unwrap_or_else(|| "Unknown error".to_string())
                    };
                    let is_error = !result.success;
                    Some(QueryEvent::ToolEnd {
                        tool_id: result.tool_call_id,
                        tool_name: result.name,
                        result: content,
                        is_error,
                    })
                }
                AgentEvent::ToolError { name, error } => {
                    Some(QueryEvent::ToolEnd {
                        tool_id: String::new(),
                        tool_name: name,
                        result: error,
                        is_error: true,
                    })
                }
                AgentEvent::Done { message } => {
                    Some(QueryEvent::TurnComplete {
                        turn: 0,
                        stop_reason: "end_turn".to_string(),
                        usage: pending_usage.take(),
                    })
                }
                AgentEvent::Error { error } => {
                    Some(QueryEvent::Error(error))
                }
                AgentEvent::Usage { input_tokens, output_tokens, total_tokens: _ } => {
                    pending_usage = Some(UsageInfo {
                        input_tokens,
                        output_tokens,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                        total_cost: 0.0,
                    });
                    None
                }
                AgentEvent::IterationComplete { .. } => None,
                AgentEvent::ToolPermissionRequest { tool_name, description, .. } => {
                    Some(QueryEvent::Status(format!(
                        "Permission needed: {} — {}",
                        tool_name,
                        description
                    )))
                }
            };

            if let Some(qe) = query_event {
                if query_tx.send(qe).await.is_err() {
                    break;
                }
            }
        }
    });

    (agent_tx, query_rx)
}

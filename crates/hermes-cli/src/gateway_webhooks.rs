use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use hermes_core::gateway::{IncomingMessage, PlatformAdapter};
use tokio::sync::mpsc;

/// Shared state for the webhook listener.
struct WebhookState {
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    message_tx: mpsc::UnboundedSender<IncomingMessage>,
}

/// Start the webhook listener as an axum HTTP server.
///
/// This receives incoming webhook requests from Telegram/Discord/Slack
/// and dispatches them to the appropriate platform adapter.
pub async fn start_webhook_listener(
    addr: &str,
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    message_tx: mpsc::UnboundedSender<IncomingMessage>,
) -> anyhow::Result<()> {
    let state = Arc::new(WebhookState {
        adapters,
        message_tx,
    });

    let app = Router::new()
        .route("/webhook/{platform}", post(handle_webhook))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Handle an incoming webhook request for a specific platform.
async fn handle_webhook(
    State(state): State<Arc<WebhookState>>,
    Path(platform): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> &'static str {
    if let Some(adapter) = state.adapters.get(&platform) {
        match adapter.handle_update(payload).await {
            Ok(Some(msg)) => {
                tracing::info!("Webhook message from {}: {}", platform, msg.content);
                state.message_tx.send(msg).ok();
                "ok"
            }
            Ok(None) => "ignored",
            Err(e) => {
                tracing::error!("Webhook handler error for {}: {}", platform, e);
                "error"
            }
        }
    } else {
        tracing::warn!("Webhook received for unknown platform: {}", platform);
        "unknown platform"
    }
}

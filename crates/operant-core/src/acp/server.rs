//! Async stdio transport for the ACP (Agent Control Protocol) server.
//!
//! Reads JSON-RPC requests from stdin (line-delimited) and writes responses
//! to stdout using tokio's async I/O to avoid blocking the event loop.

use super::{
    AcpHandler, RpcNotification, RpcResponse, dispatch, parse_request, serialize_response,
    validate_request,
};
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Run the ACP server over stdio using async tokio I/O.
///
/// This function:
/// 1. Reads JSON-RPC request lines from stdin
/// 2. Dispatches them to the provided `handler`
/// 3. Writes JSON-RPC responses to stdout
/// 4. Returns gracefully on EOF or `stop` method
///
/// # Arguments
///
/// * `handler` - The ACP handler that provides agent access
///
/// # Returns
///
/// `Ok(())` on normal shutdown, or an error if I/O fails.
pub async fn run_stdio_server(handler: Arc<dyn AcpHandler>) -> Result<()> {
    // Session state lives for the process, not the request: a client holds a
    // sessionId across `session/new` and a later `session/prompt`.
    let sessions = crate::acp::SessionRegistry::new();
    serve_connection(&handler, &sessions, tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serve the ACP protocol over a UNIX socket (RT-513).
///
/// The socket is the SEAM that lets many clients share ONE process — and
/// therefore one registry, one database and one MCP child set. Every
/// connection is served on its own task, and the sessions are shared
/// across connections: the daemon owns the sessions, not the client.
///
/// The socket file is removed on the way out, so a restarted daemon can
/// bind the same path.
pub async fn run_socket_server(handler: Arc<dyn AcpHandler>, path: &std::path::Path) -> Result<()> {
    // A stale socket file from a killed daemon would make bind() fail with
    // "address already in use" — and the daemon would refuse to start for a
    // file nobody is listening on any more.
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|e| anyhow::anyhow!("Failed to bind the ACP socket {}: {e}", path.display()))?;
    tracing::info!(socket = %path.display(), "ACP socket server listening");

    let sessions = std::sync::Arc::new(crate::acp::SessionRegistry::new());
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::error!(error = %e, "ACP socket accept failed");
                continue;
            }
        };
        let handler = handler.clone();
        let sessions = sessions.clone();
        tokio::spawn(async move {
            // The same socket is both halves: split it so the reader and
            // the writer own one side each.
            let (read_half, write_half) = tokio::io::split(stream);
            if let Err(e) = serve_connection(&handler, &sessions, read_half, write_half).await {
                tracing::debug!(error = %e, "ACP socket connection ended with an error");
            }
        });
    }
}

/// Serve ONE client connection: read JSON-RPC request lines, write the
/// responses back.
async fn serve_connection<R, W>(
    handler: &Arc<dyn AcpHandler>,
    sessions: &crate::acp::SessionRegistry,
    reader: R,
    mut writer: W,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                // EOF on stdin — normal shutdown
                break;
            }
            Err(e) => {
                // I/O error on stdin
                tracing::error!("ACP stdin read error: {}", e);
                break;
            }
        };

        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        // Parse the JSON-RPC request
        let request = match parse_request(&trimmed) {
            Ok(req) => req,
            Err(e) => {
                let err_resp = RpcResponse::error(serde_json::Value::Null, -32700, e);
                write_response(&mut writer, &err_resp).await?;
                continue;
            }
        };

        // Validate the JSON-RPC 2.0 framing (version member, id type).
        // (R18: a wrong/missing `jsonrpc` version or an invalid id type was
        // previously accepted silently; per spec these are -32600 Invalid
        // Request.)
        if let Err(code) = validate_request(&request) {
            let err_resp = RpcResponse::error(
                request.response_id(),
                code,
                "Invalid Request: `jsonrpc` must be \"2.0\" and `id` must be a string, number, or null",
            );
            write_response(&mut writer, &err_resp).await?;
            continue;
        }

        // Dispatch and check if we should shut down. `session/update` carries no
        // `id` and must never be answered, so notifications and the response are
        // separate channels rather than one return value.
        let outcome = dispatch(&request, handler.as_ref(), sessions).await;

        // Notifications are written BEFORE the response. For `session/prompt` the
        // agent's whole turn is already complete by this point, so emitting the
        // chunk first means a client sees the text arrive before the turn is
        // reported finished — the ordering a streaming client expects.
        for notification in &outcome.notifications {
            write_notification(&mut writer, notification).await?;
        }

        if let Some(response) = &outcome.response
            && !request.is_notification()
        {
            write_response(&mut writer, response).await?;
        }

        if outcome.should_shutdown {
            break;
        }
    }

    Ok(())
}

/// Write a JSON-RPC notification to stdout as a single line.
///
/// Notifications omit `id` entirely — that absence is what makes them
/// notifications rather than unanswered requests, so it must not be
/// serialised as `"id": null`.
async fn write_notification<W>(writer: &mut W, notification: &RpcNotification) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let json = serde_json::to_string(notification)
        .map_err(|e| anyhow::anyhow!("Failed to serialize ACP notification: {}", e))?;

    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

/// Write a JSON-RPC response to stdout as a single line.
async fn write_response<W>(writer: &mut W, response: &RpcResponse) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let json = serialize_response(response)
        .map_err(|e| anyhow::anyhow!("Failed to serialize ACP response: {}", e))?;

    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_response_to_vec() {
        // We can't easily test the full server loop without mocking stdin,
        // but we can verify response serialization works.
        let resp = RpcResponse::success(serde_json::json!(1), serde_json::json!("pong"));
        let json = serialize_response(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["result"], "pong");
        assert_eq!(parsed["id"], 1);
    }
}

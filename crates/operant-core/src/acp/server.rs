//! Async stdio transport for the ACP (Agent Control Protocol) server.
//!
//! Reads JSON-RPC requests from stdin (line-delimited) and writes responses
//! to stdout using tokio's async I/O to avoid blocking the event loop.

use super::{
    AcpHandler, RpcResponse, dispatch, parse_request, serialize_response, validate_request,
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
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

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
                write_response(&err_resp).await?;
                continue;
            }
        };

        // Validate the JSON-RPC 2.0 framing (version member, id type).
        // (R18: a wrong/missing `jsonrpc` version or an invalid id type was
        // previously accepted silently; per spec these are -32600 Invalid
        // Request.)
        if let Err(code) = validate_request(&request) {
            let err_resp = RpcResponse::error(
                request.id.clone(),
                code,
                "Invalid Request: `jsonrpc` must be \"2.0\" and `id` must be a string, number, or null",
            );
            write_response(&err_resp).await?;
            continue;
        }

        // Dispatch and check if we should shut down. JSON-RPC notifications
        // (no `id`) must not receive a response.
        let (response, should_shutdown) = dispatch(&request, &*handler).await;
        if !request.is_notification() {
            write_response(&response).await?;
        }

        if should_shutdown {
            break;
        }
    }

    Ok(())
}

/// Write a JSON-RPC response to stdout as a single line.
async fn write_response(response: &RpcResponse) -> Result<()> {
    let json = serialize_response(response)
        .map_err(|e| anyhow::anyhow!("Failed to serialize ACP response: {}", e))?;

    let mut stdout = tokio::io::stdout();
    stdout.write_all(json.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
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

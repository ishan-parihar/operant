use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::error::{Error, Result};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub async fn send_cdp_command(url: &str, command: &Value) -> Result<Value> {
    let (ws_stream, _): (WsStream, _) =
        connect_async(url).await.map_err(|e| Error::ToolExecution {
            name: "cdp".into(),
            source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, e.to_string())
                .into(),
        })?;

    let (mut write, mut read) = ws_stream.split();

    let cmd_str = serde_json::to_string(command).map_err(|e| Error::ToolExecution {
        name: "cdp".into(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()).into(),
    })?;

    write
        .send(Message::Text(cmd_str.into()))
        .await
        .map_err(|e| Error::ToolExecution {
            name: "cdp".into(),
            source: e.into(),
        })?;

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| Error::ToolExecution {
            name: "cdp".into(),
            source: e.into(),
        })?;

        if let Message::Text(text) = msg {
            let response: Value =
                serde_json::from_str(&text).map_err(|e| Error::ToolExecution {
                    name: "cdp".into(),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                        .into(),
                })?;

            if let Some(error) = response.get("error") {
                return Err(Error::ToolExecution {
                    name: "cdp".into(),
                    source: std::io::Error::new(std::io::ErrorKind::Other, error.to_string())
                        .into(),
                });
            }

            if response.get("result").is_some() || response.get("id").is_some() {
                return Ok(response);
            }
        }
    }

    Err(Error::ToolExecution {
        name: "cdp".into(),
        source: std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "WebSocket connection closed without response",
        )
        .into(),
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_cdp_command_serializes_correctly() {
        // Test that CDP commands serialize to valid JSON
        let command = serde_json::json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "1+1"
            }
        });
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains("Runtime.evaluate"));
        assert!(json.contains("1+1"));
    }
}

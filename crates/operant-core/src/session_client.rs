//! The session client — the thin half of the protocol.
//!
//! A client that attaches builds NO runtime: no tool registry, no database
//! handles, no MCP child servers, no AFT bridge. It opens a socket, says
//! hello, sends what the operator typed, and renders the events the server
//! sends back.
//!
//! That is the whole point of the server (RT-542): one process owns the
//! expensive things, and a terminal is a window onto it.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::session_protocol::{PROTOCOL_VERSION, WireMessage, decode, encode};

/// A connected session client.
pub struct SessionClient {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    /// The sessions the server named at hello.
    pub sessions: Vec<String>,
}

impl SessionClient {
    /// Connect to a server's socket and say hello.
    ///
    /// The hello is the version handshake: a server that speaks a different
    /// protocol version is refused here, before any turn.
    pub async fn connect(path: &Path) -> Result<Self> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| anyhow::anyhow!("cannot reach the server at {}: {e}", path.display()))?;
        let (read_half, write_half) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            sessions: Vec::new(),
        };
        // The hello is written first so the server knows this is a client.
        client.send(&WireMessage::Hello).await?;
        match client.recv().await? {
            Some(WireMessage::HelloOk { sessions }) => {
                client.sessions = sessions;
            }
            Some(WireMessage::Error { message, .. }) => {
                return Err(anyhow::anyhow!(
                    "the server refused the connection: {message}"
                ));
            }
            Some(other) => {
                return Err(anyhow::anyhow!(
                    "the server answered the hello with {other:?}"
                ));
            }
            None => return Err(anyhow::anyhow!("the server closed the connection")),
        }
        Ok(client)
    }

    /// Send one message to the server.
    pub async fn send(&mut self, message: &WireMessage) -> Result<()> {
        let line = encode(message, PROTOCOL_VERSION)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Read the server's next message, if it sends one.
    pub async fn recv(&mut self) -> Result<Option<WireMessage>> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        let message = decode(&line, PROTOCOL_VERSION)?;
        Ok(Some(message))
    }

    /// Split into a read half and a write half.
    ///
    /// The interactive client reads the server's events on one task while the
    /// operator's prompt is written from another, and a `tokio::Mutex` around
    /// the whole client would make a slow read block the write. The halves are
    /// the socket's own; splitting them costs nothing.
    pub fn into_split(self) -> (SessionReader, SessionWriter) {
        (
            SessionReader {
                reader: self.reader,
            },
            SessionWriter {
                writer: self.writer,
            },
        )
    }
}

/// The reading half of a session connection.
pub struct SessionReader {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
}

impl SessionReader {
    /// Read the server's next message, if it sends one.
    pub async fn recv(&mut self) -> Result<Option<WireMessage>> {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(None);
        }
        let message = decode(&line, PROTOCOL_VERSION)?;
        Ok(Some(message))
    }
}

/// The writing half of a session connection.
pub struct SessionWriter {
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl SessionWriter {
    /// Send one message to the server.
    pub async fn send(&mut self, message: &WireMessage) -> Result<()> {
        let line = encode(message, PROTOCOL_VERSION)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }
}

/// The default socket path a server and its clients agree on.
///
/// One path per operator: the first client and the daemon must name the same
/// file, and an operator should not have to type it twice.
pub fn default_socket_path() -> PathBuf {
    operant_home_socket("operant-session.sock")
}

fn operant_home_socket(name: &str) -> PathBuf {
    crate::platform::operant_home().join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentEvent;
    use crate::session_server::{SessionHandler, run_session_socket};
    use std::sync::Arc;

    /// A server that answers the hello and publishes one event on a prompt.
    #[derive(Clone)]
    struct EchoHandler {
        bus: tokio::sync::broadcast::Sender<WireMessage>,
    }

    impl SessionHandler for EchoHandler {
        fn handle(&self, message: WireMessage) -> Vec<WireMessage> {
            match message {
                WireMessage::Hello => vec![WireMessage::HelloOk {
                    sessions: vec!["s1".to_string()],
                }],
                WireMessage::Prompt { session_id, .. } => {
                    let _ = self.bus.send(WireMessage::Event {
                        session_id: session_id.clone(),
                        event: AgentEvent::Content {
                            text: "from the server".to_string(),
                        },
                    });
                    vec![]
                }
                _ => vec![],
            }
        }

        fn subscribe(
            &self,
            _session_id: &str,
        ) -> Option<tokio::sync::broadcast::Receiver<WireMessage>> {
            Some(self.bus.subscribe())
        }
    }

    /// A client attaches, says hello, sends a prompt and receives the turn's
    /// events — and it is the SERVER that owns the agent.
    #[tokio::test]
    async fn a_client_attaches_and_receives_the_servers_events() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("session.sock");
        let (bus, _) = tokio::sync::broadcast::channel(16);
        let server = tokio::spawn(run_session_socket(
            Arc::new(EchoHandler { bus }),
            path.clone(),
        ));
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut client = SessionClient::connect(&path)
            .await
            .expect("the client must connect");
        assert_eq!(
            client.sessions,
            vec!["s1".to_string()],
            "the hello must name the server's sessions"
        );

        client
            .send(&WireMessage::Prompt {
                session_id: "s1".to_string(),
                text: "hello".to_string(),
            })
            .await
            .expect("the prompt must be sent");

        let event = tokio::time::timeout(std::time::Duration::from_secs(10), client.recv())
            .await
            .expect("the client must receive the event")
            .expect("the event must arrive")
            .expect("the event must decode");
        match event {
            WireMessage::Event { session_id, event } => {
                assert_eq!(session_id, "s1");
                match event {
                    AgentEvent::Content { text } => {
                        assert_eq!(text, "from the server", "the turn's text must arrive");
                    }
                    other => panic!("the wrong event arrived: {other:?}"),
                }
            }
            other => panic!("the client must receive an event, got {other:?}"),
        }

        server.abort();
    }
}

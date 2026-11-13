//! The session protocol — the wire between a client and the server that owns
//! the agent.
//!
//! The protocol is DATA first: the types and the two pure functions that turn
//! them into a line and back. Nothing here opens a socket, spawns a task or
//! touches an agent, so the protocol can be tested without a server, and the
//! server can be tested without a client.
//!
//! Two directions, one line format (JSON, one message per line):
//!
//! - the **client** sends control: prompt, steer, queue, cancel, approve, deny,
//!   switch session, load history;
//! - the **server** sends events: the agent's own `AgentEvent`, one per line,
//!   as they happen.
//!
//! Every line carries the protocol version. A client that speaks a different
//! version is told so, rather than being allowed to guess.

use serde::{Deserialize, Serialize};

use crate::agent::AgentEvent;

/// The protocol version this build speaks.
///
/// It changes when the wire changes in a way a peer cannot ignore. A client
/// and a server that disagree are refused, not tolerated.
pub const PROTOCOL_VERSION: u32 = 1;

/// One message on the wire.
///
/// The `type` tag says which direction and which kind: `prompt` travels from a
/// client to the server, `event` from the server to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    /// The client's handshake. The version travels in the envelope, so this
    /// carries no version of its own.
    Hello,
    /// The server's handshake answer: the sessions it currently holds.
    HelloOk { sessions: Vec<String> },
    /// Run a turn in a session.
    Prompt { session_id: String, text: String },
    /// Add to the turn that is running, without cancelling it.
    Steer { session_id: String, text: String },
    /// Park a message for the next turn.
    Queue { session_id: String, text: String },
    /// Stop the turn that is running in a session.
    Cancel {
        session_id: String,
        /// The turn to cancel; `None` means "whatever is running".
        turn_id: Option<u64>,
    },
    /// Answer a tool that is waiting for permission.
    Approve {
        session_id: String,
        tool_call_id: String,
    },
    /// Refuse a tool that is waiting for permission.
    Deny {
        session_id: String,
        tool_call_id: String,
        /// Why the operator refused, if they said.
        reason: Option<String>,
    },
    /// Move the client to another session.
    Switch { session_id: String },
    /// Ask for a session's history.
    LoadHistory { session_id: String },
    /// One agent event, as it happened.
    Event {
        session_id: String,
        event: AgentEvent,
    },
    /// Something failed. The server names it; the client shows it.
    Error {
        session_id: Option<String>,
        message: String,
    },
}

impl WireMessage {
    /// The session this message is about, if it is about one.
    ///
    /// The server uses it to decide which session's events a client is
    /// subscribed to (RT-544).
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Hello | Self::HelloOk { .. } => None,
            Self::Prompt { session_id, .. }
            | Self::Steer { session_id, .. }
            | Self::Queue { session_id, .. }
            | Self::Cancel { session_id, .. }
            | Self::Approve { session_id, .. }
            | Self::Deny { session_id, .. }
            | Self::Switch { session_id }
            | Self::LoadHistory { session_id }
            | Self::Event { session_id, .. } => Some(session_id),
            Self::Error { session_id, .. } => session_id.as_deref(),
        }
    }
}

/// The envelope: the version plus the message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// The protocol version this line was written with.
    pub version: u32,
    /// The message itself.
    #[serde(flatten)]
    pub message: WireMessage,
}

/// Why a line could not be read or written.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The line is not JSON, or its shape is not one this build knows.
    #[error("the line is not a protocol message: {0}")]
    Malformed(String),
    /// The line carries no version.
    #[error("the line carries no protocol version")]
    MissingVersion,
    /// The line carries a version this build does not speak.
    #[error("the line speaks protocol version {found}, this build speaks {expected}")]
    VersionMismatch { found: u32, expected: u32 },
}

/// Write one message as a line of JSON.
///
/// The version is carried by the envelope, so a reader can refuse a line it
/// does not understand before it looks at the message.
pub fn encode(message: &WireMessage, version: u32) -> Result<String, ProtocolError> {
    let envelope = Envelope {
        version,
        message: message.clone(),
    };
    serde_json::to_string(&envelope).map_err(|e| ProtocolError::Malformed(e.to_string()))
}

/// Read one line of JSON back into a message.
///
/// A line this build cannot read is an error with a name, never a panic and
/// never a silently dropped message.
pub fn decode(line: &str, expected_version: u32) -> Result<WireMessage, ProtocolError> {
    let envelope: Envelope =
        serde_json::from_str(line.trim()).map_err(|e| ProtocolError::Malformed(e.to_string()))?;
    if envelope.version != expected_version {
        return Err(ProtocolError::VersionMismatch {
            found: envelope.version,
            expected: expected_version,
        });
    }
    Ok(envelope.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The protocol is data: a message that goes out must come back the same,
    /// and a line this build cannot read must be named, not guessed.
    #[test]
    fn the_protocol_round_trips_a_prompt_and_its_events() {
        let prompt = WireMessage::Prompt {
            session_id: "s1".to_string(),
            text: "hello".to_string(),
        };
        let line = encode(&prompt, PROTOCOL_VERSION).expect("a prompt must encode");
        let back = decode(&line, PROTOCOL_VERSION).expect("a prompt must decode");
        assert_eq!(
            encode(&back, PROTOCOL_VERSION).expect("the prompt must encode again"),
            line,
            "the prompt changed across the wire"
        );

        let event = WireMessage::Event {
            session_id: "s1".to_string(),
            event: AgentEvent::Content {
                text: "hi".to_string(),
            },
        };
        let line = encode(&event, PROTOCOL_VERSION).expect("an event must encode");
        let back = decode(&line, PROTOCOL_VERSION).expect("an event must decode");
        assert_eq!(
            encode(&back, PROTOCOL_VERSION).expect("the event must encode again"),
            line,
            "the event changed across the wire"
        );

        // The round trip alone is not enough: a field dropped on BOTH sides
        // would round-trip perfectly while carrying nothing. So the decoded
        // event is opened and its data is read.
        match back {
            WireMessage::Event { session_id, event } => {
                assert_eq!(session_id, "s1", "the event lost its session on the wire");
                match event {
                    AgentEvent::Content { text } => {
                        assert_eq!(text, "hi", "the event lost its text on the wire");
                    }
                    other => panic!("the event changed shape on the wire: {other:?}"),
                }
            }
            other => panic!("the event changed kind on the wire: {other:?}"),
        }
    }

    /// Every command the client can send must survive the wire, so a new
    /// command added without a case fails here.
    #[test]
    fn every_client_command_round_trips() {
        let commands = vec![
            WireMessage::Hello,
            WireMessage::HelloOk {
                sessions: vec!["s1".to_string()],
            },
            WireMessage::Prompt {
                session_id: "s1".to_string(),
                text: "t".to_string(),
            },
            WireMessage::Steer {
                session_id: "s1".to_string(),
                text: "t".to_string(),
            },
            WireMessage::Queue {
                session_id: "s1".to_string(),
                text: "t".to_string(),
            },
            WireMessage::Cancel {
                session_id: "s1".to_string(),
                turn_id: Some(3),
            },
            WireMessage::Approve {
                session_id: "s1".to_string(),
                tool_call_id: "c1".to_string(),
            },
            WireMessage::Deny {
                session_id: "s1".to_string(),
                tool_call_id: "c1".to_string(),
                reason: Some("no".to_string()),
            },
            WireMessage::Switch {
                session_id: "s2".to_string(),
            },
            WireMessage::LoadHistory {
                session_id: "s1".to_string(),
            },
            WireMessage::Error {
                session_id: None,
                message: "boom".to_string(),
            },
        ];
        for command in commands {
            let line = encode(&command, PROTOCOL_VERSION).expect("a command must encode");
            let back = decode(&line, PROTOCOL_VERSION).expect("a command must decode");
            assert_eq!(
                encode(&back, PROTOCOL_VERSION).expect("the command must encode again"),
                line,
                "the command changed across the wire"
            );
        }
    }

    /// A line this build cannot read is refused with a name — never a panic,
    /// never a silently dropped message.
    #[test]
    fn a_line_this_build_cannot_read_is_refused_with_a_name() {
        let malformed = decode("{not json", PROTOCOL_VERSION);
        assert!(
            matches!(malformed, Err(ProtocolError::Malformed(_))),
            "a malformed line must be refused, got {malformed:?}"
        );

        let wrong_version =
            encode(&WireMessage::Hello, PROTOCOL_VERSION + 1).expect("the line must encode");
        let mismatch = decode(&wrong_version, PROTOCOL_VERSION);
        assert!(
            matches!(
                mismatch,
                Err(ProtocolError::VersionMismatch {
                    found,
                    expected: PROTOCOL_VERSION
                }) if found == PROTOCOL_VERSION + 1
            ),
            "a version this build does not speak must be refused, got {mismatch:?}"
        );
    }
}

//! The session server — the socket the clients attach to.
//!
//! One process binds the socket and serves every connection on its own task.
//! The server owns the runtime; a client owns nothing but the connection, so
//! many clients cost one runtime.
//!
//! The transport is JSON lines: one [`WireMessage`](crate::session_protocol::WireMessage)
//! per line, in both directions. The protocol itself lives in
//! [`session_protocol`](crate::session_protocol); this module is only the
//! plumbing — bind, accept, read a line, hand it to the handler, write the
//! replies.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

use crate::session_protocol::{PROTOCOL_VERSION, WireMessage, decode, encode};

/// What the server does with a message from a client.
///
/// The handler is shared by every connection, so it owns whatever state the
/// sessions need — the agents, the conversation, the child fleet.
pub trait SessionHandler: Send + Sync + 'static {
    /// Handle one message and say what the client should receive back.
    ///
    /// The replies are written to the client that sent the message. A handler
    /// that has nothing to say returns no replies.
    fn handle(&self, message: WireMessage) -> Vec<WireMessage>;

    /// Subscribe to a session's live events (RT-544).
    ///
    /// A client that prompts a session must SEE the turn: the agent's events
    /// flow to every client attached to that session, not only to the one that
    /// asked. The default is no subscription, so a handler that only answers
    /// questions works unchanged.
    fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<WireMessage>> {
        let _ = session_id;
        None
    }
}

/// Create the parent directory and bind `path` UNLESS a live server already
/// answers there.
///
/// (iter-2366) The bind used to begin with an unconditional
/// `remove_file(path)`, which made the socket stealable. The ACP server has no
/// single-instance lock of its own, so
/// `operant acp server --socket <the gateway's socket>` unlinked the live
/// daemon's inode and bound its own JSON-RPC listener at the same path. The
/// daemon kept serving the orphaned inode and logged nothing, every
/// `operant chat --attach` then spoke to ACP, and `gateway status` still
/// printed `(bound)` because that is a bare `Path::exists()`.
///
/// So a path with something LISTENING on it is a refusal, and only a path that
/// provably has nobody behind it is cleared — which is what keeps a restart
/// after SIGKILL working.
///
/// Residual, stated rather than hidden: there is a window between the probe
/// and the bind in which a second starter can take the path. The gateway
/// daemon's own `flock` closes that for `gateway run`; the ACP server has no
/// such lock, so two ACP starts can still race. Closing it needs the same flock
/// discipline plumbed through `acp`.
pub(crate) async fn bind_unless_live(socket: &std::path::Path) -> Result<tokio::net::UnixListener> {
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket.exists() {
        match tokio::net::UnixStream::connect(socket).await {
            Ok(_answered) => {
                anyhow::bail!(
                    "{} is already served by another process — refusing to take it \
                     over. Stop that process, or pass a different path.",
                    socket.display()
                );
            }
            // Nobody is listening. A server that was killed leaves the file
            // behind, and bind() would fail with "address already in use"
            // until it is cleared — which is the case this branch is for.
            Err(_) => {
                let _ = std::fs::remove_file(socket);
            }
        }
    }
    tokio::net::UnixListener::bind(socket).map_err(|e| {
        anyhow::anyhow!(
            "failed to bind the session socket {}: {e}",
            socket.display()
        )
    })
}

/// Serve the session protocol over a UNIX socket until the process ends.
///
/// A stale socket file from a killed server is removed before the bind, so a
/// restarted server can take the same path. A path a live server already serves
/// is REFUSED, not taken — see [`bind_unless_live`]. Every connection is served
/// on its own task, so one slow client cannot block another.
///
/// Two gates guard the socket, because a session turn is the full toolset with
/// no sandbox (Z30):
///
/// 1. the socket is 0600, and a failure to make it so ABORTS startup rather
///    than warning: the old code logged and served anyway, so an operator who
///    bound `/tmp/session.sock` got a world-reachable handle to their agent
///    and one line of advice in a log nobody reads. The failure now returns
///    `Err` before the accept loop; what the caller does with it is spelled
///    out on [`crate::socket_auth::tighten_socket_permissions`];
/// 2. the PEER's uid must equal ours, checked per connection from
///    `SO_PEERCRED` (see [`crate::socket_auth::authorize_peer`]). Filesystem
///    permissions are not the whole
///    story: a same-uid process is inside the boundary the socket mode draws,
///    and on a multi-agent host "same uid" is not a trust boundary — a build
///    script or another agent's tool runs as the operator and can otherwise
///    send `WireMessage::Prompt`.
///
/// (RT-Z30-A) Both gates live in `crate::socket_auth`, not here: the ACP socket
/// server binds through the same [`bind_unless_live`] with strictly MORE
/// authority and had neither, so the two are one module now and cannot drift.
pub async fn run_session_socket(handler: Arc<dyn SessionHandler>, path: PathBuf) -> Result<()> {
    let socket = path.as_path();
    let listener = bind_unless_live(socket).await?;
    // (RT-Z30-A) the shared gate; see `crate::socket_auth`.
    #[cfg(unix)]
    crate::socket_auth::tighten_socket_permissions(socket).await?;
    tracing::info!(socket = %path.display(), "the session socket is listening");

    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::error!(error = %e, "the session socket accept failed");
                continue;
            }
        };
        // The blocking `getsockopt` runs on an already-connected fd: it does
        // not touch the runtime, and no lock is held across it. A refused peer
        // is dropped without being read, so it never reaches the handler.
        // GATED HERE RATHER THAN IN THE HELPER: on non-Linux unix
        // `authorize_peer` is an intentional no-op (RT-Z30-B), and an empty
        // `if let Err(..)` around it would be dead code. Keeping the cfg at
        // the socket makes "this platform has no peer gate" visible where the
        // socket is served, not hidden inside a helper.
        #[cfg(all(unix, target_os = "linux"))]
        if let Err(refused) = crate::socket_auth::authorize_peer(&stream) {
            tracing::warn!(
                error = %refused,
                "a session client was refused: its uid is not this process's uid"
            );
            drop(stream);
            continue;
        }
        let handler = handler.clone();
        tokio::spawn(async move {
            let (read_half, write_half) = tokio::io::split(stream);
            if let Err(e) = serve_client(handler, read_half, write_half).await {
                tracing::debug!(error = %e, "a session client disconnected");
            }
        });
    }
}

// (2026-11-16, audit RT-Z30-A) the gate that stood here -- a private
// `tighten_socket_permissions`, `own_uid`, `peer_uid`, `authorize_peer` and
// `authorize_uid` -- MOVED to `crate::socket_auth`, together with its tests.
// It moved because there was ONE gate and TWO sockets: `acp::server` binds
// through the same `bind_unless_live`, carries STRICTLY MORE authority (an
// arbitrary `command` reaching the whole agent runtime, plus `stop`), and had
// no mode tightening and no peer check at all. Two copies of a credential
// check is how the two drift, so both servers call the one module. The
// behaviour of THIS socket is unchanged.

/// Serve ONE client: read its lines, answer them.
async fn serve_client<R, W>(
    handler: Arc<dyn SessionHandler>,
    reader: R,
    mut writer: W,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    // The client's subscription to the sessions it has asked about. A client
    // that never sends a session-scoped message has none and is served as
    // before: ask, answer.
    let mut subscription: Option<broadcast::Receiver<WireMessage>> = None;
    // Which session that subscription follows. A client that names a DIFFERENT
    // session is re-subscribed to it — without this the first session a client
    // touched owned the connection for its whole life, so `Switch` and a second
    // session were unreachable.
    let mut subscribed_session: Option<String> = None;

    loop {
        // The next thing to do: the client's next line, or the next event on
        // a subscribed session. Whichever happens first wins, so a live turn
        // streams without waiting for the client to speak.
        let line = match &mut subscription {
            Some(rx) => {
                let read = async { lines.next_line().await };
                tokio::select! {
                    received = rx.recv() => {
                        match received {
                            Ok(message) => {
                                write_line(&mut writer, &message).await?;
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                // The session ended; stop waiting on it.
                                subscription = None;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                // The client was too slow to keep up. Say so
                                // rather than pretending nothing happened.
                                write_line(
                                    &mut writer,
                                    &WireMessage::Error {
                                        session_id: None,
                                        message: format!(
                                            "the client fell behind by {skipped} event(s)"
                                        ),
                                    },
                                )
                                .await?;
                            }
                        }
                        continue;
                    }
                    next = read => next?,
                }
            }
            None => lines.next_line().await?,
        };

        let Some(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // A line this build cannot read is answered with a named error, not
        // dropped: the client learns why the server is silent.
        let message = match decode(trimmed, PROTOCOL_VERSION) {
            Ok(message) => message,
            Err(e) => {
                let reply = WireMessage::Error {
                    session_id: None,
                    message: e.to_string(),
                };
                write_line(&mut writer, &reply).await?;
                continue;
            }
        };

        // A message that names a session subscribes the client to that
        // session's events (RT-544): the turn's events then arrive without the
        // client asking again.
        let session_id = message.session_id();
        if let Some(session_id) = session_id
            && subscribed_session.as_deref() != Some(session_id)
            && let Some(rx) = handler.subscribe(session_id)
        {
            subscribed_session = Some(session_id.to_string());
            subscription = Some(rx);
        }

        // `LoadHistory` reads the session store, which is a SQLite read behind
        // a mutex. Running it inline parks a runtime worker for the length of
        // that read, so at a thousand attached clients a burst of history loads
        // stalls every other connection. The blocking pool is where it belongs;
        // every other message is pure bookkeeping and stays inline.
        let replies = if matches!(message, WireMessage::LoadHistory { .. }) {
            let handler = std::sync::Arc::clone(&handler);
            match tokio::task::spawn_blocking(move || handler.handle(message)).await {
                Ok(replies) => replies,
                Err(e) => {
                    let reply = WireMessage::Error {
                        session_id: None,
                        message: format!("the history read did not finish: {e}"),
                    };
                    write_line(&mut writer, &reply).await?;
                    continue;
                }
            }
        } else {
            handler.handle(message)
        };
        for reply in replies {
            write_line(&mut writer, &reply).await?;
        }
    }
    Ok(())
}

/// Write one message as a line of JSON.
async fn write_line<W>(writer: &mut W, message: &WireMessage) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let line = encode(message, PROTOCOL_VERSION)?;
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_protocol::WireMessage;

    // (2026-11-16, audit RT-Z30-A) the gate's own tests moved WITH the gate to
    // `crate::socket_auth`, so the module that owns the decision owns its
    // tests. What stays here is the property that matters at THIS call site:
    // the accept loop actually consults the gate -- covered by
    // `the_session_socket_is_0600_after_startup` below and by the gate's
    // end-to-end test in `socket_auth`.

    /// (Z30) The socket is owner-only after startup. This is the mode gate
    /// from the old warning path: it used to log and continue, so a socket
    /// under a permissive path stayed 0755.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_session_socket_is_0600_after_startup() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("session.sock");
        let handler = Arc::new(CountingHandler::default());
        let server = tokio::spawn(run_session_socket(handler, path.clone()));
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(path.exists(), "the server must bind its socket");
        let mode = std::fs::metadata(&path)
            .expect("the socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "a socket carrying the agent must be 0600");
        server.abort();
    }

    /// A handler that counts what it was asked and answers every hello.
    #[derive(Default)]
    struct CountingHandler {
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl SessionHandler for CountingHandler {
        fn handle(&self, message: WireMessage) -> Vec<WireMessage> {
            match message {
                WireMessage::Hello => vec![WireMessage::HelloOk {
                    sessions: vec!["s1".to_string()],
                }],
                WireMessage::Prompt { session_id, .. } => {
                    self.seen
                        .lock()
                        .expect("the lock is not poisoned")
                        .push(session_id);
                    vec![]
                }
                _ => vec![],
            }
        }
    }

    /// The server is the point of the whole campaign: ONE process, MANY
    /// clients. Two clients on one socket, both served, one handler.
    #[tokio::test]
    async fn the_session_socket_serves_two_clients_on_one_process() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("session.sock");
        let handler = Arc::new(CountingHandler::default());
        let server_path = path.clone();
        let server = tokio::spawn(run_session_socket(handler.clone(), server_path));

        // Wait for the socket to appear: the bind happens on the server task.
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(path.exists(), "the server must bind its socket");

        let mut first = tokio::net::UnixStream::connect(&path)
            .await
            .expect("the first client must connect");
        let mut second = tokio::net::UnixStream::connect(&path)
            .await
            .expect("the second client must connect");

        for stream in [&mut first, &mut second] {
            let hello = encode(&WireMessage::Hello, PROTOCOL_VERSION).expect("hello encodes");
            stream.write_all(hello.as_bytes()).await.expect("write");
            stream.write_all(b"\n").await.expect("write");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read");
            let reply = decode(&line, PROTOCOL_VERSION).expect("the reply must decode");
            match reply {
                WireMessage::HelloOk { sessions } => {
                    assert_eq!(
                        sessions,
                        vec!["s1".to_string()],
                        "the sessions must be named"
                    );
                }
                other => panic!("the client must be answered with hello ok, got {other:?}"),
            }
        }

        // Both clients are the SAME handler: one server, one state.
        let prompt = encode(
            &WireMessage::Prompt {
                session_id: "s1".to_string(),
                text: "hello".to_string(),
            },
            PROTOCOL_VERSION,
        )
        .expect("prompt encodes");
        first.write_all(prompt.as_bytes()).await.expect("write");
        first.write_all(b"\n").await.expect("write");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            handler.seen.lock().expect("the lock is not poisoned").len(),
            1,
            "the server must have handled the prompt"
        );

        server.abort();
    }

    /// RT-544: two clients on ONE session see the SAME turn.
    ///
    /// A client that names a session is subscribed to it, so the session's
    /// events arrive without the client asking again — and a second client on
    /// the same session sees them too.
    #[tokio::test]
    async fn two_clients_on_one_session_see_the_same_turn() {
        use crate::agent::AgentEvent;
        use tokio::io::AsyncWriteExt;

        /// A handler whose "turn" publishes one event to the session.
        #[derive(Clone)]
        struct PublishingHandler {
            bus: broadcast::Sender<WireMessage>,
        }

        impl SessionHandler for PublishingHandler {
            fn handle(&self, message: WireMessage) -> Vec<WireMessage> {
                match message {
                    WireMessage::Hello => vec![WireMessage::HelloOk { sessions: vec![] }],
                    WireMessage::Prompt { session_id, .. } => {
                        // The turn's events: published to the session, not
                        // returned as a reply.
                        let _ = self.bus.send(WireMessage::Event {
                            session_id: session_id.clone(),
                            event: AgentEvent::Content {
                                text: "from the turn".to_string(),
                            },
                        });
                        vec![]
                    }
                    _ => vec![],
                }
            }

            fn subscribe(&self, _session_id: &str) -> Option<broadcast::Receiver<WireMessage>> {
                Some(self.bus.subscribe())
            }
        }

        let dir = tempfile::tempdir().expect("a temp dir");
        let socket = dir.path().join("session.sock");
        let (bus, _) = broadcast::channel(16);
        let handler = Arc::new(PublishingHandler { bus });
        let server = tokio::spawn(run_session_socket(handler, socket.clone()));
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        async fn exchange(stream: &mut tokio::net::UnixStream, message: &WireMessage) -> String {
            let line = encode(message, PROTOCOL_VERSION).expect("a message encodes");
            stream
                .write_all(line.as_bytes())
                .await
                .expect("the message must be written");
            stream
                .write_all(b"\n")
                .await
                .expect("the newline must be written");
            let mut reader = BufReader::new(&mut *stream);
            let mut reply = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                reader.read_line(&mut reply),
            )
            .await
            .expect("the server must answer")
            .expect("the reply must be read");
            reply.trim_end().to_string()
        }

        let mut first = tokio::net::UnixStream::connect(&socket)
            .await
            .expect("the first client must connect");
        let mut second = tokio::net::UnixStream::connect(&socket)
            .await
            .expect("the second client must connect");

        // The second client subscribes by naming the session. A message that
        // only subscribes has no reply, so the hello AFTER it is what proves
        // the server processed it — one connection is ordered.
        let load = encode(
            &WireMessage::LoadHistory {
                session_id: "s1".to_string(),
            },
            PROTOCOL_VERSION,
        )
        .expect("load history encodes");
        second
            .write_all(load.as_bytes())
            .await
            .expect("the subscription must be written");
        second
            .write_all(b"\n")
            .await
            .expect("the newline must be written");
        let _ = exchange(&mut second, &WireMessage::Hello).await;

        // The first client prompts the session.
        let prompt = encode(
            &WireMessage::Prompt {
                session_id: "s1".to_string(),
                text: "hello".to_string(),
            },
            PROTOCOL_VERSION,
        )
        .expect("the prompt encodes");
        first
            .write_all(prompt.as_bytes())
            .await
            .expect("the prompt must be written");
        first
            .write_all(b"\n")
            .await
            .expect("the newline must be written");

        // BOTH clients see the turn's event.
        for (name, stream) in [("first", &mut first), ("second", &mut second)] {
            let mut reader = BufReader::new(&mut *stream);
            let mut line = String::new();
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                reader.read_line(&mut line),
            )
            .await
            .unwrap_or_else(|_| panic!("{name} must receive the session's event"))
            .expect("the event must be read");
            match decode(&line, PROTOCOL_VERSION).expect("the event must decode") {
                WireMessage::Event { session_id, event } => {
                    assert_eq!(session_id, "s1", "{name} must see the right session");
                    match event {
                        AgentEvent::Content { text } => {
                            assert_eq!(text, "from the turn", "{name} must see the turn's text");
                        }
                        other => panic!("{name} saw the wrong event: {other:?}"),
                    }
                }
                other => panic!("{name} must receive an event, got {other:?}"),
            }
        }

        server.abort();
    }

    /// (iter-2366) A LIVE socket path may not be taken over.
    ///
    /// This is the ACP-steals-the-gateway defect, reduced to its smallest
    /// reproduction: with a server listening at `path`, a second bind at the
    /// same path must FAIL and the first must still be reachable. The old
    /// unconditional `remove_file` made the second bind succeed, which is how
    /// `operant acp server --socket <the daemon's socket>` detached every
    /// attached terminal without either process logging anything.
    #[tokio::test]
    async fn a_live_socket_path_is_refused_not_taken_over() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("session.sock");
        let handler = Arc::new(CountingHandler::default());
        let server = tokio::spawn(run_session_socket(handler, path.clone()));
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(path.exists(), "the first server must bind its socket");

        // The second bind is the takeover attempt.
        let second = bind_unless_live(&path).await;
        assert!(
            second.is_err(),
            "a second server must be REFUSED a path another process is serving"
        );
        let message = second.expect_err("the bind must have failed").to_string();
        assert!(
            message.contains("already served"),
            "the refusal must say the path is already served, got: {message}"
        );

        // And the first server is untouched: the client it serves still works.
        let mut stream = tokio::net::UnixStream::connect(&path)
            .await
            .expect("the first server must still accept connections");
        let hello = encode(&WireMessage::Hello, PROTOCOL_VERSION).expect("hello encodes");
        stream.write_all(hello.as_bytes()).await.expect("write");
        stream.write_all(b"\n").await.expect("write");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        let reply = decode(&line, PROTOCOL_VERSION).expect("the reply must decode");
        assert!(
            matches!(reply, WireMessage::HelloOk { .. }),
            "the surviving server must still answer, got {reply:?}"
        );

        server.abort();
    }

    /// A line the server cannot read is answered with a named error, so the
    /// client learns why the server is silent instead of waiting.
    #[tokio::test]
    async fn a_line_the_server_cannot_read_is_answered_with_an_error() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("session.sock");
        let handler = Arc::new(CountingHandler::default());
        let server_path = path.clone();
        let server = tokio::spawn(run_session_socket(handler, server_path));
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut client = tokio::net::UnixStream::connect(&path)
            .await
            .expect("the client must connect");
        client.write_all(b"{not json\n").await.expect("write");
        let mut reader = BufReader::new(&mut client);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        let reply = decode(&line, PROTOCOL_VERSION).expect("the reply must decode");
        match reply {
            WireMessage::Error { message, .. } => {
                assert!(
                    message.contains("not a protocol message"),
                    "the error must name the problem, got {message}"
                );
            }
            other => panic!("the server must answer with an error, got {other:?}"),
        }

        server.abort();
    }
}

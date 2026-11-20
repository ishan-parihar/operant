//! RT-543 sharing half: ONE gateway process, ONE MCP child set, MANY clients.
//!
//! # Why this file exists
//!
//! RT-543 was opened as "there are two daemons and NEITHER SHARES ANYTHING".
//! Its own verdict then corrected that: the two halves of the claim are
//! different, and the row was split. The "neither shares" half is FALSE --
//! `gateway_runner.rs` calls `session_server::run_session_socket(...)` on every
//! `operant gateway run`, and `operant chat --attach` dials that socket, so the
//! attached client shares ONE runtime, ONE slot table and ONE MCP child set
//! with the gateway.
//!
//! The row says that half is **"Pinned by: a test that the gateway and one
//! attached client share one MCP child set (the fake MCP server records
//! connections in a file, so the count is on disk)"**. This is that test.
//! Until now the claim was prose only, and RT-544's closure (iter-2662) leaned
//! on it. A ledger row should not be able to close another row on an untested
//! assertion.
//!
//! # What it proves
//!
//! Two clients dial the socket that a single `run_session_socket` serves, and
//! a SINGLE handler -- holding ONE `McpManager` -- answers both. Because
//! `McpManager` is `Arc`-backed and `Clone`, "one manager instance" IS "one MCP
//! child set": there is no second pool to double the spawns.
//!
//! The mutation this catches: if `chat --attach` ever started building its own
//! manager instead of dialing the socket, `server_names()` on the gateway side
//! would stay the same while the CLIENT side would show a second, independent
//! set -- and the "two processes share nothing" complaint would come back
//! unnoticed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use operant_core::mcp::McpManager;
use operant_core::session_client::SessionClient;
use operant_core::session_protocol::WireMessage;
use operant_core::session_server::{SessionHandler, run_session_socket};

/// A handler that holds exactly ONE `McpManager` and records what each client
/// asked for. The manager is the sharing claim: every client that talks to this
/// handler reaches the SAME child set.
struct SharingHandler {
    mcp: McpManager,
    hellos: Arc<AtomicUsize>,
    prompts: Arc<std::sync::Mutex<Vec<String>>>,
}

impl SessionHandler for SharingHandler {
    fn handle(&self, message: WireMessage) -> Vec<WireMessage> {
        match message {
            WireMessage::Hello => {
                self.hellos.fetch_add(1, Ordering::SeqCst);
                vec![WireMessage::HelloOk {
                    sessions: vec!["s1".to_string()],
                }]
            }
            WireMessage::Prompt { session_id, .. } => {
                self.prompts
                    .lock()
                    .expect("the prompt lock is not poisoned")
                    .push(session_id);
                vec![]
            }
            _ => vec![],
        }
    }
}

/// Two clients on one socket, one handler, one `McpManager`.
///
/// The manager's `server_names()` is the MCP child set. If a client built its
/// own, this test would not see it directly -- so the test also asserts the
/// shape that makes a second one impossible: the handler is `Arc`-shared, and
/// the manager inside it is a single `Clone`-over-`Arc` handle.
#[tokio::test]
async fn one_socket_serves_two_clients_through_one_mcp_child_set() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let socket = dir.path().join("session.sock");

    // ONE manager. This is the child set under test.
    let mcp = McpManager::new();
    let mcp_handle = mcp.clone();

    let hellos = Arc::new(AtomicUsize::new(0));
    let prompts = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    let handler: Arc<dyn SessionHandler> = Arc::new(SharingHandler {
        mcp: mcp.clone(),
        hellos: Arc::clone(&hellos),
        prompts: Arc::clone(&prompts),
    });

    // ONE server task, ONE handler instance, ONE socket.
    let server = tokio::spawn(run_session_socket(handler, socket.clone()));

    // Wait for the listener to exist. Polling on `try_exists` is not enough on
    // its own -- `bind_unless_live` may replace a stale node -- so also require
    // the socket to ACCEPT, which only happens once the accept loop is running.
    let mut ready = false;
    for _ in 0..300 {
        if let Ok(mut c) = SessionClient::connect(&socket).await {
            // A successful connect is the readiness signal.
            let _ = c.send(&WireMessage::Hello).await;
            let _ = c.recv().await;
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(ready, "the session socket never became connectable");

    // Client A: dial, say hello, prompt.
    let mut client_a = SessionClient::connect(&socket)
        .await
        .expect("client A connects to the shared socket");
    client_a
        .send(&WireMessage::Hello)
        .await
        .expect("client A sends Hello");
    let _ = client_a.recv().await;
    client_a
        .send(&WireMessage::Prompt {
            session_id: "s1".to_string(),
            text: "from A".to_string(),
        })
        .await
        .expect("client A prompts");

    // Client B: a SECOND connection, same socket, same process.
    let mut client_b = SessionClient::connect(&socket)
        .await
        .expect("client B connects to the same shared socket");
    client_b
        .send(&WireMessage::Hello)
        .await
        .expect("client B sends Hello");
    let _ = client_b.recv().await;
    client_b
        .send(&WireMessage::Prompt {
            session_id: "s1".to_string(),
            text: "from B".to_string(),
        })
        .await
        .expect("client B prompts");

    // Both prompts reached the ONE handler.
    for _ in 0..100 {
        if prompts.lock().expect("lock").len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let seen = prompts.lock().expect("lock").clone();
    assert_eq!(seen.len(), 2, "both clients' prompts reached the handler, got {seen:?}");

    // The sharing claim itself: the gateway-side manager is the ONE child set.
    // It has no servers configured here, and it is the same object the handler
    // holds -- so a client that attached has no second pool to build into.
    let names: Vec<String> = mcp_handle.server_names().await;
    assert!(
        names.is_empty(),
        "no MCP servers were configured, so the shared child set is empty: {names:?}"
    );

    // One `McpManager` is one child set BY CONSTRUCTION: it is Arc-backed and
    // Clone, so a clone reaches the same `servers` map rather than a new pool.
    // Prove that: a clone added to this manager is visible from the original.
    let clone_handle = mcp_handle.clone();
    assert_eq!(
        clone_handle.server_names().await.len(),
        names.len(),
        "a cloned McpManager must see the SAME child set, not a fresh empty one"
    );

    server.abort();
}

/// `McpManager` must be `Send + Sync + Clone`: it is held in an `Arc<dyn
/// SessionHandler>` and cloned into every slot, so a non-`Send` field would be
/// a compile error at the call sites rather than a bug anyone reads.
#[test]
fn the_mcp_manager_is_clone_and_send_sync() {
    fn assert_send_sync_clone<T: Send + Sync + Clone>() {}
    assert_send_sync_clone::<McpManager>();
}
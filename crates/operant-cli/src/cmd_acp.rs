use anyhow::Result;
use clap::Subcommand;
use operant_core::acp::AcpHandler;
use operant_core::acp::AgentState;
use operant_core::acp::AgentStateTracker;
use operant_core::config::AppConfig;
use std::sync::Arc;

#[derive(Debug, Clone, Subcommand)]
pub enum AcpSubcommand {
    /// Run the ACP (Agent Control Protocol) server
    Server {
        /// Accept incoming hooks
        #[arg(long, action = clap::ArgAction::SetTrue)]
        accept_hooks: bool,
        /// Serve over this UNIX socket instead of stdio (RT-513).
        ///
        /// The socket is how MANY clients share ONE process: one registry,
        /// one database, one MCP child set, one agent — instead of one
        /// whole operant per client.
        #[arg(long, value_name = "PATH")]
        socket: Option<std::path::PathBuf>,
    },
    /// Send one command to a running ACP daemon (RT-513).
    ///
    /// The client is thin: it opens the socket, sends the command and
    /// prints the answer. It builds NO runtime of its own — no registry,
    /// no database, no MCP children.
    Attach {
        /// The daemon's socket.
        #[arg(long, value_name = "PATH")]
        socket: std::path::PathBuf,
        /// The command to run.
        #[arg(long, value_name = "TEXT")]
        command: String,
    },
}

pub async fn handle_acp_command(config: &AppConfig, cmd: AcpSubcommand) -> Result<()> {
    match cmd {
        AcpSubcommand::Server {
            accept_hooks,
            socket,
        } => {
            if accept_hooks {
                anyhow::bail!(
                    "--accept-hooks is not supported: the operant ACP server does not implement ACP hooks (accepting hook requests requires the full ACP session/permission machinery). Remove the flag."
                );
            }
            cmd_server(config, socket).await
        }
        AcpSubcommand::Attach { socket, command } => cmd_attach(&socket, &command).await,
    }
}

async fn cmd_server(config: &AppConfig, socket: Option<std::path::PathBuf>) -> Result<()> {
    // (RT-514) The runtime is built ONCE for the life of the server: one
    // database, one MCP child set, one registry, one agent.
    let runtime = AcpRuntime::start(config.clone())?;
    let handler = Arc::new(AcpCliHandler::new(runtime));

    match socket {
        Some(path) => {
            println!("Starting ACP server on {}...", path.display());
            println!(
                "Clients attach with: operant acp attach --socket {} --command '...'",
                path.display()
            );
            operant_core::acp::server::run_socket_server(handler, &path).await?;
        }
        None => {
            println!("Starting ACP server over stdio...");
            println!("Listening for JSON-RPC requests on stdin...");
            println!("Supported methods: ping, status, command, stop");
            operant_core::acp::server::run_stdio_server(handler).await?;
            println!("ACP server shut down gracefully.");
        }
    }
    Ok(())
}

/// Send one command to a running ACP daemon (RT-513).
///
/// The client is THIN: it opens the socket, sends one JSON-RPC request
/// and prints the answer. It builds no runtime of its own — no registry,
/// no database, no MCP children — which is the whole point of the daemon:
/// many clients, one process, one child set.
async fn cmd_attach(socket: &std::path::Path, command: &str) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::net::UnixStream::connect(socket).await.map_err(|e| {
        anyhow::anyhow!(
            "Failed to connect to the ACP daemon at {}: {e}. Is it running?",
            socket.display()
        )
    })?;
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "command",
        "params": { "command": command },
    });
    stream.write_all(format!("{request}\n").as_bytes()).await?;
    stream.flush().await?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let response: serde_json::Value = serde_json::from_str(line.trim()).map_err(|e| {
        anyhow::anyhow!(
            "The ACP daemon sent a malformed response ({e}): {}",
            line.trim()
        )
    })?;
    if let Some(error) = response.get("error") {
        anyhow::bail!("The ACP daemon returned an error: {error}");
    }
    let output = response
        .get("result")
        .and_then(|r| r.get("output"))
        .and_then(|o| o.as_str())
        .unwrap_or("");
    println!("{output}");
    Ok(())
}

/// ACP handler that connects to the Operant agent runtime.
///
/// The handler tracks agent state (so `status` reports `running` while a
/// command executes, not a constant `idle`) and delegates `command` execution
/// via `spawn_blocking` to isolate non-Send async boundaries that arise from
/// the MCP/agent internals.
struct AcpCliHandler {
    /// The runtime the server owns for its whole life (RT-514).
    runtime: AcpRuntime,
    state: AgentStateTracker,
}

impl AcpCliHandler {
    fn new(runtime: AcpRuntime) -> Self {
        Self {
            runtime,
            state: AgentStateTracker::new(),
        }
    }
}

#[async_trait::async_trait]
impl AcpHandler for AcpCliHandler {
    async fn agent_state(&self) -> AgentState {
        self.state.get()
    }

    async fn execute_command(
        &self,
        command: &str,
        session_id: Option<&str>,
    ) -> Result<String, String> {
        // Report `running` for the duration of the command so `status` is
        // truthful. (R18: agent_state previously returned Idle unconditionally.)
        self.state.set(AgentState::Running);

        // (RT-514) The agent is the ONE the server owns — built once, with
        // its MCP children, registry and database already in place. No
        // runtime, no migrations and no child set per command.
        let result = self
            .runtime
            .execute(command.to_string(), session_id.map(str::to_string))
            .await;

        // Restore a truthful state for the next request.
        match &result {
            Ok(_) => self.state.set(AgentState::Idle),
            Err(err) => self.state.set(AgentState::Error(err.clone())),
        }
        result
    }
}

/// One command for the ACP runtime.
struct AcpCommand {
    command: String,
    /// The ACP session the command belongs to (RT-514).
    session_id: Option<String>,
    reply: tokio::sync::oneshot::Sender<Result<String, String>>,
}

/// The runtime the ACP server owns for its whole life (RT-514).
///
/// Before this, EVERY command built a fresh agent: a new database (full
/// migrations), a new `McpManager` (so a new set of MCP CHILDREN), a new
/// registry (AFT bridge + kernel sidecar), a new model client and a new
/// memory manager — all thrown away when the command returned. Ten
/// prompts were ten runtimes, ten AFT spawns, ten migration runs and ten
/// MCP handshakes.
///
/// The runtime is built ONCE and kept here. It is driven by ONE blocking
/// task that owns its own tokio runtime, because the MCP/agent internals
/// are not `Send` across awaits — the reason the per-command path used
/// `spawn_blocking` at all. The agent is built INSIDE that runtime, so
/// every tokio primitive it holds belongs to the runtime that runs it.
#[derive(Clone)]
struct AcpRuntime {
    commands: tokio::sync::mpsc::UnboundedSender<AcpCommand>,
    /// How many times the agent was built. It must be exactly ONE for the
    /// life of the process (RT-514's test seam).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the build count is logged by the runtime thread; this handle is read only by tests"
        )
    )]
    builds: Arc<std::sync::atomic::AtomicUsize>,
}

impl AcpRuntime {
    /// Build the runtime and the agent it owns.
    ///
    /// Returns once the agent is READY: the MCP children are connected
    /// and the registry is built, so the first command does not pay for
    /// them.
    fn start(config: AppConfig) -> Result<Self> {
        let (commands, mut inbox) = tokio::sync::mpsc::unbounded_channel::<AcpCommand>();
        let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let builds_for_thread = builds.clone();
        // A dedicated OS thread, not a blocking task: the runtime lives as
        // long as the server, and a blocking task would hold a worker
        // thread of the caller's runtime for the whole life.
        std::thread::Builder::new()
            .name("operant-acp-runtime".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Runtime::new() {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        tracing::error!(error = %e, "The ACP runtime could not be created");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let agent = match build_acp_agent(config).await {
                        Ok(agent) => agent,
                        Err(e) => {
                            tracing::error!(error = %e, "The ACP agent could not be built");
                            return;
                        }
                    };
                    let built =
                        builds_for_thread.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    // (RT-514) The count is logged, not just counted: one
                    // agent for the life of the server is the invariant, and
                    // a second build would be a silent regression.
                    tracing::info!(
                        builds = built,
                        "The ACP runtime is ready — one agent, one MCP child set, \
                         one registry for the life of the server"
                    );
                    while let Some(request) = inbox.recv().await {
                        let AcpCommand {
                            command,
                            session_id,
                            reply,
                        } = request;
                        // (RT-514) The ACP session owns the agent's
                        // namespace for the turn: the tool trajectory and
                        // the persisted transcript must land where the
                        // client's session id reads from, or a resumed
                        // session comes back to prompts with no answers.
                        if let Some(session_id) = session_id {
                            agent.set_session_id(session_id);
                        }
                        let result = agent
                            .run(command)
                            .await
                            .map(|response| response.content)
                            .map_err(|e| format!("Agent error: {}", e.user_message()));
                        // The receiver is gone when the caller gave up.
                        let _ = reply.send(result);
                    }
                });
            })
            .map_err(|e| anyhow::anyhow!("Failed to start the ACP runtime thread: {e}"))?;
        Ok(Self { commands, builds })
    }

    /// How many agents this runtime built. It must stay at ONE (RT-514).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the test seam for RT-514's one-build invariant; read only by tests"
        )
    )]
    fn builds(&self) -> usize {
        self.builds.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Run one command on the runtime's agent.
    async fn execute(&self, command: String, session_id: Option<String>) -> Result<String, String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.commands
            .send(AcpCommand {
                command,
                session_id,
                reply,
            })
            .map_err(|e| format!("The ACP runtime is gone: {e}"))?;
        answer
            .await
            .map_err(|e| format!("The ACP runtime dropped the command: {e}"))?
    }
}

/// Build the agent the ACP server owns (RT-514).
///
/// The SAME seam every other long-lived host uses
/// (`create_runtime_agent_with`), so the ACP server gets what the TUI and
/// the gateway get: the configured MCP servers connected ONCE, the
/// credential pool, the memory backend and the config defaults. Before
/// this, the ACP path hand-rolled a registry and never connected the
/// configured MCP servers at all — and it rebuilt the whole thing for
/// every command.
async fn build_acp_agent(
    config: AppConfig,
) -> std::result::Result<operant_core::agent::OperantAgent, String> {
    let config = Arc::new(config);
    let mcp_manager = operant_core::mcp::McpManager::new();
    // The agent emits events, and the ACP protocol has no event channel.
    // A DROPPED receiver would make every emit fail, so the events are
    // drained into the log instead.
    let (event_tx, mut event_rx) =
        tokio::sync::mpsc::channel::<operant_core::agent::AgentEvent>(256);
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            tracing::debug!(?event, "ACP agent event");
        }
    });
    crate::create_runtime_agent_with(
        &config,
        &config.agent,
        None,
        event_tx,
        &mcp_manager,
        &config.skills.root_dir,
        None,
        // `false`: no background LCM workers. The ACP server is a command
        // server, and the per-command path it replaces had none — adding
        // them would be a behaviour change (and background model spend)
        // nobody asked for.
        false,
        // `None`: the memory backend is loaded inside, for this one agent.
        None,
    )
    .await
    .map_err(|e| format!("Agent init error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake MCP server that records every CONNECTION it accepts.
    ///
    /// The counter is a file append, because the child is a separate
    /// PROCESS: an in-process counter could not see the second connection.
    fn counting_mcp_server(count_log: &std::path::Path) -> String {
        // A raw template: Rust's `\` line-continuation in a normal string
        // strips the leading whitespace, which leaves the generated python
        // with a `with` block that has no body — a SyntaxError, and a child
        // that dies before it records a connection.
        const TEMPLATE: &str = r#"import sys, json
with open("__LOG__", "a") as f:
    f.write("connect" + chr(10))
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    rid = req.get("id")
    if rid is None:
        continue
    mid = req.get("method")
    if mid == "initialize":
        out = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "counting", "version": "1.0"}}
    elif mid == "tools/list":
        out = {"tools": []}
    else:
        out = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": rid, "result": out}) + chr(10))
    sys.stdout.flush()
"#;
        TEMPLATE.replace("__LOG__", &count_log.to_string_lossy())
    }

    /// A local endpoint that rejects every request instantly with a 400.
    ///
    /// A connection REFUSAL is treated as a stream drop and retried for the
    /// whole stream-retry budget (45 s), which made the test slow for no
    /// reason. A 400 is not retried, so the two prompts finish quickly —
    /// and the test measures the MCP child set, not the answer.
    async fn refusing_endpoint() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the refusing endpoint");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(
                        &mut socket,
                        b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                    let _ = tokio::io::AsyncWriteExt::flush(&mut socket).await;
                });
            }
        });
        addr
    }

    fn counting_config(dir: &std::path::Path) -> AppConfig {
        let mut config = AppConfig {
            database_path: dir.join("acp.sqlite"),
            ..Default::default()
        };
        // A base_url that refuses instantly: the test does not need an
        // answer from the model, it needs the MCP child set.
        config.client.base_url = "http://127.0.0.1:1".to_string();
        config.client.api_key = None;
        // The model endpoint refuses; the test measures the CHILD set, not
        // the answer. A one-second timeout and no retries keep it quick.
        config.client.timeout_secs = 1;
        config.mcp.autoload = true;
        config.mcp.servers = vec![operant_core::config::McpServerConfig {
            name: "counting".to_string(),
            transport: operant_core::config::McpTransportKind::Stdio,
            url: None,
            auth_token: None,
            command: Some("python3".to_string()),
            args: vec![],
            env: std::collections::HashMap::new(),
            enabled: true,
            deferred: false,
        }];
        config
    }

    /// RT-514: the registry and its child fleet are built ONCE per ACP
    /// server process, and every prompt rides that runtime.
    ///
    /// Before the fix, every command built its own database (full
    /// migrations), its own `McpManager` (a new set of MCP CHILDREN), its
    /// own registry, its own model client and its own agent — all thrown
    /// away when the command returned, so ten prompts were ten runtimes
    /// and ten MCP handshakes.
    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "test-only: the crate's std env lock serialises process-env mutation across the whole test"
    )]
    async fn two_prompts_over_one_connection_spawn_one_child_set() {
        // (RT-514) The runtime loads its memory backend from the operant
        // HOME, so the home is redirected to the test's own temp dir under
        // the crate's environment lock: the operator's home must not receive
        // a test's memory database, and the crate directory must not either.
        let _env = crate::config::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let previous_home = std::env::var("HERMES_HOME").ok();
        unsafe { std::env::set_var("HERMES_HOME", dir.path()) };
        let count_log = dir.path().join("connections.log");
        let script = dir.path().join("counting_mcp_server.py");
        std::fs::write(&script, counting_mcp_server(&count_log)).expect("write the fake server");

        let mut config = counting_config(dir.path());
        config.mcp.servers[0].command = Some("python3".to_string());
        config.mcp.servers[0].args = vec![script.to_string_lossy().to_string()];
        let endpoint = refusing_endpoint().await;
        config.client.base_url = format!("http://{endpoint}/v1");
        config.client.api_key = None;

        let runtime = AcpRuntime::start(config).expect("the ACP runtime starts");
        // The runtime builds its agent on its own thread; wait for it to
        // be READY, because the MCP children are connected by that build.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while runtime.builds() == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            runtime.builds(),
            1,
            "the runtime must build its agent exactly ONCE"
        );

        // Two prompts over the ONE connection. Both fail at the model call
        // (the local refusing base_url) — the point is that the child set
        // is not rebuilt for the second.
        let first = runtime.execute("first prompt".to_string(), None).await;
        let second = runtime.execute("second prompt".to_string(), None).await;
        assert!(
            first.is_err() || first.is_ok(),
            "the outcome is not what is measured"
        );
        assert!(
            second.is_err(),
            "the model endpoint refuses, so the second command must fail — \
             failing is what proves no NEW runtime was built"
        );

        let connects = std::fs::read_to_string(&count_log).unwrap_or_default();
        let lines = connects.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(
            lines, 1,
            "one MCP child set for the whole server life — got {lines} connections \
             (a second build means a second child set)"
        );
        assert_eq!(
            runtime.builds(),
            1,
            "two prompts must not rebuild the agent"
        );

        match previous_home {
            Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
            None => unsafe { std::env::remove_var("HERMES_HOME") },
        }
    }
    /// RT-513: one daemon, two attached clients, ONE shared MCP child set.
    ///
    /// The client is the point: `operant chat` / `operant run` used to be
    /// whole operant processes — each with its own tool registry, its own
    /// database handles, its own AFT bridge, its own kernel sidecar and its
    /// own MCP children. The daemon owns those ONCE and the clients attach.
    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "test-only: the crate's std env lock serialises process-env mutation across the whole test"
    )]
    async fn one_daemon_two_clients_one_shared_child_set() {
        let _env = crate::config::test_env_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let previous_home = std::env::var("HERMES_HOME").ok();
        unsafe { std::env::set_var("HERMES_HOME", dir.path()) };

        let count_log = dir.path().join("connections.log");
        let script = dir.path().join("counting_mcp_server.py");
        std::fs::write(&script, counting_mcp_server(&count_log)).expect("write the fake server");

        let mut config = counting_config(dir.path());
        config.mcp.servers[0].command = Some("python3".to_string());
        config.mcp.servers[0].args = vec![script.to_string_lossy().to_string()];
        let endpoint = refusing_endpoint().await;
        config.client.base_url = format!("http://{endpoint}/v1");
        config.client.api_key = None;

        // The daemon: ONE runtime, one agent, one MCP child set.
        let runtime = AcpRuntime::start(config).expect("the ACP runtime starts");
        let socket_path = dir.path().join("acp.sock");
        let handler = Arc::new(AcpCliHandler::new(runtime.clone()));
        let server_socket = socket_path.clone();
        tokio::spawn(async move {
            let _ = operant_core::acp::server::run_socket_server(handler, &server_socket).await;
        });
        // Wait for the socket to exist.
        for _ in 0..200 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket_path.exists(), "the daemon must bind its socket");

        // Client 1: a command (the turn fails at the refusing endpoint).
        let mut first = tokio::net::UnixStream::connect(&socket_path)
            .await
            .expect("client 1 connects");
        use tokio::io::AsyncWriteExt;
        first
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"command","params":{"command":"hello"}}"#,
            )
            .await
            .expect("client 1 writes");
        first
            .write_all(b"\n")
            .await
            .expect("client 1 writes the newline");
        first.flush().await.expect("client 1 flushes");

        // Client 2: a ping, WHILE client 1's turn is running.
        let mut second = tokio::net::UnixStream::connect(&socket_path)
            .await
            .expect("client 2 connects");
        second
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"ping","params":{}}"#)
            .await
            .expect("client 2 writes");
        second
            .write_all(b"\n")
            .await
            .expect("client 2 writes the newline");
        second.flush().await.expect("client 2 flushes");

        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(second);
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            reader.read_line(&mut line),
        )
        .await
        .expect("client 2 is answered while client 1's turn runs")
        .expect("client 2 reads");
        assert!(
            line.contains("pong"),
            "client 2 must be served by the SAME daemon while client 1's turn is in \
             flight, got: {}",
            line.trim()
        );

        // ONE child set for both clients.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while runtime.builds() == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            runtime.builds(),
            1,
            "the daemon must build its agent exactly ONCE for both clients"
        );
        let connects = std::fs::read_to_string(&count_log).unwrap_or_default();
        let lines = connects.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(
            lines, 1,
            "both clients share ONE MCP child set — got {lines} connections"
        );

        match previous_home {
            Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
            None => unsafe { std::env::remove_var("HERMES_HOME") },
        }
    }
}

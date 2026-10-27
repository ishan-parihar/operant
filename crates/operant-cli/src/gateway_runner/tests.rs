//! The gateway-runner main test suite, moved to its own file (2026-10-25
//! split). `super` is still `gateway_runner`, so every reference inside is
/// unchanged from when this was an inline `mod tests`.
/// An EMPTY message id must NOT suppress the dispatch loop's full
/// send.
///
/// Only Telegram overrode `send_message_return_id`; the default sends
/// the message and returns an EMPTY id, which means "this platform
/// cannot edit". On Discord and Slack the later chunks' edits were
/// no-ops, so the user received the FIRST CHUNK of the answer and
/// never the rest — and the recorded delivery told the dispatch loop
/// to skip its full send (RT-105).
#[test]
fn an_empty_message_id_does_not_suppress_the_full_send() {
    assert!(
        !should_record_stream_delivery(""),
        "an empty id means the platform cannot edit — the full answer \
             must still be sent"
    );
    assert!(
        should_record_stream_delivery("1234"),
        "a real id means the platform can edit — the duplicate must be \
             suppressed"
    );
}

#[test]
fn endpoint_edit_is_reported_once_after_the_baseline() {
    let endpoint = |url: &str| ConfigEndpoint {
        base_url: url.into(),
        api_key: None,
        model: "m".into(),
    };
    let mut last = None;
    assert_eq!(
        endpoint_changed_since(&mut last, endpoint("http://a")),
        None
    );
    assert_eq!(
        endpoint_changed_since(&mut last, endpoint("http://a")),
        None
    );
    assert_eq!(
        endpoint_changed_since(&mut last, endpoint("http://b")),
        Some(endpoint("http://b"))
    );
    assert_eq!(
        endpoint_changed_since(&mut last, endpoint("http://b")),
        None
    );
}

use super::*;
use operant_core::config::GatewaySettings;

/// The gateway must actually SWEEP the session store.
///
/// `prune_old_entries` existed since it was written and was called ONLY
/// from its own tests, so every conversation the gateway ever saw stayed
/// in the store forever — the prune existed and nothing called it.
///
/// This test drives the REAL sweep — the same function the periodic task
/// runs — on a real store, and observes the session leave. It is
/// deliberately NOT a re-implementation: a test that calls
/// `prune_old_entries` directly proves the prune works, not that the
/// gateway calls it, and that is the difference this defect turned on.
/// Emptying the sweep body fails this test, and so does removing the
/// spawn that starts it.
#[tokio::test]
async fn the_gateway_sweep_prunes_idle_sessions() {
    use operant_core::gateway_session::{PersistentSessionStore, SessionSource};

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let store = std::sync::Arc::new(
        PersistentSessionStore::open(db_path.to_str().expect("utf-8 path"))
            .expect("open the store"),
    );
    let source = SessionSource {
        platform: "telegram".to_string(),
        chat_id: "c1".to_string(),
        chat_name: None,
        chat_type: "dm".to_string(),
        user_id: Some("u1".to_string()),
        user_name: None,
        thread_id: None,
        chat_topic: None,
        user_id_alt: None,
        chat_id_alt: None,
        is_bot: false,
        guild_id: None,
        parent_chat_id: None,
        message_id: None,
        role_authorized: false,
    };
    store.get_or_create_session(&source, false).expect("create");
    assert_eq!(store.session_count(), 1);

    // Age the entry past the cutoff.
    store
        .set_session_activity("agent:main:telegram:dm:c1", "2000-01-01T00:00:00+00:00")
        .expect("age the session");

    // Run the REAL sweep on a fast period. The sweep skips its first
    // tick, so the prune lands on the second one.
    let handle = tokio::spawn(session_retention_sweep(
        store.clone(),
        30,
        std::time::Duration::from_millis(10),
    ));

    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if store.session_count() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the sweep must prune the idle session within the deadline");

    handle.abort();
    assert_eq!(store.session_count(), 0);
}

/// iter-720 (plan-2026-10-09 §3): the REAL `record_feed` override's
/// seat-map → `record_feed_item` → DB-row link had no test — the core
/// route-time branch is pinned with a counting stub and the injector
/// round-trip is core-tested, but the handler wiring between them was
/// exercised by nothing. Real injector over a temp db: an unmapped
/// channel defaults to `premiere`; a `feed_seat_map` entry routes the
/// post to the mapped seat. The agent stub is never invoked.
#[tokio::test]
async fn record_feed_routes_channel_posts_through_the_seat_map() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(
                operant_core::client::OpenAIClient::new(
                    operant_core::client::ClientConfig::default(),
                ),
            ),
        ),
        operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
        agent_db.clone(),
    ));
    let settings = operant_core::config::ContextInjectionSettings {
        enabled: true,
        feed_seat_map: std::collections::HashMap::from([(
            "-100mapped".to_string(),
            "compass".to_string(),
        )]),
        ..Default::default()
    };
    let injector = Arc::new(
        operant_core::org::context_injection::ContextInjector::open(
            &dir.path().join("agent.sqlite"),
            settings,
        )
        .expect("injector"),
    );
    let handler = GatewayMessageHandler {
        admins: vec![],
        agent,
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: Arc::new(Mutex::new(std::collections::HashMap::new())),
        turn_usage: Arc::new(Mutex::new(std::collections::HashMap::new())),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: Some(injector),
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        // RT-175: runtime-command handles (unused by this suite — see
        // slow_echo_handler for the rationale).
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // Unmapped channel → the `premiere` default (same default as the
    // DM tap's employee binding).
    let unmapped = IncomingMessage::new(
        "telegram",
        "anon",
        "Some Channel",
        "-100unmapped",
        "daily update",
    )
    .with_channel_post(true);
    handler.record_feed(&unmapped);
    // Mapped channel → its seat.
    let mapped = IncomingMessage::new(
        "telegram",
        "anon",
        "Other Channel",
        "-100mapped",
        "strategy note",
    )
    .with_channel_post(true);
    handler.record_feed(&mapped);

    let org_db = dir.path().join("operant_kanban.db");
    let conn = rusqlite::Connection::open(&org_db).expect("org db");
    let mut stmt = conn
        .prepare("SELECT class, seat_hint, author, text FROM context_items ORDER BY id")
        .expect("select");
    let rows: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0).expect("class"),
                row.get(1).expect("seat"),
                row.get(2).expect("author"),
                row.get(3).expect("text"),
            ))
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        rows,
        vec![
            (
                "feed".to_string(),
                "premiere".to_string(),
                "Some Channel".to_string(),
                "daily update".to_string()
            ),
            (
                "feed".to_string(),
                "compass".to_string(),
                "Other Channel".to_string(),
                "strategy note".to_string()
            ),
        ],
        "unmapped channel defaults to premiere; mapped channel lands on its seat"
    );
}

// ── Plan 06A: one turn at a time on the shared agent ──────────────

/// A scripted model client whose every turn SLEEPS before answering
/// — the window in which a second chat's turn used to retarget the
/// shared agent mid-run. The answer echoes the last user message so
/// each session's stored transcript is self-describing.
struct SlowEchoClient {
    delay_ms: u64,
}

fn echo_chat_response(content: &str) -> operant_core::client::ChatResponse {
    use operant_core::client::{Choice, MessageDelta, Role, Usage};
    operant_core::client::ChatResponse {
        id: "resp_echo".into(),
        object: "chat.completion".into(),
        created: 0,
        model: "slow-echo".into(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(content.to_string()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some("stop".into()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

#[async_trait::async_trait]
impl operant_core::agent::ModelClient for SlowEchoClient {
    fn provider_name(&self) -> &str {
        "slow-echo"
    }

    async fn chat(
        &self,
        request: operant_core::agent::ChatRequest,
    ) -> operant_core::error::Result<operant_core::client::ChatResponse> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        let echo = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == operant_core::client::Role::User)
            // The user's own text: the per-turn `<turn_context>` block
            // (workspace files, recall) rides the latest user message.
            .map(|m| {
                m.content
                    .rsplit("</turn_context>\n\n")
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default();
        Ok(echo_chat_response(&format!("answer-{echo}")))
    }

    async fn chat_streaming(
        &self,
        _request: operant_core::agent::ChatRequest,
    ) -> operant_core::error::Result<
        futures::stream::BoxStream<
            'static,
            operant_core::error::Result<operant_core::agent::StreamChunk>,
        >,
    > {
        Err(operant_core::error::Error::Agent(
            "streaming not used in this test".into(),
        ))
    }
}

/// A handler over ONE shared agent with the scripted client — the
/// production shape: every chat talks to the same `Arc<OperantAgent>`.
fn slow_echo_handler(dir: &tempfile::TempDir, delay_ms: u64) -> Arc<GatewayMessageHandler> {
    let db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let cfg = operant_core::agent::AgentConfig {
        stream: false, // the scripted client answers through `chat`
        ..operant_core::agent::AgentConfig::default()
    };
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        cfg,
        Box::new(SlowEchoClient { delay_ms }),
        operant_core::tools::ToolRegistry::new(Duration::from_secs(5)),
        db,
    ));
    Arc::new(GatewayMessageHandler {
        admins: vec![],
        agent,
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        // RT-175: runtime-command handles. Empty MCP set (no servers to
        // reconnect) and a nonexistent skills dir (a rescan loads zero
        // skills) — the tests exercise turns, not /reload-*.
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    })
}

/// Two chats, one shared agent, both turns in flight at once. The
/// model sleeps inside each turn, so pre-06A chat B's `set_session_id`
/// landed mid-A: A's answer persisted under B's session id and A's
/// stored transcript never saw its own answer. With the turn lock the
/// turns serialize and each session's store holds exactly its own
/// user message and answer.
#[tokio::test]
async fn concurrent_chats_keep_disjoint_transcripts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let handler = slow_echo_handler(&dir, 200);
    let msg_a = IncomingMessage::new("telegram", "u-a", "A", "c-a", "from-A");
    let msg_b = IncomingMessage::new("discord", "u-b", "B", "c-b", "from-B");

    let h = Arc::clone(&handler);
    let turn_a = tokio::spawn(async move { h.handle(msg_a).await });
    let h = Arc::clone(&handler);
    let turn_b = tokio::spawn(async move { h.handle(msg_b).await });

    let out_a = turn_a.await.expect("join A").expect("A turn completes");
    let out_b = turn_b.await.expect("join B").expect("B turn completes");
    assert!(
        out_a.content.contains("answer-from-A"),
        "{:?}",
        out_a.content
    );
    assert!(
        out_b.content.contains("answer-from-B"),
        "{:?}",
        out_b.content
    );

    // Each stored conversation holds ONLY its own user message and
    // answer — the corruption pre-06A put A's answer in B's rows.
    for (platform, channel, user, tag) in [
        ("telegram", "c-a", "u-a", "A"),
        ("discord", "c-b", "u-b", "B"),
    ] {
        let key = format!("{platform}:{channel}:{user}");
        let sid = derived_gateway_session_id(&key);
        handler.agent.set_session_id(sid);
        let contents: Vec<String> = handler
            .agent
            .conversation()
            .await
            .iter()
            .map(|m| m.content.clone())
            .collect();
        assert_eq!(
            contents,
            vec![format!("from-{tag}"), format!("answer-from-{tag}")],
            "session {key} must hold only its own turn"
        );
    }
}

/// `/stop` may only attach the shared interrupt flag for the session
/// whose turn actually holds the lock. Pre-06A every entry in
/// `turn_inflight` got the flag, so a `/stop` naming a queued chat
/// killed the RUNNING chat's turn.
#[tokio::test]
async fn stop_attaches_only_to_the_running_turns_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let handler = slow_echo_handler(&dir, 300);
    let key_a = "telegram:c-a:u-a".to_string();
    let key_b = "discord:c-b:u-b".to_string();
    // Both turns registered the way the dispatch loop registers them.
    let mut turn_inflight: HashMap<String, InterruptFlag> = HashMap::new();
    turn_inflight.insert(key_a.clone(), handler.agent.interrupt_flag());
    turn_inflight.insert(key_b.clone(), handler.agent.interrupt_flag());

    let h = Arc::clone(&handler);
    let msg_a = IncomingMessage::new("telegram", "u-a", "A", "c-a", "from-A");
    let turn = tokio::spawn(async move { h.handle(msg_a).await });

    // Wait until A holds the lock (running_session names A).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let running = handler
            .running_session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if running.as_deref() == Some(key_a.as_str()) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "A never started — running_session never named A"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // B is registered but NOT running: its /stop must carry no flag —
    // the shared flag here would let /stop in B kill A's live turn.
    assert!(
        handler
            .stop_flag_if_running(&turn_inflight, &key_b)
            .is_none(),
        "a non-running session must not receive the shared interrupt flag"
    );
    assert!(
        handler
            .stop_flag_if_running(&turn_inflight, &key_a)
            .is_some(),
        "the running session's /stop must find its flag"
    );

    turn.await.expect("join A").expect("A completes normally");
}

/// W3-2: `/stop` for a session whose turn is queued behind another
/// chat's turn cancels it — the queued turn exits with `Interrupted`
/// the moment it would start, and never runs the agent.
#[tokio::test]
async fn stop_cancels_a_queued_turn_before_it_starts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let handler = slow_echo_handler(&dir, 300);
    let key_a = "telegram:c-a:u-a".to_string();
    let key_b = "discord:c-b:u-b".to_string();
    let mut turn_inflight: HashMap<String, InterruptFlag> = HashMap::new();
    turn_inflight.insert(key_a.clone(), handler.agent.interrupt_flag());
    turn_inflight.insert(key_b.clone(), handler.agent.interrupt_flag());

    let h = Arc::clone(&handler);
    let msg_a = IncomingMessage::new("telegram", "u-a", "A", "c-a", "from-A");
    let turn_a = tokio::spawn(async move { h.handle(msg_a).await });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while handler
        .running_session
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_deref()
        != Some(key_a.as_str())
    {
        assert!(std::time::Instant::now() < deadline, "A never started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // B queues behind A, then B's user /stops it.
    let h = Arc::clone(&handler);
    let msg_b = IncomingMessage::new("discord", "u-b", "B", "c-b", "from-B");
    let turn_b = tokio::spawn(async move { h.handle(msg_b).await });
    assert!(
        handler.cancel_if_queued(&turn_inflight, &key_b),
        "B is registered but not running: its /stop must cancel the queued turn"
    );
    assert!(
        !handler.cancel_if_queued(&turn_inflight, &key_a),
        "the RUNNING turn is stopped via the interrupt flag, not the queue path"
    );
    assert!(
        !handler.cancel_if_queued(&turn_inflight, "slack:idle"),
        "an idle session has nothing queued"
    );

    turn_a.await.expect("join A").expect("A completes normally");
    let b = turn_b.await.expect("join B");
    assert!(
        matches!(b, Err(operant_core::Error::Interrupted)),
        "B must exit Interrupted without running: {b:?}"
    );
}

/// iter-632 regression (Wave-4 metering wire): a turn's accumulated
/// usage must land in the `gateway_sessions` accumulator the budget
/// rollup reads. Before the wire, `update_tokens` had zero production
/// callers — `employee_window_usage` returned (0, 0.0) forever, the
/// hard gate at turn start could never trip, and `<budget_state>`
/// always claimed the full cap remained.
#[tokio::test]
async fn wave4_turn_usage_drains_into_the_budget_accumulator() {
    use operant_core::gateway_session::{PersistentSessionStore, SessionSource};

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let store =
        PersistentSessionStore::open(db_path.to_str().expect("utf-8 path")).expect("open store");

    let source = SessionSource {
        platform: "telegram".to_string(),
        chat_id: "42".to_string(),
        chat_name: None,
        chat_type: "dm".to_string(),
        user_id: Some("7".to_string()),
        user_name: None,
        thread_id: None,
        chat_topic: None,
        user_id_alt: None,
        chat_id_alt: None,
        is_bot: false,
        guild_id: None,
        parent_chat_id: None,
        message_id: None,
        role_authorized: false,
    };
    let entry = store
        .get_or_create_session(&source, false)
        .expect("create session");
    store
        .bind_employee(&entry.session_key, "dp-the-program")
        .expect("bind employee");

    // The receiver folds two model calls' Usage/Cost events into the
    // per-thread map — exactly as the live arms do.
    let map: TurnUsageMap = Arc::new(Mutex::new(HashMap::new()));
    let key: ThreadKey = ("telegram".to_string(), "42".to_string(), None);
    {
        let mut guard = map.lock().await;
        let u = guard.entry(key.clone()).or_default();
        u.input_tokens = 400;
        u.output_tokens = 100;
        u.cost_usd = 0.025;
    }

    drain_turn_usage_into_store(
        &store,
        &map,
        key,
        &entry.session_key,
        Some("dp-the-program"),
    )
    .await;

    // The map is drained (no leak into the next turn).
    assert!(
        map.lock().await.is_empty(),
        "turn usage entry must be consumed"
    );

    // The accumulator the Wave-4 rollup reads now sees real spend.
    let since = "1970-01-01T00:00:00+00:00".to_string();
    let (tokens, usd) = store
        .employee_window_usage("dp-the-program", &since)
        .expect("rollup");
    assert_eq!(tokens, 500);
    assert!((usd - 0.025).abs() < 1e-9);
}

#[test]
fn format_turn_elapsed_boundaries() {
    assert_eq!(format_turn_elapsed(std::time::Duration::from_secs(0)), "0s");
    assert_eq!(
        format_turn_elapsed(std::time::Duration::from_secs(59)),
        "59s"
    );
    assert_eq!(
        format_turn_elapsed(std::time::Duration::from_secs(60)),
        "1m 0s"
    );
    assert_eq!(
        format_turn_elapsed(std::time::Duration::from_secs(1207)),
        "20m 7s"
    );
}

#[test]
fn grace_call_with_empty_partial_marks_stopped() {
    // The 2026-10-03 specimen shape: A1's gate emptied a degenerate
    // grace summary — the operator must read stopped + elapsed + reason,
    // never silence. (Before S5 this fell through to the generic
    // provider-overload wording.)
    let out = turn_end_content(
        "",
        None,
        Some(operant_core::agent::TurnExitReason::GraceCall),
        std::time::Duration::from_secs(1207),
    );
    assert!(out.contains("stopped early"), "{out}");
    assert!(out.contains("20m 7s"), "{out}");
    assert!(out.contains("grace_call"), "{out}");
    assert!(out.contains("continue"), "{out}");
    assert!(!out.contains("provider overload"), "{out}");
}

#[test]
fn circuit_breaker_with_partial_keeps_and_labels_it() {
    let out = turn_end_content(
        "Found three of five records.",
        None,
        Some(operant_core::agent::TurnExitReason::CircuitBreaker),
        std::time::Duration::from_secs(45),
    );
    assert!(out.contains("stopped early"), "{out}");
    assert!(out.contains("45s"), "{out}");
    assert!(out.contains("circuit_breaker"), "{out}");
    assert!(out.contains("Found three of five records."), "{out}");
}

#[test]
fn empty_with_reasoning_keeps_reasoning_fallback_wording() {
    // Positive control for the pre-existing parity path: a TextResponse
    // turn with empty content + a reasoning trace still surfaces the
    // reasoning verbatim (R32).
    let out = turn_end_content(
        "",
        Some("thinking trace"),
        None,
        std::time::Duration::from_secs(3),
    );
    assert_eq!(out, "thinking trace");
}

#[test]
fn empty_without_reasoning_keeps_provider_fallback_wording() {
    let out = turn_end_content("", None, None, std::time::Duration::from_secs(3));
    assert!(out.contains("empty response after retries"), "{out}");
    assert!(!out.contains("stopped early"), "{out}");
}

#[test]
fn normal_answer_passes_through_unmodified() {
    // The positive control for the whole cascade: a normal turn is
    // byte-identical before and after the extraction.
    let out = turn_end_content(
        "Full answer.",
        None,
        None,
        std::time::Duration::from_secs(3),
    );
    assert_eq!(out, "Full answer.");
}

#[test]
fn build_adapters_wires_whatsapp_phone_number_id() {
    // R22: WhatsApp outbound was permanently broken — the adapter's
    // `phone_number_id` (required for the Graph API URL) had no wiring
    // path: no config field, wizard skipped it, factory never called
    // `with_phone_number_id`. This guards the full chain.
    let config = GatewayConfig {
        whatsapp_enabled: true,
        whatsapp_token: Some("wa-token".to_string()),
        whatsapp_phone_number_id: Some("123456789".to_string()),
        ..GatewayConfig::default()
    };
    let adapters = build_adapters(&config);
    let wa = adapters
        .iter()
        .find(|a| a.name() == "whatsapp")
        .expect("whatsapp adapter built when enabled");
    assert!(
        wa.config_json()["phone_number_id_configured"]
            .as_bool()
            .unwrap(),
        "configured phone_number_id must reach the adapter"
    );

    // Without phone_number_id the adapter must report the misconfig so
    // doctor/status surfaces surface it instead of 404ing at send time.
    let bare = GatewayConfig {
        whatsapp_enabled: true,
        whatsapp_token: Some("wa-token".to_string()),
        ..GatewayConfig::default()
    };
    let adapters = build_adapters(&bare);
    let wa = adapters
        .iter()
        .find(|a| a.name() == "whatsapp")
        .expect("whatsapp adapter built when enabled");
    assert!(
        !wa.config_json()["phone_number_id_configured"]
            .as_bool()
            .unwrap(),
        "missing phone_number_id must be visible in config_json"
    );
}

#[test]
fn yolo_set_clear_and_enabled_roundtrip() {
    set_yolo("telegram", "chan-a", true);
    set_yolo("telegram", "chan-b", false);
    assert!(yolo_enabled("telegram", "chan-a"));
    assert!(
        !yolo_enabled("telegram", "chan-b"),
        "explicitly-disabled channel must not be YOLO"
    );
    assert!(
        !yolo_enabled("slack", "chan-a"),
        "platform must be part of the key"
    );
    assert!(
        !yolo_enabled("telegram", "chan-other"),
        "unrelated channel must not be YOLO"
    );
    set_yolo("telegram", "chan-a", false);
    assert!(
        !yolo_enabled("telegram", "chan-a"),
        "cleared channel must not be YOLO"
    );
}

#[tokio::test]
async fn test_build_adapters_all_disabled() {
    let config = GatewayConfig {
        telegram_enabled: false,
        telegram_token: None,
        discord_enabled: false,
        discord_token: None,
        slack_enabled: false,
        slack_token: None,
        whatsapp_enabled: false,
        whatsapp_token: None,
        whatsapp_phone_number_id: None,
        email_enabled: false,
        email_smtp_host: None,
        email_smtp_user: None,
        email_smtp_pass: None,
        sms_twilio_enabled: false,
        webhooks_enabled: false,
        webhooks_addr: None,
        webhooks_secret: None,
        holon_notion: None,
        admins: vec![],
        streaming_transport: "auto".to_string(),
        telegram_proxy: None,
        telegram_bot_username: None,
        telegram_dm_topics_enabled: false,
        max_concurrent_sessions: None,
    };
    let adapters = build_adapters(&config);
    assert_eq!(adapters.len(), 0);
}

#[tokio::test]
async fn test_build_adapters_telegram_enabled() {
    // O-28 (audit): the coverage iter-831 neutered — an ENABLED
    // telegram (token injected, no network touched at build time)
    // constructs its adapter. The old version flipped every platform
    // off and still asserted a telegram adapter.
    let config = GatewayConfig {
        telegram_enabled: true,
        telegram_token: Some("test-token".to_string()),
        discord_enabled: false,
        discord_token: None,
        slack_enabled: false,
        slack_token: None,
        whatsapp_enabled: false,
        whatsapp_token: None,
        whatsapp_phone_number_id: None,
        email_enabled: false,
        email_smtp_host: None,
        email_smtp_user: None,
        email_smtp_pass: None,
        sms_twilio_enabled: false,
        webhooks_enabled: false,
        webhooks_addr: None,
        webhooks_secret: None,
        holon_notion: None,
        admins: vec![],
        streaming_transport: "auto".to_string(),
        telegram_proxy: None,
        telegram_bot_username: None,
        telegram_dm_topics_enabled: false,
        max_concurrent_sessions: None,
    };
    let adapters = build_adapters(&config);
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].name(), "telegram");
}

#[tokio::test]
async fn test_build_adapters_multi_platform() {
    // O-28 (audit): restore the intent — two enabled platforms build
    // two adapters (tokens injected; the constructors do not dial).
    let config = GatewayConfig {
        telegram_enabled: true,
        telegram_token: Some("test-token".to_string()),
        discord_enabled: false,
        discord_token: None,
        slack_enabled: true,
        slack_token: Some("slack-token".to_string()),
        whatsapp_enabled: false,
        whatsapp_token: None,
        whatsapp_phone_number_id: None,
        email_enabled: false,
        email_smtp_host: None,
        email_smtp_user: None,
        email_smtp_pass: None,
        sms_twilio_enabled: false,
        webhooks_enabled: true,
        webhooks_addr: None,
        webhooks_secret: None,
        holon_notion: None,
        admins: vec![],
        streaming_transport: "auto".to_string(),
        telegram_proxy: None,
        telegram_bot_username: None,
        telegram_dm_topics_enabled: false,
        max_concurrent_sessions: None,
    };
    let adapters = build_adapters(&config);
    assert_eq!(adapters.len(), 3, "telegram + slack + webhook");
    let names: Vec<&str> = adapters.iter().map(|a| a.name()).collect();
    assert!(names.contains(&"telegram"), "{names:?}");
    assert!(names.contains(&"slack"), "{names:?}");
}

#[test]
fn test_gateway_config_from_appconfig_fields() {
    let app_config = AppConfig {
        gateway: GatewaySettings {
            telegram_enabled: true,
            telegram_token: Some("tok".to_string()),
            discord_enabled: false,
            discord_token: None,
            slack_enabled: true,
            slack_token: Some("stok".to_string()),
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            admins: vec!["admin1".to_string()],
            ..Default::default()
        },
        ..Default::default()
    };

    let gw_config = gateway_config_from_app(&app_config);

    assert!(gw_config.telegram_enabled);
    assert!(!gw_config.discord_enabled);
    assert!(gw_config.slack_enabled);
    assert!(!gw_config.webhooks_enabled);
    assert_eq!(gw_config.admins, vec!["admin1"]);
}

#[tokio::test]
async fn test_send_channel_message_no_enabled_platforms_errors() {
    // Regression for R15-2: `operant channel send` used to print a fake
    // "sent" status without delivering. With nothing enabled it must
    // return a real error explaining the config fix.
    let app_config = AppConfig::default();
    let err = send_channel_message(&app_config, "telegram", "123", "hi")
        .await
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("No channels are enabled"),
        "unexpected error: {msg}"
    );
    assert!(msg.contains("telegram_enabled = true"));
}

#[tokio::test]
async fn test_send_channel_message_unknown_platform_errors() {
    // A configured-but-unknown platform name must surface the gateway's
    // real error instead of a fake success.
    let app_config = AppConfig {
        gateway: GatewaySettings {
            telegram_enabled: true,
            telegram_token: Some("tok".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    let err = send_channel_message(&app_config, "pager", "123", "hi")
        .await
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("Unknown platform"), "unexpected error: {msg}");
}

/// The gateway start/stop round trip, with no platform enabled.
///
/// The home is isolated through the ENVIRONMENT, not through the
/// thread-local profile override: `platform::operant_home()` reads the
/// env var, and the gateway's stores live there — the PID FILE above
/// all. With the thread-local override the test wrote the operator's
/// REAL `~/.operant/gateway.pid` and then deleted it, so a test run
/// destroyed the pid of a gateway the operator had running. That is the
/// same class as the ESTOP incident (iter-935). (iter-1134)
#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "test-only: the std mutex serialises process-env mutation across the whole test"
)]
async fn test_gateway_start_stop_with_disabled_platforms() {
    let _env_lock = crate::config::test_env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let previous_home = std::env::var("HERMES_HOME").ok();
    // SAFETY: serialised by the crate's environment lock.
    unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
    // PROVE the isolation before writing anything (the RT-99 pattern):
    // a test that cannot prove it is writing to its own sandbox must not
    // write at all.
    assert_eq!(
        operant_core::platform::operant_home(),
        tmp.path(),
        "the operant home must be the test's temp directory — the gateway \
             writes its pid file there, and the operator's real one must not \
             be touched"
    );

    stop_gateway().await.ok();

    // Ensure a fresh DB exists with the full schema before gateway touches it
    let db_path = tmp.path().join("database.db");
    let _db = operant_core::database::Database::init(db_path.clone()).expect("init test db");

    let config = AppConfig {
        database_path: db_path,
        gateway: GatewaySettings {
            telegram_enabled: false,
            discord_enabled: false,
            slack_enabled: false,
            webhooks_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };

    let start_msg = start_gateway(&config).await.unwrap();
    let running = is_running().await;
    let stop_msg = stop_gateway().await.unwrap();
    let not_running = is_running().await;

    // The PID file must be the TEST's, in the TEST's home — never the
    // operator's.
    let pid_path = tmp.path().join("gateway.pid");
    let pid_after_stop = std::fs::read_to_string(&pid_path).ok();

    // Restore the environment BEFORE any assertion, so a failing assert
    // cannot leave the rest of the test binary with a redirected home.
    match previous_home {
        Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
        None => unsafe { std::env::remove_var("HERMES_HOME") },
    }

    assert!(start_msg.contains("0 platform(s)"));
    assert!(running);
    assert!(stop_msg.contains("stopped"));
    assert!(!not_running);
    assert_eq!(
        pid_after_stop, None,
        "the gateway pid file must be removed on stop, in the test's own home"
    );
}

/// iter-1338: the queue-wait measurement — the ponytail gate the
/// turn_lock comment names. The agent-per-seat upgrade is taken when the
/// numbers show a busy chat starving others, so the numbers must be
/// honest: events, the total, the worst single wait, the busiest chat.
#[test]
fn queue_wait_stats_count_events_total_worst_and_busiest_chat() {
    use super::QueueWaitStats;
    let mut stats = QueueWaitStats::default();

    // The empty state says so — no chat has waited, not "zero seconds".
    assert!(
        stats.line().contains("queued turns: 0"),
        "got: {}",
        stats.line()
    );

    stats.record(std::time::Duration::from_millis(12), "telegram:c1:u1");
    stats.record(std::time::Duration::from_millis(34), "telegram:c1:u2");
    stats.record(std::time::Duration::from_millis(7), "telegram:c1:u1");

    // The busiest chat is the one that queued TWICE, and the worst is the
    // 34ms tail — the number a user actually feels.
    assert_eq!(stats.events, 3);
    assert_eq!(stats.total_ms, 53);
    assert_eq!(stats.worst_ms, 34);
    let line = stats.line();
    assert!(line.contains("queued turns: 3"), "got: {line}");
    assert!(line.contains("total 53 ms"), "got: {line}");
    assert!(line.contains("worst 34 ms"), "got: {line}");
    assert!(
        line.contains("telegram:c1:u1 ×2"),
        "busiest chat should be the two-wait session, got: {line}"
    );
}

/// iter-1335 (audit finding 1): the daemon lock is exclusive — a second
/// `gateway run` on the same machine must refuse to start, and dropping the
/// lock must free the path for the next start. The whole point of the
/// audit fix: two daemons double-poll Telegram and double the RSS.
#[test]
fn second_gateway_daemon_lock_is_refused_until_released() {
    let _env_lock = crate::config::test_env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let previous_home = std::env::var("HERMES_HOME").ok();
    // SAFETY: serialised by the crate's environment lock.
    unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
    // PROVE the isolation before writing anything (RT-99 pattern): this
    // test takes a real flock in the operant home.
    assert_eq!(
        operant_core::platform::operant_home(),
        tmp.path(),
        "the daemon lock lives in the operant home — must be the test's temp dir"
    );

    let first =
        super::acquire_gateway_daemon_lock().expect("first gateway daemon lock acquisition");
    assert!(first.is_some(), "a fresh home's lock must be acquirable");
    let second = super::acquire_gateway_daemon_lock()
        .expect("second gateway daemon lock acquisition attempt");
    assert!(
        second.is_none(),
        "a live gateway holds the flock — the second start must be refused"
    );
    drop(first);
    let third = super::acquire_gateway_daemon_lock()
        .expect("third gateway daemon lock acquisition after release");
    assert!(
        third.is_some(),
        "dropping the lock (gateway stopped) must free the next start"
    );

    match previous_home {
        Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
        None => unsafe { std::env::remove_var("HERMES_HOME") },
    }
}

/// iter-1345 — the LIFETIME half of the lock (exclusivity is tested above).
///
/// The defect live verification caught: `start_gateway` bound the acquired
/// guard to a LOCAL `let _daemon_lock`, which dropped at the function's
/// return, closed the flock fd, and let the kernel release the lock while
/// the daemon kept serving every platform. A second `gateway run` then
/// acquired the lock happily and started a DUPLICATE daemon (verified live:
/// the running daemon held zero fds on gateway.lock).
///
/// This test drives the REAL `start_gateway` (not a mirror of its shape, which
/// would keep passing if the push regressed) and then attempts the acquire a
/// second `gateway run` would make. It must be REFUSED: the guard now lives in
/// the process-global `GATEWAY_DAEMON_LOCKS` slot, so it outlives the function
/// frame.
///
/// Mutation shape: revert the production push to a local binding (the
/// iter-1335 state) and the guard drops when `start_gateway` returns — the
/// second acquire SUCCEEDS, the assertion fails, and that success is precisely
/// what let the second daemon start.
#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "test-only: the std mutex serialises process-env mutation across the whole test"
)]
async fn the_daemon_lock_survives_the_start_functions_return() {
    let _env_lock = crate::config::test_env_lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let previous_home = std::env::var("HERMES_HOME").ok();
    // SAFETY: serialised by the crate's environment lock.
    unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
    // PROVE the isolation before writing anything (RT-99 pattern): the
    // gateway takes a real flock and writes its pid file in the operant home.
    assert_eq!(
        operant_core::platform::operant_home(),
        tmp.path(),
        "the daemon lock lives in the operant home — must be the test's temp dir"
    );

    stop_gateway().await.ok();
    // Clear any guard left in the process-global slot by an earlier test in
    // this binary: a stale guard would hold a DEAD temp home's lock. Clearing
    // is the in-process restart the slot's own comment describes — the push
    // adds a NEW guard for this start.
    {
        if let Ok(mut held) = super::GATEWAY_DAEMON_LOCKS.lock() {
            held.clear();
        }
    }

    // Ensure a fresh DB exists with the full schema before the gateway touches it.
    let db_path = tmp.path().join("database.db");
    let _db = operant_core::database::Database::init(db_path.clone()).expect("init test db");

    let config = AppConfig {
        database_path: db_path,
        gateway: GatewaySettings {
            telegram_enabled: false,
            discord_enabled: false,
            slack_enabled: false,
            webhooks_enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };

    // Drive the REAL start: it acquires the lock and (the fix) leaks the guard
    // into the process-global slot. No platform is enabled, so this starts
    // nothing but the lock and the runner handle.
    let start_msg = start_gateway(&config)
        .await
        .expect("the first start must succeed");
    assert!(
        start_msg.contains("0 platform(s)"),
        "the gateway started with no platforms, got: {start_msg}"
    );

    // THE ASSERTION: after `start_gateway` RETURNED, a second attempt — what a
    // second `gateway run` does — must still be refused. With the iter-1335
    // local binding the guard dropped at the return above, the kernel released
    // the flock, and this acquire would return `Some` (the second daemon).
    let second = super::acquire_gateway_daemon_lock().expect("second acquire attempt");
    assert!(
        second.is_none(),
        "the guard leaked into the process-global slot keeps the flock held \
         after `start_gateway` returned — a second `gateway run` must be refused \
         (the iter-1345 defect was a second daemon while the first kept serving)"
    );
    drop(second);

    // Restore the environment BEFORE any further gateway work, so a failure
    // cannot leave the rest of the test binary with a redirected home.
    match previous_home {
        Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
        None => unsafe { std::env::remove_var("HERMES_HOME") },
    }

    // Stop first, then clear the slot.
    stop_gateway().await.ok();
    {
        if let Ok(mut held) = super::GATEWAY_DAEMON_LOCKS.lock() {
            held.clear();
        }
    }
}

/// RT-175 — the /goal + /subgoal TURN-SIDE read. The commands write
/// `goal`/`subgoal` + `goal_pending_inject` on the session row; the
/// turn must inject the `[session goal]` marker into the query and
/// consume the flag. Before this read existed, /goal replied "set" and
/// the agent never saw the goal (theater). Mutation shape: remove the
/// injection block (or move it after the `query` binding, the bug my
/// first draft had) and the echoed answer loses the marker; skip the
/// flag consumption and the SECOND turn re-injects forever.
#[tokio::test]
async fn pending_goal_is_injected_into_the_next_turn_once() {
    use operant_core::gateway::GatewayConfig;

    let dir = tempfile::tempdir().expect("tempdir");
    let handler = slow_echo_handler(&dir, 0);

    // A real Gateway, attached the way start_gateway does — the read
    // reaches the session store through it.
    let gateway = Arc::new(operant_core::gateway::Gateway::new(GatewayConfig::default()));
    {
        let mut gw_slot = handler.gateway.lock().await;
        *gw_slot = Some(gateway.clone());
    }

    // The session row the /goal command would have written: goal set,
    // pending-inject flag armed.
    {
        let store = gateway.get_session_store();
        store
            .create_session("telegram", "u-goal", "c-goal")
            .expect("session row");
        assert!(store.update_session_metadata(
            "telegram",
            "u-goal",
            "c-goal",
            &[
                ("goal".to_string(), "ship the release".to_string()),
                ("subgoal".to_string(), "write the changelog".to_string()),
                ("goal_pending_inject".to_string(), "true".to_string()),
            ],
        ));
    }

    // Turn 1: the goal MUST reach the agent's query.
    let msg = IncomingMessage::new("telegram", "u-goal", "Goal User", "c-goal", "status update");
    let out = handler.handle(msg).await.expect("turn 1 completes");
    assert!(
        out.content.contains("[session goal] ship the release"),
        "the goal marker must reach the agent's query, got: {:?}",
        out.content
    );
    assert!(
        out.content
            .contains("[session subgoals] write the changelog"),
        "the subgoal marker must ride the same injection, got: {:?}",
        out.content
    );

    // The flag was consumed: turn 2 must NOT re-inject.
    let msg2 = IncomingMessage::new("telegram", "u-goal", "Goal User", "c-goal", "next update");
    let out2 = handler.handle(msg2).await.expect("turn 2 completes");
    assert!(
        !out2.content.contains("[session goal]"),
        "the goal injects exactly once — the flag must be consumed, got: {:?}",
        out2.content
    );
    let store = gateway.get_session_store();
    let session = store
        .find_session("telegram", "u-goal", "c-goal")
        .expect("row survives");
    assert_eq!(
        session
            .metadata
            .get("goal_pending_inject")
            .map(String::as_str),
        Some("false"),
        "the consumed flag is written back"
    );
}

/// RT-175 live-acceptance regression: `/goal` can be the FIRST message a
/// chat ever sends, so no session row exists when the command runs and the
/// flag write silently no-ops (update_session_metadata returns false on a
/// missing row) — the next turn then injects nothing while the command's
/// reply promised it would. Found by the deployed-binary acceptance harness
/// (stand-in Bot-API drive), not by unit tests: every unit test pre-created
/// the session row, so the no-row path was never exercised.
#[tokio::test]
async fn goal_on_a_fresh_chat_arms_the_injection_anyway() {
    use operant_core::gateway::GatewayConfig;

    let dir = tempfile::tempdir().expect("tempdir");
    let handler = slow_echo_handler(&dir, 0);

    let gateway = Arc::new(operant_core::gateway::Gateway::new(GatewayConfig::default()));
    {
        let mut gw_slot = handler.gateway.lock().await;
        *gw_slot = Some(gateway.clone());
    }

    // NO session row — the exact fresh-chat shape the acceptance run hit.
    // The /goal command handler must create-or-find before writing.
    let config = operant_core::config::AppConfig::default();
    let ctx = crate::gateway_commands::CommandContext::new(
        Some(&gateway),
        &config,
        true,
        "u-fresh",
        "telegram",
        "c-fresh",
    );
    let reply = crate::gateway_commands::handle_command("goal", "ship it", &ctx);
    assert!(reply.is_some(), "the goal command replies");

    // The flag must have landed on a real row, not vanished.
    let store = gateway.get_session_store();
    let session = store
        .find_session("telegram", "u-fresh", "c-fresh")
        .expect("the command must have created the session row");
    assert_eq!(
        session.metadata.get("goal").map(String::as_str),
        Some("ship it"),
        "the goal text landed"
    );
    assert_eq!(
        session
            .metadata
            .get("goal_pending_inject")
            .map(String::as_str),
        Some("true"),
        "the pending-inject flag is armed on a fresh chat"
    );

    // And the next turn injects it.
    let msg = IncomingMessage::new("telegram", "u-fresh", "Fresh", "c-fresh", "go");
    let out = handler.handle(msg).await.expect("turn completes");
    assert!(
        out.content.contains("[session goal] ship it"),
        "fresh-chat goal must inject into the next turn, got: {:?}",
        out.content
    );
}

/// RT-179 plan item 2.3 — `/effort` must drive the AGENT's live
/// reasoning-effort cell, not reply plausibly. The sync-path test in
/// gateway_commands.rs only covers the "no agent here" arm; this one
/// drives the real `handle_runtime_command` through
/// `run_session_scoped_command` and then READS the cell back, which is
/// the only way a removed `set_reasoning_effort` call fails.
///
/// The turn lock and session resolution are real too, so this also guards
/// the seam: an arm that ran outside the scope would act on whichever
/// conversation happened to be loaded.
#[tokio::test]
async fn gateway_effort_settles_into_the_agents_live_cell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(
                operant_core::client::OpenAIClient::new(
                    operant_core::client::ClientConfig::default(),
                ),
            ),
        ),
        operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
        agent_db.clone(),
    ));
    let handler = GatewayMessageHandler {
        admins: vec![],
        agent: Arc::clone(&agent),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // Start from a KNOWN cell, so the assertion is about this command's
    // effect rather than the agent's default.
    agent.set_reasoning_effort(None);
    assert_eq!(agent.reasoning_effort(), None);

    let reply = handle_runtime_command(&handler, "effort", "high", "telegram:dm:u1")
        .await
        .expect("/effort must be a runtime command");
    assert!(
        reply.contains("high"),
        "the reply must name the effort now in force, got: {reply}"
    );
    assert_eq!(
        agent.reasoning_effort(),
        Some("high".to_string()),
        "/effort must settle into the agent's live cell — an arm that only \
         replies leaves the wire field unchanged, which is the metadata \
         theater R17 records"
    );

    // `off` clears it (None = the field is not sent), and `status` reads
    // the cell rather than echoing the argument.
    let off = handle_runtime_command(&handler, "effort", "off", "telegram:dm:u1")
        .await
        .expect("off must reply");
    assert!(off.contains("off"), "got: {off}");
    assert_eq!(
        agent.reasoning_effort(),
        None,
        "/effort off must clear the cell"
    );

    let status = handle_runtime_command(&handler, "effort", "status", "telegram:dm:u1")
        .await
        .expect("status must reply");
    assert!(
        status.contains("not set"),
        "status must report the real cell state, got: {status}"
    );

    // A junk level is refused and, critically, does NOT touch the cell.
    let junk = handle_runtime_command(&handler, "effort", "banana", "telegram:dm:u1")
        .await
        .expect("junk must reply");
    assert!(junk.contains("Unknown effort"), "got: {junk}");
    assert_eq!(
        agent.reasoning_effort(),
        None,
        "a refused /effort must leave the cell alone"
    );
}

/// RT-179 plan item 2.3 — the plan-mode accessor pair, pinned to the same
/// contract as `yolo_set_clear_and_enabled_roundtrip` above: the platform
/// is part of the key, clearing works, and an untoggled channel defaults
/// to OFF (a posture that defaulted ON would silently make every silent
/// channel read-only).
#[test]
fn plan_mode_set_clear_and_enabled_roundtrip() {
    set_plan_mode("telegram", "chan-a", true);
    set_plan_mode("telegram", "chan-b", false);
    assert!(
        plan_mode_enabled("telegram", "chan-a"),
        "an enabled channel must be plan mode"
    );
    assert!(
        !plan_mode_enabled("telegram", "chan-b"),
        "an explicitly-disabled channel must prompt as usual"
    );
    assert!(
        !plan_mode_enabled("slack", "chan-a"),
        "platform must be part of the key"
    );
    assert!(
        !plan_mode_enabled("telegram", "chan-other"),
        "an untoggled channel must prompt as usual"
    );
    set_plan_mode("telegram", "chan-a", false);
    assert!(
        !plan_mode_enabled("telegram", "chan-a"),
        "a cleared channel must prompt as usual"
    );
}

/// RT-179 plan item 3.4 — `/export` must write a REAL markdown file of
/// THIS chat's conversation, in the SAME format the TUI's `/transcript`
/// writes (one formatter, so the two surfaces cannot drift). Read-only
/// except for the file it creates, so the turn lock is about reading the
/// right conversation rather than about mutation.
///
/// Drives the real `handle_runtime_command` through
/// `run_session_scoped_command`, with a seeded conversation, and then reads
/// the file back — the only way "the file exists and has the right shape"
/// is a fact rather than an assertion about a reply string.
#[tokio::test]
async fn gateway_export_writes_this_sessions_transcript_as_markdown() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(
                operant_core::client::OpenAIClient::new(
                    operant_core::client::ClientConfig::default(),
                ),
            ),
        ),
        operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
        agent_db.clone(),
    ));
    let handler = GatewayMessageHandler {
        admins: vec![],
        agent: Arc::clone(&agent),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // A two-turn conversation, so the file has both roles in it.
    //
    // Seed the DURABLE store under the session id the handler will resolve,
    // NOT the agent's hot slot: `run_session_scoped_command` calls
    // `set_session_id`, which swaps the hot slot for whatever the store holds
    // for that session (events.rs:748's retarget swap). Seeding the hot slot
    // would be swapped out and the export would see an empty transcript —
    // the exact failure this shape avoids.
    let store_key =
        operant_core::session::SessionKey::new(derived_gateway_session_id("telegram:dm:u1"));
    agent.session_store().append(
        &store_key,
        operant_core::client::Message::user("what files changed today?"),
    );
    agent.session_store().append(
        &store_key,
        operant_core::client::Message::assistant("Three: src/main.rs, src/lib.rs, README.md."),
    );

    // `/export <explicit path>` — an operator who names a file gets it.
    let out_path = dir.path().join("exported.md");
    let reply = handle_runtime_command(
        &handler,
        "export",
        out_path.to_str().expect("utf-8 path"),
        "telegram:dm:u1",
    )
    .await
    .expect("/export must be a runtime command");

    assert!(
        reply.contains("exported.md"),
        "the reply must name the file it wrote, got: {reply}"
    );
    assert!(
        reply.contains("2 message"),
        "the reply must report the message count, got: {reply}"
    );
    let written = std::fs::read_to_string(&out_path).expect("the file must exist");
    // The TUI formatter's own shape — pinned here so a second format cannot
    // appear without failing this test.
    assert!(
        written.contains("**User**") && written.contains("**Operant**"),
        "the file must use the TUI export's role labels, got: {written}"
    );
    assert!(
        written.contains("what files changed today?") && written.contains("Three: src/main.rs"),
        "the file must carry this chat's actual turns, got: {written}"
    );

    // A failure must be reported AND must not claim the conversation is
    // lost: the exporter degrades without destroying state. A path whose
    // parent cannot be created (a FILE sits where the directory would go):
    // create_dir_all refuses, so the exporter must report it rather than
    // write somewhere else.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a dir").expect("write the blocker");
    let impossible = blocker.join("under-a-file.md");
    let refused = handle_runtime_command(
        &handler,
        "export",
        impossible.to_str().expect("utf-8 path"),
        "telegram:dm:u1",
    )
    .await
    .expect("even a failure must reply");
    assert!(
        refused.contains("Could not"),
        "an unwritable path must be reported, got: {refused}"
    );
}

/// RT-179 plan item 3.2 — `/memory` must report the agent's LIVE provider
/// and route a query through its own recall, over a channel. The TUI's twin
/// shells out to `operant memory`, a second process with its own store view,
/// so this is the first surface that answers in-process.
///
/// A REAL `MemoryWireProvider` over a temp dir is attached — not a stub —
/// because the point is that the command reads the same engine the turn's
/// prefetch reads. The trait surface is used on purpose (the comment at the
/// arm says why): `recall_scored` is concrete-only with no `as_any` seam, so
/// a downcast would compile and silently fail on any other provider.
use operant_core::memory_provider::MemoryProvider as _;
#[tokio::test]
async fn gateway_memory_reports_the_live_provider_and_recalls_through_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let provider = Arc::new(
        operant_core::memory_wire::MemoryWireProvider::new(dir.path().to_path_buf())
            .expect("a memory-wire provider over the temp dir"),
    );
    let agent = Arc::new(
        operant_core::agent::OperantAgent::new(
            operant_core::agent::AgentConfig::default(),
            Box::new(
                operant_core::agent::clients::openai::OpenAIModelClient::new(
                    operant_core::client::OpenAIClient::new(
                        operant_core::client::ClientConfig::default(),
                    ),
                ),
            ),
            operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
            agent_db.clone(),
        )
        .with_memory_provider(
            provider.clone() as Arc<dyn operant_core::memory_provider::MemoryProvider>
        ),
    );
    let handler = GatewayMessageHandler {
        admins: vec![],
        agent: Arc::clone(&agent),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // A bare /memory is a status report, and it must name the engine the
    // agent is ACTUALLY running — not a hard-coded default.
    let status = handle_runtime_command(&handler, "memory", "", "telegram:dm:u1")
        .await
        .expect("/memory must be a runtime command");
    assert!(
        status.contains("memory_wire"),
        "the status must name the live provider, got: {status}"
    );
    assert!(
        status.contains("Persistent: true"),
        "the status must report the engine's own persistence, got: {status}"
    );

    // A seeded fact, recalled through the SAME prefetch path the turn uses.
    provider
        .sync_turn("the deploy target is the gateway", "noted")
        .await
        .expect("seed the bank");
    // Give the background sync executor a moment: sync_turn queues on the
    // agent's FIFO worker, so the recall that follows must wait for it.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let recalled = handle_runtime_command(&handler, "memory", "deploy target", "telegram:dm:u1")
        .await
        .expect("a query must reply");
    assert!(
        recalled.contains("gateway"),
        "the recall must return the seeded fact, got: {recalled}"
    );
    assert!(
        recalled.contains("Recall for") || recalled.contains("memory"),
        "the reply must be shaped as a recall, got: {recalled}"
    );
}

/// RT-179 plan item 3.3 — `/skill` over a channel must EXPAND a real
/// installed skill, through the SAME `build_skill_invocation_message_in` the
/// TUI injects. One code path, one format, so the channel answer IS the
/// invocation message the agent will follow.
///
/// A REAL skill directory is created (the handler's `skills_root_dir`, not
/// `~/.operant`) so the arm is proven to read the configured root rather
/// than a fixture baked into the binary.
///
/// Note the command is registered as `skills` with `skill` as its alias
/// (the registry's RT-178 uniqueness guard refuses a colliding name), so
/// this drives the shared arm through the singular the operator types.
#[tokio::test]
async fn gateway_skill_expands_a_real_installed_skill() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(
                operant_core::client::OpenAIClient::new(
                    operant_core::client::ClientConfig::default(),
                ),
            ),
        ),
        operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
        agent_db.clone(),
    ));
    // The configured skills root, carrying a real skill.
    let skills_root = dir.path().join("skills");
    let skill_dir = skills_root.join("probe-skill");
    std::fs::create_dir_all(&skill_dir).expect("skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: probe-skill\ndescription: A probe\n---\n\n# Do the probe step.\n",
    )
    .expect("SKILL.md");

    let handler = GatewayMessageHandler {
        admins: vec![],
        agent: Arc::clone(&agent),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: skills_root.clone(),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // Bare /skill lists the REAL installed skills from the configured root.
    let list = handle_runtime_command(&handler, "skill", "", "telegram:dm:u1")
        .await
        .expect("/skill must be a runtime command");
    assert!(
        list.contains("probe-skill"),
        "bare /skill must list the REAL installed skill, got: {list}"
    );

    // The named form expands the skill body and folds the instruction in.
    let expanded = handle_runtime_command(
        &handler,
        "skill",
        "probe-skill run it twice",
        "telegram:dm:u1",
    )
    .await
    .expect("a named /skill must expand a real skill");
    assert!(
        expanded.contains("Do the probe step"),
        "the expansion must carry the skill's body, got: {expanded}"
    );
    assert!(
        expanded.contains("has invoked the \"probe-skill\" skill"),
        "the expansion must be the invocation message the agent follows, got: {expanded}"
    );
    assert!(
        expanded.contains("run it twice"),
        "the trailing instruction must ride the expansion, got: {expanded}"
    );

    // An unknown name must say not found — and must NOT be shaped like a
    // successful expansion.
    let missing = handle_runtime_command(&handler, "skill", "no-such-skill", "telegram:dm:u1")
        .await
        .expect("a missing skill must still reply");
    assert!(
        missing.contains("not found"),
        "a missing skill must say not found, got: {missing}"
    );
    assert!(
        !missing.contains("has invoked"),
        "a missing skill must not produce an invocation message, got: {missing}"
    );
}

/// RT-179 plan item 3.5 — `/search` must drive the REAL `session_search`
/// tool over the agent's OWN store, so the channel answer is what the model
/// itself would find. The plan said "resolve which search first": resolved
/// to session recall, because the TUI's `/search` is a ripgrep over the
/// TUI's CWD and a gateway daemon has no project root (see the arm comment).
///
/// Seeds the durable store under a session id, then searches for a token
/// only that seed carries — proving the search reaches the seeded session
/// rather than answering an empty store.
#[tokio::test]
async fn gateway_search_drives_the_session_search_tool_over_the_real_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agent_db = Arc::new(
        operant_core::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
    );
    let agent = Arc::new(operant_core::agent::OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(
                operant_core::client::OpenAIClient::new(
                    operant_core::client::ClientConfig::default(),
                ),
            ),
        ),
        operant_core::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
        agent_db.clone(),
    ));
    let handler = GatewayMessageHandler {
        admins: vec![],
        agent: Arc::clone(&agent),
        current_session_id: tokio::sync::Mutex::new(None),
        gateway: tokio::sync::Mutex::new(None),
        session_pins: Arc::new(SessionPins::default()),
        exit_reasons: ExitReasonMap::default(),
        turn_usage: TurnUsageMap::default(),
        summary_ttl_minutes: 30,
        default_budget: operant_core::config::BudgetSettings::default(),
        context_injection: None,
        turn_lock: tokio::sync::Mutex::new(()),
        running_session: std::sync::Mutex::new(None),
        cancelled_queued: Default::default(),
        mcp_manager: operant_core::mcp::McpManager::new(),
        mcp_server_configs: Vec::new(),
        skills_root_dir: dir.path().join("skills-absent"),
        queue_waits: tokio::sync::Mutex::new(Default::default()),
    };

    // Seed the durable store with a query-unique token. `run_session_scoped_command`
    // swaps in the store's contents for the resolved session id, so the seed
    // must land under the id the handler will resolve.
    let seed_key =
        operant_core::session::SessionKey::new(derived_gateway_session_id("telegram:dm:q9"));
    let seed_id = seed_key.id().to_string();
    agent.session_store().append(
        &seed_key,
        operant_core::client::Message::user("the quokkabirch hatch ships on friday"),
    );

    // A query must surface the seeded session. The FTS5 index is behind a
    // flush, so give it a moment the same way the memory twin does.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    let found = handle_runtime_command(&handler, "search", "quokkabirch", "telegram:dm:q9")
        .await
        .expect("/search must be a runtime command");
    assert!(
        found.contains("quokkabirch") || found.contains(&seed_id),
        "the search must reach the seeded session, got: {found}"
    );

    // A bare /search browses, and must say what it is browsing — the STORE
    // (all chats), not this chat — so an empty-looking browse does not read
    // as "no history anywhere".
    let browse = handle_runtime_command(&handler, "search", "", "telegram:dm:q9")
        .await
        .expect("a bare /search must browse");
    assert!(
        browse.contains("all chats"),
        "a bare /search must name the store it is browsing, got: {browse}"
    );
    assert!(
        browse.contains("Recent sessions"),
        "a bare /search must be shaped as a browse, got: {browse}"
    );
}

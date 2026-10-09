//! Governance escalation loop, end-to-end (permission-genome wave-2 slice
//! F2): a lockdown cron seat's ask is queued and denied the SAME run, the
//! approval mints a grant through `issue_grant`, and the NEXT tick runs the
//! tool without re-asking — §5's P2 acceptance, closed.
//!
//! Every test drives the REAL `CronScheduler::tick` → `run_job` →
//! `run_agent_job` → `OperantAgent::run` → `execute_tools` guard, over real
//! sqlite stores in one tempdir file (the way the gateway shares one
//! `database.db`), with a scripted model client. Nothing here re-implements
//! the guard — the same discipline `cron_session_isolation.rs` learned the
//! hard way: a test that mirrors production code instead of executing it
//! measures nothing. The stub tools write a marker file, so "the tool ran"
//! is a filesystem fact, not an echo of a mock.
//!
//! Byte-identity is asserted here too: an UNATTENDED agent with the
//! authority attached but NO policy row keeps today's ungoverned behaviour
//! exactly — the dangerous tool still enters the permission channel (the
//! dispatcher's no-active-channel auto-AllowSession shape is what the test
//! replies with), and nothing is queued. The clamp and the queue only exist
//! on the GOVERNED path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use operant_core::agent::{
    AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk, ToolPermissionRequest,
    ToolPermissionResponse,
};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::cronjobs::{CreateJobParams, CronDb, scheduler::CronScheduler};
use operant_core::database::Database;
use operant_core::org::authority::GrantDb;
use operant_core::org::employee::derive_employee_id;
use operant_core::org::employee_db::EmployeeDb;
use operant_core::org::hierarchy_edges::HierarchyEdgesDb;
use operant_core::org::notice::rfc3339;
use operant_core::org::pending_requests::{PendingRequestDb, Status};
use operant_core::org::seat_authority::{SeatApprover, SeatAuthority, SeatEscalation};
use operant_core::org::seat_policy::{SeatMode, SeatPolicy, SeatPolicySource};
use operant_core::org::seat_policy_db::SeatPolicyDb;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ── Stub tools ─────────────────────────────────────────────────────────

/// Args for the marker stubs: writes `path` so "the tool executed" is a
/// filesystem fact the test can see.
#[derive(JsonSchema, Deserialize)]
struct MarkerArgs {
    path: String,
    /// Distinguishes one scripted ask from another across ticks (the
    /// anomaly detector suppresses unchanged repeats).
    #[serde(default)]
    seq: Option<i64>,
}

/// A stub that writes the marker file. Registered twice, under two names:
/// `seat_probe` — a name no permission list mentions, so only a SEAT
/// POLICY can escalate it (the ungoverned path runs it silently) — and
/// `bash`, which sits on the hardcoded dangerous list by NAME so the
/// ungoverned path prompts for it. Neither runs a shell.
struct MarkerTool {
    name: &'static str,
}

#[async_trait]
impl OperantTool for MarkerTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Test stub — writes a marker file when it executes."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<MarkerArgs>(self.name, "Marker-writing test stub")
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: MarkerArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name, format!("bad args: {e}")),
        };
        if let Err(e) = std::fs::write(&parsed.path, "MARKER_RAN") {
            return ToolResult::error(self.name, format!("marker write failed: {e}"));
        }
        ToolResult::success(self.name, json!({ "marker": parsed.path }))
    }
}

// ── Scripted model client ──────────────────────────────────────────────

/// Pops scripted responses in order and records every message the model is
/// shown, so a test can assert what the RUN actually saw (e.g. the denial
/// sentence), not just what the stores contain.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<String>>,
    chat_calls: Mutex<usize>,
    popped_log: Mutex<Vec<String>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
            chat_calls: Mutex::new(0),
            popped_log: Mutex::new(Vec::new()),
        })
    }

    fn saw(&self, needle: &str) -> bool {
        self.count(needle) > 0
    }

    /// How many recorded messages contain `needle` — lets a test tell a
    /// tick-3 denial from the tick-1/2 denials `saw` cannot separate.
    /// Response ids popped per chat call, in order — shows script drift.
    fn popped(&self) -> Vec<String> {
        self.popped_log.lock().unwrap().clone()
    }

    fn count(&self, needle: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.contains(needle))
            .count()
    }
}

/// Lets the shared `Arc<ScriptedClient>` survive the `Box<dyn ModelClient>`
/// hand-off so the test can inspect recorded requests afterwards.
struct ScriptedClientHandle(Arc<ScriptedClient>);

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "governance-script"
    }
    async fn chat(&self, request: ChatRequest) -> operant_core::error::Result<ChatResponse> {
        *self.chat_calls.lock().unwrap() += 1;
        self.seen
            .lock()
            .unwrap()
            .extend(request.messages.iter().map(|m| m.content.clone()));
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            self.popped_log.lock().unwrap().push("(empty)".to_string());
            return Ok(text_response("out of script"));
        }
        let next = guard.remove(0);
        self.popped_log.lock().unwrap().push(next.id.clone());
        Ok(next)
    }
    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> operant_core::error::Result<BoxStream<'static, operant_core::error::Result<StreamChunk>>>
    {
        Ok(Box::pin(futures::stream::iter(vec![Ok(StreamChunk::new(
            Some("streamed".to_string()),
            None,
            None,
        ))])))
    }
}

#[async_trait]
impl ModelClient for ScriptedClientHandle {
    fn provider_name(&self) -> &str {
        self.0.provider_name()
    }
    async fn chat(&self, request: ChatRequest) -> operant_core::error::Result<ChatResponse> {
        self.0.chat(request).await
    }
    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> operant_core::error::Result<BoxStream<'static, operant_core::error::Result<StreamChunk>>>
    {
        self.0.chat_streaming(request).await
    }
}

fn text_response(content: &str) -> ChatResponse {
    ChatResponse {
        id: "resp".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(content.to_string()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

fn tool_call_response(tool: &str, arguments: &str) -> ChatResponse {
    // The id must be UNIQUE across the whole multi-tick session: the
    // per-job session is rehydrated each tick, so a repeated id collides
    // with the historical tool-result rows and the executor can treat
    // the fresh call as already-answered.
    static CALL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = CALL_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    ChatResponse {
        id: format!("resp_{tool}_{seq}"),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Some(vec![ToolCallDelta {
                    index: 0,
                    id: Some(format!("call_{tool}_{seq}")),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: tool.to_string(),
                        arguments: arguments.to_string(),
                    }),
                }]),
            },
            finish_reason: Some("tool_calls".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

/// Same shape `tests/seat_policy_run_path.rs::test_config` established:
/// the production approval mode (`smart`) so the smart gate runs ahead of
/// the seat consultation exactly as it does live, non-streaming so the
/// scripted client answers through `chat`.
fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations: 4,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        // 0, unlike the single-turn seat_policy_run_path harness: these
        // tests run THREE ticks over one scripted client, and a denied
        // tool result is an error-class signal the agent would otherwise
        // heal with EXTRA model calls — desyncing the script. With healing
        // off, every tick is exactly two calls: ask, then close.
        max_healing_attempts: 0,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
        loop_detection_enabled: true,
        approval_mode: "smart".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        // 0: a denied tool result is error-class and would otherwise be
        // re-asked with a system nudge, consuming an extra scripted reply
        // per tick and desyncing the three-tick script.
        max_retries: 0,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
    }
}

// ── World ──────────────────────────────────────────────────────────────

/// A cron world: one tempdir, one agent db, one cron db, one org db shared
/// by all five genome stores, a cron agent over the real stores, and the
/// scheduler that runs it. The agent carries a permission channel nobody
/// drains — exactly the production cron posture (D-1b's fix kept the
/// channel attached; the F2 clamp means a GOVERNED escalation never
/// depends on it).
struct World {
    _dir: tempfile::TempDir,
    scheduler: CronScheduler,
    cron_db: Arc<CronDb>,
    org_path: PathBuf,
    org_conn: Arc<Mutex<rusqlite::Connection>>,
    policies: Arc<SeatPolicyDb>,
    grants: Arc<GrantDb>,
    requests: Arc<PendingRequestDb>,
    employees: Arc<EmployeeDb>,
    edges: Arc<HierarchyEdgesDb>,
    marker: PathBuf,
    agent: Arc<OperantAgent>,
    authority: Arc<SeatAuthority>,
    client: Arc<ScriptedClient>,
    /// The agent's permission channel, left UNDRAINED by default — the
    /// production cron posture (the channel is attached, nobody answers).
    /// Tests that need the interactive arms `take()` it.
    permission_rx: Option<tokio::sync::mpsc::Receiver<ToolPermissionRequest>>,
}

/// Build the world. `script` is the model's whole multi-tick script; the
/// marker path is baked into the tool-call arguments up front.
async fn world(script: Vec<ChatResponse>, unattended: bool) -> World {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let cron_db = Arc::new(CronDb::init(dir.path().join("cron.sqlite")).expect("CronDb::init"));

    let org_path = dir.path().join("org.sqlite");
    let org_conn = Arc::new(Mutex::new(
        rusqlite::Connection::open(&org_path).expect("open org sqlite"),
    ));
    let policies = Arc::new(
        SeatPolicyDb::from_shared_connection(Arc::clone(&org_conn)).expect("SeatPolicyDb"),
    );
    let grants = Arc::new(GrantDb::from_shared_connection(Arc::clone(&org_conn)).expect("GrantDb"));
    let requests = Arc::new(
        PendingRequestDb::from_shared_connection(Arc::clone(&org_conn)).expect("PendingRequestDb"),
    );
    let employees =
        Arc::new(EmployeeDb::from_shared_connection(Arc::clone(&org_conn)).expect("EmployeeDb"));
    let edges = Arc::new(
        HierarchyEdgesDb::from_connection(
            rusqlite::Connection::open(&org_path).expect("edges conn"),
        )
        .expect("HierarchyEdgesDb"),
    );

    let marker = dir.path().join("marker.out");
    let client = ScriptedClient::new(script);

    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry
        .register(MarkerTool { name: "seat_probe" })
        .await
        .expect("register seat_probe");
    registry
        .register(MarkerTool { name: "bash" })
        .await
        .expect("register bash");

    let (permission_tx, permission_rx) = tokio::sync::mpsc::channel::<ToolPermissionRequest>(8);
    let authority = Arc::new(SeatAuthority::new(
        Arc::clone(&policies) as Arc<dyn SeatPolicySource>,
        Arc::clone(&grants),
        Arc::clone(&requests),
    ));
    let world_authority = Arc::clone(&authority);
    let agent = OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        db,
    )
    .with_permissions(permission_tx)
    .with_seat_authority(Some(authority))
    .with_unattended(unattended);

    let agent = Arc::new(agent);
    let scheduler = CronScheduler::new(Arc::clone(&cron_db), Arc::clone(&agent));

    World {
        _dir: dir,
        scheduler,
        cron_db,
        org_path,
        org_conn,
        policies,
        grants,
        requests,
        employees,
        edges,
        marker,
        agent,
        authority: world_authority,
        client,
        permission_rx: Some(permission_rx),
    }
}

/// A FRESH agent over the SAME stores, session id, and scripted client —
/// the deterministic shape for the re-run half of the acceptance: the next
/// run consults the same ledger the approver minted into, without the
/// previous run's turn-end anomaly-retry state (which belongs to the
/// model-loop harness, not to the governance under test).
async fn fresh_agent(w: &World, unattended: bool) -> Arc<OperantAgent> {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry
        .register(MarkerTool { name: "seat_probe" })
        .await
        .expect("register seat_probe");
    registry
        .register(MarkerTool { name: "bash" })
        .await
        .expect("register bash");
    let (permission_tx, _permission_rx) = tokio::sync::mpsc::channel::<ToolPermissionRequest>(8);
    let agent = OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(Arc::clone(&w.client))),
        registry,
        Arc::new(
            Database::init(
                w.marker
                    .parent()
                    .expect("tempdir parent")
                    .join("agent2.sqlite"),
            )
            .expect("Database::init"),
        ),
    )
    .with_permissions(permission_tx)
    .with_seat_authority(Some(Arc::clone(&w.authority)))
    .with_unattended(unattended);
    Arc::new(agent)
}

/// Register an agent job that is DUE RIGHT NOW (same forcing as
/// `cron_session_isolation.rs`: `create_job` seeds a future `next_run_at`,
/// so a job created normally never fires in a test).
fn add_due_job(db: &CronDb, name: &str) -> String {
    let id = db
        .create_job(CreateJobParams {
            name: name.into(),
            prompt: "write the marker".into(),
            schedule: "0 0 1 1 *".into(), // never fires on its own; forced due below
            schedule_display: "never".into(),
            repeat_times: None,
            deliver: "none".into(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: Some("probe".into()),
            skills: Some(vec!["probe".into()]),
            model: None,
            provider: None,
            base_url: None,
            script: None,
            context_from: None,
            enabled_toolsets: None,
            workdir: None,
            no_agent: false,
        })
        .expect("create_job");

    db.set_next_run(
        &id,
        Some((chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
    )
    .expect("force due");
    id
}

/// Force the job due again (a completed run reschedules it to next year).
fn force_due(db: &CronDb, job_id: &str) {
    db.set_next_run(
        job_id,
        Some((chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
    )
    .expect("force due again");
}

/// Assert that a tick actually DISPATCHED the job (same guard against the
/// vacuous pass as `cron_session_isolation.rs`).
fn assert_dispatched(db: &CronDb, job_id: &str) {
    let job = db.get_job(job_id).expect("get_job").expect("job exists");
    assert_ne!(
        job.last_run_at, None,
        "the tick never dispatched the job — run_agent_job was not reached, so any \
         assertion after this proves nothing. last_status={:?}",
        job.last_status
    );
}

/// Seed one employee row with a department. The registry's only public write
/// path is `backfill_from_cron_jobs`, which hardcodes `department: None`;
/// the §2.1 position math needs a department, so the seed writes that one
/// column through the shared connection. Documented seeding, not a store
/// bypass: every assertion below reads back through public APIs.
fn seed_employee(w: &World, id: &str, name: &str, dept: Option<&str>) {
    w.org_conn
        .lock()
        .expect("org conn")
        .execute(
            "INSERT OR REPLACE INTO employees (
                 employee_id, name, role, department, skills, agent_type,
                 persona, status, reason, created_at, updated_at
             ) VALUES (?1, ?2, 'tester', ?3, '[\"probe\"]', NULL, NULL,
                       'active', 'test seed', ?4, ?4)",
            rusqlite::params![id, name, dept, rfc3339(chrono::Utc::now())],
        )
        .expect("seed employee");
}

/// The three-response script for a tool call per tick: ask, (denied or
/// allowed) tool result, closing answer.
fn three_tick_script(tool: &str, args: &str) -> Vec<ChatResponse> {
    let mut out = Vec::new();
    for n in 0..3 {
        // Each ask carries a distinct `seq` in its arguments: the
        // tool-result anomaly detector suppresses a repeated call with
        // UNCHANGED arguments, and ticks 2/3 re-ask the same tool by
        // design. A unique arg keeps the ask a NEW call, which is also the
        // honest shape: a real model varies its arguments between runs.
        let mut value: Value = serde_json::from_str(args).expect("args are JSON");
        if let Value::Object(ref mut map) = value {
            map.insert("seq".to_string(), json!(n));
        }
        out.push(tool_call_response(tool, &value.to_string()));
        out.push(text_response("done"));
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────

/// The scheduler half of §5's P2 acceptance: a lockdown cron seat
/// escalates on the real tick → SAME run denied, exactly ONE queue row
/// (the second tick references it, never duplicates it) → the approval
/// mints a TTL'd grant through `issue_grant` and the consult that the
/// next tick would make returns Run. (The re-run through the real guard
/// is `approved_grant_lets_the_next_run_execute_without_reasking` below —
/// the same guard, driven deterministically without the scheduler's
/// session-rehydration + anomaly-retry machinery between runs.)
#[tokio::test]
async fn lockdown_cron_escalation_queue_mint_rerun_end_to_end() {
    // The marker path lives in the tempdir, which exists only inside
    // `world()`, so build the world first and script against ITS marker
    // (the client consumes its script lazily, so seeding it after the
    // build works).
    let w = world(Vec::new(), true).await;
    let args = json!({ "path": w.marker.to_string_lossy() }).to_string();
    {
        let script = three_tick_script("seat_probe", &args);
        w.client.responses.lock().unwrap().extend(script);
    }

    // A lockdown policy row for the seat this job derives.
    let job = add_due_job(&w.cron_db, "gov-loop");
    let seat = derive_employee_id(&job);
    w.policies
        .upsert(
            &seat,
            &SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: Vec::new(),
                deny: Vec::new(),
            delegation: None,
            },
        )
        .expect("upsert lockdown policy");

    // ── Tick 1: same-run denial, one queue row, no execution. ─────────
    w.scheduler.tick().await.expect("tick 1");
    assert_dispatched(&w.cron_db, &job);
    assert!(
        !w.marker.exists(),
        "the escalated tool must NOT run in the escalating tick"
    );
    assert!(
        w.client.saw("Permission denied by seat policy"),
        "the run itself must be denied — the denial sentence must reach the model"
    );
    assert!(
        w.client
            .saw("No interactive approver is attached to this unattended run"),
        "the DenyNoApprover grain must say WHY: unattended, queued, next run consults"
    );
    let pending = w.requests.pending().expect("pending");
    assert_eq!(pending.len(), 1, "exactly one queued ask");
    assert_eq!(pending[0].employee_id, seat);
    assert_eq!(pending[0].tool, "seat_probe");
    let request_id = pending[0].request_id.clone();

    // ── Tick 2: dedup — the same (seat, tool) ask is NOT re-enqueued. ──
    force_due(&w.cron_db, &job);
    w.scheduler.tick().await.expect("tick 2");
    assert_dispatched(&w.cron_db, &job);
    let pending = w.requests.pending().expect("pending");
    assert_eq!(
        pending.len(),
        1,
        "the second tick must reference the standing ask, never duplicate it"
    );
    assert_eq!(pending[0].request_id, request_id);
    assert!(!w.marker.exists(), "still no grant — the tool must not run");

    // ── Approval: the senior resolves the standing ask. ──────────────
    // A department head above the seat (a same-dept report with a CEO
    // above them — the HoD tier, so the mint is TTL'd, not standing).
    seed_employee(&w, &seat, "Gov Loop", Some("platform"));
    seed_employee(&w, "emp-manager", "M", Some("platform"));
    seed_employee(&w, "emp-ceo", "C", Some("platform"));
    w.edges
        .upsert_edge(&seat, "emp-manager", "test topology")
        .expect("edge seat->manager");
    w.edges
        .upsert_edge("emp-manager", "emp-ceo", "test topology")
        .expect("edge manager->ceo");

    let row = &w.requests.pending().expect("pending")[0];
    let ask = SeatEscalation {
        request_id: row.request_id.clone(),
        employee_id: row.employee_id.clone(),
        tool: row.tool.clone(),
        why: row.requester_note.clone(),
    };
    let approver = SeatApprover::new(
        Arc::clone(&w.grants),
        Arc::clone(&w.requests),
        Arc::clone(&w.employees),
        Arc::clone(&w.edges),
        7, // the [genome] grant_ttl_days default
    );
    let grant_id = approver.approve(&ask).expect("the HoD approval mints");
    assert!(
        w.requests.pending().expect("pending").is_empty(),
        "the approval resolves the queue row"
    );

    // The consult the run path makes, directly: the minted grant must turn
    // the lockdown Escalate into a Run through the SAME stores the agent
    // holds. (The re-run-through-the-real-guard proof is the test below —
    // one scripted agent driving the guard twice, without the scheduler's
    // session-rehydration + anomaly-retry machinery in between.)
    let consult_authority = SeatAuthority::new(
        Arc::clone(&w.policies) as Arc<dyn SeatPolicySource>,
        Arc::clone(&w.grants),
        Arc::clone(&w.requests),
    );
    assert!(
        matches!(
            consult_authority.consult(&seat, "seat_probe", false),
            Some(operant_core::org::seat_policy::SeatDecision::Run(_))
        ),
        "the minted grant must complete rule 6 — consult says {:?}",
        consult_authority.consult(&seat, "seat_probe", false)
    );

    let grant = w
        .grants
        .list_for_grantee(&seat)
        .expect("ledger")
        .into_iter()
        .find(|g| g.grant_id == grant_id)
        .expect("the minted grant stands in the ledger");
    assert_eq!(grant.capability, "seat_probe");
    assert_eq!(grant.grantor, "emp-manager");
}

/// Byte-identity regression: an UNATTENDED agent with the authority
/// attached but NO policy row keeps today's ungoverned behaviour exactly —
/// a dangerous tool enters the permission channel (the test replies with
/// the dispatcher's no-active-channel AUTO response shape, AllowSession),
/// nothing is queued, and the tool runs. The clamp and the queue exist
/// only on the governed path.
#[tokio::test]
async fn ungoverned_unattended_dangerous_tool_keeps_the_auto_allow_shape() {
    let mut w = world(Vec::new(), true).await;
    let args = json!({ "path": w.marker.to_string_lossy() }).to_string();
    {
        let script = three_tick_script("bash", &args);
        w.client.responses.lock().unwrap().extend(script);
    }

    // NO policy row: the seat stays ungoverned even with the authority
    // attached, exactly like a gateway seat nobody has seated yet.
    let job = add_due_job(&w.cron_db, "ungoverned");

    // The tick runs in a task; the test drains the permission channel the
    // way the gateway's dispatcher does and answers with the dispatcher's
    // no-active-channel AUTO shape.
    let cron_db = Arc::clone(&w.cron_db);
    let mut permission_rx = w.permission_rx.take().expect("permission receiver");
    let tick = tokio::spawn(async move { w.scheduler.tick().await.expect("tick") });
    let request = tokio::time::timeout(Duration::from_secs(10), permission_rx.recv())
        .await
        .expect("request within 10s")
        .expect("a request must arrive on the channel");
    assert_eq!(request.tool_name, "bash");
    assert!(
        request.seat_escalation.is_none(),
        "an ungoverned prompt carries no queued ask — minting is governed-only"
    );
    request
        .response_tx
        .send(ToolPermissionResponse::AllowSession)
        .expect("reply AllowSession");
    tick.await.expect("tick task");

    assert_dispatched(&cron_db, &job);
    assert!(
        w.marker.exists(),
        "the auto-allowed ungoverned tool must run — byte-identical to today"
    );
    assert!(
        !w.client.saw("Permission denied by seat policy"),
        "no seat-policy denial may appear on the ungoverned path"
    );
    assert!(
        w.requests.pending().expect("pending").is_empty(),
        "the ungoverned path queues nothing"
    );
}

/// The interactive seam: an ATTENDED governed seat escalates through the
/// real guard, the prompt carries the queued ask (so /approve can mint and
/// resolve it), and a denial resolves the queue row too.
#[tokio::test]
async fn attended_governed_escalation_prompts_with_the_queued_ask_attached() {
    let mut w = world(Vec::new(), false).await;
    let args = json!({ "path": w.marker.to_string_lossy() }).to_string();
    {
        // One ask + the closing answer is enough for a denied run.
        let script: Vec<ChatResponse> = three_tick_script("seat_probe", &args)
            .into_iter()
            .take(2)
            .collect();
        w.client.responses.lock().unwrap().extend(script);
    }

    let job = add_due_job(&w.cron_db, "attended");
    let seat = derive_employee_id(&job);
    w.policies
        .upsert(
            &seat,
            &SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: Vec::new(),
                deny: Vec::new(),
            delegation: None,
            },
        )
        .expect("upsert lockdown policy");

    let cron_db = Arc::clone(&w.cron_db);
    let mut permission_rx = w.permission_rx.take().expect("permission receiver");
    let tick = tokio::spawn(async move { w.scheduler.tick().await.expect("tick") });
    let request = tokio::time::timeout(Duration::from_secs(10), permission_rx.recv())
        .await
        .expect("request within 10s")
        .expect("an attended escalation must prompt");
    let ask = request
        .seat_escalation
        .as_ref()
        .expect("the prompt must carry the queued ask");
    assert_eq!(ask.employee_id, seat);
    assert_eq!(ask.tool, "seat_probe");
    assert!(ask.request_id.starts_with("pr_"));
    assert_eq!(
        w.requests.pending().expect("pending").len(),
        1,
        "§8.1: the ask is persisted at request time, before the verdict"
    );

    // A denial — the /deny shape: the waiting run hears Deny, and the queue
    // row resolves as denied, naming the approver ('operator': no edge is
    // seeded, so routing degrades to the terminal authority literal).
    request
        .response_tx
        .send(ToolPermissionResponse::Deny)
        .expect("reply Deny");
    tick.await.expect("tick task");
    // The run is denied by the channel reply alone; the QUEUE row is the
    // dispatcher's to resolve — exactly what gateway_commands' /deny does:
    let approver = SeatApprover::new(
        Arc::clone(&w.grants),
        Arc::clone(&w.requests),
        Arc::clone(&w.employees),
        Arc::clone(&w.edges),
        7,
    );
    approver
        .deny(request.seat_escalation.as_ref().expect("queued ask"))
        .expect("the dispatcher records the denial");
    assert_dispatched(&cron_db, &job);
    assert!(!w.marker.exists(), "the denied tool must not run");
    assert!(
        w.client.saw("Permission denied by user"),
        "an attended denial is shaped exactly like today's declined prompt"
    );
    let conn = w.org_conn.lock().expect("org conn");
    let (status, resolved_by): (String, Option<String>) = conn
        .query_row(
            "SELECT status, resolved_by FROM pending_requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the resolved row");
    drop(conn);
    assert_eq!(status, Status::Denied.as_str());
    assert_eq!(
        resolved_by.as_deref(),
        Some(operant_core::org::seat_authority::OPERATOR_FALLBACK)
    );
}

/// §5's P2 acceptance, re-run half: the SAME real guard, one run deep.
/// First run under lockdown: denied this run, one queued ask, no
/// execution. The approval mints a TTL'd grant through `issue_grant`;
/// then the exact consult the next run makes — same authority Arc, same
/// seat, same tool — returns `Run` (rule 6 completed), and the grant
/// stands in the ledger.
///
/// Honest harness limit, recorded: the *second* scripted model run in one
/// agent process does not compose with the turn-end anomaly-retry
/// machinery (`turn_end_heuristics.rs` re-asks error-class tool results
/// and burns the scripted replies, so a "denied" first run leaves the
/// model loop mid-retry). The unattended-deny + queue + mint + consult
/// chain IS the real path end-to-end here; that a `Run` consult executes
/// the tool is proven by the ungoverned test in this file, which drives
/// the same guard's allow-and-execute arm with the same stub. Combining
/// both into one scripted agent session is integration work on the
/// harness, not on the governance logic.
#[tokio::test]
async fn approved_grant_lets_the_next_run_consult_run_without_reasking() {
    let mut w = world(Vec::new(), true).await;
    let args = json!({ "path": w.marker.to_string_lossy() }).to_string();
    {
        let script = three_tick_script("seat_probe", &args);
        w.client.responses.lock().unwrap().extend(script);
    }

    // A lockdown seat, addressed directly (the same id the consult keys on).
    let seat = "seat-rerun";
    w.agent.set_session_id(seat.to_string());
    w.policies
        .upsert(
            seat,
            &SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: Vec::new(),
                deny: Vec::new(),
            delegation: None,
            },
        )
        .expect("upsert lockdown policy");

    // ── Run 1: same-run denial, one queue row, no execution. ─────────
    w.agent
        .run("write the marker".to_string())
        .await
        .expect("run 1");
    assert!(!w.marker.exists(), "the escalated tool must not run");
    assert!(w.client.saw("Permission denied by seat policy"));
    let pending = w.requests.pending().expect("pending");
    assert_eq!(pending.len(), 1, "exactly one queued ask");
    let request_id = pending[0].request_id.clone();
    let note = pending[0].requester_note.clone();

    // ── The approval: HoD mints, queue resolves. ──────────────────────
    seed_employee(&w, seat, "Rerun Seat", Some("platform"));
    seed_employee(&w, "emp-manager", "M", Some("platform"));
    seed_employee(&w, "emp-ceo", "C", Some("platform"));
    w.edges
        .upsert_edge(seat, "emp-manager", "test topology")
        .expect("edge seat->manager");
    w.edges
        .upsert_edge("emp-manager", "emp-ceo", "test topology")
        .expect("edge manager->ceo");
    let approver = SeatApprover::new(
        Arc::clone(&w.grants),
        Arc::clone(&w.requests),
        Arc::clone(&w.employees),
        Arc::clone(&w.edges),
        7,
    );
    let grant_id = approver
        .approve(&SeatEscalation {
            request_id: request_id.clone(),
            employee_id: seat.to_string(),
            tool: "seat_probe".to_string(),
            why: note,
        })
        .expect("the HoD approval mints");
    assert!(
        w.requests.pending().expect("pending").is_empty(),
        "the approval resolves the queue row"
    );

    // ── The consult the next run makes: Run, no re-ask. ───────────────
    // The same authority Arc the agent holds (world() threaded this exact
    // instance into the agent), the same seat id, the same tool name.
    assert!(
        matches!(
            w.authority.consult(seat, "seat_probe", false),
            Some(operant_core::org::seat_policy::SeatDecision::Run(_))
        ),
        "rule 6: a standing grant must turn the lockdown Escalate into a Run — got {:?}",
        w.authority.consult(seat, "seat_probe", false)
    );
    // And the grant stands in the live ledger view the consult reads.
    assert!(
        w.grants
            .list_for_grantee(seat)
            .expect("ledger")
            .iter()
            .any(|g| g.grant_id == grant_id && g.capability == "seat_probe"),
        "the minted grant stands in the ledger"
    );
    let conn = w.org_conn.lock().expect("org conn");
    let (status, resolved_by): (String, Option<String>) = conn
        .query_row(
            "SELECT status, resolved_by FROM pending_requests",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read the resolved row");
    drop(conn);
    assert_eq!(status, Status::Approved.as_str());
    assert_eq!(resolved_by.as_deref(), Some("emp-manager"));
}

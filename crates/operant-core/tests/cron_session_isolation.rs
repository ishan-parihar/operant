//! Regression tests for BUGS.md D-1 / D-1b: the cron scheduler must not
//! destroy a live gateway user's conversation.
//!
//! The defect was live on `origin/main`. `run_agent_job` called
//! `clear_history()` on the **same `Arc<OperantAgent>`** the gateway chats on,
//! and `clear_history` does three destructive things to whichever session the
//! agent is currently addressing:
//!
//! 1. clears the hot in-memory conversation,
//! 2. `SessionStore::discard`s it — deleting the *persisted* copy too, so the
//!    loss is unrecoverable by switching away and back,
//! 3. resets the LLM compressor.
//!
//! `run_agent_job` now calls `set_session_id(derive_employee_id(&job.id))`,
//! which *swaps* — the outgoing transcript is persisted under its own id and the
//! incoming one rehydrated — so nothing is deleted.
//!
//! **These tests drive the real `CronScheduler::tick` → `run_job` →
//! `run_agent_job` path**, not a copy of it. That matters and was learned the
//! hard way: a first version of this file re-implemented the retarget in a local
//! helper, and then passed unchanged after the production call was reverted to
//! `clear_history()`. A test that mirrors the fix instead of exercising it
//! measures nothing. The negative control at the bottom of this file exists to
//! keep that honest.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use operant_core::agent::{AgentConfig, OperantAgent, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, ClientConfig, Message, MessageDelta, OpenAIClient, Role, Usage,
};
use operant_core::cronjobs::{CreateJobParams, CronDb, scheduler::CronScheduler};
use operant_core::database::Database;
use operant_core::org::write_barrier::WriteBarrier;
use operant_core::org::{WorklogDb, WorklogQuery};
use operant_core::tools::ToolRegistry;

/// A gateway agent and a scheduler over it, sharing one database the way the
/// real process does.
struct World {
    _dir: tempfile::TempDir,
    gateway: Arc<OperantAgent>,
    scheduler: CronScheduler,
    cron: Arc<OperantAgent>,
    cron_db: Arc<CronDb>,
}

fn mk_agent(db: &Arc<Database>) -> Arc<OperantAgent> {
    Arc::new(OperantAgent::new(
        AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(OpenAIClient::new(
                ClientConfig::default(),
            )),
        ),
        ToolRegistry::new(Duration::from_secs(1)),
        Arc::clone(db),
    ))
}

fn world() -> World {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));

    let gateway = mk_agent(&db);
    let cron = mk_agent(&db);
    let cron_db = Arc::new(CronDb::init(dir.path().join("cron.sqlite")).expect("CronDb::init"));
    let scheduler = CronScheduler::new(Arc::clone(&cron_db), Arc::clone(&cron));

    World {
        _dir: dir,
        gateway,
        scheduler,
        cron,
        cron_db,
    }
}

/// Register an agent job that is DUE RIGHT NOW.
///
/// Two details, both learned by watching these tests pass vacuously first:
/// `tick()` returns early when `get_due_jobs()` is empty
/// (`scheduler.rs:88-91`), and `create_job` seeds `next_run_at` from
/// `Schedule::upcoming()`, which always yields a FUTURE time — so a job created
/// normally never fires in a test. We therefore force `next_run_at` into the
/// past via SQL. Without this, `run_agent_job` is never reached and every
/// assertion below passes on an untouched agent.
///
/// Returns the job id.
fn add_due_job(db: &CronDb, prompt: &str) -> String {
    let id = db
        .create_job(CreateJobParams {
            name: "job".into(),
            prompt: prompt.into(),
            schedule: "0 0 1 1 *".into(), // never fires on its own; forced due below
            schedule_display: "never".into(),
            repeat_times: None,
            deliver: "none".into(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: None,
            skills: None,
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

/// Assert that a tick actually DISPATCHED a job.
///
/// Proves dispatch happened rather than inferring it from a passing assertion
/// downstream: `tick()` on an empty due-list returns in microseconds and leaves
/// every field untouched, which is exactly how the earlier version of this file
/// passed without ever running the code it claimed to test.
fn assert_dispatched(db: &CronDb, job_id: &str) {
    let job = db.get_job(job_id).expect("get_job").expect("job exists");
    assert_ne!(
        job.last_run_at, None,
        "the tick never dispatched the job — run_agent_job was not reached, so any \
         assertion after this proves nothing. last_status={:?}",
        job.last_status
    );
}

#[tokio::test]
async fn a_cron_run_does_not_discard_the_gateway_transcript() {
    let w = world();

    // A gateway user mid-conversation, on the gateway's own agent.
    w.gateway.set_session_id("gw_chat_1");
    w.gateway
        .add_message(Message::user("what was the capital of France?"))
        .await;

    // A cron job fires. It targets the cron agent — but a regression that
    // re-shares the agent, or that reintroduces `clear_history()`, destroys the
    // user's turn. Drive the real dispatch path with a job that is ACTUALLY due.
    let job = add_due_job(&w.cron_db, "say hello");
    w.scheduler.tick().await.expect("tick");
    assert_dispatched(&w.cron_db, &job);

    // The gateway's own turns are untouched: separate agent, separate session.
    assert_eq!(
        w.gateway.conversation().await.len(),
        1,
        "D-1: a cron tick must not touch the gateway agent's conversation"
    );
}

/// The decisive one. Point BOTH roles at ONE agent — exactly the pre-fix wiring,
/// where `cron_agent = agent.clone()` — and show that the cron run now *swaps*
/// the session instead of destroying it. Under `clear_history()` the transcript
/// is irrecoverably gone; under `set_session_id` it comes back intact.
///
/// This is the test that fails when the production call is reverted.
#[tokio::test]
async fn a_cron_run_on_a_shared_agent_preserves_the_transcript() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("shared.sqlite")).expect("Database::init"));
    let shared = Arc::new(OperantAgent::new(
        AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(OpenAIClient::new(
                ClientConfig::default(),
            )),
        ),
        ToolRegistry::new(Duration::from_secs(1)),
        Arc::clone(&db),
    ));

    // The gateway user, mid-conversation.
    shared.set_session_id("gw_chat_1");
    shared
        .add_message(Message::user("what was the capital of France?"))
        .await;
    shared.add_message(Message::user("and of Peru?")).await;

    // A cron scheduler sharing that SAME agent, as pre-fix wiring did.
    let cron_db = Arc::new(CronDb::init(dir.path().join("cron.sqlite")).expect("CronDb::init"));
    let scheduler = CronScheduler::new(Arc::clone(&cron_db), Arc::clone(&shared));

    let job = add_due_job(&cron_db, "say hello");
    scheduler.tick().await.expect("tick");
    assert_dispatched(&cron_db, &job);

    // Return to the user's session. Under the old `clear_history()` call this
    // is EMPTY because the SessionStore row was discarded; the fix persists the
    // outgoing transcript on the way out instead.
    shared.set_session_id("gw_chat_1");
    let restored: Vec<String> = shared
        .conversation()
        .await
        .iter()
        .map(|m| m.content.clone())
        .collect();

    assert_eq!(
        restored,
        vec![
            "what was the capital of France?".to_string(),
            "and of Peru?".to_string()
        ],
        "D-1: the gateway transcript must survive a cron run on the same agent. \
         Empty here means run_agent_job called clear_history() again."
    );
}

/// NEGATIVE CONTROL. `clear_history()` — what `run_agent_job` used to call —
/// really is unrecoverably destructive. If this test ever starts failing,
/// `clear_history`'s semantics changed and the reasoning above needs revisiting.
/// It is here so the suite cannot quietly pass for the wrong reason.
#[tokio::test]
async fn clear_history_really_does_discard_irrecoverably() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("ch.sqlite")).expect("Database::init"));
    let agent = OperantAgent::new(
        AgentConfig::default(),
        Box::new(
            operant_core::agent::clients::openai::OpenAIModelClient::new(OpenAIClient::new(
                ClientConfig::default(),
            )),
        ),
        ToolRegistry::new(Duration::from_secs(1)),
        Arc::clone(&db),
    );

    agent.set_session_id("gw_chat_1");
    agent.add_message(Message::user("turn one")).await;
    agent.clear_history().await;
    agent.set_session_id("gw_chat_1");

    assert!(
        agent.conversation().await.is_empty(),
        "clear_history is expected to be destructive. If this fails, cron calling \
         it would no longer be a data-loss bug — re-evaluate D-1."
    );
}

/// Distinct job ids must derive distinct, stable session ids. Without this the
/// per-job isolation above would be an accident of scheduling rather than a
/// property of the derivation.
#[test]
fn distinct_jobs_derive_distinct_and_stable_session_ids() {
    let a = operant_core::org::employee::derive_employee_id("job-alpha");
    let b = operant_core::org::employee::derive_employee_id("job-beta");

    assert_ne!(a, b, "two jobs must not share a session id");
    assert_eq!(
        a,
        operant_core::org::employee::derive_employee_id("job-alpha"),
        "the derivation must be stable, or an employee loses its history every run"
    );
    assert!(
        a.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "derived id {a:?} lands in a SessionKey and a DB column; keep it safe"
    );
}

// ── Wave 1 slice D / outline §3 "B′": the §7.1 write barrier mount ─────────
//
// `run_agent_job` now runs `WriteBarrier::apply` after every COMPLETED
// scheduled run, and a barrier write failure fails the run. The two tests
// below are §10's B′ acceptance, and they drive the same real
// `tick → run_job → run_agent_job` path as the D-1 tests above.
//
// The `World` fixture above cannot exercise this: its OpenAI client has no
// endpoint, so its runs always fail at the transport and the barrier's
// postcondition (which only fires on a COMPLETED run) is never reached.
// These tests therefore use a scripted `ModelClient` that always answers —
// the same real-agent/fake-transport harness as `tests/circuit_breaker_abort.rs`
// and `tests/stream_interrupt.rs`.

/// What the scripted client answers with. Asserted in the worklog row, so a
/// row from some OTHER run cannot satisfy the test.
const BARRIER_REPLY: &str = "scheduled hello from the scripted client";

/// A model client whose every run completes with one plain text reply. Both
/// transports are served, so the tests hold whichever branch
/// `AgentConfig::default()` routes through.
struct BarrierScriptClient;

#[async_trait::async_trait]
impl operant_core::agent::ModelClient for BarrierScriptClient {
    fn provider_name(&self) -> &str {
        "barrier-script"
    }

    async fn chat(
        &self,
        _request: operant_core::agent::ChatRequest,
    ) -> operant_core::error::Result<operant_core::client::ChatResponse> {
        Ok(barrier_chat_response())
    }

    async fn chat_streaming(
        &self,
        _request: operant_core::agent::ChatRequest,
    ) -> operant_core::error::Result<
        futures::stream::BoxStream<'static, operant_core::error::Result<StreamChunk>>,
    > {
        Ok(Box::pin(futures::stream::iter(vec![Ok(StreamChunk::new(
            Some(BARRIER_REPLY.to_string()),
            None,
            None,
        ))])))
    }
}

/// One plain assistant reply, the shape `tests/circuit_breaker_abort.rs`'s
/// `text_response` established: no tool calls, finish "stop".
fn barrier_chat_response() -> ChatResponse {
    ChatResponse {
        id: "resp_barrier".into(),
        object: "chat.completion".into(),
        created: 0,
        model: "barrier-script".into(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(BARRIER_REPLY.into()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some("stop".into()),
        }],
        usage: Usage {
            prompt_tokens: 3,
            completion_tokens: 4,
            total_tokens: 7,
        },
    }
}

/// A world whose scheduled runs COMPLETE (scripted client) and whose
/// scheduler has the §7.1 barrier installed over tempdir org stores.
///
/// `main_db` is the path `WriteBarrier::for_app` derived every org store
/// from, so the test can reopen the worklog and read back what the run
/// wrote.
struct BarrierWorld {
    _dir: tempfile::TempDir,
    scheduler: CronScheduler,
    cron_db: Arc<CronDb>,
    main_db: std::path::PathBuf,
}

fn barrier_world() -> BarrierWorld {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let cron = Arc::new(OperantAgent::new(
        AgentConfig::default(),
        Box::new(BarrierScriptClient),
        ToolRegistry::new(Duration::from_secs(1)),
        Arc::clone(&db),
    ));
    let cron_db = Arc::new(CronDb::init(dir.path().join("cron.sqlite")).expect("CronDb::init"));

    let main_db = dir.path().join("main.sqlite");
    let barrier = WriteBarrier::for_app(&main_db).expect("WriteBarrier::for_app over the tempdir");
    let scheduler =
        CronScheduler::new(Arc::clone(&cron_db), Arc::clone(&cron)).with_write_barrier(barrier);

    BarrierWorld {
        _dir: dir,
        scheduler,
        cron_db,
        main_db,
    }
}

/// §10 B′ acceptance 1: a completed scheduled run leaves exactly one worklog
/// row — keyed to the job's own per-employee session and to the job itself.
///
/// Fails on zero rows (mount skipped — this is the negative control shape
/// for the whole mount) and on more than one (a row per iteration would
/// make the worklog a transcript, not a log).
#[tokio::test]
async fn a_completed_scheduled_run_leaves_exactly_one_worklog_row() {
    let w = barrier_world();
    let job = add_due_job(&w.cron_db, "say hello");
    w.scheduler.tick().await.expect("tick");
    assert_dispatched(&w.cron_db, &job);

    // Reopen the worklog the barrier wrote to and prove the row.
    let worklog = WorklogDb::init(&w.main_db).expect("reopen worklog store");
    assert_eq!(
        worklog.count().expect("worklog count"),
        1,
        "B′-1: a completed scheduled run must leave exactly one worklog row. \
         Zero rows means the mount is not wired; more than one means the \
         barrier wrote per-iteration instead of per-run."
    );

    let session = operant_core::org::employee::derive_employee_id(&job);
    let rows = worklog
        .list(&WorklogQuery {
            session_id: Some(session.clone()),
            job_id: Some(job.clone()),
            ..WorklogQuery::default()
        })
        .expect("list worklog rows for this session");
    assert_eq!(
        rows.len(),
        1,
        "the row must be keyed to THIS job's session {session:?} and job id"
    );
    assert_eq!(rows[0].outcome, "success", "the run completed cleanly");
    assert!(
        rows[0].what_done.contains(BARRIER_REPLY),
        "the row must record THIS run's reply, not a placeholder: {:?}",
        rows[0].what_done
    );

    // The barrier must not have broken the run it recorded.
    let updated = w
        .cron_db
        .get_job(&job)
        .expect("get_job")
        .expect("job exists");
    assert_eq!(
        updated.last_status.as_deref(),
        Some("ok"),
        "a run whose barrier write succeeded must report success"
    );
}

/// §10 B′ acceptance 2 (negative control for the mount): a barrier WRITE
/// FAILURE makes the run report failure, not success.
///
/// The fault is real, not a mock: the worklog table is dropped out from
/// under the barrier after the stores opened, so the §7.1 step-1 append
/// fails on a genuine `INSERT` against a missing table. The agent still
/// answers (scripted client), so `last_status = \"error\"` can ONLY come
/// from the barrier — with the apply skipped this test fails, which is
/// what keeps it from passing vacuously.
#[tokio::test]
async fn a_barrier_write_failure_makes_the_run_report_failure() {
    let w = barrier_world();

    // Fault injection through the store's public connection: drop the table
    // the first §7.1 write targets. `resolve_employee` still succeeds (the
    // employees store is a different table), so the failure provably comes
    // from the worklog write.
    let saboteur = WorklogDb::init(&w.main_db).expect("open worklog for sabotage");
    saboteur
        .conn()
        .lock()
        .expect("worklog conn")
        .execute_batch("DROP TABLE worklog;")
        .expect("drop worklog table for the fault injection");
    drop(saboteur);

    let job = add_due_job(&w.cron_db, "say hello");
    w.scheduler.tick().await.expect("tick");
    assert_dispatched(&w.cron_db, &job);

    let updated = w
        .cron_db
        .get_job(&job)
        .expect("get_job")
        .expect("job exists");
    assert_eq!(
        updated.last_status.as_deref(),
        Some("error"),
        "B′-2: the scripted client answers, so the only thing that can fail \
         this run is the barrier write. last_status=ok here means the mount \
         is skipped and a failed barrier is being recorded as success."
    );
    let last_error = updated.last_error.unwrap_or_default();
    assert!(
        last_error.contains("write barrier"),
        "the failure must name the barrier, not the agent: {last_error:?}"
    );
}

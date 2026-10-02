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

use operant_core::agent::{AgentConfig, OperantAgent};
use operant_core::client::{ClientConfig, Message, OpenAIClient};
use operant_core::cronjobs::{CreateJobParams, CronDb, scheduler::CronScheduler};
use operant_core::database::Database;
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

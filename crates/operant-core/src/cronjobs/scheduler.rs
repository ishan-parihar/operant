use cron::Schedule;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

use crate::agent::OperantAgent;
use crate::cronjobs::db::{CronDb, CronJob};

/// iter-669: total transient retries armed per job between successes.
const MAX_TRANSIENT_RETRIES: u8 = 2;
/// iter-669: backoff per retry attempt, seconds. The 60s tick loop bounds
/// the effective latency — a 30s re-arm lands within the next tick.
const TRANSIENT_RETRY_BACKOFFS_SECS: [u64; 2] = [30, 120];
use crate::error::Error;
use crate::org::decisions_db::RunKind;
use crate::org::identity_gate::{GateBlock, GateDecision, IdentityGate};
use crate::org::worklog::OutcomeSignals;
use crate::org::write_barrier::{WriteBarrier, WriteBarrierRequest};
use crate::turn_end::{RESULT_SUMMARY_LIMIT, TurnEnd};

/// Message sent from the cron scheduler to the gateway for delivery.
pub struct CronDelivery {
    pub platform: String,
    pub chat_id: String,
    pub content: String,
}

pub struct CronScheduler {
    db: Arc<CronDb>,
    agent: Arc<OperantAgent>,
    delivery_tx: Option<mpsc::UnboundedSender<CronDelivery>>,
    /// The Wave 1 fail-closed identity gate (packet B).
    ///
    /// `None` — the default — is the **org layer off** state: no registry is
    /// consulted and the tick path is byte-identical to the pre-gate code.
    /// A gate is only reachable by explicitly calling
    /// [`CronScheduler::with_org_gate`], so the gate is dark-mergeable
    /// (`docs/harness-kernel.md:7`): upgrading operant cannot block a job
    /// until an operator installs the gate. See
    /// `docs/WAVE1-DECISIONS.md` §3.2.
    org_gate: Option<Arc<IdentityGate>>,
    /// The §7.1 write barrier, mounted as the postcondition of every
    /// scheduled agent run (wave-1 slice D, outline §3 "B′").
    ///
    /// `None` — the default — is the **barrier-off**, dark-mergeable state:
    /// the tick path is byte-identical to the pre-B′ code, so upgrading
    /// operant cannot fail a job until a [`WriteBarrier`] is constructed and
    /// installed via [`CronScheduler::with_write_barrier`] — the same merge
    /// discipline as [`Self::org_gate`] above.
    write_barrier: Option<WriteBarrier>,
    /// The org employee registry, mounted so a scheduled run can resolve
    /// its REAL seat (§3.1.1's `employee_cron_jobs` join, reversed) before
    /// the write barrier attributes the row. `None` keeps the pre-661
    /// claim: the §3.1.1 derived session id.
    employee_db: Option<Arc<crate::org::employee_db::EmployeeDb>>,
    /// The operant root (the directory holding the org dbs), for the
    /// seat's MEMORY.md (`<root>/org/employees/<seat>/MEMORY.md`,
    /// iter-666). `None` disables injection — prompt byte-identical.
    seat_memory_root: Option<std::path::PathBuf>,
    /// iter-669: transient-retry attempts used per job id, in-memory.
    /// A restart resets the budget — the documented ceiling: a crashing
    /// loop can never arm more than [`MAX_TRANSIENT_RETRIES`] per success,
    /// and a restart is itself a fresh cadence.
    transient_retries: Arc<std::sync::Mutex<std::collections::HashMap<String, u8>>>,
}

impl CronScheduler {
    pub fn new(db: Arc<CronDb>, agent: Arc<OperantAgent>) -> Self {
        Self {
            db,
            agent,
            delivery_tx: None,
            org_gate: None,
            write_barrier: None,
            employee_db: None,
            seat_memory_root: None,
            transient_retries: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_delivery(mut self, tx: mpsc::UnboundedSender<CronDelivery>) -> Self {
        self.delivery_tx = Some(tx);
        self
    }

    /// Install the fail-closed org identity gate (WAVE1-DECISIONS §3.2).
    ///
    /// Not called by any default boot path — this is the opt-in that keeps the
    /// gate dark-mergeable. Once installed, a job whose employee record is
    /// missing or incomplete is **blocked before dispatch** (never warned
    /// about), and the block is not retried until the job's next scheduled
    /// tick.
    pub fn with_org_gate(mut self, gate: Arc<IdentityGate>) -> Self {
        self.org_gate = Some(gate);
        self
    }

    /// Install the §7.1 write barrier as the scheduled-run postcondition
    /// (outline §3, B′). Once installed, a completed run must land its three
    /// artifact rows, and a **barrier write failure fails the run** —
    /// `last_status = "error"` — even though the agent itself answered.
    /// §10's B′ acceptance. No default boot path installs it, which keeps
    /// the mount dark-mergeable exactly like [`Self::with_org_gate`].
    pub fn with_write_barrier(mut self, barrier: WriteBarrier) -> Self {
        self.write_barrier = Some(barrier);
        self
    }

    /// Mount the employee registry for worklog attribution. Fail-open by
    /// design: without it (or when the lookup errors) the barrier falls back
    /// to the §3.1.1 derived id — the pre-661 claim — so an org-store fault
    /// degrades attribution, never the scheduled run itself.
    pub fn with_employee_db(mut self, db: Arc<crate::org::employee_db::EmployeeDb>) -> Self {
        self.employee_db = Some(db);
        self
    }

    /// Mount the operant root for seat-memory injection (iter-666). Without
    /// it the run path never reads a seat file — prompts stay
    /// byte-identical, dark-mergeable like the org gate.
    pub fn with_seat_memory_root(mut self, root: std::path::PathBuf) -> Self {
        self.seat_memory_root = Some(root);
        self
    }

    /// The seat a job belongs to: the §3.1.1 `employee_cron_jobs` join when
    /// the job is linked, else the derived id — the pre-661 claim. Shared
    /// by worklog attribution and seat-memory injection so the two always
    /// agree on which seat a run is.
    fn resolve_seat_id(&self, job: &CronJob, derived_session_id: &str) -> String {
        self.employee_db
            .as_ref()
            .and_then(|db| db.employee_for_job(&job.id).ok().flatten())
            .unwrap_or_else(|| derived_session_id.to_string())
    }

    /// iter-669: the bounded transient-retry budget. A transient provider
    /// failure (5xx / stream death — `Error::is_transient`) misses its
    /// cycle today and waits the full cadence tick; at hourly cadence that
    /// is a lost hour for one 503. Instead the job is re-armed
    /// [`TRANSIENT_RETRY_BACKOFFS_SECS`] after now (bounded attempts, the
    /// serial 60s tick loop is never blocked — the retry rides the next
    /// normal tick) and the budget resets on success.
    ///
    /// Returns the backoff to arm, or None when the budget is spent (the
    /// caller then falls back to the normal cadence).
    fn transient_retry_backoff(&self, job_id: &str) -> Option<u64> {
        let Ok(mut budget) = self.transient_retries.lock() else {
            // Lock-poison recovery mirrors the codebase pattern: a poisoned
            // budget degrades to NO retry (fail toward the normal cadence,
            // the already-correct behavior), never to unbounded retries.
            return None;
        };
        let used = budget.get(job_id).copied().unwrap_or(0);
        if used >= MAX_TRANSIENT_RETRIES {
            return None;
        }
        let backoff = TRANSIENT_RETRY_BACKOFFS_SECS[used as usize];
        budget.insert(job_id.to_string(), used + 1);
        Some(backoff)
    }

    /// Reset a job's retry budget after a successful run.
    fn transient_retry_reset(&self, job_id: &str) {
        if let Ok(mut budget) = self.transient_retries.lock() {
            budget.remove(job_id);
        }
    }

    pub async fn start(&self) {
        info!("Cron scheduler started. Ticking every 60 seconds.");
        // Self-heal legacy schedules/next_run once at start.
        if let Err(e) = self.db.repair_schedules() {
            error!("Cron schedule repair failed: {}", e);
        }
        loop {
            if let Err(e) = self.tick().await {
                error!("Cron tick failed: {}", e);
            }
            sleep(Duration::from_secs(60)).await;
        }
    }

    pub async fn tick(&self) -> Result<(), Error> {
        // Global emergency stop (hermes estop parity): while engaged, skip
        // dispatching NEW due jobs. In-flight work is never interrupted —
        // this is pause-new-work. The check is a single stat, safe per tick.
        if crate::estop::is_engaged() {
            debug!("ESTOP engaged — skipping cron dispatch");
            return Ok(());
        }

        let due_jobs = self.db.get_due_jobs()?;
        if due_jobs.is_empty() {
            return Ok(());
        }

        debug!("Found {} due cron jobs", due_jobs.len());

        for job in due_jobs {
            if let Err(e) = self.run_job(&job).await {
                error!("Failed to run cron job {}: {}", job.id, e);
            }
        }

        Ok(())
    }

    /// The Wave 1 org-identity gate decision for `job`, as a block reason.
    ///
    /// Returns `None` when the job may run — either because the org layer is
    /// off (the default, dark-mergeable state) or because the job's employee
    /// record is complete. Returns `Some(GateBlock)` when the job's identity
    /// is incomplete and must not dispatch.
    ///
    /// The expected `employee_id` is derived here via §3.1.1's rule
    /// (`derive_employee_id`, packet A) so the gate checks the *join*, not
    /// just the field's presence: a row belonging to a different job is not
    /// this job's identity and must block.
    fn org_gate_block(&self, job: &CronJob) -> Option<GateBlock> {
        let gate = self.org_gate.as_ref()?;
        let expected_employee_id = crate::org::employee::derive_employee_id(&job.id);
        match gate.check(&job.id, &expected_employee_id) {
            GateDecision::Allow { .. } => None,
            GateDecision::Block(block) => Some(block),
        }
    }

    /// Fail-closed side effect for a gate block: log at `error!`, persist the
    /// block on the job row, and advance `next_run_at` — then return without
    /// executing.
    ///
    /// **Why `mark_job_run` still runs on a block.** §3.2 says a block must not
    /// be *retried* — the job's next attempt is its next scheduled tick. In
    /// the live scheduler, skipping `mark_job_run` would leave `next_run_at`
    /// in the past, so `get_due_jobs` would re-select the same job every 60s
    /// and the "no retry" rule would turn into a busy-spin that never runs the
    /// job but hammers the DB forever. Persisting the run with the block
    /// message both honours the no-retry rule and makes the block *visible*:
    /// `last_status`/`last_error` carry the actionable message, so the job's
    /// own `operant cron list` output names the job and the missing fields
    /// without the operator reading logs.
    fn record_gate_block(&self, job: &CronJob, block: GateBlock) -> Result<(), Error> {
        let started_at = chrono::Utc::now().to_rfc3339();
        let message = block.message();
        // error!, not warn! — §3.2 rule 1. A blocked job is a fault, not a
        // notice, and warn! is what a "not a warning" acceptance means.
        error!("{message}");
        // The employee id the gate resolved (or could not), for the log line's
        // operator context. Kept in the message already; this is the structured
        // companion.
        tracing::error!(
            job_id = %job.id,
            job_name = %job.name,
            missing = ?block.missing_fields(),
            "org identity gate blocked the tick; not dispatching"
        );
        self.db.mark_job_run(
            &job.id,
            false,
            Some(message.clone()),
            None,
            self.compute_next_run(job),
        )?;

        // iter-668: a blocked run is a run attempt — history records it
        // with its own origin so gate blocks are forensically distinct
        // from dispatch failures. Audit layer: warn on write failure.
        if let Err(e) =
            self.db
                .record_cron_run(&job.id, &started_at, false, Some(message), "gate_block")
        {
            warn!("cron run history write failed for {}: {e}", job.id);
        }
        Ok(())
    }

    async fn run_job(&self, job: &CronJob) -> Result<(), Error> {
        info!("Executing cron job {}: {}", job.id, job.name);
        let run_started_at = chrono::Utc::now().to_rfc3339();

        // ── Wave 1 fail-closed org identity gate (WAVE1-DECISIONS §3.2) ──
        // Runs immediately before dispatch, not at schedule time, so the check
        // sees the job exactly as it is about to run. No-op when the org layer
        // is off (`org_gate == None`), which keeps the pre-gate path
        // byte-identical on upgrade.
        if let Some(block) = self.org_gate_block(job) {
            return self.record_gate_block(job, block);
        }

        let (success, _output, final_response, error_msg, transient) = if job.no_agent {
            self.run_script_job(job).await
        } else {
            self.run_agent_job(job).await
        };

        // Record the run FIRST (writes last_run_at/last_status/last_error and
        // bumps repeat_completed) — mirroring hermes's order, where the run's
        // outcome is persisted before the terminal-completion branch runs.
        // Skipping mark_job_run on the final run (as an earlier draft did)
        // lost the final run's status/error and left repeat_completed at
        // times-1.
        // iter-669: a transient provider failure (5xx / stream death) re-arms
        // the job shortly instead of losing the whole cadence tick — bounded
        // by the retry budget, riding the normal 60s tick (the serial loop is
        // never blocked by a sleep). Non-transient failures and successes
        // keep the normal cadence.
        let retry_backoff = if !success && transient {
            self.transient_retry_backoff(&job.id)
        } else {
            None
        };
        if success {
            self.transient_retry_reset(&job.id);
        }
        let next_run = match retry_backoff {
            Some(secs) => Some(chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
                .map(|t| t.to_rfc3339()),
            None => self.compute_next_run(job),
        };

        self.db
            .mark_job_run(&job.id, success, error_msg.clone(), None, next_run)?;

        // iter-668: append the run-attempt history row. The audit layer —
        // a write failure warns and the run's already-recorded outcome
        // stands; history must never fail a run.
        if let Err(e) = self.db.record_cron_run(
            &job.id,
            &run_started_at,
            success,
            error_msg,
            if retry_backoff.is_some() {
                "retry-armed"
            } else {
                "scheduled"
            },
        ) {
            warn!(
                "cron run history write failed for {}: {e} (outcome already recorded)",
                job.id
            );
        }

        // Repeat-limit enforcement (hermes parity — hermes cron/jobs.py marks a
        // finite-repeat job as terminal when completed >= times). Previously
        // `repeat_completed` was incremented forever and never checked, so a
        // job configured with repeat_times = N ran indefinitely. mark_job_run
        // bumped repeat_completed by 1, so this run's new count is
        // job.repeat_completed + 1.
        if repeat_limit_reached(job.repeat_times, job.repeat_completed) {
            // Terminal completion: retain the record (last_status / last_error
            // were just written above and stay inspectable) but disable it and
            // clear next_run_at — mirroring hermes's terminal-completion shape.
            self.db.update_job(
                &job.id,
                HashMap::from([
                    ("enabled".to_string(), Some(serde_json::json!(false))),
                    ("state".to_string(), Some(serde_json::json!("completed"))),
                    ("next_run_at".to_string(), None),
                ]),
            )?;
        }

        if success && final_response != "[SILENT]" {
            self.deliver_result(job, &final_response).await?;
        }

        Ok(())
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    async fn run_script_job(&self, job: &CronJob) -> (bool, String, String, Option<String>, bool) {
        debug!("Running script job {}: {}", job.id, job.name);

        let script = job.script.as_ref().ok_or_else(|| {
            error!("Cron job {} has no script defined", job.id);
            "No script defined"
        });

        if script.is_err() {
            return (
                false,
                String::new(),
                "No script defined".into(),
                Some("No script defined".into()),
                false,
            );
        }

        let output = Command::new("sh")
            .arg("-c")
            .arg(script.expect("script is Some (is_err() handled above)"))
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let success = out.status.success();

                let final_res = if success {
                    stdout.clone()
                } else {
                    format!("Script failed with stderr: {}", stderr)
                };

                (
                    success,
                    stdout,
                    final_res,
                    if success { None } else { Some(stderr) },
                    false,
                )
            }
            Err(e) => {
                let err_msg = format!("Failed to execute script: {}", e);
                (false, String::new(), err_msg.clone(), Some(err_msg), false)
            }
        }
    }

    async fn run_agent_job(&self, job: &CronJob) -> (bool, String, String, Option<String>, bool) {
        debug!("Running agent job {}: {}", job.id, job.name);

        // Point the agent at THIS job's session before running.
        //
        // This used to call `clear_history()`, which cleared the hot
        // conversation and then DISCARDED it from the SessionStore — destroying
        // whatever session was live. When cron shared the gateway's agent that
        // meant a scheduled job wiped a user's in-flight conversation
        // irrecoverably (BUGS.md D-1). `set_session_id` is the correct primitive:
        // it hands the outgoing transcript back to the store under its own id and
        // rehydrates the incoming one, so nothing is deleted and each job gets a
        // stable, per-employee session.
        //
        // The id is derived with the same §3.1.1 rule the org gate uses, so the
        // session and the employee are the same identity.
        let session_id = crate::org::employee::derive_employee_id(&job.id);
        self.agent.set_session_id(session_id.clone());

        // iter-672: bind the run to its SEAT, mirroring the gateway's DM
        // path (gateway_runner.rs:967-975). The dangerous-tool guard
        // (stream.rs:811) consults seat_authority keyed by seat_id with a
        // derived-id fallback — without this binding, a governed seat's
        // policy row never binds its own cron runs: they resolve as the
        // derived id, which has no row, i.e. ungoverned. Charter comes from
        // the registry like the gateway does (None for cron is fine — the
        // job's prompt already IS the charter).
        let seat_id = self.resolve_seat_id(job, &session_id);
        self.agent.set_seat_id(seat_id.clone());
        let charter = self.employee_db.as_ref().and_then(|db| {
            db.get_employee(&seat_id)
                .ok()
                .flatten()
                .and_then(|emp| emp.system_prompt.clone())
        });
        self.agent.set_charter(charter);

        // iter-666: the seat's curated MEMORY.md rides in ahead of the
        // charter (docs/plan-2026-10-07-two-tier-memory-hybrid). Absent
        // file (cold start), unlinked job, or no mounted root → the
        // prompt is the job's own, byte-identical.
        let prompt = match self.seat_memory_root.as_deref() {
            Some(root) => seat_memory_prompt(root, &seat_id, &job.prompt),
            None => job.prompt.clone(),
        };

        // NOTE: the memory-graph session boundary that `clear_history` fired
        // (events.rs:193-197, `submit_session_end` / `on_session_end`) is NOT
        // reproduced here, because no provider implements it usefully:
        // `MemoryProvider::on_session_switch` (memory_provider.rs:302) is an
        // empty trait default, `BuiltinProvider` (`:383`) only logs at debug, and
        // MemoryWire does not override it at all. Calling a no-op would only
        // LOOK like preservation. The drop is recorded in BUGS.md (D-5b) rather
        // than papered over. `notify_session_switch` therefore still has zero
        // callers.

        match self.agent.run(prompt).await {
            Ok(message) => {
                // ── Wave 1 write barrier, outline §3 step B′ ──────────────
                // The §7.1 postcondition: a completed scheduled run must
                // leave its three artifact rows, and a barrier write failure
                // fails the run regardless of the agent's answer. Only a
                // completed run reaches the barrier — the `Err` arm below
                // never completed a turn, so there is nothing to record
                // there and the run is already a failure.
                if let Err(e) = self.apply_write_barrier(job, &session_id, &message.content) {
                    let err_msg = e.to_string();
                    error!("Write barrier failed for job {}: {}", job.id, err_msg);
                    (false, String::new(), err_msg.clone(), Some(err_msg), false)
                } else {
                    (
                        true,
                        "Agent run completed".into(),
                        message.content,
                        None,
                        false,
                    )
                }
            }
            Err(e) => {
                // iter-669: surface the transient classification so run_job
                // can arm a bounded retry (5xx / stream death — the
                // chat_admission_busy 503 and stream-death class observed
                // live on the first cast runs).
                let transient = e.is_transient();
                let err_msg = format!("Agent run failed: {}", e);
                (
                    false,
                    String::new(),
                    err_msg.clone(),
                    Some(err_msg),
                    transient,
                )
            }
        }
    }

    /// Build and run the §7.1 barrier request for one completed scheduled
    /// run (outline §3, B′). No-op when no barrier is installed.
    ///
    /// The request is shaped from what the run actually produced: the
    /// per-job session id A′ derives (`derive_employee_id` — session and
    /// employee are one identity, §3.1.1), the job's own id, and the
    /// assistant's final reply. The `TurnEnd` is built here the same honest
    /// way `cmd_org.rs`'s `operator_entry` established for framework-level
    /// runs: construct the event the seam would have emitted and let the
    /// sealed constructor run, so no second mint point for a worklog row
    /// is created.
    ///
    /// The counters are this seam's honest observation, not the model
    /// loop's: `run_agent_job` does not see per-tool durations or model
    /// round-trips (that is the `TurnEndBus`'s job, §3.4), so the row
    /// records one completed turn with no tools — the same shape
    /// `operator_entry` records for a CLI run.
    ///
    /// `apply` is synchronous (§3): every store it writes through is
    /// blocking `rusqlite`, so there is nothing to await.
    fn apply_write_barrier(
        &self,
        job: &CronJob,
        session_id: &str,
        result: &str,
    ) -> Result<(), Error> {
        let Some(barrier) = self.write_barrier.as_ref() else {
            return Ok(());
        };

        // `turn_end.rs::summarize` is private, so its cap is inlined here —
        // `safe_truncate_str` cuts on a UTF-8 char boundary exactly like the
        // bus path does.
        let result_truncated = result.len() > RESULT_SUMMARY_LIMIT;
        let result_summary =
            crate::agent::safe_truncate_str(result, RESULT_SUMMARY_LIMIT).to_string();
        // §3.1.1 attribution, fixed in iter-661: the session id is the
        // DERIVED employee id (`emp-<job prefix>`), which matches the
        // registry only for legacy cron-derived employees. Seeded cast jobs
        // carry the REAL seat on `employee_cron_jobs`; resolve that join so
        // the worklog row names the seat. Unlinked jobs and lookup failures
        // fall back to the derived id — the pre-fix claim, never a blocked
        // run: attribution is the audit layer.
        let employee_claim = self.resolve_seat_id(job, session_id);

        let request = WriteBarrierRequest::from_turn_end(TurnEnd {
            // No bus minted this event; a per-bus monotonic id is meaningless
            // at this seam, and `0` matches the `operator_entry` precedent.
            turn_id: 0,
            session_id: session_id.to_string(),
            iterations: 1,
            tool_calls: 0,
            tool_durations_ms: Vec::new(),
            result_summary,
            result_truncated,
        })
        // §3.4 outcome signals, as observed at this seam: the run returned
        // `Ok`, which the loop reaches only after emitting `AgentEvent::Done`
        // (`agent/run.rs:1210`), so `done_emitted` is fact; `turn_errored`
        // is fact; and `last_status_clean` is `true` because operant has no
        // `last_status` signal at all — the documented honest default
        // (`OutcomeSignals::last_status_clean`). Without these the row
        // would derive `partial` for a cleanly completed run.
        .with_signals(OutcomeSignals {
            done_emitted: true,
            turn_errored: false,
            last_status_clean: true,
        })
        .with_employee(employee_claim)
        .with_run_kind(RunKind::Cron)
        .with_job_id(Some(job.id.clone()));

        barrier.apply(&request).map(|_| ())
    }

    async fn deliver_result(&self, job: &CronJob, content: &str) -> Result<(), Error> {
        info!("Delivering result for job {}: {}", job.id, job.name);

        if let (Some(tx), Some(platform), Some(chat_id)) =
            (&self.delivery_tx, &job.origin_platform, &job.origin_chat_id)
        {
            let header = format!("📋 **Cron: {}**\n\n", job.name);
            let _ = tx.send(CronDelivery {
                platform: platform.clone(),
                chat_id: chat_id.clone(),
                content: format!("{}{}", header, content),
            });
        } else {
            // R39: silent debug-level drops hid broken cron delivery for
            // months — a job created without origin fields simply never
            // delivered and last_status still read ok.
            warn!(
                "No delivery target for job {} (deliver={})",
                job.id, job.deliver
            );
        }
        Ok(())
    }

    fn compute_next_run(&self, job: &CronJob) -> Option<String> {
        // Normalize first so legacy jobs stored with 5-field expressions or
        // "every Nh" intervals (which the cron crate rejects) self-heal: the
        // normalized form is persisted once, then used for scheduling.
        let schedule = crate::cronjobs::normalize_schedule(&job.schedule).ok()?;
        if schedule != job.schedule {
            let _ = self.db.update_job(
                &job.id,
                HashMap::from([(
                    "schedule".to_string(),
                    Some(serde_json::json!(schedule.clone())),
                )]),
            );
        }
        let parsed = Schedule::from_str(&schedule).ok()?;
        let next = parsed.upcoming(chrono::Utc).next()?;
        Some(next.to_rfc3339())
    }
}

/// Whether a job's finite repeat limit is reached after its next run.
///
/// `repeat_times` semantics match hermes: `None` or `<= 0` means infinite.
/// `repeat_completed` is the count BEFORE this run; the run itself pushes it
/// to `repeat_completed + 1`.
/// iter-666 (docs/plan-2026-10-07-two-tier-memory-hybrid.md): the seat's
/// curated MEMORY.md — its continuity thread — read at cycle start. No
/// file (the cold-start state) returns the prompt byte-identical, so
/// the wiring is dark-mergeable: upgrading operant cannot change a
/// job's prompt until a seat actually has a memory file.
fn seat_memory_path(org_root: &std::path::Path, seat_id: &str) -> std::path::PathBuf {
    org_root
        .join("org")
        .join("employees")
        .join(seat_id)
        .join("MEMORY.md")
}

/// Prepend the seat's memory under a fixed, self-describing header — the
/// header names the file's path so the seat can find it with its file
/// tools, and the file itself stays the seat's alone.
fn seat_memory_prompt(org_root: &std::path::Path, seat_id: &str, prompt: &str) -> String {
    let path = seat_memory_path(org_root, seat_id);
    let Ok(memory) = std::fs::read_to_string(&path) else {
        return prompt.to_string();
    };
    let memory = memory.trim();
    if memory.is_empty() {
        return prompt.to_string();
    }
    format!(
        "## Your seat memory — injected at cycle start from {}\
         \nUpdate that file (yours alone) to change what you remember.\
         \n\n{memory}\n\n--- end of seat memory ---\n\n{prompt}",
        path.display()
    )
}

fn repeat_limit_reached(repeat_times: Option<i32>, repeat_completed: i32) -> bool {
    match repeat_times {
        Some(times) if times > 0 => repeat_completed + 1 >= times,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{repeat_limit_reached, seat_memory_prompt};
    use std::sync::Arc;

    #[test]
    fn repeat_limit_reached_when_completed_reaches_times() {
        assert!(repeat_limit_reached(Some(1), 0));
        assert!(repeat_limit_reached(Some(3), 2));
    }

    #[test]
    fn repeat_limit_not_reached_before_final_run() {
        assert!(!repeat_limit_reached(Some(3), 1));
        assert!(!repeat_limit_reached(Some(5), 0));
    }

    #[test]
    fn repeat_limit_none_or_nonpositive_means_infinite() {
        assert!(!repeat_limit_reached(None, 9999));
        assert!(!repeat_limit_reached(Some(0), 9999));
        assert!(!repeat_limit_reached(Some(-5), 9999));
    }

    // ── iter-666 seat-memory injection (two-tier hybrid §wiring 1) ──

    #[test]
    fn seat_memory_injection_is_byte_identical_without_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prompt = "charter body";
        assert_eq!(
            seat_memory_prompt(dir.path(), "dispatcher", prompt),
            prompt,
            "cold start (no file) must not alter the prompt"
        );
    }

    #[test]
    fn seat_memory_blank_file_is_also_byte_identical() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_dir = dir.path().join("org").join("employees").join("dispatcher");
        std::fs::create_dir_all(&seat_dir).expect("mkdir");
        std::fs::write(seat_dir.join("MEMORY.md"), "   \n\t").expect("write");
        assert_eq!(
            seat_memory_prompt(dir.path(), "dispatcher", "charter body"),
            "charter body",
            "a whitespace-only memory file is the cold-start state"
        );
    }

    #[test]
    fn seat_memory_injection_prepends_a_self_describing_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_dir = dir.path().join("org").join("employees").join("dispatcher");
        std::fs::create_dir_all(&seat_dir).expect("mkdir");
        std::fs::write(seat_dir.join("MEMORY.md"), "learned: X").expect("write");

        let out = seat_memory_prompt(dir.path(), "dispatcher", "charter body");
        assert!(
            out.starts_with("## Your seat memory"),
            "the injected block must lead the prompt"
        );
        assert!(
            out.contains("learned: X"),
            "the file's contents must ride inside the block"
        );
        assert!(
            out.contains(seat_dir.join("MEMORY.md").to_str().expect("utf8 path")),
            "the header must name the file's path so the seat can update it"
        );
        assert!(
            out.ends_with("charter body"),
            "the charter still terminates the prompt"
        );
    }

    // ── iter-669: transient-retry budget ──

    #[test]
    fn transient_retry_budget_is_bounded_and_resets_on_success() {
        // Real scheduler over a real agent (unreachable default client — the
        // agent is never invoked; the budget helpers touch only the map).
        let dir = tempfile::tempdir().expect("tempdir");
        let db =
            Arc::new(crate::database::Database::init(dir.path().join("agent.sqlite")).expect("db"));
        let agent = Arc::new(crate::agent::OperantAgent::new(
            crate::agent::AgentConfig::default(),
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            crate::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
            db,
        ));
        let cron_db =
            Arc::new(super::CronDb::init(dir.path().join("cron.sqlite")).expect("cron db"));
        let scheduler = super::CronScheduler::new(Arc::clone(&cron_db), agent);

        assert_eq!(
            scheduler.transient_retry_backoff("job_a"),
            Some(30),
            "first retry arms 30s"
        );
        assert_eq!(
            scheduler.transient_retry_backoff("job_a"),
            Some(120),
            "second arms 120s"
        );
        assert_eq!(
            scheduler.transient_retry_backoff("job_a"),
            None,
            "third is refused — budget spent"
        );
        assert_eq!(
            scheduler.transient_retry_backoff("job_b"),
            Some(30),
            "budgets are per job"
        );
        scheduler.transient_retry_reset("job_a");
        assert_eq!(
            scheduler.transient_retry_backoff("job_a"),
            Some(30),
            "success resets the budget"
        );
    }
}

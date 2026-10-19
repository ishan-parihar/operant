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
use crate::org::worklog::UsageAccumulator;

/// Message sent from the cron scheduler to the gateway for delivery.
/// `delivery_id` (iter-720) names the `cron_deliveries` ledger row the
/// consumer settles via `CronDb::mark_delivery_outcome` — the consumer
/// reports outcomes against the row id, not the job id, so replays and
/// first sends stay distinguishable in history.
/// `job_id` (v6 row 5) names the originating job so the consumer can
/// attribute the outcome event (`ObserverEvent::CronDeliveryOutcome`)
/// without a ledger lookup.
pub struct CronDelivery {
    pub platform: String,
    pub chat_id: String,
    pub content: String,
    pub delivery_id: Option<i64>,
    pub job_id: String,
}

/// iter-720 (trinity P2-A concrete, hermes deliveries.db shape): a
/// delivery is PARKED — the tombstone — after this many handoffs to the
/// consumer without a delivered confirmation. Hermes' parallel is the
/// lease/retry redelivery cap + poison-park.
pub const MAX_DELIVERY_ATTEMPTS: u32 = 3;

/// iter-720: the stale-claim reclaim horizon — hermes' `max(3×timeout,
/// 2h)` with the send being a fire-and-forget handoff (no per-send
/// timeout), so the 2h floor applies. A pending row whose last handoff
/// (or creation) is older than this is presumed dead — crashed consumer
/// or gateway outage — and returns to the scheduler for re-send.
pub const DELIVERY_RECLAIM_AFTER_SECS: i64 = 2 * 60 * 60;

pub struct CronScheduler {
    db: Arc<CronDb>,
    agent: Arc<OperantAgent>,
    delivery_tx: Option<mpsc::UnboundedSender<CronDelivery>>,
    /// The operant root (the directory holding the org dbs), for the
    /// seat's MEMORY.md (`<root>/org/employees/<seat>/MEMORY.md`,
    /// iter-666). `None` disables injection — prompt byte-identical.
    seat_memory_root: Option<std::path::PathBuf>,
    /// iter-669: transient-retry attempts used per job id, in-memory.
    /// A restart resets the budget — the documented ceiling: a crashing
    /// loop can never arm more than [`MAX_TRANSIENT_RETRIES`] per success,
    /// and a restart is itself a fresh cadence.
    transient_retries: Arc<std::sync::Mutex<std::collections::HashMap<String, u8>>>,
    /// iter-678 (gap 1, metering): the accumulator fed by the cron agent's
    /// OWN event channel (the gateway's receiver never sees cron usage —
    /// that was the misattribution root). `None` = no metering, runs stay
    /// un-drained exactly like before (dark-mergeable).
    usage_meter: Option<Arc<std::sync::Mutex<UsageAccumulator>>>,
    /// iter-678: the persistent session store the scheduler drains into —
    /// the same store the gateway's budget rollup reads
    /// (`employee_window_usage`). `None` = no drain (dark-mergeable).
    usage_store: Option<Arc<crate::gateway_session::PersistentSessionStore>>,
    /// iter-678 (gap 1, consult): per-seat budget overrides. `None` = no
    /// seat is budget-governed on the cron path (the default; `[bylaws].budget`
    /// cap 0 = ungoverned keeps everything dark-mergeable).
    seat_budgets: Option<Arc<crate::org::seat_budgets::SeatBudgetDb>>,
    /// iter-678: the org-wide budget default (`[bylaws].budget`), resolved
    /// per seat via `resolve_budget` when `seat_budgets` is mounted.
    default_budget: crate::config::BudgetSettings,
    /// iter-679 (gap 5, phase 1): the DM/feed context injector. `None` =
    /// no injection, prompts byte-identical (dark-mergeable).
    context_injection: Option<Arc<crate::org::context_injection::ContextInjector>>,
    /// iter-684 (gap 6): the daily seat-pairing sessions' settings.
    /// `None` = never mounted, the tick never checks (dark-mergeable).
    /// iter-720 (trinity P2-A): the observer told about delivery-ledger
    /// lifecycle points (queued / reclaimed / parked). `None` = the
    /// default no-op, tick byte-identical (dark-mergeable).
    observer: Option<Arc<dyn crate::observer::Observer>>,
}

/// A seat's effective budget, with its window usage or the read error.
type SeatBudgetState = (
    crate::org::seat_budgets::EffectiveBudget,
    Result<(i64, f64), String>,
);

impl CronScheduler {
    pub fn new(db: Arc<CronDb>, agent: Arc<OperantAgent>) -> Self {
        Self {
            db,
            agent,
            delivery_tx: None,
            seat_memory_root: None,
            transient_retries: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            usage_meter: None,
            usage_store: None,
            seat_budgets: None,
            default_budget: crate::config::BudgetSettings::default(),
            context_injection: None,
            observer: None,
        }
    }

    /// Mount an observer for delivery-ledger lifecycle events (iter-720).
    /// Not called by any default boot path — the opt-in that keeps
    /// emission dark-mergeable.
    pub fn with_observer(mut self, observer: Arc<dyn crate::observer::Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn with_delivery(mut self, tx: mpsc::UnboundedSender<CronDelivery>) -> Self {
        self.delivery_tx = Some(tx);
        self
    }

    /// Mount the operant root for seat-memory injection (iter-666). Without
    /// it the run path never reads a seat file — prompts stay
    /// byte-identical, dark-mergeable like the org gate.
    pub fn with_seat_memory_root(mut self, root: std::path::PathBuf) -> Self {
        self.seat_memory_root = Some(root);
        self
    }

    /// iter-678: mount the cron agent's usage accumulator (fed by its own
    /// event channel) so runs meter. Without it no drain happens —
    /// dark-mergeable.
    pub fn with_usage_meter(mut self, meter: Arc<std::sync::Mutex<UsageAccumulator>>) -> Self {
        self.usage_meter = Some(meter);
        self
    }

    /// iter-678: mount the persistent session store runs drain their usage
    /// into (the store `employee_window_usage` reads for budget rollups).
    pub fn with_usage_store(
        mut self,
        store: Arc<crate::gateway_session::PersistentSessionStore>,
    ) -> Self {
        self.usage_store = Some(store);
        self
    }

    /// iter-678: mount per-seat budget overrides + the org-wide default for
    /// the cron-path budget envelope (resolve → turn-start gate → envelope).
    pub fn with_budgets(
        mut self,
        seat_budgets: Arc<crate::org::seat_budgets::SeatBudgetDb>,
        default_budget: crate::config::BudgetSettings,
    ) -> Self {
        self.seat_budgets = Some(seat_budgets);
        self.default_budget = default_budget;
        self
    }

    /// iter-679: mount the DM/feed context injector. Renders the bounded,
    /// deduplicated, ranked context section (org feeds + DMs) ahead of the
    /// seat's prompt; absent/empty → byte-identical.
    pub fn with_context_injection(
        mut self,
        injector: Arc<crate::org::context_injection::ContextInjector>,
    ) -> Self {
        self.context_injection = Some(injector);
        self
    }

    /// The seat a job belongs to: the §3.1.1 `employee_cron_jobs` join when
    /// the job is linked, else the derived id — the pre-661 claim. Shared
    /// by worklog attribution and seat-memory injection so the two always
    /// agree on which seat a run is.
    fn resolve_seat_id(&self, _job: &CronJob, derived_session_id: &str) -> String {
        // G+1: the employee registry retired to Holon (`api_employees`);
        // the derived session id is the whole claim again.
        derived_session_id.to_string()
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

        // iter-720 (trinity P2-A): replay-after-unblock — stale pending
        // delivery rows come back around on EVERY tick, empty-due ticks
        // included (an outage-recovery window usually has no due jobs;
        // same lesson as the empty-tick socialization fix below). A
        // bounded batch that never delays a live dispatch.
        self.replay_stale_deliveries(DELIVERY_RECLAIM_AFTER_SECS)
            .await;

        let due_jobs = self.db.get_due_jobs()?;
        if due_jobs.is_empty() {
            // iter-684 gap 6 (defect found live 2026-10-09, arming the
            // sessions for the first time): the socialization coordinator
            // must be checked on EVERY tick — this early return used to
            // skip it, so a schedule fire landing on an empty tick was
            // delayed to the next busy one (the 09:30 session would have
            // waited for the 10:00 hourly jobs; an armed-with-no-stamp box
            // waits even longer). The unit tests call the coordinator
            // directly, so the tick-level gate was never pinned.
            return Ok(());
        }

        debug!("Found {} due cron jobs", due_jobs.len());

        for job in due_jobs {
            if let Err(e) = self.run_job(&job).await {
                error!("Failed to run cron job {}: {}", job.id, e);
            }
        }

        // iter-684 (gap 6): the daily seat-pairing sessions — a schedule
        // check, not a job row (infra-owned, outside the cast registry).
        // After the due jobs so a busy tick never delays a live dispatch.

        Ok(())
    }

    async fn run_job(&self, job: &CronJob) -> Result<(), Error> {
        info!("Executing cron job {}: {}", job.id, job.name);
        let run_started_at = chrono::Utc::now().to_rfc3339();

        // ── iter-678: cron-path budget gate (gap 1) ────────────────────
        // Mirrors the gateway's turn-start gate: a run that would START
        // over a HARD cap is refused with an actionable message and never
        // dispatched. Soft mode never refuses (it self-economizes via the
        // injection in `run_agent_job`); ungoverned (cap <= 0, the
        // default) is `None` — byte-identical.
        if let Some(reason) = self.budget_gate_block(job) {
            return self.record_budget_block(job, reason);
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
        reason = "`script` is Ok here: the `is_err()` branch above returns early"
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

    /// iter-678: the effective budget + window usage for a seat, when the
    /// budget stores are mounted. `None` = ungoverned (no budget consulted
    /// at all) — the dark-mergeable default. A usage read error comes back
    /// as `Err`, and the caller decides by mode (GW-8: a hard cap refuses,
    /// soft mode runs). A budget-override read error still degrades to the
    /// default budget (AD-009 residual).
    fn seat_budget_state(&self, seat_id: &str) -> Option<SeatBudgetState> {
        let budgets = self.seat_budgets.as_ref()?;
        let store = self.usage_store.as_ref()?;
        let seat_override = budgets.get(seat_id).ok().flatten();
        let effective =
            crate::org::seat_budgets::resolve_budget(seat_override.as_ref(), &self.default_budget)?;
        let since = crate::org::seat_budgets::window_start(&effective.window);
        let usage = store
            .employee_window_usage(seat_id, &since)
            .map_err(|e| e.to_string());
        Some((effective, usage))
    }

    /// iter-678: the budget gate decision for `job`, as a refusal reason.
    /// `None` = the run may proceed. A hard cap already blown at run start
    /// refuses — the run was never going to fit the window.
    fn budget_gate_block(&self, job: &CronJob) -> Option<String> {
        let derived = crate::org::worklog::derive_employee_id(&job.id);
        let seat_id = self.resolve_seat_id(job, &derived);
        self.budget_refusal_for_seat(&seat_id)
    }

    /// iter-684: the hard-cap refusal for a SEAT (not a job) — the
    /// socialization turns consult the same posture a cron run does.
    /// `None` = the turn may run. Soft mode and ungoverned seats never refuse.
    fn budget_refusal_for_seat(&self, seat_id: &str) -> Option<String> {
        let (budget, usage) = self.seat_budget_state(seat_id)?;
        // GW-8: a hard cap whose spend cannot be read is refused, not run uncapped.
        let (tokens_used, usd_used) = match usage {
            Ok(usage) => usage,
            Err(err) if budget.mode == "hard" => {
                return Some(format!(
                    "budget for seat `{seat_id}` could not be read ({err}) — the run was not executed"
                ));
            }
            Err(_) => return None,
        };
        let (used, remaining, unit) = if budget.basis == "usd" {
            (usd_used, budget.cap - usd_used, "USD")
        } else {
            (
                tokens_used as f64,
                budget.cap - tokens_used as f64,
                "tokens",
            )
        };
        if remaining > 0.0 || budget.mode != "hard" {
            return None;
        }
        Some(format!(
            "budget cap reached for seat `{}` — {:.0} of {:.0} {} used this {}; \
             the run was not executed. Raise the `seat_budgets` cap, or wait \
             for the window to roll",
            seat_id, used, budget.cap, unit, budget.window
        ))
    }

    /// iter-684: bind the shared agent to a seat and compose its full prompt
    /// — session key, seat + charter, seat memory, injected feeds. The
    /// shared preamble of every seat-shaped run (cron jobs and socialization
    /// turns), so the two paths can never disagree about what a seat sees.
    fn bind_seat_run(&self, seat_id: &str, session_key: &str, base_prompt: &str) -> String {
        self.agent.set_session_id(session_key.to_string());
        self.agent.set_seat_id(seat_id.to_string());
        // GW-2: a seat run reads and writes its own seat's bank, never the
        // scope the last chat turn left on the shared agent.
        if let Some(p) = self.agent.memory_provider() {
            p.set_scope(&crate::memory_wire::MemoryScope::Seat {
                identity: seat_id.to_string(),
            });
        }
        // G+1: seat charters live in Holon job descriptions now; the
        // personal scheduler injects no charter.
        let charter: Option<String> = None;
        self.agent.set_charter(charter.clone());
        let prompt = match self.seat_memory_root.as_deref() {
            Some(root) => seat_memory_prompt(root, seat_id, base_prompt),
            None => base_prompt.to_string(),
        };
        let prompt = match self.context_injection.as_ref() {
            Some(injector) => injector.render_section(seat_id, charter.as_deref(), &prompt, &[]),
            None => prompt,
        };
        // iter-688: the seat's unacknowledged notices ride ahead of every
        // feed/directive section — a pending notice is a request awaiting
        // action, the board's read side at last (the ack is the watermark).
        // `session_key` is the employee id the run executes as (the
        // identity gate's binding); a non-employee or org-off session
        // yields an empty block (fail-open inside the injector).
        match self.context_injection.as_ref() {
            Some(injector) => {
                let notices = injector.pending_notices_block(session_key);
                if notices.is_empty() {
                    prompt
                } else {
                    format!("{notices}\n\n{prompt}")
                }
            }
            None => prompt,
        }
    }

    /// iter-678: fail-open side effect for a budget refusal — the gate's
    /// mirror of [`Self::record_gate_block`]: persist the block on the job
    /// row, append the history row with its own forensically distinct
    /// `budget_block` origin, advance `next_run_at` (no busy-spin), and
    /// return without executing.
    fn record_budget_block(&self, job: &CronJob, reason: String) -> Result<(), Error> {
        let started_at = chrono::Utc::now().to_rfc3339();
        error!("Cron job {} blocked by the budget gate: {}", job.id, reason);
        self.db.mark_job_run(
            &job.id,
            false,
            Some(reason.clone()),
            None,
            self.compute_next_run(job),
        )?;
        if let Err(e) =
            self.db
                .record_cron_run(&job.id, &started_at, false, Some(reason), "budget_block")
        {
            warn!("cron run history write failed for {}: {e}", job.id);
        }
        Ok(())
    }

    /// iter-678 (gap 1, metering): fold this run's meter delta into the
    /// persistent session store under the seat — the exact write
    /// `employee_window_usage` (the rollup the envelope consult reads)
    /// sums. Runs are serial in the tick loop, so the delta between the
    /// pre-run snapshot and the post-run read is this run's usage alone.
    /// Fail-open (bylaws invariant): a metering write never fails a run.
    fn drain_run_usage(
        &self,
        source_id: &str,
        source_name: &str,
        seat_id: &str,
        meter_before: Option<(u64, u64, f64)>,
    ) {
        let (Some(meter), Some(store)) = (self.usage_meter.as_ref(), self.usage_store.as_ref())
        else {
            return;
        };
        let Some((b_in, b_out, b_cost)) = meter_before else {
            return;
        };
        let Ok(acc) = meter.lock() else {
            return; // poisoned meter = no drain, fail-open
        };
        let in_delta = acc.input_tokens().saturating_sub(b_in);
        let out_delta = acc.output_tokens().saturating_sub(b_out);
        let cost_delta = (acc.cost_usd() - b_cost).max(0.0);
        drop(acc);
        if in_delta == 0 && out_delta == 0 && cost_delta == 0.0 {
            return;
        }
        let source = crate::gateway_session::SessionSource {
            platform: "cron".to_string(),
            chat_id: source_id.to_string(),
            chat_name: Some(source_name.to_string()),
            chat_type: "cron".to_string(),
            ..Default::default()
        };
        match store.get_or_create_session(&source, false) {
            Ok(entry) => {
                if let Err(e) = store.bind_employee(&entry.session_key, seat_id) {
                    warn!("cron usage bind failed for {}: {e} (fail-open)", source_id);
                }
                if let Err(e) =
                    store.update_tokens(&entry.session_key, in_delta, out_delta, 0, 0, cost_delta)
                {
                    warn!("cron usage drain failed for {}: {e} (fail-open)", source_id);
                }
                // GW-1: the run's spend is charged to the seat it ran under.
                if let Err(e) = store.record_seat_spend(seat_id, in_delta + out_delta, cost_delta) {
                    warn!("cron seat spend failed for {}: {e} (fail-open)", source_id);
                }
            }
            Err(e) => {
                warn!(
                    "cron usage session create failed for {}: {e} (fail-open)",
                    source_id
                )
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
        //
        // iter-684: the seat preamble (binding, charter, seat memory,
        // injected feeds) is the shared `bind_seat_run` — one composition
        // path for cron runs and socialization turns, so the two can never
        // disagree about what a seat sees. Its history: iter-672 bound the
        // seat (the guard at stream.rs:811 keys on it); iter-666 injected
        // MEMORY.md; iter-679 injected the bounded feed section.
        let session_id = crate::org::worklog::derive_employee_id(&job.id);
        let seat_id = self.resolve_seat_id(job, &session_id);
        let prompt = self.bind_seat_run(&seat_id, &session_id, &job.prompt);

        // ── iter-678: budget posture (gap 1) ──────────────────────────
        // Mirrors the gateway's turn-start construction
        // (gateway_runner.rs:1040-1126): a HARD token cap the run still
        // fits under arms the mid-flight envelope (the loop stops at its
        // next iteration boundary once spend crosses the window line);
        // every governed run gets the remaining-budget injection so it
        // self-economizes BEFORE the cap. The hard refuse itself already
        // ran in `run_job` — this is the in-run half. Ungoverned seats:
        // `None`, prompt byte-identical.
        let mut seat_envelope: Option<crate::agent::SeatBudgetEnvelope> = None;
        let prompt = match self.seat_budget_state(&seat_id) {
            None | Some((_, Err(_))) => prompt,
            Some((budget, Ok((tokens_used, usd_used)))) => {
                let (_used, remaining, unit) = if budget.basis == "usd" {
                    (usd_used, budget.cap - usd_used, "USD")
                } else {
                    (
                        tokens_used as f64,
                        budget.cap - tokens_used as f64,
                        "tokens",
                    )
                };
                if budget.mode == "hard" && budget.basis == "tokens" && remaining > 0.0 {
                    seat_envelope = Some(crate::agent::SeatBudgetEnvelope {
                        cap_tokens: budget.cap,
                        used_at_turn_start: tokens_used as f64,
                    });
                }
                let posture = if remaining <= 0.0 {
                    "OVER the cap (soft mode: continue, but say so)".to_string()
                } else {
                    format!("{remaining:.0} of {:.0} {unit} remain", budget.cap)
                };
                format!(
                    "<budget_state>\nSeat `{seat_id}` budget — {} window {}, mode {}: {posture}. \
                     Be mindful of consumption; prefer efficient tool use and concise \
                     reasoning while the budget is tight.\n</budget_state>\n\n{prompt}",
                    budget.basis, budget.window, budget.mode
                )
            }
        };

        // iter-678: snapshot the meter before the run — the tick loop is
        // serial, so the post-run delta is this run's usage alone.
        let meter_before = self.usage_meter.as_ref().and_then(|m| {
            m.lock()
                .ok()
                .map(|acc| (acc.input_tokens(), acc.output_tokens(), acc.cost_usd()))
        });

        // NOTE: the memory-graph session boundary that `clear_history` fired
        // (events.rs:193-197, `submit_session_end` / `on_session_end`) is NOT
        // reproduced here, because no provider implements it usefully:
        // `MemoryProvider::on_session_switch` (memory_provider.rs:302) is an
        // empty trait default, `BuiltinProvider` (`:383`) only logs at debug, and
        // MemoryWire does not override it at all. Calling a no-op would only
        // LOOK like preservation. The drop is recorded in BUGS.md (D-5b) rather
        // than papered over. `notify_session_switch` therefore still has zero
        // callers.

        // iter-678: the envelope-scoped run — ungoverned runs take the
        // byte-identical legacy call.
        let run_result = match seat_envelope {
            Some(envelope) => {
                crate::agent::SEAT_BUDGET_ENVELOPE
                    .scope(envelope, self.agent.run(prompt))
                    .await
            }
            None => self.agent.run(prompt).await,
        };

        // iter-678 (gap 1, metering): drain the run's usage into the seat's
        // rollup BEFORE the outcome arms — error runs are metered too, the
        // same order the gateway drains at turn end.
        self.drain_run_usage(&job.id, &job.name, &seat_id, meter_before);

        match run_result {
            Ok(message) => (
                true,
                "Agent run completed".into(),
                message.content,
                None,
                false,
            ),
            Err(e) => {
                let transient = should_arm_transient_retry(&e);
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

    async fn deliver_result(&self, job: &CronJob, content: &str) -> Result<(), Error> {
        info!("Delivering result for job {}: {}", job.id, job.name);

        if let (Some(tx), Some(platform), Some(chat_id)) =
            (&self.delivery_tx, &job.origin_platform, &job.origin_chat_id)
        {
            let header = format!("📋 **Cron: {}**\n\n", job.name);
            let payload = format!("{}{}", header, content);
            let target = format!("{}:{}", platform, chat_id);

            // iter-720: queue the delivery on the durable ledger BEFORE
            // the handoff — the in-memory channel drops silently on a
            // dead consumer, the ledger row is what makes the drop
            // visible and replayable. Audit-layer discipline: a write
            // failure warns and never fails the run.
            let delivery_id = match self.db.record_cron_delivery(&job.id, &target, &payload) {
                Ok(id) => {
                    if let Some(observer) = &self.observer {
                        observer.record_event(
                            &crate::observer::ObserverEvent::CronDeliveryQueued {
                                job_id: job.id.clone(),
                                delivery_id: id,
                                target: target.clone(),
                            },
                        );
                    }
                    Some(id)
                }
                Err(e) => {
                    warn!(
                        "cron delivery ledger write failed for {}: {e} (handoff proceeds)",
                        job.id
                    );
                    None
                }
            };

            // The send is the attempt: count it, park (tombstone) on the
            // cap. `tx.send` on an unbounded channel errors only when the
            // receiver is GONE (gateway shutdown) — that error counts.
            if let Err(e) = tx.send(CronDelivery {
                platform: platform.clone(),
                chat_id: chat_id.clone(),
                content: payload,
                delivery_id,
                job_id: job.id.clone(),
            }) {
                if let Some(id) = delivery_id {
                    self.account_delivery_attempt(id, &job.id, Some(&e.to_string()));
                }
            } else if let Some(id) = delivery_id {
                self.account_delivery_attempt(id, &job.id, None);
            }
        } else {
            // R39: silent debug-level drops hid broken cron delivery for
            // months — a job created without origin fields simply never
            // delivered and last_status still read ok. No ledger row is
            // written here: nothing was ever deliverable, a row would be
            // fiction.
            warn!(
                "No delivery target for job {} (deliver={})",
                job.id, job.deliver
            );
        }
        Ok(())
    }

    /// One handoff's ledger accounting (iter-720): bump the attempt
    /// counter, PARK on the cap (the tombstone — replay stops, the row
    /// stays inspectable), emit the observer events. Warn-only — the
    /// audit layer never fails the run whose outcome is already sent.
    fn account_delivery_attempt(&self, delivery_id: i64, job_id: &str, error: Option<&str>) {
        match self
            .db
            .mark_delivery_attempt(delivery_id, MAX_DELIVERY_ATTEMPTS, error)
        {
            Ok(state) if state == "parked" => {
                warn!(
                    "cron delivery {delivery_id} for {job_id} parked after \
                     {MAX_DELIVERY_ATTEMPTS} attempts (tombstone; replay stops)"
                );
                if let Some(observer) = &self.observer {
                    observer.record_event(&crate::observer::ObserverEvent::CronDeliveryParked {
                        job_id: job_id.to_string(),
                        delivery_id,
                        attempts: MAX_DELIVERY_ATTEMPTS,
                    });
                }
            }
            Ok(_) => {}
            Err(e) => warn!("cron delivery attempt write failed for {delivery_id}: {e}"),
        }
    }

    /// iter-720 (trinity P2-A): the stale-claim reclaim — hermes'
    /// replay-after-unblock. Pending ledger rows whose last handoff (or
    /// creation) is older than the horizon are presumed dead (crashed
    /// consumer, gateway outage) and re-sent; the attempt counter walks
    /// each toward the [`MAX_DELIVERY_ATTEMPTS`] tombstone. With no
    /// delivery channel mounted the rows stay pending and retry on a
    /// later tick — an outage queues instead of drops.
    /// `horizon_secs` is a parameter so tests can drive the reclaim
    /// without a wall-clock wait; the tick path passes the const.
    async fn replay_stale_deliveries(&self, horizon_secs: i64) {
        let stale = match self
            .db
            .reclaim_stale_deliveries(chrono::Duration::seconds(horizon_secs))
        {
            Ok(rows) => rows,
            Err(e) => {
                warn!("cron delivery reclaim scan failed: {e}");
                return;
            }
        };
        if stale.is_empty() {
            return;
        }
        let Some(tx) = &self.delivery_tx else {
            // No consumer mounted — the rows stay pending and retry
            // when the gateway next arms the channel.
            return;
        };
        for row in stale {
            let Some((platform, chat_id)) = row.target.split_once(':') else {
                warn!(
                    "cron delivery {} has unparseable target {}",
                    row.id, row.target
                );
                continue;
            };
            if let Err(e) = tx.send(CronDelivery {
                platform: platform.to_string(),
                chat_id: chat_id.to_string(),
                content: row.payload.clone(),
                delivery_id: Some(row.id),
                job_id: row.job_id.clone(),
            }) {
                self.account_delivery_attempt(row.id, &row.job_id, Some(&e.to_string()));
            } else {
                self.account_delivery_attempt(row.id, &row.job_id, None);
                if let Some(observer) = &self.observer {
                    observer.record_event(&crate::observer::ObserverEvent::CronDeliveryReclaimed {
                        job_id: row.job_id.clone(),
                        delivery_id: row.id,
                        attempt: row.attempts + 1,
                    });
                }
            }
        }
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

/// iter-684 (gap 6): append a bounded socialization outcome to a seat's
/// MEMORY.md — the two-tier memory doc's seat tier, appended by the
/// socialization coordinator (the seat itself curates what stays). Creates
/// the file and its directory when absent — a cold-start seat's first
/// socialization IS its first memory. `None` root = no-op (the caller
/// warns); a failed write warns and never fails the session.
#[cfg(test)]
fn seat_memory_append(
    org_root: Option<&std::path::Path>,
    seat_id: &str,
    heading: &str,
    text: &str,
) {
    let Some(org_root) = org_root else {
        return;
    };
    let path = seat_memory_path(org_root, seat_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entry = format!("\n{heading}\n{}\n", text.trim());
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, entry.as_bytes()))
    {
        Ok(()) => {}
        Err(e) => warn!("seat memory append failed for `{seat_id}`: {e} (fail-open)"),
    }
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

/// iter-673: whether a failed agent run is worth a bounded re-arm. The
/// global classifier's exclusion of `StreamDied` is about the IN-STREAM
/// ladder (a spent per-turn retry budget must not invite another ladder
/// — the scheduler's stream retry budget); the cron cadence seam is a
/// different knob: a dead stream that exhausted the in-turn ladder is,
/// at cadence granularity, a flake that recovers (observed live: 20:09
/// warden and 20:31 dispatcher stream-deaths each recovered on the next
/// scheduled tick — a missed cycle is a missed deliverable, while a
/// re-armed retry would have delivered ~60s late). The 2-per-success
/// budget (iter-669) caps the ladder risk here.
fn should_arm_transient_retry(e: &crate::error::Error) -> bool {
    e.is_transient() || matches!(e, crate::error::Error::StreamDied { .. })
}

fn repeat_limit_reached(repeat_times: Option<i32>, repeat_completed: i32) -> bool {
    match repeat_times {
        Some(times) if times > 0 => repeat_completed + 1 >= times,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        repeat_limit_reached, seat_memory_append, seat_memory_prompt, should_arm_transient_retry,
    };
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

    // ── iter-678 (gap 1): cron-path budget gate + metering drain ──

    /// A real scheduler over a real (uninvoked) agent, with the budget
    /// stores mounted. Mirrors `transient_retry_budget_is_bounded...`'s
    /// construction: the helpers never invoke the agent.
    fn scheduler_with_budgets(
        dir: &std::path::Path,
        default_budget: crate::config::BudgetSettings,
        seat_budget: Option<crate::org::seat_budgets::SeatBudget>,
    ) -> super::CronScheduler {
        let db = Arc::new(crate::database::Database::init(dir.join("agent.sqlite")).expect("db"));
        let agent = Arc::new(crate::agent::OperantAgent::new(
            crate::agent::AgentConfig::default(),
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            crate::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
            db,
        ));
        let cron_db = Arc::new(super::CronDb::init(dir.join("cron.sqlite")).expect("cron db"));
        let seat_budgets = Arc::new(
            crate::org::seat_budgets::SeatBudgetDb::init(&dir.join("org.sqlite")).expect("budgets"),
        );
        if let Some(budget) = seat_budget {
            seat_budgets.upsert(&budget).expect("upsert budget");
        }
        let store = Arc::new(
            crate::gateway_session::PersistentSessionStore::open(
                dir.join("sessions.sqlite").to_str().expect("utf-8 path"),
            )
            .expect("session store"),
        );
        super::CronScheduler::new(Arc::clone(&cron_db), agent)
            .with_budgets(seat_budgets, default_budget)
            .with_usage_store(store)
    }

    fn test_job() -> crate::cronjobs::db::CronJob {
        crate::cronjobs::db::CronJob {
            id: "job_budget_probe".to_string(),
            name: "budget probe".to_string(),
            prompt: "probe".to_string(),
            schedule: "0 0 * * * *".to_string(),
            schedule_display: "hourly".to_string(),
            repeat_times: None,
            repeat_completed: 0,
            deliver: "none".to_string(),
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
            enabled: true,
            state: "scheduled".to_string(),
            paused_at: None,
            paused_reason: None,
            created_at: String::new(),
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            last_error: None,
            last_delivery_error: None,
        }
    }

    /// GW-2: a provider that records each seat the shared agent is scoped to.
    #[derive(Default)]
    struct ScopeRecorder {
        seats: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::memory_provider::MemoryProvider for ScopeRecorder {
        fn name(&self) -> &str {
            "scope-recorder"
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn initialize(&self, _session_id: &str) -> crate::Result<()> {
            Ok(())
        }

        fn set_scope(&self, scope: &crate::memory_wire::MemoryScope) {
            if let crate::memory_wire::MemoryScope::Seat { identity } = scope {
                self.seats.lock().unwrap().push(identity.clone());
            }
        }
    }

    #[tokio::test]
    async fn a_seat_run_sets_its_own_seat_as_the_memory_scope() {
        // GW-2: a cron or socialization run reads and writes its seat's bank,
        // not whatever scope the last chat turn left on the shared agent.
        let dir = tempfile::tempdir().expect("tempdir");
        let recorder = Arc::new(ScopeRecorder::default());
        let db =
            Arc::new(crate::database::Database::init(dir.path().join("agent.sqlite")).expect("db"));
        let agent = Arc::new(
            crate::agent::OperantAgent::new(
                crate::agent::AgentConfig::default(),
                Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                    crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
                )),
                crate::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
                db,
            )
            .with_memory_provider(recorder.clone()),
        );
        let cron_db =
            Arc::new(super::CronDb::init(dir.path().join("cron.sqlite")).expect("cron db"));
        let scheduler = super::CronScheduler::new(cron_db, agent);

        scheduler.bind_seat_run("dispatcher", "cron_session", "prompt");

        assert_eq!(
            *recorder.seats.lock().unwrap(),
            vec!["dispatcher".to_string()]
        );
    }

    fn spend_window_for(scheduler: &super::CronScheduler, seat_id: &str, tokens: u64) {
        let store = scheduler.usage_store.clone().expect("store");
        let source = crate::gateway_session::SessionSource {
            platform: "cron".to_string(),
            chat_id: "job_budget_probe".to_string(),
            chat_type: "cron".to_string(),
            ..Default::default()
        };
        let entry = store
            .get_or_create_session(&source, false)
            .expect("session");
        store
            .bind_employee(&entry.session_key, seat_id)
            .expect("bind");
        store
            .update_tokens(&entry.session_key, tokens, 0, 0, 0, 0.0)
            .expect("spend");
        store
            .record_seat_spend(seat_id, tokens, 0.0)
            .expect("seat spend ledger");
    }

    /// GW-8: break the meter. The usage roll-up reads the `seat_spend` ledger,
    /// so dropping that table makes every read fail.
    fn break_the_usage_meter(dir: &std::path::Path) {
        rusqlite::Connection::open(dir.join("sessions.sqlite"))
            .expect("open sessions db")
            .execute_batch("DROP TABLE seat_spend")
            .expect("drop usage table");
    }

    #[test]
    fn budget_gate_refuses_a_hard_cap_whose_spend_cannot_be_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_id = crate::org::worklog::derive_employee_id("job_budget_probe");
        let scheduler = scheduler_with_budgets(
            dir.path(),
            crate::config::BudgetSettings::default(),
            Some(crate::org::seat_budgets::SeatBudget {
                employee_id: seat_id,
                basis: None,
                window: None,
                cap: 50.0,
                mode: Some("hard".to_string()),
            }),
        );
        break_the_usage_meter(dir.path());

        let Some(reason) = scheduler.budget_gate_block(&test_job()) else {
            panic!("an unreadable hard cap must refuse, not run uncapped");
        };
        assert!(reason.contains("could not be read"), "got: {reason}");
    }

    #[test]
    fn budget_gate_soft_mode_still_runs_when_the_meter_is_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_id = crate::org::worklog::derive_employee_id("job_budget_probe");
        let scheduler = scheduler_with_budgets(
            dir.path(),
            crate::config::BudgetSettings::default(),
            Some(crate::org::seat_budgets::SeatBudget {
                employee_id: seat_id,
                basis: None,
                window: None,
                cap: 50.0,
                mode: Some("soft".to_string()),
            }),
        );
        break_the_usage_meter(dir.path());

        assert!(
            scheduler.budget_gate_block(&test_job()).is_none(),
            "soft mode never refuses, even when the meter is unreadable"
        );
    }

    #[test]
    fn budget_gate_ungoverned_keeps_the_run_byte_identical() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No seat row + cap 0 default = ungoverned — the dark-mergeable path.
        let scheduler =
            scheduler_with_budgets(dir.path(), crate::config::BudgetSettings::default(), None);
        assert!(
            scheduler.budget_gate_block(&test_job()).is_none(),
            "ungoverned seats must not be budget-gated"
        );
    }

    #[test]
    fn budget_gate_refuses_a_hard_cap_already_blown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_id = crate::org::worklog::derive_employee_id("job_budget_probe");
        let scheduler = scheduler_with_budgets(
            dir.path(),
            crate::config::BudgetSettings::default(),
            Some(crate::org::seat_budgets::SeatBudget {
                employee_id: seat_id.clone(),
                basis: None,
                window: None,
                cap: 50.0,
                mode: Some("hard".to_string()),
            }),
        );
        // Pre-spend the window: a session row bound to the seat with 60
        // tokens already drained — exactly what the metering wire writes.
        spend_window_for(&scheduler, &seat_id, 60);

        let Some(reason) = scheduler.budget_gate_block(&test_job()) else {
            panic!("a blown hard cap must refuse");
        };
        assert!(reason.contains("budget cap reached"), "got: {reason}");
    }

    #[test]
    fn budget_gate_soft_mode_never_refuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_id = crate::org::worklog::derive_employee_id("job_budget_probe");
        let scheduler = scheduler_with_budgets(
            dir.path(),
            crate::config::BudgetSettings::default(),
            Some(crate::org::seat_budgets::SeatBudget {
                employee_id: seat_id.clone(),
                basis: None,
                window: None,
                cap: 50.0,
                mode: Some("soft".to_string()),
            }),
        );
        spend_window_for(&scheduler, &seat_id, 999);
        assert!(
            scheduler.budget_gate_block(&test_job()).is_none(),
            "soft mode must never refuse at the gate"
        );
    }

    #[test]
    fn drain_run_usage_writes_the_seat_rollup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seat_id = crate::org::worklog::derive_employee_id("job_budget_probe");
        let scheduler =
            scheduler_with_budgets(dir.path(), crate::config::BudgetSettings::default(), None);
        // The meter saw a run's usage after the snapshot: 120 in / 45 out
        // + $0.01 cost — the delta the drain must fold into the seat's
        // rollup.
        let meter = Arc::new(std::sync::Mutex::new(
            crate::org::worklog::UsageAccumulator::new(),
        ));
        {
            let mut acc = meter.lock().expect("meter");
            acc.record(120, 45);
            acc.record_cost(0.01);
        }
        let scheduler = scheduler.with_usage_meter(meter);
        scheduler.drain_run_usage(
            "job_budget_probe",
            "budget probe",
            &seat_id,
            Some((0, 0, 0.0)),
        );

        let store = scheduler.usage_store.clone().expect("store");
        let since = crate::org::seat_budgets::window_start("daily");
        let (tokens, usd) = store
            .employee_window_usage(&seat_id, &since)
            .expect("rollup");
        assert_eq!(tokens, 165, "in+out must land in the seat's rollup");
        assert!((usd - 0.01).abs() < 1e-9, "cost must land too, got {usd}");
    }

    #[test]
    fn seat_memory_append_creates_and_appends() {
        let dir = tempfile::tempdir().expect("tempdir");
        seat_memory_append(
            Some(dir.path()),
            "identity-warden",
            "## Socialization 2026-10-08",
            "Session with premiere: aligned on the audit cadence.",
        );
        seat_memory_append(
            Some(dir.path()),
            "identity-warden",
            "## Socialization 2026-10-09",
            "Session with premiere: follow-up held.",
        );
        let read =
            std::fs::read_to_string(dir.path().join("org/employees/identity-warden/MEMORY.md"))
                .expect("read back");
        assert!(read.contains("2026-10-08") && read.contains("2026-10-09"));
        assert!(read.contains("audit cadence"));
        // No root = a no-op, never a panic.
        seat_memory_append(None, "x", "h", "t");
    }

    #[test]
    fn bind_seat_run_retargets_the_shared_agent_not_clears_it() {
        // BUGS.md D-1's owed verification, pinned iter-684: the scheduler
        // must retarget the shared agent's session via `set_session_id`
        // (the non-destructive swap the 548 substrate built), never the
        // global `clear_history`. A regression back to clear_history
        // fails here: it sets no id, so both asserts below trip.
        let dir = tempfile::tempdir().expect("tempdir");
        let scheduler =
            scheduler_with_budgets(dir.path(), crate::config::BudgetSettings::default(), None);
        let prompt = scheduler.bind_seat_run("identity-warden", "emp-warden-1", "BASE PROMPT");
        assert_eq!(
            scheduler.agent.session_id().as_deref(),
            Some("emp-warden-1")
        );
        assert_eq!(
            scheduler.agent.seat_id().as_deref(),
            Some("identity-warden")
        );
        assert_eq!(
            prompt, "BASE PROMPT",
            "no memory/injection mounted → byte-identical"
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

    // ── iter-673: stream-death is retryable at the cadence seam ──

    #[test]
    fn stream_death_arms_a_retry_but_a_permanent_error_does_not() {
        let died = crate::error::Error::StreamDied {
            cause: "error decoding response body: buffer error while streaming".into(),
            attempts: 3,
            elapsed_secs: 0,
        };
        assert!(
            should_arm_transient_retry(&died),
            "a spent in-stream ladder is a cadence-level flake — retry it"
        );
        let permanent = crate::error::Error::Provider {
            status: 400,
            body: "bad request".into(),
            retry_after: None,
        };
        assert!(
            !should_arm_transient_retry(&permanent),
            "a 400 is the model's answer, not a flake — do not invite it again"
        );
    }

    // ── iter-720: the durable delivery ledger — run-path tests ──

    /// Records only the cron-delivery observer events, for asserting
    /// the lifecycle points fire (queued / reclaimed / parked).
    #[derive(Default)]
    struct DeliveryRecorder {
        events: parking_lot::Mutex<Vec<String>>,
    }

    impl crate::observer::Observer for DeliveryRecorder {
        fn record_event(&self, event: &crate::observer::ObserverEvent) {
            use crate::observer::ObserverEvent as E;
            let name = match event {
                E::CronDeliveryQueued { delivery_id, .. } => {
                    format!("queued:{delivery_id}")
                }
                E::CronDeliveryReclaimed {
                    delivery_id,
                    attempt,
                    ..
                } => format!("reclaimed:{delivery_id}:{attempt}"),
                E::CronDeliveryParked {
                    delivery_id,
                    attempts,
                    ..
                } => format!("parked:{delivery_id}:{attempts}"),
                _ => return,
            };
            self.events.lock().push(name);
        }
        fn record_metric(&self, _: &crate::observer::ObserverMetric) {}
        fn name(&self) -> &str {
            "delivery-recorder"
        }
    }

    /// A real scheduler over a real (uninvoked) agent with a delivery
    /// channel mounted — `scheduler_with_budgets`' construction minus
    /// the budget stores. Returns the cron_db Arc too so tests can
    /// inspect the ledger directly.
    fn scheduler_with_channel(
        dir: &std::path::Path,
        observer: Option<std::sync::Arc<dyn crate::observer::Observer>>,
    ) -> (
        super::CronScheduler,
        tokio::sync::mpsc::UnboundedReceiver<super::CronDelivery>,
        std::sync::Arc<crate::cronjobs::db::CronDb>,
    ) {
        let db = std::sync::Arc::new(
            crate::database::Database::init(dir.join("agent.sqlite")).expect("db"),
        );
        let agent = std::sync::Arc::new(crate::agent::OperantAgent::new(
            crate::agent::AgentConfig::default(),
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            crate::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
            db,
        ));
        let cron_db = std::sync::Arc::new(
            crate::cronjobs::db::CronDb::init(dir.join("cron.sqlite")).expect("cron db"),
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<super::CronDelivery>();
        let mut scheduler =
            super::CronScheduler::new(std::sync::Arc::clone(&cron_db), agent).with_delivery(tx);
        if let Some(observer) = observer {
            scheduler = scheduler.with_observer(observer);
        }
        (scheduler, rx, cron_db)
    }

    #[tokio::test]
    async fn deliver_result_queues_a_ledger_row_and_hands_the_id_off() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (scheduler, mut rx, cron_db) = scheduler_with_channel(dir.path(), None);

        let mut job = test_job();
        job.origin_platform = Some("telegram".to_string());
        job.origin_chat_id = Some("5297486612".to_string());
        scheduler
            .deliver_result(&job, "the run output")
            .await
            .unwrap();

        // The consumer sees the rendered payload AND the ledger id it
        // must settle.
        let msg = rx.try_recv().expect("handoff reached the channel");
        let id = msg.delivery_id.expect("the handoff carries the ledger id");
        assert_eq!(
            msg.job_id, "job_budget_probe",
            "the handoff carries the job id so the consumer can attribute outcomes"
        );
        assert!(
            msg.content.contains("📋 **Cron: budget probe**"),
            "the header still rides the payload: {}",
            msg.content
        );
        assert!(msg.content.contains("the run output"));

        // The ledger row exists, pending, with exactly one attempt (the
        // handoff) and the same rendered payload.
        let rows = cron_db
            .reclaim_stale_deliveries(chrono::Duration::seconds(0))
            .expect("reclaim scan");
        assert_eq!(rows.len(), 1, "one queued row");
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].attempts, 1, "the handoff is attempt one");
        assert_eq!(rows[0].target, "telegram:5297486612");

        // A job with no origin fields delivers nothing and queues nothing
        // — the R39 arm writes no fiction to the ledger.
        scheduler
            .deliver_result(&test_job(), "never arrives")
            .await
            .unwrap();
        assert!(rx.try_recv().is_err(), "no second handoff happened");
        let rows = cron_db
            .reclaim_stale_deliveries(chrono::Duration::seconds(0))
            .expect("reclaim scan");
        assert_eq!(rows.len(), 1, "the no-target job queued no row");
    }

    #[tokio::test]
    async fn replay_resends_stale_rows_then_parks_a_dead_consumer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let recorder = std::sync::Arc::new(DeliveryRecorder::default());
        let (scheduler, mut rx, cron_db) = scheduler_with_channel(
            dir.path(),
            Some(std::sync::Arc::clone(&recorder) as std::sync::Arc<dyn crate::observer::Observer>),
        );

        // Queue a row that "went stale" immediately (horizon 0 in tests).
        let id = cron_db
            .record_cron_delivery("job_replay", "telegram:9", "raw payload")
            .expect("record");

        // Live consumer: the stale row replays verbatim, the observer
        // sees the reclaim, the attempt walks.
        scheduler.replay_stale_deliveries(0).await;
        let msg = rx.try_recv().expect("replay re-sent the row");
        assert_eq!(msg.delivery_id, Some(id));
        assert_eq!(
            msg.job_id, "job_replay",
            "the replay handoff carries the row's job id"
        );
        assert_eq!(msg.content, "raw payload", "replay is verbatim");
        assert_eq!(
            *recorder.events.lock(),
            vec![format!("reclaimed:{id}:1")],
            "the reclaim lifecycle point fired"
        );

        // Dead consumer: drop rx so tx.send fails — two more reclaims
        // walk the row to the tombstone.
        drop(rx);
        scheduler.replay_stale_deliveries(0).await; // attempt 2, still pending
        scheduler.replay_stale_deliveries(0).await; // attempt 3 -> parked

        let parked = cron_db
            .reclaim_stale_deliveries(chrono::Duration::seconds(0))
            .expect("reclaim scan");
        assert!(parked.is_empty(), "a parked row never replays again");
        assert_eq!(
            *recorder.events.lock(),
            vec![format!("reclaimed:{id}:1"), format!("parked:{id}:3")],
            "the park tombstone fired the parked lifecycle point"
        );
    }

    #[tokio::test]
    async fn replay_without_a_consumer_leaves_rows_pending() {
        // The outage-queues-instead-of-drops guarantee: no channel
        // mounted, the row survives every tick untouched.
        let dir = tempfile::tempdir().expect("tempdir");
        let db = std::sync::Arc::new(
            crate::database::Database::init(dir.path().join("agent.sqlite")).expect("db"),
        );
        let agent = std::sync::Arc::new(crate::agent::OperantAgent::new(
            crate::agent::AgentConfig::default(),
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            crate::tools::ToolRegistry::new(std::time::Duration::from_secs(1)),
            db,
        ));
        let cron_db = std::sync::Arc::new(
            crate::cronjobs::db::CronDb::init(dir.path().join("cron.sqlite")).expect("cron db"),
        );
        let scheduler = super::CronScheduler::new(std::sync::Arc::clone(&cron_db), agent);

        cron_db
            .record_cron_delivery("job_q", "telegram:1", "payload")
            .expect("record");
        scheduler.replay_stale_deliveries(0).await;

        let rows = cron_db
            .reclaim_stale_deliveries(chrono::Duration::seconds(0))
            .expect("reclaim scan");
        assert_eq!(
            rows.len(),
            1,
            "the row stays queued for the next armed tick"
        );
        assert_eq!(rows[0].attempts, 0, "no handoff was counted");
    }
}

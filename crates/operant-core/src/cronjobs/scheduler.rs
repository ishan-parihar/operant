use cron::Schedule;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info};

use crate::agent::OperantAgent;
use crate::cronjobs::db::{CronDb, CronJob};
use crate::error::Error;

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
}

impl CronScheduler {
    pub fn new(db: Arc<CronDb>, agent: Arc<OperantAgent>) -> Self {
        Self {
            db,
            agent,
            delivery_tx: None,
        }
    }

    pub fn with_delivery(mut self, tx: mpsc::UnboundedSender<CronDelivery>) -> Self {
        self.delivery_tx = Some(tx);
        self
    }

    pub async fn start(&self) {
        info!("Cron scheduler started. Ticking every 60 seconds.");
        loop {
            if let Err(e) = self.tick().await {
                error!("Cron tick failed: {}", e);
            }
            sleep(Duration::from_secs(60)).await;
        }
    }

    pub async fn tick(&self) -> Result<(), Error> {
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

    async fn run_job(&self, job: &CronJob) -> Result<(), Error> {
        info!("Executing cron job {}: {}", job.id, job.name);

        let (success, _output, final_response, error_msg) = if job.no_agent {
            self.run_script_job(job).await
        } else {
            self.run_agent_job(job).await
        };

        // Repeat-limit enforcement (hermes parity — hermes cron/jobs.py marks a
        // finite-repeat job as terminal when completed >= times). Previously
        // `repeat_completed` was incremented forever and never checked, so a
        // job configured with repeat_times = N ran indefinitely.
        // `mark_job_run` bumps repeat_completed by 1, so this run's new count
        // is job.repeat_completed + 1.
        let repeat_exhausted = repeat_limit_reached(job.repeat_times, job.repeat_completed);

        if repeat_exhausted {
            // Terminal completion: retain the record (so last_status / last_error
            // stay inspectable) but disable it and clear next_run_at — mirroring
            // hermes's terminal-completion shape.
            self.db.update_job(
                &job.id,
                HashMap::from([
                    ("enabled".to_string(), Some(serde_json::json!(false))),
                    ("state".to_string(), Some(serde_json::json!("completed"))),
                    ("next_run_at".to_string(), None),
                ]),
            )?;
        } else {
            self.db.mark_job_run(
                &job.id,
                success,
                error_msg,
                None,
                self.compute_next_run(job),
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
    async fn run_script_job(&self, job: &CronJob) -> (bool, String, String, Option<String>) {
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
                )
            }
            Err(e) => {
                let err_msg = format!("Failed to execute script: {}", e);
                (false, String::new(), err_msg.clone(), Some(err_msg))
            }
        }
    }

    async fn run_agent_job(&self, job: &CronJob) -> (bool, String, String, Option<String>) {
        debug!("Running agent job {}: {}", job.id, job.name);

        self.agent.clear_history().await;
        match self.agent.run(job.prompt.clone()).await {
            Ok(message) => (true, "Agent run completed".into(), message.content, None),
            Err(e) => {
                let err_msg = format!("Agent run failed: {}", e);
                (false, String::new(), err_msg.clone(), Some(err_msg))
            }
        }
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
            debug!(
                "No delivery target for job {} (deliver={})",
                job.id, job.deliver
            );
        }
        Ok(())
    }

    fn compute_next_run(&self, job: &CronJob) -> Option<String> {
        let schedule = Schedule::from_str(&job.schedule).ok()?;
        let next = schedule.upcoming(chrono::Utc).next()?;
        Some(next.to_rfc3339())
    }
}

/// Whether a job's finite repeat limit is reached after its next run.
///
/// `repeat_times` semantics match hermes: `None` or `<= 0` means infinite.
/// `repeat_completed` is the count BEFORE this run; the run itself pushes it
/// to `repeat_completed + 1`.
fn repeat_limit_reached(repeat_times: Option<i32>, repeat_completed: i32) -> bool {
    match repeat_times {
        Some(times) if times > 0 => repeat_completed + 1 >= times,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::repeat_limit_reached;

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
}

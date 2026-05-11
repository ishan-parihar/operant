use std::path::PathBuf;
use std::sync::Arc;
use std::str::FromStr;
use tokio::time::{sleep, Duration};
use tokio::process::Command;
use tracing::{debug, error, info};
use cron::Schedule;

use crate::cronjobs::db::{CronDb, CronJob};
use crate::agent::HermesAgent;
use crate::error::Error;

pub struct CronScheduler {
    db: Arc<CronDb>,
    agent_builder: Arc<dyn Fn() -> HermesAgent + Send + Sync>,
}

impl CronScheduler {
    pub fn new(db: Arc<CronDb>, agent_builder: Arc<dyn Fn() -> HermesAgent + Send + Sync>) -> Self {
        Self { db, agent_builder }
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

        let (success, output, final_response, error_msg) = if job.no_agent {
            self.run_script_job(job).await
        } else {
            self.run_agent_job(job).await
        };;

        self.db.mark_job_run(
            &job.id,
            success,
            error_msg,
            None,
            self.compute_next_run(&job),
        )?;

        if success && final_response != "[SILENT]" {
            self.deliver_result(&job, &final_response).await?;
        }

        Ok(())
    }

    async fn run_script_job(&self, job: &CronJob) -> (bool, String, String, Option<String>) {
        debug!("Running script job {}: {}", job.id, job.name);
        
        let script = job.script.as_ref().ok_or_else(|| {
            error!("Cron job {} has no script defined", job.id);
            "No script defined"
        });

        if script.is_err() {
            return (false, String::new(), "No script defined".into(), Some("No script defined".into()));
        }

        let output = Command::new("sh")
            .arg("-c")
            .arg(script.unwrap())
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

                (success, stdout, final_res, if success { None } else { Some(stderr) })
            }
            Err(e) => {
                let err_msg = format!("Failed to execute script: {}", e);
                (false, String::new(), err_msg.clone(), Some(err_msg))
            }
        }
    }

    async fn run_agent_job(&self, job: &CronJob) -> (bool, String, String, Option<String>) {
        debug!("Running agent job {}: {}", job.id, job.name);
        
        let agent = (self.agent_builder)();
        match agent.run(job.prompt.clone()).await {
            Ok(message) => {
                (true, "Agent run completed".into(), message.content, None)
            }
            Err(e) => {
                let err_msg = format!("Agent run failed: {}", e);
                (false, String::new(), err_msg.clone(), Some(err_msg))
            }
        }
    }

    async fn deliver_result(&self, job: &CronJob, content: &str) -> Result<(), Error> {
        info!("Delivering result for job {}: {}", job.id, job.name);
        debug!("Result: {}", content);
        Ok(())
    }

    fn compute_next_run(&self, job: &CronJob) -> Option<String> {
        let schedule = Schedule::from_str(&job.schedule).ok()?;
        let next = schedule.upcoming(chrono::Utc).next()?;
        Some(next.to_rfc3339())
    }
}

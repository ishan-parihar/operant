//! Cron Tool - Scheduled Task Management
//!
//! Provides an interface for creating, listing, and managing scheduled jobs.
//! Integration with the CronScheduler for execution.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::cronjobs::db::CronDb;
use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum CronAction {
    Create,
    List,
    Get,
    Update,
    Delete,
    Pause,
    Resume,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CronToolArgs {
    action: CronAction,
    job_id: Option<String>,
    name: Option<String>,
    schedule: Option<String>,
    schedule_display: Option<String>,
    prompt: Option<String>,
    deliver: Option<String>,
    script: Option<String>,
    no_agent: Option<bool>,
    workdir: Option<String>,
    updates: Option<Value>,
}

pub struct CronTool {
    db: Arc<CronDb>,
}

impl CronTool {
    pub fn new(db: Arc<CronDb>) -> Self {
        Self { db }
    }

    fn handle_create(&self, args: &CronToolArgs) -> Result<ToolResult> {
        let name = args.name.as_ref().ok_or_else(|| Error::InvalidToolArgs {
            name: "cron".to_string(),
            details: "Missing 'name' for job creation".to_string(),
        })?;
        let schedule = args
            .schedule
            .as_ref()
            .ok_or_else(|| Error::InvalidToolArgs {
                name: "cron".to_string(),
                details: "Missing 'schedule' for job creation".to_string(),
            })?;
        let display = args
            .schedule_display
            .as_ref()
            .ok_or_else(|| Error::InvalidToolArgs {
                name: "cron".to_string(),
                details: "Missing 'schedule_display' for job creation".to_string(),
            })?;
        let prompt = args.prompt.as_ref().ok_or_else(|| Error::InvalidToolArgs {
            name: "cron".to_string(),
            details: "Missing 'prompt' for job creation".to_string(),
        })?;

        let id = self.db.create_job(
            name.clone(),
            prompt.clone(),
            schedule.clone(),
            display.clone(),
            None,
            args.deliver.clone().unwrap_or_else(|| "local".to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            args.script.clone(),
            None,
            None,
            args.workdir.clone(),
            args.no_agent.unwrap_or(false),
        )?;

        // Compute and set the first next_run_at
        let next_run = compute_first_run(schedule);
        if let Some(ref next) = next_run {
            self.db.set_next_run(&id, Some(next.clone()))?;
        }

        Ok(ToolResult::success(
            "cron_create",
            json!({ "success": true, "job_id": id, "next_run_at": next_run, "message": format!("Job '{}' created successfully", name) }),
        ))
    }

    fn handle_list(&self) -> Result<ToolResult> {
        let jobs = self.db.list_jobs(true)?;
        Ok(ToolResult::success(
            "cron_list",
            json!({ "success": true, "jobs": jobs, "count": jobs.len() }),
        ))
    }

    fn handle_get(&self, job_id: &str) -> Result<ToolResult> {
        let job = self.db.get_job(job_id)?;
        if let Some(job) = job {
            Ok(ToolResult::success(
                "cron_get",
                json!({ "success": true, "job": job }),
            ))
        } else {
            Ok(ToolResult::error(
                "cron_get",
                format!("Job {} not found", job_id),
            ))
        }
    }

    fn handle_update(&self, job_id: &str, updates: &Value) -> Result<ToolResult> {
        let mut update_map = std::collections::HashMap::new();
        if let Some(obj) = updates.as_object() {
            for (k, v) in obj {
                update_map.insert(k.clone(), Some(v.clone()));
            }
        }

        let updated_job = self.db.update_job(job_id, update_map)?;
        if let Some(job) = updated_job {
            Ok(ToolResult::success(
                "cron_update",
                json!({ "success": true, "job": job, "message": "Job updated successfully" }),
            ))
        } else {
            Ok(ToolResult::error(
                "cron_update",
                format!("Job {} not found", job_id),
            ))
        }
    }

    fn handle_delete(&self, job_id: &str) -> Result<ToolResult> {
        let deleted = self.db.delete_job(job_id)?;
        if deleted {
            Ok(ToolResult::success(
                "cron_delete",
                json!({ "success": true, "message": format!("Job {} deleted", job_id) }),
            ))
        } else {
            Ok(ToolResult::error(
                "cron_delete",
                format!("Job {} not found", job_id),
            ))
        }
    }

    fn handle_pause(&self, job_id: &str) -> Result<ToolResult> {
        let mut updates = std::collections::HashMap::new();
        updates.insert("enabled".to_string(), Some(json!(false)));
        updates.insert("state".to_string(), Some(json!("paused")));
        updates.insert(
            "paused_at".to_string(),
            Some(json!(chrono::Utc::now().to_rfc3339())),
        );

        let updated_job = self.db.update_job(job_id, updates)?;
        if let Some(job) = updated_job {
            Ok(ToolResult::success(
                "cron_pause",
                json!({ "success": true, "job": job, "message": "Job paused successfully" }),
            ))
        } else {
            Ok(ToolResult::error(
                "cron_pause",
                format!("Job {} not found", job_id),
            ))
        }
    }

    fn handle_resume(&self, job_id: &str) -> Result<ToolResult> {
        let mut updates = std::collections::HashMap::new();
        updates.insert("enabled".to_string(), Some(json!(true)));
        updates.insert("state".to_string(), Some(json!("scheduled")));
        updates.insert("paused_at".to_string(), None);

        let updated_job = self.db.update_job(job_id, updates)?;
        if let Some(job) = updated_job {
            Ok(ToolResult::success(
                "cron_resume",
                json!({ "success": true, "job": job, "message": "Job resumed successfully" }),
            ))
        } else {
            Ok(ToolResult::error(
                "cron_resume",
                format!("Job {} not found", job_id),
            ))
        }
    }
}

#[async_trait]
impl OperantTool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Manage scheduled tasks (cron jobs). \
         Use 'create' to schedule a new task, 'list' to see all jobs, \
         'get' for details, 'update' to modify a job, or 'delete' to remove one. \
         Supports both agent-based prompts and raw shell scripts."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CronToolArgs>(
            "cron",
            "Manage scheduled tasks: create, list, get, update, delete, pause, resume",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: CronToolArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("cron", format!("Invalid arguments: {}", e)),
        };

        match args.action {
            CronAction::Create => match self.handle_create(&args) {
                Ok(res) => res,
                Err(e) => ToolResult::error("cron_create", e.to_string()),
            },
            CronAction::List => match self.handle_list() {
                Ok(res) => res,
                Err(e) => ToolResult::error("cron_list", e.to_string()),
            },
            CronAction::Get => {
                let id = match args.job_id {
                    Some(id) => id,
                    None => return ToolResult::error("cron_get", "Missing 'jobId'".to_string()),
                };
                match self.handle_get(&id) {
                    Ok(res) => res,
                    Err(e) => ToolResult::error("cron_get", e.to_string()),
                }
            }
            CronAction::Update => {
                let id = match args.job_id {
                    Some(id) => id,
                    None => return ToolResult::error("cron_update", "Missing 'jobId'".to_string()),
                };
                let updates = args.updates.clone().unwrap_or(json!({}));
                match self.handle_update(&id, &updates) {
                    Ok(res) => res,
                    Err(e) => ToolResult::error("cron_update", e.to_string()),
                }
            }
            CronAction::Delete => {
                let id = match args.job_id {
                    Some(id) => id,
                    None => return ToolResult::error("cron_delete", "Missing 'jobId'".to_string()),
                };
                match self.handle_delete(&id) {
                    Ok(res) => res,
                    Err(e) => ToolResult::error("cron_delete", e.to_string()),
                }
            }
            CronAction::Pause => {
                let id = match args.job_id {
                    Some(id) => id,
                    None => return ToolResult::error("cron_pause", "Missing 'jobId'".to_string()),
                };
                match self.handle_pause(&id) {
                    Ok(res) => res,
                    Err(e) => ToolResult::error("cron_pause", e.to_string()),
                }
            }
            CronAction::Resume => {
                let id = match args.job_id {
                    Some(id) => id,
                    None => return ToolResult::error("cron_resume", "Missing 'jobId'".to_string()),
                };
                match self.handle_resume(&id) {
                    Ok(res) => res,
                    Err(e) => ToolResult::error("cron_resume", e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_schema() {
        let schema = ToolSchema::from_type::<CronToolArgs>("cron", "test");
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "cron");
    }
}

/// Compute the first next_run_at for a schedule string.
/// Supports cron expressions (5-field) and interval shorthand (e.g. "every 30m").
fn compute_first_run(schedule: &str) -> Option<String> {
    use chrono::Utc;
    // Try cron expression first
    if let Ok(sched) = cron::Schedule::from_str(schedule) {
        return sched.upcoming(Utc).next().map(|t| t.to_rfc3339());
    }
    // Try interval shorthand: "every 30m", "every 2h", "30m", "2h", "1d"
    let s = schedule.trim().trim_start_matches("every").trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let num: u64 = num.trim().parse().ok()?;
    let secs = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => return None,
    };
    let next = Utc::now() + chrono::Duration::seconds(secs as i64);
    Some(next.to_rfc3339())
}

use std::str::FromStr;

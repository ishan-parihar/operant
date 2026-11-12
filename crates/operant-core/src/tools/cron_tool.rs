//! Cron Tool - Scheduled Task Management
//!
//! Provides an interface for creating, listing, and managing scheduled jobs.
//! Integration with the CronScheduler for execution.
//!
//! Schedule semantics (shared with `cronjobs::schedule`): cron expressions
//! are calendar rules evaluated in **UTC** — `0 9 * * *` fires at 09:00 UTC
//! — while `every N<unit>` phrases are fixed durations.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::cronjobs::db::CronDb;
use crate::error::{Error, Result};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Delivery origin captured from the live execution surface (gateway turn,
/// ACP session, …) so jobs the model creates can deliver their results back
/// to the requesting chat. The gateway sets this before routing a turn and
/// clears it after; prompts only ever see PII-redacted channel ids, so the
/// model itself CANNOT supply a usable delivery target (R39).
/// Where a cron job was created from (RT-534).
///
/// It rides the turn: the agent carries it and hands it to the tool through
/// the [`crate::tools::ToolContext`], so a concurrent turn in another agent
/// slot cannot overwrite it between the turn's start and the tool call.
#[derive(Debug, Clone)]
pub struct CronOrigin {
    pub platform: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
}

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

    fn handle_create(&self, args: &CronToolArgs, origin: Option<CronOrigin>) -> Result<ToolResult> {
        // Phase G (docs/agentic-os/): inside a Holon run recurring work is
        // the organization's — the agent proposes a routine (the kernel's
        // classifier + the Chair approve), never a local cron. The gate is
        // the run env, not config: a Holon body proposes; a personal session
        // on the same box keeps its own crons.
        if crate::platform::is_holon_run() {
            let name = args.name.clone().unwrap_or_default();
            let schedule = args.schedule.clone().unwrap_or_default();
            let prompt = args.prompt.clone().unwrap_or_default();
            return Ok(ToolResult::success(
                "cron",
                // O-23: the kernel's real argv — a positional title, --intent
                // and --dept required, --cron the schedule — and an AGENT
                // reports it in its result's `proposals`, because routine
                // add is Chair-only.
                format!(
                    "This is a Holon run: recurring work belongs to the organization. \
                     Report this as a proposal in your result's `proposals`: \
                     routine add {name:?} --intent {prompt:?} --dept <slug> --cron {schedule:?} \
                     (the Chair approves routines; an agent cannot add one)."
                ),
            ));
        }
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
        // The display is DERIVED from the schedule when the caller did not
        // supply one: the schema marks `scheduleDisplay` optional, and the
        // sibling CLI path derives it the same way (`schedule.to_string()`).
        // Requiring it rejected schema-compliant calls (RT-298).
        let display = args
            .schedule_display
            .clone()
            .unwrap_or_else(|| schedule.to_string());
        let prompt = args.prompt.as_ref().ok_or_else(|| Error::InvalidToolArgs {
            name: "cron".to_string(),
            details: "Missing 'prompt' for job creation".to_string(),
        })?;

        // Where this job was created from, so the scheduler can deliver its
        // result back to the requesting chat (R39). Prompts only carry
        // redacted channel ids, so `args.deliver` alone is unusable — and
        // the origin comes from the TURN (RT-534), so a concurrent chat's
        // turn cannot move it.
        let id = self.db.create_job(crate::cronjobs::db::CreateJobParams {
            name: name.clone(),
            prompt: prompt.clone(),
            schedule: schedule.clone(),
            schedule_display: display.clone(),
            repeat_times: None,
            deliver: args.deliver.clone().unwrap_or_else(|| "local".to_string()),
            origin_platform: origin.as_ref().map(|o| o.platform.clone()),
            origin_chat_id: origin.as_ref().map(|o| o.chat_id.clone()),
            origin_thread_id: origin.as_ref().and_then(|o| o.thread_id.clone()),
            skill: None,
            skills: None,
            model: None,
            provider: None,
            base_url: None,
            script: args.script.clone(),
            context_from: None,
            enabled_toolsets: None,
            workdir: args.workdir.clone(),
            no_agent: args.no_agent.unwrap_or(false),
        })?;

        // Compute and set the first next_run_at -- with the SHARED rule
        // (the same one the database and the scheduler use). The tool's
        // private copy parsed only what the cron crate accepts, so the
        // 5-field form its own doc advertises ("0 9 * * *") came back as
        // `next_run_at: null` for a job the database had already scheduled
        // (RT-298).
        let next_run = crate::cronjobs::schedule::next_run_from_schedule(schedule);
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

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: CronToolArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("cron", format!("Invalid arguments: {}", e)),
        };

        match args.action {
            CronAction::Create => match self.handle_create(&args, context.cron_origin.clone()) {
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

    /// RT-298: the cron tool must accept a SCHEMA-COMPLIANT create call.
    ///
    /// The schema marks `scheduleDisplay` optional, but the body demanded
    /// it ("Missing 'schedule_display' for job creation"), so a model that
    /// followed the schema could not create a job. And the tool's private
    /// copy of the schedule rule did not accept the 5-field form its own
    /// doc advertises ("0 9 * * *"), so the tool answered
    /// `next_run_at: null` for a job the database had already scheduled.
    #[tokio::test]
    async fn cron_create_accepts_a_five_field_schedule_and_reports_the_next_run() {
        let dir = std::env::temp_dir().join(format!(
            "operant_cron_tool_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db = std::sync::Arc::new(
            crate::cronjobs::db::CronDb::init(dir.join("cron.db")).expect("cron db"),
        );
        let tool = CronTool::new(db.clone());
        let result = tool
            .execute(
                json!({
                    "action": "create",
                    "name": "daily-nine",
                    "schedule": "0 9 * * *",
                    "prompt": "say good morning"
                }),
                crate::tools::ToolContext::default(),
            )
            .await;
        assert!(
            result.success,
            "a schema-compliant call must succeed: {}",
            result.content
        );
        let payload: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert!(
            payload["next_run_at"].is_string(),
            "the next run must be reported: {payload}"
        );
        let job_id = payload["job_id"].as_str().expect("the job id");
        let job = db.get_job(job_id).expect("read").expect("the job");
        assert_eq!(
            job.schedule_display, "0 9 * * *",
            "the display is derived from the schedule"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// RT-534: the cron job's delivery origin comes from the TURN, not a
    /// process global.
    ///
    /// The origin used to live in a process-global cell that the gateway set
    /// at the start of every turn. The gateway runs agent slots
    /// concurrently now, so a concurrent chat's turn could overwrite the
    /// origin before this chat's `cron` tool call ran — and the job would
    /// be delivered to the wrong chat. It rides the ToolContext.
    #[tokio::test]
    async fn a_cron_job_is_delivered_to_the_chat_that_created_it() {
        let dir = std::env::temp_dir().join(format!(
            "operant_cron_origin_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db = std::sync::Arc::new(
            crate::cronjobs::db::CronDb::init(dir.join("cron.db")).expect("cron db"),
        );
        let tool = CronTool::new(db.clone());

        // Chat A creates a job inside a turn whose context carries A.
        let context_a = crate::tools::ToolContext {
            cron_origin: Some(CronOrigin {
                platform: "telegram".to_string(),
                chat_id: "chat-a".to_string(),
                thread_id: None,
            }),
            ..Default::default()
        };
        let result = tool
            .execute(
                json!({
                    "action": "create",
                    "name": "morning-a",
                    "schedule": "0 9 * * *",
                    "prompt": "say good morning"
                }),
                context_a,
            )
            .await;
        assert!(
            result.success,
            "the create must succeed: {}",
            result.content
        );
        let payload: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        let job = db
            .get_job(payload["job_id"].as_str().expect("the job id"))
            .expect("read")
            .expect("the job");
        assert_eq!(job.origin_chat_id.as_deref(), Some("chat-a"));

        // A SECOND slot's turn (chat B) must not move chat A's job.
        let context_b = crate::tools::ToolContext {
            cron_origin: Some(CronOrigin {
                platform: "telegram".to_string(),
                chat_id: "chat-b".to_string(),
                thread_id: None,
            }),
            ..Default::default()
        };
        let result = tool
            .execute(
                json!({
                    "action": "create",
                    "name": "morning-b",
                    "schedule": "0 9 * * *",
                    "prompt": "say good morning"
                }),
                context_b,
            )
            .await;
        let payload: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        let job_b = db
            .get_job(payload["job_id"].as_str().expect("the job id"))
            .expect("read")
            .expect("the job");
        assert_eq!(
            job_b.origin_chat_id.as_deref(),
            Some("chat-b"),
            "chat B's job must be delivered to chat B"
        );
        let still_a = db
            .list_jobs(false)
            .expect("list")
            .into_iter()
            .find(|j| j.name == "morning-a")
            .expect("chat A's job");
        assert_eq!(
            still_a.origin_chat_id.as_deref(),
            Some("chat-a"),
            "chat A's job must keep A's origin after B's turn ran"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cron_schema() {
        let schema = ToolSchema::from_type::<CronToolArgs>("cron", "test");
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "cron");
    }
}

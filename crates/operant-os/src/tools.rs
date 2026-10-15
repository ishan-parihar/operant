//! Native Holon tools (boundary doc §2: replaces one resident Python MCP
//! server per run).
//!
//! In a Holon run the agent gets these instead of a spawned `holon mcp`:
//! reads go through the read-only API client (invariant 1 — no writable
//! handle exists), and the organization-facing writes a body legitimately
//! makes (memory, messages, its own run's heartbeat/claim, submit) go
//! through the kernel CLI with the run token the kernel exported. The
//! token authorizes exactly the agent-command set (`AGENT_COMMANDS` in
//! holon/mcp.py); the kernel enforces the rest.
//!
//! Every tool is inert outside a Holon run (`is_available` requires
//! HOLON_HOME) — invariant 5.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{Value, json};

use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolResult};

use crate::api::HolonApi;

/// Shared plumbing: where the instance is, and how to call its CLI.
struct HolonEnv {
    home: PathBuf,
    bin: String,
}

fn holon_env() -> Option<HolonEnv> {
    let home = std::env::var_os("HOLON_HOME")?;
    let bin = std::env::var("HOLON_BIN").unwrap_or_else(|_| "holon".to_string());
    Some(HolonEnv {
        home: home.into(),
        bin,
    })
}

fn run_cli(env: &HolonEnv, args: &[&str]) -> ToolResult {
    let out = std::process::Command::new(&env.bin)
        .args(["--home", &env.home.to_string_lossy()])
        .args(args)
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            return ToolResult::error("holon-cli", format!("holon cli: {e}"));
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return ToolResult::error(
            "holon-cli",
            format!(
                "holon: {}",
                stderr
                    .is_empty()
                    .then(|| stdout.clone())
                    .unwrap_or_else(|| stderr.clone())
            ),
        );
    }
    ToolResult::success("holon-cli", stdout)
}

// ── read tools (read-only API) ─────────────────────────────────────────────

/// `holon_whoami`: the acting employee's identity card (who they are, their
/// department, their roles — the kernel's `emp card` shape).
pub struct Whoami;

#[async_trait]
impl OperantTool for Whoami {
    fn name(&self) -> &str {
        "holon_whoami"
    }
    fn description(&self) -> &str {
        "Your identity card in the organization: employee id, title, department, roles. You are this employee for the whole run."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {}}),
        )
    }
    fn is_available(&self) -> bool {
        std::env::var_os("HOLON_EMPLOYEE_ID").is_some()
    }
    async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let emp = std::env::var("HOLON_EMPLOYEE_ID").unwrap_or_default();
        run_cli(&env, &["emp", "card", &emp])
    }
}

/// `holon_org`: departments, heads, and the org chart (read API).
pub struct OrgChart;

#[async_trait]
impl OperantTool for OrgChart {
    fn name(&self) -> &str {
        "holon_org"
    }
    fn description(&self) -> &str {
        "The organization's structure: departments with their missions and heads, and the employee roster with roles."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {}}),
        )
    }
    fn is_available(&self) -> bool {
        holon_env().is_some()
    }
    async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let api = match HolonApi::open(&env.home) {
            Ok(api) => api,
            Err(e) => return ToolResult::error(self.name(), format!("read API: {e}")),
        };
        match (api.departments(), api.employees()) {
            (Ok(depts), Ok(emps)) => {
                let mut out = String::from("Departments:\n");
                for d in &depts {
                    let head = d.head.as_deref().unwrap_or("-");
                    out.push_str(&format!("  {} — {} (head {})\n", d.slug, d.name, head));
                }
                out.push_str("Employees:\n");
                for e in &emps {
                    out.push_str(&format!(
                        "  {} — {} · {} · {}\n",
                        e.id, e.name, e.title, e.department
                    ));
                }
                ToolResult::success(self.name(), out)
            }
            (Err(e), _) | (_, Err(e)) => ToolResult::error(self.name(), format!("read API: {e}")),
        }
    }
}

/// `holon_board`: the board — ready and running stages (read API).
pub struct Board;

#[async_trait]
impl OperantTool for Board {
    fn name(&self) -> &str {
        "holon_board"
    }
    fn description(&self) -> &str {
        "The work board: stages ready to claim and stages running now, with their titles and departments."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {}}),
        )
    }
    fn is_available(&self) -> bool {
        holon_env().is_some()
    }
    async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let api = match HolonApi::open(&env.home) {
            Ok(api) => api,
            Err(e) => return ToolResult::error(self.name(), format!("read API: {e}")),
        };
        match api.board() {
            Ok(rows) => {
                if rows.is_empty() {
                    return ToolResult::success(self.name(), "The board is empty.");
                }
                let mut out = String::new();
                for b in &rows {
                    out.push_str(&format!(
                        "  {} — {} · {} · {}\n",
                        b.stage,
                        b.title,
                        b.department.as_deref().unwrap_or("-"),
                        b.status
                    ));
                }
                ToolResult::success(self.name(), out)
            }
            Err(e) => ToolResult::error(self.name(), format!("read API: {e}")),
        }
    }
}

// ── organization-facing writes (kernel CLI, run token) ─────────────────────

/// `holon_memory_add`: store a memory the organization keeps about its work.
pub struct MemoryAdd;

#[async_trait]
impl OperantTool for MemoryAdd {
    fn name(&self) -> &str {
        "holon_memory_add"
    }
    fn description(&self) -> &str {
        "Store a memory in the organization's memory: lessons, terrain notes, decisions worth keeping. Use for what the org should remember beyond this run."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {"text": {"type": "string", "description": "the memory to store"}}, "required": ["text"]}),
        )
    }
    fn is_available(&self) -> bool {
        std::env::var_os("HOLON_RUN_TOKEN").is_some()
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let Some(text) = args.get("text").and_then(Value::as_str) else {
            return ToolResult::error(self.name(), "text is required");
        };
        run_cli(&env, &["memory", "add", text])
    }
}

/// `holon_memory_search`: recall the organization's memories.
pub struct MemorySearch;

#[async_trait]
impl OperantTool for MemorySearch {
    fn name(&self) -> &str {
        "holon_memory_search"
    }
    fn description(&self) -> &str {
        "Search the organization's memory: lessons and terrain notes from past work."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}, "required": ["query"]}),
        )
    }
    fn is_available(&self) -> bool {
        holon_env().is_some()
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return ToolResult::error(self.name(), "query is required");
        };
        run_cli(&env, &["memory", "search", query])
    }
}

/// `holon_msg_send`: message another employee (through the org's channels).
pub struct MsgSend;

#[async_trait]
impl OperantTool for MsgSend {
    fn name(&self) -> &str {
        "holon_msg_send"
    }
    fn description(&self) -> &str {
        "Send a message to another employee in the organization."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {"to": {"type": "string", "description": "employee id"}, "body": {"type": "string"}}, "required": ["to", "body"]}),
        )
    }
    fn is_available(&self) -> bool {
        std::env::var_os("HOLON_RUN_TOKEN").is_some()
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let (Some(to), Some(body)) = (
            args.get("to").and_then(Value::as_str),
            args.get("body").and_then(Value::as_str),
        ) else {
            return ToolResult::error(self.name(), "to and body are required");
        };
        run_cli(&env, &["msg", "send", to, body])
    }
}

/// `holon_submit`: hand this run's result envelope to the kernel (the
/// observe-stage path — an observer cannot write result.json but can
/// always submit).
pub struct Submit;

#[async_trait]
impl OperantTool for Submit {
    fn name(&self) -> &str {
        "holon_submit"
    }
    fn description(&self) -> &str {
        "Hand this run's result envelope to the organization. The envelope must match your stage's contract (the brief lists its fields)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            json!({"type": "object", "properties": {"result": {"type": "object", "description": "the stage's result envelope as JSON"}}, "required": ["result"]}),
        )
    }
    fn is_available(&self) -> bool {
        std::env::var_os("HOLON_RUN_TOKEN").is_some()
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let Some(env) = holon_env() else {
            return ToolResult::error(self.name(), "not a Holon run (HOLON_HOME unset)");
        };
        let run = std::env::var("HOLON_RUN_ID").unwrap_or_default();
        let Some(result) = args.get("result") else {
            return ToolResult::error(self.name(), "result is required");
        };
        let Ok(text) = serde_json::to_string(result) else {
            return ToolResult::error(self.name(), "result is not serializable");
        };
        run_cli(&env, &["--json", "submit", &run, &text])
    }
}

/// Register every native Holon tool on the registry. Called by the CLI's
/// tool build when `[holon] enabled` (and inert outside a Holon run — each
/// tool's `is_available` gates on the Holon-run env, so registration when
/// a non-Holon session shares the process costs only schema lines).
pub async fn register_all(registry: &operant_core::tools::ToolRegistry) {
    for tool in [
        Box::new(Whoami) as Box<dyn OperantTool>,
        Box::new(OrgChart),
        Box::new(Board),
        Box::new(MemoryAdd),
        Box::new(MemorySearch),
        Box::new(MsgSend),
        Box::new(Submit),
    ] {
        let name = tool.name().to_string();
        if let Err(e) = registry.register_dyn(std::sync::Arc::from(tool)).await {
            tracing::warn!(tool = %name, error = %e, "holon tool registration failed");
        }
    }
}

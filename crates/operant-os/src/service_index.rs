//! Holon's pooled services as deferred tools (H2): `svc_<unit>_<service>`.
//!
//! The v4 capability layer publishes three read views: `api_units` (with the
//! owner that keeps a unit true), `api_services` (the per-service contract),
//! and `api_pool` (unit -> dept edges; `"*"` = org-wide). This module turns
//! the rows a run is entitled to into deferred tools that feed the same
//! catalogue and `tool_search` bridge as `mcp_*` tools (operant-core
//! `tools::tool_search::is_deferrable`).
//!
//! - Entitlement comes from `api_pool` only, never from names: a service is
//!   indexed for a dept when a pool edge names its unit for that dept or for
//!   `"*"`. A unit with no row in `api_units` or no edge is not indexed.
//! - Schema: `args` (fallback `accepts`) is the call contract; `output`
//!   (fallback `returns`) is described in the tool description.
//! - Execution goes through the commands fence (`holon_command_fence`), and
//!   the argv is split without a shell, so model text cannot add a command.
//! - Posture: an observer gets read services only; a write service is never
//!   registered for an observe run, and the observe allow list names only the
//!   read services (see `PolicyDoc::seat_policy_with_services`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};

use operant_core::schema::ToolSchema;
use operant_core::tools::file_tools::holon_command_fence;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

use crate::api::{HolonApi, PoolEdge, Service, Unit};
use crate::tools::split_word_list;

/// Name prefix of deferred service tools. Reserved for this index.
pub const SVC_PREFIX: &str = "svc_";

/// One deferred service tool, fully resolved for one run.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceEntry {
    pub name: String,
    pub unit: String,
    pub service: String,
    pub owner: Option<String>,
    /// The raw `effect` column; only `"read"` is observer-visible.
    pub effect: String,
    /// The executable the call runs (first token is what the fence judges).
    pub command: String,
    pub description: String,
    pub parameters: Value,
}

impl ServiceEntry {
    pub fn is_read(&self) -> bool {
        self.effect == "read"
    }
}

/// `svc_<unit>_<service>`, with anything outside `[A-Za-z0-9]` mapped to `_`.
pub fn tool_name(unit: &str, service: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };
    format!("{SVC_PREFIX}{}_{}", clean(unit), clean(service))
}

/// Whether `dept` may use `unit`: a pool edge for the dept or for `"*"`.
pub fn entitled(pool: &[PoolEdge], unit: &str, dept: Option<&str>) -> bool {
    pool.iter()
        .any(|e| e.unit == unit && (e.dept == "*" || dept == Some(e.dept.as_str())))
}

/// The parameter schema: `{ "args": string }`, described by `args`
/// (fallback `accepts`).
pub fn parameters(service: &Service) -> Value {
    let described = match args_text(service) {
        Some(a) => format!("Call contract: {a}"),
        None => "No call contract is published; pass the arguments as one string.".to_string(),
    };
    json!({
        "type": "object",
        "properties": { "args": { "type": "string", "description": described } }
    })
}

/// `args` (fallback `accepts`): the call contract the model sees.
fn args_text(service: &Service) -> Option<&str> {
    service
        .args
        .as_deref()
        .or(service.accepts.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `output` (fallback `returns`): what the call hands back.
fn returns_text(service: &Service) -> Option<&str> {
    service
        .output
        .as_deref()
        .or(service.returns.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn describe(unit: &Unit, service: &Service) -> String {
    let mut d = format!("Pooled service `{}` of unit `{}`", service.service, unit.slug);
    if let Some(owner) = &unit.owner {
        d += &format!(" (owner {owner})");
    }
    if let Some(note) = service.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        d += &format!(". {note}");
    }
    if let Some(returns) = returns_text(service) {
        d += &format!(". Returns: {returns}");
    }
    d
}

/// Every service the dept is entitled to, in `api_services` order.
pub fn index_for(
    units: &[Unit],
    services: &[Service],
    pool: &[PoolEdge],
    dept: Option<&str>,
) -> Vec<ServiceEntry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for service in services {
        let Some(unit) = units.iter().find(|u| u.slug == service.unit) else {
            continue; // no api_units row: no owner, not a published unit
        };
        if !entitled(pool, &unit.slug, dept) {
            continue;
        }
        let Some(command) = service
            .command
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        else {
            continue; // nothing to execute
        };
        let name = tool_name(&service.unit, &service.service);
        if !seen.insert(name.clone()) {
            continue;
        }
        out.push(ServiceEntry {
            name,
            unit: service.unit.clone(),
            service: service.service.clone(),
            owner: unit.owner.clone(),
            effect: service.effect.clone().unwrap_or_default(),
            command: command.to_string(),
            description: describe(unit, service),
            parameters: parameters(service),
        });
    }
    out
}

/// The run's service index, with the posture it was built for.
#[derive(Debug, Clone)]
pub struct RunServices {
    pub posture: String,
    pub entries: Vec<ServiceEntry>,
}

impl RunServices {
    /// Names the observe allow list must carry: read services only.
    pub fn read_names(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.is_read())
            .map(|e| e.name.clone())
            .collect()
    }

    /// Register every entry as a deferred tool on the registry.
    pub async fn register(&self, registry: &ToolRegistry) {
        for entry in &self.entries {
            let name = entry.name.clone();
            let tool = ServiceTool::new(entry.clone());
            if let Err(e) = registry.register_dyn(Arc::new(tool)).await {
                tracing::warn!(tool = %name, error = %e, "holon service registration failed");
            }
        }
    }
}

/// The entries a posture may see: an act run sees all of them, an observe
/// run (anything that is not `act`, fail closed) sees the read services only.
pub fn visible_for(entries: Vec<ServiceEntry>, posture: &str) -> Vec<ServiceEntry> {
    if posture == "act" {
        entries
    } else {
        entries.into_iter().filter(ServiceEntry::is_read).collect()
    }
}

/// The Holon instance this run reads from: the configured home, else `$HOLON_HOME`.
pub fn holon_home(configured: &Path) -> Option<PathBuf> {
    if !configured.as_os_str().is_empty() {
        return Some(configured.to_path_buf());
    }
    std::env::var_os("HOLON_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The service index for this Holon run. `Ok(None)` when this is not a Holon
/// run (no policy.json): no authority, no services, today's behaviour.
pub fn for_run(holon_home: &Path) -> crate::Result<Option<RunServices>> {
    let Some(loaded) = crate::enforcement::run_policy() else {
        return Ok(None);
    };
    let policy = loaded?;
    let api = HolonApi::open(holon_home)?;
    // entitlement is the seat's department (api_employees) against api_pool
    let seat = crate::enforcement::holon_seat();
    let dept = api
        .employees()?
        .into_iter()
        .find(|e| e.id == seat)
        .map(|e| e.department);
    let entries = index_for(
        &api.units()?,
        &api.services()?,
        &api.pool()?,
        dept.as_deref(),
    );
    Ok(Some(RunServices {
        entries: visible_for(entries, &policy.posture),
        posture: policy.posture,
    }))
}

/// One deferred service: a thin `OperantTool` over one `ServiceEntry`.
pub struct ServiceTool {
    entry: ServiceEntry,
}

impl ServiceTool {
    pub fn new(entry: ServiceEntry) -> Self {
        Self { entry }
    }
}

#[async_trait]
impl OperantTool for ServiceTool {
    fn name(&self) -> &str {
        &self.entry.name
    }

    fn description(&self) -> &str {
        &self.entry.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.name(),
            self.description(),
            self.entry.parameters.clone(),
        )
    }

    fn is_available(&self) -> bool {
        operant_core::platform::is_holon_run()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = self.entry.name.clone();
        let tail = match args.get("args") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(_) => return ToolResult::error(name, "args must be a string".to_string()),
        };
        match self.run(&tail).await {
            Ok(out) => ToolResult::success(name, out),
            Err(e) => ToolResult::error(name, e),
        }
    }
}

impl ServiceTool {
    /// The commands fence judges the binary; the argv is split without a shell,
    /// so the model's text can add arguments but never a second command.
    async fn run(&self, tail: &str) -> Result<String, String> {
        holon_command_fence(&self.entry.command)?;
        let mut argv = split_word_list(&self.entry.command);
        argv.extend(split_word_list(tail));
        let Some((bin, rest)) = argv.split_first() else {
            return Err("the service has no command".to_string());
        };
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(rest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(120), cmd.output())
            .await
            .map_err(|_| "service call timed out (120s)".to_string())?
            .map_err(|e| format!("service not reachable ({e})"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() {
            return Ok(stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("service exit {}: {stdout}", out.status.code().unwrap_or(-1))
        } else {
            stderr
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PolicyDoc;
    use operant_core::org::seat_authority::SeatAuthority;
    use operant_core::org::seat_policy::{SeatDecision, SeatPolicy, SeatPolicySource};
    use std::sync::Arc;

    fn unit(slug: &str, owner: Option<&str>) -> Unit {
        Unit {
            slug: slug.into(),
            charter: "charter".into(),
            category: Some("surface".into()),
            tier: Some("core".into()),
            owner: owner.map(str::to_string),
            synced_at: None,
        }
    }

    fn svc(unit: &str, service: &str, effect: Option<&str>) -> Service {
        Service {
            unit: unit.into(),
            service: service.into(),
            command: Some(service.into()),
            accepts: None,
            returns: None,
            effect: effect.map(str::to_string),
            args: Some(format!("{service} <query>")),
            output: Some("json".into()),
            note: None,
            health: None,
        }
    }

    fn edge(unit: &str, dept: &str) -> PoolEdge {
        PoolEdge {
            unit: unit.into(),
            dept: dept.into(),
        }
    }

    /// An observe policy as Holon compiles it, with the given read services
    /// added to the allow list the way `holon_authority` adds them.
    fn observe_source(read_services: &[String]) -> Arc<dyn SeatPolicySource> {
        struct One(SeatPolicy);
        impl SeatPolicySource for One {
            fn policy_for(&self, _employee_id: &str) -> Option<SeatPolicy> {
                Some(self.0.clone())
            }
        }
        let doc: PolicyDoc = serde_json::from_value(json!({
            "version": 1,
            "stage": "execute",
            "posture": "observe",
            "budget": {}
        }))
        .expect("observe policy parses");
        Arc::new(One(doc.seat_policy_with_services(read_services)))
    }

    fn consult(source: &Arc<dyn SeatPolicySource>, tool: &str) -> SeatDecision {
        SeatAuthority::new(Arc::clone(source))
            .consult("seat-1", tool, false)
            .expect("the seat has a policy row")
    }

    /// (a) a read service is visible in observe; (b) a write service is denied.
    #[test]
    fn observe_shows_read_services_and_denies_write_services() {
        let units = [unit("browser", Some("platform-ops"))];
        let services = [
            svc("browser", "sourcehound", Some("read")),
            svc("browser", "publish", Some("write")),
            svc("browser", "unlabelled", None),
        ];
        let pool = [edge("browser", "*")];
        // the observer's index carries the read service, and only it
        let run = RunServices {
            posture: "observe".into(),
            entries: visible_for(index_for(&units, &services, &pool, None), "observe"),
        };
        let visible: Vec<&str> = run.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(visible, ["svc_browser_sourcehound"]);
        assert_eq!(run.read_names(), ["svc_browser_sourcehound"]);

        let source = observe_source(&run.read_names());
        assert!(
            matches!(consult(&source, "svc_browser_sourcehound"), SeatDecision::Run(_)),
            "a read service must run from observe"
        );
        assert!(
            !matches!(consult(&source, "svc_browser_publish"), SeatDecision::Run(_)),
            "a write service must not run from observe"
        );
        assert!(
            !matches!(consult(&source, "svc_browser_unlabelled"), SeatDecision::Run(_)),
            "a service with no effect is treated as a write"
        );
    }

    /// (c) entitlement is the api_pool edge, never the name.
    #[test]
    fn a_dept_outside_the_pool_cannot_see_the_service() {
        let units = [unit("research", Some("growth"))];
        let services = [svc("research", "crawl", Some("read"))];
        let pool = [edge("research", "growth")];

        let names = |dept: Option<&str>| -> Vec<String> {
            index_for(&units, &services, &pool, dept)
                .into_iter()
                .map(|e| e.name)
                .collect()
        };
        assert_eq!(names(Some("growth")), ["svc_research_crawl"]);
        assert!(names(Some("legal")).is_empty(), "legal is not in the pool");
        assert!(names(None).is_empty(), "a department-less run is not in the pool");

        // an org-wide edge entitles every dept, including department-less work
        let org = [edge("research", "*")];
        let org_names: Vec<String> = index_for(&units, &services, &org, Some("legal"))
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(org_names, ["svc_research_crawl"]);
        assert_eq!(
            index_for(&units, &services, &org, None).len(),
            1,
            "org-wide reaches department-less runs"
        );

        // a service whose unit has no api_units row is not indexed (no owner, no unit)
        let orphan = [svc("ghost", "haunt", Some("read"))];
        assert!(index_for(&[], &orphan, &[edge("ghost", "*")], None).is_empty());
    }

    /// (d) schema falls back from args/output to accepts/returns.
    #[test]
    fn schema_falls_back_to_accepts_and_returns() {
        let mut fallback = svc("browser", "fetch", Some("read"));
        fallback.args = None;
        fallback.accepts = Some("url: string".into());
        fallback.output = None;
        fallback.returns = Some("page markdown".into());

        let p = parameters(&fallback);
        let desc = p["properties"]["args"]["description"]
            .as_str()
            .expect("args carries a description");
        assert!(desc.contains("url: string"), "accepts fills args: {desc}");

        let entry = index_for(
            &[unit("browser", None)],
            std::slice::from_ref(&fallback),
            &[edge("browser", "*")],
            None,
        )
        .remove(0);
        assert!(
            entry.description.contains("page markdown"),
            "returns fills output: {}",
            entry.description
        );

        // args/output win when present
        let explicit = svc("browser", "fetch", Some("read"));
        let p = parameters(&explicit);
        let desc = p["properties"]["args"]["description"]
            .as_str()
            .expect("args carries a description");
        assert!(desc.contains("fetch <query>"), "args wins over accepts: {desc}");
    }
}

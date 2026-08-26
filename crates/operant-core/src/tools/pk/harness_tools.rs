//! `pk_harness_get` / `pk_refine` — continual-harness read + manual refine.
//!
//! The store carries ONLY `prompt` and `subagent` kinds × local/global scopes
//! (skills/memories remain owned by curator/skills/MEMORY.md lanes). Refinement
//! passes are all-or-nothing with byte-exact snapshot rollback.

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use super::runtime::PkRuntime;

fn scope_of(v: Option<&str>) -> String {
    match v {
        Some("global") => "global".into(),
        _ => "local".into(),
    }
}

// ---------------------------------------------------------------------------
// pk_harness_get
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PkHarnessGetArgs {
    /// "prompt" | "subagent" | "overview" (default overview).
    kind: Option<String>,
    /// Entry id — omit to list that kind's entries via overview.
    id: Option<String>,
    /// "local" (default) | "global". Local is scoped to the CURRENT session.
    scope: Option<String>,
}

pub struct PkHarnessGetTool {
    rt: Arc<PkRuntime>,
}

impl PkHarnessGetTool {
    pub fn new(rt: Arc<PkRuntime>) -> Self {
        Self { rt }
    }
}

#[async_trait]
impl OperantTool for PkHarnessGetTool {
    fn name(&self) -> &str {
        "pk_harness_get"
    }

    fn description(&self) -> &str {
        "Read records from the continual harness store (kinds: prompt = \
         behavioral policy addendums, subagent = reusable delegation specs). \
         Omit kind for an overview; pass kind+id for one entry."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<PkHarnessGetArgs>("pk_harness_get", "Continual harness read")
    }

    fn toolset(&self) -> &str {
        "prime_kernel"
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: PkHarnessGetArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("pk_harness_get", format!("Invalid arguments: {e}"));
            }
        };
        let scope = scope_of(args.scope.as_deref());
        let session_key = context
            .get("session_id")
            .map(str::to_string)
            .unwrap_or_else(|| "default".to_string());
        let method = match (args.kind.as_deref(), args.id.as_deref()) {
            (Some(kind), Some(id)) => {
                let params = json!({"kind": kind, "id": id, "scope": scope,
                                    "session_key": session_key});
                return match self.rt.request("harness_get", params).await {
                    Ok(v) => ToolResult::success("pk_harness_get", v.to_string()),
                    Err(e) => ToolResult::error("pk_harness_get", e),
                };
            }
            (Some("overview"), _) | (None, _) => "harness_overview",
            (Some(_), None) => "harness_overview",
        };
        match self
            .rt
            .request(
                method,
                json!({ "scope": scope, "session_key": session_key }),
            )
            .await
        {
            Ok(v) => ToolResult::success("pk_harness_get", v.to_string()),
            Err(e) => ToolResult::error("pk_harness_get", e),
        }
    }
}

// ---------------------------------------------------------------------------
// pk_refine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefineEditArgs {
    action: String,
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PkRefineArgs {
    /// Short evidence summary of what happened (required when edits omitted).
    evidence: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    /// Explicit all-or-nothing CRUD edits (create/update/delete × prompt/subagent).
    #[serde(default)]
    edits: Option<Vec<RefineEditArgs>>,
    /// "local" (default) | "global".
    scope: Option<String>,
}

pub struct PkRefineTool {
    rt: Arc<PkRuntime>,
}

impl PkRefineTool {
    pub fn new(rt: Arc<PkRuntime>) -> Self {
        Self { rt }
    }
}

fn refine_session_key(context: &ToolContext) -> String {
    context
        .get("session_id")
        .map(str::to_string)
        .unwrap_or_else(|| "default".to_string())
}

#[async_trait]
impl OperantTool for PkRefineTool {
    fn name(&self) -> &str {
        "pk_refine"
    }

    fn description(&self) -> &str {
        "Record or apply one evidence-backed continual-harness refinement. \
         With edits[]: applies all-or-nothing CRUD over prompt/subagent kinds \
         (snapshot kept for rollback). Without edits: records the evidence as \
         a refinement event. Local scope by default."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<PkRefineArgs>("pk_refine", "Continual harness refinement")
    }

    fn toolset(&self) -> &str {
        "prime_kernel"
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: PkRefineArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("pk_refine", format!("Invalid arguments: {e}")),
        };
        let scope = scope_of(args.scope.as_deref());
        let sk = refine_session_key(&context);
        let (method, params) = match args.edits {
            Some(edits) => {
                let payload: Vec<Value> = edits
                    .iter()
                    .map(|e| {
                        json!({
                            "action": e.action, "kind": e.kind,
                            "id": e.id, "title": e.title, "content": e.content,
                        })
                    })
                    .collect();
                (
                    "refine_apply",
                    json!({
                        "edits": payload,
                        "trigger": args.trigger.unwrap_or_else(|| "manual".into()),
                        "evidence": args.evidence.unwrap_or_default(),
                        "scope": scope,
                        "session_key": sk,
                    }),
                )
            }
            None => {
                let evidence = args.evidence.unwrap_or_default();
                if evidence.trim().is_empty() {
                    return ToolResult::error(
                        "pk_refine",
                        "'evidence' is required (or provide edits[])",
                    );
                }
                (
                    "refine_record",
                    json!({
                        "evidence": evidence,
                        "trigger": args.trigger.unwrap_or_else(|| "manual".into()),
                        "scope": scope,
                        "session_key": sk,
                    }),
                )
            }
        };
        match self.rt.request(method, params).await {
            Ok(v) => ToolResult::success("pk_refine", v.to_string()),
            Err(e) => ToolResult::error("pk_refine", e),
        }
    }
}

//! LifeOS tools — Notion-backed holonic life-management system.
//!
//! Integrates `lifeos-core` as native operant tools. LifeOS models a 5-database
//! "HoloOS" ontology (Matrix / Potentiator / Nexus / Significator / GreatWay)
//! with deliberate relational writes via the Notion API.
//!
//! ## Registration
//!
//! Tools are registered via `register_lifeos_tools()`, which requires a
//! shared `LifeosState` (config + Notion client + schema cache). Call this
//! only when `config.lifeos.enabled == true`.
//!
//! ## Tool surface (28 tools)
//!
//! - Query/Mutate: `lifeos_query`, `lifeos_mutate`, `lifeos_get_schema`
//! - Intelligence: `lifeos_intelligence`, `lifeos_review`, `lifeos_strategic`, `lifeos_sync_note`
//! - Holonic: `lifeos_holonic_synthesis`, `lifeos_energy_flow`
//! - Relational: `lifeos_get_page`, `lifeos_build_context`, `lifeos_trace`, `lifeos_ancestors`, `lifeos_backlinks`
//! - Relational writes: `lifeos_link`, `lifeos_unlink`, `lifeos_batch_link`
//! - Audit: `lifeos_orphans`, `lifeos_validate`, `lifeos_suggest_links`
//! - Ontology: `lifeos_archetype_index`, `lifeos_derive_type`, `lifeos_valence_signature`
//! - Workflows: `lifeos_daily`, `lifeos_dashboard`

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Shared state carried by every lifeos tool wrapper.
#[derive(Clone)]
pub struct LifeosState {
    pub config: Arc<lifeos_core::config::LifeOSConfig>,
    pub notion: Arc<lifeos_core::notion::client::NotionClient>,
    pub schema_cache: Arc<lifeos_core::util::schema_engine::SchemaCache>,
}

/// Register all LifeOS tools with a registry, sharing the given state.
///
/// Call this only when `config.lifeos.enabled == true`. The state (config +
/// Notion client + schema cache) is shared across all tools to amortize
/// initialization cost.
pub async fn register_lifeos_tools(
    registry: &ToolRegistry,
    state: LifeosState,
) -> Result<()> {
    registry.register(LifeosQueryTool { state: state.clone() }).await?;
    registry.register(LifeosMutateTool { state: state.clone() }).await?;
    registry.register(LifeosGetSchemaTool { state: state.clone() }).await?;
    registry.register(LifeosIntelligenceTool { state: state.clone() }).await?;
    registry.register(LifeosReviewTool { state: state.clone() }).await?;
    registry.register(LifeosStrategicTool { state: state.clone() }).await?;
    registry.register(LifeosSyncNoteTool { state: state.clone() }).await?;
    registry.register(LifeosHolonicSynthesisTool { state: state.clone() }).await?;
    registry.register(LifeosEnergyFlowTool { state: state.clone() }).await?;
    registry.register(LifeosGetPageTool { state: state.clone() }).await?;
    registry.register(LifeosBuildContextTool { state: state.clone() }).await?;
    registry.register(LifeosTraceTool { state: state.clone() }).await?;
    registry.register(LifeosAncestorsTool { state: state.clone() }).await?;
    registry.register(LifeosBacklinksTool { state: state.clone() }).await?;
    registry.register(LifeosLinkTool { state: state.clone() }).await?;
    registry.register(LifeosUnlinkTool { state: state.clone() }).await?;
    registry.register(LifeosBatchLinkTool { state: state.clone() }).await?;
    registry.register(LifeosOrphansTool { state: state.clone() }).await?;
    registry.register(LifeosValidateTool { state: state.clone() }).await?;
    registry.register(LifeosSuggestLinksTool { state: state.clone() }).await?;
    registry.register(LifeosDailyTool { state: state.clone() }).await?;
    registry.register(LifeosDashboardTool { state }).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: build ToolSchema from lifeos's serde_json::Value schema
// ---------------------------------------------------------------------------

/// Construct a ToolSchema from a lifeos JSON schema value. lifeos-core
/// exposes `pub fn schema() -> serde_json::Value` for each tool — we consume
/// that directly instead of re-deriving JsonSchema (which would couple us
/// to lifeos's schemars version, if it had one).
fn schema_from_json(name: &str, description: &str, json: Value) -> ToolSchema {
    // ToolSchema stores the name + description + the raw JSON schema.
    // The lifeos schema() functions return a full JSON Schema object
    // (type: object, properties: {...}, required: [...]). We wrap it.
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json,
    }
}

/// Execute a lifeos tool function and convert the result to ToolResult.
/// All lifeos execute() functions return `Result<String, String>`.
fn run_lifeos_tool(
    tool_name: &str,
    result: std::result::Result<String, String>,
) -> ToolResult {
    match result {
        Ok(output) => ToolResult::success(tool_name, output),
        Err(e) => ToolResult::error(tool_name, e),
    }
}

// ---------------------------------------------------------------------------
// Query / Mutate / Schema tools
// ---------------------------------------------------------------------------

pub struct LifeosQueryTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosQueryTool {
    fn name(&self) -> &str { "lifeos_query" }
    fn description(&self) -> &str {
        "Query a LifeOS Notion database (matrix/potentiator/nexus/significator/greatway) with filters, sorting, and limits."
    }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_query", "Query LifeOS database",
            lifeos_core::tools::query::schema(&self.state.config, &self.state.schema_cache))
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::query::QueryParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_query", format!("args: {e}")),
        };
        let result = lifeos_core::tools::query::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_query", result)
    }
}

pub struct LifeosMutateTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosMutateTool {
    fn name(&self) -> &str { "lifeos_mutate" }
    fn description(&self) -> &str {
        "Create, update, or archive entries in a LifeOS Notion database."
    }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_mutate", "Mutate LifeOS database",
            lifeos_core::tools::mutate::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::mutate::MutateParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_mutate", format!("args: {e}")),
        };
        let result = lifeos_core::tools::mutate::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_mutate", result)
    }
}

pub struct LifeosGetSchemaTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosGetSchemaTool {
    fn name(&self) -> &str { "lifeos_get_schema" }
    fn description(&self) -> &str {
        "Get the schema (properties, types) of a LifeOS Notion database."
    }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_get_schema", "Get LifeOS database schema",
            lifeos_core::tools::query::schema(&self.state.config, &self.state.schema_cache))
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::query::QueryParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_get_schema", format!("args: {e}")),
        };
        // Return the property names for the requested database
        let db = &params.database;
        let props = self.state.schema_cache.get_property_names(db);
        ToolResult::success("lifeos_get_schema", serde_json::json!({"database": db, "properties": props}))
    }
}

// ---------------------------------------------------------------------------
// Intelligence tools
// ---------------------------------------------------------------------------

pub struct LifeosIntelligenceTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosIntelligenceTool {
    fn name(&self) -> &str { "lifeos_intelligence" }
    fn description(&self) -> &str { "Generate an intelligence briefing from LifeOS data." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_intelligence", "Intelligence briefing",
            lifeos_core::tools::intelligence::schema(&self.state.schema_cache))
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::intelligence::IntelligenceParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_intelligence", format!("args: {e}")),
        };
        let result = lifeos_core::tools::intelligence::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_intelligence", result)
    }
}

pub struct LifeosReviewTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosReviewTool {
    fn name(&self) -> &str { "lifeos_review" }
    fn description(&self) -> &str { "Run a review pipeline over LifeOS entries." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_review", "Review pipeline",
            lifeos_core::tools::review::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::review::ReviewParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_review", format!("args: {e}")),
        };
        let result = lifeos_core::tools::review::execute(
            &params, &self.state.config, &self.state.notion,
        ).await;
        run_lifeos_tool("lifeos_review", result)
    }
}

pub struct LifeosStrategicTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosStrategicTool {
    fn name(&self) -> &str { "lifeos_strategic" }
    fn description(&self) -> &str { "Run a strategic simulation over LifeOS data." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_strategic", "Strategic simulation",
            lifeos_core::tools::strategic::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::strategic::StrategicParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_strategic", format!("args: {e}")),
        };
        let result = lifeos_core::tools::strategic::execute(
            &params, &self.state.config, &self.state.notion,
        ).await;
        run_lifeos_tool("lifeos_strategic", result)
    }
}

pub struct LifeosSyncNoteTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosSyncNoteTool {
    fn name(&self) -> &str { "lifeos_sync_note" }
    fn description(&self) -> &str { "Sync a note to LifeOS Notion databases." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_sync_note", "Sync note",
            lifeos_core::tools::sync_note::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::sync_note::SyncNoteParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_sync_note", format!("args: {e}")),
        };
        let result = lifeos_core::tools::sync_note::execute(
            &params, &self.state.config, &self.state.notion,
        ).await;
        run_lifeos_tool("lifeos_sync_note", result)
    }
}

// ---------------------------------------------------------------------------
// Holonic tools
// ---------------------------------------------------------------------------

pub struct LifeosHolonicSynthesisTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosHolonicSynthesisTool {
    fn name(&self) -> &str { "lifeos_holonic_synthesis" }
    fn description(&self) -> &str { "Run holonic synthesis over LifeOS entries." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_holonic_synthesis", "Holonic synthesis",
            lifeos_core::tools::holonic_synthesis::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::holonic_synthesis::HolonicSynthesisParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_holonic_synthesis", format!("args: {e}")),
        };
        let result = lifeos_core::tools::holonic_synthesis::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_holonic_synthesis", result)
    }
}

pub struct LifeosEnergyFlowTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosEnergyFlowTool {
    fn name(&self) -> &str { "lifeos_energy_flow" }
    fn description(&self) -> &str { "Analyze energy flow across LifeOS entries." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_energy_flow", "Energy flow analysis",
            lifeos_core::tools::energy_flow::schema(&self.state.config))
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::energy_flow::EnergyFlowParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_energy_flow", format!("args: {e}")),
        };
        let result = lifeos_core::tools::energy_flow::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_energy_flow", result)
    }
}

// ---------------------------------------------------------------------------
// Relational navigation tools
// ---------------------------------------------------------------------------

macro_rules! lifeos_relational_tool {
    ($tool_name:ident, $tool_str:expr, $exec_path:path, $schema_fn:ident, $params_ty:ty, $desc:expr) => {
        pub struct $tool_name { state: LifeosState }

        #[async_trait]
        impl OperantTool for $tool_name {
            fn name(&self) -> &str { $tool_str }
            fn description(&self) -> &str { $desc }
            fn schema(&self) -> ToolSchema {
                schema_from_json($tool_str, $desc,
                    lifeos_core::tools::relations::$schema_fn())
            }
            async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
                let params: $params_ty = match serde_json::from_value(args) {
                    Ok(p) => p,
                    Err(e) => return ToolResult::error($tool_str, format!("args: {e}")),
                };
                let result = $exec_path(
                    &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
                ).await;
                run_lifeos_tool($tool_str, result)
            }
        }
    };
}

lifeos_relational_tool!(LifeosGetPageTool, "lifeos_get_page", lifeos_core::tools::relations::execute_get_page, schema_get_page, lifeos_core::tools::relations::GetPageParams, "Get a single LifeOS page with all properties and relations.");
lifeos_relational_tool!(LifeosTraceTool, "lifeos_trace", lifeos_core::tools::relations::execute_trace, schema_trace, lifeos_core::tools::relations::TraceParams, "Trace relational paths from a starting entry.");
lifeos_relational_tool!(LifeosAncestorsTool, "lifeos_ancestors", lifeos_core::tools::relations::execute_ancestors, schema_ancestors, lifeos_core::tools::relations::AncestorsParams, "Get all ancestor entries of a LifeOS entry.");
lifeos_relational_tool!(LifeosBacklinksTool, "lifeos_backlinks", lifeos_core::tools::relations::execute_backlinks, schema_backlinks, lifeos_core::tools::relations::BacklinksParams, "Get all entries that link to a LifeOS entry.");

pub struct LifeosBuildContextTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosBuildContextTool {
    fn name(&self) -> &str { "lifeos_build_context" }
    fn description(&self) -> &str { "Build a context window from LifeOS entries for LLM consumption." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_build_context", "Build context",
            lifeos_core::tools::build_context::schema())
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let params: lifeos_core::tools::build_context::BuildContextParams = match serde_json::from_value(args) {
            Ok(p) => p,
            Err(e) => return ToolResult::error("lifeos_build_context", format!("args: {e}")),
        };
        let result = lifeos_core::tools::build_context::execute(
            &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_build_context", result)
    }
}

// ---------------------------------------------------------------------------
// Relational write tools
// ---------------------------------------------------------------------------

lifeos_relational_tool!(LifeosLinkTool, "lifeos_link", lifeos_core::tools::relations::execute_link, schema_link, lifeos_core::tools::relations::LinkParams, "Create a relation between two LifeOS entries.");
lifeos_relational_tool!(LifeosUnlinkTool, "lifeos_unlink", lifeos_core::tools::relation_ops::execute_unlink, schema_link, lifeos_core::tools::relation_ops::UnlinkParams, "Remove a relation between two LifeOS entries.");
lifeos_relational_tool!(LifeosBatchLinkTool, "lifeos_batch_link", lifeos_core::tools::relation_ops::execute_batch_link, schema_link, lifeos_core::tools::relation_ops::BatchLinkParams, "Create multiple relations in a batch.");

// ---------------------------------------------------------------------------
// Audit tools
// ---------------------------------------------------------------------------

macro_rules! lifeos_audit_tool {
    ($tool_name:ident, $tool_str:expr, $exec_fn:ident, $schema_fn:ident, $params_ty:ty, $desc:expr) => {
        pub struct $tool_name { state: LifeosState }

        #[async_trait]
        impl OperantTool for $tool_name {
            fn name(&self) -> &str { $tool_str }
            fn description(&self) -> &str { $desc }
            fn schema(&self) -> ToolSchema {
                schema_from_json($tool_str, $desc,
                    lifeos_core::tools::audit::$schema_fn())
            }
            async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
                let params: $params_ty = match serde_json::from_value(args) {
                    Ok(p) => p,
                    Err(e) => return ToolResult::error($tool_str, format!("args: {e}")),
                };
                let result = lifeos_core::tools::audit::$exec_fn(
                    &params, &self.state.config, &self.state.notion, &self.state.schema_cache,
                ).await;
                run_lifeos_tool($tool_str, result)
            }
        }
    };
}

lifeos_audit_tool!(LifeosOrphansTool, "lifeos_orphans", execute_orphans, schema_orphans, lifeos_core::tools::audit::OrphansParams, "Find orphan entries with no relations.");
lifeos_audit_tool!(LifeosValidateTool, "lifeos_validate", execute_validate, schema_validate, lifeos_core::tools::audit::ValidateParams, "Validate LifeOS entries for schema compliance.");
lifeos_audit_tool!(LifeosSuggestLinksTool, "lifeos_suggest_links", execute_suggest_links, schema_suggest_links, lifeos_core::tools::audit::SuggestLinksParams, "Suggest potential links between entries.");

// ---------------------------------------------------------------------------
// Workflow tools
// ---------------------------------------------------------------------------

pub struct LifeosDailyTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosDailyTool {
    fn name(&self) -> &str { "lifeos_daily" }
    fn description(&self) -> &str { "Generate a daily briefing from LifeOS data." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_daily", "Daily briefing",
            lifeos_core::tools::workflows::schema_daily())
    }
    async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
        let result = lifeos_core::tools::workflows::execute_daily(
            &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_daily", result)
    }
}

pub struct LifeosDashboardTool { state: LifeosState }

#[async_trait]
impl OperantTool for LifeosDashboardTool {
    fn name(&self) -> &str { "lifeos_dashboard" }
    fn description(&self) -> &str { "Generate a dashboard view of LifeOS status." }
    fn schema(&self) -> ToolSchema {
        schema_from_json("lifeos_dashboard", "Dashboard view",
            lifeos_core::tools::workflows::schema_dashboard())
    }
    async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
        let result = lifeos_core::tools::workflows::execute_dashboard(
            &self.state.config, &self.state.notion, &self.state.schema_cache,
        ).await;
        run_lifeos_tool("lifeos_dashboard", result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifeos_tool_names_are_prefixed() {
        // All lifeos tools must have names starting with "lifeos_"
        let names = vec![
            "lifeos_query", "lifeos_mutate", "lifeos_get_schema",
            "lifeos_intelligence", "lifeos_review", "lifeos_strategic",
            "lifeos_sync_note", "lifeos_holonic_synthesis", "lifeos_energy_flow",
            "lifeos_get_page", "lifeos_build_context", "lifeos_trace",
            "lifeos_ancestors", "lifeos_backlinks", "lifeos_link",
            "lifeos_unlink", "lifeos_batch_link", "lifeos_orphans",
            "lifeos_validate", "lifeos_suggest_links", "lifeos_daily",
            "lifeos_dashboard",
        ];
        for name in &names {
            assert!(name.starts_with("lifeos_"), "tool '{}' should start with 'lifeos_'", name);
        }
    }
}

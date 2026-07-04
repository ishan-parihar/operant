//! TDG graph memory tools.
//!
//! These tools expose the TDG (Teleological Developmental Graph) to the agent
//! as callable functions: search, create, connect, get_related.
//!
//! **iter-31 fix**: Previously each tool called `tdg_pool()` which created
//! its OWN connection pool at `~/.operant/tdg/graph.db` — a DIFFERENT
//! database from the `TdgMemoryProvider` (which uses
//! `<storage_dir>/tdg/graph.db`). Nodes created via `tdg_create` were
//! invisible to `prefetch`, and nodes created via `sync_turn` were invisible
//! to `tdg_search`. Now the tools share the provider's pool via
//! `register_tdg_tools(pool)`, so there's a single unified graph.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult, ToolRegistry};
use crate::error::Result;

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all TDG tools with a registry, sharing the given connection pool.
///
/// **Call this only when `config.memory.provider == "tdg"`** — the tools are
/// meaningless without a TDG backend, and registering them unconditionally
/// (the old behavior) meant every agent got 4 graph-memory tools even when
/// using the builtin file-backed memory provider.
///
/// The `pool` should be the same pool used by `TdgMemoryProvider` so that
/// nodes created by the agent via `tdg_create` are visible to the provider's
/// `prefetch` / `sync_turn`, and vice versa. This fixes the dual-database
/// bug where tools and provider talked to different graph.db files.
pub async fn register_tdg_tools(
    registry: &ToolRegistry,
    pool: Arc<tdg_rust::ConnectionPool>,
) -> Result<()> {
    registry.register(TdgSearchTool { pool: pool.clone() }).await?;
    registry.register(TdgCreateTool { pool: pool.clone() }).await?;
    registry.register(TdgConnectTool { pool: pool.clone() }).await?;
    registry.register(TdgGetRelatedTool { pool }).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// TdgSearchTool
// ---------------------------------------------------------------------------

pub struct TdgSearchTool {
    pool: Arc<tdg_rust::ConnectionPool>,
}

#[derive(JsonSchema, Deserialize)]
struct TdgSearchArgs {
    query: String,
}

#[async_trait]
impl OperantTool for TdgSearchTool {
    fn name(&self) -> &str {
        "tdg_search"
    }

    fn description(&self) -> &str {
        "Search graph memory using full-text search. Returns matching nodes with their types, names, and descriptions."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TdgSearchArgs>("tdg_search", "Search graph memory")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TdgSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("tdg_search", format!("Invalid arguments: {}", e)),
        };

        let pool = self.pool.clone();
        let query = args.query.clone();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<Value>, String> {
            pool.with_connection(|conn| {
                // Use FTS5 for search (previously used LIKE %query% — a
                // sequential scan that ignored the FTS5 virtual table that
                // init_fts() created and maintained on every write).
                // The FTS table is named `nodes_fts` and mirrors the
                // `name` + `description` columns of `nodes`.
                let mut stmt = conn.prepare(
                    "SELECT n.id, n.node_type, n.name, n.description
                     FROM nodes_fts f
                     JOIN nodes n ON n.rowid = f.rowid
                     WHERE n.valid_to IS NULL AND nodes_fts MATCH ?1
                     ORDER BY rank
                     LIMIT 10"
                )?;
                // FTS5 MATCH syntax: for multi-word queries, we want each
                // word to be a prefix match (OR semantics). E.g. "hello world"
                // should match nodes containing "hello" OR "world" as prefixes.
                //
                // FTS5 special characters (colon, asterisk, parens, quotes)
                // must be stripped or they cause syntax errors. E.g. "C++"
                // or "3:1" would error without sanitization. We strip any
                // char that isn't alphanumeric, underscore, or hyphen, then
                // wrap each token in double-quotes (FTS5 string literal) +
                // append * for prefix matching.
                let fts_query: String = query
                    .split_whitespace()
                    .filter_map(|token| {
                        // Sanitize: keep only alnum + underscore + hyphen
                        let clean: String = token
                            .chars()
                            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                            .collect();
                        if clean.is_empty() {
                            None
                        } else {
                            // Wrap in double-quotes (FTS5 string literal) to
                            // prevent any residual special chars from being
                            // interpreted, then append * for prefix match.
                            Some(format!("\"{}\"*", clean))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if fts_query.is_empty() {
                    return Ok(vec![]);
                }
                let rows: Vec<Value> = stmt
                    .query_map(rusqlite::params![fts_query], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "node_type": row.get::<_, String>(1)?,
                            "name": row.get::<_, String>(2)?,
                            "description": row.get::<_, String>(3)?
                        }))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .map_err(|e| e.to_string())
        })
        .await;

        match result {
            Ok(Ok(rows)) => ToolResult::success(
                "tdg_search",
                serde_json::json!({"query": args.query, "results": rows, "count": rows.len()}),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_search", format!("Query failed: {}", e)),
            Err(e) => ToolResult::error("tdg_search", format!("Task failed: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// TdgCreateTool
// ---------------------------------------------------------------------------

pub struct TdgCreateTool {
    pool: Arc<tdg_rust::ConnectionPool>,
}

#[derive(JsonSchema, Deserialize)]
struct TdgCreateArgs {
    node_type: String,
    name: String,
    description: Option<String>,
}

#[async_trait]
impl OperantTool for TdgCreateTool {
    fn name(&self) -> &str {
        "tdg_create"
    }

    fn description(&self) -> &str {
        "Create a new entity node in the graph memory. Use for storing facts, observations, or any structured information."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TdgCreateArgs>("tdg_create", "Create a graph node")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TdgCreateArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("tdg_create", format!("Invalid arguments: {}", e)),
        };

        let pool = self.pool.clone();
        let node_type = args.node_type.clone();
        let node_name = args.name.clone();
        let desc = args.description.clone().unwrap_or_default();
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<tdg_rust::Node, String> {
                pool.with_connection(|conn| {
                    let new_node = tdg_rust::NewNode {
                        node_type,
                        name: node_name,
                        description: Some(desc),
                        properties: None,
                        quadrants: None,
                        drives: None,
                        lifecycle_state: None,
                        teleological_level: None,
                        // Fixed: was Some(0) which is invalid (Stage enum
                        // is 1-8). Stage 1 = "Seed" — the initial
                        // developmental stage for a freshly-created node.
                        developmental_stage: Some(1),
                        confidence: Some(0.5),
                        source: Some("operant-agent".to_string()),
                        parent_ids: None,
                        agent_id: None,
                        ..Default::default()
                    };
                    let node = tdg_rust::db::crud::add_node(conn, &new_node)?;
                    Ok(node)
                })
                .map_err(|e| e.to_string())
            })
            .await;

        match result {
            Ok(Ok(node)) => ToolResult::success(
                "tdg_create",
                serde_json::json!({
                    "id": node.id,
                    "node_type": node.node_type,
                    "name": node.name,
                    "description": node.description,
                }),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_create", format!("Create failed: {}", e)),
            Err(e) => ToolResult::error("tdg_create", format!("Task failed: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// TdgConnectTool
// ---------------------------------------------------------------------------

pub struct TdgConnectTool {
    pool: Arc<tdg_rust::ConnectionPool>,
}

#[derive(JsonSchema, Deserialize)]
struct TdgConnectArgs {
    source_id: String,
    target_id: String,
    relation: String,
    #[serde(default)]
    strength: Option<f64>,
}

#[async_trait]
impl OperantTool for TdgConnectTool {
    fn name(&self) -> &str {
        "tdg_connect"
    }

    fn description(&self) -> &str {
        "Create a relationship (edge) between two nodes in the graph memory."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TdgConnectArgs>("tdg_connect", "Connect two graph nodes")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TdgConnectArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("tdg_connect", format!("Invalid arguments: {}", e)),
        };

        let pool = self.pool.clone();
        let source_id = args.source_id.clone();
        let target_id = args.target_id.clone();
        let relation = args.relation.clone();
        let strength = args.strength.unwrap_or(0.5);
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<tdg_rust::Edge, String> {
                pool.with_connection(|conn| {
                    let new_edge = tdg_rust::NewEdge {
                        source_id,
                        target_id,
                        edge_type: relation,
                        weight: Some(strength),
                        ..Default::default()
                    };
                    let edge = tdg_rust::db::crud::add_edge(conn, &new_edge)?;
                    Ok(edge)
                })
                .map_err(|e| e.to_string())
            })
            .await;

        match result {
            Ok(Ok(edge)) => ToolResult::success(
                "tdg_connect",
                serde_json::json!({
                    "id": edge.id,
                    "source_id": edge.source_id,
                    "target_id": edge.target_id,
                    "edge_type": edge.edge_type,
                }),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_connect", format!("Connect failed: {}", e)),
            Err(e) => ToolResult::error("tdg_connect", format!("Task failed: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// TdgGetRelatedTool
// ---------------------------------------------------------------------------

pub struct TdgGetRelatedTool {
    pool: Arc<tdg_rust::ConnectionPool>,
}

#[derive(JsonSchema, Deserialize)]
struct TdgGetRelatedArgs {
    node_id: String,
}

#[async_trait]
impl OperantTool for TdgGetRelatedTool {
    fn name(&self) -> &str {
        "tdg_get_related"
    }

    fn description(&self) -> &str {
        "Get all nodes connected to a given node (both outgoing and incoming edges). Returns edge details and connected node IDs."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TdgGetRelatedArgs>("tdg_get_related", "Get related nodes")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TdgGetRelatedArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("tdg_get_related", format!("Invalid arguments: {}", e))
            }
        };

        let pool = self.pool.clone();
        let node_id = args.node_id.clone();
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<Vec<Value>, String> {
                pool.with_connection(|conn| {
                    // Fixed: previously only queried outgoing edges
                    // (source_id=Some). Now queries BOTH directions:
                    // outgoing (source_id=node_id) and incoming
                    // (target_id=node_id).
                    let outgoing =
                        tdg_rust::db::crud::get_edges(conn, Some(&node_id), None, None, None, 20)?;
                    let incoming =
                        tdg_rust::db::crud::get_edges(conn, None, Some(&node_id), None, None, 20)?;

                    let mut relations: Vec<Value> = Vec::new();
                    for e in &outgoing {
                        relations.push(serde_json::json!({
                            "edge_id": e.id,
                            "direction": "outgoing",
                            "source_id": e.source_id,
                            "target_id": e.target_id,
                            "relation_type": e.edge_type,
                            "strength": e.weight,
                        }));
                    }
                    for e in &incoming {
                        relations.push(serde_json::json!({
                            "edge_id": e.id,
                            "direction": "incoming",
                            "source_id": e.source_id,
                            "target_id": e.target_id,
                            "relation_type": e.edge_type,
                            "strength": e.weight,
                        }));
                    }
                    Ok(relations)
                })
                .map_err(|e| e.to_string())
            })
            .await;

        match result {
            Ok(Ok(relations)) => ToolResult::success(
                "tdg_get_related",
                serde_json::json!({"node_id": args.node_id, "relations": relations, "count": relations.len()}),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_get_related", format!("Query failed: {}", e)),
            Err(e) => ToolResult::error("tdg_get_related", format!("Task failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pool() -> Arc<tdg_rust::ConnectionPool> {
        let pool = tdg_rust::ConnectionPool::new(":memory:", 1, 5000).unwrap();
        pool.with_connection(|conn| {
            tdg_rust::init_schema(conn)?;
            tdg_rust::init_fts(conn)?;
            Ok(())
        }).unwrap();
        Arc::new(pool)
    }

    #[test]
    fn tdg_search_tool_schema_is_valid() {
        let tool = TdgSearchTool { pool: test_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "tdg_search");
    }

    #[test]
    fn tdg_create_tool_schema_is_valid() {
        let tool = TdgCreateTool { pool: test_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "tdg_create");
    }

    #[test]
    fn tdg_connect_tool_schema_is_valid() {
        let tool = TdgConnectTool { pool: test_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "tdg_connect");
    }

    #[test]
    fn tdg_get_related_tool_schema_is_valid() {
        let tool = TdgGetRelatedTool { pool: test_pool() };
        let schema = tool.schema();
        assert_eq!(schema.name, "tdg_get_related");
    }
}

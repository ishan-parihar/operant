use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

fn tdg_pool() -> std::sync::Arc<tdg_rust::ConnectionPool> {
    use std::sync::OnceLock;
    static POOL: OnceLock<std::sync::Arc<tdg_rust::ConnectionPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let db_path = home.join(".hermes").join("tdg").join("graph.db");
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let pool = tdg_rust::ConnectionPool::new(
            db_path.to_str().unwrap_or("~/.hermes/tdg/graph.db"),
            5,
            30_000,
        )
        .expect("failed to create TDG pool");
        pool.with_connection(|conn| {
            tdg_rust::init_schema(conn)?;
            tdg_rust::init_fts(conn)?;
            tdg_rust::run_migrations(conn)?;
            Ok(())
        })
        .expect("failed to init TDG schema");
        std::sync::Arc::new(pool)
    })
    .clone()
}

pub struct TdgSearchTool;

#[derive(JsonSchema, Deserialize)]
struct TdgSearchArgs {
    query: String,
}

#[async_trait]
impl HermesTool for TdgSearchTool {
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

        let pool = tdg_pool();
        let query = args.query.clone();
        let result = tokio::task::spawn_blocking(move || -> std::result::Result<Vec<Value>, String> {
            pool.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, node_type, name, description FROM nodes WHERE valid_to IS NULL AND (name LIKE ?1 OR description LIKE ?1) LIMIT 10"
                )?;
                let pattern = format!("%{}%", query);
                let rows: Vec<Value> = stmt
                    .query_map(rusqlite::params![pattern], |row| {
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

pub struct TdgCreateTool;

#[derive(JsonSchema, Deserialize)]
struct TdgCreateArgs {
    node_type: String,
    name: String,
    description: Option<String>,
}

#[async_trait]
impl HermesTool for TdgCreateTool {
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

        let pool = tdg_pool();
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
                        developmental_stage: Some(0),
                        confidence: Some(0.5),
                        source: Some("hermes-agent".to_string()),
                        parent_ids: None,
                        agent_id: None,
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
                    "created": true
                }),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_create", format!("Create failed: {}", e)),
            Err(e) => ToolResult::error("tdg_create", format!("Task failed: {}", e)),
        }
    }
}

pub struct TdgConnectTool;

#[derive(JsonSchema, Deserialize)]
struct TdgConnectArgs {
    source_id: String,
    target_id: String,
    edge_type: String,
}

#[async_trait]
impl HermesTool for TdgConnectTool {
    fn name(&self) -> &str {
        "tdg_connect"
    }

    fn description(&self) -> &str {
        "Create a relationship between two nodes in the graph. Edge types include: RELATES_TO, ENABLES, CONTEXT, BLOCKS, SUPPORTS, EVIDENCES, etc."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TdgConnectArgs>("tdg_connect", "Create a graph edge")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TdgConnectArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("tdg_connect", format!("Invalid arguments: {}", e)),
        };

        let pool = tdg_pool();
        let src = args.source_id.clone();
        let tgt = args.target_id.clone();
        let edge_type = args.edge_type.clone();
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<tdg_rust::Edge, String> {
                pool.with_connection(|conn| {
                    let new_edge = tdg_rust::NewEdge {
                        source_id: src,
                        target_id: tgt,
                        edge_type,
                        weight: None,
                        properties: None,
                        agent_id: None,
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
                    "edge_id": edge.id,
                    "source": edge.source_id,
                    "target": edge.target_id,
                    "edge_type": edge.edge_type
                }),
            ),
            Ok(Err(e)) => ToolResult::error("tdg_connect", format!("Connect failed: {}", e)),
            Err(e) => ToolResult::error("tdg_connect", format!("Task failed: {}", e)),
        }
    }
}

pub struct TdgGetRelatedTool;

#[derive(JsonSchema, Deserialize)]
struct TdgGetRelatedArgs {
    node_id: String,
}

#[async_trait]
impl HermesTool for TdgGetRelatedTool {
    fn name(&self) -> &str {
        "tdg_get_related"
    }

    fn description(&self) -> &str {
        "Get all nodes connected to a given node. Returns edge details and connected node IDs."
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

        let pool = tdg_pool();
        let node_id = args.node_id.clone();
        let result =
            tokio::task::spawn_blocking(move || -> std::result::Result<Vec<Value>, String> {
                pool.with_connection(|conn| {
                    let edges =
                        tdg_rust::db::crud::get_edges(conn, Some(&node_id), None, None, None, 20)?;
                    let relations: Vec<Value> = edges
                        .iter()
                        .map(|e| {
                            let other = if e.source_id == node_id {
                                &e.target_id
                            } else {
                                &e.source_id
                            };
                            serde_json::json!({
                                "edge_id": e.id,
                                "edge_type": e.edge_type,
                                "connected_to": other
                            })
                        })
                        .collect();
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

    #[test]
    fn test_tdg_search_schema() {
        let schema = TdgSearchTool.schema();
        assert_eq!(schema.name, "tdg_search");
    }

    #[test]
    fn test_tdg_create_schema() {
        let schema = TdgCreateTool.schema();
        assert_eq!(schema.name, "tdg_create");
    }

    #[test]
    fn test_tdg_connect_schema() {
        let schema = TdgConnectTool.schema();
        assert_eq!(schema.name, "tdg_connect");
    }

    #[test]
    fn test_tdg_get_related_schema() {
        let schema = TdgGetRelatedTool.schema();
        assert_eq!(schema.name, "tdg_get_related");
    }
}

//! fixture-echo — the in-tree WASM fixture (plan gap G8).
//!
//! One module pinning BOTH live contracts so the integration tests in
//! `crates/operant-plugins/tests/fixture_plugin.rs` exercise real WASM, not
//! serde-only pins:
//! - tool: `tool_metadata` + `execute` (echo)
//! - observer: `observe` — Allow by default; Block when the event's `tool`
//!   is `blocked-tool`, or when the manifest `[config]` table carries
//!   `block_all = true` (proves config passthrough end-to-end).
//!
//! Rebuild the committed artifact with `scripts/build-plugin-fixture.sh`
//! (requires `rustup target add wasm32-wasip1`).
use extism_pdk::{plugin_fn, FnResult, Json};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct ToolMetadata {
    name: String,
    description: String,
    parameters_schema: serde_json::Value,
}

#[derive(Serialize)]
struct ToolResult {
    success: bool,
    output: String,
}

#[derive(Deserialize)]
struct ToolArgs {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct ObserveEvent {
    tool: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default)]
    config: serde_json::Value,
}

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Verdict {
    Allow,
    Block { reason: String },
    Replace { content: String },
}

#[plugin_fn]
pub fn tool_metadata(_input: String) -> FnResult<Json<ToolMetadata>> {
    Ok(Json(ToolMetadata {
        name: "fixture-echo".into(),
        description: "in-tree fixture module (plan gap G8)".into(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "properties": {"text": {"type": "string"}}
        }),
    }))
}

#[plugin_fn]
pub fn execute(args: Json<ToolArgs>) -> FnResult<Json<ToolResult>> {
    let args = args.0;
    Ok(Json(ToolResult {
        success: true,
        output: format!("fixture echo: {}", args.text),
    }))
}

#[plugin_fn]
pub fn observe(event: Json<ObserveEvent>) -> FnResult<Json<Verdict>> {
    let event = event.0;
    // gap G1: on the turn pre-step seam (config turn_gate = true), a query
    // containing `block_query_contains` is vetoed whole.
    if let Some(serde_json::Value::String(needle)) = event.config.get("block_query_contains") {
        if let Some(query) = event
            .args
            .get("query")
            .and_then(|q| q.as_str())
        {
            if query.contains(needle.as_str()) {
                return Ok(Json(Verdict::Block {
                    reason: format!("fixture: query contains {needle}"),
                }));
            }
        }
    }
    // gap G2: config.replace_content = "<text>" makes the fixture emit a
    // Replace verdict — the redaction/enrichment seat, exercised for real.
    if let Some(serde_json::Value::String(content)) = event.config.get("replace_content") {
        return Ok(Json(Verdict::Replace {
            content: content.clone(),
        }));
    }
    if event.config.get("block_all") == Some(&serde_json::Value::Bool(true)) {
        return Ok(Json(Verdict::Block {
            reason: "fixture: config.block_all".into(),
        }));
    }
    if event.tool == "blocked-tool" {
        return Ok(Json(Verdict::Block {
            reason: "fixture: tool is blocked-tool".into(),
        }));
    }
    Ok(Json(Verdict::Allow))
}

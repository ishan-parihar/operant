//! Gap G8: the observe/tool contracts exercised against a REAL WASM module
//! (no serde-only pins). The fixture artifact is COMMITTED at
//! `fixtures/fixture.wasm` so these tests run without the wasm32-wasip1
//! target; regenerate it with `scripts/build-plugin-fixture.sh` after
//! editing `fixtures/fixture-plugin/src/lib.rs`.

use operant_api::tool::Tool as _;
use operant_plugins::runtime::PluginPool;
use operant_plugins::wasm_tool::WasmTool;
use serde_json::json;

fn fixture_wasm() -> std::path::PathBuf {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/fixture.wasm");
    assert!(
        path.exists(),
        "fixture module missing at {} — regenerate with scripts/build-plugin-fixture.sh",
        path.display()
    );
    path
}

/// The `execute` contract, proven on real WASM: the module echoes its args.
#[tokio::test]
async fn execute_runs_against_the_real_fixture_module() {
    let tool = WasmTool::from_wasm(
        fixture_wasm(),
        Vec::new(),
        "fallback".to_string(),
        "fallback".to_string(),
    );
    // from_wasm read the module's own `tool_metadata` export, not the fallback
    assert_eq!(tool.name(), "fixture-echo");
    assert_eq!(tool.description(), "in-tree fixture module (plan gap G8)");

    let result = tool
        .execute(json!({"text": "ping"}))
        .await
        .expect("execute");
    assert!(result.success);
    assert_eq!(result.output, "fixture echo: ping");
}

/// The `observe` contract, proven on real WASM: default verdict is Allow.
#[test]
fn observe_allows_by_default_against_the_real_fixture_module() {
    let pool = PluginPool::new(&fixture_wasm(), &[]);
    let event = json!({
        "point": "tools/pre-execute",
        "tool": "any-tool",
        "args": {"text": "ping"},
        "result": serde_json::Value::Null,
        "config": {}
    });
    match pool
        .observe(&event.to_string())
        .expect("observe call on real module")
    {
        Some(verdict) => assert_eq!(verdict, operant_plugins::runtime::ObserveVerdict::Allow),
        None => panic!("pool reported busy on an uncontended fixture instance"),
    }
}

/// A real module's Block verdict (by tool name) reaches the host verbatim.
#[test]
fn observe_blocks_by_tool_name_through_the_real_module() {
    let pool = PluginPool::new(&fixture_wasm(), &[]);
    let event = json!({
        "point": "tools/pre-execute",
        "tool": "blocked-tool",
        "args": {},
        "result": serde_json::Value::Null,
        "config": {}
    });
    match pool.observe(&event.to_string()).expect("observe call") {
        Some(operant_plugins::runtime::ObserveVerdict::Block { reason }) => {
            assert!(
                reason.contains("blocked-tool"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected a Block verdict, got {other:?}"),
    }
}

/// gap G1: on the turn pre-step envelope, the REAL module vetoes a query
/// containing the configured needle.
#[test]
fn observe_vetoes_a_turn_through_the_real_fixture_module() {
    let pool = PluginPool::new(&fixture_wasm(), &[]);
    let event = json!({
        "point": "agent/pre-step",
        "tool": "turn",
        "args": {"query": "please do the forbidden thing"},
        "result": serde_json::Value::Null,
        "config": {"turn_gate": true, "block_query_contains": "forbidden"}
    });
    match pool.observe(&event.to_string()).expect("observe call") {
        Some(operant_plugins::runtime::ObserveVerdict::Block { reason }) => {
            assert!(reason.contains("forbidden"), "unexpected reason: {reason}");
        }
        other => panic!("expected a Block verdict, got {other:?}"),
    }
}

/// gap G2: the module emits a Replace verdict when the envelope config asks
/// — the post-execute replace seat, driven by REAL WASM.
#[test]
fn observe_replaces_through_the_real_fixture_module() {
    let pool = PluginPool::new(&fixture_wasm(), &[]);
    let event = json!({
        "point": "tools/post-execute",
        "tool": "any-tool",
        "args": {},
        "result": {"content": "sensitive"},
        "config": {"replace_content": "[redacted by policy]"}
    });
    match pool.observe(&event.to_string()).expect("observe call") {
        Some(operant_plugins::runtime::ObserveVerdict::Replace { content }) => {
            assert_eq!(content, "[redacted by policy]");
        }
        other => panic!("expected a Replace verdict, got {other:?}"),
    }
}

/// Manifest `[config]` rides the envelope and changes a real module's verdict
/// — the config-passthrough path pinned end-to-end (the CLI-level pin in
/// plugin_tools.rs carries config; here the MODULE acts on it).
#[test]
fn observe_honors_the_envelope_config_through_the_real_module() {
    let pool = PluginPool::new(&fixture_wasm(), &[]);
    let event = json!({
        "point": "tools/pre-execute",
        "tool": "any-tool",
        "args": {},
        "result": serde_json::Value::Null,
        "config": {"block_all": true}
    });
    match pool.observe(&event.to_string()).expect("observe call") {
        Some(operant_plugins::runtime::ObserveVerdict::Block { reason }) => {
            assert!(
                reason.contains("config.block_all"),
                "unexpected reason: {reason}"
            );
        }
        other => panic!("expected a Block verdict, got {other:?}"),
    }
}

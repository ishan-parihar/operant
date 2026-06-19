use std::path::PathBuf;
use hermes_core::MemoryProvider;

fn test_storage_dir() -> PathBuf {
    let dir = tempfile::tempdir().unwrap();
    dir.into_path()
}

#[tokio::test]
async fn test_tdg_provider_initialization() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage.clone());
    assert_eq!(provider.name(), "tdg");
    assert!(provider.is_available());
    provider.initialize("test-session").await.unwrap();
}

#[tokio::test]
async fn test_tdg_provider_prefetch_empty() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    let result = provider.prefetch("test query").await;
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_tdg_provider_sync_turn() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    provider.sync_turn("user message", "assistant response").await.unwrap();
}

#[tokio::test]
async fn test_tdg_provider_system_prompt() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    let prompt = provider.system_prompt_block();
    assert!(prompt.contains("TDG"));
    assert!(prompt.contains("graph memory"));
}

#[tokio::test]
async fn test_tdg_provider_tool_schemas() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    let schemas = provider.tool_schemas();
    assert_eq!(schemas.len(), 4);
    let names: Vec<&str> = schemas.iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"tdg_search"));
    assert!(names.contains(&"tdg_create"));
    assert!(names.contains(&"tdg_connect"));
    assert!(names.contains(&"tdg_get_related"));
}

#[tokio::test]
async fn test_tdg_provider_handle_search() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    let result = provider.handle_tool_call(
        "tdg_search",
        serde_json::json!({"query": "test"}),
    ).await;
    assert!(result.contains("results"));
}

#[tokio::test]
async fn test_tdg_provider_handle_create() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    let result = provider.handle_tool_call(
        "tdg_create",
        serde_json::json!({
            "node_type": "observation",
            "name": "test entity",
            "description": "a test observation"
        }),
    ).await;
    assert!(result.contains("id"));
    assert!(result.contains("test entity"));
}

#[tokio::test]
async fn test_tdg_provider_handle_connect() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    let create1 = provider.handle_tool_call(
        "tdg_create",
        serde_json::json!({"node_type": "observation", "name": "node A"}),
    ).await;
    let id1: String = serde_json::from_str::<serde_json::Value>(&create1).unwrap()
        .get("id").unwrap().as_str().unwrap().to_string();
    let create2 = provider.handle_tool_call(
        "tdg_create",
        serde_json::json!({"node_type": "observation", "name": "node B"}),
    ).await;
    let id2: String = serde_json::from_str::<serde_json::Value>(&create2).unwrap()
        .get("id").unwrap().as_str().unwrap().to_string();
    let result = provider.handle_tool_call(
        "tdg_connect",
        serde_json::json!({
            "source_id": id1,
            "target_id": id2,
            "edge_type": "RELATES_TO"
        }),
    ).await;
    assert!(result.contains("edge_id"));
}

#[tokio::test]
async fn test_tdg_provider_handle_get_related() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.initialize("test-session").await.unwrap();
    let create = provider.handle_tool_call(
        "tdg_create",
        serde_json::json!({"node_type": "observation", "name": "orphan"}),
    ).await;
    let id: String = serde_json::from_str::<serde_json::Value>(&create).unwrap()
        .get("id").unwrap().as_str().unwrap().to_string();
    let result = provider.handle_tool_call(
        "tdg_get_related",
        serde_json::json!({"node_id": id}),
    ).await;
    assert!(result.contains("relations"));
}

#[tokio::test]
async fn test_tdg_provider_shutdown() {
    let storage = test_storage_dir();
    let provider = hermes_core::TdgMemoryProvider::new(storage);
    provider.shutdown().await;
}

#[test]
fn test_build_memory_provider_tdg() {
    let storage = test_storage_dir();
    let provider = hermes_core::build_memory_provider("tdg", storage);
    assert_eq!(provider.name(), "tdg");
}

#[test]
fn test_build_memory_provider_default() {
    let storage = test_storage_dir();
    let provider = hermes_core::build_memory_provider("unknown", storage);
    assert_eq!(provider.name(), "builtin");
}

//! Integration tests for the operant-core tool system.
//!
//! These tests verify the public API end-to-end, covering:
//! - ToolRegistry registration, lookup, and lifecycle (Test 1)
//! - Tool schema generation and JSON Schema round-trip (Test 2)
//! - NotificationTool execution (Test 4)
//! - Schema consistency across all zero-dependency built-in tools (Test 5)
//!
//! Test 3 (cross-module composition) is skipped because the tool modules
//! are independent — the registry dispatches to individual tools without
//! chaining their execution.
//!
//! All tests use only the public API: `operant_core::tools::*`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::PathBuf;
use std::time::Duration;

use operant_core::tools::builtin::{
    ApprovalTool, CamofoxStateTool, CheckpointTool, ClarifyTool, CodeExecutionTool, DateTimeTool,
    DiscordAdminTool, DiscordTool, EnvVarTool, FileListTool, FileReadTool, FileSearchTool,
    FileStateTool, FileWriteTool, HomeAssistantTool, HttpRequestTool, ImageGenerationTool,
    MemoryRecallTool, MemorySearchTool, MemoryStoreTool, NeuttsSynthTool, NotificationTool,
    OpenRouterTool, OsvCheckTool, PatchTool, SendMessageTool, SkillViewTool, SkillsTool,
    SpotifyAlbumsTool, SpotifyDevicesTool, SpotifyLibraryTool, SpotifyPlaybackTool,
    SpotifyPlaylistsTool, SpotifyQueueTool, SpotifySearchTool, SystemInfoTool, TerminalTool,
    TimestampTool, TodoTool, ToolBackendTool, TranscriptionTool, TtsTool, VideoAnalysisTool,
    VisionTool, WebFetchTool, WebSearchTool, XaiHttpTool,
};
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry};

// =============================================================================
// Test 1: ToolRegistry Registration Flow
// =============================================================================

/// A helper that returns a registry pre-loaded with 6 zero-dependency tools.
async fn setup_registry() -> ToolRegistry {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(NotificationTool).await.unwrap();
    registry.register(DateTimeTool).await.unwrap();
    registry.register(TimestampTool).await.unwrap();
    registry.register(ClarifyTool).await.unwrap();
    registry.register(ApprovalTool).await.unwrap();
    registry.register(EnvVarTool).await.unwrap();
    registry
}

#[tokio::test]
async fn test_registry_empty_returns_empty() {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    assert_eq!(registry.len().await, 0);
    assert!(registry.is_empty().await);
}

#[tokio::test]
async fn test_registry_register_and_lookup() {
    let registry = setup_registry().await;

    // All registered tools should be found by name
    assert!(registry.contains("notify").await);
    assert!(registry.contains("datetime").await);
    assert!(registry.contains("timestamp").await);
    assert!(registry.contains("clarify").await);
    assert!(registry.contains("approval_request").await);
    assert!(registry.contains("debug_env").await);

    // Total count matches
    assert_eq!(registry.len().await, 6);
}

#[tokio::test]
async fn test_registry_get_returns_arc_tool() {
    let registry = setup_registry().await;

    let tool = registry.get("notify").await;
    assert!(tool.is_some());
    assert_eq!(tool.unwrap().name(), "notify");
}

#[tokio::test]
async fn test_registry_get_returns_none_for_unknown() {
    let registry = setup_registry().await;

    let tool = registry.get("nonexistent_tool").await;
    assert!(tool.is_none());
}

#[tokio::test]
async fn test_registry_duplicate_overwrites() {
    let registry = ToolRegistry::new(Duration::from_secs(5));

    registry.register(NotificationTool).await.unwrap();
    assert_eq!(registry.len().await, 1);

    // Register a different tool with same name would need a renamed tool,
    // which we can't construct here. Instead verify that re-registering
    // the same type keeps the count the same.
    registry.register(NotificationTool).await.unwrap();
    assert_eq!(registry.len().await, 1);
    assert!(registry.contains("notify").await);
}

#[tokio::test]
async fn test_registry_unregister() {
    let registry = setup_registry().await;
    assert!(registry.contains("notify").await);

    let removed = registry.unregister("notify").await;
    assert!(removed);
    assert!(!registry.contains("notify").await);
    assert_eq!(registry.len().await, 5);
}

#[tokio::test]
async fn test_registry_unregister_unknown() {
    let registry = setup_registry().await;

    let removed = registry.unregister("i_do_not_exist").await;
    assert!(!removed);
}

#[tokio::test]
async fn test_registry_case_sensitive_lookup() {
    let registry = setup_registry().await;

    // Registered as lowercase "notify", uppercase should not match
    assert!(registry.contains("notify").await);
    assert!(!registry.contains("NOTIFY").await);
    assert!(!registry.contains("Notify").await);
}

#[tokio::test]
async fn test_registry_get_schemas_returns_all() {
    let registry = setup_registry().await;

    let schemas = registry.get_schemas().await;
    assert_eq!(schemas.len(), 6);

    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"notify"));
    assert!(names.contains(&"datetime"));
    assert!(names.contains(&"timestamp"));
    assert!(names.contains(&"clarify"));
    assert!(names.contains(&"approval_request"));
    assert!(names.contains(&"debug_env"));
}

// =============================================================================
// Test 2: Tool Schema Round-Trip
// =============================================================================

#[tokio::test]
async fn test_schema_serializes_to_valid_json() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    for schema in &schemas {
        let json = serde_json::to_value(schema).expect("Schema must serialize");
        assert!(
            json.is_object(),
            "Schema for '{}' must be a JSON object",
            schema.name
        );
    }
}

#[tokio::test]
async fn test_schema_has_required_fields() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    for schema in &schemas {
        let json = serde_json::to_value(schema).expect("Schema must serialize to a JSON value");
        let obj = json
            .as_object()
            .expect("Schema JSON value should be a JSON object");

        assert!(
            obj.contains_key("name"),
            "Schema '{}' is missing 'name' field",
            schema.name
        );
        assert!(
            obj.contains_key("description"),
            "Schema '{}' is missing 'description' field",
            schema.name
        );
        assert!(
            obj.contains_key("parameters"),
            "Schema '{}' is missing 'parameters' field",
            schema.name
        );
    }
}

#[tokio::test]
async fn test_schema_name_matches_tool_name() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    // Build a set of expected tool names from the actual tool instances
    let expected_names: std::collections::HashSet<&str> = [
        NotificationTool.name(),
        DateTimeTool.name(),
        TimestampTool.name(),
        ClarifyTool.name(),
        ApprovalTool.name(),
        EnvVarTool.name(),
    ]
    .into_iter()
    .collect();

    // Every schema name must match one of the expected tool names
    for schema in &schemas {
        assert!(
            expected_names.contains(schema.name.as_str()),
            "Unexpected schema name '{}' — not in registered tool set",
            schema.name
        );
    }

    // Every registered tool must have a schema
    assert_eq!(schemas.len(), expected_names.len());
}

#[tokio::test]
async fn test_schema_has_non_empty_description() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    for schema in &schemas {
        assert!(
            !schema.description.is_empty(),
            "Schema for '{}' has empty description",
            schema.name
        );
    }
}

#[tokio::test]
async fn test_schema_parameters_is_valid_json_schema() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    for schema in &schemas {
        let params = &schema.parameters;
        assert!(
            params.is_object(),
            "Parameters for '{}' must be a JSON object",
            schema.name
        );

        // A valid JSON Schema for tool parameters should be an object type
        let params_obj = params.as_object().unwrap();
        if let Some(schema_type) = params_obj.get("type") {
            assert_eq!(
                schema_type.as_str(),
                Some("object"),
                "Schema '{}' parameters.type should be 'object'",
                schema.name
            );
        }
        // Every schema_from_type! generates at minimum { "type": "object" }
        // so `properties` may be absent but `type` should be present
        let has_type_or_properties =
            params_obj.contains_key("type") || params_obj.contains_key("properties");
        assert!(
            has_type_or_properties,
            "Schema '{}' parameters must have at least 'type' or 'properties'",
            schema.name
        );
    }
}

#[tokio::test]
async fn test_schema_deserialize_round_trip() {
    let registry = setup_registry().await;
    let schemas = registry.get_schemas().await;

    for original in &schemas {
        let json = serde_json::to_string(original).unwrap();
        let deserialized: operant_core::schema::ToolSchema =
            serde_json::from_str(&json).expect("Schema must deserialize cleanly");

        assert_eq!(deserialized.name, original.name);
        assert_eq!(deserialized.description, original.description);
        assert_eq!(deserialized.parameters, original.parameters);
    }
}

// =============================================================================
// Test 3: Cross-Module Composition
//
// The tool modules in operant-core are designed as independent units. Each tool
// is self-contained and the ToolRegistry dispatches to individual tools without
// chaining their execution (no tool A → tool B pipelines through the registry).
//
// While individual tools may internally call other services (e.g., file_state
// tracks file system state), this is an internal implementation detail not
// exposed through the public API as a composable cross-module flow.
//
// Therefore, cross-module integration tests are not applicable.
// =============================================================================

// =============================================================================
// Test 4: NotificationTool Execution
// =============================================================================

#[tokio::test]
async fn test_notification_tool_execute_success() {
    let tool = NotificationTool;
    let result = tool
        .execute(
            serde_json::json!({ "message": "Hello from integration test" }),
            ToolContext::default(),
        )
        .await;

    assert!(result.success, "NotificationTool should succeed");
}

#[tokio::test]
async fn test_notification_tool_returns_expected_content() {
    let tool = NotificationTool;
    let result = tool
        .execute(
            serde_json::json!({ "message": "test message" }),
            ToolContext::default(),
        )
        .await;

    assert!(result.success);

    let parsed: serde_json::Value =
        serde_json::from_str(&result.content).expect("Result content must be valid JSON");
    assert_eq!(parsed["success"], true);
    assert_eq!(parsed["message"], "test message");
    assert_eq!(parsed["delivered"], true);
}

#[tokio::test]
async fn test_notification_tool_with_all_fields() {
    let tool = NotificationTool;
    let result = tool
        .execute(
            serde_json::json!({
                "message": "urgent alert",
                "title": "Critical",
                "priority": "high"
            }),
            ToolContext::default(),
        )
        .await;

    assert!(result.success);
    let parsed: serde_json::Value =
        serde_json::from_str(&result.content).expect("Result content must be valid JSON");
    assert_eq!(parsed["message"], "urgent alert");
    assert_eq!(parsed["title"], "Critical");
}

#[tokio::test]
async fn test_notification_tool_missing_message_returns_error() {
    let tool = NotificationTool;
    let result = tool
        .execute(serde_json::json!({}), ToolContext::default())
        .await;

    assert!(!result.success, "Should fail without message");
    assert!(result.error.is_some());
    let err = result.error.unwrap();
    assert!(
        err.contains("message is required"),
        "Error should mention missing message, got: {}",
        err
    );
}

#[tokio::test]
async fn test_notification_tool_accepts_context_metadata() {
    let tool = NotificationTool;
    let context = ToolContext::default()
        .with_metadata("session_id", "test-123")
        .with_metadata("user", "integration-test");

    let result = tool
        .execute(serde_json::json!({ "message": "context test" }), context)
        .await;

    assert!(result.success, "Should succeed with context metadata");
}

// =============================================================================
// Test 5: Builtin Registration Consistency
// =============================================================================

/// Returns a list of all zero-dependency tools along with their expected names.
fn zero_dep_tools() -> Vec<(&'static str, Box<dyn OperantTool>)> {
    vec![
        ("notify", Box::new(NotificationTool)),
        ("approval_request", Box::new(ApprovalTool)),
        ("datetime", Box::new(DateTimeTool)),
        ("timestamp", Box::new(TimestampTool)),
        ("clarify", Box::new(ClarifyTool)),
        ("debug_env", Box::new(EnvVarTool)),
        ("debug_system", Box::new(SystemInfoTool)),
        ("send_message", Box::new(SendMessageTool)),
        ("file_read", Box::new(FileReadTool)),
        ("file_write", Box::new(FileWriteTool)),
        ("file_search", Box::new(FileSearchTool)),
        ("file_list", Box::new(FileListTool)),
        ("file_state", Box::new(FileStateTool)),
        ("terminal", Box::new(TerminalTool)),
        ("web_search", Box::new(WebSearchTool)),
        ("web_fetch", Box::new(WebFetchTool)),
        ("http_request", Box::new(HttpRequestTool)),
        ("patch", Box::new(PatchTool)),
        ("vision_analyze", Box::new(VisionTool)),
        ("code_execution", Box::new(CodeExecutionTool)),
        ("todo", Box::new(TodoTool)),
        ("memory_store", Box::new(MemoryStoreTool)),
        ("memory_search", Box::new(MemorySearchTool)),
        ("memory_recall", Box::new(MemoryRecallTool)),
        ("tool_backend", Box::new(ToolBackendTool)),
        ("browser_camofox_state", Box::new(CamofoxStateTool)),
        ("openrouter_query", Box::new(OpenRouterTool)),
        ("xai_http_request", Box::new(XaiHttpTool)),
        ("osv_check", Box::new(OsvCheckTool)),
        ("neutts_synthesize", Box::new(NeuttsSynthTool)),
        ("discord", Box::new(DiscordTool)),
        ("discord_admin", Box::new(DiscordAdminTool)),
        (
            "skill_view",
            Box::new(SkillViewTool::new(PathBuf::from("/tmp"))),
        ),
        ("spotify_playback", Box::new(SpotifyPlaybackTool)),
        ("spotify_devices", Box::new(SpotifyDevicesTool)),
        ("spotify_queue", Box::new(SpotifyQueueTool)),
        ("spotify_search", Box::new(SpotifySearchTool)),
        ("spotify_playlists", Box::new(SpotifyPlaylistsTool)),
        ("spotify_albums", Box::new(SpotifyAlbumsTool)),
        ("spotify_library", Box::new(SpotifyLibraryTool)),
        // Tools constructed via ::new()
        ("checkpoint", Box::new(CheckpointTool::new())),
        ("image_generate", Box::new(ImageGenerationTool::new())),
        ("text_to_speech", Box::new(TtsTool::new())),
        ("video_analyze", Box::new(VideoAnalysisTool::new())),
        ("transcribe_audio", Box::new(TranscriptionTool::new())),
        ("homeassistant", Box::new(HomeAssistantTool::new())),
        (
            "skills_list",
            Box::new(SkillsTool::new(PathBuf::from("/tmp"))),
        ),
    ]
}

#[test]
fn test_builtin_schema_name_matches_tool_name() {
    let mut failures = Vec::new();

    for (expected_name, tool) in zero_dep_tools() {
        let schema = tool.schema();
        if schema.name != expected_name {
            failures.push(format!(
                "{}: expected name '{}', got '{}'",
                expected_name, expected_name, schema.name
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Schema name mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_builtin_schema_serializes() {
    for (_name, tool) in zero_dep_tools() {
        let schema = tool.schema();
        let json = serde_json::to_value(&schema)
            .unwrap_or_else(|e| panic!("{} schema failed to serialize: {}", tool.name(), e));
        assert!(
            json.is_object(),
            "{} schema must serialize to an object",
            tool.name()
        );
    }
}

#[test]
fn test_builtin_schema_has_description() {
    for (_name, tool) in zero_dep_tools() {
        let schema = tool.schema();
        assert!(
            !schema.description.is_empty(),
            "Tool '{}' has empty description",
            tool.name()
        );
    }
}

#[test]
fn test_builtin_schema_parameters_are_object() {
    for (_name, tool) in zero_dep_tools() {
        let schema = tool.schema();
        assert!(
            schema.parameters.is_object(),
            "Tool '{}' parameters must be a JSON object, got {:?}",
            tool.name(),
            schema.parameters
        );
    }
}

#[test]
fn test_builtin_schema_is_valid_json_schema() {
    let mut issues = Vec::new();

    for (_name, tool) in zero_dep_tools() {
        let schema = tool.schema();
        let params = &schema.parameters;
        let params_obj = match params.as_object() {
            Some(obj) => obj,
            None => {
                issues.push(format!("{}: parameters is not an object", tool.name()));
                continue;
            }
        };

        // Must have either "type" or "$schema" as a valid JSON Schema
        if let Some(schema_type) = params_obj.get("type") {
            if schema_type.as_str() != Some("object") {
                issues.push(format!(
                    "{}: expected parameters.type = 'object', got {:?}",
                    tool.name(),
                    schema_type
                ));
            }
        } else {
            issues.push(format!("{}: parameters missing 'type' field", tool.name()));
        }

        // The schema must not crash when converted to JSON string
        if serde_json::to_string(&schema).is_err() {
            issues.push(format!(
                "{}: schema failed JSON string conversion",
                tool.name()
            ));
        }
    }

    assert!(
        issues.is_empty(),
        "JSON Schema validation issues:\n{}",
        issues.join("\n")
    );
}

#[test]
fn test_builtin_schema_name_uniqueness() {
    let mut names = std::collections::HashSet::new();
    let mut duplicates = Vec::new();

    for (_expected, tool) in zero_dep_tools() {
        let name = tool.name().to_string();
        if !names.insert(name.clone()) {
            duplicates.push(name);
        }
    }

    assert!(
        duplicates.is_empty(),
        "Duplicate tool names found: {:?}",
        duplicates
    );
}

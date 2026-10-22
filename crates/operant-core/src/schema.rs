//! JSON Schema generation for tool definitions
//!
//! Automatically generates OpenAI-compatible JSON Schema definitions from Rust structs
//! using the `schemars` crate.

use schemars::{JsonSchema, schema_for};
use serde_json::{Value, json};

use crate::error::{Error, Result};

/// Sanitize a tool name to be compatible with provider APIs (Google, OpenAI).
///
/// Google requires: `[a-zA-Z_][a-zA-Z0-9_.:-]{0,127}`
/// This function replaces invalid characters with `_`, ensures the name starts
/// with a letter or underscore, and truncates to 128 characters.
pub fn sanitize_tool_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == ':' || ch == '-' {
            result.push(ch);
        } else {
            result.push('_');
        }
    }
    // Ensure starts with letter or underscore
    if result.is_empty()
        || (!result.as_bytes()[0].is_ascii_alphabetic() && result.as_bytes()[0] != b'_')
    {
        result.insert(0, '_');
    }
    // Truncate to 128 chars
    if result.len() > 128 {
        result.truncate(128);
    }
    result
}

/// Description of the shared `intent` parameter, injected into every tool schema.
///
/// Deliberately terse and name-free: this string rides on EVERY tool schema on
/// EVERY request, so each word is paid for as tokens forever. The full
/// explanation lives in the renderer that consumes it.
///
/// Ported verbatim from jcode's `TOOL_INTENT_DESCRIPTION`
/// (`jcode-tool-core/src/lib.rs:17-19`) — upstream uses the identical wording.
pub const TOOL_INTENT_DESCRIPTION: &str =
    "Required short label shown in the UI: why this call is being made.";

/// The parameter name the model writes its intent into. Display-only in this
/// tree (the TUI and the gateway read it); the tool itself ignores it.
pub const TOOL_INTENT_KEY: &str = "intent";

/// JSON Schema fragment for the `intent` property.
pub fn intent_schema_property() -> Value {
    json!({
        "type": "string",
        "description": TOOL_INTENT_DESCRIPTION,
    })
}

/// Ensure a tool parameter schema declares the shared `intent` property and
/// marks it required.
///
/// Applied centrally at the registry boundary rather than per tool, so every
/// tool — including MCP proxies whose schemas arrive from a foreign server —
/// asks the model for an intent without each tool wiring it manually. The
/// MCP proxy strips the parameter before forwarding (see
/// `tools/mcp_tool.rs`), so an undeclared parameter never reaches a server.
///
/// Ported from jcode's `ensure_intent_in_schema`
/// (`jcode-tool-core/src/lib.rs:48-90`).
pub fn ensure_intent_in_schema(mut schema: Value) -> Value {
    let Some(object) = schema.as_object_mut() else {
        return schema;
    };
    // Only touch object-shaped parameter schemas. A schema with no `type` and
    // no `properties` is not a parameter schema we can usefully extend.
    let is_object_schema = object
        .get("type")
        .and_then(|t| t.as_str())
        .is_some_and(|t| t == "object")
        || object.contains_key("properties");
    if !is_object_schema {
        return schema;
    }

    let Some(properties) = object
        .entry("properties")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
    else {
        return schema;
    };
    properties
        .entry(TOOL_INTENT_KEY)
        .or_insert_with(intent_schema_property);

    match object.get_mut("required") {
        Some(Value::Array(required)) => {
            if !required.iter().any(|v| v.as_str() == Some(TOOL_INTENT_KEY)) {
                required.push(Value::String(TOOL_INTENT_KEY.to_string()));
            }
        }
        _ => {
            object.insert(
                "required".to_string(),
                Value::Array(vec![Value::String(TOOL_INTENT_KEY.to_string())]),
            );
        }
    }

    schema
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSchema {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: sanitize_tool_name(&name.into()),
            description: description.into(),
            parameters,
        }
    }

    pub fn from_type<T: JsonSchema>(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let parameters = schema_for!(T);
        let parameters_value = serde_json::to_value(&parameters).unwrap_or_else(|_| {
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        });

        Self {
            name: sanitize_tool_name(&name.into()),
            description: description.into(),
            parameters: parameters_value,
        }
    }

    /// Attach the shared `intent` property to this schema's parameters and mark
    /// it required, so every model call is asked for a display label.
    ///
    /// Idempotent: calling it on an already-augmented schema is a no-op.
    pub fn with_intent(mut self) -> Self {
        self.parameters = ensure_intent_in_schema(self.parameters);
        self
    }

    #[expect(
        clippy::expect_used,
        reason = "is_string() was checked above, so as_str() is Some"
    )]
    /// Sanitize and repair input arguments against the schema properties and required fields.
    /// This fixes cases where the model or gateway proxy outputs type mismatches (e.g. empty objects
    /// `{}` for optional fields like integer `num_results` or array `tags`) or misses/misnames required fields.
    pub fn sanitize_args(&self, args: &mut Value) {
        // 1. If args is a string but the schema expects an object, try to wrap it if there is exactly 1 required field.
        if args.is_string()
            && let Some(required) = self.parameters.get("required").and_then(|r| r.as_array())
            && required.len() == 1
            && let Some(req_key) = required[0].as_str()
        {
            // Coerce gracefully: only treat args as a string when
            // it actually is one (no panicking on mismatched input).
            if let Some(str_val) = args.as_str() {
                *args = json!({ req_key: str_val });
            }
        }

        // 2. If args is an object, clean type mismatches and handle missing required fields via aliases.
        if let Some(args_map) = args.as_object_mut() {
            let properties = self
                .parameters
                .get("properties")
                .and_then(|p| p.as_object());
            let required_fields: std::collections::HashSet<&str> = self
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            // Helpers for case conversion
            let to_snake_case = |s: &str| -> String {
                let mut snake = String::new();
                for ch in s.chars() {
                    if ch.is_uppercase() {
                        if !snake.is_empty() {
                            snake.push('_');
                        }
                        snake.extend(ch.to_lowercase());
                    } else {
                        snake.push(ch);
                    }
                }
                snake
            };

            let to_camel_case = |s: &str| -> String {
                let mut camel = String::new();
                let mut next_upper = false;
                for ch in s.chars() {
                    if ch == '_' {
                        next_upper = true;
                    } else if next_upper {
                        camel.extend(ch.to_uppercase());
                        next_upper = false;
                    } else {
                        camel.push(ch);
                    }
                }
                camel
            };

            // 2a. Map aliases for missing required fields
            for &req_key in &required_fields {
                if !args_map.contains_key(req_key) {
                    let mut found_val = None;
                    let mut key_to_remove = None;

                    let camel = to_camel_case(req_key);
                    let snake = to_snake_case(req_key);

                    if args_map.contains_key(&camel) {
                        key_to_remove = Some(camel);
                    } else if args_map.contains_key(&snake) {
                        key_to_remove = Some(snake);
                    } else {
                        // Check common generic aliases
                        let generic_aliases = match req_key {
                            "query" => vec![
                                "q",
                                "search",
                                "text",
                                "question",
                                "query_str",
                                "topic",
                                "payload",
                            ],
                            "key" => vec!["k", "name", "id", "search_key"],
                            "content" => vec!["text", "body", "message", "data"],
                            "action" => vec!["act", "command", "cmd", "operation", "op"],
                            _ => vec![],
                        };
                        for alias in generic_aliases {
                            if args_map.contains_key(alias) {
                                key_to_remove = Some(alias.to_string());
                                break;
                            }
                            let alias_camel = to_camel_case(alias);
                            if args_map.contains_key(&alias_camel) {
                                key_to_remove = Some(alias_camel);
                                break;
                            }
                            let alias_snake = to_snake_case(alias);
                            if args_map.contains_key(&alias_snake) {
                                key_to_remove = Some(alias_snake);
                                break;
                            }
                        }
                    }

                    if let Some(k) = key_to_remove {
                        found_val = args_map.remove(&k);
                    }

                    if let Some(val) = found_val {
                        args_map.insert(req_key.to_string(), val);
                    }
                }
            }

            // 2b. Clean type mismatches (e.g. optional fields initialized as empty objects `{}` by gateways)
            if let Some(props) = properties {
                let mut keys_to_remove = Vec::new();
                for (key, val) in args_map.iter_mut() {
                    let prop_schema = props
                        .get(key)
                        .or_else(|| props.get(&to_camel_case(key)))
                        .or_else(|| props.get(&to_snake_case(key)));

                    if let Some(prop_schema) = prop_schema {
                        let mut expected_types = Vec::new();
                        if let Some(t) = prop_schema.get("type") {
                            if let Some(s) = t.as_str() {
                                expected_types.push(s);
                            } else if let Some(arr) = t.as_array() {
                                for item in arr {
                                    if let Some(s) = item.as_str() {
                                        expected_types.push(s);
                                    }
                                }
                            }
                        }

                        let expects_object =
                            expected_types.contains(&"object") || expected_types.is_empty();
                        if val.is_object() && !expects_object {
                            // Mismatch! e.g. expected array or integer but got object.
                            // Remove empty objects or non-required mismatched objects.
                            if val.as_object().is_some_and(|o| o.is_empty())
                                || !required_fields.contains(key.as_str())
                            {
                                keys_to_remove.push(key.clone());
                            }
                        }

                        // 2c. Type coercion for primitive types (string -> integer, string -> array)
                        if val.is_string() {
                            let s = val.as_str().expect("is_string() check guarantees as_str()");
                            // Try to coerce string to integer if expected type is integer
                            if expected_types.contains(&"integer")
                                || expected_types.contains(&"number")
                            {
                                if let Ok(parsed) = s.parse::<serde_json::Number>() {
                                    *val = Value::Number(parsed);
                                }
                            }
                            // Try to coerce string to array if expected type is array
                            else if expected_types.contains(&"array") {
                                // Try to parse as JSON array first
                                if let Ok(parsed) = serde_json::from_str::<Value>(s)
                                    && parsed.is_array()
                                {
                                    *val = parsed;
                                }
                            }
                        }
                    }
                }

                for key in keys_to_remove {
                    args_map.remove(&key);
                }
            }
        }
    }

    /// Validate arguments against the schema
    pub fn validate_args(&self, args: &Value) -> Result<()> {
        // Basic validation - check if args is an object
        if !args.is_object() {
            return Err(Error::InvalidToolArgs {
                name: self.name.clone(),
                details: "Arguments must be a JSON object".to_string(),
            });
        }

        // For now, we do basic structural validation
        // A full JSON Schema validator would be more robust
        if let Some(required) = self.parameters.get("required").and_then(|r| r.as_array()) {
            for req_field in required {
                if let Some(key) = req_field.as_str()
                    && args.get(key).is_none()
                {
                    return Err(Error::InvalidToolArgs {
                        name: self.name.clone(),
                        details: format!("Missing required field: {}", key),
                    });
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[derive(JsonSchema)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "test-only schema struct")]
    struct TestParams {
        query: String,
        limit: Option<i32>,
    }

    #[test]
    fn test_schema_generation() {
        let schema = ToolSchema::from_type::<TestParams>("test_tool", "A test tool");

        assert_eq!(schema.name, "test_tool");
        assert_eq!(schema.description, "A test tool");
        assert!(schema.parameters.is_object());
    }

    #[test]
    fn test_sanitize_args() {
        let schema = ToolSchema::from_type::<TestParams>("test_tool", "A test tool");

        // 1. Primitive/object mismatch: optional limit field set to {} should be removed
        let mut args1 = json!({
            "query": "hello",
            "limit": {}
        });
        schema.sanitize_args(&mut args1);
        assert_eq!(args1, json!({ "query": "hello" }));

        // 2. Alias mapping: 'q' should be mapped to 'query'
        let mut args2 = json!({
            "q": "search term",
            "limit": 10
        });
        schema.sanitize_args(&mut args2);
        assert_eq!(args2, json!({ "query": "search term", "limit": 10 }));

        // 3. String wrapping: single string should be wrapped to {"query": "text"}
        let mut args3 = json!("raw string");
        schema.sanitize_args(&mut args3);
        assert_eq!(args3, json!({ "query": "raw string" }));
    }

    #[test]
    fn test_sanitize_args_type_coercion() {
        // Test with integer and array fields
        #[derive(JsonSchema)]
        #[serde(rename_all = "camelCase")]
        #[expect(dead_code, reason = "test-only schema struct")]
        struct CoercionParams {
            query: String,
            limit: Option<i32>,
            tags: Option<Vec<String>>,
        }

        let schema = ToolSchema::from_type::<CoercionParams>("test_tool", "A test tool");

        // 1. String -> integer coercion
        let mut args1 = json!({
            "query": "test",
            "limit": "42"
        });
        schema.sanitize_args(&mut args1);
        assert_eq!(args1["limit"], json!(42));
        assert!(args1["limit"].is_number());

        // 2. String -> array coercion (JSON array string)
        let mut args2 = json!({
            "query": "test",
            "tags": "[\"rust\", \"web\"]"
        });
        schema.sanitize_args(&mut args2);
        assert_eq!(args2["tags"], json!(["rust", "web"]));
        assert!(args2["tags"].is_array());

        // 3. String -> integer with "number" type (not just "integer")
        #[derive(JsonSchema)]
        #[serde(rename_all = "camelCase")]
        #[expect(dead_code, reason = "test-only schema struct")]
        struct NumberParams {
            query: String,
            threshold: Option<f64>,
        }
        let num_schema = ToolSchema::from_type::<NumberParams>("test_tool", "A test tool");
        let mut args3 = json!({
            "query": "test",
            "threshold": "1.5"
        });
        num_schema.sanitize_args(&mut args3);
        assert_eq!(args3["threshold"], json!(1.5));
        assert!(args3["threshold"].is_number());
    }

    // ── the shared `intent` parameter ───────────────────────────────────

    #[test]
    fn ensure_intent_injects_property_and_marks_it_required() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        });
        let out = ensure_intent_in_schema(schema);
        assert_eq!(
            out["properties"]["intent"],
            intent_schema_property(),
            "the intent property is injected"
        );
        assert_eq!(out["properties"]["intent"]["type"], json!("string"));
        assert_eq!(
            out["required"],
            json!(["command", "intent"]),
            "required keeps its existing entries and gains intent"
        );
    }

    #[test]
    fn ensure_intent_creates_a_required_list_when_the_schema_has_none() {
        let schema = json!({
            "type": "object",
            "properties": { "command": { "type": "string" } }
        });
        let out = ensure_intent_in_schema(schema);
        assert_eq!(out["required"], json!(["intent"]));
    }

    #[test]
    fn ensure_intent_is_idempotent() {
        let once = ensure_intent_in_schema(json!({
            "type": "object",
            "properties": {},
            "required": ["command"]
        }));
        let twice = ensure_intent_in_schema(once.clone());
        assert_eq!(once, twice, "a second pass must be a no-op");
        assert_eq!(
            twice["required"],
            json!(["command", "intent"]),
            "and intent must not be duplicated into required"
        );
    }

    /// A tool whose own schema ALREADY declares `intent` keeps its description:
    /// the injector fills in what a tool left out, it never overwrites it.
    #[test]
    fn ensure_intent_never_overwrites_an_existing_property() {
        let schema = json!({
            "type": "object",
            "properties": { "intent": {"type": "string", "description": "the tool's own"} }
        });
        let out = ensure_intent_in_schema(schema);
        assert_eq!(
            out["properties"]["intent"]["description"],
            json!("the tool's own")
        );
    }

    #[test]
    fn ensure_intent_leaves_a_non_object_schema_alone() {
        // A schema with neither an object type nor any properties is not a
        // parameter schema this can usefully extend.
        assert_eq!(ensure_intent_in_schema(json!("string")), json!("string"));
        assert_eq!(ensure_intent_in_schema(json!(null)), json!(null));
        // No `type` at all, but it has properties — still a parameter schema.
        let schemaless = ensure_intent_in_schema(json!({
            "properties": { "command": { "type": "string" } }
        }));
        assert_eq!(schemaless["required"], json!(["intent"]));
    }

    #[test]
    fn tool_schema_with_intent_augments_the_generated_parameters() {
        let schema = ToolSchema::from_type::<TestParams>("test_tool", "A test tool").with_intent();
        assert_eq!(
            schema.parameters["properties"]["intent"]["description"],
            json!(TOOL_INTENT_DESCRIPTION),
            "a generated schema exposes the intent property"
        );
        let required = schema.parameters["required"].as_array().expect("required");
        assert!(required.iter().any(|v| v.as_str() == Some("intent")));
    }
}

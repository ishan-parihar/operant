//! JSON Schema sanitizer — removes dangerous or malformed properties from
//! tool parameter schemas for broad LLM-backend compatibility.
//!
//! Some local inference backends (notably llama.cpp's
//! `json-schema-to-grammar` converter) are strict about which JSON Schema
//! shapes they accept.  This module walks a schema tree and removes
//! properties that are known to cause failures.

use serde_json::Value;

/// Maximum recursion depth to prevent stack overflow on deeply nested schemas.
const MAX_RECURSION_DEPTH: usize = 128;

/// Property names whose values should be checked for dangerous content at the
/// string leaf level — these are sanitised by `sanitize_schema`.
const DANGEROUS_VALUE_KEYS: &[&str] = &["enum", "default", "const"];

/// Schema-level property names that are stripped unconditionally when they
/// contain code-or-execution-risk content.
pub const SANITIZED_PROPERTIES: &[&str] = &["$comment", "examples"];

/// Keys whose `pattern` string values are checked for dangerous regex.
const PATTERN_KEYS: &[&str] = &["pattern"];

/// Check whether a string value contains shell-metacharacters or dangerous
/// patterns that could lead to code injection.
///
/// Returns `true` if the value contains any of:
/// - Shell metacharacters: `` & | ; $ ` \ { } ( ) < > `` (outside of
///   common / cross-platform punctuation).
/// - Subshell invocation: `$(`, `` ` `` (backtick).
/// - Line-continuation / escape sequences that could mask injection.
pub fn is_dangerous_value(value: &str) -> bool {
    // Shell metacharacters that should never appear in trusted schema values.
    // Note: `$` alone is excluded because it is a valid regex anchor (`^[a-z]+$`);
    // only `$(` (command substitution) is flagged.
    let dangerous_chars = &['&', '|', ';', '`', '\\', '{', '}', '(', ')', '<', '>'];
    if value.contains(dangerous_chars) {
        return true;
    }
    // Subshell patterns — `$(` is the dangerous form, not bare `$`.
    if value.contains("$(") {
        return true;
    }
    if value.contains('`') {
        return true;
    }
    // Shell injection via newlines / carriage returns.
    if value.contains('\n') || value.contains('\r') {
        return true;
    }
    false
}

/// Check if a string value contains "exec" or "eval" as a word (substring
/// match is sufficient — these are so rarely legitimate schema values that
/// simple containment is the safer heuristic).
fn contains_exec_or_eval(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("exec") || lower.contains("eval")
}

/// Check whether a string is "code-like" — heuristic for `$comment` and
/// `examples` fields that embed source code rather than prose.
fn is_code_like(value: &str) -> bool {
    // Common code indicators.
    if value.contains("function ") || value.contains("def ") || value.contains("=>") {
        return true;
    }
    if value.contains("import ") || value.contains("require(") || value.contains("#include") {
        return true;
    }
    if value.contains("console.") || value.contains("System.") || value.contains("std::") {
        return true;
    }
    if value.contains("```") || value.contains("    ") {
        // Code blocks or indented code.
        return true;
    }
    false
}

/// Recursively sanitize a JSON Schema value in-place.
///
/// 1. Removes `"exec"` / `"eval"` from `enum`, `default`, and `const` arrays.
/// 2. Strips `$comment` and `examples` fields when they contain code.
/// 3. Removes `pattern` values that contain dangerous regex / shell patterns.
/// 4. Recurses into `properties`, `items`, `additionalProperties`, `anyOf`,
///    `oneOf`, `allOf`, and `$defs` / `definitions`.
///
/// Returns `Ok(true)` if the schema was modified, `Ok(false)` if no changes
/// were needed, or `Err` if the recursion depth limit is exceeded.
pub fn sanitize_schema(schema: &mut Value) -> Result<bool, String> {
    sanitize_node(schema, 0)
}

/// Internal recursive node sanitizer.  Tracks `depth` to enforce recursion
/// limit.
fn sanitize_node(node: &mut Value, depth: usize) -> Result<bool, String> {
    if depth > MAX_RECURSION_DEPTH {
        return Err("Max recursion depth exceeded in schema sanitizer".into());
    }

    let mut modified = false;

    match node {
        Value::Object(map) => {
            // Collect keys to avoid borrow issues.
            let keys: Vec<String> = map.keys().cloned().collect();

            for key in &keys {
                // ── Handle enum / default / const — filter dangerous values ──
                if DANGEROUS_VALUE_KEYS.contains(&key.as_str()) {
                    if let Some(arr) = map.get(key).and_then(|v| v.as_array()) {
                        // Array case: enum values.
                        let filtered: Vec<Value> = arr
                            .iter()
                            .filter(|item| {
                                if let Value::String(s) = item {
                                    !(contains_exec_or_eval(s) || is_dangerous_value(s))
                                } else {
                                    true
                                }
                            })
                            .cloned()
                            .collect();
                        if filtered.len() != arr.len() {
                            modified = true;
                            if filtered.is_empty() {
                                map.remove(key.as_str());
                            } else {
                                map.insert(key.clone(), Value::Array(filtered));
                            }
                        }
                    } else if let Some(Value::String(s)) = map.get(key).map(|v| v.clone()) {
                        // Scalar string case: default / const values.
                        if contains_exec_or_eval(&s) || is_dangerous_value(&s) {
                            map.remove(key.as_str());
                            modified = true;
                        }
                    }
                    continue;
                }

                // ── Strip $comment / examples when they contain code ──
                if SANITIZED_PROPERTIES.contains(&key.as_str()) {
                    if should_strip_field(map.get(key.as_str())) {
                        map.remove(key.as_str());
                        modified = true;
                    }
                    continue;
                }

                // ── Pattern keys — remove dangerous regex patterns ──
                if PATTERN_KEYS.contains(&key.as_str()) {
                    if let Some(Value::String(pat)) = map.get(key.as_str()) {
                        if is_dangerous_value(pat) || contains_exec_or_eval(pat) {
                            map.remove(key.as_str());
                            modified = true;
                        }
                    }
                    continue;
                }

                // ── Recurse into container schemas ──
                let recurse_keys = [
                    "properties",
                    "$defs",
                    "definitions",
                    "items",
                    "additionalProperties",
                ];
                if recurse_keys.contains(&key.as_str()) {
                    if let Some(val) = map.get_mut(key.as_str()) {
                        if sanitize_node(val, depth + 1)? {
                            modified = true;
                        }
                    }
                    continue;
                }

                // ── Recurse into combinator arrays ──
                let combinator_keys = ["anyOf", "oneOf", "allOf"];
                if combinator_keys.contains(&key.as_str()) {
                    if let Some(Value::Array(arr)) = map.get_mut(key.as_str()) {
                        for item in arr.iter_mut() {
                            if sanitize_node(item, depth + 1)? {
                                modified = true;
                            }
                        }
                    }
                    continue;
                }

                // ── Recurse into any nested object or array ──
                if let Some(val) = map.get_mut(key.as_str()) {
                    if sanitize_node(val, depth + 1)? {
                        modified = true;
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                if sanitize_node(item, depth + 1)? {
                    modified = true;
                }
            }
        }
        Value::String(s) => {
            // If a bare string is in a schema position, it's malformed.
            // But at this level we don't know the context — the parent
            // handles replacement.
            if contains_exec_or_eval(s) || is_dangerous_value(s) {
                modified = true;
            }
        }
        _ => {}
    }

    Ok(modified)
}

/// Decide whether a schema field value should be stripped.
fn should_strip_field(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(s)) => is_code_like(s),
        Some(Value::Array(arr)) => arr.iter().any(|v| {
            if let Value::String(s) = v {
                is_code_like(s)
            } else {
                false
            }
        }),
        _ => false,
    }
}

/// Sanitize an OpenAI-format tool list.
///
/// Input is an array of tool definitions:
/// ```json
/// [{"type": "function", "function": {"name": ..., "parameters": {...}}}]
/// ```
///
/// Each tool's `parameters` schema is sanitized in-place.
pub fn sanitize_tool_schemas(tools: &mut Vec<Value>) -> Result<(), String> {
    for tool in tools.iter_mut() {
        if let Some(func) = tool.get_mut("function") {
            if let Some(params) = func.get_mut("parameters") {
                sanitize_schema(params)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_dangerous_value_shell_chars() {
        assert!(is_dangerous_value("hello; world"));
        assert!(is_dangerous_value("$(cat /etc/passwd)"));
        assert!(is_dangerous_value("`ls`"));
        assert!(is_dangerous_value("a|b"));
        assert!(is_dangerous_value("a&b"));
    }

    #[test]
    fn test_is_dangerous_value_safe() {
        assert!(!is_dangerous_value("hello world"));
        assert!(!is_dangerous_value("simple-string"));
        assert!(!is_dangerous_value("2024-01-01"));
        assert!(!is_dangerous_value("user@example.com"));
    }

    #[test]
    fn test_sanitize_enum_removes_exec_eval() {
        let mut schema = json!({
            "type": "string",
            "enum": ["ls", "exec", "cat", "eval"]
        });
        sanitize_schema(&mut schema).unwrap();
        let enm = schema["enum"].as_array().unwrap();
        assert_eq!(enm.len(), 2);
        assert_eq!(enm[0], "ls");
        assert_eq!(enm[1], "cat");
    }

    #[test]
    fn test_sanitize_enum_empty_becomes_absent() {
        let mut schema = json!({
            "type": "string",
            "enum": ["exec", "eval"]
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("enum").is_none());
    }

    #[test]
    fn test_sanitize_default_removes_dangerous() {
        let mut schema = json!({
            "type": "string",
            "default": "exec ls -la"
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("default").is_none());
    }

    #[test]
    fn test_sanitize_const_removes_dangerous() {
        let mut schema = json!({
            "type": "string",
            "const": "eval(something)"
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("const").is_none());
    }

    #[test]
    fn test_sanitize_comment_stripped_when_code() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "$comment": "function foo() { return 1; }"
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("$comment").is_none());
    }

    #[test]
    fn test_sanitize_comment_preserved_when_prose() {
        let mut schema = json!({
            "type": "object",
            "$comment": "This is a helpful note about the schema"
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("$comment").is_some());
    }

    #[test]
    fn test_sanitize_examples_stripped_when_code() {
        let mut schema = json!({
            "type": "string",
            "examples": ["hello", "import os; os.system('ls')"]
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("examples").is_none());
    }

    #[test]
    fn test_sanitize_pattern_dangerous_removed() {
        let mut schema = json!({
            "type": "string",
            "pattern": "^[a-z]+$"
        });
        sanitize_schema(&mut schema).unwrap();
        assert!(schema.get("pattern").is_some());

        let mut schema2 = json!({
            "type": "string",
            "pattern": "^(.*)$(ls)"
        });
        sanitize_schema(&mut schema2).unwrap();
        assert!(schema2.get("pattern").is_none());
    }

    #[test]
    fn test_sanitize_recursion_depth_guard() {
        // Build a deeply nested schema.
        let mut schema = json!({"type": "object", "properties": {}});
        let mut current = &mut schema;
        for _ in 0..130 {
            let inner = json!({"type": "object", "properties": {}});
            current["properties"] = json!({"nested": inner});
            current = &mut current["properties"]["nested"];
        }
        let result = sanitize_schema(&mut schema);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("recursion depth"));
    }

    #[test]
    fn test_sanitize_tool_schemas() {
        let mut tools = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "test_tool",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "cmd": {
                                "type": "string",
                                "enum": ["ls", "exec", "cat"]
                            }
                        }
                    }
                }
            }),
        ];
        sanitize_tool_schemas(&mut tools).unwrap();
        let params = &tools[0]["function"]["parameters"]["properties"]["cmd"];
        let enm = params["enum"].as_array().unwrap();
        assert_eq!(enm.len(), 2);
        assert_eq!(enm[0], "ls");
        assert_eq!(enm[1], "cat");
    }

    #[test]
    fn test_sanitize_nested_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["exec", "run", "eval"]
                        }
                    }
                }
            }
        });
        sanitize_schema(&mut schema).unwrap();
        let action = &schema["properties"]["command"]["properties"]["action"];
        let enm = action["enum"].as_array().unwrap();
        assert_eq!(enm.len(), 1);
        assert_eq!(enm[0], "run");
    }

    #[test]
    fn test_sanitize_anyof_recurse() {
        let mut schema = json!({
            "anyOf": [
                {"type": "string", "enum": ["exec", "safe"]},
                {"type": "integer"}
            ]
        });
        sanitize_schema(&mut schema).unwrap();
        let first = &schema["anyOf"][0];
        let enm = first["enum"].as_array().unwrap();
        assert_eq!(enm.len(), 1);
        assert_eq!(enm[0], "safe");
    }

    #[test]
    fn test_sanitize_noop_on_clean_schema() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"}
            },
            "required": ["name"]
        });
        let result = sanitize_schema(&mut schema).unwrap();
        assert!(!result);
    }
}

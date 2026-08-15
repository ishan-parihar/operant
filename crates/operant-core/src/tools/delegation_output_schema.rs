//! Structured-output schema helpers for `delegate_task` — hermes
//! `tools/delegation_output_schema.py` parity (T1-24).
//!
//! Optional per-task `output_schema` (a JSON Schema object): the child is
//! told about the contract via an OUTPUT CONTRACT block appended to its
//! context, the parent validates the child's final answer with a lightweight
//! JSON-Schema subset validator, and on failure sends exactly ONE bounded
//! retry turn carrying the validation errors verbatim (max 1 retry, exact
//! errors, no schema re-paste — per llm-structured-output-schema-design).
//!
//! The validator implements the JSON-Schema subset that matters for
//! structured delegation: root `type`, per-property `type`, `required`,
//! nested `properties` recursion, and `items` for arrays. A full draft
//! validator is deliberately NOT pulled in — operant principle: no heavy
//! deps for minor convenience.

use serde_json::Value;

/// Exactly one retry turn — bounded by design. More retries make frontier
/// models drop fields that were right the first time.
pub const MAX_SCHEMA_RETRIES: usize = 1;

/// How many validation errors to render into a retry/notice message.
const MAX_RENDERED_ERRORS: usize = 10;

const CONTRACT_HEADER: &str = "OUTPUT CONTRACT (machine-validated)";

/// Validate a model/caller-supplied `output_schema` value.
///
/// Returns `Ok(Some(schema))` when usable, `Ok(None)` when no schema was
/// requested (`None` input passes through), and `Err(error)` when the value
/// is not a usable JSON Schema object.
pub fn coerce_output_schema(raw: Option<Value>) -> Result<Option<Value>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut value = raw;
    if let Some(s) = value.as_str() {
        // Models sometimes double-encode the schema as a JSON string.
        value = serde_json::from_str(s)
            .map_err(|_| "output_schema must be a JSON Schema object, got a non-JSON string.")?;
    }
    if !value.is_object() {
        return Err(format!(
            "output_schema must be a JSON Schema object, got {}.",
            json_kind(&value)
        ));
    }
    Ok(Some(value))
}

/// Append the explicit output contract block to a child's context.
pub fn append_output_contract(context: Option<&str>, schema: &Value) -> String {
    let schema_text = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
    let block = format!(
        "{CONTRACT_HEADER}:\n\
         Your FINAL response must be a single JSON object that validates \
         against this JSON Schema. No prose before or after the JSON; a \
         ```json code fence is acceptable but not required.\n\
         {schema_text}"
    );
    match context {
        Some(base) if !base.trim().is_empty() => format!("{base}\n\n{block}"),
        _ => block,
    }
}

/// Best-effort extraction of a JSON payload from model output.
///
/// Strips markdown code fences and leading/trailing prose around the
/// outermost `{...}` / `[...]` span. Returns the (possibly unchanged)
/// candidate string; parsing errors are reported by [`validate_output`].
pub fn extract_json_candidate(text: &str) -> String {
    let mut raw = text.trim().to_string();
    if raw.starts_with("```") {
        if let Some(idx) = raw.find('\n') {
            raw = raw[idx + 1..].to_string();
        }
        if raw.trim_end().ends_with("```") {
            let trimmed = raw.trim_end();
            raw = trimmed[..trimmed.len() - 3].to_string();
        }
        raw = raw.trim().to_string();
        // A `json` language tag on its own line (``` then json then content).
        if raw.to_ascii_lowercase().starts_with("json\n") {
            raw = raw["json\n".len()..].to_string();
        }
    }
    for (opener, closer) in [('{', '}'), ('[', ']')] {
        let start = raw.find(opener);
        let end = raw.rfind(closer);
        if let (Some(s), Some(e)) = (start, end)
            && e > s
        {
            return raw[s..=e].to_string();
        }
    }
    raw
}

/// Validate a child's final answer against `schema`.
///
/// Returns `(true, [])` on success or `(false, errors)` where errors are
/// human-readable strings suitable for the retry turn (hermes
/// `validate_output` parity, `$.path`-style rendering, bounded volume).
pub fn validate_output(text: &str, schema: &Value) -> (bool, Vec<String>) {
    let candidate = extract_json_candidate(text);
    if candidate.trim().is_empty() {
        return (
            false,
            vec!["Response was empty — expected a JSON object matching the schema.".to_string()],
        );
    }
    let parsed: Value = match serde_json::from_str(&candidate) {
        Ok(value) => value,
        Err(error) => {
            return (false, vec![format!("Response is not valid JSON: {error}")]);
        }
    };
    let mut errors = Vec::new();
    validate_value(&parsed, schema, "$", &mut errors, 0);
    if errors.is_empty() {
        (true, Vec::new())
    } else {
        (false, errors)
    }
}

/// Maximum schema nesting depth the validator will recurse into — hardens
/// against adversarial deeply-nested schemas overflowing the stack.
const MAX_VALIDATION_DEPTH: usize = 16;

/// Lightweight JSON-Schema subset validation: root `type`, `required`,
/// per-property `type`, nested `properties` recursion, and `items` for
/// arrays. Errors rendered as `$.prop[0]`-style paths, bounded to
/// [`MAX_RENDERED_ERRORS`] so a retry prompt never blows up.
fn validate_value(
    value: &Value,
    schema: &Value,
    path: &str,
    errors: &mut Vec<String>,
    depth: usize,
) {
    if errors.len() >= MAX_RENDERED_ERRORS || depth > MAX_VALIDATION_DEPTH {
        return;
    }
    if let Some(type_name) = schema.get("type").and_then(|t| t.as_str())
        && !type_matches(value, type_name)
    {
        errors.push(format!("{path}: value is not of type '{type_name}'"));
        return; // wrong type at this node is fatal for the subtree
    }
    match value {
        Value::Object(obj) => {
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                for key in required {
                    if let Some(key) = key.as_str()
                        && !obj.contains_key(key)
                    {
                        errors.push(format!("{path}: missing required property '{key}'"));
                    }
                }
            }
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                for (key, prop_schema) in properties {
                    if let Some(child) = obj.get(key) {
                        validate_value(
                            child,
                            prop_schema,
                            &format!("{path}.{key}"),
                            errors,
                            depth + 1,
                        );
                    }
                }
            }
        }
        Value::Array(arr) => {
            if let Some(items) = schema.get("items") {
                for (i, item) in arr.iter().enumerate() {
                    if errors.len() >= MAX_RENDERED_ERRORS {
                        return;
                    }
                    validate_value(item, items, &format!("{path}[{i}]"), errors, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn type_matches(value: &Value, type_name: &str) -> bool {
    match (type_name, value) {
        ("object", Value::Object(_)) => true,
        ("array", Value::Array(_)) => true,
        ("string", Value::String(_)) => true,
        ("boolean", Value::Bool(_)) => true,
        ("null", Value::Null) => true,
        ("number", Value::Number(n)) => n.is_f64() || n.as_i64().is_some() || n.as_u64().is_some(),
        ("integer", Value::Number(n)) => n.as_i64().is_some() || n.as_u64().is_some(),
        _ => false,
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Build the single bounded retry message sent to the child (hermes
/// `build_retry_message` parity): the errors verbatim, no schema re-paste.
pub fn build_retry_message(errors: &[String]) -> String {
    let mut out = String::from(
        "Your previous answer failed validation against the OUTPUT CONTRACT \
         with exactly these errors. Fix your response and resend the COMPLETE \
         answer as a single JSON object matching the contract. Do not repeat \
         the schema; fix only the flagged fields.\n\nValidation errors:\n",
    );
    for error in errors.iter().take(MAX_RENDERED_ERRORS) {
        out.push_str(&format!("- {error}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerce_accepts_object_schema() {
        let schema = json!({ "type": "object", "required": ["summary"] });
        let out = coerce_output_schema(Some(schema.clone())).unwrap();
        assert_eq!(out, Some(schema));
    }

    #[test]
    fn coerce_accepts_string_encoded_schema() {
        let raw = json!(r#"{"type":"object","required":["x"]}"#);
        let out = coerce_output_schema(Some(raw)).unwrap();
        assert_eq!(out, Some(json!({ "type": "object", "required": ["x"] })));
    }

    #[test]
    fn coerce_none_passes_through() {
        assert_eq!(coerce_output_schema(None).unwrap(), None);
    }

    #[test]
    fn coerce_rejects_non_object() {
        let err = coerce_output_schema(Some(json!([1, 2, 3]))).unwrap_err();
        assert!(err.contains("object"));
        let err2 = coerce_output_schema(Some(json!("not a schema"))).unwrap_err();
        assert!(err2.contains("object"));
    }

    #[test]
    fn append_contract_contains_schema_and_header() {
        let schema = json!({ "type": "object", "required": ["x"] });
        let block = append_output_contract(Some("CONTEXT LINE"), &schema);
        assert!(block.starts_with("CONTEXT LINE"));
        assert!(block.contains("OUTPUT CONTRACT (machine-validated)"));
        assert!(block.contains("\"required\""));
        // Standalone call with no context works too.
        let block2 = append_output_contract(None, &schema);
        assert!(!block2.contains("CONTEXT LINE"));
        assert!(block2.contains("OUTPUT CONTRACT (machine-validated)"));
    }

    #[test]
    fn extract_strips_fence_and_prose() {
        // Fenced ```json block (text starts with the fence — exercises the
        // fence-stripping branch, which prose-only inputs never reach).
        let text = "```json\n{\"a\": 1}\n```";
        assert_eq!(extract_json_candidate(text).trim(), "{\"a\": 1}");
        // Fence with a `json` language tag on its own line (the json\n branch).
        let text2 = "```\njson\n{\"a\": 1}\n```";
        assert_eq!(extract_json_candidate(text2).trim(), "{\"a\": 1}");
        // Prose before the fence + bare object with prose around it.
        let text3 = "Here is my answer:\n```json\n{\"a\": 1}\n```\nHope that helps.";
        assert_eq!(extract_json_candidate(text3).trim(), "{\"a\": 1}");
        let candidate2 = extract_json_candidate("Sure: {\"b\": 2} that's all");
        assert_eq!(candidate2, "{\"b\": 2}");
    }

    #[test]
    fn validate_passes_on_conforming_output() {
        let schema = json!({
            "type": "object",
            "required": ["summary", "files"],
            "properties": {
                "summary": { "type": "string" },
                "files": { "type": "array" }
            }
        });
        let (valid, errors) = validate_output(
            "```json\n{\"summary\": \"done\", \"files\": [\"a.rs\"]}\n```",
            &schema,
        );
        assert!(valid, "expected valid, got {errors:?}");
    }

    #[test]
    fn validate_reports_missing_required_and_type() {
        let schema = json!({
            "type": "object",
            "required": ["summary"],
            "properties": {
                "summary": { "type": "string" },
                "count": { "type": "integer" }
            }
        });
        let (valid, errors) = validate_output(r#"{"count": "not-an-int"}"#, &schema);
        assert!(!valid);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("missing required property 'summary'"))
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("$.count: value is not of type 'integer'"))
        );
    }

    #[test]
    fn validate_rejects_invalid_json() {
        let schema = json!({ "type": "object" });
        let (valid, errors) = validate_output("this is not json at all", &schema);
        assert!(!valid);
        assert!(errors[0].contains("not valid JSON"));
    }

    #[test]
    fn validate_nested_object_and_array_items() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": { "type": "object", "required": ["id"] }
                }
            }
        });
        let (valid, errors) = validate_output(r#"{"items": [{"id": 1}, {"wrong": 2}]}"#, &schema);
        assert!(!valid);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("$.items[1]: missing required property 'id'"))
        );
    }

    #[test]
    fn retry_message_carries_errors_verbatim() {
        let msg = build_retry_message(&["$.x: missing".to_string()]);
        assert!(msg.contains("$.x: missing"));
        assert!(msg.contains("OUTPUT CONTRACT"));
    }
}

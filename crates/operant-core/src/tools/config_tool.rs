//! Config management tool — agentic-loop access to operant's own configuration.
//!
//! Hermes-agent parity: the hermes agent can inspect and modify its own
//! configuration through `/config` slash commands and file tools. Operant
//! exposes the same capability as a first-class agentic-loop tool so the
//! model can:
//!
//! - `get <dotted.key>` — read a single config value (runtime-effective).
//! - `set <dotted.key> <value>` — change a config value at runtime, with the
//!   same typed coercion as the CLI's `operant config set` (bool → int →
//!   float → string). The change is validated by round-tripping through
//!   `AppConfig` deserialization; invalid changes are rejected, never
//!   silently applied.
//! - `show` — dump the effective configuration as JSON with secrets masked.
//! - `path` — the resolved config file path (or "built-in defaults").
//! - `reload` — re-read the config file from disk into the runtime config.
//!
//! The tool operates on the process-global runtime config (`runtime_config()`
//! / `install_runtime_config`), exactly like `operant config set`, so every
//! subsequent tool/agent decision sees the new value immediately. Changes
//! are NOT persisted to the TOML file by design — persistence is the
//! user's explicit action (`file_write` to the config path returned by
//! `path`, or the CLI). This mirrors hermes: runtime overrides apply now,
//! disk edits apply on next launch.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::config::{install_runtime_config, load_app_config, runtime_config};
use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Argument struct for `config_manage`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[allow(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON"
)]
pub struct ConfigManageArgs {
    /// Action: "get" | "set" | "show" | "path" | "reload"
    pub action: String,
    /// Dotted config key path, e.g. "agent.model" (get/set)
    pub key: Option<String>,
    /// Value to assign (set only). Accepts any JSON scalar — numbers and
    /// booleans are stringified then type-coerced (bool → int → float →
    /// string) exactly like `operant config set`, so passing `12` or
    /// `"12"` both work without retries.
    pub value: Option<Value>,
}

/// `config_manage` tool — inspect and modify operant's runtime configuration.
pub struct ConfigManageTool;

impl Default for ConfigManageTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl OperantTool for ConfigManageTool {
    fn name(&self) -> &str {
        "config_manage"
    }

    fn description(&self) -> &str {
        "Inspect and modify operant's own configuration at runtime. Actions: \
         'get' (read a dotted key, e.g. agent.model), 'set' (change a dotted key; \
         bool/int/float/string typed coercion, validated before applying), \
         'show' (full effective config as JSON with secrets masked), \
         'path' (config file path), 'reload' (re-read config from disk). \
         Changes apply immediately to the running session but are not persisted \
         to the TOML file — use file_write on the path returned by 'path' to persist."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ConfigManageArgs>("config_manage", self.description())
    }

    fn toolset(&self) -> &str {
        "system"
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: ConfigManageArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("config_manage", format!("Invalid args: {e}")),
        };

        match parsed.action.as_str() {
            "get" => {
                let Some(key) = parsed.key.as_deref().filter(|k| !k.is_empty()) else {
                    return ToolResult::error("config_manage", "get requires a 'key'");
                };
                match get_config_value(key) {
                    Ok(value) => {
                        ToolResult::success("config_manage", json!({ "key": key, "value": value }))
                    }
                    Err(e) => ToolResult::error("config_manage", e.to_string()),
                }
            }
            "set" => {
                let Some(key) = parsed.key.as_deref().filter(|k| !k.is_empty()) else {
                    return ToolResult::error("config_manage", "set requires a 'key'");
                };
                let Some(value) = parsed.value.as_ref() else {
                    return ToolResult::error("config_manage", "set requires a 'value'");
                };
                let value_str = match value {
                    Value::String(s) => s.clone(),
                    Value::Null => {
                        return ToolResult::error("config_manage", "set value cannot be null");
                    }
                    other => other.to_string(),
                };
                match set_config_value(key, &value_str) {
                    Ok(()) => ToolResult::success(
                        "config_manage",
                        json!({ "key": key, "value": value, "applied": true, "validated_against": "AppConfig" }),
                    ),
                    Err(e) => ToolResult::error("config_manage", e.to_string()),
                }
            }
            "show" => {
                let config = runtime_config();
                let redacted = redact_secrets(&serde_json::to_value(&config).unwrap_or_default());
                ToolResult::success("config_manage", json!({ "config": redacted }))
            }
            "path" => {
                let path = config_file_path();
                ToolResult::success("config_manage", json!({ "path": path }))
            }
            "reload" => match load_app_config(None) {
                Ok(loaded) => {
                    install_runtime_config(loaded.config.clone());
                    ToolResult::success(
                        "config_manage",
                        json!({ "reloaded": true, "source": loaded.source.map(|p| p.display().to_string()) }),
                    )
                }
                Err(e) => ToolResult::error("config_manage", format!("reload failed: {e}")),
            },
            other => ToolResult::error(
                "config_manage",
                format!("Unknown action '{other}' — expected get|set|show|path|reload"),
            ),
        }
    }
}

/// Read a dotted config key from the runtime config as a JSON value.
fn get_config_value(key: &str) -> Result<Value> {
    let config = runtime_config();
    let root = serde_json::to_value(&config)
        .map_err(|e| crate::error::Error::Agent(format!("config serialize failed: {e}")))?;

    let mut current: &Value = &root;
    for segment in key.split('.') {
        current = current
            .get(segment)
            .ok_or_else(|| crate::error::Error::Agent(format!("config key '{key}' not found")))?;
    }
    Ok(current.clone())
}

/// Set a dotted config key on the runtime config (mirrors `operant config set`).
fn set_config_value(key: &str, value: &str) -> Result<()> {
    let current = runtime_config();
    let mut root = serde_json::to_value(&current)
        .map_err(|e| crate::error::Error::Agent(format!("config serialize failed: {e}")))?;

    let segments: Vec<&str> = key.split('.').collect();
    if segments.is_empty() {
        return Err(crate::error::Error::Agent(
            "config key must not be empty".into(),
        ));
    }

    set_nested(&mut root, &segments, value)?;

    let updated = serde_json::from_value(root).map_err(|e| {
        crate::error::Error::Agent(format!("invalid config value for '{key}': {e}"))
    })?;

    install_runtime_config(updated);
    Ok(())
}

/// Recursively navigate into a nested `serde_json::Value` object and set the
/// leaf value identified by the remaining path segments.
fn set_nested(target: &mut Value, segments: &[&str], raw: &str) -> Result<()> {
    if segments.is_empty() {
        return Err(crate::error::Error::Agent(
            "config key must not be empty".into(),
        ));
    }
    let head = segments[0];

    if segments.len() == 1 {
        let parsed = coerce_json_value(raw);
        let obj = target.as_object_mut().ok_or_else(|| {
            crate::error::Error::Agent(format!("cannot index into non-object with '{head}'"))
        })?;
        obj.insert(head.to_string(), parsed);
        return Ok(());
    }

    let tail = &segments[1..];
    let obj = target.as_object_mut().ok_or_else(|| {
        crate::error::Error::Agent(format!("cannot index into non-object with '{head}'"))
    })?;

    if !obj.contains_key(head) {
        obj.insert(head.to_string(), Value::Object(Map::new()));
    }
    let child = obj
        .get_mut(head)
        .ok_or_else(|| crate::error::Error::Agent(format!("could not access key '{head}'")))?;

    set_nested(child, tail, raw)
}

/// Best-effort coercion of a string argument into a typed JSON value.
/// Precedence: boolean → integer → float → string (mirrors the CLI).
fn coerce_json_value(raw: &str) -> Value {
    match raw.to_ascii_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }

    if let Ok(n) = raw.parse::<i64>() {
        return Value::Number(n.into());
    }

    if let Ok(f) = raw.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return Value::Number(n);
    }

    Value::String(raw.to_string())
}

/// Mask secret-looking values (api_key / token / secret / password / auth)
/// in a config dump so the model never echoes credentials back.
fn redact_secrets(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                let lower = k.to_ascii_lowercase();
                let secret_like = lower.contains("api_key")
                    || lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("password")
                    || lower == "auth";
                if secret_like {
                    out.insert(k.clone(), Value::String("***".to_string()));
                } else {
                    out.insert(k.clone(), redact_secrets(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_secrets).collect()),
        other => other.clone(),
    }
}

/// Resolve the effective config file path (mirrors `operant config path`).
fn config_file_path() -> String {
    if let Ok(path) = std::env::var("HERMES_CONFIG") {
        return path;
    }
    for path in crate::config::default_config_paths() {
        if path.exists() {
            return path.canonicalize().unwrap_or(path).display().to_string();
        }
    }
    "No configuration file found. Using built-in defaults.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coerce_boolean_and_number() {
        assert_eq!(coerce_json_value("true"), Value::Bool(true));
        assert_eq!(coerce_json_value("false"), Value::Bool(false));
        assert_eq!(coerce_json_value("42"), Value::Number(42.into()));
        assert_eq!(coerce_json_value("-5"), Value::Number((-5).into()));
        assert_eq!(
            coerce_json_value("2.71"),
            Value::Number(serde_json::Number::from_f64(2.71).unwrap())
        );
        assert_eq!(coerce_json_value("gpt-4o"), Value::String("gpt-4o".into()));
    }

    #[test]
    fn set_nested_creates_intermediates() {
        let mut obj = serde_json::json!({});
        set_nested(&mut obj, &["a", "b", "c"], "42").unwrap();
        assert_eq!(obj["a"]["b"]["c"], 42);
    }

    #[test]
    fn set_nested_rejects_empty_key() {
        let mut obj = serde_json::json!({});
        assert!(set_nested(&mut obj, &[], "v").is_err());
    }

    #[test]
    fn redact_masks_secret_keys() {
        let value = serde_json::json!({
            "client": { "api_key": "sk-123", "base_url": "http://x" },
            "gateway": { "telegram_token": "tok" },
            "agent": { "model": "gpt-4o" },
        });
        let redacted = redact_secrets(&value);
        assert_eq!(redacted["client"]["api_key"], "***");
        assert_eq!(redacted["gateway"]["telegram_token"], "***");
        assert_eq!(redacted["client"]["base_url"], "http://x");
        assert_eq!(redacted["agent"]["model"], "gpt-4o");
    }

    #[test]
    fn get_missing_key_errors() {
        let result = get_config_value("agent.does_not_exist_xyz");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_unknown_action_errors() {
        let tool = ConfigManageTool;
        let result = tool
            .execute(
                serde_json::json!({ "action": "bogus" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("Unknown action"));
    }

    #[tokio::test]
    async fn execute_show_returns_masked_config() {
        let tool = ConfigManageTool;
        let result = tool
            .execute(
                serde_json::json!({ "action": "show" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        assert!(result.content.contains("\"config\""));
        // No raw secret-shaped values leak.
        assert!(!result.content.contains("sk-"));
    }

    #[tokio::test]
    async fn execute_path_returns_string() {
        let tool = ConfigManageTool;
        let result = tool
            .execute(
                serde_json::json!({ "action": "path" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        assert!(result.content.contains("\"path\""));
    }

    #[tokio::test]
    async fn set_accepts_numeric_and_string_values() {
        let tool = ConfigManageTool;

        // Models often emit a JSON number instead of a quoted string; both
        // forms must set the same value without retry loops.
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "set",
                    "key": "agent.max_iterations",
                    "value": 42,
                }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "numeric value rejected: {:?}", result.error);
        assert_eq!(get_config_value("agent.max_iterations").unwrap(), 42);

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "set",
                    "key": "agent.model",
                    "value": "gpt-5",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "string value rejected: {:?}", result.error);
        assert_eq!(
            get_config_value("agent.model").unwrap(),
            serde_json::json!("gpt-5")
        );

        // Unknown keys are rejected (deny_unknown_fields), never silently
        // dropped — so the model sees a real error instead of a no-op.
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "set",
                    "key": "agent.totally_unknown_key_xyz",
                    "value": "x",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success, "unknown key must error, got success");
        assert!(
            result.error.unwrap_or_default().contains("unknown field"),
            "expected unknown-field error"
        );
    }
}

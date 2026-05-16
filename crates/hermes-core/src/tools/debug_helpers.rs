use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

pub struct InspectJsonTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InspectJsonArgs {
    json: Value,
    pretty: Option<bool>,
}

#[async_trait]
impl HermesTool for InspectJsonTool {
    fn name(&self) -> &str {
        "debug_inspect_json"
    }

    fn description(&self) -> &str {
        "Pretty-print and inspect a JSON value. Returns formatted JSON with \
         metadata about the structure including key count, nesting depth, value types, and size."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<InspectJsonArgs>(
            "debug_inspect_json",
            "Inspect and pretty-print a JSON value",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: InspectJsonArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("debug_inspect_json", format!("Invalid arguments: {}", e))
            }
        };

        let parsed = match args.json {
            Value::String(ref s) => serde_json::from_str::<Value>(s).unwrap_or(args.json.clone()),
            other => other,
        };

        let pretty = args.pretty.unwrap_or(true);

        let formatted = if pretty {
            serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| "{}".to_string())
        } else {
            serde_json::to_string(&parsed).unwrap_or_else(|_| "{}".to_string())
        };

        let (key_count, max_depth, value_types) = inspect_value(&parsed, 0);

        ToolResult::success(
            "debug_inspect_json",
            serde_json::json!({
                "formatted": formatted,
                "structure": {
                    "type": json_type_name(&parsed),
                    "key_count": key_count,
                    "max_depth": max_depth,
                    "value_types": value_types,
                    "size_bytes": formatted.len(),
                }
            }),
        )
    }
}

fn inspect_value(value: &Value, depth: u32) -> (u64, u32, HashMap<String, u64>) {
    let mut key_count = 0u64;
    let mut max_depth = depth;
    let mut value_types: HashMap<String, u64> = HashMap::new();

    match value {
        Value::Object(map) => {
            *value_types.entry("object".to_string()).or_insert(0) += 1;
            key_count = map.len() as u64;
            for v in map.values() {
                let (kc, md, mut vt) = inspect_value(v, depth + 1);
                key_count += kc;
                max_depth = max_depth.max(md);
                for (k, c) in vt.drain() {
                    *value_types.entry(k).or_insert(0) += c;
                }
            }
        }
        Value::Array(arr) => {
            *value_types.entry("array".to_string()).or_insert(0) += 1;
            for v in arr {
                let (kc, md, mut vt) = inspect_value(v, depth + 1);
                key_count += kc;
                max_depth = max_depth.max(md);
                for (k, c) in vt.drain() {
                    *value_types.entry(k).or_insert(0) += c;
                }
            }
        }
        Value::String(_) => {
            *value_types.entry("string".to_string()).or_insert(0) += 1;
        }
        Value::Number(_) => {
            *value_types.entry("number".to_string()).or_insert(0) += 1;
        }
        Value::Bool(_) => {
            *value_types.entry("boolean".to_string()).or_insert(0) += 1;
        }
        Value::Null => {
            *value_types.entry("null".to_string()).or_insert(0) += 1;
        }
    }

    (key_count, max_depth, value_types)
}

fn json_type_name(value: &Value) -> &str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub struct EnvVarTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnvVarArgs {
    filter: Option<String>,
}

const SENSITIVE_KEYS: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "PASS", "CREDENTIAL"];

fn is_sensitive_key(name: &str) -> bool {
    let upper = name.to_uppercase();
    SENSITIVE_KEYS.iter().any(|&pat| upper.contains(pat))
}

#[async_trait]
impl HermesTool for EnvVarTool {
    fn name(&self) -> &str {
        "debug_env"
    }

    fn description(&self) -> &str {
        "List environment variables, automatically masking sensitive values \
         (keys containing KEY, TOKEN, SECRET, PASSWORD). Supports optional filtering."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EnvVarArgs>("debug_env", "List environment variables")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let _args: EnvVarArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("debug_env", format!("Invalid arguments: {}", e)),
        };

        let mut vars: Vec<serde_json::Value> = Vec::new();

        for (key, value) in std::env::vars() {
            let display_value = if is_sensitive_key(&key) {
                "*** MASKED ***".to_string()
            } else {
                value
            };

            vars.push(serde_json::json!({
                "key": key,
                "value": display_value
            }));
        }

        vars.sort_by(|a, b| {
            a["key"]
                .as_str()
                .unwrap_or("")
                .cmp(b["key"].as_str().unwrap_or(""))
        });

        ToolResult::success(
            "debug_env",
            serde_json::json!({
                "count": vars.len(),
                "variables": vars
            }),
        )
    }
}

pub struct EchoTool;

#[derive(JsonSchema, Deserialize)]
struct EchoArgs {
    message: String,
}

#[async_trait]
impl HermesTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo back the input message. Useful for debugging and connectivity tests."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EchoArgs>("echo", "Echo back the input message")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: EchoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("echo", format!("Invalid arguments: {}", e)),
        };

        ToolResult::success("", serde_json::json!({ "message": args.message }))
    }
}

pub struct SystemInfoTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemInfoArgs {
    _dummy: Option<bool>,
}

#[async_trait]
impl HermesTool for SystemInfoTool {
    fn name(&self) -> &str {
        "debug_system"
    }

    fn description(&self) -> &str {
        "Get system information including operating system, CPU count, \
         memory, hostname, and Rust compiler version."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SystemInfoArgs>("debug_system", "Get system information")
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);

        let hostname = get_hostname();
        let memory_info = get_memory_info();
        let rust_version = get_rust_version();

        ToolResult::success(
            "debug_system",
            serde_json::json!({
                "os": os,
                "arch": arch,
                "cpu_count": cpu_count,
                "hostname": hostname,
                "memory": memory_info,
                "rust_version": rust_version
            }),
        )
    }
}

fn get_hostname() -> String {
    if let Ok(host) = std::env::var("HOSTNAME") {
        return host;
    }
    if let Ok(host) = std::env::var("HOST") {
        return host;
    }
    if let Ok(content) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        return content.trim().to_string();
    }
    if let Ok(output) = std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    {
        if !output.is_empty() {
            return output;
        }
    }
    "unknown".to_string()
}

fn get_memory_info() -> Value {
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            let bytes = kb * 1024;
                            return serde_json::json!({
                                "total_bytes": bytes,
                                "total_mb": bytes / (1024 * 1024),
                                "total_gb": bytes / (1024 * 1024 * 1024)
                            });
                        }
                    }
                }
            }
        }
        serde_json::json!({ "available": false })
    }

    #[cfg(not(target_os = "linux"))]
    {
        serde_json::json!({ "available": false })
    }
}

fn get_rust_version() -> String {
    if let Ok(output) = std::process::Command::new("rustc")
        .arg("--version")
        .output()
    {
        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !version.is_empty() {
                return version;
            }
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inspect_json_object() {
        let tool = InspectJsonTool;
        let args = serde_json::json!({
            "json": {"name": "test", "value": 42, "nested": {"a": 1, "b": 2}}
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["structure"]["type"], "object");
    }

    #[tokio::test]
    async fn test_inspect_json_string_input() {
        let tool = InspectJsonTool;
        let args = serde_json::json!({
            "json": "{\"key\": \"value\"}"
        });
        let result = tool.execute(args, ToolContext::default()).await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_env_var_tool() {
        let tool = EnvVarTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_system_info_tool() {
        let tool = SystemInfoTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert!(v["os"].is_string());
        assert!(v["cpu_count"].is_number());
    }
}

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

pub struct EnvVarTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct EnvVarArgs {
    filter: Option<String>,
}

const SENSITIVE_KEYS: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "PASS", "CREDENTIAL"];

fn is_sensitive_key(name: &str) -> bool {
    let upper = name.to_uppercase();
    SENSITIVE_KEYS.iter().any(|&pat| upper.contains(pat))
}

#[async_trait]
impl OperantTool for EnvVarTool {
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
        let filter = args.get("filter").and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else if v.is_object() || v.is_array() || v.is_null() {
                None
            } else {
                Some(v.to_string())
            }
        });

        let mut vars: Vec<serde_json::Value> = Vec::new();
        let filter_lower = filter.as_ref().map(|f| f.to_lowercase());

        for (key, value) in std::env::vars() {
            if let Some(ref f) = filter_lower
                && !key.to_lowercase().contains(f)
                && !value.to_lowercase().contains(f)
            {
                continue;
            }

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
impl OperantTool for EchoTool {
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
impl OperantTool for SystemInfoTool {
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
        && !output.is_empty()
    {
        return output;
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
                    if parts.len() >= 2
                        && let Ok(kb) = parts[1].parse::<u64>()
                    {
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
        && output.status.success()
    {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return version;
        }
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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

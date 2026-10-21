//! The `sop` tool — the agent reads the operator's standard operating
//! procedures on demand.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Read the operator's SOP playbooks.
pub struct SopTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SopArgs {
    /// The SOP to read. Omit to list the available SOPs.
    name: Option<String>,
}

#[async_trait]
impl OperantTool for SopTool {
    fn name(&self) -> &str {
        "sop"
    }

    fn description(&self) -> &str {
        "Read the operator's standard operating procedures (SOPs) — the playbooks for how a \
        recurring task is done here. Call with no name to list them, or with a name to read one."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SopArgs>("sop", "List or read the operator's SOP playbooks")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: SopArgs = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(err) => return ToolResult::error("sop", format!("Invalid arguments: {err}")),
        };
        let dir = crate::sops::sops_dir();
        let sops = crate::sops::discover_sops(&dir);
        match args.name {
            None => {
                if sops.is_empty() {
                    return ToolResult::success(
                        "sop",
                        format!(
                            "No SOPs found in {}. Add a .toml file there to define one.",
                            dir.display()
                        ),
                    );
                }
                let listing = sops
                    .iter()
                    .map(|sop| format!("- {}: {}", sop.name, sop.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolResult::success("sop", format!("SOPs in {}:\n{listing}", dir.display()))
            }
            Some(name) => {
                let wanted = name.trim().to_ascii_lowercase();
                match sops
                    .iter()
                    .find(|sop| sop.name.to_ascii_lowercase() == wanted)
                {
                    Some(sop) => {
                        ToolResult::success("sop", format!("# {}\n\n{}", sop.name, sop.content))
                    }
                    None => ToolResult::error(
                        "sop",
                        format!(
                            "No SOP named '{name}'. Available: {}",
                            sops.iter()
                                .map(|s| s.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn listing_and_reading_an_sop_works() {
        let _guard = crate::platform::test_env_lock().await;
        let home = std::env::temp_dir().join(format!("operant-sop-tool-{}", std::process::id()));
        let dir = home.join("sops");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("playbook.toml"),
            "description = \"how we do it\"\nsteps = \"do the thing\"\n",
        )
        .expect("write");
        unsafe { std::env::set_var("HERMES_HOME", &home) };

        let tool = SopTool;
        let listed = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(listed.success, "{}", listed.content);
        assert!(listed.content.contains("playbook"), "{}", listed.content);

        let read = tool
            .execute(
                serde_json::json!({"name": "playbook"}),
                ToolContext::default(),
            )
            .await;
        assert!(read.success, "{}", read.content);
        assert!(read.content.contains("how we do it"), "{}", read.content);

        let missing = tool
            .execute(serde_json::json!({"name": "nope"}), ToolContext::default())
            .await;
        assert!(!missing.success, "{}", missing.content);

        unsafe { std::env::remove_var("HERMES_HOME") };
        let _ = std::fs::remove_dir_all(&home);
    }
}

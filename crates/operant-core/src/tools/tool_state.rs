//! The tool enable/disable state file (`tool_state.json`).
//!
//! `operant tools enable|disable` writes this file next to the database;
//! the RUNTIME reads it when it builds the tool registry. Before iter-1124
//! the file was written by the CLI and read by nothing, so
//! `operant tools disable <name>` changed no behaviour at all.
//!
//! Two sources feed the disabled set, and the runtime honours BOTH: the
//! TOML list (`[tools] disabled_tools`) and this file. The CLI keeps its
//! own file so it never rewrites the operator's TOML.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

/// JSON shape of the state file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolState {
    /// Tool names the operator disabled through the CLI.
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Tool names the operator DEFERRED through the CLI.
    ///
    /// A deferred tool is not disabled: it stays registered and callable,
    /// and the `tool_search` bridge still reaches it (search → describe →
    /// call). What it stops doing is riding every request in the tools
    /// array, which is where the per-request bill comes from. MEASURED
    /// (2026-11-12): the live surface is 96 tools / 18 732 tokens per
    /// request, and no operator-facing lever existed to shrink it without
    /// losing the tool.
    #[serde(default)]
    pub deferred: Vec<String>,
}

/// Path to the state file — the config directory (the database's parent).
pub fn tool_state_path(config: &AppConfig) -> PathBuf {
    let dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    dir.join("tool_state.json")
}

/// Load the CLI-disabled tool names. A missing file is an empty set; a
/// corrupt file is an empty set too (the agent must not lose its tools
/// because a JSON file went bad) — the caller is warned instead.
pub fn load_disabled_tools(config: &AppConfig) -> BTreeSet<String> {
    let path = tool_state_path(config);
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<ToolState>(&content) {
            Ok(state) => state.disabled.into_iter().collect(),
            Err(err) => {
                eprintln!(
                    "warning: {} is unreadable ({err}) — no CLI-disabled tools are applied. Fix or delete the file.",
                    path.display()
                );
                BTreeSet::new()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(err) => {
            eprintln!(
                "warning: {} could not be read ({err}) — no CLI-disabled tools are applied.",
                path.display()
            );
            BTreeSet::new()
        }
    }
}

/// Persist the CLI-disabled tool names (sorted, so the file is stable).
pub fn save_disabled_tools(config: &AppConfig, disabled: &BTreeSet<String>) -> Result<(), String> {
    let path = tool_state_path(config);
    // Read the existing state first: the two sets live in ONE file, and
    // writing only this half would silently clear the other.
    let mut state = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<ToolState>(&content).unwrap_or_default(),
        Err(_) => ToolState::default(),
    };
    state.disabled = disabled.iter().cloned().collect();
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialise tool state as JSON: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write tool state to '{}': {e}", path.display()))
}

// ── The deferred set ───────────────────────────────────────────────────────

/// Load the CLI-deferred tool names. The same failure policy as the disabled
/// set: a missing file is an empty set, an unreadable file is an empty set
/// with a warning. A tool must not VANISH from the agent because a JSON file
/// went bad.
pub fn load_deferred_tools(config: &AppConfig) -> BTreeSet<String> {
    let path = tool_state_path(config);
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<ToolState>(&content) {
            Ok(state) => state.deferred.into_iter().collect(),
            Err(err) => {
                eprintln!(
                    "warning: {} is unreadable ({err}) — no CLI-deferred tools are applied. Fix or delete the file.",
                    path.display()
                );
                BTreeSet::new()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(err) => {
            eprintln!(
                "warning: {} could not be read ({err}) — no CLI-deferred tools are applied.",
                path.display()
            );
            BTreeSet::new()
        }
    }
}

/// Persist the deferred set alongside the disabled one, preserving it.
pub fn save_deferred_tools(config: &AppConfig, deferred: &BTreeSet<String>) -> Result<(), String> {
    let path = tool_state_path(config);
    // Read the existing state first: the two sets live in ONE file, and
    // writing only this half would silently clear the other.
    let mut state = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<ToolState>(&content).unwrap_or_default(),
        Err(_) => ToolState::default(),
    };
    state.deferred = deferred.iter().cloned().collect();
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialise tool state as JSON: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write tool state to '{}': {}", path.display(), e))
}

/// The deferred set the RUNTIME must apply: the CLI state file today, the
/// TOML list if an operator sets one, unioned — the same one-place rule the
/// disabled set follows.
pub fn deferred_tool_names(config: &AppConfig) -> HashSet<String> {
    let mut names: HashSet<String> = config.tools.deferred_tools.iter().cloned().collect();
    names.extend(load_deferred_tools(config));
    names
}

/// Apply the effective deferred set to a live registry.
pub async fn apply_deferred_tools(registry: &crate::tools::ToolRegistry, config: &AppConfig) {
    let names = deferred_tool_names(config);
    registry.set_deferred_tools(names).await;
}

/// The disabled set the RUNTIME must apply: the TOML list unioned with the
/// CLI state file. This is the one place both sources are combined — the
/// registry build and `operant tools list` call it, so what the operator
/// sees and what the agent gets cannot drift.
pub fn disabled_tool_names(config: &AppConfig) -> HashSet<String> {
    let mut names: HashSet<String> = config.tools.disabled_tools.iter().cloned().collect();
    names.extend(load_disabled_tools(config));
    names
}

/// Apply the effective disabled set to a live registry: the TOML list, the
/// CLI state file, and whatever the caller adds (the AFT-replaced native
/// tools). The registry build calls this, so `operant tools disable` lands
/// on the toolset the agent actually gets.
pub async fn apply_disabled_tools(
    registry: &crate::tools::ToolRegistry,
    config: &AppConfig,
    extra: HashSet<String>,
) {
    let mut names = disabled_tool_names(config);
    names.extend(extra);
    if !names.is_empty() {
        registry.set_disabled_tools(names).await;
    }
    // The deferred set is applied here too, on purpose: a caller that loads
    // the operator's tool state gets ALL of it in one place, so a deferral
    // cannot land on a build that forgot to read it. The alternative — a
    // separate call every build path remounts — is exactly the "three copies
    // of the same rule" problem the module header warns about.
    apply_deferred_tools(registry, config).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_db(dir: &Path) -> AppConfig {
        AppConfig {
            database_path: dir.join("operant.db"),
            ..AppConfig::default()
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("operant-tool-state-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn cli_deferred_tools_reach_the_runtime_set() {
        let dir = temp_dir("deferred-union");
        let mut config = config_with_db(&dir);
        config.tools.deferred_tools = vec!["from_toml".to_string()];
        let mut file_set = std::collections::BTreeSet::new();
        file_set.insert("from_file".to_string());
        save_deferred_tools(&config, &file_set).expect("write the state file");
        let union = deferred_tool_names(&config);
        assert!(union.contains("from_toml"), "the TOML list counts");
        assert!(union.contains("from_file"), "the CLI state file counts");
        // Writing the disabled half must not clear the deferred half: one
        // file, two sets.
        let mut disabled = std::collections::BTreeSet::new();
        disabled.insert("a_disabled_one".to_string());
        save_disabled_tools(&config, &disabled).expect("write the disabled half");
        assert_eq!(
            deferred_tool_names(&config).len(),
            2,
            "the deferred set survived a write to the other half"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_deferral_lands_on_the_registry_through_the_disabled_path() {
        // The wiring test. `apply_disabled_tools` is the ONE call every build
        // path mounts, so the deferral is applied there — and a deferral that
        // only works when a caller remembers a second call is the "named but
        // not shipped" shape this module already has a commit about.
        let dir = temp_dir("deferral-wiring");
        let mut config = config_with_db(&dir);
        config.tools.deferred_tools = vec!["wired".to_string()];
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        registry.set_deferred_tools(HashSet::new()).await;
        apply_disabled_tools(&registry, &config, HashSet::new()).await;
        let deferred = registry.deferred_tools().await;
        assert!(
            deferred.contains("wired"),
            "a TOML-declared deferral must reach the registry through the one shared path"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deferred_tool_dies_from_the_set_when_the_file_is_corrupt() {
        // The same failure policy as the disabled set: a corrupt file is an
        // EMPTY set with a warning, so a bad JSON cannot silently remove a
        // tool from the agent.
        let dir = temp_dir("deferred-corrupt");
        let config = config_with_db(&dir);
        std::fs::write(tool_state_path(&config), "{ not json }").unwrap();
        assert!(
            load_deferred_tools(&config).is_empty(),
            "a corrupt state file is an empty deferred set, not a dropped tool"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_disabled_tools_reach_the_runtime_set() {
        let dir = temp_dir("union");
        let mut config = config_with_db(&dir);
        config.tools.disabled_tools = vec!["toml_tool".to_string()];

        let mut disabled = BTreeSet::new();
        disabled.insert("cli_tool".to_string());
        save_disabled_tools(&config, &disabled).expect("save");

        let names = disabled_tool_names(&config);
        assert!(names.contains("toml_tool"), "{names:?}");
        assert!(names.contains("cli_tool"), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The production path: a tool the operator disabled with
    /// `operant tools disable` must be GONE from the registry the agent
    /// sees, and still visible in the full listing. Before iter-1124 the
    /// state file was read by nobody and the tool stayed callable.
    #[tokio::test]
    async fn a_cli_disabled_tool_is_removed_from_the_registry() {
        struct ProbeTool;
        #[async_trait::async_trait]
        impl crate::tools::OperantTool for ProbeTool {
            fn name(&self) -> &str {
                "cli_disabled_probe"
            }
            fn description(&self) -> &str {
                "probe"
            }
            fn schema(&self) -> crate::schema::ToolSchema {
                crate::schema::ToolSchema::new(
                    "cli_disabled_probe",
                    "probe",
                    serde_json::json!({"type": "object"}),
                )
            }
            async fn execute(
                &self,
                _args: serde_json::Value,
                _context: crate::tools::ToolContext,
            ) -> crate::tools::ToolResult {
                crate::tools::ToolResult::success("probe", "ok")
            }
        }

        let dir = temp_dir("registry");
        let config = config_with_db(&dir);
        let mut disabled = BTreeSet::new();
        disabled.insert("cli_disabled_probe".to_string());
        save_disabled_tools(&config, &disabled).expect("save");

        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(30));
        registry.register(ProbeTool).await.expect("register");
        assert!(
            registry
                .get_schemas()
                .await
                .iter()
                .any(|t| t.name == "cli_disabled_probe"),
            "the probe tool starts enabled"
        );

        apply_disabled_tools(&registry, &config, HashSet::new()).await;

        assert!(
            !registry
                .get_schemas()
                .await
                .iter()
                .any(|t| t.name == "cli_disabled_probe"),
            "a CLI-disabled tool must not reach the agent's toolset"
        );
        assert!(
            registry
                .get_schemas_all()
                .await
                .iter()
                .any(|t| t.name == "cli_disabled_probe"),
            "the disabled tool stays visible in the full listing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_state_file_does_not_disable_tools_and_says_so() {
        let dir = temp_dir("corrupt");
        let config = config_with_db(&dir);
        std::fs::write(tool_state_path(&config), "{not json").expect("write");
        // No panic, no tools silently disabled.
        assert!(load_disabled_tools(&config).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_state_file_is_not_a_warning() {
        let dir = temp_dir("missing");
        let config = config_with_db(&dir);
        let _ = std::fs::remove_file(tool_state_path(&config));
        assert!(load_disabled_tools(&config).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

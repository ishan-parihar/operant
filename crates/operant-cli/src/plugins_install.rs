//! Plugin installation from Git repositories and plugin scaffolding
//! (`operant plugins new` / `operant plugins validate` — gap G7, the creator
//! mode).
//! The install half is ported from operant-agent/operant_cli/plugins_cmd.py.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Install a plugin from a git URL or owner/repo shorthand.
/// Returns the plugin name (derived from directory name).
pub async fn install_plugin(identifier: &str, plugins_dir: &Path, force: bool) -> Result<String> {
    // Resolve identifier to a git URL
    let git_url = resolve_git_url(identifier);
    let plugin_name = derive_plugin_name(identifier);

    let target = plugins_dir.join(&plugin_name);

    if target.exists() {
        if force {
            std::fs::remove_dir_all(&target).with_context(|| {
                format!("Failed to remove existing plugin at {}", target.display())
            })?;
        } else {
            anyhow::bail!(
                "Plugin '{}' is already installed at {}. Use --force to reinstall.",
                plugin_name,
                target.display()
            );
        }
    }

    // Ensure parent directory exists
    std::fs::create_dir_all(plugins_dir).with_context(|| {
        format!(
            "Failed to create plugins directory at {}",
            plugins_dir.display()
        )
    })?;

    // Clone the repository
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &git_url, &target.to_string_lossy()])
        .status()
        .context("Failed to execute git clone. Is git installed?")?;

    if !status.success() {
        anyhow::bail!("git clone failed for URL: {}", git_url);
    }

    // Validate plugin manifest
    validate_plugin(&target)?;

    Ok(plugin_name)
}

/// Resolve an identifier to a git URL.
/// Supports: full URLs, owner/repo shorthand -> <https://github.com/owner/repo>
fn resolve_git_url(identifier: &str) -> String {
    if identifier.starts_with("http://")
        || identifier.starts_with("https://")
        || identifier.starts_with("git@")
        || identifier.starts_with("file://")
    {
        identifier.to_string()
    } else if let Some((owner, repo)) = identifier.split_once('/') {
        // Owner/repo shorthand -> GitHub
        format!("https://github.com/{}/{}", owner, repo)
    } else {
        // Assume it's a full name, try GitHub
        format!("https://github.com/{}/{}", identifier, identifier)
    }
}

/// Derive a plugin name from the identifier (last path segment, no .git).
fn derive_plugin_name(identifier: &str) -> String {
    let name = identifier.strip_suffix(".git").unwrap_or(identifier);
    let name = name.trim_end_matches('/');
    name.rsplit_once('/')
        .map(|(_, last)| last.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Scaffold a new plugin project (gap G7 — the creator mode).
///
/// Writes a minimal, validator-accepted plugin directory at `dest`: the
/// manifest (`manifest.toml`), the capability's export-contract README, and —
/// for WASM capabilities — a `Cargo.toml` + `src/lib.rs` PDK stub. Returns
/// the scaffold directory. Pure templating; nothing is installed.
pub fn scaffold_plugin(name: &str, capability: &str, dest: &Path) -> Result<PathBuf> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "invalid plugin name '{name}': use letters, digits, '-' or '_' (it doubles as the directory name)"
        );
    }
    match capability {
        "tool" | "observer" | "memory" | "skill" => {}
        other => {
            anyhow::bail!("unknown capability '{other}': expected tool, observer, memory or skill")
        }
    }
    if dest.exists() {
        anyhow::bail!(
            "'{}' already exists — refusing to overwrite",
            dest.display()
        );
    }

    let manifest = if capability == "skill" {
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"{name} — scaffolded by operant plugins new\"\ncapabilities = [\"skill\"]\n"
        )
    } else {
        format!(
            "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"{name} — scaffolded by operant plugins new\"\ncapabilities = [\"{capability}\"]\nwasm_path = \"{name}.wasm\"\n# permissions = [\"http_client\"]  # request host permissions (docs/library/plugins.md)\n# [config]                       # optional free-form table; rides the observe envelope verbatim\n"
        )
    };
    let skill_rel = format!("skills/{name}/SKILL.md");
    let files: Vec<(&str, String)> = match capability {
        "tool" | "observer" | "memory" => vec![
            ("manifest.toml", manifest),
            ("README.md", readme_for(name, capability)),
            ("Cargo.toml", cargo_toml_for(name)),
            ("src/lib.rs", lib_rs_for(name, capability)),
        ],
        // skill-only plugins carry no WASM payload; the manifest omits wasm_path
        _ => vec![
            ("manifest.toml", manifest),
            ("README.md", readme_for(name, capability)),
            (skill_rel.as_str(), skill_md_for(name)),
        ],
    };
    for (rel, content) in &files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(dest.to_path_buf())
}

/// The author's contract sheet: every WASM export the capability must provide,
/// plus the build + install loop.
fn readme_for(name: &str, capability: &str) -> String {
    let mut s = format!(
        "# {name}\n\nScaffolded by `operant plugins new {name} --capability {capability}`.\n\n"
    );
    match capability {
        "tool" => s.push_str(
            "## Export contract (WASM exports)\n\n\
- `tool_metadata` — returns JSON `{\"name\", \"description\", \"parameters_schema\"}` \
(parameters_schema is the tool's JSON Schema). Called once at discovery; a missing \
export falls back to manifest defaults.\n\
- `execute` — receives the tool-call arguments as JSON, returns \
`{\"success\": bool, \"output\": string, \"error\": string?}`.\n",
        ),
        "observer" => s.push_str(
            "## Export contract (WASM exports)\n\n\
- `observe` — receives an interception event \
`{\"point\": \"tools/pre-execute\"|\"tools/post-execute\", \"tool\", \"args\", \"result\", \"config\"}` \
(`result` is null on pre-execute; `config` is the manifest `[config]` table verbatim). \
Returns a verdict: `{\"action\":\"allow\"}`, `{\"action\":\"block\",\"reason\":\"...\"}`, or \
`{\"action\":\"replace\",\"content\":\"...\"}` (rewrites the tool result at post-execute). \
The host FAILS OPEN: a broken export, a failed module or a busy instance pool never \
blocks a tool.\n",
        ),
        "memory" => s.push_str(
            "## Export contract (WASM exports)\n\n\
- `prefetch` — query in, memories out (an odd output degrades to an empty recall)\n\
- `sync_turn` — per-turn write-back\n\
- `tool_schemas` — the memory tools this provider contributes (read once at load)\n\n\
Only ONE memory plugin may be active at a time. Exact JSON shapes: \
`crates/operant-cli/src/plugin_memory.rs`.\n",
        ),
        "skill" => s.push_str(&format!(
            "## Skill layout\n\n\
`skills/{name}/SKILL.md` — agents.io-format skill: YAML frontmatter (`name` + \
`description`), then the markdown instructions. No WASM required.\n"
        )),
        _ => {}
    }
    if capability == "skill" {
        s.push_str(&format!(
            "\n## Install\n\n```sh\noperant plugins install file://$PWD/{name}\n```\n"
        ));
    } else {
        s.push_str(&format!(
            "\n## Build & install\n\n```sh\nrustup target add wasm32-wasip1\ncargo build --release --target wasm32-wasip1\ncp target/wasm32-wasip1/release/{name}.wasm .\noperant plugins install file://$PWD/{name}\noperant plugins list   # verify\n```\n"
        ));
    }
    s
}

fn cargo_toml_for(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
[lib]\ncrate-type = [\"cdylib\"]\n\n\
[dependencies]\nextism-pdk = \"1\"\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n"
    )
}

fn lib_rs_for(name: &str, capability: &str) -> String {
    let header = format!("//! {name} — operant {capability} plugin (scaffold).\n");
    match capability {
        "tool" => format!(
            "{header}//!\n//! Contract: `tool_metadata` returns the tool card, `execute` runs the tool.\n\
use extism_pdk::{{plugin_fn, FnResult, Json}};\nuse serde::{{Deserialize, Serialize}};\n\n\
#[derive(Serialize)]\nstruct ToolMetadata {{\n    name: String,\n    description: String,\n    parameters_schema: serde_json::Value,\n}}\n\n\
#[derive(Serialize)]\nstruct ToolResult {{\n    success: bool,\n    output: String,\n    #[serde(skip_serializing_if = \"Option::is_none\")]\n    error: Option<String>,\n}}\n\n\
#[plugin_fn]\npub fn tool_metadata(_input: String) -> FnResult<Json<ToolMetadata>> {{\n    \
Ok(Json(ToolMetadata {{\n        name: \"{name}\".into(),\n        description: \"scaffolded demo tool — replace me\".into(),\n        \
parameters_schema: serde_json::json!({{\"type\": \"object\", \"properties\": {{}}}}),\n    }}))\n}}\n\n\
#[plugin_fn]\npub fn execute(input: String) -> FnResult<Json<ToolResult>> {{\n    \
// `input` is the tool-call arguments JSON; do the work here.\n    \
Ok(Json(ToolResult {{ success: true, output: format!(\"{name} received: {{input}}\"), error: None }}))\n}}\n"
        ),
        "observer" => format!(
            "{header}//!\n//! Contract: `observe` receives the interception envelope and returns a verdict.\n\
//! The host fails open — a broken observer never blocks a tool — so the scaffold\n\
//! default is Allow.\n\
use extism_pdk::{{plugin_fn, FnResult, Json}};\nuse serde::{{Deserialize, Serialize}};\n\n\
#[derive(Deserialize)]\nstruct ObserveEvent {{\n    point: String,\n    tool: String,\n    args: serde_json::Value,\n    result: Option<serde_json::Value>,\n    config: serde_json::Value,\n}}\n\n\
#[derive(Serialize)]\n#[serde(tag = \"action\", rename_all = \"snake_case\")]\nenum Verdict {{\n    Allow,\n    Block {{ reason: String }},\n}}\n\n\
#[plugin_fn]\npub fn observe(event: Json<ObserveEvent>) -> FnResult<Json<Verdict>> {{\n    \
let _ = event; // your policy here\n    Ok(Json(Verdict::Allow))\n}}\n"
        ),
        _ => format!(
            "{header}//!\n//! Contract (graceful degradation): `prefetch`, `sync_turn`, `tool_schemas`.\n\
//! A missing export or an odd output degrades to an empty recall / no-op sync.\n\
//! Only ONE memory plugin may be active at a time.\n\
use extism_pdk::{{plugin_fn, FnResult, Json}};\nuse serde::Deserialize;\n\n\
#[derive(Deserialize)]\nstruct PrefetchQuery {{\n    query: String,\n}}\n\n\
#[plugin_fn]\npub fn prefetch(_query: Json<PrefetchQuery>) -> FnResult<Json<serde_json::Value>> {{\n    \
Ok(Json(serde_json::json!([])))\n}}\n\n\
#[plugin_fn]\npub fn sync_turn(_input: String) -> FnResult<Json<serde_json::Value>> {{\n    \
Ok(Json(serde_json::json!({{}})))\n}}\n\n\
#[plugin_fn]\npub fn tool_schemas(_input: String) -> FnResult<Json<serde_json::Value>> {{\n    \
Ok(Json(serde_json::json!([])))\n}}\n"
        ),
    }
}

fn skill_md_for(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: scaffolded skill — replace this description\n---\n\n\
# {name}\n\nInstructions the agent follows when this skill is invoked. Replace with real steps.\n"
    )
}

/// Validate that a plugin directory has a usable manifest.
///
/// `manifest.toml` is the ONE manifest authority (the WASM host's format —
/// validated with the host's own parser, so an install-time rejection names
/// the same defect discovery would); `plugin.yaml` / `__init__.py` are the
/// python-era fallbacks kept for old installs.
pub fn validate_plugin(plugin_dir: &Path) -> Result<()> {
    let manifest_path = plugin_dir.join("manifest.toml");
    if manifest_path.exists() {
        #[cfg(feature = "plugins-wasm")]
        return validate_manifest_toml(&manifest_path);
        #[cfg(not(feature = "plugins-wasm"))]
        return Ok(());
    }
    // Legacy order preserved: plugin.yaml was the historical primary,
    // __init__.py the python-era fallback.
    let yaml_path = plugin_dir.join("plugin.yaml");
    if yaml_path.exists() {
        return validate_plugin_yaml(&yaml_path);
    }
    let init_path = plugin_dir.join("__init__.py");
    if !init_path.exists() {
        anyhow::bail!(
            "No manifest.toml, plugin.yaml or __init__.py found in {}. Is this a valid Operant plugin?",
            plugin_dir.display()
        );
    }
    Ok(())
}

/// Validate `manifest.toml` with the host's own manifest parser — one
/// authority for what a valid plugin manifest is.
#[cfg(feature = "plugins-wasm")]
fn validate_manifest_toml(manifest_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: operant_plugins::PluginManifest = toml::from_str(&content).map_err(|e| {
        anyhow::anyhow!(
            "{} is not a valid plugin manifest: {e}",
            manifest_path.display()
        )
    })?;
    if manifest.name.trim().is_empty() {
        anyhow::bail!("manifest.toml is missing a non-empty 'name' field.");
    }
    Ok(())
}

/// The legacy plugin.yaml check, extracted unchanged.
fn validate_plugin_yaml(yaml_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(yaml_path)
        .with_context(|| format!("Failed to read {}", yaml_path.display()))?;

    // Basic YAML check: must have a name field
    if !content.contains("name:") && !content.contains("name :") {
        anyhow::bail!("plugin.yaml is missing a 'name' field.");
    }

    Ok(())
}

/// Remove an installed plugin.
pub fn remove_plugin(name: &str, plugins_dir: &Path) -> Result<()> {
    let target = plugins_dir.join(name);
    if !target.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }
    std::fs::remove_dir_all(&target)
        .with_context(|| format!("Failed to remove plugin '{}'", name))?;
    Ok(())
}

/// List installed plugins.
pub fn list_plugins(plugins_dir: &Path) -> Result<Vec<String>> {
    if !plugins_dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(plugins_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(names)
}

/// Check that git is available on the system.
pub fn check_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_resolve_git_url_https() {
        assert_eq!(
            resolve_git_url("https://github.com/owner/repo.git"),
            "https://github.com/owner/repo.git"
        );
    }

    #[test]
    fn test_resolve_git_url_git_ssh() {
        assert_eq!(
            resolve_git_url("git@github.com:owner/repo.git"),
            "git@github.com:owner/repo.git"
        );
    }

    #[test]
    fn test_resolve_git_url_owner_repo_shorthand() {
        assert_eq!(
            resolve_git_url("owner/repo"),
            "https://github.com/owner/repo"
        );
    }

    #[test]
    fn test_resolve_git_url_single_name() {
        assert_eq!(
            resolve_git_url("my-plugin"),
            "https://github.com/my-plugin/my-plugin"
        );
    }

    #[test]
    fn test_derive_plugin_name_from_url() {
        assert_eq!(
            derive_plugin_name("https://github.com/owner/my-plugin.git"),
            "my-plugin"
        );
    }

    #[test]
    fn test_derive_plugin_name_from_owner_repo() {
        assert_eq!(derive_plugin_name("owner/my-plugin"), "my-plugin");
    }

    #[test]
    fn test_derive_plugin_name_single() {
        assert_eq!(derive_plugin_name("my-plugin"), "my-plugin");
    }

    #[test]
    fn test_derive_plugin_name_with_trailing_slash() {
        assert_eq!(derive_plugin_name("owner/repo/"), "repo");
    }

    #[test]
    fn test_validate_plugin_with_plugin_yaml() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("plugin.yaml"),
            "name: test-plugin\nversion: 1.0",
        )
        .unwrap();
        assert!(validate_plugin(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_plugin_with_init_py() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("__init__.py"), "# plugin").unwrap();
        assert!(validate_plugin(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_plugin_missing_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_plugin(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No manifest.toml, plugin.yaml or __init__.py")
        );
    }

    #[test]
    fn test_validate_plugin_with_manifest_toml() {
        // iter-1721: the WASM host's format is the one manifest authority —
        // the installer validates it with the host's own parser.
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("manifest.toml"),
            "name = \"wasm-plugin\"\nversion = \"0.1.0\"\ncapabilities = [\"tool\"]\nwasm_path = \"p.wasm\"\n",
        )
        .unwrap();
        assert!(validate_plugin(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_plugin_rejects_invalid_manifest_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        // `capabilities` is required by validate_manifest_shape at discovery;
        // here the TOML itself must parse as a PluginManifest.
        fs::write(dir.path().join("manifest.toml"), "not = [valid\n").unwrap();
        let result = validate_plugin(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a valid plugin manifest")
        );
    }

    #[test]
    fn test_validate_plugin_yaml_missing_name() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("plugin.yaml"),
            "version: 1.0\ndescription: foo",
        )
        .unwrap();
        let result = validate_plugin(dir.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing a 'name' field")
        );
    }

    #[test]
    fn test_list_plugins_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let plugins = list_plugins(dir.path()).unwrap();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_list_plugins_with_entries() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir(dir.path().join("plugin-a")).unwrap();
        fs::create_dir(dir.path().join("plugin-b")).unwrap();
        let plugins = list_plugins(dir.path()).unwrap();
        assert_eq!(plugins, vec!["plugin-a", "plugin-b"]);
    }

    #[test]
    fn test_remove_plugin() {
        let dir = tempfile::TempDir::new().unwrap();
        let plugin_dir = dir.path().join("test-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        assert!(plugin_dir.exists());
        remove_plugin("test-plugin", dir.path()).unwrap();
        assert!(!plugin_dir.exists());
    }

    #[test]
    fn test_remove_nonexistent_plugin() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = remove_plugin("nonexistent", dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_check_git_available() {
        // This should be true in any dev environment with git installed
        assert!(check_git_available());
    }

    // ── scaffolding (gap G7, creator mode) ─────────────────────────────

    #[test]
    fn test_scaffold_tool_produces_a_validator_accepted_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = scaffold_plugin("demo", "tool", &tmp.path().join("demo")).unwrap();
        // the gate: the installer's own validator accepts what `new` produced
        validate_plugin(&dir).unwrap();
        let manifest = fs::read_to_string(dir.join("manifest.toml")).unwrap();
        assert!(manifest.contains("capabilities = [\"tool\"]"));
        assert!(manifest.contains("wasm_path = \"demo.wasm\""));
        assert!(dir.join("Cargo.toml").exists());
        let lib = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub fn tool_metadata"));
        assert!(lib.contains("pub fn execute"));
        let readme = fs::read_to_string(dir.join("README.md")).unwrap();
        assert!(readme.contains("tool_metadata"));
        assert!(readme.contains("parameters_schema"));
        assert!(readme.contains("wasm32-wasip1"));
    }

    #[test]
    fn test_scaffold_observer_names_the_envelope_and_verdict() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = scaffold_plugin("watch", "observer", &tmp.path().join("watch")).unwrap();
        validate_plugin(&dir).unwrap();
        let readme = fs::read_to_string(dir.join("README.md")).unwrap();
        // every contract element the envelope carries
        for needle in [
            "observe",
            "pre-execute",
            "post-execute",
            "action",
            "allow",
            "block",
            "config",
        ] {
            assert!(readme.contains(needle), "README missing '{needle}'");
        }
        let lib = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("pub fn observe"));
        assert!(lib.contains("Verdict::Allow"));
    }

    #[test]
    fn test_scaffold_skill_has_no_wasm_and_carries_a_skill_md() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = scaffold_plugin("guide", "skill", &tmp.path().join("guide")).unwrap();
        validate_plugin(&dir).unwrap();
        let manifest = fs::read_to_string(dir.join("manifest.toml")).unwrap();
        assert!(manifest.contains("capabilities = [\"skill\"]"));
        assert!(!manifest.contains("wasm_path"));
        assert!(!dir.join("Cargo.toml").exists());
        let skill = fs::read_to_string(dir.join("skills/guide/SKILL.md")).unwrap();
        assert!(skill.contains("name: guide"));
    }

    #[test]
    fn test_scaffold_memory_names_the_three_exports() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = scaffold_plugin("mem", "memory", &tmp.path().join("mem")).unwrap();
        validate_plugin(&dir).unwrap();
        let readme = fs::read_to_string(dir.join("README.md")).unwrap();
        for needle in ["prefetch", "sync_turn", "tool_schemas"] {
            assert!(readme.contains(needle), "README missing '{needle}'");
        }
    }

    #[test]
    fn test_scaffold_refuses_existing_dir_bad_names_and_capabilities() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dest = tmp.path().join("demo");
        fs::create_dir(&dest).unwrap();
        assert!(
            scaffold_plugin("demo", "tool", &dest)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert!(scaffold_plugin("a/b", "tool", &tmp.path().join("x")).is_err());
        assert!(scaffold_plugin("demo", "banana", &tmp.path().join("y")).is_err());
    }

    #[test]
    fn test_validate_rejects_a_broken_scaffold() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("broken");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("manifest.toml"), "not = [valid\n").unwrap();
        let result = validate_plugin(&dir);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a valid plugin manifest")
        );
    }
}

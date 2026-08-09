//! Skills tools — list, view, and manage skills.
//!
//! Provides three tools that mirror the Python `skills_tool.py` +
//! `skill_manager_tool.py` stack:
//!
//! - **`skills_list`** — metadata-only scan (progressive disclosure tier 1).
//! - **`skill_view`** — load full SKILL.md + supporting files (tier 2-3).
//! - **`skill_manage`** — full CRUD: create, edit, patch (find-replace),
//!   delete, write_file, remove_file.  Frontmatter validation, content size
//!   limits, and security-scan-on-write match the Python reference.

use async_trait::async_trait;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

use crate::schema::ToolSchema;
use crate::skill_usage::{SkillUsageTracker, with_exclusive_file_lock};
use crate::tools::{OperantTool, ToolContext, ToolResult};
use crate::write_origin::is_background_review;

// ── Constants ────────────────────────────────────────────────────────────────

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;
const MAX_SKILL_CONTENT_CHARS: usize = 100_000; // ~36k tokens at 2.75 chars/token
const MAX_SKILL_FILE_BYTES: usize = 1_048_576; // 1 MiB per supporting file

/// Skills that ship with Operant and must never be modified by the background
/// review agent. Matches hermes-agent's "bundled" + "hub-installed" protection.
const PROTECTED_SKILL_PREFIXES: &[&str] = &[
    "operant-agent",
    "operant-dev",
    "hermes-agent",
    "hermes-dev",
    "claw-dev",
];

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Characters allowed in skill names (filesystem-safe, URL-friendly).
static VALID_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9][a-z0-9._-]*$").expect("static regex literal is invalid — authoring bug")
});

/// Subdirectories allowed for write_file / remove_file.
const ALLOWED_SUBDIRS: &[&str] = &["references", "templates", "scripts", "assets"];

// ── Background review write guards ────────────────────────────────────────

/// Tracks which skills have been read via `skill_view` during a background
/// review session. The review agent must read a skill before modifying it
/// to prevent uninformed mutations.
static REVIEW_READ_SKILLS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// Serializes `.usage.json` read-modify-write cycles. The main agent and the
/// background-review daemon both call `skill_manage` concurrently, and other
/// operant processes may share the same skills dir — without this, two
/// interleaved non-atomic writes corrupt the telemetry file (silently
/// unpinning skills and zeroing usage data). Mirrors hermes's
/// `.usage.json.lock` (`skill_usage.py`). (R20)
static USAGE_TELEMETRY_LOCK: LazyLock<std::sync::Mutex<()>> =
    LazyLock::new(|| std::sync::Mutex::new(()));

/// Mark a skill as having been read during the current background review.
/// Called by `skill_view` when the origin is `background_review`.
pub fn mark_review_skill_read(name: &str) {
    if is_background_review()
        && let Ok(mut set) = REVIEW_READ_SKILLS.write()
    {
        set.insert(name.to_string());
    }
}

/// Check whether a skill has been read during the current background review.
fn review_has_read(name: &str) -> bool {
    REVIEW_READ_SKILLS
        .read()
        .map(|set| set.contains(name))
        .unwrap_or(false)
}

/// Reset the read tracking set. Called at the start of each background review
/// session so that a stale session doesn't leak read-tracking state.
pub fn reset_review_read_marks() {
    if let Ok(mut set) = REVIEW_READ_SKILLS.write() {
        set.clear();
    }
}

/// Check if a skill name matches a protected prefix (bundled/hub-installed).
fn is_protected_skill(name: &str) -> bool {
    PROTECTED_SKILL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix) || name == *prefix)
}

/// Check if a skill was installed from a hub (lives under `.hub/` directory).
/// Hub-installed skills are protected from background review modifications.
fn is_hub_installed(skill_dir: &Path) -> bool {
    let parent = skill_dir.parent().unwrap_or(skill_dir);
    parent.file_name().map(|n| n == ".hub").unwrap_or(false)
}

/// Guard result for background review write operations.
enum ReviewGuardResult {
    /// Write is allowed.
    Allowed,
    /// Write is blocked with a reason.
    Blocked(String),
}

/// Check if a background review is allowed to modify a skill.
///
/// Rules (matching hermes-agent):
/// 1. Bundled skills (operant-agent, hermes-agent, etc.) — NEVER edit.
/// 2. Hub-installed skills — NEVER edit.
/// 3. Pinned skills — CAN be improved (pin only blocks delete/archive).
/// 4. The review must have READ the skill before modifying it.
fn review_write_guard(name: &str, skill_dir: &Path, action: &str) -> ReviewGuardResult {
    if !is_background_review() {
        return ReviewGuardResult::Allowed;
    }

    // Block edits/patches/deletes on bundled skills
    if is_protected_skill(name) && (action == "delete" || action == "edit" || action == "patch") {
        return ReviewGuardResult::Blocked(format!(
            "Skill '{}' is a bundled/protected skill and cannot be modified by the background review.",
            name
        ));
    }

    // Block edits/patches/deletes on hub-installed skills
    if is_hub_installed(skill_dir) && (action == "delete" || action == "edit" || action == "patch")
    {
        return ReviewGuardResult::Blocked(format!(
            "Skill '{}' is hub-installed and cannot be modified by the background review.",
            name
        ));
    }

    // For edit/patch: require that the skill was read first
    if (action == "edit" || action == "patch") && !review_has_read(name) {
        return ReviewGuardResult::Blocked(format!(
            "Skill '{}' has not been read via skill_view. Read it first before modifying.",
            name
        ));
    }

    ReviewGuardResult::Allowed
}

// ── Frontmatter parsing ──────────────────────────────────────────────────────

fn parse_frontmatter(content: &str) -> (serde_json::Value, String) {
    let trimmed = content.trim_start_matches('\u{feff}'); // strip BOM
    if !trimmed.starts_with("---") {
        return (serde_json::Value::Null, trimmed.to_string());
    }
    let parts: Vec<&str> = trimmed.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (serde_json::Value::Null, trimmed.to_string());
    }
    let yaml_str = parts[1].trim();
    let body = parts[2].trim();
    let frontmatter: serde_json::Value =
        serde_yaml::from_str(yaml_str).unwrap_or(serde_json::Value::Null);
    (frontmatter, body.to_string())
}

/// Validate that SKILL.md content has proper frontmatter with required fields.
fn validate_frontmatter(content: &str) -> Option<String> {
    let trimmed = content.trim_start_matches('\u{feff}');
    if trimmed.is_empty() {
        return Some("Content cannot be empty.".into());
    }
    if !trimmed.starts_with("---") {
        return Some(
            "SKILL.md must start with YAML frontmatter (---). See existing skills for format."
                .into(),
        );
    }
    let parts: Vec<&str> = trimmed.splitn(3, "---").collect();
    if parts.len() < 3 {
        return Some(
            "SKILL.md frontmatter is not closed. Ensure you have a closing '---' line.".into(),
        );
    }
    let yaml_str = parts[1].trim();
    let body = parts[2].trim();

    match serde_yaml::from_str::<serde_json::Value>(yaml_str) {
        Ok(val) => {
            if !val.is_object() {
                return Some("Frontmatter must be a YAML mapping (key: value pairs).".into());
            }
            if val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
            {
                return Some("Frontmatter must include 'name' field.".into());
            }
            if val
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
            {
                return Some("Frontmatter must include 'description' field.".into());
            }
            if body.is_empty() {
                return Some(
                    "SKILL.md must have content after the frontmatter (instructions, procedures, etc.)."
                        .into(),
                );
            }
        }
        Err(e) => return Some(format!("YAML frontmatter parse error: {}", e)),
    }
    None
}

/// Check that content doesn't exceed the character limit for agent writes.
fn validate_content_size(content: &str, label: &str) -> Option<String> {
    if content.len() > MAX_SKILL_CONTENT_CHARS {
        Some(format!(
            "{} content is {} characters (limit: {}). Consider splitting into a smaller SKILL.md with supporting files in references/ or templates/.",
            label,
            content.len(),
            MAX_SKILL_CONTENT_CHARS
        ))
    } else {
        None
    }
}

// ── Skill discovery helpers ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct SkillMeta {
    name: String,
    description: String,
    category: Option<String>,
}

fn find_skills_in_dir(skills_dir: &Path) -> Vec<SkillMeta> {
    let mut skills = Vec::new();
    if !skills_dir.exists() {
        return skills;
    }
    collect_skills_recursive(skills_dir, skills_dir, &mut skills);
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn collect_skills_recursive(base_dir: &Path, current_dir: &Path, skills: &mut Vec<SkillMeta>) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.')
            || dir_name == "node_modules"
            || dir_name == ".archive"
            || dir_name == ".hub"
        {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if skill_md.exists() {
            if let Ok(content) = fs::read_to_string(&skill_md) {
                let (frontmatter, body) = parse_frontmatter(&content);
                let name = frontmatter
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(dir_name)
                    .chars()
                    .take(MAX_NAME_LENGTH)
                    .collect::<String>();
                let description = frontmatter
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        if s.len() > MAX_DESCRIPTION_LENGTH {
                            format!("{}...", &s[..MAX_DESCRIPTION_LENGTH - 3])
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| {
                        body.lines()
                            .find(|l| !l.trim().starts_with('#'))
                            .map(|l| l.trim().to_string())
                            .unwrap_or_default()
                    });
                let category = path
                    .parent()
                    .filter(|p| *p != base_dir)
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());
                skills.push(SkillMeta {
                    name,
                    description,
                    category,
                });
            }
        } else {
            collect_skills_recursive(base_dir, &path, skills);
        }
    }
}

/// Find a skill by name across all skill directories.
fn find_skill(skills_dir: &Path, name: &str) -> Option<PathBuf> {
    if !skills_dir.exists() {
        return None;
    }
    // Direct lookup
    let direct = skills_dir.join(name);
    if direct.is_dir() && direct.join("SKILL.md").exists() {
        return Some(direct);
    }
    // Recursive search (category/subcategory/skill)
    find_skill_recursive(skills_dir, skills_dir, name)
}

fn find_skill_recursive(_base_dir: &Path, current_dir: &Path, name: &str) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.')
            || dir_name == "node_modules"
            || dir_name == ".archive"
            || dir_name == ".hub"
        {
            continue;
        }
        if dir_name == name && path.join("SKILL.md").exists() {
            return Some(path);
        }
        if let Some(found) = find_skill_recursive(_base_dir, &path, name) {
            return Some(found);
        }
    }
    None
}

/// Resolve a supporting-file path and ensure it stays within the skill directory.
/// Uses lexical path checking (no canonicalize) so it works for new files too.
fn resolve_skill_target(skill_dir: &Path, file_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(file_path);
    // Reject absolute paths — Path::join replaces the entire path on Unix
    if p.is_absolute() {
        return Err("Absolute paths are not allowed.".into());
    }
    let target = skill_dir.join(file_path);
    // Lexical traversal check: reject paths with `..` components that escape
    // the skill directory. Works for new files (no canonicalize needed).
    let mut depth: i32 = 0;
    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err("Path traversal ('..') is not allowed.".into());
                }
            }
            std::path::Component::Normal(_) => {
                depth += 1;
            }
            _ => {}
        }
    }
    Ok(target)
}

/// Validate a file path for write_file / remove_file.
fn validate_file_path(file_path: &str) -> Option<String> {
    if file_path.is_empty() {
        return Some("file_path is required.".into());
    }
    let normalized = Path::new(file_path);
    // SKILL.md is at the skill root
    if normalized
        .file_name()
        .map(|n| n == "SKILL.md")
        .unwrap_or(false)
        && normalized.components().count() <= 2
    {
        return None;
    }
    // Must be under an allowed subdirectory
    match normalized.components().next() {
        Some(std::path::Component::Normal(first)) => {
            if !ALLOWED_SUBDIRS.contains(&first.to_string_lossy().as_ref()) {
                let allowed = ALLOWED_SUBDIRS.join(", ");
                return Some(format!(
                    "File must be under one of: {}. Got: '{}'",
                    allowed, file_path
                ));
            }
        }
        _ => {
            return Some(format!(
                "File must be under one of: {}. Got: '{}'",
                ALLOWED_SUBDIRS.join(", "),
                file_path
            ));
        }
    }
    if normalized.components().count() < 2 {
        return Some(format!(
            "Provide a file path, not just a directory. Example: '{}/myfile.md'",
            normalized
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .unwrap_or_default()
        ));
    }
    None
}

// ── SkillsTool (skills_list) ────────────────────────────────────────────────

pub struct SkillsTool {
    root_dir: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct SkillsListArgs {
    category: Option<String>,
}

impl SkillsTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }
}

#[async_trait]
impl OperantTool for SkillsTool {
    fn name(&self) -> &str {
        "skills_list"
    }

    fn description(&self) -> &str {
        "List all available skills with metadata. Use skill_view to load full content."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillsListArgs>(
            "skills_list",
            "List all available skills with metadata (name, description, category)",
        )
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        let skills_dir = &self.root_dir;
        if !skills_dir.exists() {
            if let Err(e) = fs::create_dir_all(skills_dir) {
                return ToolResult::error(
                    "skills_list",
                    format!("Failed to create skills directory: {}", e),
                );
            }
            return ToolResult::success(
                "skills_list",
                json!({ "skills": [], "categories": [], "message": "No skills found. Skills directory created." }),
            );
        }
        let skills = find_skills_in_dir(skills_dir);
        if skills.is_empty() {
            return ToolResult::success(
                "skills_list",
                json!({ "skills": [], "categories": [], "message": "No skills found in skills/ directory." }),
            );
        }
        let categories: Vec<String> = skills
            .iter()
            .filter_map(|s| s.category.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        ToolResult::success(
            "skills_list",
            json!({
                "skills": skills,
                "categories": categories,
                "count": skills.len(),
                "hint": "Use skill_view to see full content"
            }),
        )
    }
}

// ── SkillViewTool ────────────────────────────────────────────────────────────

pub struct SkillViewTool {
    root_dir: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct SkillViewArgs {
    name: String,
    file_path: Option<String>,
}

impl SkillViewTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }
}

#[async_trait]
impl OperantTool for SkillViewTool {
    fn name(&self) -> &str {
        "skill_view"
    }

    fn description(&self) -> &str {
        "View the full content of a skill by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillViewArgs>(
            "skill_view",
            "View the full content of a skill including instructions, tags, and linked files",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return ToolResult::error("skill_view", "name is required"),
        };
        let skills_dir = &self.root_dir;
        if !skills_dir.exists() {
            return ToolResult::error(
                "skill_view",
                "Skills directory does not exist. It will be created on first install.",
            );
        }

        // If file_path is specified, load a supporting file
        if let Some(file_path) = args.get("file_path").and_then(|v| v.as_str()) {
            let Some(skill_dir) = find_skill(skills_dir, name) else {
                return ToolResult::error("skill_view", format!("Skill '{}' not found", name));
            };
            // Track that this skill was read during background review
            mark_review_skill_read(name);
            let target = match resolve_skill_target(&skill_dir, file_path) {
                Ok(t) => t,
                Err(e) => return ToolResult::error("skill_view", e),
            };
            if !target.exists() {
                return ToolResult::error(
                    "skill_view",
                    format!("File '{}' not found in skill '{}'", file_path, name),
                );
            }
            return match fs::read_to_string(&target) {
                Ok(content) => ToolResult::success(
                    "skill_view",
                    json!({
                        "name": name,
                        "file_path": file_path,
                        "content": content,
                        "path": target.to_string_lossy()
                    }),
                ),
                Err(e) => ToolResult::error("skill_view", format!("Failed to read: {}", e)),
            };
        }

        let skill_dir = match find_skill(skills_dir, name) {
            Some(d) => {
                // Track that this skill was read during background review
                mark_review_skill_read(name);
                d
            }
            None => {
                let available: Vec<String> = find_skills_in_dir(skills_dir)
                    .iter()
                    .take(20)
                    .map(|s| s.name.clone())
                    .collect();
                return ToolResult::error(
                    "skill_view",
                    format!("Skill '{}' not found. Available: {:?}", name, available),
                );
            }
        };
        let skill_md = skill_dir.join("SKILL.md");
        match fs::read_to_string(&skill_md) {
            Ok(content) => {
                let (frontmatter, body) = parse_frontmatter(&content);
                let skill_name = frontmatter
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(name)
                    .to_string();
                let description = frontmatter
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let tags: Vec<String> = frontmatter
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                // List supporting files
                let supporting_files: Vec<String> = fs::read_dir(&skill_dir)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n != "SKILL.md")
                    .collect();
                ToolResult::success(
                    "skill_view",
                    json!({
                        "name": skill_name,
                        "description": description,
                        "content": body,
                        "tags": tags,
                        "supporting_files": supporting_files,
                        "path": skill_md.to_string_lossy()
                    }),
                )
            }
            Err(e) => ToolResult::error("skill_view", format!("Failed to read skill: {}", e)),
        }
    }
}

// ── SkillManageTool — full CRUD ──────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillManageArgs {
    /// Action: "create", "edit", "patch", "delete", "write_file", "remove_file"
    action: String,
    /// Skill name (directory name)
    name: String,
    /// Full SKILL.md content (required for create/edit)
    content: Option<String>,
    /// Category for organize (create only)
    category: Option<String>,
    /// Supporting file path (write_file/remove_file/patch)
    file_path: Option<String>,
    /// File content (write_file)
    file_content: Option<String>,
    /// Text to find (patch)
    old_string: Option<String>,
    /// Replacement text (patch)
    new_string: Option<String>,
    /// Replace all occurrences (patch)
    replace_all: Option<bool>,
}

pub struct SkillManageTool {
    root_dir: PathBuf,
}

impl SkillManageTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            root_dir: skills_dir,
        }
    }

    fn validate_name(name: &str) -> Option<String> {
        if name.is_empty() {
            return Some("Skill name is required.".into());
        }
        if name.len() > MAX_NAME_LENGTH {
            return Some(format!(
                "Skill name exceeds {} characters.",
                MAX_NAME_LENGTH
            ));
        }
        if !VALID_NAME_RE.is_match(name) {
            return Some(format!(
                "Invalid skill name '{}'. Use lowercase letters, numbers, hyphens, dots, and underscores. Must start with a letter or digit.",
                name
            ));
        }
        None
    }
}

#[async_trait]
impl OperantTool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Manage skills: create, edit (full rewrite), patch (find-replace), delete, write_file, remove_file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SkillManageArgs>(
            "skill_manage",
            "Create, edit, patch, delete, write_file, remove_file for skills",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: SkillManageArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("skill_manage", format!("Invalid args: {}", e)),
        };

        match parsed.action.as_str() {
            "create" => self.action_create(&parsed).await,
            "edit" => self.action_edit(&parsed).await,
            "patch" => self.action_patch(&parsed).await,
            "delete" => self.action_delete(&parsed).await,
            "write_file" => self.action_write_file(&parsed).await,
            "remove_file" => self.action_remove_file(&parsed).await,
            other => ToolResult::error(
                "skill_manage",
                format!(
                    "Unknown action '{}'. Use: create, edit, patch, delete, write_file, remove_file",
                    other
                ),
            ),
        }
    }
}

impl SkillManageTool {
    // ── create ──────────────────────────────────────────────────────
    async fn action_create(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(content) = &parsed.content else {
            return ToolResult::error(
                "skill_manage",
                "content is required for create. Provide the full SKILL.md text (frontmatter + body).",
            );
        };
        if let Some(err) = Self::validate_name(&parsed.name) {
            return ToolResult::error("skill_manage", err);
        }
        if let Some(err) = validate_frontmatter(content) {
            return ToolResult::error("skill_manage", err);
        }
        if let Some(err) = validate_content_size(content, "SKILL.md") {
            return ToolResult::error("skill_manage", err);
        }
        let skill_dir = if let Some(cat) = &parsed.category {
            self.root_dir.join(cat).join(&parsed.name)
        } else {
            self.root_dir.join(&parsed.name)
        };
        if skill_dir.exists() {
            return ToolResult::error(
                "skill_manage",
                format!("A skill named '{}' already exists", parsed.name),
            );
        }
        if let Err(e) = fs::create_dir_all(&skill_dir) {
            return ToolResult::error("skill_manage", format!("Failed to create directory: {}", e));
        }
        let skill_md = skill_dir.join("SKILL.md");
        if let Err(e) = fs::write(&skill_md, content) {
            let _ = fs::remove_dir_all(&skill_dir);
            return ToolResult::error("skill_manage", format!("Failed to write SKILL.md: {}", e));
        }

        // Record usage telemetry
        self.record_usage(&parsed.name, "create");
        ToolResult::success(
            "skill_manage",
            json!({
                "action": "create",
                "name": parsed.name,
                "message": format!("Skill '{}' created.", parsed.name),
                "path": skill_md.to_string_lossy()
            }),
        )
    }

    // ── edit (full rewrite) ──────────────────────────────────────────
    async fn action_edit(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(content) = &parsed.content else {
            return ToolResult::error(
                "skill_manage",
                "content is required for edit. Provide the full updated SKILL.md text.",
            );
        };
        if let Some(err) = validate_frontmatter(content) {
            return ToolResult::error("skill_manage", err);
        }
        if let Some(err) = validate_content_size(content, "SKILL.md") {
            return ToolResult::error("skill_manage", err);
        }
        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error("skill_manage", format!("Skill '{}' not found", parsed.name));
        };
        // Background review write guard
        if let ReviewGuardResult::Blocked(msg) =
            review_write_guard(&parsed.name, &skill_dir, "edit")
        {
            return ToolResult::error("skill_manage", msg);
        }
        let skill_md = skill_dir.join("SKILL.md");
        // Backup original for rollback
        let original = fs::read_to_string(&skill_md).ok();
        if let Err(e) = fs::write(&skill_md, content) {
            return ToolResult::error("skill_manage", format!("Failed to write: {}", e));
        }
        self.record_usage(&parsed.name, "edit");
        let msg = if original.is_some() {
            format!("Skill '{}' updated (full rewrite).", parsed.name)
        } else {
            format!("Skill '{}' written.", parsed.name)
        };
        ToolResult::success(
            "skill_manage",
            json!({ "action": "edit", "name": parsed.name, "message": msg }),
        )
    }

    // ── patch (find-replace) ─────────────────────────────────────────
    async fn action_patch(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(old_string) = &parsed.old_string else {
            return ToolResult::error("skill_manage", "old_string is required for patch.");
        };
        let Some(new_string) = &parsed.new_string else {
            return ToolResult::error(
                "skill_manage",
                "new_string is required for patch. Use an empty string to delete matched text.",
            );
        };
        let replace_all = parsed.replace_all.unwrap_or(false);

        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error("skill_manage", format!("Skill '{}' not found", parsed.name));
        };
        // Background review write guard
        if let ReviewGuardResult::Blocked(msg) =
            review_write_guard(&parsed.name, &skill_dir, "patch")
        {
            return ToolResult::error("skill_manage", msg);
        }

        // Determine target file
        let target = if let Some(fp) = &parsed.file_path {
            if let Some(err) = validate_file_path(fp) {
                return ToolResult::error("skill_manage", err);
            }
            match resolve_skill_target(&skill_dir, fp) {
                Ok(t) => t,
                Err(e) => return ToolResult::error("skill_manage", e),
            }
        } else {
            skill_dir.join("SKILL.md")
        };

        if !target.exists() {
            let label = parsed.file_path.as_deref().unwrap_or("SKILL.md");
            return ToolResult::error(
                "skill_manage",
                format!("File '{}' not found in skill '{}'", label, parsed.name),
            );
        }

        let content = match fs::read_to_string(&target) {
            Ok(c) => c,
            Err(e) => return ToolResult::error("skill_manage", format!("Failed to read: {}", e)),
        };

        // Find-replace
        if replace_all {
            let count = content.matches(old_string.as_str()).count();
            if count == 0 {
                return ToolResult::error(
                    "skill_manage",
                    format!(
                        "old_string not found in {}",
                        target.file_name().unwrap_or_default().to_string_lossy()
                    ),
                );
            }
            let updated = content.replace(old_string.as_str(), new_string);
            if let Some(err) = validate_content_size(&updated, "SKILL.md") {
                return ToolResult::error("skill_manage", err);
            }
            // If patching SKILL.md, validate frontmatter still intact
            if target.file_name().map(|n| n == "SKILL.md").unwrap_or(false)
                && let Some(err) = validate_frontmatter(&updated)
            {
                return ToolResult::error(
                    "skill_manage",
                    format!("Patch would break SKILL.md structure: {}", err),
                );
            }
            if let Err(e) = fs::write(&target, &updated) {
                return ToolResult::error("skill_manage", format!("Failed to write: {}", e));
            }
            self.record_usage(&parsed.name, "patch");
            return ToolResult::success(
                "skill_manage",
                json!({
                    "action": "patch",
                    "name": parsed.name,
                    "message": format!("Patched {} ({} replacement(s)).", target.file_name().unwrap_or_default().to_string_lossy(), count),
                }),
            );
        }

        // Single match
        let occurrences = content.matches(old_string.as_str()).count();
        if occurrences == 0 {
            return ToolResult::error(
                "skill_manage",
                format!(
                    "old_string not found in {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
        }
        if occurrences > 1 {
            return ToolResult::error(
                "skill_manage",
                format!(
                    "old_string found {} times. Use replace_all=true or provide more context for a unique match.",
                    occurrences
                ),
            );
        }
        let updated = content.replacen(old_string.as_str(), new_string, 1);
        if let Some(err) = validate_content_size(&updated, "SKILL.md") {
            return ToolResult::error("skill_manage", err);
        }
        if target.file_name().map(|n| n == "SKILL.md").unwrap_or(false)
            && let Some(err) = validate_frontmatter(&updated)
        {
            return ToolResult::error(
                "skill_manage",
                format!("Patch would break SKILL.md structure: {}", err),
            );
        }
        if let Err(e) = fs::write(&target, &updated) {
            return ToolResult::error("skill_manage", format!("Failed to write: {}", e));
        }
        self.record_usage(&parsed.name, "patch");
        ToolResult::success(
            "skill_manage",
            json!({
                "action": "patch",
                "name": parsed.name,
                "message": format!("Patched {} (1 replacement).", target.file_name().unwrap_or_default().to_string_lossy()),
            }),
        )
    }

    // ── delete ───────────────────────────────────────────────────────
    async fn action_delete(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error("skill_manage", format!("Skill '{}' not found", parsed.name));
        };
        // Background review write guard
        if let ReviewGuardResult::Blocked(msg) =
            review_write_guard(&parsed.name, &skill_dir, "delete")
        {
            return ToolResult::error("skill_manage", msg);
        }
        // Check pinned status before deleting
        if self.is_pinned(&parsed.name) {
            return ToolResult::error(
                "skill_manage",
                format!(
                    "Skill '{}' is pinned and cannot be deleted. Unpin it first.",
                    parsed.name
                ),
            );
        }
        if let Err(e) = fs::remove_dir_all(&skill_dir) {
            return ToolResult::error("skill_manage", format!("Failed to delete: {}", e));
        }
        // Clean up empty category directories
        if let Some(parent) = skill_dir.parent()
            && parent != self.root_dir
            && parent.exists()
            && fs::read_dir(parent)
                .map(|mut e| e.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(parent);
        }
        self.record_usage(&parsed.name, "delete");
        ToolResult::success(
            "skill_manage",
            json!({ "action": "delete", "name": parsed.name, "message": format!("Skill '{}' deleted.", parsed.name) }),
        )
    }

    // ── write_file ───────────────────────────────────────────────────
    async fn action_write_file(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(fp) = &parsed.file_path else {
            return ToolResult::error("skill_manage", "file_path is required for write_file.");
        };
        let Some(file_content) = &parsed.file_content else {
            return ToolResult::error("skill_manage", "file_content is required for write_file.");
        };
        if let Some(err) = validate_file_path(fp) {
            return ToolResult::error("skill_manage", err);
        }
        let content_bytes = file_content.len();
        if content_bytes > MAX_SKILL_FILE_BYTES {
            return ToolResult::error(
                "skill_manage",
                format!(
                    "File content is {} bytes (limit: {} bytes / 1 MiB).",
                    content_bytes, MAX_SKILL_FILE_BYTES
                ),
            );
        }
        if let Some(err) = validate_content_size(file_content, fp) {
            return ToolResult::error("skill_manage", err);
        }
        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error(
                "skill_manage",
                format!("Skill '{}' not found. Create it first.", parsed.name),
            );
        };
        let target = match resolve_skill_target(&skill_dir, fp) {
            Ok(t) => t,
            Err(e) => return ToolResult::error("skill_manage", e),
        };
        if let Some(parent) = target.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            return ToolResult::error("skill_manage", format!("Failed to create dir: {}", e));
        }
        if let Err(e) = fs::write(&target, file_content) {
            return ToolResult::error("skill_manage", format!("Failed to write: {}", e));
        }
        self.record_usage(&parsed.name, "write_file");
        ToolResult::success(
            "skill_manage",
            json!({
                "action": "write_file",
                "name": parsed.name,
                "file_path": fp,
                "message": format!("File '{}' written to skill '{}'.", fp, parsed.name),
            }),
        )
    }

    // ── remove_file ──────────────────────────────────────────────────
    async fn action_remove_file(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(fp) = &parsed.file_path else {
            return ToolResult::error("skill_manage", "file_path is required for remove_file.");
        };
        if let Some(err) = validate_file_path(fp) {
            return ToolResult::error("skill_manage", err);
        }
        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error("skill_manage", format!("Skill '{}' not found", parsed.name));
        };
        let target = match resolve_skill_target(&skill_dir, fp) {
            Ok(t) => t,
            Err(e) => return ToolResult::error("skill_manage", e),
        };
        if !target.exists() {
            // List what's actually there
            let mut available = Vec::new();
            for subdir in ALLOWED_SUBDIRS {
                let d = skill_dir.join(subdir);
                if d.exists()
                    && let Ok(entries) = fs::read_dir(&d)
                {
                    for entry in entries.flatten() {
                        if entry.path().is_file() {
                            available.push(
                                entry
                                    .path()
                                    .strip_prefix(&skill_dir)
                                    .unwrap_or(&entry.path())
                                    .to_string_lossy()
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            return ToolResult::error(
                "skill_manage",
                format!(
                    "File '{}' not found in skill '{}'. Available: {:?}",
                    fp, parsed.name, available
                ),
            );
        }
        if let Err(e) = fs::remove_file(&target) {
            return ToolResult::error("skill_manage", format!("Failed to remove: {}", e));
        }
        // Clean up empty subdirectories
        if let Some(parent) = target.parent()
            && parent != skill_dir
            && parent.exists()
            && fs::read_dir(parent)
                .map(|mut e| e.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(parent);
        }
        self.record_usage(&parsed.name, "remove_file");
        ToolResult::success(
            "skill_manage",
            json!({
                "action": "remove_file",
                "name": parsed.name,
                "file_path": fp,
                "message": format!("File '{}' removed from skill '{}'.", fp, parsed.name),
            }),
        )
    }

    // ── Pinned check ────────────────────────────────────────────────
    fn is_pinned(&self, name: &str) -> bool {
        let usage_path = self.root_dir.join(".usage.json");
        if !usage_path.exists() {
            return false;
        }
        let telemetry: std::collections::HashMap<String, serde_json::Value> =
            fs::read_to_string(&usage_path)
                .ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default();
        telemetry
            .get(name)
            .and_then(|v| v.get("pinned"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    // ── Telemetry ────────────────────────────────────────────────────
    fn record_usage(&self, name: &str, action: &str) {
        // Serialize the read-modify-write across every writer of these sidecar
        // files: in-process (main agent task + background-review daemon) via
        // USAGE_TELEMETRY_LOCK, and across processes sharing the skills dir
        // (e.g. `operant curator`, concurrent agent runs) via an OS advisory
        // lock on the sidecar files themselves. (R20: the previous direct
        // `fs::write` could interleave two writers and corrupt `.usage.json`,
        // silently unpinning skills and zeroing telemetry; R21: the OS lock
        // also covers the curator-tracker write below.)
        let _guard = USAGE_TELEMETRY_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let usage_path = self.root_dir.join(".usage.json");
        with_exclusive_file_lock(&usage_path, || {
            let mut telemetry: std::collections::HashMap<String, serde_json::Value> =
                if usage_path.exists() {
                    fs::read_to_string(&usage_path)
                        .ok()
                        .and_then(|c| serde_json::from_str(&c).ok())
                        .unwrap_or_default()
                } else {
                    std::collections::HashMap::new()
                };
            let entry = telemetry
                .entry(name.to_string())
                .or_insert_with(|| json!({ "use_count": 0, "patch_count": 0 }));
            if action == "delete" {
                telemetry.remove(name);
            } else if action == "patch"
                || action == "edit"
                || action == "write_file"
                || action == "remove_file"
            {
                if let Some(obj) = entry.as_object_mut() {
                    let count = obj.get("patch_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    obj.insert("patch_count".into(), json!(count + 1));
                }
            } else if action == "create"
                && let Some(obj) = entry.as_object_mut()
            {
                obj.insert("created_by".into(), json!("agent"));
            }
            if let Some(parent) = usage_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(content) = serde_json::to_string_pretty(&telemetry) {
                atomic_write_json(&usage_path, &content);
            }

            // R21: bridge real agent activity into the curator tracker so the
            // archival pipeline has data to work on (hermes
            // skill_manager_tool.py parity — record_created / bump_patch /
            // forget go to the same `.curator/usage.json` the curator reads).
            // `with_exclusive_lock` re-loads fresh state from disk inside the
            // OS lock, so concurrent skill_manage actions (main agent +
            // background-review daemon, or a separate `operant curator`
            // process) never lose curator records, and a corrupt sidecar
            // self-heals on the next save.
            let tracker = SkillUsageTracker::new(self.root_dir.join(".curator").join("usage.json"));
            let _ = tracker.with_exclusive_lock(|t| {
                match action {
                    "create" => t.record_created(name, is_background_review()),
                    "delete" => t.remove(name),
                    "patch" | "edit" | "write_file" | "remove_file" => t.bump_patch(name),
                    _ => {}
                }
                Ok(())
            });
        });
    }
}

/// Write a JSON file atomically: write to a sibling temp file, then rename
/// over the target. A crash or concurrent reader never observes a
/// partially-written file. (R20)
fn atomic_write_json(path: &Path, content: &str) {
    let tmp = path.with_extension("json.tmp");
    if let Some(parent) = tmp.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::write(&tmp, content).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_env() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();
        let skill_path = skills_dir.join("test-skill");
        std::fs::create_dir(&skill_path).unwrap();
        std::fs::write(
            skill_path.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\n---\n\n# Test\n\nBody here.\n",
        )
        .unwrap();
        (dir, skills_dir)
    }

    fn sample_skill_md() -> &'static str {
        "---\nname: new-skill\ndescription: A new skill\n---\n\n# New\n\nInstructions here.\n"
    }

    #[test]
    fn test_parse_frontmatter_valid() {
        let (fm, body) = parse_frontmatter(sample_skill_md());
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("new-skill"));
        assert!(body.contains("Instructions here."));
    }

    #[test]
    fn record_usage_bridges_curator_tracker_on_create() {
        let (_tmp, skills_dir) = setup_test_env();
        let tool = SkillManageTool::new(skills_dir.clone());
        // Simulate the background-review fork creating a skill.
        let _guard = crate::write_origin::WriteOriginGuard::background_review();
        tool.record_usage("test-skill", "create");

        let usage_path = skills_dir.join(".curator").join("usage.json");
        let content = std::fs::read_to_string(&usage_path).expect("curator usage file written");
        let records: Vec<crate::skill_usage::UsageRecord> =
            serde_json::from_str(&content).expect("valid JSON");
        let rec = records
            .iter()
            .find(|r| r.name == "test-skill")
            .expect("create recorded in curator tracker");
        assert!(rec.agent_created, "review-created skills are agent-managed");
        assert_eq!(rec.provenance.as_deref(), Some("agent"));
    }

    #[test]
    fn record_usage_bridge_patches_and_forgets() {
        let (_tmp, skills_dir) = setup_test_env();
        let tool = SkillManageTool::new(skills_dir.clone());
        tool.record_usage("test-skill", "patch");

        let usage_path = skills_dir.join(".curator").join("usage.json");
        let content = std::fs::read_to_string(&usage_path).expect("curator usage file written");
        let records: Vec<crate::skill_usage::UsageRecord> =
            serde_json::from_str(&content).expect("valid JSON");
        let rec = records
            .iter()
            .find(|r| r.name == "test-skill")
            .expect("patch recorded in curator tracker");
        assert!(!rec.agent_created, "patches don't mark agent-created");
        assert!(rec.last_used.timestamp() > 0, "last_used bumped");

        tool.record_usage("test-skill", "delete");
        let content = std::fs::read_to_string(&usage_path).expect("file still present");
        let records: Vec<crate::skill_usage::UsageRecord> =
            serde_json::from_str(&content).expect("valid JSON");
        assert!(
            records.iter().all(|r| r.name != "test-skill"),
            "deleted skills are forgotten by the tracker"
        );
    }

    #[test]
    fn record_usage_bridge_tolerates_corrupt_curator_file() {
        let (_tmp, skills_dir) = setup_test_env();
        let curator_dir = skills_dir.join(".curator");
        std::fs::create_dir_all(&curator_dir).expect("create .curator dir");
        std::fs::write(curator_dir.join("usage.json"), "{not-json").expect("write corrupt file");
        let tool = SkillManageTool::new(skills_dir.clone());
        // Must not panic or fail; the corrupt sidecar self-heals.
        tool.record_usage("test-skill", "patch");
        let content =
            std::fs::read_to_string(curator_dir.join("usage.json")).expect("file present");
        let records: Vec<crate::skill_usage::UsageRecord> =
            serde_json::from_str(&content).expect("self-healed to valid JSON");
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn concurrent_record_usage_preserves_telemetry_file() {
        // The main agent and the background-review daemon both call
        // `skill_manage` concurrently; a non-atomic `.usage.json` write would
        // corrupt the file. The telemetry must remain valid JSON with intact
        // counts after concurrent writers. (R20)
        let (_tmp, skills_dir) = setup_test_env();

        let mut handles = Vec::new();
        for i in 0..8 {
            let dir = skills_dir.clone();
            handles.push(std::thread::spawn(move || {
                let tool = SkillManageTool::new(dir);
                for _ in 0..25 {
                    tool.record_usage(&format!("skill-{}", i % 4), "patch");
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let usage_path = skills_dir.join(".usage.json");
        let content = fs::read_to_string(&usage_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .expect("usage telemetry must remain valid JSON after concurrent writes");
        // 4 skills × 8 threads × 25 patches = 200 total patch_count.
        let total: u64 = parsed
            .as_object()
            .expect("telemetry must be an object")
            .values()
            .filter_map(|v| v.get("patch_count").and_then(|c| c.as_u64()))
            .sum();
        assert_eq!(total, 200, "no patch_count may be lost to write races");
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let (fm, body) = parse_frontmatter("Just text");
        assert!(fm.is_null());
        assert_eq!(body, "Just text");
    }

    #[test]
    fn test_validate_frontmatter_ok() {
        assert!(validate_frontmatter(sample_skill_md()).is_none());
    }

    #[test]
    fn test_validate_frontmatter_missing_name() {
        let content = "---\ndescription: desc\n---\n\nBody\n";
        assert!(validate_frontmatter(content).is_some());
    }

    #[test]
    fn test_validate_frontmatter_empty_body() {
        let content = "---\nname: x\ndescription: y\n---\n";
        assert!(validate_frontmatter(content).is_some());
    }

    #[test]
    fn test_validate_content_size_ok() {
        assert!(validate_content_size("small", "test").is_none());
    }

    #[test]
    fn test_validate_content_size_too_large() {
        let big = "x".repeat(MAX_SKILL_CONTENT_CHARS + 1);
        assert!(validate_content_size(&big, "test").is_some());
    }

    #[test]
    fn test_validate_file_path_ok() {
        assert!(validate_file_path("references/api.md").is_none());
    }

    #[test]
    fn test_validate_file_path_traversal() {
        assert!(validate_file_path("../etc/passwd").is_some());
    }

    #[test]
    fn test_validate_file_path_bad_dir() {
        assert!(validate_file_path("hack/exploit.sh").is_some());
    }

    #[test]
    fn test_validate_name_ok() {
        assert!(SkillManageTool::validate_name("my-skill_1").is_none());
    }

    #[test]
    fn test_validate_name_empty() {
        assert!(SkillManageTool::validate_name("").is_some());
    }

    #[test]
    fn test_validate_name_too_long() {
        let long = "a".repeat(MAX_NAME_LENGTH + 1);
        assert!(SkillManageTool::validate_name(&long).is_some());
    }

    #[test]
    fn test_validate_name_invalid_chars() {
        assert!(SkillManageTool::validate_name("My Skill!").is_some());
    }

    #[test]
    fn test_find_skill_direct() {
        let (_dir, skills_dir) = setup_test_env();
        assert!(find_skill(&skills_dir, "test-skill").is_some());
    }

    #[test]
    fn test_find_skill_recursive() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let cat_dir = skills_dir.join("devops");
        let skill_dir = cat_dir.join("my-deploy");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-deploy\ndescription: Deploy\n---\n\nGo.\n",
        )
        .unwrap();
        assert!(find_skill(&skills_dir, "my-deploy").is_some());
    }

    #[test]
    fn test_find_skill_not_found() {
        let (_dir, skills_dir) = setup_test_env();
        assert!(find_skill(&skills_dir, "nonexistent").is_none());
    }

    #[test]
    fn test_skills_list_name() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillsTool::new(skills_dir);
        assert_eq!(tool.name(), "skills_list");
    }

    #[test]
    fn test_skill_view_name() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillViewTool::new(skills_dir);
        assert_eq!(tool.name(), "skill_view");
    }

    #[test]
    fn test_skill_manage_name() {
        let (_dir, skills_dir) = setup_test_env();
        let tool = SkillManageTool::new(skills_dir);
        assert_eq!(tool.name(), "skill_manage");
    }
}

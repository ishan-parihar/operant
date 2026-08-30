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
use crate::skill_usage::SkillUsageTracker;
use crate::tools::{OperantTool, ToolContext, ToolResult};
use crate::write_origin::is_background_review;

// ── Constants ────────────────────────────────────────────────────────────────

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;
const MAX_SKILL_CONTENT_CHARS: usize = 100_000; // ~36k tokens at 2.75 chars/token
const MAX_SKILL_FILE_BYTES: usize = 1_048_576; // 1 MiB per supporting file

/// Meta-skill validation budgets (meta-skill-creator registry.py parity).
/// A router body is read on EVERY traversal through its subtree, so it pays
/// rent constantly — aim well under 200 lines. Leaves get the normal budget.
const MAX_ROUTER_BODY_LINES: usize = 200;
const MAX_LEAF_BODY_LINES: usize = 500;
/// Below this many chars a description cannot carry the routing surface
/// (what it does AND when to use it).
const MIN_DESCRIPTION_CHARS: usize = 12;
/// Lowercased substrings that mark a description as an unwritten placeholder.
const PLACEHOLDER_MARKERS: &[&str] = &[
    "todo",
    "fixme",
    "tbd",
    "placeholder",
    "lorem ipsum",
    "coming soon",
    "to be written",
    "to be added",
    "insert description",
    "fill this in",
    "example description",
];

/// Skills that ship with Operant and must never be modified by the background
/// review agent. Matches hermes-agent's "bundled" + "hub-installed" protection.
///
/// The bare `operant` prefix deliberately covers the entire bundled self-skill
/// family in one entry (`operant`, `operant-skill-authoring`, and any future
/// `operant-*` infra skill) — `operant-agent`/`operant-dev` are subsumed by it.
/// Over-protection is conservative-safe: it only restrains the background
/// reviewer and never blocks direct user-initiated edits.
const PROTECTED_SKILL_PREFIXES: &[&str] = &["operant", "hermes-agent", "hermes-dev", "claw-dev"];

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Characters allowed in skill names (filesystem-safe, URL-friendly).
static VALID_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9][a-z0-9._-]*$").expect("static regex literal is invalid — authoring bug")
});

/// Subdirectories allowed for write_file / remove_file.
const ALLOWED_SUBDIRS: &[&str] = &["references", "templates", "scripts", "assets"];

/// Cheap check: does `dir` contain at least one non-skipped subdirectory?
/// Used to avoid full tree walks for leaf skills (no children possible).
fn dir_has_subdirs(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if !is_skill_dir_skipped(&name) {
            return true;
        }
    }
    false
}

/// Whether a directory should be skipped by the skill-tree walkers (hidden,
/// build, vendored, or resource dirs are never skill nodes).
fn is_skill_dir_skipped(dir_name: &str) -> bool {
    dir_name.starts_with('.')
        || dir_name.starts_with('_')
        || dir_name == "node_modules"
        || dir_name == ".archive"
        || dir_name == ".hub"
        || ALLOWED_SUBDIRS.contains(&dir_name)
}

// ── Background review write guards ────────────────────────────────────────

/// Tracks which skills have been read via `skill_view` during a background
/// review session. The review agent must read a skill before modifying it
/// to prevent uninformed mutations.
static REVIEW_READ_SKILLS: LazyLock<RwLock<HashSet<String>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// Serializes `.usage.json` read-modify-write cycles. The main agent and the
/// `USAGE_TELEMETRY_LOCK` was the in-process lock around the legacy
/// `.usage.json` writer. Plan 007 removed the legacy writer; the
/// curator tracker takes its own `with_exclusive_lock` (advisory OS
/// file lock + atomic save) so this is no longer needed. Kept as
/// dead-code with a `#[expect]` so future agents who grep for the
/// name see the migration note rather than re-introducing the
/// legacy writer.
#[allow(dead_code)]
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
/// 1. Bundled skills (operant*, hermes-agent, etc.) — NEVER edit.
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
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    /// Child skill nodes (meta-skill routing surface). Populated when the
    /// skill directory contains nested skill directories of its own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SkillMeta>,
    /// Path relative to the parent skill root (meta-skill children only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
        if dir_name.starts_with('.') || dir_name == "node_modules" {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if skill_md.exists()
            && let Ok(content) = fs::read_to_string(&skill_md)
        {
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
                .map(|s| s.to_string()); // Meta-skill support: a skill whose directory contains nested
            // skill directories is a router — surface its children so the
            // model sees the routing surface without loading every leaf.
            // Cheap gate: only walk when the dir has subdirectories, so
            // the 84+ leaf skills in a typical pool don't pay for tree
            // discovery on every skills_list call.
            let children = if dir_has_subdirs(&path) {
                collect_skill_children(&path)
            } else {
                Vec::new()
            };
            skills.push(SkillMeta {
                name,
                description,
                category,
                children,
                path: None,
            });
        } else {
            collect_skills_recursive(base_dir, &path, skills);
        }
    }
}

/// One node of a meta-skill tree, as collected for `_map.md` generation.
struct MapNode {
    rel_path: String,
    description: String,
    depth: usize,
    /// Body text (post-frontmatter) — used for line-count and
    /// resource-reference validation.
    body: String,
    /// Resource files (references/templates/scripts/assets) at this node.
    resources: Vec<String>,
}

/// Validation report for a meta-skill tree (CLI `skills audit` gate + the
/// `skill_manage generate_map` walker).
pub struct SkillTreeValidation {
    pub node_count: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Walk a meta-skill subtree (descending THROUGH routers) and run the full
/// registry.py-parity validation: node health (name/dir mismatch, missing or
/// vague description, oversized bodies), child reachability, orphan SKILL.md
/// files under resource dirs, and unreferenced resource files. Shared by
/// `skill_manage generate_map` and the CLI `skills audit` tree gate — one
/// code path, one contract.
fn collect_tree_validation(
    skill_dir: &Path,
    name: &str,
) -> (Vec<MapNode>, Vec<String>, Vec<String>) {
    let mut nodes: Vec<MapNode> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    collect_map_nodes(
        skill_dir,
        skill_dir,
        1,
        &mut nodes,
        &mut errors,
        &mut warnings,
    );
    nodes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // Phase 2 — unreferenced resource files (registry.py): a resource no
    // SKILL.md mentions will never be loaded. The reference corpus is the
    // root router's own body plus every node body in the tree.
    let root_content = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap_or_default();
    let (_, root_body) = parse_frontmatter(&root_content);
    let joined_bodies = nodes
        .iter()
        .map(|n| n.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let all_bodies = format!("{root_body}\n{joined_bodies}");
    let mut root_resources = collect_node_resources(skill_dir);
    root_resources.sort();
    warn_unreferenced_resources(name, &root_resources, &all_bodies, &mut warnings);
    for n in &nodes {
        warn_unreferenced_resources(&n.rel_path, &n.resources, &all_bodies, &mut warnings);
    }
    (nodes, errors, warnings)
}

/// Validate a meta-skill tree without writing `_map.md` — the non-destructive
/// entry point for the CLI `skills audit` gate. Mirrors
/// `skill_manage generate_map --check-only` semantics.
pub fn validate_skill_tree(skill_dir: &Path, name: &str) -> SkillTreeValidation {
    let (nodes, errors, warnings) = collect_tree_validation(skill_dir, name);
    SkillTreeValidation {
        node_count: nodes.len(),
        errors,
        warnings,
    }
}

/// registry.py's missing/vague-description validation: a description must be
/// specific enough to route on (what it does AND when to use it). Returns a
/// reason the description fails, or None when it is specific enough.
fn vague_description_reason(description: &str) -> Option<String> {
    let d = description.trim();
    if d.is_empty() {
        return Some(
            "description is missing — the routing surface cannot work without it. Write what the skill does and when to use it.".to_string(),
        );
    }
    if d.chars().count() < MIN_DESCRIPTION_CHARS {
        return Some(format!(
            "description is only {} chars — too short to route on. Describe what it does AND when to use it.",
            d.chars().count()
        ));
    }
    let lower = d.to_lowercase();
    for marker in PLACEHOLDER_MARKERS {
        if lower.contains(marker) {
            return Some(format!(
                "description reads like an unwritten placeholder (contains '{marker}'). Write a real routing description."
            ));
        }
    }
    None
}

/// Validate one node's frontmatter-level health (registry.py per-node
/// validation): name/dir mismatch (error), missing/vague description
/// (error/warning), oversized body (warning — routers >200 lines, leaves
/// >500). Shared by the root router and every child node.
fn validate_node_health(
    frontmatter: &Value,
    body: &str,
    dir_name: &str,
    rel: &str,
    dir_path: &Path,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    // Name/dir mismatch — routing pointers target directories, so a
    // frontmatter name that disagrees with its directory silently breaks
    // every pointer that uses the path form.
    if let Some(fm_name) = frontmatter.get("name").and_then(|v| v.as_str())
        && !fm_name.is_empty()
        && fm_name != dir_name
    {
        errors.push(format!(
            "node '{rel}': frontmatter name '{fm_name}' does not match its directory '{dir_name}' — routing pointers target directories, so the name must match. Rename the directory or fix the frontmatter."
        ));
    }
    // Missing/vague description.
    if let Some(reason) = vague_description_reason(
        frontmatter
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    ) {
        // Missing descriptions break the routing surface (error); vague ones
        // (too short / placeholder) are review prompts (warning).
        if frontmatter
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            errors.push(format!("node '{rel}': {reason}"));
        } else {
            warnings.push(format!("node '{rel}': {reason}"));
        }
    }
    // Oversized bodies — routers pay rent on every traversal; leaves get the
    // normal skill budget.
    let body_lines = body.lines().count();
    if dir_has_subdirs(dir_path) {
        if body_lines > MAX_ROUTER_BODY_LINES {
            warnings.push(format!(
                "router '{rel}': body is {body_lines} lines (router budget: {MAX_ROUTER_BODY_LINES}) — routers are read on every traversal; move how-to text into leaves."
            ));
        }
    } else if body_lines > MAX_LEAF_BODY_LINES {
        warnings.push(format!(
            "leaf '{rel}': body is {body_lines} lines (leaf budget: {MAX_LEAF_BODY_LINES}) — consider splitting into child nodes."
        ));
    }
}

/// List a node's resource files (references/templates/scripts/assets),
/// relative to the node directory. Recursive so nested reference dirs count.
/// Used by the unreferenced-resource validation (registry.py).
fn collect_node_resources(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Hidden entries and SKILL.md (already handled by the orphan
            // check) are never loadable resources.
            if name.starts_with('.') || name == "SKILL.md" {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    let mut out = Vec::new();
    for sub in ALLOWED_SUBDIRS {
        let subdir = dir.join(sub);
        if subdir.is_dir() {
            walk(&subdir, dir, &mut out);
        }
    }
    out.sort();
    out
}

/// Emit warnings for resource files no SKILL.md in the tree mentions
/// (registry.py: unreferenced resource files — dead weight that will never be
/// loaded because nothing points at it). Matches on the basename OR the
/// node-relative path so both `run scripts/x.py` and `see scripts/x.py`
/// reference styles are recognized.
fn warn_unreferenced_resources(
    rel_node: &str,
    resources: &[String],
    all_bodies: &str,
    warnings: &mut Vec<String>,
) {
    for res in resources {
        let basename = res.rsplit('/').next().unwrap_or(res.as_str());
        if !all_bodies.contains(basename) && !all_bodies.contains(res.as_str()) {
            warnings.push(format!(
                "resource '{}' of node '{}' is not referenced by any SKILL.md in the tree — it will never be loaded. Reference it from a SKILL.md body (e.g. 'see {res}') or remove it.",
                res, rel_node
            ));
        }
    }
}

/// Recursively walk a meta-skill subtree (descending THROUGH routers) and
/// collect every node for `_map.md` + a validation report.
///
/// Mirrors the meta-skill-creator registry.py walk: routers are nodes that
/// contain child nodes; resource dirs (references/scripts/templates/assets)
/// are not skill nodes and are skipped. Validation is split into `errors`
/// (fix every: unreachable children, name/dir mismatches, missing
/// descriptions, orphan SKILL.md under resource dirs) and `warnings` (review
/// prompts: vague descriptions, oversized bodies, unreferenced resources).
fn collect_map_nodes(
    root: &Path,
    dir: &Path,
    depth: usize,
    nodes: &mut Vec<MapNode>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Parent router body, used for the reachability check. Containers without
    // a SKILL.md have no routing table, so no check applies at that level.
    let parent_body = fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();

    // Validate the entry node itself (the root of the walk). Children are
    // validated in the loop below; recursion re-enters child dirs, so the
    // `dir == root` guard prevents double-reporting.
    if dir == root && !parent_body.is_empty() {
        let (frontmatter, body) = parse_frontmatter(&parent_body);
        validate_node_health(
            &frontmatter,
            &body,
            &dir_name,
            &dir_name,
            dir,
            errors,
            warnings,
        );
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let child_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_skill_dir_skipped(child_name) {
            // registry.py: a SKILL.md under a resource directory is an orphan
            // — the tree scanner never descends there, so nothing can ever
            // route to it.
            if ALLOWED_SUBDIRS.contains(&child_name) && path.join("SKILL.md").exists() {
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| child_name.to_string());
                errors.push(format!(
                    "orphan SKILL.md under resource directory '{rel}' — the tree scanner skips resource dirs, so this skill can never be routed to. Move it into a proper child directory."
                ));
            }
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if skill_md.exists()
            && let Ok(content) = fs::read_to_string(&skill_md)
        {
            let (frontmatter, body) = parse_frontmatter(&content);
            let name = frontmatter
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(child_name)
                .to_string();
            let description = frontmatter
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let rel = path
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| child_name.to_string());
            // Reachability: a child that isn't referenced by its router's
            // SKILL.md will never be reached — the whole reason for the map.
            // Test the dir name, the frontmatter name (a router may reference
            // a child by either), and the relative path. An ERROR: this is
            // the failure mode that silently kills a hierarchy.
            if !parent_body.is_empty()
                && !parent_body.contains(child_name)
                && !parent_body.contains(&name)
                && !parent_body.contains(&rel)
            {
                errors.push(format!(
                    "child '{}' is not referenced in parent router '{}' SKILL.md — it will never be routed to. Add a routing line.",
                    rel, dir_name
                ));
            }
            validate_node_health(
                &frontmatter,
                &body,
                child_name,
                &rel,
                &path,
                errors,
                warnings,
            );
            nodes.push(MapNode {
                rel_path: rel,
                description,
                depth,
                body,
                resources: collect_node_resources(&path),
            });
        }
        // Descend through routers AND containers — leaves live below.
        collect_map_nodes(root, &path, depth + 1, nodes, errors, warnings);
    }
}

/// Recursively collect the child skill nodes of a meta-skill router.
///
/// A child node is any directory (at any depth below `skill_dir`, excluding
/// resource and hidden directories) that contains its own `SKILL.md`. Returns
/// paths relative to `skill_dir` so the model can route with
/// `skill_view(name='<parent>/<child>')`.
pub fn collect_skill_children(skill_dir: &Path) -> Vec<SkillMeta> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<SkillMeta>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_skill_dir_skipped(dir_name) {
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
                    let rel = path
                        .strip_prefix(base)
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| dir_name.to_string());
                    let mut children = Vec::new();
                    walk(&path, base, &mut children);
                    children.sort_by(|a, b| a.name.cmp(&b.name));
                    // category stays None for meta-skill nodes — the `path`
                    // field (relative to the router root) encodes position.
                    out.push(SkillMeta {
                        name,
                        description,
                        category: None,
                        children,
                        path: Some(rel),
                    });
                }
            } else {
                // Sub-container (router without SKILL.md yet, or grouped dirs).
                walk(&path, base, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(skill_dir, skill_dir, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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

/// Recursively collect every file inside a skill directory (hermes parity:
/// `rglob("*")` in `skills_tool.py`). Skips hidden entries, `node_modules`,
/// and the `.archive` / `.hub` markers. `SKILL.md` itself is excluded by the
/// caller. Returns paths relative to `skill_dir` so the model sees stable
/// `references/x.md` / `scripts/y.sh` references.
fn collect_skill_files(skill_dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, out);
            } else if name != "SKILL.md"
                && let Ok(rel) = path.strip_prefix(base)
            {
                out.push(rel.to_string_lossy().to_string());
            }
        }
    }
    let mut files = Vec::new();
    walk(skill_dir, skill_dir, &mut files);
    files.sort();
    files
}

/// Parse a frontmatter tag value that may be a YAML list (`[a, b]`) or a
/// comma-separated string (`a, b`). Mirrors hermes `_parse_tags`.
fn parse_tags(value: &Value) -> Vec<String> {
    match value {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|t| t.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Resolve tags the way hermes does: `metadata.<ns>.tags` (hermes ns first,
/// then operant ns for ported skills) OR top-level `tags`. Falls back to
/// `related_skills` from the same sources.
fn frontmatter_tags(frontmatter: &Value) -> Vec<String> {
    let meta = frontmatter.get("metadata");
    for ns in ["hermes", "operant"] {
        if let Some(tags) = meta
            .and_then(|m| m.get(ns))
            .and_then(|m| m.get("tags"))
            .filter(|v| !v.is_null())
            && !parse_tags(tags).is_empty()
        {
            return parse_tags(tags);
        }
    }
    if let Some(tags) = frontmatter.get("tags") {
        return parse_tags(tags);
    }
    Vec::new()
}

/// Resolve `related_skills` from metadata.<ns>.related_skills OR top-level.
fn frontmatter_related_skills(frontmatter: &Value) -> Vec<String> {
    let meta = frontmatter.get("metadata");
    for ns in ["hermes", "operant"] {
        if let Some(rel) = meta
            .and_then(|m| m.get(ns))
            .and_then(|m| m.get("related_skills"))
            .filter(|v| !v.is_null())
            && !parse_tags(rel).is_empty()
        {
            return parse_tags(rel);
        }
    }
    if let Some(rel) = frontmatter.get("related_skills") {
        return parse_tags(rel);
    }
    Vec::new()
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
    /// When true, return the full nested tree (routers with their children)
    /// instead of the flat catalog. Meta-skill discovery surface.
    tree: Option<bool>,
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

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let skills_dir = &self.root_dir;
        let tree_mode = args.get("tree").and_then(|v| v.as_bool()).unwrap_or(false);
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
        if tree_mode {
            // Meta-skill discovery surface: nested tree of routers/leaves.
            let roots = collect_skill_children(skills_dir);
            return ToolResult::success(
                "skills_list",
                json!({
                    "tree": roots,
                    "count": roots.len(),
                    "hint": "Use skill_view(name='<parent>/<child>') to read any node. Use skill_manage(action='generate_map', name='<router>') to build _map.md."
                }),
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
                "hint": "Use skill_view to see full content. Use skills_list(tree=true) for the meta-skill tree."
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
                // Apply SKILL.md preprocessing (template vars + inline shell)
                // before the content reaches the model — hermes parity
                // (`skill_preprocessing.preprocess_skill_content`). The inline
                // shell runs with the skill directory as CWD. Unresolved
                // template vars are left in place for the author to debug.
                let body = crate::agent::skill_preprocessing::preprocess_skill_content(
                    &body,
                    Some(&skill_dir),
                    None,
                    None,
                );
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
                let tags = frontmatter_tags(&frontmatter);
                let related_skills = frontmatter_related_skills(&frontmatter);
                // Recursive support-file listing, categorized like hermes's
                // `linked_files` (references/templates/assets/scripts).
                let all_files = collect_skill_files(&skill_dir);
                let supporting_files: Vec<String> = all_files.clone();
                let mut linked_files = serde_json::Map::new();
                for category in ["references", "templates", "assets", "scripts"] {
                    // Match on the FIRST path component so a file directly under
                    // the category dir counts, but a sibling like
                    // `scripts-tools/x` never does.
                    let in_category: Vec<String> = all_files
                        .iter()
                        .filter(|f| f.split('/').next().is_some_and(|first| first == category))
                        .cloned()
                        .collect();
                    if !in_category.is_empty() {
                        linked_files.insert(category.to_string(), json!(in_category));
                    }
                }
                ToolResult::success(
                    "skill_view",
                    json!({
                        "name": skill_name,
                        "description": description,
                        "content": body,
                        "tags": tags,
                        "related_skills": related_skills,
                        "supporting_files": supporting_files,
                        "linked_files": Value::Object(linked_files),
                        "children": collect_skill_children(&skill_dir),
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
    /// Action: "create", "edit", "patch", "delete", "write_file",
    /// "remove_file", "generate_map"
    action: String,
    /// Skill name (directory name)
    name: String,
    /// Full SKILL.md content (required for create/edit)
    content: Option<String>,
    /// Category for organize (create only)
    category: Option<String>,
    /// Supporting file path (write_file/remove_file/patch).
    ///
    /// Accepted as `file_path` (snake_case, hermes convention) or `filePath`
    /// (camelCase, operant schema convention).
    #[serde(alias = "file_path")]
    file_path: Option<String>,
    /// File content (write_file).
    #[serde(alias = "file_content")]
    file_content: Option<String>,
    /// Text to find (patch).
    #[serde(alias = "old_string")]
    old_string: Option<String>,
    /// Replacement text (patch).
    #[serde(alias = "new_string")]
    new_string: Option<String>,
    /// Replace all occurrences (patch).
    #[serde(alias = "replace_all")]
    replace_all: Option<bool>,
    /// Delete a meta-skill router and all of its child skills (delete only).
    recursive: Option<bool>,
    /// generate_map only: validate without writing _map.md (registry.py
    /// `--check` parity). Returns the errors/warnings report only.
    check_only: Option<bool>,
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
        // Meta-skill support: names may be slash-separated nested paths
        // (e.g. `trading-systems/strategy-research/backtesting`). Each segment
        // must be a valid skill name; the path must be relative and clean.
        if name.starts_with('/') || name.ends_with('/') {
            return Some("Skill path must not start or end with '/'".into());
        }
        if name.contains("//") {
            return Some("Skill path must not contain '//'".into());
        }
        if name.split('/').any(|seg| seg == ".." || seg == ".") {
            return Some("Skill path must not contain '.' or '..' segments".into());
        }
        for segment in name.split('/') {
            if segment.is_empty() {
                return Some("Skill path contains an empty segment".into());
            }
            if segment.len() > MAX_NAME_LENGTH {
                return Some(format!(
                    "Skill path segment '{}' exceeds {} characters.",
                    segment, MAX_NAME_LENGTH
                ));
            }
            if !VALID_NAME_RE.is_match(segment) {
                return Some(format!(
                    "Invalid skill path segment '{}'. Use lowercase letters, numbers, hyphens, dots, and underscores. Must start with a letter or digit.",
                    segment
                ));
            }
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
            "generate_map" => self.action_generate_map(&parsed).await,
            other => ToolResult::error(
                "skill_manage",
                format!(
                    "Unknown action '{}'. Use: create, edit, patch, delete, write_file, remove_file, generate_map",
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
        // Nested paths (meta-skill children) and category are mutually
        // exclusive: a nested path already encodes its position in the tree.
        if parsed.name.contains('/') && parsed.category.is_some() {
            return ToolResult::error(
                "skill_manage",
                "Use either a category or a nested path (meta-skill child), not both.",
            );
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
        // Meta-skill safety: never delete a router that has children unless
        // recursive is set — mirrors hermes curator's never-auto-delete
        // invariant and prevents accidental tree destruction.
        let children = collect_skill_children(&skill_dir);
        if !children.is_empty() && !parsed.recursive.unwrap_or(false) {
            let names: Vec<&str> = children.iter().map(|c| c.name.as_str()).collect();
            return ToolResult::error(
                "skill_manage",
                format!(
                    "Skill '{}' is a meta-skill router with {} child skill(s): {:?}. Pass recursive=true to delete the whole tree.",
                    parsed.name,
                    children.len(),
                    names
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

    // ── generate_map (meta-skill registry, registry.py parity) ─────
    async fn action_generate_map(&self, parsed: &SkillManageArgs) -> ToolResult {
        let Some(skill_dir) = find_skill(&self.root_dir, &parsed.name) else {
            return ToolResult::error("skill_manage", format!("Skill '{}' not found", parsed.name));
        };
        let check_only = parsed.check_only.unwrap_or(false);
        // Walk the subtree and build the map + validation report (registry.py
        // parity, shared with the CLI `skills audit` tree gate). Unlike the
        // flat scanner (which stops at the first SKILL.md), this walker
        // descends THROUGH routers so nested leaves are indexed too.
        let (nodes, errors, warnings) = collect_tree_validation(&skill_dir, &parsed.name);

        if nodes.is_empty() {
            // A leaf has no children — writing an empty map would just be
            // noise, but its own health was still validated above. The schema
            // stays stable across the leaf/router cases (errors/warnings
            // always present, map_path null when nothing was written).
            let base = format!(
                "'{}' has no child skills — it is a leaf, not a router. No _map.md written.",
                parsed.name
            );
            let message = if errors.is_empty() && warnings.is_empty() {
                base
            } else {
                format!(
                    "{base} Validation: {} error(s), {} warning(s).",
                    errors.len(),
                    warnings.len()
                )
            };
            return ToolResult::success(
                "skill_manage",
                json!({
                    "action": "generate_map",
                    "name": parsed.name,
                    "check_only": check_only,
                    "map_path": Value::Null,
                    "node_count": 0,
                    "map": Value::Null,
                    "errors": errors,
                    "warnings": warnings,
                    "message": message,
                }),
            );
        }
        if check_only {
            // registry.py `--check`: validate only, write nothing.
            return ToolResult::success(
                "skill_manage",
                json!({
                    "action": "generate_map",
                    "name": parsed.name,
                    "check_only": true,
                    "map_path": Value::Null,
                    "node_count": nodes.len(),
                    "map": Value::Null,
                    "errors": errors,
                    "warnings": warnings,
                    "message": format!(
                        "Validation only (no _map.md written): {} error(s), {} warning(s) across {} node(s). Fix every error; treat warnings as review prompts.",
                        errors.len(),
                        warnings.len(),
                        nodes.len()
                    ),
                }),
            );
        }
        let mut map = String::new();
        map.push_str(&format!("# {} — meta-skill tree map\n\n", parsed.name));
        map.push_str("Indented index: each line is a node (path — description).\n");
        map.push_str("Jump: read this map and go straight to the leaf you need.\n");
        map.push_str("Walk: descend router by router when you don't know the leaf.\n\n");
        for n in &nodes {
            let indent = "  ".repeat(n.depth);
            let first = n.description.split(['.', '\n']).next().unwrap_or("").trim();
            map.push_str(&format!("{}- {} — {}\n", indent, n.rel_path, first));
        }

        let error_count = errors.len();
        let warning_count = warnings.len();
        let map_path = skill_dir.join("_map.md");
        if let Err(e) = fs::write(&map_path, &map) {
            return ToolResult::error("skill_manage", format!("Failed to write _map.md: {}", e));
        }
        self.record_usage(&parsed.name, "generate_map");
        ToolResult::success(
            "skill_manage",
            json!({
                "action": "generate_map",
                "name": parsed.name,
                "map_path": map_path.to_string_lossy(),
                "node_count": nodes.len(),
                "map": map,
                "errors": errors,
                "warnings": warnings,
                "message": format!(
                    "Wrote _map.md with {} node(s). Validation: {} error(s), {} warning(s). Read it with skill_view(name='{}', file_path='_map.md') to route the tree.",
                    nodes.len(),
                    error_count,
                    warning_count,
                    parsed.name
                ),
            }),
        )
    }

    // ── Pinned check ────────────────────────────────────────────────
    fn is_pinned(&self, name: &str) -> bool {
        // Plan 007: read pinned from the curator tracker (the
        // single source of truth for skill telemetry). The legacy
        // `.usage.json` reader was removed — see `record_usage` below
        // for the consolidation rationale.
        let tracker = SkillUsageTracker::new(self.root_dir.join(".curator").join("usage.json"));
        tracker.is_pinned(name)
    }

    // ── Telemetry ────────────────────────────────────────────────────
    fn record_usage(&self, name: &str, action: &str) {
        // Plan 007: the curator tracker (`.curator/usage.json`) is the
        // single source of truth for skill telemetry. R21's bridge call
        // stays; the legacy `.usage.json` write is removed.
        //
        // Locking: `with_exclusive_lock` re-loads fresh state from
        // disk inside the OS lock, so concurrent `skill_manage` actions
        // (main agent + background-review daemon, or a separate
        // `operant curator` process) never lose records, and a corrupt
        // sidecar self-heals on the next save. (R20/R21 contract.)
        let tracker = SkillUsageTracker::new(self.root_dir.join(".curator").join("usage.json"));
        let is_bg = is_background_review();
        let _ = tracker.with_exclusive_lock(|t| {
            match action {
                "create" => t.record_created(name, is_bg),
                "delete" => t.remove(name),
                "patch" | "edit" | "write_file" | "remove_file" => t.bump_patch(name),
                _ => {}
            }
            Ok(())
        });
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
    fn concurrent_record_usage_preserves_curator_telemetry() {
        // The main agent and the background-review daemon both call
        // `skill_manage` concurrently; a non-atomic telemetry write would
        // corrupt the file. The curator tracker is now the single
        // source of truth (plan 007) and the lock + atomic save in
        // `SkillUsageTracker` must keep patch counts intact under
        // concurrent writers. (R20 — same contract, new home.)
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

        // The legacy `.usage.json` file is no longer written.
        let legacy_path = skills_dir.join(".usage.json");
        assert!(
            !legacy_path.exists(),
            "plan 007: legacy telemetry file must not be created (got {legacy_path:?})"
        );

        // The curator store carries every patch.
        let curator_path = skills_dir.join(".curator").join("usage.json");
        let content = std::fs::read_to_string(&curator_path)
            .expect("curator usage.json must exist after concurrent writers");
        let records: Vec<crate::skill_usage::UsageRecord> =
            serde_json::from_str(&content).expect("self-healed to valid JSON");
        // 4 skills × 8 threads × 25 patches = 200 total patch_count.
        let total: u64 = records.iter().map(|r| r.patch_count).sum();
        assert_eq!(total, 200, "no patch may be lost to write races");
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
    fn test_validate_name_accepts_nested_meta_skill_paths() {
        // Meta-skill: slash-separated nested paths are valid node addresses.
        assert!(SkillManageTool::validate_name("trading-systems").is_none());
        assert!(SkillManageTool::validate_name("trading-systems/strategy-research").is_none());
        assert!(
            SkillManageTool::validate_name("trading-systems/strategy-research/backtesting")
                .is_none()
        );
        // Traversal and malformed paths are rejected.
        assert!(SkillManageTool::validate_name("../escape").is_some());
        assert!(SkillManageTool::validate_name("trading-systems/../escape").is_some());
        assert!(SkillManageTool::validate_name("/leading").is_some());
        assert!(SkillManageTool::validate_name("trailing/").is_some());
        assert!(SkillManageTool::validate_name("a//b").is_some());
        assert!(SkillManageTool::validate_name("a/.").is_some());
        // Each segment is validated individually.
        assert!(SkillManageTool::validate_name("ok/Bad-Segment").is_some());
    }

    #[test]
    fn test_collect_skill_children_finds_nested_tree() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let root = skills_dir.join("trading-systems");
        fs::create_dir_all(root.join("strategy-research/backtesting")).unwrap();
        fs::create_dir_all(root.join("execution")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: trading-systems\ndescription: Trading domain router\n---\n\nRoutes to strategy-research and execution.\n",
        )
        .unwrap();
        fs::write(
            root.join("strategy-research/SKILL.md"),
            "---\nname: strategy-research\ndescription: Researches strategies\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            root.join("strategy-research/backtesting/SKILL.md"),
            "---\nname: backtesting\ndescription: Backtests strategies\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            root.join("execution/SKILL.md"),
            "---\nname: execution\ndescription: Executes trades\n---\n\nBody.\n",
        )
        .unwrap();

        let children = collect_skill_children(&root);
        assert_eq!(children.len(), 2, "router lists its children");
        let sr = children
            .iter()
            .find(|c| c.name == "strategy-research")
            .unwrap();
        assert_eq!(sr.children.len(), 1, "router child surfaces its own leaf");
        assert_eq!(sr.children[0].name, "backtesting");
        // Paths are relative to the router root so the model can route.
        assert_eq!(sr.path.as_deref(), Some("strategy-research"));
        assert_eq!(
            sr.children[0].path.as_deref(),
            Some("strategy-research/backtesting")
        );
    }

    #[test]
    fn test_collect_map_nodes_walks_through_routers() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("trading-systems");
        fs::create_dir_all(root.join("strategy-research/backtesting")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: trading-systems\ndescription: Trading domain router covering research and execution\n---\n\nRoutes to strategy-research and execution.\n",
        )
        .unwrap();
        fs::write(
            root.join("strategy-research/SKILL.md"),
            "---\nname: strategy-research\ndescription: Researches and validates trading strategies before execution\n---\n\nRoutes to backtesting. See references/guide.md.\n",
        )
        .unwrap();
        fs::write(
            root.join("strategy-research/backtesting/SKILL.md"),
            "---\nname: backtesting\ndescription: Backtests strategies against historical market data\n---\n\nBody.\n",
        )
        .unwrap();
        // A resource dir is NOT a skill node — and a referenced resource
        // must not trip the unreferenced-resource warning.
        fs::create_dir_all(root.join("strategy-research/references")).unwrap();
        fs::write(
            root.join("strategy-research/references/guide.md"),
            "# Guide",
        )
        .unwrap();

        let mut nodes = Vec::new();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        collect_map_nodes(&root, &root, 1, &mut nodes, &mut errors, &mut warnings);
        let paths: Vec<&str> = nodes.iter().map(|n| n.rel_path.as_str()).collect();
        assert!(paths.contains(&"strategy-research"), "got: {paths:?}");
        assert!(
            paths.contains(&"strategy-research/backtesting"),
            "leaf below a router is indexed: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("references")),
            "resource dirs are not skill nodes: {paths:?}"
        );
        // A healthy, fully-referenced tree reports no issues.
        assert!(errors.is_empty(), "got: {errors:?}");
        assert!(
            !warnings.iter().any(|w| w.contains("guide.md")),
            "referenced resource must not be flagged: {warnings:?}"
        );
    }

    #[test]
    fn test_vague_description_heuristics() {
        // Missing, too-short, and placeholder descriptions are flagged.
        assert!(vague_description_reason("").is_some());
        assert!(vague_description_reason("Do stuff").is_some()); // 8 chars
        assert!(vague_description_reason("TODO: write this later").is_some());
        assert!(vague_description_reason("A placeholder description").is_some());
        // Specific, concrete descriptions pass.
        assert!(
            vague_description_reason(
                "Drive the user's desktop in the background — clicking, typing, and navigating."
            )
            .is_none()
        );
        assert!(
            vague_description_reason(
                "Authorized web application penetration testing — reconnaissance and exploitation."
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn test_generate_map_flags_registry_validation_issues() {
        // Exercises the FULL pipeline (walk + resource phase + response shape)
        // against a deliberately broken tree — one violation per category.
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        let root = skills_dir.join("ops-domain");
        fs::create_dir_all(root.join("deploy-ops/scripts")).unwrap();
        fs::create_dir_all(root.join("deploy-ops/references")).unwrap();
        fs::create_dir_all(root.join("monitoring-ops")).unwrap();
        // Root: name/dir mismatch + vague description + oversized router body
        // (250 lines, over the 200-line router budget).
        let mut root_body = String::from("Routes to deploy-ops.\n");
        for _ in 0..249 {
            root_body.push_str("context line for the router\n");
        }
        fs::write(
            root.join("SKILL.md"),
            format!("---\nname: domain-ops\ndescription: router\n---\n\n{root_body}"),
        )
        .unwrap();
        // Child: referenced by the root, but its deploy.sh is never mentioned
        // anywhere, and an orphan SKILL.md hides under references/.
        fs::write(
            root.join("deploy-ops/SKILL.md"),
            "---\nname: deploy-ops\ndescription: Deploys services to production clusters safely\n---\n\nRuns the deployment playbook. See references/runbook.md.\n",
        )
        .unwrap();
        fs::write(
            root.join("deploy-ops/scripts/deploy.sh"),
            "#!/bin/sh\necho hi\n",
        )
        .unwrap();
        fs::write(root.join("deploy-ops/references/runbook.md"), "# Runbook\n").unwrap();
        fs::write(root.join("deploy-ops/references/SKILL.md"), "orphan\n").unwrap();
        // Child: never referenced by the root + placeholder description.
        fs::write(
            root.join("monitoring-ops/SKILL.md"),
            "---\nname: monitoring-ops\ndescription: A placeholder description for now\n---\n\nBody.\n",
        )
        .unwrap();

        let tool = SkillManageTool::new(skills_dir);
        let args = SkillManageArgs {
            action: "generate_map".into(),
            name: "ops-domain".into(),
            content: None,
            category: None,
            file_path: None,
            file_content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            recursive: None,
            check_only: None,
        };
        let result = tool.action_generate_map(&args).await;
        let parsed: serde_json::Value = result.parse_content().expect("generate_map response");
        let errors: Vec<String> = parsed["errors"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let warnings: Vec<String> = parsed["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        let err_text = errors.join("\n");
        let warn_text = warnings.join("\n");

        // Errors: fix every.
        assert!(
            err_text.contains("does not match its directory"),
            "{err_text}"
        );
        assert!(
            err_text.contains("not referenced in parent router"),
            "{err_text}"
        );
        assert!(
            err_text.contains("orphan SKILL.md under resource directory"),
            "{err_text}"
        );
        // Warnings: review prompts.
        assert!(
            warn_text.contains("router 'ops-domain': body is 250 lines"),
            "{warn_text}"
        );
        assert!(warn_text.contains("description is only"), "{warn_text}");
        assert!(warn_text.contains("unwritten placeholder"), "{warn_text}");
        assert!(
            warn_text.contains("deploy.sh") && warn_text.contains("not referenced"),
            "{warn_text}"
        );
        // Referenced resources stay quiet.
        assert!(!warn_text.contains("runbook.md"), "{warn_text}");
    }

    #[tokio::test]
    async fn test_generate_map_check_only_skips_write() {
        // registry.py `--check` parity: validate without writing _map.md.
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        let root = skills_dir.join("ops-domain");
        fs::create_dir_all(root.join("deploy-ops")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            "---\nname: ops-domain\ndescription: Ops domain router for deployment and monitoring\n---\n\nRoutes to deploy-ops.\n",
        )
        .unwrap();
        fs::write(
            root.join("deploy-ops/SKILL.md"),
            "---\nname: deploy-ops\ndescription: Deploys services to production clusters safely\n---\n\nBody.\n",
        )
        .unwrap();

        let tool = SkillManageTool::new(skills_dir.clone());
        let args = SkillManageArgs {
            action: "generate_map".into(),
            name: "ops-domain".into(),
            content: None,
            category: None,
            file_path: None,
            file_content: None,
            old_string: None,
            new_string: None,
            replace_all: None,
            recursive: None,
            check_only: Some(true),
        };
        let result = tool.action_generate_map(&args).await;
        let parsed: serde_json::Value = result.parse_content().expect("generate_map response");
        assert_eq!(parsed["check_only"], serde_json::Value::Bool(true));
        assert_eq!(parsed["node_count"], serde_json::Value::from(1));
        assert!(!root.join("_map.md").exists(), "check_only must not write");

        // Same call without check_only writes the map.
        let args = SkillManageArgs {
            check_only: None,
            ..args
        };
        let result = tool.action_generate_map(&args).await;
        let parsed: serde_json::Value = result.parse_content().expect("generate_map response");
        assert!(root.join("_map.md").exists(), "normal run writes _map.md");
        assert!(parsed["map"].as_str().unwrap_or("").contains("deploy-ops"));
        assert!(parsed["errors"].as_array().unwrap().is_empty());
    }
    #[test]
    fn test_validate_name_invalid_chars() {
        assert!(SkillManageTool::validate_name("My Skill!").is_some());
    }

    #[test]
    fn test_validate_skill_tree_flags_unreachable_child_in_categorized_tree() {
        // CLI `skills audit` tree-gate parity: validate_skill_tree is the
        // non-destructive walker that flags a child never referenced by its
        // router, even when the router sits under category dirs.
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let router = skills_dir.join("creative/website-design/components");
        fs::create_dir_all(router.join("heroes")).unwrap();
        fs::create_dir_all(router.join("orphan-leaf")).unwrap();
        fs::write(
            router.join("SKILL.md"),
            "---\nname: components\ndescription: Website component router\n---\n\nRoutes to heroes.\n",
        )
        .unwrap();
        fs::write(
            router.join("heroes/SKILL.md"),
            "---\nname: heroes\ndescription: Hero sections\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            router.join("orphan-leaf/SKILL.md"),
            "---\nname: orphan-leaf\ndescription: Never referenced by the router\n---\n\nBody.\n",
        )
        .unwrap();

        let report = validate_skill_tree(&router, "components");
        assert_eq!(report.node_count, 2);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("orphan-leaf") && e.contains("not referenced")),
            "expected unreachable-child error, got: {:?}",
            report.errors
        );
        // The referenced child stays quiet.
        assert!(!report.errors.iter().any(|e| e.contains("heroes")));
    }

    #[test]
    fn test_validate_skill_tree_leaf_has_no_children() {
        // A leaf (no child skill dirs) validates with zero nodes — the
        // CLI gate skips it before calling, but the API must stay stable.
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        let leaf = skills_dir.join("arxiv");
        fs::create_dir_all(&leaf).unwrap();
        fs::write(
            leaf.join("SKILL.md"),
            "---\nname: arxiv\ndescription: Search arXiv papers\n---\n\nBody.\n",
        )
        .unwrap();
        let report = validate_skill_tree(&leaf, "arxiv");
        assert_eq!(report.node_count, 0);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn test_self_skill_is_protected_from_background_review() {
        // The bundled self-skill is named `operant` (not `operant-agent`);
        // the infrastructure skills that ship with it must never be edited
        // by the background review agent (hermes parity for bundled skills).
        assert!(is_protected_skill("operant"));
        assert!(is_protected_skill("operant-skill-authoring"));
        assert!(is_protected_skill("operant-agent"));
        assert!(is_protected_skill("hermes-agent"));
        assert!(is_protected_skill("claw-dev"));
        // Unrelated user skills stay editable.
        assert!(!is_protected_skill("arxiv"));
        assert!(!is_protected_skill("trading-systems"));
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
    fn test_frontmatter_tags_prefers_metadata_ns_then_top_level() {
        // metadata.operant.tags wins over top-level tags (ported skills).
        let fm: Value = serde_json::from_str(
            "{\"name\": \"arxiv\", \"description\": \"d\", \"tags\": [\"top\"], \n\
             \"metadata\": {\"operant\": {\"tags\": [\"Research\", \"Arxiv\"]}}}",
        )
        .unwrap();
        assert_eq!(frontmatter_tags(&fm), vec!["Research", "Arxiv"]);

        // hermes ns also honored (hermes-authored skills).
        let fm2: Value =
            serde_json::from_str("{\"metadata\": {\"hermes\": {\"tags\": [\"llm\"]}}}").unwrap();
        assert_eq!(frontmatter_tags(&fm2), vec!["llm"]);

        // Fallback to top-level only.
        let fm3: Value = serde_json::from_str("{\"tags\": \"a, b\"}").unwrap();
        assert_eq!(frontmatter_tags(&fm3), vec!["a", "b"]);

        // Nothing at all.
        let fm4: Value = serde_json::from_str("{}").unwrap();
        assert!(frontmatter_tags(&fm4).is_empty());
    }

    #[test]
    fn test_collect_skill_files_recursive_and_categorized() {
        let dir = tempfile::TempDir::new().unwrap();
        let skill = dir.path().join("demo");
        fs::create_dir_all(skill.join("scripts")).unwrap();
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: demo\n---\nbody").unwrap();
        fs::write(skill.join("scripts/search.sh"), "#!/bin/sh").unwrap();
        fs::write(skill.join("references/api.md"), "# API").unwrap();
        fs::write(skill.join("notes.txt"), "root file").unwrap();
        fs::create_dir(skill.join(".hidden")).unwrap();
        fs::write(skill.join(".hidden/secret.md"), "hidden").unwrap();
        // A sibling dir whose name merely PREFIXES a category must not count.
        fs::create_dir_all(skill.join("scripts-tools")).unwrap();
        fs::write(skill.join("scripts-tools/x.sh"), "#!\n").unwrap();

        let files = collect_skill_files(&skill);
        // SKILL.md and hidden entries excluded; nested + root files included.
        assert!(!files.iter().any(|f| f == "SKILL.md"));
        assert!(!files.iter().any(|f| f.starts_with(".hidden")));
        assert!(files.contains(&"scripts/search.sh".to_string()));
        assert!(files.contains(&"references/api.md".to_string()));
        assert!(files.contains(&"notes.txt".to_string()));
        assert!(files.is_sorted());

        // Category filter used by linked_files: first path component must
        // match exactly (a `scripts-tools/x` sibling must not count).
        let scripts: Vec<String> = files
            .iter()
            .filter(|f| f.split('/').next().is_some_and(|first| first == "scripts"))
            .cloned()
            .collect();
        assert_eq!(scripts, vec!["scripts/search.sh"]);
        let no_false_positive: Vec<String> = files
            .iter()
            .filter(|f| f.split('/').next().is_some_and(|f| f == "assets"))
            .cloned()
            .collect();
        assert!(no_false_positive.is_empty(), "no assets dir in fixture");
        // `scripts-tools/x.sh` must NOT be categorized as `scripts`.
        assert_eq!(scripts, vec!["scripts/search.sh"]);
    }

    #[test]
    fn test_skill_manage_args_accept_both_naming_conventions() {
        // hermes parity: models trained on hermes call snake_case keys
        // (old_string/new_string), while operant's schema advertises
        // camelCase (oldString/newString). Both must deserialize.
        let camel: SkillManageArgs = serde_json::from_value(json!({
            "action": "patch",
            "name": "demo",
            "oldString": "a",
            "newString": "b",
            "filePath": "references/x.md",
            "fileContent": "hi",
            "replaceAll": true
        }))
        .unwrap();
        assert_eq!(camel.old_string.as_deref(), Some("a"));
        assert_eq!(camel.new_string.as_deref(), Some("b"));
        assert_eq!(camel.file_path.as_deref(), Some("references/x.md"));
        assert_eq!(camel.replace_all, Some(true));

        let snake: SkillManageArgs = serde_json::from_value(json!({
            "action": "patch",
            "name": "demo",
            "old_string": "a",
            "new_string": "b",
            "file_path": "references/x.md",
            "file_content": "hi",
            "replace_all": false
        }))
        .unwrap();
        assert_eq!(snake.old_string.as_deref(), Some("a"));
        assert_eq!(snake.new_string.as_deref(), Some("b"));
        assert_eq!(snake.file_path.as_deref(), Some("references/x.md"));
        assert_eq!(snake.replace_all, Some(false));
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

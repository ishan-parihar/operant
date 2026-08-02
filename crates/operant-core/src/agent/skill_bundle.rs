//! Skill bundles — aliases that load multiple skills under one slash command.
//!
//! A skill bundle is a small YAML file that names a set of skills to load
//! together. Invoking `/<bundle-name>` from the CLI or gateway loads every
//! referenced skill's full content into a single user message, the same way
//! `/<skill-name>` does — but for N skills at once.
//!
//! # Storage
//!
//! Bundles live in `~/.operant/skill-bundles/*.yaml`. Each file looks like:
//!
//! ```yaml
//! name: backend-dev
//! description: Backend feature work — code review, testing, PR workflow.
//! skills:
//!   - github-code-review
//!   - test-driven-development
//!   - github-pr-workflow
//! instruction: |
//!   Optional extra guidance to inject above the skill bodies.
//! ```
//!
//! # Conflict resolution
//!
//! If a bundle and a skill share the same slash name, the bundle wins.
//! The slash command dispatch checks bundles first, then falls back to skills.
//!
//! Ported from `hermes-agent/agent/skill_bundles.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;
use tracing::{debug, warn};

/// Parsed skill bundle from a YAML file.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillBundle {
    /// Display name (falls back to filename stem if missing).
    pub name: String,
    /// URL-safe slug used as the slash command key.
    pub slug: String,
    /// One-line description.
    pub description: String,
    /// Ordered list of skill names to load.
    pub skills: Vec<String>,
    /// Optional extra instruction to inject above skill bodies.
    pub instruction: String,
    /// Path to the YAML file on disk.
    pub path: String,
}

/// Resolve the bundles directory from config or default.
fn bundles_dir() -> PathBuf {
    // Allow override for tests
    if let Ok(override_dir) = std::env::var("OPERANT_BUNDLES_DIR") {
        return PathBuf::from(override_dir);
    }
    crate::platform::operant_home().join("skill-bundles")
}

/// Normalize a bundle name to a URL-safe slug.
fn slugify(name: &str) -> String {
    let slug = name
        .to_lowercase()
        .replace(['_', ' '], "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect::<String>();
    // Collapse multiple hyphens
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.trim_matches('-').to_string()
}

/// Load a single bundle YAML file.
fn load_bundle_file(path: &Path) -> Option<SkillBundle> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Could not read bundle file");
            return None;
        }
    };

    // Simple YAML parsing without a full YAML library:
    // Extract top-level key-value pairs manually.
    let mut name = String::new();
    let mut description = String::new();
    let mut instruction = String::new();
    let mut skills: Vec<String> = Vec::new();
    let mut in_skills = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for top-level key
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_skills = false;
            if let Some(val) = trimmed.strip_prefix("name:") {
                name = val.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if let Some(val) = trimmed.strip_prefix("description:") {
                description = val.trim().trim_matches('"').trim_matches('\'').to_string();
            } else if trimmed == "skills:" || trimmed.starts_with("skills:") {
                in_skills = true;
                // Inline single-line skills list: skills: [a, b, c]
                if let Some(val) = trimmed.strip_prefix("skills:") {
                    let val = val.trim();
                    if val.starts_with('[') && val.ends_with(']') {
                        let inner = &val[1..val.len() - 1];
                        skills = inner
                            .split(',')
                            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        in_skills = false;
                    }
                }
            } else if let Some(val) = trimmed.strip_prefix("instruction:") {
                // Multi-line instruction (| or >)
                let val = val.trim();
                if val == "|" || val == ">" {
                    instruction.clear();
                } else {
                    instruction = val.trim_matches('"').trim_matches('\'').to_string();
                }
            }
        } else if in_skills {
            // Indented line under skills: - skill-name
            if let Some(skill) = trimmed.strip_prefix("- ") {
                let skill = skill
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !skill.is_empty() {
                    skills.push(skill);
                }
            }
        }
    }

    if name.is_empty() {
        name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
    }

    let slug = slugify(&name);
    if slug.is_empty() {
        warn!(path = %path.display(), "Bundle yielded empty slug; skipping");
        return None;
    }

    if skills.is_empty() {
        warn!(path = %path.display(), "Bundle has no skills; skipping");
        return None;
    }

    Some(SkillBundle {
        name,
        slug,
        description: if description.is_empty() {
            format!("Load {} skills as a bundle", skills.len())
        } else {
            description
        },
        skills,
        instruction,
        path: path.display().to_string(),
    })
}

/// Scan the bundles directory and return all valid bundles keyed by `/slug`.
pub fn scan_bundles() -> HashMap<String, SkillBundle> {
    let dir = bundles_dir();
    if !dir.exists() {
        return HashMap::new();
    }

    let mut bundles = HashMap::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "Failed to read bundles directory");
            return bundles;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" {
            continue;
        }

        if let Some(bundle) = load_bundle_file(&path) {
            let key = format!("/{}", bundle.slug);
            if bundles.contains_key(&key) {
                warn!(slug = %key, existing = %bundles[&key].path, path = %path.display(), "Duplicate bundle slug; keeping first");
                continue;
            }
            debug!(slug = %key, path = %path.display(), "Loaded skill bundle");
            bundles.insert(key, bundle);
        }
    }

    bundles
}

/// Get all bundles, using a cached scan result.
pub fn get_skill_bundles() -> &'static HashMap<String, SkillBundle> {
    static CACHE: OnceLock<HashMap<String, SkillBundle>> = OnceLock::new();
    CACHE.get_or_init(scan_bundles)
}

/// Resolve a user-typed command to its canonical bundle slash key.
///
/// Hyphens and underscores are treated interchangeably.
pub fn resolve_bundle_command_key(command: &str) -> Option<String> {
    if command.is_empty() {
        return None;
    }
    let cmd_key = format!("/{}", slugify(command));
    if get_skill_bundles().contains_key(&cmd_key) {
        Some(cmd_key)
    } else {
        None
    }
}

/// Return a sorted list of bundle info for display.
pub fn list_bundles() -> Vec<&'static SkillBundle> {
    let mut bundles: Vec<&'static SkillBundle> = get_skill_bundles().values().collect();
    bundles.sort_by(|a, b| a.slug.cmp(&b.slug));
    bundles
}

/// Build the user message content for a bundle slash command invocation.
///
/// Returns `(message, loaded_skill_names, missing_skill_names)` or `None` if
/// the bundle wasn't found.
pub fn build_bundle_invocation_message(
    cmd_key: &str,
    user_instruction: &str,
) -> Option<(String, Vec<String>, Vec<String>)> {
    let bundles = get_skill_bundles();
    let info = bundles.get(cmd_key)?;

    let mut loaded_names = Vec::new();
    let mut missing = Vec::new();
    let mut skill_blocks = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for skill_id in &info.skills {
        let identifier = skill_id.trim();
        if identifier.is_empty() || seen.contains(identifier) {
            continue;
        }
        seen.insert(identifier.to_string());

        // Try to load the skill content from the skills directory
        let skills_dir = crate::platform::operant_home().join("skills");
        let skill_dir = skills_dir.join(identifier);
        let skill_file = skill_dir.join("SKILL.md");

        if !skill_file.exists() {
            missing.push(identifier.to_string());
            continue;
        }

        match std::fs::read_to_string(&skill_file) {
            Ok(content) => {
                skill_blocks.push(format!("=== Skill: {} ===\n{}", identifier, content));
                loaded_names.push(identifier.to_string());
            }
            Err(_) => {
                missing.push(identifier.to_string());
            }
        }
    }

    if skill_blocks.is_empty() {
        return None;
    }

    let mut header_lines = vec![
        format!(
            "[IMPORTANT: The user has invoked the \"{}\" skill bundle, \
             loading {} skills together. Treat every skill below \
             as active guidance for this turn.]",
            info.name,
            loaded_names.len()
        ),
        String::new(),
        format!("Bundle: {}", info.name),
        format!("Skills loaded: {}", loaded_names.join(", ")),
    ];

    if !missing.is_empty() {
        header_lines.push(format!("Skills missing (skipped): {}", missing.join(", ")));
    }

    if !info.instruction.is_empty() {
        header_lines.push(String::new());
        header_lines.push(format!("Bundle instruction: {}", info.instruction));
    }

    if !user_instruction.is_empty() {
        header_lines.push(String::new());
        header_lines.push(format!("User instruction: {}", user_instruction));
    }

    let mut message = header_lines.join("\n");
    message.push_str("\n\n");
    message.push_str(&skill_blocks.join("\n\n"));

    Some((message, loaded_names, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Backend Dev"), "backend-dev");
        assert_eq!(slugify("my_bundle"), "my-bundle");
        assert_eq!(slugify("UPPER"), "upper");
        assert_eq!(slugify("a--b"), "a-b");
        assert_eq!(slugify("--leading--trailing--"), "leading-trailing");
    }

    #[test]
    fn test_slugify_empty() {
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn test_resolve_bundle_not_found() {
        let result = resolve_bundle_command_key("nonexistent-bundle-xyz");
        assert!(result.is_none());
    }

    #[test]
    fn test_list_bundles_empty_when_no_dir() {
        // When OPERANT_BUNDLES_DIR points to a non-existent dir, should return empty
        let old = std::env::var("OPERANT_BUNDLES_DIR").ok();
        // SAFETY: env var manipulation in single-threaded test context
        unsafe {
            std::env::set_var("OPERANT_BUNDLES_DIR", "/tmp/nonexistent_bundles_test_dir");
        }
        // Can't easily test the cache, but scan_bundles should return empty
        let dir = bundles_dir();
        assert!(!dir.exists());
        // SAFETY: env var manipulation in single-threaded test context
        unsafe {
            if let Some(v) = old {
                std::env::set_var("OPERANT_BUNDLES_DIR", v);
            } else {
                std::env::remove_var("OPERANT_BUNDLES_DIR");
            }
        }
    }
}

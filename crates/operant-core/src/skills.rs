//! Skills system for Operant-RS
//!
//! Provides skill discovery, loading, and management matching
//! the Python operant-agent's skills architecture. Skills are
//! directories containing a SKILL.md file with YAML front matter.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Parsed YAML frontmatter from a SKILL.md file.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    version: Option<String>,
    author: Option<String>,
    license: Option<String>,
    platforms: Option<Vec<String>>,
    tags: Option<Vec<String>>,
    category: Option<String>,
    prerequisites_env: Option<Vec<String>>,
    prerequisites_commands: Option<Vec<String>>,
    /// Nested metadata (e.g. metadata.operant.tags, metadata.operant.category)
    metadata: Option<serde_json::Value>,
}

/// Where a loaded skill came from — the operator's own pool or a managed
/// integration (ponytail phase 3). `create`/`delete` must only ever touch
/// the operator's pool, so integration-sourced skills carry their origin
/// and the mutators can refuse them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillOrigin {
    /// `~/.operant/skills/` — the operator's own pool.
    OperatorPool,
    /// A versioned vendor dir under `~/.operant/integrations/` (e.g.
    /// ponytail-v5.3.2/skills/) — auto-updated content, never hand-edited.
    Integration,
}

/// A loaded skill with parsed metadata and content.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Unique skill name (derived from directory name)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Semantic version string
    pub version: String,
    /// The full SKILL.md content (body after front matter)
    pub content: String,
    /// Supported platforms (e.g. ["linux", "macos", "windows"])
    pub platforms: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Skill category
    pub category: String,
    /// Required environment variables
    pub prerequisites_env: Vec<String>,
    /// Required commands on PATH
    pub prerequisites_commands: Vec<String>,
    /// Supporting reference files: filename -> content
    pub references: HashMap<String, String>,
    /// Which root this skill was loaded from (phase 3).
    pub origin: SkillOrigin,
}

/// Manages skill discovery, loading, and lifecycle.
pub struct SkillManager {
    /// Root directory containing skill subdirectories
    pub skills_dir: PathBuf,
    /// Additional read-only roots scanned after the operator pool
    /// (ponytail phase 3: installed integration skills). Additive,
    /// default empty — the pre-phase-3 behavior is unchanged.
    extra_roots: Vec<PathBuf>,
    /// Cache of loaded skills keyed by name
    skills: HashMap<String, Skill>,
}

impl SkillManager {
    /// Create a new SkillManager pointing at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            extra_roots: Vec::new(),
            skills: HashMap::new(),
        }
    }

    /// Add a read-only extra root scanned AFTER the operator pool
    /// (ponytail phase 3: the installed integration's skills dir).
    /// Operator-pool skills win name collisions by load order. The root
    /// is optional at the call site — `None` (integration not installed)
    /// is a no-op, so wiring can be unconditional.
    pub fn add_extra_root(&mut self, root: Option<PathBuf>) {
        if let Some(r) = root {
            self.extra_roots.push(r);
        }
    }

    /// Scan the skills directory and load all valid skills.
    ///
    /// Each subdirectory containing a `SKILL.md` file is treated as a skill.
    /// Skills with parse errors are logged and skipped. Extra roots
    /// (phase 3) are scanned after the primary; their skills are invocable
    /// and listable but NOT creatable/deletable through this manager
    /// (see [`Skill::origin`]).
    pub fn load_all(&mut self) -> Result<Vec<Skill>> {
        self.skills.clear();

        let mut loaded = Vec::new();
        // Same borrow shape as the extra-roots loop below: load_root takes
        // &mut self, so the root path is cloned out first.
        let primary = self.skills_dir.clone();
        self.load_root(&primary, SkillOrigin::OperatorPool, &mut loaded)?;
        // Take the extra roots out of self for the loop — load_root needs
        // &mut self (it inserts into the cache), so iterating a self-borrow
        // in the loop head would not compile.
        let extra_roots = std::mem::take(&mut self.extra_roots);
        for root in &extra_roots {
            // A missing integration dir is a normal state (not installed
            // yet) — skip it silently, never a load error.
            if root.is_dir() {
                self.load_root(root, SkillOrigin::Integration, &mut loaded)?;
            }
        }
        self.extra_roots = extra_roots;

        Ok(loaded)
    }

    /// Scan one root. `load_all`'s old body, parameterized per-root.
    fn load_root(
        &mut self,
        root: &Path,
        origin: SkillOrigin,
        loaded: &mut Vec<Skill>,
    ) -> Result<()> {
        let entries = std::fs::read_dir(root).map_err(|e| {
            Error::Config(format!(
                "Failed to read skills directory '{}': {}",
                root.display(),
                e
            ))
        })?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }

            match load_skill(&path, origin) {
                Ok(skill) => {
                    // Index by both the frontmatter name and the directory
                    // name so lookups by either resolve (a skill's directory
                    // can differ from its `name:` frontmatter). Without the
                    // directory alias, `skills audit` misreports such skills
                    // as "Failed to parse SKILL.md". First root wins: the
                    // operator pool is scanned before extra roots, so an
                    // operator's same-named skill shadows the integration's.
                    let dir_key = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| skill.name.clone());
                    let shadowed =
                        self.skills.contains_key(&skill.name) || self.skills.contains_key(&dir_key);
                    if !shadowed {
                        self.skills.insert(skill.name.clone(), skill.clone());
                        if dir_key != skill.name {
                            self.skills.insert(dir_key, skill.clone());
                        }
                        loaded.push(skill);
                    }
                }
                Err(_) => {
                    // Skip skills that fail to parse
                    continue;
                }
            }
        }

        Ok(())
    }

    /// Get a specific skill by name.
    ///
    /// Returns `None` if the skill hasn't been loaded or doesn't exist.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// List all loaded skills as `(name, description)` pairs.
    pub fn list(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<_> = self
            .skills
            .values()
            .map(|s| (s.name.clone(), s.description.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    /// Check whether a skill is available on the current platform
    /// and all prerequisites are met.
    pub fn is_available(&self, skill: &Skill) -> bool {
        // Check platform
        if !skill.platforms.is_empty() {
            let current = current_platform();
            if !skill.platforms.iter().any(|p| p == current) {
                return false;
            }
        }

        // Check required environment variables
        for var in &skill.prerequisites_env {
            if std::env::var(var).is_err() {
                return false;
            }
        }

        // Check required commands
        for cmd in &skill.prerequisites_commands {
            if which::which(cmd).is_err() {
                return false;
            }
        }

        true
    }

    /// Create a new skill with the given name and SKILL.md content.
    ///
    /// Creates a subdirectory under `skills_dir` with a `SKILL.md` file.
    pub fn create(&mut self, name: &str, content: &str) -> Result<()> {
        let skill_dir = self.skills_dir.join(name);

        if skill_dir.exists() {
            return Err(Error::Config(format!("Skill '{}' already exists", name)));
        }

        std::fs::create_dir_all(&skill_dir).map_err(|e| {
            Error::Config(format!(
                "Failed to create skill directory '{}': {}",
                skill_dir.display(),
                e
            ))
        })?;

        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, content).map_err(|e| {
            Error::Config(format!("Failed to write SKILL.md for '{}': {}", name, e))
        })?;

        // Reload the newly created skill into cache (always OperatorPool —
        // `create` only ever writes under skills_dir).
        if let Ok(skill) = load_skill(&skill_dir, SkillOrigin::OperatorPool) {
            self.skills.insert(skill.name.clone(), skill);
        }

        Ok(())
    }

    /// Delete a skill by removing its directory.
    ///
    /// Refuses integration-sourced skills (phase 3): the vendor tree is
    /// version-managed by `operant update --integrations` — deleting a
    /// skill from it would be silently undone by the next update anyway.
    /// Delete the operator's shadowing copy or disable the integration
    /// instead.
    pub fn delete(&mut self, name: &str) -> Result<()> {
        if let Some(skill) = self.skills.get(name)
            && skill.origin == SkillOrigin::Integration
        {
            return Err(Error::Config(format!(
                "Skill '{}' is managed by an integration — delete is refused \
                 (the next `operant update --integrations` would restore it). \
                 Disable the integration or shadow it with an operator skill.",
                name
            )));
        }

        let skill_dir = self.skills_dir.join(name);

        if !skill_dir.exists() {
            return Err(Error::Config(format!("Skill '{}' not found", name)));
        }

        std::fs::remove_dir_all(&skill_dir)
            .map_err(|e| Error::Config(format!("Failed to delete skill '{}': {}", name, e)))?;

        self.skills.remove(name);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Load a single skill from its directory.
fn load_skill(skill_dir: &Path, origin: SkillOrigin) -> Result<Skill> {
    let dir_name = skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Config("Invalid skill directory name".into()))?
        .to_string();

    let skill_file = skill_dir.join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_file)
        .map_err(|e| Error::Config(format!("Failed to read SKILL.md in '{}': {}", dir_name, e)))?;

    let fm = parse_front_matter(&raw)?;
    let body = extract_body(&raw);

    let (tags, category) = extract_metadata_extras(&fm);

    let name = fm.name.unwrap_or_else(|| dir_name.clone());
    let description = fm.description.unwrap_or_default();
    let version = fm.version.unwrap_or_else(|| "0.1.0".into());
    let platforms = fm.platforms.unwrap_or_default();
    let prerequisites_env = fm.prerequisites_env.unwrap_or_default();
    let prerequisites_commands = fm.prerequisites_commands.unwrap_or_default();

    // Load reference files (everything in the skill dir that isn't SKILL.md)
    let mut references = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(fname) = path.file_name().and_then(|n| n.to_str())
                && fname != "SKILL.md"
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                references.insert(fname.to_string(), content);
            }
        }
    }

    Ok(Skill {
        name,
        description,
        version,
        content: body,
        platforms,
        tags,
        category,
        prerequisites_env,
        prerequisites_commands,
        references,
        origin,
    })
}

/// Extract tags and category from top-level fields or metadata.operant.*.
fn extract_metadata_extras(fm: &SkillFrontmatter) -> (Vec<String>, String) {
    let mut tags = fm.tags.clone().unwrap_or_default();
    let mut category = fm.category.clone().unwrap_or_default();

    if let Some(meta) = &fm.metadata
        && let Some(operant) = meta.get("operant")
    {
        if tags.is_empty()
            && let Some(t) = operant.get("tags").and_then(|v| v.as_array())
        {
            tags = t
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if category.is_empty()
            && let Some(c) = operant.get("category").and_then(|v| v.as_str())
        {
            category = c.to_string();
        }
    }

    (tags, category)
}

fn extract_body(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return raw.to_string();
    }
    let after_open = &trimmed[3..];
    match after_open.find("\n---") {
        Some(pos) => {
            let body_start = 3 + pos + 4;
            if body_start < trimmed.len() {
                trimmed[body_start..].trim_start_matches('\n').to_string()
            } else {
                String::new()
            }
        }
        None => raw.to_string(),
    }
}

fn parse_front_matter(raw: &str) -> Result<SkillFrontmatter> {
    let trimmed = raw.trim_start();

    if !trimmed.starts_with("---") {
        return Ok(SkillFrontmatter::default());
    }

    let after_open = &trimmed[3..];
    let close_pos = after_open
        .find("\n---")
        .ok_or_else(|| Error::Config("SKILL.md front matter missing closing '---'".into()))?;

    let fm_block = &after_open[..close_pos];

    // Try full YAML parse first
    if let Ok(fm) = serde_yaml::from_str::<SkillFrontmatter>(fm_block) {
        return Ok(fm);
    }

    // Fall back to flat key-value parsing for backward compatibility
    Ok(parse_flat_front_matter(fm_block))
}

/// Flat key-value fallback for legacy SKILL.md files.
fn parse_flat_front_matter(fm_block: &str) -> SkillFrontmatter {
    let mut map = HashMap::new();
    for line in fm_block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_string();
            let value = line[colon_pos + 1..].trim().to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }

    SkillFrontmatter {
        name: map.get("name").cloned(),
        description: map.get("description").cloned(),
        version: map.get("version").cloned(),
        author: map.get("author").cloned(),
        license: map.get("license").cloned(),
        platforms: map.get("platforms").map(|v| parse_list_value(v)),
        tags: map.get("tags").map(|v| parse_list_value(v)),
        category: map.get("category").cloned(),
        prerequisites_env: map.get("prerequisites_env").map(|v| parse_list_value(v)),
        prerequisites_commands: map
            .get("prerequisites_commands")
            .map(|v| parse_list_value(v)),
        metadata: None,
    }
}

fn parse_list_value(s: &str) -> Vec<String> {
    let s = s.trim();
    let inner = if s.starts_with('[') && s.ends_with(']') {
        &s[1..s.len() - 1]
    } else {
        s
    };
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Return the current platform as a lowercase string matching common conventions.
fn current_platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use std::sync::atomic::{AtomicU64, Ordering};
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_temp_dir() -> PathBuf {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "operant_skills_test_{}_{}",
            std::process::id(),
            count
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn sample_skill_md() -> &'static str {
        "---\n\
         name: test-skill\n\
         description: A test skill for unit tests\n\
         version: 1.0.0\n\
         platforms: [linux, macos, windows]\n\
         prerequisites_env: [HOME]\n\
         prerequisites_commands: []\n\
         ---\n\
         # Test Skill\n\
         \n\
         This is the skill content.\n"
    }

    #[test]
    fn test_parse_front_matter() {
        let fm = parse_front_matter(sample_skill_md()).unwrap();
        assert_eq!(fm.name.as_deref(), Some("test-skill"));
        assert_eq!(
            fm.description.as_deref(),
            Some("A test skill for unit tests")
        );
        assert_eq!(fm.version.as_deref(), Some("1.0.0"));
        let body = extract_body(sample_skill_md());
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("This is the skill content."));
    }

    #[test]
    fn test_parse_front_matter_no_front_matter() {
        let fm = parse_front_matter("# Just content\nNo front matter here").unwrap();
        assert!(fm.name.is_none());
        let body = extract_body("# Just content\nNo front matter here");
        assert!(body.contains("# Just content"));
    }

    #[test]
    fn test_parse_list_inline() {
        let result = parse_list_value("[linux, macos, windows]");
        assert_eq!(result, vec!["linux", "macos", "windows"]);
    }

    #[test]
    fn test_parse_list_bare() {
        let result = parse_list_value("linux, macos");
        assert_eq!(result, vec!["linux", "macos"]);
    }

    #[test]
    fn test_parse_list_empty() {
        assert!(parse_list_value("").is_empty());
        assert!(parse_list_value("[]").is_empty());
    }

    #[test]
    fn test_load_skill() {
        let tmp = make_temp_dir();
        let skill_dir = tmp.join("my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), sample_skill_md()).unwrap();
        fs::write(skill_dir.join("helper.py"), "print('hello')").unwrap();

        let skill = load_skill(&skill_dir, SkillOrigin::OperatorPool).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.version, "1.0.0");
        assert_eq!(skill.platforms, vec!["linux", "macos", "windows"]);
        assert!(skill.content.contains("# Test Skill"));
        assert_eq!(skill.references.get("helper.py").unwrap(), "print('hello')");

        cleanup(&tmp);
    }

    #[test]
    fn test_skill_manager_load_all() {
        let tmp = make_temp_dir();

        // Create two skills
        let s1 = tmp.join("skill-a");
        fs::create_dir_all(&s1).unwrap();
        fs::write(
            s1.join("SKILL.md"),
            "---\nname: skill-a\ndescription: First\nversion: 0.1.0\n---\nContent A\n",
        )
        .unwrap();

        let s2 = tmp.join("skill-b");
        fs::create_dir_all(&s2).unwrap();
        fs::write(
            s2.join("SKILL.md"),
            "---\nname: skill-b\ndescription: Second\nversion: 0.2.0\n---\nContent B\n",
        )
        .unwrap();

        // Create a non-skill directory (no SKILL.md)
        let s3 = tmp.join("not-a-skill");
        fs::create_dir_all(&s3).unwrap();
        fs::write(s3.join("README.md"), "nothing").unwrap();

        let mut mgr = SkillManager::new(tmp.clone());
        let loaded = mgr.load_all().unwrap();
        assert_eq!(loaded.len(), 2);

        let list = mgr.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|(n, _)| n == "skill-a"));
        assert!(list.iter().any(|(n, _)| n == "skill-b"));

        assert!(mgr.get("skill-a").is_some());
        assert!(mgr.get("nonexistent").is_none());

        cleanup(&tmp);
    }

    /// Ponytail phase 3: extra roots load alongside the operator pool,
    /// carry `SkillOrigin::Integration`, and a missing extra root is a
    /// silent no-op (integration not installed is a normal state).
    #[test]
    fn extra_root_loads_integration_skills_and_missing_root_is_noop() {
        let pool = make_temp_dir();
        let extra = make_temp_dir();

        fs::create_dir_all(pool.join("mine")).unwrap();
        fs::write(
            pool.join("mine").join("SKILL.md"),
            "---\nname: mine\ndescription: operator skill\n---\nbody\n",
        )
        .unwrap();
        fs::create_dir_all(extra.join("ponytail-review")).unwrap();
        fs::write(
            extra.join("ponytail-review").join("SKILL.md"),
            "---\nname: ponytail-review\ndescription: vendor skill\n---\nbody\n",
        )
        .unwrap();

        let mut mgr = SkillManager::new(pool.clone());
        mgr.add_extra_root(Some(extra.clone()));
        mgr.add_extra_root(None); // None must be a no-op, not an error
        mgr.add_extra_root(Some(pool.join("does-not-exist"))); // missing root skipped
        let loaded = mgr.load_all().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(mgr.get("mine").unwrap().origin, SkillOrigin::OperatorPool);
        assert_eq!(
            mgr.get("ponytail-review").unwrap().origin,
            SkillOrigin::Integration
        );

        cleanup(&pool);
        cleanup(&extra);
    }

    /// Ponytail phase 3: the operator pool wins name collisions — the
    /// integration's same-named skill is shadowed in BOTH indexes.
    #[test]
    fn operator_pool_shadows_integration_skill_on_name_collision() {
        let pool = make_temp_dir();
        let extra = make_temp_dir();

        for root in [&pool, &extra] {
            fs::create_dir_all(root.join("clash")).unwrap();
            fs::write(
                root.join("clash").join("SKILL.md"),
                "---\nname: clash\ndescription: from this root\n---\nbody\n",
            )
            .unwrap();
        }

        let mut mgr = SkillManager::new(pool.clone());
        mgr.add_extra_root(Some(extra.clone()));
        let loaded = mgr.load_all().unwrap();
        assert_eq!(loaded.len(), 1, "shadowed duplicate must not double-list");
        assert_eq!(mgr.get("clash").unwrap().origin, SkillOrigin::OperatorPool);

        cleanup(&pool);
        cleanup(&extra);
    }

    /// Ponytail phase 3: delete refuses integration-sourced skills — the
    /// vendor tree is version-managed and a hand delete would be undone by
    /// the next `operant update --integrations` anyway.
    #[test]
    fn delete_refuses_integration_skills() {
        let pool = make_temp_dir();
        let extra = make_temp_dir();
        fs::create_dir_all(extra.join("vendor")).unwrap();
        fs::write(
            extra.join("vendor").join("SKILL.md"),
            "---\nname: vendor\ndescription: v\n---\nbody\n",
        )
        .unwrap();

        let mut mgr = SkillManager::new(pool.clone());
        mgr.add_extra_root(Some(extra.clone()));
        mgr.load_all().unwrap();

        assert!(mgr.delete("vendor").is_err());
        assert!(extra.join("vendor").join("SKILL.md").exists());

        cleanup(&pool);
        cleanup(&extra);
    }

    #[test]
    fn test_skill_manager_create_and_delete() {
        let tmp = make_temp_dir();
        let mut mgr = SkillManager::new(tmp.clone());

        let content =
            "---\nname: new-skill\ndescription: Created skill\nversion: 1.0.0\n---\n# New\n";
        mgr.create("new-skill", content).unwrap();

        assert!(tmp.join("new-skill").join("SKILL.md").exists());
        assert!(mgr.get("new-skill").is_some());
        assert_eq!(mgr.get("new-skill").unwrap().description, "Created skill");

        // Creating duplicate should fail
        assert!(mgr.create("new-skill", content).is_err());

        // Delete
        mgr.delete("new-skill").unwrap();
        assert!(!tmp.join("new-skill").exists());
        assert!(mgr.get("new-skill").is_none());

        // Deleting non-existent should fail
        assert!(mgr.delete("new-skill").is_err());

        cleanup(&tmp);
    }

    #[test]
    fn test_is_available_platform_match() {
        let skill = Skill {
            name: "test".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            content: String::new(),
            platforms: vec![current_platform().to_string()],
            tags: vec![],
            category: String::new(),
            prerequisites_env: vec![],
            prerequisites_commands: vec![],
            references: HashMap::new(),
            origin: SkillOrigin::OperatorPool,
        };

        let mgr = SkillManager::new(PathBuf::from("."));
        assert!(mgr.is_available(&skill));
    }

    #[test]
    fn test_is_available_platform_mismatch() {
        let skill = Skill {
            name: "test".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            content: String::new(),
            platforms: vec!["plan9".to_string()],
            tags: vec![],
            category: String::new(),
            prerequisites_env: vec![],
            prerequisites_commands: vec![],
            references: HashMap::new(),
            origin: SkillOrigin::OperatorPool,
        };

        let mgr = SkillManager::new(PathBuf::from("."));
        assert!(!mgr.is_available(&skill));
    }

    #[test]
    fn test_is_available_empty_platforms_means_all() {
        let skill = Skill {
            name: "test".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            content: String::new(),
            platforms: vec![],
            tags: vec![],
            category: String::new(),
            prerequisites_env: vec![],
            prerequisites_commands: vec![],
            references: HashMap::new(),
            origin: SkillOrigin::OperatorPool,
        };

        let mgr = SkillManager::new(PathBuf::from("."));
        assert!(mgr.is_available(&skill));
    }

    #[test]
    fn test_is_available_missing_env() {
        let skill = Skill {
            name: "test".into(),
            description: "test".into(),
            version: "1.0.0".into(),
            content: String::new(),
            platforms: vec![],
            tags: vec![],
            category: String::new(),
            prerequisites_env: vec!["HERMES_NONEXISTENT_VAR_12345".into()],
            prerequisites_commands: vec![],
            references: HashMap::new(),
            origin: SkillOrigin::OperatorPool,
        };

        let mgr = SkillManager::new(PathBuf::from("."));
        assert!(!mgr.is_available(&skill));
    }

    #[test]
    fn test_load_skill_defaults() {
        let tmp = make_temp_dir();
        let skill_dir = tmp.join("minimal");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Just content, no front matter\n",
        )
        .unwrap();

        let skill = load_skill(&skill_dir, SkillOrigin::OperatorPool).unwrap();
        // Name falls back to directory name
        assert_eq!(skill.name, "minimal");
        assert_eq!(skill.version, "0.1.0");
        assert!(skill.platforms.is_empty());
        assert!(skill.content.contains("# Just content"));

        cleanup(&tmp);
    }
}

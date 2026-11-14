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
    /// `<workspace>/skills` (or an explicit `project_root` path) — the
    /// repository's own skills, version-controlled by someone else. Readable
    /// and invocable; `create`/`delete` refuse it for the same reason they
    /// refuse an integration: an in-place edit is a change to another tree.
    Project,
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

/// The skills tree the operator configured (RT-265).
///
/// `[skills] root_dir` is the operator's setting; the platform default is
/// only its DEFAULT value. A caller that reaches for
/// [`crate::platform::operant_skills_dir`] directly ignores an operator's
/// setting and reads — or writes — a tree the agent never uses.
///
/// Seven callers did exactly that: the curator's auto-run archived the
/// lifecycle of skills IN ANOTHER TREE, the skill-bundle loader reported
/// every skill in a bundle as "missing", `skill marketplace install` wrote
/// the skill where the agent never looks, and the status/dump panels
/// displayed a directory nobody configured.
///
/// Empty means "unset": fall back to the platform default (also what a fresh
/// config resolves to) — the same rule the kernel tool's config-aware branch
/// has always applied.
pub fn configured_skills_dir() -> PathBuf {
    let root = crate::config::runtime_config().skills.root_dir;
    if root.as_os_str().is_empty() {
        crate::platform::operant_skills_dir()
    } else {
        root
    }
}

// ---------------------------------------------------------------------------
// The ONE rule for which roots a skill may live in (iter-2159)
// ---------------------------------------------------------------------------

/// The vendor roots a skill may live in besides the operator's own pool:
/// the installed content integrations (the ponytail package) plus whatever
/// extra roots the HOST contributes (plugin-contributed skill dirs).
///
/// ONE list, so every surface that can see or run a skill — the manager,
/// the in-loop tools, the overlays and the slash-command invocation — agrees
/// about which skills exist. The operator's pool always wins a name
/// collision (it is searched first), and a Holon run carries no vendor root
/// at all: an org body does not get a personal integration.
pub fn vendor_skill_roots(host_roots: &[PathBuf]) -> Vec<PathBuf> {
    if crate::platform::is_holon_run() {
        return Vec::new();
    }
    let mut roots: Vec<PathBuf> = crate::ponytail::installed_skills_dir()
        .into_iter()
        .collect();
    roots.extend(host_roots.iter().cloned());
    roots
}

/// The directory holding `name`, searched in `roots` order — the operator's
/// pool first, so the operator's own copy wins a collision with a vendor
/// skill of the same name. `name` may be a bare skill name (found by the
/// recursive search, including category-nested ones) or a nested path
/// (`category/skill`); the path guard keeps a name from escaping its root.
pub fn find_skill_in_roots(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    for root in roots {
        if let Some(found) = find_skill_in_root(root, name) {
            return Some(found);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The ONE resolver for which roots a skill lives in (iter-2254)
// ---------------------------------------------------------------------------

/// One entry in a root walk that is not a loadable skill — reported, never
/// swallowed. A dead symlink is a configuration defect the operator has to
/// be able to see, and a credential-store path is a refusal they have to be
/// able to audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootIssue {
    /// The root the problem was found under.
    pub root: PathBuf,
    /// The offending entry (absolute where it could be resolved).
    pub entry: PathBuf,
    /// Where a symlink pointed, when that is what made it an issue.
    pub link_target: Option<PathBuf>,
    /// Machine-stable reason: `dead_symlink`, `sensitive_path`, `missing`,
    /// `unreadable`, `not_a_directory`.
    pub reason: String,
}

impl RootIssue {
    /// The human-readable line the doctor prints.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.link_target {
            Some(target) => format!(
                "{}: {} -> {} ({})",
                self.root.display(),
                self.entry.display(),
                target.display(),
                self.reason
            ),
            None => format!(
                "{}: {} ({})",
                self.root.display(),
                self.entry.display(),
                self.reason
            ),
        }
    }
}

/// Every directory a skill may live in, in precedence order, plus the
/// problems found while resolving them.
///
/// This is the single owner of the list. Every surface that can see or run a
/// skill takes it: the `$` popup and `/skill <Tab>`, the `/skills` overlay,
/// the `$name` expansion in both the TUI and the gateway, the `skills_list` /
/// `skill_view` tools, the gateway's picker, and the agent's `SkillManager`.
/// iter-2159 and iter-2198 each hand-threaded a list and each found a surface
/// that disagreed with the rest; the fix is one producer, not a third copy.
#[derive(Debug, Clone, Default)]
pub struct SkillRoots {
    /// Roots in precedence order. The first root that yields a name wins.
    pub roots: Vec<PathBuf>,
    /// Roots that were configured but missing, unreadable, denied, or that
    /// contained a dead symlink. Printed by `operant doctor`.
    pub issues: Vec<RootIssue>,
    /// True when a walk hit its cap, so the set is known to be incomplete.
    pub truncated: bool,
    /// The project root that was resolved, when one was.
    pub project: Option<PathBuf>,
}

/// The most directory entries one root walk will visit.
///
/// `~/.agents/skills` holds 3 517 files over 285 skills; a cap of 20 000
/// visits covers any plausible pool without letting a pathological tree (a
/// recursive symlink farm, a `node_modules` that is not skipped because it
/// sits above the skill) stall a turn. Truncation is REPORTED, never silent.
pub const SKILL_ROOT_WALK_LIMIT: usize = 20_000;

impl SkillRoots {
    /// Resolve the root list from the config, the host's vendor roots and the
    /// session workspace.
    ///
    /// Precedence: the primary pool (`config.root_dir`, or `holon_override`
    /// on a Holon run) → the operator's `extra_roots` → the project root →
    /// the vendor roots. A Holon run keeps its existing rule: the brief's
    /// library only, no personal pool and no vendor integration — the
    /// decision stays with the caller that already reads `is_holon_run()`.
    #[must_use]
    pub fn resolve(
        config: &crate::config::SkillsSettings,
        vendor_roots: &[PathBuf],
        workspace: Option<&Path>,
        holon_override: Option<&Path>,
    ) -> SkillRoots {
        let mut out = SkillRoots::default();

        // The primary pool. `root_dir` is already `~`-expanded by
        // `SkillsSettings::expand_paths`.
        let primary = holon_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| config.root_dir.clone());
        out.push_root(&primary);

        if holon_override.is_some() {
            // A Holon run resolves to the brief's library only: the operator's
            // `extra_roots`, the project and the vendor roots would all hand a
            // personal or ambient library to an org body.
            return out;
        }

        for extra in &config.extra_roots {
            out.push_root(extra);
        }

        let project = if config.project_discovery_enabled() {
            config
                .explicit_project_root()
                .map(Path::to_path_buf)
                .or_else(|| workspace.map(|ws| ws.join("skills")))
        } else {
            None
        };
        if let Some(dir) = &project {
            out.push_root(dir);
        }
        out.project = project;

        for vendor in vendor_roots {
            out.push_root(vendor);
        }

        out
    }

    /// Add one root, recording why it was refused instead of dropping it.
    fn push_root(&mut self, root: &Path) {
        // Canonicalize so a symlinked root and its target dedupe, and so a
        // relative configured path is resolved before anything is compared.
        let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if self.roots.contains(&canonical) {
            return;
        }
        if let Some(reason) = crate::context_references::sensitive_path_reason(&canonical) {
            self.issues.push(RootIssue {
                root: canonical,
                entry: root.to_path_buf(),
                link_target: None,
                reason: format!("sensitive_path: {reason}"),
            });
            return;
        }
        match std::fs::metadata(&canonical) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.issues.push(RootIssue {
                    root: canonical,
                    entry: root.to_path_buf(),
                    link_target: None,
                    reason: "missing".to_string(),
                });
            }
            Err(e) => {
                self.issues.push(RootIssue {
                    root: canonical,
                    entry: root.to_path_buf(),
                    link_target: None,
                    reason: format!("unreadable: {e}"),
                });
            }
            Ok(meta) if !meta.is_dir() => {
                self.issues.push(RootIssue {
                    root: canonical,
                    entry: root.to_path_buf(),
                    link_target: None,
                    reason: "not_a_directory".to_string(),
                });
            }
            Ok(_) => self.roots.push(canonical),
        }
    }

    /// The roots as a flat slice, in the order `find_skill_in_roots` and
    /// `SkillManager::load_all` consume them.
    #[must_use]
    pub fn as_paths(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    /// True when nothing at all resolved — a configuration the operator has
    /// to hear about, not a silently empty skill list.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

/// Walk one root for skills, depth-first, honouring the ONE skip rule and the
/// ONE cap. Returns each skill directory (`<dir>/SKILL.md` exists).
///
/// The listing and the resolution used to disagree about depth: `load_root`
/// read immediate children while `find_skill_recursive` recursed, so a
/// category-nested skill was FINDABLE but not LISTABLE. This is the single
/// walk every surface uses now.
pub fn walk_skill_dirs(root: &Path, limit: usize) -> (Vec<PathBuf>, Vec<RootIssue>, bool) {
    let mut found = Vec::new();
    let mut issues = Vec::new();
    let mut visited = 0usize;
    let mut truncated = false;

    fn recurse(
        dir: &Path,
        root: &Path,
        limit: usize,
        visited: &mut usize,
        found: &mut Vec<PathBuf>,
        issues: &mut Vec<RootIssue>,
        truncated: &mut bool,
    ) {
        if *visited >= limit {
            *truncated = true;
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for path in children {
            if *visited >= limit {
                *truncated = true;
                return;
            }
            *visited += 1;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            // A dead symlink is a defect to REPORT, not an entry to skip in
            // silence: `Path::is_dir()` follows links, so the loader below
            // would simply not see it.
            if path.is_symlink() && !path.exists() {
                issues.push(RootIssue {
                    root: root.to_path_buf(),
                    entry: path.clone(),
                    link_target: std::fs::read_link(&path).ok(),
                    reason: "dead_symlink".to_string(),
                });
                continue;
            }
            if !path.is_dir() || is_skipped_walk_name(&name) {
                continue;
            }
            if path.join("SKILL.md").is_file() {
                found.push(path);
                continue;
            }
            recurse(&path, root, limit, visited, found, issues, truncated);
        }
    }

    recurse(
        root,
        root,
        limit,
        &mut visited,
        &mut found,
        &mut issues,
        &mut truncated,
    );
    (found, issues, truncated)
}

/// The ONE skip rule for a directory name during a root walk.
///
/// Hidden directories (`.git`, `.archive`, `.curator`, `.hub`), `_`-prefixed
/// names, and the build/vendor trees that are never skills. This replaces
/// three different rules across four walkers; the seeder calls the same
/// function, so a bundled install and a live root cannot disagree.
#[must_use]
pub fn is_skipped_walk_name(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('_')
        || matches!(name, "node_modules" | "target" | "__pycache__")
}

fn find_skill_in_root(root: &Path, name: &str) -> Option<PathBuf> {
    if !root.exists() || !is_plain_relative_path(name) {
        return None;
    }
    let direct = root.join(name);
    if direct.is_dir() && direct.join("SKILL.md").exists() {
        return Some(direct);
    }
    find_skill_recursive(root, name)
}

fn find_skill_recursive(current_dir: &Path, name: &str) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(current_dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_skipped_walk_name(dir_name) {
            continue;
        }
        if path.join("SKILL.md").exists() {
            // Match the DIRECTORY name first (cheap, no read), then the
            // frontmatter `name:`. The frontmatter is what the listing
            // advertises and what the `$` popup offers, so a resolver that
            // matched only the directory made an advertised skill
            // uninvocable: `mlops/training/fine-tuning-with-trl` carries
            // `name: trl-fine-tuning`, was listed and offered as
            // `trl-fine-tuning`, and resolved to nothing. Two real skills on
            // this host had that shape.
            if dir_name == name || skill_frontmatter_name(&path).as_deref() == Some(name) {
                return Some(path);
            }
        }
        if let Some(found) = find_skill_recursive(&path, name) {
            return Some(found);
        }
    }
    None
}

/// The `name:` a skill's `SKILL.md` declares, or `None` when it declares none
/// (or cannot be read). A malformed frontmatter is a non-match, never an error:
/// the resolver's job is to answer "is there such a skill", and the loader is
/// what reports a broken one.
fn skill_frontmatter_name(skill_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(skill_dir.join("SKILL.md")).ok()?;
    parse_front_matter(&raw).ok()?.name
}

/// A skill name is a plain relative path: never absolute, and never
/// carrying a `.`/`..` component that would escape the root.
fn is_plain_relative_path(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
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
    /// Problems the last `load_all` found: dead symlinks and refused roots.
    issues: Vec<RootIssue>,
    /// Whether the last `load_all` hit the walk cap.
    truncated: bool,
}

impl SkillManager {
    /// Create a new SkillManager pointing at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            extra_roots: Vec::new(),
            skills: HashMap::new(),
            issues: Vec::new(),
            truncated: false,
        }
    }

    /// Build a manager over a resolved root list — the same list every other
    /// surface takes, so the manager, the tools and the popup cannot disagree
    /// about which skills exist.
    ///
    /// The first root is the pool (`create`/`delete` target it); the rest are
    /// read-only, with their origin carried per skill.
    #[must_use]
    pub fn from_roots(roots: &SkillRoots) -> Self {
        let mut iter = roots.roots.iter();
        let primary = iter
            .next()
            .cloned()
            .unwrap_or_else(crate::platform::operant_skills_dir);
        let mut manager = Self::new(primary);
        for root in iter {
            manager.add_extra_root(Some(root.clone()));
        }
        manager
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
    /// Each directory containing a `SKILL.md` file is treated as a skill,
    /// at ANY depth under the root: a category-nested skill
    /// (`software-development/axi`) is listed as well as found, which is what
    /// `find_skill_in_roots` always did and `load_root` did not. Skills with
    /// parse errors are logged and skipped. Extra roots are scanned after the
    /// primary; their skills are invocable and listable but NOT
    /// creatable/deletable through this manager (see [`Skill::origin`]).
    pub fn load_all(&mut self) -> Result<Vec<Skill>> {
        self.skills.clear();
        self.issues.clear();
        self.truncated = false;

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

    /// The problems found while loading: dead symlinks under any root, and
    /// anything the walk had to refuse. Empty on a clean configuration.
    #[must_use]
    pub fn issues(&self) -> &[RootIssue] {
        &self.issues
    }

    /// True when a walk hit [`SKILL_ROOT_WALK_LIMIT`] and the loaded set is
    /// known to be incomplete.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Scan one root. `load_all`'s body, parameterized per-root.
    fn load_root(
        &mut self,
        root: &Path,
        origin: SkillOrigin,
        loaded: &mut Vec<Skill>,
    ) -> Result<()> {
        if !root.is_dir() {
            // A missing root is normal (an integration that is not installed);
            // the resolver has already recorded it if the operator named it.
            return Ok(());
        }

        let (dirs, issues, truncated) = walk_skill_dirs(root, SKILL_ROOT_WALK_LIMIT);
        self.issues.extend(issues);
        self.truncated |= truncated;

        for path in dirs {
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
                    // A category-qualified alias too, so `$category/skill`
                    // resolves by the path the listing shows. Built from the
                    // root-relative path, which cannot escape the root.
                    let qualified = path
                        .strip_prefix(root)
                        .ok()
                        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                        .filter(|rel| rel.contains('/'));
                    // SHADOWING IS BY SKILL NAME, NOT BY DIRECTORY NAME.
                    // The operator pool is scanned before extra roots, so an
                    // operator's same-NAMED skill shadows an integration's --
                    // but a shared DIRECTORY name must only cost the alias, not
                    // the skill. Measuring the old rule (skip when either the
                    // name OR the dir key was already present): a pool skill
                    // at `a/shared` named `alpha` and an integration skill at
                    // `b/shared` named `beta` left `beta` unlisted AND
                    // unresolvable, because `shared` was already taken by
                    // `alpha`'s alias.
                    if self.skills.contains_key(&skill.name) {
                        continue;
                    }
                    self.skills.insert(skill.name.clone(), skill.clone());
                    // The directory alias is best-effort: an existing entry
                    // (the operator's own alias, or another root's) wins.
                    if dir_key != skill.name {
                        self.skills.entry(dir_key).or_insert_with(|| skill.clone());
                    }
                    if let Some(qualified) = qualified
                        && qualified != skill.name
                        && !self.skills.contains_key(&qualified)
                    {
                        self.skills.insert(qualified, skill.clone());
                    }
                    loaded.push(skill);
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
    ///
    /// Kept unfiltered: it is the inventory surface, and a skill that is
    /// present-but-unusable still has to be describable. Callers that ADVERTISE
    /// a skill to a model must use [`Self::list_available`]; see the row-7 note
    /// on that method for why.
    pub fn list(&self) -> Vec<(String, String)> {
        // ONE row per SKILL, not per alias. The map is keyed by every spelling
        // a skill answers to -- its frontmatter name, its directory name and,
        // for a category-nested skill, its root-relative `category/skill` path
        // -- and each key holds a CLONE of the same `Skill`. Iterating keys
        // therefore listed one nested skill twice (measured: `["deep", "deep"]`)
        // while the qualified spelling the listing is supposed to show appeared
        // nowhere. Deduplicating on the skill's own name is what makes the
        // inventory one row per skill again.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for skill in self.skills.values() {
            if pairs.iter().any(|(name, _)| name == &skill.name) {
                continue;
            }
            pairs.push((skill.name.clone(), skill.description.clone()));
        }
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    /// List the skills that are actually usable on this machine, as
    /// `(name, description)` pairs.
    ///
    /// Row 7 (codex-architecture-deepen): `is_available()` existed and worked,
    /// and no production code called it. The frozen prompt's
    /// `<available_skills>` block was built from the unfiltered [`Self::list`],
    /// so the model was advertised skills whose `prerequisites_env` or
    /// `prerequisites_commands` were missing. It would pick one, the tool
    /// would refuse, and the whole turn degraded. Advertising is the one place
    /// availability matters, so this is the door every advertising caller uses.
    ///
    /// Availability is re-evaluated on every call, not cached: the prerequisites
    /// are process env vars and on-disk commands, both of which can change
    /// underneath a long-running gateway (RT-175's `/reload-skills` is the
    /// live example).
    pub fn list_available(&self) -> Vec<(String, String)> {
        // One row per skill, same reasoning as `list()`: the map is keyed by
        // every alias, so iterating values lists a nested skill once per
        // spelling. Here the cost is higher than a duplicated row -- this is
        // the ADVERTISED block, so a duplicate spends the token budget twice
        // on one skill and can push a real skill out under the ceiling.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for skill in self.skills.values() {
            if !self.is_available(skill) {
                continue;
            }
            if pairs.iter().any(|(name, _)| name == &skill.name) {
                continue;
            }
            pairs.push((skill.name.clone(), skill.description.clone()));
        }
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
    /// Refuses integration-sourced and project-sourced skills: the vendor tree
    /// is version-managed by `operant update --integrations` — deleting a
    /// skill from it would be silently undone by the next update anyway — and
    /// a project `skills/` belongs to the repository, so deleting from it is a
    /// change to someone else's tree. Delete the operator's shadowing copy or
    /// disable the integration instead.
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
        if let Some(skill) = self.skills.get(name)
            && skill.origin == SkillOrigin::Project
        {
            return Err(Error::Config(format!(
                "Skill '{}' comes from a project or configured root — delete is \
                 refused (that tree is version-controlled by someone else). Copy it \
                 into '{}' first if you want your own.",
                name,
                self.skills_dir.display()
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

    #[test]
    fn the_prompt_surface_advertises_only_usable_skills() {
        // Row 7 (codex-architecture-deepen): `is_available()` existed and worked,
        // and NOTHING called it. The frozen prompt's `<available_skills>` block
        // was built from the unfiltered `list()`, so the model was advertised
        // skills whose `prerequisites_env` was missing and whose
        // `prerequisites_commands` did not exist -- it would pick one, the tool
        // would refuse, and the turn degraded.
        //
        // The env var name is deliberately implausible so the test does not
        // depend on (or perturb) the real environment: nothing sets this, and
        // `which` cannot find this either.
        let tmp = make_temp_dir();
        for (name, prereq) in [
            (
                "needs-env",
                "prerequisites_env:\n  - OPERANT_TEST_ABSENT_VAR_XYZ\n",
            ),
            (
                "needs-command",
                "prerequisites_commands:\n  - operant_test_absent_cmd_xyz\n",
            ),
            ("plain", ""),
        ] {
            let dir = tmp.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} desc\n{prereq}---\nBody\n"),
            )
            .unwrap();
        }

        let mut mgr = SkillManager::new(tmp.clone());
        mgr.load_all().unwrap();

        // The INVENTORY surface stays unfiltered: all three are loaded.
        let all = mgr.list();
        assert_eq!(all.len(), 3, "list() must keep the full inventory");

        // The ADVERTISING surface filters: only the one with satisfiable
        // prerequisites is offered to a model.
        let advertised = mgr.list_available();
        assert_eq!(
            advertised.len(),
            1,
            "only the skill with no unmet prerequisites is advertised"
        );
        assert_eq!(advertised[0].0, "plain");

        // And the gap is exactly what this row closes: the old prompt path used
        // list(), so two of these three were being advertised.
        assert_ne!(
            all.len(),
            advertised.len(),
            "the filter must actually remove something, or this test proves nothing"
        );

        cleanup(&tmp);
    }

    #[test]
    fn reverting_the_advertised_list_to_the_inventory_list_fails_this_test() {
        // The mutation-guard bookend for row 7. If `list_available()` ever stops
        // filtering (or a caller goes back to `list()`), the count assertion
        // above goes from 1 to 3 and names the regression.
        let tmp = make_temp_dir();
        let dir = tmp.join("needs-env");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: needs-env\ndescription: d\nprerequisites_env:\n  - OPERANT_TEST_ABSENT_VAR_XYZ\n---\nBody\n",
        )
        .unwrap();

        let mut mgr = SkillManager::new(tmp.clone());
        mgr.load_all().unwrap();

        assert!(!mgr.is_available(mgr.get("needs-env").unwrap()));
        assert!(
            mgr.list_available().is_empty(),
            "a skill with an unmet prerequisite must not be advertised"
        );

        cleanup(&tmp);
    }

    // ── SkillRoots: one resolver, N locations (iter-2254) ───────────────

    fn settings_with(
        root: &Path,
        extra_roots: Vec<PathBuf>,
        project_root: &str,
    ) -> crate::config::SkillsSettings {
        crate::config::SkillsSettings {
            root_dir: root.to_path_buf(),
            extra_roots,
            project_root: project_root.to_string(),
            ..crate::config::SkillsSettings::default()
        }
    }

    fn skill_at(dir: &Path, name: &str) {
        let d = dir.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} desc\n---\nBody of {name}\n"),
        )
        .unwrap();
    }

    #[test]
    fn a_manual_location_resolves_for_every_surface() {
        // The operator's ask: a directory that is not `root_dir` has to be
        // reachable from the popup, the listing and the expansion alike.
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        let manual = tmp.join("manual");
        // `pool` is deliberately NOT created: only the manual root can resolve,
        // so the assertion cannot be perturbed by a real `~/.operant/skills`
        // on the host running the test.
        skill_at(&manual, "only-manual");

        let cfg = settings_with(&pool, vec![manual.clone()], "off");
        let roots = SkillRoots::resolve(&cfg, &[], None, None);
        assert_eq!(roots.roots.len(), 1, "only the manual root resolved");
        assert_eq!(roots.roots[0], fs::canonicalize(&manual).unwrap());

        let names: Vec<String> = crate::skill_mentions::loadable_skills_in(&roots)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["only-manual".to_string()]);

        cleanup(&tmp);
    }

    /// A category-nested skill is LISTABLE and RESOLVABLE by both spellings.
    ///
    /// This is the disagreement the whole plan was written for: `load_root`
    /// read immediate children while `find_skill_recursive` recursed, so six
    /// pool skills were FINDABLE by `$name` and INVISIBLE to `/skills`,
    /// `skills_list` and the popup. One walk fixed it; this pins that the fix
    /// covers the qualified spelling the listing shows.
    #[test]
    fn a_nested_skill_is_listable_and_resolvable_by_its_qualified_path() {
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        // `mlops/eval/deep` -- two category levels, the shape the old
        // one-level listing dropped.
        skill_at(&pool.join("mlops").join("eval"), "deep");

        let cfg = settings_with(&pool, vec![], "off");
        let roots = SkillRoots::resolve(&cfg, &[], None, None);

        // The listing is ONE row per skill, under the leaf name the operator
        // types. Before the walk was unified the listing read immediate
        // children only, so a nested skill was invisible here; and when the
        // qualified alias was added, iterating the map's keys listed the same
        // skill twice under its leaf and never under the path.
        let listed: Vec<String> = crate::skill_mentions::loadable_skills_in(&roots)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            listed,
            vec!["deep".to_string()],
            "the nested skill is not listed exactly once under its leaf: {listed:?}"
        );

        // The ADVERTISED list has the same aliasing hazard, and a higher
        // cost: a duplicate row spends the token budget twice on one skill and
        // can push a real skill out under the ceiling.
        let advertised: Vec<String> = {
            let mut mgr = SkillManager::from_roots(&roots);
            mgr.load_all().expect("load");
            mgr.list_available().into_iter().map(|(n, _)| n).collect()
        };
        assert_eq!(
            advertised,
            vec!["deep".to_string()],
            "the advertised block lists one skill once: {advertised:?}"
        );

        // And both spellings resolve to the same directory.
        let mut manager = SkillManager::from_roots(&roots);
        manager.load_all().expect("load");
        let by_leaf = manager.get("deep").map(|s| s.name.clone());
        let by_path = manager.get("mlops/eval/deep").map(|s| s.name.clone());
        assert!(by_leaf.is_some(), "the leaf name does not resolve");
        assert_eq!(
            by_leaf, by_path,
            "the leaf and the qualified path must resolve to the same skill"
        );

        cleanup(&tmp);
    }

    /// A skill whose DIRECTORY name differs from its frontmatter `name:` is
    /// resolvable by the name the listing advertises.
    ///
    /// The listing and the `$` popup both key on the frontmatter name, but
    /// `find_skill_recursive` matched only the directory name, so the name
    /// offered to the operator could not be invoked. Two real skills on the
    /// dev host had that shape (`mlops/training/fine-tuning-with-trl` carrying
    /// `name: trl-fine-tuning`; `skill-snapshot` carrying
    /// `name: harness-service-management`), and `operant skills inspect <name>`
    /// answered "not found" for both.
    ///
    /// MUTATION-PROOF: matching only the directory name in
    /// `find_skill_recursive` fails the resolution assertion below.
    #[test]
    fn a_skill_resolves_by_the_frontmatter_name_the_listing_advertises() {
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        let nested = pool
            .join("mlops")
            .join("training")
            .join("fine-tuning-with-trl");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: trl-fine-tuning\ndescription: TRL fine-tuning\n---\nBody\n",
        )
        .unwrap();

        let cfg = settings_with(&pool, vec![], "off");
        let roots = SkillRoots::resolve(&cfg, &[], None, None);

        // What the operator is shown...
        let listed: Vec<String> = crate::skill_mentions::loadable_skills_in(&roots)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            listed,
            vec!["trl-fine-tuning".to_string()],
            "the frontmatter name is what gets advertised: {listed:?}"
        );

        // ...must be exactly what resolves. Before the fix this returned None
        // and the expansion reported the name as unresolved.
        assert!(
            find_skill_in_roots(&roots.roots, "trl-fine-tuning").is_some(),
            "the advertised name must resolve"
        );
        // The directory name keeps working too -- nothing that worked before
        // may stop working.
        assert!(
            find_skill_in_roots(&roots.roots, "fine-tuning-with-trl").is_some(),
            "the directory name must still resolve"
        );

        cleanup(&tmp);
    }

    /// A shared DIRECTORY name across roots costs the alias, not the skill.
    ///
    /// The old rule skipped a skill when EITHER its name OR its directory alias
    /// was already taken, so a pool skill at `a/shared` named `alpha` made an
    /// integration skill at `b/shared` named `beta` disappear entirely: not
    /// listed, and not resolvable. Shadowing is by SKILL name; the directory
    /// is only an alias.
    ///
    /// MUTATION-PROOF: restoring `shadowed = name_taken || dir_key_taken` fails
    /// the listing assertion.
    #[test]
    fn a_shared_directory_name_across_roots_does_not_swallow_a_skill() {
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        let integ = tmp.join("integ");
        // Same LEAF directory name in both roots, different skill names.
        let a = pool.join("a").join("shared");
        fs::create_dir_all(&a).unwrap();
        fs::write(
            a.join("SKILL.md"),
            "---\nname: alpha\ndescription: pool skill\n---\nBody\n",
        )
        .unwrap();
        let b = integ.join("b").join("shared");
        fs::create_dir_all(&b).unwrap();
        fs::write(
            b.join("SKILL.md"),
            "---\nname: beta\ndescription: vendor skill\n---\nBody\n",
        )
        .unwrap();

        let cfg = settings_with(&pool, vec![integ.clone()], "off");
        let roots = SkillRoots::resolve(&cfg, &[], None, None);

        let listed: Vec<String> = crate::skill_mentions::loadable_skills_in(&roots)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            listed,
            vec!["alpha".to_string(), "beta".to_string()],
            "both skills must be listed; the second root's skill is not the first's alias: {listed:?}"
        );
        assert!(
            find_skill_in_roots(&roots.roots, "beta").is_some(),
            "the vendor skill must resolve by its own name"
        );

        cleanup(&tmp);
    }

    #[test]
    fn a_project_root_resolves_without_copying_into_the_pool() {
        // The jcode rule: a project pool is an OVERLAY, never a copy. The
        // seeder would otherwise copy 100 directories into the operator's home.
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        let ws = tmp.join("ws");
        fs::create_dir_all(&pool).unwrap();
        fs::create_dir_all(&ws).unwrap();
        // The repository's shape: <category>/<skill>/SKILL.md.
        let nested = ws.join("skills").join("testing").join("project-skill");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: project-skill\ndescription: lives in the repo\n---\nBody\n",
        )
        .unwrap();

        let cfg = settings_with(&pool, vec![], "auto");
        let roots = SkillRoots::resolve(&cfg, &[], Some(&ws), None);
        assert_eq!(roots.roots.len(), 2, "pool then project");
        assert_eq!(roots.project.as_deref(), Some(ws.join("skills").as_path()));

        let mut mgr = SkillManager::from_roots(&roots);
        mgr.load_all().unwrap();
        assert!(mgr.get("project-skill").is_some(), "leaf name resolves");
        assert!(
            mgr.get("testing/project-skill").is_some(),
            "the category-qualified path the listing shows resolves too"
        );
        // The pool is untouched: nothing was copied.
        assert!(!pool.join("project-skill").exists());
        assert_eq!(fs::read_dir(&pool).unwrap().count(), 0);

        cleanup(&tmp);
    }

    #[test]
    fn a_denied_root_is_refused_and_names_the_reason() {
        let tmp = make_temp_dir();
        // The deny-list is anchored to HOME (`is_sensitive_path_with_home`), so
        // the refused root has to be the REAL credential directory — a tmp dir
        // merely NAMED `.ssh` is an ordinary directory, and refusing it would be
        // wrong. It is not created: the deny check runs before the metadata
        // check, so its existence is irrelevant, and no test writes into `~/.ssh`.
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let Some(home) = home else {
            // No HOME means fail-closed: every path is denied, which is the
            // other half of the contract.
            return;
        };
        let denied = home.join(".ssh");
        let pool = tmp.join("pool");

        let cfg = settings_with(&pool, vec![denied.clone()], "off");
        let roots = SkillRoots::resolve(&cfg, &[], None, None);
        assert!(
            roots.roots.is_empty(),
            "neither a missing pool nor a denied root is a root"
        );
        let issue = roots
            .issues
            .iter()
            .find(|i| i.reason.starts_with("sensitive_path"))
            .expect("the denial must be reported, not swallowed");
        assert!(issue.describe().contains("credential directory .ssh"));

        cleanup(&tmp);
    }

    #[test]
    fn a_dead_symlink_is_reported_with_its_target() {
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        fs::create_dir_all(&pool).unwrap();
        skill_at(&pool, "live");
        let dead = pool.join("dead-link");
        let _ = fs::remove_file(&dead);
        std::os::unix::fs::symlink("/nonexistent/target", &dead).unwrap();

        let mut mgr = SkillManager::new(pool.clone());
        mgr.load_all().unwrap();
        assert!(mgr.get("live").is_some());
        assert!(mgr.get("dead-link").is_none(), "a dead link is not a skill");
        let issue = mgr
            .issues()
            .iter()
            .find(|i| i.reason == "dead_symlink")
            .expect("the dead link must be reported");
        assert_eq!(issue.entry, dead);
        assert_eq!(
            issue.link_target.as_deref(),
            Some(Path::new("/nonexistent/target"))
        );

        cleanup(&tmp);
    }

    #[test]
    fn the_walk_is_bounded_and_says_so() {
        let tmp = make_temp_dir();
        // One skill directory per level, nested far past any plausible depth.
        let mut dir = tmp.clone();
        for i in 0..50 {
            dir = dir.join(format!("level-{i}"));
            fs::create_dir_all(&dir).unwrap();
        }
        skill_at(&dir, "deep");

        let (found, _issues, truncated) = walk_skill_dirs(&tmp, 10);
        assert!(truncated, "a cap of 10 visits must truncate this tree");
        assert!(
            found.is_empty() || found.len() < 2,
            "a truncated walk must not report the deep skill as found"
        );

        cleanup(&tmp);
    }

    #[test]
    fn a_holon_run_resolves_to_the_brief_library_only() {
        let tmp = make_temp_dir();
        let pool = tmp.join("pool");
        let brief = tmp.join("brief-skills");
        let manual = tmp.join("manual");
        let ws = tmp.join("ws");
        for d in [&pool, &brief, &manual, &ws] {
            fs::create_dir_all(d).unwrap();
        }
        skill_at(&brief, "brief-only");

        let cfg = settings_with(&pool, vec![manual.clone()], "auto");
        let roots = SkillRoots::resolve(&cfg, &[], Some(&ws), Some(&brief));
        assert_eq!(roots.roots.len(), 1, "the brief's library only");
        assert_eq!(roots.roots[0], fs::canonicalize(&brief).unwrap());

        cleanup(&tmp);
    }
}

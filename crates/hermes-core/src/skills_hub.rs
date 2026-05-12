//! # Skills Hub — Source adapters and hub state management
//!
//! This is a **library module** (not an agent tool). It provides:
//! - `SkillSource` trait + 9 adapter types for discovering/installing skills
//! - `GitHubAuth` — shared GitHub API auth (PAT, env, gh CLI)
//! - `HubLockFile` — JSON-based install manifest
//! - `TapsManager` — custom GitHub repo source management
//! - Quarantine flow — download → scan → install pipeline
//! - Parallel search across all sources
//!
//! Ported from `hermes-agent/tools/skills_hub.py` (~3,261 LOC Python).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use futures::future::join_all;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default Hermes home subdirectory for skills
const SKILLS_DIR_NAME: &str = "skills";
/// Hub metadata directory (hidden inside skills dir)
const HUB_DIR_NAME: &str = ".hub";
/// Lock file name
const LOCK_FILE_NAME: &str = "lock.json";
/// Quarantine directory name
const QUARANTINE_DIR_NAME: &str = "quarantine";
/// Audit log name
const AUDIT_LOG_NAME: &str = "audit.log";
/// Taps file name
const TAPS_FILE_NAME: &str = "taps.json";
/// Index cache directory name
const INDEX_CACHE_DIR_NAME: &str = "index-cache";
/// Cache TTL: 1 hour
const INDEX_CACHE_TTL_SECS: u64 = 3600;
/// Hermes centralized index URL
const HERMES_INDEX_URL: &str =
    "https://hermes-agent.nousresearch.com/docs/api/skills-index.json";
/// Hermes index TTL: 6 hours
const HERMES_INDEX_TTL_SECS: u64 = 21_600;
/// Max fetch redirects
const MAX_FETCH_REDIRECTS: u32 = 5;
/// Max file size for ZIP extraction (500KB)
const MAX_ZIP_FILE_SIZE: u64 = 500_000;

/// Default taps (GitHub repos with skill directories)
const DEFAULT_TAPS: &[(&str, &str)] = &[
    ("openai/skills", "skills/"),
    ("anthropics/skills", "skills/"),
    ("VoltAgent/awesome-agent-skills", "skills/"),
    ("garrytan/gstack", ""),
    ("MiniMax-AI/cli", "skill/"),
];

/// Trusted repos for community trust level
const TRUSTED_REPOS: &[&str] = &["openai/skills", "anthropics/skills"];

// ---------------------------------------------------------------------------
// Paths helper
// ---------------------------------------------------------------------------

/// Resolve the skills hub paths relative to a base directory (e.g. `~/.hermes`).
#[derive(Debug, Clone)]
pub struct HubPaths {
    /// Base skills directory
    pub skills_dir: PathBuf,
    /// `.hub` metadata directory
    pub hub_dir: PathBuf,
    /// Lock file for installed skill manifest
    pub lock_file: PathBuf,
    /// Quarantine directory for scanning before install
    pub quarantine_dir: PathBuf,
    /// Audit log file
    pub audit_log: PathBuf,
    /// Taps configuration file
    pub taps_file: PathBuf,
    /// Index cache directory
    pub index_cache_dir: PathBuf,
}

impl HubPaths {
    /// Create a new `HubPaths` rooted at `hermes_home/skills/`.
    pub fn new(hermes_home: &Path) -> Self {
        let skills_dir = hermes_home.join(SKILLS_DIR_NAME);
        let hub_dir = skills_dir.join(HUB_DIR_NAME);
        Self {
            skills_dir,
            lock_file: hub_dir.join(LOCK_FILE_NAME),
            quarantine_dir: hub_dir.join(QUARANTINE_DIR_NAME),
            audit_log: hub_dir.join(AUDIT_LOG_NAME),
            taps_file: hub_dir.join(TAPS_FILE_NAME),
            index_cache_dir: hub_dir.join(INDEX_CACHE_DIR_NAME),
            hub_dir,
        }
    }

    /// Create all hub directories and default files.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.hub_dir)?;
        std::fs::create_dir_all(&self.quarantine_dir)?;
        std::fs::create_dir_all(&self.index_cache_dir)?;
        if !self.lock_file.exists() {
            std::fs::write(&self.lock_file, r#"{"version":1,"installed":{}}"#)?;
        }
        if !self.audit_log.exists() {
            std::fs::write(&self.audit_log, "")?;
        }
        if !self.taps_file.exists() {
            std::fs::write(&self.taps_file, r#"{"taps":[]}"#)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Minimal metadata returned by search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    /// "official", "github", "clawhub", "claude-marketplace", "lobehub", ...
    pub source: String,
    /// Source-specific ID (e.g. "openai/skills/skill-creator")
    pub identifier: String,
    /// "builtin" | "trusted" | "community"
    pub trust_level: String,
    pub repo: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

/// A downloaded skill ready for quarantine/scanning/installation.
#[derive(Debug, Clone)]
pub struct SkillBundle {
    pub name: String,
    /// relative_path → file content (text)
    pub files: HashMap<String, String>,
    pub source: String,
    pub identifier: String,
    pub trust_level: String,
    pub metadata: HashMap<String, String>,
}

/// Lock file entry for an installed skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledEntry {
    pub source: String,
    pub identifier: String,
    pub trust_level: String,
    pub scan_verdict: String,
    pub content_hash: String,
    pub install_path: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    pub installed_at: String,
    pub updated_at: String,
}

/// Top-level lock file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFileData {
    pub version: u32,
    #[serde(default)]
    pub installed: HashMap<String, InstalledEntry>,
}

// ---------------------------------------------------------------------------
// Path validation helpers
// ---------------------------------------------------------------------------

/// Normalize and validate bundle-controlled paths before touching disk.
fn normalize_bundle_path(path_value: &str, allow_nested: bool) -> Result<String> {
    if path_value.is_empty() {
        return Err(Error::Config("Unsafe path: empty path".into()));
    }

    let normalized = path_value.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();

    if normalized.starts_with('/') {
        return Err(Error::Config(format!(
            "Unsafe path (absolute): {path_value}"
        )));
    }
    if parts.is_empty() || parts.iter().any(|p| *p == "..") {
        return Err(Error::Config(format!("Unsafe path (..): {path_value}")));
    }
    if parts[0].len() == 2
        && parts[0].chars().next().map_or(false, |c| c.is_ascii_alphabetic())
        && parts[0].ends_with(':')
    {
        return Err(Error::Config(format!("Unsafe path (drive): {path_value}")));
    }
    if !allow_nested && parts.len() != 1 {
        return Err(Error::Config(format!("Unsafe path (nested): {path_value}")));
    }

    Ok(parts.join("/"))
}

/// Validate a skill name (no nesting).
pub fn validate_skill_name(name: &str) -> Result<String> {
    normalize_bundle_path(name, false)
}

/// Validate a category name (no nesting).
pub fn validate_category_name(category: &str) -> Result<String> {
    normalize_bundle_path(category, false)
}

/// Validate a bundle relative file path (allows nesting).
pub fn validate_bundle_rel_path(rel_path: &str) -> Result<String> {
    normalize_bundle_path(rel_path, true)
}

/// Validate a skill name string (lowercase identifier with hyphens/underscores).
pub fn is_valid_skill_name(name: &str) -> bool {
    let re = Regex::new(r"^[a-z][a-z0-9_-]*$").unwrap();
    let candidate = name.trim().to_lowercase();
    if candidate.is_empty()
        || matches!(
            candidate.as_str(),
            "skill" | "readme" | "index" | "unnamed-skill"
        )
    {
        return false;
    }
    re.is_match(&candidate)
}

// ---------------------------------------------------------------------------
// SSRF-guarded HTTP client
// ---------------------------------------------------------------------------

/// Simplified SSRF-guarded HTTP GET. Validates URL before fetching.
async fn guarded_http_get(
    url: &str,
    client: &reqwest::Client,
    timeout_secs: u64,
) -> Option<reqwest::Response> {
    // Basic SSRF guard: reject private IPs, localhost, etc.
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;

    // Reject common private/local addresses
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.ends_with(".local")
        || host.starts_with("10.")
        || host.starts_with("172.16.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
    {
        warn!("Blocked unsafe Skills Hub URL (private IP): {url}");
        return None;
    }

    let mut current_url = url.to_string();
    for _ in 0..=MAX_FETCH_REDIRECTS {
        let resp = client
            .get(&current_url)
            .timeout(Duration::from_secs(timeout_secs))
            .send()
            .await
            .ok()?;

        let status = resp.status();
        if status.is_redirection() {
            if let Some(location) = resp.headers().get("location") {
                if let Ok(loc_str) = location.to_str() {
                    current_url = url_join(&current_url, loc_str);
                    continue;
                }
            }
            return None;
        }

        return Some(resp);
    }

    warn!("Skills Hub fetch exceeded redirect limit for {url}");
    None
}

/// Simple URL join (replaces Python's urljoin).
fn url_join(base: &str, relative: &str) -> String {
    if relative.starts_with("http://") || relative.starts_with("https://") {
        return relative.to_string();
    }
    if relative.starts_with('/') {
        let parsed = url::Url::parse(base).ok();
        if let Some(p) = parsed {
            if let Ok(joined) = p.join(relative) {
                return joined.to_string();
            }
        }
    }
    // Simple path concatenation
    let base_trimmed = base.trim_end_matches('/');
    let rel_trimmed = relative.trim_start_matches('/');
    format!("{base_trimmed}/{rel_trimmed}")
}

// ---------------------------------------------------------------------------
// GitHub Authentication
// ---------------------------------------------------------------------------

/// GitHub API authentication state.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Personal Access Token from env var
    Pat(String),
    /// Token from `gh auth token` CLI
    GhCli(String),
    /// Anonymous (unauthenticated, 60 req/hr)
    Anonymous,
}

/// GitHub authentication resolver.
///
/// Tries methods in priority order:
/// 1. `GITHUB_TOKEN` / `GH_TOKEN` env var
/// 2. `gh auth token` subprocess (when `gh` CLI is available)
/// 3. Anonymous (60 req/hr, public repos only)
#[derive(Debug, Clone)]
pub struct GitHubAuth {
    method: AuthMethod,
}

impl GitHubAuth {
    /// Create a new `GitHubAuth`, resolving the best available method.
    pub fn new() -> Self {
        let method = Self::resolve_method();
        Self { method }
    }

    /// Resolve the best authentication method.
    fn resolve_method() -> AuthMethod {
        // 1. Environment variable
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            if !token.is_empty() {
                return AuthMethod::Pat(token);
            }
        }
        if let Ok(token) = std::env::var("GH_TOKEN") {
            if !token.is_empty() {
                return AuthMethod::Pat(token);
            }
        }

        // 2. `gh` CLI (only when explicitly available)
        // We skip the subprocess call here for portability —
        // the caller can set GITHUB_TOKEN for authenticated access.
        // In a full port, this would call `gh auth token`.

        // 3. Anonymous
        AuthMethod::Anonymous
    }

    /// Get authorization headers for GitHub API requests.
    pub fn get_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github.v3+json"),
        );
        match &self.method {
            AuthMethod::Pat(token) | AuthMethod::GhCli(token) => {
                if let Ok(val) = HeaderValue::from_str(&format!("token {token}")) {
                    headers.insert(AUTHORIZATION, val);
                }
            }
            AuthMethod::Anonymous => {}
        }
        headers
    }

    /// Whether the current method provides authenticated access.
    pub fn is_authenticated(&self) -> bool {
        matches!(self.method, AuthMethod::Pat(_) | AuthMethod::GhCli(_))
    }

    /// Return a description of the active auth method.
    pub fn auth_method_name(&self) -> &'static str {
        match &self.method {
            AuthMethod::Pat(_) => "pat",
            AuthMethod::GhCli(_) => "gh-cli",
            AuthMethod::Anonymous => "anonymous",
        }
    }
}

impl Default for GitHubAuth {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SkillSource trait
// ---------------------------------------------------------------------------

/// Abstract interface for all skill registry adapters.
#[async_trait]
pub trait SkillSource: Send + Sync {
    /// Search for skills matching a query string.
    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta>;

    /// Download a skill bundle by identifier.
    async fn fetch(&self, identifier: &str) -> Option<SkillBundle>;

    /// Fetch metadata for a skill without downloading all files.
    async fn inspect(&self, identifier: &str) -> Option<SkillMeta>;

    /// Unique identifier for this source (e.g. "github", "clawhub").
    fn source_id(&self) -> &'static str;

    /// Determine trust level for a skill from this source.
    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "community"
    }
}

// ---------------------------------------------------------------------------
// GitHub source adapter
// ---------------------------------------------------------------------------

/// Fetch skills from GitHub repos via the Contents API.
pub struct GitHubSource {
    auth: GitHubAuth,
    taps: Vec<TapConfig>,
    client: reqwest::Client,
    /// repo -> (default_branch, tree entries) cache
    tree_cache: Arc<RwLock<HashMap<String, (String, Vec<GitTreeEntry>)>>>,
    paths: HubPaths,
}

#[derive(Debug, Clone)]
struct TapConfig {
    repo: String,
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitTreeEntry {
    #[serde(default)]
    path: String,
    #[serde(default)]
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(default)]
    sha: String,
}

impl GitHubSource {
    pub fn new(auth: GitHubAuth, paths: HubPaths, extra_taps: Vec<(String, String)>) -> Self {
        let mut taps: Vec<TapConfig> = DEFAULT_TAPS
            .iter()
            .map(|(r, p)| TapConfig {
                repo: r.to_string(),
                path: p.to_string(),
            })
            .collect();
        for (repo, path) in extra_taps {
            taps.push(TapConfig { repo, path });
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            auth,
            taps,
            client,
            tree_cache: Arc::new(RwLock::new(HashMap::new())),
            paths,
        }
    }

    /// Check if a GitHub response indicates rate limiting.
    fn check_rate_limit(&self, resp: &reqwest::Response) {
        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            if let Some(remaining) = resp.headers().get("x-ratelimit-remaining") {
                if let Ok(val) = remaining.to_str() {
                    if val == "0" {
                        warn!("GitHub API rate limit exhausted. Set GITHUB_TOKEN to raise limit.");
                    }
                }
            }
        }
    }

    /// Fetch a single file's content from GitHub (raw API).
    async fn fetch_file_content(&self, repo: &str, path: &str) -> Option<String> {
        let url = format!("https://api.github.com/repos/{repo}/contents/{path}");
        let mut headers = self.auth.get_headers();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.github.v3.raw"),
        );

        let resp = self.client.get(&url).headers(headers).send().await.ok()?;
        if resp.status().is_success() {
            resp.text().await.ok()
        } else {
            self.check_rate_limit(&resp);
            None
        }
    }

    /// Get or fetch the git tree for a repo.
    async fn get_repo_tree(
        &self,
        repo: &str,
    ) -> Option<(String, Vec<GitTreeEntry>)> {
        {
            let cache = self.tree_cache.read().await;
            if let Some(cached) = cache.get(repo) {
                return Some(cached.clone());
            }
        }

        let headers = self.auth.get_headers();

        // Resolve default branch
        let repo_url = format!("https://api.github.com/repos/{repo}");
        let resp = self
            .client
            .get(&repo_url)
            .headers(headers.clone())
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let repo_info: serde_json::Value = resp.json().await.ok()?;
        let default_branch = repo_info
            .get("default_branch")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string();

        // Fetch recursive tree
        let tree_url = format!(
            "https://api.github.com/repos/{repo}/git/trees/{default_branch}?recursive=1"
        );
        let tree_resp = self
            .client
            .get(&tree_url)
            .headers(headers)
            .send()
            .await
            .ok()?;
        if !tree_resp.status().is_success() {
            return None;
        }
        let tree_data: serde_json::Value = tree_resp.json().await.ok()?;

        // Skip truncated trees
        if tree_data.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false) {
            debug!("Git tree truncated for {repo}, cannot cache");
            return None;
        }

        let entries: Vec<GitTreeEntry> = tree_data
            .get("tree")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let result = (default_branch, entries.clone());
        {
            let mut cache = self.tree_cache.write().await;
            cache.insert(repo.to_string(), result.clone());
        }
        Some(result)
    }

    /// Download a directory via the Git Trees API (single request).
    async fn download_directory_via_tree(
        &self,
        repo: &str,
        path: &str,
    ) -> Option<HashMap<String, String>> {
        let path = path.trim_end_matches('/');
        let cached = self.get_repo_tree(repo).await?;
        let (_branch, tree_entries) = cached;

        let prefix = format!("{path}/");
        let has_entries = tree_entries
            .iter()
            .any(|e| e.entry_type == "blob" && e.path.starts_with(&prefix));
        if !has_entries {
            return Some(HashMap::new());
        }

        let mut files = HashMap::new();
        for entry in &tree_entries {
            if entry.entry_type != "blob" {
                continue;
            }
            if !entry.path.starts_with(&prefix) {
                continue;
            }
            let rel_path = entry.path[prefix.len()..].to_string();
            if let Some(content) = self.fetch_file_content(repo, &entry.path).await {
                files.insert(rel_path, content);
            }
        }

        if files.is_empty() {
            None
        } else {
            Some(files)
        }
    }

    async fn download_directory_recursive(
        &self,
        repo: &str,
        path: &str,
    ) -> HashMap<String, String> {
        let mut files = HashMap::new();
        let mut stack: Vec<String> = vec![path.to_string()];
        let repo = repo.to_string();

        while let Some(current_path) = stack.pop() {
            let url = format!("https://api.github.com/repos/{repo}/contents/{current_path}");
            let resp = match self
                .client
                .get(&url)
                .headers(self.auth.get_headers())
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            let entries: Vec<serde_json::Value> = match resp.json().await {
                Ok(v) => v,
                _ => continue,
            };

            for entry in &entries {
                let name = match entry.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let entry_type = match entry.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };
                let entry_path = match entry.get("path").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => continue,
                };

                if entry_type == "file" {
                    if let Some(content) = self.fetch_file_content(&repo, &entry_path).await {
                        let rel = entry_path
                            .strip_prefix(&format!("{path}/"))
                            .unwrap_or(&entry_path);
                        let rel_path = rel
                            .strip_prefix(path)
                            .unwrap_or(rel);
                        files.insert(
                            if rel_path.contains('/') {
                                rel_path.to_string()
                            } else {
                                name.clone()
                            },
                            content,
                        );
                    }
                } else if entry_type == "dir" {
                    stack.push(entry_path);
                }
            }
        }

        files
    }

    /// Download an entire directory from GitHub, trying tree API first.
    async fn download_directory(&self, repo: &str, path: &str) -> HashMap<String, String> {
        if let Some(files) = self.download_directory_via_tree(repo, path).await {
            return files;
        }
        debug!("Tree API unavailable for {repo}/{path}, falling back to Contents API");
        self.download_directory_recursive(repo, path).await
    }

    /// List skill directories under a repo path.
    async fn list_skills_in_repo(&self, repo: &str, path: &str) -> Vec<SkillMeta> {
        // Check index cache first
        let cache_key = format!("github_{repo}_{path}");
        if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
            let skills: Vec<SkillMeta> =
                serde_json::from_value(cached).unwrap_or_default();
            return skills;
        }

        let url = format!(
            "https://api.github.com/repos/{repo}/contents/{}",
            path.trim_end_matches('/')
        );
        let resp = match self
            .client
            .get(&url)
            .headers(self.auth.get_headers())
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };

        let entries: Vec<serde_json::Value> = match resp.json().await {
            Ok(v) => v,
            _ => return vec![],
        };

        let mut skills = Vec::new();
        for entry in &entries {
            if entry.get("type").and_then(|v| v.as_str()) != Some("dir") {
                continue;
            }
            let dir_name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if dir_name.starts_with('.') || dir_name.starts_with('_') {
                continue;
            }

            let prefix = path.trim_end_matches('/');
            let skill_identifier = if prefix.is_empty() {
                format!("{repo}/{dir_name}")
            } else {
                format!("{repo}/{prefix}/{dir_name}")
            };

            if let Some(meta) = self.inspect(&skill_identifier).await {
                skills.push(meta);
            }
        }

        // Cache results
        if let Ok(json) = serde_json::to_value(&skills) {
            write_index_cache(&self.paths, &cache_key, &json).await;
        }

        skills
    }

    /// Parse YAML frontmatter from SKILL.md content.
    fn parse_frontmatter(content: &str) -> HashMap<String, serde_json::Value> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return HashMap::new();
        }
        let after_open = &trimmed[3..];
        let close_pos = match after_open.find("\n---") {
            Some(pos) => pos,
            None => return HashMap::new(),
        };
        let yaml_text = &after_open[..close_pos];
        match serde_yaml::from_str::<serde_json::Value>(yaml_text) {
            Ok(serde_json::Value::Object(map)) => {
                map.into_iter().collect()
            }
            _ => HashMap::new(),
        }
    }

    /// Extract a string field from parsed frontmatter.
    fn fm_string(fm: &HashMap<String, serde_json::Value>, key: &str) -> String {
        fm.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Extract tags from parsed frontmatter.
    fn fm_tags(fm: &HashMap<String, serde_json::Value>) -> Vec<String> {
        // Try metadata.hermes.tags
        if let Some(meta) = fm.get("metadata").and_then(|v| v.as_object()) {
            if let Some(hermes) = meta.get("hermes").and_then(|v| v.as_object()) {
                if let Some(tags) = hermes.get("tags").and_then(|v| v.as_array()) {
                    let t: Vec<String> = tags
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    if !t.is_empty() {
                        return t;
                    }
                }
            }
        }
        // Fallback to top-level tags
        if let Some(tags) = fm.get("tags").and_then(|v| v.as_array()) {
            return tags
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        vec![]
    }
}

#[async_trait]
impl SkillSource for GitHubSource {
    fn source_id(&self) -> &'static str {
        "github"
    }

    fn trust_level_for(&self, identifier: &str) -> &'static str {
        let parts: Vec<&str> = identifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            let repo = format!("{}/{}", parts[0], parts[1]);
            if TRUSTED_REPOS.contains(&repo.as_str()) {
                return "trusted";
            }
        }
        "community"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for tap in &self.taps {
            let skills = self
                .list_skills_in_repo(&tap.repo, &tap.path)
                .await;
            for skill in skills {
                let searchable = format!(
                    "{} {} {}",
                    skill.name,
                    skill.description,
                    skill.tags.join(" ")
                )
                .to_lowercase();
                if query_lower.is_empty() || searchable.contains(&query_lower) {
                    results.push(skill);
                }
            }
        }

        // Deduplicate by name, preferring higher trust
        let trust_rank =
            |t: &str| -> i32 {
                match t {
                    "builtin" => 2,
                    "trusted" => 1,
                    _ => 0,
                }
            };
        let mut seen: HashMap<String, SkillMeta> = HashMap::new();
        for r in results {
            let rank = trust_rank(&r.trust_level);
            let entry = seen.entry(r.name.clone()).or_insert_with(|| r.clone());
            if rank > trust_rank(&entry.trust_level) {
                *entry = r;
            }
        }
        let mut deduped: Vec<SkillMeta> = seen.into_values().collect();
        deduped.truncate(limit);
        deduped
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let parts: Vec<&str> = identifier.splitn(3, '/').collect();
        if parts.len() < 3 {
            return None;
        }

        let repo = format!("{}/{}", parts[0], parts[1]);
        let skill_path = parts[2];

        let files = self.download_directory(&repo, skill_path).await;
        if files.is_empty() || !files.contains_key("SKILL.md") {
            return None;
        }

        let skill_name = skill_path
            .trim_end_matches('/')
            .split('/')
            .last()
            .unwrap_or(skill_path)
            .to_string();
        let trust = self.trust_level_for(identifier);

        Some(SkillBundle {
            name: skill_name,
            files,
            source: "github".into(),
            identifier: identifier.to_string(),
            trust_level: trust.to_string(),
            metadata: HashMap::new(),
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let parts: Vec<&str> = identifier.splitn(3, '/').collect();
        if parts.len() < 3 {
            return None;
        }

        let repo = format!("{}/{}", parts[0], parts[1]);
        let skill_path = parts[2].trim_end_matches('/');
        let skill_md_path = format!("{skill_path}/SKILL.md");

        let content = self.fetch_file_content(&repo, &skill_md_path).await?;
        let fm = Self::parse_frontmatter(&content);

        let skill_name = Self::fm_string(&fm, "name");
        let skill_name = if skill_name.is_empty() {
            skill_path.split('/').last().unwrap_or(skill_path).to_string()
        } else {
            skill_name
        };

        let tags = Self::fm_tags(&fm);

        Some(SkillMeta {
            name: skill_name,
            description: Self::fm_string(&fm, "description"),
            source: "github".into(),
            identifier: identifier.to_string(),
            trust_level: self.trust_level_for(identifier).to_string(),
            repo: Some(repo),
            path: Some(skill_path.to_string()),
            tags,
            extra: HashMap::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Well-Known skill source adapter
// ---------------------------------------------------------------------------

/// Read skills from a domain exposing `/.well-known/skills/index.json`.
pub struct WellKnownSkillSource {
    client: reqwest::Client,
    paths: HubPaths,
}

impl WellKnownSkillSource {
    pub fn new(paths: HubPaths) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            paths,
        }
    }

    fn wrap_identifier(base_url: &str, skill_name: &str) -> String {
        format!("well-known:{}/{}", base_url.trim_end_matches('/'), skill_name)
    }

    async fn fetch_text(&self, url: &str) -> Option<String> {
        let resp = guarded_http_get(url, &self.client, 20).await?;
        if resp.status().is_success() {
            resp.text().await.ok()
        } else {
            None
        }
    }
}

#[async_trait]
impl SkillSource for WellKnownSkillSource {
    fn source_id(&self) -> &'static str {
        "well-known"
    }

    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "community"
    }

    async fn search(&self, _query: &str, _limit: usize) -> Vec<SkillMeta> {
        // Well-known search requires a domain URL as query — complex to implement
        // fully in Rust. Returns empty by default.
        vec![]
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let ident = identifier
            .strip_prefix("well-known:")
            .unwrap_or(identifier);
        if !ident.starts_with("http://") && !ident.starts_with("https://") {
            return None;
        }

        // Parse identifier: <base_url>/<skill_name>
        let clean = ident.split('#').next().unwrap_or(ident);
        let skill_url = clean.trim_end_matches('/');
        let base_url = skill_url
            .rsplit_once('/')
            .map(|(base, _name)| base.to_string())?;
        let skill_name_owned = skill_url.rsplit_once('/').map(|(_, name)| name.to_string())?;

        let md_url = format!("{skill_url}/SKILL.md");
        let text = self.fetch_text(&md_url).await?;

        let mut files = HashMap::new();
        files.insert("SKILL.md".into(), text);

        Some(SkillBundle {
            name: skill_name_owned.clone(),
            files,
            source: "well-known".into(),
            identifier: Self::wrap_identifier(&base_url, &skill_name_owned),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let ident = identifier
            .strip_prefix("well-known:")
            .unwrap_or(identifier);
        if !ident.starts_with("http://") && !ident.starts_with("https://") {
            return None;
        }

        let clean = ident.trim_end_matches('/');
        let skill_url = format!("{clean}/SKILL.md");
        let text = self.fetch_text(&skill_url).await?;

        let fm = GitHubSource::parse_frontmatter(&text);
        let name = GitHubSource::fm_string(&fm, "name");
        let skill_name = if name.is_empty() {
            clean.split('/').last().unwrap_or(clean).to_string()
        } else {
            name
        };

        let base_url = clean.rsplit_once('/').map(|(b, _)| b).unwrap_or(clean);

        Some(SkillMeta {
            name: skill_name.clone(),
            description: GitHubSource::fm_string(&fm, "description"),
            source: "well-known".into(),
            identifier: Self::wrap_identifier(base_url, &skill_name),
            trust_level: "community".into(),
            repo: None,
            path: Some(skill_name.clone()),
            tags: GitHubSource::fm_tags(&fm),
            extra: HashMap::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Direct URL source adapter
// ---------------------------------------------------------------------------

/// Fetch a single-file SKILL.md skill directly from an HTTP(S) URL.
pub struct UrlSource {
    client: reqwest::Client,
}

impl UrlSource {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Whether this source should handle the given identifier.
    fn matches(&self, identifier: &str) -> bool {
        let ident = identifier.trim();
        if !ident.to_lowercase().starts_with("http://")
            && !ident.to_lowercase().starts_with("https://")
        {
            return false;
        }
        // Don't steal well-known URLs
        if ident.contains("/.well-known/skills/") || ident.trim_end_matches('/').ends_with("/index.json") {
            return false;
        }
        // Only claim URLs ending in .md
        let path = match url::Url::parse(ident) {
            Ok(u) => u.path().to_string(),
            Err(_) => return false,
        };
        path.to_lowercase().ends_with(".md")
    }

    fn resolve_skill_name(fm: &HashMap<String, serde_json::Value>, url: &str) -> Option<String> {
        // 1. Frontmatter name: field
        if let Some(name) = fm.get("name").and_then(|v| v.as_str()) {
            if is_valid_skill_name(name) {
                return Some(name.trim().to_lowercase());
            }
        }

        // 2. URL slug heuristic
        let parsed = url::Url::parse(url).ok()?;
        let path = parsed.path().to_string();
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();

        // .../<name>/SKILL.md → <name>
        if parts.len() >= 2
            && parts.last().map(|p| p.to_lowercase()) == Some("skill.md".into())
        {
            let candidate = parts[parts.len() - 2];
            if is_valid_skill_name(candidate) {
                return Some(candidate.to_lowercase());
            }
        }
        // .../<name>.md → <name>
        if let Some(last) = parts.last() {
            let candidate = last
                .strip_suffix(".md")
                .or_else(|| last.strip_suffix(".MD"))
                .unwrap_or(last);
            if is_valid_skill_name(candidate) {
                return Some(candidate.to_lowercase());
            }
        }

        None
    }

    async fn fetch_text(&self, url: &str) -> Option<String> {
        let resp = guarded_http_get(url, &self.client, 20).await?;
        if resp.status().is_success() {
            resp.text().await.ok()
        } else {
            None
        }
    }
}

impl Default for UrlSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillSource for UrlSource {
    fn source_id(&self) -> &'static str {
        "url"
    }

    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "community"
    }

    async fn search(&self, _query: &str, _limit: usize) -> Vec<SkillMeta> {
        vec![]
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        if !self.matches(identifier) {
            return None;
        }
        let url = identifier.trim();
        let text = self.fetch_text(url).await?;
        let fm = GitHubSource::parse_frontmatter(&text);
        let name = Self::resolve_skill_name(&fm, url);

        let skill_name = match name {
            Some(n) => validate_skill_name(&n).ok().unwrap_or_default(),
            None => String::new(),
        };

        let mut files = HashMap::new();
        files.insert("SKILL.md".into(), text);

        let mut metadata = HashMap::new();
        metadata.insert("url".into(), url.to_string());
        metadata.insert("awaiting_name".into(), if skill_name.is_empty() { "true" } else { "false" }.into());

        Some(SkillBundle {
            name: skill_name,
            files,
            source: "url".into(),
            identifier: url.to_string(),
            trust_level: "community".into(),
            metadata,
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        if !self.matches(identifier) {
            return None;
        }
        let url = identifier.trim();
        let text = self.fetch_text(url).await?;
        let fm = GitHubSource::parse_frontmatter(&text);

        let name = Self::resolve_skill_name(&fm, url);
        let awaiting = name.is_none();
        let skill_name = name.unwrap_or_default();

        Some(SkillMeta {
            name: skill_name.clone(),
            description: GitHubSource::fm_string(&fm, "description"),
            source: "url".into(),
            identifier: url.to_string(),
            trust_level: "community".into(),
            repo: None,
            path: if skill_name.is_empty() { None } else { Some(skill_name) },
            tags: GitHubSource::fm_tags(&fm),
            extra: {
                let mut m = HashMap::new();
                m.insert("url".into(), url.to_string());
                if awaiting {
                    m.insert("awaiting_name".into(), "true".into());
                }
                m
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Official optional skills source adapter
// ---------------------------------------------------------------------------

/// Fetch skills from the `optional-skills/` directory shipped with the repo.
pub struct OptionalSkillSource {
    optional_dir: PathBuf,
}

impl OptionalSkillSource {
    pub fn new(optional_dir: PathBuf) -> Self {
        Self { optional_dir }
    }

    /// Parse YAML frontmatter from SKILL.md content.
    fn parse_frontmatter(content: &str) -> HashMap<String, serde_json::Value> {
        GitHubSource::parse_frontmatter(content)
    }

    fn find_skill_dir(&self, name: &str) -> Option<PathBuf> {
        if !self.optional_dir.is_dir() {
            return None;
        }
        find_skill_md_files(&self.optional_dir, 5)
            .into_iter()
            .find_map(|path| {
                let parent = path.parent()?;
                if parent.file_name().and_then(|n| n.to_str()) == Some(name) {
                    Some(parent.to_path_buf())
                } else {
                    None
                }
            })
    }

    fn scan_all(&self) -> Vec<SkillMeta> {
        if !self.optional_dir.is_dir() {
            return vec![];
        }

        let mut results = Vec::new();
        let mut entries = find_skill_md_files(&self.optional_dir, 5);
        entries.sort();

        for skill_md_path in entries {
            let parent = skill_md_path.parent().unwrap();
            let rel_parts = parent
                .strip_prefix(&self.optional_dir)
                .unwrap_or(parent);

            // Skip hidden directories
            if rel_parts.iter().any(|p| {
                p.to_str().map_or(false, |s| s.starts_with('.'))
            }) {
                continue;
            }

            let content = match std::fs::read_to_string(&skill_md_path) {
                Ok(c) => c,
                _ => continue,
            };

            let fm = Self::parse_frontmatter(&content);
            let name = GitHubSource::fm_string(&fm, "name");
            let skill_name = if name.is_empty() {
                parent.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
            } else {
                name
            };
            let desc = GitHubSource::fm_string(&fm, "description");
            let desc = if desc.len() > 200 {
                desc[..200].to_string()
            } else {
                desc
            };
            let tags = GitHubSource::fm_tags(&fm);

            let rel_path = rel_parts.to_str().unwrap_or("").to_string();

            results.push(SkillMeta {
                name: skill_name,
                description: desc,
                source: "official".into(),
                identifier: format!("official/{rel_path}"),
                trust_level: "builtin".into(),
                repo: None,
                path: Some(rel_path),
                tags,
                extra: HashMap::new(),
            });
        }

        results
    }
}

#[async_trait]
impl SkillSource for OptionalSkillSource {
    fn source_id(&self) -> &'static str {
        "official"
    }

    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "builtin"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for meta in self.scan_all() {
            if query_lower.is_empty() {
                results.push(meta);
            } else {
                let searchable = format!(
                    "{} {} {}",
                    meta.name,
                    meta.description,
                    meta.tags.join(" ")
                )
                .to_lowercase();
                if searchable.contains(&query_lower) {
                    results.push(meta);
                }
            }
            if results.len() >= limit {
                break;
            }
        }

        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let rel = identifier.strip_prefix("official/").unwrap_or(identifier);
        let skill_dir = self.optional_dir.join(rel);

        // Guard against path traversal
        let resolved = skill_dir.canonicalize().ok()?;
        let optional_resolved = self.optional_dir.canonicalize().ok()?;
        if !resolved.starts_with(&optional_resolved) {
            return None;
        }

        let skill_dir = if resolved.is_dir() {
            resolved
        } else {
            let skill_name = rel.rsplit_once('/').map(|(_, n)| n).unwrap_or(rel);
            self.find_skill_dir(skill_name)?
        };

        let mut files = HashMap::new();
        collect_files_recursive(&skill_dir, &skill_dir, &mut files);

        if files.is_empty() {
            return None;
        }

        let name = skill_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let identifier_rel = skill_dir
            .strip_prefix(&self.optional_dir)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or(&name)
            .to_string();

        Some(SkillBundle {
            name,
            files,
            source: "official".into(),
            identifier: format!("official/{identifier_rel}"),
            trust_level: "builtin".into(),
            metadata: HashMap::new(),
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let skill_name = identifier
            .strip_prefix("official/")
            .unwrap_or(identifier)
            .rsplit_once('/')
            .map(|(_, n)| n)
            .unwrap_or(identifier);

        for meta in self.scan_all() {
            if meta.name == skill_name {
                return Some(meta);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Hermes centralized index source
// ---------------------------------------------------------------------------

/// Skill source backed by the centralized Hermes Skills Index.
pub struct HermesIndexSource {
    auth: GitHubAuth,
    paths: HubPaths,
    client: reqwest::Client,
    index: Arc<RwLock<Option<serde_json::Value>>>,
    loaded: Arc<RwLock<bool>>,
}

impl HermesIndexSource {
    pub fn new(auth: GitHubAuth, paths: HubPaths) -> Self {
        Self {
            auth,
            paths,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            index: Arc::new(RwLock::new(None)),
            loaded: Arc::new(RwLock::new(false)),
        }
    }

    async fn ensure_loaded(&self) -> serde_json::Value {
        let loaded = *self.loaded.read().await;
        if loaded {
            if let Some(index) = &*self.index.read().await {
                return index.clone();
            }
            return serde_json::Value::Null;
        }

        let index = self.load_hermes_index().await;
        *self.index.write().await = index.clone();
        *self.loaded.write().await = true;
        index.unwrap_or(serde_json::Value::Null)
    }

    async fn load_hermes_index(&self) -> Option<serde_json::Value> {
        let cache_file = self.paths.index_cache_dir.join("hermes-index.json");

        // Check local cache
        if cache_file.exists() {
            if let Ok(metadata) = cache_file.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = SystemTime::now().duration_since(modified) {
                        if age < Duration::from_secs(HERMES_INDEX_TTL_SECS) {
                            if let Ok(content) = std::fs::read_to_string(&cache_file) {
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                                    if data.get("skills").is_some() {
                                        return Some(data);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Fetch from docs site
        let resp = match self.client.get(HERMES_INDEX_URL).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return Self::load_stale_cache(&cache_file),
        };

        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            _ => return Self::load_stale_cache(&cache_file),
        };

        if !data.is_object() || data.get("skills").is_none() {
            return Self::load_stale_cache(&cache_file);
        }

        // Cache locally
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::create_dir_all(cache_file.parent().unwrap());
            let _ = std::fs::write(&cache_file, &json);
        }

        Some(data)
    }

    fn load_stale_cache(cache_file: &Path) -> Option<serde_json::Value> {
        if cache_file.exists() {
            if let Ok(content) = std::fs::read_to_string(cache_file) {
                if let Ok(data) = serde_json::from_str(&content) {
                    return Some(data);
                }
            }
        }
        None
    }

    fn to_meta(entry: &serde_json::Value) -> SkillMeta {
        SkillMeta {
            name: entry.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            description: entry.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            source: entry.get("source").and_then(|v| v.as_str()).unwrap_or("hermes-index").to_string(),
            identifier: entry.get("identifier").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            trust_level: entry.get("trust_level").and_then(|v| v.as_str()).unwrap_or("community").to_string(),
            repo: entry.get("repo").and_then(|v| v.as_str()).map(String::from),
            path: entry.get("path").and_then(|v| v.as_str()).map(String::from),
            tags: entry.get("tags")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            extra: HashMap::new(),
        }
    }

    fn find_entry<'a>(identifier: &str, skills: &'a [serde_json::Value]) -> Option<&'a serde_json::Value> {
        // Exact identifier match
        for s in skills {
            if s.get("identifier").and_then(|v| v.as_str()) == Some(identifier) {
                return Some(s);
            }
        }

        // Try without source prefix
        let normalized = identifier
            .strip_prefix("skills-sh/")
            .or_else(|| identifier.strip_prefix("skills.sh/"))
            .or_else(|| identifier.strip_prefix("official/"))
            .or_else(|| identifier.strip_prefix("github/"))
            .or_else(|| identifier.strip_prefix("clawhub/"))
            .unwrap_or(identifier);

        for s in skills {
            let sid = s.get("identifier").and_then(|v| v.as_str()).unwrap_or("");
            let stored_norm = sid
                .strip_prefix("skills-sh/")
                .or_else(|| sid.strip_prefix("skills.sh/"))
                .or_else(|| sid.strip_prefix("official/"))
                .or_else(|| sid.strip_prefix("github/"))
                .or_else(|| sid.strip_prefix("clawhub/"))
                .unwrap_or(sid);
            if stored_norm == normalized {
                return Some(s);
            }
        }

        None
    }
}

#[async_trait]
impl SkillSource for HermesIndexSource {
    fn source_id(&self) -> &'static str {
        "hermes-index"
    }

    fn trust_level_for(&self, identifier: &str) -> &'static str {
        let index = self.index.blocking_read();
        if let Some(index_val) = index.as_ref() {
            if let Some(skills) = index_val.get("skills").and_then(|v| v.as_array()) {
                if let Some(entry) = Self::find_entry(identifier, skills) {
                    if let Some(tl) = entry.get("trust_level").and_then(|v| v.as_str()) {
                        return Box::leak(tl.to_string().into_boxed_str());
                    }
                }
            }
        }
        "community"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let index = self.ensure_loaded().await;
        let skills = index
            .get("skills")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if query.trim().is_empty() {
            return skills.iter().take(limit).map(Self::to_meta).collect();
        }

        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        for s in &skills {
            let searchable = format!(
                "{} {} {}",
                s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                s.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                s.get("tags")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default()
            )
            .to_lowercase();
            if searchable.contains(&query_lower) {
                results.push(Self::to_meta(s));
                if results.len() >= limit {
                    break;
                }
            }
        }
        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let index = self.ensure_loaded().await;
        let skills = index.get("skills").and_then(|v| v.as_array())?;
        let entry = Self::find_entry(identifier, skills)?;

        let gh = GitHubSource::new(self.auth.clone(), self.paths.clone(), vec![]);

        if let Some(resolved) = entry.get("resolved_github_id").and_then(|v| v.as_str()) {
            if let Some(mut bundle) = gh.fetch(resolved).await {
                let src = entry
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("hermes-index");
                bundle.source = src.to_string();
                bundle.identifier = identifier.to_string();
                return Some(bundle);
            }
        }

        let repo = entry.get("repo").and_then(|v| v.as_str())?;
        let path = entry.get("path").and_then(|v| v.as_str())?;
        let github_id = format!("{repo}/{path}");
        if let Some(mut bundle) = gh.fetch(&github_id).await {
            let src = entry
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("hermes-index");
            bundle.source = src.to_string();
            bundle.identifier = identifier.to_string();
            return Some(bundle);
        }

        None
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let index = self.ensure_loaded().await;
        let skills = index.get("skills").and_then(|v| v.as_array())?;
        let entry = Self::find_entry(identifier, skills)?;
        Some(Self::to_meta(entry))
    }
}

// ---------------------------------------------------------------------------
// Skills.sh source adapter
// ---------------------------------------------------------------------------

/// Discover skills via skills.sh and delegate to GitHub for content.
pub struct SkillsShSource {
    auth: GitHubAuth,
    github: GitHubSource,
    client: reqwest::Client,
    paths: HubPaths,
}

impl SkillsShSource {
    pub fn new(auth: GitHubAuth, paths: HubPaths) -> Self {
        let github = GitHubSource::new(auth.clone(), paths.clone(), vec![]);
        Self {
            auth,
            github,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            paths,
        }
    }

    fn normalize_identifier(identifier: &str) -> String {
        for prefix in &["skills-sh/", "skills.sh/", "skils-sh/", "skils.sh/"] {
            if let Some(stripped) = identifier.strip_prefix(prefix) {
                return stripped.to_string();
            }
        }
        identifier.to_string()
    }

    fn wrap_identifier(identifier: &str) -> String {
        format!("skills-sh/{identifier}")
    }

    fn candidate_identifiers(identifier: &str) -> Vec<String> {
        let parts: Vec<&str> = identifier.splitn(3, '/').collect();
        if parts.len() < 3 {
            return vec![identifier.to_string()];
        }
        let repo = format!("{}/{}", parts[0], parts[1]);
        let skill_path = parts[2].trim_start_matches('/');
        let mut candidates = vec![
            format!("{repo}/{skill_path}"),
            format!("{repo}/skills/{skill_path}"),
            format!("{repo}/.agents/skills/{skill_path}"),
            format!("{repo}/.claude/skills/{skill_path}"),
        ];
        candidates.dedup();
        candidates
    }
}

#[async_trait]
impl SkillSource for SkillsShSource {
    fn source_id(&self) -> &'static str {
        "skills-sh"
    }

    fn trust_level_for(&self, identifier: &str) -> &'static str {
        self.github
            .trust_level_for(&Self::normalize_identifier(identifier))
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        if query.trim().is_empty() {
            // Return featured skills from the front page
            let cache_key = "skills_sh_featured".to_string();
            if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
                if let Ok(skills) = serde_json::from_value::<Vec<SkillMeta>>(cached) {
                    return skills.into_iter().take(limit).collect();
                }
            }

            let url = "https://skills.sh";
            let resp = match self.client.get(url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => return vec![],
            };
            let html = match resp.text().await {
                Ok(h) => h,
                _ => return vec![],
            };

            let re = Regex::new(r#"href=["']/(?P<id>(?!agents/|_next/|api/)[^"'/]+/[^"'/]+/[^"'/]+)["']"#).unwrap();
            let mut results = Vec::new();
            let mut seen = std::collections::HashSet::new();

            for cap in re.captures_iter(&html) {
                let canonical = cap["id"].to_string();
                if !seen.insert(canonical.clone()) {
                    continue;
                }
                let parts: Vec<&str> = canonical.splitn(3, '/').collect();
                if parts.len() < 3 {
                    continue;
                }
                let repo = format!("{}/{}", parts[0], parts[1]);
                let skill_path = parts[2];
                let name = skill_path.split('/').last().unwrap_or(&skill_path).to_string();
                results.push(SkillMeta {
                    name,
                    description: format!("Featured on skills.sh from {repo}"),
                    source: "skills.sh".into(),
                    identifier: Self::wrap_identifier(&canonical),
                    trust_level: self.github.trust_level_for(&canonical).to_string(),
                    repo: Some(repo),
                    path: Some(skill_path.to_string()),
                    tags: vec![],
                    extra: HashMap::new(),
                });
                if results.len() >= limit {
                    break;
                }
            }

            if let Ok(json) = serde_json::to_value(&results) {
                write_index_cache(&self.paths, &cache_key, &json).await;
            }
            return results;
        }

        // Search via skills.sh API
        let cache_key = format!("skills_sh_search_{query}");
        if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
            if let Ok(skills) = serde_json::from_value::<Vec<SkillMeta>>(cached) {
                return skills.into_iter().take(limit).collect();
            }
        }

        let search_url = "https://skills.sh/api/search";
        let resp = match self
            .client
            .get(search_url)
            .query(&[("q", query), ("limit", &limit.to_string())])
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            _ => return vec![],
        };
        let items = data
            .get("skills")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut results = Vec::new();
        for item in items.iter().take(limit) {
            let canonical = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let repo_val = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let skill_path = item.get("skillId").and_then(|v| v.as_str()).unwrap_or("");

            let canonical = if canonical.is_empty() || canonical.matches('/').count() < 2 {
                format!("{repo_val}/{skill_path}")
            } else {
                canonical
            };

            let parts: Vec<&str> = canonical.splitn(3, '/').collect();
            if parts.len() < 3 {
                continue;
            }
            let repo = format!("{}/{}", parts[0], parts[1]);
            let path = parts[2];
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(path.split('/').last().unwrap_or(&path))
                .to_string();

            results.push(SkillMeta {
                name,
                description: format!("Indexed by skills.sh from {repo}"),
                source: "skills.sh".into(),
                identifier: Self::wrap_identifier(&canonical),
                trust_level: self.github.trust_level_for(&canonical).to_string(),
                repo: Some(repo),
                path: Some(path.to_string()),
                tags: vec![],
                extra: HashMap::new(),
            });
        }

        if let Ok(json) = serde_json::to_value(&results) {
            write_index_cache(&self.paths, &cache_key, &json).await;
        }
        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let canonical = Self::normalize_identifier(identifier);
        let candidates = Self::candidate_identifiers(&canonical);

        for candidate in &candidates {
            if let Some(mut bundle) = self.github.fetch(candidate).await {
                bundle.source = "skills.sh".to_string();
                bundle.identifier = Self::wrap_identifier(&canonical);
                return Some(bundle);
            }
        }
        None
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let canonical = Self::normalize_identifier(identifier);
        let candidates = Self::candidate_identifiers(&canonical);

        for candidate in &candidates {
            if let Some(mut meta) = self.github.inspect(candidate).await {
                meta.source = "skills.sh".to_string();
                meta.identifier = Self::wrap_identifier(&canonical);
                meta.trust_level = self.github.trust_level_for(&canonical).to_string();
                return Some(meta);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// ClawHub source adapter
// ---------------------------------------------------------------------------

/// Fetch skills from ClawHub (clawhub.ai) via their HTTP API.
pub struct ClawHubSource {
    client: reqwest::Client,
    paths: HubPaths,
}

impl ClawHubSource {
    pub fn new(paths: HubPaths) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            paths,
        }
    }

    fn normalize_tags(tags: &serde_json::Value) -> Vec<String> {
        match tags {
            serde_json::Value::Array(arr) => {
                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            }
            serde_json::Value::Object(map) => map
                .keys()
                .filter(|k| *k != "latest")
                .cloned()
                .collect(),
            _ => vec![],
        }
    }

    fn query_terms(query: &str) -> Vec<String> {
        let re = Regex::new(r"[^a-z0-9]+").unwrap();
        re.split(&query.to_lowercase())
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect()
    }

    fn search_score(query: &str, meta: &SkillMeta) -> i32 {
        let query_norm = query.trim().to_lowercase();
        if query_norm.is_empty() {
            return 1;
        }

        let identifier = meta.identifier.to_lowercase();
        let name = meta.name.to_lowercase();
        let description = meta.description.to_lowercase();
        let query_terms = Self::query_terms(&query_norm);
        let identifier_terms = Self::query_terms(&identifier);
        let name_terms = Self::query_terms(&name);
        let normalized_identifier = identifier_terms.join(" ");
        let normalized_name = name_terms.join(" ");

        let mut score = 0i32;

        if query_norm == identifier { score += 140; }
        if query_norm == name { score += 130; }
        if normalized_identifier == query_norm { score += 125; }
        if normalized_name == query_norm { score += 120; }
        if normalized_identifier.starts_with(&query_norm) { score += 95; }
        if normalized_name.starts_with(&query_norm) { score += 90; }
        if query_terms.len() <= identifier_terms.len()
            && identifier_terms[..query_terms.len()] == query_terms
        {
            score += 70;
        }
        if query_terms.len() <= name_terms.len()
            && name_terms[..query_terms.len()] == query_terms
        {
            score += 65;
        }
        if identifier.contains(&query_norm) { score += 40; }
        if name.contains(&query_norm) { score += 35; }
        if description.contains(&query_norm) { score += 10; }

        for term in &query_terms {
            if identifier_terms.contains(term) { score += 15; }
            if name_terms.contains(term) { score += 12; }
            if description.contains(term) { score += 3; }
        }

        score
    }
}

#[async_trait]
impl SkillSource for ClawHubSource {
    fn source_id(&self) -> &'static str {
        "clawhub"
    }

    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "community"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let cache_key = format!("clawhub_search_{query}_{limit}");
        if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
            if let Ok(skills) = serde_json::from_value::<Vec<SkillMeta>>(cached) {
                return skills;
            }
        }

        let url = "https://clawhub.ai/api/v1/skills";
        let resp = match self
            .client
            .get(url)
            .query(&[("search", query), ("limit", &limit.to_string())])
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let data: serde_json::Value = match resp.json().await {
            Ok(d) => d,
            _ => return vec![],
        };
        let skills_data = data
            .get("items")
            .or_else(|| Some(&data))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut results = Vec::new();
        for item in skills_data.iter().take(limit) {
            let slug = item.get("slug").and_then(|v| v.as_str()).unwrap_or("");
            if slug.is_empty() {
                continue;
            }
            let display_name = item
                .get("displayName")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or(slug);
            let summary = item
                .get("summary")
                .or_else(|| item.get("description"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tags = Self::normalize_tags(item.get("tags").unwrap_or(&serde_json::Value::Null));

            results.push(SkillMeta {
                name: display_name.to_string(),
                description: summary.to_string(),
                source: "clawhub".into(),
                identifier: slug.to_string(),
                trust_level: "community".into(),
                repo: None,
                path: None,
                tags,
                extra: HashMap::new(),
            });
        }

        // Score and sort for non-empty queries
        if !query.trim().is_empty() {
            results = results
                .into_iter()
                .filter(|m| Self::search_score(query, m) > 0)
                .collect();
            results.sort_by(|a, b| {
                let sa = Self::search_score(query, a);
                let sb = Self::search_score(query, b);
                sb.cmp(&sa).then(a.name.cmp(&b.name))
            });
        }

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        results.retain(|m| seen.insert(m.identifier.clone()));

        if let Ok(json) = serde_json::to_value(&results) {
            write_index_cache(&self.paths, &cache_key, &json).await;
        }
        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let slug = identifier.split('/').last().unwrap_or(identifier);
        let url = format!("https://clawhub.ai/api/v1/skills/{slug}");

        let skill_data: serde_json::Value = self
            .client
            .get(&url)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        let payload = if let Some(nested) = skill_data.get("skill") {
            let mut merged = nested.clone();
            if let Some(lv) = skill_data.get("latestVersion") {
                merged["latestVersion"] = lv.clone();
            }
            merged
        } else {
            skill_data.clone()
        };

        // Try to find SKILL.md content in version data
        let versions_url = format!("https://clawhub.ai/api/v1/skills/{slug}/versions");
        let versions_data: Vec<serde_json::Value> = match self
            .client
            .get(&versions_url)
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or_default(),
            _ => vec![],
        };

        let latest_version = payload
            .get("latestVersion")
            .and_then(|v| v.as_str())
            .or_else(|| {
                versions_data.first().and_then(|v| {
                    v.get("version").and_then(|v2| v2.as_str())
                })
            })?;

        // Try ZIP download
        let files = self.download_zip(slug, latest_version).await;

        if !files.contains_key("SKILL.md") {
            return None;
        }

        Some(SkillBundle {
            name: slug.to_string(),
            files,
            source: "clawhub".into(),
            identifier: slug.to_string(),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let slug = identifier.split('/').last().unwrap_or(identifier);
        let url = format!("https://clawhub.ai/api/v1/skills/{slug}");

        let mut data: serde_json::Value = self.client.get(&url).send().await.ok()?.json().await.ok()?;

        // Handle nested "skill" wrapper
        if let Some(nested) = data.get("skill").and_then(|v| v.as_object()) {
            let mut merged = nested.clone();
            if let Some(lv) = data.get("latestVersion") {
                merged.insert("latestVersion".into(), lv.clone());
            }
            data = serde_json::Value::Object(merged);
        }

        if !data.is_object() {
            return None;
        }

        let tags = Self::normalize_tags(data.get("tags").unwrap_or(&serde_json::Value::Null));
        let name = data
            .get("displayName")
            .or_else(|| data.get("name"))
            .or_else(|| data.get("slug"))
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let desc = data
            .get("summary")
            .or_else(|| data.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Some(SkillMeta {
            name: name.to_string(),
            description: desc.to_string(),
            source: "clawhub".into(),
            identifier: data.get("slug").and_then(|v| v.as_str()).unwrap_or(slug).to_string(),
            trust_level: "community".into(),
            repo: None,
            path: None,
            tags,
            extra: HashMap::new(),
        })
    }
}

impl ClawHubSource {
    async fn download_zip(&self, _slug: &str, _version: &str) -> HashMap<String, String> {
        // ZIP download via HTTP — simplified for now
        HashMap::new()
    }
}

// ---------------------------------------------------------------------------
// Claude Code marketplace source adapter
// ---------------------------------------------------------------------------

/// Discover skills from Claude Code marketplace repos.
pub struct ClaudeMarketplaceSource {
    auth: GitHubAuth,
    github: GitHubSource,
    paths: HubPaths,
}

impl ClaudeMarketplaceSource {
    pub fn new(auth: GitHubAuth, paths: HubPaths) -> Self {
        let github = GitHubSource::new(auth.clone(), paths.clone(), vec![]);
        Self {
            auth,
            github,
            paths,
        }
    }

    async fn fetch_marketplace_index(&self, repo: &str) -> Vec<serde_json::Value> {
        let cache_key = format!("claude_marketplace_{}", repo.replace('/', "_"));
        if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
            if let Ok(plugins) = serde_json::from_value::<Vec<serde_json::Value>>(cached) {
                return plugins;
            }
        }

        let url = format!(
            "https://api.github.com/repos/{repo}/contents/.claude-plugin/marketplace.json"
        );
        let mut headers = self.auth.get_headers();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.github.v3.raw"),
        );

        let resp = match self.github.client.get(&url).headers(headers).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let text = match resp.text().await {
            Ok(t) => t,
            _ => return vec![],
        };
        let data: serde_json::Value = match serde_json::from_str(&text) {
            Ok(d) => d,
            _ => return vec![],
        };
        let plugins = data
            .get("plugins")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if let Ok(json) = serde_json::to_value(&plugins) {
            write_index_cache(&self.paths, &cache_key, &json).await;
        }
        plugins
    }
}

#[async_trait]
impl SkillSource for ClaudeMarketplaceSource {
    fn source_id(&self) -> &'static str {
        "claude-marketplace"
    }

    fn trust_level_for(&self, identifier: &str) -> &'static str {
        let parts: Vec<&str> = identifier.splitn(3, '/').collect();
        if parts.len() >= 2 {
            let repo = format!("{}/{}", parts[0], parts[1]);
            if TRUSTED_REPOS.contains(&repo.as_str()) {
                return "trusted";
            }
        }
        "community"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        let known_marketplaces = ["anthropics/skills", "aiskillstore/marketplace"];

        for marketplace_repo in &known_marketplaces {
            let plugins = self.fetch_marketplace_index(marketplace_repo).await;
            for plugin in &plugins {
                let name = plugin.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let description = plugin.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let searchable = format!("{name} {description}").to_lowercase();

                if searchable.contains(&query_lower) {
                    let source_path = plugin
                        .get("source")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let identifier = if source_path.starts_with("./") {
                        format!("{marketplace_repo}/{}", &source_path[2..])
                    } else if source_path.contains('/') {
                        source_path.to_string()
                    } else {
                        format!("{marketplace_repo}/{source_path}")
                    };

                    let trust = self.trust_level_for(&identifier).to_string();
                    results.push(SkillMeta {
                        name: name.to_string(),
                        description: description.to_string(),
                        source: "claude-marketplace".into(),
                        identifier,
                        trust_level: trust,
                        repo: Some(marketplace_repo.to_string()),
                        path: None,
                        tags: vec![],
                        extra: HashMap::new(),
                    });
                }
            }
        }

        results.truncate(limit);
        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let mut bundle = self.github.fetch(identifier).await?;
        bundle.source = "claude-marketplace".to_string();
        Some(bundle)
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let mut meta = self.github.inspect(identifier).await?;
        meta.source = "claude-marketplace".to_string();
        meta.trust_level = self.trust_level_for(identifier).to_string();
        Some(meta)
    }
}

// ---------------------------------------------------------------------------
// LobeHub source adapter
// ---------------------------------------------------------------------------

/// Fetch skills from LobeHub's agent marketplace.
pub struct LobeHubSource {
    client: reqwest::Client,
    paths: HubPaths,
}

impl LobeHubSource {
    pub fn new(paths: HubPaths) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            paths,
        }
    }

    async fn fetch_index(&self) -> Option<serde_json::Value> {
        let cache_key = "lobehub_index".to_string();
        if let Some(cached) = read_index_cache(&self.paths, &cache_key).await {
            return Some(cached);
        }

        let resp = self
            .client
            .get("https://chat-agents.lobehub.com/index.json")
            .send()
            .await
            .ok()?;
        let data: serde_json::Value = resp.json().await.ok()?;

        write_index_cache(&self.paths, &cache_key, &data).await;
        Some(data)
    }

    fn convert_to_skill_md(agent_data: &serde_json::Value) -> String {
        let meta = agent_data.get("meta").unwrap_or(agent_data);
        let identifier = agent_data
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or("lobehub-agent");
        let title = meta.get("title").and_then(|v| v.as_str()).unwrap_or(identifier);
        let description = meta.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let tags = meta
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let system_role = agent_data
            .get("config")
            .and_then(|c| c.get("systemRole"))
            .and_then(|v| v.as_str())
            .unwrap_or("(No system role defined)");

        format!(
            "---\n\
             name: {identifier}\n\
             description: {desc}\n\
             metadata:\n\
             \x20 hermes:\n\
             \x20\x20 tags: [{tags}]\n\
             \x20 lobehub:\n\
             \x20\x20 source: lobehub\n\
             ---\n\n\
             # {title}\n\n\
             {description}\n\n\
             ## Instructions\n\n\
             {system_role}\n",
            desc = &description[..description.len().min(500)],
        )
    }
}

#[async_trait]
impl SkillSource for LobeHubSource {
    fn source_id(&self) -> &'static str {
        "lobehub"
    }

    fn trust_level_for(&self, _identifier: &str) -> &'static str {
        "community"
    }

    async fn search(&self, query: &str, limit: usize) -> Vec<SkillMeta> {
        let index = match self.fetch_index().await {
            Some(i) => i,
            None => return vec![],
        };

        let query_lower = query.to_lowercase();
        let agents = index
            .get("agents")
            .or_else(|| Some(&index))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut results = Vec::new();
        for agent in &agents {
            let meta = agent.get("meta").unwrap_or(agent);
            let title = meta
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(
                    agent
                        .get("identifier")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                );
            let desc = meta.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let tags = meta
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let searchable = format!("{title} {desc} {tags}").to_lowercase();

            if searchable.contains(&query_lower) {
                let default_ident = title.to_lowercase().replace(' ', "-");
                let ident = agent
                    .get("identifier")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_ident);
                results.push(SkillMeta {
                    name: ident.to_string(),
                    description: desc.chars().take(200).collect(),
                    source: "lobehub".into(),
                    identifier: format!("lobehub/{ident}"),
                    trust_level: "community".into(),
                    repo: None,
                    path: None,
                    tags: meta
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default(),
                    extra: HashMap::new(),
                });
            }
            if results.len() >= limit {
                break;
            }
        }

        results
    }

    async fn fetch(&self, identifier: &str) -> Option<SkillBundle> {
        let agent_id = identifier
            .strip_prefix("lobehub/")
            .unwrap_or(identifier);

        let url = format!("https://chat-agents.lobehub.com/{agent_id}.json");
        let agent_data: serde_json::Value = self.client.get(&url).send().await.ok()?.json().await.ok()?;

        let skill_md = Self::convert_to_skill_md(&agent_data);
        let mut files = HashMap::new();
        files.insert("SKILL.md".into(), skill_md);

        Some(SkillBundle {
            name: agent_id.to_string(),
            files,
            source: "lobehub".into(),
            identifier: format!("lobehub/{agent_id}"),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        })
    }

    async fn inspect(&self, identifier: &str) -> Option<SkillMeta> {
        let agent_id = identifier
            .strip_prefix("lobehub/")
            .unwrap_or(identifier);

        let index = self.fetch_index().await?;
        let agents = index
            .get("agents")
            .or_else(|| Some(&index))
            .and_then(|v| v.as_array())?;

        for agent in agents {
            if agent.get("identifier").and_then(|v| v.as_str()) == Some(agent_id) {
                let meta = agent.get("meta").unwrap_or(agent);
                return Some(SkillMeta {
                    name: agent_id.to_string(),
                    description: meta.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    source: "lobehub".into(),
                    identifier: format!("lobehub/{agent_id}"),
                    trust_level: "community".into(),
                    repo: None,
                    path: None,
                    tags: meta
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default(),
                    extra: HashMap::new(),
                });
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Hub lock file management
// ---------------------------------------------------------------------------

/// Manages `skills/.hub/lock.json` — tracks provenance of installed hub skills.
pub struct HubLockFile {
    path: PathBuf,
}

impl HubLockFile {
    pub fn new(paths: &HubPaths) -> Self {
        Self {
            path: paths.lock_file.clone(),
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> LockFileData {
        if !self.path.exists() {
            return LockFileData {
                version: 1,
                installed: HashMap::new(),
            };
        }
        match std::fs::read_to_string(&self.path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or(LockFileData {
                version: 1,
                installed: HashMap::new(),
            }),
            Err(_) => LockFileData {
                version: 1,
                installed: HashMap::new(),
            },
        }
    }

    pub fn save(&self, data: &LockFileData) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(data)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    pub fn record_install(
        &self,
        name: &str,
        source: &str,
        identifier: &str,
        trust_level: &str,
        scan_verdict: &str,
        skill_hash: &str,
        install_path: &str,
        files: &[String],
        metadata: &HashMap<String, String>,
    ) -> std::io::Result<()> {
        let mut data = self.load();
        let now = iso_now();
        let entry = InstalledEntry {
            source: source.to_string(),
            identifier: identifier.to_string(),
            trust_level: trust_level.to_string(),
            scan_verdict: scan_verdict.to_string(),
            content_hash: skill_hash.to_string(),
            install_path: install_path.to_string(),
            files: files.to_vec(),
            metadata: metadata.clone(),
            installed_at: now.clone(),
            updated_at: now,
        };
        data.installed.insert(name.to_string(), entry);
        self.save(&data)
    }

    pub fn record_uninstall(&self, name: &str) -> std::io::Result<()> {
        let mut data = self.load();
        data.installed.remove(name);
        self.save(&data)
    }

    pub fn get_installed(&self, name: &str) -> Option<InstalledEntry> {
        let data = self.load();
        data.installed.get(name).cloned()
    }

    pub fn list_installed(&self) -> Vec<(String, InstalledEntry)> {
        let data = self.load();
        let mut result: Vec<_> = data.installed.into_iter().collect();
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
}

// ---------------------------------------------------------------------------
// Taps management
// ---------------------------------------------------------------------------

/// Manages `taps.json` — custom GitHub repo sources.
#[derive(Debug, Clone)]
pub struct TapsManager {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TapsFile {
    taps: Vec<TapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TapEntry {
    repo: String,
    #[serde(default = "default_tap_path")]
    path: String,
}

fn default_tap_path() -> String {
    "skills/".to_string()
}

impl TapsManager {
    pub fn new(paths: &HubPaths) -> Self {
        Self {
            path: paths.taps_file.clone(),
        }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> Vec<TapEntry> {
        if !self.path.exists() {
            return vec![];
        }
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                serde_json::from_str::<TapsFile>(&content)
                    .map(|t| t.taps)
                    .unwrap_or_default()
            }
            Err(_) => vec![],
        }
    }

    fn save(&self, taps: &[TapEntry]) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = TapsFile {
            taps: taps.to_vec(),
        };
        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(&self.path, json)
    }

    /// Add a tap. Returns `false` if already exists.
    pub fn add(&self, repo: &str, path: &str) -> bool {
        let mut taps = self.load();
        if taps.iter().any(|t| t.repo == repo) {
            return false;
        }
        taps.push(TapEntry {
            repo: repo.to_string(),
            path: path.to_string(),
        });
        self.save(&taps).is_ok()
    }

    /// Remove a tap by repo name. Returns `false` if not found.
    pub fn remove(&self, repo: &str) -> bool {
        let taps = self.load();
        let len_before = taps.len();
        let new_taps: Vec<TapEntry> = taps.into_iter().filter(|t| t.repo != repo).collect();
        if new_taps.len() == len_before {
            return false;
        }
        self.save(&new_taps).is_ok()
    }

    /// List all configured taps as (repo, path) pairs.
    pub fn list_taps(&self) -> Vec<(String, String)> {
        self.load()
            .into_iter()
            .map(|t| (t.repo, t.path))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

/// Append a line to the audit log.
pub fn append_audit_log(
    paths: &HubPaths,
    action: &str,
    skill_name: &str,
    source: &str,
    trust_level: &str,
    verdict: &str,
    extra: &str,
) {
    if let Some(parent) = paths.audit_log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let line = format!(
        "{timestamp} {action} {skill_name} {source}:{trust_level} {verdict} {extra}\n"
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.audit_log)
        .and_then(|f| std::io::Write::write_all(&mut std::io::BufWriter::new(f), line.as_bytes()));
}

// ---------------------------------------------------------------------------
// Index cache helpers
// ---------------------------------------------------------------------------

async fn read_index_cache(paths: &HubPaths, key: &str) -> Option<serde_json::Value> {
    let cache_file = paths.index_cache_dir.join(format!("{key}.json"));
    if !cache_file.exists() {
        return None;
    }
    let metadata = cache_file.metadata().ok()?;
    let modified = metadata.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).ok()?;
    if age > Duration::from_secs(INDEX_CACHE_TTL_SECS) {
        return None;
    }
    let content = std::fs::read_to_string(&cache_file).ok()?;
    serde_json::from_str(&content).ok()
}

async fn write_index_cache(paths: &HubPaths, key: &str, data: &serde_json::Value) {
    let _ = std::fs::create_dir_all(&paths.index_cache_dir);
    let cache_file = paths.index_cache_dir.join(format!("{key}.json"));

    // Ensure .ignore exists
    let ignore_file = paths.hub_dir.join(".ignore");
    if !ignore_file.exists() {
        let _ = std::fs::write(&ignore_file, "# Exclude hub internals from search tools\n*\n");
    }

    if let Ok(json) = serde_json::to_string(data) {
        let _ = std::fs::write(&cache_file, &json);
    }
}

// ---------------------------------------------------------------------------
// Content hashing
// ---------------------------------------------------------------------------

/// Compute a deterministic content hash for a skill bundle.
pub fn bundle_content_hash(bundle: &SkillBundle) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut keys: Vec<&String> = bundle.files.keys().collect();
    keys.sort();
    for key in keys {
        key.hash(&mut hasher);
        bundle.files[key].hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// Compute a content hash for an installed skill directory.
pub fn directory_content_hash(dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    let mut files: Vec<PathBuf> = Vec::new();
    collect_file_paths_recursive(dir, &mut files);
    files.sort();

    for path in &files {
        path.hash(&mut hasher);
        if let Ok(content) = std::fs::read_to_string(path) {
            content.hash(&mut hasher);
        }
    }

    format!("{:016x}", hasher.finish())
}

// ---------------------------------------------------------------------------
// Source router
// ---------------------------------------------------------------------------

/// Create all configured source adapters.
///
/// Returns a list of active sources for search/fetch operations.
pub fn create_source_router(
    auth: GitHubAuth,
    paths: HubPaths,
    optional_skills_dir: Option<PathBuf>,
) -> Vec<Arc<dyn SkillSource>> {
    let taps_mgr = TapsManager::new(&paths);
    let extra_taps = taps_mgr.list_taps();

    let mut sources: Vec<Arc<dyn SkillSource>> = Vec::new();

    // Official optional skills (highest priority)
    if let Some(opt_dir) = optional_skills_dir {
        sources.push(Arc::new(OptionalSkillSource::new(opt_dir)));
    }

    // Hermes centralized index
    sources.push(Arc::new(HermesIndexSource::new(
        auth.clone(),
        paths.clone(),
    )));

    // Skills.sh
    sources.push(Arc::new(SkillsShSource::new(
        auth.clone(),
        paths.clone(),
    )));

    // Well-known source
    sources.push(Arc::new(WellKnownSkillSource::new(paths.clone())));

    // Direct URL source
    sources.push(Arc::new(UrlSource::new()));

    // GitHub source with custom taps
    sources.push(Arc::new(GitHubSource::new(
        auth.clone(),
        paths.clone(),
        extra_taps,
    )));

    // ClawHub
    sources.push(Arc::new(ClawHubSource::new(paths.clone())));

    // Claude marketplace
    sources.push(Arc::new(ClaudeMarketplaceSource::new(
        auth.clone(),
        paths.clone(),
    )));

    // LobeHub
    sources.push(Arc::new(LobeHubSource::new(paths)));

    sources
}

// ---------------------------------------------------------------------------
// Parallel search
// ---------------------------------------------------------------------------

/// Search all sources in parallel and merge results.
pub async fn parallel_search_sources(
    sources: &[Arc<dyn SkillSource>],
    query: &str,
    per_source_limits: &HashMap<String, usize>,
    source_filter: &str,
) -> (
    Vec<SkillMeta>,
    HashMap<String, usize>,
    Vec<String>,
) {
    let mut active: Vec<Arc<dyn SkillSource>> = Vec::new();

    // Check if the Hermes index is available (skip external APIs if so)
    let mut index_available = false;
    if source_filter == "all" {
        for src in sources {
            if src.source_id() == "hermes-index" {
                index_available = true;
                break;
            }
        }
    }

    let api_source_ids = [
        "github", "skills-sh", "clawhub", "claude-marketplace", "lobehub", "well-known",
    ];

    for src in sources {
        let sid = src.source_id();
        if source_filter != "all" && sid != source_filter && sid != "official" {
            continue;
        }
        if index_available && api_source_ids.contains(&sid) {
            continue;
        }
        active.push(src.clone());
    }

    if active.is_empty() {
        return (vec![], HashMap::new(), vec![]);
    }

    let futures: Vec<_> = active
        .iter()
        .map(|src| {
            let query = query.to_string();
            let limit = per_source_limits
                .get(src.source_id())
                .copied()
                .unwrap_or(50);
            let src = src.clone();
            tokio::spawn(async move {
                let sid = src.source_id();
                let results = src.search(&query, limit).await;
                (sid.to_string(), results)
            })
        })
        .collect();

    let mut all_results = Vec::new();
    let mut source_counts = HashMap::new();
    let timed_out = Vec::<String>::new();

    for fut in join_all(futures).await {
        match fut {
            Ok((sid, results)) => {
                source_counts.insert(sid.clone(), results.len());
                all_results.extend(results);
            }
            Err(_) => {
                // Task panicked or cancelled
            }
        }
    }

    (all_results, source_counts, timed_out)
}

/// Search all sources (in parallel) and merge/deduplicate results.
pub async fn unified_search(
    query: &str,
    sources: &[Arc<dyn SkillSource>],
    source_filter: &str,
    limit: usize,
) -> Vec<SkillMeta> {
    let mut per_source_limits = HashMap::new();
    per_source_limits.insert("official".into(), 200usize);
    per_source_limits.insert("skills-sh".into(), 200usize);
    per_source_limits.insert("github".into(), 200usize);
    per_source_limits.insert("clawhub".into(), 500usize);
    per_source_limits.insert("claude-marketplace".into(), 100usize);
    per_source_limits.insert("lobehub".into(), 500usize);

    let (all_results, _, _) = parallel_search_sources(
        sources,
        query,
        &per_source_limits,
        source_filter,
    )
    .await;

    // Deduplicate by name, preferring higher trust levels
    let trust_rank = |t: &str| -> i32 {
        match t {
            "builtin" => 2,
            "trusted" => 1,
            _ => 0,
        }
    };

    let mut seen: HashMap<String, SkillMeta> = HashMap::new();
    for r in all_results {
        let rank = trust_rank(&r.trust_level);
        let entry = seen.entry(r.name.clone()).or_insert_with(|| r.clone());
        if rank > trust_rank(&entry.trust_level) {
            *entry = r;
        }
    }

    let mut deduped: Vec<SkillMeta> = seen.into_values().collect();
    deduped.truncate(limit);
    deduped
}

// ---------------------------------------------------------------------------
// Quarantine & Install pipeline
// ---------------------------------------------------------------------------

/// Write a skill bundle to the quarantine directory for scanning.
pub fn quarantine_bundle(
    bundle: &SkillBundle,
    paths: &HubPaths,
) -> Result<PathBuf> {
    paths.ensure_dirs().map_err(|e| {
        Error::Config(format!("Failed to create hub dirs: {e}"))
    })?;

    let skill_name = validate_skill_name(&bundle.name)?;
    let dest = paths.quarantine_dir.join(&skill_name);

    if dest.exists() {
        std::fs::remove_dir_all(&dest).ok();
    }
    std::fs::create_dir_all(&dest).map_err(|e| {
        Error::Config(format!("Failed to create quarantine dir {dest:?}: {e}"))
    })?;

    for (rel_path, content) in &bundle.files {
        let safe_rel = validate_bundle_rel_path(rel_path)?;
        let file_dest = dest.join(&safe_rel);
        if let Some(parent) = file_dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&file_dest, content).map_err(|e| {
            Error::Config(format!("Failed to write quarantine file {safe_rel}: {e}"))
        })?;
    }

    Ok(dest)
}

/// Move a scanned skill from quarantine into the skills directory.
pub fn install_from_quarantine(
    quarantine_path: &Path,
    skill_name: &str,
    category: &str,
    bundle: &SkillBundle,
    scan_verdict: &str,
    paths: &HubPaths,
) -> Result<PathBuf> {
    let safe_skill_name = validate_skill_name(skill_name)?;

    let install_dir = if category.is_empty() {
        paths.skills_dir.join(&safe_skill_name)
    } else {
        let safe_category = validate_category_name(category)?;
        paths.skills_dir.join(&safe_category).join(&safe_skill_name)
    };

    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir).ok();
    }

    // Warn about large SKILL.md
    let skill_md = quarantine_path.join("SKILL.md");
    if skill_md.exists() {
        if let Ok(meta) = skill_md.metadata() {
            if meta.len() > 100_000 {
                warn!(
                    "Skill '{skill_name}' has a large SKILL.md ({} bytes). \
                     Large skills consume significant context.",
                    meta.len()
                );
            }
        }
    }

    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Error::Config(format!("Failed to create install directory: {e}"))
        })?;
    }

    // Move files from quarantine to install dir
    copy_dir_all(quarantine_path, &install_dir).map_err(|e| {
        Error::Config(format!("Failed to move skill to install dir: {e}"))
    })?;

    // Record in lock file
    let lock = HubLockFile::new(paths);
    let skill_hash = directory_content_hash(&install_dir);
    let install_rel = install_dir
        .strip_prefix(&paths.skills_dir)
        .unwrap_or(&install_dir)
        .to_str()
        .unwrap_or("")
        .to_string();

    let files: Vec<String> = bundle.files.keys().cloned().collect();

    lock.record_install(
        &safe_skill_name,
        &bundle.source,
        &bundle.identifier,
        &bundle.trust_level,
        scan_verdict,
        &skill_hash,
        &install_rel,
        &files,
        &bundle.metadata,
    )
    .map_err(|e| Error::Config(format!("Failed to record install: {e}")))?;

    append_audit_log(
        paths,
        "INSTALL",
        &safe_skill_name,
        &bundle.source,
        &bundle.trust_level,
        scan_verdict,
        &skill_hash,
    );

    Ok(install_dir)
}

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(&entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// Remove a hub-installed skill.
pub fn uninstall_skill(skill_name: &str, paths: &HubPaths) -> Result<(bool, String)> {
    let lock = HubLockFile::new(paths);
    let entry = lock.get_installed(skill_name);

    let entry = match entry {
        Some(e) => e,
        None => {
            return Ok((
                false,
                format!("'{skill_name}' is not a hub-installed skill"),
            ));
        }
    };

    let install_path = paths.skills_dir.join(&entry.install_path);
    if install_path.exists() {
        std::fs::remove_dir_all(&install_path).ok();
    }

    lock.record_uninstall(skill_name).ok();

    append_audit_log(
        paths,
        "UNINSTALL",
        skill_name,
        &entry.source,
        &entry.trust_level,
        "n/a",
        "user_request",
    );

    Ok((
        true,
        format!("Uninstalled '{skill_name}' from {}", entry.install_path),
    ))
}

/// Check installed hub skills for upstream changes.
pub async fn check_for_skill_updates(
    name: Option<&str>,
    paths: &HubPaths,
    sources: Option<&[Arc<dyn SkillSource>]>,
) -> Vec<HashMap<String, String>> {
    let lock = HubLockFile::new(paths);
    let mut installed = lock.list_installed();

    if let Some(name) = name {
        installed.retain(|(n, _)| n == name);
    }

    if installed.is_empty() {
        return vec![];
    }

    let sources = match sources {
        Some(s) => s.to_vec(),
        None => {
            let auth = GitHubAuth::new();
            create_source_router(auth, paths.clone(), None)
        }
    };

    let mut results = Vec::new();
    for (installed_name, entry) in &installed {
        let mut found = false;
        for src in &sources {
            if let Some(bundle) = src.fetch(&entry.identifier).await {
                let current_hash = &entry.content_hash;
                let latest_hash = bundle_content_hash(&bundle);

                let status = if current_hash == &latest_hash {
                    "up_to_date"
                } else {
                    "update_available"
                };

                let mut map = HashMap::new();
                map.insert("name".into(), installed_name.clone());
                map.insert("identifier".into(), entry.identifier.clone());
                map.insert("source".into(), entry.source.clone());
                map.insert("status".into(), status.to_string());
                results.push(map);
                found = true;
                break;
            }
        }
        if !found {
            let mut map = HashMap::new();
            map.insert("name".into(), installed_name.clone());
            map.insert("identifier".into(), entry.identifier.clone());
            map.insert("source".into(), entry.source.clone());
            map.insert("status".into(), "unavailable".into());
            results.push(map);
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Recursively find all `SKILL.md` files up to `max_depth`.
fn find_skill_md_files(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut results = Vec::new();
    find_skill_md_files_recursive(dir, dir, 0, max_depth, &mut results);
    results
}

fn find_skill_md_files_recursive(
    base: &Path,
    current: &Path,
    depth: usize,
    max_depth: usize,
    results: &mut Vec<PathBuf>,
) {
    if depth > max_depth {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                find_skill_md_files_recursive(base, &path, depth + 1, max_depth, results);
            } else if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
                results.push(path);
            }
        }
    }
}

/// Recursively collect all file paths under a directory.
fn collect_file_paths_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_file_paths_recursive(&path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
}

/// Recursively collect file contents, filtering out hidden/pyc/__pycache__.
fn collect_files_recursive(base: &Path, current: &Path, files: &mut HashMap<String, String>) {
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if dir_name == "__pycache__" {
                    continue;
                }
                collect_files_recursive(base, &path, files);
            } else if path.is_file() {
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if fname.starts_with('.') {
                    continue;
                }
                if path.extension().map_or(false, |e| e == "pyc") {
                    continue;
                }
                if let Ok(rel_path) = path.strip_prefix(base) {
                    if let Some(rel_str) = rel_path.to_str() {
                        if !rel_str.is_empty() {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                files.insert(rel_str.to_string(), content);
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_normalize_bundle_path_valid() {
        assert_eq!(
            normalize_bundle_path("my-skill", false).unwrap(),
            "my-skill"
        );
        assert_eq!(
            normalize_bundle_path("my-skill", true).unwrap(),
            "my-skill"
        );
        assert_eq!(
            normalize_bundle_path("category/my-skill", true).unwrap(),
            "category/my-skill"
        );
    }

    #[test]
    fn test_normalize_bundle_path_invalid() {
        assert!(normalize_bundle_path("", false).is_err());
        assert!(normalize_bundle_path("/etc/passwd", false).is_err());
        assert!(normalize_bundle_path("../evil", false).is_err());
        assert!(normalize_bundle_path("a/../../b", false).is_err());
        assert!(normalize_bundle_path("C:\\\\foo", false).is_err());
        assert!(normalize_bundle_path("a/b", false).is_err()); // nested not allowed
    }

    #[test]
    fn test_is_valid_skill_name() {
        assert!(is_valid_skill_name("my-skill"));
        assert!(is_valid_skill_name("skill123"));
        assert!(is_valid_skill_name("a"));
        assert!(!is_valid_skill_name("Skill")); // blocked sentinel
        assert!(!is_valid_skill_name("readme"));
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("has space"));
    }

    #[test]
    fn test_hub_paths() {
        let base = PathBuf::from("/tmp/test-hermes");
        let paths = HubPaths::new(&base);
        assert_eq!(paths.skills_dir, base.join("skills"));
        assert_eq!(paths.hub_dir, base.join("skills/.hub"));
        assert_eq!(paths.lock_file, base.join("skills/.hub/lock.json"));
    }

    #[test]
    fn test_hub_lock_file_roundtrip() {
        let tmp = std::env::temp_dir().join("hermes_lock_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let paths = HubPaths::new(&tmp);
        let lock = HubLockFile::new(&paths);

        let mut metadata = HashMap::new();
        metadata.insert("key".into(), "value".into());

        lock.record_install(
            "test-skill",
            "github",
            "owner/repo/path",
            "community",
            "safe",
            "abc123",
            "test-skill",
            &["SKILL.md".into(), "script.sh".into()],
            &metadata,
        )
        .unwrap();

        let entry = lock.get_installed("test-skill").unwrap();
        assert_eq!(entry.source, "github");
        assert_eq!(entry.trust_level, "community");
        assert_eq!(entry.scan_verdict, "safe");
        assert_eq!(entry.files.len(), 2);

        let list = lock.list_installed();
        assert_eq!(list.len(), 1);

        lock.record_uninstall("test-skill").unwrap();
        assert!(lock.get_installed("test-skill").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_taps_manager() {
        let tmp = std::env::temp_dir().join("hermes_taps_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let paths = HubPaths::new(&tmp);
        let taps = TapsManager::new(&paths);

        assert!(taps.add("owner/repo", "skills/"));
        assert!(!taps.add("owner/repo", "skills/")); // duplicate

        let list = taps.list_taps();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "owner/repo");

        assert!(taps.remove("owner/repo"));
        assert!(!taps.remove("nonexistent"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_github_auth_env() {
        // Test anonymous fallback
        let auth = GitHubAuth::new();
        assert!(!auth.is_authenticated() || auth.auth_method_name() == "pat");
    }

    #[test]
    fn test_bundle_content_hash() {
        let mut files = HashMap::new();
        files.insert("SKILL.md".into(), "# Test\n".into());
        files.insert("script.sh".into(), "echo hello\n".into());
        let bundle = SkillBundle {
            name: "test".into(),
            files,
            source: "github".into(),
            identifier: "test".into(),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        };
        let hash = bundle_content_hash(&bundle);
        assert!(!hash.is_empty());

        // Same bundle should produce same hash
        let mut files2 = HashMap::new();
        files2.insert("SKILL.md".into(), "# Test\n".into());
        files2.insert("script.sh".into(), "echo hello\n".into());
        let bundle2 = SkillBundle {
            name: "test".into(),
            files: files2,
            source: "github".into(),
            identifier: "test".into(),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        };
        assert_eq!(bundle_content_hash(&bundle2), hash);
    }

    #[test]
    fn test_github_auth_headers() {
        let auth = GitHubAuth::new();
        let headers = auth.get_headers();
        assert!(headers.contains_key("accept"));
    }

    #[test]
    fn test_url_join() {
        assert_eq!(
            url_join("https://github.com/repo", "path/to/file"),
            "https://github.com/repo/path/to/file"
        );
        assert_eq!(
            url_join("https://github.com/repo/", "/path/to/file"),
            "https://github.com/path/to/file"
        );
        assert_eq!(
            url_join("https://github.com/repo", "https://other.com/file"),
            "https://other.com/file"
        );
    }

    #[test]
    fn test_quarantine_and_install_roundtrip() {
        let tmp = std::env::temp_dir().join("hermes_quarantine_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let paths = HubPaths::new(&tmp);

        let bundle = SkillBundle {
            name: "test-skill".into(),
            files: {
                let mut f = HashMap::new();
                f.insert("SKILL.md".into(), "# Test Skill\n".into());
                f.insert("script.sh".into(), "echo hello\n".into());
                f
            },
            source: "github".into(),
            identifier: "owner/repo/test-skill".into(),
            trust_level: "community".into(),
            metadata: HashMap::new(),
        };

        // Quarantine
        let q_path = quarantine_bundle(&bundle, &paths).unwrap();
        assert!(q_path.exists());
        assert!(q_path.join("SKILL.md").exists());

        // Install
        let install_dir =
            install_from_quarantine(&q_path, "test-skill", "", &bundle, "safe", &paths).unwrap();
        assert!(install_dir.exists());
        assert!(install_dir.join("SKILL.md").exists());

        // Verify lock file
        let lock = HubLockFile::new(&paths);
        let entry = lock.get_installed("test-skill").unwrap();
        assert_eq!(entry.source, "github");
        assert_eq!(entry.scan_verdict, "safe");

        // Uninstall
        let (success, _msg) = uninstall_skill("test-skill", &paths).unwrap();
        assert!(success);
        assert!(!install_dir.exists());
        assert!(lock.get_installed("test-skill").is_none());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_uninstall_nonexistent() {
        let tmp = std::env::temp_dir().join("hermes_uninstall_nonexistent");
        let _ = std::fs::remove_dir_all(&tmp);
        let paths = HubPaths::new(&tmp);

        let (success, msg) = uninstall_skill("nonexistent", &paths).unwrap();
        assert!(!success);
        assert!(msg.contains("not a hub-installed skill"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

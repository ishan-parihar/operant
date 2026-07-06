//! Skill marketplace — remote registry index fetch + search + install +
//! version comparison.
//!
//! Closes the ponytail-audit gap B8: "Skill marketplace missing.
//! `skills_guard.rs` advertises 'every skill downloaded from a registry
//! passes through this scanner' but there is NO registry fetch/install/
//! search code — only SkillManager (local dir scan) + SkillManageTool
//! (create/patch/delete local)."
//!
//! ## Design
//!
//! The registry is a JSON index hosted at a configurable URL (defaults to
//! `https://raw.githubusercontent.com/ishan-parihar/operant-skills/main/index.json`).
//! The index is a flat array of `SkillRegistryEntry` objects:
//!
//! ```json
//! [
//!   {
//!     "name": "rust-cargo-audit",
//!     "description": "Run cargo-audit on the current project",
//!     "version": "1.2.0",
//!     "author": "ishan-parihar",
//!     "license": "MIT",
//!     "tags": ["rust", "security"],
//!     "category": "development",
//!     "homepage": "https://github.com/ishan-parihar/operant-skills/tree/main/skills/rust-cargo-audit",
//!     "download_url": "https://raw.githubusercontent.com/ishan-parihar/operant-skills/main/skills/rust-cargo-audit/SKILL.md",
//!     "checksum": "sha256:abcdef..."
//!   }
//! ]
//! ```
//!
//! The index is fetched lazily on first search/install, cached to
//! `~/.operant/skill-registry.json` with a 1-hour TTL, and re-fetched
//! if expired or missing.
//!
//! ## Security
//!
//! Every downloaded SKILL.md passes through `skills_guard::scan_skill`
//! before being written to disk. If the scan returns a Dangerous verdict,
//! the install is refused and the partial file is deleted.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Default registry index URL. Points at the operant-skills GitHub repo.
const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/ishan-parihar/operant-skills/main/index.json";

/// Cache TTL — re-fetch the registry if the local copy is older than this.
const CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

/// A single entry in the skill registry index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRegistryEntry {
    /// Unique skill name (matches the directory name when installed)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Semantic version string (e.g. "1.2.0")
    pub version: String,
    /// Author/handle
    #[serde(default)]
    pub author: Option<String>,
    /// License identifier (SPDX)
    #[serde(default)]
    pub license: Option<String>,
    /// Tags for categorization + search
    #[serde(default)]
    pub tags: Vec<String>,
    /// Category (e.g. "development", "writing", "automation")
    #[serde(default)]
    pub category: Option<String>,
    /// Homepage URL (GitHub repo, docs, etc.)
    #[serde(default)]
    pub homepage: Option<String>,
    /// Direct download URL for the SKILL.md file
    pub download_url: String,
    /// Optional SHA-256 checksum of the SKILL.md content
    /// (format: "sha256:<hex>"). If present, the download is verified.
    #[serde(default)]
    pub checksum: Option<String>,
}

/// Cached registry index with fetch timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRegistry {
    /// Fetched-at Unix timestamp (seconds)
    fetched_at: u64,
    /// The registry entries
    entries: Vec<SkillRegistryEntry>,
    /// The URL we fetched from (for cache invalidation if URL changes)
    source_url: String,
}

impl Default for CachedRegistry {
    fn default() -> Self {
        Self {
            fetched_at: 0,
            entries: Vec::new(),
            source_url: String::new(),
        }
    }
}

/// The marketplace client. Stateless — all state is in the cache file.
pub struct SkillMarketplace {
    /// Registry index URL (overridable via OPERANT_SKILL_REGISTRY env var)
    registry_url: String,
    /// Cache file path (~/.operant/skill-registry.json)
    cache_path: PathBuf,
    /// HTTP client with a 30s timeout
    client: reqwest::Client,
}

impl SkillMarketplace {
    /// Create a new marketplace client. Uses the default registry URL
    /// unless `OPERANT_SKILL_REGISTRY` env var is set.
    pub fn new() -> Self {
        let registry_url = std::env::var("OPERANT_SKILL_REGISTRY")
            .unwrap_or_else(|_| DEFAULT_REGISTRY_URL.to_string());
        let cache_path = dirs::home_dir()
            .map(|h| h.join(".operant").join("skill-registry.json"))
            .unwrap_or_else(|| PathBuf::from(".skill-registry.json"));
        Self {
            registry_url,
            cache_path,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(format!("operant/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Fetch the registry index, using the cache if fresh.
    pub async fn fetch_index(&self) -> anyhow::Result<Vec<SkillRegistryEntry>> {
        // Try the cache first.
        if let Some(cached) = self.load_cache() {
            if cached.source_url == self.registry_url
                && !is_expired(cached.fetched_at, CACHE_TTL)
            {
                tracing::debug!(
                    entries = cached.entries.len(),
                    "Skill registry: using cached index"
                );
                return Ok(cached.entries);
            }
        }

        // Cache miss or stale — fetch fresh.
        tracing::info!(url = %self.registry_url, "Skill registry: fetching index");
        let response = self
            .client
            .get(&self.registry_url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Registry fetch failed: {e}"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "Registry fetch returned {} {}",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("")
            );
        }
        let entries: Vec<SkillRegistryEntry> = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Registry parse failed: {e}"))?;
        tracing::info!(entries = entries.len(), "Skill registry: fetched fresh index");

        // Save to cache.
        let cached = CachedRegistry {
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            entries: entries.clone(),
            source_url: self.registry_url.clone(),
        };
        self.save_cache(&cached);

        Ok(entries)
    }

    /// Force a fresh fetch (ignore cache).
    pub async fn refresh_index(&self) -> anyhow::Result<Vec<SkillRegistryEntry>> {
        let _ = std::fs::remove_file(&self.cache_path);
        self.fetch_index().await
    }

    /// Search the registry index by query string.
    pub async fn search(&self, query: &str) -> anyhow::Result<Vec<SkillRegistryEntry>> {
        let entries = self.fetch_index().await?;
        let q = query.to_lowercase();
        let q_words: Vec<&str> = q.split_whitespace().collect();

        let mut scored: Vec<(i32, SkillRegistryEntry)> = entries
            .into_iter()
            .map(|e| {
                let score = score_entry(&e, &q, &q_words);
                (score, e)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        Ok(scored.into_iter().map(|(_, e)| e).collect())
    }

    /// Look up a single skill by exact name.
    pub async fn get(&self, name: &str) -> anyhow::Result<Option<SkillRegistryEntry>> {
        let entries = self.fetch_index().await?;
        Ok(entries.into_iter().find(|e| e.name == name))
    }

    /// Download + install a skill from the registry.
    pub async fn install(
        &self,
        name: &str,
        skills_dir: &Path,
        force: bool,
    ) -> anyhow::Result<PathBuf> {
        let entry = self
            .get(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found in registry", name))?;

        tracing::info!(url = %entry.download_url, "Downloading SKILL.md");
        let content = self
            .client
            .get(&entry.download_url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Download failed: {e}"))?
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Download body failed: {e}"))?;

        // Verify checksum if present.
        if let Some(ref checksum) = entry.checksum {
            verify_checksum(&content, checksum)?;
        }

        // Security scan (unless --force).
        if !force {
            scan_downloaded_skill(name, &content)?;
        } else {
            tracing::warn!(
                skill = %name,
                "Security scan skipped (--force). Only use --force for skills you trust."
            );
        }

        // Write to skills_dir/<name>/SKILL.md.
        let skill_dir = skills_dir.join(name);
        std::fs::create_dir_all(&skill_dir)
            .map_err(|e| anyhow::anyhow!("Failed to create skill dir: {e}"))?;
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md, &content)
            .map_err(|e| anyhow::anyhow!("Failed to write SKILL.md: {e}"))?;

        tracing::info!(path = %skill_md.display(), "Skill installed");
        Ok(skill_md)
    }

    /// Compare an installed skill's version against the registry's latest.
    pub async fn check_for_update(
        &self,
        name: &str,
        installed_version: &str,
    ) -> anyhow::Result<Option<SkillRegistryEntry>> {
        let Some(entry) = self.get(name).await? else {
            return Ok(None);
        };
        if is_newer_version(&entry.version, installed_version) {
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }

    fn load_cache(&self) -> Option<CachedRegistry> {
        let content = std::fs::read_to_string(&self.cache_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save_cache(&self, cached: &CachedRegistry) {
        if let Some(parent) = self.cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self.cache_path.with_extension("json.tmp");
        if let Ok(json) = serde_json::to_string_pretty(cached) {
            if std::fs::write(&tmp, &json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.cache_path);
            }
        }
    }
}

impl Default for SkillMarketplace {
    fn default() -> Self {
        Self::new()
    }
}

fn is_expired(fetched_at: u64, ttl: Duration) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(fetched_at) > ttl.as_secs()
}

fn score_entry(entry: &SkillRegistryEntry, q: &str, q_words: &[&str]) -> i32 {
    let mut score = 0;
    let name_lower = entry.name.to_lowercase();
    let desc_lower = entry.description.to_lowercase();
    let tags_lower: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();
    let cat_lower = entry.category.as_deref().unwrap_or("").to_lowercase();

    // Name match — use else-if so only the highest name match counts.
    if name_lower == q {
        score += 100;
    } else if name_lower.starts_with(q) {
        score += 50;
    } else if name_lower.contains(q) {
        score += 20;
    }
    if desc_lower.contains(q) {
        score += 10;
    }
    if tags_lower.iter().any(|t| t == q) {
        score += 15;
    }
    if !cat_lower.is_empty() && cat_lower == q {
        score += 5;
    }
    // Per-word matching — only for multi-word queries (single-word is
    // already covered by the name/desc/tag checks above).
    if q_words.len() > 1 {
        for word in q_words {
            if name_lower.contains(word) {
                score += 5;
            }
            if desc_lower.contains(word) {
                score += 2;
            }
            if tags_lower.iter().any(|t| t.contains(word)) {
                score += 3;
            }
        }
    }
    score
}

fn verify_checksum(content: &str, expected: &str) -> anyhow::Result<()> {
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let actual = hex::encode(hasher.finalize());
    if actual != expected {
        anyhow::bail!(
            "Checksum mismatch: expected sha256:{expected}, got sha256:{actual}"
        );
    }
    Ok(())
}

fn scan_downloaded_skill(name: &str, content: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let temp_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("Failed to create temp dir for scan: {e}"))?;
    let skill_dir = temp_dir.path().join(name);
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create skill dir for scan: {e}"))?;
    let skill_md = skill_dir.join("SKILL.md");
    let mut f = std::fs::File::create(&skill_md)
        .map_err(|e| anyhow::anyhow!("Failed to create SKILL.md for scan: {e}"))?;
    f.write_all(content.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to write SKILL.md for scan: {e}"))?;
    drop(f);

    let scanner = crate::skills_guard::GuardScanner::new();
    let result = scanner.scan_directory(&skill_dir, "registry");
    match result.scan_verdict {
        crate::skills_guard::ScanVerdict::Safe => Ok(()),
        crate::skills_guard::ScanVerdict::Caution => {
            tracing::warn!(
                skill = %name,
                findings = result.findings.len(),
                "skills_guard Caution verdict — installing anyway"
            );
            Ok(())
        }
        crate::skills_guard::ScanVerdict::Dangerous => {
            let findings: Vec<String> = result
                .findings
                .iter()
                .map(|f| format!("{}: {}", f.pattern_id, f.description))
                .collect();
            anyhow::bail!(
                "skills_guard BLOCKED installation of '{}': {} finding(s). Use --force to override. Findings: {}",
                name,
                result.findings.len(),
                findings.join("; ")
            );
        }
    }
}

fn is_newer_version(a: &str, b: &str) -> bool {
    let a_parts: Vec<u32> = a
        .split('-')
        .next()
        .unwrap_or(a)
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let b_parts: Vec<u32> = b
        .split('-')
        .next()
        .unwrap_or(b)
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let max_len = a_parts.len().max(b_parts.len());
    for i in 0..max_len {
        let av = a_parts.get(i).copied().unwrap_or(0);
        let bv = b_parts.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    a > b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_exact_name_match() {
        let entry = SkillRegistryEntry {
            name: "rust-audit".to_string(),
            description: "Run cargo audit".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            license: None,
            tags: vec!["rust".to_string()],
            category: Some("development".to_string()),
            homepage: None,
            download_url: "https://example.com/SKILL.md".to_string(),
            checksum: None,
        };
        assert_eq!(score_entry(&entry, "rust-audit", &["rust-audit"]), 100);
    }

    #[test]
    fn test_score_partial_match() {
        let entry = SkillRegistryEntry {
            name: "rust-cargo-audit".to_string(),
            description: "Run cargo audit on Rust projects".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            license: None,
            tags: vec!["rust".to_string(), "security".to_string()],
            category: Some("development".to_string()),
            homepage: None,
            download_url: "https://example.com/SKILL.md".to_string(),
            checksum: None,
        };
        let score = score_entry(&entry, "rust", &["rust"]);
        assert!(score > 0);
    }

    #[test]
    fn test_score_no_match() {
        let entry = SkillRegistryEntry {
            name: "rust-audit".to_string(),
            description: "Run cargo audit".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            license: None,
            tags: vec![],
            category: None,
            homepage: None,
            download_url: "https://example.com/SKILL.md".to_string(),
            checksum: None,
        };
        assert_eq!(score_entry(&entry, "python", &["python"]), 0);
    }

    #[test]
    fn test_is_newer_version_basic() {
        assert!(is_newer_version("1.2.0", "1.1.0"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_is_newer_version_short_forms() {
        assert!(is_newer_version("1.2", "1.1"));
        assert!(is_newer_version("2", "1"));
        assert!(is_newer_version("1.2.3", "1.2"));
    }

    #[test]
    fn test_verify_checksum_valid() {
        let content = "hello world";
        let checksum =
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_checksum(content, checksum).is_ok());
    }

    #[test]
    fn test_verify_checksum_invalid() {
        let content = "hello world";
        let checksum = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_checksum(content, checksum).is_err());
    }

    #[test]
    fn test_is_expired_fresh() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        assert!(!is_expired(now, Duration::from_secs(3600)));
    }

    #[test]
    fn test_is_expired_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let one_hour_ago = now.saturating_sub(3700);
        assert!(is_expired(one_hour_ago, Duration::from_secs(3600)));
    }
}

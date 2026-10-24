//! Cached, gitignore-aware recursive directory walk — the discovery half of
//! the OMP-style `@` file mention (plan 2026-10-24, M2).
//!
//! Oh-my-pi's `pi-walker` fronts a parallel directory walk with a shared
//! scan cache keyed `(root, walk options)`: TTL 1 s (`FS_SCAN_CACHE_TTL_MS`
//! override), empty results rechecked after 200 ms, entry/byte caps, and
//! `invalidate_path` on writes. There is NO persistent index database — the
//! cache IS the index, and it is always at most one TTL away from the
//! filesystem. This module ports that shape (single-threaded walk v1: the
//! top-K heap + deadline in `score.rs` make one thread adequate for
//! repo-scale trees; parallelism is a measured follow-up, not a default).
//!
//! `.git` is never yielded (OMP filters it from results); other dot-entries
//! are walked but hidden-filtered by the caller's policy, so a query
//! starting with `.` can still surface them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// One discovered filesystem entry, relative to the walk root, with the
/// fields the scorer and the popup need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredEntry {
    /// Path relative to the walk root, `/`-separated.
    pub rel_path: String,
    /// Directories carry the OMP `+10` ranking bonus and render with a
    /// trailing `/`.
    pub is_dir: bool,
    /// Depth from the root (0 = the root's direct children). Shallower
    /// entries win score ties (OMP `cmp_candidate`).
    pub depth: usize,
}

/// Walk policy. Gitignore rules (repo, global, exclude) are always on —
/// the OMP autocomplete profile passes `gitignore: true`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WalkOptions {
    /// Include dot-entries while walking. `.git` is skipped regardless;
    /// whether hidden entries are OFFERED is the caller's policy.
    pub include_hidden: bool,
}

/// Scan-cache TTL (OMP default 1 s; `FS_SCAN_CACHE_TTL_MS` override).
fn cache_ttl() -> Duration {
    static TTL: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TTL.get_or_init(|| {
        Duration::from_millis(
            std::env::var("FS_SCAN_CACHE_TTL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_000),
        )
    })
}

/// An EMPTY cached scan is rewalked after this much time instead of the
/// full TTL — an empty result is usually a directory mid-creation (OMP
/// `EMPTY_RECHECK_MS`).
fn empty_recheck() -> Duration {
    static RECHECK: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *RECHECK.get_or_init(|| {
        Duration::from_millis(
            std::env::var("FS_SCAN_CACHE_EMPTY_RECHECK_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(200),
        )
    })
}

/// Maximum cached roots. Small: each entry is one `Arc`'d vector.
const MAX_CACHE_ROOTS: usize = 16;

/// Hard ceiling on entries per walk so a pathological root (a home
/// directory, a vendored mono-tree) cannot exhaust memory. OMP bounds by
/// deadline and byte budget; the entry cap is the equivalent here.
const MAX_WALK_ENTRIES: usize = 65_536;

/// Walk heartbeat: deadline checked every N yielded entries (OMP
/// `HEARTBEAT_INTERVAL`).
const HEARTBEAT_INTERVAL: usize = 128;

struct CacheEntry {
    created_at: Instant,
    entries: Arc<Vec<DiscoveredEntry>>,
}

static SCAN_CACHE: std::sync::LazyLock<Mutex<HashMap<(PathBuf, WalkOptions), CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Walk `root` (or fetch the cached walk when fresh enough). The returned
/// vector is shared via `Arc` — scoring borrows it; nobody mutates it.
///
/// Deadlines and the entry cap make the result potentially PARTIAL: a huge
/// root beyond `MAX_WALK_ENTRIES` or slower than the caller's budget is
/// truncated, exactly like OMP's timed-out fuzzy walks.
pub fn discover(root: &Path, opts: WalkOptions, deadline: Instant) -> Arc<Vec<DiscoveredEntry>> {
    let key = (root.to_path_buf(), opts);
    let ttl = cache_ttl();
    let now = Instant::now();
    {
        let cache = SCAN_CACHE.lock();
        if let Some(hit) = cache.get(&key) {
            let fresh = hit.entries.is_empty()
                && now.duration_since(hit.created_at) < empty_recheck()
                || !hit.entries.is_empty() && now.duration_since(hit.created_at) < ttl;
            if fresh {
                return Arc::clone(&hit.entries);
            }
        }
    }

    let entries = Arc::new(walk_uncached(root, opts, deadline));
    let mut cache = SCAN_CACHE.lock();
    // Insert-or-refresh LRU-ish behavior: reinserting the key moves nothing
    // (HashMap has no order), but the cap evicts arbitrary stale roots when
    // full — prefer dropping the OLDEST entry, which is the one most likely
    // past its TTL anyway.
    if cache.len() >= MAX_CACHE_ROOTS
        && let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, e)| e.created_at)
            .map(|(k, _)| k.clone())
    {
        cache.remove(&oldest);
    }
    cache.insert(
        key,
        CacheEntry {
            created_at: now,
            entries: Arc::clone(&entries),
        },
    );
    entries
}

/// Drop every cached walk touching `path` (OMP `invalidate_path`): a
/// created/deleted/renamed file makes its root's cache suspect.
pub fn invalidate_path(path: &Path) {
    let mut cache = SCAN_CACHE.lock();
    cache.retain(|(root, _), _| !path.starts_with(root) && !root.starts_with(path));
}

fn walk_uncached(root: &Path, opts: WalkOptions, deadline: Instant) -> Vec<DiscoveredEntry> {
    let mut out = Vec::new();
    let mut walker = ignore::WalkBuilder::new(root);
    walker
        .hidden(!opts.include_hidden)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .follow_links(false)
        // require_git=false: a non-git directory still respects a plain
        // .gitignore file, matching how the user expects ignores to apply.
        .require_git(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".git"
        });
    let mut yielded = 0usize;
    for entry in walker.build() {
        yielded += 1;
        if yielded.is_multiple_of(HEARTBEAT_INTERVAL) && Instant::now() >= deadline {
            break;
        }
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        out.push(DiscoveredEntry {
            rel_path: rel_str.replace('\\', "/"),
            is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
            depth: entry.depth(),
        });
        if out.len() >= MAX_WALK_ENTRIES {
            break;
        }
    }
    out
}

#[cfg(test)]
mod walker_tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant-fd-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp fixture dir");
        dir
    }

    fn write(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, contents).expect("write");
    }

    #[test]
    fn walk_respects_gitignore_and_skips_git() {
        let root = temp_root("gitignore");
        write(&root, ".gitignore", "ignored-dir/\n*.tmp\n");
        write(&root, "src/main.rs", "fn main() {}");
        write(&root, "ignored-dir/x.rs", "// ignored");
        write(&root, "keep.tmp", "ignored by pattern");
        write(&root, ".git/config", "[core]");
        let entries = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            far_deadline(),
        );
        let paths: Vec<&str> = entries.iter().map(|e| e.rel_path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"), "kept: {paths:?}");
        assert!(
            !paths.iter().any(|p| p.starts_with("ignored-dir")),
            "{paths:?}"
        );
        assert!(!paths.iter().any(|p| p.contains(".tmp")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with(".git")), "{paths:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_marks_dirs_and_depths() {
        let root = temp_root("depths");
        write(&root, "a/b/c.txt", "x");
        let entries = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            far_deadline(),
        );
        let dir = entries.iter().find(|e| e.rel_path == "a/b").expect("a/b");
        assert!(dir.is_dir && dir.depth == 2);
        let file = entries
            .iter()
            .find(|e| e.rel_path == "a/b/c.txt")
            .expect("c");
        assert!(!file.is_dir && file.depth == 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hidden_walk_policy_and_git_never() {
        let root = temp_root("hidden");
        write(&root, ".dotfile", "x");
        write(&root, ".git/HEAD", "ref");
        let visible = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            far_deadline(),
        );
        assert!(!visible.iter().any(|e| e.rel_path == ".dotfile"));
        let hidden = discover(
            &root,
            WalkOptions {
                include_hidden: true,
            },
            far_deadline(),
        );
        assert!(hidden.iter().any(|e| e.rel_path == ".dotfile"));
        assert!(!hidden.iter().any(|e| e.rel_path.contains(".git")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn invalidation_drops_the_root() {
        let root = temp_root("invalidate");
        write(&root, "a.txt", "x");
        invalidate_all_for_tests();
        let first = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            far_deadline(),
        );
        assert!(first.iter().any(|e| e.rel_path == "a.txt"));
        invalidate_path(&root);
        write(&root, "b.txt", "y");
        let second = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            far_deadline(),
        );
        assert!(
            second.iter().any(|e| e.rel_path == "b.txt"),
            "invalidation must force a rewalk"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A deadline already in the past still yields SOME entries: the
    /// heartbeat check runs every 128 yields, so the first slice survives
    /// (OMP's timed-out walks return partial results, not errors).
    #[test]
    fn expired_deadline_returns_partial_not_nothing() {
        let root = temp_root("deadline");
        for i in 0..50 {
            write(&root, &format!("f{i}.txt"), "x");
        }
        let entries = discover(
            &root,
            WalkOptions {
                include_hidden: false,
            },
            Instant::now(),
        );
        assert!(!entries.is_empty(), "partial result expected");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn far_deadline() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    /// Test helper: the cache is process-global, so tests that assert on
    /// freshness must start from a known-empty state.
    pub(super) fn invalidate_all_for_tests() {
        SCAN_CACHE.lock().clear();
    }
}

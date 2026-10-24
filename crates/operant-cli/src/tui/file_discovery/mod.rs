//! OMP-style file discovery for the `@` mention popup (plan 2026-10-24).
//!
//! Two result sources, matching oh-my-pi's autocomplete:
//! - [`list_dir`]: the IMMEDIATE directory listing (fast, synchronous —
//!   `readdir` of one directory, `.git` skipped, symlinks stat'd, hidden
//!   per policy). This is the interim/partial tier: it renders instantly.
//! - [`fuzzy_top`]: the RECURSIVE tier — a cached, gitignore-aware walk of
//!   the project root scored against the query (see [`walker`] and
//!   [`score`]).
//!
//! There is no persistent index database; the walk cache (TTL 1 s) is the
//! index, exactly the OMP mechanism.

pub mod score;
pub mod walker;

pub use score::ScoredEntry;
pub use walker::{DiscoveredEntry, WalkOptions, discover, invalidate_path};

use std::path::Path;

/// One entry of an immediate directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedEntry {
    /// File name only (no directory part).
    pub name: String,
    /// `name + '/'` is appended by the caller for display; the flag also
    /// decides chaining behavior on accept.
    pub is_dir: bool,
    /// Symlinks are reported as links (OMP stats symlinks to learn whether
    /// they point at a directory; broken links are dropped).
    pub is_symlink: bool,
}

/// OMP caps the fuzzy tier at 100 results (`maxResults: 100`).
pub const FUZZY_RESULT_LIMIT: usize = 100;

/// Deadline budget for one fuzzy discovery pass (OMP
/// `AT_FUZZY_TIMEOUT_MS = 1_500`).
pub const FUZZY_DEADLINE_MS: u64 = 1_500;

/// List `dir`'s entries whose names start with `prefix` (case-insensitive).
///
/// Policy (ported from the composer's previous inline listing + OMP):
/// - `.git` never appears;
/// - hidden entries appear when `show_hidden` is on OR the user's own
///   prefix starts with `.` (typing `@.serv` is an explicit ask);
/// - a directory whose only contents would all be hidden is still listed
///   (the drill-down decides what to show — filtering here would hide
///   `~/` style roots behind invisible children);
/// - symlinks are stat'd: a link to a directory lists as a directory; a
///   broken link is dropped.
pub fn list_dir(dir: &Path, prefix: &str, show_hidden: bool, limit: usize) -> Vec<ListedEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix_lower = prefix.to_lowercase();
    let mut out: Vec<ListedEntry> = entries
        .filter_map(|e| {
            let entry = e.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            if !name.to_lowercase().starts_with(&prefix_lower) {
                return None;
            }
            if !show_hidden
                && name.starts_with('.')
                && !prefix.starts_with('.')
                && !prefix_lower.starts_with('.')
            {
                return None;
            }
            let ft = entry.file_type().ok()?;
            let (is_dir, is_symlink) = if ft.is_symlink() {
                // Stat the target: broken links vanish, dir-links chain.
                match std::fs::metadata(entry.path()) {
                    Ok(md) => (md.is_dir(), true),
                    Err(_) => return None,
                }
            } else {
                (ft.is_dir(), false)
            };
            Some(ListedEntry {
                name,
                is_dir,
                is_symlink,
            })
        })
        .collect();
    // Directories first, then lexicographic (the OMP listing sort).
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out.truncate(limit);
    out
}

/// Top fuzzy matches for `query` across the recursive walk of `root`.
///
/// `deadline` bounds the walk (partial results are returned, not errors).
/// Hidden entries participate in the walk only per `opts.include_hidden`
/// (the project walk passes the popup's hidden policy).
pub fn fuzzy_top(
    root: &Path,
    query: &str,
    limit: usize,
    opts: WalkOptions,
    deadline: std::time::Instant,
) -> Vec<ScoredEntry> {
    if query.is_empty() {
        return Vec::new();
    }
    let entries = discover(root, opts, deadline);
    rank_bounded(&entries, query, limit, deadline)
}

/// [`score::rank_entries`] with a hidden-path filter applied first: the
/// walk may include hidden entries, but the popup hides them unless the
/// query itself starts with a dot (the same policy as [`list_dir`]).
fn rank_bounded(
    entries: &[DiscoveredEntry],
    query: &str,
    limit: usize,
    _deadline: std::time::Instant,
) -> Vec<ScoredEntry> {
    let dot_query = query.starts_with('.');
    let visible: Vec<DiscoveredEntry> = entries
        .iter()
        .filter(|e| dot_query || !path_has_hidden_component(&e.rel_path))
        .cloned()
        .collect();
    score::rank_entries(&visible, query, limit)
}

/// Whether any `/`-separated component of `rel_path` starts with `.`.
fn path_has_hidden_component(rel_path: &str) -> bool {
    rel_path.split('/').any(|c| c.starts_with('.'))
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant-fd-list-{}-{}-{}",
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

    #[test]
    fn listing_skips_git_and_orders_dirs_first() {
        let root = temp_root("order");
        std::fs::create_dir_all(root.join("zed")).expect("mkdir");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        std::fs::write(root.join("alpha.txt"), "x").expect("write");
        std::fs::write(root.join(".git/HEAD"), "x").expect("write");
        let listed = list_dir(&root, "", false, 10);
        let names: Vec<&str> = listed.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names.first(), Some(&"zed"), "dirs first: {names:?}");
        assert!(!names.contains(&".git"));
        assert!(names.contains(&"alpha.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hidden_policy_dot_prefix_unlocks() {
        let root = temp_root("hiddenlist");
        std::fs::write(root.join(".serv"), "x").expect("write");
        assert!(
            list_dir(&root, "", false, 10).is_empty(),
            "hidden by default"
        );
        assert!(
            list_dir(&root, ".s", false, 10)
                .iter()
                .any(|l| l.name == ".serv"),
            "dot prefix unlocks hidden"
        );
        assert!(
            list_dir(&root, "", true, 10)
                .iter()
                .any(|l| l.name == ".serv"),
            "config unlocks hidden"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn symlinked_dir_lists_as_dir_broken_link_drops() {
        let root = temp_root("links");
        let target = root.join("real");
        std::fs::create_dir_all(&target).expect("mkdir");
        std::os::unix::fs::symlink(&target, root.join("dirlink")).expect("symlink");
        std::os::unix::fs::symlink(root.join("nope"), root.join("broken")).expect("broken");
        let listed = list_dir(&root, "", false, 10);
        let link = listed
            .iter()
            .find(|l| l.name == "dirlink")
            .expect("dirlink");
        assert!(link.is_dir && link.is_symlink);
        assert!(
            !listed.iter().any(|l| l.name == "broken"),
            "broken link dropped"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

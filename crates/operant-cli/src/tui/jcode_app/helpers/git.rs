// Vendored from jcode (crates/jcode-tui/src/tui/app/helpers.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] the four git-gathering fns the ported info widget's tests
// reach through `jcode_app::app::helpers` (info_widget_commits.rs:260,
// info_widget_git.rs:422/:560/:575) live in this leaf because `app.rs:21`
// re-exports `jcode_app::helpers` wholesale — which today carries only
// `model_names`. Upstream `tui/app/helpers.rs` is 1682 lines; only the git
// family is ported, verbatim: `gather_git_info_inner` (:1405),
// `gather_git_info_in` (:1411), `parse_recent_commits` (:1581), `parse_numstat`
// (:1622), `count_text_lines` (:1645), `porcelain_status_letter` (:1667), and
// the `GitInfo`/`RecentCommit`/`DirtyFile` constructors they call.
// [port-decision] upstream reaches those three data types through
// `crate::tui::info_widget::*`; here they are already ported into
// `crate::tui::jcode_app::info_widget` (info_widget/mod.rs:571/:594/:608), so
// this leaf imports them instead of duplicating the definitions.
use crate::tui::jcode_app::info_widget::{DirtyFile, GitInfo, RecentCommit};

pub(crate) fn gather_git_info_inner() -> Option<GitInfo> {
    gather_git_info_in(None)
}

/// Git status for `dir` (or the process working directory when `None`).
pub(crate) fn gather_git_info_in(dir: Option<&std::path::Path>) -> Option<GitInfo> {
    let git = || {
        let mut cmd = std::process::Command::new("git");
        if let Some(dir) = dir {
            cmd.current_dir(dir);
        }
        cmd
    };

    let in_repo = git()
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !in_repo {
        return None;
    }

    let branch = git()
        .args(["branch", "--show-current"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if b.is_empty() { None } else { Some(b) }
            } else {
                None
            }
        })
        .unwrap_or_else(|| "HEAD".to_string());

    let mut modified = 0;
    let mut staged = 0;
    let mut untracked = 0;
    let mut all_files: Vec<DirtyFile> = Vec::new();

    let repo_root = git()
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));

    if let Ok(output) = git()
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        && output.status.success()
    {
        let status = String::from_utf8_lossy(&output.stdout);
        for line in status.lines() {
            if line.len() < 3 {
                continue;
            }
            let index_status = line.as_bytes()[0];
            let worktree_status = line.as_bytes()[1];
            let file_path = line[3..].to_string();

            if index_status == b'?' {
                untracked += 1;
            } else {
                if index_status != b' ' && index_status != b'?' {
                    staged += 1;
                }
                if worktree_status != b' ' && worktree_status != b'?' {
                    modified += 1;
                }
            }
            all_files.push(DirtyFile::new(
                porcelain_status_letter(index_status, worktree_status),
                file_path,
            ));
        }
    }

    // Line counts: tracked files from one numstat against HEAD (staged plus
    // unstaged), untracked files by counting their lines.
    let numstat = git()
        .args(["diff", "--numstat", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_numstat(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let mut added_total = 0usize;
    let mut removed_total = 0usize;
    for file in &mut all_files {
        let key = file
            .path
            .rsplit(" -> ")
            .next()
            .unwrap_or(&file.path)
            .trim_matches('"');
        let abs = repo_root.as_ref().map(|root| root.join(key));
        if file.status == '?' {
            file.added = abs.as_deref().and_then(count_text_lines);
            file.removed = file.added.map(|_| 0);
        } else if let Some(&(a, r)) = numstat.get(key) {
            file.added = a;
            file.removed = r;
        }
        added_total += file.added.unwrap_or(0);
        removed_total += file.removed.unwrap_or(0);
        file.modified_at = abs
            .as_deref()
            .and_then(|p| std::fs::symlink_metadata(p).ok())
            .and_then(|m| m.modified().ok());
    }
    // Newest first, so the file being worked on stays visible under the cap.
    // Deleted files have no mtime and sort last.
    all_files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    let dirty_total = all_files.len();
    all_files.truncate(10);
    let dirty_files = all_files;

    let (ahead, behind) = git()
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let parts: Vec<&str> = text.split('\t').collect();
                if parts.len() == 2 {
                    let a = parts[0].parse::<usize>().unwrap_or(0);
                    let b = parts[1].parse::<usize>().unwrap_or(0);
                    Some((a, b))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap_or((0, 0));

    let recent_commits = git()
        .args([
            "log",
            "-n",
            "8",
            "--shortstat",
            "--format=%x1e%h%x1f%ct%x1f%s",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_recent_commits(&String::from_utf8_lossy(&o.stdout), ahead))
        .unwrap_or_default();

    Some(GitInfo {
        branch,
        modified,
        staged,
        untracked,
        ahead,
        behind,
        dirty_files,
        dirty_total,
        added_total,
        removed_total,
        repo_root,
        recent_commits,
    })
}

/// Parse `git log --shortstat --format=%x1e%h%x1f%ct%x1f%s`. The first
/// `ahead` commits are the ones not yet on the upstream.
pub(crate) fn parse_recent_commits(text: &str, ahead: usize) -> Vec<RecentCommit> {
    text.split('\x1e')
        .filter(|record| !record.trim().is_empty())
        .enumerate()
        .filter_map(|(index, record)| {
            let mut lines = record.lines();
            let mut fields = lines.next()?.splitn(3, '\x1f');
            let hash = fields.next()?.trim().to_string();
            let timestamp = fields.next()?.trim().parse().ok()?;
            let subject = fields.next().unwrap_or("").trim().to_string();
            let (mut added, mut removed) = (None, None);
            // " 3 files changed, 12 insertions(+), 4 deletions(-)"
            for part in lines.flat_map(|l| l.split(',')) {
                let part = part.trim();
                let n = part.split_whitespace().next().and_then(|n| n.parse().ok());
                if part.contains("insertion") {
                    added = n;
                } else if part.contains("deletion") {
                    removed = n;
                } else if part.contains("changed") {
                    added = added.or(Some(0));
                    removed = removed.or(Some(0));
                }
            }

            Some(RecentCommit {
                hash,
                subject,
                timestamp,
                unpushed: index < ahead,
                added,
                removed,
            })
        })
        .collect()
}

/// Parse `git diff --numstat` into path -> (added, removed). Binary files
/// report `-` and map to `None` counts.
pub(crate) fn parse_numstat(
    text: &str,
) -> std::collections::HashMap<String, (Option<usize>, Option<usize>)> {
    let mut out = std::collections::HashMap::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        // Renames: `old => new` or `dir/{old => new}/file`.
        let path = if let (Some(open), Some(close)) = (path.find('{'), path.find('}')) {
            let inner = &path[open + 1..close];
            let new = inner.rsplit(" => ").next().unwrap_or(inner);
            format!("{}{}{}", &path[..open], new, &path[close + 1..]).replace("//", "/")
        } else {
            path.rsplit(" => ").next().unwrap_or(path).to_string()
        };
        out.insert(path, (a.parse().ok(), r.parse().ok()));
    }
    out
}

/// Line count of a small text file, for untracked files. Large or binary
/// files return `None` so the widget shows no count rather than a bogus one.
fn count_text_lines(path: &std::path::Path) -> Option<usize> {
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
    Some(if bytes.last().is_some_and(|&b| b != b'\n') {
        lines + 1
    } else {
        lines
    })
}

/// Collapse a porcelain `XY` pair into the single letter
/// the widget shows. Conflicts win, then the worktree side (what the user is editing),
/// then the index side.
pub(crate) fn porcelain_status_letter(index: u8, worktree: u8) -> char {
    if index == b'?' {
        return '?';
    }
    if index == b'U' || worktree == b'U' || (index == b'A' && worktree == b'A') {
        return 'U';
    }
    let pick = if worktree != b' ' { worktree } else { index };
    match pick {
        b'M' | b'T' => 'M',
        b'A' => 'A',
        b'D' => 'D',
        b'R' | b'C' => 'R',
        _ => 'M',
    }
}

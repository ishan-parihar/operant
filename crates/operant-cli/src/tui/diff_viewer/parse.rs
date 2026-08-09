// diff_viewer/parse.rs — Unified-diff parsing and git diff loading.
//
// Extracted from the diff_viewer.rs monolith. load_git_diff shells out to
// `git diff HEAD`, parse_unified_diff turns raw text into FileDiffStats.

use super::*;

pub fn load_git_diff(project_root: &std::path::Path) -> Vec<FileDiffStats> {
    let output = std::process::Command::new("git")
        .args(["diff", "HEAD", "--unified=3"])
        .current_dir(project_root)
        .output();

    let text = match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
        Ok(_out) => {
            // Try just `git diff` (no HEAD) for unstaged changes
            let out2 = std::process::Command::new("git")
                .args(["diff", "--unified=3"])
                .current_dir(project_root)
                .output();
            match out2 {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => return Vec::new(),
            }
        }
        Err(_) => return Vec::new(),
    };

    parse_unified_diff(&text)
}

// (iter-209: build_turn_diff deleted — took a &FileHistory stub that
// always returned empty snapshots. /changes now uses the git-diff path.
// To re-implement per-turn diffs, wire to a real snapshot store in core.)

/// Parse unified diff text into `Vec<FileDiffStats>`.
pub fn parse_unified_diff(text: &str) -> Vec<FileDiffStats> {
    let mut files: Vec<FileDiffStats> = Vec::new();
    let mut current_file: Option<FileDiffStats> = None;
    let mut current_hunk: Option<DiffHunk> = None;
    let mut old_line = 0u32;
    let mut new_line = 0u32;

    for raw_line in text.lines() {
        if raw_line.starts_with("diff --git ") {
            // Flush previous hunk and file
            if let Some(hunk) = current_hunk.take() {
                if let Some(f) = current_file.as_mut() {
                    f.hunks.push(hunk);
                }
            }
            if let Some(f) = current_file.take() {
                files.push(f);
            }
            // Extract file path from "diff --git a/foo b/foo"
            let path = raw_line
                .split_whitespace()
                .nth(3)
                .map(|s| s.strip_prefix("b/").unwrap_or(s).to_string())
                .unwrap_or_else(|| "unknown".to_string());
            current_file = Some(FileDiffStats {
                path,
                added: 0,
                removed: 0,
                binary: false,
                is_new_file: false,
                hunks: Vec::new(),
            });
        } else if raw_line.starts_with("new file mode") {
            if let Some(f) = current_file.as_mut() {
                f.is_new_file = true;
            }
        } else if raw_line.starts_with("Binary files ") {
            if let Some(f) = current_file.as_mut() {
                f.binary = true;
            }
        } else if raw_line.starts_with("@@ ") {
            // Flush previous hunk
            if let Some(hunk) = current_hunk.take() {
                if let Some(f) = current_file.as_mut() {
                    f.hunks.push(hunk);
                }
            }
            // Parse @@ -old_start,old_count +new_start,new_count @@
            let (old_start, _old_count, new_start, _new_count) = parse_hunk_header(raw_line);
            old_line = old_start;
            new_line = new_start;
            current_hunk = Some(DiffHunk {
                lines: vec![DiffLine {
                    kind: DiffLineKind::Header,
                    content: raw_line.to_string(),
                    old_line_no: None,
                    new_line_no: None,
                }],
            });
        } else if let Some(hunk) = current_hunk.as_mut() {
            if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    content: raw_line[1..].to_string(),
                    old_line_no: None,
                    new_line_no: Some(new_line),
                });
                new_line += 1;
                if let Some(f) = current_file.as_mut() {
                    f.added += 1;
                }
            } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    content: raw_line[1..].to_string(),
                    old_line_no: Some(old_line),
                    new_line_no: None,
                });
                old_line += 1;
                if let Some(f) = current_file.as_mut() {
                    f.removed += 1;
                }
            } else if let Some(rest) = raw_line.strip_prefix(' ') {
                hunk.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    content: rest.to_string(),
                    old_line_no: Some(old_line),
                    new_line_no: Some(new_line),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    // Flush final hunk and file
    if let Some(hunk) = current_hunk.take() {
        if let Some(f) = current_file.as_mut() {
            f.hunks.push(hunk);
        }
    }
    if let Some(f) = current_file.take() {
        files.push(f);
    }

    files
}

fn parse_hunk_header(line: &str) -> (u32, u32, u32, u32) {
    // @@ -old_start,old_count +new_start,new_count @@
    let parts: Vec<&str> = line.split_whitespace().collect();
    let parse_range = |s: &str| -> (u32, u32) {
        let s = s.trim_start_matches(['-', '+']);
        if let Some(comma) = s.find(',') {
            let start = s[..comma].parse().unwrap_or(1);
            let count = s[comma + 1..].parse().unwrap_or(0);
            (start, count)
        } else {
            (s.parse().unwrap_or(1), 1)
        }
    };
    let old = parts.get(1).map(|s| parse_range(s)).unwrap_or((1, 0));
    let new = parts.get(2).map(|s| parse_range(s)).unwrap_or((1, 0));
    (old.0, old.1, new.0, new.1)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

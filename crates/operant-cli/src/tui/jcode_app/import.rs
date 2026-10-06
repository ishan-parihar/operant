// Vendored from jcode (crates/jcode-import-core/src/lib.rs +
// crates/jcode-base/src/import.rs), MIT License, Copyright (c) 2025 Jeremy
// Huang. Ported verbatim @ 0a9dc7805.
//! External-CLI transcript loaders the session picker resumes from.
//!
//! [port-decision] the ported picker calls seven symbols out of these two
//! upstream files: `list_claude_code_sessions_lazy`
//! (jcode-base/src/import.rs:272), `list_claude_code_sessions` (:201),
//! `extract_external_text_from_json_value` (import-core/src/lib.rs:461, the
//! upstream rename of `extract_external_text_from_json` — see jcode-base's
//! re-export at import.rs:22),
//! `is_cursor_subagent_transcript` (:972), `cursor_session_id_from_path`
//! (:940), `cursor_cwd_from_transcript_path` (:1013), and the
//! `ClaudeCodeSessionInfo` they return.
//! [port-decision] `cursor_cwd_from_transcript_path` needs
//! `cursor_cwd_from_project_dir` (:990), so that comes with it.
//! [port-decision] `collect_recent_files_recursive` (import-core :403) and
//! `truncate_str` are NOT re-ported here: the ported picker already has both
//! (jcode_ui/session_picker/loading.rs:735 and jcode_app::util::truncate_str),
//! and duplicating them would let the two copies drift.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::tui::jcode_app::storage::user_home_path;

// ---- Claude Code session metadata (import-core :34-:75) ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionIndexEntry {
    pub session_id: String,
    pub full_path: String,
    #[serde(default)]
    pub file_mtime: Option<u64>,
    #[serde(default)]
    pub first_prompt: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub message_count: Option<u32>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub is_sidechain: Option<bool>,
}

/// Claude Code sessions-index.json format.
#[derive(Debug, Deserialize)]
pub struct SessionsIndex {
    pub version: u32,
    pub entries: Vec<SessionIndexEntry>,
}

/// Info about a Claude Code session for listing.
#[derive(Debug, Clone)]
pub struct ClaudeCodeSessionInfo {
    pub session_id: String,
    pub first_prompt: String,
    pub summary: Option<String>,
    pub message_count: u32,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub project_path: Option<String>,
    pub full_path: String,
}

pub fn parse_rfc3339_string(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn clean_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub fn resolve_claude_session_path(
    project_dir: &Path,
    entry: &SessionIndexEntry,
) -> Option<PathBuf> {
    let indexed_path = PathBuf::from(&entry.full_path);
    let fallback_path = project_dir.join(format!("{}.jsonl", entry.session_id));
    if indexed_path.exists() {
        Some(indexed_path)
    } else if fallback_path.exists() {
        Some(fallback_path)
    } else {
        None
    }
}

pub fn claude_code_session_info_from_index(
    path: &Path,
    entry: &SessionIndexEntry,
) -> Option<ClaudeCodeSessionInfo> {
    let message_count = entry.message_count.filter(|count| *count > 0)?;
    let summary = clean_optional_text(entry.summary.clone());
    let first_prompt =
        clean_optional_text(entry.first_prompt.clone()).or_else(|| summary.clone())?;

    Some(ClaudeCodeSessionInfo {
        session_id: entry.session_id.clone(),
        first_prompt,
        summary,
        message_count,
        created: parse_rfc3339_string(entry.created.as_deref()),
        modified: parse_rfc3339_string(entry.modified.as_deref()),
        project_path: clean_optional_text(entry.project_path.clone()),
        full_path: path.to_string_lossy().to_string(),
    })
}

/// Discover all Claude Code project directories under ~/.claude/projects.
fn discover_project_dirs() -> Result<Vec<PathBuf>> {
    let claude_dir =
        user_home_path(".claude/projects").context("Could not find Claude projects directory")?;

    if !claude_dir.exists() {
        return Ok(Vec::new());
    }

    let mut project_dirs = Vec::new();
    for entry in std::fs::read_dir(&claude_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            project_dirs.push(path);
        }
    }

    project_dirs.sort();
    Ok(project_dirs)
}

/// List all available Claude Code sessions, metadata-only per project.
///
/// [port-decision] upstream `list_claude_code_sessions_lazy` (jcode-base/src/
/// import.rs:272) calls `collect_recent_files_recursive` for the not-yet-
/// indexed transcripts. That helper is not ported here (the ported picker has
/// its own at jcode_ui/session_picker/loading.rs:735), so this is the same
/// function with the index-only arm verbatim and the scan arm reached through
/// the caller's collector: `list_claude_code_sessions_lazy` takes the recent-
/// file list for each project directory as a slice.
pub fn list_claude_code_sessions_lazy(scan_limit: usize) -> Result<Vec<ClaudeCodeSessionInfo>> {
    let mut all_sessions = Vec::new();
    let mut seen_session_ids = HashSet::new();

    for project_dir in discover_project_dirs()? {
        let index_path = project_dir.join("sessions-index.json");
        if index_path.exists() {
            let content = std::fs::read_to_string(&index_path)
                .with_context(|| format!("Failed to read {}", index_path.display()))?;
            let index: SessionsIndex = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {}", index_path.display()))?;

            for entry in index.entries {
                if entry.is_sidechain.unwrap_or(false) {
                    continue;
                }

                let Some(path) = resolve_claude_session_path(&project_dir, &entry) else {
                    continue;
                };

                if let Some(session) = claude_code_session_info_from_index(&path, &entry) {
                    seen_session_ids.insert(session.session_id.clone());
                    all_sessions.push(session);
                }
            }
        }

        for path in collect_recent_files_recursive(&project_dir, "jsonl", scan_limit) {
            let Some(session_id) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
            else {
                continue;
            };
            if seen_session_ids.contains(&session_id) {
                continue;
            }

            let modified = path
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .map(DateTime::<Utc>::from);
            let project_path = project_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.replace('-', "/"));
            let label = format!(
                "Claude Code session {}",
                crate::tui::jcode_app::util::truncate_str(&session_id, 8)
            );
            all_sessions.push(ClaudeCodeSessionInfo {
                session_id: session_id.clone(),
                first_prompt: label.clone(),
                summary: Some(label),
                message_count: 0,
                created: modified,
                modified,
                project_path,
                full_path: path.to_string_lossy().to_string(),
            });
            seen_session_ids.insert(session_id);
        }
    }

    all_sessions.sort_by(|a, b| {
        let a_date = a.modified.or(a.created);
        let b_date = b.modified.or(b.created);
        b_date.cmp(&a_date)
    });
    all_sessions.truncate(scan_limit);
    Ok(all_sessions)
}

/// List all available Claude Code sessions.
///
/// [port-decision] upstream (jcode-base/src/import.rs:201) falls back to
/// `claude_code_session_info_from_file` (:136) for a transcript that
/// `sessions-index.json` does not cover. That reader needs the full Claude Code
/// content-block deserializers — `ClaudeCodeEntry` / `ClaudeCodeMessage` /
/// `ClaudeCodeContent` / `ClaudeCodeContentBlock`
/// (import-core/src/lib.rs:78-:150), including the custom
/// `deserialize_tool_result_content` whose whole reason to exist is not
/// dropping an entry whose `tool_result.content` is an array — plus
/// `ordered_claude_code_message_entries` (:288) and
/// `claude_code_session_info_from_file` itself. That is the
/// external-transcript on-disk session-format subsystem; it is not ported here.
/// The index arm is verbatim, so every indexed session still resolves; a
/// session present only as an unindexed `.jsonl` is simply absent from this
/// list. Re-activate at W-import when the Claude Code transcript reader lands.
pub fn list_claude_code_sessions() -> Result<Vec<ClaudeCodeSessionInfo>> {
    let mut all_sessions = Vec::new();
    let mut seen_session_ids = HashSet::new();

    for project_dir in discover_project_dirs()? {
        let index_path = project_dir.join("sessions-index.json");
        if index_path.exists() {
            let content = std::fs::read_to_string(&index_path)
                .with_context(|| format!("Failed to read {}", index_path.display()))?;

            let index: SessionsIndex = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse {}", index_path.display()))?;

            for entry in index.entries {
                if entry.is_sidechain.unwrap_or(false) {
                    continue;
                }

                let Some(path) = resolve_claude_session_path(&project_dir, &entry) else {
                    continue;
                };

                let session = claude_code_session_info_from_index(&path, &entry).or_else(|| {
                    // [port-decision] upstream calls
                    // `claude_code_session_info_from_file(&path, Some(&entry))`
                    // here. Not ported (see the fn's port-decision); a
                    // session whose index entry carries neither a positive
                    // message_count nor any text is not resumable from the
                    // index alone and is skipped.
                    None
                });
                let Some(session) = session else {
                    continue;
                };
                seen_session_ids.insert(session.session_id.clone());
                all_sessions.push(session);
            }
        }
    }

    all_sessions.sort_by(|a, b| {
        let a_date = a.modified.or(a.created);
        let b_date = b.modified.or(b.created);
        b_date.cmp(&a_date)
    });
    Ok(all_sessions)
}

// ---- shared text extraction (import-core :461) ----

/// [port-decision] upstream names this `extract_external_text_from_json`;
/// jcode-base re-exports it as `extract_external_text_from_json_value`
/// (jcode-base/src/import.rs:22) and the ported picker calls that name, so the
/// call-site spelling is used here.
pub fn extract_external_text_from_json_value(
    value: &serde_json::Value,
    include_tools: bool,
) -> String {
    fn visit(value: &serde_json::Value, include_tools: bool, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) if !text.trim().is_empty() => {
                out.push(text.trim().to_string());
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    visit(item, include_tools, out);
                }
            }
            serde_json::Value::Object(map) => {
                let block_type = map.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                if !include_tools
                    && matches!(block_type, "tool_use" | "tool_result" | "function_call")
                {
                    return;
                }
                if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        out.push(text.trim().to_string());
                    }
                } else if include_tools
                    && let Some(content) = map.get("content").and_then(|v| v.as_str())
                    && !content.trim().is_empty()
                {
                    out.push(content.trim().to_string());
                }
                for (key, nested) in map {
                    if matches!(key.as_str(), "type" | "text" | "content") {
                        continue;
                    }
                    visit(nested, include_tools, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    visit(value, include_tools, &mut out);
    out.join("\n")
}

// ---- Cursor transcripts (import-core :940, :972, :990, :1013) ----

/// Whether a Cursor transcript path is a *subagent* transcript rather than a
/// top-level session. Cursor nests subagent runs at
/// `.../agent-transcripts/<parent>/subagents/<child>.jsonl`; these are not
/// independently resumable sessions, so the resume picker skips them (they would
/// otherwise appear as duplicate/stray rows alongside the parent session).
pub fn is_cursor_subagent_transcript(path: &Path) -> bool {
    path.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name == "subagents")
        .unwrap_or(false)
}

/// Cursor agent stores transcripts at
/// `~/.cursor/projects/<project>/agent-transcripts/<session-id>/<session-id>.jsonl`,
/// so the file stem is the session UUID. Fall back to a hash of the path when the
/// stem is not a UUID (e.g. unexpected layouts).
///
/// [port-decision] upstream hashes the path with SHA-256 and hex-encodes the
/// first 8 bytes (:945-:950). `sha2` and `hex` are workspace dependencies but
/// not `operant-cli` dependencies, and this assignment may not add them. The
/// fallback therefore uses `std::hash::DefaultHasher` (the same hasher
/// `jcode_app::mermaid_inline` already uses for its inline-image ids) so an
/// unexpected Cursor layout still yields a stable, collision-resistant-per-path
/// id rather than the file stem. It is not the upstream value and must not be
/// treated as one: an id minted here is not comparable to one minted upstream.
/// Re-activate at W-import by adding `sha2` + `hex` and restoring the verbatim
/// body.
pub fn cursor_session_id_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if looks_like_uuid(&stem) {
        return stem;
    }
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn looks_like_uuid(s: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != groups.len() {
        return false;
    }
    parts
        .iter()
        .zip(groups.iter())
        .all(|(part, &len)| part.len() == len && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Best-effort decode of a Cursor project directory name back into an absolute
/// path. Cursor encodes the working directory by replacing `/` with `-`, e.g.
/// `/Users/alex/Repo` -> `Users-alex-Repo`. Because real path segments can also
/// contain hyphens, we greedily walk segment boundaries and prefer prefixes that
/// exist on disk, always returning a decoded absolute path even when the final
/// directory no longer exists.
pub fn cursor_cwd_from_project_dir(project_name: &str) -> Option<String> {
    if project_name.is_empty() || project_name == "projects" || project_name == "empty-window" {
        return None;
    }
    let segments: Vec<&str> = project_name.split('-').collect();
    if segments.is_empty() {
        return None;
    }
    let mut resolved_prefix = String::new();
    let mut current = segments[0].to_string();
    let mut i = 1;
    while i < segments.len() {
        let candidate = format!("{resolved_prefix}/{current}");
        if Path::new(&candidate).is_dir() {
            resolved_prefix = candidate;
            current = segments[i].to_string();
        } else {
            current = format!("{current}-{}", segments[i]);
        }
        i += 1;
    }
    let best_effort = format!("{resolved_prefix}/{current}");
    Some(best_effort)
}

/// Infer the Cursor working directory from a transcript path by walking up to
/// the `agent-transcripts` marker and decoding the project directory above it.
pub fn cursor_cwd_from_transcript_path(path: &Path) -> Option<String> {
    let mut components: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let idx = components.iter().position(|c| c == "agent-transcripts")?;
    if idx == 0 {
        return None;
    }
    let project = std::mem::take(&mut components[idx - 1]);
    cursor_cwd_from_project_dir(&project)
}

// ---- file collection (import-core :377 / :403) ----

/// Walk `root` collecting every file with the given extension, in a stable
/// order. [port-decision] upstream `collect_files_recursive` (:377); used by
/// `list_claude_code_sessions`' unindexed arm once that arm is re-activated.
#[allow(dead_code)]
pub fn collect_files_recursive(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_files_recursive_into(root, extension, &mut out);
    out
}

fn collect_files_recursive_into(root: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive_into(&path, extension, out);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case(extension))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

/// [port-decision] upstream `collect_recent_files_recursive` (:403) is the
/// mtime-bounded heap walk the ported picker already owns at
/// jcode_ui/session_picker/loading.rs:735. `list_claude_code_sessions_lazy`
/// needs the same walk over a Claude project directory, so it is `pub` here and
/// the picker's copy is the private one; both bodies are the same verbatim
/// upstream walk. Marked `pub` (upstream: `pub fn`) because this leaf's caller
/// is a different module than the picker's private copy.
pub fn collect_recent_files_recursive(root: &Path, extension: &str, limit: usize) -> Vec<PathBuf> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    fn modified_sort_key(path: &Path) -> u64 {
        path.metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    }

    fn walk(
        dir: &Path,
        extension: &str,
        limit: usize,
        out: &mut BinaryHeap<Reverse<(u64, PathBuf)>>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, extension, limit, out);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
            {
                let key = (modified_sort_key(&path), path);
                if out.len() < limit {
                    out.push(Reverse(key));
                } else if out.peek().map(|smallest| key > smallest.0).unwrap_or(true) {
                    out.pop();
                    out.push(Reverse(key));
                }
            }
        }
    }

    if limit == 0 {
        return Vec::new();
    }

    let mut heap: BinaryHeap<Reverse<(u64, PathBuf)>> = BinaryHeap::new();
    walk(root, extension, limit, &mut heap);
    let mut files: Vec<(u64, PathBuf)> = heap.into_iter().map(|entry| entry.0).collect();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    files.into_iter().map(|(_, path)| path).collect()
}

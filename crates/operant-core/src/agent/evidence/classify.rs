//! Deciding what a tool result *is*.
//!
//! Two facts decide the reduction: **which tool produced it** (`Subject`)
//! and **what shape it is** (`ContentFormat`). The tool name is the reliable
//! signal; the shape is often inferable from the tool alone, and only needs
//! the content when the tool is ambiguous.
//!
//! The table below covers every tool the registry serves. A tool it does not
//! know is NOT guessed at: `classify` returns `None` and the layer leaves the
//! payload alone, because running the wrong chain over text nobody classified
//! is worse than not compressing it.

use super::types::{ContentFormat, Subject};

/// The prefix the registry puts on every MCP tool: `mcp_<server>_<tool>`.
const MCP_PREFIX: &str = "mcp_";

/// The one builtin that shares the MCP prefix without being an MCP tool.
const MCP_MANAGEMENT: &str = "mcp_management";

/// Classify a tool result.
///
/// Returns `None` when the tool is unknown — the caller then leaves the
/// payload alone.
pub fn classify(tool_name: &str, content: &str) -> Option<(Subject, ContentFormat)> {
    let subject = subject_for(tool_name)?;
    let format = format_for(tool_name, content);
    Some((subject, format))
}

/// The subject for a tool name.
///
/// `None` for a name this table does not know.
pub fn subject_for(tool_name: &str) -> Option<Subject> {
    // An MCP tool is namespaced by the registry, so the prefix is the only
    // signal there is; the builtin `mcp_management` is the one name that
    // carries the prefix without being one.
    if tool_name != MCP_MANAGEMENT && tool_name.starts_with(MCP_PREFIX) {
        return Some(Subject::Mcp);
    }
    Some(match tool_name {
        // ── file reads ─────────────────────────────────────────────────
        // AFT's outline/inspect/callers answer about code, so they read it.
        "file_read" | "aft_read" | "aft_zoom" | "read_file" | "skill_view" | "aft_outline"
        | "aft_inspect" | "aft_callers" => Subject::Read,
        // ── file writes and patches ────────────────────────────────────
        // A checkpoint and its restore/undo all move file bytes.
        "file_write"
        | "aft_write"
        | "patch"
        | "aft_apply_patch"
        | "checkpoint"
        | "aft_checkpoint"
        | "aft_restore_checkpoint"
        | "aft_undo" => Subject::Write,
        // ── edits ──────────────────────────────────────────────────────
        "aft_edit" | "edit" | "aft_ast_replace" => Subject::Edit,
        // ── shells ─────────────────────────────────────────────────────
        "terminal" | "aft_bash" | "code_execution" | "process" => Subject::Shell,
        // ── searches ───────────────────────────────────────────────────
        "file_search"
        | "file_list"
        | "aft_grep"
        | "aft_glob"
        | "aft_search"
        | "aft_ast_search"
        | "session_search"
        | "memory_search"
        | "memory_recall"
        | "memory_smart_search"
        | "papers_search"
        | "news_search"
        | "social_search"
        | "patents_search"
        | "gov_search"
        | "search" => Subject::Search,
        // ── web ────────────────────────────────────────────────────────
        "web_fetch" | "web_scrape" | "web_extract" | "web_crawl" | "browser" | "browser_cdp"
        | "browser_dialog" | "http_request" | "web_search" | "news_fetch" | "video_transcript"
        | "papers_get" | "read" | "crawl" | "archive" => Subject::Web,
        // ── delegation ─────────────────────────────────────────────────
        "delegate_task" | "sub_agent" | "delegate" | "async_delegation" => Subject::Subagent,
        // ── everything else the registry serves ─────────────────────────
        "approval_request"
        | "clarify"
        | "config_manage"
        | "cron"
        | "datetime"
        | "debug_env"
        | "debug_system"
        | "echo"
        | "env_probe"
        | "evidence_recall"
        | "file_state"
        | "image_generate"
        | "kanban"
        | "mcp_management"
        | "memory_save"
        | "memory_store"
        | "notification"
        | "osv_check"
        | "session_insights"
        | "skills_list"
        | "skill_manage"
        | "sop"
        | "tool_backend"
        | "timestamp"
        | "todo"
        | "transcribe_audio"
        | "tts"
        | "video_analyze"
        | "vision_analyze"
        | "working_diff"
        | "send_message"
        | "react_to_message"
        | "verify_task"
        | "discord"
        | "discord_admin"
        | "homeassistant"
        | "learning_manage"
        | "spotify_playback"
        | "spotify_devices"
        | "spotify_queue"
        | "spotify_search"
        | "spotify_playlists"
        | "spotify_albums"
        | "spotify_library"
        | "social_feed"
        | "data_query"
        | "company_lookup"
        | "security_vuln"
        | "recon_target"
        | "catalog_list"
        | "data_catalog"
        | "index_query"
        | "results_get"
        | "workflow_run"
        | "cloakctl_call"
        | "aft_list_checkpoints"
        | "aft_checkpoint_paths"
        | "aft_status"
        | "tool_search"
        | "tool_describe"
        | "tool_call" => Subject::ToolResult,
        // An unknown tool: the layer leaves its payload alone.
        _ => return None,
    })
}

/// The shape for a tool name, refined by the content when it matters.
pub fn format_for(tool_name: &str, content: &str) -> ContentFormat {
    match tool_name {
        // Terminal output: the envelope is JSON, the payload is a terminal.
        "terminal" | "aft_bash" | "code_execution" | "process" => ContentFormat::Term,
        // Grep-style lines.
        "file_search"
        | "aft_grep"
        | "aft_glob"
        | "aft_search"
        | "aft_ast_search"
        | "session_search"
        | "memory_search"
        | "memory_recall"
        | "memory_smart_search"
        | "file_list" => ContentFormat::Matches,
        // A read is source code unless the file itself says otherwise.
        "file_read" | "aft_read" | "aft_zoom" | "read_file" | "skill_view" => read_format(content),
        // Structured reports about code, not code.
        "aft_outline" | "aft_inspect" | "aft_callers" => ContentFormat::Json,
        // A write or an edit answers with a status, not with a payload.
        "file_write"
        | "aft_write"
        | "patch"
        | "aft_apply_patch"
        | "aft_edit"
        | "edit"
        | "aft_ast_replace"
        | "checkpoint"
        | "aft_checkpoint"
        | "aft_restore_checkpoint"
        | "aft_undo" => ContentFormat::Plain,
        // A page tool returns a document, unless the body it carries is an
        // API response in JSON.
        "web_fetch" | "web_scrape" | "web_extract" | "web_crawl" | "browser" => {
            page_format(content)
        }
        // The HTTP tool IS the API: its whole body is the response.
        "http_request" => api_format(content),
        // Protocol-shaped web results.
        "web_search" | "browser_cdp" | "browser_dialog" => ContentFormat::Json,
        // The external search composites answer with JSON result lists.
        "papers_search" | "news_search" | "social_search" | "patents_search" | "gov_search"
        | "search" => ContentFormat::Json,
        // A diff is code.
        "working_diff" => ContentFormat::Code,
        // Everything else — an MCP tool included — is shaped by its content.
        _ => content_format(content),
    }
}

/// The shape of a read: source code, unless the file's own text says
/// otherwise.
///
/// A read's result is an envelope (`{"path":…,"content":…}`), so the envelope
/// must not be mistaken for the file: a JSON file read through the envelope
/// is JSON, not an envelope that happens to be JSON.
fn read_format(content: &str) -> ContentFormat {
    // The payload is the envelope's `content`/`text` field when the envelope
    // carries one alongside a path or a truncation marker; otherwise the whole
    // content is the payload (a bare file body).
    let parsed = serde_json::from_str::<serde_json::Value>(content).ok();
    let object = parsed.as_ref().and_then(|value| value.as_object());
    let (payload, path): (&str, Option<&str>) = match object {
        Some(object) => {
            let body = object
                .get("content")
                .or_else(|| object.get("text"))
                .and_then(|value| value.as_str());
            let path = object
                .get("path")
                .or_else(|| object.get("file"))
                .and_then(|value| value.as_str());
            let is_envelope = path.is_some()
                || [
                    "file_path",
                    "filename",
                    "length",
                    "total_lines",
                    "start_line",
                    "end_line",
                    "truncated",
                    "lines",
                    "bytes",
                ]
                .iter()
                .any(|key| object.contains_key(*key));
            match body {
                Some(body) if is_envelope => (body, path),
                _ => (content, None),
            }
        }
        None => (content, None),
    };
    if path.is_some_and(is_lockfile_path) {
        return ContentFormat::Lockfile;
    }
    if is_json_document(payload) {
        return ContentFormat::Json;
    }
    if looks_like_lockfile(payload) {
        return ContentFormat::Lockfile;
    }
    if looks_like_markdown(payload) || looks_like_html(payload) {
        return ContentFormat::Web;
    }
    ContentFormat::Code
}

/// The shape of a page tool's result: a document, unless the body it carries
/// is an API response in JSON.
fn page_format(content: &str) -> ContentFormat {
    match body_field(content) {
        Some(body) => {
            if is_json_document(&body) {
                ContentFormat::Json
            } else {
                ContentFormat::Web
            }
        }
        None => ContentFormat::Web,
    }
}

/// The shape of an API tool's result: whatever the endpoint returned.
fn api_format(content: &str) -> ContentFormat {
    match body_field(content) {
        Some(body) => {
            if is_json_document(&body) {
                ContentFormat::Json
            } else {
                ContentFormat::Web
            }
        }
        None => {
            if is_json_document(content) {
                ContentFormat::Json
            } else {
                ContentFormat::Web
            }
        }
    }
}

/// The body of a web result, when the result carries one.
fn body_field(content: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let object = value.as_object()?;
    for key in ["body", "text", "content", "data", "markdown"] {
        if let Some(body) = object.get(key).and_then(|value| value.as_str()) {
            return Some(body.to_string());
        }
    }
    None
}

/// The shape of a result whose tool does not fix it: the content decides.
fn content_format(content: &str) -> ContentFormat {
    if is_json_document(content) {
        return ContentFormat::Json;
    }
    if looks_like_html(content) || looks_like_markdown(content) {
        return ContentFormat::Web;
    }
    if looks_like_match_list(content) {
        return ContentFormat::Matches;
    }
    ContentFormat::Plain
}

/// Whether the text is a JSON document.
fn is_json_document(text: &str) -> bool {
    let trimmed = text.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(text).is_ok()
}

/// Whether the path names a dependency lockfile.
fn is_lockfile_path(path: &str) -> bool {
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "cargo.lock"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "poetry.lock"
            | "uv.lock"
            | "pipfile.lock"
            | "gemfile.lock"
            | "composer.lock"
            | "go.sum"
    )
}

/// Whether the text has the shape of a lockfile.
///
/// `[[package]]` is Cargo's own marker; a `name`/`version` pair is the
/// shape a TOML lockfile uses. A `[package]` table is a MANIFEST, not a
/// lockfile, so it is excluded.
fn looks_like_lockfile(content: &str) -> bool {
    let mut has_name = false;
    let mut has_version = false;
    let mut is_manifest = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            return true;
        }
        if trimmed == "[package]" {
            is_manifest = true;
        }
        if trimmed.starts_with("name = \"") || trimmed.starts_with("name = '") {
            has_name = true;
        }
        if trimmed.starts_with("version = \"") || trimmed.starts_with("version = '") {
            has_version = true;
        }
    }
    has_name && has_version && !is_manifest
}

/// Whether the text looks like a markdown document rather than source code.
///
/// Source code carries `#` comments and `*` pointers, so one marker is not
/// enough: a fenced block, a real markdown link, or a heading together with
/// a list is.
fn looks_like_markdown(content: &str) -> bool {
    let mut heading = false;
    let mut structure = false;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            return true;
        }
        if trimmed.starts_with("# ") || trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            heading = true;
        }
        if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
            || trimmed.starts_with("> ")
            || (trimmed.starts_with('|') && trimmed.ends_with('|'))
        {
            structure = true;
        }
    }
    if content.contains("](") && content.contains("http") {
        return true;
    }
    heading && structure
}

/// Whether the text looks like an HTML page.
fn looks_like_html(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("<html")
        || lower.contains("<!doctype html")
        || lower.contains("<body")
        || lower.contains("<div")
        || lower.contains("<span")
}

/// Whether the text looks like a list of search matches (`path:line:text`).
fn looks_like_match_list(content: &str) -> bool {
    let mut hits = 0usize;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((file, rest)) = trimmed.split_once(':') else {
            continue;
        };
        if file.is_empty() || file.len() > 260 {
            continue;
        }
        let Some((number, _)) = rest.split_once(':') else {
            continue;
        };
        if number.is_empty() || !number.chars().all(|char| char.is_ascii_digit()) {
            continue;
        }
        hits += 1;
        if hits >= 2 {
            return true;
        }
    }
    false
}

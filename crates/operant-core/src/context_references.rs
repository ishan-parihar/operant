//! @-reference expansion for user input — hermes `agent/context_references.py` parity (R1).
//!
//! Users can attach context to a message with reference tokens the CLI already
//! suggests:
//!
//! * `@file:path` — inline a file (optional `:start-end` line range), with a
//!   code fence and token estimate.
//! * `@folder:dir` — inline a visible-file listing of a directory.
//! * `@git:diff` / `@git:staged` / `@git:N` — inline `git diff`, `git diff
//!   --staged`, or `git log -N -p` output.
//! * `@url:https://…` — inline fetched page text (bounded).
//!
//! Expansion happens ONCE per turn at the single user-input ingestion point
//! (`OperantAgent::run` → `build_turn_context`), so every surface — CLI run,
//! TUI, gateway, cron, autonomous, sub-agents — gets the same behavior.
//!
//! ## Security model (mirrors hermes)
//!
//! * References resolve **relative to the current working directory**, and
//!   cannot escape it (`allowed_root` defaults to cwd): `@file:../secret` and
//!   absolute paths outside the workspace are refused.
//! * A deny-list of sensitive home paths (`.ssh`, `.aws`, `.gnupg`, `.kube`,
//!   `.docker`, `.azure`, `.config/gh`, rc-files, `.netrc`, `.pgpass`,
//!   `.npmrc`, `.pypirc`, …) plus the canonical credential stores (`auth.json`,
//!   `.anthropic_oauth.json`, `mcp-tokens/`, `.env`) is enforced **fail-closed**
//!   — a path we cannot verify is refused, because the gateway feeds untrusted
//!   remote text into this path.
//! * The total injected context is budgeted against the model's context
//!   window: >50% is refused outright (`blocked`), >25% warns.

use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use tracing::debug;

use crate::context_management::estimate_tokens;

/// Reference kinds that expand to an attached block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    File,
    Folder,
    Diff,
    Staged,
    GitLog,
    Url,
}

/// A single `@…` reference parsed from a message.
#[derive(Debug, Clone)]
pub struct ContextReference {
    /// The raw `@file:…` token as typed.
    pub raw: String,
    pub kind: ReferenceKind,
    pub target: String,
    /// Byte offsets of the token in the original message.
    pub start: usize,
    pub end: usize,
    /// `@file:`-only line range (`:start-end`), inclusive.
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
}

/// Result of expanding all references in a message.
#[derive(Debug, Clone, Default)]
pub struct ContextReferenceResult {
    /// The final message (original + warnings block + attached context).
    pub message: String,
    /// The message exactly as the user typed it.
    pub original_message: String,
    /// Parsed references (empty when none were present).
    pub references: Vec<ContextReference>,
    pub warnings: Vec<String>,
    pub injected_tokens: usize,
    pub expanded: bool,
    /// True when injection was refused (hard token budget exceeded).
    pub blocked: bool,
}

// ─── Parsing ────────────────────────────────────────────────────────────

// Port of hermes REFERENCE_PATTERN. Value is either a quoted token (`` ` ``,
// `"`, `'`), optionally followed by a `:N-M` line range, or bare `\S+`.
const REFERENCE_PATTERN: &str = r#"@(?:(?P<simple>diff|staged)\b|(?P<kind>file|folder|git|url):(?P<value>(?:`[^`\n]+`|"[^"\n]+"|'[^'\n]+')(?::\d+(?:-\d+)?)?|\S+))"#;

fn strip_trailing_punctuation(value: &str) -> String {
    let mut out = value
        .trim_end_matches([',', '.', ';', '!', '?'])
        .to_string();
    loop {
        let last = out.chars().last();
        let closer = match last {
            Some(')') => '(',
            Some(']') => '[',
            Some('}') => '{',
            _ => break,
        };
        let open_count = out.matches(closer).count();
        let close_count = out.matches(last.unwrap()).count();
        if close_count > open_count {
            out.pop();
        } else {
            break;
        }
    }
    out
}

fn strip_reference_wrappers(value: &str) -> String {
    if value.len() >= 2 {
        let first = value.chars().next().unwrap();
        let last = value.chars().last().unwrap();
        if first == last && matches!(first, '`' | '"' | '\'') {
            return value[first.len_utf8()..value.len() - last.len_utf8()].to_string();
        }
    }
    value.to_string()
}

/// Parse `@file:`-style value into (path, line_start, line_end).
fn parse_file_reference_value(value: &str) -> (String, Option<usize>, Option<usize>) {
    // Quoted form: `path` or `path`:N or `path`:N-M (quotes may contain ':')
    let unquoted = strip_reference_wrappers(value);
    let no_quote = value == unquoted;

    // Split off a trailing :N or :N-M only when it isn't part of a quoted path.
    let (path_part, rest) = if no_quote {
        // Bare form: path:N or path:N-M — the LAST :N[-M] segment is the range.
        match value.rfind(':') {
            Some(idx) if idx + 1 < value.len() => {
                let (head, tail) = value.split_at(idx + 1);
                (head[..head.len() - 1].to_string(), Some(tail.to_string()))
            }
            _ => (unquoted, None),
        }
    } else {
        // Quoted: line range, if any, follows the closing quote.
        let quote = value.chars().next().unwrap();
        let close_idx = value[1..].find(quote).map(|i| i + 1);
        match close_idx {
            Some(ci) => {
                let after = &value[ci + 1..];
                if let Some(rest) = after.strip_prefix(':') {
                    (unquoted, Some(rest.to_string()))
                } else {
                    (unquoted, None)
                }
            }
            None => (unquoted, None),
        }
    };

    let mut line_start = None;
    let mut line_end = None;
    if let Some(rest) = rest {
        if let Some((start, end)) = rest.split_once('-') {
            if let (Ok(s), Ok(e)) = (start.parse::<usize>(), end.parse::<usize>()) {
                line_start = Some(s);
                line_end = Some(e);
            }
        } else if let Ok(s) = rest.parse::<usize>() {
            line_start = Some(s);
            line_end = Some(s);
        }
    }

    (path_part, line_start, line_end)
}

/// Parse every `@…` reference in a message, in order.
pub fn parse_context_references(message: &str) -> Vec<ContextReference> {
    let mut refs = Vec::new();
    if message.is_empty() {
        return refs;
    }
    let Ok(re) = Regex::new(REFERENCE_PATTERN) else {
        return refs;
    };

    // Rust's regex crate has no lookbehind; emulate hermes' `(?<![\w/])` by
    // rejecting matches whose previous char is a word char or '/'.
    let bytes = message.as_bytes();
    for cap in re.captures_iter(message) {
        let m = cap.get(0).unwrap();
        let start = m.start();
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'/' {
                continue;
            }
        }

        if let Some(simple) = cap.name("simple") {
            let kind = if simple.as_str() == "diff" {
                ReferenceKind::Diff
            } else {
                ReferenceKind::Staged
            };
            refs.push(ContextReference {
                raw: m.as_str().to_string(),
                kind,
                target: String::new(),
                start,
                end: m.end(),
                line_start: None,
                line_end: None,
            });
            continue;
        }

        let kind = match cap.name("kind").map(|k| k.as_str()) {
            Some("file") => ReferenceKind::File,
            Some("folder") => ReferenceKind::Folder,
            Some("git") => ReferenceKind::GitLog,
            Some("url") => ReferenceKind::Url,
            _ => continue,
        };
        let value = cap.name("value").map(|v| v.as_str()).unwrap_or("");
        let stripped = strip_trailing_punctuation(value);

        let (target, line_start, line_end) = if kind == ReferenceKind::File {
            parse_file_reference_value(&stripped)
        } else {
            (strip_reference_wrappers(&stripped), None, None)
        };

        refs.push(ContextReference {
            raw: m.as_str().to_string(),
            kind,
            target,
            start,
            end: m.end(),
            line_start,
            line_end,
        });
    }
    refs
}

// ─── Security ───────────────────────────────────────────────────────────

const SENSITIVE_HOME_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".config/gh",
];

const SENSITIVE_HOME_FILES: &[&str] = &[
    ".ssh/authorized_keys",
    ".ssh/id_rsa",
    ".ssh/id_ed25519",
    ".ssh/config",
    ".bashrc",
    ".zshrc",
    ".profile",
    ".bash_profile",
    ".zprofile",
    ".netrc",
    ".pgpass",
    ".npmrc",
    ".pypirc",
];

/// Canonical credential-store names (path suffix matched) — fail-closed list.
const CREDENTIAL_STORE_SUFFIXES: &[&str] = &[
    "auth.json",
    ".anthropic_oauth.json",
    "mcp-tokens",
    ".env",
    ".git-credentials",
    "operant.toml", // may embed keys in user configs
];

fn is_sensitive_path(path: &Path) -> bool {
    match dirs_home_dir() {
        Some(home) => is_sensitive_path_with_home(path, &home),
        // No HOME: fail closed on anything that looks like a dotfile credential.
        None => true,
    }
}

/// Deny-list check with an explicit home (testable without env mutation).
fn is_sensitive_path_with_home(path: &Path, home: &Path) -> bool {
    for rel in SENSITIVE_HOME_FILES {
        if path == home.join(rel) {
            return true;
        }
    }
    for rel in SENSITIVE_HOME_DIRS {
        if path.starts_with(home.join(rel)) {
            return true;
        }
    }

    // Canonical credential stores anywhere in the path.
    let path_str = path.to_string_lossy();
    for suffix in CREDENTIAL_STORE_SUFFIXES {
        if path_str.ends_with(&format!("/{suffix}")) || path_str.ends_with(suffix) {
            // `.env` must be a full component — the '/' prefix above plus the
            // exact-match below handle it; `my.env` must NOT match.
            if suffix == &".env" && !path_str.ends_with("/.env") && path_str != ".env" {
                continue;
            }
            return true;
        }
    }
    // `.env` exact-component match (covers bare `project/.env`).
    if path_str.ends_with("/.env") || path_str == ".env" {
        return true;
    }
    false
}

fn dirs_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Resolve `target` against `cwd`, enforcing the allowed-root boundary.
/// Returns Err(message) on refusal — fail-closed.
fn resolve_path(cwd: &Path, target: &str, allowed_root: &Path) -> Result<PathBuf, String> {
    let expanded = if target.starts_with('~') {
        match dirs_home_dir() {
            Some(home) => {
                let rest = target.trim_start_matches('~').trim_start_matches('/');
                home.join(rest)
            }
            None => return Err("cannot expand ~ (no HOME set)".to_string()),
        }
    } else {
        PathBuf::from(target)
    };

    let path = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };

    let resolved = path
        .canonicalize()
        .map_err(|_| "path does not exist".to_string())?;
    if !resolved.starts_with(allowed_root) {
        return Err("path is outside the allowed workspace".to_string());
    }
    if is_sensitive_path(&resolved) {
        return Err(
            "path is a sensitive credential or internal path and cannot be attached".to_string(),
        );
    }
    Ok(resolved)
}

// ─── Expansion ──────────────────────────────────────────────────────────

fn code_fence_language(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "json" => "json",
        "md" => "markdown",
        "sh" => "bash",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        _ => "",
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = n as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < 3 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size:.0} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0)
}

fn expand_file_reference(
    ref_: &ContextReference,
    cwd: &Path,
    allowed_root: &Path,
) -> (Option<String>, Option<String>) {
    let path = match resolve_path(cwd, &ref_.target, allowed_root) {
        Ok(p) => p,
        Err(e) => return (Some(format!("{}: {e}", ref_.raw)), None),
    };
    if !path.is_file() {
        return (Some(format!("{}: path is not a file", ref_.raw)), None);
    }
    if is_binary(&path) {
        let meta = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let block = format!(
            "🗂 {} (binary, {}) — use your tools to read/convert/view this file\n```\nPath: {}\n```",
            ref_.raw,
            human_bytes(meta),
            path.display()
        );
        return (None, Some(block));
    }

    let mut text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return (Some(format!("{}: {e}", ref_.raw)), None),
    };
    if let Some(ls) = ref_.line_start {
        let lines: Vec<&str> = text.lines().collect();
        let start_idx = ls.saturating_sub(1);
        let end_idx = ref_.line_end.unwrap_or(ls).min(lines.len());
        if start_idx < end_idx {
            text = lines[start_idx..end_idx].join("\n");
        }
    }

    let lang = code_fence_language(&path);
    let block = format!(
        "📄 {} ({} tokens)\n```{lang}\n{text}\n```",
        ref_.raw,
        estimate_tokens(&text)
    );
    (None, Some(block))
}

fn iter_visible_entries(path: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| {
                        !n.starts_with('.')
                            && n != "__pycache__"
                            && n != "node_modules"
                            && n != "target"
                    })
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        for entry in entries {
            if out.len() >= limit {
                return out;
            }
            out.push(entry.clone());
            if entry.is_dir() {
                stack.push(entry);
            }
        }
    }
    out
}

fn build_folder_listing(path: &Path, cwd: &Path, limit: usize) -> String {
    let rel_root = path.strip_prefix(cwd).unwrap_or(path);
    let root_parts = rel_root.components().count();
    let mut lines = vec![format!("{}/", rel_root.display())];
    let entries = iter_visible_entries(path, limit);
    for entry in &entries {
        let rel = entry.strip_prefix(cwd).unwrap_or(entry);
        let depth = rel
            .components()
            .count()
            .saturating_sub(root_parts)
            .saturating_sub(1);
        let indent = "  ".repeat(depth);
        if entry.is_dir() {
            lines.push(format!(
                "{indent}- {}/",
                entry.file_name().unwrap_or_default().to_string_lossy()
            ));
        } else {
            let meta = std::fs::metadata(entry)
                .map(|m| human_bytes(m.len()))
                .unwrap_or_else(|_| "?".to_string());
            lines.push(format!(
                "{indent}- {} ({meta})",
                entry.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }
    if entries.len() >= limit {
        lines.push("- ...".to_string());
    }
    lines.join("\n")
}

fn expand_folder_reference(
    ref_: &ContextReference,
    cwd: &Path,
    allowed_root: &Path,
) -> (Option<String>, Option<String>) {
    let path = match resolve_path(cwd, &ref_.target, allowed_root) {
        Ok(p) => p,
        Err(e) => return (Some(format!("{}: {e}", ref_.raw)), None),
    };
    if !path.is_dir() {
        return (Some(format!("{}: path is not a folder", ref_.raw)), None);
    }
    let listing = build_folder_listing(&path, cwd, 200);
    let block = format!(
        "📁 {} ({} tokens)\n{listing}",
        ref_.raw,
        estimate_tokens(&listing)
    );
    (None, Some(block))
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("git unavailable: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git command failed".to_string()
        } else {
            stderr
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(if stdout.is_empty() {
        "(no output)".to_string()
    } else {
        stdout
    })
}

fn expand_git_reference(
    ref_: &ContextReference,
    cwd: &Path,
    args: &[&str],
    label: &str,
) -> (Option<String>, Option<String>) {
    match run_git(cwd, args) {
        Ok(content) => {
            let block = format!(
                "🧾 {label} ({} tokens)\n```diff\n{content}\n```",
                estimate_tokens(&content)
            );
            (None, Some(block))
        }
        Err(e) => (Some(format!("{}: {e}", ref_.raw)), None),
    }
}

fn url_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("operant-agent/0.2")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

async fn fetch_url_content(url: &str) -> String {
    match url_client().get(url).send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                return String::new();
            }
            let bytes = resp.bytes().await.unwrap_or_default();
            // Bound the payload — web pages can be multi-MB.
            let bytes = &bytes[..bytes.len().min(512 * 1024)];
            match std::str::from_utf8(bytes) {
                Ok(s) => s.trim().to_string(),
                Err(_) => String::new(),
            }
        }
        Err(e) => {
            debug!(url, error = %e, "@url fetch failed");
            String::new()
        }
    }
}

async fn expand_reference_async(
    ref_: &ContextReference,
    cwd: &Path,
    allowed_root: &Path,
) -> (Option<String>, Option<String>) {
    match ref_.kind {
        ReferenceKind::File => expand_file_reference(ref_, cwd, allowed_root),
        ReferenceKind::Folder => expand_folder_reference(ref_, cwd, allowed_root),
        ReferenceKind::Diff => expand_git_reference(ref_, cwd, &["diff"], "git diff"),
        ReferenceKind::Staged => {
            expand_git_reference(ref_, cwd, &["diff", "--staged"], "git diff --staged")
        }
        ReferenceKind::GitLog => {
            let count = ref_.target.parse::<usize>().unwrap_or(1).clamp(1, 10);
            expand_git_reference(
                ref_,
                cwd,
                &["log", &format!("-{count}"), "-p"],
                &format!("git log -{count} -p"),
            )
        }
        ReferenceKind::Url => {
            let content = fetch_url_content(&ref_.target).await;
            if content.is_empty() {
                return (Some(format!("{}: no content extracted", ref_.raw)), None);
            }
            let block = format!(
                "🌐 {} ({} tokens)\n{content}",
                ref_.raw,
                estimate_tokens(&content)
            );
            (None, Some(block))
        }
    }
}

// ─── Public entry point ─────────────────────────────────────────────────

/// Expand all `@…` references in `message` and return the augmented message.
///
/// `cwd` is the workspace root references resolve against; references cannot
/// escape it. `context_length` is the model's context window used for the
/// 50%/25% injection budget. Mirrors hermes `preprocess_context_references`.
pub async fn preprocess_context_references(
    message: &str,
    cwd: &Path,
    context_length: usize,
) -> ContextReferenceResult {
    let original = message.to_string();
    let refs = parse_context_references(message);
    if refs.is_empty() {
        return ContextReferenceResult {
            message: original.clone(),
            original_message: original,
            references: refs,
            ..Default::default()
        };
    }

    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let allowed_root = cwd.clone();

    let mut warnings = Vec::new();
    let mut blocks = Vec::new();
    let mut injected_tokens = 0usize;

    for ref_ in &refs {
        let (warning, block) = expand_reference_async(ref_, &cwd, &allowed_root).await;
        if let Some(w) = warning {
            warnings.push(w);
        }
        if let Some(b) = block {
            injected_tokens += estimate_tokens(&b);
            blocks.push(b);
        }
    }

    let hard_limit = (context_length as f64 * 0.50).max(1.0) as usize;
    let soft_limit = (context_length as f64 * 0.25).max(1.0) as usize;

    if injected_tokens > hard_limit {
        warnings.push(format!(
            "@ context injection refused: {injected_tokens} tokens exceeds the 50% hard limit ({hard_limit})."
        ));
        return ContextReferenceResult {
            message: original.clone(),
            original_message: original,
            references: refs,
            warnings,
            injected_tokens,
            expanded: true,
            blocked: true,
        };
    }

    if injected_tokens > soft_limit {
        warnings.push(format!(
            "@ context injection warning: {injected_tokens} tokens exceeds the 25% soft limit ({soft_limit})."
        ));
    }

    // Keep the `@file:` tokens in place (they're the visual anchor) and append
    // the warnings + attached context blocks — matches hermes exactly.
    let mut final_msg = original.clone();
    if !warnings.is_empty() {
        final_msg.push_str("\n\n--- Context Warnings ---\n");
        final_msg.push_str(&warnings.join("\n- "));
    }
    if !blocks.is_empty() {
        final_msg.push_str("\n\n--- Attached Context ---\n\n");
        final_msg.push_str(&blocks.join("\n\n"));
    }

    ContextReferenceResult {
        message: final_msg.trim_end().to_string(),
        original_message: original,
        references: refs,
        warnings,
        injected_tokens,
        expanded: true,
        blocked: false,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp_repo() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn write_file(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    #[tokio::test]
    async fn parses_all_kinds() {
        // hermes syntax: @diff / @staged simple forms, @git:N for log.
        let refs = parse_context_references(
            "see @file:src/main.rs:10-20 and @folder:src and @diff and @staged and @git:3 and @url:https://example.com",
        );
        assert_eq!(refs.len(), 6);
        assert_eq!(refs[0].kind, ReferenceKind::File);
        assert_eq!(refs[0].target, "src/main.rs");
        assert_eq!(refs[0].line_start, Some(10));
        assert_eq!(refs[0].line_end, Some(20));
        assert_eq!(refs[1].kind, ReferenceKind::Folder);
        assert_eq!(refs[2].kind, ReferenceKind::Diff);
        assert_eq!(refs[3].kind, ReferenceKind::Staged);
        assert_eq!(refs[4].kind, ReferenceKind::GitLog);
        assert_eq!(refs[4].target, "3");
        assert_eq!(refs[5].kind, ReferenceKind::Url);
        assert_eq!(refs[5].target, "https://example.com");
    }

    #[tokio::test]
    async fn ignores_email_like_at_mentions() {
        // `@user` without a kind suffix is not a reference.
        let refs = parse_context_references("ping @alice about @file:README.md");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].target, "README.md");
    }

    #[tokio::test]
    async fn expands_file_with_line_range() {
        let dir = tmp_repo();
        write_file(
            dir.path(),
            "src/main.rs",
            "line1\nline2\nline3\nline4\nline5\n",
        );
        let result =
            preprocess_context_references("review @file:src/main.rs:2-3", dir.path(), 100_000)
                .await;
        assert!(result.expanded);
        assert!(!result.blocked);
        assert!(result.message.contains("line2"));
        assert!(result.message.contains("line3"));
        assert!(!result.message.contains("line1"));
        assert!(result.message.contains("```rust"));
    }

    #[tokio::test]
    async fn missing_file_becomes_warning() {
        let dir = tmp_repo();
        let result =
            preprocess_context_references("check @file:nope.txt", dir.path(), 100_000).await;
        assert!(result.expanded);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.message.contains("not found") || result.message.contains("does not exist"));
    }

    #[test]
    fn sensitive_home_paths_denied() {
        let dir = tmp_repo();
        let home = dir.path();
        write_file(home, ".ssh/id_rsa", "PRIVATE KEY");
        write_file(home, ".env", "SECRET=1");
        write_file(home, "auth.json", "{\"openai\": \"sk-xxx\"}");
        write_file(home, "project/.env", "DB_PASSWORD=x");
        write_file(home, "safe.txt", "hello");

        assert!(is_sensitive_path_with_home(&home.join(".ssh/id_rsa"), home));
        assert!(is_sensitive_path_with_home(&home.join(".ssh/config"), home));
        assert!(is_sensitive_path_with_home(&home.join(".env"), home));
        assert!(is_sensitive_path_with_home(&home.join("auth.json"), home));
        assert!(is_sensitive_path_with_home(
            &home.join("project/.env"),
            home
        ));
        assert!(is_sensitive_path_with_home(
            &home.join(".aws/credentials"),
            home
        ));
        assert!(!is_sensitive_path_with_home(&home.join("safe.txt"), home));
        assert!(!is_sensitive_path_with_home(&home.join("my.env"), home));
    }

    #[tokio::test]
    async fn refuses_paths_outside_workspace() {
        let dir = tmp_repo();
        // A path outside the tempdir workspace (canonicalize needs it to exist).
        let escape = dir.path().join("../escape.txt");
        let _ = write_file(dir.path().parent().unwrap(), "escape.txt", "secret");
        let result =
            preprocess_context_references("@file:../escape.txt", dir.path(), 100_000).await;
        let _ = std::fs::remove_file(&escape);
        assert!(result.warnings.iter().any(|w| w.contains("outside")));
    }

    #[tokio::test]
    async fn refuses_over_budget_injection() {
        let dir = tmp_repo();
        let big = "x".repeat(100_000);
        write_file(dir.path(), "big.txt", &big);
        let result = preprocess_context_references("@file:big.txt", dir.path(), 10_000).await;
        assert!(result.blocked);
        assert!(result.warnings.iter().any(|w| w.contains("50% hard limit")));
    }

    #[tokio::test]
    async fn folder_listing_expands() {
        let dir = tmp_repo();
        write_file(dir.path(), "src/main.py", "def main():\n    pass\n");
        write_file(dir.path(), "src/helper.py", "x = 1\n");
        let result = preprocess_context_references("list @folder:src", dir.path(), 100_000).await;
        assert!(result.expanded);
        assert!(result.message.contains("main.py"));
        assert!(result.message.contains("helper.py"));
    }

    #[tokio::test]
    async fn no_refs_returns_unchanged() {
        let dir = tmp_repo();
        let result = preprocess_context_references("plain message", dir.path(), 100_000).await;
        assert!(!result.expanded);
        assert_eq!(result.message, "plain message");
    }
}

// Vendored from jcode (crates/operant-logging/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; the whole
// upstream crate landed as one file (it is a self-contained compile-cache
// boundary upstream). See operant_app/mod.rs for scope.
//! Logging infrastructure for jcode
//!
//! Logs to ~/.operant/logs/ with automatic rotation
//!
//! Supports thread-local context for server, session, provider, and model info.

pub mod watchdog;

use chrono::Local;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

use std::sync::{Mutex, OnceLock};

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);
static TASK_LOG_CONTEXTS: OnceLock<Mutex<HashMap<String, LogContext>>> = OnceLock::new();

/// Thread-local logging context
#[derive(Default, Clone)]
pub struct LogContext {
    pub server: Option<String>,
    pub session: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

thread_local! {
    static LOG_CONTEXT: RefCell<LogContext> = RefCell::new(LogContext::default());
}

/// Get the current context as a prefix string
fn context_prefix() -> String {
    if let Some(task_ctx) = task_context_snapshot() {
        return context_prefix_for(&task_ctx);
    }

    LOG_CONTEXT.with(|c| context_prefix_for(&c.borrow()))
}

fn current_task_id() -> Option<String> {
    tokio::task::try_id().map(|id| id.to_string())
}

fn task_context_snapshot() -> Option<LogContext> {
    let task_id = current_task_id()?;
    let store = TASK_LOG_CONTEXTS.get()?;
    let contexts = store.lock().ok()?;
    contexts.get(&task_id).cloned()
}

fn context_prefix_for(ctx: &LogContext) -> String {
    let mut parts = Vec::new();

    if let Some(ref server) = ctx.server {
        parts.push(format!("srv:{}", server));
    }
    if let Some(ref session) = ctx.session {
        // Truncate session name if too long
        let short = if session.len() > 20 {
            &session[..20]
        } else {
            session
        };
        parts.push(format!("ses:{}", short));
    }
    if let Some(ref provider) = ctx.provider {
        parts.push(format!("prv:{}", provider));
    }
    if let Some(ref model) = ctx.model {
        // Just use first part of model name
        let short = model.split('-').next().unwrap_or(model);
        parts.push(format!("mod:{}", short));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("[{}] ", parts.join("|"))
    }
}

pub struct Logger {
    file: File,
}

fn log_dir() -> Option<PathBuf> {
    crate::tui::operant_app::storage::logs_dir().ok()
}

impl Logger {
    // (plan 01, S1b) the only permitted direct-stderr site: the logger
    // itself just failed, so tracing is dead here and a mid-frame
    // corruption is the lesser evil against losing the failure signal.
    #[allow(
        clippy::print_stderr,
        reason = "logger self-failure: tracing is dead here"
    )]
    fn write(&mut self, level: &str, message: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let ctx = context_prefix();
        let line = format!("[{}] [{}] {}{}\n", timestamp, level, ctx, message);
        if let Err(err) = self.file.write_all(line.as_bytes()) {
            eprintln!("operant logger write failed: {err}");
            return;
        }
        if let Err(err) = self.file.flush() {
            eprintln!("operant logger flush failed: {err}");
        }
    }
}

/// Log an info message
#[expect(
    clippy::collapsible_if,
    reason = "Logger lock + optional logger branching is intentionally straightforward"
)]
pub fn info(message: &str) {
    if let Ok(mut guard) = LOGGER.lock() {
        if let Some(logger) = guard.as_mut() {
            logger.write("INFO", message);
        }
    }
}

/// Log an error message
#[expect(
    clippy::collapsible_if,
    reason = "Logger lock + optional logger branching is intentionally straightforward"
)]
pub fn error(message: &str) {
    if let Ok(mut guard) = LOGGER.lock() {
        if let Some(logger) = guard.as_mut() {
            logger.write("ERROR", message);
        }
    }
}

/// Log a warning message
#[expect(
    clippy::collapsible_if,
    reason = "Logger lock + optional logger branching is intentionally straightforward"
)]
pub fn warn(message: &str) {
    if let Ok(mut guard) = LOGGER.lock() {
        if let Some(logger) = guard.as_mut() {
            logger.write("WARN", message);
        }
    }
}

fn format_structured_event<K, V, I>(event: &str, fields: I) -> String
where
    K: AsRef<str>,
    V: ToString,
    I: IntoIterator<Item = (K, V)>,
{
    let event = sanitize_log_value(event);
    let mut ordered = BTreeMap::new();
    for (key, value) in fields {
        let raw_key = key.as_ref();
        let key = sanitize_log_key(raw_key);
        let value = redact_auth_field(raw_key, &value.to_string());
        ordered.insert(key, value);
    }

    if structured_json_enabled() {
        let mut object = serde_json::Map::new();
        object.insert("event".to_string(), serde_json::Value::String(event));
        for (key, value) in ordered {
            object.insert(key, serde_json::Value::String(value));
        }
        return format!("EVENT_JSON {}", serde_json::Value::Object(object));
    }

    let mut parts = Vec::with_capacity(ordered.len() + 1);
    parts.push(format!("event={}", format_log_field_value(&event)));
    parts.extend(
        ordered
            .into_iter()
            .map(|(key, value)| format!("{}={}", key, format_log_field_value(&value))),
    );
    format!("EVENT {}", parts.join(" "))
}

fn structured_json_enabled() -> bool {
    matches!(
        std::env::var("OPERANT_LOG_JSON").as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

fn sanitize_log_key(key: &str) -> String {
    let sanitized: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "field".to_string()
    } else {
        truncate(&sanitized, 80)
    }
}

fn format_log_field_value(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c == '=' || c == '"' || c == '\'')
    {
        serde_json::to_string(value).unwrap_or_else(|_| "\"<unserializable>\"".to_string())
    } else {
        value.to_string()
    }
}

/// Get path to today's log file
pub fn log_path() -> Option<PathBuf> {
    let log_dir = log_dir()?;
    let date = Local::now().format("%Y-%m-%d");
    Some(log_dir.join(format!("operant-{}.log", date)))
}

/// Core of [`cleanup_old_logs_in`], parameterized on the directory and "now" so it
/// can be unit-tested without touching the real log directory or process env.
// (plan 01, S1b) best-effort failure surfacing during log rotation.
#[allow(clippy::print_stderr, reason = "logger cleanup: best-effort surfacing")]
fn cleanup_old_logs_in(log_dir: &std::path::Path, now: chrono::DateTime<Local>) {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = now - chrono::Duration::days(7);
    for entry in entries.flatten() {
        // Only consider our own date-stamped log files.
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_operant_log = (name.starts_with("operant-") || name.starts_with("operant-desktop-"))
            && name.ends_with(".log");
        if !is_operant_log {
            continue;
        }

        if let Ok(metadata) = entry.metadata()
            && metadata.is_file()
            && let Ok(modified) = metadata.modified()
        {
            let modified: chrono::DateTime<Local> = modified.into();
            if modified < cutoff
                && let Err(err) = fs::remove_file(entry.path())
            {
                eprintln!("operant logger cleanup failed: {err}");
            }
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!(
            "{}...",
            crate::tui::operant_app::util::truncate_str(s, max_len)
        )
    } else {
        s.to_string()
    }
}

fn redact_auth_field(key: &str, value: &str) -> String {
    let key = key.to_ascii_lowercase();
    if key.contains("token")
        || key.contains("secret")
        || key.contains("key")
        || key.contains("credential")
        || key.contains("callback")
        || key.contains("authorization")
        || key.contains("auth_code")
        || key.contains("oauth_code")
        || key.contains("code_verifier")
        || key.contains("code_challenge")
    {
        return "<redacted>".to_string();
    }
    sanitize_log_value(value)
}

fn sanitize_log_value(value: &str) -> String {
    let value = value.replace(['\n', '\r', '\t'], " ");
    let value = redact_url_queries(&value);
    truncate(&value, 160)
}

fn redact_url_queries(value: &str) -> String {
    value
        .split(' ')
        .map(|word| {
            if (word.starts_with("http://") || word.starts_with("https://")) && word.contains('?') {
                let (base, _) = word.split_once('?').unwrap_or((word, ""));
                format!("{}?<redacted>", base)
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_log_redacts_secret_like_fields() {
        assert_eq!(redact_auth_field("api_key", "sk-secret"), "<redacted>");
        assert_eq!(
            redact_auth_field("callback_url", "https://example.com/?code=secret"),
            "<redacted>"
        );
    }

    #[test]
    fn auth_log_sanitizes_urls_and_control_characters() {
        assert_eq!(
            sanitize_log_value("failed\nhttps://login.example.com/cb?code=secret&state=abc"),
            "failed https://login.example.com/cb?<redacted>"
        );
    }

    #[test]
    fn structured_event_orders_and_redacts_fields() {
        let line = format_structured_event(
            "server_request",
            vec![
                ("z", "last"),
                ("api_key", "sk-secret"),
                ("a field", "hello world"),
            ],
        );
        assert_eq!(
            line,
            "EVENT event=server_request a_field=\"hello world\" api_key=<redacted> z=last"
        );
    }

    #[test]
    fn structured_event_redacts_url_queries() {
        let line = format_structured_event(
            "callback",
            vec![("url", "https://example.test/cb?code=secret&state=abc")],
        );
        assert_eq!(
            line,
            "EVENT event=callback url=https://example.test/cb?<redacted>"
        );
    }

    #[test]
    fn structured_event_keeps_non_secret_code_fields() {
        let line = format_structured_event("tool_done", vec![("exit_code", "127")]);
        assert_eq!(line, "EVENT event=tool_done exit_code=127");
    }

    #[test]
    fn diagnostic_field_names_are_not_redacted() {
        // The model-picker / login diagnostics intentionally avoid field names
        // that collide with the secret-redaction heuristic (which masks any
        // field whose name contains key/token/secret/credential/...). If any of
        // these regress, the uploaded logs we ask users for would show
        // `<redacted>` instead of the boolean/count/env-var-name we need.
        for name in [
            "env_var",
            "input_len",
            "optional",
            "anthropic_api",
            "openai_api",
            "azure_api",
            "copilot_cred",
            "session_provider",
            "routes_in",
            "by_provider",
            "requested_model",
            "route_provider",
        ] {
            assert_eq!(
                redact_auth_field(name, "value"),
                "value",
                "diagnostic field `{name}` should not be redacted",
            );
        }
    }

    #[test]
    fn cleanup_removes_only_old_operant_logs() {
        use std::time::{Duration, SystemTime};

        let dir = std::env::temp_dir().join(format!(
            "operant-log-cleanup-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).expect("create temp log dir");

        let old_mtime = SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30); // 30 days

        let write = |name: &str, age_old: bool| {
            let path = dir.join(name);
            let mut f = File::create(&path).expect("create file");
            f.write_all(b"x").ok();
            if age_old {
                f.set_modified(old_mtime).expect("set mtime");
            }
            path
        };

        // Old log files that SHOULD be deleted.
        let old_log = write("operant-2000-01-01.log", true);
        let old_desktop = write("operant-desktop-2000-01-01.log", true);
        // Recent log file that SHOULD survive.
        let new_log = write("operant-2099-01-01.log", false);
        // Non-log data that SHOULD survive even though it is old.
        let old_memory = write("memory-events-2000-01-01.jsonl", true);
        let old_other = write("notes-2000-01-01.txt", true);
        // A subdirectory (e.g. `memory/`) must never be removed.
        let subdir = dir.join("memory");
        fs::create_dir_all(&subdir).expect("create subdir");

        cleanup_old_logs_in(&dir, Local::now());

        assert!(!old_log.exists(), "old jcode log should be deleted");
        assert!(!old_desktop.exists(), "old desktop log should be deleted");
        assert!(new_log.exists(), "recent jcode log must survive");
        assert!(old_memory.exists(), "memory-events jsonl must survive");
        assert!(old_other.exists(), "unrelated files must survive");
        assert!(subdir.is_dir(), "subdirectories must survive");

        fs::remove_dir_all(&dir).ok();
    }
}

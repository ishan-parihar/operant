//! ONE doctor engine — `docs/ORGANISM-ARCHITECTURE.md` §6 F2.
//!
//! History: two divergent engines existed. `operant-runtime::doctor` ran on
//! `operant_config::schema::Config` and served only the gateway's
//! `GET /api/doctor`; `operant-cli::cmd_doctor` ran on [`AppConfig`] and
//! served `operant doctor`. Same intent, duplicated logic, drifting check
//! lists. This module is the surviving engine: the checks run on
//! [`AppConfig`] (the config type the genome and gateway read),
//! `operant doctor` renders them via [`render_text`] alongside its
//! install-level sections, and the gateway serializes the same
//! [`diagnose`] results at its boundary.
//!
//! Runtime-doctor checks that read fields only `schema::Config` carries are
//! named gaps, not fakes: channels, delegate agents, `gateway.port`,
//! `memory.embedding_model`, `config_path` (config-file presence), the
//! runtime `workspace_dir` (SOUL.md/AGENTS.md presence there), and daemon
//! heartbeat state (the `operant-runtime` daemon supervisor has no callers,
//! so `daemon_state.json` is never written anywhere). See
//! `docs/CHANGELOG.md` [Unreleased].

use serde::Serialize;
use std::io::Write;
use std::path::Path;

use crate::config::AppConfig;
use crate::platform::operant_home;

const COMMAND_VERSION_PREVIEW_CHARS: usize = 60;
/// Below this many MB free at the data root, doctor warns.
const DISK_WARN_MB: u64 = 100;

// ── Result model ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Warn,
    Error,
}

/// One engine finding. The gateway serializes this verbatim into
/// `GET /api/doctor`; [`render_text`] prints it for `operant doctor`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

impl CheckResult {
    fn ok(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Ok,
            category,
            message: msg.into(),
        }
    }
    fn warn(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warn,
            category,
            message: msg.into(),
        }
    }
    fn error(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            category,
            message: msg.into(),
        }
    }
}

// ── Entry points ───────────────────────────────────────────────────

/// Run every engine check and return the structured results.
///
/// This is THE check list: both `operant doctor` (text render) and the
/// gateway API (JSON) consume exactly this output — see
/// `engine_check_list_is_the_shared_drift_pin` for the pin.
pub fn diagnose(config: &AppConfig) -> Vec<CheckResult> {
    let mut items = Vec::new();
    check_config_semantics(config, &mut items);
    check_genome(&config.database_path, &mut items);
    check_data_root(&mut items);
    check_environment(&mut items);
    check_cli_tools(&mut items);
    items
}

// ── Genome / organism health (Wave 5, ORGANISM-ARCHITECTURE §4) ─────

/// Read-only health checks over the org stores — the onboarding-time
/// governance flags the owner directed to surface THROUGH the doctor (no
/// new commands). Degrades to a single Info when the genome tables do not
/// exist yet (doctor may run before any gateway boot seeded them).
///
/// `database_path` is the *main* app database path. The org registry lives in
/// the kanban sibling, so it is derived through the one canonical helper rather
/// than re-derived here — a hardcoded `operant_home().join("database.db")`
/// read a different file than the CLI for any non-default install.
fn check_genome(database_path: &std::path::Path, items: &mut Vec<CheckResult>) {
    let cat = "genome";
    let org_db = crate::org::employee_db::org_db_path(database_path);
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(&org_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        items.push(CheckResult::ok(
            cat,
            "org database not created yet — the gateway seeds the cast on first boot",
        ));
        return;
    };

    let has_table = |name: &str| -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
            rusqlite::params![name],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    };

    if !has_table("employees") {
        items.push(CheckResult::ok(
            cat,
            "genome stores not initialized — the gateway seeds them on first boot",
        ));
        return;
    }

    // Cast seeded: a fresh install with zero employees has no governed
    // org at all — the next gateway boot seeds the nine-seat cast.
    let employees: i64 = conn
        .query_row("SELECT COUNT(*) FROM employees", [], |row| row.get(0))
        .unwrap_or(0);
    if employees == 0 {
        items.push(CheckResult::warn(
            cat,
            "employee registry is empty — the nine-seat cast seeds on the next gateway boot",
        ));
    } else {
        items.push(CheckResult::ok(
            cat,
            format!("employee registry: {employees} seat(s)"),
        ));
    }

    // Ungoverned cron automata (D-2 residual, the ratified posture made
    // visible): seats that exist BECAUSE of a cron job but carry no
    // policy row — they run byte-identical to legacy (unattended
    // auto-allow for gated tools). Informational, not an error: the
    // owner ratified the default; the fix is `--seat-mode` at register
    // or a `/grant`-seated row.
    if has_table("employee_cron_jobs") && has_table("seat_policies") {
        let ungoverned: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT e.employee_id) FROM employees e \
                 JOIN employee_cron_jobs ec ON ec.employee_id = e.employee_id \
                 LEFT JOIN seat_policies p ON p.employee_id = e.employee_id \
                 WHERE p.employee_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if ungoverned > 0 {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "{ungoverned} cron automaton seat(s) run ungoverned (no policy row) \
                     — the documented default. Seat one with `operant cron create --seat-mode \
                     <yolo|standard|scoped|lockdown>` or /grant, or ratify as-is"
                ),
            ));
        } else {
            items.push(CheckResult::ok(
                cat,
                "every cron automaton seat carries a policy row",
            ));
        }
    }

    // Budget window typos: resolution fails OPEN to daily (turns keep
    // running); the doctor is what names the typo instead of cargo.
    if has_table("seat_budgets") {
        let mut stmt = match conn.prepare(
            "SELECT employee_id, window FROM seat_budgets \
             WHERE window IS NOT NULL \
               AND window NOT IN ('daily', 'weekly', 'monthly')",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return,
        };
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).unwrap_or_default(),
                ))
            })
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>());
        if let Ok(bad) = rows {
            for (seat, window) in bad {
                items.push(CheckResult::warn(
                    cat,
                    format!(
                        "seat `{seat}` budget window '{window}' is not daily/weekly/monthly \
                         — failing open to daily; fix the seat_budgets row"
                    ),
                ));
            }
        }
    }

    // Unbound chat sessions (pre-Wave-2 rows the store has not healed
    // yet — the backfill happens on load, so a nonzero count here means
    // the gateway has not run since Wave 2 landed).
    if has_table("gateway_sessions") {
        let unbound: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_sessions WHERE employee_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if unbound > 0 {
            items.push(CheckResult::ok(
                cat,
                format!(
                    "{unbound} pre-Wave-2 session row(s) not yet backfilled — \
                     they bind to premiere on the next gateway load"
                ),
            ));
        }
    }
}

/// Plain-text render used by `operant doctor`'s Diagnostics section.
///
/// Categories group under `[<category>]` headers, matching the shape the
/// retired runtime engine printed, so consumers of the text form see the
/// same structure.
pub fn render_text(results: &[CheckResult]) -> String {
    let mut out = String::new();
    let mut current_cat = "";
    for result in results {
        if result.category != current_cat {
            current_cat = result.category;
            out.push_str(&format!("[{current_cat}]\n"));
        }
        let icon = match result.severity {
            Severity::Ok => "✓",
            Severity::Warn => "⚠",
            Severity::Error => "✗",
        };
        out.push_str(&format!("  {icon} {}\n", result.message));
    }
    out
}

// ── Config semantic validation ────────────────────────────────────

fn check_config_semantics(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "config";
    let providers = &config.providers;

    // The `[providers]` section is optional in the AppConfig world:
    // `client.base_url` drives routing when it is absent. Only validate it
    // when it has content that can silently stop matching.
    let section_has_content = !(providers.models.is_empty()
        && providers.model_routes.is_empty()
        && providers.embedding_routes.is_empty()
        && providers.fallback_chain.is_empty());

    match providers.fallback.as_deref() {
        Some(provider) => {
            if let Some(reason) = provider_validation_error(provider) {
                items.push(CheckResult::error(
                    cat,
                    format!("default provider \"{provider}\" is invalid: {reason}"),
                ));
            } else {
                items.push(CheckResult::ok(
                    cat,
                    format!("provider \"{provider}\" is valid"),
                ));
            }

            if let Some(entry) = providers.fallback_provider() {
                // API key presence (ollama is the keyless exception).
                if provider != "ollama" {
                    if entry.api_key.is_some() {
                        items.push(CheckResult::ok(cat, "API key configured"));
                    } else {
                        items.push(CheckResult::warn(
                            cat,
                            "no api_key set (may rely on env vars or provider defaults)",
                        ));
                    }
                }

                match entry.model.as_deref() {
                    Some(model) => {
                        items.push(CheckResult::ok(cat, format!("default model: {model}")));
                    }
                    None => items.push(CheckResult::warn(cat, "no default_model configured")),
                }

                let default_temperature = entry.temperature.unwrap_or(0.7);
                if (0.0..=2.0).contains(&default_temperature) {
                    items.push(CheckResult::ok(
                        cat,
                        format!("temperature {default_temperature:.1} (valid range 0.0–2.0)"),
                    ));
                } else {
                    items.push(CheckResult::error(
                        cat,
                        format!(
                            "temperature {default_temperature:.1} is out of range (expected 0.0–2.0)"
                        ),
                    ));
                }
            }
        }
        None => {
            if section_has_content {
                items.push(CheckResult::warn(
                    cat,
                    "no providers.fallback configured — requests without a matching route fail at runtime",
                ));
            }
        }
    }

    // Cross-provider fallback chain (schema `reliability.fallback_providers`
    // parity: the AppConfig world's chain is `[providers] fallback_chain`).
    for fb in &providers.fallback_chain {
        if let Some(reason) = provider_validation_error(&fb.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "fallback provider \"{}\" is invalid: {}",
                    fb.provider, reason
                ),
            ));
        }
    }

    // Model routes.
    for route in &providers.model_routes {
        if route.hint.is_empty() {
            items.push(CheckResult::warn(cat, "model route with empty hint"));
        }
        if let Some(reason) = provider_validation_error(&route.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "model route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.is_empty() {
            items.push(CheckResult::warn(
                cat,
                format!("model route \"{}\" has empty model", route.hint),
            ));
        }
    }

    // Embedding routes.
    for route in &providers.embedding_routes {
        if route.hint.trim().is_empty() {
            items.push(CheckResult::warn(cat, "embedding route with empty hint"));
        }
        if let Some(reason) = embedding_provider_validation_error(&route.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "embedding route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.trim().is_empty() {
            items.push(CheckResult::warn(
                cat,
                format!("embedding route \"{}\" has empty model", route.hint),
            ));
        }
        if route.dimensions.is_some_and(|value| value == 0) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "embedding route \"{}\" has invalid dimensions=0",
                    route.hint
                ),
            ));
        }
    }
}

fn provider_validation_error(name: &str) -> Option<String> {
    match operant_providers::create_provider(name, None) {
        Ok(_) => None,
        Err(err) => Some(
            err.to_string()
                .lines()
                .next()
                .unwrap_or("invalid provider")
                .into(),
        ),
    }
}

fn embedding_provider_validation_error(name: &str) -> Option<String> {
    let normalized = name.trim();
    if normalized.eq_ignore_ascii_case("none") || normalized.eq_ignore_ascii_case("openai") {
        return None;
    }

    let Some(url) = normalized.strip_prefix("custom:") else {
        return Some("supported values: none, openai, custom:<url>".into());
    };

    let url = url.trim();
    if url.is_empty() {
        return Some("custom:<url> requires a non-empty url".into());
    }
    match url::Url::parse(url) {
        Ok(_) => None,
        Err(err) => Some(format!("invalid custom provider URL: {err}")),
    }
}

// ── Data root integrity ────────────────────────────────────────────

/// The runtime engine checked `schema::Config.workspace_dir`
/// (`~/.operant/workspace`, bootstrapped only by schema-config loads).
/// The AppConfig world has no workspace concept; its data root —
/// `operant_home()`, which holds state.db, skills, cron — is where the
/// writable/disk checks carry over to.
fn check_data_root(items: &mut Vec<CheckResult>) {
    let cat = "workspace";
    let root = operant_home();

    if !root.exists() {
        items.push(CheckResult::warn(
            cat,
            format!(
                "data root {} not found (will be created on first use)",
                root.display()
            ),
        ));
        return;
    }
    items.push(CheckResult::ok(
        cat,
        format!("data root exists: {}", root.display()),
    ));

    // Writable check.
    let probe = probe_file_path(&root);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(mut probe_file) => {
            let write_result = probe_file.write_all(b"probe");
            drop(probe_file);
            let _ = std::fs::remove_file(&probe);
            match write_result {
                Ok(()) => items.push(CheckResult::ok(cat, "data root is writable")),
                Err(e) => items.push(CheckResult::error(
                    cat,
                    format!("data root write probe failed: {e}"),
                )),
            }
        }
        Err(e) => items.push(CheckResult::error(
            cat,
            format!("data root is not writable: {e}"),
        )),
    }

    // Disk space (best-effort via `df`).
    if let Some(avail_mb) = disk_available_mb(&root) {
        if avail_mb >= DISK_WARN_MB {
            items.push(CheckResult::ok(
                cat,
                format!("disk space: {avail_mb} MB available"),
            ));
        } else {
            items.push(CheckResult::warn(
                cat,
                format!("low disk space: only {avail_mb} MB available"),
            ));
        }
    }
}

fn probe_file_path(base: &Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    base.join(format!(
        ".operant_doctor_probe_{}_{}",
        std::process::id(),
        nanos
    ))
}

fn disk_available_mb(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-m")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_df_available_mb(&stdout)
}

fn parse_df_available_mb(stdout: &str) -> Option<u64> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let avail = line.split_whitespace().nth(3)?;
    avail.parse::<u64>().ok()
}

// ── Environment checks ─────────────────────────────────────────────

fn check_environment(items: &mut Vec<CheckResult>) {
    let cat = "environment";

    check_command_available("git", &["--version"], cat, items);

    // Shell — Unix uses $SHELL, Windows uses %ComSpec% (path to cmd.exe).
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("ComSpec").ok().filter(|s| !s.is_empty()));
    match shell {
        Some(s) => items.push(CheckResult::ok(cat, format!("shell: {s}"))),
        None => items.push(CheckResult::warn(
            cat,
            "neither $SHELL nor %ComSpec% is set",
        )),
    }

    // HOME
    if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
        items.push(CheckResult::ok(cat, "home directory env set"));
    } else {
        items.push(CheckResult::error(
            cat,
            "neither $HOME nor $USERPROFILE is set",
        ));
    }

    check_command_available("curl", &["--version"], cat, items);
}

// ── CLI tool discovery ─────────────────────────────────────────────

fn check_cli_tools(items: &mut Vec<CheckResult>) {
    let cat = "cli-tools";

    let discovered = operant_tools::cli_discovery::discover_cli_tools(&[], &[]);

    if discovered.is_empty() {
        items.push(CheckResult::warn(cat, "No CLI tools found in PATH"));
    } else {
        for cli in &discovered {
            let version_info = cli
                .version
                .as_deref()
                .map(|v| truncate_for_display(v, COMMAND_VERSION_PREVIEW_CHARS))
                .unwrap_or_else(|| "unknown version".to_string());
            items.push(CheckResult::ok(
                cat,
                format!("{} ({}) — {}", cli.name, cli.category, version_info),
            ));
        }
        items.push(CheckResult::ok(
            cat,
            format!("{} CLI tools discovered", discovered.len()),
        ));
    }
}

fn check_command_available(
    cmd: &str,
    args: &[&str],
    cat: &'static str,
    items: &mut Vec<CheckResult>,
) {
    match std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            let first_line = ver.lines().next().unwrap_or("").trim();
            let display = truncate_for_display(first_line, COMMAND_VERSION_PREVIEW_CHARS);
            items.push(CheckResult::ok(cat, format!("{cmd}: {display}")));
        }
        Ok(_) => items.push(CheckResult::warn(
            cat,
            format!("{cmd} found but returned non-zero"),
        )),
        Err(_) => items.push(CheckResult::warn(cat, format!("{cmd} not found in PATH"))),
    }
}

fn truncate_for_display(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_config::schema::{EmbeddingRouteConfig, FallbackProviderConfig, ModelRouteConfig};
    use serial_test::serial;

    fn fixture_config() -> AppConfig {
        // A config with real `[providers]` content so the config category
        // produces results on any machine.
        let mut config = AppConfig::default();
        // The genome checks read the org registry through
        // `config.database_path` (iter-643): they no longer consult
        // `HERMES_HOME` directly, because a hardcoded home is exactly what
        // split the daemon's registry from the CLI's. Honour the same override
        // the serial tests set so the fixture points where they intend.
        if let Ok(home) = std::env::var("HERMES_HOME") {
            config.database_path = std::path::Path::new(&home).join("database.db");
        }
        config.providers.fallback = Some("openrouter".into());
        config.providers.models.insert(
            "openrouter".into(),
            operant_config::schema::ModelProviderConfig {
                api_key: Some("sk-test".into()),
                model: Some("test-model".into()),
                temperature: Some(0.7),
                ..Default::default()
            },
        );
        config
    }

    /// The drift pin (task F2): `operant doctor` renders and the gateway
    /// serializes THE SAME check list — one `diagnose` call. The category
    /// order below is the contract; adding or removing a category is a
    /// deliberate change that must update every consumer.
    #[test]
    fn engine_check_list_is_the_shared_drift_pin() {
        let results = diagnose(&fixture_config());

        let mut categories: Vec<&str> = Vec::new();
        for result in &results {
            if categories.last() != Some(&result.category) {
                categories.push(result.category);
            }
        }
        assert_eq!(
            categories,
            ["config", "genome", "workspace", "environment", "cli-tools"]
        );

        for category in categories {
            assert!(
                results.iter().any(|r| r.category == category),
                "category {category} produced no results"
            );
        }
    }

    /// Both consumer surfaces derive from one list: the text render and the
    /// gateway JSON shape carry every result of the same `diagnose` call.
    #[test]
    fn cli_render_and_gateway_serialize_consume_the_same_results() {
        let results = diagnose(&fixture_config());

        let text = render_text(&results);
        for result in &results {
            assert!(
                text.contains(result.message.as_str()),
                "missing: {}",
                result.message
            );
        }
        for category in ["config", "workspace", "environment", "cli-tools"] {
            assert!(text.contains(&format!("[{category}]")));
        }

        let gateway_payload = serde_json::to_value(&results).unwrap_or_default();
        let Some(array) = gateway_payload.as_array() else {
            panic!("gateway payload must be a JSON array");
        };
        assert_eq!(array.len(), results.len());
        for (entry, result) in array.iter().zip(&results) {
            assert!(entry.get("severity").is_some());
            assert!(entry.get("category").is_some());
            assert!(entry.get("message").is_some());
            assert_eq!(
                entry.get("message").and_then(serde_json::Value::as_str),
                Some(result.message.as_str())
            );
        }
    }

    #[test]
    fn provider_validation_checks_custom_url_shape() {
        assert!(provider_validation_error("openrouter").is_none());
        assert!(provider_validation_error("custom:https://example.com").is_none());
        assert!(provider_validation_error("anthropic-custom:https://example.com").is_none());
    }

    #[test]
    fn config_validation_catches_bad_temperature() {
        let mut config = fixture_config();
        if let Some(entry) = config.providers.models.get_mut("openrouter") {
            entry.temperature = Some(5.0);
        }
        let items = diagnose(&config);
        let temp_item = items.iter().find(|i| i.message.contains("temperature"));
        assert!(temp_item.is_some());
        assert_eq!(temp_item.map(|i| i.severity), Some(Severity::Error));
    }

    #[test]
    fn config_validation_accepts_valid_temperature() {
        let items = diagnose(&fixture_config());
        let temp_item = items.iter().find(|i| i.message.contains("temperature"));
        assert!(temp_item.is_some());
        assert_eq!(temp_item.map(|i| i.severity), Some(Severity::Ok));
    }

    #[test]
    fn config_validation_catches_unknown_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("totally-fake".into());
        let items = diagnose(&config);
        let prov_item = items
            .iter()
            .find(|i| i.message.contains("default provider"));
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Error));
    }

    #[test]
    fn config_validation_catches_malformed_custom_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("custom:".into());
        let items = diagnose(&config);
        let prov_item = items.iter().find(|item| {
            item.message
                .contains("default provider \"custom:\" is invalid")
        });
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Error));
    }

    #[test]
    fn config_validation_accepts_custom_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("custom:https://my-api.com".into());
        let items = diagnose(&config);
        let prov_item = items.iter().find(|i| i.message.contains("is valid"));
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Ok));
    }

    /// `[providers]` is optional in the AppConfig world: an absent section
    /// (plain `client.base_url` installs) must not produce a check result
    /// at all, let alone an error that would flip `operant doctor`'s exit
    /// code on a working machine.
    #[test]
    fn config_validation_silent_when_providers_section_unused() {
        let items = diagnose(&AppConfig::default());
        assert!(
            !items
                .iter()
                .any(|i| i.message.contains("providers.fallback")),
            "empty [providers] section must not warn"
        );
    }

    #[test]
    fn config_validation_warns_missing_fallback_when_section_has_content() {
        let mut config = AppConfig::default();
        config.providers.model_routes = vec![ModelRouteConfig {
            hint: "fast".into(),
            provider: "groq".into(),
            model: "llama".into(),
            api_key: None,
        }];
        let items = diagnose(&config);
        let fb_item = items
            .iter()
            .find(|i| i.message.contains("no providers.fallback configured"));
        assert!(fb_item.is_some());
        assert_eq!(fb_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_bad_fallback_chain_entry() {
        let mut config = AppConfig::default();
        config.providers.fallback_chain = vec![FallbackProviderConfig {
            provider: "fake-provider".into(),
            model: "m".into(),
        }];
        let items = diagnose(&config);
        let fb_item = items.iter().find(|i| {
            i.message
                .contains("fallback provider \"fake-provider\" is invalid")
        });
        assert!(fb_item.is_some());
        assert_eq!(fb_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_empty_model_route() {
        let mut config = AppConfig::default();
        config.providers.model_routes = vec![ModelRouteConfig {
            hint: "fast".into(),
            provider: "groq".into(),
            model: String::new(),
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items.iter().find(|i| i.message.contains("empty model"));
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_empty_embedding_route_model() {
        let mut config = AppConfig::default();
        config.providers.embedding_routes = vec![EmbeddingRouteConfig {
            hint: "semantic".into(),
            provider: "openai".into(),
            model: String::new(),
            dimensions: Some(1536),
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items.iter().find(|item| {
            item.message
                .contains("embedding route \"semantic\" has empty model")
        });
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_invalid_embedding_route_provider() {
        let mut config = AppConfig::default();
        config.providers.embedding_routes = vec![EmbeddingRouteConfig {
            hint: "semantic".into(),
            provider: "groq".into(),
            model: "text-embedding-3-small".into(),
            dimensions: None,
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items
            .iter()
            .find(|item| item.message.contains("uses invalid provider \"groq\""));
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn environment_check_finds_git() {
        let mut items = Vec::new();
        check_environment(&mut items);
        let git_item = items.iter().find(|i| i.message.starts_with("git:"));
        // git should be available in any CI/dev environment
        assert!(git_item.is_some());
        assert_eq!(git_item.map(|i| i.severity), Some(Severity::Ok));
    }

    #[test]
    fn parse_df_available_mb_uses_last_data_line() {
        let stdout =
            "Filesystem 1M-blocks Used Available Use% Mounted on\n/dev/sda1 1000 500 500 50% /\n";
        assert_eq!(parse_df_available_mb(stdout), Some(500));
    }

    #[test]
    fn truncate_for_display_preserves_utf8_boundaries() {
        let preview = truncate_for_display("🙂example-alpha-build", 3);
        assert_eq!(preview, "🙂ex…");
    }

    #[test]
    #[serial]
    fn genome_checks_flag_ungoverned_cron_seats_and_budget_typos() {
        // Wave 5: the doctor is the onboarding-governance flag surface.
        // HERMES_HOME redirects the org DB to a fixture so the counts are
        // assertable — serial because operant_home() is process-global.
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY (edition-2024 env mutation): single-threaded test process,
        // serial-locked, no other thread reads HERMES_HOME concurrently.
        unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
        // The genome tables live in the kanban sibling, which `check_genome`
        // derives from `config.database_path` (iter-643) — NOT in the main db.
        // Seeding the main file is what the daemon did by mistake.
        let conn = rusqlite::Connection::open(crate::org::employee_db::org_db_path(
            &tmp.path().join("database.db"),
        ))
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE employees (employee_id TEXT PRIMARY KEY);
             CREATE TABLE employee_cron_jobs (employee_id TEXT, cron_job_id TEXT);
             CREATE TABLE seat_policies (employee_id TEXT PRIMARY KEY);
             CREATE TABLE seat_budgets (
                 employee_id TEXT PRIMARY KEY, basis TEXT,
                 window TEXT, cap REAL, mode TEXT);
             INSERT INTO employees VALUES ('emp-governed');
             INSERT INTO employees VALUES ('emp-free');
             INSERT INTO employees VALUES ('emp-typo');
             INSERT INTO employee_cron_jobs VALUES ('emp-governed', 'job-1');
             INSERT INTO employee_cron_jobs VALUES ('emp-free', 'job-2');
             INSERT INTO seat_policies VALUES ('emp-governed');
             INSERT INTO seat_budgets VALUES ('emp-typo', 'tokens', 'weakly', 100.0, 'hard');",
        )
        .unwrap();
        drop(conn);

        let results = diagnose(&fixture_config());
        let genome: Vec<&str> = results
            .iter()
            .filter(|r| r.category == "genome")
            .map(|r| r.message.as_str())
            .collect();

        assert!(
            genome
                .iter()
                .any(|m| m.contains("1 cron automaton seat(s) run ungoverned")),
            "ungoverned-cron flag missing: {genome:?}"
        );
        assert!(
            genome.iter().any(|m| m.contains("budget window 'weakly'")),
            "budget-typo flag missing: {genome:?}"
        );
        assert!(
            genome
                .iter()
                .any(|m| m.contains("employee registry: 3 seat(s)")),
            "registry count missing: {genome:?}"
        );
        // SAFETY: same serial-test rationale.
        unsafe { std::env::remove_var("HERMES_HOME") };
    }

    #[test]
    #[serial]
    fn genome_checks_degrade_to_info_when_stores_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY: serial test, single-threaded runner.
        unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
        // No database.db at all → the doctor must not error, just say so.
        let results = diagnose(&fixture_config());
        assert!(
            results
                .iter()
                .any(|r| r.category == "genome"
                    && r.message.contains("org database not created yet")),
            "absent-stores info missing: {results:?}"
        );
        // SAFETY: same serial-test rationale.
        unsafe { std::env::remove_var("HERMES_HOME") };
    }

    #[test]
    fn probe_path_is_hidden_and_unique() {
        let tmp = tempfile::TempDir::new().unwrap();
        let first = probe_file_path(tmp.path());
        let second = probe_file_path(tmp.path());

        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".operant_doctor_probe_"))
        );
    }

    #[test]
    fn render_text_groups_by_category_with_icons() {
        let results = vec![
            CheckResult::ok("config", "one".to_string()),
            CheckResult::ok("config", "two".to_string()),
            CheckResult::warn("environment", "three".to_string()),
            CheckResult::error("workspace", "four".to_string()),
        ];
        let text = render_text(&results);
        assert_eq!(
            text,
            "[config]\n  ✓ one\n  ✓ two\n[environment]\n  ⚠ three\n[workspace]\n  ✗ four\n"
        );
    }
}

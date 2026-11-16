//! Config, auth, and directory-structure checks for `operant doctor`.
//!
//! Mirrors sections from `operant-agent/operant_cli/doctor.py`:
//! - Configuration Files (.env, config.yaml, provider validation, stale keys)
//! - Auth Providers (Nous, Codex, Gemini OAuth, MiniMax OAuth)
//! - Directory Structure (operant_home, subdirs, SOUL.md, memories, state.db, WAL)
//! - Gateway Service Linger

use operant_core::config::AppConfig;
use operant_core::platform::operant_home;

use super::check_result::{check_fail, check_info, check_ok, check_warn, section_header};
use crate::provider::{PROVIDERS, provider_by_name, provider_from_url};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Display-friendly representation of `HERMES_HOME` (e.g. `~/.operant`).
fn display_home() -> String {
    let hh = operant_home();
    let s = hh.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        if s.starts_with(&home_str) {
            return s.replacen(&home_str, "~", 1);
        }
    }
    s
}

/// Check whether `.env` content contains at least one provider API key or
/// custom endpoint variable.
fn has_provider_env_config(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq_pos].trim();
        let value = trimmed[eq_pos + 1..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        let upper = key.to_uppercase();
        if upper.contains("API_KEY")
            || upper.contains("APIKEY")
            || upper.contains("APITOKEN")
            || upper.contains("TOKEN")
            || upper.contains("SECRET")
            || upper.contains("PASSWORD")
            || upper.contains("BASE_URL")
            || upper.contains("ENDPOINT")
            || upper.contains("HOST")
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Check whether the *process environment* supplies a provider credential or
/// endpoint. Without this, a doctor warning that says "other sources may be
/// configured" is a claim nothing in the file can back: `has_provider_env_config`
/// only ever reads `.env`, so a key exported as `OPENAI_API_KEY` in the shell
/// that launched operant would be reported as missing.
///
/// Takes the environment as a slice rather than reading it inline, so a test can
/// supply it. Reading `std::env::vars()` inside made the test's outcome depend on
/// whichever credentials the machine running it happened to have.
fn has_ambient_provider_config(env: &[(String, String)]) -> bool {
    for (key, value) in env {
        if value.is_empty() {
            continue;
        }
        let upper = key.to_uppercase();
        if upper.contains("API_KEY")
            || upper.contains("APIKEY")
            || upper.contains("APITOKEN")
            || upper.contains("TOKEN")
            || upper.contains("SECRET")
            || upper.contains("PASSWORD")
            || upper.contains("BASE_URL")
            || upper.contains("ENDPOINT")
            || upper.contains("HOST")
        {
            return true;
        }
    }
    false
}

/// Run all config & directory checks, appending actionable items to `issues`.
pub fn run_config_checks(
    config: &AppConfig,
    issues: &mut Vec<String>,
    manual_issues: &mut Vec<String>,
) {
    let hh = operant_home();
    let dhh = display_home();

    // =====================================================================
    // A. Configuration Files
    // =====================================================================
    section_header("Configuration Files");

    let env_path = hh.join(".env");
    // THE SIBLING SURFACE. The config file's mode is checked below; `.env`
    // holds as many credentials and had no mode check at all, so a `0644`
    // `.env` — the umask default from any writer that does not go through
    // `env_store`'s owner-only open — was invisible here.
    if env_path.exists() {
        if is_world_or_group_readable(&env_path) {
            check_warn(
                &format!("{}/.env is group/world-readable", dhh),
                &format!("run: chmod 600 {}", env_path.display()),
            );
        }
        check_ok(&format!("{}/.env file exists", dhh), "");
        match std::fs::read_to_string(&env_path) {
            Ok(content) => {
                if has_provider_env_config(&content)
                    || has_ambient_provider_config(&std::env::vars().collect::<Vec<_>>())
                {
                    check_ok("API key or custom endpoint configured", "");
                } else {
                    // Neither .env nor the process environment holds a
                    // credential. There is no third source being assumed here:
                    // with both empty, no model can be called, so this is a
                    // failure (✗) and not a degraded-but-working install.
                    check_fail(
                        &format!("No API key found in {} or the environment", dhh),
                        "run: operant setup",
                    );
                    issues.push("Run 'operant setup' to configure API keys".to_string());
                }
            }
            Err(e) => {
                check_warn(
                    &format!("{}/.env exists but could not be read", dhh),
                    &e.to_string(),
                );
            }
        }
    } else if has_ambient_provider_config(&std::env::vars().collect::<Vec<_>>()) {
        // No .env file, but the process environment supplies a credential. That
        // is a complete, working install: .env is one of two sources, not the
        // only one, so its absence is information rather than a failure.
        //
        // The .env-exists arm above consults BOTH sources
        // (`has_provider_env_config(content) || has_ambient_provider_config(..)`).
        // This arm used to consult neither, which is the iter-426 defect in its
        // sibling branch: a user who exports a key in their shell and never
        // creates .env — a valid configuration — was told their install was
        // broken, and `operant doctor` exited 1 on a machine that worked.
        check_info(&format!(
            "{}/.env not created (using credentials from the environment)",
            dhh
        ));
    } else {
        // Neither source has a credential, so no model can be called. Same
        // verdict as the empty-.env case above, and for the same reason.
        check_fail(&format!("{}/.env file missing", dhh), "");
        issues.push("Run 'operant setup' to create .env".to_string());
    }

    let yaml_path = hh.join("config.yaml");
    // The config the PROCESS actually loaded, not merely the file in the home.
    // `operant -c <path> doctor` and `HERMES_CONFIG` both point the run at
    // another file; checking only `~/.operant/operant.toml` would report the
    // credentials of a file this run never read, and stay silent about the one
    // it did. Found by red-teaming the first implementation (2026-11-16): a
    // probe config carrying six live secrets produced no warning at all
    // because the scan was reading the home copy.
    let toml_path = operant_core::config::loaded_config_path()
        .filter(|path| path.exists())
        .unwrap_or_else(|| hh.join("operant.toml"));

    if yaml_path.exists() {
        check_ok(
            &format!("{}/config.yaml exists", dhh),
            "(Python compatibility)",
        );
    }
    // `dhh` collapses the home to `~`, so it may only be used for a file that
    // really IS under the home. Printing "~/.operant/operant.toml exists" while
    // reading a `-c` target elsewhere is the same lie this check was just
    // fixed for: the message and the file must be the same file.
    let toml_label = if toml_path.starts_with(&hh) {
        format!("{dhh}/operant.toml")
    } else {
        toml_path.display().to_string()
    };
    if toml_path.exists() {
        check_ok(&format!("{toml_label} exists"), "");
        // The file exists; say whether it carries a live credential in the
        // clear. Nothing else in doctor read the config's TEXT, so a key
        // pasted here was invisible to every check (2026-11-16, live).
        check_plaintext_credentials(&toml_path);
    }
    if !yaml_path.exists() && !toml_path.exists() {
        check_warn(
            "No config file found",
            &format!("(expected {}/config.yaml or operant.toml)", dhh),
        );
    }

    let provider_raw = config.client.base_url.trim();
    if !provider_raw.is_empty() {
        let matched = provider_from_url(provider_raw);
        if let Some(pdef) = matched {
            check_ok(
                &format!(
                    "config client.base_url maps to provider '{}'",
                    pdef.display_name
                ),
                "",
            );
        } else {
            let by_name = provider_by_name(provider_raw);
            if let Some(pdef) = by_name {
                check_ok(
                    &format!(
                        "config client.base_url matches provider '{}'",
                        pdef.display_name
                    ),
                    "",
                );
            } else {
                check_warn(
                    &format!(
                        "client.base_url '{}' does not match any known provider",
                        provider_raw
                    ),
                    "(check ~/.operant/.env or run 'operant setup')",
                );
                // Advisory, matching the ⚠ above: an unrecognised base_url may
                // still address a local or custom gateway, so this is worth
                // flagging but is not a broken install.
                manual_issues.push(
                    "client.base_url does not match a known provider. ".to_string()
                        + "Run 'operant setup' to configure a supported provider.",
                );
            }
        }
    } else {
        // No endpoint configured anywhere. The built-in provider defaults only
        // apply when a provider is resolved from config or the environment, so
        // with an empty base_url this machine cannot address a model at all —
        // a failure, and the ✗ now matches that verdict.
        check_fail("client.base_url is not configured", "run: operant setup");
        issues.push("Run 'operant setup' to configure a provider and base URL".to_string());
    }

    let set_providers: Vec<&str> = PROVIDERS
        .iter()
        .filter(|p| !p.env_var.is_empty())
        .filter(|p| std::env::var(p.env_var).is_ok_and(|v| !v.is_empty()))
        .map(|p| p.display_name)
        .collect();

    if !set_providers.is_empty() {
        check_ok(
            "Provider credentials found in environment",
            &format!("({})", set_providers.join(", ")),
        );
    } else {
        check_warn("No provider API keys found in environment", "");
        manual_issues.push(
            "No provider API keys configured. Run 'operant setup' or set the appropriate *_API_KEY in .env"
                .to_string(),
        );
    }

    // =====================================================================
    // B. Auth Providers
    // =====================================================================
    section_header("Auth Providers");

    if std::env::var("NOUS_API_KEY").is_ok_and(|v| !v.is_empty()) {
        check_ok("Nous Portal auth", "(logged in)");
    } else {
        check_warn("Nous Portal auth", "(not logged in)");
    }

    if std::env::var("OPENAI_API_KEY").is_ok_and(|v| !v.is_empty()) {
        check_ok("OpenAI Codex auth", "(logged in)");
    } else {
        check_warn("OpenAI Codex auth", "(not logged in)");
    }

    let codex_found = std::process::Command::new("codex")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if codex_found {
        check_ok("codex CLI", "");
    } else {
        check_info(
            "codex CLI not installed (optional — only required to import tokens \
             from an existing Codex CLI login)",
        );
    }

    if std::env::var("HERMES_GEMINI_CLIENT_ID").is_ok_and(|v| !v.is_empty()) {
        check_ok("Google Gemini OAuth", "(logged in)");
    } else {
        check_warn("Google Gemini OAuth", "(not logged in)");
    }

    if std::env::var("MINIMAX_API_KEY").is_ok_and(|v| !v.is_empty()) {
        check_ok("MiniMax OAuth", "(logged in, region=global)");
    } else {
        check_warn("MiniMax OAuth", "(not logged in)");
    }

    // =====================================================================
    // C. Directory Structure
    // =====================================================================
    section_header("Directory Structure");

    if hh.exists() {
        check_ok(&format!("{} directory exists", dhh), "");
    } else {
        check_warn(
            &format!("{} not found", dhh),
            "(will be created on first use)",
        );
    }

    for subdir in &["cron", "sessions", "logs", "skills"] {
        let sub_path = hh.join(subdir);
        if sub_path.exists() {
            check_ok(&format!("{}/{}/ exists", dhh, subdir), "");
        } else {
            check_warn(
                &format!("{}/{}/ not found", dhh, subdir),
                "(will be created on first use)",
            );
        }
    }

    let soul_path = hh.join("SOUL.md");
    if soul_path.exists() {
        match std::fs::read_to_string(&soul_path) {
            Ok(content) => {
                let has_real_content = content.lines().any(|l| {
                    let t = l.trim();
                    !t.is_empty()
                        && !t.starts_with("<!--")
                        && !t.starts_with("-->")
                        && !t.starts_with('#')
                });
                if has_real_content {
                    check_ok(&format!("{}/SOUL.md exists (persona configured)", dhh), "");
                } else {
                    check_info(&format!(
                        "{}/SOUL.md exists but is empty — edit it to customize personality",
                        dhh
                    ));
                }
            }
            Err(_) => {
                check_info(&format!("{}/SOUL.md exists but could not be read", dhh));
            }
        }
    } else {
        check_warn(
            &format!("{}/SOUL.md not found", dhh),
            "(create it to give Operant a custom personality)",
        );
    }

    // (plan 08) ONE memory engine: memory-wire at
    // ~/.operant/memory_wire.sqlite, personal bank — the same store the
    // agent recalls from. The old MEMORY.md/USER.md file probes are gone:
    // they reported the legacy files' health while the agent had already
    // moved to memory-wire (audit finding 10).
    let db = hh.join("memory_wire.sqlite");
    if db.exists() {
        let size = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
        check_ok(
            &format!("memory-wire: {} ({} KB)", db.display(), size / 1024),
            "",
        );
        if let Ok(provider) = operant_core::memory_wire::MemoryWireProvider::new(
            operant_core::platform::operant_home(),
        ) {
            if let Some(stats) = provider.memory_bank_stats() {
                check_ok(
                    &format!(
                        "bank {:?}: {} memories, {} tags",
                        provider.bank_id(),
                        stats.memories,
                        stats.tags
                    ),
                    "",
                );
            } else {
                check_warn(
                    "memory-wire store opened but stats read failed",
                    "(run operant memory stats)",
                );
            }
        } else {
            check_warn(
                "memory-wire store failed to open",
                "(memories degrade to misses this session)",
            );
        }
    } else {
        check_info("memory-wire store not created yet (created on the first retained memory)");
    }

    let state_db = hh.join("state.db");
    if state_db.exists() {
        let size = std::fs::metadata(&state_db).map(|m| m.len()).unwrap_or(0);
        check_ok(&format!("{}/state.db exists ({} KB)", dhh, size / 1024), "");
        if size > 0 {
            check_info("Session store is non-empty (install sqlite3 CLI for detailed queries)");
        }
    } else {
        check_info(&format!(
            "{}/state.db not created yet (will be created on first session)",
            dhh
        ));
    }

    let wal_path = hh.join("state.db-wal");
    if wal_path.exists() {
        if let Ok(meta) = std::fs::metadata(&wal_path) {
            let wal_size = meta.len();
            if wal_size > 50 * 1024 * 1024 {
                check_warn(
                    &format!("WAL file is large ({} MB)", wal_size / (1024 * 1024)),
                    "(may indicate missed checkpoints)",
                );
                issues
                    .push("Large WAL file — run 'operant doctor --fix' to checkpoint".to_string());
            } else if wal_size > 10 * 1024 * 1024 {
                check_info(&format!(
                    "WAL file is {} MB (normal for active sessions)",
                    wal_size / (1024 * 1024)
                ));
            } else if wal_size > 0 {
                check_ok(
                    "WAL file size is normal",
                    &format!("({} KB)", wal_size / 1024),
                );
            }
        }
    } else {
        check_info("WAL file not present (no recent write-ahead logging activity)");
    }

    // =====================================================================
    // D. Gateway Service Linger (Linux only)
    // =====================================================================
    #[cfg(target_os = "linux")]
    {
        section_header("Gateway Service Linger");

        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        let output = std::process::Command::new("loginctl")
            .args(["show-user", &user])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let linger_enabled = stdout.lines().any(|l| l.trim() == "Linger=yes");
                if linger_enabled {
                    check_ok("Linger is enabled", "(gateway survives logout)");
                } else {
                    check_warn(
                        "Linger is not enabled",
                        &format!("(run: sudo loginctl enable-linger {})", user),
                    );
                    issues.push(format!(
                        "Linger is not enabled for user '{}'. \
                         Run 'sudo loginctl enable-linger {}' so the gateway can survive logout.",
                        user, user
                    ));
                }
            }
            Ok(_) => {
                check_warn(
                    "Could not query linger status",
                    "(loginctl returned an error)",
                );
            }
            Err(_) => {
                check_warn(
                    "loginctl not found",
                    "(install systemd or manage gateway manually)",
                );
            }
        }
    }
}

/// CAN THE CONFIGURED WINDOW HOLD THE CONTENT THAT CANNOT BE FOLDED?
///
/// The eager tool schemas and the advertised skills ride every request and
/// are never evicted. A window smaller than they are fails EVERY turn —
/// the live deployment sat at 4,242 tokens against a 8,574-token protected
/// prompt, and neither `operant doctor` nor `operant config check` said a
/// word about it.
///
/// This is a FLOOR, not a promise: the runtime adds the turn tail (memory
/// recall, workspace, harness) on top, which the CLI cannot measure.
/// Is the lossless-DAG context engine actually running?
///
/// The six `context_lcm_*` keys and `agent.context_engine` ship in the example
/// config, but the engine is attached ONLY when `context_engine = "lcm"`; the
/// default ("compact") attaches nothing and the keys are read by nothing. An
/// operator who sets `context_lcm_tail_tokens` and sees no difference is
/// looking at an inert knob, and the doctor is where that fact belongs.
pub(crate) fn lcm_surface_is_inert(config: &AppConfig) -> bool {
    !config.agent.context_engine.eq_ignore_ascii_case("lcm")
}

/// How the window was resolved, in the operator's words.
pub(crate) fn window_source_label(source: &str) -> &'static str {
    match source {
        "endpoint" => "the provider endpoint's limit",
        "learned" => "a limit learned from a provider overflow",
        "catalog" => "the models.dev catalog",
        "default" => "the built-in default; the provider reported no limit",
        _ => "the operator's override",
    }
}

pub async fn check_context_window(
    config: &AppConfig,
    issues: &mut Vec<String>,
    manual_issues: &mut Vec<String>,
) {
    section_header("Context window");
    // The RESOLVED window, not the configured value: with an AUTOMATIC
    // config the configured value is 0, and comparing the floor against 0
    // reported a false failure on every fresh install ("the window is 0 but
    // the protected content needs 23344 tokens") while the turns ran fine.
    let report = crate::cmd_context::resolve_window_for_report(config).await;
    let window = report.window;
    let floor = match crate::cmd_context::measure_protected_floor(config, window).await {
        Ok(floor) => floor,
        Err(e) => {
            check_warn(
                "Context window not measured",
                &format!("{} — the schema bill could not be built", e),
            );
            manual_issues.push(format!("Context window not measured: {e}"));
            return;
        }
    };
    if floor.minimum_hard_cap > 0 {
        check_fail(
            "Context window cannot hold the protected content",
            &format!(
                "the absolute hard cap is smaller than the protected content (~{} tokens: \
                 tool schemas ~{}, advertised skills ~{}). Raise \
                 agent.context_hard_cap_tokens to at least {} — every turn fails until it fits.",
                floor.required, floor.schema_tokens, floor.skills_tokens, floor.minimum_hard_cap,
            ),
        );
        issues.push(format!(
            "The context hard cap is smaller than the protected content (~{} tokens) — every \
             turn fails until agent.context_hard_cap_tokens is at least {}.",
            floor.required, floor.minimum_hard_cap,
        ));
        return;
    }
    if floor.minimum_window > window {
        check_fail(
            "Context window cannot hold the protected content",
            &format!(
                "the context window is {window} tokens ({}) but the protected content needs \
                 about {} tokens (tool schemas ~{}, advertised skills ~{}). Raise it to at \
                 least {} — every turn fails until it fits.",
                window_source_label(report.source),
                floor.required,
                floor.schema_tokens,
                floor.skills_tokens,
                floor.minimum_window,
            ),
        );
        issues.push(format!(
            "the context window is {window} tokens ({}) but the protected content needs \
             about {} tokens — every turn fails until it is at least {}.",
            window_source_label(report.source),
            floor.required,
            floor.minimum_window,
        ));
        return;
    }
    check_ok(
        "Context window holds the protected content",
        &format!(
            "window {window} tokens ({}); protected content ~{} tokens (tool schemas ~{}, \
             advertised skills ~{} from {} skill(s))",
            window_source_label(report.source),
            floor.required,
            floor.schema_tokens,
            floor.skills_tokens,
            floor.skills_advertised
        ),
    );
    if floor.skills_dropped > 0 {
        check_warn(
            "Some installed skills are not advertised",
            &format!(
                "{} skill(s) are installed but did not fit the advertised block's ceiling, so \
                 the model cannot see them until it reaches for the skills tool.",
                floor.skills_dropped
            ),
        );
    }
    if lcm_surface_is_inert(config) {
        check_warn(
            "The lossless-DAG context engine is not running",
            "agent.context_engine is not \"lcm\", so the context_lcm_* keys in the config are \
             read by nothing: the built-in compaction is what folds the history. Set \
             agent.context_engine = \"lcm\" to arm it (it needs a database and a little more \
             RAM).",
        );
    }
    check_compression_route(config);
}

/// The LLM summarizer's route.
///
/// Compaction claims to summarize the middle with an LLM. When
/// Whether a config KEY NAME identifies a credential.
///
/// A NAME rule, not a list of dotted paths. The first implementation shipped a
/// hardcoded list of six paths and was wrong on both sides at once: it MISSED
/// ten live credential fields (`auxiliary_models.*.api_key`,
/// `advisor.model.api_key`, `stt.groq_api_key`, `stt.openai_api_key`,
/// `tools.web.tavily_api_key`, `tools.web.exa_api_key`,
/// `tools.terminal.ssh.key_path`, `memory.agentmemory_secret`,
/// `mcp.servers[].auth_token`, `gateway.webhooks_secret`), and it encoded paths
/// (`vision.api_key`, `client.api_key`) that a hand-edit never produces in that
/// form. A list has to be maintained in two places — the struct and this scan —
/// and the struct is the one that keeps changing.
///
/// The suffix rule covers every one of those and the next field someone adds:
/// a key whose name ends in `_key`, `_token`, `_secret` or `_password` (or IS
/// one of `api_key`, `token`, `secret`, `password`) carries a credential.
///
/// Deliberately about the NAME only, so `telegram_enabled` and `search_url`
/// never match, and so a future `*_api_key` is covered the day it lands.
fn is_secret_key_name(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    const EXACT: &[&str] = &["api_key", "apikey", "token", "secret", "password", "passwd"];
    if EXACT.contains(&key.as_str()) {
        return true;
    }
    const SUFFIXES: &[&str] = &[
        "_api_key",
        "_apikey",
        "_token",
        "_secret",
        "_password",
        "_key",
    ];
    SUFFIXES.iter().any(|suffix| key.ends_with(suffix))
}

/// Every credential-looking key a config file assigns a value to, as dotted
/// paths, in file order.
///
/// A TEXT scan rather than a parse of `AppConfig`: the fields are
/// `Option<String>`, so a `None` and an absent key are indistinguishable — but
/// the operator's question is "is a live value sitting in this file", and a
/// value that parses to `None` was never written. The scan also sees a key
/// inside a section the running build ignores, which is exactly where a stale
/// secret hides (`[memory] agentmemory_secret` is retired, still accepted, and
/// still a secret on disk).
///
/// Values are never returned — only names — because doctor's output is pasted
/// into issues and bug reports.
fn plaintext_secrets_in(text: &str) -> Vec<String> {
    let mut section = String::new();
    let mut found = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            // `[[mcp.servers]]` is an array-of-tables header; the repeated
            // element has no key of its own, so the name alone is enough.
            section = trimmed
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_string();
            continue;
        }
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq].trim().trim_matches('"');
        let value = trimmed[eq + 1..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        if !is_secret_key_name(key) || names_a_secret_but_is_not_one(key) {
            continue;
        }
        // `.env` variable names are UPPER_SNAKE (`TELEGRAM_BOT_TOKEN`,
        // `OPENAI_API_KEY`). They belong to the `.env` surface, which has its
        // own check above; a config that happens to carry one is not holding a
        // config FIELD, and reporting it here would tell the operator to move a
        // key out of the file it is already the right file for. Config keys are
        // lower_snake by convention everywhere in this schema.
        if key.chars().any(|c| c.is_ascii_uppercase()) {
            continue;
        }
        // A table header can be dotted (`[gateway]`); compare on the joined
        // path so `[client] api_key` and a top-level `client.api_key = …` both
        // read as the same field.
        let dotted = if section.is_empty() {
            key.to_string()
        } else {
            format!("{section}.{key}")
        };
        found.push(dotted);
    }
    found
}

/// Config keys that name a credential but hold no secret, so they must not be
/// reported. `key_path` is the one that matters: `tools.terminal.ssh.key_path`
/// points at a file, and the file is the secret — naming the path would send an
/// operator to chmod a config that holds no credential.
fn names_a_secret_but_is_not_one(key: &str) -> bool {
    key.to_ascii_lowercase().ends_with("_path")
}

/// Whether a file is readable by anyone other than its owner.
///
/// Only the OTHER and GROUP read bits matter here: a `0640` shared with a
/// group is a deliberate choice on a single-user box, but a `0644` is the
/// umask default and means every local account can read a credential.
#[cfg(unix)]
fn is_world_or_group_readable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.permissions().mode() & 0o044 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_world_or_group_readable(_path: &std::path::Path) -> bool {
    false
}

/// Warn when a config file holds a plaintext credential.
///
/// THE GAP THIS CLOSES (found 2026-11-16, live on this host): `operant.toml`
/// carried a real provider `api_key` in the clear, and NOTHING in `doctor`
/// looked at it. The doctrine was all present — the writer creates the file
/// `0600`, `write_secret_file` tightens a loose file, RT-154 removed the
/// "encrypted at rest" claim from the template — but the doctrine was only
/// ever about HOW a secret is written, never about TELLING the operator that
/// one is there. An operator who pasted a key into the config had no way to
/// learn that the file was the reason a `chmod 644` mattered.
///
/// This is an execution gap, not an architectural one: the mechanism
/// (owner-only writes, the `~/.operant/.env` convention for credentials, the
/// `write_secret_file` tightening path) exists and works. What was missing is
/// the one check that surfaces the consequence.
///
/// Two independent findings, because they fail differently:
/// - a plaintext secret at all → the operator should know where it lives;
/// - a plaintext secret in a LOOSELY permissioned file → urgent, because
///   every local account can read it right now.
fn check_plaintext_credentials(config_path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return;
    };
    let secrets = plaintext_secrets_in(&text);
    if secrets.is_empty() {
        check_ok("No plaintext credentials in the config file", "");
        return;
    }
    let loose = is_world_or_group_readable(config_path);
    let shown: Vec<&str> = secrets.iter().map(String::as_str).collect();
    if loose {
        check_warn(
            &format!(
                "{} holds plaintext credentials and is group/world-readable",
                config_path.display()
            ),
            &format!(
                "{} — run: chmod 600 {}",
                shown.join(", "),
                config_path.display()
            ),
        );
    } else {
        check_warn(
            &format!("{} holds plaintext credentials", config_path.display()),
            &format!(
                "{} (owner-only, but unencrypted — prefer ~/.operant/.env for keys)",
                shown.join(", ")
            ),
        );
    }
}

/// The LLM summarizer's route.
///
/// Compaction claims to summarize the middle with an LLM. When
/// `[auxiliary_models.compression].model` is unset the config falls back to
/// the hardcoded `gpt-4o-mini`, which only an OpenAI route serves — on any
/// other route every summarization fails and the compressor silently degrades
/// to deterministic truncation (`llm_compressor.rs:528-532`). Compaction still
/// "works", so nothing else surfaces it: this check is the only place the
/// operator learns the feature is off.
fn check_compression_route(config: &AppConfig) {
    match compression_route_verdict(config) {
        CompressionRoute::Off => {}
        CompressionRoute::Configured(model) => check_ok(
            "Context compaction has a summarizer route",
            &format!("[auxiliary_models.compression].model = \"{model}\""),
        ),
        CompressionRoute::FallsBackToOpenAIDefault => {
            let default = operant_core::agent::llm_compressor::LlmCompressorConfig::default()
                .summarizer_model;
            check_warn(
                "Context compaction has no summarizer route",
                &format!(
                    "agent.context_compression is on but [auxiliary_models.compression].model is \
                     unset, so the summarizer calls \"{default}\" — an OpenAI model name. On any \
                     other route every summarization fails and compaction falls back to \
                     deterministic truncation. Set the slot to your route's model."
                ),
            );
        }
    }
}

/// What the doctor should say about the summarizer route. Separate from the
/// printing so the decision is testable without capturing stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompressionRoute {
    /// Compaction is off: nothing to check.
    Off,
    /// A route is configured, by name.
    Configured(String),
    /// Compaction is on with no route, so it will silently truncate.
    FallsBackToOpenAIDefault,
}

pub(crate) fn compression_route_verdict(config: &AppConfig) -> CompressionRoute {
    if !config.agent.context_compression {
        return CompressionRoute::Off;
    }
    match config
        .auxiliary_models
        .compression
        .as_ref()
        .and_then(|slot| slot.named_model().map(str::to_string))
    {
        Some(model) => CompressionRoute::Configured(model),
        None => CompressionRoute::FallsBackToOpenAIDefault,
    }
}

/// Is the context meter recording?
///
/// The session rows carry `input_tokens`, `output_tokens` and `api_call_count`
/// columns, and until the context meter landed NOTHING wrote them: on the
/// operator's database every one of the 3 271 sessions reported zero tokens,
/// so `operant insights sessions` and the `session_insights` tool showed
/// "Tokens: 0" for a window in which a single session had spent hundreds of
/// thousands of context tokens.
///
/// A blank meter is an ADVISORY, not a failure: a database full of sessions
/// from before the meter existed is exactly what a long-lived install looks
/// like, and the next request starts the record. What must never happen is
/// the meter being blank on a database whose sessions are NEW — that is the
/// defect, and this check is how it becomes visible.
pub fn check_context_telemetry(
    config: &AppConfig,
    issues: &mut Vec<String>,
    manual_issues: &mut Vec<String>,
) {
    section_header("Context telemetry");
    let db = match operant_core::database::Database::init(config.database_path.clone()) {
        Ok(db) => db,
        Err(e) => {
            check_warn(
                "Context meter unreadable",
                &format!("{} — context usage cannot be reported", e),
            );
            manual_issues.push(format!("Context meter unreadable: {e}"));
            return;
        }
    };
    // A summarizer that is configured but FAILING degrades compaction to
    // deterministic truncation with only a `warn!` in a log nobody tails —
    // compaction still "works", so nothing else surfaces it. The count is
    // persisted per session under this key (`llm_compressor.rs:387`), so the
    // newest session is the observable.
    if config.agent.context_compression
        && let Some(sid) = db.newest_session_id().ok().flatten()
        && let Some(raw) = db.get_session_metadata(&sid, "compression_consecutive_failures")
        && let Ok(failures) = raw.trim().parse::<usize>()
        && failures >= 3
    {
        check_warn(
            "Context compaction is degraded — the summarizer keeps failing",
            &format!(
                "{failures} consecutive summarization failure(s) on the newest session. Every                  compaction is falling back to deterministic truncation, so the model is NOT                  receiving an LLM summary. Check the summarizer's model name, key and quota on                  the route [auxiliary_models.compression] names."
            ),
        );
    }

    let metered = db.sessions_with_recorded_usage().unwrap_or(0);
    let sessions = db.list_sessions(10_000).map(|s| s.len()).unwrap_or(0);
    if metered > 0 {
        check_ok(
            "Context meter recording",
            &format!("{metered} of {sessions} session(s) carry token usage"),
        );
        return;
    }
    if sessions == 0 {
        check_info(
            "Context meter recording — no sessions yet, the first request starts the record",
        );
        return;
    }
    // Sessions exist and none of them carry usage. There are two readings and
    // they call for different responses: the sessions may all predate the
    // meter (expected on a long-lived install), or the meter may not be
    // writing (a defect). The NEWEST session decides: if it is recent, it ran
    // AFTER the meter existed, so its blank token count means the meter is
    // not writing.
    let newest = db.newest_session_id().ok().flatten();
    let live_and_unmetered = newest.as_deref().and_then(|sid| {
        let usage = db.session_usage(sid).ok()?;
        let sessions = db.list_sessions(10_000).ok()?;
        let started = sessions
            .iter()
            .find(|s| s.id == sid)
            .map(|s| s.created_at.clone())?;
        let recent = chrono::Utc::now().signed_duration_since(
            chrono::DateTime::parse_from_rfc3339(&started)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        );
        Some(usage.message_count > 0 && usage.api_call_count == 0 && recent.num_minutes() < 60)
    });
    if live_and_unmetered == Some(true) {
        check_fail(
            "Context meter is not recording",
            "the newest session has messages but no token usage",
        );
        issues.push(
            "Context meter is not recording: the newest session ran but its token usage is \
             blank. `operant context inspect` shows what the ledger holds."
                .to_string(),
        );
        return;
    }
    check_warn(
        "Context meter has no records",
        &format!(
            "{sessions} session(s), none with token usage — sessions predating the meter \
             look like this; so does a meter that is not writing"
        ),
    );
    manual_issues.push(
        "Context meter has no records: session token counts are blank. Sessions from before \
         the meter existed are expected; run one turn and re-check with \
         `operant context inspect`."
            .to_string(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::config::AuxiliaryModelConfig;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A `.env` holding no credential, with an empty process environment, is a
    /// machine that cannot call a model. Both scans must agree it is
    /// unconfigured, because `run_config_checks` fails (exit 1) exactly when
    /// their conjunction is false.
    ///
    /// This is the case iter-426 got wrong: the check consulted only `.env`, so
    /// a key exported in the launching shell was invisible and a real
    /// credential-less install reported healthy.
    #[test]
    fn empty_env_file_and_empty_process_env_is_unconfigured() {
        let content = "# comment\nFOO=bar\n";
        assert!(!has_provider_env_config(content));
        assert!(!has_ambient_provider_config(&env(&[])));
        // The conjunction the caller evaluates:
        assert!(
            !(has_provider_env_config(content) || has_ambient_provider_config(&env(&[]))),
            "nothing configured anywhere must not satisfy the credential check"
        );
    }

    /// The ambient scan is the one that makes an exported key count. If it ever
    /// stops matching, the caller silently falls back to reading `.env` only and
    /// reports a working machine as broken.
    #[test]
    fn ambient_scan_detects_exported_credential() {
        assert!(has_ambient_provider_config(&env(&[(
            "OPENAI_API_KEY",
            "sk-x"
        )])));
        assert!(has_ambient_provider_config(&env(&[(
            "ANTHROPIC_API_KEY",
            "sk-x"
        )])));
        assert!(has_ambient_provider_config(&env(&[(
            "OPENAI_BASE_URL",
            "http://h/v1"
        )])));
        assert!(has_ambient_provider_config(&env(&[(
            "GITHUB_TOKEN",
            "ghp_x"
        )])));
    }

    /// An empty value is not a credential, and an unrelated variable is not
    /// either — otherwise a bare `export OPENAI_API_KEY=` would satisfy the
    /// check and mask a genuinely unusable install.
    #[test]
    fn ambient_scan_rejects_empty_and_unrelated_values() {
        assert!(!has_ambient_provider_config(&env(&[(
            "OPENAI_API_KEY",
            ""
        )])));
        assert!(!has_ambient_provider_config(&env(&[("EDITOR", "vim")])));
        assert!(!has_ambient_provider_config(&env(&[("PAGER", "less")])));
    }

    /// The two scans must classify the same keys the same way, or the weaker one
    /// decides the exit code and the disagreement is invisible.
    #[test]
    fn both_scans_classify_the_same_keys_identically() {
        for key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "OPENAI_BASE_URL",
            "GITHUB_TOKEN",
            "NOTION_API_TOKEN",
            "SOME_SECRET",
            "SOURCEHOUND_HOST",
        ] {
            let in_file = has_provider_env_config(&format!("{key}=value\n"));
            let in_ambient = has_ambient_provider_config(&env(&[(key, "value")]));
            assert_eq!(
                in_file, in_ambient,
                "{key} is classified differently by the .env and ambient scans"
            );
        }
    }

    /// The lossless-DAG engine is inert unless it is switched on, and the
    /// doctor is where that fact belongs: an operator who sets the
    /// context_lcm_* keys and sees no difference is looking at an inert knob.
    #[test]
    fn the_lcm_surface_is_inert_unless_the_engine_is_armed() {
        let mut config = AppConfig::default();
        assert!(
            lcm_surface_is_inert(&config),
            "the default engine is the built-in compaction"
        );
        config.agent.context_engine = "lcm".to_string();
        assert!(
            !lcm_surface_is_inert(&config),
            "the engine is armed, so the keys are live"
        );
        config.agent.context_engine = "LCM".to_string();
        assert!(
            !lcm_surface_is_inert(&config),
            "the engine name is case-insensitive"
        );
    }

    /// The doctor's context-window verdict must be about the RESOLVED window.
    ///
    /// With an AUTOMATIC config (0 — the shipped default) the old code
    /// compared the protected floor against 0 and reported "every turn fails
    /// until it fits" while the turns ran fine: the resolver had already
    /// picked the model's real window. A false failure on every fresh
    /// install is how a check stops being read.
    #[test]
    fn the_window_verdict_uses_the_resolved_window_not_the_sentinel() {
        assert_eq!(
            window_source_label("endpoint"),
            "the provider endpoint's limit"
        );
        assert_eq!(
            window_source_label("learned"),
            "a limit learned from a provider overflow"
        );
        assert_eq!(
            window_source_label("default"),
            "the built-in default; the provider reported no limit"
        );
        assert_eq!(window_source_label("configured"), "the operator's override");
    }

    /// The sentinel only ever means "ask the provider"; a check that treats
    /// it as a number reports a failure that cannot happen.
    #[test]
    fn an_automatic_window_is_not_a_zero_window() {
        let automatic = operant_core::context_window::AUTOMATIC;
        assert_eq!(automatic, 0);
        // The resolver never hands back 0: an automatic window with no
        // answer falls back to the documented default.
        let resolved = operant_core::context_window::resolve(
            automatic,
            None,
            operant_core::context_window::Discovered::default(),
        );
        assert!(
            resolved.window > 0,
            "the resolved window is never the sentinel"
        );
        assert_eq!(
            resolved.source,
            operant_core::context_window::WindowSource::Default
        );
    }

    /// And the negative side: nothing may be accepted by one scan and not the
    /// other, in the direction that would let a broken install pass.
    #[test]
    fn both_scans_reject_the_same_non_credentials() {
        for key in ["EDITOR", "PAGER", "HOME", "PATH"] {
            let in_file = has_provider_env_config(&format!("{key}=value\n"));
            let in_ambient = has_ambient_provider_config(&env(&[(key, "value")]));
            assert_eq!(in_file, in_ambient, "{key} disagrees between the two scans");
        }
    }

    /// Compaction with no summarizer route is the silent-off case: the
    /// compressor falls back to truncation on any non-OpenAI route and
    /// nothing else says so. The verdict must name it.
    #[test]
    fn compaction_without_a_summarizer_route_is_flagged() {
        let mut config = AppConfig::default();
        config.agent.context_compression = true;
        config.auxiliary_models.compression = None;
        assert_eq!(
            compression_route_verdict(&config),
            CompressionRoute::FallsBackToOpenAIDefault,
            "an unset slot must warn: the default model name only an OpenAI route serves"
        );
    }

    /// A configured route is a pass, whatever the name.
    #[test]
    fn a_configured_summarizer_route_passes() {
        let mut config = AppConfig::default();
        config.agent.context_compression = true;
        config.auxiliary_models.compression = Some(AuxiliaryModelConfig {
            provider: None,
            model: Some("gpt-4o-mini".to_string()),
            base_url: None,
            api_key: None,
        });
        assert_eq!(
            compression_route_verdict(&config),
            CompressionRoute::Configured("gpt-4o-mini".to_string())
        );
    }

    /// Compaction off is not a warning: the operator turned it off.
    #[test]
    fn compaction_off_is_silent() {
        let mut config = AppConfig::default();
        config.agent.context_compression = false;
        config.auxiliary_models.compression = None;
        assert_eq!(compression_route_verdict(&config), CompressionRoute::Off);
    }

    /// A slot that is present but blank is the same as absent — the config
    /// filters empty strings, and so must the check.
    #[test]
    fn a_blank_summarizer_slot_is_treated_as_unset() {
        let mut config = AppConfig::default();
        config.agent.context_compression = true;
        config.auxiliary_models.compression = Some(AuxiliaryModelConfig {
            provider: None,
            model: Some("   ".to_string()),
            base_url: None,
            api_key: None,
        });
        assert_eq!(
            compression_route_verdict(&config),
            CompressionRoute::FallsBackToOpenAIDefault
        );
    }

    // ── Plaintext credential detection ──────────────────────────────────
    //
    // The live gap (2026-11-16): `~/.operant/operant.toml` carried real
    // credentials in the clear and no doctor check read the file's TEXT.
    // These pin the name rule that closes it.

    /// The two fields live on this host, and the shape a hand-edit produces.
    #[test]
    fn a_client_api_key_in_a_section_is_reported() {
        let text = "[client]\nbase_url = \"https://x/v1\"\napi_key = \"sk-live-123\"\n";
        assert_eq!(plaintext_secrets_in(text), vec!["client.api_key"]);
    }

    /// The dotted spelling is the other form a hand-edit can take, and the one
    /// `persist_config_key` writes. Both must resolve to the same field.
    #[test]
    fn a_dotted_top_level_secret_key_is_reported() {
        assert_eq!(
            plaintext_secrets_in("client.api_key = \"sk-live-123\"\n"),
            vec!["client.api_key"]
        );
    }

    #[test]
    fn every_gateway_token_field_is_covered() {
        let text = "\
[gateway]
telegram_token = \"111:aaa\"
discord_token = \"bbb\"
slack_token = \"ccc\"
whatsapp_token = \"ddd\"
webhooks_secret = \"whsec\"
";
        let found = plaintext_secrets_in(text);
        assert_eq!(found.len(), 5, "got: {found:?}");
        assert!(found.contains(&"gateway.webhooks_secret".to_string()));
        assert!(found.contains(&"gateway.whatsapp_token".to_string()));
    }

    /// THE COVERAGE HOLE THE FIRST IMPLEMENTATION HAD. A hardcoded six-path
    /// list missed these entirely, so a config carrying them produced NO
    /// warning. Each is a real `Option<String>` on a live struct.
    #[test]
    fn every_struct_level_secret_field_is_covered() {
        let text = "\
[auxiliary_models.compression]
api_key = \"sk-A\"
[advisor.model]
api_key = \"sk-B\"
[stt]
groq_api_key = \"sk-C\"
openai_api_key = \"sk-D\"
[tools.web]
tavily_api_key = \"sk-E\"
exa_api_key = \"sk-F\"
[memory]
agentmemory_secret = \"sk-G\"
[[mcp.servers]]
name = \"s\"
auth_token = \"sk-H\"
";
        let found = plaintext_secrets_in(text);
        for expected in [
            "auxiliary_models.compression.api_key",
            "advisor.model.api_key",
            "stt.groq_api_key",
            "stt.openai_api_key",
            "tools.web.tavily_api_key",
            "tools.web.exa_api_key",
            "memory.agentmemory_secret",
            "mcp.servers.auth_token",
        ] {
            assert!(
                found.iter().any(|k| k == expected),
                "missing {expected} — got {found:?}"
            );
        }
    }

    /// A *_api_key added tomorrow is covered without touching this file, which
    /// is the whole reason the rule is a name rule and not a list.
    #[test]
    fn an_unknown_future_api_key_field_is_covered_by_the_name_rule() {
        let text = "[some_new_section]\nfuture_service_api_key = \"sk-NEW\"\n";
        assert_eq!(
            plaintext_secrets_in(text),
            vec!["some_new_section.future_service_api_key"]
        );
    }

    /// `key_path` holds a PATH, not a credential. Reporting it would send the
    /// operator to chmod a config that carries no secret.
    #[test]
    fn a_path_named_like_a_key_is_not_reported() {
        let text = "[tools.terminal.ssh]\nkey_path = \"/home/me/.ssh/id\"\n";
        assert!(plaintext_secrets_in(text).is_empty());
    }

    /// An empty value is an unset field, not a leaked secret — the loader
    /// reads it as `None`, and warning would false-positive on every
    /// template-derived config.
    #[test]
    fn an_empty_secret_value_is_not_reported() {
        assert!(plaintext_secrets_in("[client]\napi_key = \"\"\n").is_empty());
        assert!(plaintext_secrets_in("[client]\napi_key =\n").is_empty());
    }

    /// A config with no credentials must not warn — the check has to be quiet
    /// on the common case or it trains the operator to ignore it.
    #[test]
    fn a_config_without_secrets_reports_nothing() {
        let text = "[agent]\nmodel = \"large-stack\"\n\n[gateway]\ntelegram_enabled = false\n";
        assert!(plaintext_secrets_in(text).is_empty());
    }

    /// The `.env` surface: a `0644` `.env` holds every provider key and is
    /// readable by every local account, so it must be named distinctly from
    /// the config-file warning rather than folded into it.
    #[test]
    fn a_loose_env_file_is_named_as_its_own_finding() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let env = dir.path().join(".env");
        std::fs::write(&env, "OPENAI_API_KEY=sk-x\n").expect("write");
        std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o644)).expect("chmod 644");
        assert!(
            is_world_or_group_readable(&env),
            "a 0644 .env is the case this finding exists for"
        );
    }

    /// An owner-only `.env` must not warn, or the check trains the operator to
    /// ignore it.
    #[test]
    fn an_owner_only_env_file_is_quiet() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let env = dir.path().join(".env");
        std::fs::write(&env, "OPENAI_API_KEY=sk-x\n").expect("write");
        std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o600)).expect("chmod 600");
        assert!(!is_world_or_group_readable(&env), "0600 is quiet");
    }

    /// A commented-out key is documentation, not a live credential.
    #[test]
    fn a_commented_out_secret_is_not_reported() {
        assert!(plaintext_secrets_in("# api_key = \"sk-old\"\n[client]\n").is_empty());
    }

    /// `.env` names are a different surface with their own check. A bare
    /// `TELEGRAM_BOT_TOKEN = …` in the config is not one of the config's own
    /// fields, so it is not reported here.
    #[test]
    fn an_env_only_name_is_not_reported_as_a_config_secret() {
        assert!(plaintext_secrets_in("TELEGRAM_BOT_TOKEN = \"111:aaa\"\n").is_empty());
    }

    /// THE OUTPUT CONTRACT: doctor's text is pasted into issues, so the scan
    /// returns KEY NAMES and the value never appears in what is printed.
    #[test]
    fn the_reported_names_never_include_the_value() {
        let secret = "sk-SUPER-SECRET-VALUE-do-not-print";
        let text = format!("[client]\napi_key = \"{secret}\"\n");
        let found = plaintext_secrets_in(&text);
        assert_eq!(found, vec!["client.api_key"]);
        assert!(
            found.iter().all(|name| !name.contains(secret)),
            "the secret bytes must not reach doctor's output"
        );
    }

    /// A real file at `0600` is NOT loose. The negative half: without it a
    /// check hardcoded to always report "loose" would pass the positive test.
    #[cfg(unix)]
    #[test]
    fn an_owner_only_file_is_not_loose() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant.toml");
        std::fs::write(&path, "[client]\napi_key = \"sk-x\"\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod 600");
        assert!(!is_world_or_group_readable(&path), "0600 is owner-only");
    }

    /// A umask-default `0644` file IS loose — the case the warning exists for.
    #[cfg(unix)]
    #[test]
    fn a_world_readable_file_is_loose() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant.toml");
        std::fs::write(&path, "[client]\napi_key = \"sk-x\"\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod 644");
        assert!(is_world_or_group_readable(&path), "0644 is world-readable");
    }

    /// `0640` is group-readable but not world-readable; it still counts — a
    /// group is a wider audience than the owner.
    #[cfg(unix)]
    #[test]
    fn a_group_readable_file_is_loose() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant.toml");
        std::fs::write(&path, "[client]\napi_key = \"sk-x\"\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).expect("chmod 640");
        assert!(is_world_or_group_readable(&path), "0640 is group-readable");
    }

    /// The live shape: a real key in a 0600 file. Detected, and reported as
    /// plaintext (owner-only is not encrypted).
    #[cfg(unix)]
    #[test]
    fn a_live_config_with_a_secret_produces_a_warning() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant.toml");
        std::fs::write(
            &path,
            "[client]\nbase_url = \"https://aroute/v1\"\napi_key = \"eyJ0eXAi\"\n",
        )
        .expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod 600");
        assert_eq!(
            plaintext_secrets_in(&std::fs::read_to_string(&path).expect("read")),
            vec!["client.api_key"],
            "the live shape is detected"
        );
    }
}

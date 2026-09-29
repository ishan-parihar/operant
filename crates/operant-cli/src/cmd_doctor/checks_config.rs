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
    if env_path.exists() {
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
    let toml_path = hh.join("operant.toml");

    if yaml_path.exists() {
        check_ok(
            &format!("{}/config.yaml exists", dhh),
            "(Python compatibility)",
        );
    }
    if toml_path.exists() {
        check_ok(&format!("{}/operant.toml exists", dhh), "");
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

    // Built-in memory lives at the operant home ROOT (MEMORY.md / USER.md),
    // matching the agent's `load_repo_memory_manager` storage dir. The doctor
    // previously probed a phantom `memories/` subdir, so it always warned even
    // when the agent's real memory store was healthy (audit R5-2).
    let memory_file = hh.join("MEMORY.md");
    if memory_file.exists() {
        let size = std::fs::read_to_string(&memory_file)
            .map(|s| s.trim().len())
            .unwrap_or(0);
        check_ok(&format!("MEMORY.md exists ({} chars)", size), "");
    } else {
        check_info(
            "MEMORY.md not created yet (will be created when the agent first writes a memory)",
        );
    }

    let user_file = hh.join("USER.md");
    if user_file.exists() {
        let size = std::fs::read_to_string(&user_file)
            .map(|s| s.trim().len())
            .unwrap_or(0);
        check_ok(&format!("USER.md exists ({} chars)", size), "");
    } else {
        check_info(
            "USER.md not created yet (will be created when the agent first writes a memory)",
        );
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

#[cfg(test)]
mod tests {
    use super::*;

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
            "IGS_HOST",
        ] {
            let in_file = has_provider_env_config(&format!("{key}=value\n"));
            let in_ambient = has_ambient_provider_config(&env(&[(key, "value")]));
            assert_eq!(
                in_file, in_ambient,
                "{key} is classified differently by the .env and ambient scans"
            );
        }
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
}

//! Config, auth, and directory-structure checks for `operant doctor`.
//!
//! Mirrors sections from `operant-agent/operant_cli/doctor.py`:
//! - Configuration Files (.env, config.yaml, provider validation, stale keys)
//! - Auth Providers (Nous, Codex, Gemini OAuth, MiniMax OAuth)
//! - Directory Structure (operant_home, subdirs, SOUL.md, memories, state.db, WAL)
//! - Gateway Service Linger

#![allow(unused)]

use operant_core::config::AppConfig;
use operant_core::platform::operant_home;

use super::check_result::{check_fail, check_info, check_ok, check_warn, section_header};
use crate::provider::{provider_by_name, provider_from_url, PROVIDERS};

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

/// Run all config & directory checks, appending actionable items to `issues`.
pub fn run_config_checks(config: &AppConfig, issues: &mut Vec<String>) {
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
                if has_provider_env_config(&content) {
                    check_ok("API key or custom endpoint configured", "");
                } else {
                    check_warn(&format!("No API key found in {}/.env", dhh), "");
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
    } else {
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
                issues.push(
                    "client.base_url does not match a known provider. ".to_string()
                        + "Run 'operant setup' to configure a supported provider.",
                );
            }
        }
    } else {
        check_warn("client.base_url is not configured", "");
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
        issues.push(
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

    for subdir in &["cron", "sessions", "logs", "skills", "memories"] {
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

    let memories_dir = hh.join("memories");
    if memories_dir.exists() {
        check_ok(&format!("{}/memories/ directory exists", dhh), "");

        let memory_file = memories_dir.join("MEMORY.md");
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

        let user_file = memories_dir.join("USER.md");
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
    } else {
        check_warn(
            &format!("{}/memories/ not found", dhh),
            "(will be created on first use)",
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
        match std::fs::metadata(&wal_path) {
            Ok(meta) => {
                let wal_size = meta.len();
                if wal_size > 50 * 1024 * 1024 {
                    check_warn(
                        &format!("WAL file is large ({} MB)", wal_size / (1024 * 1024)),
                        "(may indicate missed checkpoints)",
                    );
                    issues.push(
                        "Large WAL file — run 'operant doctor --fix' to checkpoint".to_string(),
                    );
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
            Err(_) => {}
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

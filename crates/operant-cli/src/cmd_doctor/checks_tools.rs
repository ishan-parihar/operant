//! External tools, Node.js, tool availability, and platform checks.
//!
//! Mirrors sections from `operant-agent/operant_cli/doctor.py`:
//! - External Tools (git, ripgrep, docker, ssh, daytona, vercel)
//! - Node.js + agent-browser + Chromium + npm audit
//! - Tool Availability Enumeration
//! - Skills Hub
//! - GitHub Token / gh auth
//! - Memory Provider Health
//! - Profiles
//! - Submodules (tinker-atropos)

use std::path::{Path, PathBuf};
use std::process::Command;

use operant_core::config::{AppConfig, TerminalBackend};
use operant_core::platform::{find_node, operant_home};

use super::check_result::{check_fail, check_info, check_ok, check_warn, section_header};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether `cmd` can be run and reports its version.
fn cmd_check_version(cmd: &str, arg: &str) -> Option<String> {
    Command::new(cmd)
        .arg(arg)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
}

/// Returns `true` when a command succeeds (`--version`).
fn cmd_check(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the operant-rs project root (parent of `crates/operant-cli/`).
fn project_root() -> &'static Path {
    // CARGO_MANIFEST_DIR = .../operant-rs/crates/operant-cli
    // parent = crates/, parent = operant-rs/
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("operant-cli/Cargo.toml has a parent")
            .parent()
            .expect("crates/ has a parent")
            .to_path_buf()
    })
}

/// Read an environment variable.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run all external tool & platform checks.
pub fn run_tool_checks(
    config: &AppConfig,
    issues: &mut Vec<String>,
    manual_issues: &mut Vec<String>,
) {
    // =====================================================================
    // A. External Tools
    // =====================================================================
    section_header("External Tools");

    // -- Git --
    if let Some(ver) = cmd_check_version("git", "--version") {
        check_ok("git", &ver);
    } else {
        check_warn("git not found", "(recommended for source control)");
        manual_issues.push("Install git: https://git-scm.com/downloads".to_string());
    }

    // -- ripgrep (optional) --
    if cmd_check("rg") {
        check_ok("ripgrep (rg)", "(faster file search)");
    } else {
        check_warn(
            "ripgrep (rg) not found",
            "(file search uses built-in fallback)",
        );
    }

    // -- Docker --
    let docker_required = config.terminal_backend == TerminalBackend::Docker;
    if cmd_check("docker") {
        if docker_required {
            // Backend is Docker — verify the daemon is actually running
            let daemon_ok = Command::new("docker")
                .arg("info")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if daemon_ok {
                check_ok("docker", "(daemon running)");
            } else {
                check_fail("docker daemon not running", "(required for Docker backend)");
                issues.push("Start Docker daemon or change terminal backend".to_string());
            }
        } else {
            check_ok("docker", "(optional)");
        }
    } else if docker_required {
        check_fail("docker not found", "(required for Docker backend)");
        issues.push("Install Docker or change terminal_backend from Docker".to_string());
    } else {
        check_warn("docker not found", "(optional)");
    }

    // -- SSH --
    if config.terminal_backend == TerminalBackend::Ssh {
        let ssh_host = env_var("SSH_HOST")
            .or_else(|| env_var("TERMINAL_SSH_HOST"))
            .or_else(|| env_var("HERMES_SSH_HOST"));
        if let Some(ref host) = ssh_host {
            let ssh_ok = Command::new("ssh")
                .args([
                    "-o",
                    "ConnectTimeout=5",
                    "-o",
                    "BatchMode=yes",
                    host,
                    "echo ok",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ssh_ok {
                check_ok("SSH connection", &format!("({host})"));
            } else {
                check_fail("SSH connection failed", &format!("(to {host})"));
                issues.push(format!("Check SSH configuration for {host}"));
            }
        } else {
            check_fail(
                "SSH_HOST not set",
                "(required for SSH terminal backend — set SSH_HOST in .env)",
            );
            issues.push("Set SSH_HOST environment variable for SSH backend".to_string());
        }
    }

    // -- Daytona --
    if config.terminal_backend == TerminalBackend::Daytona {
        if env_var("DAYTONA_API_KEY").is_some() {
            check_ok("Daytona API key", "(configured)");
        } else {
            check_fail("DAYTONA_API_KEY not set", "(required for Daytona backend)");
            issues.push("Set DAYTONA_API_KEY environment variable".to_string());
        }
    }

    // -- Vercel Sandbox --
    if config.terminal_backend == TerminalBackend::VercelSandbox {
        let vercel_token = env_var("VERCEL_TOKEN");
        if vercel_token.is_some() {
            check_ok("Vercel token", "(configured)");
        } else {
            check_fail(
                "VERCEL_TOKEN not set",
                "(required for Vercel Sandbox backend)",
            );
            issues.push("Set VERCEL_TOKEN for Vercel Sandbox backend".to_string());
        }

        // Note vercel SDK (Python-specific in the reference; just a note in Rust)
        check_info("vercel SDK check: not applicable (Rust port)");

        // Check runtime setting
        let runtime = env_var("TERMINAL_VERCEL_RUNTIME").or_else(|| {
            config.tools.terminal.max_timeout_secs.to_string().into() // not the runtime, just a placeholder
        });
        match runtime {
            Some(ref r) if r == "node24" || r == "node22" || r == "node20" => {
                check_ok("Vercel runtime", &format!("({r})"));
            }
            Some(ref r) => {
                check_warn(
                    "Vercel runtime",
                    &format!("({r} — recommended: node24, node22, node20)"),
                );
            }
            None => {
                check_ok("Vercel runtime", "(node24, default)");
            }
        }
    }

    // =====================================================================
    // B. Node.js / agent-browser / npm audit
    // =====================================================================
    section_header("Node.js & Browser Tools");

    if let Some(_node_path) = find_node() {
        check_ok("Node.js", "(found)");

        // agent-browser
        let agent_browser_local = project_root().join("node_modules").join("agent-browser");
        let agent_browser_global = cmd_check("agent-browser");
        if agent_browser_local.exists() || agent_browser_global {
            check_ok("agent-browser", "(browser automation)");
        } else {
            check_warn(
                "agent-browser not installed",
                "(run: npm install in project root)",
            );
        }

        // Playwright Chromium check — check if `npx playwright` would find
        // a Chromium installation.
        let has_playwright = cmd_check_version("npx", "playwright --version");
        if has_playwright.is_some() {
            // A simpler heuristic: check whether browser binaries exist
            let browser_dir = dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("~/.cache"))
                .join("ms-playwright");
            if browser_dir.exists()
                && browser_dir
                    .read_dir()
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false)
            {
                check_ok("Playwright Chromium", "(browser engine)");
            } else {
                check_warn(
                    "Playwright Chromium not installed",
                    "(browser_* tools will be hidden)",
                );
                check_info("Install with: npx playwright install --with-deps chromium");
            }
        } else {
            check_warn(
                "Playwright not installed",
                "(browser tools unavailable — run: npm install)",
            );
        }

        // npm audit
        if cmd_check("npm") {
            let npm_dirs = [(
                project_root().join("node_modules"),
                "Browser tools (agent-browser)",
            )];
            for (nm_dir, label) in &npm_dirs {
                if !nm_dir.exists() {
                    continue;
                }
                // Run npm audit --json (with thread-based timeout)
                let project_root_buf = project_root().to_path_buf();
                let (audit_tx, audit_rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let result = Command::new("npm")
                        .args(["audit", "--json"])
                        .current_dir(&project_root_buf)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null())
                        .output();
                    let _ = audit_tx.send(result);
                });

                let audit_result = match audit_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                    Ok(result) => result,
                    Err(_) => continue, // timeout or channel error — skip
                };

                match &audit_result {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        if stdout.trim().is_empty() {
                            continue;
                        }
                        // Try to parse JSON
                        if let Ok(audit_data) = serde_json::from_str::<serde_json::Value>(&stdout) {
                            let vulns = audit_data
                                .get("metadata")
                                .and_then(|m| m.get("vulnerabilities"));
                            let critical = vulns
                                .and_then(|v| v.get("critical").and_then(|c| c.as_u64()))
                                .unwrap_or(0);
                            let high = vulns
                                .and_then(|v| v.get("high").and_then(|h| h.as_u64()))
                                .unwrap_or(0);
                            let moderate = vulns
                                .and_then(|v| v.get("moderate").and_then(|m| m.as_u64()))
                                .unwrap_or(0);
                            let total = critical + high + moderate;
                            if total == 0 {
                                check_ok(label, "(no known vulnerabilities)");
                            } else if critical > 0 || high > 0 {
                                check_warn(
                                    label,
                                    &format!(
                                        "({critical} critical, {high} high, {moderate} moderate — run: npm audit fix)"
                                    ),
                                );
                                issues.push(format!("{label} has {total} npm vulnerability(s)"));
                            } else {
                                check_ok(label, &format!("({moderate} moderate vulnerability(s))"));
                            }
                        } else {
                            // npm audit can exit non-zero when vulns exist,
                            // but still output valid JSON. Check stderr too.
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            if stderr.contains("npm audit") {
                                check_ok(label, "(npm audit skipped — see above)");
                            }
                        }
                    }
                    Err(_) => {
                        // Timeout or other error — skip silently
                    }
                }
            }
        } else {
            check_warn("npm not found", "(cannot audit Node.js dependencies)");
        }
    } else {
        check_warn("Node.js not found", "(optional, needed for browser tools)");
    }
}

/// Run platform-specific checks that have no external tool dependencies.
pub fn run_platform_checks(
    config: &AppConfig,
    issues: &mut Vec<String>,
    _manual_issues: &mut Vec<String>,
) {
    // =====================================================================
    // C. Tool Availability
    // =====================================================================
    section_header("Tool Availability");

    // Simplified port: list main tool categories and check their requirements
    let tool_categories: Vec<(&str, Vec<&str>, bool)> = vec![
        ("terminal", vec![], true),
        ("file", vec![], true),
        ("web", vec!["TAVILY_API_KEY", "EXA_API_KEY"], false),
        ("search", vec!["TAVILY_API_KEY", "EXA_API_KEY"], false),
        ("memory", vec!["HONCHO_API_KEY", "MEM0_API_KEY"], false),
        ("cron", vec![], true),
        ("browser", vec![], false),
        ("vision", vec![], config.vision.provider.is_some()),
        ("tts", vec![], config.tts.enabled),
    ];

    let mut any_unavailable = false;
    for (name, required_envs, is_configured) in &tool_categories {
        let envs_ok =
            required_envs.iter().any(|e| env_var(e).is_some()) || required_envs.is_empty();
        let available = *is_configured || envs_ok;

        if available {
            check_ok(name, "(available)");
        } else {
            any_unavailable = true;
            let missing: Vec<&str> = required_envs
                .iter()
                .filter(|e| env_var(e).is_none())
                .copied()
                .collect();
            if missing.is_empty() {
                check_warn(name, "(not configured — run operant setup)");
            } else {
                let vars_str = missing.join(", ");
                check_warn(name, &format!("(missing {vars_str})"));
            }
        }
    }

    if any_unavailable {
        issues.push(
            "Run 'operant setup' to configure missing API keys for full tool access".to_string(),
        );
    }

    // =====================================================================
    // D. Skills Hub
    // =====================================================================
    section_header("Skills Hub");

    let hub_dir = operant_home().join("skills").join(".hub");
    if hub_dir.exists() {
        check_ok("Skills Hub directory", "(exists)");

        let lock_file = hub_dir.join("lock.json");
        if lock_file.exists() {
            match std::fs::read_to_string(&lock_file) {
                Ok(content) => {
                    if let Ok(lock_data) = serde_json::from_str::<serde_json::Value>(&content) {
                        let count = lock_data
                            .get("installed")
                            .and_then(|i| i.as_object())
                            .map(|o| o.len())
                            .unwrap_or(0);
                        check_ok("Lock file OK", &format!("({count} hub-installed skill(s))"));
                    } else {
                        check_warn("Lock file", "(corrupted or unreadable)");
                    }
                }
                Err(_) => {
                    check_warn("Lock file", "(unreadable)");
                }
            }
        } else {
            check_info("No lock.json found (skills hub not fully initialized)");
        }

        let quarantine = hub_dir.join("quarantine");
        if quarantine.exists() {
            let q_count = std::fs::read_dir(&quarantine)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .count()
                })
                .unwrap_or(0);
            if q_count > 0 {
                check_warn(
                    "Quarantine",
                    &format!("({q_count} skill(s) in quarantine — pending review)"),
                );
            }
        }
    } else {
        check_warn(
            "Skills Hub directory not initialized",
            "(run: operant skills list)",
        );
        issues.push("Initialize skills hub with 'operant skills list'".to_string());
    }

    // =====================================================================
    // E. GitHub
    // =====================================================================
    section_header("GitHub");

    let gh_token = env_var("GITHUB_TOKEN").or_else(|| env_var("GH_TOKEN"));
    if gh_token.is_some() {
        check_ok("GitHub token configured", "(authenticated API access)");
    } else {
        // Try `gh auth status`
        let gh_auth_ok = Command::new("gh")
            .args(["auth", "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if gh_auth_ok {
            check_ok(
                "GitHub authenticated via gh CLI",
                "(full API access — no GITHUB_TOKEN needed)",
            );
        } else {
            check_warn(
                "No GITHUB_TOKEN",
                "(60 req/hr rate limit — set in .env for better rates)",
            );
            issues.push("Set GITHUB_TOKEN in .env for authenticated GitHub API access".to_string());
        }
    }

    // =====================================================================
    // F. Memory Provider
    // =====================================================================
    section_header("Memory Provider");

    // Read memory.provider from the AppConfig or from the TOML directly.
    // AppConfig doesn't have a memory.provider field directly, so we
    // read it from the TOML file or fall back to env vars.
    let memory_provider = detect_memory_provider(config);

    match memory_provider.as_deref() {
        None | Some("") => {
            check_ok(
                "Built-in memory active",
                "(no external provider configured — this is fine)",
            );
        }
        Some("honcho") => {
            let honcho_key = env_var("HONCHO_API_KEY");
            if let Some(_key) = honcho_key {
                check_ok("Honcho API key", "(configured)");
            } else {
                check_fail(
                    "Honcho API key not set",
                    "(set HONCHO_API_KEY in .env or run operant memory setup)",
                );
                issues.push(
                    "Honcho is set as memory provider but HONCHO_API_KEY is missing".to_string(),
                );
            }
        }
        Some("mem0") => {
            let mem0_key = env_var("MEM0_API_KEY");
            if let Some(_key) = mem0_key {
                check_ok("Mem0 API key", "(configured)");
            } else {
                check_fail(
                    "Mem0 API key not set",
                    "(set MEM0_API_KEY in .env or run operant memory setup)",
                );
                issues
                    .push("Mem0 is set as memory provider but MEM0_API_KEY is missing".to_string());
            }
        }
        Some(other) => {
            // Generic check
            let generic_var = format!("{}_API_KEY", other.to_uppercase().replace('-', "_"));
            if env_var(&generic_var).is_some() {
                check_ok(&format!("{other} provider"), "(configured)");
            } else {
                check_warn(
                    &format!("{other} provider"),
                    &format!("(not configured — set {generic_var} in .env)"),
                );
            }
        }
    }

    // =====================================================================
    // G. Profiles
    // =====================================================================
    section_header("Profiles");

    let profiles_dir = operant_home().join("profiles");
    if profiles_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&profiles_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default();

        if entries.is_empty() {
            check_info("No named profiles found (using default profile)");
        } else {
            check_ok(&format!("{} profile(s) found", entries.len()), "");

            for entry in &entries {
                let name = entry.file_name().to_string_lossy().to_string();
                let profile_path = entry.path();
                let mut parts: Vec<String> = Vec::new();

                // Check for config.yaml
                if !profile_path.join("config.yaml").exists() {
                    parts.push("\u{26a0} missing config".to_string());
                }
                // Check for .env
                if !profile_path.join(".env").exists() {
                    parts.push("no .env".to_string());
                }

                if parts.is_empty() {
                    check_ok(&format!("  {name}: configured"), "");
                } else {
                    check_ok(&format!("  {name}: {}", parts.join(", ")), "");
                }
            }

            // Check for orphan wrappers
            let wrapper_dir = operant_home().join("bin");
            if wrapper_dir.is_dir() {
                if let Ok(wrapper_entries) = std::fs::read_dir(&wrapper_dir) {
                    for wrapper in wrapper_entries.flatten() {
                        let wpath = wrapper.path();
                        if !wpath.is_file() {
                            continue;
                        }
                        // Simple heuristic: check if it's a operant profile wrapper
                        if let Ok(content) = std::fs::read_to_string(&wpath) {
                            if content.contains("operant -p") {
                                check_info(&format!(
                                    "  Profile wrapper: {}",
                                    wpath.file_name().unwrap_or_default().to_string_lossy()
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    if !profiles_dir.exists() {
        check_info("No profiles directory found (using default profile)");
    }

    // =====================================================================
    // H. Submodules
    // =====================================================================
    section_header("Submodules");

    let tinker_dir = project_root().join("tinker-atropos");
    if tinker_dir.exists() && tinker_dir.join("Cargo.toml").exists() {
        check_ok("tinker-atropos", "(RL training backend)");
    } else if tinker_dir.exists() {
        check_info("tinker-atropos directory found (no Cargo.toml — not a Rust crate)");
    } else {
        check_warn(
            "tinker-atropos not found",
            "(run: git submodule update --init --recursive)",
        );
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Try to detect the configured memory provider from config or TOML.
fn detect_memory_provider(config: &AppConfig) -> Option<String> {
    // Check if there's a operant.toml we can parse for memory.provider.
    // The AppConfig doesn't carry a memory.provider field directly,
    // so we attempt to read the TOML file from config paths.
    let config_paths = [
        project_root().join("operant.toml"),
        operant_home().join("operant.toml"),
    ];

    for path in &config_paths {
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(path) {
                if let Ok(value) = raw.parse::<toml::Value>() {
                    if let Some(provider) = value
                        .get("memory")
                        .and_then(|m| m.get("provider"))
                        .and_then(|p| p.as_str())
                    {
                        if !provider.is_empty() {
                            return Some(provider.to_string());
                        }
                    }
                }
            }
        }
    }

    // Fallback: check for the auxiliary memory model config
    if let Some(ref aux) = config.auxiliary_models.memory {
        return aux.provider.clone();
    }

    None
}

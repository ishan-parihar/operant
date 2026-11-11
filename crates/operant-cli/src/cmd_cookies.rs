//! CLI subcommand for browser cookie management (`operant cookies`).
//!
//! Imports cookies from any browser (Chrome / Brave / Edge / Chromium /
//! Firefox) into the sourcehound browser session so accounts are usable without
//! manual login — the multi-browser cookie import mechanism.
//!
//! # Usage
//!
//! - `operant cookies import <file>`         — import a cookies.txt / JSON export
//! - `operant cookies import --browser brave` — read directly from a browser's
//!   cookie database (Firefox plaintext; Chromium-family v10 decrypted)
//! - `operant cookies list`                  — list cookies in the browser session
//! - `operant cookies export <file>`         — dump session cookies as cookies.txt
//! - `operant cookies clear`                 — clear all cookies in the session

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::cookies::{self, Cookie};

/// Subcommands for browser cookie management.
#[derive(Debug, Clone, Subcommand)]
pub enum CookiesSubcommand {
    /// Import cookies from a file (Netscape cookies.txt or EditThisCookie
    /// JSON) or directly from a browser profile.
    Import {
        /// Path to a cookies.txt / JSON export file.
        file: Option<PathBuf>,
        /// Import directly from a browser's cookie database:
        /// chrome, chromium, brave, brave-beta, brave-origin-beta, edge,
        /// vivaldi, opera, firefox, zen.
        #[arg(long)]
        browser: Option<String>,
        /// Show the cookies that would be imported without applying them.
        #[arg(long)]
        dry_run: bool,
    },
    /// List cookies currently in the browser session.
    List,
    /// Export session cookies to a Netscape cookies.txt file.
    Export {
        /// Output file path (default: cookies.txt in CWD).
        output: Option<PathBuf>,
    },
    /// Clear all cookies in the browser session.
    Clear,
    /// Discover browser cookie databases on this machine.
    Discover,
}

/// Dispatch and execute a cookies subcommand.
pub async fn handle_cookies_command(cmd: CookiesSubcommand) -> Result<()> {
    match cmd {
        CookiesSubcommand::Import {
            file,
            browser,
            dry_run,
        } => handle_import(file, browser, dry_run).await,
        CookiesSubcommand::List => handle_list().await,
        CookiesSubcommand::Export { output } => handle_export(output).await,
        CookiesSubcommand::Clear => handle_clear().await,
        CookiesSubcommand::Discover => handle_discover(),
    }
}

/// The browser names `operant cookies import --browser` accepts.
///
/// Mirrors the core's discovery list. It lives here so the CLI can tell a
/// browser NAME from a FILE PATH without reaching the store — see
/// [`load_cookies`].
const BROWSER_NAMES: &[&str] = &[
    "chrome",
    "chromium",
    "brave",
    "brave-beta",
    "brave-origin-beta",
    "edge",
    "vivaldi",
    "opera",
    "firefox",
    "zen",
];

/// Decide where a positional import argument goes.
///
/// `operant cookies import chrome` was read as a FILE named "chrome":
/// "Failed to read cookie file chrome". A browser name in the positional
/// slot is what the operator meant, so it is routed to the browser path
/// rather than to the file reader. An explicit `--browser` always wins, and
/// a path that is not a browser name is left exactly as it was.
fn route_import_target(
    file: Option<PathBuf>,
    browser: Option<String>,
) -> (Option<PathBuf>, Option<String>) {
    match (file, browser) {
        (Some(path), None) => {
            let name = path.to_string_lossy().to_ascii_lowercase();
            if BROWSER_NAMES.contains(&name.as_str()) {
                (None, Some(name))
            } else {
                (Some(path), None)
            }
        }
        (file, browser) => (file, browser),
    }
}

/// Load cookies from a file (auto-detect cookies.txt vs JSON) or browser DB.
fn load_cookies(file: Option<PathBuf>, browser: Option<String>) -> Result<Vec<Cookie>> {
    let (file, browser) = route_import_target(file, browser);
    if let Some(browser_name) = browser {
        let found = cookies::find_browser_cookie_source(&browser_name).with_context(|| {
            format!(
                "No cookie database found for browser '{browser_name}'. \
                 Try `operant cookies discover` to list detected browsers, \
                 or export a cookies.txt from the browser and import that file."
            )
        })?;
        let (label, db, local_state) = found;
        let cookies = if label == "firefox" || label == "zen" {
            // Firefox-family (Firefox, Zen Browser): plaintext values.
            cookies::read_firefox_cookies(&db)
        } else {
            let (cookies, report) =
                cookies::read_chromium_cookies_report(&db, local_state.as_deref());
            if cookies.is_empty() {
                let mut hints = Vec::new();
                if report.app_bound > 0 {
                    hints.push(format!(
                        "{label} encrypts {} cookies with app-bound encryption \
                         (v11) — a deliberate anti-cookie-theft layer that only \
                         the browser itself can decrypt.",
                        report.app_bound
                    ));
                }
                if report.undecryptable > 0 || report.total_rows == 0 {
                    let n = report.undecryptable;
                    hints.push(format!(
                        "{n} cookie(s) use a key stored in the \
                         OS keyring that could not be resolved on this machine."
                    ));
                }
                anyhow::bail!(
                    "Read 0 cookies from {label} ({}). {}\n\nThe universal fix for \
                     app-bound/keyring profiles: export a cookies.txt from the \
                     browser (e.g. with the 'Get cookies.txt LOCALLY' or \
                     'Cookie-Editor' extension) and import it with \
                     `operant cookies import <file>`.",
                    db.display(),
                    hints.join(" ")
                );
            }
            if report.app_bound > 0 || report.undecryptable > 0 {
                eprintln!(
                    "Note: skipped {} app-bound + {} undecryptable cookie(s) \
                     (modern Chromium anti-theft encryption).",
                    report.app_bound, report.undecryptable
                );
            }
            cookies
        };
        if cookies.is_empty() {
            anyhow::bail!("No cookies found in {label} database at {}", db.display());
        }
        return Ok(cookies);
    }

    let path = file.context("Provide a cookie file path or --browser <name>")?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read cookie file {}", path.display()))?;
    let trimmed = text.trim_start();
    let cookies = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        cookies::parse_json_cookies(&text)
    } else {
        cookies::parse_netscape_cookies_txt(&text)
    };
    if cookies.is_empty() {
        anyhow::bail!("No cookies parsed from {}", path.display());
    }
    Ok(cookies)
}

async fn handle_import(
    file: Option<PathBuf>,
    browser: Option<String>,
    dry_run: bool,
) -> Result<()> {
    let cookies = load_cookies(file, browser)?;
    let domains: Vec<&str> = cookies
        .iter()
        .map(|c| c.domain.trim_start_matches('.'))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    println!(
        "Loaded {} cookies for {} domain(s):",
        cookies.len(),
        domains.len()
    );
    for d in domains.iter().take(12) {
        println!("  - {}", d);
    }
    if domains.len() > 12 {
        println!("  - …and {} more", domains.len() - 12);
    }

    if dry_run {
        println!("Dry run — no cookies applied.");
        return Ok(());
    }

    let applied = operant_core::sourcehound_cdp::import_cookies(&cookies).await?;
    println!(
        "✓ Applied {}/{} cookies to the browser session.",
        applied,
        cookies.len()
    );
    if applied < cookies.len() {
        println!(
            "Note: {} cookies were skipped (server rejected or undecryptable).",
            cookies.len() - applied
        );
    }
    println!("Cookies persist for this session and apply to every page the browser navigates to.");
    Ok(())
}

async fn handle_list() -> Result<()> {
    let cookies = operant_core::sourcehound_cdp::export_cookies().await?;
    if cookies.is_empty() {
        println!("No cookies in the browser session.");
        return Ok(());
    }
    println!("{} cookie(s) in the browser session:", cookies.len());
    for c in cookies {
        let flags = if c.secure { "S" } else { "-" };
        let http = if c.http_only { "H" } else { "-" };
        let expiry = c
            .expires
            .map(|e| e.to_string())
            .unwrap_or_else(|| "session".to_string());
        println!(
            "  [{}{}] {}  {}  (path={}, expires={})",
            flags, http, c.domain, c.name, c.path, expiry
        );
    }
    Ok(())
}

async fn handle_export(output: Option<PathBuf>) -> Result<()> {
    let cookies = operant_core::sourcehound_cdp::export_cookies().await?;
    if cookies.is_empty() {
        println!("No cookies in the browser session — nothing exported.");
        return Ok(());
    }
    let path = output.unwrap_or_else(|| PathBuf::from("cookies.txt"));
    let text = cookies::cookies_to_netscape(&cookies);
    std::fs::write(&path, text).with_context(|| format!("Failed to write {}", path.display()))?;
    println!("✓ Exported {} cookies to {}", cookies.len(), path.display());
    Ok(())
}

async fn handle_clear() -> Result<()> {
    operant_core::sourcehound_cdp::clear_cookies().await?;
    println!("✓ Cleared all cookies in the browser session.");
    Ok(())
}

fn handle_discover() -> Result<()> {
    let sources = cookies::discover_browser_cookie_sources();
    if sources.is_empty() {
        println!("No browser cookie databases detected on this machine.");
        return Ok(());
    }
    println!("Detected browser cookie databases:");
    for (label, db, _ls) in sources {
        println!("  {:<10} {}", label, db.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_returns_list() {
        // Function must not panic and returns a Vec (may be empty in CI).
        let _ = cookies::discover_browser_cookie_sources();
    }

    #[test]
    fn a_browser_name_in_the_positional_slot_routes_to_the_browser_path() {
        // `operant cookies import chrome` was read as a file named "chrome".
        let (file, browser) = route_import_target(Some("chrome".into()), None);
        assert_eq!(file, None);
        assert_eq!(browser.as_deref(), Some("chrome"));
    }

    #[test]
    fn a_real_file_path_is_left_alone() {
        let path = std::path::PathBuf::from("/tmp/cookies.txt");
        let (file, browser) = route_import_target(Some(path.clone()), None);
        assert_eq!(file, Some(path));
        assert_eq!(browser, None);
    }

    #[test]
    fn an_explicit_browser_flag_wins_over_the_position() {
        let path = std::path::PathBuf::from("/tmp/cookies.txt");
        let (file, browser) = route_import_target(Some(path.clone()), Some("brave".into()));
        assert_eq!(file, Some(path));
        assert_eq!(browser.as_deref(), Some("brave"));
    }
}

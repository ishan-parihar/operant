//! Module-level re-exports for the `operant doctor` command.
//!
//! This module is a drop-in replacement for the original flat `cmd_doctor.rs`.
//! It delegates individual check groups to sub-modules and exposes the same
//! public API (`handle_doctor_command`) that `main.rs` calls.

pub mod check_result;
pub mod checks_api;
pub mod checks_config;
pub mod checks_fix;
pub mod checks_tools;

use anyhow::Result;
use operant_core::config::AppConfig;

use self::check_result::{print_banner, print_summary};

/// The exit code `operant doctor` should return.
///
/// Split out because `process::exit` cannot be called from a test without
/// killing the test binary, and this is the only part of the command worth
/// asserting on.
///
/// It used to always be 0. On a fresh machine doctor printed "Found 4 issue(s)
/// to address" and still returned success, so the natural shell incantation
/// `operant doctor && operant chat` walked straight into the failure doctor had
/// just described. A diagnostic that cannot fail is a diagnostic nobody can
/// gate on.
///
/// Both lists count. `issues` are auto-fixable and `manual_issues` need a human,
/// but neither means the system is healthy, and a caller gating on the exit code
/// has no way to tell the two apart without parsing the output.
/// Machine-readable exit status.
///
/// Only `issues` — genuine problems — produce a non-zero code.
/// `_manual_issues` are ADVISORIES: "git not found (recommended)", "Nous Portal
/// auth (not logged in)", "~/.operant/cron/ will be created on first use".
/// Counting them made `operant doctor` exit 1 on a perfectly working install
/// that simply had not configured optional integrations — 13 advisories were
/// present on a working machine, so the exit code was permanently 1 and
/// carried no information. That is the mirror image of the bug this function
/// was written to fix (a doctor that could never fail), and just as useless to
/// anything scripting it. A check that always fails carries no more
/// information than one that never does.
///
/// Exit 1 = something is wrong. Exit 0 = healthy, advisories displayed.
fn doctor_exit_code(issues: &[String], _manual_issues: &[String]) -> i32 {
    if issues.is_empty() { 0 } else { 1 }
}

/// Dispatch handle — called from `main.rs` for `operant doctor [--fix] [--json]`.
pub async fn handle_doctor_command(config: &AppConfig, fix: bool, json: bool) -> Result<()> {
    if fix {
        return checks_fix::cmd_fix(config).await;
    }

    let mut issues: Vec<String> = Vec::new();
    let mut manual_issues: Vec<String> = Vec::new();

    // Each section runs its checks and returns (issues, manual_issues).
    checks_config::run_config_checks(config, &mut issues);
    checks_tools::run_tool_checks(config, &mut issues, &mut manual_issues);
    checks_api::run_api_checks(config, &mut issues).await;
    checks_tools::run_platform_checks(config, &mut issues, &mut manual_issues);

    if json {
        let result = serde_json::json!({
            "issues": issues,
            "manual_issues": manual_issues,
            "total_issues": issues.len() + manual_issues.len(),
            "auto_fixable": issues.len(),
            "manual_required": manual_issues.len(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_banner();
        print_summary(&issues, &manual_issues, 0, false);
    }

    // Flush before exiting. stdout is block-buffered when it is not a terminal
    // and `process::exit` runs no destructors, so a non-zero exit here would
    // otherwise throw away the report the user just asked for — and the case
    // where that bites hardest is the piped one (`operant doctor | tee log`),
    // which is exactly where a machine-readable exit code is most wanted.
    use std::io::Write;
    let _ = std::io::stdout().flush();

    let code = doctor_exit_code(&issues, &manual_issues);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::doctor_exit_code;

    fn issues(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("issue {i}")).collect()
    }

    #[test]
    fn a_clean_doctor_run_exits_zero() {
        assert_eq!(doctor_exit_code(&[], &[]), 0);
    }

    /// The regression: doctor found things and reported success anyway.
    #[test]
    fn finding_issues_exits_nonzero() {
        assert_eq!(doctor_exit_code(&issues(4), &[]), 1);
    }

    /// Manual issues count too — a caller gating on the exit code cannot tell
    /// Manual issues are advisories ("git not found (recommended)"), not
    /// failures. Counting them meant a healthy install with unconfigured
    /// optional integrations could never exit 0 — 13 such advisories were
    /// present on a working machine — so the exit code carried no information
    /// at all. This pins the corrected contract: advisories are displayed, and
    /// they do not gate a caller.
    #[test]
    fn manual_issues_do_not_fail_the_run() {
        assert_eq!(doctor_exit_code(&[], &issues(1)), 0);
        assert_eq!(doctor_exit_code(&[], &issues(13)), 0);
    }

    /// …but a real issue still fails even alongside advisories, which is the
    /// half of the contract that matters for scripting.
    #[test]
    fn real_issue_fails_even_with_advisories_present() {
        assert_eq!(doctor_exit_code(&issues(1), &issues(5)), 1);
    }

    #[test]
    fn a_single_issue_is_enough_to_fail() {
        assert_eq!(doctor_exit_code(&["one".to_string()], &[]), 1);
    }
}

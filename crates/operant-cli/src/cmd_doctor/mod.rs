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
    checks_api::run_api_checks(&mut issues).await;
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
    Ok(())
}

//! Check result type and display helpers for `hermes doctor`.
//!
//! Mirrors the Python `check_ok` / `check_warn` / `check_fail` / `check_info`
//! helpers from `hermes-agent/hermes_cli/doctor.py`.

use console::style;

/// The outcome of a single diagnostic check.
#[derive(Debug)]
pub struct CheckResult {
    /// Human-readable check label (e.g. `"Config file loaded"`).
    pub label: String,
    /// Whether the check passed (`true`) or had an issue (`false`).
    pub passed: bool,
    /// Optional detail / hint string shown after the label.
    pub detail: Option<String>,
}

impl CheckResult {
    pub fn new(label: impl Into<String>, passed: bool, detail: Option<String>) -> Self {
        Self {
            label: label.into(),
            passed,
            detail,
        }
    }

    pub fn ok(label: impl Into<String>) -> Self {
        Self::new(label, true, None)
    }

    pub fn ok_with(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(label, true, Some(detail.into()))
    }

    pub fn fail(label: impl Into<String>) -> Self {
        Self::new(label, false, None)
    }

    pub fn fail_with(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(label, false, Some(detail.into()))
    }
}

/// Print a passed check line.
///
/// Output: `  ✓ <label> <detail>` (detail in dim style).
pub fn check_ok(label: &str, detail: &str) {
    let icon = style("✓").green();
    if detail.is_empty() {
        println!("  {} {}", icon, label);
    } else {
        println!("  {} {} {}", icon, label, style(detail).dim());
    }
}

/// Print a warning check line.
///
/// Output: `  ⚠ <label> <detail>` (detail in dim style).
pub fn check_warn(label: &str, detail: &str) {
    let icon = style("⚠").yellow();
    if detail.is_empty() {
        println!("  {} {}", icon, label);
    } else {
        println!("  {} {} {}", icon, label, style(detail).dim());
    }
}

/// Print a failed check line.
///
/// Output: `  ✗ <label> <detail>` (detail in dim style).
pub fn check_fail(label: &str, detail: &str) {
    let icon = style("✗").red();
    if detail.is_empty() {
        println!("  {} {}", icon, label);
    } else {
        println!("  {} {} {}", icon, label, style(detail).dim());
    }
}

/// Print an informational line (indented).
///
/// Output: `    → <text>` (text in cyan).
pub fn check_info(text: &str) {
    println!("  {} {}", style("→").cyan(), text);
}

/// Print a section header.
///
/// Output: `◆ <title>` (bold, cyan) with a preceding blank line.
pub fn section_header(title: &str) {
    println!();
    println!("{}", style(format!("◆ {}", title)).cyan().bold());
}

/// Print the doctor banner at the start.
pub fn print_banner() {
    println!();
    println!(
        "{}",
        style("┌─────────────────────────────────────────────────────────┐").cyan()
    );
    println!(
        "{}",
        style("│                 🩺 Hermes Doctor                        │").cyan()
    );
    println!(
        "{}",
        style("└─────────────────────────────────────────────────────────┘").cyan()
    );
}

/// Print the summary section with issue counts and fix suggestions.
pub fn print_summary(
    issues: &[String],
    manual_issues: &[String],
    fixed_count: usize,
    should_fix: bool,
) {
    let all_issues: Vec<&String> = issues.iter().chain(manual_issues.iter()).collect();
    println!();

    if should_fix && fixed_count > 0 {
        println!("{}", style("─".repeat(60)).green());
        print!("{}", style("  Fixed ").green().bold());
        print!(
            "{}",
            style(format!("{} issue(s).", fixed_count)).green().bold()
        );
        if !all_issues.is_empty() {
            println!(
                "{}",
                style(format!(
                    " {} issue(s) require manual intervention.",
                    all_issues.len()
                ))
                .yellow()
                .bold()
            );
        } else {
            println!();
        }
        println!();
        if !all_issues.is_empty() {
            for (i, issue) in all_issues.iter().enumerate() {
                println!("  {}. {}", i + 1, issue);
            }
            println!();
        }
    } else if !all_issues.is_empty() {
        println!("{}", style("─".repeat(60)).yellow());
        println!(
            "{}",
            style(format!("  Found {} issue(s) to address:", all_issues.len()))
                .yellow()
                .bold()
        );
        println!();
        for (i, issue) in all_issues.iter().enumerate() {
            println!("  {}. {}", i + 1, issue);
        }
        println!();
        if !should_fix {
            println!(
                "{}",
                style("  Tip: run 'hermes doctor --fix' to auto-fix what's possible.").dim()
            );
        }
    } else {
        println!("{}", style("─".repeat(60)).green());
        println!(
            "{} {}",
            style("  All checks passed!").green().bold(),
            style("🎉").green()
        );
    }
}

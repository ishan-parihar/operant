//! Shared interactive prompt helpers for the setup wizard.
//!
//! Follows Python's pattern of "show current value, Enter to keep, or type new value".
//! Provides reusable prompts for text, passwords, confirmations, selections, and more.

use anyhow::{Context, Result};
use console::style;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Password, Select};
use std::fmt::Display;

/// Prompt user for text input with a current/default value.
/// Empty input returns the default.
pub fn prompt_text<T: Display>(question: &str, default: T) -> Result<String> {
    let display = format!("{} [{}]: ", question, default);
    let value: String = Input::new()
        .with_prompt(&display)
        .allow_empty(true)
        .default(default.to_string())
        .interact_text()
        .context("Failed to read input")?;
    Ok(value.trim().to_string())
}

/// Prompt user for text input with password masking.
pub fn prompt_password(question: &str) -> Result<String> {
    let value = Password::new()
        .with_prompt(question)
        .allow_empty_password(true)
        .interact()
        .context("Failed to read password")?;
    Ok(value)
}

/// Prompt for yes/no with a default.
pub fn prompt_yes_no(question: &str, default: bool) -> Result<bool> {
    Confirm::new()
        .with_prompt(question)
        .default(default)
        .interact()
        .context("Failed to read confirmation")
}

/// Select from a list of options with a default index.
pub fn prompt_select(question: &str, options: &[&str], default: usize) -> Result<usize> {
    Select::new()
        .with_prompt(question)
        .items(options)
        .default(default.min(options.len().saturating_sub(1)))
        .interact()
        .context("Failed to select option")
}

/// Select from a list of (label, description) pairs, rendered as "label — description".
pub fn prompt_select_with_desc(
    question: &str,
    options: &[(&str, &str)],
    default: usize,
) -> Result<usize> {
    let items: Vec<String> = options
        .iter()
        .map(|(label, desc)| format!("{} — {}", label, desc))
        .collect();
    Select::new()
        .with_prompt(question)
        .items(&items)
        .default(default.min(items.len().saturating_sub(1)))
        .interact()
        .context("Failed to select option")
}

/// Fuzzy-select from a list of options with a default index.
pub fn prompt_fuzzy_select(question: &str, items: &[String], default: usize) -> Result<usize> {
    FuzzySelect::new()
        .with_prompt(question)
        .items(items)
        .default(default.min(items.len().saturating_sub(1)))
        .interact()
        .context("Failed to select option")
}

/// Multi-select checklist. Returns indices of selected items.
/// `defaults` specifies which items should be pre-selected by their index.
pub fn prompt_multi_select(
    question: &str,
    items: &[String],
    defaults: &[usize],
) -> Result<Vec<usize>> {
    let mut selected = vec![false; items.len()];
    for &idx in defaults {
        if idx < items.len() {
            selected[idx] = true;
        }
    }
    MultiSelect::new()
        .with_prompt(question)
        .items(items)
        .defaults(&selected)
        .interact()
        .context("Failed to select options")
}

/// Select a number within a range.
pub fn prompt_range(question: &str, default: f64, min: f64, max: f64) -> Result<f64> {
    let value: String = Input::new()
        .with_prompt(format!("{} ({}–{}) [{}]", question, min, max, default))
        .default(default.to_string())
        .allow_empty(true)
        .interact_text()
        .context("Failed to read number")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    let parsed: f64 = trimmed.parse().unwrap_or(default);
    Ok(parsed.clamp(min, max))
}

/// Action to take for an existing API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Keep,
    Replace,
    Clear,
}

/// [K]eep/[R]eplace/[C]lear prompt for existing API keys.
/// Shows masked key, returns Keep/Replace/Clear decision.
pub fn prompt_key_action(label: &str, key: &str) -> Result<KeyAction> {
    let masked = if key.len() > 8 {
        format!("{}…{}", &key[..4], &key[key.len() - 4..])
    } else {
        "configured".to_string()
    };
    println!(
        "  {} API key: {} {}",
        style(label).bold(),
        masked,
        style("✓").green()
    );

    let choice: String = Input::new()
        .with_prompt("[K]eep / [R]eplace / [C]lear (default K)")
        .default("k".to_string())
        .allow_empty(true)
        .interact_text()
        .context("Failed to read choice")?;

    match choice.trim().to_lowercase().chars().next() {
        Some('r') => Ok(KeyAction::Replace),
        Some('c') => Ok(KeyAction::Clear),
        _ => Ok(KeyAction::Keep),
    }
}

/// Print a section header matching the Python style.
pub fn print_header(title: &str) {
    println!();
    println!("{}", style(format!("  ◆ {}", title)).yellow().bold());
}

/// Print an info message.
pub fn print_info(msg: &str) {
    println!("  {}", msg);
}

/// Print a success message.
pub fn print_success(msg: &str) {
    println!("  {} {}", style("✓").green(), msg);
}

/// Print a warning message.
pub fn print_warning(msg: &str) {
    println!("  {} {}", style("⚠").yellow(), msg);
}

/// Print a bordered page header with centered title (60 chars wide).
pub fn print_page_header(title: &str) {
    // Clear the terminal before each page so the wizard doesn't append and
    // extend everything into a long scrolling wall of text.
    // (iter-119 — user-reported bug: setup wizard pages stack on top of
    // each other instead of refreshing.)
    print!("\x1b[2J\x1b[H"); // ANSI clear screen + move cursor to top
    use std::io::Write;
    let _ = std::io::stdout().flush();

    const WIDTH: usize = 60;
    const INNER: usize = WIDTH - 2; // 58 chars between borders
    println!();
    println!("╔{}╗", "═".repeat(INNER));
    let char_count = title.chars().count();
    let display: String = if char_count > INNER {
        title.chars().take(INNER).collect()
    } else {
        title.to_string()
    };
    let padding = INNER - display.chars().count();
    let left_pad = padding / 2;
    let right_pad = padding - left_pad;
    println!(
        "║{:left$}{}{:right$}║",
        "",
        display,
        "",
        left = left_pad,
        right = right_pad
    );
    println!("╚{}╝", "═".repeat(INNER));
}

/// Print a page footer separator (60 chars wide).
pub fn print_page_footer() {
    const WIDTH: usize = 60;
    const INNER: usize = WIDTH - 2;
    println!("╚{}╝", "═".repeat(INNER));
    println!();
}

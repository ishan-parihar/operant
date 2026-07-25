//! Free functions used by the app module.
//!
//! Contains command registry helpers, provider picker items, and import
//! configuration picker items — all module-level functions with no `self` dependency.

use crate::tui::dialog_select::SelectItem;
use crate::tui::overlays::HelpEntry;
use crate::provider::PROVIDERS;

// ---------------------------------------------------------------------------
// Unified command data (single source of truth from COMMAND_REGISTRY)
// ---------------------------------------------------------------------------

/// All TUI-available slash commands as `(name, description)` pairs.
/// Derived from the canonical `COMMAND_REGISTRY` — replaces the old hardcoded
/// `PROMPT_SLASH_COMMANDS` constant. Used by typeahead and command palette.
pub(super) fn tui_slash_command_data() -> Vec<(&'static str, &'static str)> {
    crate::commands::COMMAND_REGISTRY
        .iter()
        .filter(|cmd| !cmd.gateway_only)
        .map(|cmd| (cmd.name, cmd.description))
        .collect()
}

/// Generate help overlay entries from the unified command registry.
/// Each entry includes aliases (e.g. "n, clear" for /new) and the
/// TUI category for grouped display.
pub(super) fn help_overlay_entries() -> Vec<HelpEntry> {
    crate::commands::tui_slash_commands()
        .into_iter()
        .map(|cmd| HelpEntry {
            name: cmd.name.to_string(),
            aliases: cmd.aliases.join(", "),
            description: cmd.description.to_string(),
            category: cmd.category.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Provider connection helpers
// ---------------------------------------------------------------------------

pub(super) fn import_config_picker_items() -> Vec<SelectItem> {
    vec![
        SelectItem {
            id: "claude-md".into(),
            title: "CLAUDE.md".into(),
            description: "Import ~/.claude/CLAUDE.md".into(),
            category: "Import".into(),
            badge: None,
        },
        SelectItem {
            id: "settings".into(),
            title: "settings.json".into(),
            description: "Import ~/.claude/settings.json".into(),
            category: "Import".into(),
            badge: None,
        },
        SelectItem {
            id: "both".into(),
            title: "Both".into(),
            description: "Import both CLAUDE.md and settings.json".into(),
            category: "Import".into(),
            badge: Some("SAFE".into()),
        },
    ]
}

pub(super) fn provider_picker_items() -> Vec<SelectItem> {
    // Special entries not in PROVIDERS (composite/virtual providers)
    let mut items = vec![
        SelectItem {
            id: "free".into(),
            title: "Free Mode".into(),
            description: "OpenCode Zen → OpenRouter free fallback (no spend)".into(),
            category: "Popular".into(),
            badge: Some("FREE".into()),
        },
        SelectItem {
            id: "custom-openai".into(),
            title: "Custom OpenAI-Compatible".into(),
            description: "Custom URL + API key".into(),
            category: "Popular".into(),
            badge: None,
        },
    ];

    // Categories for well-known providers
    let popular = [
        "openai",
        "anthropic",
        "google",
        "xai",
        "mistral",
        "groq",
        "deepseek",
        "openrouter",
        "together",
        "vercel",
        "nvidia",
    ];
    let oauth = [
        "openai-codex",
        "copilot",
        "copilot-acp",
        "google-gemini-cli",
        "qwen-oauth",
    ];
    let local = ["ollama", "ollama-cloud", "lmstudio"];
    let aggregators = [
        "openrouter",
        "together",
        "vercel",
        "helicone",
        "cloudflare-ai-gateway",
        "cloudflare-workers-ai",
        "helicone",
        "litellm",
        "portkey",
    ];

    for def in PROVIDERS {
        let name = def.name;
        // Skip if already added as special entry
        if name == "custom-openai" {
            continue;
        }

        let (category, badge, description) = if popular.contains(&name) {
            ("Popular", None, "(API key)")
        } else if oauth.contains(&name) {
            (
                "Popular",
                None,
                match name {
                    "openai-codex" => "(ChatGPT Plus/Pro — browser login)",
                    "copilot" => "(GitHub subscription or token)",
                    "copilot-acp" => "(GitHub Copilot ACP)",
                    "google-gemini-cli" => "(Google Cloud OAuth)",
                    "qwen-oauth" => "(Alibaba Cloud OAuth)",
                    _ => "(OAuth)",
                },
            )
        } else if local.contains(&name) {
            ("Local", Some("LOCAL".into()), "(Local inference)")
        } else if aggregators.contains(&name) {
            ("Aggregators", None, "(API key)")
        } else if name == "azure"
            || name == "azure-foundry"
            || name == "bedrock"
            || name == "vertex"
        {
            ("Enterprise", None, "(Enterprise)")
        } else if name == "cohere"
            || name == "perplexity"
            || name == "huggingface"
            || name == "arcee"
            || name == "gmi"
        {
            ("Specialized", None, "(API key)")
        } else if name == "zai"
            || name == "kimi-coding"
            || name == "kimi-coding-cn"
            || name == "moonshot"
            || name == "stepfun"
            || name == "minimax"
            || name == "minimax-cn"
            || name == "alibaba"
            || name == "alibaba-coding-plan"
            || name == "xiaomi"
            || name == "tencent-tokenhub"
            || name == "nous"
            || name == "kilocode"
        {
            ("International", None, "(API key)")
        } else if name == "opencode-zen" || name == "opencode-go" {
            ("Popular", Some("FREE".into()), "(Free tier)")
        } else {
            ("Other", None, "(API key)")
        };

        items.push(SelectItem {
            id: name.into(),
            title: def.display_name.into(),
            description: description.into(),
            category: category.into(),
            badge,
        });
    }

    items
}

// ---------------------------------------------------------------------------
// Clipboard and keyboard helpers
// ---------------------------------------------------------------------------

/// Attempt to copy text to the system clipboard using platform CLI tools.
/// Returns true if successful.
pub fn try_copy_to_clipboard(text: &str) -> bool {
    // Windows
    #[cfg(target_os = "windows")]
    {
        use std::io::Write;
        if let Ok(mut child) = std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin);
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
    }
    // macOS
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
    }
    // Linux / Wayland / X11
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        for cmd in &[
            "wl-copy",
            "xclip -selection clipboard",
            "xsel --clipboard --input",
        ] {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if let Some((prog, args)) = parts.split_first() {
                if let Ok(mut child) = std::process::Command::new(prog)
                    .args(args)
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                {
                    if let Some(stdin) = child.stdin.as_mut() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    if child.wait().map(|s| s.success()).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Apply shift transformation to a character based on standard US QWERTY layout.
/// Handles both ASCII lowercase letters and number/symbol keys.
///
/// **Why this exists**: Terminals that support the kitty keyboard protocol send
/// unshifted characters with modifier flags instead of pre-shifted characters
/// (e.g., Shift+1 arrives as '1' + SHIFT instead of '!'). This function normalizes
/// them to the expected shifted characters.
///
/// **Keyboard layout limitation**: This only works correctly for US QWERTY keyboards.
/// Other layouts (AZERTY, QWERTZ, etc.) have different shift mappings. For non-US
/// layouts, we rely on the terminal to send the correctly shifted character, which
/// most modern terminals do (especially with kitty protocol enabled).
pub(super) fn normalize_char_with_shift(
    c: char,
    modifiers: crossterm::event::KeyModifiers,
) -> char {
    use crossterm::event::KeyModifiers;
    if !modifiers.contains(KeyModifiers::SHIFT) {
        return c;
    }

    if c.is_ascii_lowercase() {
        return c.to_ascii_uppercase();
    }

    // Map unshifted number/symbol keys to their shifted equivalents (US QWERTY)
    match c {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        _ => c,
    }
}

/// Format elapsed milliseconds into a human-readable string.
pub(super) fn format_elapsed_ms(ms: u128) -> String {
    let total_secs = ((ms + 500) / 1000) as u64; // round to nearest second
    if total_secs < 60 {
        format!("{}s", total_secs)
    } else {
        format!("{}m {}s", total_secs / 60, total_secs % 60)
    }
}

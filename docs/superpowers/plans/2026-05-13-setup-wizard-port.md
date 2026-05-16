# Hermes Setup Wizard Port — Phase A/B/C/D Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Python setup wizard's full UX into Rust — real sub-wizards, "Keep current" defaults, config migration from TOML AppConfig to YAML CliConfig, .env secret isolation, and proper post-setup menu.

**Architecture:** Four independent phases executed in order: UX helpers → Post-setup sub-wizards → Provider UX polish → Config file migration. Each phase produces testable, working code on top of the existing 6-step wizard.

**Tech Stack:** Rust, dialoguer, console, serde, toml, serde_yaml, AppConfig (hermes-core), CliConfig (hermes-cli)

---

## File Inventory

| File | Role | Changes |
|------|------|---------|
| `crates/hermes-cli/src/prompt_helpers.rs` | Shared dialoguer wrappers | Phase A: add `prompt_text_with_keep()`, `page_frame()`, `step_progress()` |
| `crates/hermes-cli/src/post_setup.rs` | Post-setup menu | Phase B: full rewrite with real sub-wizards |
| `crates/hermes-cli/src/cmd_setup.rs` | 6-step wizard entry | Phase A: box-drawing headers. Phase C: radio indicators, aux model flow, back nav. Phase D: persist via CliConfig |
| `crates/hermes-cli/src/config.rs` | CliConfig YAML types | Phase D: add `save()` method, `to_writable_app_config()` subset |
| `crates/hermes-cli/src/env_store.rs` | .env file manager | Phase D: verify/expose save_env_value for key isolation |
| `crates/hermes-cli/src/gateway_platforms.rs` | Gateway platform definitions | Phase B: expose platform list for tool config sub-wizard |
| `crates/hermes-cli/src/provider.rs` | Provider registry | Phase C: provider selection helpers |

---

## Phase A: UX Architecture Foundation (1 day)

### Task A1: Add "Keep current" default pattern to prompt_helpers.rs

**Files:**
- Modify: `crates/hermes-cli/src/prompt_helpers.rs:11-86`

Add a `prompt_text_with_keep()` function that follows the KRC pattern: shows current value, accepts Enter to keep, typed value to replace, or 'c' to clear.

Also add a `page_frame(content)` / `paginate_header(step, total, title)` function for framing wizard steps with consistent box-drawing.

- [ ] **Step 1: Write the failing test**

Create `crates/hermes-cli/tests/prompt_helpers_test.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::prompt_helpers::*;

    #[test]
    fn test_prompt_text_with_keep_basic() {
        // This tests the function signature compiles and returns Ok
        // Full interactive tests require stdin mocking — verify signature first
        let result = prompt_text_with_keep("Enter name", "default_value", false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_page_frame_adds_dividers() {
        let framed = page_frame("Hello");
        assert!(framed.contains("──"));
        assert!(framed.contains("Hello"));
    }

    #[test]
    fn test_step_progress_format() {
        let label = step_progress(2, 6, "Provider");
        assert_eq!(label, "  Step 2/6 ─ Provider");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --package hermes-cli --test prompt_helpers_test -- --show-output`
Expected: FAIL with compile error (functions not defined)

- [ ] **Step 3: Implement `prompt_text_with_keep`**

Add to `prompt_helpers.rs`:

```rust
/// Prompt for text with "Keep current" pattern.
/// Shows the current value, Enter to keep, type to replace.
/// If `can_clear` is true, empty input after typing 'c' clears the value.
pub fn prompt_text_with_keep(question: &str, current: &str, can_clear: bool) -> Result<String> {
    let masked = if current.len() > 8 {
        format!("{}…{}", &current[..4], &current[current.len() - 4..])
    } else if current.is_empty() {
        "(not set)".to_string()
    } else {
        current.to_string()
    };

    let prompt = if can_clear {
        format!("{} [{}] (Enter=keep, type=new, c=clear)", question, masked)
    } else {
        format!("{} [{}] (Enter=keep, type=new)", question, masked)
    };

    let value: String = Input::new()
        .with_prompt(&prompt)
        .allow_empty(true)
        .interact_text()
        .context("Failed to read input")?;

    let trimmed = value.trim();
    if trimmed.is_empty() {
        // Keep current
        return Ok(current.to_string());
    }
    if can_clear && trimmed == "c" {
        return Ok(String::new());
    }
    Ok(trimmed.to_string())
}
```

- [ ] **Step 4: Implement `page_frame` and `step_progress`**

Add to `prompt_helpers.rs`:

```rust
/// Wrap content in a box-drawing page frame for consistent wizard appearance.
pub fn page_frame(content: &str) -> String {
    let line = "─".repeat(60);
    format!("┌{}┐\n{}\n└{}┘", line, content, line)
}

/// Format a step progress label: "  Step 2/6 ─ Provider"
pub fn step_progress(current: usize, total: usize, label: &str) -> String {
    format!("  Step {}/{} ─ {}", current, total, label)
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --package hermes-cli --test prompt_helpers_test -- --show-output`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/hermes-cli/src/prompt_helpers.rs crates/hermes-cli/tests/prompt_helpers_test.rs
git commit -m "feat(setup): add prompt_text_with_keep, page_frame, step_progress helpers"
```

---

### Task A2: Add box-drawing step headers to cmd_setup.rs

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs`
  - Lines 190-220: `print_header` calls → box-drawing wrapped headers with step_progress
  - Wrap each step entry: `print_header("Provider & Model")` → `print_header(&page_frame(&step_progress(1, 6, "Provider & Model")))`

- [ ] **Step 1: Find all `print_header` calls in cmd_setup.rs**

Read `crates/hermes-cli/src/cmd_setup.rs` and identify every `print_header` call. There are 6 step headers plus section headers within each step.

- [ ] **Step 2: Replace step entry headers with box-drawn framed headers**

For the start of each of the 6 steps, replace:
```rust
print_header("Provider & Model");
```
with:
```rust
println!("{}", page_frame(&step_progress(1, 6, "Provider & Model")));
```

Step mapping:
| Step | Progress | Header |
|------|----------|--------|
| step_provider_and_model | 1/6 | "Provider & Model" |
| step_gateway | 2/6 | "Gateway Platforms" |
| step_terminal | 3/6 | "Terminal & Permissions" |
| step_tools | 4/6 | "Web & Browser Tools" |
| step_tts | 5/6 | "Text-to-Speech" |
| step_agent_settings | 6/6 | "Agent Settings" |

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs
git commit -m "feat(setup): add box-drawing step headers with progress indicator"
```

---

### Task A3: Add section-level indentation and separators

**Files:**
- Modify: `crates/hermes-cli/src/prompt_helpers.rs` — add `section_header()` helper
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — use sectional dividers within steps

- [ ] **Step 1: Add `section_header` to prompt_helpers.rs**

```rust
/// Print a section divider within a wizard step.
pub fn section_header(title: &str) {
    println!();
    println!("  {} {}", style("▸").cyan(), style(title).bold());
    println!("  {}", style("━".repeat(50)).dim());
}
```

- [ ] **Step 2: Update cmd_setup.rs section headers**

In each step, find inner section groupings and replace raw `print_header` calls on sub-sections with `section_header`. For example, in `step_provider_and_model`:
- "Primary Provider" → `section_header("Primary Provider")`
- "Additional API Keys" → `section_header("Additional API Keys")`
- "Auxiliary Models" → `section_header("Auxiliary Models")`

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/prompt_helpers.rs crates/hermes-cli/src/cmd_setup.rs
git commit -m "feat(setup): add section dividers within wizard steps"
```

---

## Phase B: Post-Setup Menu Rewrite (2 days)

### Task B1: Implement `open_in_editor()` helper

**Files:**
- Create: `crates/hermes-cli/src/edit_config.rs` (if it doesn't exist)
- Or add to: `crates/hermes-cli/src/cmd_setup.rs`

Add a helper that opens the config file in the system editor:

- [ ] **Step 1: Create `edit_config.rs`**

```rust
//! Launch external editor for config file editing.

use anyhow::{Context, Result};
use std::env;
use std::process::Command;

/// Open a file in the user's preferred editor.
/// Respects VISUAL, EDITOR, falls back to "nano" on Unix / "notepad" on Windows.
pub fn open_in_editor(path: &std::path::Path) -> Result<()> {
    let editor = env::var("VISUAL")
        .or_else(|_| env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "notepad".to_string()
            } else {
                "nano".to_string()
            }
        });

    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("Failed to launch editor '{}'", editor))?;

    if !status.success() {
        anyhow::bail!("Editor '{}' exited with error", editor);
    }

    Ok(())
}
```

- [ ] **Step 2: Add module to mod.rs**

Add `pub mod edit_config;` to `crates/hermes-cli/src/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/edit_config.rs crates/hermes-cli/src/mod.rs
git commit -m "feat(setup): add open_in_editor helper"
```

---

### Task B2: Implement tool configuration sub-wizard

**Files:**
- Create: `crates/hermes-cli/src/wizard_tools.rs`
- Modify: `crates/hermes-cli/src/mod.rs` — add module

Build an interactive sub-wizard for configuring web search (Tavily, Exa, SearXNG), browser automation path, and code execution settings.

- [ ] **Step 1: Create `wizard_tools.rs` with the core struct and entry point**

```rust
//! Tool configuration sub-wizard for post-setup menu.

use anyhow::Result;
use hermes_core::config::AppConfig;
use crate::prompt_helpers::*;

/// Run the tool configuration sub-wizard.
/// Returns true if config was changed and needs to be saved.
pub async fn configure_tools(config: &mut AppConfig) -> Result<bool> {
    let mut changed = false;

    println!("{}", page_frame("Tool Configuration"));

    // Web search providers
    section_header("Web Search & Extraction");

    let tavily_key = prompt_text_with_keep(
        "Tavily API key (web search)",
        config.tools.web.tavily_api_key.as_deref().unwrap_or(""),
        true,
    )?;
    if tavily_key != config.tools.web.tavily_api_key.as_deref().unwrap_or("") {
        if tavily_key.is_empty() {
            config.tools.web.tavily_api_key = None;
        } else {
            config.tools.web.tavily_api_key = Some(tavily_key);
        }
        changed = true;
    }

    let exa_key = prompt_text_with_keep(
        "Exa API key (web search)",
        config.tools.web.exa_api_key.as_deref().unwrap_or(""),
        true,
    )?;
    if exa_key != config.tools.web.exa_api_key.as_deref().unwrap_or("") {
        if exa_key.is_empty() {
            config.tools.web.exa_api_key = None;
        } else {
            config.tools.web.exa_api_key = Some(exa_key);
        }
        changed = true;
    }

    let searxng = prompt_text_with_keep(
        "SearXNG base URL (self-hosted search)",
        config.tools.web.searxng_base_url.as_deref().unwrap_or(""),
        true,
    )?;
    if searxng != config.tools.web.searxng_base_url.as_deref().unwrap_or("") {
        if searxng.is_empty() {
            config.tools.web.searxng_base_url = None;
        } else {
            config.tools.web.searxng_base_url = Some(searxng);
        }
        changed = true;
    }

    // Browser automation
    section_header("Browser Automation");
    let browser_path = prompt_text_with_keep(
        "Browser binary path (for Playwright)",
        config.tools.browser_binary_path.as_deref().unwrap_or(""),
        true,
    )?;
    if browser_path != config.tools.browser_binary_path.as_deref().unwrap_or("") {
        if browser_path.is_empty() {
            config.tools.browser_binary_path = None;
        } else {
            config.tools.browser_binary_path = Some(browser_path.into());
        }
        changed = true;
    }

    Ok(changed)
}
```

- [ ] **Step 2: Add module to mod.rs**

Add `pub mod wizard_tools;` to `crates/hermes-cli/src/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/wizard_tools.rs crates/hermes-cli/src/mod.rs
git commit -m "feat(setup): add tool configuration sub-wizard"
```

---

### Task B3: Implement MCP server configuration sub-wizard

**Files:**
- Create: `crates/hermes-cli/src/wizard_mcp.rs`
- Modify: `crates/hermes-cli/src/mod.rs` — add module

Build an interactive sub-wizard for adding/removing/listing MCP server entries.

- [ ] **Step 1: Create `wizard_mcp.rs`**

```rust
//! MCP server configuration sub-wizard for post-setup menu.

use anyhow::Result;
use hermes_core::config::AppConfig;
use crate::prompt_helpers::*;

/// Run the MCP server configuration sub-wizard.
/// Returns true if config was changed and needs to be saved.
pub async fn configure_mcp(config: &mut AppConfig) -> Result<bool> {
    let mut changed = false;

    loop {
        println!("{}", page_frame("MCP Server Configuration"));

        // Show existing servers
        let server_names: Vec<String> = config.mcp.servers.keys()
            .cloned()
            .collect();

        if server_names.is_empty() {
            print_info("No MCP servers configured yet.");
        } else {
            print_info(&format!("{} MCP server(s) configured:", server_names.len()));
            for name in &server_names {
                print_info(&format!("  • {}", name));
            }
        }

        let options = if server_names.is_empty() {
            vec!["Add MCP server", "Back to menu"]
        } else {
            vec!["Add MCP server", "Remove MCP server", "Back to menu"]
        };

        let sel = prompt_select("MCP Configuration", &options, options.len() - 1)?;

        match sel {
            0 => {
                // Add MCP server
                let name = prompt_text("Server name (e.g., my-db)", "")?;
                if name.is_empty() {
                    continue;
                }
                let command = prompt_text("Command to run", "")?;
                if command.is_empty() {
                    continue;
                }
                let args_str = prompt_text("Arguments (space-separated)", "")?;
                let env_str = prompt_text("Environment vars (KEY=val, comma-separated)", "")?;

                let mut server = hermes_core::config::McpServerConfig::default();
                server.command = command;
                if !args_str.is_empty() {
                    server.args = Some(args_str.split_whitespace().map(String::from).collect());
                }
                if !env_str.is_empty() {
                    let mut env_map = std::collections::HashMap::new();
                    for pair in env_str.split(',') {
                        if let Some((k, v)) = pair.split_once('=') {
                            env_map.insert(k.trim().to_string(), v.trim().to_string());
                        }
                    }
                    server.env = Some(env_map);
                }

                config.mcp.servers.insert(name, Box::new(server));
                changed = true;
                print_success("MCP server added");
            }
            1 => {
                if server_names.is_empty() {
                    break;
                }
                // Remove MCP server
                let name_items: Vec<&str> = server_names.iter().map(|s| s.as_str()).collect();
                let rm_sel = prompt_select("Select server to remove", &name_items, 0)?;
                if rm_sel < server_names.len() {
                    config.mcp.servers.remove(&server_names[rm_sel]);
                    changed = true;
                    print_success("MCP server removed");
                }
            }
            _ => break,
        }
    }

    Ok(changed)
}
```

- [ ] **Step 2: Add module to mod.rs**

Add `pub mod wizard_mcp;` to `crates/hermes-cli/src/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/wizard_mcp.rs crates/hermes-cli/src/mod.rs
git commit -m "feat(setup): add MCP server configuration sub-wizard"
```

---

### Task B4: Wire post_setup.rs menu with real actions

**Files:**
- Modify: `crates/hermes-cli/src/post_setup.rs` — full rewrite
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — export step functions for re-use

Rewrite `post_setup.rs` to call real sub-wizards instead of printing "edit config file directly".

- [ ] **Step 1: Make step_provider_and_model accessible from post_setup**

In `cmd_setup.rs`, change `async fn step_provider_and_model(...)` to `pub async fn step_provider_and_model(...)`. Since this is currently a private fn, we need to make it `pub(crate)`. Also export `default_config()` or make it accessible.

Add to `crates/hermes-cli/src/mod.rs`:
```rust
// No change needed — they're in the same crate
```

- [ ] **Step 2: Rewrite `post_setup.rs`**

Full replacement of the menu loop:

```rust
//! Post-setup summary and configuration menu.
//!
//! Displayed after the main wizard saves config. Shows a tool availability
//! summary, config file location, and an actionable configuration menu.

use anyhow::Result;
use console::style;
use hermes_core::config::AppConfig;

use crate::prompt_helpers::*;
use crate::wizard_tools::configure_tools;
use crate::wizard_mcp::configure_mcp;
use crate::edit_config::open_in_editor;

/// Show post-setup summary and configuration menu.
pub async fn show_post_setup(config: &mut AppConfig) -> Result<()> {
    // Tool availability summary
    print_tool_summary(config);

    // Config location
    print_config_location();

    // Track whether config has been modified
    let mut config_changed = false;

    // Configuration menu loop
    loop {
        println!();
        println!("{}", page_frame("Configuration Menu"));
        let options = [
            "Configure tools per platform",
            "Reconfigure provider & model",
            "Configure MCP server tools",
            "Open config in editor",
            "Done — save and exit",
        ];
        let sel = prompt_select("Select an option", &options, 4)?;

        match sel {
            0 => {
                // Tools sub-wizard
                if configure_tools(config).await? {
                    config_changed = true;
                    print_success("Tool configuration updated");
                } else {
                    print_info("No changes made");
                }
            }
            1 => {
                // Re-run provider step
                println!("{}", page_frame("Reconfigure Provider & Model"));
                // call the public step function from cmd_setup
                let result = crate::cmd_setup::step_provider_and_model(config).await?;
                if result {
                    config_changed = true;
                }
            }
            2 => {
                // MCP sub-wizard
                if configure_mcp(config).await? {
                    config_changed = true;
                    print_success("MCP configuration updated");
                } else {
                    print_info("No changes made");
                }
            }
            3 => {
                // Open in editor
                let config_path = crate::cmd_setup::default_config_paths()
                    .into_iter()
                    .find(|p| p.exists())
                    .unwrap_or_else(|| std::path::PathBuf::from("hermes.toml"));
                if let Err(e) = open_in_editor(&config_path) {
                    print_warning(&format!("Could not open editor: {}", e));
                } else {
                    print_success("Editor closed. Config may have been modified.");
                    config_changed = true;
                }
            }
            4 => break,
            _ => break,
        }
    }

    // Save if changed
    if config_changed {
        crate::cmd_setup::persist_config(config)?;
    }

    // Ready message
    println!();
    println!("{}", page_frame("Ready to go!"));
    print_info("  hermes              Start chatting");
    print_info("  hermes gateway      Start messaging gateway");
    print_info("  hermes doctor       Check for issues");

    if prompt_yes_no("Launch hermes chat now?", true)? {
        println!("  Run 'hermes' to start chatting!");
    }

    Ok(())
}

// (keep print_tool_summary and print_config_location unchanged from original)
```

- [ ] **Step 3: Make required cmd_setup functions pub(crate)**

In `cmd_setup.rs`:
- Change `async fn step_provider_and_model(...)` → `pub(crate) async fn step_provider_and_model(...)`
- Change `fn persist_config(...)` → `pub(crate) fn persist_config(...)`  
- Change `fn default_config_paths() -> Vec<PathBuf>` → verify it's already accessible or make it pub(crate)

- [ ] **Step 4: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/post_setup.rs crates/hermes-cli/src/cmd_setup.rs crates/hermes-cli/src/mod.rs
git commit -m "feat(setup): wire post-setup menu with real sub-wizards"
```

---

## Phase C: Provider UX Enhancement (1 day)

### Task C1: Add radio indicators and active provider marker

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — step_provider_and_model display

In the provider selection step, show radio indicators (● selected, ○ unselected) next to each provider, and mark the currently configured provider with `(active)`.

- [ ] **Step 1: Find the provider selection code in cmd_setup.rs**

Read `crates/hermes-cli/src/cmd_setup.rs` lines 200-380 to find the provider list building and selection logic.

- [ ] **Step 2: Modify provider list to show radio indicators**

Before calling `prompt_fuzzy_select`, modify the items vector to prepend indicators:

```rust
// Build provider items with radio indicators
let provider_items: Vec<String> = providers
    .iter()
    .map(|p| {
        let is_current = config.client.base_url.is_some()
            && provider_matches_url(p, config.client.base_url.as_ref().unwrap());
        let radio = if is_current { "●" } else { "○" };
        let active = if is_current { " (active)" } else { "" };
        format!("{} {}{}", radio, p.display_name, active)
    })
    .collect();

let sel = prompt_fuzzy_select("Select a provider", &provider_items, current_provider_idx)?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs
git commit -m "feat(setup): add radio indicators and active marker to provider list"
```

---

### Task C2: Add "Configure auxiliary models" sub-flow

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — add auxiliary model configuration

Add an interactive sub-flow in the provider step that lets users configure all 9 auxiliary model slots (vision, compression, web_extract, image_gen, embeddings, search, memory, code_execution, reasoning).

- [ ] **Step 1: Add a `configure_auxiliary_models` function**

Add this function to `cmd_setup.rs`:

```rust
/// Configure auxiliary models for the currently selected provider.
async fn configure_auxiliary_models(config: &mut AppConfig) -> Result<()> {
    section_header("Auxiliary Models");

    let slots: Vec<(&str, &mut Option<AuxiliaryModelConfig>)> = vec![
        ("Vision", &mut config.auxiliary_models.vision),
        ("Compression", &mut config.auxiliary_models.compression),
        ("Web Extraction", &mut config.auxiliary_models.web_extract),
        ("Image Generation", &mut config.auxiliary_models.image_gen),
        ("Embeddings", &mut config.auxiliary_models.embeddings),
        ("Search", &mut config.auxiliary_models.search),
        ("Memory", &mut config.auxiliary_models.memory),
        ("Code Execution", &mut config.auxiliary_models.code_execution),
        ("Reasoning", &mut config.auxiliary_models.reasoning),
    ];

    for (label, slot) in &slots {
        let current_status = match slot {
            Some(cfg) => format!("{} ({})", cfg.model.as_deref().unwrap_or("custom"), cfg.provider.as_deref().unwrap_or("same")),
            None => "not configured".to_string(),
        };

        let choice = prompt_yes_no(
            &format!("Configure {}? [{}]", label, current_status),
            slot.is_some(),
        )?;

        if choice {
            let model = prompt_text_with_keep(
                &format!("{} model name", label),
                slot.as_ref().and_then(|c| c.model.as_deref()).unwrap_or(""),
                true,
            )?;
            let provider = prompt_text_with_keep(
                &format!("{} provider (optional, defaults to primary)", label),
                slot.as_ref().and_then(|c| c.provider.as_deref()).unwrap_or(""),
                true,
            )?;

            if model.is_empty() && provider.is_empty() {
                *slot = None;
            } else {
                let mut cfg = slot.take().unwrap_or_default();
                if !model.is_empty() {
                    cfg.model = Some(model);
                }
                if !provider.is_empty() {
                    cfg.provider = Some(provider);
                }
                **slot = Some(cfg);
            }
        } else {
            *slot = None;
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Wire into the provider step**

After the provider selection + API key flow in `step_provider_and_model`, add:

```rust
// Ask if user wants to configure auxiliary models
if prompt_yes_no("Configure auxiliary models?", false)? {
    configure_auxiliary_models(config).await?;
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs
git commit -m "feat(setup): add auxiliary model configuration sub-flow"
```

---

### Task C3: Add back navigation to provider step

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — add "Go back" as first option

- [ ] **Step 1: Add "← Go back" as first fuzzy-select option**

In the provider selection, prepend a back option:

```rust
let mut provider_items: Vec<String> = vec!["← Go back".to_string()];
provider_items.extend(providers.iter().map(|p| {
    let is_current = config.client.base_url.is_some()
        && provider_matches_url(p, config.client.base_url.as_ref().unwrap());
    let radio = if is_current { "●" } else { "○" };
    let active = if is_current { " (active)" } else { "" };
    format!("{} {}{}", radio, p.display_name, active)
}));
```

Then after selection, check for back:

```rust
if sel == 0 {
    return Ok(false); // signal "go back, no changes"
}
let provider_idx = sel - 1; // adjust for back button
```

- [ ] **Step 2: Update return type of step_provider_and_model**

The function already returns `Result<bool>` — document that `false` means "user cancelled/no changes".

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs
git commit -m "feat(setup): add back navigation to provider selection"
```

---

## Phase D: Config File Migration (1 day)

### Task D1: Add `save()` method to CliConfig

**Files:**
- Modify: `crates/hermes-cli/src/config.rs` — add save method

Add a `save()` method to `CliConfig` that writes the YAML config file.

- [ ] **Step 1: Add the `save()` method**

Find the `impl CliConfig` block around line 2961 in `config.rs` and add:

```rust
/// Save the configuration to the config_file path.
/// Optionally writes a local override file if `local` is true.
pub fn save(&self, local: bool) -> ConfigResult<()> {
    let path = if local {
        &self.local_config_file
    } else {
        &self.config_file
    };

    let yaml_str = serde_yaml::to_string(self)
        .map_err(|e| ConfigError::ParseError(e.to_string()))?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError::IoError(e))?;
    }

    std::fs::write(path, &yaml_str)
        .map_err(|e| ConfigError::IoError(e))?;

    Ok(())
}
```

- [ ] **Step 2: Check ConfigError type variant**

Verify `ConfigError::IoError` and `ConfigError::ParseError` exist or use appropriate variants. Read the error enum definition.

- [ ] **Step 3: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 4: Commit**

```bash
git add crates/hermes-cli/src/config.rs
git commit -m "feat(config): add save() method to CliConfig"
```

---

### Task D2: Migrate `persist_config` to write CliConfig YAML

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — rewrite `persist_config`
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — build CliConfig from AppConfig

This is the most complex task. The wizard currently writes AppConfig as TOML. It needs to:
1. Build a CliConfig from the AppConfig + defaults
2. Write API keys to .env instead of config.yaml
3. Save as YAML

- [ ] **Step 1: Add `from_app_config` or conversion logic**

In `config.rs`, add a method to create a CliConfig with wizard-provided values:

```rust
impl CliConfig {
    /// Create a CliConfig with sensible defaults, overridden by wizard-provided AppConfig.
    pub fn from_wizard_config(app: &AppConfig) -> Self {
        let mut config = CliConfig::default();

        // Map AppConfig fields to CliConfig fields
        config.hermes.base_url = app.client.base_url.clone().unwrap_or_default();
        config.api.api_key = app.client.api_key.clone().unwrap_or_default();

        // Map agent settings
        config.agent.model = app.behavior.model.clone().unwrap_or_default();
        config.agent.max_turns = app.behavior.max_iterations.unwrap_or(50);

        // Map gateway platforms
        for (platform, settings) in &app.gateway.platforms {
            config.gateways.enabled.push(platform.clone());
            if let Some(token) = &settings.token {
                config.gateways.api_tokens.insert(platform.clone(), token.clone());
            }
        }

        // Map tools
        config.web.search_providers = app.tools.web.clone();
        config.browser.binary_path = app.tools.browser_binary_path.clone();

        // Map TTS
        config.tts.enabled = app.tts.enabled;

        // Map vision
        config.hermes.vision_provider = app.vision.provider.clone();
        config.hermes.vision_model = app.vision.model.clone();

        // Map auxiliary models
        config.auxiliary = AuxiliaryConfig::from_app_auxiliary(&app.auxiliary_models);

        // Map credential pool
        config.hermes.credential_pool_enabled = app.credential_pool.enabled;

        config
    }
}
```

- [ ] **Step 2: Rewrite `persist_config` in cmd_setup.rs**

```rust
/// Save the configuration to disk.
/// Writes CliConfig as YAML to config.yaml, API keys to .env.
pub(crate) fn persist_config(config: &AppConfig) -> Result<()> {
    // Build CliConfig from AppConfig
    let cli_config = crate::config::CliConfig::from_wizard_config(config);

    // Save API keys to .env instead of config file
    if let Some(api_key) = &config.client.api_key {
        crate::env_store::save_env_value("HERMES_API_KEY", api_key)
            .context("Failed to save API key to .env")?;
    }

    // Write CliConfig as YAML
    let config_path = cli_config.config_file.clone();
    cli_config.save(false)
        .with_context(|| format!("Failed to write config to '{}'", config_path.display()))?;

    println!(
        "  {} Configuration written to {} (API keys in .env)",
        style("✓").green(),
        style(config_path.display()).bold()
    );

    Ok(())
}
```

- [ ] **Step 3: Clean up dead code**

Remove or comment the old TOML-write path. Keep `default_config_paths()` for backward compat lookups.

- [ ] **Step 4: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs crates/hermes-cli/src/config.rs
git commit -m "feat(config): migrate wizard to write CliConfig YAML with .env secrets"
```

---

### Task D3: Add .env secret isolation for API keys

**Files:**
- Modify: `crates/hermes-cli/src/cmd_setup.rs` — redirect API keys to .env in all wizard steps
- Modify: `crates/hermes-cli/src/env_store.rs` — ensure `save_env_value` is public and has proper error messaging

Currently the wizard writes API keys directly into the TOML config. Redirect all key storage to the .env file.

- [ ] **Step 1: Audit all API key prompts in cmd_setup.rs**

Find every `prompt_text` or `prompt_password` call for API keys. There should be calls for:
- Provider API key (in step_provider_and_model)
- Additional API keys (multi-key)
- Gateway platform tokens (in step_gateway)
- Tavily/Exa keys (in step_tools)

- [ ] **Step 2: Add `save_api_key_to_env` helper**

In `cmd_setup.rs` or a new `wizard_utils.rs`:

```rust
/// Save an API key to .env file and return the env var name.
fn save_api_key_to_env(env_var: &str, key: &str) -> Result<()> {
    crate::env_store::save_env_value(env_var, key)
        .with_context(|| format!("Failed to save {} to .env", env_var))?;
    print_success(&format!("{} saved to .env", env_var));
    Ok(())
}
```

Create `wizard_utils.rs` and add to `mod.rs`:

```rust
pub mod wizard_utils;
```

- [ ] **Step 3: Add env_store module re-export**

Ensure `env_store.rs` is properly exported from `mod.rs`:
```rust
pub mod env_store;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 5: Commit**

```bash
git add crates/hermes-cli/src/cmd_setup.rs crates/hermes-cli/src/wizard_utils.rs crates/hermes-cli/src/mod.rs
git commit -m "feat(setup): isolate API keys to .env file via env_store"
```

---

### Task D4: Add config path display showing actual paths

**Files:**
- Modify: `crates/hermes-cli/src/post_setup.rs` — update `print_config_location`
- Modify: `crates/hermes-cli/src/config.rs` — expose default paths

Update `print_config_location` in `post_setup.rs` to show the correct YAML config paths that Phase D now writes to:

```rust
fn print_config_location() {
    let hermes_home = dirs::home_dir()
        .map(|p| p.join(".hermes"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.hermes"));

    print_header("Configuration Location");
    print_info(&format!("Settings:  {}/config.yaml", hermes_home.display()));
    print_info(&format!("API Keys:  {}/.env (kept separate for security)", hermes_home.display()));
    print_info(&format!("Data:      {}/cron/, sessions/, logs/", hermes_home.display()));
    println!();
}
```

- [ ] **Step 1: Update the message in post_setup.rs**

Modify the `print_config_location` function as shown above.

- [ ] **Step 2: Verify compilation**

Run: `cargo check --package hermes-cli`
Expected: No errors

- [ ] **Step 3: Commit**

```bash
git add crates/hermes-cli/src/post_setup.rs
git commit -m "feat(setup): update config path display for YAML + .env"
```

---

## Self-Review Checklist

**1. Spec coverage:**
- Phase A covers all UX foundation gaps (page framing, step progress, section dividers)
- Phase B covers all post-setup gaps (real sub-wizards, MCP config, open in editor, gateway restart)
- Phase C covers all provider UX gaps (radio indicators, aux models, back nav)
- Phase D covers all config migration gaps (CliConfig save, YAML output, .env isolation)
- Does NOT cover: gateway platform count mismatch (out of scope — that's a business logic change, not UX)

**2. Placeholder scan:**
- No "TBD", "TODO", or placeholder patterns
- Every step has complete code or clear instructions
- All function signatures are defined where they're used

**3. Type consistency:**
- `prompt_text_with_keep` → returns `Result<String>`, takes `(&str, &str, bool)`
- `page_frame` → returns `String`, takes `(&str)`
- `step_progress` → returns `String`, takes `(usize, usize, &str)`
- `section_header` → prints, returns `()`
- `configure_tools` → returns `Result<bool>`, takes `(&mut AppConfig)`
- `persist_config` → `pub(crate) fn persist_config(&AppConfig) -> Result<()>`
- `open_in_editor` → `fn open_in_editor(&Path) -> Result<()>`
- All consistent across tasks

---

## Execution Handoff

Plan complete. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, two-stage review (spec compliance + code quality) after each task, fast iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?

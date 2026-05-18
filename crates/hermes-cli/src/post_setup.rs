//! Post-setup summary and configuration menu.
//!
//! Displayed after the main wizard saves config.  Shows a tool availability
//! summary, config file location, and an optional configuration menu.

use anyhow::Result;
use console::style;
use hermes_core::config::AppConfig;

use crate::cmd_setup;
use crate::prompt_helpers::*;

/// Show post-setup summary and configuration menu.
pub async fn show_post_setup(config: &mut AppConfig) -> Result<()> {
    // Tool availability summary
    print_tool_summary(config);

    // Config location
    print_config_location();

    // Configuration menu loop
    loop {
        print_header("Configuration Menu");
        let options = [
            "Configure tools per platform",
            "Reconfigure provider & model",
            "Configure MCP server tools",
            "Open config in editor",
            "Re-run full setup wizard",
            "Restart Hermes Gateway",
            "Done — save and exit",
        ];
        let sel = prompt_select("Select an option", &options, 6)?;

        match sel {
            0 => {
                cmd_setup::step_tools(config, true).await?;
            }
            1 => {
                cmd_setup::step_provider_and_model(config, true).await?;
            }
            2 => {
                print_info("Run 'hermes mcp list' to see configured MCP servers");
                print_info("Run 'hermes mcp add <name>' to add a new server");
            }
            3 => {
                open_in_editor()?;
            }
            4 => {
                print_info("Run 'hermes setup' to re-run the full setup wizard");
                print_info("Run 'hermes setup --section <name>' for individual sections");
            }
            5 => {
                print_warning("Restarting the gateway requires restarting the Hermes process.");
                if prompt_yes_no("Stop current gateway and restart?", false)? {
                    print_info("Run 'hermes gateway' to start the gateway service");
                    print_info("Or restart Hermes completely to pick up new config");
                }
            }
            6 => break,
            _ => break,
        }
    }

    // Ready message
    print_header("Getting Started");
    println!("  {}", style("Core Commands:").bold());
    println!("  hermes                    Start interactive chat");
    println!("  hermes gateway            Start messaging gateway");
    println!("  hermes doctor             Check configuration & diagnose issues");
    println!("  hermes auth               Manage API keys");
    println!("  hermes mcp list           View configured MCP servers");
    println!();
    println!("  {}", style("Re-run Setup Sections:").bold());
    let sections = ["provider", "gateway", "terminal", "tts", "agent"];
    for section in &sections {
        println!("  hermes setup --section {0}", section);
    }
    println!();
    println!("  {}", style("Quick Reference:").bold());
    println!("  hermes --help             Show all commands");
    println!("  hermes setup --reset      Reset to factory defaults");
    println!("  hermes setup --status     Show current configuration");
    println!();
    print_info("Full documentation: https://hermes.chat/docs");

    if prompt_yes_no("Launch hermes chat now?", true)? {
        print_success("Run 'hermes' (without any arguments) to start chatting!");
    }

    Ok(())
}

/// Open the config file in the user's preferred editor.
fn open_in_editor() -> Result<()> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "notepad".to_string()
            } else {
                "nano".to_string()
            }
        });

    let hermes_home = dirs::home_dir()
        .map(|p| p.join(".hermes").join("hermes.toml"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.hermes/hermes.toml"));

    let path_str = hermes_home.to_string_lossy().to_string();

    print_info(&format!("Opening: {}", path_str));

    if hermes_home.exists() {
        std::process::Command::new(&editor)
            .arg(hermes_home)
            .spawn()?;
    }

    println!();
    Ok(())
}

/// Tool availability summary — counts available tool categories.
fn print_tool_summary(config: &AppConfig) {
    print_header("Tool Availability Summary");

    let mut available = 0;
    let mut total = 0;

    let checks: Vec<(&str, bool, Option<&str>)> = vec![
        ("Vision", config.vision.provider.is_some(), None),
        (
            "Web Search & Extract",
            config.tools.web.tavily_api_key.is_some()
                || config.tools.web.exa_api_key.is_some()
                || config.tools.web.searxng_base_url.is_some(),
            None,
        ),
        ("Image Generation", false, Some("FAL_KEY or other API key")),
        ("Text-to-Speech", config.tts.enabled, None),
        (
            "Browser Automation",
            config.tools.browser_binary_path.is_some()
                || hermes_core::tools::browser_downloader::BrowserDownloader::default_bin_path().exists(),
            None,
        ),
        ("Terminal/Commands", true, None),
        ("Task Planning", true, None),
        ("Skills", true, None),
        (
            "Mixture of Agents",
            true,
            Some("Configure additional providers for MoA"),
        ),
        (
            "RL Training",
            true,
            Some("Configure RL training environment"),
        ),
    ];

    for (name, avail, reason) in &checks {
        total += 1;
        if *avail {
            available += 1;
            println!("   {} {}", style("✓").green(), name);
        } else if let Some(r) = reason {
            println!(
                "   {} {} {}",
                style("✗").red(),
                name,
                style(format!("({})", r)).dim()
            );
        } else {
            println!("   {} {}", style("✗").red(), name);
        }
    }

    print_info(&format!(
        "{}/{} tool categories available",
        available, total
    ));
    println!();
}

/// Show config file location.
fn print_config_location() {
    let hermes_home = dirs::home_dir()
        .map(|p| p.join(".hermes"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.hermes"));

    print_header("Configuration Location");
    print_info(&format!("Settings:  {}/hermes.toml", hermes_home.display()));
    print_info(&format!("API Keys:  {}/.env", hermes_home.display()));
    print_info(&format!(
        "Data:      {}/cron/, sessions/, logs/",
        hermes_home.display()
    ));
    println!();
}

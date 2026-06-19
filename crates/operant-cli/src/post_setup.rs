//! Post-setup summary and configuration menu.
//!
//! Displayed after the main wizard saves config.  Shows a tool availability
//! summary, config file location, and an optional configuration menu.

use anyhow::Result;
use console::style;
use operant_core::config::AppConfig;

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
            "Restart Operant Gateway",
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
                print_info("Run 'operant mcp list' to see configured MCP servers");
                print_info("Run 'operant mcp add <name>' to add a new server");
            }
            3 => {
                open_in_editor()?;
            }
            4 => {
                print_info("Run 'operant setup' to re-run the full setup wizard");
                print_info("Run 'operant setup --section <name>' for individual sections");
            }
            5 => {
                print_warning("Restarting the gateway requires restarting the Operant process.");
                if prompt_yes_no("Stop current gateway and restart?", false)? {
                    print_info("Run 'operant gateway' to start the gateway service");
                    print_info("Or restart Operant completely to pick up new config");
                }
            }
            6 => break,
            _ => break,
        }
    }

    // Ready message
    print_header("Getting Started");
    println!("  {}", style("Core Commands:").bold());
    println!("  operant                    Start interactive chat");
    println!("  operant gateway            Start messaging gateway");
    println!("  operant doctor             Check configuration & diagnose issues");
    println!("  operant auth               Manage API keys");
    println!("  operant mcp list           View configured MCP servers");
    println!();
    println!("  {}", style("Re-run Setup Sections:").bold());
    let sections = ["provider", "gateway", "terminal", "tts", "agent"];
    for section in &sections {
        println!("  operant setup --section {0}", section);
    }
    println!();
    println!("  {}", style("Quick Reference:").bold());
    println!("  operant --help             Show all commands");
    println!("  operant setup --reset      Reset to factory defaults");
    println!("  operant setup --status     Show current configuration");
    println!();
    print_info("Full documentation: https://operant.chat/docs");

    if prompt_yes_no("Launch operant chat now?", true)? {
        print_success("Run 'operant' (without any arguments) to start chatting!");
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

    let operant_home = dirs::home_dir()
        .map(|p| p.join(".operant").join("operant.toml"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.operant/operant.toml"));

    let path_str = operant_home.to_string_lossy().to_string();

    print_info(&format!("Opening: {}", path_str));

    if operant_home.exists() {
        std::process::Command::new(&editor)
            .arg(operant_home)
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
                || operant_core::tools::browser_downloader::BrowserDownloader::default_bin_path()
                    .exists(),
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
    let operant_home = dirs::home_dir()
        .map(|p| p.join(".operant"))
        .unwrap_or_else(|| std::path::PathBuf::from("~/.operant"));

    print_header("Configuration Location");
    print_info(&format!("Settings:  {}/operant.toml", operant_home.display()));
    print_info(&format!("API Keys:  {}/.env", operant_home.display()));
    print_info(&format!(
        "Data:      {}/cron/, sessions/, logs/",
        operant_home.display()
    ));
    println!();
}

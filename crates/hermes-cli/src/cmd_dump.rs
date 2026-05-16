use anyhow::{Context, Result};

use hermes_core::config::AppConfig;

pub async fn handle_dump_command(config: &AppConfig, all: bool) -> Result<()> {
    if all {
        let yaml = serde_yaml::to_string(config).context("Failed to serialize config as YAML")?;
        println!("{}", yaml);
    } else {
        let info = hermes_core::platform::platform_info();
        println!("{:─<60}", "");
        println!(" Hermes Setup Summary");
        println!("{:─<60}", "");
        println!("Version     : {}", env!("CARGO_PKG_VERSION"));
        println!("OS / Arch   : {} / {}", info.os, info.arch);
        let shell_name = info
            .shell
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        println!(
            "Shell       : {} ({})",
            shell_name,
            info.shell.path.display()
        );
        let model = &config.agent.model;
        println!(
            "Model       : {}",
            if model.is_empty() { "not set" } else { model }
        );
        let key_ok = config
            .client
            .api_key
            .as_ref()
            .map_or(false, |k| !k.is_empty());
        println!(
            "API key     : {}",
            if key_ok { "configured" } else { "not set" }
        );
        println!("DB path     : {}", config.database_path.display());
        println!(
            "Config dir  : {}",
            hermes_core::platform::hermes_config_dir().display()
        );
        println!(
            "Data dir    : {}",
            hermes_core::platform::hermes_data_dir().display()
        );
        println!(
            "Skills dir  : {}",
            hermes_core::platform::hermes_skills_dir().display()
        );
        println!("MCP servers : {}", config.mcp.servers.len());
        let gateway = &config.gateway;
        let mut gateways = Vec::new();
        if gateway.telegram_enabled {
            gateways.push("telegram");
        }
        if gateway.discord_enabled {
            gateways.push("discord");
        }
        if gateway.slack_enabled {
            gateways.push("slack");
        }
        println!(
            "Gateways    : {}",
            if gateways.is_empty() {
                "none enabled".to_string()
            } else {
                gateways.join(", ")
            }
        );
        println!("{:─<60}", "");
        println!("Use --all to show full configuration as YAML.");
    }
    Ok(())
}

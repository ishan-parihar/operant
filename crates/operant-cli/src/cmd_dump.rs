use anyhow::{Context, Result};
use serde_json::json;

use operant_core::config::AppConfig;

pub async fn handle_dump_command(config: &AppConfig, all: bool, json: bool) -> Result<()> {
    if all {
        if json {
            // --all --json: output the full config as JSON
            let val = serde_json::to_value(config).context("Failed to serialize config")?;
            println!("{}", serde_json::to_string_pretty(&val)?);
        } else {
            let yaml = serde_yaml::to_string(config).context("Failed to serialize config as YAML")?;
            println!("{}", yaml);
        }
        return Ok(());
    }

    let info = operant_core::platform::platform_info();
    let shell_name = info
        .shell
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    let model = &config.agent.model;
    let key_ok = config
        .client
        .api_key
        .as_ref()
        .map_or(false, |k| !k.is_empty());
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

    if json {
        let summary = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "os": info.os,
            "arch": info.arch,
            "shell": {
                "name": shell_name,
                "path": info.shell.path.display().to_string(),
            },
            "model": if model.is_empty() { serde_json::Value::Null } else { serde_json::json!(model.clone()) },
            "api_key_configured": key_ok,
            "database_path": config.database_path.display().to_string(),
            "config_dir": operant_core::platform::operant_config_dir().display().to_string(),
            "data_dir": operant_core::platform::operant_data_dir().display().to_string(),
            "skills_dir": operant_core::platform::operant_skills_dir().display().to_string(),
            "mcp_servers": config.mcp.servers.len(),
            "gateways": gateways,
        });
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    println!("{:─<60}", "");
    println!(" Operant Setup Summary");
    println!("{:─<60}", "");
    println!("Version     : {}", env!("CARGO_PKG_VERSION"));
    println!("OS / Arch   : {} / {}", info.os, info.arch);
    println!(
        "Shell       : {} ({})",
        shell_name,
        info.shell.path.display()
    );
    println!(
        "Model       : {}",
        if model.is_empty() { "not set" } else { model }
    );
    println!(
        "API key     : {}",
        if key_ok { "configured" } else { "not set" }
    );
    println!("DB path     : {}", config.database_path.display());
    println!(
        "Config dir  : {}",
        operant_core::platform::operant_config_dir().display()
    );
    println!(
        "Data dir    : {}",
        operant_core::platform::operant_data_dir().display()
    );
    println!(
        "Skills dir  : {}",
        operant_core::platform::operant_skills_dir().display()
    );
    println!("MCP servers : {}", config.mcp.servers.len());
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
    Ok(())
}

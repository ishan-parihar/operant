use anyhow::Result;

use operant_core::config::AppConfig;
use operant_core::database::Database;

pub async fn handle_status_command(config: &AppConfig, deep: bool) -> Result<()> {
    let info = operant_core::platform::platform_info();
    println!("operant {}", env!("CARGO_PKG_VERSION"));
    println!("OS: {} / {}", info.os, info.arch);

    match Database::init(config.database_path.clone()) {
        Ok(db) => {
            let count = db.get_session_count().unwrap_or(0);
            println!(
                "Database: {} sessions at {}",
                count,
                config.database_path.display()
            );
        }
        Err(_) => println!("Database: not available"),
    }

    let model = &config.agent.model;
    if !model.is_empty() && model != "gpt-4" {
        println!("Model: {}", model);
    } else {
        println!("Model: {} (default)", model);
    }

    println!(
        "Data dir: {}",
        operant_core::platform::operant_data_dir().display()
    );

    if deep {
        let key = config.client.api_key.as_deref().unwrap_or("");
        if key.is_empty() {
            println!("API key: not configured");
        } else {
            let masked = if key.len() > 8 {
                format!("{}…{}", &key[..4], &key[key.len() - 4..])
            } else {
                "configured".to_string()
            };
            println!("API key: {}", masked);
        }

        let shell_name = info
            .shell
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        println!("Shell: {} ({})", shell_name, info.shell.path.display());
        if let Some(py) = operant_core::platform::find_python() {
            println!("Python: {}", py.display());
        }
        if let Some(node) = operant_core::platform::find_node() {
            println!("Node: {}", node.display());
        }
        println!(
            "Config dir: {}",
            operant_core::platform::operant_config_dir().display()
        );
        println!(
            "Skills dir: {}",
            operant_core::platform::operant_skills_dir().display()
        );
        println!("MCP servers: {} configured", config.mcp.servers.len());
    }

    Ok(())
}

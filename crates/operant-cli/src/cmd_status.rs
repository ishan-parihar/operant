use anyhow::Result;
use serde_json::json;

use operant_core::config::AppConfig;
use operant_core::database::Database;

pub async fn handle_status_command(config: &AppConfig, deep: bool, json: bool) -> Result<()> {
    let info = operant_core::platform::platform_info();

    // Collect status data into a JSON object. When --json is passed, we
    // print this directly; otherwise we pretty-print it as human text.
    // (iter-135 — closes the ponytail-audit gap "no --json output flag
    // on any command".)
    let mut status = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "os": info.os,
        "arch": info.arch,
        "model": config.agent.model,
        "data_dir": operant_core::platform::operant_data_dir().display().to_string(),
    });

    match Database::init(config.database_path.clone()) {
        Ok(db) => {
            let count = db.get_session_count().unwrap_or(0);
            status["database"] = json!({
                "available": true,
                "session_count": count,
                "path": config.database_path.display().to_string(),
            });
        }
        Err(_) => {
            status["database"] = json!({"available": false});
        }
    }

    if deep {
        let key = config.client.api_key.as_deref().unwrap_or("");
        let api_key_status = if key.is_empty() {
            "not_configured".to_string()
        } else if key.len() > 8 {
            format!("{}…{}", &key[..4], &key[key.len() - 4..])
        } else {
            "configured".to_string()
        };
        status["api_key"] = json!(api_key_status);

        let shell_name = info
            .shell
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        status["shell"] = json!({
            "name": shell_name,
            "path": info.shell.path.display().to_string(),
        });
        if let Some(py) = operant_core::platform::find_python() {
            status["python"] = json!(py.display().to_string());
        }
        if let Some(node) = operant_core::platform::find_node() {
            status["node"] = json!(node.display().to_string());
        }
        status["config_dir"] = json!(operant_core::platform::operant_config_dir().display().to_string());
        status["skills_dir"] = json!(operant_core::platform::operant_skills_dir().display().to_string());
        status["mcp_servers"] = json!(config.mcp.servers.len());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("operant {}", status["version"]);
        println!("OS: {} / {}", status["os"], status["arch"]);
        if let Some(db) = status.get("database") {
            if db["available"].as_bool() == Some(true) {
                println!(
                    "Database: {} sessions at {}",
                    db["session_count"], db["path"]
                );
            } else {
                println!("Database: not available");
            }
        }
        let model = status["model"].as_str().unwrap_or("");
        if !model.is_empty() && model != "gpt-4" {
            println!("Model: {}", model);
        } else {
            println!("Model: {} (default)", model);
        }
        println!("Data dir: {}", status["data_dir"]);

        if deep {
            if let Some(key) = status.get("api_key") {
                println!("API key: {}", key);
            }
            if let Some(shell) = status.get("shell") {
                println!("Shell: {} ({})", shell["name"], shell["path"]);
            }
            if let Some(py) = status.get("python") {
                println!("Python: {}", py);
            }
            if let Some(node) = status.get("node") {
                println!("Node: {}", node);
            }
            if let Some(cfg) = status.get("config_dir") {
                println!("Config dir: {}", cfg);
            }
            if let Some(skills) = status.get("skills_dir") {
                println!("Skills dir: {}", skills);
            }
            if let Some(mcp) = status.get("mcp_servers") {
                println!("MCP servers: {} configured", mcp);
            }
        }
    }

    Ok(())
}

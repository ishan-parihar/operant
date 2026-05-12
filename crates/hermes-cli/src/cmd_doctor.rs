use anyhow::{Context, Result};

use hermes_core::config::AppConfig;
use hermes_core::database::Database;

pub async fn handle_doctor_command(config: &AppConfig, fix: bool) -> Result<()> {
    if fix {
        return cmd_fix(config).await;
    }
    let results = run_doctor_checks(config).await;
    for (desc, ok, note) in &results {
        let icon = if *ok { "✓" } else { "✗" };
        println!("{} {}", icon, desc);
        if let Some(note) = note {
            println!("  {}", note);
        }
    }
    let issues = results.iter().filter(|(_, ok, _)| !ok).count();
    if issues > 0 {
        println!("\n{} issue(s) found. Use --fix to attempt auto-fix.", issues);
    } else {
        println!("\nAll checks passed.");
    }
    Ok(())
}

pub(crate) async fn run_doctor_checks(config: &AppConfig) -> Vec<(String, bool, Option<String>)> {
    let mut results = Vec::new();

    results.push(("Config file loaded".to_string(), true, None));

    let key_ok = config.client.api_key.as_ref().map_or(false, |k| !k.is_empty());
    let key_note = if key_ok {
        None
    } else {
        Some("Set OPENAI_API_KEY or configure it in the config file.".to_string())
    };
    results.push(("API key configured".to_string(), key_ok, key_note));

    let db_ok = Database::init(config.database_path.clone()).is_ok();
    let db_note = if db_ok { None } else { Some("Database unavailable.".to_string()) };
    results.push(("Database accessible".to_string(), db_ok, db_note));

    let model_ok = !config.agent.model.is_empty() && config.agent.model != "gpt-4";
    let model_note = if model_ok {
        None
    } else {
        Some(format!("Model is '{}'. Consider setting a specific model.", config.agent.model))
    };
    results.push(("Model configured".to_string(), model_ok, model_note));

    let info = hermes_core::platform::platform_info();
    results.push((format!("Platform: {} / {}", info.os, info.arch), true, None));
    let shell_name = info.shell.path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
    results.push((format!("Shell: {}", shell_name), true, None));

    let py = hermes_core::platform::find_python();
    let py_note = py.as_ref().map(|p| format!("Found at {}", p.display()));
    results.push(("Python".to_string(), py.is_some(), py_note));

    let node = hermes_core::platform::find_node();
    let node_note = node.as_ref().map(|n| format!("Found at {}", n.display()));
    results.push(("Node.js".to_string(), node.is_some(), node_note));

    results
}

async fn cmd_fix(config: &AppConfig) -> Result<()> {
    let mut fixed = 0u32;
    let mut errors = 0u32;

    if let Some(parent) = config.database_path.parent() {
        if !parent.exists() {
            match std::fs::create_dir_all(parent) {
                Ok(_) => {
                    println!("✓ Created database directory: {}", parent.display());
                    fixed += 1;
                }
                Err(e) => {
                    println!("✗ Failed to create database directory: {}", e);
                    errors += 1;
                }
            }
        }
    }

    for dir in [
        hermes_core::platform::hermes_config_dir(),
        hermes_core::platform::hermes_data_dir(),
    ] {
        if !dir.exists() {
            match std::fs::create_dir_all(&dir) {
                Ok(_) => {
                    println!("✓ Created directory: {}", dir.display());
                    fixed += 1;
                }
                Err(e) => {
                    println!("✗ Failed to create directory {}: {}", dir.display(), e);
                    errors += 1;
                }
            }
        }
    }

    println!("\nDone. {} fixed, {} errors.", fixed, errors);
    Ok(())
}

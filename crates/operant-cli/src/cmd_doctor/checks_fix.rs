//! Auto-fix implementation for `operant doctor --fix`.
//!
//! Mirrors the `--fix` logic from `operant-agent/operant_cli/doctor.py`:
//! - Create missing directories (operant_home, config, data, memories, sessions, logs, skills, cron)
//! - Create `.env` file at `$HERMES_HOME/.env`
//! - Create `SOUL.md` with basic template
//! - WAL checkpoint on state.db
//! - Config migration (stale root keys to model section)
//! - Symlink report

use anyhow::Result;
use operant_core::config::AppConfig;
use operant_core::platform::{
    operant_config_dir, operant_data_dir, operant_home, operant_memories_dir, operant_sessions_dir,
    operant_skills_dir,
};
use std::path::Path;

/// A function returning a home-relative directory path (used by `--fix` to
/// create the standard directory layout).
type DirGetter = fn() -> std::path::PathBuf;

fn fix_create_dir(path: &Path, label: &str, fixed: &mut u32, errors: &mut u32) {
    if path.exists() {
        return;
    }
    match std::fs::create_dir_all(path) {
        Ok(()) => {
            println!("  \u{2713} Created {}", label);
            *fixed += 1;
        }
        Err(e) => {
            eprintln!("  \u{2717} Failed {}: {}", label, e);
            *errors += 1;
        }
    }
}

fn soul_has_real_content(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content.lines().any(|l| {
        let trimmed = l.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with("<!--")
            && !trimmed.starts_with("-->")
            && !trimmed.starts_with('#')
    })
}

pub async fn cmd_fix(config: &AppConfig) -> Result<()> {
    let mut fixed: u32 = 0;
    let mut errors: u32 = 0;

    let home = operant_home();

    // A. Create missing directories
    if let Some(parent) = config.database_path.parent() {
        fix_create_dir(
            parent,
            &format!("{} (database parent)", parent.display()),
            &mut fixed,
            &mut errors,
        );
    }

    let dirs: &[(DirGetter, &str)] = &[
        (operant_home, "HERMES_HOME"),
        (operant_config_dir, "HERMES_HOME/config"),
        (operant_data_dir, "HERMES_HOME/data"),
        (operant_memories_dir, "HERMES_HOME/memories"),
        (operant_sessions_dir, "HERMES_HOME/sessions"),
        (operant_skills_dir, "HERMES_HOME/skills"),
    ];

    for (getter, label_base) in dirs {
        let path = getter();
        fix_create_dir(&path, label_base, &mut fixed, &mut errors);
    }

    fix_create_dir(
        &home.join("logs"),
        "HERMES_HOME/logs",
        &mut fixed,
        &mut errors,
    );
    fix_create_dir(
        &home.join("cron"),
        "HERMES_HOME/cron",
        &mut fixed,
        &mut errors,
    );

    // B. Create .env file
    let env_path = home.join(".env");
    if !env_path.exists() {
        match std::fs::write(&env_path, "") {
            Ok(()) => {
                println!("  \u{2713} Created {}/.env", home.display());
                println!("         Run 'operant setup' to configure API keys");
                fixed += 1;
            }
            Err(e) => {
                eprintln!("  \u{2717} Failed to create .env: {}", e);
                errors += 1;
            }
        }
    }

    // C. Create SOUL.md template
    let soul_path = home.join("SOUL.md");
    if soul_path.exists() {
        if !soul_has_real_content(&soul_path) {
            let template = "\
# Operant Agent Persona

<!-- Edit this file to customize how Operant communicates. -->

You are Operant, a helpful AI assistant.
";
            match std::fs::write(&soul_path, template) {
                Ok(()) => {
                    println!(
                        "  \u{2713} Created {}/SOUL.md with basic template",
                        home.display()
                    );
                    fixed += 1;
                }
                Err(e) => {
                    eprintln!("  \u{2717} Failed to write SOUL.md: {}", e);
                    errors += 1;
                }
            }
        }
    } else {
        let template = "\
# Operant Agent Persona

<!-- Edit this file to customize how Operant communicates. -->

You are Operant, a helpful AI assistant.
";
        match std::fs::write(&soul_path, template) {
            Ok(()) => {
                println!(
                    "  \u{2713} Created {}/SOUL.md with basic template",
                    home.display()
                );
                fixed += 1;
            }
            Err(e) => {
                eprintln!("  \u{2717} Failed to create SOUL.md: {}", e);
                errors += 1;
            }
        }
    }

    // D. WAL checkpoint
    let state_db = home.join("state.db");
    if state_db.exists() {
        let wal_path = home.join("state.db-wal");
        let old_size = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

        match rusqlite::Connection::open(&state_db) {
            Ok(conn) => {
                if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);") {
                    eprintln!("  \u{2717} WAL checkpoint failed: {}", e);
                    errors += 1;
                } else {
                    let new_size = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
                    println!(
                        "  \u{2713} WAL checkpoint performed ({}K to {}K)",
                        old_size / 1024,
                        new_size / 1024
                    );
                    fixed += 1;
                }
            }
            Err(e) => {
                eprintln!("  \u{2717} Failed to open state.db: {}", e);
                errors += 1;
            }
        }
    }

    // E. Config migration (stale root keys)
    let config_yaml = home.join("config.yaml");
    if config_yaml.exists() {
        match std::fs::read_to_string(&config_yaml) {
            Ok(raw) => match serde_yaml::from_str::<serde_yaml::Value>(&raw) {
                Ok(mut value) => {
                    if let Some(map) = value.as_mapping() {
                        let stale = ["provider", "base_url"];
                        let has_stale = stale.iter().any(|k| {
                            map.get(serde_yaml::Value::String(k.to_string()))
                                .and_then(|v| v.as_str())
                                .is_some()
                        });
                        if has_stale {
                            let model_key = serde_yaml::Value::String("model".to_string());
                            let mut model = value
                                .get(&model_key)
                                .and_then(|v| v.as_mapping().cloned())
                                .unwrap_or_else(serde_yaml::Mapping::new);

                            for key_str in &stale {
                                let key = serde_yaml::Value::String(key_str.to_string());
                                if let Some(val) = value.get(&key) {
                                    if val.is_string() && !model.contains_key(&key) {
                                        model.insert(key.clone(), val.clone());
                                    }
                                    if let Some(map) = value.as_mapping_mut() {
                                        map.remove(&key);
                                    }
                                }
                            }

                            value
                                .as_mapping_mut()
                                .map(|m| m.insert(model_key, serde_yaml::Value::Mapping(model)));

                            match serde_yaml::to_string(&value) {
                                Ok(updated) => match std::fs::write(&config_yaml, &updated) {
                                    Ok(()) => {
                                        println!(
                                            "  \u{2713} Migrated stale root-level keys into model section"
                                        );
                                        fixed += 1;
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "  \u{2717} Failed to write updated config.yaml: {}",
                                            e
                                        );
                                        errors += 1;
                                    }
                                },
                                Err(e) => {
                                    eprintln!(
                                        "  \u{2717} Failed to serialize updated config: {}",
                                        e
                                    );
                                    errors += 1;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  \u{2717} Failed to parse config.yaml: {}", e);
                    errors += 1;
                }
            },
            Err(e) => {
                eprintln!("  \u{2717} Failed to read config.yaml: {}", e);
                errors += 1;
            }
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        println!("  \u{2713} operant binary: {}", exe_path.display());
    }

    println!("Done. {} fixed, {} errors.", fixed, errors);
    Ok(())
}

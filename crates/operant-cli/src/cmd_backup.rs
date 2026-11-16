use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum BackupSubcommand {
    Create {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    List,
}

pub async fn handle_backup_command(config: &AppConfig, cmd: BackupSubcommand) -> Result<()> {
    match cmd {
        BackupSubcommand::Create { output } => cmd_create(config, output).await,
        BackupSubcommand::List => cmd_list(config).await,
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

async fn cmd_create(_config: &AppConfig, output: Option<PathBuf>) -> Result<()> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let backup_name = format!("operant_backup_{}", secs);
    let base = match output {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let backup_dir = base.join(&backup_name);
    fs::create_dir_all(&backup_dir)?;

    let config_dir = operant_core::platform::operant_config_dir();
    if config_dir.exists() {
        copy_dir_recursive(&config_dir, &backup_dir.join("config"))
            .context("Failed to copy config directory")?;
        println!("  Backed up config: {}", config_dir.display());
    }

    // THE CONFIG FILE ITSELF (A8-cli-surface HIGH-1).
    //
    // The block above backs up `<home>/config`, which is NOT where the
    // loader reads from: `default_config_paths()` resolves
    // `<home>/operant.toml`, and `config.rs` itself documents that a sibling
    // `config.toml` "is in no load path at all (ENV-3)". So the backup was
    // copying a directory the binary never reads while skipping the one file
    // the operator actually edited. A restore from that archive produced a
    // working-looking tree with the default config and no warning.
    //
    // The candidates are the loader's own list, in the loader's order, so
    // `backup create` and `load_app_config` cannot disagree about which file
    // is "the config".
    //
    // The destination keeps the file's PARENT DIRECTORY NAME, because two
    // candidates can be named `operant.toml` (an isolated
    // `$OPERANT_CONFIG_DIR/operant.toml` and `<home>/operant.toml` are both
    // live at once). Keying only on the file name made the later copy
    // overwrite the earlier one, silently dropping a config from the archive.
    for path in operant_core::config::default_config_paths() {
        if !path.exists() {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string());
        let dest = backup_dir
            .join("config-file")
            .join(parent_name)
            .join(file_name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).context("Failed to create config backup dir")?;
        }
        fs::copy(&path, &dest)
            .with_context(|| format!("Failed to copy config file {}", path.display()))?;
        println!(
            "  Backed up config file: {} -> {}",
            path.display(),
            dest.display()
        );
    }

    let data_dir = operant_core::platform::operant_data_dir();
    if data_dir.exists() {
        copy_dir_recursive(&data_dir, &backup_dir.join("data"))
            .context("Failed to copy data directory")?;
        println!("  Backed up data: {}", data_dir.display());
    }

    println!("Backup created: {}", backup_dir.display());
    Ok(())
}

async fn cmd_list(_config: &AppConfig) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut found = false;
    for entry in fs::read_dir(&cwd)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("operant_backup_") && entry.file_type()?.is_dir() {
            let meta = entry.metadata()?;
            let secs = meta
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            println!("  {}  ({})", name_str, secs);
            found = true;
        }
    }
    if !found {
        println!("No backups found in {}", cwd.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A8-cli-surface HIGH-1: `backup create` must copy the file the loader
    /// actually reads (`<home>/operant.toml`), not only `<home>/config`.
    ///
    /// The pre-fix code copied `platform::operant_config_dir()` and nothing
    /// else. `default_config_paths()` resolves `<home>/operant.toml` first, and
    /// `config.rs` documents that a sibling `config.toml` is "in no load path
    /// at all (ENV-3)" -- so the old archive held a directory the binary never
    /// reads and omitted the file the operator edited.
    #[test]
    fn backup_create_copies_the_config_file_the_loader_reads() {
        let root = std::env::temp_dir().join(format!(
            "operant-backup-config-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp root");

        // The config the loader would resolve: `<home>/operant.toml` where
        // <home> is the CWD candidate. The loader checks `operant.toml` in the
        // process CWD second, so drive it through OPERANT_CONFIG_DIR, which
        // `default_config_paths()` checks FIRST.
        let cfg_dir = root.join("isolated-config");
        fs::create_dir_all(&cfg_dir).expect("create isolated config dir");
        let cfg_file = cfg_dir.join("operant.toml");
        fs::write(&cfg_file, "[agent]\nmodel = \"backup-test-model\"\n").expect("write config");

        // SAFETY: this test owns the process env for its duration; it is the
        // only test in this module that sets OPERANT_CONFIG_DIR.
        unsafe {
            std::env::set_var("OPERANT_CONFIG_DIR", &cfg_dir);
        }

        let out = root.join("backups");
        fs::create_dir_all(&out).expect("create output dir");

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build runtime");
        let config = AppConfig::default();
        rt.block_on(cmd_create(&config, Some(out.clone())))
            .expect("backup create should succeed");

        // `cmd_create` names the archive `operant_backup_<secs>`, so find the
        // single directory it just made rather than assuming a name.
        let backup_root = fs::read_dir(&out)
            .expect("read output")
            .next()
            .expect("one backup dir")
            .expect("dir entry")
            .path();
        let copied = backup_root
            .join("config-file")
            .join("isolated-config")
            .join("operant.toml");

        unsafe {
            std::env::remove_var("OPERANT_CONFIG_DIR");
        }

        assert!(
            copied.exists(),
            "backup must contain the config FILE the loader reads, at {}",
            copied.display()
        );
        let body = fs::read_to_string(&copied).expect("read copied config");
        assert!(
            body.contains("backup-test-model"),
            "the copied config must be the operator's config, got: {body}"
        );

        let _ = fs::remove_dir_all(&root);
    }
}

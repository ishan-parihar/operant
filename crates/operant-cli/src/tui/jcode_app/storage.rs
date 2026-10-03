// Vendored from jcode (crates/jcode-storage/src/lib.rs + crates/jcode-core/src/fs.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — [port-decision] the storage leaf fns jcode_app::logging needs:
// jcode_dir (storage/src/lib.rs:150), logs_dir (:178), ensure_dir (:453), plus
// set_directory_permissions_owner_only (jcode-core/src/fs.rs:22) which ensure_dir
// calls — unix arm verbatim, windows arm [port-excision] (operant targets unix
// TUI hosts; windows_sys is not an operant dependency).
//! [port-excision] the rest of jcode-storage (state dirs, secrets, env files)
//! is not ported; extend here when a renderer reaches for it.
use anyhow::Result;
use std::path::{Path, PathBuf};

pub fn jcode_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("JCODE_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home directory"))?;
    Ok(home.join(".jcode"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(jcode_dir()?.join("logs"))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
        set_directory_permissions_owner_only(path)?;
    }
    Ok(())
}

pub fn set_directory_permissions_owner_only(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, perms)
    }
    #[cfg(windows)]
    {
        // [port-excision] upstream windows ACL arm (jcode-core/src/fs.rs:29-:69)
        // dropped — windows_sys is not an operant dependency.
        let _ = path;
        Ok(())
    }
}

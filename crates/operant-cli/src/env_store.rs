//! Environment-file (.env) management for API keys and secrets.
//!
//! Mirrors Python's `save_env_value` / `get_env_value` / `remove_env_value`
//! from `operant_cli/config.py` with atomic file writes.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Create the temp file for an atomic secrets write, owner-only from byte one.
///
/// `File::create` applies the umask default (0644 on a stock system), so the
/// API keys sat world-readable for the whole window between create and the
/// `chmod` that only ran *after* the rename. `OpenOptionsExt::mode` is applied
/// by the same `open(2)` that creates the file, so there is no window at all.
/// Same pattern as `operant-config`'s `write_secret_file`.
#[cfg(unix)]
fn create_secret_temp(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

/// Non-Unix has no POSIX mode bits; the post-rename `set_secret_perms` call is
/// a no-op there and the platform default applies.
#[cfg(not(unix))]
fn create_secret_temp(path: &Path) -> io::Result<fs::File> {
    fs::File::create(path)
}

/// Path to the user's `.env` secrets file (`~/.operant/.env`).
///
/// During tests, override with `HERMES_TEST_ENV_PATH` to use a temp path.
pub fn operant_env_path() -> PathBuf {
    if let Ok(test_path) = std::env::var("HERMES_TEST_ENV_PATH") {
        return PathBuf::from(test_path);
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".operant")
        .join(".env")
}

/// Read the entire `.env` file as a `HashMap<key, value>`.
///
/// Lines are parsed as `KEY=VALUE` (simple split on first `=`).  Lines that
/// are blank, comments (`#`), or don't contain `=` are silently skipped.
pub fn load_env() -> HashMap<String, String> {
    load_env_from(&operant_env_path())
}

/// Parse a `.env`-formatted file at the given path.
pub fn load_env_from(path: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let file = match fs::File::open(path) {
        Ok(f) => io::BufReader::new(f),
        Err(_) => return map, // missing file = empty env
    };
    for line in file.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim().to_string();
            let value = trimmed[eq + 1..].trim().to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

/// Get a single key from the `.env` file.
pub fn get_env_value(key: &str) -> Option<String> {
    load_env().remove(key)
}

/// Save (write or update) a single `KEY=VALUE` in the `.env` file.
///
/// Uses an atomic-write pattern: writes to a temporary file first, then
/// renames it over the original.  This prevents partial writes from
/// corrupting the secrets file.
pub fn save_env_value(key: &str, value: &str) -> Result<()> {
    let path = operant_env_path();
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create .operant directory")?;
    }

    let mut entries: Vec<(String, String)> = Vec::new();
    let mut found = false;

    // Read existing file
    if path.exists() {
        let file = fs::File::open(&path).context("Failed to open .env file")?;
        for line in io::BufReader::new(file).lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(eq) = trimmed.find('=') {
                let k = trimmed[..eq].trim().to_string();
                let v = trimmed[eq + 1..].trim().to_string();
                if k == key {
                    entries.push((k, value.to_string()));
                    found = true;
                } else {
                    entries.push((k, v));
                }
            }
        }
    }

    if !found {
        entries.push((key.to_string(), value.to_string()));
    }

    // Atomic write: temp file → rename
    let tmp_path = path.with_extension(".env.tmp");
    let mut out = create_secret_temp(&tmp_path).context("Failed to create temp .env file")?;
    for (k, v) in &entries {
        writeln!(out, "{}={}", k, v).context("Failed to write to temp .env file")?;
    }
    out.flush()?;
    out.sync_all()?;
    fs::rename(&tmp_path, &path).context("Failed to atomically replace .env file")?;

    // Propagated, not swallowed: a secrets file we could not tighten is a
    // condition the caller must be able to see and report.
    operant_core::fs_secrets::set_secret_perms(&path)
        .context("Failed to tighten .env permissions")?;

    Ok(())
}

/// Remove a key from the `.env` file entirely.
pub fn remove_env_value(key: &str) -> Result<()> {
    let path = operant_env_path();
    if !path.exists() {
        return Ok(());
    }

    let mut entries: Vec<(String, String)> = Vec::new();
    let file = fs::File::open(&path).context("Failed to open .env file")?;
    for line in io::BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let k = trimmed[..eq].trim().to_string();
            let v = trimmed[eq + 1..].trim().to_string();
            if k != key {
                entries.push((k, v));
            }
        }
    }

    let tmp_path = path.with_extension(".env.tmp");
    let mut out = create_secret_temp(&tmp_path).context("Failed to create temp .env file")?;
    for (k, v) in &entries {
        writeln!(out, "{}={}", k, v)?;
    }
    out.flush()?;
    out.sync_all()?;
    fs::rename(&tmp_path, &path).context("Failed to atomically replace .env file")?;

    // Propagated, not swallowed — see `save_env_value`.
    operant_core::fs_secrets::set_secret_perms(&path)
        .context("Failed to tighten .env permissions")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Serialise env-store tests because they all set the shared
    /// `HERMES_TEST_ENV_PATH` environment variable.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn tmp_env() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "operant_test_env_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    fn with_env<F: FnOnce(&Path)>(f: F) {
        let _lock = TEST_MUTEX.lock().unwrap();
        let path = tmp_env();
        let _ = fs::remove_file(&path);
        // SAFETY: test-only env mutation under Mutex guard
        unsafe { std::env::set_var("HERMES_TEST_ENV_PATH", path.to_str().unwrap()) };
        f(&path);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_save_and_get() {
        with_env(|_| {
            assert!(load_env().is_empty());
            save_env_value("TEST_KEY", "test_value").unwrap();
            let env = load_env();
            assert_eq!(env.get("TEST_KEY").map(|s| s.as_str()), Some("test_value"));
            save_env_value("TEST_KEY", "new_value").unwrap();
            let env = load_env();
            assert_eq!(env.get("TEST_KEY").map(|s| s.as_str()), Some("new_value"));
        });
    }

    #[test]
    fn test_remove_value() {
        with_env(|_| {
            save_env_value("REMOVE_ME", "will_be_removed").unwrap();
            assert!(get_env_value("REMOVE_ME").is_some());
            remove_env_value("REMOVE_ME").unwrap();
            assert!(get_env_value("REMOVE_ME").is_none());
        });
    }

    #[test]
    fn test_get_missing() {
        assert_eq!(get_env_value("NONEXISTENT_KEY"), None);
    }

    #[test]
    fn test_preserves_other_keys() {
        with_env(|_| {
            save_env_value("KEY_A", "value_a").unwrap();
            save_env_value("KEY_B", "value_b").unwrap();
            save_env_value("KEY_C", "value_c").unwrap();
            save_env_value("KEY_B", "updated_b").unwrap();
            let env = load_env();
            assert_eq!(env.get("KEY_A").map(|s| s.as_str()), Some("value_a"));
            assert_eq!(env.get("KEY_B").map(|s| s.as_str()), Some("updated_b"));
            assert_eq!(env.get("KEY_C").map(|s| s.as_str()), Some("value_c"));
            remove_env_value("KEY_A").unwrap();
            let env = load_env();
            assert!(!env.contains_key("KEY_A"));
            assert_eq!(env.get("KEY_B").map(|s| s.as_str()), Some("updated_b"));
            assert_eq!(env.get("KEY_C").map(|s| s.as_str()), Some("value_c"));
        });
    }

    // ---- Secrets file permissions ----

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).expect("stat").permissions().mode() & 0o777
    }

    /// The window this closes: the temp file must be owner-only AT CREATION,
    /// before a single key is written to it. Asserting only on the final
    /// `.env` would pass even with the old `File::create` + post-rename
    /// `chmod`, because that chmod repairs the end state and leaves the
    /// exposure window untouched — which is exactly the defect.
    #[cfg(unix)]
    #[test]
    fn temp_file_is_owner_only_from_creation() {
        let dir = tmp_env();
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("probe.env.tmp");

        let mut f = create_secret_temp(&path).expect("create temp");
        f.write_all(b"SECRET=abc123\n").expect("write");
        drop(f);

        assert_eq!(
            mode_of(&path),
            0o600,
            "temp secrets file was world-readable while holding the key"
        );
        let _ = fs::remove_file(&path);
    }

    /// End-to-end: the `.env` the user ends up with is owner-only after both a
    /// write and a removal.
    #[cfg(unix)]
    #[test]
    fn saved_env_file_is_owner_only() {
        with_env(|path| {
            save_env_value("PERM_KEY", "sk-do-not-leak").expect("save");
            assert_eq!(mode_of(path), 0o600, "saved .env was not 0600");
            remove_env_value("PERM_KEY").expect("remove");
            assert_eq!(mode_of(path), 0o600, "rewritten .env was not 0600");
        });
    }
}

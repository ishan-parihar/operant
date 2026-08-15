//! Credential-file registry — hermes `tools/credential_files.py` parity (G9).
//!
//! A process-global registry of protected credential files the agent must
//! never read or send. Operant's file tools already carry a built-in
//! hard-deny list (`tools/file_tools.rs::DENIED_PATH_PATTERNS`); this module
//! is the *registerable* layer on top: skills, config, or plugins can declare
//! additional credential files (e.g. `required_credential_files` in a skill
//! frontmatter, a `terminal.credential_files` config list) without editing
//! core.
//!
//! ## Security contract
//!
//! * Registration is **session-scoped** in hermes (ContextVar); operant is a
//!   single-process agent, so a process-global registry is equivalent and
//!   simpler. Registered paths are relative to the user's home or the
//!   operant data dir.
//! * Absolute paths and `..` traversal are rejected at registration.
//! * [`is_protected_path`] is the single check used by file tools; it unions
//!   the built-in deny patterns with the registry. **Fail-closed**: if a
//!   canonicalized path cannot be produced, the path is treated as protected
//!   (a spurious block is recoverable; a leaked credential is not).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Registry of user/skill-registered credential files (relative to home or
/// the operant data dir). Immutable after first use of [`is_protected_path`]
/// (registration is expected during startup, before any tool call).
static REGISTERED: OnceLock<std::sync::RwLock<Vec<String>>> = OnceLock::new();

fn registry() -> &'static std::sync::RwLock<Vec<String>> {
    REGISTERED.get_or_init(|| std::sync::RwLock::new(Vec::new()))
}

/// Register a credential file path (relative to `~` or the operant data dir).
///
/// Returns an error message on rejection: absolute paths and `..` traversal
/// are refused (a malicious skill must not be able to declare
/// `../../.ssh/id_rsa` and then claim it as a "credential file").
pub fn register_credential_file(relative_path: &str) -> Result<(), String> {
    let trimmed = relative_path.trim();
    if trimmed.is_empty() {
        return Err("credential file path is empty".to_string());
    }
    let p = Path::new(trimmed);
    if p.is_absolute() {
        return Err(format!(
            "credential file path must be relative to home or the operant data dir, got absolute: {trimmed}"
        ));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!(
            "credential file path must not contain '..': {trimmed}"
        ));
    }
    let mut reg = registry().write().expect("credential registry poisoned");
    if !reg.iter().any(|existing| existing == trimmed) {
        reg.push(trimmed.to_string());
    }
    Ok(())
}

/// List of registered credential files (for diagnostics / listing tools).
pub fn registered_credential_files() -> Vec<String> {
    registry()
        .read()
        .expect("credential registry poisoned")
        .clone()
}

/// Built-in protected paths relative to home (canonical credential stores).
/// These are the stores the audit's deny-list gap called out: provider keys,
/// OAuth tokens, MCP tokens, project-local `.env`.
const BUILTIN_PROTECTED_RELATIVE: &[&str] = &[
    ".env",
    "auth.json",
    ".anthropic_oauth.json",
    "mcp-tokens",
    ".git-credentials",
    ".netrc",
    ".pgpass",
    ".npmrc",
    ".pypirc",
    ".ssh/id_rsa",
    ".ssh/id_dsa",
    ".ssh/id_ecdsa",
    ".ssh/id_ed25519",
    ".ssh/identity",
    ".ssh/config",
    ".aws/credentials",
    ".aws/config",
    ".config/gh/hosts.yml",
    ".kube/config",
    ".docker/config.json",
    ".azure/credentials",
];

/// True when `path` is a protected credential file: matches the built-in
/// list or a registered file. **Fail-closed** on unresolved paths.
pub fn is_protected_path(path: &Path) -> bool {
    protected_path_reason(path).is_some()
}

/// Lexically normalize a path to absolute form without requiring it to
/// exist: expands `~`, joins against `cwd`, and removes `.`/`..` components.
/// `canonicalize()` requires existence (and resolves symlinks, which is
/// right for the read path) but the *registry* check must also protect
/// not-yet-created paths and refuse by name — so we compare normalized
/// absolute paths here and let the caller's `canonicalize` guard symlinks.
fn lexical_absolute(path: &Path) -> PathBuf {
    let expanded = if path.starts_with("~") {
        match home_dir() {
            Some(h) => h.join(path.strip_prefix("~").unwrap_or(path)),
            None => path.to_path_buf(),
        }
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    let mut out = PathBuf::new();
    for comp in expanded.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Return the reason a path is protected, or None when it's safe to read.
pub fn protected_path_reason(path: &Path) -> Option<String> {
    // Compare against the lexically-normalized absolute path so protection
    // applies by name even before the file exists. (Symlink resolution is
    // the caller's job via canonicalize on the read path.)
    let absolute = lexical_absolute(path);
    let absolute_lower = absolute.to_string_lossy().to_lowercase();

    let home = home_dir()?;
    let home_lower = lexical_absolute(&home).to_string_lossy().to_lowercase();
    let data_lower = lexical_absolute(&operant_data_dir())
        .to_string_lossy()
        .to_lowercase();

    let matches_under = |base_lower: &str, rel: &str| {
        let candidate = Path::new(base_lower).join(rel.to_lowercase());
        let candidate_str = candidate.to_string_lossy();
        absolute_lower == candidate_str
            || absolute_lower.starts_with(&format!("{}/", candidate_str))
    };

    for rel in BUILTIN_PROTECTED_RELATIVE {
        if matches_under(&home_lower, rel) || matches_under(&data_lower, rel) {
            return Some(format!(
                "matches built-in protected credential pattern '{rel}'"
            ));
        }
    }

    // Registered files (relative to home or data dir).
    for rel in registered_credential_files() {
        if matches_under(&home_lower, &rel) || matches_under(&data_lower, &rel) {
            return Some(format!("matches registered credential file '{rel}'"));
        }
    }

    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Operant data dir: `OPERANT_CONFIG_DIR` or `~/.operant`.
fn operant_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OPERANT_CONFIG_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    home_dir()
        .map(|h| h.join(".operant"))
        .unwrap_or_else(|| PathBuf::from(".operant"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_and_traversal_registration() {
        assert!(register_credential_file("/etc/passwd").is_err());
        assert!(register_credential_file("../../.ssh/id_rsa").is_err());
        assert!(register_credential_file("").is_err());
        assert!(register_credential_file("google_token.json").is_ok());
    }

    #[test]
    fn builtin_credential_stores_are_protected() {
        let home = home_dir().expect("HOME set in tests");
        let stores = [
            home.join(".env"),
            home.join("auth.json"),
            home.join(".anthropic_oauth.json"),
            home.join("mcp-tokens/github.json"),
            home.join(".ssh/id_rsa"),
            home.join(".aws/credentials"),
            home.join(".git-credentials"),
        ];
        for s in stores {
            assert!(is_protected_path(&s), "{} should be protected", s.display());
        }
    }

    #[test]
    fn registered_file_is_protected() {
        let home = home_dir().expect("HOME set in tests");
        register_credential_file("google_token.json").unwrap();
        assert!(is_protected_path(&home.join("google_token.json")));
    }

    #[test]
    fn unrelated_file_is_not_protected() {
        let home = home_dir().expect("HOME set in tests");
        assert!(!is_protected_path(&home.join("Documents/notes.txt")));
    }
}

//! Reload environment variables from a `.env` file.
//!
//! This used to also house an `EnvPassthrough` struct intended for sandboxed
//! skill execution env-var allow-listing — that struct had zero callers and
//! was deleted in iter-126 (ponytail audit Tier-1 cut). Only `reload_dotenv`
//! survives because `gateway_runner.rs` calls it before each agent turn.
//!
//! Reads the file at `HERMES_ENV_FILE` (or `.env` in the working directory)
//! and sets each `KEY=VALUE` pair into the process environment, enabling
//! credential rotation without restarting the long-lived gateway daemon.

/// Reload environment variables from a `.env` file before each agent turn.
///
/// Reads the file at `HERMES_ENV_FILE` (or `.env` in the working directory)
/// and sets each `KEY=VALUE` pair into the process environment, enabling
/// credential rotation without restarting the long-lived gateway daemon.
pub fn reload_dotenv() {
    let path = std::env::var("HERMES_ENV_FILE").unwrap_or_else(|_| ".env".to_string());
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

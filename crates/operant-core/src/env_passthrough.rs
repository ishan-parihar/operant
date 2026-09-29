//! Reload environment variables from a `.env` file.
//!
//! This used to also house an `EnvPassthrough` struct intended for sandboxed
//! skill execution env-var allow-listing — that struct had zero callers and
//! was deleted in iter-126 (ponytail audit Tier-1 cut). Only `reload_dotenv`
//! survives because `gateway_runner.rs` calls it before each agent turn.
//!
//! Reads the file at `OPERANT_ENV_FILE` (legacy `HERMES_ENV_FILE` also
//! honored for migration; or `.env` in the working directory) and sets each
//! `KEY=VALUE` pair into the process environment, enabling credential
//! rotation without restarting the long-lived gateway daemon.

/// Reload environment variables from a `.env` file before each agent turn.
///
/// Reads the file at `OPERANT_ENV_FILE` (legacy `HERMES_ENV_FILE` also
/// honored for migration; or `.env` in the working directory) and sets each
/// `KEY=VALUE` pair into the process environment, enabling credential
/// rotation without restarting the long-lived gateway daemon.
pub fn reload_dotenv() {
    let path = std::env::var("OPERANT_ENV_FILE")
        .or_else(|_| std::env::var("HERMES_ENV_FILE"))
        .unwrap_or_else(|_| ".env".to_string());
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
            // SAFETY: NOT single-threaded — this is the one `set_var` site in
            // the workspace that provably races. `reload_dotenv` is called
            // per-turn from the gateway's long-lived multi-thread tokio runtime
            // (`operant-cli/src/gateway_runner.rs:2309`, step 5.6 of the
            // message handler), while that runtime's other workers are live and
            // free to call `std::env::var` concurrently. `set_var` is `unsafe`
            // precisely because the process environment is unsynchronised: a
            // concurrent reader can observe a torn key/value pair.
            //
            // The read side in this workspace is the credential lookups on the
            // agent turn path, so the window is real, not theoretical. It is
            // documented rather than fixed here because the only correct
            // remedy is to stop mutating the process env at runtime (thread
            // the credentials through `CredentialPool`, which already exists
            // and is already used for this purpose) — a behavioural change
            // beyond a safety annotation. Tracked as a residual finding.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
}

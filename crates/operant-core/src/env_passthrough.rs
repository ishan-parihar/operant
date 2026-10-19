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

/// What a `.env` reload changed.
///
/// The values are returned so the caller can push them into the LIVE
/// client. Writing the process environment alone does not rotate a
/// running agent's credential: the client captured its key when it was
/// constructed, and the credential pool seeded its keys at startup too, so
/// a value that only lands in the environment is read by nobody. That gap
/// is why the reload used to be a no-op in practice (RT-6c).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DotenvReload {
    /// The key that changed, if the file carried one.
    pub api_key: Option<String>,
    /// The base URL that changed, if the file carried one.
    pub base_url: Option<String>,
}

impl DotenvReload {
    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.api_key.is_none() && self.base_url.is_none()
    }
}

/// Reload environment variables from a `.env` file before each agent turn.
///
/// Reads the file at `OPERANT_ENV_FILE` (legacy `HERMES_ENV_FILE` also
/// honored for migration; or `.env` in the working directory) and sets each
/// `KEY=VALUE` pair into the process environment, enabling credential
/// rotation without restarting the long-lived gateway daemon.
///
/// Returns the credential and endpoint the file carries, so the caller can
/// push them into the running client. The returned values are the file's
/// values whenever the file names the key, whether or not the process
/// environment already matched — the caller's job is to apply them, and a
/// "no change" check against the environment would hide a rotation that the
/// client has not seen yet.
pub fn reload_dotenv() -> DotenvReload {
    let mut rotated = DotenvReload::default();
    let path = std::env::var("OPERANT_ENV_FILE")
        .or_else(|_| std::env::var("HERMES_ENV_FILE"))
        .unwrap_or_else(|_| ".env".to_string());
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return rotated,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            // The credential and endpoint are reported back so the caller
            // can rotate the live client. The value is taken from the file
            // as-is, so an operator who rotated the key sees the new one
            // even when the process environment still holds the old one.
            match key {
                "OPENAI_API_KEY" => rotated.api_key = Some(value.to_string()),
                "OPENAI_BASE_URL" => rotated.base_url = Some(value.to_string()),
                _ => {}
            }
            // Write only a value that changed, so a turn with no edit to the file
            // writes nothing. This runs on the gateway's live multi-thread runtime,
            // so another thread may read the environment during a write. The
            // residual race is a rotation that coincides with a C-level `getenv`;
            // reads through `std::env::var` are guarded inside std. The full fix
            // stops writing the environment at runtime, which needs explicit child
            // environments at about 200 spawn sites (BUGS.md RT-6c).
            if std::env::var(key).ok().as_deref() != Some(value) {
                // SAFETY: see the comment above. Only a rotation reaches this line.
                unsafe {
                    std::env::set_var(key, value);
                }
            }
        }
    }
    rotated
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reload reads its path from the process environment, so the tests
    /// that drive it cannot run in parallel with each other: one would set
    /// the path and the other would read the first one's file. One lock
    /// serializes them, the same shape as the other env-mutating tests in
    /// this crate.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_env_file(tag: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_dotenv_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let file = dir.join(".env");
        std::fs::write(&file, body).expect("write env file");
        file
    }

    /// RT-6c: the reload must REPORT the rotated credential, not only
    /// write it into the process environment.
    ///
    /// Writing the environment is not enough: the running client captured
    /// its key when it was constructed, so a rotated key reached nobody and
    /// the feature ("credential rotation without restarting the daemon") did
    /// not work. The returned value is what the caller pushes into the live
    /// client.
    #[test]
    fn the_reload_reports_the_credential_it_found() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let file = write_env_file(
            "reload",
            "# a comment\nOPENAI_API_KEY=\"rotated-key\"\nOPENAI_BASE_URL=http://rotated.example/v1\nOTHER=ignored\n",
        );

        // The reload reads the path from the environment, so it is set
        // around the call and restored after.
        let previous = std::env::var("OPERANT_ENV_FILE").ok();
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("OPERANT_ENV_FILE", &file) };

        let rotated = reload_dotenv();

        match previous {
            Some(v) => unsafe { std::env::set_var("OPERANT_ENV_FILE", v) },
            None => unsafe { std::env::remove_var("OPERANT_ENV_FILE") },
        }
        let _ = std::fs::remove_dir_all(file.parent().unwrap_or(&file));

        assert_eq!(
            rotated.api_key.as_deref(),
            Some("rotated-key"),
            "the rotated credential must be reported to the caller"
        );
        assert_eq!(
            rotated.base_url.as_deref(),
            Some("http://rotated.example/v1"),
            "the rotated endpoint must be reported to the caller"
        );
        assert!(!rotated.is_empty());
    }

    /// A `.env` with no credential in it reports nothing, so the caller
    /// does not clear a working credential.
    #[test]
    fn a_reload_without_a_credential_reports_nothing() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let file = write_env_file("empty", "# nothing here\nOTHER=value\n");

        let previous = std::env::var("OPERANT_ENV_FILE").ok();
        // SAFETY: test-only env mutation, serialized by ENV_LOCK.
        unsafe { std::env::set_var("OPERANT_ENV_FILE", &file) };

        let rotated = reload_dotenv();

        match previous {
            Some(v) => unsafe { std::env::set_var("OPERANT_ENV_FILE", v) },
            None => unsafe { std::env::remove_var("OPERANT_ENV_FILE") },
        }
        let _ = std::fs::remove_dir_all(file.parent().unwrap_or(&file));

        assert!(rotated.is_empty(), "no credential means nothing to rotate");
        assert!(rotated.api_key.is_none());
    }
}

use super::config::Settings;

// adapter_types/auth.rs — API credential management.
//
// `auth.json` is a CREDENTIAL store: it holds API keys in plaintext, so it must
// be owner-only. `std::fs::write` applies the umask default (0644 on a stock
// system), which is what this file used — the keys a `Save` click wrote were
// world-readable for the file's whole life. That is RT-154 residual (a): the
// config file's own plaintext claim was fixed in the schema docs, and this file
// is the same class one layer over, where the reader is anyone on the box.
// `operant_core::fs_secrets::write_secret_file` creates with 0600 in the SAME
// open(2) and tightens an existing loose file BEFORE writing, so there is no
// window and a pre-existing 0644 file is repaired on the next save.

pub struct AuthStore {
    pub credentials: std::collections::HashMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
pub enum StoredCredential {
    ApiKey { key: String },
}

impl AuthStore {
    pub fn load() -> Self {
        let mut credentials = std::collections::HashMap::new();

        // Load from environment variables
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
            && !key.is_empty()
        {
            credentials.insert("anthropic".to_string(), StoredCredential::ApiKey { key });
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY")
            && !key.is_empty()
        {
            credentials.insert("openai".to_string(), StoredCredential::ApiKey { key });
        }

        // Load from persisted auth file (simple format: {"provider": "key", ...})
        if let Ok(auth_data) = std::fs::read_to_string(Self::auth_path())
            && let Ok(saved) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(&auth_data)
        {
            for (provider, key) in saved {
                if !key.is_empty() {
                    credentials
                        .entry(provider)
                        .or_insert(StoredCredential::ApiKey { key });
                }
            }
        }

        Self { credentials }
    }

    fn auth_path() -> std::path::PathBuf {
        let dir = Settings::config_dir();
        dir.join("auth.json")
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Settings::config_dir();
        std::fs::create_dir_all(&dir)?;
        let mut map = std::collections::HashMap::new();
        for (provider, cred) in &self.credentials {
            match cred {
                StoredCredential::ApiKey { key } => {
                    map.insert(provider.clone(), key.clone());
                }
            }
        }
        let json = serde_json::to_string_pretty(&map)?;
        // A credential file, not a preference file: owner-only, created in the
        // same open(2) rather than chmod'd after. RT-154 residual (a).
        operant_core::fs_secrets::write_secret_file(&Self::auth_path(), json.as_bytes())?;
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: StoredCredential) {
        self.credentials.insert(key.to_string(), value);
        let _ = self.save();
    }
    pub fn api_key_for(&self, provider: impl Into<String>) -> Option<String> {
        let provider = provider.into();
        let StoredCredential::ApiKey { key } = self.credentials.get(&provider)?;
        Some(key.clone())
    }
    pub fn has_any_key(&self) -> bool {
        self.credentials
            .values()
            .any(|c| matches!(c, StoredCredential::ApiKey { key } if !key.is_empty()))
    }
}

// (iter-223: pub mod file_injection { AtFileRef, AtFileIssue, parse_at_refs }
// deleted — zero callers anywhere; the @-file parsing path they supported
// was never wired to a consumer.)

#[cfg(test)]
mod credential_file_tests {
    use super::*;

    // The ONE crate-wide environment lock (see `crate::config::TEST_ENV_LOCK`):
    // `HERMES_HOME` decides where `Settings::config_dir()` points, so a test
    // that sets it must exclude every other environment test in the crate.
    // Minting a module-local lock would let two modules mutate the same var
    // concurrently, which is the failure that lock was created to stop.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::config::test_env_lock()
    }

    fn set_env(name: &str, value: &std::path::Path) -> Option<std::ffi::OsString> {
        let prev = std::env::var_os(name);
        // SAFETY: test-only env mutation, guarded by the crate-wide lock.
        unsafe { std::env::set_var(name, value) };
        prev
    }

    fn restore_env(name: &str, prev: Option<std::ffi::OsString>) {
        match prev {
            Some(val) => unsafe { std::env::set_var(name, val) },
            None => unsafe { std::env::remove_var(name) },
        }
    }

    #[cfg(unix)]
    fn mode_of(path: &std::path::Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// RT-154 residual (a). `auth.json` holds API keys, and the writer used
    /// `std::fs::write`, which applies the umask default — 0644 on a stock box.
    /// The keys an operator saved from `/connect` sat world-readable for the
    /// file's whole life. This is the same class as the config-file plaintext
    /// claim fixed in the schema, one layer over, where the reader is anyone.
    #[test]
    fn a_saved_api_key_is_written_owner_only_not_umask_default() {
        let _guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let prev = set_env("HERMES_HOME", dir.path());

        let mut store = AuthStore::load();
        store.set(
            "openai",
            StoredCredential::ApiKey {
                key: "sk-secret".into(),
            },
        );

        let mode = mode_of(&Settings::config_dir().join("auth.json"));
        restore_env("HERMES_HOME", prev);
        drop(store);
        drop(dir);

        assert_eq!(
            mode & 0o077,
            0,
            "a credential file must not be group- or world-readable, got {:o}",
            mode
        );
    }

    /// A file that is already loose is repaired on the next save, not left
    /// readable: an existing file keeps its mode when it is opened, so a
    /// create-only fix would silently pass on every operator already holding
    /// a 0644 `auth.json` from a previous session.
    #[test]
    fn a_previously_world_readable_auth_file_is_tightened_on_save() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _guard = env_lock();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("auth.json");
            std::fs::write(&path, r#"{"openai":"old"}"#).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(
                mode_of(&path) & 0o004,
                0o004,
                "test starts from a world-readable file"
            );

            let prev = set_env("HERMES_HOME", dir.path());
            let mut store = AuthStore::load();
            store.set(
                "anthropic",
                StoredCredential::ApiKey {
                    key: "sk-new".into(),
                },
            );
            let mode = mode_of(&path);
            restore_env("HERMES_HOME", prev);
            drop(store);
            drop(dir);

            assert_eq!(
                mode & 0o077,
                0,
                "saving over a loose credential file must tighten it, got {:o}",
                mode
            );
            assert!(
                mode & 0o400 != 0,
                "the owner must still be able to read their own keys, got {:o}",
                mode
            );
        }
    }
}

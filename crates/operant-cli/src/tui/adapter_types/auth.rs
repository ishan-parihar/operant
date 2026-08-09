use super::config::Settings;

// adapter_types/auth.rs — API credential management.

pub struct AuthStore {
    pub credentials: std::collections::HashMap<String, StoredCredential>,
}

#[derive(Debug, Clone)]
pub enum StoredCredential {
    ApiKey {
        key: String,
    },
    OAuthToken {
        access: String,
        #[allow(dead_code)] // Prepared for OAuth token refresh logic
        refresh: String,
        #[allow(dead_code)] // Prepared for OAuth token expiry tracking
        expires: u64,
    },
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
                StoredCredential::OAuthToken { access, .. } => {
                    map.insert(provider.clone(), access.clone());
                }
            }
        }
        let json = serde_json::to_string_pretty(&map)?;
        std::fs::write(Self::auth_path(), json)?;
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: StoredCredential) {
        self.credentials.insert(key.to_string(), value);
        let _ = self.save();
    }
    pub fn api_key_for(&self, provider: impl Into<String>) -> Option<String> {
        let provider = provider.into();
        match self.credentials.get(&provider)? {
            StoredCredential::ApiKey { key } => Some(key.clone()),
            _ => None,
        }
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

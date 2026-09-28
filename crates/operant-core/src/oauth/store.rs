//! The account index for provider OAuth: **keys and ordering, never tokens**.
//!
//! # What this file is
//!
//! An index. It answers "which accounts exist, in what order, and where does
//! each one's credential live" — the two things a login flow needs to resume,
//! rotate and re-authenticate without a network round trip.
//!
//! # What this file is not
//!
//! It is **not** a token store. No access token, refresh token, `code_verifier`,
//! `user_code` or `device_code` is ever written here. That is enforced
//! structurally, not by convention:
//!
//! * the only field that can even point at a secret is
//!   [`AccountRecord::token_ref`], a [`TokenRef`];
//! * [`TokenRef`]'s `Serialize` impl emits a **truncated SHA-256 fingerprint**
//!   of the handle, never the handle. So even if a caller constructs a
//!   `TokenRef` from something that *is* a secret, the bytes cannot reach disk
//!   through this type.
//!
//! The credential itself belongs in whatever secret store the deployment uses;
//! this index only records the fingerprint that store is keyed by. See
//! [`AccountStore::path`] for the on-disk location.
//!
//! # Posture
//!
//! The file is written through [`crate::fs_secrets::write_secret_file`], which
//! forces mode `0o600` and tightens a looser existing file. The parent
//! directory is forced to `0o700`. It is still plaintext — the content is not
//! secret by design — but a `0o644` copy of a credential *index* should not
//! exist on a shared box.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// On-disk schema version, so a future format change can migrate rather than
/// misparse.
pub const STORE_VERSION: u32 = 1;

/// Default index filename under the operant home.
pub const STORE_FILENAME: &str = "accounts.json";

/// Bytes of the handle digest kept in the index. 16 base64url chars ≈ 96 bits —
/// enough to key a lookup, far too little to reconstruct anything.
const FINGERPRINT_BYTES: usize = 12;

/// A handle into the deployment's secret store.
///
/// The handle is kept in memory verbatim (it is needed to fetch the secret) but
/// **never serialized**: [`Serialize`] writes only a truncated digest, so the
/// index on disk cannot contain the handle — or anything derived from a
/// credential that was mistakenly put here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRef {
    handle: String,
    fingerprint: String,
}

impl TokenRef {
    /// Build a reference from a secret-store handle.
    ///
    /// Rejects handles that are empty or longer than 256 bytes. A real
    /// credential is orders of magnitude longer than that, so this catches the
    /// "pasted the access token into the ref" mistake even though the
    /// fingerprinting below would already have contained the damage.
    pub fn new(handle: impl Into<String>) -> Result<Self> {
        let handle = handle.into();
        if handle.is_empty() {
            return Err(Error::Config("TokenRef handle is empty".to_string()));
        }
        if handle.len() > 256 {
            return Err(Error::Config(format!(
                "TokenRef handle is {} bytes; a secret-store handle is expected, not a credential",
                handle.len()
            )));
        }
        let digest = Sha256::digest(handle.as_bytes());
        let fingerprint = format!(
            "sha256:{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..FINGERPRINT_BYTES])
        );
        Ok(Self {
            handle,
            fingerprint,
        })
    }

    /// The handle, for the in-process secret-store lookup. Never logged.
    pub fn handle(&self) -> &str {
        &self.handle
    }

    /// The on-disk digest. Safe to log and to store.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl Serialize for TokenRef {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.fingerprint)
    }
}

impl<'de> Deserialize<'de> for TokenRef {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        // A `TokenRef` round-tripped through JSON comes back as its
        // fingerprint. It is not the handle and must never be used as one, so
        // the in-memory handle is empty and `handle()` yields `""` — a
        // miss, not a wrong secret.
        let fingerprint = String::deserialize(deserializer)?;
        Ok(Self {
            handle: String::new(),
            fingerprint,
        })
    }
}

/// Which authorization flow produced an account. Selection metadata, not a
/// secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthFlow {
    /// Authorization Code + PKCE against a loopback redirect.
    Pkce,
    /// RFC 8628 device authorization.
    DeviceCode,
    /// Credentials were supplied out of band (env var, pasted key).
    External,
}

/// One indexed account. Every field here is non-secret by construction; the
/// credential is reached through `token_ref`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountRecord {
    /// Stable, non-secret identifier. This is also the key the refresh
    /// coordinator scopes by.
    pub account_key: String,
    /// Provider slug (`anthropic`, `openai-codex`, `xai-oauth`, `nous`).
    pub provider: String,
    /// Human label for listings.
    pub label: String,
    /// Handle into the secret store — persisted only as a digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_ref: Option<TokenRef>,
    /// Public client id. Not a secret (it ships inside distributed binaries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Flow that produced this account.
    pub flow: AuthFlow,
    /// Token endpoint used for refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    /// Device authorization endpoint, when the flow used one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_authorization_endpoint: Option<String>,
    /// Granted scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// When the current credential was obtained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued_at: Option<DateTime<Utc>>,
    /// When the current access token expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// When the credential was last refreshed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<DateTime<Utc>>,
}

impl AccountRecord {
    /// A minimal record for `provider`, keyed by `account_key`.
    pub fn new(
        account_key: impl Into<String>,
        provider: impl Into<String>,
        flow: AuthFlow,
    ) -> Self {
        Self {
            account_key: account_key.into(),
            provider: provider.into(),
            label: String::new(),
            token_ref: None,
            client_id: None,
            flow,
            token_endpoint: None,
            device_authorization_endpoint: None,
            scopes: Vec::new(),
            issued_at: None,
            expires_at: None,
            refreshed_at: None,
        }
    }

    /// Set the human label.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Attach the secret-store handle.
    pub fn with_token_ref(mut self, token_ref: TokenRef) -> Self {
        self.token_ref = Some(token_ref);
        self
    }

    /// Attach the public client id.
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Record the access token's absolute lifetime.
    pub fn with_lifetime(
        mut self,
        issued_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        self.issued_at = Some(issued_at);
        self.expires_at = expires_at;
        self
    }
}

/// The serialized index: accounts plus their insertion order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountIndex {
    /// [`STORE_VERSION`].
    pub version: u32,
    /// Account keys in the order they were first added. Refresh rotation and
    /// UI listing both depend on a stable order, so it is persisted rather
    /// than inferred from a map.
    #[serde(default)]
    pub order: Vec<String>,
    /// Accounts by key.
    #[serde(default)]
    pub accounts: HashMap<String, AccountRecord>,
}

impl Default for AccountIndex {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            order: Vec::new(),
            accounts: HashMap::new(),
        }
    }
}

/// The account index: keys, ordering, and non-secret metadata.
#[derive(Debug, Clone)]
pub struct AccountStore {
    path: PathBuf,
    index: AccountIndex,
}

impl AccountStore {
    /// An empty store rooted at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            index: AccountIndex::default(),
        }
    }

    /// The default location, `<operant home>/accounts.json`
    /// (`$HERMES_HOME` or `~/.operant`).
    pub fn default_path() -> PathBuf {
        std::env::var("HERMES_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".operant")))
            .unwrap_or_else(|| PathBuf::from(".operant"))
            .join(STORE_FILENAME)
    }

    /// A store rooted at [`AccountStore::default_path`].
    pub fn with_default_path() -> Self {
        Self::new(AccountStore::default_path())
    }

    /// Where the index lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the index. A missing file is an empty store, not an error.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self::new(path));
        }
        let content = std::fs::read_to_string(&path).map_err(Error::Io)?;
        let index: AccountIndex = serde_json::from_str(&content)
            .map_err(|e| Error::ParseResponse(format!("{} parse error: {e}", path.display())))?;
        Ok(Self { path, index })
    }

    /// Write the index with `0o600` on the file and `0o700` on its directory.
    pub fn save(&self) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| Error::Config(format!("{} has no parent", self.path.display())))?;
        std::fs::create_dir_all(parent).map_err(Error::Io)?;
        set_dir_private(parent)?;

        let json = serde_json::to_string_pretty(&self.index)
            .map_err(|e| Error::ParseResponse(format!("account index serialize error: {e}")))?;
        crate::fs_secrets::write_secret_file(&self.path, json.as_bytes()).map_err(Error::Io)
    }

    /// The persisted account keys, in insertion order.
    pub fn ordering(&self) -> &[String] {
        &self.index.order
    }

    /// Look up a record.
    pub fn get(&self, account_key: &str) -> Option<&AccountRecord> {
        self.index.accounts.get(account_key)
    }

    /// Every record, in persisted order.
    pub fn ordered(&self) -> Vec<&AccountRecord> {
        self.index
            .order
            .iter()
            .filter_map(|key| self.index.accounts.get(key))
            .collect()
    }

    /// Insert or replace a record, appending to the order if it is new.
    pub fn upsert(&mut self, record: AccountRecord) {
        if !self.index.order.contains(&record.account_key) {
            self.index.order.push(record.account_key.clone());
        }
        self.index
            .accounts
            .insert(record.account_key.clone(), record);
    }

    /// Remove a record and its ordering entry.
    pub fn remove(&mut self, account_key: &str) -> Option<AccountRecord> {
        self.index.order.retain(|k| k != account_key);
        self.index.accounts.remove(account_key)
    }

    /// Number of indexed accounts.
    pub fn len(&self) -> usize {
        self.index.accounts.len()
    }

    /// `true` when nothing is indexed.
    pub fn is_empty(&self) -> bool {
        self.index.accounts.is_empty()
    }
}

#[cfg(unix)]
fn set_dir_private(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(Error::Io)
}

#[cfg(not(unix))]
fn set_dir_private(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values that must never reach the index file.
    const SECRETS: &[&str] = &[
        "sk-ant-oat01-REAL-ACCESS-TOKEN",
        "sk-ant-ort01-REAL-REFRESH-TOKEN",
        "the-code-verifier-from-pkce",
        "USER-CODE-1234",
        "device-code-abcdef",
    ];

    fn temp_index() -> PathBuf {
        // Same shape as `fs_secrets`'s own test helper: a unique directory the
        // test owns outright, so no TempDir API churn and the file outlives the
        // assertion.
        let mut dir = std::env::temp_dir();
        dir.push(format!("operant-account-store-{}", std::process::id()));
        dir.push(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
                .to_string(),
        );
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir.join("operant").join(STORE_FILENAME)
    }

    #[test]
    fn account_store_should_not_persist_raw_tokens() {
        let path = temp_index();
        let mut store = AccountStore::new(&path);

        let mut record = AccountRecord::new("anthropic/work", "anthropic", AuthFlow::Pkce)
            .labelled("work account")
            .with_token_ref(TokenRef::new("vault://operant/anthropic/work").expect("ref"))
            .with_client_id("9d1c250a-e61b-44d9-88ed-5944d1962f5e");
        record.scopes = vec!["user:inference".to_string()];
        record.issued_at = Some(Utc::now());
        record.expires_at = Some(Utc::now() + chrono::Duration::hours(8));
        record.refreshed_at = Some(Utc::now());
        record.token_endpoint = Some("https://platform.claude.com/v1/oauth/token".to_string());
        store.upsert(record);
        store.upsert(
            AccountRecord::new("nous/personal", "nous", AuthFlow::DeviceCode)
                .labelled("personal")
                .with_token_ref(TokenRef::new("vault://operant/nous/personal").expect("ref")),
        );
        store.save().expect("save");

        let raw = std::fs::read_to_string(&path).expect("read index");

        // 1. No secret in any form, verbatim.
        for secret in SECRETS {
            assert!(!raw.contains(secret), "account index leaked {secret:?}");
        }
        // 2. Not even the in-memory handle the store legitimately holds.
        assert!(
            !raw.contains("vault://operant/anthropic/work"),
            "account index leaked the secret-store handle"
        );

        // 3. Allowlist every JSON key in the document. A banned list would only
        //    catch the names we happened to think of; an allowlist catches
        //    *any* new field, so a token cannot be added to the index later
        //    without failing here. (`flow: "device_code"` is a value, not a
        //    key, which is why this is a key check and not a substring scan.)
        const ALLOWED_KEYS: &[&str] = &[
            "version",
            "order",
            "accounts",
            "account_key",
            "provider",
            "label",
            "token_ref",
            "client_id",
            "flow",
            "token_endpoint",
            "device_authorization_endpoint",
            "scopes",
            "issued_at",
            "expires_at",
            "refreshed_at",
        ];
        let document: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
        let mut keys = Vec::new();
        collect_json_keys(&document, &mut keys);
        assert!(!keys.is_empty(), "sanity: the document parsed to nothing");
        for key in &keys {
            assert!(
                ALLOWED_KEYS.contains(&key.as_str()),
                "account index has an undeclared field {key:?}"
            );
        }
        for banned in [
            "access_token",
            "refresh_token",
            "code_verifier",
            "user_code",
            "device_code",
            "token",
        ] {
            assert!(
                !keys.iter().any(|k| k == banned),
                "account index has a {banned:?} field"
            );
        }

        // 4. Only a digest of the ref is persisted.
        let ref_fingerprint = TokenRef::new("vault://operant/anthropic/work")
            .expect("ref")
            .fingerprint()
            .to_string();
        assert!(
            raw.contains(&ref_fingerprint),
            "expected the digest in {raw}"
        );
        assert!(ref_fingerprint.starts_with("sha256:"));

        // 5. What *is* persisted: keys and ordering.
        let reloaded = AccountStore::load(&path).expect("load");
        assert_eq!(reloaded.ordering(), ["anthropic/work", "nous/personal"]);
        let first = reloaded.get("anthropic/work").expect("record");
        assert_eq!(first.provider, "anthropic");
        assert_eq!(first.flow, AuthFlow::Pkce);
        assert_eq!(first.scopes, vec!["user:inference".to_string()]);
        // The handle does not survive the round trip — by design.
        assert_eq!(
            first.token_ref.as_ref().map(TokenRef::handle),
            Some(""),
            "a reloaded ref must not present a usable handle"
        );
        assert_eq!(
            first.token_ref.as_ref().map(TokenRef::fingerprint),
            Some(ref_fingerprint.as_str())
        );
    }

    /// Every schema key in an account-index document.
    ///
    /// `accounts` is keyed by account id, so its *own* keys are data rather
    /// than schema and are skipped; the records beneath it are walked.
    fn collect_json_keys(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    if key == "accounts" {
                        if let serde_json::Value::Object(accounts) = child {
                            for record in accounts.values() {
                                collect_json_keys(record, out);
                            }
                        }
                        continue;
                    }
                    out.push(key.clone());
                    collect_json_keys(child, out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_json_keys(item, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn account_store_is_written_private() {
        let path = temp_index();
        let mut store = AccountStore::new(&path);
        store.upsert(AccountRecord::new("k", "anthropic", AuthFlow::Pkce));
        store.save().expect("save");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "index must be owner-only, got {mode:o}");
            let dir_mode = std::fs::metadata(path.parent().expect("parent"))
                .expect("stat dir")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir_mode, 0o700, "index dir must be owner-only");
        }
    }

    #[test]
    fn account_store_upsert_preserves_insertion_order() {
        let mut store = AccountStore::new(temp_index());
        store.upsert(AccountRecord::new("a", "anthropic", AuthFlow::Pkce));
        store.upsert(AccountRecord::new("b", "nous", AuthFlow::DeviceCode));
        // Re-upserting an existing key must not reorder it.
        store.upsert(AccountRecord::new("a", "anthropic", AuthFlow::Pkce));
        assert_eq!(store.ordering(), ["a", "b"]);

        store.remove("a");
        assert_eq!(store.ordering(), ["b"]);
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
    }

    #[test]
    fn token_ref_rejects_a_credential_in_the_handle_slot() {
        assert!(TokenRef::new("").is_err(), "empty handle");
        assert!(
            TokenRef::new("x".repeat(1024)).is_err(),
            "a 1 KiB handle is a pasted credential, not a vault key"
        );
        assert!(TokenRef::new("vault://operant/x").is_ok());
    }
}

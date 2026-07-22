//! Multi-credential pool for same-provider failover and load balancing.
//!
//! Provides thread-safe management of multiple credentials for the same
//! provider with configurable selection strategies (fill-first, round-robin,
//! random, least-used). Supports credential seeding from environment
//! variables, configuration files, and OAuth refresh dispatching.
//!
//! Ported from `operant-agent/agent/credential_pool.py`.

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Cooldown before retrying an exhausted credential (seconds).
pub const EXHAUSTED_TTL_SECONDS: u64 = 300; // 5 minutes

/// Status value: credential is healthy and usable.
pub const STATUS_OK: &str = "ok";
/// Status value: credential is exhausted and should be avoided.
pub const STATUS_EXHAUSTED: &str = "exhausted";

// ---------------------------------------------------------------------------
// Auth type enum
// ---------------------------------------------------------------------------

/// The authentication type for a pooled credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AuthType {
    /// API key based authentication.
    #[default]
    ApiKey,
    /// OAuth 2.0 based authentication.
    OAuth,
    /// HTTP Basic authentication.
    Basic,
    /// Bearer token based authentication.
    Token,
    /// Custom authentication scheme.
    Custom,
}

impl AuthType {
    /// Return the string representation of this auth type.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthType::ApiKey => "api_key",
            AuthType::OAuth => "oauth",
            AuthType::Basic => "basic",
            AuthType::Token => "token",
            AuthType::Custom => "custom",
        }
    }
}

// ---------------------------------------------------------------------------
// Pool strategy enum
// ---------------------------------------------------------------------------

/// Selection strategy for choosing a credential from the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum PoolStrategy {
    /// Always select the first available credential (default).
    #[default]
    FillFirst,
    /// Cycle through credentials in order.
    RoundRobin,
    /// Pick a credential at random.
    Random,
    /// Pick the credential with the fewest uses so far.
    LeastUsed,
}

impl PoolStrategy {
    /// Parse a strategy from a string, returning `FillFirst` for unknown values.
    pub fn parse_strategy(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "fill_first" | "fill-first" => PoolStrategy::FillFirst,
            "round_robin" | "round-robin" => PoolStrategy::RoundRobin,
            "random" => PoolStrategy::Random,
            "least_used" | "least-used" => PoolStrategy::LeastUsed,
            _ => PoolStrategy::FillFirst,
        }
    }
}

// ---------------------------------------------------------------------------
// PooledCredential
// ---------------------------------------------------------------------------

/// A single credential entry in the credential pool.
///
/// Each entry holds an encrypted credential value along with metadata
/// about usage, source, and error status for failover decisions.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PooledCredential {
    /// Unique identifier for this credential entry.
    pub id: String,
    /// Human-readable name/label for this credential.
    pub name: String,
    /// Authentication type (api_key, oauth, basic, token, custom).
    pub credential_type: AuthType,
    /// The credential value (base64-encoded placeholder — not real encryption).
    pub value: String,
    /// Where this credential was sourced from (e.g., "env:OPENAI_API_KEY", "config", "manual").
    pub source: String,
    /// When this credential was created/added to the pool.
    #[schemars(with = "String")]
    pub created_at: DateTime<Utc>,
    /// Optional expiration timestamp.
    #[schemars(with = "Option<String>")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Arbitrary key-value metadata attached to this credential.
    pub metadata: HashMap<String, String>,
    /// Number of times this credential has been selected/used.
    pub usage_count: u64,
    /// Timestamp of the most recent usage.
    #[schemars(with = "Option<String>")]
    pub last_used_at: Option<DateTime<Utc>>,
    /// Current status string (None = ok, Some("exhausted") = exhausted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// HTTP status code from the last error (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<i32>,
    /// Short reason string from the last error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_reason: Option<String>,
    /// Human-readable error message from the last failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_message: Option<String>,
    /// When the current exhaustion cooldown expires.
    #[schemars(with = "Option<String>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reset_at: Option<DateTime<Utc>>,

    // OAuth-specific fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_key: Option<String>,
    #[schemars(with = "Option<String>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_key_expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
}

impl PooledCredential {
    /// Create a new pooled credential with default values.
    pub fn new(name: &str, credential_type: AuthType, value: &str, source: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            credential_type,
            value: value.to_string(),
            source: source.to_string(),
            created_at: Utc::now(),
            expires_at: None,
            metadata: HashMap::new(),
            usage_count: 0,
            last_used_at: None,
            status: None,
            last_error_code: None,
            last_error_reason: None,
            last_error_message: None,
            error_reset_at: None,
            refresh_token: None,
            client_id: None,
            portal_base_url: None,
            inference_base_url: None,
            agent_key: None,
            agent_key_expires_at: None,
            token_endpoint: None,
            token_type: None,
            last_refresh: None,
        }
    }

    /// Returns `true` if this credential is currently usable (not exhausted).
    pub fn is_available(&self) -> bool {
        if self.status.as_deref() == Some(STATUS_EXHAUSTED) {
            if let Some(reset_at) = self.error_reset_at {
                return Utc::now() >= reset_at;
            }
            // No reset_at means we use the default TTL from last_error
            if let Some(_code) = self.last_error_code {
                // Without last_status_at we can't compute TTL, so assume available
                return true;
            }
            return false;
        }
        true
    }

    /// Returns `true` if the credential has not expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|exp| Utc::now() >= exp)
    }

    /// Mark this credential as used, incrementing the usage counter.
    pub fn mark_used(&mut self) {
        self.usage_count = self.usage_count.saturating_add(1);
        self.last_used_at = Some(Utc::now());
    }

    /// Check if an OAuth access token is expiring within the given skew window.
    pub fn is_oauth_expiring(&self, skew_seconds: u64) -> bool {
        if self.credential_type != AuthType::OAuth {
            return false;
        }
        self.expires_at
            .is_some_and(|exp| Utc::now() + chrono::Duration::seconds(skew_seconds as i64) >= exp)
    }

    /// Check if this OAuth credential needs a token refresh.
    pub fn needs_oauth_refresh(&self, skew_seconds: u64) -> bool {
        if self.credential_type != AuthType::OAuth {
            return false;
        }
        if self.refresh_token.is_none() {
            return false;
        }
        self.is_oauth_expiring(skew_seconds)
    }
}

impl Default for PooledCredential {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: String::new(),
            credential_type: AuthType::default(),
            value: String::new(),
            source: String::new(),
            created_at: Utc::now(),
            expires_at: None,
            metadata: HashMap::new(),
            usage_count: 0,
            last_used_at: None,
            status: None,
            last_error_code: None,
            last_error_reason: None,
            last_error_message: None,
            error_reset_at: None,
            refresh_token: None,
            client_id: None,
            portal_base_url: None,
            inference_base_url: None,
            agent_key: None,
            agent_key_expires_at: None,
            token_endpoint: None,
            token_type: None,
            last_refresh: None,
        }
    }
}

// ---------------------------------------------------------------------------
// CredentialPool
// ---------------------------------------------------------------------------

/// Internal state protected by the pool's `RwLock`.
struct PoolInner {
    /// Credentials keyed by their unique id.
    credentials: HashMap<String, PooledCredential>,
    /// Ordered list of credential ids for strategy-based iteration.
    ordered_ids: Vec<String>,
    /// Current round-robin index.
    round_robin_index: usize,
}

/// A thread-safe credential pool with configurable selection strategies.
///
/// Supports multiple credentials for the same provider with automatic
/// failover via `FillFirst`, `RoundRobin`, `Random`, or `LeastUsed` strategies.
///
/// # Examples
///
/// ```ignore
/// use operant_core::credential_pool::{CredentialPool, PooledCredential, AuthType, PoolStrategy};
///
/// let pool = CredentialPool::new("openai");
/// let cred = PooledCredential::new("my-key", AuthType::ApiKey, "sk-...", "env:OPENAI_API_KEY");
/// let id = pool.add(cred);
/// let selected = pool.select();
/// ```
pub struct CredentialPool {
    /// Provider name this pool manages credentials for.
    provider: String,
    /// Selection strategy.
    strategy: PoolStrategy,
    /// Internal state protected by a read-write lock.
    inner: RwLock<PoolInner>,
}

impl CredentialPool {
    /// Create a new credential pool for the given provider with the default
    /// `FillFirst` strategy.
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            strategy: PoolStrategy::FillFirst,
            inner: RwLock::new(PoolInner {
                credentials: HashMap::new(),
                ordered_ids: Vec::new(),
                round_robin_index: 0,
            }),
        }
    }

    /// Create a new credential pool with a specific strategy.
    pub fn with_strategy(provider: &str, strategy: PoolStrategy) -> Self {
        Self {
            provider: provider.to_string(),
            strategy,
            inner: RwLock::new(PoolInner {
                credentials: HashMap::new(),
                ordered_ids: Vec::new(),
                round_robin_index: 0,
            }),
        }
    }

    /// Return the provider name associated with this pool.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Return the current selection strategy.
    pub fn strategy(&self) -> PoolStrategy {
        self.strategy
    }

    /// Set a new selection strategy.
    pub fn set_strategy(&mut self, strategy: PoolStrategy) {
        self.strategy = strategy;
    }

    /// Add a credential to the pool.
    ///
    /// Returns the credential's unique id.
    pub fn add(&self, credential: PooledCredential) -> String {
        let id = credential.id.clone();
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.ordered_ids.push(id.clone());
        inner.credentials.insert(id.clone(), credential);
        debug!(provider = %self.provider, id = %id, "Added credential to pool");
        id
    }

    /// Retrieve a credential by id.
    pub fn get(&self, id: &str) -> Option<PooledCredential> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.credentials.get(id).cloned()
    }

    /// Remove a credential from the pool by id.
    ///
    /// Returns the removed credential, or `None` if it was not found.
    pub fn remove(&self, id: &str) -> Option<PooledCredential> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let removed = inner.credentials.remove(id)?;
        inner.ordered_ids.retain(|i| i != id);
        info!(provider = %self.provider, id = %id, "Removed credential from pool");
        Some(removed)
    }

    /// Return a snapshot of all credentials in the pool.
    pub fn list(&self) -> Vec<PooledCredential> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .ordered_ids
            .iter()
            .filter_map(|id| inner.credentials.get(id).cloned())
            .collect()
    }

    /// Return the number of credentials in the pool.
    pub fn len(&self) -> usize {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.credentials.len()
    }

    /// Returns `true` if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the pool has at least one credential.
    pub fn has_credentials(&self) -> bool {
        !self.is_empty()
    }

    /// Returns `true` if at least one credential is not exhausted.
    pub fn has_available(&self) -> bool {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.credentials.values().any(|c| c.is_available())
    }

    /// Update an existing credential in the pool.
    ///
    /// Replaces the credential with the matching id. Returns an error if
    /// the credential id is not found in the pool.
    pub fn update(&self, credential: PooledCredential) -> Result<()> {
        let id = credential.id.clone();
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if !inner.credentials.contains_key(&id) {
            return Err(Error::Agent(format!(
                "Credential '{}' not found in pool for provider '{}'",
                id, self.provider
            )));
        }
        inner.credentials.insert(id.clone(), credential);
        debug!(provider = %self.provider, id = %id, "Updated credential in pool");
        Ok(())
    }

    /// Select a credential from the pool based on the configured strategy.
    ///
    /// Returns `None` if no available credentials exist.
    pub fn select(&self) -> Option<PooledCredential> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        self.select_inner(&mut inner)
    }

    /// Peek at the current or first available credential without consuming it.
    ///
    /// Unlike [`select`](Self::select), this does not apply strategy rotation
    /// or update usage counts.
    pub fn peek(&self) -> Option<PooledCredential> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        // Return first available credential
        inner
            .ordered_ids
            .iter()
            .filter_map(|id| inner.credentials.get(id))
            .find(|c| c.is_available())
            .cloned()
    }

    /// Mark a credential as exhausted with an error context.
    ///
    /// If `rotate` is `true`, automatically selects the next available
    /// credential after marking.
    ///
    /// Returns the newly selected credential if rotation was requested
    /// and an alternative was available.
    pub fn invalidate(
        &self,
        id: &str,
        error_code: Option<i32>,
        reason: Option<&str>,
        message: Option<&str>,
        rotate: bool,
    ) -> Option<PooledCredential> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());

        if let Some(cred) = inner.credentials.get_mut(id) {
            cred.status = Some(STATUS_EXHAUSTED.to_string());
            cred.last_error_code = error_code;
            cred.last_error_reason = reason.map(|s| s.to_string());
            cred.last_error_message = message.map(|s| s.to_string());
            cred.error_reset_at =
                Some(Utc::now() + chrono::Duration::seconds(EXHAUSTED_TTL_SECONDS as i64));

            info!(
                provider = %self.provider,
                credential = %id,
                code = ?error_code,
                reason = ?reason,
                "Credential marked exhausted"
            );
        }

        if rotate {
            self.select_inner(&mut inner)
        } else {
            None
        }
    }

    /// Refresh all OAuth credentials in the pool.
    pub async fn refresh_async(&self) -> Result<()> {
        let refresher = crate::oauth_refresh::OAuthRefresher::new()?;
        // Collect the credentials that need refreshing and drop the read
        // guard before the loop below — the loop awaits a network call per
        // credential, and holding a std::sync::RwLock guard across an await
        // point would block any writer for the whole loop's duration instead
        // of just this synchronous filter.
        let to_refresh: Vec<PooledCredential> = {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            inner
                .credentials
                .values()
                .filter(|cred| {
                    cred.credential_type == AuthType::OAuth && cred.needs_oauth_refresh(120)
                })
                .cloned()
                .collect()
        };

        for cred in &to_refresh {
            info!(
                provider = %self.provider,
                credential = %cred.name,
                "Refreshing OAuth token"
            );
            match refresher.refresh(&self.provider, cred).await {
                Ok(response) => {
                    info!(
                        provider = %self.provider,
                        credential = %cred.name,
                        "OAuth token refreshed successfully"
                    );
                    let _ = refresher.persist_to_auth_store(&self.provider, cred, &response);
                }
                Err(e) => {
                    warn!(
                        provider = %self.provider,
                        credential = %cred.name,
                        error = %e,
                        "OAuth refresh failed, attempting sync from auth store"
                    );
                    if let Some(synced) = refresher.sync_from_auth_store(&self.provider, cred) {
                        if synced.value != cred.value {
                            info!(provider = %self.provider, credential = %cred.name, "Adopted synced token from auth store");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Attempt an OAuth refresh for a specific provider type.
    pub async fn refresh_oauth_async(&self, provider_type: &str) -> Result<()> {
        let refresher = crate::oauth_refresh::OAuthRefresher::new()?;
        // See refresh_async above — collect first, drop the read guard
        // before the awaiting loop.
        let to_refresh: Vec<PooledCredential> = {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            inner
                .credentials
                .values()
                .filter(|cred| cred.credential_type == AuthType::OAuth)
                .cloned()
                .collect()
        };

        for cred in &to_refresh {
            match refresher.refresh(provider_type, cred).await {
                Ok(response) => {
                    info!(
                        provider = %self.provider,
                        oauth_provider = %provider_type,
                        credential = %cred.name,
                        "OAuth credential refreshed"
                    );
                    let _ = refresher.persist_to_auth_store(provider_type, cred, &response);
                }
                Err(e) => {
                    warn!(
                        provider = %self.provider,
                        oauth_provider = %provider_type,
                        credential = %cred.name,
                        error = %e,
                        "OAuth refresh failed"
                    );
                    if let Some(synced) = refresher.sync_from_auth_store(provider_type, cred) {
                        if synced.value != cred.value {
                            info!(provider = %self.provider, "Adopted synced token from auth store");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Reset all exhausted credentials back to available status.
    ///
    /// Returns the number of credentials that were reset.
    pub fn reset_statuses(&self) -> usize {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let mut count = 0usize;
        for cred in inner.credentials.values_mut() {
            if cred.status.is_some()
                || cred.last_error_code.is_some()
                || cred.last_error_reason.is_some()
            {
                cred.status = None;
                cred.last_error_code = None;
                cred.last_error_reason = None;
                cred.last_error_message = None;
                cred.error_reset_at = None;
                count += 1;
            }
        }
        if count > 0 {
            info!(provider = %self.provider, count = %count, "Reset credential statuses");
        }
        count
    }

    // -----------------------------------------------------------------------
    // Seeding helpers
    // -----------------------------------------------------------------------

    /// Seed a credential from an environment variable.
    ///
    /// Reads the given environment variable and creates a credential entry
    /// with `credential_type = ApiKey`. Does nothing if the variable is
    /// empty or not set.
    ///
    /// Returns the credential id if a credential was added.
    pub fn seed_from_env(&self, env_var: &str) -> Option<String> {
        let value = std::env::var(env_var).ok()?;
        if value.trim().is_empty() {
            return None;
        }
        let cred =
            PooledCredential::new(env_var, AuthType::ApiKey, &value, &format!("env:{env_var}"));
        let id = self.add(cred);
        debug!(provider = %self.provider, env_var = %env_var, "Seeded credential from env");
        Some(id)
    }

    /// Seed a credential from a config key-value pair.
    ///
    /// Returns the credential id.
    pub fn seed_from_config(&self, key: &str, value: &str, source: &str) -> String {
        let cred = PooledCredential::new(key, AuthType::ApiKey, value, source);
        let id = self.add(cred);
        debug!(provider = %self.provider, key = %key, "Seeded credential from config");
        id
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Internal credential selection by strategy (expects write lock held).
    fn select_inner(&self, inner: &mut PoolInner) -> Option<PooledCredential> {
        // Build available list (not exhausted, not expired)
        let available_indices: Vec<usize> = inner
            .ordered_ids
            .iter()
            .enumerate()
            .filter_map(|(idx, id)| {
                let cred = inner.credentials.get(id)?;
                if cred.is_available() && !cred.is_expired() {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect();

        if available_indices.is_empty() {
            info!(provider = %self.provider, "No available credentials in pool");
            return None;
        }

        let chosen_idx = match self.strategy {
            PoolStrategy::FillFirst => {
                // First available
                available_indices[0]
            }
            PoolStrategy::RoundRobin => {
                // Find the next available index at or after round_robin_index
                let start = inner.round_robin_index;
                let mut selected = None;
                for &idx in &available_indices {
                    if idx >= start {
                        selected = Some(idx);
                        break;
                    }
                }
                let idx = selected.unwrap_or(available_indices[0]);
                inner.round_robin_index = (idx + 1) % inner.ordered_ids.len();
                idx
            }
            PoolStrategy::Random => {
                // Pseudo-random selection using sub-second timing
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize;
                available_indices[nanos % available_indices.len()]
            }
            PoolStrategy::LeastUsed => {
                // Pick credential with lowest usage_count
                let mut best = available_indices[0];
                let mut best_count = u64::MAX;
                for &idx in &available_indices {
                    if let Some(cred) = inner.credentials.get(&inner.ordered_ids[idx]) {
                        if cred.usage_count < best_count {
                            best_count = cred.usage_count;
                            best = idx;
                        }
                    }
                }
                best
            }
        };

        let id = &inner.ordered_ids[chosen_idx];
        if let Some(cred) = inner.credentials.get_mut(id) {
            cred.mark_used();
            debug!(
                provider = %self.provider,
                credential = %cred.name,
                strategy = ?self.strategy,
                usage = %cred.usage_count,
                "Selected credential from pool"
            );
            Some(cred.clone())
        } else {
            None
        }
    }
}

impl std::fmt::Debug for CredentialPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialPool")
            .field("provider", &self.provider)
            .field("strategy", &self.strategy)
            .field("credential_count", &self.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Free helper functions
// ---------------------------------------------------------------------------

/// Determine the appropriate exhaustion cooldown TTL based on HTTP status code.
pub fn exhausted_ttl(error_code: Option<i32>) -> u64 {
    match error_code {
        Some(401) => 300,  // 5 minutes — transient auth
        Some(429) => 3600, // 1 hour — rate limited
        Some(402) => 3600, // 1 hour — billing/quota
        _ => 3600,         // 1 hour — default
    }
}

/// Create a credential pool with the given strategy and pre-seed it from a
/// set of `(name, value, source)` tuples.
pub fn create_pool_from_entries(
    provider: &str,
    strategy: PoolStrategy,
    entries: Vec<(String, String, String)>,
) -> CredentialPool {
    let pool = CredentialPool::with_strategy(provider, strategy);
    for (name, value, source) in entries {
        let cred = PooledCredential::new(&name, AuthType::ApiKey, &value, &source);
        pool.add(cred);
    }
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pool() -> CredentialPool {
        let pool = CredentialPool::new("test-provider");
        let c1 = PooledCredential::new("key-1", AuthType::ApiKey, "val1", "manual");
        let c2 = PooledCredential::new("key-2", AuthType::ApiKey, "val2", "manual");
        pool.add(c1);
        pool.add(c2);
        pool
    }

    #[test]
    fn test_new_pool_is_empty() {
        let pool = CredentialPool::new("test");
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_add_and_list() {
        let pool = test_pool();
        assert_eq!(pool.len(), 2);
        let all = pool.list();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_get_by_id() {
        let pool = CredentialPool::new("test");
        let cred = PooledCredential::new("my-key", AuthType::ApiKey, "secret", "env:TEST_KEY");
        let id = pool.add(cred);
        let fetched = pool.get(&id);
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().name, "my-key");
    }

    #[test]
    fn test_get_nonexistent() {
        let pool = test_pool();
        assert!(pool.get("nonexistent-id").is_none());
    }

    #[test]
    fn test_remove() {
        let pool = test_pool();
        let id = {
            let all = pool.list();
            all[0].id.clone()
        };
        let removed = pool.remove(&id);
        assert!(removed.is_some());
        assert_eq!(pool.len(), 1);
        assert!(pool.get(&id).is_none());
    }

    #[test]
    fn test_remove_nonexistent() {
        let pool = test_pool();
        assert!(pool.remove("nonexistent").is_none());
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_select_fill_first() {
        let pool = test_pool();
        let selected = pool.select();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().name, "key-1");
    }

    #[test]
    fn test_select_from_empty_pool() {
        let pool = CredentialPool::new("empty");
        assert!(pool.select().is_none());
    }

    #[test]
    fn test_peek() {
        let pool = test_pool();
        let peeked = pool.peek();
        assert!(peeked.is_some());
    }

    #[test]
    fn test_peek_empty() {
        let pool = CredentialPool::new("empty");
        assert!(pool.peek().is_none());
    }

    #[test]
    fn test_has_credentials() {
        let pool = test_pool();
        assert!(pool.has_credentials());
        let empty = CredentialPool::new("empty");
        assert!(!empty.has_credentials());
    }

    #[test]
    fn test_has_available() {
        let pool = test_pool();
        assert!(pool.has_available());
    }

    #[test]
    fn test_invalidate_and_rotate() {
        let pool = test_pool();
        let id = {
            let all = pool.list();
            all[0].id.clone()
        };
        let next = pool.invalidate(
            &id,
            Some(429),
            Some("rate_limited"),
            Some("Too many requests"),
            true,
        );
        assert!(next.is_some());
        assert_eq!(next.unwrap().name, "key-2");
    }

    #[test]
    fn test_invalidate_all_exhausted() {
        let pool = test_pool();
        let all = pool.list();
        let id1 = all[0].id.clone();
        let id2 = all[1].id.clone();
        pool.invalidate(&id1, Some(429), Some("rate_limited"), Some("msg"), true);
        let next = pool.invalidate(&id2, Some(429), Some("rate_limited"), Some("msg"), true);
        assert!(next.is_none());
    }

    #[test]
    fn test_reset_statuses() {
        let pool = test_pool();
        let all = pool.list();
        pool.invalidate(&all[0].id, Some(500), Some("error"), Some("fail"), false);
        pool.invalidate(&all[1].id, Some(500), Some("error"), Some("fail"), false);
        assert_eq!(pool.reset_statuses(), 2);
        // Second reset should return 0
        assert_eq!(pool.reset_statuses(), 0);
    }

    #[test]
    fn test_update() {
        let pool = test_pool();
        let all = pool.list();
        let mut cred = all[0].clone();
        cred.name = "updated-name".to_string();
        assert!(pool.update(cred).is_ok());
        let fetched = pool.get(&all[0].id).unwrap();
        assert_eq!(fetched.name, "updated-name");
    }

    #[test]
    fn test_update_nonexistent() {
        let pool = test_pool();
        let cred = PooledCredential::new("ghost", AuthType::ApiKey, "val", "test");
        assert!(pool.update(cred).is_err());
    }

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(
            PoolStrategy::parse_strategy("fill_first"),
            PoolStrategy::FillFirst
        );
        assert_eq!(
            PoolStrategy::parse_strategy("fill-first"),
            PoolStrategy::FillFirst
        );
        assert_eq!(
            PoolStrategy::parse_strategy("round_robin"),
            PoolStrategy::RoundRobin
        );
        assert_eq!(
            PoolStrategy::parse_strategy("round-robin"),
            PoolStrategy::RoundRobin
        );
        assert_eq!(PoolStrategy::parse_strategy("random"), PoolStrategy::Random);
        assert_eq!(
            PoolStrategy::parse_strategy("least_used"),
            PoolStrategy::LeastUsed
        );
        assert_eq!(
            PoolStrategy::parse_strategy("least-used"),
            PoolStrategy::LeastUsed
        );
        assert_eq!(
            PoolStrategy::parse_strategy("unknown"),
            PoolStrategy::FillFirst
        );
    }

    #[test]
    fn test_credential_available() {
        let mut cred = PooledCredential::new("test", AuthType::ApiKey, "val", "src");
        assert!(cred.is_available());
        cred.status = Some(STATUS_EXHAUSTED.to_string());
        assert!(!cred.is_available());
        // Expire the cooldown
        cred.error_reset_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert!(cred.is_available());
    }

    #[test]
    fn test_credential_expired() {
        let mut cred = PooledCredential::new("test", AuthType::ApiKey, "val", "src");
        assert!(!cred.is_expired());
        cred.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert!(cred.is_expired());
    }

    #[test]
    fn test_mark_used() {
        let mut cred = PooledCredential::new("test", AuthType::ApiKey, "val", "src");
        assert_eq!(cred.usage_count, 0);
        cred.mark_used();
        assert_eq!(cred.usage_count, 1);
        cred.mark_used();
        assert_eq!(cred.usage_count, 2);
        assert!(cred.last_used_at.is_some());
    }

    #[test]
    fn test_different_strategies() {
        let pool = CredentialPool::with_strategy("test", PoolStrategy::RoundRobin);
        assert_eq!(pool.strategy(), PoolStrategy::RoundRobin);
    }

    #[test]
    fn test_set_strategy() {
        let mut pool = CredentialPool::new("test");
        assert_eq!(pool.strategy(), PoolStrategy::FillFirst);
        pool.set_strategy(PoolStrategy::LeastUsed);
        assert_eq!(pool.strategy(), PoolStrategy::LeastUsed);
    }

    #[test]
    fn test_debug_format() {
        let pool = test_pool();
        let debug = format!("{:?}", pool);
        assert!(debug.contains("CredentialPool"));
        assert!(debug.contains("test-provider"));
    }

    #[test]
    fn test_auth_type_default() {
        assert_eq!(AuthType::default(), AuthType::ApiKey);
    }

    #[test]
    fn test_strategy_default() {
        assert_eq!(PoolStrategy::default(), PoolStrategy::FillFirst);
    }

    #[test]
    fn test_auth_type_as_str() {
        assert_eq!(AuthType::ApiKey.as_str(), "api_key");
        assert_eq!(AuthType::OAuth.as_str(), "oauth");
        assert_eq!(AuthType::Basic.as_str(), "basic");
        assert_eq!(AuthType::Token.as_str(), "token");
        assert_eq!(AuthType::Custom.as_str(), "custom");
    }

    #[test]
    fn test_credential_default() {
        let cred = PooledCredential::default();
        assert!(!cred.id.is_empty());
        assert_eq!(cred.credential_type, AuthType::ApiKey);
    }

    #[test]
    fn test_serialize_deserialize() {
        let cred = PooledCredential::new("test", AuthType::OAuth, "token123", "oauth-provider");
        let json = serde_json::to_string(&cred).expect("serialize");
        let deserialized: PooledCredential = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.name, "test");
        assert_eq!(deserialized.credential_type, AuthType::OAuth);
        assert_eq!(deserialized.value, "token123");
    }

    #[test]
    fn test_exhausted_ttl() {
        assert_eq!(exhausted_ttl(Some(401)), 300);
        assert_eq!(exhausted_ttl(Some(429)), 3600);
        assert_eq!(exhausted_ttl(Some(402)), 3600);
        assert_eq!(exhausted_ttl(Some(500)), 3600);
        assert_eq!(exhausted_ttl(None), 3600);
    }

    #[test]
    fn test_seed_from_config() {
        let pool = CredentialPool::new("test");
        let id = pool.seed_from_config("my-key", "my-value", "config.yaml");
        assert!(!id.is_empty());
        let cred = pool.get(&id).unwrap();
        assert_eq!(cred.name, "my-key");
        assert_eq!(cred.value, "my-value");
    }

    #[test]
    fn test_create_pool_from_entries() {
        let entries = vec![
            (
                "key-a".to_string(),
                "val-a".to_string(),
                "env:A".to_string(),
            ),
            (
                "key-b".to_string(),
                "val-b".to_string(),
                "env:B".to_string(),
            ),
        ];
        let pool = create_pool_from_entries("test", PoolStrategy::Random, entries);
        assert_eq!(pool.len(), 2);
    }
}

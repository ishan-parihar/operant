//! OAuth token refresh for credential pool entries.
//!
//! Ported from operant-agent's `agent/credential_pool.py` and `operant_cli/auth.py`.
//! Supports Anthropic, Codex (OpenAI), xAI, and Nous Research OAuth flows.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::credential_pool::PooledCredential;
use crate::error::{Error, Result};

// ── Constants ─────────────────────────────────────────────────────────────

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_TOKEN_URLS: &[&str] = &[
    "https://platform.claude.com/v1/oauth/token",
    "https://console.anthropic.com/v1/oauth/token",
];
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const NOUS_CLIENT_ID: &str = "operant-cli";
const NOUS_PORTAL_URL: &str = "https://portal.nousresearch.com";
const NOUS_INFERENCE_URL: &str = "https://inference-api.nousresearch.com/v1";

// ── Auth Store Structures ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthStore {
    #[serde(default)]
    pub providers: HashMap<String, ProviderState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_key_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenState {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

// ── OAuth Response Types ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub id_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NousOAuthState {
    pub access_token: String,
    pub refresh_token: String,
    pub client_id: String,
    pub portal_base_url: String,
    pub inference_base_url: String,
    pub token_type: String,
    pub agent_key: Option<String>,
    pub agent_key_expires_at: Option<DateTime<Utc>>,
    pub obtained_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ── OAuthRefresher ────────────────────────────────────────────────────────

pub struct OAuthRefresher {
    client: reqwest::Client,
    auth_store_path: PathBuf,
}

impl OAuthRefresher {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| Error::Config(format!("Failed to build HTTP client: {e}")))?;

        let mut auth_store_path = dirs::home_dir()
            .ok_or_else(|| Error::Config("Cannot determine home directory".to_string()))?;
        auth_store_path.push(".operant");
        auth_store_path.push("auth.json");

        Ok(Self {
            client,
            auth_store_path,
        })
    }

    pub fn with_auth_store_path(path: &Path) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| Error::Config(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self {
            client,
            auth_store_path: path.to_path_buf(),
        })
    }

    /// Refresh OAuth tokens for a credential entry.
    pub async fn refresh(
        &self,
        provider: &str,
        entry: &PooledCredential,
    ) -> Result<OAuthTokenResponse> {
        let refresh_token = entry.refresh_token.as_ref().ok_or_else(|| {
            Error::Authentication(format!(
                "No refresh token for OAuth credential '{}'",
                entry.name
            ))
        })?;

        match provider.to_lowercase().as_str() {
            "anthropic" => self.refresh_anthropic(refresh_token, entry).await,
            "openai-codex" | "codex" => self.refresh_codex(refresh_token).await,
            "xai-oauth" | "xai" => self.refresh_xai(refresh_token, entry).await,
            "nous" => self.refresh_nous(entry).await,
            other => Err(Error::Config(format!(
                "Unsupported OAuth provider for refresh: {other}"
            ))),
        }
    }

    async fn refresh_anthropic(
        &self,
        refresh_token: &str,
        entry: &PooledCredential,
    ) -> Result<OAuthTokenResponse> {
        let use_json = entry.source.ends_with("operant_pkce");
        let body = if use_json {
            serde_json::json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": ANTHROPIC_CLIENT_ID,
            })
            .to_string()
        } else {
            format!(
                "grant_type=refresh_token&refresh_token={}&client_id={}",
                urlencoding::encode(refresh_token),
                ANTHROPIC_CLIENT_ID,
            )
        };
        let content_type = if use_json {
            "application/json"
        } else {
            "application/x-www-form-urlencoded"
        };

        let mut last_error = None;
        for endpoint in ANTHROPIC_TOKEN_URLS {
            match self
                .client
                .post(*endpoint)
                .header("Content-Type", content_type)
                .body(body.clone())
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let result: serde_json::Value = resp.json().await.map_err(|e| {
                        Error::ParseResponse(format!("Anthropic refresh invalid JSON: {e}"))
                    })?;

                    let access_token = result
                        .get("access_token")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            Error::Authentication(
                                "Anthropic refresh response missing access_token".to_string(),
                            )
                        })?;

                    let next_refresh = result
                        .get("refresh_token")
                        .and_then(|v| v.as_str())
                        .unwrap_or(refresh_token)
                        .to_string();

                    let expires_in = result.get("expires_in").and_then(|v| v.as_u64());

                    return Ok(OAuthTokenResponse {
                        access_token: access_token.to_string(),
                        refresh_token: next_refresh,
                        expires_in,
                        token_type: Some("Bearer".to_string()),
                        id_token: None,
                    });
                }
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    debug!("Anthropic token refresh failed at {endpoint}: {status} {text}");
                    last_error = Some(Error::Authentication(format!(
                        "Anthropic refresh failed at {endpoint}: {status}"
                    )));
                }
                Err(e) => {
                    debug!("Anthropic token refresh error at {endpoint}: {e}");
                    last_error = Some(Error::Network(e));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            Error::Authentication(
                "Anthropic token refresh failed — all endpoints exhausted".to_string(),
            )
        }))
    }

    async fn refresh_codex(&self, refresh_token: &str) -> Result<OAuthTokenResponse> {
        if refresh_token.trim().is_empty() {
            return Err(Error::Authentication(
                "Codex auth is missing refresh_token. Re-authenticate with `operant auth`."
                    .to_string(),
            ));
        }

        let resp = self
            .client
            .post(CODEX_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(format!(
                "grant_type=refresh_token&refresh_token={}&client_id={}",
                urlencoding::encode(refresh_token),
                CODEX_CLIENT_ID,
            ))
            .send()
            .await
            .map_err(Error::Network)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let mut code = "codex_refresh_failed".to_string();
            let mut message = format!("Codex token refresh failed with status {status}.");
            let mut _relogin_required = false;

            if let Ok(err) = serde_json::from_str::<serde_json::Value>(&text)
                && let Some(err_obj) = err.get("error")
            {
                if let Some(obj) = err_obj.as_object() {
                    if let Some(nested_code) = obj
                        .get("code")
                        .or_else(|| obj.get("type"))
                        .and_then(|v| v.as_str())
                    {
                        code = nested_code.to_string();
                    }
                    if let Some(nested_msg) = obj.get("message").and_then(|v| v.as_str()) {
                        message = format!("Codex token refresh failed: {nested_msg}");
                    }
                } else if let Some(code_str) = err_obj.as_str() {
                    code = code_str.to_string();
                    if let Some(desc) = err.get("error_description").and_then(|v| v.as_str()) {
                        message = format!("Codex token refresh failed: {desc}");
                    }
                }
            }

            if matches!(
                code.as_str(),
                "invalid_grant" | "invalid_token" | "invalid_request"
            ) {
                _relogin_required = true;
            }
            if code == "refresh_token_reused" {
                message =
                    "Codex refresh token was already consumed by another client. Re-authenticate."
                        .to_string();
                _relogin_required = true;
            }
            if status.as_u16() == 401 || status.as_u16() == 403 {
                _relogin_required = true;
            }

            return Err(Error::Authentication(format!("{message} [code={code}]")));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::ParseResponse(format!("Codex refresh invalid JSON: {e}")))?;

        let access_token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::Authentication("Codex refresh response missing access_token".to_string())
            })?;

        Ok(OAuthTokenResponse {
            access_token: access_token.to_string(),
            refresh_token: payload
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .unwrap_or(refresh_token)
                .to_string(),
            expires_in: payload.get("expires_in").and_then(|v| v.as_u64()),
            token_type: payload
                .get("token_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            id_token: payload
                .get("id_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    async fn refresh_xai(
        &self,
        refresh_token: &str,
        entry: &PooledCredential,
    ) -> Result<OAuthTokenResponse> {
        if refresh_token.trim().is_empty() {
            return Err(Error::Authentication(
                "xAI OAuth is missing refresh_token. Re-authenticate.".to_string(),
            ));
        }

        let token_endpoint = entry
            .token_endpoint
            .as_deref()
            .unwrap_or("https://accounts.x.ai/v1/oauth/token");

        if !token_endpoint.contains("x.ai") && !token_endpoint.contains("xai") {
            return Err(Error::Config(format!(
                "xAI token_endpoint does not appear to be an xAI endpoint: {token_endpoint}"
            )));
        }

        let resp = self
            .client
            .post(token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(format!(
                "grant_type=refresh_token&refresh_token={}&client_id={}",
                urlencoding::encode(refresh_token),
                XAI_CLIENT_ID,
            ))
            .send()
            .await
            .map_err(Error::Network)?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(Error::Authentication(format!(
                "xAI token refresh failed with status {status}. Re-authenticate required."
            )));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::ParseResponse(format!("xAI refresh invalid JSON: {e}")))?;

        let access_token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::Authentication("xAI refresh response missing access_token".to_string())
            })?;

        Ok(OAuthTokenResponse {
            access_token: access_token.to_string(),
            refresh_token: payload
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .unwrap_or(refresh_token)
                .to_string(),
            expires_in: payload.get("expires_in").and_then(|v| v.as_u64()),
            token_type: payload
                .get("token_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some("Bearer".to_string())),
            id_token: payload
                .get("id_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    async fn refresh_nous(&self, entry: &PooledCredential) -> Result<OAuthTokenResponse> {
        let refresh_token = entry
            .refresh_token
            .as_deref()
            .ok_or_else(|| Error::Authentication("Nous OAuth missing refresh_token".to_string()))?;

        let client_id = entry.client_id.as_deref().unwrap_or(NOUS_CLIENT_ID);
        let portal_base_url = entry
            .portal_base_url
            .as_deref()
            .unwrap_or(NOUS_PORTAL_URL)
            .trim_end_matches('/');

        let token_url = format!("{portal_base_url}/api/oauth/token");

        let resp = self
            .client
            .post(&token_url)
            .header("x-nous-refresh-token", refresh_token)
            .header("Accept", "application/json")
            .form(&[("grant_type", "refresh_token"), ("client_id", client_id)])
            .send()
            .await
            .map_err(Error::Network)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let mut message = "Nous token refresh failed.".to_string();
            let mut relogin = false;

            if let Ok(err) = serde_json::from_str::<serde_json::Value>(&text) {
                let code = err
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("invalid_grant");
                let desc = err
                    .get("error_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Token refresh failed");
                message = format!("Nous token refresh failed: {desc}");
                if matches!(code, "invalid_grant" | "invalid_token") {
                    relogin = true;
                }
                if desc.to_lowercase().contains("reuse") {
                    message = "Nous Portal detected refresh-token reuse. Re-authenticate with `operant auth add nous`.".to_string();
                    relogin = true;
                }
            }

            if status.as_u16() == 401 || status.as_u16() == 403 {
                relogin = true;
            }

            let mut err_msg = message;
            if relogin {
                err_msg.push_str(" Re-authenticate required.");
            }
            return Err(Error::Authentication(err_msg));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| Error::ParseResponse(format!("Nous refresh invalid JSON: {e}")))?;

        let access_token = payload
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Error::Authentication("Nous refresh response missing access_token".to_string())
            })?;

        let _inference_base_url = payload
            .get("inference_base_url")
            .and_then(|v| v.as_str())
            .unwrap_or(NOUS_INFERENCE_URL);

        let expires_in = payload.get("expires_in").and_then(|v| v.as_u64());

        info!(
            provider = "nous",
            "Nous OAuth access token refreshed successfully"
        );

        let _ = self
            .mint_nous_agent_key(access_token, portal_base_url)
            .await;

        Ok(OAuthTokenResponse {
            access_token: access_token.to_string(),
            refresh_token: payload
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .unwrap_or(refresh_token)
                .to_string(),
            expires_in,
            token_type: payload
                .get("token_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some("Bearer".to_string())),
            id_token: None,
        })
    }

    async fn mint_nous_agent_key(&self, access_token: &str, portal_base_url: &str) -> Result<()> {
        let mint_url = format!("{portal_base_url}/api/keys/agent");

        let resp = self
            .client
            .post(&mint_url)
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(Error::Network)?;

        if resp.status().is_success() {
            debug!("Nous agent key minted successfully");
            Ok(())
        } else {
            warn!("Nous agent key mint failed: {}", resp.status());
            Err(Error::Authentication(format!(
                "Agent key mint failed: {}",
                resp.status()
            )))
        }
    }

    /// Sync credential tokens from auth.json (handles single-use refresh token race).
    pub fn sync_from_auth_store(
        &self,
        provider: &str,
        entry: &PooledCredential,
    ) -> Option<PooledCredential> {
        let store = load_auth_store(&self.auth_store_path).ok()?;
        let state = store.providers.get(provider)?;

        let mut updated = entry.clone();

        match provider {
            "anthropic" => {
                if let Some(tokens) = &state.tokens {
                    if tokens.access_token != entry.value {
                        debug!("Adopting newer Anthropic access token from auth store");
                        updated.value = tokens.access_token.clone();
                    }
                    if let Some(rt) = &tokens.refresh_token
                        && Some(rt) != entry.refresh_token.as_ref()
                    {
                        updated.refresh_token = Some(rt.clone());
                    }
                }
            }
            "openai-codex" | "codex" => {
                if let Some(tokens) = &state.tokens {
                    if tokens.access_token != entry.value {
                        debug!("Adopting newer Codex access token from auth store");
                        updated.value = tokens.access_token.clone();
                    }
                    if let Some(rt) = &tokens.refresh_token
                        && Some(rt) != entry.refresh_token.as_ref()
                    {
                        updated.refresh_token = Some(rt.clone());
                    }
                }
                if let Some(lr) = &state.last_refresh {
                    updated.last_refresh = Some(lr.clone());
                }
            }
            "xai-oauth" | "xai" => {
                if let Some(tokens) = &state.tokens {
                    if tokens.access_token != entry.value {
                        debug!("Adopting newer xAI access token from auth store");
                        updated.value = tokens.access_token.clone();
                    }
                    if let Some(rt) = &tokens.refresh_token
                        && Some(rt) != entry.refresh_token.as_ref()
                    {
                        updated.refresh_token = Some(rt.clone());
                    }
                }
                if let Some(te) = &state.token_endpoint {
                    updated.token_endpoint = Some(te.clone());
                }
            }
            "nous" => {
                if let Some(at) = &state.access_token
                    && at != &entry.value
                {
                    debug!("Adopting newer Nous access token from auth store");
                    updated.value = at.clone();
                }
                if let Some(rt) = &state.refresh_token
                    && Some(rt) != entry.refresh_token.as_ref()
                {
                    updated.refresh_token = Some(rt.clone());
                }
                if let Some(ak) = &state.agent_key {
                    updated.agent_key = Some(ak.clone());
                }
                if let Some(url) = &state.inference_base_url {
                    updated.inference_base_url = Some(url.clone());
                }
            }
            _ => return None,
        }

        Some(updated)
    }

    /// Persist refreshed tokens back to auth.json.
    pub fn persist_to_auth_store(
        &self,
        provider: &str,
        entry: &PooledCredential,
        response: &OAuthTokenResponse,
    ) -> Result<()> {
        let mut store = load_auth_store(&self.auth_store_path).unwrap_or_default();

        let state = store.providers.entry(provider.to_string()).or_default();

        match provider {
            "anthropic" => {
                state.tokens = Some(TokenState {
                    access_token: response.access_token.clone(),
                    refresh_token: Some(response.refresh_token.clone()),
                    id_token: None,
                });
            }
            "openai-codex" | "codex" => {
                state.tokens = Some(TokenState {
                    access_token: response.access_token.clone(),
                    refresh_token: Some(response.refresh_token.clone()),
                    id_token: response.id_token.clone(),
                });
                let now = Utc::now().to_rfc3339();
                state.last_refresh = Some(now);
            }
            "xai-oauth" | "xai" => {
                state.tokens = Some(TokenState {
                    access_token: response.access_token.clone(),
                    refresh_token: Some(response.refresh_token.clone()),
                    id_token: response.id_token.clone(),
                });
                let now = Utc::now().to_rfc3339();
                state.last_refresh = Some(now);
            }
            "nous" => {
                state.access_token = Some(response.access_token.clone());
                state.refresh_token = Some(response.refresh_token.clone());
                if let Some(ak) = &entry.agent_key {
                    state.agent_key = Some(ak.clone());
                }
                if let Some(url) = &entry.inference_base_url {
                    state.inference_base_url = Some(url.clone());
                }
            }
            _ => {}
        }

        save_auth_store(&self.auth_store_path, &store)
    }
}

// ── Auth Store I/O ────────────────────────────────────────────────────────

pub fn auth_store_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_default();
    path.push(".operant");
    path.push("auth.json");
    path
}

pub fn load_auth_store(path: &Path) -> Result<AuthStore> {
    let content = fs::read_to_string(path).map_err(Error::Io)?;
    let store: AuthStore = serde_json::from_str(&content)
        .map_err(|e| Error::ParseResponse(format!("auth.json parse error: {e}")))?;
    Ok(store)
}

pub fn save_auth_store(path: &Path, store: &AuthStore) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(Error::Io)?;
    }

    let json = serde_json::to_string_pretty(store)
        .map_err(|e| Error::ParseResponse(format!("auth.json serialize error: {e}")))?;

    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(Error::Io)?;
    std::io::Write::write_all(&mut tmp, json.as_bytes()).map_err(Error::Io)?;
    tmp.persist(path).map_err(|e| Error::Io(e.into()))?;

    Ok(())
}

// ── OAuth URL Encoding Helper ─────────────────────────────────────────────

mod urlencoding {
    pub fn encode(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                ' ' => "%20".to_string(),
                _ => {
                    let mut encoded = String::new();
                    for byte in c.to_string().as_bytes() {
                        encoded.push_str(&format!("%{:02X}", byte));
                    }
                    encoded
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_store_roundtrip() {
        let store = AuthStore {
            providers: HashMap::from([(
                "anthropic".to_string(),
                ProviderState {
                    tokens: Some(TokenState {
                        access_token: "test-token".to_string(),
                        refresh_token: Some("test-refresh".to_string()),
                        id_token: None,
                    }),
                    ..Default::default()
                },
            )]),
            active_provider: Some("anthropic".to_string()),
        };

        let json = serde_json::to_string(&store).unwrap();
        let parsed: AuthStore = serde_json::from_str(&json).unwrap();
        assert!(parsed.providers.contains_key("anthropic"));
    }

    #[test]
    fn test_urlencoding_basic() {
        assert_eq!(urlencoding::encode("hello"), "hello");
        assert_eq!(urlencoding::encode("hello world"), "hello%20world");
    }

    #[test]
    fn test_oauth_refresher_path() {
        let path = auth_store_path();
        assert!(path.to_string_lossy().contains("auth.json"));
    }
}

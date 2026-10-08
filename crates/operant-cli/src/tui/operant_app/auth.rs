// Vendored from jcode (crates/operant-base/src/auth/mod.rs + auth/active_method.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see operant_app/mod.rs for scope.
//! Included: AUTH_STATUS_GENERATION + auth_status_generation (:82) +
//! bump_auth_status_generation_for_tests (:91) from auth/mod.rs; ActiveCredential
//! (:21) from auth/active_method.rs. [port-excision] upstream's
//! `impl From<operant_provider_core::ResolvedCredential> for ActiveCredential`
//! (active_method.rs:29-:36) is kept re-rooted to operant_app::provider.

/// Current auth-status generation; see [`AUTH_STATUS_GENERATION`].
pub fn auth_status_generation() -> u64 {
    AUTH_STATUS_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bump the auth generation without clearing the cached status.
///
/// Tests that only need to observe generation-driven invalidation use this so
/// they do not evict the process-global auth cache that sibling tests in the
/// same binary rely on.
pub fn bump_auth_status_generation_for_tests() {
    AUTH_STATUS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// The credential a request will actually be sent with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveCredential {
    /// OAuth / subscription login (Claude subscription, Codex login, ...).
    OAuth,
    /// Direct provider API key.
    ApiKey,
}

impl From<crate::tui::operant_app::provider::ResolvedCredential> for ActiveCredential {
    fn from(value: crate::tui::operant_app::provider::ResolvedCredential) -> Self {
        match value {
            crate::tui::operant_app::provider::ResolvedCredential::Oauth => Self::OAuth,
            crate::tui::operant_app::provider::ResolvedCredential::ApiKey => Self::ApiKey,
        }
    }
}

// ─── [port-decision] added at batch-3: TuiState trait dependency ─────────────
// tui_state.rs is the cutover App-seam contract (TuiState). Its signatures and
// the already-landed ui_header.rs tests need these symbols from this module:
// AuthState — crates/operant-auth-types/src/lib.rs:3-24, ported verbatim.
// AUTH_STATUS_GENERATION — crates/operant-base/src/auth/mod.rs:73-79, ported
//   verbatim; the landed auth_status_generation()/bump_ functions above
//   reference this static but the static itself was missing from the file.
// AuthStatus + ProviderAuth — crates/operant-base/src/auth/status_types.rs:8-75,
//   ported verbatim; upstream's `pub use operant_auth_types::{...}` re-export
//   line is collapsed to the one re-exported type actually in this closure
//   (AuthState, ported in full below).
use serde::{Deserialize, Serialize};

/// Bumped whenever the cached auth status is invalidated.
///
/// Lets downstream caches (notably the TUI's prepared-header cache) key on
/// "have credentials changed" without re-running the expensive probes
/// themselves, so a credential change repaints immediately instead of waiting
/// out an unrelated TTL.
static AUTH_STATUS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// State of a single auth credential
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthState {
    /// Credential is available and valid
    Available,
    /// Partial configuration exists (or OAuth may be expired)
    Expired,
    /// Credential is not configured
    #[default]
    NotConfigured,
}

impl AuthState {
    /// Short, stable, log-friendly label for this credential state.
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Expired => "expired",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// Cached low-level authentication snapshot for all supported providers.
///
/// This is the probe/cache substrate. New CLI and UI surfaces should prefer
/// `AuthStatus::assessment_for_provider`, which normalizes these raw fields into
/// the canonical provider auth contract (`ProviderAuthAssessment`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthStatus {
    /// Operant subscription router credentials
    pub subscription: AuthState,
    /// Anthropic provider (Claude models) - via OAuth or API key
    pub anthropic: ProviderAuth,
    /// OpenRouter provider - via API key
    pub openrouter: AuthState,
    /// Azure OpenAI provider - via Entra ID or API key
    pub azure: AuthState,
    /// AWS Bedrock provider - via Bedrock API key or AWS credentials
    pub bedrock: AuthState,
    /// OpenAI provider - via OAuth or API key
    pub openai: AuthState,
    /// OpenAI has OAuth credentials
    pub openai_has_oauth: bool,
    /// OpenAI OAuth credential state alone (ignores any API key). Lets the
    /// OAuth/subscription login surface report honestly instead of borrowing
    /// the API key's availability.
    pub openai_oauth_state: AuthState,
    /// OpenAI has API key available
    pub openai_has_api_key: bool,
    /// Azure OpenAI has API key available
    pub azure_has_api_key: bool,
    /// Azure OpenAI is configured for Entra ID authentication
    pub azure_uses_entra: bool,
    /// Copilot API available (GitHub OAuth token found)
    pub copilot: AuthState,
    /// Copilot has API token (from hosts.json/apps.json/GITHUB_TOKEN)
    pub copilot_has_api_token: bool,
    /// Antigravity OAuth configured
    pub antigravity: AuthState,
    /// Gemini CLI available
    pub gemini: AuthState,
    /// Cursor provider configured via Cursor Agent plus API key or CLI session
    pub cursor: AuthState,
    /// Grok Build CLI is installed. Runtime auth is delegated to its cached login.
    pub grok_build: AuthState,
    /// Any OpenAI-compatible catalog profile (Cerebras, Groq, ...) has usable
    /// credentials. These have no dedicated field and, since e0796a51c, no
    /// longer count toward the native `openrouter` slot.
    #[serde(default)]
    pub openai_compatible_any: AuthState,
    /// Google/Gmail OAuth configured
    pub google: AuthState,
    /// Google Gmail has send capability (Full tier)
    pub google_can_send: bool,
}

/// Auth state for Anthropic which has multiple auth methods
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProviderAuth {
    /// Overall state (best of available methods)
    pub state: AuthState,
    /// Has OAuth credentials
    pub has_oauth: bool,
    /// OAuth credential state alone (ignores any API key). Lets the
    /// OAuth/subscription login surface report honestly instead of borrowing
    /// the API key's availability.
    pub oauth_state: AuthState,
    /// Has API key
    pub has_api_key: bool,
}

// [port-decision] leaf port: ResolvedProviderAuth + resolve_dual_credential_auth
// verbatim from operant-base/src/auth/active_method.rs:38-119 (the header-status
// dual-auth arm reads it); upstream paths re-rooted: operant_provider_core::
// {DualAuthProvider,AuthMode,pinned_mode_for} -> crate::tui::operant_app::provider.
/// Resolved auth picture for a provider that supports both OAuth and API-key
/// credentials (currently Anthropic and OpenAI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedProviderAuth {
    /// Credential the next request will actually use.
    pub active: ActiveCredential,
    /// Whether an OAuth credential is configured at all.
    pub has_oauth: bool,
    /// Whether an API key is configured at all.
    pub has_api_key: bool,
    /// True when the active credential was pinned via `OPERANT_RUNTIME_PROVIDER`
    /// rather than chosen by the auto heuristic. Lets surfaces distinguish a
    /// deliberate "use OAuth" from "auto happened to pick OAuth".
    pub explicit: bool,
}

impl ResolvedProviderAuth {
    /// True when both credential types are configured (the request still uses
    /// exactly one, reported by [`Self::active`]).
    pub fn has_both(&self) -> bool {
        self.has_oauth && self.has_api_key
    }
}

/// Resolve the credential a dual-auth provider (Anthropic / OpenAI) will use.
///
/// `runtime_provider` is the raw `OPERANT_RUNTIME_PROVIDER` value (if any); it
/// lets the user pin OAuth-vs-key explicitly and always wins over the auto
/// heuristic. In "auto" mode we prefer OAuth (subscription) and fall back to the
/// API key, matching the credential the provider layer actually selects.
///
/// Returns `None` for providers without OAuth-vs-key ambiguity, or when neither
/// credential is configured.
pub fn resolve_dual_credential_auth(
    provider: crate::tui::operant_app::provider::ActiveProvider,
    auth: &AuthStatus,
    runtime_provider: Option<&str>,
) -> Option<ResolvedProviderAuth> {
    // Map the execution slot onto the canonical dual-auth provider. Anything
    // without an OAuth-vs-API decision (Copilot, Gemini, ...) returns None.
    let dual = crate::tui::operant_app::provider::DualAuthProvider::from_active_provider(provider)?;

    // A single canonical parser decides whether `runtime_provider` explicitly
    // pins OAuth or API key for *this* provider. This replaces the per-provider
    // hand-written alias matches that used to drift apart.
    let forced =
        crate::tui::operant_app::provider::pinned_mode_for(dual, runtime_provider).map(|mode| {
            match mode {
                crate::tui::operant_app::provider::AuthMode::Oauth => ActiveCredential::OAuth,
                crate::tui::operant_app::provider::AuthMode::ApiKey => ActiveCredential::ApiKey,
            }
        });

    let (has_oauth, has_api_key) = match dual {
        crate::tui::operant_app::provider::DualAuthProvider::Anthropic => {
            let has_oauth = auth.anthropic.has_oauth;
            // `has_api_key` already folds in the ANTHROPIC_API_KEY env var via the
            // auth probe, but re-check defensively so an env-only key set after the
            // cached snapshot still reports honestly.
            let has_api_key =
                auth.anthropic.has_api_key || std::env::var("ANTHROPIC_API_KEY").is_ok();
            (has_oauth, has_api_key)
        }
        crate::tui::operant_app::provider::DualAuthProvider::OpenAI => {
            (auth.openai_has_oauth, auth.openai_has_api_key)
        }
    };

    let active = match forced {
        // An explicit selection wins outright: it reflects what requests will use
        // even if the matching credential probe is momentarily stale.
        Some(kind) => kind,
        None if has_oauth => ActiveCredential::OAuth,
        None if has_api_key => ActiveCredential::ApiKey,
        None => return None,
    };

    Some(ResolvedProviderAuth {
        active,
        has_oauth,
        has_api_key,
        explicit: forced.is_some(),
    })
}

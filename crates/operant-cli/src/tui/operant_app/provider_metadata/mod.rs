// Vendored from jcode (crates/operant-provider-metadata/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial:
// the type definitions (:1-:145), the catalog re-exports (:145-:148),
// openai_compatible_profiles (:150) + login_providers (:154), and the
// provider-name resolution fns resolve_login_provider (:188),
// resolve_login_provider_loose (:202) + normalize_provider_input (:304).
// [port-excision] the surface-filter helpers (:158-:186),
// resolve_login_selection (:213), is_safe_env_key_name/is_safe_env_file_name
// (:232-:241), normalize_api_base (:248) + allows_insecure_http_host (:270)
// (url-crate dep), and the test module (:312+) are unreferenced by the
// ported tree. Re-activate with the provider-subsystem cutover.
// See operant_app/mod.rs for scope.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginProviderAuthKind {
    OAuth,
    ApiKey,
    DeviceCode,
    Cli,
    Hybrid,
    Local,
}

impl LoginProviderAuthKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OAuth => "OAuth",
            Self::ApiKey => "API key",
            Self::DeviceCode => "device code",
            Self::Cli => "CLI",
            Self::Hybrid => "API key / CLI",
            Self::Local => "local endpoint",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginProviderTarget {
    AutoImport,
    Jcode,
    Claude,
    ClaudeApiKey,
    OpenAi,
    OpenAiApiKey,
    OpenRouter,
    Bedrock,
    Azure,
    OpenAiCompatible(OpenAiCompatibleProfile),
    Cursor,
    GrokBuild,
    Copilot,
    Gemini,
    Antigravity,
    Google,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginProviderAuthStateKey {
    ExternalImport,
    Jcode,
    Anthropic,
    OpenAi,
    Azure,
    Bedrock,
    OpenRouterLike,
    Copilot,
    Gemini,
    Antigravity,
    Cursor,
    GrokBuild,
    Google,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginProviderSurface {
    CliLogin,
    TuiLogin,
    ServerBootstrap,
    AutoInit,
    AuthStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginProviderSurfaceOrder {
    pub cli_login: Option<u8>,
    pub tui_login: Option<u8>,
    pub server_bootstrap: Option<u8>,
    pub auto_init: Option<u8>,
    pub auth_status: Option<u8>,
}

impl LoginProviderSurfaceOrder {
    pub const fn new(
        cli_login: Option<u8>,
        tui_login: Option<u8>,
        server_bootstrap: Option<u8>,
        auto_init: Option<u8>,
        auth_status: Option<u8>,
    ) -> Self {
        Self {
            cli_login,
            tui_login,
            server_bootstrap,
            auto_init,
            auth_status,
        }
    }

    pub const fn for_surface(self, surface: LoginProviderSurface) -> Option<u8> {
        match surface {
            LoginProviderSurface::CliLogin => self.cli_login,
            LoginProviderSurface::TuiLogin => self.tui_login,
            LoginProviderSurface::ServerBootstrap => self.server_bootstrap,
            LoginProviderSurface::AutoInit => self.auto_init,
            LoginProviderSurface::AuthStatus => self.auth_status,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginProviderDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub auth_kind: LoginProviderAuthKind,
    pub auth_state_key: LoginProviderAuthStateKey,
    pub auth_status_method: &'static str,
    pub aliases: &'static [&'static str],
    pub menu_detail: &'static str,
    pub recommended: bool,
    pub target: LoginProviderTarget,
    pub order: LoginProviderSurfaceOrder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenAiCompatibleProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    pub api_base: &'static str,
    pub api_key_env: &'static str,
    pub env_file: &'static str,
    pub setup_url: &'static str,
    pub default_model: Option<&'static str>,
    pub requires_api_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedOpenAiCompatibleProfile {
    pub id: String,
    pub display_name: String,
    pub api_base: String,
    pub api_key_env: String,
    pub env_file: String,
    pub setup_url: String,
    pub default_model: Option<String>,
    pub requires_api_key: bool,
}

mod catalog;
use catalog::{LOGIN_PROVIDERS, OPENAI_COMPAT_PROFILES};

pub fn openai_compatible_profiles() -> &'static [OpenAiCompatibleProfile] {
    &OPENAI_COMPAT_PROFILES
}

pub fn login_providers() -> &'static [LoginProviderDescriptor] {
    &LOGIN_PROVIDERS
}

pub fn resolve_login_provider(input: &str) -> Option<LoginProviderDescriptor> {
    let normalized = normalize_provider_input(input)?;
    login_providers().iter().copied().find(|provider| {
        provider.id == normalized || provider.aliases.iter().any(|alias| *alias == normalized)
    })
}

/// Resolve a login provider by id, alias, or display name.
///
/// Login completion events carry the human-readable provider label (e.g.
/// "Anthropic API") rather than the canonical id/alias, so the stricter
/// [`resolve_login_provider`] (id/alias only) misses them. Auth-change routing
/// needs to map those labels back to a provider id; matching the display name
/// here keeps the post-login model refresh attributed to the correct provider.
pub fn resolve_login_provider_loose(input: &str) -> Option<LoginProviderDescriptor> {
    if let Some(provider) = resolve_login_provider(input) {
        return Some(provider);
    }
    let normalized = normalize_provider_input(input)?;
    login_providers()
        .iter()
        .copied()
        .find(|provider| provider.display_name.to_ascii_lowercase() == normalized)
}

fn normalize_provider_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

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

fn normalize_provider_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

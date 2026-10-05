// Vendored from jcode (crates/jcode-provider-core/src/{lib.rs, selection.rs,
// auth_mode.rs, model_names.rs}), MIT License, Copyright (c) 2025 Jeremy Huang.
// Ported verbatim @ 0a9dc7805; partial — see jcode_app/mod.rs for scope.
//! Included from lib.rs: ResolvedCredential (:1216, plus its impl),
//! ModelRouteApiMethod (:898, plus its whole impl block at :917). Included from
//! selection.rs: ActiveProvider (:5). Included from auth_mode.rs: the whole
//! DualAuthProvider / AuthMode / AuthRoute trio + impls (:29-:238 region).
//! Included from model_names.rs: pretty_model_display_name (:29),
//! pretty_known_model_family (:272). Included from models.rs: DEFAULT_CONTEXT_LIMIT
//! (:125). [port-excision] the rest of jcode-provider-core (provider traits,
//! catalog, failover) is not ported. Upstream `crate::selection::`/
//! `crate::auth_mode::` re-root to this module.
use serde::{Deserialize, Serialize};

// [port-decision] batch-4: bus.rs's publish_models_updated calls
// `crate::provider::catalog_scheduler::bump_catalog_generation` (upstream
// jcode-base/src/bus.rs:157); re-export the ported catalog_scheduler module
// here so that re-rooted path resolves without touching bus.rs's call site.
pub use super::catalog_scheduler;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveProvider {
    Claude,
    OpenAI,
    Copilot,
    Antigravity,
    Gemini,
    Cursor,
    Bedrock,
    OpenRouter,
}

/// The credential a dual-auth provider (Anthropic / OpenAI) will actually use
/// for the next request. This is the authoritative billing identity: `Oauth`
/// means subscription usage, `ApiKey` means cost-based usage. It is resolved
/// once, server-side, from the provider's live credential mode and shipped to
/// remote clients so they never have to re-derive it from a provider name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedCredential {
    /// OAuth / subscription login (Claude subscription, Codex login, ...).
    Oauth,
    /// Direct provider API key (cost-based billing).
    ApiKey,
}

// --- auth_mode.rs ----------------------------------------------------------------

// here instead of re-deriving the decision from ad-hoc string matches.

// [port-decision] dedup: two upstream concatenated sources (provider-core lib.rs
// and auth_mode.rs region) both imported these; ResolvedCredential and
// ActiveProvider are defined in this file (:17/:36), so the imports are dropped.

/// A provider that supports *both* a subscription/OAuth login and a direct
/// API-key credential, and therefore needs an explicit OAuth-vs-API decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DualAuthProvider {
    /// Anthropic / Claude (Claude subscription OAuth vs `ANTHROPIC_API_KEY`).
    Anthropic,
    /// OpenAI (ChatGPT/Codex OAuth vs `OPENAI_API_KEY`).
    OpenAI,
}

impl DualAuthProvider {
    /// The dual-auth provider backing an [`ActiveProvider`], if any. Returns
    /// `None` for providers with no OAuth-vs-API-key ambiguity.
    pub const fn from_active_provider(provider: ActiveProvider) -> Option<Self> {
        match provider {
            ActiveProvider::Claude => Some(Self::Anthropic),
            ActiveProvider::OpenAI => Some(Self::OpenAI),
            _ => None,
        }
    }

    /// The execution slot this credential decision routes through.
    pub const fn active_provider(self) -> ActiveProvider {
        match self {
            Self::Anthropic => ActiveProvider::Claude,
            Self::OpenAI => ActiveProvider::OpenAI,
        }
    }

    /// Model prefix that routes to this provider *without* pinning a
    /// credential, leaving the runtime in automatic mode (prefer OAuth, fall
    /// back to an API key). This is the counterpart to
    /// [`AuthRoute::model_prefix`], which always pins one credential.
    ///
    /// `AuthRoute::parse_explicit_credential_prefix` treats exactly these
    /// prefixes as non-pinning, so the two stay in lockstep.
    pub const fn bare_model_prefix(self) -> &'static str {
        match self {
            Self::Anthropic => "claude",
            Self::OpenAI => "openai",
        }
    }
}

/// Which credential a dual-auth provider will actually use for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuthMode {
    /// OAuth / subscription login (Claude subscription, ChatGPT/Codex login).
    Oauth,
    /// Direct provider API key (metered / cost-based billing).
    ApiKey,
}

impl AuthMode {
    /// True when requests bill against a subscription rather than a metered key.
    pub const fn is_subscription(self) -> bool {
        matches!(self, Self::Oauth)
    }

    /// Map to the wire-level [`ResolvedCredential`] billing identity.
    pub const fn resolved_credential(self) -> ResolvedCredential {
        match self {
            Self::Oauth => ResolvedCredential::Oauth,
            Self::ApiKey => ResolvedCredential::ApiKey,
        }
    }
}

impl From<AuthMode> for ResolvedCredential {
    fn from(mode: AuthMode) -> Self {
        mode.resolved_credential()
    }
}

/// A fully resolved dual-auth credential decision: *which provider* and *which
/// credential*. This is the structured value that every vocabulary string maps
/// to and is generated from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AuthRoute {
    pub provider: DualAuthProvider,
    pub mode: AuthMode,
}

impl AuthRoute {
    pub const fn new(provider: DualAuthProvider, mode: AuthMode) -> Self {
        Self { provider, mode }
    }

    pub const fn anthropic(mode: AuthMode) -> Self {
        Self::new(DualAuthProvider::Anthropic, mode)
    }

    pub const fn openai(mode: AuthMode) -> Self {
        Self::new(DualAuthProvider::OpenAI, mode)
    }

    /// Parse a dual-auth token from *any* of jcode's overlapping vocabularies
    /// (runtime env, route stable-id, CLI `--provider`, or bare model prefix).
    ///
    /// Returns `None` for tokens that do not pin a dual-auth credential route,
    /// including bare aliases for non-dual providers (`openrouter`, `copilot`,
    /// ...), unknown strings, and the empty string. A `None` result is what the
    /// providers treat as "auto" (no explicit OAuth-vs-API pin).
    ///
    /// A single trailing `:` is tolerated so callers can pass a model prefix
    /// such as `claude-oauth:` directly. Full prefixed model specs
    /// (`claude-oauth:model`) are *not* parsed here; resolve the prefix with
    /// `explicit_model_provider_prefix` first.
    pub fn parse(token: &str) -> Option<Self> {
        let token = token.trim().strip_suffix(':').unwrap_or(token.trim());
        match token.trim().to_ascii_lowercase().as_str() {
            // Anthropic / Claude -- OAuth / subscription.
            "claude" | "anthropic" | "claude-oauth" | "anthropic-oauth" => {
                Some(Self::anthropic(AuthMode::Oauth))
            }
            // Anthropic / Claude -- direct API key.
            //
            // Bare `api-key` historically resolves to Anthropic in the route
            // vocabulary (see `ModelRouteApiMethod::parse`), so keep that.
            "claude-api" | "anthropic-api" | "anthropic-api-key" | "claude-api-key"
            | "anthropic-key" | "claude-key" | "api-key" => Some(Self::anthropic(AuthMode::ApiKey)),
            // OpenAI -- OAuth / ChatGPT-Codex login.
            "openai" | "openai-oauth" => Some(Self::openai(AuthMode::Oauth)),
            // OpenAI -- direct API key.
            "openai-api" | "openai-api-key" | "openai-key" | "openai-apikey"
            | "openai-platform" | "platform-openai" => Some(Self::openai(AuthMode::ApiKey)),
            _ => None,
        }
    }

    /// The execution slot this route runs through.
    pub const fn active_provider(self) -> ActiveProvider {
        self.provider.active_provider()
    }

    /// The wire-level billing identity for this route.
    pub const fn resolved_credential(self) -> ResolvedCredential {
        self.mode.resolved_credential()
    }

    /// Parse a *model prefix* that explicitly pins a dual-auth credential.
    ///
    /// This differs from [`AuthRoute::parse`] in the bare-provider cases: in the
    /// model-prefix vocabulary `claude:` / `anthropic:` / `openai:` mean "route
    /// to this provider but keep the current credential (auto)", so they do NOT
    /// pin a credential and return `None` here. Only the explicit credential
    /// prefixes (`claude-oauth:`, `claude-api:`, `openai-oauth:`, `openai-api:`,
    /// and their stable-id spellings) pin one.
    ///
    /// A single trailing `:` is tolerated so callers can pass the raw prefix.
    pub fn parse_explicit_credential_prefix(prefix: &str) -> Option<Self> {
        let token = prefix.trim().strip_suffix(':').unwrap_or(prefix.trim());
        match token.trim().to_ascii_lowercase().as_str() {
            // Bare provider aliases do not pin a credential in this vocabulary.
            "claude" | "anthropic" | "openai" => None,
            other => Self::parse(other),
        }
    }

    /// Canonical `JCODE_RUNTIME_PROVIDER` value that pins this route.
    pub const fn runtime_provider_key(self) -> &'static str {
        match (self.provider, self.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => "claude",
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => "claude-api",
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => "openai",
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => "openai-api",
        }
    }

    /// Canonical route `api_method` / [`crate::RuntimeKey`] stable-id.
    pub const fn route_api_method(self) -> &'static str {
        match (self.provider, self.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => "claude-oauth",
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => "anthropic-api-key",
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => "openai-oauth",
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => "openai-api-key",
        }
    }

    /// Canonical model-switch prefix (without the trailing colon).
    pub const fn model_prefix(self) -> &'static str {
        match (self.provider, self.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => "claude-oauth",
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => "claude-api",
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => "openai-oauth",
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => "openai-api",
        }
    }

    /// Canonical session `provider_key` (the folded, route-free form).
    ///
    /// Unlike [`Self::runtime_provider_key`], the OAuth variants spell
    /// themselves out (`claude-oauth` / `openai-oauth`) instead of reusing the
    /// bare provider name. A session's `provider_key` is also written by
    /// `derive_session_provider_key` for sessions in *automatic* credential
    /// mode, where it is the bare name. Sharing one spelling made a deliberate
    /// OAuth pin indistinguishable from "auto" on restore, so restore promoted
    /// every Anthropic session to a hard OAuth pin and disabled the runtime's
    /// API-key fallback.
    pub const fn session_provider_key(self) -> &'static str {
        match (self.provider, self.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => "claude-oauth",
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => "claude-api",
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => "openai-oauth",
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => "openai-api",
        }
    }

    /// Canonical CLI `--provider` argument value.
    pub const fn cli_provider_arg(self) -> &'static str {
        match (self.provider, self.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => "claude",
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => "anthropic-api",
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => "openai",
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => "openai-api",
        }
    }
}

/// Typed view of [`ModelRoute::api_method`].
///
/// The wire format intentionally remains a string so older clients and saved
/// catalogs continue to round-trip, but routing/picker code should parse it at
/// module boundaries instead of scattering string comparisons everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelRouteApiMethod {
    JcodeSubscription,
    ClaudeOAuth,
    AnthropicApiKey,
    OpenAIOAuth,
    OpenAIApiKey,
    OpenRouter,
    OpenAiCompatible { profile_id: Option<String> },
    Copilot,
    Cursor,
    Bedrock,
    CodeAssistOAuth,
    AntigravityHttps,
    RemoteCatalog,
    Current,
    GrokBuild,
    Other(String),
}

impl ModelRouteApiMethod {
    /// The route-vocabulary api_method for a canonical dual-auth route.
    pub fn from_auth_route(route: AuthRoute) -> Self {
        use {AuthMode, DualAuthProvider};
        match (route.provider, route.mode) {
            (DualAuthProvider::Anthropic, AuthMode::Oauth) => Self::ClaudeOAuth,
            (DualAuthProvider::Anthropic, AuthMode::ApiKey) => Self::AnthropicApiKey,
            (DualAuthProvider::OpenAI, AuthMode::Oauth) => Self::OpenAIOAuth,
            (DualAuthProvider::OpenAI, AuthMode::ApiKey) => Self::OpenAIApiKey,
        }
    }

    pub fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        let lower = trimmed.to_ascii_lowercase();
        // Dual-auth (Anthropic/OpenAI OAuth-vs-API) tokens share one canonical
        // alias table so the route vocabulary never drifts from the runtime/CLI
        // vocabularies. Anything else falls through to the route-only methods.
        if let Some(route) = AuthRoute::parse(&lower) {
            return Self::from_auth_route(route);
        }
        match lower.as_str() {
            "jcode-subscription" => Self::JcodeSubscription,
            "grok-build" | "grok-build-acp" => Self::GrokBuild,
            "openrouter" => Self::OpenRouter,
            "openai-compatible" => Self::OpenAiCompatible { profile_id: None },
            "copilot" => Self::Copilot,
            "cursor" => Self::Cursor,
            "bedrock" => Self::Bedrock,
            "code-assist-oauth" => Self::CodeAssistOAuth,
            "https" => Self::AntigravityHttps,
            "remote-catalog" => Self::RemoteCatalog,
            "current" => Self::Current,
            _ => {
                if let Some(("openai-compatible", profile_id)) = lower.split_once(':') {
                    let profile_id = profile_id.trim();
                    Self::OpenAiCompatible {
                        profile_id: (!profile_id.is_empty()).then(|| profile_id.to_string()),
                    }
                } else {
                    Self::Other(trimmed.to_string())
                }
            }
        }
    }

    pub fn profile_id(&self) -> Option<&str> {
        match self {
            Self::OpenAiCompatible {
                profile_id: Some(profile_id),
            } => Some(profile_id.as_str()),
            _ => None,
        }
    }

    pub fn is_openai_compatible(&self) -> bool {
        matches!(self, Self::OpenAiCompatible { .. })
    }

    pub fn is_openrouter(&self) -> bool {
        matches!(self, Self::OpenRouter)
    }

    pub fn is_copilot(&self) -> bool {
        matches!(self, Self::Copilot)
    }

    pub fn is_cursor(&self) -> bool {
        matches!(self, Self::Cursor)
    }

    pub fn is_bedrock(&self) -> bool {
        matches!(self, Self::Bedrock)
    }

    pub fn matches_openai_compatible_profile(&self, provider_id: &str) -> bool {
        self.profile_id()
            .is_some_and(|profile_id| profile_id.eq_ignore_ascii_case(provider_id))
    }

    pub fn is_anthropic_credential_route(&self) -> bool {
        matches!(self, Self::ClaudeOAuth | Self::AnthropicApiKey)
    }

    pub fn is_openai_credential_route(&self) -> bool {
        matches!(self, Self::OpenAIOAuth | Self::OpenAIApiKey)
    }

    pub fn display_label(&self) -> String {
        match self {
            Self::JcodeSubscription => "subscription".to_string(),
            Self::ClaudeOAuth | Self::OpenAIOAuth | Self::CodeAssistOAuth => "oauth".to_string(),
            Self::AnthropicApiKey | Self::OpenAIApiKey | Self::OpenAiCompatible { .. } => {
                "api key".to_string()
            }
            Self::OpenRouter => "openrouter".to_string(),
            Self::Copilot => "copilot".to_string(),
            Self::Cursor => "cursor".to_string(),
            Self::Bedrock => "bedrock".to_string(),
            Self::AntigravityHttps => "https".to_string(),
            Self::RemoteCatalog => "remote-catalog".to_string(),
            Self::Current => "current".to_string(),
            Self::GrokBuild => "grok-build-acp".to_string(),
            Self::Other(method) => method
                .split_once(':')
                .map(|(method, _)| method)
                .unwrap_or(method)
                .to_string(),
        }
    }
}

/// Default context window size when model-specific data isn't known.
pub const DEFAULT_CONTEXT_LIMIT: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub provider: Option<String>,
    pub context_window: Option<usize>,
}

// --- model_names.rs ---------------------------------------------------------------

// Human-friendly model name rendering.
//
// Model ids arrive as raw provider slugs (`claude-opus-4-8`, `gpt-5.1-codex-max`,
// `gemini-3.1-pro-preview`). Every user-facing surface (the `/model` picker,
// header, status line, info widgets, onboarding copy) wants the same friendly
// rendering, so the formatting rules live here rather than being reinvented per
// call site.
//
// Two entry points, with deliberately different policies:
//
// * [`pretty_model_display_name`] always returns something readable. Prose
//   surfaces use it because a raw slug in a sentence reads badly.
// * [`pretty_known_model_family`] returns `None` for anything outside the
//   curated GPT/Claude/Gemini families. List surfaces use it so third-party,
//   open-weights, namespaced, and profile-scoped ids stay byte-exact and remain
//   copy-pasteable.

/// Turn a raw model id into a friendlier display name.
///
/// Examples:
///   `gpt-5.5`                   -> `GPT-5.5`
///   `gpt-5.1-codex-max`         -> `GPT-5.1 Codex Max`
///   `gpt-5.6-pro[web]`          -> `GPT-5.6 Pro (web)`
///   `claude-opus-4-8`           -> `Claude Opus 4.8`
///   `claude-opus-4-6[1m]`       -> `Claude Opus 4.6 (1M)`
///   `claude-haiku-4-5-20251001` -> `Claude Haiku 4.5 (2025-10-01)`
///   `gemini-2.5-pro`            -> `Gemini 2.5 Pro`
/// Unknown shapes are returned mostly as-is so we never hide the real id.
pub fn pretty_model_display_name(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        return "your default model".to_string();
    }

    // Preserve bracketed route suffixes (`[1m]`, `[web]`) and re-attach them as
    // a parenthetical, since they are jcode-side route markers rather than part
    // of the upstream family/version name.
    let (core, bracket_suffix) = split_bracket_suffix(model);
    // Dated snapshots (`-20251001`) stay visible so a snapshot row is never
    // confused with its floating alias, but read as a date instead of being
    // glued onto the version number.
    let (core, snapshot_date) = split_snapshot_date(core);

    let lower = core.to_ascii_lowercase();
    let mut pretty = if lower.starts_with("gpt-") {
        prettify_versioned_family("GPT", core)
    } else if lower.starts_with("claude-") {
        // Anthropic: claude-opus-4-8 -> Claude Opus 4.8. Convert the trailing
        // `-<major>-<minor>` version into `<major>.<minor>` and title-case the
        // family/tier words.
        prettify_claude(core)
    } else {
        // Gemini and everything else: just title-case the dashed segments.
        title_case_dashed(core)
    };

    // Merge the date/route markers into one parenthetical so a dated 1M row
    // reads `Claude Sonnet 4.5 (2025-09-29, 1M)` rather than stacking two
    // separate groups.
    let markers: Vec<String> = snapshot_date.into_iter().chain(bracket_suffix).collect();
    if !markers.is_empty() {
        pretty.push_str(&format!(" ({})", markers.join(", ")));
    }
    pretty
}

/// Split a trailing bracketed route marker, normalizing `[1m]` to `1M`.
fn split_bracket_suffix(model: &str) -> (&str, Option<String>) {
    let Some(open) = model.rfind('[') else {
        return (model, None);
    };
    if !model.ends_with(']') {
        return (model, None);
    }
    let inner = &model[open + 1..model.len() - 1];
    if inner.is_empty() {
        return (model, None);
    }
    let normalized = if inner.eq_ignore_ascii_case("1m") {
        "1M".to_string()
    } else {
        inner.to_string()
    };
    (&model[..open], Some(normalized))
}

/// Split a trailing `-YYYYMMDD` snapshot date into a `YYYY-MM-DD` label.
fn split_snapshot_date(model: &str) -> (&str, Option<String>) {
    // Compact form: `-20251001`.
    if let Some((head, tail)) = model.rsplit_once('-')
        && !head.is_empty()
        && tail.len() == 8
        && tail.chars().all(|c| c.is_ascii_digit())
    {
        return (
            head,
            Some(format!("{}-{}-{}", &tail[..4], &tail[4..6], &tail[6..])),
        );
    }

    // Dashed form: `-2025-04-14`, as used by OpenAI snapshot ids. Without this
    // the date leaks into the name as three separate title-cased words
    // (`GPT-4.1 Mini 2025 04 14`).
    let mut segments = model.rsplitn(4, '-');
    let day = segments.next();
    let month = segments.next();
    let year = segments.next();
    let head = segments.next();
    if let (Some(head), Some(year), Some(month), Some(day)) = (head, year, month, day)
        && !head.is_empty()
        && is_ascii_digits(year, 4)
        && is_ascii_digits(month, 2)
        && is_ascii_digits(day, 2)
    {
        return (&model[..head.len()], Some(format!("{year}-{month}-{day}")));
    }

    (model, None)
}

/// True when `value` is exactly `len` ASCII digits.
fn is_ascii_digits(value: &str, len: usize) -> bool {
    value.len() == len && value.chars().all(|c| c.is_ascii_digit())
}

/// Render a `<family>-<version>[-<qualifier>...]` id such as `gpt-5.1-codex-max`
/// as `GPT-5.1 Codex Max`: the family keeps its canonical casing, the version
/// stays attached to it, and the trailing qualifier words are title-cased so the
/// name does not trail off into raw lowercase slug text.
fn prettify_versioned_family(family_label: &str, core: &str) -> String {
    let rest = match core.split_once('-') {
        Some((_, rest)) => rest,
        None => return family_label.to_string(),
    };
    let mut parts = rest.split('-');
    let Some(version) = parts.next() else {
        return family_label.to_string();
    };
    // `gpt-oss-120b` and friends have no version: fall back to title-casing so
    // the caller still gets a readable label instead of `GPT-oss-120b`.
    if !version.starts_with(|c: char| c.is_ascii_digit()) {
        return title_case_dashed(core);
    }
    let mut out = format!("{family_label}-{version}");
    for part in parts {
        out.push(' ');
        out.push_str(&title_case_word(part));
    }
    out
}

/// AWS Bedrock region routing prefixes on cross-region inference profile ids.
const BEDROCK_REGION_PREFIXES: [&str; 4] = ["us.", "eu.", "apac.", "global."];

/// Bedrock vendor namespaces jcode knows how to render.
///
/// `anthropic.` reuses the Claude formatter. `amazon.` covers the first-party
/// Nova family, which users see most on Bedrock and which title-cases cleanly.
/// `meta.`, `qwen.`, `cohere.`, `ai21.`, `writer.`, `stability.` and friends are
/// deliberately absent: their slugs carry parameter counts and quantization
/// detail (`llama3-1-405b-instruct`, `qwen3-coder-480b-a35b`) that must stay
/// byte-exact to remain meaningful.
const BEDROCK_PRETTY_VENDORS: [&str; 2] = ["anthropic.", "amazon."];

/// Split an AWS Bedrock model id into its region prefix, vendor namespace, and
/// model portion.
///
/// Bedrock ids are structured rather than opaque:
/// `us.anthropic.claude-opus-4-20250514-v1:0` is a region-routed cross-region
/// inference profile for `anthropic`'s `claude-opus-4` snapshot at API revision
/// `v1:0`. Rendering that raw makes Bedrock rows the least readable in the
/// picker, so the parts are separated and reassembled by the caller.
fn split_bedrock_model_id(model: &str) -> Option<(Option<&str>, &str, &str, Option<String>)> {
    // Full ARNs carry account/region routing detail that must not be hidden.
    let trimmed = model.trim();
    if trimmed.starts_with("arn:aws:bedrock:") {
        return None;
    }

    let mut rest = trimmed;
    let mut region = None;
    for prefix in BEDROCK_REGION_PREFIXES {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            region = Some(&prefix[..prefix.len() - 1]);
            rest = stripped;
            break;
        }
    }

    let vendor = BEDROCK_PRETTY_VENDORS
        .iter()
        .find(|vendor| rest.starts_with(**vendor))?;
    let model_part = &rest[vendor.len()..];
    if model_part.is_empty() {
        return None;
    }

    // A trailing `-v1`, `-v1:0`, or `-v1:0:200k` is a Bedrock API revision plus
    // an optional context/modality variant, not part of the model version. Keep
    // it as a marker so distinct revisions and context variants stay
    // distinguishable, but stop it from corrupting the version number.
    let (model_part, revision) = match split_bedrock_revision(model_part) {
        Some((head, revision)) => (head, Some(revision)),
        None => (model_part, None),
    };

    Some((region, &vendor[..vendor.len() - 1], model_part, revision))
}

/// Split a trailing Bedrock revision segment (`-v1`, `-v1:0`, `-v1:0:200k`,
/// `-v1:0:mm`) off a model id, returning the head plus the revision label.
fn split_bedrock_revision(model_part: &str) -> Option<(&str, String)> {
    let (head, tail) = model_part.rsplit_once("-v")?;
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    let mut segments = tail.split(':');
    // The revision major must be numeric (`v1`), which is what distinguishes it
    // from a family word such as the `-vl` in `qwen3-vl-235b`.
    let major = segments.next()?;
    if major.is_empty() || !major.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Remaining segments are the revision minor and an optional context or
    // modality variant (`200k`, `mm`); accept alphanumerics so new variants do
    // not silently fall back to a raw id.
    for segment in segments {
        if segment.is_empty() || !segment.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
    }
    Some((head, format!("v{tail}")))
}

/// Render `us.anthropic.claude-opus-4-20250514-v1:0` as
/// `Claude Opus 4 (2025-05-14, us, v1:0)`.
fn prettify_bedrock(model: &str) -> Option<String> {
    let (region, vendor, model_part, revision) = split_bedrock_model_id(model)?;
    // Only render when the model portion is a family we know how to format;
    // otherwise the raw id is more informative than a half-prettified one.
    let base = match vendor {
        // `Amazon Nova Pro` is the official product name, and unlike `Claude`
        // the family slug alone would not identify the vendor.
        "amazon" if model_part.to_ascii_lowercase().starts_with("nova") => {
            format!("Amazon {}", title_case_dashed(model_part))
        }
        _ => pretty_known_model_family(model_part)?,
    };
    let mut markers: Vec<String> = Vec::new();
    if let Some(region) = region {
        markers.push(region.to_string());
    }
    if let Some(revision) = revision {
        markers.push(revision);
    }
    if markers.is_empty() {
        return Some(base);
    }
    // The base may already carry its own parenthetical (snapshot date, 1M);
    // merge into a single group rather than stacking them.
    Some(match base.strip_suffix(')') {
        Some(head) => format!("{head}, {})", markers.join(", ")),
        None => format!("{base} ({})", markers.join(", ")),
    })
}

/// Prettify only recognized model families (`gpt-*`, `claude-*`, `gemini-*`,
/// plus AWS Bedrock ids wrapping those families), returning `None` for anything
/// else so unfamiliar or namespaced ids (`vendor/model`, `profile:model`) keep
/// their exact spelling. Used by the `/model` picker, where hiding the real id
/// would break copy-paste and provider-specific naming.
pub fn pretty_known_model_family(model: &str) -> Option<String> {
    let (core, _) = split_bracket_suffix(model);
    if core.contains('.') && core.contains('-') {
        // Possibly a Bedrock id; `gpt-5.5` and `gemini-2.5-pro` also contain a
        // dot, so only take this path when a vendor namespace actually matches.
        if let Some(pretty) = prettify_bedrock(core) {
            return Some(pretty);
        }
    }
    if core.contains('/') || core.contains(':') {
        return None;
    }
    let lower = core.to_ascii_lowercase();
    // `gpt-` only counts when a version number follows (`gpt-5.5`), so
    // open-weights ids like `gpt-oss-120b` are not mangled into `GPT-oss-…`.
    let versioned_gpt = lower
        .strip_prefix("gpt-")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    if !(versioned_gpt || lower.starts_with("claude-") || lower.starts_with("gemini-")) {
        return None;
    }
    Some(pretty_model_display_name(model))
}

/// Render `claude-opus-4-8` as `Claude Opus 4.8`.
fn prettify_claude(core: &str) -> String {
    let parts: Vec<&str> = core.split('-').collect();
    let mut words: Vec<String> = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        // Collapse a `<major>-<minor>` numeric pair into `<major>.<minor>`.
        if part.chars().all(|c| c.is_ascii_digit())
            && i + 1 < parts.len()
            && parts[i + 1].chars().all(|c| c.is_ascii_digit())
        {
            words.push(format!("{}.{}", part, parts[i + 1]));
            i += 2;
            continue;
        }
        words.push(title_case_word(part));
        i += 1;
    }
    words.join(" ")
}

/// Title-case a dash-separated id (`gemini-2.5-pro` -> `Gemini 2.5 Pro`).
fn title_case_dashed(core: &str) -> String {
    core.split('-')
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Acronyms that read wrong when naively title-cased (`Tts`, `Api`, `Vl`).
const UPPERCASE_TOKENS: [&str; 6] = ["tts", "stt", "api", "vl", "ocr", "id"];

/// Title-case a single token, leaving anything containing a digit untouched so
/// version fragments like `4.8` or `2.5` are preserved.
fn title_case_word(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }
    if word.chars().any(|c| c.is_ascii_digit()) {
        return word.to_string();
    }
    let lower = word.to_ascii_lowercase();
    if UPPERCASE_TOKENS.contains(&lower.as_str()) {
        return lower.to_ascii_uppercase();
    }
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// Brand spellings that plain title-casing gets wrong (`Deepseek`, `Glm`).
const BRAND_TOKENS: [(&str, &str); 24] = [
    ("gpt", "GPT"),
    ("oss", "OSS"),
    ("glm", "GLM"),
    ("deepseek", "DeepSeek"),
    ("minimax", "MiniMax"),
    ("openai", "OpenAI"),
    ("xai", "xAI"),
    ("tts", "TTS"),
    ("stt", "STT"),
    ("api", "API"),
    ("vl", "VL"),
    ("ocr", "OCR"),
    ("id", "ID"),
    ("it", "IT"),
    ("moe", "MoE"),
    ("fp8", "FP8"),
    ("fp4", "FP4"),
    ("nvfp4", "NVFP4"),
    ("awq", "AWQ"),
    ("gguf", "GGUF"),
    ("hd", "HD"),
    ("r1", "R1"),
    ("ai", "AI"),
    ("lfm", "LFM"),
];

/// OpenRouter-style `:variant` suffixes rendered as a parenthetical.
const PICKER_VARIANT_TAGS: [&str; 9] = [
    "free", "batch", "thinking", "beta", "nitro", "floor", "online", "exacto", "extended",
];

/// Families whose official names hyphenate the family to its version
/// (`GPT-5.5`, `GLM-5.1`, `GPT-OSS`).
const HYPHENATED_FAMILIES: [&str; 2] = ["GPT", "GLM"];

/// Render any model id as a readable picker title.
///
/// Unlike [`pretty_known_model_family`], this never returns the raw id for
/// unfamiliar shapes. It is meant for surfaces that also show the exact id
// [port-decision] leaf port: `explicit_model_provider_prefix` verbatim from
// jcode-provider-core/src/selection.rs:165-199 — the routing-prefix splitter
// `pretty_picker_model_name` reaches. ActiveProvider is the local :17 enum.
pub fn explicit_model_provider_prefix(model: &str) -> Option<(ActiveProvider, &'static str, &str)> {
    if let Some(rest) = model.strip_prefix("claude-api:") {
        Some((ActiveProvider::Claude, "claude-api:", rest))
    } else if let Some(rest) = model.strip_prefix("claude-oauth:") {
        Some((ActiveProvider::Claude, "claude-oauth:", rest))
    } else if let Some(rest) = model.strip_prefix("anthropic-api:") {
        // The dual-auth API-key key for Anthropic. It must route through the
        // native Messages runtime, not the same-named OpenAI-compatible
        // catalog profile, which drops reasoning effort and prompt caching.
        Some((ActiveProvider::Claude, "anthropic-api:", rest))
    } else if let Some(rest) = model.strip_prefix("claude:") {
        Some((ActiveProvider::Claude, "claude:", rest))
    } else if let Some(rest) = model.strip_prefix("anthropic:") {
        Some((ActiveProvider::Claude, "anthropic:", rest))
    } else if let Some(rest) = model.strip_prefix("openai-api:") {
        Some((ActiveProvider::OpenAI, "openai-api:", rest))
    } else if let Some(rest) = model.strip_prefix("openai-oauth:") {
        Some((ActiveProvider::OpenAI, "openai-oauth:", rest))
    } else if let Some(rest) = model.strip_prefix("openai:") {
        Some((ActiveProvider::OpenAI, "openai:", rest))
    } else if let Some(rest) = model.strip_prefix("copilot:") {
        Some((ActiveProvider::Copilot, "copilot:", rest))
    } else if let Some(rest) = model.strip_prefix("antigravity:") {
        Some((ActiveProvider::Antigravity, "antigravity:", rest))
    } else if let Some(rest) = model.strip_prefix("gemini:") {
        Some((ActiveProvider::Gemini, "gemini:", rest))
    } else if let Some(rest) = model.strip_prefix("cursor:") {
        Some((ActiveProvider::Cursor, "cursor:", rest))
    } else if let Some(rest) = model.strip_prefix("bedrock:") {
        Some((ActiveProvider::Bedrock, "bedrock:", rest))
    } else if let Some(rest) = model.strip_prefix("openrouter:") {
        Some((ActiveProvider::OpenRouter, "openrouter:", rest))
    } else {
        None
    }
}

/// nearby (such as the `/model` picker's detail line), so a friendlier title
/// cannot hide which route will actually run.
///
/// Examples:
///   `openai-api:gpt-5.5`               -> `GPT-5.5`
///   `anthropic/claude-opus-4.6`        -> `Claude Opus 4.6`
///   `deepseek/deepseek-v4-pro`         -> `DeepSeek V4 Pro`
///   `Llama-3.3-70B-Instruct`           -> `Llama 3.3 70B Instruct`
///   `moonshotai/kimi-k2.5:free`        -> `Kimi K2.5 (free)`
///   `gpt-oss-120b`                     -> `GPT-OSS 120B`
///   `o3-mini`                          -> `o3 Mini`
pub fn pretty_picker_model_name(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Routing prefixes (`openai-api:`) and vendor namespaces (`anthropic/`)
    // are shown by the picker's group header, not the title.
    let bare = explicit_model_provider_prefix(trimmed).map_or(trimmed, |(_, _, bare)| bare);
    if bare.starts_with("arn:") {
        return bare.to_string();
    }
    if let Some(pretty) = pretty_known_model_family(bare) {
        return pretty;
    }
    let mut bare = bare.rsplit('/').next().unwrap_or(bare);
    // Profile or provider namespaces (`comtegra:glm-51`, `google:gemini-x`)
    // are purely alphabetic heads. The picker's group header names them.
    while let Some((head, rest)) = bare.split_once(':') {
        if head.is_empty() || rest.is_empty() || !head.chars().all(|c| c.is_ascii_alphabetic()) {
            break;
        }
        if PICKER_VARIANT_TAGS.contains(&rest.to_ascii_lowercase().as_str()) {
            break;
        }
        bare = rest;
    }
    // A known `:tag` (`:free`, `:thinking`) is a variant marker. Anything else
    // with a colon (Bedrock `-v1:0`) stays exact.
    let (bare, tag) = match bare.split_once(':') {
        Some((head, tag))
            if !head.is_empty()
                && PICKER_VARIANT_TAGS.contains(&tag.to_ascii_lowercase().as_str()) =>
        {
            (head, Some(tag.to_ascii_lowercase()))
        }
        Some(_) => return bare.to_string(),
        None => (bare, None),
    };
    if let Some(pretty) = pretty_known_model_family(bare) {
        return append_markers(pretty, tag.into_iter().collect());
    }
    // Bedrock-style `vendor.model` namespaces: drop a purely alphabetic vendor
    // segment that ends before the first dash (`google.gemma-3-27b-it`).
    let bare = match bare.split_once('.') {
        Some((vendor, rest))
            if !rest.is_empty()
                && vendor.chars().all(|c| c.is_ascii_alphabetic())
                && !bare[..vendor.len()].contains('-')
                && !rest.starts_with(|c: char| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => bare,
    };
    if let Some(pretty) = pretty_known_model_family(bare) {
        return append_markers(pretty, tag.into_iter().collect());
    }
    let (core, bracket) = split_bracket_suffix(bare);
    let (core, date) = split_snapshot_date(core);
    let parts: Vec<&str> = core
        .split(['-', '_', ' '])
        .filter(|p| !p.is_empty())
        .collect();
    let mut words: Vec<String> = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        // `3-1` reads as version `3.1`, as Claude and Llama slugs intend.
        if part.len() <= 2
            && part.chars().all(|c| c.is_ascii_digit())
            && i + 1 < parts.len()
            && parts[i + 1].len() <= 2
            && parts[i + 1].chars().all(|c| c.is_ascii_digit())
        {
            words.push(format!("{part}.{}", parts[i + 1]));
            i += 2;
            continue;
        }
        let word = pretty_picker_token(part);
        match words.last_mut() {
            Some(previous)
                if HYPHENATED_FAMILIES.contains(&previous.as_str())
                    && (word.starts_with(|c: char| c.is_ascii_digit()) || word == "OSS") =>
            {
                previous.push('-');
                previous.push_str(&word);
            }
            _ => words.push(word),
        }
        i += 1;
    }
    if words.is_empty() {
        return bare.to_string();
    }
    let markers = date.into_iter().chain(bracket).chain(tag).collect();
    append_markers(words.join(" "), markers)
}

fn append_markers(base: String, markers: Vec<String>) -> String {
    if markers.is_empty() {
        return base;
    }
    match base.strip_suffix(')') {
        Some(head) if head.contains(" (") => format!("{head}, {})", markers.join(", ")),
        _ => format!("{base} ({})", markers.join(", ")),
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Case one model-id token for a picker title.
fn pretty_picker_token(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    if let Some((_, brand)) = BRAND_TOKENS.iter().find(|(raw, _)| *raw == lower) {
        return (*brand).to_string();
    }
    let bytes = lower.as_bytes();
    let digits_then = |suffix: &[u8]| {
        bytes.len() > 1
            && suffix.contains(&bytes[bytes.len() - 1])
            && lower[..lower.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == 'x')
            && lower.starts_with(|c: char| c.is_ascii_digit())
    };
    // Parameter and context sizes: `70b` -> `70B`, `8x7b` -> `8x7B`, `200k`.
    if digits_then(b"bmkt") {
        let (head, unit) = lower.split_at(lower.len() - 1);
        return format!("{head}{}", unit.to_ascii_uppercase());
    }
    // Active-parameter counts: `a35b` -> `A35B`.
    if bytes.len() > 2
        && bytes[0] == b'a'
        && bytes[bytes.len() - 1] == b'b'
        && lower[1..lower.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return lower.to_ascii_uppercase();
    }
    // OpenAI reasoning families stay lowercase: `o3`, `o4`.
    if bytes[0] == b'o' && lower[1..].chars().all(|c| c.is_ascii_digit()) && bytes.len() > 1 {
        return lower;
    }
    // Single-letter generations: `k2.5` -> `K2.5`, `v4` -> `V4`, `m2` -> `M2`.
    if bytes.len() > 1
        && bytes[0].is_ascii_alphabetic()
        && lower[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return lower.to_ascii_uppercase();
    }
    // Mixed tokens: keep digits, capitalize a leading word (`qwen3` -> `Qwen3`).
    if token.chars().any(|c| c.is_ascii_digit()) {
        if token.starts_with(|c: char| c.is_ascii_lowercase()) {
            let letters: String = token
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if letters.len() > 1 {
                let mut chars = token.chars();
                let first = chars.next().unwrap().to_ascii_uppercase();
                return format!("{first}{}", chars.as_str());
            }
        }
        return token.to_string();
    }
    // Preserve deliberate mixed case (`MiniMax`), title-case plain words.
    if token.chars().any(|c| c.is_ascii_uppercase()) {
        return token.to_string();
    }
    title_case_word(token)
}

// --- selection.rs additions (referenced by ui_header.rs) ---------------------------

pub fn parse_provider_hint(value: &str) -> Option<ActiveProvider> {
    match value.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some(ActiveProvider::Claude),
        "openai" => Some(ActiveProvider::OpenAI),
        "copilot" => Some(ActiveProvider::Copilot),
        "antigravity" => Some(ActiveProvider::Antigravity),
        "gemini" => Some(ActiveProvider::Gemini),
        "cursor" => Some(ActiveProvider::Cursor),
        "bedrock" | "aws-bedrock" | "aws_bedrock" => Some(ActiveProvider::Bedrock),
        "openrouter" => Some(ActiveProvider::OpenRouter),
        _ => None,
    }
}

pub fn provider_label(provider: ActiveProvider) -> &'static str {
    match provider {
        ActiveProvider::Claude => "Anthropic",
        ActiveProvider::OpenAI => "OpenAI",
        ActiveProvider::Copilot => "GitHub Copilot",
        ActiveProvider::Antigravity => "Antigravity",
        ActiveProvider::Gemini => "Gemini",
        ActiveProvider::Cursor => "Cursor",
        ActiveProvider::Bedrock => "AWS Bedrock",
        ActiveProvider::OpenRouter => "OpenRouter",
    }
}

// [port-decision] leaf port: pinned_mode_for verbatim from
// jcode-provider-core/src/auth_mode.rs:258-264 — resolve_dual_credential_auth's
// env-pin parser (just outside the first wave's :29-238 auth_mode cut).
/// Returns `None` (i.e. "auto") when `runtime_provider` is absent, does not pin
/// a dual-auth route, or pins the *other* dual-auth provider.
pub fn pinned_mode_for(
    provider: DualAuthProvider,
    runtime_provider: Option<&str>,
) -> Option<AuthMode> {
    let route = AuthRoute::parse(runtime_provider?)?;
    (route.provider == provider).then_some(route.mode)
}

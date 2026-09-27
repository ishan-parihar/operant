//! API connectivity probes for `operant doctor`.
//!
//! Mirrors the parallel HTTP probe section from `operant-agent/operant_cli/doctor.py`:
//! - OpenRouter, Anthropic, 20+ API-key providers, AWS Bedrock
//!
//! All probes run concurrently via `tokio::spawn`.

use std::io::Write;
use std::time::Duration;

use console::style;

use super::check_result::{check_fail, check_ok, check_warn, section_header};
use crate::provider::{PROVIDERS, provider_from_url};
use operant_core::config::AppConfig;

// ---------------------------------------------------------------------------
// Probe result type
// ---------------------------------------------------------------------------

/// Outcome of a single connectivity probe.
enum ProbeKind {
    /// ✓ – the endpoint responded as expected.
    Ok,
    /// ⚠ – unexpected HTTP status or network error.
    Warn,
    /// ✗ – authentication failure (401/403) or hard error.
    Fail,
    /// No probe was attempted (no key configured for this provider).
    Skipped,
}

/// In-memory result returned by each probe future.
struct ProbeResult {
    label: String,
    kind: ProbeKind,
    detail: String,
    issue: Option<String>,
}

impl ProbeResult {
    fn ok(label: &str) -> Self {
        Self {
            label: label.into(),
            kind: ProbeKind::Ok,
            detail: String::new(),
            issue: None,
        }
    }

    fn ok_with(label: &str, detail: &str) -> Self {
        Self {
            label: label.into(),
            kind: ProbeKind::Ok,
            detail: detail.into(),
            issue: None,
        }
    }

    fn fail(label: &str, detail: &str) -> Self {
        Self {
            label: label.into(),
            kind: ProbeKind::Fail,
            detail: detail.into(),
            issue: None,
        }
    }

    fn fail_with(label: &str, detail: &str, issue: &str) -> Self {
        Self {
            label: label.into(),
            kind: ProbeKind::Fail,
            detail: detail.into(),
            issue: Some(issue.into()),
        }
    }

    fn warn(label: &str, detail: &str) -> Self {
        Self {
            label: label.into(),
            kind: ProbeKind::Warn,
            detail: detail.into(),
            issue: None,
        }
    }

    fn skipped() -> Self {
        Self {
            label: String::new(),
            kind: ProbeKind::Skipped,
            detail: String::new(),
            issue: None,
        }
    }
}

impl Default for ProbeResult {
    fn default() -> Self {
        Self::skipped()
    }
}

// ---------------------------------------------------------------------------
// Arguments for each generic API-key provider probe
// ---------------------------------------------------------------------------

struct ApiKeyProbeConfig {
    display_name: &'static str,
    url: String,
    key: String,
    env_var: &'static str,
}

/// Decide which URL (if any) a generic provider probe should hit, given the
/// configured active endpoint (`config.client.base_url` after env overrides).
///
/// - `Some(<base>/models)` when the provider should be probed.
/// - `None` when the probe must be skipped: this provider's key came from
///   `OPENAI_API_KEY` — the var `apply_env_overrides` routes to
///   `client.api_key`, i.e. the key that authenticates at the *configured*
///   endpoint — but the configured endpoint belongs to a different provider.
///   Firing that key at this provider's public default URL reports a false
///   "(invalid API key)" (observed with the omp omniroute gateway on
///   `localhost:20129`, which `provider_from_url` maps to "ollama": every
///   `operant doctor` run showed `✗ OpenAI (invalid API key)` and
///   `✗ OpenAI Codex (invalid API key)` while the configured endpoint
///   answered 200).
/// - When the configured endpoint maps to this provider, probe the
///   *configured* URL rather than the provider default, so the check reflects
///   what the agent actually calls.
fn generic_probe_url(
    env_used: &str,
    provider_name: &str,
    provider_default_base: &str,
    active_base: &str,
    active_provider: Option<&str>,
) -> Option<String> {
    if !active_base.is_empty()
        && env_used == "OPENAI_API_KEY"
        && active_provider != Some(provider_name)
    {
        return None;
    }
    let base = if active_provider == Some(provider_name) && !active_base.is_empty() {
        active_base
    } else {
        provider_default_base.trim_end_matches('/')
    };
    Some(format!("{}/models", base))
}

// ---------------------------------------------------------------------------
// Individual probe functions
// ---------------------------------------------------------------------------

/// **Section A** – OpenRouter connectivity probe.
async fn probe_openrouter() -> ProbeResult {
    let key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return ProbeResult::skipped(),
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return ProbeResult::warn("OpenRouter API", "(could not create HTTP client)"),
    };

    match client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
    {
        Ok(resp) => match resp.status().as_u16() {
            200 => ProbeResult::ok("OpenRouter API"),
            401 => ProbeResult::fail_with(
                "OpenRouter API",
                "(invalid API key)",
                "Check OPENROUTER_API_KEY in .env",
            ),
            402 => ProbeResult::fail("OpenRouter API", "(out of credits — payment required)"),
            429 => ProbeResult::fail("OpenRouter API", "(rate limited)"),
            code => ProbeResult::warn("OpenRouter API", &format!("(HTTP {})", code)),
        },
        Err(e) => ProbeResult::warn("OpenRouter API", &format!("({})", e)),
    }
}

/// **Section B** – Anthropic connectivity probe.
async fn probe_anthropic() -> ProbeResult {
    let key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => return ProbeResult::skipped(),
    };

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return ProbeResult::warn("Anthropic API", "(could not create HTTP client)"),
    };

    match client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
    {
        Ok(resp) => match resp.status().as_u16() {
            200 => ProbeResult::ok("Anthropic API"),
            401 => ProbeResult::fail("Anthropic API", "(invalid API key)"),
            code => ProbeResult::warn("Anthropic API", &format!("(HTTP {})", code)),
        },
        Err(e) => ProbeResult::warn("Anthropic API", &format!("({})", e)),
    }
}

/// **Section C** – Generic `auth_type == "api_key"` provider probe.
///
/// Sends `Authorization: Bearer {key}` to `{base_url}/models`.
async fn probe_apikey_provider(
    display_name: &'static str,
    url: String,
    key: String,
    env_var: &'static str,
) -> ProbeResult {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return ProbeResult::warn(display_name, "(could not create HTTP client)"),
    };

    match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
    {
        Ok(resp) => match resp.status().as_u16() {
            200 => ProbeResult::ok(display_name),
            401 | 403 => ProbeResult::fail_with(
                display_name,
                "(invalid API key)",
                &format!("Check {} in .env", env_var),
            ),
            code => ProbeResult::warn(display_name, &format!("(HTTP {})", code)),
        },
        Err(e) => ProbeResult::warn(display_name, &format!("({})", e)),
    }
}

/// **Section D** – AWS Bedrock credential check.
///
/// Rust does not include a first-class AWS SDK, so this probe simply verifies
/// that the two required environment variables are present (matching the
/// Python doctor's credential-only path).
async fn probe_bedrock() -> ProbeResult {
    let has_key = std::env::var("AWS_ACCESS_KEY_ID")
        .ok()
        .is_some_and(|v| !v.is_empty());
    let has_secret = std::env::var("AWS_SECRET_ACCESS_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty());

    if has_key && has_secret {
        ProbeResult::ok_with("AWS Bedrock", "(credentials configured)")
    } else {
        ProbeResult::skipped()
    }
}

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Run all API connectivity probes in parallel.
///
/// Checks:
/// - OpenRouter API (`OPENROUTER_API_KEY`)
/// - Anthropic API (`ANTHROPIC_API_KEY`)
/// - Every provider in `crate::provider::PROVIDERS` with `auth_type == "api_key"`
///   and a configured environment variable
/// - AWS Bedrock (credential-only check)
///
/// Each probe runs on its own `tokio` task.  Results are collected first, then
/// printed, so the output is never interleaved.
pub async fn run_api_checks(config: &AppConfig, issues: &mut Vec<String>) {
    section_header("API Connectivity");

    // The endpoint the agent actually calls: `client.base_url` after env
    // overrides (`apply_env_overrides` maps OPENAI_BASE_URL → client.base_url,
    // OPENAI_API_KEY → client.api_key).
    let active_base = config.client.base_url.trim().trim_end_matches('/');
    let active_provider: Option<&str> = if active_base.is_empty() {
        None
    } else {
        provider_from_url(active_base).map(|p| p.name)
    };
    let active_key = config
        .client
        .api_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    // -- Prepare generic API-key provider configs --------------------------------
    // Collect every provider with auth_type == "api_key" that has a configured
    // env var, skipping OpenRouter & Anthropic (they have dedicated probes above)
    // and providers with an empty base URL.
    let mut api_key_configs: Vec<ApiKeyProbeConfig> = PROVIDERS
        .iter()
        .filter(|p| {
            p.auth_type == "api_key"
                && !p.env_var.is_empty()
                && p.name != "anthropic"
                && p.name != "openrouter"
                && !p.default_base_url.is_empty()
        })
        .filter_map(|p| {
            let (env_used, key) = p.env_vars.iter().find_map(|ev| {
                std::env::var(ev)
                    .ok()
                    .filter(|s| !s.is_empty())
                    .map(|k| (*ev, k))
            })?;
            let url = generic_probe_url(
                env_used,
                p.name,
                p.default_base_url,
                active_base,
                active_provider,
            )?;
            Some(ApiKeyProbeConfig {
                display_name: p.display_name,
                url,
                key,
                env_var: p.env_var,
            })
        })
        .collect();

    // Sort by display name so output order is stable & predictable.
    api_key_configs.sort_by(|a, b| a.display_name.cmp(b.display_name));

    // Dedicated probe for the configured endpoint when the generic sweep does
    // not already cover it (e.g. `provider_from_url` maps a localhost gateway
    // to "ollama", which has no api_key sweep entry): check the URL and key
    // the agent actually uses instead of leaving the active endpoint unprobed.
    let active_url = format!("{}/models", active_base);
    let active_probe = if !active_base.is_empty()
        && !active_key.is_empty()
        && !api_key_configs.iter().any(|c| c.url == active_url)
    {
        Some(ApiKeyProbeConfig {
            display_name: "Active LLM endpoint",
            url: active_url,
            key: active_key,
            env_var: "OPENAI_API_KEY",
        })
    } else {
        None
    };

    // -- Count probes for the status line ---------------------------------------
    let total_probes = 2 // A: OpenRouter + B: Anthropic
        + api_key_configs.len() // C: API-key providers with a key
        + usize::from(active_probe.is_some()) // C': configured custom endpoint
        + 1; // D: AWS Bedrock

    // Dim status line shown while probes are in-flight.
    print!(
        "  {}",
        style(format!(
            "Running {} connectivity checks in parallel…",
            total_probes
        ))
        .dim()
    );
    let _ = std::io::stdout().flush();

    // -- Spawn all probes concurrently ------------------------------------------
    let mut handles: Vec<tokio::task::JoinHandle<ProbeResult>> = Vec::new();

    // A. OpenRouter
    handles.push(tokio::spawn(probe_openrouter()));

    // B. Anthropic
    handles.push(tokio::spawn(probe_anthropic()));

    // B'. The configured custom gateway endpoint, if any — checked with the
    // URL and key the agent actually uses.
    if let Some(cfg) = active_probe {
        handles.push(tokio::spawn(probe_apikey_provider(
            cfg.display_name,
            cfg.url,
            cfg.key,
            cfg.env_var,
        )));
    }

    // C. Generic API-key provider probes
    for cfg in api_key_configs {
        handles.push(tokio::spawn(probe_apikey_provider(
            cfg.display_name,
            cfg.url,
            cfg.key,
            cfg.env_var,
        )));
    }

    // D. AWS Bedrock
    handles.push(tokio::spawn(probe_bedrock()));

    // -- Collect all results ----------------------------------------------------
    let mut results: Vec<ProbeResult> = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await.unwrap_or_default());
    }

    // Clear the "Running …" line.
    print!("\r{}\r", " ".repeat(70));
    let _ = std::io::stdout().flush();

    // -- Print non-skipped results in submission order --------------------------
    for r in &results {
        match r.kind {
            ProbeKind::Ok => check_ok(&r.label, &r.detail),
            ProbeKind::Fail => check_fail(&r.label, &r.detail),
            ProbeKind::Warn => check_warn(&r.label, &r.detail),
            ProbeKind::Skipped => continue,
        }
        if let Some(ref issue) = r.issue {
            issues.push(issue.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::generic_probe_url;

    #[test]
    fn omp_gateway_skips_openai_env_key_probes() {
        // omp-style local gateway: OPENAI_API_KEY holds a gateway key, so
        // probing api.openai.com would report a false "(invalid API key)".
        // `provider_from_url` maps localhost → "ollama", so the active
        // provider is ollama, not openai.
        assert_eq!(
            generic_probe_url(
                "OPENAI_API_KEY",
                "openai",
                "https://api.openai.com/v1",
                "http://localhost:20129/v1",
                Some("ollama")
            ),
            None
        );
        // Same ownership rule covers every provider fed from the shared
        // override var while the configured endpoint belongs elsewhere.
        assert_eq!(
            generic_probe_url(
                "OPENAI_API_KEY",
                "openai-codex",
                "https://api.openai.com/v1",
                "http://localhost:20129/v1",
                Some("ollama")
            ),
            None
        );
    }

    #[test]
    fn non_active_endpoint_still_probes_unrelated_keys() {
        // A distinct provider key (e.g. GROQ_API_KEY) still belongs to that
        // provider's public endpoint even when the active endpoint is a
        // local gateway.
        assert_eq!(
            generic_probe_url(
                "GROQ_API_KEY",
                "groq",
                "https://api.groq.com/openai/v1",
                "http://localhost:20129/v1",
                Some("ollama")
            )
            .as_deref(),
            Some("https://api.groq.com/openai/v1/models")
        );
    }

    #[test]
    fn mapped_active_provider_probes_configured_url() {
        // When the configured base URL maps to a known provider, that
        // provider's probe must hit the configured URL, not the default.
        assert_eq!(
            generic_probe_url(
                "OPENAI_API_KEY",
                "openai",
                "https://api.openai.com/v1",
                "https://api.openai-proxy.example.com/v1",
                Some("openai")
            )
            .as_deref(),
            Some("https://api.openai-proxy.example.com/v1/models")
        );
    }

    #[test]
    fn empty_active_base_keeps_default_behaviour() {
        // No configured base URL: probe the provider default, as before.
        assert_eq!(
            generic_probe_url(
                "OPENAI_API_KEY",
                "openai",
                "https://api.openai.com/v1",
                "",
                None
            )
            .as_deref(),
            Some("https://api.openai.com/v1/models")
        );
    }
}

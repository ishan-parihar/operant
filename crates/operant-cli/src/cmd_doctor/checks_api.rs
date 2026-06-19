//! API connectivity probes for `operant doctor`.
//!
//! Mirrors the parallel HTTP probe section from `operant-agent/operant_cli/doctor.py`:
//! - OpenRouter, Anthropic, 20+ API-key providers, AWS Bedrock
//!
//! All probes run concurrently via `tokio::spawn`.

#![allow(unused)]

use std::io::Write;
use std::time::Duration;

use console::style;

use super::check_result::{check_fail, check_info, check_ok, check_warn, section_header};
use crate::provider::PROVIDERS;

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
        .map_or(false, |v| !v.is_empty());
    let has_secret = std::env::var("AWS_SECRET_ACCESS_KEY")
        .ok()
        .map_or(false, |v| !v.is_empty());

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
pub async fn run_api_checks(issues: &mut Vec<String>) {
    section_header("API Connectivity");

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
            let key = p
                .env_vars
                .iter()
                .find_map(|ev| std::env::var(ev).ok().filter(|s| !s.is_empty()))?;
            let url = format!("{}/models", p.default_base_url.trim_end_matches('/'));
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

    // -- Count probes for the status line ---------------------------------------
    let total_probes = 2               // A: OpenRouter + B: Anthropic
        + api_key_configs.len()        // C: API-key providers with a key
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

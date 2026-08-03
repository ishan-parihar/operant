#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Per-platform tool policy.
//!
//! Ported from `hermes-agent-ultra/crates/hermes-tool-planning`. Runtime policy
//! for **which tools a given platform may call**: normalize platform aliases,
//! canonicalize fuzzy toolset tokens, and filter a tool list down to the
//! platform's configured set.
//!
//! The module is pure — no async, no I/O — so it is trivially testable and
//! safe to call from any adapter (CLI, gateway, channels orchestrator).
//!
//! ## Semantics
//!
//! - An **empty** `platform_toolsets` map (the default) means *all tools*:
//!   every call is a no-op pass-through, preserving legacy behavior.
//! - A configured entry is an **allow-list** of toolset tokens / tool names
//!   for that platform; tokens are canonicalized so `"browser-use"`,
//!   `"browser_use"`, and `"browser"` are interchangeable.

use std::collections::{HashMap, HashSet};

use operant_api::tool::ToolSpec;

/// Normalize platform aliases used by runtime adapters to config keys.
pub fn normalize_platform_key(platform: &str) -> String {
    match platform.trim().to_ascii_lowercase().as_str() {
        "local" => "cli".to_string(),
        "tg" => "telegram".to_string(),
        "dc" => "discord".to_string(),
        "api" | "http" => "api_server".to_string(),
        "sms" => "sms_twilio".to_string(),
        other => other.to_string(),
    }
}

/// Canonicalize a fuzzy toolset token to the canonical config spelling.
///
/// Strips a legacy `_tools` / `-tools` suffix and folds common aliases so a
/// config can say `browser`, `browser-use`, or `browser_use`
/// interchangeably.
pub fn canonical_toolset_token(token: &str) -> String {
    let mut token = token.trim().to_ascii_lowercase();
    if let Some(stripped) = token
        .strip_suffix("_tools")
        .or_else(|| token.strip_suffix("-tools"))
    {
        token = stripped.to_string();
    }
    match token.as_str() {
        "image-gen" | "imagegen" => "image_gen".to_string(),
        "video-gen" | "videogen" => "video_gen".to_string(),
        "code-execution" | "code" => "code_execution".to_string(),
        "session-search" => "session_search".to_string(),
        "home-assistant" | "home_assistant" | "ha" => "homeassistant".to_string(),
        "browser-use" | "browser_use" => "browser".to_string(),
        "voice-mode" | "voice_mode" => "voice".to_string(),
        "web-scrape" | "web_scrape" | "web-crawl" | "web_crawl" => "web".to_string(),
        _ => token,
    }
}

/// Built-in per-platform toolset defaults when no explicit
/// `platform_toolsets` entry exists.
///
/// Only platforms with meaningful non-trivial defaults are listed; anything
/// missing resolves to the platform's configured entry or, failing that, to
/// an empty list (which the resolver treats as "all tools" — see the
/// [`resolve_platform_tool_names`] fallback).
pub fn default_platform_toolsets() -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    // Messaging-first platforms get a safe subset: no terminal / filesystem
    // mutation tools by default.
    map.insert(
        "telegram".to_string(),
        vec![
            "web".to_string(),
            "browser".to_string(),
            "skills".to_string(),
            "memory".to_string(),
            "todo".to_string(),
            "delegation".to_string(),
        ],
    );
    map.insert(
        "discord".to_string(),
        vec![
            "web".to_string(),
            "browser".to_string(),
            "skills".to_string(),
            "memory".to_string(),
            "todo".to_string(),
            "delegation".to_string(),
        ],
    );
    map.insert(
        "slack".to_string(),
        vec![
            "web".to_string(),
            "browser".to_string(),
            "skills".to_string(),
            "memory".to_string(),
            "todo".to_string(),
        ],
    );
    // NOTE: `api_server` intentionally has NO built-in default — the gateway
    // daemon path must keep *all* tools unless the operator explicitly adds a
    // `platform_toolsets.api_server` allow-list, preserving legacy behavior.
    map
}

/// Configured toolset tokens for a platform, with default fallback.
///
/// `toolsets` is the raw `platform_toolsets` config map (keyed by normalized
/// platform key). An explicit non-empty entry wins; otherwise the built-in
/// default for that platform is used; otherwise `[]` (→ all tools).
pub fn configured_platform_toolsets(
    toolsets: &HashMap<String, Vec<String>>,
    platform: &str,
) -> Vec<String> {
    let key = normalize_platform_key(platform);
    if let Some(custom) = toolsets.get(&key)
        && !custom.is_empty()
    {
        return custom.clone();
    }
    default_platform_toolsets().remove(&key).unwrap_or_default()
}

/// Resolve the tool *names* allowed for this platform.
///
/// `all_names` is the full set of available tool names. Returns:
/// - every name (pass-through) when the platform's configured list is empty
///   (legacy default: all tools), or when the list contains the `all` / `*`
///   token;
/// - otherwise the `all_names` that match the configured tokens after
///   canonicalization. Unknown tokens are logged and ignored.
pub fn resolve_platform_tool_names(
    toolsets: &HashMap<String, Vec<String>>,
    platform: &str,
    all_names: &[&str],
) -> Vec<String> {
    let requested = configured_platform_toolsets(toolsets, platform);
    let mut names: HashSet<String> = HashSet::new();

    let mut all_requested = false;
    for token in requested {
        let original = token.trim();
        if original.is_empty() {
            continue;
        }
        if original == "all" || original == "*" {
            all_requested = true;
            continue;
        }
        let canonical = canonical_toolset_token(original);
        // Toolset token matches a real tool name directly?
        if all_names.iter().any(|n| *n == canonical) {
            names.insert(canonical);
            continue;
        }
        // Toolset token names a *group* — expand to every tool whose name
        // contains the token (e.g. "browser" → browser_navigate, browser_click).
        let matched: Vec<&str> = all_names
            .iter()
            .copied()
            .filter(|n| n.starts_with(&canonical))
            .collect();
        if matched.is_empty() {
            tracing::warn!(
                platform = %platform,
                token = %original,
                "Unknown platform toolset/token — no matching tool or group"
            );
        }
        names.extend(matched.into_iter().map(str::to_string));
    }

    if all_requested || names.is_empty() {
        // Fallback: empty config (or explicit `all`) → every tool.
        return all_names.iter().map(|n| (*n).to_string()).collect();
    }

    let mut out: Vec<String> = names.into_iter().collect();
    out.sort();
    out
}

/// Resolve and filter [`ToolSpec`]s to those allowed for the given platform.
///
/// Empty allowed set → all specs pass through (legacy default).
pub fn resolve_platform_tool_specs(
    toolsets: &HashMap<String, Vec<String>>,
    platform: &str,
    specs: &[ToolSpec],
) -> Vec<ToolSpec> {
    let allowed = resolve_platform_tool_names(toolsets, platform, &spec_names(specs));
    if allowed.is_empty() {
        return specs.to_vec();
    }
    let allowed_set: HashSet<&str> = allowed.iter().map(String::as_str).collect();
    specs
        .iter()
        .filter(|spec| allowed_set.contains(spec.name.as_str()))
        .cloned()
        .collect()
}

/// Compact `{name, description}` summary for hooks / transcript metadata.
pub fn tool_definition_summary(specs: &[ToolSpec]) -> Vec<serde_json::Value> {
    specs
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "description": spec.description,
            })
        })
        .collect()
}

fn spec_names(specs: &[ToolSpec]) -> Vec<&str> {
    specs.iter().map(|spec| spec.name.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: format!("{name} tool"),
            parameters: serde_json::json!({"type": "object"}),
        }
    }

    fn all_tool_names() -> Vec<&'static str> {
        vec![
            "web_search",
            "web_extract",
            "browser_navigate",
            "browser_click",
            "terminal",
            "read_file",
            "write_file",
            "skills_list",
            "memory_recall",
            "todo",
            "delegate_task",
        ]
    }

    #[test]
    fn normalize_platform_aliases() {
        assert_eq!(normalize_platform_key("local"), "cli");
        assert_eq!(normalize_platform_key("TG"), "telegram");
        assert_eq!(normalize_platform_key("dc"), "discord");
        assert_eq!(normalize_platform_key("api"), "api_server");
        assert_eq!(normalize_platform_key("whatsapp"), "whatsapp");
    }

    #[test]
    fn canonicalize_toolset_aliases() {
        assert_eq!(canonical_toolset_token("browser-use"), "browser");
        assert_eq!(canonical_toolset_token("browser_use"), "browser");
        assert_eq!(canonical_toolset_token("Browser"), "browser");
        assert_eq!(canonical_toolset_token("code"), "code_execution");
        assert_eq!(
            canonical_toolset_token("home_assistant_tools"),
            "homeassistant"
        );
        assert_eq!(canonical_toolset_token("web-tools"), "web");
    }

    #[test]
    fn empty_config_resolves_all_tools() {
        let toolsets = HashMap::new();
        let names = resolve_platform_tool_names(&toolsets, "cli", &all_tool_names());
        assert_eq!(names, all_tool_names());
    }

    #[test]
    fn explicit_entry_is_an_allow_list() {
        let mut toolsets = HashMap::new();
        toolsets.insert("cli".to_string(), vec!["web".to_string()]);
        let names = resolve_platform_tool_names(&toolsets, "cli", &all_tool_names());
        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"web_extract".to_string()));
        assert!(!names.contains(&"terminal".to_string()));
    }

    #[test]
    fn toolset_group_expands_to_prefix_tools() {
        let mut toolsets = HashMap::new();
        toolsets.insert("telegram".to_string(), vec!["browser".to_string()]);
        let names = resolve_platform_tool_names(&toolsets, "telegram", &all_tool_names());
        assert!(names.contains(&"browser_navigate".to_string()));
        assert!(names.contains(&"browser_click".to_string()));
        assert!(!names.contains(&"web_search".to_string()));
    }

    #[test]
    fn default_platform_toolset_used_when_no_config() {
        let toolsets = HashMap::new();
        let names = resolve_platform_tool_names(&toolsets, "telegram", &all_tool_names());
        // Default telegram set: web + browser + skills + memory + todo + delegation.
        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"browser_navigate".to_string()));
        assert!(!names.contains(&"terminal".to_string()));
        assert!(!names.contains(&"write_file".to_string()));
    }

    #[test]
    fn all_and_star_expand_to_everything() {
        for token in ["all", "*"] {
            let mut toolsets = HashMap::new();
            toolsets.insert("cli".to_string(), vec![token.to_string()]);
            let names = resolve_platform_tool_names(&toolsets, "cli", &all_tool_names());
            assert_eq!(names, all_tool_names(), "token {token} should mean all");
        }
    }

    #[test]
    fn unknown_token_is_ignored_and_empty_result_falls_back_to_all() {
        let mut toolsets = HashMap::new();
        toolsets.insert("cli".to_string(), vec!["no-such-tool".to_string()]);
        let names = resolve_platform_tool_names(&toolsets, "cli", &all_tool_names());
        // Unknown token matches nothing → resolver falls back to all (never
        // silently strips the whole toolset on a typo).
        assert_eq!(names, all_tool_names());
    }

    #[test]
    fn resolve_specs_filters_and_summary_extracts_name_description() {
        let all_specs: Vec<ToolSpec> = all_tool_names().iter().map(|n| spec(n)).collect();
        let mut toolsets = HashMap::new();
        toolsets.insert("discord".to_string(), vec!["web".to_string()]);

        let mut filtered = resolve_platform_tool_specs(&toolsets, "discord", &all_specs);
        filtered.sort_by(|a, b| a.name.cmp(&b.name));
        let filtered_names: Vec<&str> = filtered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(filtered_names, vec!["web_extract", "web_search"]);

        let summary = tool_definition_summary(&filtered);
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0]["name"], "web_extract");
        assert!(
            summary[0]["description"]
                .as_str()
                .unwrap()
                .contains("web_extract")
        );
    }
}

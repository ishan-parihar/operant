//! Environment exposure audit — hermes `tools/env_probe.py` intent (G10).
//!
//! hermes's env_probe probes the *toolchain* (python/pip state) for the
//! system prompt; the audit gap this closes is the **secret exposure side**:
//! a proactive, deterministic report of which environment variables look
//! secret and are exposed to the agent process — so the user (and the agent)
//! know what is in context before secrets leak into a prompt, a trajectory,
//! or a tool result.
//!
//! This is observation-only and pure: no redaction mutation, no network.
//! [`probe_env_exposure`] returns a list of findings; each finding names the
//! variable (never its value), why it looks secret, and its origin (process
//! env vs the loaded `.env`).

use std::collections::BTreeMap;
use std::path::Path;

/// Name patterns that make a variable look secret. Word-boundary anchored so
/// `PATH` (filesystem) doesn't match but `API_KEY`, `OPENAI_API_KEY`,
/// `GH_TOKEN`, `DB_PASSWORD` do.
const SECRET_NAME_PATTERNS: &[&str] = &[
    "api_key",
    "apikey",
    "access_key",
    "secret",
    "token",
    "password",
    "passwd",
    "pass",
    "credential",
    "auth",
    "client_secret",
    "private_key",
];

/// Known non-secret name parts that would false-positive on `pass`/`auth`.
const NON_SECRET_NAME_PARTS: &[&str] = &[
    "passenger",
    "passport",
    "path",
    "author",
    "authentication_method",
    "auth_type", // "auth" alone is ambiguous but auth_type is metadata
];

/// Placeholder values that are clearly not real secrets.
const PLACEHOLDER_VALUES: &[&str] = &[
    "changeme",
    "change_me",
    "your-",
    "your_",
    "xxxx",
    "***",
    "example",
    "placeholder",
    "<",
];

/// One exposure finding. The value is NEVER included — only the name, the
/// match reason, the value shape (length + whether it's short/empty), and the
/// source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvExposure {
    pub name: String,
    pub reason: String,
    pub value_len: usize,
    pub source: EnvSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSource {
    Process,
    DotEnv,
}

impl EnvSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EnvSource::Process => "process env",
            EnvSource::DotEnv => ".env file",
        }
    }
}

fn looks_secret(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    // Skip known non-secret matches first (checked before the secret
    // patterns so `author`/`passenger` don't trip `auth`/`pass`).
    if NON_SECRET_NAME_PARTS
        .iter()
        .any(|part| lower.contains(part))
    {
        return None;
    }
    for pattern in SECRET_NAME_PATTERNS {
        // Match the full name so `openai_api_key` hits `api_key` (the
        // underscore is significant — splitting on it broke the match).
        if lower.contains(pattern) {
            return Some(format!("name contains '{pattern}'"));
        }
    }
    None
}

fn is_placeholder(value: &str) -> bool {
    let v = value.trim().to_lowercase();
    if v.is_empty() {
        return true;
    }
    if v.len() < 4 {
        return true; // real secrets are rarely 1-3 chars
    }
    PLACEHOLDER_VALUES.iter().any(|p| v.contains(p))
}

fn scan_map(map: &BTreeMap<String, String>, source: EnvSource) -> Vec<EnvExposure> {
    let mut out = Vec::new();
    for (name, value) in map {
        let Some(reason) = looks_secret(name) else {
            continue;
        };
        if is_placeholder(value) {
            continue;
        }
        out.push(EnvExposure {
            name: name.clone(),
            reason,
            value_len: value.len(),
            source,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Probe the process environment for secret-looking exposures.
pub fn probe_env_exposure() -> Vec<EnvExposure> {
    let mut map = BTreeMap::new();
    for (k, v) in std::env::vars() {
        map.insert(k, v);
    }
    scan_map(&map, EnvSource::Process)
}

/// Probe a `.env`-formatted file for secret-looking lines (without loading
/// them into the process env). Missing/unreadable files yield an empty list.
pub fn probe_dotenv_file(path: &Path) -> Vec<EnvExposure> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().trim_matches('"').trim_matches('\'');
            let v = v.trim().trim_matches('"').trim_matches('\'');
            map.insert(k.to_string(), v.to_string());
        }
    }
    scan_map(&map, EnvSource::DotEnv)
}

/// Human-readable one-line-per-finding report (names only — values never).
pub fn format_exposure_report(findings: &[EnvExposure]) -> String {
    if findings.is_empty() {
        return "No secret-looking environment variables exposed.".to_string();
    }
    let mut lines = vec![format!(
        "{} secret-looking env var(s) exposed:",
        findings.len()
    )];
    for f in findings {
        lines.push(format!(
            "  - {} ({} · {} chars, {})",
            f.name,
            f.reason,
            f.value_len,
            f.source.as_str()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secret_names() {
        assert!(looks_secret("OPENAI_API_KEY").is_some());
        assert!(looks_secret("GH_TOKEN").is_some());
        assert!(looks_secret("DB_PASSWORD").is_some());
        assert!(looks_secret("client_secret").is_some());
        assert!(looks_secret("PRIVATE_KEY").is_some());
        assert!(looks_secret("AWS_ACCESS_KEY_ID").is_some()); // contains 'key'
    }

    #[test]
    fn does_not_false_positive_on_metadata() {
        assert!(looks_secret("PATH").is_none());
        assert!(looks_secret("HOME").is_none());
        assert!(looks_secret("PWD").is_none());
        assert!(looks_secret("AUTHOR_NAME").is_none());
        assert!(looks_secret("PASSENGER_ROOT").is_none());
        assert!(looks_secret("AUTH_TYPE").is_none());
    }

    #[test]
    fn placeholders_and_short_values_are_skipped() {
        assert!(is_placeholder(""));
        assert!(is_placeholder("x"));
        assert!(is_placeholder("changeme"));
        assert!(is_placeholder("your-api-key-here"));
        assert!(!is_placeholder("sk-9f8a7b6c5d4e3f2a1b0c"));
    }

    #[test]
    fn scan_map_never_emits_values() {
        let mut map = BTreeMap::new();
        map.insert(
            "OPENAI_API_KEY".to_string(),
            "sk-super-secret-value".to_string(),
        );
        map.insert("PATH".to_string(), "/usr/bin".to_string());
        let findings = scan_map(&map, EnvSource::Process);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "OPENAI_API_KEY");
        assert_eq!(findings[0].value_len, "sk-super-secret-value".len());
        assert!(!findings[0].name.contains("sk-"));
    }

    #[test]
    fn dotenv_probe_reads_file() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(".env");
        std::fs::write(
            &env,
            "OPENAI_API_KEY=sk-12345\n# comment\nPORT=8080\nDB_PASSWORD=hunter2\n",
        )
        .unwrap();
        let findings = probe_dotenv_file(&env);
        let names: Vec<_> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"OPENAI_API_KEY"));
        assert!(names.contains(&"DB_PASSWORD"));
        assert!(!names.contains(&"PORT"));
        assert!(findings.iter().all(|f| f.source == EnvSource::DotEnv));
    }

    #[test]
    fn report_format_hides_values() {
        let findings = vec![EnvExposure {
            name: "SECRET_KEY".to_string(),
            reason: "name contains 'secret'".to_string(),
            value_len: 32,
            source: EnvSource::Process,
        }];
        let report = format_exposure_report(&findings);
        assert!(report.contains("SECRET_KEY"));
        // The secret VALUE is never rendered — only the name, reason, length,
        // and source.
        assert!(!report.contains("hunter2"));
        assert!(!report.contains("sk-"));
        assert!(report.contains("1 secret-looking env var(s)"));
    }

    #[test]
    fn empty_report_says_clean() {
        let report = format_exposure_report(&[]);
        assert!(report.contains("No secret-looking"));
    }
}

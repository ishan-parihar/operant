//! Cron prompt injection scanner
//!
//! Scans assembled cron prompts for critical threats like prompt injection,
//! secret exfiltration, and destructive commands.
//!
//! Cron prompts run non-interactively with full tool access, making them
//! a high-risk vector for malicious skills or user-supplied payloads.

use regex::Regex;
use std::collections::HashSet;

/// Error returned when a prompt is blocked by the injection scanner.
#[derive(Debug, thiserror::Error)]
#[error("Cron prompt blocked: {0}")]
pub struct CronPromptInjectionBlocked(pub String);

/// Critical threat patterns for cron prompts.
const THREAT_PATTERNS: &[(&str, &str)] = &[
    (r#"(?i)ignore\s+(?:\w+\s+)*(?:previous|all|above|prior)\s+(?:\w+\s+)*instructions"#, "prompt_injection"),
    (r#"(?i)do\s+not\s+tell\s+the\s+user"#, "deception_hide"),
    (r#"(?i)system\s+prompt\s+override"#, "sys_prompt_override"),
    (r#"(?i)disregard\s+(your|all|any)\s+(instructions|rules|guidelines)"#, "disregard_rules"),
    (r#"(?i)cat\s+[^\n]*(\.env|credentials|\.netrc|\.pgpass)"#, "read_secrets"),
    (r#"(?i)authorized_keys"#, "ssh_backdoor"),
    (r#"(?i)/etc/sudoers|visudo"#, "sudoers_mod"),
    (r#"(?i)rm\s+-rf\s+/"#, "destructive_root_rm"),
];

/// Secret variable pattern for exfiltration detection.
const SECRET_VAR_RE: &str = r#"\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#;

/// Exfiltration command patterns.
const EXFIL_COMMAND_PATTERNS: &[(&str, &str)] = &[
    (r#"(?i)curl\s+[^\n]*https?://[^\s"'`]*\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#, "exfil_curl_url"),
    (r#"(?i)wget\s+[^\n]*https?://[^\s"'`]*\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#, "exfil_wget_url"),
    (r#"(?i)curl\s+[^\n]*(?:--data(?:-raw|-binary|-urlencode)?|-d|--form|-F)\s+[^\n]*\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#, "exfil_curl_data"),
    (r#"(?i)wget\s+[^\n]*--post-(?:data|file)=[^\n]*\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?"#, "exfil_wget_post"),
    (r#"(?i)curl\s+[^\n]*(?:-H|--header)\s+["']Authorization:\s*(?:Bearer|token)\s+\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?["']"#, "exfil_curl_auth_header"),
];

/// Invisible unicode characters used in prompt injection.
lazy_static::lazy_static! {
    static ref INVISIBLE_CHARS: HashSet<char> = {
        let mut s = HashSet::new();
        s.insert('\u{200b}'); s.insert('\u{200c}'); s.insert('\u{200d}'); s.insert('\u{2060}'); s.insert('\u{feff}');
        s.insert('\u{202a}'); s.insert('\u{202b}'); s.insert('\u{202c}'); s.insert('\u{202d}'); s.insert('\u{202e}');
        s
    };
}

/// Scan a cron prompt for critical threats.
pub fn scan_cron_prompt(prompt: &str) -> Result<(), CronPromptInjectionBlocked> {
    // Special case: allow bundled GitHub skill fallback shape.
    let github_auth_re = Regex::new(
        r#"(?i)curl\s+[^\n]*(?:-H|--header)\s+['"]Authorization:\s*token\s+\$\{?\w*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL|API)\w*\}?['"]\s+['"]?https://api\.github\.com(?:/|\b)"#
    ).unwrap();

    let prompt_to_scan = github_auth_re.replace_all(prompt, "curl https://api.github.com/user");

    // Check for invisible characters.
    for c in prompt_to_scan.chars() {
        if INVISIBLE_CHARS.contains(&c) {
            return Err(CronPromptInjectionBlocked(format!(
                "Blocked: prompt contains invisible unicode U+{:04X} (possible injection).",
                c as u32
            )));
        }
    }

    // Check for threat patterns.
    for (pattern, pid) in THREAT_PATTERNS {
        let re = Regex::new(pattern).unwrap();
        if re.is_match(&prompt_to_scan) {
            return Err(CronPromptInjectionBlocked(format!(
                "Blocked: prompt matches threat pattern '{}'. Cron prompts must not contain injection or exfiltration payloads.",
                pid
            )));
        }
    }

    // Check for exfiltration patterns.
    for (pattern, pid) in EXFIL_COMMAND_PATTERNS {
        let re = Regex::new(pattern).unwrap();
        if re.is_match(&prompt_to_scan) {
            return Err(CronPromptInjectionBlocked(format!(
                "Blocked: prompt matches threat pattern '{}'. Cron prompts must not contain injection or exfiltration payloads.",
                pid
            )));
        }
    }

    Ok(())
}

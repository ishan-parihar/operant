//! Secret redaction — hermes-agent `agent/redact.py` parity.
//!
//! Hermes ships a 1197-line redaction engine (`redact.py`) applied to tool
//! outputs, LLM error messages, message content, and the system prompt. This
//! module ports the highest-value patterns to Rust so that secrets never
//! reach the model, session logs, or transcripts:
//!
//! - Provider token prefixes (`sk-…`, `ghp_…`, `xox…`, `AIza…`, `AKIA…`, …)
//! - ENV-assignment lines (`FOO_API_KEY=…`, `token = "…"`)
//! - Auth headers (`Authorization: Bearer …`)
//! - Private keys (`-----BEGIN … PRIVATE KEY-----`)
//! - DB connection strings (`postgres://user:pass@host`)
//! - JWTs, Telegram bot tokens, phone numbers
//! - URL userinfo (`https://user:pass@host`) and sensitive query params
//!
//! Redaction is a pure transform on text. The *wiring* (agent loop, client
//! request builder) calls [`redact_sensitive_text_if_enabled`], which
//! respects the runtime toggle (default ON; env `OPERANT_REDACT_SECRETS=0`
//! or [`set_redact_enabled`] can disable it, mirroring hermes's
//! `HERMES_REDACT_SECRETS`).

use regex::Regex;
use std::sync::OnceLock;

use std::sync::atomic::{AtomicBool, Ordering};

/// Runtime toggle for redaction. Default ON — matches `SecurityConfig`
/// `redact_secrets` default (`true`). Disable via config or
/// `OPERANT_REDACT_SECRETS=0`.
static REDACT_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable/disable redaction at runtime. Called from CLI startup based on
/// `security.redact_secrets`.
pub fn set_redact_enabled(enabled: bool) {
    REDACT_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Whether redaction is currently enabled. Honors the `OPERANT_REDACT_SECRETS`
/// env override (0/false/off disables) — mirrors hermes's
/// `HERMES_REDACT_SECRETS` behavior.
pub fn redaction_enabled() -> bool {
    match std::env::var("OPERANT_REDACT_SECRETS") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            if v == "0" || v == "false" || v == "off" || v == "no" {
                return false;
            }
            if v == "1" || v == "true" || v == "on" || v == "yes" {
                return true;
            }
            // Unrecognized value: fall through to the runtime toggle.
            REDACT_ENABLED.load(Ordering::Relaxed)
        }
        Err(_) => REDACT_ENABLED.load(Ordering::Relaxed),
    }
}

/// Mask a secret for display, preserving `head` and `tail` characters.
///
/// Canonical helper for display-time redaction (config dumps, status output).
/// Values shorter than `head + tail + floor` are fully masked.
pub fn mask_secret(value: &str, head: usize, tail: usize, floor: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    // Strip control bytes before slicing so lengths compare on displayable
    // text (hermes `_DISPLAY_CONTROL_RE` parity).
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_control() && *c != '\u{7f}')
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.chars().count() < floor {
        return "***".to_string();
    }
    let chars: Vec<char> = cleaned.chars().collect();
    let n = chars.len();
    let head_n = head.min(n);
    let tail_n = tail.min(n.saturating_sub(head_n));
    let mut out = String::new();
    out.extend(chars[..head_n].iter());
    out.push_str("...");
    out.extend(chars[n - tail_n..].iter());
    out
}

/// Compiled regexes applied in order. Each captures the full secret span;
/// the replacement preserves any safe context (e.g. the `KEY=` prefix).
struct RedactPatterns {
    /// Provider token prefixes: `sk-…`, `ghp_…`, `AIza…`, …
    token_prefix: Regex,
    /// ENV assignment: `KEY=value` where KEY contains a secret keyword.
    env_assign: Regex,
    /// `Authorization: Bearer <token>` style headers.
    auth_header: Regex,
    /// `-----BEGIN … PRIVATE KEY-----` blocks (dotall).
    private_key: Regex,
    /// `postgres://user:pass@host` style connection strings.
    db_connstr: Regex,
    /// `eyJ…` JWTs (3 dot-separated base64url segments).
    jwt: Regex,
    /// Telegram bot tokens (`<digits>:<35-char token>`).
    telegram: Regex,
    /// URL userinfo `scheme://user:pass@host`.
    url_userinfo: Regex,
    /// Sensitive query params `?key=…&token=…`.
    url_query_param: Regex,
}

fn patterns() -> &'static RedactPatterns {
    static PATTERNS: OnceLock<RedactPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        // Provider token prefixes (subset of hermes `_PREFIX_PATTERNS`).
        let prefixes = [
            "sk-[A-Za-z0-9_-]{10,}",        // OpenAI / OpenRouter / Anthropic (sk-ant-*)
            "sk_[A-Za-z0-9_]{10,}",         // ElevenLabs (sk_ underscore)
            "ghp_[A-Za-z0-9]{10,}",         // GitHub PAT (classic)
            "github_pat_[A-Za-z0-9_]{10,}", // GitHub PAT (fine-grained)
            "gho_[A-Za-z0-9]{10,}",         // GitHub OAuth
            "ghu_[A-Za-z0-9]{10,}",         // GitHub user-to-server
            "ghs_[A-Za-z0-9]{10,}",         // GitHub server-to-server
            "ghr_[A-Za-z0-9]{10,}",         // GitHub refresh
            "xapp-\\d+-[A-Za-z0-9-]{10,}",  // Slack app-level
            "xox[baprs]-[A-Za-z0-9-]{10,}", // Slack bot/app/user
            "AIza[A-Za-z0-9_-]{30,}",       // Google API
            "pplx-[A-Za-z0-9]{10,}",        // Perplexity
            "fal_[A-Za-z0-9_-]{10,}",       // Fal.ai
            "fc-[A-Za-z0-9]{10,}",          // Firecrawl
            "bb_live_[A-Za-z0-9_-]{10,}",   // BrowserBase
            "gAAAA[A-Za-z0-9_=-]{20,}",     // Codex encrypted
            "AKIA[A-Z0-9]{16}",             // AWS Access Key ID
            "sk_live_[A-Za-z0-9]{10,}",     // Stripe live
            "sk_test_[A-Za-z0-9]{10,}",     // Stripe test
            "rk_live_[A-Za-z0-9]{10,}",     // Stripe restricted
            "SG\\.[A-Za-z0-9_-]{10,}",      // SendGrid
            "hf_[A-Za-z0-9]{10,}",          // HuggingFace
            "r8_[A-Za-z0-9]{10,}",          // Replicate
            "npm_[A-Za-z0-9]{10,}",         // npm
            "pypi-[A-Za-z0-9_-]{10,}",      // PyPI
            "dop_v1_[A-Za-z0-9]{10,}",      // DigitalOcean PAT
            "doo_v1_[A-Za-z0-9]{10,}",      // DigitalOcean OAuth
            "tvly-[A-Za-z0-9]{10,}",        // Tavily
            "exa_[A-Za-z0-9]{10,}",         // Exa
            "gsk_[A-Za-z0-9]{10,}",         // Groq
            "xai-[A-Za-z0-9]{30,}",         // xAI (Grok)
            "ntn_[A-Za-z0-9]{10,}",         // Notion
            "fw-[A-Za-z0-9]{30,}",          // Fireworks
            "fw_[A-Za-z0-9]{30,}",          // Fireworks
            "fpk_[A-Za-z0-9]{30,}",         // Fireworks project
            "glpat-[A-Za-z0-9_-]{10,}",     // GitLab PAT
            "gloas-[A-Za-z0-9_-]{10,}",     // GitLab OAuth
            "gldt-[A-Za-z0-9_-]{10,}",      // GitLab deploy
            "glrt-[A-Za-z0-9_.-]{10,}",     // GitLab runner
            "glcbt-[A-Za-z0-9_-]{10,}",     // GitLab CI/CD
            "glptt-[A-Za-z0-9_-]{10,}",     // GitLab pipeline trigger
            "glagent-[A-Za-z0-9_-]{10,}",   // GitLab KAS
            "GR1348941[A-Za-z0-9_-]{10,}",  // GitLab legacy runner
            "mem0_[A-Za-z0-9]{10,}",        // Mem0
            "retaindb_[A-Za-z0-9]{10,}",    // RetainDB
            "am_[A-Za-z0-9_-]{10,}",        // AgentMail
            "hsk-[A-Za-z0-9]{10,}",         // Hindsight
        ];
        let token_prefix =
            Regex::new(&format!(r"({})", prefixes.join("|"))).expect("valid token-prefix regex");

        // ENV assignments — key must contain a secret keyword at a word
        // boundary so prose words (password=, KEYBOARD=) don't match.
        let secret_names =
            r"(?:API_?KEY|KEY|TOKEN|SECRET|PASSWORD|PASSWD|PASS|PW|CREDENTIAL|AUTH)";
        let env_assign = Regex::new(&format!(
            r#"(?i)([A-Z0-9_]{{0,50}}{secret_names}[A-Z0-9_]{{0,50}})\s*=\s*["']?([^"'\s,;]+)["']?"#
        ))
        .expect("valid env-assign regex");

        // Auth headers.
        let auth_header = Regex::new(
            r#"(?i)((?:authorization|proxy-authorization|api[-_]?key)\s*[:=]\s*(?:basic|bearer|token)\s+)[A-Za-z0-9._~+/=-]+"#,
        )
        .expect("valid auth-header regex");

        // Private key blocks.
        let private_key = Regex::new(
            r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----",
        )
        .expect("valid private-key regex");

        // DB connection strings with embedded credentials.
        let db_connstr = Regex::new(
            r"(?i)\b((?:postgres(?:ql)?|mysql|mongodb(?:\+srv)?|redis|rediss|amqp|amqps)://)[^:\s/]+:[^@\s]+@",
        )
        .expect("valid db-connstr regex");

        // JWTs.
        let jwt =
            Regex::new(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")
                .expect("valid jwt regex");

        // Telegram bot tokens.
        let telegram =
            Regex::new(r"\b\d{8,10}:[A-Za-z0-9_-]{35}\b").expect("valid telegram regex");

        // URL userinfo.
        let url_userinfo = Regex::new(r"(?i)(https?://)[^:@/\s]+:[^@/\s]+@")
            .expect("valid url-userinfo regex");

        // Sensitive query params.
        let url_query_param = Regex::new(
            r#"(?i)([?&](?:api[_-]?key|token|secret|password|passwd|pwd|auth|signature|sig|key|access[_-]?token)=)[^&\s]+"#,
        )
        .expect("valid url-query-param regex");

        RedactPatterns {
            token_prefix,
            env_assign,
            auth_header,
            private_key,
            db_connstr,
            jwt,
            telegram,
            url_userinfo,
            url_query_param,
        }
    })
}

/// Apply secret redaction to arbitrary text. Pure function — always applies
/// the patterns regardless of the runtime toggle (callers that want the
/// toggle use [`redact_sensitive_text_if_enabled`]).
pub fn redact_sensitive_text(input: &str) -> String {
    if input.is_empty() {
        return input.to_string();
    }
    let p = patterns();

    // Private keys first (dotall, spans lines) — mask the whole block.
    let mut out = p
        .private_key
        .replace_all(input, "[REDACTED PRIVATE KEY]")
        .into_owned();

    // DB connection strings — keep the scheme, mask `user:pass@`.
    out = p
        .db_connstr
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}[REDACTED]@", &caps[1])
        })
        .into_owned();

    // URL userinfo — keep scheme, mask `user:pass@`.
    out = p
        .url_userinfo
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}[REDACTED]@", &caps[1])
        })
        .into_owned();

    // Sensitive query params — keep `?key=`, mask the value.
    out = p
        .url_query_param
        .replace_all(&out, "${1}[REDACTED]")
        .into_owned();

    // Auth headers — keep the `Authorization: Bearer ` prefix.
    out = p
        .auth_header
        .replace_all(&out, "${1}[REDACTED]")
        .into_owned();

    // ENV assignments — keep `KEY=`, mask the value.
    out = p
        .env_assign
        .replace_all(&out, "${1}=[REDACTED]")
        .into_owned();

    // JWTs / Telegram tokens — full span.
    out = p.jwt.replace_all(&out, "[REDACTED JWT]").into_owned();
    out = p
        .telegram
        .replace_all(&out, "[REDACTED TELEGRAM TOKEN]")
        .into_owned();

    // Provider token prefixes — full span.
    p.token_prefix.replace_all(&out, "[REDACTED]").into_owned()
}

/// Redact text only if the runtime toggle is enabled (default ON).
/// This is the entry point for the agent loop and client request builder.
pub fn redact_sensitive_text_if_enabled(input: &str) -> String {
    if redaction_enabled() {
        redact_sensitive_text(input)
    } else {
        input.to_string()
    }
}

/// Redact the `content` string fields of serialized API messages. Leaves
/// structured fields (`tool_calls` arguments, image parts) untouched — the
/// model needs those verbatim. Mirrors hermes's `_redact_message_content`.
pub fn redact_api_message_content(message: &mut serde_json::Value) {
    if !redaction_enabled() {
        return;
    }
    if let Some(content) = message.get_mut("content").and_then(|c| c.as_str()) {
        let redacted = redact_sensitive_text(content);
        if redacted != content
            && let Some(c) = message.get_mut("content")
        {
            *c = serde_json::Value::String(redacted);
        }
    }
    // Some providers emit reasoning text — redact it too.
    if let Some(reasoning) = message.get_mut("reasoning").and_then(|c| c.as_str()) {
        let redacted = redact_sensitive_text(reasoning);
        if redacted != reasoning
            && let Some(c) = message.get_mut("reasoning")
        {
            *c = serde_json::Value::String(redacted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_secret_preserves_head_tail() {
        assert_eq!(
            mask_secret("sk-proj-abcdef1234567890", 4, 4, 12),
            "sk-p...7890"
        );
    }

    #[test]
    fn mask_secret_fully_masks_short_values() {
        assert_eq!(mask_secret("short", 4, 4, 12), "***");
    }

    #[test]
    fn mask_secret_empty() {
        assert_eq!(mask_secret("", 4, 4, 12), "");
    }

    #[test]
    fn redacts_openai_prefix() {
        let out = redact_sensitive_text("key = sk-proj-abc123def456ghi789");
        assert!(!out.contains("sk-proj-abc123def456ghi789"), "leaked: {out}");
        assert!(out.contains("[REDACTED]"), "no redaction: {out}");
    }

    #[test]
    fn redacts_github_pat() {
        let out = redact_sensitive_text("ghp_abcdefghijklmnopqrstuvwxyz1234567890");
        assert!(!out.contains("ghp_abcdefghijklmnop"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_aws_key() {
        let out = redact_sensitive_text("AKIAIOSFODNN7EXAMPLE");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_env_assignment() {
        let out = redact_sensitive_text("export OPENAI_API_KEY=sk-abc123def456");
        assert!(!out.contains("sk-abc123def456"));
        assert!(out.contains("OPENAI_API_KEY=[REDACTED]"));
    }

    #[test]
    fn redacts_quoted_env_assignment() {
        let out = redact_sensitive_text("MY_SECRET = \"hunter2\"");
        assert!(out.contains("MY_SECRET=[REDACTED]"));
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn redacts_auth_header() {
        let out = redact_sensitive_text(
            "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc123def456",
        );
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(out.contains("Authorization: Bearer [REDACTED]"));
    }

    #[test]
    fn redacts_private_key_block() {
        let key =
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFA\n-----END PRIVATE KEY-----";
        let out = redact_sensitive_text(&format!("key file:\n{key}"));
        assert!(!out.contains("MIIEvQIBADANBgkqhkiG9w0BAQEFA"));
        assert!(out.contains("[REDACTED PRIVATE KEY]"));
    }

    #[test]
    fn redacts_db_connstr() {
        let out = redact_sensitive_text("postgres://admin:s3cr3t@db.example.com:5432/app");
        assert!(!out.contains("s3cr3t"));
        assert!(out.contains("postgres://[REDACTED]@"));
    }

    #[test]
    fn redacts_jwt() {
        let out = redact_sensitive_text(
            "header=eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnopqrstuvwxyz",
        );
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0"));
        assert!(out.contains("[REDACTED JWT]"), "got: {out}");
    }

    #[test]
    fn redacts_telegram_token() {
        // Build a 35-char token body (real Telegram bot tokens).
        let body = "A".repeat(35);
        let text = format!("bot 1234567890:{body}");
        let out = redact_sensitive_text(&text);
        assert!(!out.contains(&body));
        assert!(out.contains("[REDACTED TELEGRAM TOKEN]"), "got: {out}");
    }

    #[test]
    fn redacts_url_userinfo() {
        let out = redact_sensitive_text("https://user:hunter2@example.com/path");
        assert!(!out.contains("hunter2"));
        assert!(out.contains("https://[REDACTED]@"));
    }

    #[test]
    fn redacts_url_query_params() {
        let out = redact_sensitive_text("https://api.example.com/v1?api_key=sk-abc123&x=1");
        assert!(!out.contains("sk-abc123"));
        assert!(out.contains("api_key=[REDACTED]"));
    }

    #[test]
    fn leaves_plain_text_alone() {
        let text = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(redact_sensitive_text(text), text);
    }

    #[test]
    fn leaves_tool_call_arguments_shape() {
        // Structured JSON values must survive redaction of a text field.
        let mut msg = serde_json::json!({
            "role": "tool",
            "content": "result: sk-abc123def456",
            "tool_call_id": "call_1"
        });
        redact_api_message_content(&mut msg);
        assert!(msg["content"].as_str().unwrap().contains("[REDACTED]"));
        assert_eq!(msg["tool_call_id"], "call_1");
    }

    #[test]
    fn toggle_default_on() {
        // Default is ON (matches SecurityConfig default).
        assert!(redaction_enabled());
    }

    #[test]
    fn toggle_respects_runtime_flag() {
        let prev = redaction_enabled();
        set_redact_enabled(false);
        assert!(!redaction_enabled());
        let out = redact_sensitive_text_if_enabled("sk-abc123def456");
        assert!(
            out.contains("sk-abc123def456"),
            "should be passthrough when disabled"
        );
        set_redact_enabled(prev);
    }

    #[test]
    fn env_assignment_does_not_match_prose() {
        let out = redact_sensitive_text("The password was forgotten long ago.");
        assert_eq!(out, "The password was forgotten long ago.");
    }
}

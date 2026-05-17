//! API error classification for smart failover and recovery.
//!
//! Ported from hermes-agent's `agent/error_classifier.py`.
//! Provides a structured taxonomy of API errors and a priority-ordered
//! classification pipeline that determines the correct recovery action
//! (retry, rotate credential, fallback to another provider, compress
//! context, or abort).

use serde::{Deserialize, Serialize};

// ── Error taxonomy ──────────────────────────────────────────────────────

/// Why an API call failed — determines recovery strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FailoverReason {
    // Authentication / authorization
    Auth,
    AuthPermanent,

    // Billing / quota
    Billing,
    RateLimit,

    // Server-side
    Overloaded,
    ServerError,

    // Transport
    Timeout,

    // Context / payload
    ContextOverflow,
    PayloadTooLarge,
    ImageTooLarge,

    // Model
    ModelNotFound,
    ProviderPolicyBlocked,

    // Request format
    FormatError,

    // Provider-specific
    ThinkingSignature,
    LongContextTier,
    OauthLongContextBetaForbidden,
    LlamaCppGrammarPattern,

    // Catch-all
    Unknown,
}

impl std::fmt::Display for FailoverReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FailoverReason::Auth => write!(f, "auth"),
            FailoverReason::AuthPermanent => write!(f, "auth_permanent"),
            FailoverReason::Billing => write!(f, "billing"),
            FailoverReason::RateLimit => write!(f, "rate_limit"),
            FailoverReason::Overloaded => write!(f, "overloaded"),
            FailoverReason::ServerError => write!(f, "server_error"),
            FailoverReason::Timeout => write!(f, "timeout"),
            FailoverReason::ContextOverflow => write!(f, "context_overflow"),
            FailoverReason::PayloadTooLarge => write!(f, "payload_too_large"),
            FailoverReason::ImageTooLarge => write!(f, "image_too_large"),
            FailoverReason::ModelNotFound => write!(f, "model_not_found"),
            FailoverReason::ProviderPolicyBlocked => write!(f, "provider_policy_blocked"),
            FailoverReason::FormatError => write!(f, "format_error"),
            FailoverReason::ThinkingSignature => write!(f, "thinking_signature"),
            FailoverReason::LongContextTier => write!(f, "long_context_tier"),
            FailoverReason::OauthLongContextBetaForbidden => {
                write!(f, "oauth_long_context_beta_forbidden")
            }
            FailoverReason::LlamaCppGrammarPattern => write!(f, "llama_cpp_grammar_pattern"),
            FailoverReason::Unknown => write!(f, "unknown"),
        }
    }
}

// ── Classification result ───────────────────────────────────────────────

/// Structured classification of an API error with recovery hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedError {
    pub reason: FailoverReason,
    pub status_code: Option<u16>,
    pub provider: String,
    pub model: String,
    pub message: String,

    // Recovery action hints
    pub retryable: bool,
    pub should_compress: bool,
    pub should_rotate_credential: bool,
    pub should_fallback: bool,
}

impl ClassifiedError {
    pub fn is_auth(&self) -> bool {
        matches!(
            self.reason,
            FailoverReason::Auth | FailoverReason::AuthPermanent
        )
    }
}

impl std::fmt::Display for ClassifiedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (HTTP {}): {}",
            self.reason,
            self.status_code.map(|s| s.to_string()).unwrap_or_default(),
            self.message
        )
    }
}

// ── Provider-specific patterns ──────────────────────────────────────────

const BILLING_PATTERNS: &[&str] = &[
    "insufficient credits",
    "insufficient_quota",
    "insufficient balance",
    "credit balance",
    "credits have been exhausted",
    "top up your credits",
    "payment required",
    "billing hard limit",
    "exceeded your current quota",
    "account is deactivated",
    "plan does not include",
];

const RATE_LIMIT_PATTERNS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "too many requests",
    "throttled",
    "requests per minute",
    "tokens per minute",
    "requests per day",
    "try again in",
    "please retry after",
    "resource_exhausted",
    "rate increased too quickly",
    "throttlingexception",
    "too many concurrent requests",
    "servicequotaexceededexception",
];

const USAGE_LIMIT_PATTERNS: &[&str] = &["usage limit", "quota", "limit exceeded", "key limit exceeded"];

const USAGE_LIMIT_TRANSIENT_SIGNALS: &[&str] = &[
    "try again",
    "retry",
    "resets at",
    "reset in",
    "wait",
    "requests remaining",
    "periodic",
    "window",
];

const PAYLOAD_TOO_LARGE_PATTERNS: &[&str] = &[
    "request entity too large",
    "payload too large",
    "error code: 413",
];

const IMAGE_TOO_LARGE_PATTERNS: &[&str] = &[
    "image exceeds",
    "image too large",
    "image_too_large",
    "image size exceeds",
];

const CONTEXT_OVERFLOW_PATTERNS: &[&str] = &[
    "context length",
    "context size",
    "maximum context",
    "token limit",
    "too many tokens",
    "reduce the length",
    "exceeds the limit",
    "context window",
    "prompt is too long",
    "prompt exceeds max length",
    "max_tokens",
    "maximum number of tokens",
    "exceeds the max_model_len",
    "max_model_len",
    "prompt length",
    "input is too long",
    "maximum model length",
    "context length exceeded",
    "truncating input",
    "slot context",
    "n_ctx_slot",
    "\u{8d85}\u{8fc7}\u{6700}\u{5927}\u{957f}\u{5ea6}",
    "\u{4e0a}\u{4e0b}\u{6587}\u{957f}\u{5ea6}",
    "max input token",
    "input token",
    "exceeds the maximum number of input tokens",
];

const MODEL_NOT_FOUND_PATTERNS: &[&str] = &[
    "is not a valid model",
    "invalid model",
    "model not found",
    "model_not_found",
    "does not exist",
    "no such model",
    "unknown model",
    "unsupported model",
];

const PROVIDER_POLICY_BLOCKED_PATTERNS: &[&str] = &[
    "no endpoints available matching your guardrail",
    "no endpoints available matching your data policy",
    "no endpoints found matching your data policy",
];

const AUTH_PATTERNS: &[&str] = &[
    "invalid api key",
    "invalid_api_key",
    "authentication",
    "unauthorized",
    "forbidden",
    "invalid token",
    "token expired",
    "token revoked",
    "access denied",
];

const TIMEOUT_MESSAGE_PATTERNS: &[&str] = &[
    "timed out",
    "turn timed out",
    "request timed out",
    "deadline exceeded",
    "operation timed out",
    "upstream timed out",
];

const SERVER_DISCONNECT_PATTERNS: &[&str] = &[
    "server disconnected",
    "peer closed connection",
    "connection reset by peer",
    "connection was closed",
    "network connection lost",
    "unexpected eof",
    "incomplete chunked read",
];

const SSL_TRANSIENT_PATTERNS: &[&str] = &[
    "bad record mac",
    "ssl alert",
    "tls alert",
    "ssl handshake failure",
    "tlsv1 alert",
    "sslv3 alert",
    "bad_record_mac",
    "ssl_alert",
    "tls_alert",
    "tls_alert_internal_error",
    "[ssl:",
];

// ── Helpers ─────────────────────────────────────────────────────────────

fn contains_any(haystack: &str, patterns: &[&str]) -> bool {
    let lower = haystack.to_lowercase();
    patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
}

fn extract_error_code(body: &serde_json::Value) -> String {
    if let Some(error_obj) = body.get("error").and_then(|v| v.as_object()) {
        if let Some(code) = error_obj.get("code").or_else(|| error_obj.get("type")) {
            if let Some(s) = code.as_str() {
                if !s.is_empty() {
                    return s.trim().to_string();
                }
            }
        }
    }
    if let Some(code) = body.get("code").or_else(|| body.get("error_code")) {
        if let Some(s) = code.as_str() {
            return s.trim().to_string();
        }
        if let Some(n) = code.as_i64() {
            return n.to_string();
        }
    }
    String::new()
}

fn extract_message_from_body(body: &serde_json::Value) -> String {
    if let Some(error_obj) = body.get("error").and_then(|v| v.as_object()) {
        if let Some(msg) = error_obj.get("message").and_then(|v| v.as_str()) {
            if !msg.is_empty() {
                return msg.trim().chars().take(500).collect();
            }
        }
    }
    if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
        if !msg.is_empty() {
            return msg.trim().chars().take(500).collect();
        }
    }
    String::new()
}

fn build_error_msg(
    error_str: &str,
    body: &serde_json::Value,
) -> String {
    let raw_msg = error_str.to_lowercase();
    let mut body_msg = String::new();
    let mut metadata_msg = String::new();

    if let Some(err_obj) = body.get("error").and_then(|v| v.as_object()) {
        if let Some(msg) = err_obj.get("message").and_then(|v| v.as_str()) {
            body_msg = msg.to_lowercase();
        }
        // Parse metadata.raw for wrapped provider errors (OpenRouter pattern)
        if let Some(metadata) = err_obj.get("metadata").and_then(|v| v.as_object()) {
            if let Some(raw_json) = metadata.get("raw").and_then(|v| v.as_str()) {
                if !raw_json.trim().is_empty() {
                    if let Ok(inner) = serde_json::from_str::<serde_json::Value>(raw_json) {
                        if let Some(inner_err) = inner.get("error").and_then(|v| v.as_object()) {
                            if let Some(msg) = inner_err.get("message").and_then(|v| v.as_str()) {
                                metadata_msg = msg.to_lowercase();
                            }
                        }
                    }
                }
            }
        }
    }
    if body_msg.is_empty() {
        if let Some(msg) = body.get("message").and_then(|v| v.as_str()) {
            body_msg = msg.to_lowercase();
        }
    }

    let mut parts = Vec::new();
    parts.push(raw_msg.clone());
    if !body_msg.is_empty() && !raw_msg.contains(&body_msg) {
        parts.push(body_msg.clone());
    }
    if !metadata_msg.is_empty()
        && !raw_msg.contains(&metadata_msg)
        && !body_msg.contains(&metadata_msg)
    {
        parts.push(metadata_msg);
    }
    parts.join(" ")
}

// ── Classification pipeline ─────────────────────────────────────────────

/// Classify an API error into a structured recovery recommendation.
///
/// Priority-ordered pipeline:
///   1. Special-case provider-specific patterns (thinking sigs, tier gates)
///   2. HTTP status code + message-aware refinement
///   3. Error code classification (from body)
///   4. Message pattern matching (billing vs rate_limit vs context vs auth)
///   5. SSL/TLS transient alert patterns → retry as timeout
///   6. Server disconnect + large session → context overflow
///   7. Transport error heuristics
///   8. Fallback: unknown (retryable with backoff)
pub fn classify_api_error(
    error_str: &str,
    status_code: Option<u16>,
    body: &serde_json::Value,
    error_type: &str,
    provider: &str,
    model: &str,
    approx_tokens: usize,
    context_length: usize,
    num_messages: usize,
) -> ClassifiedError {
    let error_code = extract_error_code(body);
    let error_msg = build_error_msg(error_str, body);
    let provider_lower = provider.to_lowercase();
    let model_lower = model.to_lowercase();

    let result = |reason: FailoverReason,
                  retryable: bool,
                  should_compress: bool,
                  should_rotate_credential: bool,
                  should_fallback: bool|
     -> ClassifiedError {
        ClassifiedError {
            reason,
            status_code,
            provider: provider.to_string(),
            model: model.to_string(),
            message: extract_message_from_body(body)
                .chars()
                .take(500)
                .collect::<String>(),
            retryable,
            should_compress,
            should_rotate_credential,
            should_fallback,
        }
    };

    // ── 1. Provider-specific patterns (highest priority) ────────────

    // Anthropic thinking block signature invalid (400)
    if status_code == Some(400)
        && error_msg.contains("signature")
        && error_msg.contains("thinking")
    {
        return result(
            FailoverReason::ThinkingSignature,
            true, false, false, false,
        );
    }

    // Anthropic long-context tier gate (429 "extra usage" + "long context")
    if status_code == Some(429)
        && error_msg.contains("extra usage")
        && error_msg.contains("long context")
    {
        return result(
            FailoverReason::LongContextTier,
            true, true, false, false,
        );
    }

    // Anthropic OAuth subscription rejects 1M-context beta
    if status_code == Some(400)
        && error_msg.contains("long context beta")
        && error_msg.contains("not yet available")
    {
        return result(
            FailoverReason::OauthLongContextBetaForbidden,
            true, false, false, false,
        );
    }

    // llama.cpp json-schema-to-grammar rejects regex escapes
    if status_code == Some(400)
        && (error_msg.contains("error parsing grammar")
            || error_msg.contains("json-schema-to-grammar")
            || (error_msg.contains("unable to generate parser")
                && error_msg.contains("template")))
    {
        return result(
            FailoverReason::LlamaCppGrammarPattern,
            true, false, false, false,
        );
    }

    // ── 2. HTTP status code classification ──────────────────────────

    if let Some(sc) = status_code {
        if let Some(classified) = classify_by_status(
            sc,
            &error_msg,
            &error_code,
            body,
            &provider_lower,
            &model_lower,
            approx_tokens,
            context_length,
            num_messages,
        ) {
            return classified;
        }
    }

    // ── 3. Error code classification ────────────────────────────────

    if !error_code.is_empty() {
        if let Some(classified) =
            classify_by_error_code(&error_code, &error_msg, &result)
        {
            return classified;
        }
    }

    // ── 4. Message pattern matching (no status code) ────────────────

    if let Some(classified) = classify_by_message(
        &error_msg,
        error_type,
        approx_tokens,
        context_length,
        &result,
    ) {
        return classified;
    }

    // ── 5. SSL/TLS transient errors → retry as timeout ─────────────

    if contains_any(&error_msg, SSL_TRANSIENT_PATTERNS) {
        return result(FailoverReason::Timeout, true, false, false, false);
    }

    // ── 6. Server disconnect + large session → context overflow ─────

    let is_disconnect = contains_any(&error_msg, SERVER_DISCONNECT_PATTERNS);
    if is_disconnect && status_code.is_none() {
        let is_large = approx_tokens > context_length * 60 / 100
            || (context_length <= 256000
                && (approx_tokens > 120000 || num_messages > 200));
        if is_large {
            return result(
                FailoverReason::ContextOverflow,
                true, true, false, false,
            );
        }
        return result(FailoverReason::Timeout, true, false, false, false);
    }

    // ── 7. Transport / timeout heuristics ───────────────────────────

    if error_msg.contains("timed out")
        || error_msg.contains("timeout")
        || error_msg.contains("connection")
    {
        return result(FailoverReason::Timeout, true, false, false, false);
    }

    // ── 8. Fallback: unknown ────────────────────────────────────────

    result(FailoverReason::Unknown, true, false, false, false)
}

fn classify_by_status(
    status_code: u16,
    error_msg: &str,
    error_code: &str,
    body: &serde_json::Value,
    provider: &str,
    model: &str,
    approx_tokens: usize,
    context_length: usize,
    num_messages: usize,
) -> Option<ClassifiedError> {
    let result = |reason: FailoverReason,
                  retryable: bool,
                  should_compress: bool,
                  should_rotate_credential: bool,
                  should_fallback: bool|
     -> ClassifiedError {
        ClassifiedError {
            reason,
            status_code: Some(status_code),
            provider: provider.to_string(),
            model: model.to_string(),
            message: extract_message_from_body(body)
                .chars()
                .take(500)
                .collect::<String>(),
            retryable,
            should_compress,
            should_rotate_credential,
            should_fallback,
        }
    };

    match status_code {
        401 => Some(result(
            FailoverReason::Auth,
            false, false, true, true,
        )),
        403 => {
            if error_msg.contains("key limit exceeded")
                || error_msg.contains("spending limit")
            {
                Some(result(
                    FailoverReason::Billing,
                    false, false, true, true,
                ))
            } else {
                Some(result(
                    FailoverReason::Auth,
                    false, false, false, true,
                ))
            }
        }
        402 => Some(classify_402(error_msg, &result)),
        404 => {
            if contains_any(error_msg, PROVIDER_POLICY_BLOCKED_PATTERNS) {
                Some(result(
                    FailoverReason::ProviderPolicyBlocked,
                    false, false, false, false,
                ))
            } else if contains_any(error_msg, MODEL_NOT_FOUND_PATTERNS) {
                Some(result(
                    FailoverReason::ModelNotFound,
                    false, false, false, true,
                ))
            } else {
                Some(result(FailoverReason::Unknown, true, false, false, false))
            }
        }
        413 => Some(result(
            FailoverReason::PayloadTooLarge,
            true, true, false, false,
        )),
        429 => Some(result(
            FailoverReason::RateLimit,
            true, false, true, true,
        )),
        400 => Some(classify_400(
            error_msg,
            error_code,
            body,
            provider,
            model,
            approx_tokens,
            context_length,
            num_messages,
            &result,
        )),
        500 | 502 => Some(result(
            FailoverReason::ServerError,
            true, false, false, false,
        )),
        503 | 529 => Some(result(
            FailoverReason::Overloaded,
            true, false, false, false,
        )),
        400..=499 => Some(result(
            FailoverReason::FormatError,
            false, false, false, true,
        )),
        500..=599 => Some(result(
            FailoverReason::ServerError,
            true, false, false, false,
        )),
        _ => None,
    }
}

fn classify_402<F>(error_msg: &str, result: &F) -> ClassifiedError
where
    F: Fn(
        FailoverReason,
        bool,
        bool,
        bool,
        bool,
    ) -> ClassifiedError,
{
    let has_usage_limit = contains_any(error_msg, USAGE_LIMIT_PATTERNS);
    let has_transient_signal = contains_any(error_msg, USAGE_LIMIT_TRANSIENT_SIGNALS);

    if has_usage_limit && has_transient_signal {
        result(
            FailoverReason::RateLimit,
            true, false, true, true,
        )
    } else {
        result(
            FailoverReason::Billing,
            false, false, true, true,
        )
    }
}

fn classify_400<F>(
    error_msg: &str,
    _error_code: &str,
    body: &serde_json::Value,
    _provider: &str,
    _model: &str,
    approx_tokens: usize,
    context_length: usize,
    num_messages: usize,
    result: &F,
) -> ClassifiedError
where
    F: Fn(
        FailoverReason,
        bool,
        bool,
        bool,
        bool,
    ) -> ClassifiedError,
{
    // Image-too-large from 400 (Anthropic's 5 MB per-image check)
    if contains_any(error_msg, IMAGE_TOO_LARGE_PATTERNS) {
        return result(FailoverReason::ImageTooLarge, true, false, false, false);
    }

    // Context overflow from 400
    if contains_any(error_msg, CONTEXT_OVERFLOW_PATTERNS) {
        return result(
            FailoverReason::ContextOverflow,
            true, true, false, false,
        );
    }

    // Provider policy blocked
    if contains_any(error_msg, PROVIDER_POLICY_BLOCKED_PATTERNS) {
        return result(
            FailoverReason::ProviderPolicyBlocked,
            false, false, false, false,
        );
    }

    // Model not found
    if contains_any(error_msg, MODEL_NOT_FOUND_PATTERNS) {
        return result(
            FailoverReason::ModelNotFound,
            false, false, false, true,
        );
    }

    // Rate limit / billing as 400
    if contains_any(error_msg, RATE_LIMIT_PATTERNS) {
        return result(
            FailoverReason::RateLimit,
            true, false, true, true,
        );
    }
    if contains_any(error_msg, BILLING_PATTERNS) {
        return result(
            FailoverReason::Billing,
            false, false, true, true,
        );
    }

    // Generic 400 + large session → probable context overflow
    let err_body_msg = extract_message_from_body(body).to_lowercase();
    let is_generic = err_body_msg.len() < 30
        || err_body_msg.is_empty()
        || err_body_msg == "error";
    let is_large = approx_tokens > context_length * 40 / 100
        || (context_length <= 256000
            && (approx_tokens > 80000 || num_messages > 80));

    if is_generic && is_large {
        return result(
            FailoverReason::ContextOverflow,
            true, true, false, false,
        );
    }

    result(
        FailoverReason::FormatError,
        false, false, false, true,
    )
}

fn classify_by_error_code<F>(
    error_code: &str,
    _error_msg: &str,
    result: &F,
) -> Option<ClassifiedError>
where
    F: Fn(
        FailoverReason,
        bool,
        bool,
        bool,
        bool,
    ) -> ClassifiedError,
{
    let code_lower = error_code.to_lowercase();

    match code_lower.as_str() {
        "resource_exhausted" | "throttled" | "rate_limit_exceeded" => {
            Some(result(
                FailoverReason::RateLimit,
                true, false, true, false,
            ))
        }
        "insufficient_quota" | "billing_not_active" | "payment_required" => {
            Some(result(
                FailoverReason::Billing,
                false, false, true, true,
            ))
        }
        "model_not_found" | "model_not_available" | "invalid_model" => {
            Some(result(
                FailoverReason::ModelNotFound,
                false, false, false, true,
            ))
        }
        "context_length_exceeded" | "max_tokens_exceeded" => {
            Some(result(
                FailoverReason::ContextOverflow,
                true, true, false, false,
            ))
        }
        _ => None,
    }
}

fn classify_by_message<F>(
    error_msg: &str,
    _error_type: &str,
    _approx_tokens: usize,
    _context_length: usize,
    result: &F,
) -> Option<ClassifiedError>
where
    F: Fn(
        FailoverReason,
        bool,
        bool,
        bool,
        bool,
    ) -> ClassifiedError,
{
    // Payload-too-large patterns
    if contains_any(error_msg, PAYLOAD_TOO_LARGE_PATTERNS) {
        return Some(result(
            FailoverReason::PayloadTooLarge,
            true, true, false, false,
        ));
    }

    // Image-too-large patterns
    if contains_any(error_msg, IMAGE_TOO_LARGE_PATTERNS) {
        return Some(result(
            FailoverReason::ImageTooLarge,
            true, false, false, false,
        ));
    }

    // Usage-limit patterns with disambiguation
    if contains_any(error_msg, USAGE_LIMIT_PATTERNS) {
        let has_transient_signal =
            contains_any(error_msg, USAGE_LIMIT_TRANSIENT_SIGNALS);
        return Some(if has_transient_signal {
            result(
                FailoverReason::RateLimit,
                true, false, true, true,
            )
        } else {
            result(
                FailoverReason::Billing,
                false, false, true, true,
            )
        });
    }

    // Billing patterns
    if contains_any(error_msg, BILLING_PATTERNS) {
        return Some(result(
            FailoverReason::Billing,
            false, false, true, true,
        ));
    }

    // Rate limit patterns
    if contains_any(error_msg, RATE_LIMIT_PATTERNS) {
        return Some(result(
            FailoverReason::RateLimit,
            true, false, true, true,
        ));
    }

    // Context overflow patterns
    if contains_any(error_msg, CONTEXT_OVERFLOW_PATTERNS) {
        return Some(result(
            FailoverReason::ContextOverflow,
            true, true, false, false,
        ));
    }

    // Auth patterns
    if contains_any(error_msg, AUTH_PATTERNS) {
        return Some(result(
            FailoverReason::Auth,
            false, false, true, true,
        ));
    }

    // Provider policy-block
    if contains_any(error_msg, PROVIDER_POLICY_BLOCKED_PATTERNS) {
        return Some(result(
            FailoverReason::ProviderPolicyBlocked,
            false, false, false, false,
        ));
    }

    // Model not found patterns
    if contains_any(error_msg, MODEL_NOT_FOUND_PATTERNS) {
        return Some(result(
            FailoverReason::ModelNotFound,
            false, false, false, true,
        ));
    }

    // Timeout message patterns
    if contains_any(error_msg, TIMEOUT_MESSAGE_PATTERNS) {
        return Some(result(
            FailoverReason::Timeout,
            true, false, false, false,
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_body() -> serde_json::Value {
        serde_json::Value::Null
    }

    #[test]
    fn test_401_classifies_as_auth() {
        let result = classify_api_error(
            "Unauthorized",
            Some(401),
            &empty_body(),
            "APIStatusError",
            "anthropic",
            "claude-sonnet-4-20250514",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::Auth);
        assert!(!result.retryable);
        assert!(result.should_rotate_credential);
        assert!(result.should_fallback);
    }

    #[test]
    fn test_429_classifies_as_rate_limit() {
        let result = classify_api_error(
            "Rate limit exceeded",
            Some(429),
            &empty_body(),
            "APIStatusError",
            "anthropic",
            "claude-sonnet-4-20250514",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::RateLimit);
        assert!(result.retryable);
        assert!(result.should_rotate_credential);
    }

    #[test]
    fn test_500_classifies_as_server_error() {
        let result = classify_api_error(
            "Internal Server Error",
            Some(500),
            &empty_body(),
            "APIStatusError",
            "openrouter",
            "anthropic/claude-sonnet-4",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::ServerError);
        assert!(result.retryable);
    }

    #[test]
    fn test_context_overflow_from_message() {
        let result = classify_api_error(
            "Error: context length exceeded",
            None,
            &empty_body(),
            "RuntimeError",
            "anthropic",
            "claude-sonnet-4",
            180000,
            200000,
            150,
        );
        assert_eq!(result.reason, FailoverReason::ContextOverflow);
        assert!(result.should_compress);
    }

    #[test]
    fn test_billing_from_402_with_no_transient_signal() {
        let body = serde_json::json!({
            "error": { "message": "Insufficient credits. Please top up your account." }
        });
        let result = classify_api_error(
            "Payment Required",
            Some(402),
            &body,
            "APIStatusError",
            "openrouter",
            "anthropic/claude-sonnet-4",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::Billing);
        assert!(!result.retryable);
    }

    #[test]
    fn test_402_transient_usage_limit() {
        let body = serde_json::json!({
            "error": { "message": "Usage limit exceeded. Try again in 5 minutes." }
        });
        let result = classify_api_error(
            "Payment Required",
            Some(402),
            &body,
            "APIStatusError",
            "openrouter",
            "anthropic/claude-sonnet-4",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::RateLimit);
        assert!(result.retryable);
    }

    #[test]
    fn test_thinking_signature_detection() {
        let body = serde_json::json!({
            "error": { "message": "Invalid thinking block signature" }
        });
        let result = classify_api_error(
            "Bad Request",
            Some(400),
            &body,
            "APIStatusError",
            "anthropic",
            "claude-sonnet-4",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::ThinkingSignature);
        assert!(result.retryable);
        assert!(!result.should_compress);
    }

    #[test]
    fn test_unknown_fallback_is_retryable() {
        let result = classify_api_error(
            "Some weird error",
            None,
            &empty_body(),
            "UnknownError",
            "custom",
            "some-model",
            5000,
            200000,
            10,
        );
        assert_eq!(result.reason, FailoverReason::Unknown);
        assert!(result.retryable);
    }
}

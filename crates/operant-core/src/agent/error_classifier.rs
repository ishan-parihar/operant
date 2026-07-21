//! API error classification for smart failover and recovery.
//!
//! Ported from `hermes-agent/agent/error_classifier.py`. Provides a
//! structured taxonomy of API errors and a priority-ordered classification
//! pipeline that determines the correct recovery action (retry, rotate
//! credential, fallback to another provider, compress context, or abort).

use std::fmt;

// ── Error taxonomy ──────────────────────────────────────────────────────

/// Why an API call failed — determines recovery strategy.
///
/// Ported from hermes-agent's `FailoverReason` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailoverReason {
    // Authentication / authorization
    /// Transient auth (401/403) — refresh/rotate
    Auth,
    /// Auth failed after refresh — abort
    AuthPermanent,

    // Billing / quota
    /// 402 or confirmed credit exhaustion — rotate immediately
    Billing,
    /// 429 or quota-based throttling — backoff then rotate
    RateLimit,
    /// Upstream model rate-limited (aggregator 429) — fallback to different model
    UpstreamRateLimit,

    // Server-side
    /// 503/529 — provider overloaded, backoff
    Overloaded,
    /// 500/502 — internal server error, retry
    ServerError,

    // Transport
    /// Connection/read timeout — rebuild client + retry
    Timeout,
    /// TLS certificate verification failure — fail fast
    SslCertVerification,

    // Context / payload
    /// Context too large — compress, not failover
    ContextOverflow,
    /// 413 — compress payload
    PayloadTooLarge,
    /// Native image part exceeds provider's per-image limit
    ImageTooLarge,

    // Model / provider policy
    /// 404 or invalid model — fallback to different model
    ModelNotFound,
    /// Aggregator blocked the only endpoint due to account data/privacy policy
    ProviderPolicyBlocked,
    /// Provider safety filter rejected this prompt — deterministic, don't retry unchanged
    ContentPolicyBlocked,

    // Request format
    /// 400 bad request — abort or strip + retry
    FormatError,
    /// Responses replay blob rejected — strip replay state and retry
    InvalidEncryptedContent,
    /// Provider rejected list-type content in tool messages — downgrade to text
    MultimodalToolContentUnsupported,

    // Provider-specific
    /// Anthropic thinking block sig invalid
    ThinkingSignature,
    /// Anthropic "extra usage" tier gate
    LongContextTier,
    /// Anthropic OAuth subscription rejects 1M context beta
    OauthLongContextBetaForbidden,
    /// llama.cpp json-schema-to-grammar rejects regex escapes
    LlamaCppGrammarPattern,

    // Catch-all
    /// Unclassifiable — retry with backoff
    Unknown,
}

impl fmt::Display for FailoverReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FailoverReason {
    /// Human-readable label for logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::AuthPermanent => "auth_permanent",
            Self::Billing => "billing",
            Self::RateLimit => "rate_limit",
            Self::UpstreamRateLimit => "upstream_rate_limit",
            Self::Overloaded => "overloaded",
            Self::ServerError => "server_error",
            Self::Timeout => "timeout",
            Self::SslCertVerification => "ssl_cert_verification",
            Self::ContextOverflow => "context_overflow",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ImageTooLarge => "image_too_large",
            Self::ModelNotFound => "model_not_found",
            Self::ProviderPolicyBlocked => "provider_policy_blocked",
            Self::ContentPolicyBlocked => "content_policy_blocked",
            Self::FormatError => "format_error",
            Self::InvalidEncryptedContent => "invalid_encrypted_content",
            Self::MultimodalToolContentUnsupported => "multimodal_tool_content_unsupported",
            Self::ThinkingSignature => "thinking_signature",
            Self::LongContextTier => "long_context_tier",
            Self::OauthLongContextBetaForbidden => "oauth_long_context_beta_forbidden",
            Self::LlamaCppGrammarPattern => "llama_cpp_grammar_pattern",
            Self::Unknown => "unknown",
        }
    }
}

// ── Classification result ───────────────────────────────────────────────

/// Structured classification of an API error with recovery hints.
#[derive(Debug, Clone)]
pub struct ClassifiedError {
    pub reason: FailoverReason,
    pub status_code: Option<u16>,
    pub message: String,

    // Recovery action hints
    /// Whether to retry the same model (with backoff)
    pub retryable: bool,
    /// Whether to try a different model/provider
    pub should_fallback: bool,
    /// Whether to trigger context compression
    pub should_compress: bool,
    /// Whether to rotate credentials (credential pool)
    pub should_rotate_credential: bool,
}

impl ClassifiedError {
    /// Whether this is an auth-related failure.
    pub fn is_auth(&self) -> bool {
        matches!(self.reason, FailoverReason::Auth | FailoverReason::AuthPermanent)
    }
}

// ── Pattern-matching constants ──────────────────────────────────────────

/// Patterns that indicate billing exhaustion (not transient rate limit).
const BILLING_PATTERNS: &[&str] = &[
    "insufficient credits",
    "insufficient_quota",
    "insufficient balance",
    "credit balance",
    "credits exhausted",
    "credits have been exhausted",
    "no usable credits",
    "top up your credits",
    "payment required",
    "billing hard limit",
    "exceeded your current quota",
    "account is deactivated",
    "plan does not include",
    "out of extra usage",
    "out of funds",
    "run out of funds",
    "balance_depleted",
    "model_not_supported_on_free_tier",
    "not available on the free tier",
];

/// Structured error codes that mean billing exhaustion.
const BILLING_ERROR_CODES: &[&str] = &[
    "insufficient_quota",
    "billing_not_active",
    "payment_required",
    "insufficient_credits",
    "no_usable_credits",
    "balance_depleted",
    "model_not_supported_on_free_tier",
];

/// Patterns that indicate rate limiting (transient, will resolve).
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
];

/// Patterns indicating provider-side overload (not per-credential rate limit).
const OVERLOADED_PATTERNS: &[&str] = &[
    "overloaded",
    "temporarily overloaded",
    "service is temporarily overloaded",
    "server is overloaded",
    "server overloaded",
    "service overloaded",
    "at capacity",
    "over capacity",
];

/// Context overflow patterns.
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
];

/// Model not found patterns.
const MODEL_NOT_FOUND_PATTERNS: &[&str] = &[
    "is not a valid model",
    "invalid model",
    "model not found",
    "model_not_found",
    "does not exist",
    "no such model",
    "unknown model",
    "unsupported model",
    "no endpoints found that support tool use",
];

/// Auth patterns (non-status-code signals).
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

/// Content policy / safety filter blocks.
const CONTENT_POLICY_PATTERNS: &[&str] = &[
    "flagged for possible cybersecurity risk",
    "trusted access for cyber",
    "violates our usage policies",
    "violates openai's usage policies",
    "your request was flagged by",
    "prompt was flagged by our safety",
    "responses cannot be generated due to safety",
    "content_filter",
    "responsibleaipolicyviolation",
    "new_sensitive",
];

/// Request-validation patterns (deterministic, don't retry).
const REQUEST_VALIDATION_PATTERNS: &[&str] = &[
    "unknown parameter",
    "unsupported parameter",
    "unrecognized request argument",
    "invalid_request_error",
    "unknown_parameter",
    "unsupported_parameter",
];

/// Provider policy blocked patterns (aggregator-side guardrail).
const PROVIDER_POLICY_PATTERNS: &[&str] = &[
    "no endpoints available matching your guardrail",
    "no endpoints available matching your data policy",
    "no endpoints found matching your data policy",
];

/// Image too large patterns.
const IMAGE_TOO_LARGE_PATTERNS: &[&str] = &[
    "image exceeds",
    "image too large",
    "image_too_large",
    "image size exceeds",
    "image dimensions exceed",
    "dimensions exceed max allowed size",
    "max allowed size: 8000",
];

/// Multimodal tool content unsupported patterns.
const MULTIMODAL_TOOL_PATTERNS: &[&str] = &[
    "text is not set",
    "tool message content must be a string",
    "tool content must be a string",
    "tool message must be a string",
    "expected string, got list",
    "expected string, got array",
    "tool_call.content must be string",
];

/// Timeout message patterns.
const TIMEOUT_PATTERNS: &[&str] = &[
    "timed out",
    "turn timed out",
    "request timed out",
    "deadline exceeded",
    "operation timed out",
    "upstream timed out",
];

/// SSL certificate verification failure patterns (deterministic — fail fast).
const SSL_CERT_VERIFY_PATTERNS: &[&str] = &[
    "certificate verify failed",
    "certificate_verify_failed",
    "unable to get local issuer certificate",
    "self-signed certificate",
    "self signed certificate",
    "certificate has expired",
    "hostname mismatch, certificate is not valid",
    "unable to verify the first certificate",
];

/// Empty provider response patterns.
const EMPTY_RESPONSE_PATTERNS: &[&str] = &[
    "returned an empty response",
    "empty response despite retries",
    "provider returned an empty response",
    "model returning empty responses",
    "empty response stream",
];

// ── Classification pipeline ─────────────────────────────────────────────

/// Classify an API error into a structured recovery recommendation.
///
/// Priority-ordered pipeline:
///   1. Provider-specific patterns (thinking sigs, tier gates)
///   2. HTTP status code + message-aware refinement
///   3. Error code classification
///   4. Message pattern matching
///   5. Transport error heuristics
///   6. Fallback: unknown (retryable with backoff)
pub fn classify_api_error(
    status_code: Option<u16>,
    error_body: &str,
    error_code: Option<&str>,
) -> ClassifiedError {
    let body_lower = error_body.to_lowercase();

    // ── 1. Provider-specific patterns (highest priority) ────────────

    // Content policy / safety filter block
    if contains_any(&body_lower, CONTENT_POLICY_PATTERNS) {
        return ClassifiedError {
            reason: FailoverReason::ContentPolicyBlocked,
            status_code,
            message: error_body.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // Anthropic thinking block signature recovery
    if status_code == Some(400)
        && body_lower.contains("thinking")
        && (body_lower.contains("signature")
            || body_lower.contains("cannot be modified")
            || body_lower.contains("must remain as they were"))
    {
        return ClassifiedError {
            reason: FailoverReason::ThinkingSignature,
            status_code,
            message: error_body.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // Anthropic long-context tier gate
    if status_code == Some(429)
        && body_lower.contains("extra usage")
        && body_lower.contains("long context")
    {
        return ClassifiedError {
            reason: FailoverReason::LongContextTier,
            status_code,
            message: error_body.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        };
    }

    // Anthropic OAuth subscription rejects 1M context beta
    if status_code == Some(400)
        && body_lower.contains("long context beta")
        && body_lower.contains("not yet available")
    {
        return ClassifiedError {
            reason: FailoverReason::OauthLongContextBetaForbidden,
            status_code,
            message: error_body.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // llama.cpp grammar pattern rejection
    if status_code == Some(400)
        && (body_lower.contains("error parsing grammar")
            || body_lower.contains("json-schema-to-grammar")
            || (body_lower.contains("unable to generate parser")
                && body_lower.contains("template")))
    {
        return ClassifiedError {
            reason: FailoverReason::LlamaCppGrammarPattern,
            status_code,
            message: error_body.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // xAI Grok subscription entitlement errors
    if body_lower.contains("do not have an active grok subscription")
        || (body_lower.contains("out of available resources")
            && body_lower.contains("grok"))
    {
        return ClassifiedError {
            reason: FailoverReason::Auth,
            status_code,
            message: error_body.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // ── 1b. SSL certificate verification failures (deterministic) ──
    // A broken certificate chain fails identically on every retry.
    // Must run BEFORE status-based classification so a 400 SSL error
    // isn't downgraded to a generic format_error.
    if contains_any(&body_lower, SSL_CERT_VERIFY_PATTERNS) {
        return ClassifiedError {
            reason: FailoverReason::SslCertVerification,
            status_code,
            message: error_body.to_string(),
            retryable: false,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        };
    }

    // ── 2. HTTP status code classification ──────────────────────────

    if let Some(status) = status_code {
        let classified = classify_by_status(status, &body_lower, error_code);
        if let Some(c) = classified {
            return c;
        }
    }

    // ── 3. Error code classification ────────────────────────────────

    if let Some(code) = error_code {
        let classified = classify_by_error_code(code, &body_lower);
        if let Some(c) = classified {
            return c;
        }
    }

    // ── 4. Message pattern matching (no status code) ────────────────

    let classified = classify_by_message(&body_lower);
    if let Some(c) = classified {
        return c;
    }

    // ── 5. Fallback: unknown ────────────────────────────────────────

    ClassifiedError {
        reason: FailoverReason::Unknown,
        status_code,
        message: error_body.to_string(),
        retryable: true,
        should_fallback: false,
        should_compress: false,
        should_rotate_credential: false,
    }
}

// ── Status code classification ──────────────────────────────────────────

fn classify_by_status(
    status: u16,
    body_lower: &str,
    error_code: Option<&str>,
) -> Option<ClassifiedError> {
    match status {
        401 => Some(ClassifiedError {
            reason: FailoverReason::Auth,
            status_code: Some(status),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        }),

        403 => {
            // Check for billing exhaustion disguised as 403
            if contains_any(body_lower, BILLING_PATTERNS)
                || body_lower.contains("key limit exceeded")
                || body_lower.contains("spending limit")
            {
                return Some(ClassifiedError {
                    reason: FailoverReason::Billing,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                });
            }
            Some(ClassifiedError {
                reason: FailoverReason::Auth,
                status_code: Some(status),
                message: body_lower.to_string(),
                retryable: false,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: false,
            })
        }

        402 => {
            // Disambiguate: billing exhaustion vs transient usage limit
            let has_usage_limit = body_lower.contains("usage limit")
                || body_lower.contains("quota")
                || body_lower.contains("limit exceeded");
            let has_transient = body_lower.contains("try again")
                || body_lower.contains("retry")
                || body_lower.contains("resets at")
                || body_lower.contains("reset in")
                || body_lower.contains("wait");

            if has_usage_limit && has_transient {
                Some(ClassifiedError {
                    reason: FailoverReason::RateLimit,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                })
            } else {
                Some(ClassifiedError {
                    reason: FailoverReason::Billing,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                })
            }
        }

        404 => {
            if contains_any(body_lower, BILLING_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::Billing,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                });
            }
            if contains_any(body_lower, PROVIDER_POLICY_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ProviderPolicyBlocked,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: false,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            if contains_any(body_lower, MODEL_NOT_FOUND_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ModelNotFound,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            // Generic 404 — unknown, retryable
            Some(ClassifiedError {
                reason: FailoverReason::Unknown,
                status_code: Some(status),
                message: body_lower.to_string(),
                retryable: true,
                should_fallback: false,
                should_compress: false,
                should_rotate_credential: false,
            })
        }

        413 => Some(ClassifiedError {
            reason: FailoverReason::PayloadTooLarge,
            status_code: Some(status),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        }),

        429 => {
            if contains_any(body_lower, OVERLOADED_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::Overloaded,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: false,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            Some(ClassifiedError {
                reason: FailoverReason::RateLimit,
                status_code: Some(status),
                message: body_lower.to_string(),
                retryable: true,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: true,
            })
        }

        400 => classify_400(body_lower, error_code),

        500 | 502 => {
            if contains_any(body_lower, REQUEST_VALIDATION_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::FormatError,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            if contains_any(body_lower, EMPTY_RESPONSE_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ServerError,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: false,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            if contains_any(body_lower, CONTEXT_OVERFLOW_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ContextOverflow,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: false,
                    should_compress: true,
                    should_rotate_credential: false,
                });
            }
            Some(ClassifiedError {
                reason: FailoverReason::ServerError,
                status_code: Some(status),
                message: body_lower.to_string(),
                retryable: true,
                should_fallback: false,
                should_compress: false,
                should_rotate_credential: false,
            })
        }

        503 | 529 => {
            if contains_any(body_lower, EMPTY_RESPONSE_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ServerError,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: false,
                    should_compress: false,
                    should_rotate_credential: false,
                });
            }
            if contains_any(body_lower, CONTEXT_OVERFLOW_PATTERNS) {
                return Some(ClassifiedError {
                    reason: FailoverReason::ContextOverflow,
                    status_code: Some(status),
                    message: body_lower.to_string(),
                    retryable: true,
                    should_fallback: false,
                    should_compress: true,
                    should_rotate_credential: false,
                });
            }
            Some(ClassifiedError {
                reason: FailoverReason::Overloaded,
                status_code: Some(status),
                message: body_lower.to_string(),
                retryable: true,
                should_fallback: false,
                should_compress: false,
                should_rotate_credential: false,
            })
        }

        408 => Some(ClassifiedError {
            reason: FailoverReason::Timeout,
            status_code: Some(status),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        }),

        s if s >= 400 && s < 500 => Some(ClassifiedError {
            reason: FailoverReason::FormatError,
            status_code: Some(status),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        }),

        s if s >= 500 => Some(ClassifiedError {
            reason: FailoverReason::ServerError,
            status_code: Some(status),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        }),

        _ => None,
    }
}

// ── 400 classification ──────────────────────────────────────────────────

fn classify_400(body_lower: &str, error_code: Option<&str>) -> Option<ClassifiedError> {
    // Multimodal tool content rejected
    if contains_any(body_lower, MULTIMODAL_TOOL_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::MultimodalToolContentUnsupported,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Image too large
    if contains_any(body_lower, IMAGE_TOO_LARGE_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ImageTooLarge,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Invalid encrypted reasoning replay
    let code_lower = error_code.unwrap_or("").to_lowercase();
    if code_lower == "invalid_encrypted_content"
        || body_lower.contains("invalid_encrypted_content")
        || (body_lower.contains("encrypted content for item")
            && body_lower.contains("could not be verified"))
    {
        return Some(ClassifiedError {
            reason: FailoverReason::InvalidEncryptedContent,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Request-validation errors (unsupported/unknown parameter)
    if contains_any(body_lower, REQUEST_VALIDATION_PATTERNS)
        || code_lower == "unknown_parameter"
        || code_lower == "unsupported_parameter"
    {
        return Some(ClassifiedError {
            reason: FailoverReason::FormatError,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Empty provider response advisories (must NOT enter compression)
    if contains_any(body_lower, EMPTY_RESPONSE_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ServerError,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Context overflow from 400
    if contains_any(body_lower, CONTEXT_OVERFLOW_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ContextOverflow,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        });
    }

    // Provider policy blocked
    if contains_any(body_lower, PROVIDER_POLICY_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ProviderPolicyBlocked,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Model not found
    if contains_any(body_lower, MODEL_NOT_FOUND_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ModelNotFound,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Rate limit / billing disguised as 400
    if contains_any(body_lower, RATE_LIMIT_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::RateLimit,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }
    if contains_any(body_lower, BILLING_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::Billing,
            status_code: Some(400),
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    // Non-retryable format error
    Some(ClassifiedError {
        reason: FailoverReason::FormatError,
        status_code: Some(400),
        message: body_lower.to_string(),
        retryable: false,
        should_fallback: true,
        should_compress: false,
        should_rotate_credential: false,
    })
}

// ── Error code classification ───────────────────────────────────────────

fn classify_by_error_code(code: &str, body_lower: &str) -> Option<ClassifiedError> {
    let code_lower = code.to_lowercase();

    if matches!(
        code_lower.as_str(),
        "resource_exhausted" | "throttled" | "rate_limit_exceeded"
    ) {
        return Some(ClassifiedError {
            reason: FailoverReason::RateLimit,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    if BILLING_ERROR_CODES.contains(&code_lower.as_str()) {
        return Some(ClassifiedError {
            reason: FailoverReason::Billing,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    if matches!(
        code_lower.as_str(),
        "model_not_found" | "model_not_available" | "invalid_model"
    ) {
        return Some(ClassifiedError {
            reason: FailoverReason::ModelNotFound,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    if matches!(
        code_lower.as_str(),
        "context_length_exceeded" | "max_tokens_exceeded"
    ) {
        return Some(ClassifiedError {
            reason: FailoverReason::ContextOverflow,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        });
    }

    if code_lower == "invalid_encrypted_content" {
        return Some(ClassifiedError {
            reason: FailoverReason::InvalidEncryptedContent,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    None
}

// ── Message pattern classification ──────────────────────────────────────

fn classify_by_message(body_lower: &str) -> Option<ClassifiedError> {
    // Payload too large
    if body_lower.contains("request entity too large")
        || body_lower.contains("payload too large")
        || body_lower.contains("error code: 413")
    {
        return Some(ClassifiedError {
            reason: FailoverReason::PayloadTooLarge,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        });
    }

    // Multimodal tool content
    if contains_any(body_lower, MULTIMODAL_TOOL_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::MultimodalToolContentUnsupported,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Image too large
    if contains_any(body_lower, IMAGE_TOO_LARGE_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ImageTooLarge,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Usage limit disambiguation
    let has_usage_limit = body_lower.contains("usage limit")
        || body_lower.contains("quota")
        || body_lower.contains("limit exceeded");
    if has_usage_limit {
        let has_transient = body_lower.contains("try again")
            || body_lower.contains("retry")
            || body_lower.contains("resets at")
            || body_lower.contains("reset in")
            || body_lower.contains("wait");
        if has_transient {
            return Some(ClassifiedError {
                reason: FailoverReason::RateLimit,
                status_code: None,
                message: body_lower.to_string(),
                retryable: true,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: true,
            });
        }
        return Some(ClassifiedError {
            reason: FailoverReason::Billing,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    // Overloaded
    if contains_any(body_lower, OVERLOADED_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::Overloaded,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Billing
    if contains_any(body_lower, BILLING_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::Billing,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    // Rate limit
    if contains_any(body_lower, RATE_LIMIT_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::RateLimit,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    // Empty response (must NOT compress)
    if contains_any(body_lower, EMPTY_RESPONSE_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ServerError,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Context overflow
    if contains_any(body_lower, CONTEXT_OVERFLOW_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ContextOverflow,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: true,
            should_rotate_credential: false,
        });
    }

    // Auth
    if contains_any(body_lower, AUTH_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::Auth,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: true,
        });
    }

    // Provider policy blocked
    if contains_any(body_lower, PROVIDER_POLICY_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ProviderPolicyBlocked,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Model not found
    if contains_any(body_lower, MODEL_NOT_FOUND_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::ModelNotFound,
            status_code: None,
            message: body_lower.to_string(),
            retryable: false,
            should_fallback: true,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    // Timeout
    if contains_any(body_lower, TIMEOUT_PATTERNS) {
        return Some(ClassifiedError {
            reason: FailoverReason::Timeout,
            status_code: None,
            message: body_lower.to_string(),
            retryable: true,
            should_fallback: false,
            should_compress: false,
            should_rotate_credential: false,
        });
    }

    None
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Check if the haystack contains any of the needle patterns.
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_overflow_400() {
        let c = classify_api_error(Some(400), "context length exceeded, reduce the length", None);
        assert_eq!(c.reason, FailoverReason::ContextOverflow);
        assert!(c.should_compress);
        assert!(c.retryable);
    }

    #[test]
    fn test_rate_limit_429() {
        let c = classify_api_error(Some(429), "rate limit exceeded, try again in 5s", None);
        assert_eq!(c.reason, FailoverReason::RateLimit);
        assert!(c.retryable);
        assert!(c.should_rotate_credential);
    }

    #[test]
    fn test_overloaded_429() {
        let c = classify_api_error(Some(429), "service is temporarily overloaded", None);
        assert_eq!(c.reason, FailoverReason::Overloaded);
        assert!(c.retryable);
        assert!(!c.should_rotate_credential);
    }

    #[test]
    fn test_billing_402() {
        let c = classify_api_error(Some(402), "insufficient credits", None);
        assert_eq!(c.reason, FailoverReason::Billing);
        assert!(!c.retryable);
        assert!(c.should_rotate_credential);
    }

    #[test]
    fn test_transient_usage_limit_402() {
        let c = classify_api_error(Some(402), "usage limit, try again in 5 minutes", None);
        assert_eq!(c.reason, FailoverReason::RateLimit);
        assert!(c.retryable);
    }

    #[test]
    fn test_auth_401() {
        let c = classify_api_error(Some(401), "invalid api key", None);
        assert_eq!(c.reason, FailoverReason::Auth);
        assert!(!c.retryable);
        assert!(c.should_rotate_credential);
    }

    #[test]
    fn test_content_policy_400() {
        let c = classify_api_error(Some(400), "violates our usage policies", None);
        assert_eq!(c.reason, FailoverReason::ContentPolicyBlocked);
        assert!(!c.retryable);
        assert!(c.should_fallback);
    }

    #[test]
    fn test_model_not_found_404() {
        let c = classify_api_error(Some(404), "model not found", None);
        assert_eq!(c.reason, FailoverReason::ModelNotFound);
        assert!(!c.retryable);
        assert!(c.should_fallback);
    }

    #[test]
    fn test_thinking_signature_400() {
        let c = classify_api_error(Some(400), "thinking block signature is invalid", None);
        assert_eq!(c.reason, FailoverReason::ThinkingSignature);
        assert!(c.retryable);
    }

    #[test]
    fn test_server_error_500() {
        let c = classify_api_error(Some(500), "internal server error", None);
        assert_eq!(c.reason, FailoverReason::ServerError);
        assert!(c.retryable);
    }

    #[test]
    fn test_overloaded_503() {
        let c = classify_api_error(Some(503), "service overloaded", None);
        assert_eq!(c.reason, FailoverReason::Overloaded);
        assert!(c.retryable);
    }

    #[test]
    fn test_format_error_400_unknown_param() {
        let c = classify_api_error(Some(400), "unknown parameter 'foo'", None);
        assert_eq!(c.reason, FailoverReason::FormatError);
        assert!(!c.retryable);
    }

    #[test]
    fn test_llama_cpp_grammar_400() {
        let c = classify_api_error(Some(400), "error parsing grammar for tool schema", None);
        assert_eq!(c.reason, FailoverReason::LlamaCppGrammarPattern);
        assert!(c.retryable);
    }

    #[test]
    fn test_message_only_billing() {
        let c = classify_api_error(None, "credits exhausted, please top up", None);
        assert_eq!(c.reason, FailoverReason::Billing);
        assert!(!c.retryable);
    }

    #[test]
    fn test_message_only_rate_limit() {
        let c = classify_api_error(None, "too many requests, try again later", None);
        assert_eq!(c.reason, FailoverReason::RateLimit);
        assert!(c.retryable);
    }

    #[test]
    fn test_unknown_fallback() {
        let c = classify_api_error(None, "something weird happened", None);
        assert_eq!(c.reason, FailoverReason::Unknown);
        assert!(c.retryable);
    }

    #[test]
    fn test_ssl_cert_verification() {
        let c = classify_api_error(Some(400), "certificate verify failed: unable to get local issuer certificate", None);
        assert_eq!(c.reason, FailoverReason::SslCertVerification);
        assert!(!c.retryable);
        assert!(!c.should_fallback);
    }

    #[test]
    fn test_error_code_context_overflow_from_status() {
        let c = classify_api_error(None, "error", Some("context_length_exceeded"));
        assert_eq!(c.reason, FailoverReason::ContextOverflow);
        assert!(c.should_compress);
    }

    #[test]
    fn test_error_code_rate_limit() {
        let c = classify_api_error(None, "quota exceeded", Some("resource_exhausted"));
        assert_eq!(c.reason, FailoverReason::RateLimit);
    }

    #[test]
    fn test_error_code_billing() {
        let c = classify_api_error(None, "error", Some("insufficient_quota"));
        assert_eq!(c.reason, FailoverReason::Billing);
    }

    #[test]
    fn test_error_code_model_not_found() {
        let c = classify_api_error(None, "error", Some("model_not_found"));
        assert_eq!(c.reason, FailoverReason::ModelNotFound);
    }

    #[test]
    fn test_error_code_context_overflow() {
        let c = classify_api_error(None, "error", Some("context_length_exceeded"));
        assert_eq!(c.reason, FailoverReason::ContextOverflow);
        assert!(c.should_compress);
    }
}

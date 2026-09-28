//! Provider OAuth 2.0: authorization-code + PKCE, device-code, and
//! single-flight refresh coordination.
//!
//! This module owns the *provider-account* OAuth surface (Anthropic, Codex,
//! xAI, Nous — the credentials `oauth_refresh` refreshes). The MCP-server
//! surface in [`crate::mcp_oauth`] is a separate code path and is not
//! re-exported here.
//!
//! ## Module map
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`pkce`] | RFC 7636 `code_verifier`/`code_challenge` + `state` minting |
//! | [`loopback`] | ephemeral-port redirect listener, bounded read, `state` verification |
//! | [`http`] | the injectable HTTP boundary ([`HttpEndpoint`]) |
//! | [`device`] | RFC 8628 device authorization + polling |
//! | [`singleflight`] | per-account refresh coalescing |
//! | [`store`] | [`store::AccountStore`] — keys and ordering, never raw tokens |
//!
//! ## Why the HTTP boundary is a trait
//!
//! Every flow here is testable without a live provider because the network
//! call is behind [`HttpEndpoint`]. The production implementation is
//! [`ReqwestEndpoint`]; tests supply a stub. This is a design constraint, not
//! a test convenience: a flow whose correctness lives in *timing* (device-code
//! polling honours the server's `interval`/`expires_in`) and in *state
//! comparison* (the redirect's `state`) cannot be exercised at all if the
//! transport is baked in.
//!
//! ## Why refreshes are coalesced
//!
//! Providers such as Anthropic, Codex and Nous issue **single-use rotating
//! refresh tokens**: spending one invalidates it and yields a replacement. If
//! N concurrent requests each discover the same account is expiring and each
//! spends the token, the first succeeds and the other N−1 are rejected as
//! `invalid_grant` / `refresh_token_reused` — and one of those rejections
//! usually poisons the stored credential, taking the account down.
//! [`singleflight::RefreshCoordinator`] collapses those N attempts into one
//! per account; see that module for the design.

pub mod device;
pub mod http;
pub mod loopback;
pub mod pkce;
pub mod singleflight;
pub mod store;

use serde::Deserialize;

pub use http::{HttpEndpoint, HttpMethod, HttpRequest, HttpResponse, ReqwestEndpoint};
pub use loopback::{Callback, Loopback, MAX_CALLBACK_BYTES};
pub use pkce::{Pkce, derive_challenge, generate_pkce, generate_state};
pub use singleflight::{CoalescedFailure, RefreshCoordinator};

/// Result alias for the OAuth flows in this module.
pub type Result<T> = std::result::Result<T, OAuthFlowError>;

/// Typed failure for an interactive or headless OAuth flow.
///
/// Every variant is a distinct, matchable condition — a caller that needs to
/// react (re-prompt, fall back to device code, tell the user to re-auth)
/// matches on it instead of parsing a message.
#[derive(Debug, thiserror::Error)]
pub enum OAuthFlowError {
    /// The caller supplied an unusable endpoint / client id / verifier.
    #[error("OAuth configuration error: {0}")]
    Config(String),

    /// The transport failed before a status line arrived.
    #[error("OAuth network error: {0}")]
    Network(String),

    /// The server answered with a non-2xx status.
    #[error("OAuth endpoint {url} returned HTTP {status}: {body}")]
    Http {
        /// Endpoint that produced the status.
        url: String,
        /// HTTP status code.
        status: u16,
        /// Truncated response body.
        body: String,
    },

    /// A 2xx body that was not the JSON we require.
    #[error("OAuth response could not be parsed: {0}")]
    InvalidResponse(String),

    /// The redirect's `state` did not match the one we issued, or was absent.
    ///
    /// This is the CSRF guard: a mismatch means the callback was not produced
    /// by our authorization request, so the code in it is untrusted.
    #[error("OAuth state mismatch — rejecting callback (possible CSRF)")]
    StateMismatch,

    /// No callback arrived within the deadline.
    #[error("OAuth callback timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// The device code or authorization code expired before use.
    #[error("OAuth authorization expired: {0}")]
    Expired(String),

    /// The user (or the server) denied the request.
    #[error("OAuth authorization denied: {0}")]
    Denied(String),

    /// The user has not finished authorizing yet — keep polling.
    #[error("OAuth authorization still pending")]
    Pending,

    /// The server asked us to poll less often (`slow_down`).
    #[error("OAuth server asked to slow down")]
    SlowDown,

    /// A redirect or callback exceeded its size budget.
    #[error("OAuth callback exceeded the {limit}-byte limit")]
    TooLarge {
        /// The configured cap.
        limit: usize,
    },

    /// Local socket / filesystem failure.
    #[error("OAuth local I/O error: {0}")]
    Io(String),
}

impl From<std::io::Error> for OAuthFlowError {
    fn from(e: std::io::Error) -> Self {
        OAuthFlowError::Io(e.to_string())
    }
}

impl From<OAuthFlowError> for crate::Error {
    fn from(e: OAuthFlowError) -> Self {
        crate::Error::Authentication(e.to_string())
    }
}

/// Truncate an error body so a hostile or verbose server cannot flood logs.
const MAX_ERROR_BODY: usize = 512;

fn clamp_body(body: &str) -> String {
    if body.len() <= MAX_ERROR_BODY {
        return body.to_string();
    }
    let mut end = MAX_ERROR_BODY;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &body[..end])
}

/// The JSON every OAuth token endpoint returns (RFC 6749 §5.1). Device-code
/// responses reuse the same shape, and so do error bodies — so `access_token`
/// and `error` are both optional and the error branch is checked first.
#[derive(Debug, Deserialize)]
struct TokenPayload {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Map an OAuth `error` code to its typed variant, or `None` when the code has
/// no special meaning (an unknown code is reported as a plain parse failure).
fn error_variant(code: &str, detail: &str) -> Option<OAuthFlowError> {
    match code {
        "authorization_pending" => Some(OAuthFlowError::Pending),
        "slow_down" => Some(OAuthFlowError::SlowDown),
        "expired_token" => Some(OAuthFlowError::Expired(code.to_string())),
        "access_denied" => Some(OAuthFlowError::Denied(detail.to_string())),
        _ => None,
    }
}

/// Parse a token-endpoint body into [`crate::oauth_refresh::OAuthTokenResponse`].
///
/// `previous_refresh` is reused when the server omits `refresh_token`
/// (legal — RFC 6749 §6 says the refresh token MAY be unchanged), and
/// defaults to the empty string when a caller has none, so a provider that
/// issues no refresh token does not force a special case at every call site.
pub fn parse_token_response(
    body: &str,
    previous_refresh: Option<&str>,
) -> Result<crate::oauth_refresh::OAuthTokenResponse> {
    let payload: TokenPayload = serde_json::from_str(body)
        .map_err(|e| OAuthFlowError::InvalidResponse(format!("token response: {e}")))?;

    if let Some(err) = payload.error.as_deref() {
        let detail = payload
            .error_description
            .as_deref()
            .unwrap_or("no description");
        return Err(error_variant(err, detail)
            .unwrap_or_else(|| OAuthFlowError::InvalidResponse(format!("{err}: {detail}"))));
    }

    let access_token = payload
        .access_token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            OAuthFlowError::InvalidResponse("token response missing access_token".to_string())
        })?;

    let _ = payload.scope.as_deref();

    Ok(crate::oauth_refresh::OAuthTokenResponse {
        access_token,
        refresh_token: payload
            .refresh_token
            .or_else(|| previous_refresh.map(str::to_string))
            .unwrap_or_default(),
        expires_in: payload.expires_in,
        token_type: payload.token_type.or_else(|| Some("Bearer".to_string())),
        id_token: payload.id_token,
    })
}

/// Raise the right variant for a non-2xx token-endpoint response.
pub fn http_failure(url: &str, status: u16, body: &str) -> OAuthFlowError {
    // A 400 from a token endpoint carries the machine-readable OAuth error in
    // the body; prefer it over the bare status so callers can match on
    // `Pending` / `Expired` / `SlowDown` even when the status is uniform.
    if (status == 400 || status == 428)
        && let Ok(payload) = serde_json::from_str::<TokenPayload>(body)
        && let Some(err) = payload.error.as_deref()
    {
        let detail = payload
            .error_description
            .as_deref()
            .unwrap_or("no description");
        if let Some(variant) = error_variant(err, detail) {
            return variant;
        }
    }
    OAuthFlowError::Http {
        url: url.to_string(),
        status,
        body: clamp_body(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_response_prefers_server_refresh_token() {
        let got = parse_token_response(
            r#"{"access_token":"at-2","refresh_token":"rt-2","expires_in":3600}"#,
            Some("rt-1"),
        )
        .expect("parsed");
        assert_eq!(got.access_token, "at-2");
        assert_eq!(got.refresh_token, "rt-2");
        assert_eq!(got.expires_in, Some(3600));
    }

    #[test]
    fn token_response_reuses_previous_refresh_token_when_absent() {
        let got = parse_token_response(r#"{"access_token":"at-2"}"#, Some("rt-1")).expect("parsed");
        assert_eq!(got.refresh_token, "rt-1");
        assert_eq!(got.token_type.as_deref(), Some("Bearer"));
    }

    #[test]
    fn token_response_maps_oauth_error_codes_to_typed_variants() {
        for (body, want) in [
            (r#"{"error":"authorization_pending"}"#, "pending"),
            (r#"{"error":"slow_down"}"#, "slow"),
            (r#"{"error":"expired_token"}"#, "expired"),
            (
                r#"{"error":"access_denied","error_description":"no"}"#,
                "denied",
            ),
            (
                r#"{"error":"invalid_grant","error_description":"bad"}"#,
                "invalid",
            ),
        ] {
            let err = parse_token_response(body, None).expect_err("must fail");
            let text = err.to_string().to_lowercase();
            assert!(
                text.contains(want),
                "{body} produced {err:?}, expected a {want:?} variant"
            );
        }
        // A body with neither an error nor a token is unparseable, not pending.
        let err = parse_token_response(r#"{"token_type":"Bearer"}"#, None).expect_err("fails");
        assert!(
            matches!(err, OAuthFlowError::InvalidResponse(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn http_failure_truncates_hostile_bodies() {
        let hostile = "x".repeat(MAX_ERROR_BODY * 3);
        let err = http_failure("https://example.test/token", 500, &hostile);
        let OAuthFlowError::Http { body, status, .. } = &err else {
            panic!("expected Http variant, got {err:?}");
        };
        assert_eq!(*status, 500);
        assert!(
            body.len() <= MAX_ERROR_BODY + 4,
            "body not clamped: {}",
            body.len()
        );
    }

    #[test]
    fn http_failure_prefers_oauth_code_over_bare_status() {
        let err = http_failure(
            "https://example.test/token",
            400,
            r#"{"error":"authorization_pending"}"#,
        );
        assert!(matches!(err, OAuthFlowError::Pending));
    }
}

//! OAuth 2.0 Device Authorization Grant ([RFC 8628](https://www.rfc-editor.org/rfc/rfc8628)).
//!
//! The only way to log in on a box with no browser: the server hands back a
//! short user code and a verification URL, the human enters them on another
//! device, and the client polls the token endpoint until approval lands.
//!
//! ## The server owns the cadence
//!
//! RFC 8628 §3.5 defines the polling contract, and this implementation
//! follows it rather than picking its own rhythm:
//!
//! * `interval` from the device-authorization response is the minimum delay
//!   between polls. It is honoured verbatim, with the spec's own default of
//!   **5 s** when the server omits it. Polling before the interval has
//!   elapsed is what earns a `slow_down`.
//! * `slow_down` increments the interval by **5 s** (§3.5) rather than
//!   polling again at the same rate.
//! * `expires_in` is the hard deadline. The loop is bounded by
//!   `authorization_started + expires_in`, so a device code that is never
//!   approved terminates instead of polling forever.
//! * `authorization_pending` continues, `access_denied` and `expired_token`
//!   stop.
//!
//! [`Clock`] makes the timing observable without real sleeping, which is what
//! lets the cadence be asserted in a unit test.

// `Arc`/`Mutex` back `VirtualClock` and the test HTTP endpoint, both of which
// are `#[cfg(test)]`; importing them unconditionally would warn in a normal
// build. `Duration`/`Instant` are used by the production `Clock` trait.
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::http::{HttpEndpoint, HttpRequest};
use super::{OAuthFlowError, Result, parse_token_response};

/// The grant type a device-code token request must use (RFC 8628 §3.4).
pub const DEVICE_CODE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// RFC 8628 §3.2 default when the server omits `interval`.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// RFC 8628 §3.5 increment applied on `slow_down`.
pub const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// A boxed, runtime-agnostic sleep future, so [`Clock`] stays object-safe
/// without every implementor restating the `Pin<Box<dyn Future>>` dance.
pub type SleepFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// Time source for the polling loop. Real in production, virtual in tests.
pub trait Clock: Send + Sync {
    /// Suspend the current task.
    fn sleep(&self, duration: Duration) -> SleepFuture<'_>;
    /// Current instant, used to enforce `expires_in`.
    fn now(&self) -> Instant;
}

/// The production clock: real `tokio::time`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn sleep(&self, duration: Duration) -> SleepFuture<'_> {
        Box::pin(tokio::time::sleep(duration))
    }

    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Endpoints and client identity for a device-code login.
#[derive(Debug, Clone)]
pub struct DeviceFlowConfig {
    /// Public client id (PKCE-less clients are fine here — RFC 8628 §3.1
    /// authenticates the *user*, not the client).
    pub client_id: String,
    /// RFC 8628 §3.1 device authorization endpoint.
    pub device_authorization_endpoint: String,
    /// Token endpoint to poll.
    pub token_endpoint: String,
    /// Space-delimited scopes.
    pub scope: Option<String>,
    /// Extra fields for the device authorization request (e.g. `audience`).
    pub extra_authorization_params: Vec<(String, String)>,
}

impl DeviceFlowConfig {
    /// Minimal config for the two required endpoints.
    pub fn new(
        client_id: impl Into<String>,
        device_authorization_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            device_authorization_endpoint: device_authorization_endpoint.into(),
            token_endpoint: token_endpoint.into(),
            scope: None,
            extra_authorization_params: Vec::new(),
        }
    }

    /// Set the requested scope.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

/// The server's instructions for the human (RFC 8628 §3.2).
#[derive(Debug, Clone)]
pub struct DeviceAuthorization {
    /// Opaque code the client polls with. Secret-ish: never log it.
    pub device_code: String,
    /// Short code the human types.
    pub user_code: String,
    /// Where the human goes.
    pub verification_uri: String,
    /// Pre-filled variant, when the server offers one.
    pub verification_uri_complete: Option<String>,
    /// How long the device code stays valid.
    pub expires_in: Duration,
    /// Minimum delay between polls. Defaults to [`DEFAULT_POLL_INTERVAL`].
    pub interval: Duration,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizationPayload {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

/// Request a device code.
///
/// Returns the instructions to show the human; nothing is polled yet.
pub async fn start(
    http: &dyn HttpEndpoint,
    config: &DeviceFlowConfig,
) -> Result<DeviceAuthorization> {
    if config.client_id.trim().is_empty() {
        return Err(OAuthFlowError::Config(
            "device flow requires a client_id".to_string(),
        ));
    }

    let mut form: Vec<(String, String)> = vec![("client_id".to_string(), config.client_id.clone())];
    if let Some(scope) = config.scope.as_deref().filter(|s| !s.is_empty()) {
        form.push(("scope".to_string(), scope.to_string()));
    }
    form.extend(config.extra_authorization_params.iter().cloned());

    let url = config.device_authorization_endpoint.clone();
    let response = http.send(HttpRequest::post_form(url.clone(), form)).await?;
    let response = response.into_success(&url)?;
    let payload: DeviceAuthorizationPayload = serde_json::from_str(&response.body)
        .map_err(|e| OAuthFlowError::InvalidResponse(format!("device authorization: {e}")))?;

    if payload.device_code.is_empty() || payload.user_code.is_empty() {
        return Err(OAuthFlowError::InvalidResponse(
            "device authorization response missing device_code or user_code".to_string(),
        ));
    }

    Ok(DeviceAuthorization {
        device_code: payload.device_code,
        user_code: payload.user_code,
        verification_uri: payload.verification_uri,
        verification_uri_complete: payload.verification_uri_complete,
        expires_in: Duration::from_secs(payload.expires_in),
        // RFC 8628 §3.2: "If no value is provided, clients MUST use 5 as the
        // default."
        interval: Duration::from_secs(payload.interval.unwrap_or(DEFAULT_POLL_INTERVAL.as_secs())),
    })
}

/// Poll the token endpoint until the human approves, denies, or the code
/// expires.
///
/// `previous_refresh` seeds the returned refresh token for servers that
/// omit it from the response.
pub async fn poll(
    http: &dyn HttpEndpoint,
    config: &DeviceFlowConfig,
    authorization: &DeviceAuthorization,
    clock: &dyn Clock,
) -> Result<crate::oauth_refresh::OAuthTokenResponse> {
    let started = clock.now();
    let deadline = started + authorization.expires_in;
    let mut interval = if authorization.interval.is_zero() {
        DEFAULT_POLL_INTERVAL
    } else {
        authorization.interval
    };

    loop {
        // RFC 8628 §3.5: do not poll faster than the advertised interval. The
        // sleep precedes the first poll, so a user who approves instantly
        // still waits exactly one interval.
        let remaining = deadline.saturating_duration_since(clock.now());
        if interval >= remaining {
            return Err(OAuthFlowError::Expired(format!(
                "device code expired after {:?} without approval",
                authorization.expires_in
            )));
        }
        clock.sleep(interval).await;

        if clock.now() >= deadline {
            return Err(OAuthFlowError::Expired(format!(
                "device code expired after {:?} without approval",
                authorization.expires_in
            )));
        }

        let url = config.token_endpoint.clone();
        let request = HttpRequest::post_form(
            url.clone(),
            [
                ("grant_type".to_string(), DEVICE_CODE_GRANT_TYPE.to_string()),
                ("client_id".to_string(), config.client_id.clone()),
                ("device_code".to_string(), authorization.device_code.clone()),
            ],
        );
        let response = http.send(request).await?;

        match response.into_success(&url) {
            Ok(ok) => {
                return parse_token_response(&ok.body, None);
            }
            Err(OAuthFlowError::Pending) => continue,
            Err(OAuthFlowError::SlowDown) => {
                // RFC 8628 §3.5: increase the interval by 5 s before retrying.
                interval += SLOW_DOWN_INCREMENT;
                continue;
            }
            Err(other) => return Err(other),
        }
    }
}

/// A [`Clock`] that never sleeps: it records the requested durations and
/// advances a virtual instant. Test-only; lives here so the trait and its
/// fake stay together.
///
/// Gated so the fake does not ship in the library. It was ungated until
/// iter-411, which put a test double on the public API and dragged its
/// `Mutex::lock().expect(..)` poison recovery into production — where the
/// clippy gate (`-D clippy::expect_used`) correctly flagged it. The production
/// clock is `SystemClock`; this exists only to make the device flow's polling
/// interval testable without real sleeps.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct VirtualClock {
    state: Arc<Mutex<VirtualState>>,
}

#[cfg(test)]
#[derive(Debug)]
struct VirtualState {
    now: Instant,
    slept: Vec<Duration>,
}

#[cfg(test)]
impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl VirtualClock {
    /// A clock starting at the process origin.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(VirtualState {
                now: Instant::now(),
                slept: Vec::new(),
            })),
        }
    }

    /// Every duration this clock was asked to sleep, in order.
    pub fn sleeps(&self) -> Vec<Duration> {
        self.state.lock().expect("clock lock").slept.clone()
    }
}

#[cfg(test)]
impl Clock for VirtualClock {
    fn sleep(&self, duration: Duration) -> SleepFuture<'_> {
        let state = self.state.clone();
        Box::pin(async move {
            let mut guard = state.lock().expect("clock lock");
            guard.now += duration;
            guard.slept.push(duration);
        })
    }

    fn now(&self) -> Instant {
        self.state.lock().expect("clock lock").now
    }
}

#[cfg(test)]
mod tests {
    use super::super::http::HttpResponse;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingEndpoint {
        replies: Mutex<Vec<HttpResponse>>,
        calls: AtomicUsize,
    }

    impl CountingEndpoint {
        fn new(replies: Vec<HttpResponse>) -> Arc<Self> {
            Arc::new(Self {
                replies: Mutex::new(replies),
                calls: AtomicUsize::new(0),
            })
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl HttpEndpoint for CountingEndpoint {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // Serve in order with a cursor. Indexing by call count would drift
            // as soon as the queue shrinks — `remove(0)` is the whole script.
            let mut replies = self.replies.lock().expect("replies");
            if replies.is_empty() {
                return Ok(HttpResponse {
                    status: 400,
                    body: r#"{"error":"expired_token"}"#.to_string(),
                });
            }
            Ok(replies.remove(0))
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_string(),
        }
    }

    fn pending() -> HttpResponse {
        HttpResponse {
            status: 400,
            body: r#"{"error":"authorization_pending"}"#.to_string(),
        }
    }

    fn slow_down() -> HttpResponse {
        HttpResponse {
            status: 400,
            body: r#"{"error":"slow_down"}"#.to_string(),
        }
    }

    fn config() -> DeviceFlowConfig {
        DeviceFlowConfig::new(
            "client-1",
            "https://as.test/device_authorization",
            "https://as.test/token",
        )
        .with_scope("openid profile")
    }

    #[tokio::test]
    async fn device_start_honours_server_interval_and_expiry() {
        let http = CountingEndpoint::new(vec![ok(
            r#"{"device_code":"dc-1","user_code":"WXYZ-1234","verification_uri":"https://as.test/device","expires_in":900,"interval":7}"#,
        )]);
        let auth = start(http.as_ref(), &config()).await.expect("device code");
        assert_eq!(auth.device_code, "dc-1");
        assert_eq!(auth.user_code, "WXYZ-1234");
        assert_eq!(auth.interval, Duration::from_secs(7));
        assert_eq!(auth.expires_in, Duration::from_secs(900));
        assert_eq!(http.calls(), 1);
    }

    #[tokio::test]
    async fn device_start_defaults_interval_to_five_seconds() {
        let http = CountingEndpoint::new(vec![ok(
            r#"{"device_code":"dc","user_code":"UC","verification_uri":"https://as.test/d","expires_in":600}"#,
        )]);
        let auth = start(http.as_ref(), &config()).await.expect("device code");
        assert_eq!(auth.interval, DEFAULT_POLL_INTERVAL);
    }

    #[tokio::test]
    async fn device_poll_sleeps_the_server_interval_between_attempts() {
        // Two pendings, then success. With interval=7 the client must sleep
        // 7s three times: the sleep precedes each poll, including the first.
        let http = CountingEndpoint::new(vec![
            pending(),
            pending(),
            ok(r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#),
        ]);
        let clock = VirtualClock::new();
        let auth = DeviceAuthorization {
            device_code: "dc-1".to_string(),
            user_code: "UC".to_string(),
            verification_uri: "https://as.test/d".to_string(),
            verification_uri_complete: None,
            expires_in: Duration::from_secs(900),
            interval: Duration::from_secs(7),
        };

        let token = poll(http.as_ref(), &config(), &auth, &clock)
            .await
            .expect("approved");
        assert_eq!(token.access_token, "at-1");
        assert_eq!(token.refresh_token, "rt-1");
        assert_eq!(http.calls(), 3);
        assert_eq!(
            clock.sleeps(),
            vec![
                Duration::from_secs(7),
                Duration::from_secs(7),
                Duration::from_secs(7)
            ],
            "poll cadence must come from the server's interval"
        );
    }

    #[tokio::test]
    async fn device_poll_applies_slow_down_increment_then_succeeds() {
        let http = CountingEndpoint::new(vec![
            slow_down(),
            ok(r#"{"access_token":"at-2","refresh_token":"rt-2"}"#),
        ]);
        let clock = VirtualClock::new();
        let auth = DeviceAuthorization {
            device_code: "dc".to_string(),
            user_code: "UC".to_string(),
            verification_uri: "https://as.test/d".to_string(),
            verification_uri_complete: None,
            expires_in: Duration::from_secs(900),
            interval: Duration::from_secs(5),
        };

        poll(http.as_ref(), &config(), &auth, &clock)
            .await
            .expect("approved after slow_down");
        assert_eq!(
            clock.sleeps(),
            vec![Duration::from_secs(5), Duration::from_secs(10)],
            "slow_down must add RFC 8628 §3.5's 5 s, not repeat the same cadence"
        );
    }

    #[tokio::test]
    async fn device_poll_stops_at_expires_in_instead_of_polling_forever() {
        // Never approved: the endpoint would hand over a token if it were ever
        // asked, so a green test proves the deadline stopped us, not the server.
        let http =
            CountingEndpoint::new(vec![pending(), pending(), ok(r#"{"access_token":"nope"}"#)]);
        let clock = VirtualClock::new();
        let auth = DeviceAuthorization {
            device_code: "dc".to_string(),
            user_code: "UC".to_string(),
            verification_uri: "https://as.test/d".to_string(),
            verification_uri_complete: None,
            // 12s window, 5s interval → at most 2 polls, then expiry.
            expires_in: Duration::from_secs(12),
            interval: Duration::from_secs(5),
        };

        let err = poll(http.as_ref(), &config(), &auth, &clock)
            .await
            .expect_err("must expire");
        assert!(matches!(err, OAuthFlowError::Expired(_)), "got {err:?}");
        assert!(
            http.calls() <= 2,
            "polled {} times past the deadline",
            http.calls()
        );
    }

    #[tokio::test]
    async fn device_poll_surfaces_denial_without_retrying() {
        let http = CountingEndpoint::new(vec![HttpResponse {
            status: 400,
            body: r#"{"error":"access_denied","error_description":"user said no"}"#.to_string(),
        }]);
        let clock = VirtualClock::new();
        let auth = DeviceAuthorization {
            device_code: "dc".to_string(),
            user_code: "UC".to_string(),
            verification_uri: "https://as.test/d".to_string(),
            verification_uri_complete: None,
            expires_in: Duration::from_secs(900),
            interval: Duration::from_secs(5),
        };

        let err = poll(http.as_ref(), &config(), &auth, &clock)
            .await
            .expect_err("denied");
        assert!(matches!(err, OAuthFlowError::Denied(_)), "got {err:?}");
        assert_eq!(http.calls(), 1, "a denial is terminal");
    }

    #[tokio::test]
    async fn device_start_rejects_a_blank_client_id() {
        let config = DeviceFlowConfig::new("", "https://as.test/d", "https://as.test/token");
        let http = CountingEndpoint::new(vec![]);
        let err = start(http.as_ref(), &config)
            .await
            .expect_err("no client id");
        assert!(matches!(err, OAuthFlowError::Config(_)));
        assert_eq!(http.calls(), 0, "must not hit the network");
    }
}

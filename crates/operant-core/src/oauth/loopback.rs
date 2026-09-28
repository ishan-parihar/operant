//! Loopback redirect listener for the authorization-code flow.
//!
//! # Why this is not "spin up a server on 8080"
//!
//! A hardcoded redirect port has three failure modes: two concurrent logins
//! collide (one wins the bind, the other is left waiting on a port it never
//! owns), a stale process holds the port, and a hostile local process can
//! pre-bind it *before* we do and receive the authorization code. Binding
//! `127.0.0.1:0` and reporting the kernel's choice removes all three: the
//! kernel hands out a port that is, at that instant, free.
//!
//! # Why the read is bounded twice over
//!
//! The socket is **unauthenticated** — anything that can reach loopback can
//! connect. Two bounds are therefore mandatory, not defensive:
//!
//! * [`MAX_CALLBACK_BYTES`] caps how much is buffered, so a peer that keeps
//!   sending cannot grow the heap.
//! * [`CALLBACK_READ_TIMEOUT`] caps how long, so a peer that connects and then
//!   says nothing cannot hold the flow open forever.
//!
//! The overall deadline ([`DEFAULT_CALLBACK_TIMEOUT`]) is separate again: it
//! bounds the *user's* time to click "Approve" in the browser.
//!
//! # State verification
//!
//! [`verify_state`] is called on every callback before the code is trusted.
//! Comparison is constant-time (`constant_time_eq`, already a dependency of
//! this crate) so a mismatched state cannot be recovered byte-by-byte from
//! timing. A mismatch is [`OAuthFlowError::StateMismatch`], never a warning.

use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout as tokio_timeout;

use super::{OAuthFlowError, Result};

/// Longest callback request we will buffer (request line + headers).
///
/// 8 KiB comfortably covers a browser redirect — the URL carries a code, a
/// state and optionally a scope, well under 2 KiB — while keeping the
/// allocation fixed.
pub const MAX_CALLBACK_BYTES: usize = 8 * 1024;

/// How long a connected peer has to deliver its request once accepted.
pub const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the user has to complete the browser redirect.
pub const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// The path we register as the redirect target.
pub const CALLBACK_PATH: &str = "/callback";

/// The parsed, still-unverified parameters of a redirect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    /// Authorization code.
    pub code: String,
    /// Echoed `state` — must be compared against the value we issued.
    pub state: String,
    /// Granted scopes, when the server sent them.
    pub scope: Option<String>,
}

/// Compare a callback's `state` against the one we issued.
///
/// Constant-time on both the length and the bytes, and a missing/empty
/// received value is a mismatch. An unverified `state` is a CSRF hole: an
/// attacker who can make our listener receive *their* authorization code
/// would otherwise have it redeemed against our client.
pub fn verify_state(expected: &str, received: &str) -> Result<()> {
    if expected.is_empty() {
        return Err(OAuthFlowError::Config(
            "refusing to accept a callback: no state was issued".to_string(),
        ));
    }
    if received.is_empty() {
        return Err(OAuthFlowError::StateMismatch);
    }
    if !constant_time_eq::constant_time_eq(expected.as_bytes(), received.as_bytes()) {
        return Err(OAuthFlowError::StateMismatch);
    }
    Ok(())
}

/// Percent-decode one query-string component (`+` is a space in
/// `application/x-www-form-urlencoded`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a raw HTTP request into [`Callback`]. Pure, so it is testable
/// without a socket.
///
/// Rejects anything that is not a `GET /callback` carrying a `code`; surfaces
/// the server's `error` parameter as the matching typed variant.
pub fn parse_callback_request(raw: &str) -> Result<Callback> {
    let request_line = raw
        .lines()
        .next()
        .ok_or_else(|| OAuthFlowError::InvalidResponse("empty callback request".to_string()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts
        .next()
        .ok_or_else(|| OAuthFlowError::InvalidResponse("callback has no target".to_string()))?;

    if !method.eq_ignore_ascii_case("GET") {
        return Err(OAuthFlowError::InvalidResponse(format!(
            "callback must be a GET, got {method}"
        )));
    }

    let path = target.split('?').next().unwrap_or("");
    if !path.eq_ignore_ascii_case(CALLBACK_PATH) {
        return Err(OAuthFlowError::InvalidResponse(format!(
            "callback path must be {CALLBACK_PATH}, got {path}"
        )));
    }

    let mut code = None;
    let mut state = None;
    let mut scope = None;
    let mut error = None;
    if let Some((_, query)) = target.split_once('?') {
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = percent_decode(value);
            match key {
                "code" => code = Some(value),
                "state" => state = Some(value),
                "scope" => scope = Some(value),
                "error" => error = Some(value),
                _ => {}
            }
        }
    }

    if let Some(err) = error {
        return Err(match err.as_str() {
            "access_denied" => OAuthFlowError::Denied("user denied the request".to_string()),
            "expired_token" | "invalid_request" => OAuthFlowError::Expired(err),
            _ => OAuthFlowError::InvalidResponse(format!("authorization error: {err}")),
        });
    }

    let code = code.ok_or_else(|| {
        OAuthFlowError::InvalidResponse("callback carried no authorization code".to_string())
    })?;

    Ok(Callback {
        code,
        // An absent state is reported as empty so `verify_state` rejects it.
        state: state.unwrap_or_default(),
        scope,
    })
}

/// Render the (tiny) HTML response the browser shows.
fn page(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

const PAGE_SUCCESS: &str = "<html><body><h2>Authorization successful</h2><p>You can close this tab and return to Operant.</p></body></html>";
const PAGE_STATE_MISMATCH: &str = "<html><body><h2>Authorization failed</h2><p>State mismatch — request rejected.</p></body></html>";
const PAGE_INVALID: &str =
    "<html><body><h2>Authorization failed</h2><p>Invalid callback.</p></body></html>";

/// Decide the browser reply and the flow's outcome from one parsed callback.
///
/// The `state` check happens here, before the code is handed back: a callback
/// we did not start is not a code we may redeem.
fn respond(
    parsed: Result<Callback>,
    expected_state: &str,
) -> (String, std::result::Result<Callback, OAuthFlowError>) {
    match parsed {
        Ok(callback) => match verify_state(expected_state, &callback.state) {
            Ok(()) => (page(PAGE_SUCCESS), Ok(callback)),
            Err(e) => (page(PAGE_STATE_MISMATCH), Err(e)),
        },
        Err(e) => (page(PAGE_INVALID), Err(e)),
    }
}

/// A bound loopback listener waiting for exactly one redirect.
///
/// The listener accepts one connection, reads it under
/// [`CALLBACK_READ_TIMEOUT`] and [`MAX_CALLBACK_BYTES`], and is done; drop the
/// `Loopback` to close the socket.
#[derive(Debug)]
pub struct Loopback {
    listener: TcpListener,
    port: u16,
    state: String,
}

impl Loopback {
    /// Bind `127.0.0.1:0` and arm it to expect `expected_state`.
    ///
    /// Binding port 0 asks the kernel for a free ephemeral port, so concurrent
    /// logins cannot collide and a pre-bound port cannot be squatted.
    pub async fn bind(expected_state: impl Into<String>) -> Result<Self> {
        let state = expected_state.into();
        if state.is_empty() {
            return Err(OAuthFlowError::Config(
                "loopback listener requires a non-empty expected state".to_string(),
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            port,
            state,
        })
    }

    /// The port the kernel actually assigned.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The redirect URI to send to the authorization server.
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}{CALLBACK_PATH}", self.port)
    }

    /// The `state` this listener will require.
    pub fn expected_state(&self) -> &str {
        &self.state
    }

    /// Wait for the redirect, then verify its `state`.
    ///
    /// Fails with [`OAuthFlowError::Timeout`] if nothing connects within
    /// `deadline`, [`OAuthFlowError::TooLarge`] if the peer exceeds
    /// [`MAX_CALLBACK_BYTES`], and [`OAuthFlowError::StateMismatch`] if the
    /// echoed state is absent or different.
    pub async fn await_callback(&self, deadline: Duration) -> Result<Callback> {
        let (mut socket, _peer) = tokio_timeout(deadline, self.listener.accept())
            .await
            .map_err(|_| OAuthFlowError::Timeout(deadline))??;

        // Accumulate until end-of-headers, but never past the cap.
        let mut raw = Vec::with_capacity(1024);
        let mut chunk = [0u8; 1024];
        loop {
            let read = tokio_timeout(CALLBACK_READ_TIMEOUT, socket.read(&mut chunk))
                .await
                .map_err(|_| OAuthFlowError::Timeout(CALLBACK_READ_TIMEOUT))??;
            if read == 0 {
                break;
            }
            if raw.len() + read > MAX_CALLBACK_BYTES {
                let _ = socket
                    .write_all(page("<html><body>Request too large</body></html>").as_bytes())
                    .await;
                return Err(OAuthFlowError::TooLarge {
                    limit: MAX_CALLBACK_BYTES,
                });
            }
            raw.extend_from_slice(&chunk[..read]);
            if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let request = String::from_utf8_lossy(&raw).into_owned();
        let (reply, result) = respond(parse_callback_request(&request), &self.state);

        // Best-effort reply: the browser is waiting on it, but a dead socket
        // must not turn a successful authorization into a failure.
        let _ = socket.write_all(reply.as_bytes()).await;
        let _ = socket.flush().await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_state_rejects_every_mismatch_shape() {
        let expected = "state-abc-123";

        // The happy path.
        assert!(verify_state(expected, expected).is_ok());

        // Every way of being wrong must be a hard rejection, never a warning.
        for received in [
            "",               // absent
            "state-abc-124",  // last character differs
            "state-abc-12",   // truncated
            "state-abc-1234", // extended
            "STATE-ABC-123",  // case-folded
            " attacker",      // unrelated
            "state-abc-12é",  // non-ASCII smuggled in
        ] {
            let err =
                verify_state(expected, received).expect_err("mismatched state must be rejected");
            assert!(
                matches!(err, OAuthFlowError::StateMismatch),
                "expected StateMismatch for {received:?}, got {err:?}"
            );
        }

        // Refusing to arm a listener with no state at all — otherwise every
        // callback would trivially "match".
        let err = verify_state("", "anything").expect_err("empty expected state");
        assert!(matches!(err, OAuthFlowError::Config(_)));
    }

    #[tokio::test]
    async fn oauth_callback_should_reject_mismatched_state() {
        // The end-to-end shape, over a real socket: a callback whose `state`
        // does not match the value we issued never yields a code, even though
        // the request is otherwise well-formed and carries a code.
        let listener = Loopback::bind("state-abc-123").await.expect("bind");
        let port = listener.port();
        let waiter =
            tokio::spawn(async move { listener.await_callback(Duration::from_secs(10)).await });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        stream
            .write_all(
                b"GET /callback?code=attacker-code&state=state-abc-124 HTTP/1.1\r\nHost: x\r\n\r\n",
            )
            .await
            .expect("write");
        let mut reply = Vec::new();
        let _ = stream.read_to_end(&mut reply).await;
        let body = String::from_utf8_lossy(&reply);
        assert!(body.contains("State mismatch"), "browser told: {body}");

        let err = waiter
            .await
            .expect("join")
            .expect_err("mismatched state rejected");
        assert!(matches!(err, OAuthFlowError::StateMismatch));
    }

    #[tokio::test]
    async fn loopback_listener_should_bind_an_ephemeral_port() {
        // Port 0 → the kernel picks; the listener must report the real port,
        // and two live listeners must not share it.
        let a = Loopback::bind("state-a").await.expect("bind a");
        let b = Loopback::bind("state-b").await.expect("bind b");

        assert_ne!(a.port(), 0, "port 0 means the bind did not resolve");
        assert_ne!(b.port(), 0, "port 0 means the bind did not resolve");
        assert_ne!(
            a.port(),
            b.port(),
            "concurrent logins must not collide on one port"
        );
        assert_eq!(
            a.redirect_uri(),
            format!("http://127.0.0.1:{}{CALLBACK_PATH}", a.port())
        );
        assert_eq!(a.expected_state(), "state-a");
        assert!(
            a.port() > 1024,
            "kernel picks an ephemeral port: {}",
            a.port()
        );
    }

    #[tokio::test]
    async fn loopback_happy_path_returns_code_after_state_check() {
        let listener = Loopback::bind("state-xyz").await.expect("bind");
        let port = listener.port();
        let waiter =
            tokio::spawn(async move { listener.await_callback(Duration::from_secs(5)).await });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let request =
            "GET /callback?code=the-code&state=state-xyz&scope=read HTTP/1.1\r\nHost: x\r\n\r\n";
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut sink = Vec::new();
        let _ = stream.read_to_end(&mut sink).await;

        let callback = waiter.await.expect("join").expect("callback accepted");
        assert_eq!(callback.code, "the-code");
        assert_eq!(callback.state, "state-xyz");
        assert_eq!(callback.scope.as_deref(), Some("read"));
    }

    #[tokio::test]
    async fn loopback_rejects_oversized_request() {
        let listener = Loopback::bind("state-xyz").await.expect("bind");
        let port = listener.port();
        let waiter =
            tokio::spawn(async move { listener.await_callback(Duration::from_secs(5)).await });

        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        let filler = "x".repeat(MAX_CALLBACK_BYTES + 1024);
        let request = format!("GET /callback?code={filler} HTTP/1.1\r\n\r\n");
        let _ = stream.write_all(request.as_bytes()).await;

        let err = waiter.await.expect("join").expect_err("must be rejected");
        assert!(matches!(err, OAuthFlowError::TooLarge { .. }));
    }

    #[test]
    fn callback_parser_surfaces_server_error_parameters() {
        let denied = parse_callback_request(
            "GET /callback?error=access_denied&error_description=no HTTP/1.1",
        )
        .expect_err("denied");
        assert!(matches!(denied, OAuthFlowError::Denied(_)));

        let no_code =
            parse_callback_request("GET /callback?state=abc HTTP/1.1").expect_err("no code");
        assert!(matches!(no_code, OAuthFlowError::InvalidResponse(_)));

        let wrong_path =
            parse_callback_request("GET /evil?code=x&state=y HTTP/1.1").expect_err("wrong path");
        assert!(matches!(wrong_path, OAuthFlowError::InvalidResponse(_)));

        // Percent-encoding round-trip, so a code containing reserved bytes
        // survives the query string intact.
        let parsed = parse_callback_request("GET /callback?code=a%2Fb%2Bc%3D&state=s%2Ft HTTP/1.1")
            .expect("parsed");
        assert_eq!(parsed.code, "a/b+c=");
        assert_eq!(parsed.state, "s/t");
    }
}

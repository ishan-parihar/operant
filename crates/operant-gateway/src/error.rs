//! Typed error type for the gateway crate.
//!
//! The gateway's own code paths — TLS/mTLS setup, pairing persistence,
//! chat dispatch, WebSocket session cwd resolution — return
//! [`Result<T>`], a typed error that replaces `anyhow::Result` for
//! internal logic. `anyhow` remains only at the trait-boundary seams
//! where upstream contracts still return it:
//!
//! - `operant-api`'s `Tool` / `Channel` / `Provider` trait methods
//!   (implemented by `node_tool.rs`, `ws_approval.rs`, and test mocks),
//! - `operant-config`'s `Config` methods (consumed by
//!   `api_config.rs::map_prop_error` and `Config::save`),
//! - `operant-runtime`'s `process_message` (the agent dispatch entry).
//!
//! Those wrap into [`Error::Backend`] via `?` at the gateway boundary.

use thiserror::Error as ThisError;

/// Error type for gateway-internal operations.
#[derive(Debug, ThisError)]
pub enum Error {
    /// Plain message with no underlying source.
    #[error("{0}")]
    Message(String),

    /// I/O error (listener bind, axum serve, cert/key file reads).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// `host:port` → `SocketAddr` parse failure.
    #[error("address parse error: {0}")]
    AddrParse(#[from] std::net::AddrParseError),

    /// Error from an upstream crate that still uses `anyhow`
    /// (trait-boundary seam: Provider, Channel, Config, runtime).
    #[error("backend error: {0}")]
    Backend(#[from] anyhow::Error),

    /// The gateway has no model configured — browser onboarding required.
    #[error(
        "needs_onboarding: gateway has no model configured. Complete browser onboarding \
         at /onboard, or set [providers.models.<name>] model = \"...\" before sending messages."
    )]
    NeedsOnboarding,
}

impl Error {
    /// Construct a plain-message error (replaces `anyhow::anyhow!` / `bail!`).
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

/// Drop-in replacement for `anyhow::Context` (`.context(...)` /
/// `.with_context(...)` on `Result`/`Option`), returning the crate's
/// typed [`Result`] instead of `anyhow::Result`. Import with
/// `use crate::error::GatewayContextExt as _;`.
pub trait GatewayContextExt<T> {
    /// Wrap the error (or absence of a value) with a context message.
    fn context(self, context: impl Into<String>) -> Result<T>;

    /// Lazy variant of [`context`](Self::context) taking a closure.
    fn with_context(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E> GatewayContextExt<T> for std::result::Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        let context = context.into();
        self.map_err(|source| Error::Message(format!("{context}: {source}")))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|source| Error::Message(format!("{}: {source}", context())))
    }
}

impl<T> GatewayContextExt<T> for Option<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.ok_or_else(|| Error::Message(context.into()))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.ok_or_else(|| Error::Message(context()))
    }
}


/// Convenience alias used throughout the crate.
///
/// The second type parameter defaults to [`Error`] (mirroring
/// `anyhow::Result`) so axum handler signatures that pin a different
/// error type — e.g. `Result<Json<WebhookBody>, JsonRejection>` — keep
/// working unchanged.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_accepts_plain_str_and_owned_string() {
        assert!(matches!(Error::message("boom"), Error::Message(m) if m == "boom"));
        assert!(matches!(
            Error::message(String::from("boom")),
            Error::Message(m) if m == "boom"
        ));
    }

    #[test]
    fn context_on_result_wraps_with_source() {
        let err = std::result::Result::<(), std::io::Error>::Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no such file",
        ))
        .context("open cert");
        assert!(matches!(err, Err(Error::Message(m)) if m.contains("open cert") && m.contains("no such file")));
    }

    #[test]
    fn context_on_option_supplies_message() {
        let err: Result<i32> = None.context("missing value");
        assert!(matches!(err, Err(Error::Message(m)) if m == "missing value"));
    }

    #[test]
    fn needs_onboarding_variant_carries_marker_and_url() {
        let msg = Error::NeedsOnboarding.to_string();
        assert!(msg.contains("needs_onboarding"), "missing marker: {msg}");
        assert!(msg.contains("/onboard"), "missing onboarding url: {msg}");
    }

    #[test]
    fn backend_wraps_anyhow_boundary_error() {
        let err = Error::Backend(anyhow::anyhow!("provider call failed"));
        assert!(matches!(err, Error::Backend(_)));
        assert!(err.to_string().contains("provider call failed"));
    }

    #[test]
    fn result_alias_preserves_two_arg_form() {
        // The alias must keep supporting a pinned error type, as the
        // axum extractor signatures in lib.rs rely on it.
        let parsed: Result<u32, std::num::ParseIntError> = "42".parse();
        assert_eq!(parsed.unwrap(), 42);
    }
}

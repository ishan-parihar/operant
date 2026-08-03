//! Typed error type for the memory subsystem.
//!
//! The public `Memory` trait (defined in `operant-api`) returns
//! [`MemoryResult<T>`](operant_api::memory_traits::MemoryResult), whose
//! [`MemoryError`](operant_api::memory_traits::MemoryError) seam carries a
//! boxed `Backend` variant for backend-specific errors. This crate converts
//! its typed [`Error`] into that seam via the `From` impls at the bottom, so
//! `?` works in both directions without the crate depending on `anyhow`.

use thiserror::Error as ThisError;

/// Error type for memory backends, embeddings, consolidation, and retrieval.
#[derive(Debug, ThisError)]
pub enum Error {
    /// Plain message with no underlying source.
    #[error("{0}")]
    Message(String),

    /// SQLite backend error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Filesystem / directory error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// HTTP error from an embedding or vector-store client.
    #[error("http error: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// PostgreSQL backend error (feature-gated).
    #[cfg(feature = "memory-postgres")]
    #[error("postgres error: {0}")]
    Postgres(#[from] postgres::Error),

    /// UUID parse/generation error.
    #[error("uuid error: {0}")]
    Uuid(#[from] uuid::Error),

    /// Background task (spawn_blocking) join error.
    #[error("background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),

    /// Error boxed from another `Memory` backend (`MemoryResult` seam).
    #[error(transparent)]
    Boxed(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Construct a plain-message error (replaces `anyhow::anyhow!` / `bail!`).
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

/// Drop-in replacement for `anyhow::Context` (`.context("...")` on
/// `Result`/`Option`), returning the crate's typed `Result` instead of
/// `anyhow::Result`. Import with `use crate::error::MemoryContextExt as _;`.
pub trait MemoryContextExt<T> {
    /// Wrap the error (or absence of a value) with a context message.
    fn context(self, context: impl Into<String>) -> Result<T>;

    /// Lazy variant of [`context`](Self::context) taking a closure.
    fn with_context(self, context: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E> MemoryContextExt<T> for std::result::Result<T, E>
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

impl<T> MemoryContextExt<T> for Option<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.ok_or_else(|| Error::Message(context.into()))
    }

    fn with_context(self, context: impl FnOnce() -> String) -> Result<T> {
        self.ok_or_else(|| Error::Message(context()))
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Convert the crate's typed error into the dependency-free `MemoryError`
/// used by the public `Memory` trait seam (`MemoryResult`).
impl From<Error> for operant_api::memory_traits::MemoryError {
    fn from(value: Error) -> Self {
        match value {
            Error::Message(msg) => Self::Message(msg),
            Error::Sqlite(e) => Self::Backend(Box::new(e)),
            Error::Io(e) => Self::Io(e),
            Error::Serde(e) => Self::Serde(e),
            Error::Reqwest(e) => Self::Backend(Box::new(e)),
            #[cfg(feature = "memory-postgres")]
            Error::Postgres(e) => Self::Backend(Box::new(e)),
            Error::Uuid(e) => Self::Backend(Box::new(e)),
            Error::Join(e) => Self::Backend(Box::new(e)),
            Error::Boxed(e) => Self::Backend(e),
        }
    }
}

/// Convert a `MemoryError` from the trait seam back into the crate's typed
/// error, so `?` works when calling `Memory` trait methods from crate-internal
/// functions that return [`Result`].
impl From<operant_api::memory_traits::MemoryError> for Error {
    fn from(value: operant_api::memory_traits::MemoryError) -> Self {
        match value {
            operant_api::memory_traits::MemoryError::Message(msg) => Self::Message(msg),
            operant_api::memory_traits::MemoryError::Io(e) => Self::Io(e),
            operant_api::memory_traits::MemoryError::Serde(e) => Self::Serde(e),
            operant_api::memory_traits::MemoryError::Backend(e) => Self::Boxed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_api::memory_traits::MemoryError as SeamError;

    #[test]
    fn message_accepts_plain_str_and_owned_string() {
        assert!(matches!(Error::message("boom"), Error::Message(m) if m == "boom"));
        assert!(matches!(Error::message(String::from("boom")), Error::Message(m) if m == "boom"));
    }

    #[test]
    fn context_on_result_wraps_with_source() {
        let err = std::result::Result::<(), std::io::Error>::Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no such file",
        ))
        .context("open db");
        assert!(matches!(err, Err(Error::Message(m)) if m.contains("open db") && m.contains("no such file")));
    }

    #[test]
    fn with_context_on_result_is_lazy() {
        let calls = std::cell::Cell::new(0);
        let err: crate::Result<()> = Err(Error::message("inner"))
            .with_context(|| {
                calls.set(1);
                "outer".to_string()
            });
        assert!(err.is_err());
        assert_eq!(calls.get(), 1, "closure must be evaluated");
    }

    #[test]
    fn context_on_option_supplies_message() {
        let err: crate::Result<i32> = None.context("missing value");
        assert!(matches!(err, Err(Error::Message(m)) if m == "missing value"));
    }

    #[test]
    fn typed_error_converts_to_seam_and_back_roundtrip() {
        let typed = Error::message("round trip");
        let seam: SeamError = typed.into();
        // Message variant must stay structured, not boxed, on the seam.
        assert!(matches!(&seam, SeamError::Message(m) if m == "round trip"));
        let back: Error = seam.into();
        assert!(matches!(&back, Error::Message(m) if m == "round trip"));
    }

    #[test]
    fn backend_errors_roundtrip_as_boxed_seam_variant() {
        // A backend-specific error (io::Error is available without extra deps)
        // must cross the seam as `Backend`, preserving the source for downcast.
        let typed = Error::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "backend"));
        let seam: SeamError = typed.into();
        assert!(matches!(&seam, SeamError::Io(_)));
        let back: Error = seam.into();
        assert!(matches!(&back, Error::Io(_)));

        // A truly foreign error type takes the boxed Backend path.
        let foreign: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "foreign"));
        let seam: SeamError = Error::Boxed(foreign).into();
        assert!(matches!(&seam, SeamError::Backend(_)));
    }
}

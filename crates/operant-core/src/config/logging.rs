//! `[logging]` — `LoggingSettings` and its `Default` impl.
//!
//! Split out of `config.rs` (iter-2625), the mechanical god-file split. This is
//! a pure move: the type, its doc comments and its `Default` impl are byte-for-
//! byte what they were. Behaviour-preserving by construction -- proven by the
//! config test suite being green before and after (3 pre-existing parallel-
//! execution flakes aside, see the commit body).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingSettings {
    /// The minimum level the logger emits: "trace" | "debug" | "info" | "warn" |
    /// "error". The value is case-insensitive; an unknown name is healed to
    /// "info" rather than silently logging nothing.
    pub level: String,
    pub format: String,
    pub log_file: Option<String>,
    pub with_target: bool,
    pub with_thread_ids: bool,
    pub with_file: bool,
    pub with_line_number: bool,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "pretty".to_string(),
            log_file: None,
            with_target: false,
            with_thread_ids: false,
            with_file: false,
            with_line_number: false,
        }
    }
}

//! `[code_execution]` — `CodeExecutionSettings` and its `Default` impl.
//!
//! Split out of `config.rs` (iter-2625), the mechanical god-file split. This is
//! a pure move: the type, its doc comments and its `Default` impl are byte-for-
//! byte what they were. Behaviour-preserving by construction -- proven by the
//! config test suite being green before and after (3 pre-existing parallel-
//! execution flakes aside, see the commit body).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodeExecutionSettings {
    pub default_timeout_secs: u64,
    pub max_timeout_secs: u64,
}

impl Default for CodeExecutionSettings {
    fn default() -> Self {
        Self {
            default_timeout_secs: 60,
            max_timeout_secs: 300,
        }
    }
}

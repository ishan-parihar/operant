//! `[ssh_terminal]` — `SshTerminalSettings` and its `Default` impl.
//!
//! Split out of `config.rs` (iter-2625), the mechanical god-file split. This is
//! a pure move: the type, its doc comments and its `Default` impl are byte-for-
//! byte what they were. Behaviour-preserving by construction -- proven by the
//! config test suite being green before and after (3 pre-existing parallel-
//! execution flakes aside, see the commit body).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SshTerminalSettings {
    pub host: Option<String>,
    pub user: Option<String>,
    pub port: u16,
    pub key_path: Option<String>,
}

impl Default for SshTerminalSettings {
    fn default() -> Self {
        Self {
            host: None,
            user: None,
            port: 22,
            key_path: None,
        }
    }
}

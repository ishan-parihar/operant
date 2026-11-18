//! `[docker_terminal]` — `DockerTerminalSettings` and its `Default` impl.
//!
//! Split out of `config.rs` (iter-2625), the mechanical god-file split. This is
//! a pure move: the type, its doc comments and its `Default` impl are byte-for-
//! byte what they were. Behaviour-preserving by construction -- proven by the
//! config test suite being green before and after (3 pre-existing parallel-
//! execution flakes aside, see the commit body).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerTerminalSettings {
    pub image: Option<String>,
    pub volumes: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    pub cpu: f64,
    pub memory_mb: u64,
}

impl Default for DockerTerminalSettings {
    fn default() -> Self {
        Self {
            image: Some("nikolaik/python-nodejs:python3.11-nodejs20".to_string()),
            volumes: Vec::new(),
            env: std::collections::HashMap::new(),
            cpu: 1.0,
            memory_mb: 5120,
        }
    }
}

//! `[stt]` — `SttSettings` and its `Default` impl.
//!
//! Split out of `config.rs` (iter-2625), the mechanical god-file split. This is
//! a pure move: the type, its doc comments and its `Default` impl are byte-for-
//! byte what they were. Behaviour-preserving by construction -- proven by the
//! config test suite being green before and after (3 pre-existing parallel-
//! execution flakes aside, see the commit body).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SttSettings {
    pub groq_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub groq_model: String,
    pub openai_model: String,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            groq_api_key: None,
            openai_api_key: None,
            groq_model: "whisper-large-v3-turbo".to_string(),
            openai_model: "whisper-1".to_string(),
        }
    }
}

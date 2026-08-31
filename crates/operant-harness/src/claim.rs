//! Claims — the string-keyed capability addresses of the harness kernel.
//!
//! A [`Claim`] is the analog of Cordis's `ctx.<key>` slot: providers *provide*
//! claims (they own a capability address) and *require* claims (dependencies
//! expressed as data, resolved by the registry rather than import order).

use serde::{Deserialize, Serialize};

/// One capability address: `<seam>/<key>` (e.g. `tool/http_get`,
/// `hook/before_tool_call`, `prompt.section/workspace_context`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Claim {
    /// Typed seam family (`tool`, `hook`, `memory.provider`, `gateway.command`,
    /// `channel.adapter`, `prompt.section`). Seam names are lowercase kebab/snake.
    pub seam: String,
    /// Key within the seam (tool name, hook event, section id, …).
    pub key: String,
}

impl Claim {
    pub fn new(seam: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            seam: seam.into(),
            key: key.into(),
        }
    }

    /// Convenience constructor for tool-seam claims.
    pub fn tool(key: impl Into<String>) -> Self {
        Self::new("tool", key)
    }
}

impl std::fmt::Display for Claim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.seam, self.key)
    }
}

//! PII redaction — SHA-256 hashes user/chat IDs for system prompts.

use sha2::{Digest, Sha256};

// (iter-148: hex_encode deleted — use hex::encode instead, already in deps)

/// Redact a user ID to a privacy-safe hash prefix.
pub fn redact_id(raw_id: &str) -> String {
    let hash = Sha256::digest(raw_id.as_bytes());
    format!("user_{}", &hex::encode(hash)[..12])
}

/// Redact a chat/channel ID to a privacy-safe hash prefix.
pub fn redact_chat_id(raw_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"chat:");
    hasher.update(raw_id.as_bytes());
    let hash = hasher.finalize();
    format!("chat_{}", &hex::encode(hash)[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_id_deterministic() {
        let a = redact_id("12345");
        let b = redact_id("12345");
        assert_eq!(a, b);
        assert!(a.starts_with("user_"));
        assert_ne!(redact_id("12345"), redact_id("67890"));
    }

    #[test]
    fn test_redact_chat_id() {
        let a = redact_chat_id("group_abc");
        assert!(a.starts_with("chat_"));
    }
}

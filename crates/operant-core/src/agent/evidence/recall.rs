//! Naming the full text that a lossy method dropped.
//!
//! A lossy method may only run when the full text can be recovered, and the
//! compressed result must say where the full text went. **The session store
//! is the owner of every full tool output** — the row the tool result was
//! written to — so the marker names that, and the `evidence_recall` tool
//! reads it back from there.
//!
//! Why the session store and not a second store: a second copy of the same
//! fact has two owners, and the two copies drift. The session store was
//! already the place a tool result lives, it already survives a restart, and
//! it is already what the aggregate trim's own marker points at ("the full
//! output is still in the session store"). One owner, one pointer, one
//! retrieval path.
//!
//! That is also why the chokepoint writes the FULL text to the session store
//! and gives the model the shaped copy: a marker that says "the full output
//! is retrievable" is only true if the store still holds it.

/// The marker printed with a compressed result.
///
/// It names the methods that ran, the size before and after, and where the
/// full text is — the model must never have to guess that something was
/// removed.
pub fn marker(key: &str, before: usize, after: usize, applied: &[&str]) -> String {
    let methods = if applied.is_empty() {
        "none".to_string()
    } else {
        applied.join(", ")
    };
    format!(
        "\n[compressed: {methods} — {before} → {after} tokens. Full output: `evidence_recall \"{key}\"`]"
    )
}

/// The key a result's full text is stored under.
///
/// The key names the session AND the tool call, because the session store
/// holds many tool results and the tool call is what pairs a result with the
/// call that produced it.
pub fn key_for(session_id: Option<&str>, tool_call_id: &str) -> String {
    match session_id {
        Some(session) if !session.is_empty() => format!("{session}:{tool_call_id}"),
        _ => tool_call_id.to_string(),
    }
}

/// Split a key back into the session id and the tool call id.
///
/// The split is on the LAST colon: a tool call id never contains one, and a
/// session id may. A key with no colon has no session, and the tool call id
/// alone is then the best we have.
pub fn split_key(key: &str) -> Option<(&str, &str)> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    match key.rsplit_once(':') {
        Some((session, tool_call_id)) if !session.is_empty() && !tool_call_id.is_empty() => {
            Some((session, tool_call_id))
        }
        _ => Some(("", key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_names_the_methods_the_sizes_and_the_key() {
        let marker = marker("sess:call_1", 12431, 3204, &["toon", "cap"]);
        assert!(marker.contains("toon, cap"), "got {marker}");
        assert!(marker.contains("12431"), "got {marker}");
        assert!(marker.contains("3204"), "got {marker}");
        assert!(marker.contains("evidence_recall"), "got {marker}");
        assert!(marker.contains("sess:call_1"), "got {marker}");
    }

    #[test]
    fn a_key_carries_the_session_and_the_tool_call() {
        assert_eq!(key_for(Some("sess-1"), "call_9"), "sess-1:call_9");
        // No session: the tool call alone is the key.
        assert_eq!(key_for(None, "call_9"), "call_9");
        assert_eq!(key_for(Some(""), "call_9"), "call_9");
    }

    #[test]
    fn a_key_splits_back_into_what_it_was_built_from() {
        assert_eq!(split_key("sess-1:call_9"), Some(("sess-1", "call_9")));
        // A session id with a colon in it: the LAST colon is the separator.
        assert_eq!(split_key("a:b:call_9"), Some(("a:b", "call_9")));
        // No session: the whole key is the tool call.
        assert_eq!(split_key("call_9"), Some(("", "call_9")));
        assert_eq!(split_key("   "), None);
    }

    #[test]
    fn a_key_round_trips() {
        let key = key_for(Some("sess-1"), "call_9");
        let (session, tool_call) = split_key(&key).expect("splits");
        assert_eq!(session, "sess-1");
        assert_eq!(tool_call, "call_9");
    }
}

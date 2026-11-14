//! The large-paste store: one owner for collapse, expand and drop.
//!
//! A paste big enough to swamp the composer is shown as a short placeholder
//! and the real text is held here, so the composer stays readable and the
//! model still receives the paste. Three things have to stay true at once,
//! and the split this replaced kept only the first:
//!
//! 1. **The placeholder expands on submit.** The model must never see
//!    `[Pasted ~N lines #K]` (2026-10-13).
//! 2. **The label is derived from the CONTENT, and the numbering is derived
//!    from the placeholders still LIVE** (codex `next_large_paste_placeholder`,
//!    `chat_composer.rs:2001`). A counter that only ever increments is two
//!    defects at once: deleting a placeholder strands its number, so the next
//!    paste of the same size reads `#7` with nothing before it, and because the
//!    label is the ONLY thing that identifies the payload, the orphan can never
//!    be expanded again (RT-559).
//! 3. **The payload dies with the placeholder.** Codex's placeholder is an
//!    atomic `TextElement` and `reconcile_deleted_elements` drops the payload
//!    when the element leaves the buffer. This store has no element ranges, so
//!    the same invariant is enforced where every mutation already converges:
//!    [`PromptInputState::drop_missing_paste_placeholders`], called from the
//!    composer's one refresh choke point.
//!
//! The store is keyed by the placeholder string rather than an id, so a label
//! can be recomputed from the live set and the payload found from the text.

/// What a paste put in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteOutcome {
    /// Small enough to insert verbatim.
    Inline,
    /// Held as a placeholder. `content` is the real text the placeholder
    /// stands for, and the caller needs it for exactly one thing: the
    /// collapse notice names the size of what was swallowed.
    Collapsed {
        placeholder: String,
        content: String,
    },
}

impl PasteOutcome {
    /// Collapse `content` to its placeholder, numbering it against the
    /// placeholders already held in `store`.
    pub fn of(content: &str, store: &[(String, String)]) -> Self {
        if !is_large_paste(content) {
            return PasteOutcome::Inline;
        }
        PasteOutcome::Collapsed {
            placeholder: paste_placeholder(content, store),
            content: content.to_string(),
        }
    }
}

/// Is this paste big enough to be worth collapsing?
///
/// Three lines or 150 characters: the composer is a prompt, not a document
/// viewer, and a paste that pushes the operator's own text off screen has
/// stopped being editable.
pub fn is_large_paste(content: &str) -> bool {
    content.lines().count() >= 3 || content.len() > 150
}

/// The placeholder standing in for `content` among the placeholders already
/// in `store`.
///
/// The base label is the CONTENT's line count (`[Pasted ~N lines]`), and the
/// suffix counts the live placeholders of that shape — so the first paste of a
/// given shape is bare, the second is `#2`, and deleting both reclaims the
/// bare label for the next one.
pub fn paste_placeholder(content: &str, store: &[(String, String)]) -> String {
    let base = format!("[Pasted ~{} lines]", content.lines().count());
    let prefix = format!("{base} #");
    let mut max_suffix = 0usize;
    for (placeholder, _) in store {
        if placeholder == &base {
            max_suffix = max_suffix.max(1);
            continue;
        }
        if let Some(suffix) = placeholder.strip_prefix(&prefix)
            && let Ok(value) = suffix.parse::<usize>()
        {
            max_suffix = max_suffix.max(value);
        }
    }
    if max_suffix == 0 {
        base
    } else {
        format!("{base} #{}", max_suffix + 1)
    }
}

/// Is `placeholder` present in `text` as a placeholder of its own?
///
/// ONE boundary rule, used by the drop rule and the expander both. The bare
/// label (`[Pasted ~3 lines]`) is a PREFIX of the numbered one
/// (`[Pasted ~3 lines] #2`), so a plain `contains` would call the bare token
/// live for as long as the numbered one existed; the character that follows
/// the candidate decides which one it is.
pub fn is_live_placeholder(text: &str, placeholder: &str) -> bool {
    if placeholder.is_empty() {
        return false;
    }
    text.match_indices(placeholder).any(|(at, _)| {
        match text[at + placeholder.len()..].chars().next() {
            // ` #2` (or a digit with no separator) belongs to the numbered
            // placeholder, which is a longer label than this one.
            Some('#') => false,
            Some(c) if c.is_ascii_digit() => false,
            _ => true,
        }
    })
}

/// Which placeholder a re-paste of `content` should expand.
///
/// The store pairs each placeholder with the content it holds, so the pair is
/// the source of truth: the placeholder that was STORED WITH this content is
/// the one the re-paste refers to, not the newest placeholder of the same
/// shape. (jcode gets away with shape arithmetic because its store is keyed
/// by id; this store is append-ordered pairs.)
///
/// Returns `None` when the content is not in the store at all — nothing to
/// expand, so the caller collapses it afresh instead of resurrecting a
/// placeholder the operator already deleted.
///
/// The `rposition` picks the LAST entry for this content: an operator who
/// collapsed the same text twice is standing next to the newest placeholder,
/// so the expand belongs to that one.
pub fn expand_target(content: &str, store: &[(String, String)]) -> Option<String> {
    store
        .iter()
        .rev()
        .find(|(_, held)| held == content)
        .map(|(placeholder, _)| placeholder.clone())
}

/// Swap every placeholder in `text` back for its stored content. The
/// placeholder keeps the composer readable; the model must get the paste.
pub fn expand_paste_placeholders(text: &str, store: &[(String, String)]) -> String {
    let mut labels: Vec<&(String, String)> = store
        .iter()
        .filter(|(placeholder, content)| {
            placeholder != content && is_live_placeholder(text, placeholder)
        })
        .collect();
    // Longest label first: the numbered token must be substituted before the
    // bare one it contains, or the replacement rewrites the numbered token's
    // head and leaves a doubled tail (`a\nb\nc y a\nb\nc #2`).
    labels.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    let mut out = text.to_string();
    for (placeholder, content) in labels {
        out = out.replace(placeholder.as_str(), content.as_str());
    }
    out
}

/// Normalize a pasted string into a filesystem path if it looks like one.
///
/// Handles:
/// - `file:///path/to/file` — URL-encoded paths
/// - `"C:\path"` / `'/path'` — quoted paths (strips quotes)
/// - Bare absolute paths (`/home/...`, `C:\...`)
///
/// Returns `None` if the text is multiline, not path-shaped, or the resolved
/// path does not exist on the filesystem.  Callers can use the returned
/// `PathBuf` to decide whether to treat the paste as a file attachment.
pub fn detect_pasted_path(text: &str) -> Option<std::path::PathBuf> {
    let trimmed = text.trim();
    // Multiline content is never a bare path.
    if trimmed.contains('\n') {
        return None;
    }
    // Strip outer matching quotes.
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed);

    // file:// URL — strip the scheme (skip the leading //).
    let candidate = if let Some(rest) = unquoted.strip_prefix("file://") {
        rest
    } else {
        unquoted
    };

    let path = std::path::Path::new(candidate);
    if path.is_absolute() && path.exists() {
        Some(path.to_path_buf())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(p, c)| ((*p).to_string(), (*c).to_string()))
            .collect()
    }

    #[test]
    fn the_first_paste_of_a_shape_is_bare_and_the_second_is_numbered() {
        let content = "a\nb\nc";
        let empty: Vec<(String, String)> = Vec::new();
        let first = paste_placeholder(content, &empty);
        assert_eq!(first, "[Pasted ~3 lines]");

        let held = store(&[(&first, content)]);
        assert_eq!(
            paste_placeholder(content, &held),
            "[Pasted ~3 lines] #2",
            "a second same-shaped paste is numbered from the LIVE set"
        );
    }

    #[test]
    fn deleting_a_placeholder_reclaims_its_number() {
        let content = "a\nb\nc";
        // The bare label and #2 are live; removing both means the next paste
        // of this shape is bare again rather than #3 with nothing before it.
        let live = store(&[
            ("[Pasted ~3 lines]", content),
            ("[Pasted ~3 lines] #2", content),
        ]);
        assert_eq!(paste_placeholder(content, &live), "[Pasted ~3 lines] #3");
        assert_eq!(
            paste_placeholder(content, &[]),
            "[Pasted ~3 lines]",
            "an empty store hands back the bare label"
        );
    }

    #[test]
    fn the_expand_target_is_the_placeholder_belonging_to_that_content() {
        let older = "1\n2\n3\n4\n5";
        let newer = "aaaa\nbbbb\ncccc\ndddd\neeee";
        // Same shape (5 lines), different text: the label cannot tell them
        // apart, the CONTENT can.
        let held = store(&[
            ("[Pasted ~5 lines]", older),
            ("[Pasted ~5 lines] #2", newer),
        ]);
        assert_eq!(
            expand_target(older, &held).as_deref(),
            Some("[Pasted ~5 lines]"),
            "the OLDER paste expands its own placeholder, not the newest"
        );
        assert_eq!(
            expand_target(newer, &held).as_deref(),
            Some("[Pasted ~5 lines] #2")
        );
    }

    #[test]
    fn content_that_is_not_held_has_nothing_to_expand() {
        let held = store(&[("[Pasted ~5 lines]", "1\n2\n3\n4\n5")]);
        assert_eq!(expand_target("zzz\nyyy\nxxx", &held), None);
    }

    #[test]
    fn a_deleted_placeholder_is_not_resurrected_by_a_later_paste() {
        // The operator pasted, deleted the placeholder, then pastes the same
        // text again: there is nothing to expand, so the caller collapses it
        // afresh instead of swapping in a placeholder that is no longer there.
        let content = "1\n2\n3\n4\n5";
        assert_eq!(expand_target(content, &[]), None);
    }

    #[test]
    fn a_small_paste_is_not_collapsed() {
        assert!(!is_large_paste("hello\nworld"));
        assert!(!is_large_paste("short"));
        assert!(is_large_paste("a\nb\nc"), "three lines is the threshold");
        assert!(is_large_paste(&"x".repeat(151)));
    }

    #[test]
    fn the_store_expands_every_placeholder_it_holds() {
        let held = store(&[
            ("[Pasted ~3 lines]", "a\nb\nc"),
            ("[Pasted ~3 lines] #2", "d\ne\nf"),
        ]);
        assert_eq!(
            expand_paste_placeholders("x [Pasted ~3 lines] y [Pasted ~3 lines] #2 z", &held),
            "x a\nb\nc y d\ne\nf z"
        );
    }

    #[test]
    fn the_placeholder_is_resolved_by_content_not_by_newest() {
        // Each entry holds DIFFERENT content, so the placeholder is resolved
        // by content, not by which one was pasted last.
        let held = store(&[
            ("[Pasted ~3 lines]", "a\nb\nc"),
            ("[Pasted ~3 lines] #2", "d\ne\nf"),
        ]);
        assert_eq!(
            expand_target("a\nb\nc", &held).as_deref(),
            Some("[Pasted ~3 lines]")
        );
        assert_eq!(
            expand_target("d\ne\nf", &held).as_deref(),
            Some("[Pasted ~3 lines] #2")
        );
        assert_eq!(expand_target("nope", &held), None);
    }

    #[test]
    fn a_path_that_exists_is_detected() {
        let Some(dir) = std::env::temp_dir().to_str().map(str::to_string) else {
            return;
        };
        let path = format!("{dir}/operant-paste-detect-probe");
        std::fs::write(&path, b"x").ok();
        assert_eq!(
            detect_pasted_path(&path),
            Some(std::path::PathBuf::from(&path)),
            "an existing absolute path is a file reference"
        );
        let _ = std::fs::remove_file(&path);
        assert_eq!(detect_pasted_path(&format!("{path}.missing")), None);
        assert_eq!(
            detect_pasted_path("one\ntwo"),
            None,
            "multiline text is never a path"
        );
    }
}

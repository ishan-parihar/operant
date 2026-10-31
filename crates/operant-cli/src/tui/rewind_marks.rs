// rewind_marks.rs — the rewind selector's live mark source (iter-1489).
//
// WHY THIS MODULE EXISTS
//
// The selector is a fullscreen overlay. Every key it consumes (arrows, `j`/`k`,
// `f`, Enter, Esc) is handled inside `App::run`'s inner loop, and
// `TuiApp::run`'s outer iteration does not resume until that inner loop
// returns — which happens only when the user submits a prompt or quits. So the
// selector cannot wait on `run_pending_agent_actions`: that async drain lives
// on the OUTER side of the overlay's whole lifetime and would never fire while
// the overlay is open. The observable consequence was a selector that opened
// empty ("No rewindable turns yet") with every key reading as dead, no matter
// how many turns the session had actually run.
//
// The fix is to hand the App the mark data directly, so the overlay can sync it
// on every frame. `OperantAgent::user_turns()` is a synchronous `RwLock` read
// returning a clone, so no await and no runtime handle is needed inside the
// inner loop — the App just needs an `Arc` to a clonable source.
//
// WHY IT IS A SEPARATE HANDLE AND NOT `Arc<OperantAgent>`
//
// The App deliberately does not hold the agent: the agent is created,
// retargeted (`set_session_id` on /resume, /new, the branch browser) and
// owned by `TuiApp::run`. A bare `Arc<OperantAgent>` would invite the overlay
// to call mutating agent methods (fork, retarget, compact) from a render-path
// frame, which is exactly the layering violation the `pending_*` pattern
// exists to prevent. This source is READ-ONLY by construction: it exposes the
// marks and nothing else, so the overlay physically cannot reach a mutating
// method through it.
//
// SESSION FILTERING: the agent's `user_turns` list is process-global and only
// ever grows. A host that served several sessions (the gateway's shared agent,
// or the TUI after /resume) must not offer turns that belong to a different
// conversation — RT-175's cross-session-rewind defect. The source therefore
// also carries the loaded session id and the conversation's user prompts, and
// `marks()` filters marks to those whose prompt text still appears in the
// current conversation, the same relocate-by-text rule
// `OperantAgent::conversation_rewind_marks` applies (which cannot be used here
// because it is `async`).

use operant_core::agent::UserTurnMark;

/// A read-only, clonable snapshot source for the rewind selector.
///
/// Constructed by `OperantAgent::rewind_marks_source()`; the App holds it in
/// `core_rewind_marks` and the selector reads it every frame.
pub struct RewindMarksSource {
    /// The agent's process-global mark list. Cloned wholesale rather than
    /// watched: the list is small (one entry per user prompt in this process)
    /// and the overlay reads it at most once per frame.
    marks: std::sync::Arc<std::sync::RwLock<Vec<UserTurnMark>>>,
    /// The session the agent is currently pointed at. `None` before the first
    /// turn (the mints-on-first-turn path), in which case no filtering is
    /// possible and the unfiltered list is offered.
    session: std::sync::Arc<std::sync::RwLock<Option<String>>>,
    /// The current conversation's non-empty user prompts, in order. Used as
    /// the relocation set: a mark whose text is absent here belongs to a
    /// different conversation (or was folded away by compaction) and must not
    /// be offered as a rewind target.
    conversation: std::sync::Arc<std::sync::RwLock<Vec<String>>>,
}

/// A poisoned `RwLock` means a writer panicked mid-update, which is a
/// programmer error rather than a runtime condition; the read recovers with
/// the pre-panic contents so the overlay still renders something instead of
/// taking the whole frame loop down.
fn read_recover<T>(lock: &std::sync::RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl RewindMarksSource {
    /// Assemble the source from the agent's live handles. Called once per
    /// process at agent creation (`TuiApp::enter`).
    pub fn new(
        marks: Vec<UserTurnMark>,
        session: Option<String>,
        conversation: Vec<String>,
    ) -> Self {
        Self {
            marks: std::sync::Arc::new(std::sync::RwLock::new(marks)),
            session: std::sync::Arc::new(std::sync::RwLock::new(session)),
            conversation: std::sync::Arc::new(std::sync::RwLock::new(conversation)),
        }
    }

    /// Replace the mirrored mark list. The agent's own list is the authority;
    /// this exists so a host that reseeds it (session load, compaction) keeps
    /// the source in step without rebuilding the handle.
    pub fn set_marks(&self, marks: Vec<UserTurnMark>) {
        let mut guard = match self.marks.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = marks;
    }

    /// Point the source at a different loaded session.
    pub fn set_session(&self, session: Option<String>) {
        let mut guard = match self.session.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = session;
    }

    /// Replace the relocation set (the loaded conversation's user prompts).
    pub fn set_conversation(&self, prompts: Vec<String>) {
        let mut guard = match self.conversation.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = prompts;
    }

    /// The marks the selector may offer, oldest first.
    ///
    /// Offers nothing when there is neither a loaded session nor a live
    /// conversation — there is then nothing to rewind within, and offering the
    /// process-global list would name turns of a session the TUI is not on
    /// (RT-175). Marks whose prompt text is absent from the live conversation
    /// are dropped: they belong to another conversation or were folded away by
    /// compaction.
    pub fn marks(&self) -> Vec<UserTurnMark> {
        let session = read_recover(&self.session).clone();
        let conversation = read_recover(&self.conversation).clone();
        let marks = read_recover(&self.marks).clone();
        // `session` is `None` only before the first turn mints an id, and the
        // drain refreshes it afterwards. A live conversation with no recorded
        // session id means the host never told us — the display is the
        // authority on what is loaded, so trust it rather than offering nothing.
        if session.is_none() && conversation.is_empty() {
            return Vec::new();
        }
        // Filter before deduplicating: `conversation_rewind_marks` keeps a
        // mark whose text appears anywhere in the loaded conversation, and
        // two marks sharing a text (the same prompt resent) are both
        // rewindable targets, so no dedup.
        marks
            .into_iter()
            .filter(|m| {
                m.text.trim().is_empty()
                    || conversation.iter().any(|t| prompt_texts_match(t, &m.text))
            })
            .collect()
    }
}

/// Whether a stored prompt and a recorded mark are the same user prompt.
///
/// The mark records what the model was given; the conversation stores what was
/// committed. Those differ only in incidental whitespace (and, for a
/// continuation, the harness's own prefixes, which never reach a mark), so a
/// trimmed comparison is the rule. Shared with the agent's own relocation
/// logic so both sides relocate a stale mark identically.
pub fn prompt_texts_match(stored: &str, marked: &str) -> bool {
    stored.trim() == marked.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(turn_id: u64, text: &str) -> UserTurnMark {
        UserTurnMark {
            turn_id,
            conv_index: 0,
            conv_epoch: 0,
            text: text.to_string(),
        }
    }

    fn source(session: Option<&str>, convo: &[&str]) -> RewindMarksSource {
        RewindMarksSource::new(
            vec![mark(1, "alpha"), mark(2, "beta"), mark(3, "gamma")],
            session.map(str::to_string),
            convo.iter().map(|s| (*s).to_string()).collect(),
        )
    }

    /// The regression the whole module exists for: with a loaded session, the
    /// selector gets marks at all. Reverting the handle wiring (or leaving
    /// `marks()` unfiltered-and-empty) turns this red.
    #[test]
    /// (iter-1544) THE OPERATOR'S LIVE DEFECT, pinned.
    ///
    /// A real TUI run sent three prompts. The agent recorded three turn marks,
    /// but the model merged two of them into ONE committed user message with no
    /// separator (`"REWINDPROBE bravo promptREWINDPROBE charlie prompt"`), so
    /// the conversation carried two prompts where the marks carried three.
    /// Exact-equality filtering dropped the two marks that had been merged, the
    /// rewind selector listed ONE entry, and the arrow keys had nothing to move
    /// through — the operator's "up/down navigation is not working".
    ///
    /// This test pins the DEFECT's mechanism, not a fix. It documents that the
    /// exact-match rule loses a mark whose prompt was merged into another
    /// message, which is the state that must change.
    #[test]
    fn a_mark_whose_prompt_was_merged_into_another_message_is_lost() {
        let src = RewindMarksSource::new(
            vec![
                mark(11, "REWINDPROBE alpha prompt"),
                mark(12, "REWINDPROBE bravo prompt"),
                mark(13, "REWINDPROBE charlie prompt"),
            ],
            Some("sess_live".to_string()),
            vec![
                "REWINDPROBE alpha prompt".to_string(),
                "REWINDPROBE bravo promptREWINDPROBE charlie prompt".to_string(),
            ],
        );
        let texts: Vec<String> = src.marks().into_iter().map(|m| m.text).collect();
        assert_eq!(
            texts,
            vec!["REWINDPROBE alpha prompt".to_string()],
            "the two merged prompts are dropped by exact equality — this is the \
             live defect, and the assertion is the record of it"
        );
    }

    fn marks_are_offered_for_the_loaded_session() {
        let src = source(Some("sess_a"), &["alpha", "beta", "gamma"]);
        let ids: Vec<u64> = src.marks().into_iter().map(|m| m.turn_id).collect();
        assert_eq!(ids, vec![1, 2, 3], "every mark of THIS conversation");
    }

    /// RT-175: a mark from another conversation must not be offered.
    #[test]
    fn marks_from_another_conversation_are_filtered_out() {
        let src = source(Some("sess_b"), &["beta"]);
        let texts: Vec<String> = src.marks().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["beta".to_string()]);
    }

    /// Before the first turn there is no session AND no conversation, and
    /// offering the process-global list would name a stranger's turns.
    #[test]
    fn no_session_and_no_conversation_offers_nothing() {
        let src = source(None, &[]);
        assert!(src.marks().is_empty(), "no session → no rewind targets");
    }

    /// A live conversation with no recorded session id still offers the marks
    /// that belong to it: the display is the authority on what is loaded, and
    /// refusing here would reintroduce the empty-selector defect.
    #[test]
    fn a_live_conversation_offers_its_marks_even_without_a_session_id() {
        let src = source(None, &["alpha", "beta"]);
        let ids: Vec<u64> = src.marks().into_iter().map(|m| m.turn_id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    /// Whitespace-only differences are the same prompt; a genuine text change
    /// is not.
    #[test]
    fn relocation_tolerates_whitespace_only() {
        assert!(prompt_texts_match("  alpha  ", "alpha"));
        assert!(!prompt_texts_match("alpha", "beta"));
    }

    /// An empty mark (a harness-injected message that got recorded) can never
    /// be relocated by text, so it is kept rather than silently dropped —
    /// the selector's `compacted` flag is what reports it to the user.
    #[test]
    fn empty_mark_texts_are_kept_for_the_selector_to_flag() {
        let src = source(Some("sess_c"), &["alpha"]);
        src.set_marks(vec![mark(9, ""), mark(10, "alpha")]);
        assert_eq!(src.marks().len(), 2, "the empty mark survives filtering");
    }

    /// The source is a live mirror, not a snapshot: reseeding it (a session
    /// switch) is what keeps the overlay honest without a new handle.
    #[test]
    fn reseeding_moves_the_source_to_the_new_session() {
        let src = source(Some("sess_a"), &["alpha"]);
        src.set_session(Some("sess_b".to_string()));
        src.set_conversation(vec!["gamma".to_string()]);
        let texts: Vec<String> = src.marks().into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["gamma".to_string()]);
    }
}

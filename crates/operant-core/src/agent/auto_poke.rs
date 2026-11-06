//! The bounded synthetic continuation — the "poke".
//!
//! Ported from jcode's auto-poke: the message builder in
//! `crates/jcode-todo-policy/src/lib.rs` (`build_auto_poke_message`) and the
//! non-retryable classifier in
//! `crates/jcode-tui/src/tui/app/commands_auto_poke_errors.rs`
//! (`is_non_retryable_auto_poke_error`).
//!
//! WHAT IT IS: a turn that runs out of room ends with a TOOLLESS wrap-up.
//! A wrap-up can summarise what happened, but it cannot finish the work that
//! is still open — and when it comes back empty or degenerate the operator
//! used to get nothing at all: an empty answer and a notice saying the
//! wrap-up "ran out of room mid-summary". The poke is the bounded
//! continuation that gives the turn one more pass WITH its tools.
//!
//! WHAT IT IS NOT: jcode's poke is a TODO-policy feature that fires after a
//! NORMAL completion whose todo list is still open, and its producer returns
//! None when nothing is actionable ("completing the loop with a generic
//! summary just spends tokens on 'all good' theater"). Operant ports the
//! MECHANISM and fires it where the operator actually loses an answer: the
//! early exit whose wrap-up produced nothing.
//!
//! The three guards that keep it from becoming a spam loop:
//! - `MAX_AUTO_POKES` pokes per user turn, each granting
//!   `AUTO_POKE_ITERATIONS` iterations that run PAST the configured budget;
//! - no poke unless there is work to continue (incomplete todos, or a turn
//!   that ran tools);
//! - no poke after a non-retryable provider error (see
//!   [`is_non_retryable_auto_poke_error`]), because re-issuing the same
//!   request cannot help and burns refused calls.

/// How many pokes one user turn may issue.
///
/// Two is enough for "the wrap-up was empty, try once more" while making an
/// infinite poke loop structurally impossible.
pub(crate) const MAX_AUTO_POKES: usize = 2;

/// How many iterations each poke phase may run.
///
/// The poke bypasses the configured iteration budget for exactly this many
/// requests, then the limits apply again.
pub(crate) const AUTO_POKE_ITERATIONS: usize = 3;

/// The synthetic continuation message, agreed on once so every producer and
/// every recogniser ([`is_auto_poke_message`]) see the same text.
pub(crate) fn auto_poke_message(unfinished_todos: usize) -> String {
    if unfinished_todos == 0 {
        // Nothing is tracked, so the nudge can only ask for the work the
        // turn was doing: a continuation that also permits a clean finish,
        // so a model that is genuinely done does not get nagged into
        // re-doing work it completed.
        return "Continue the task from where you stopped and finish it. If it \
                is already complete, summarize what was done."
            .to_string();
    }
    format!(
        "You have {unfinished_todos} incomplete todo{}. Continue working, or \
         update the todo tool.",
        if unfinished_todos == 1 { "" } else { "s" },
    )
}

/// Whether `message` is a poke this harness wrote.
///
/// The poke is persisted as a user row — the transcript records what the
/// model was actually told — so a renderer needs a way to tell "the operator
/// typed this" from "the harness synthesised this". jcode keeps the same
/// predicate for the identical reason (its live UI hides the poke behind an
/// "Auto-poking…" notice instead of rendering it as a user prompt).
pub fn is_auto_poke_message(message: &str) -> bool {
    let trimmed = message.trim();
    (trimmed.contains(" incomplete todo") && trimmed.ends_with("update the todo tool."))
        || trimmed.starts_with("Continue the task from where you stopped")
}

/// The per-turn poke bookkeeping.
#[derive(Debug, Default, Clone)]
pub(crate) struct PokeState {
    /// How many pokes this turn has issued.
    pub used: usize,
    /// How many more iterations the current poke phase may run.
    pub iterations_left: usize,
    /// The turn's last provider error, for [`is_non_retryable_auto_poke_error`].
    ///
    /// A turn that ended with a deterministic rejection (context length,
    /// invalid model, quota) would have its poke rejected the same way, so
    /// the poke is not issued at all. A transient failure stays recorded
    /// but does not block one.
    pub last_error: Option<String>,
}

impl PokeState {
    /// Whether the poke phase is currently running the turn past its limits.
    pub fn poking(&self) -> bool {
        self.iterations_left > 0
    }

    /// Whether the turn's last provider error makes a poke pointless.
    pub fn blocked_by_non_retryable(&self) -> bool {
        self.last_error
            .as_deref()
            .is_some_and(is_non_retryable_auto_poke_error)
    }

    /// Record the turn's last provider error.
    pub fn record_error(&mut self, error: &str) {
        self.last_error = Some(error.to_string());
    }
}

/// A deterministic provider failure that a second identical request cannot
/// fix. Ported verbatim in intent from jcode's
/// `is_non_retryable_auto_poke_error`: retrying these "creates an infinite
/// spam loop", so the poke is not issued at all.
pub fn is_non_retryable_auto_poke_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    const DETERMINISTIC_MARKERS: &[&str] = &[
        "400 bad request",
        "invalid_request_error",
        "string_above_max_length",
        "string_too_long",
        "maximum length",
        "request too large",
        "payload too large",
        "body too large",
        "input too large",
        "context length exceeded",
        "context_length_exceeded",
        "maximum context length",
        "token limit exceeded",
        "invalid model",
        "model_not_found",
        "model_not_supported",
        "unsupported model",
        "unsupported parameter",
        "unsupported_value",
        "invalid parameter",
        "invalid schema",
        "invalid tool",
        "invalid image",
        "image too large",
        "unsupported image",
        "unsupported file",
        "file too large",
        "content_policy_violation",
        "safety_violation",
        "permission_denied",
        "unauthorized",
        "401 unauthorized",
        "403 forbidden",
        "insufficient_quota",
        "402 payment required",
        "payment required",
        "requires more credits",
        "add more credits",
        "credit balance",
        "out of credits",
        "billing",
        "usage_limit_reached",
        "usage limit has been reached",
        "usage limit reached",
        "quota exceeded",
        "quota_exceeded",
    ];
    DETERMINISTIC_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_poke_message_names_the_open_todos() {
        let one = auto_poke_message(1);
        assert!(
            one.contains("1 incomplete todo. Continue working"),
            "one todo is singular: {one}"
        );
        let many = auto_poke_message(3);
        assert!(
            many.contains("3 incomplete todos. Continue working"),
            "several todos are plural: {many}"
        );
    }

    #[test]
    fn a_poke_without_todos_asks_for_the_work_not_the_list() {
        let text = auto_poke_message(0);
        assert!(
            !text.contains("todo"),
            "a model that never planned must not be nagged about a list: {text}"
        );
        assert!(
            text.contains("Continue the task"),
            "it must still ask for the work: {text}"
        );
    }

    #[test]
    fn a_written_poke_is_recognised_as_one() {
        assert!(is_auto_poke_message(&auto_poke_message(4)));
        assert!(is_auto_poke_message(&auto_poke_message(0)));
        assert!(
            !is_auto_poke_message("summarise the repo for me"),
            "an operator's own words must never look synthetic"
        );
    }

    #[test]
    fn non_retryable_errors_are_deterministic_failures_not_flakes() {
        for yes in [
            "HTTP 400 - context length exceeded",
            "invalid model: gpt-5-nope",
            "insufficient_quota",
            "usage limit reached; resets in 4h",
            "content_policy_violation",
            "401 unauthorized",
        ] {
            assert!(
                is_non_retryable_auto_poke_error(yes),
                "a second identical request cannot fix: {yes}"
            );
        }
        for no in [
            "error sending request for url",
            "HTTP 500 - internal server error",
            "connection reset by peer",
            "HTTP 429 - rate limited",
        ] {
            assert!(
                !is_non_retryable_auto_poke_error(no),
                "a transient failure must stay pokeable: {no}"
            );
        }
    }
}

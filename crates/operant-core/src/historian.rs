//! The HISTORIAN: durable compartments for CLOSED spans of a conversation.
//!
//! # Why this exists
//!
//! `context_management` folds history with a marker string in the message list
//! (`MICRO_MARKER_PREFIX`). That works, and it is cheap, but the summary then
//! lives IN the transcript: it is re-sent on every later turn, it cannot be
//! re-rendered at another fidelity (the marker replaced the original), and
//! nothing can tell whether two folds covered the same span.
//!
//! A compartment is the same summary stored OUT of band — in the `compartments`
//! table — so the request references it instead of carrying it. That buys four
//! things: the fold is paid for once, an old compartment can be re-rendered at a
//! lower tier when the window is tight, a refold is idempotent, and "what did
//! this summarize" has an answer.
//!
//! # The rule this module exists to enforce
//!
//! **The host decides the span. The model never does.**
//!
//! The next span is derived from durable state (`last_compartment_span_end + 1`)
//! and the model's answer is VALIDATED against it. A historian that reports a
//! span it did not advance is a hard error, not a retry — the reference
//! implementation learned this the expensive way, with a recorded session that
//! spent an hour dropping one small tool result per pass against a 9,200-token
//! gap, because every pass looked like progress.
//!
//! # Everything here is pure
//!
//! No database, no model, no I/O. The caller resolves the anchor and writes the
//! row; this module decides WHICH messages may be folded and whether a returned
//! span is safe to store. That is what makes the guards testable.

use crate::client::Message;
use crate::context_management::tool_group_starts;

/// The system prompt for a fold.
///
/// Deliberately narrow: the model's whole job is to compress ONE closed span
/// into a short factual paragraph that preserves what a later reader needs —
/// decisions, constraints, open threads. It is not asked to evaluate, to advise,
/// or to continue anything, because a compartment is reference material, not
/// instruction.
pub fn historian_system_prompt() -> &'static str {
    "You compress one closed stretch of a coding session into a short factual \
     reference paragraph. Preserve: what was decided, what constraint was \
     established, what is still open. Drop: pleasantries, retries that reached \
     nothing, and anything already stated more precisely elsewhere. Write prose, \
     no headings, no bullets, at most 150 words. Do not advise, do not continue \
     the work, do not address the reader."
}

/// The user-role instruction that carries the span.
pub const HISTORIAN_INSTRUCTION: &str =
    "Compress the stretch above into one factual reference paragraph.";

/// Token ceiling on the rendered span one fold call may carry.
///
/// Smaller than the advisor's and recap's: a compartment is a fixed-length
/// output, so it does not need to read as much as an advisory review does, and
/// the span is bounded by `MAX_SPAN_MESSAGES` in messages before this is applied.
pub const HISTORIAN_VIEW_TOKEN_BUDGET: usize = 6_000;

/// The render tier a new compartment is stored at.
///
/// P2 — the middle of the four fidelity levels. Storing the middle is what makes
/// re-rendering possible in BOTH directions: a tight window can drop to P3/P4
/// without a model call, and a roomy one can keep P2 rather than having stored a
/// lossy P4 with no way back.
pub const DEFAULT_COMPARTMENT_TIER: i64 = 2;

/// The decay weight a new compartment starts at.
///
/// Compartments are reference material the operator may still want, so they
/// start above the middle of the 1..100 importance scale rather than at it.
pub const DEFAULT_COMPARTMENT_IMPORTANCE: i64 = 60;

/// Pull the compartment body out of a model reply.
///
/// The prompt asks for prose, but a model that answers with a heading, a fence or
/// a preamble is common enough that the parser must be tolerant rather than
/// strict: a fold that reached the model and came back in the wrong shape is
/// still a fold, and discarding it wastes the call.
///
/// Returns `None` when there is no usable text at all, which is the one case the
/// caller must treat as a failure.
pub fn parse_compartment(reply: &str) -> Option<String> {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Strip a wrapping code fence if the model added one.
    let body = match trimmed.find("```") {
        Some(start) => {
            let after = &trimmed[start + 3..];
            // Skip the language tag on the opening fence.
            let content_start = after.find('\n').map(|i| i + 1).unwrap_or(0);
            let content = &after[content_start..];
            match content.rfind("```") {
                Some(end) => content[..end].trim(),
                None => content.trim(),
            }
        }
        None => trimmed,
    };
    let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// A span of message positions proposed for folding, or a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanPlan {
    /// Fold `messages[start..=end]` (inclusive) into one compartment.
    Fold { start: usize, end: usize },
    /// Do not fold, and why.
    Skip(SkipReason),
}

/// Why a fold was refused.
///
/// Every variant is a distinct operator-visible answer, because "nothing to fold"
/// and "the fold would break a tool group" need different responses: one is
/// normal, the other is a policy limit being hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// Fewer messages than the minimum span.
    TooFewMessages,
    /// The next span is still open — its newest turn has no answer yet.
    SpanNotClosed,
    /// The messages in the span carry nothing worth summarizing.
    NothingToFold,
}

/// The skip reason of a plan, when it is one.
pub fn skip_reason(plan: &SpanPlan) -> Option<SkipReason> {
    match plan {
        SpanPlan::Skip(reason) => Some(reason.clone()),
        SpanPlan::Fold { .. } => None,
    }
}

impl SkipReason {
    /// A stable, log-friendly label.
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::TooFewMessages => "too-few-messages",
            SkipReason::SpanNotClosed => "span-not-closed",
            SkipReason::NothingToFold => "nothing-to-fold",
        }
    }
}

/// The smallest span the historian will fold.
///
/// Below this a compartment costs more to read than the messages it replaces:
/// the render adds a boundary the model has to parse, and the saving is a
/// handful of tokens. The reference implementation uses the same idea as a
/// floor on a fit chunk (1,000 tokens there); this is a message count because
/// the decision is made before any token estimate is worth taking.
pub const MIN_SPAN_MESSAGES: usize = 4;

/// The largest span folded in one pass.
///
/// A span that large is one model call's worth of input. The bound exists so a
/// backlog cannot become a single unbounded request; the caller folds the
/// backlog in successive passes instead.
pub const MAX_SPAN_MESSAGES: usize = 200;

/// Plan the next fold.
///
/// `messages` is the conversation oldest-first. `last_covered` is the position
/// the store says is already compartmented, or `None` when nothing is.
///
/// The returned span is aligned to TOOL GROUP boundaries: a `tool_use` is never
/// separated from its `tool_result`, because a lone half is a malformed history
/// the providers reject. The same invariant the eviction pass keeps, for the same
/// reason, computed with the same helper.
pub fn plan_span(messages: &[Message], last_covered: Option<usize>) -> SpanPlan {
    let start = match last_covered {
        Some(covered) => covered.saturating_add(1),
        None => 0,
    };
    let remaining = messages.len().saturating_sub(start);
    if remaining < MIN_SPAN_MESSAGES {
        return SpanPlan::Skip(SkipReason::TooFewMessages);
    }

    // The span must END on a group boundary. Walk forward from `start` to the
    // last position that is the START of a group (or the end of the list), so
    // the cut never lands mid-group.
    let starts = tool_group_starts(messages);
    let mut end = start;
    let mut i = start;
    while i < messages.len() {
        // `i` is a group start, so the group runs to the next start (exclusive).
        let mut group_end = i + 1;
        while group_end < messages.len() && starts[group_end] == i {
            group_end += 1;
        }
        // Stop if taking this group would exceed the span ceiling; a group
        // larger than the ceiling is taken whole anyway, because the alternative
        // (splitting it) produces a malformed history.
        if group_end - start > MAX_SPAN_MESSAGES && end > start {
            break;
        }
        end = group_end - 1; // inclusive
        i = group_end;
        if group_end - start >= MAX_SPAN_MESSAGES {
            break;
        }
    }

    if end < start {
        return SpanPlan::Skip(SkipReason::NothingToFold);
    }

    // The newest turn must be CLOSED before it is folded: a span whose last
    // message is an unanswered user turn or an unanswered tool call would
    // summarize a question nobody has answered yet.
    if !is_closed(messages, end) {
        // Try to back off to the last closed position rather than skipping
        // outright — the earlier turns are still foldable.
        let mut closed_end = None;
        let mut j = end;
        loop {
            if is_closed(messages, j) {
                closed_end = Some(j);
                break;
            }
            if j == start {
                break;
            }
            j -= 1;
        }
        match closed_end {
            Some(closed) if closed >= start => {
                return SpanPlan::Fold { start, end: closed };
            }
            _ => return SpanPlan::Skip(SkipReason::SpanNotClosed),
        }
    }

    SpanPlan::Fold { start, end }
}

/// Is the message at `index` a safe END for a fold?
///
/// A fold end is closed when it is not an unanswered question and not an
/// unanswered call: the last message may not be a user turn (nothing has
/// answered it) and may not be a `tool_use` whose `tool_result` has not arrived
/// (the pair would be split, or the call would be summarized as if it had
/// returned).
fn is_closed(messages: &[Message], index: usize) -> bool {
    let Some(msg) = messages.get(index) else {
        return false;
    };
    if msg.role == crate::client::Role::User {
        return false;
    }
    if crate::context_management::is_tool_use(msg) {
        // A tool_use is closed only when its results follow it.
        return messages
            .get(index + 1)
            .is_some_and(|next| next.role == crate::client::Role::Tool);
    }
    true
}

/// The outcome of validating a model's reported span against the one that was
/// requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanVerdict {
    /// The model advanced the span as asked (or further, within the same
    /// backlog). Safe to store.
    Accepted { start: usize, end: usize },
    /// The model reported a span that does not advance past what is already
    /// covered. A hard error: accepting it would make the next pass re-fold the
    /// same messages forever, which is exactly the hour-long stall the reference
    /// implementation recorded.
    NoProgress {
        reported_end: usize,
        covered_end: usize,
    },
    /// The model reported a span that starts before the requested start, so it
    /// would re-summarize messages already compartmented.
    Rewound {
        reported_start: usize,
        expected_start: usize,
    },
    /// The model reported an end beyond the conversation. Nothing to store.
    OutOfRange {
        reported_end: usize,
        message_count: usize,
    },
}

/// Validate a model's reported span against the requested one.
///
/// The model is asked to fill `requested`; what comes back is untrusted. Four
/// things can be wrong, and each is a separate verdict because each needs a
/// different response — a rewind is a different bug from a stall, and neither is
/// "the conversation is too short".
pub fn validate_span(
    requested: &SpanPlan,
    reported_start: usize,
    reported_end: usize,
    message_count: usize,
) -> SpanVerdict {
    let SpanPlan::Fold { start, end } = requested else {
        // Nothing was requested, so any report is out of range by definition.
        return SpanVerdict::OutOfRange {
            reported_end,
            message_count,
        };
    };
    if reported_end >= message_count {
        return SpanVerdict::OutOfRange {
            reported_end,
            message_count,
        };
    }
    if reported_start < *start {
        return SpanVerdict::Rewound {
            reported_start,
            expected_start: *start,
        };
    }
    if reported_end < *start {
        return SpanVerdict::NoProgress {
            reported_end,
            covered_end: start.saturating_sub(1),
        };
    }
    SpanVerdict::Accepted {
        start: reported_start,
        end: reported_end.min(*end).max(*start),
    }
}

/// A stable idempotency key for one fold.
///
/// The span plus the content it covers: two folds of the same span over the SAME
/// messages are the same fold and must collide, while a re-fold after the span
/// was edited must not. Hashing the content as well as the bounds is what makes
/// a retry after a crash a no-op while a genuinely different fold is a new row.
pub fn fold_hash(session_id: &str, start: usize, end: usize, messages: &[Message]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    start.hash(&mut hasher);
    end.hash(&mut hasher);
    for msg in messages.iter().take(end + 1).skip(start) {
        msg.role.as_str().hash(&mut hasher);
        msg.content.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Message {
        Message::user(text)
    }
    fn assistant(text: &str) -> Message {
        Message::assistant(text)
    }
    /// An assistant turn carrying a tool call.
    fn tool_use(id: &str) -> Message {
        Message::assistant("").with_tool_calls(vec![crate::client::ToolCall {
            id: id.to_string(),
            function: crate::client::ToolCallFunction {
                name: "read_file".to_string(),
                arguments: "{}".to_string(),
            },
        }])
    }
    fn tool_result(id: &str) -> Message {
        Message::tool(id, "the file contents")
    }

    /// The floor: fewer than `MIN_SPAN_MESSAGES` is skipped with the reason the
    /// operator needs, not a generic "no".
    #[test]
    fn a_short_history_is_skipped_not_folded() {
        let msgs = vec![user("one"), assistant("two")];
        let plan = plan_span(&msgs, None);
        assert_eq!(plan, SpanPlan::Skip(SkipReason::TooFewMessages));
        assert_eq!(
            skip_reason(&plan),
            Some(SkipReason::TooFewMessages),
            "the skip reason is part of the contract"
        );
        assert_eq!(
            skip_reason(&plan).map(|r| r.as_str()),
            Some("too-few-messages")
        );
    }

    /// The span is aligned to tool groups, because a lone `tool_use` is a
    /// malformed history the providers reject.
    #[test]
    fn the_span_never_splits_a_tool_group() {
        let mut msgs = Vec::new();
        for i in 0..6 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        // A tool group as the newest pair: the fold must END after the result.
        msgs.push(tool_use("call-1"));
        msgs.push(tool_result("call-1"));
        msgs.push(assistant("done"));

        let SpanPlan::Fold { end, .. } = plan_span(&msgs, None) else {
            panic!("a foldable history must fold");
        };
        // The end must be a position whose message is NOT a tool_use awaiting
        // its result, and the group it sits in must be complete.
        assert_eq!(end, msgs.len() - 1);
        assert!(
            !crate::context_management::is_tool_use(&msgs[end]),
            "a fold may not end on an unanswered tool_use"
        );
    }

    /// An unanswered user turn is not folded: summarizing a question nobody has
    /// answered yet loses the thing the next turn exists to answer. The planner
    /// backs off to the last closed turn instead of giving up.
    #[test]
    fn an_open_user_turn_is_backed_off_not_folded() {
        let mut msgs = Vec::new();
        for i in 0..6 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        msgs.push(user("the question nobody has answered"));

        let SpanPlan::Fold { start, end } = plan_span(&msgs, None) else {
            panic!("the earlier turns are still foldable");
        };
        assert_eq!(start, 0);
        assert_eq!(
            end, 11,
            "the fold stops at the last closed turn, before the open question"
        );
        assert_eq!(msgs[end].role, crate::client::Role::Assistant);
    }

    /// An unanswered tool_use is not folded either, and the back-off covers it.
    #[test]
    fn an_open_tool_call_is_backed_off_not_folded() {
        let mut msgs = Vec::new();
        for i in 0..6 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        msgs.push(tool_use("call-1")); // result has not arrived

        let SpanPlan::Fold { end, .. } = plan_span(&msgs, None) else {
            panic!("the earlier turns are still foldable");
        };
        assert_eq!(end, 11, "the fold stops before the unanswered call");
    }

    /// The anchor advances the start: the next span begins AFTER what is covered.
    #[test]
    fn the_anchor_starts_the_next_span_after_the_covered_one() {
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(user(&format!("q{i}")));
            msgs.push(assistant(&format!("a{i}")));
        }
        let SpanPlan::Fold { start, .. } = plan_span(&msgs, Some(11)) else {
            panic!("a covered span of 12 leaves plenty to fold");
        };
        assert_eq!(start, 12, "one past the covered end");
    }

    /// THE GUARD: a reported span that does not advance is a hard error.
    ///
    /// This is the failure that matters. A historian that reports the span it
    /// was handed — or none — would be stored, and the next pass would start
    /// from the same anchor, so every turn re-folds the same messages. Nothing
    /// upstream notices, because each pass individually "succeeded".
    #[test]
    fn a_reported_span_that_does_not_advance_is_a_no_progress_error() {
        // The anchor is 20: everything before position 20 already has a
        // compartment. A model that reports an end INSIDE the covered span has
        // folded nothing new, and accepting it would re-fold the same messages
        // every turn forever.
        let requested = SpanPlan::Fold { start: 20, end: 27 };
        let verdict = validate_span(&requested, 20, 19, 40);
        assert_eq!(
            verdict,
            SpanVerdict::NoProgress {
                reported_end: 19,
                covered_end: 19,
            },
            "a reported span that does not advance the anchor must be refused"
        );

        // THE BOUNDARY, exactly. Position 19 is the last covered one, so a report
        // ending at 19 folds nothing new; a report ending at 20 folds the first
        // uncovered message and IS progress. The guard sits on the boundary
        // rather than one side of it, because a guard that is off by one either
        // refuses every fold or accepts every re-fold.
        let at_the_boundary = validate_span(&requested, 20, 19, 40);
        assert_eq!(
            at_the_boundary,
            SpanVerdict::NoProgress {
                reported_end: 19,
                covered_end: 19,
            },
            "ending on the last already-covered position is a no-op"
        );
        let one_past = validate_span(&requested, 20, 20, 40);
        assert_eq!(
            one_past,
            SpanVerdict::Accepted { start: 20, end: 20 },
            "the first uncovered position is progress, even by one message"
        );
    }

    /// A rewound span (starting before the requested start) is refused, because
    /// it would re-summarize messages that already have a compartment.
    #[test]
    fn a_rewound_span_is_refused() {
        let requested = SpanPlan::Fold { start: 8, end: 20 };
        let verdict = validate_span(&requested, 2, 10, 40);
        assert_eq!(
            verdict,
            SpanVerdict::Rewound {
                reported_start: 2,
                expected_start: 8
            }
        );
    }

    /// An end past the conversation is refused: there is nothing there to store,
    /// and storing it would leave the anchor past the real history.
    #[test]
    fn an_end_past_the_conversation_is_refused() {
        let requested = SpanPlan::Fold { start: 0, end: 7 };
        let verdict = validate_span(&requested, 0, 99, 10);
        assert_eq!(
            verdict,
            SpanVerdict::OutOfRange {
                reported_end: 99,
                message_count: 10
            }
        );
    }

    /// A well-formed report is accepted, and its end is clamped to the requested
    /// ceiling so an over-long report cannot commit the caller to a span it did
    /// not authorize.
    #[test]
    fn a_valid_span_is_accepted_and_clamped_to_the_request() {
        // In range, but past the requested ceiling: the model over-reported
        // within the conversation. The request is the authority on how much the
        // caller authorized, so the end is clamped to it.
        let requested = SpanPlan::Fold { start: 0, end: 7 };
        let verdict = validate_span(&requested, 0, 30, 40);
        assert_eq!(
            verdict,
            SpanVerdict::Accepted { start: 0, end: 7 },
            "the request bounds what may be committed"
        );

        // A report that stops short of the request is still accepted — it is
        // progress, just not all of it.
        let verdict = validate_span(&requested, 0, 3, 40);
        assert_eq!(verdict, SpanVerdict::Accepted { start: 0, end: 3 });

        // A report past the END OF THE CONVERSATION is a different failure: there
        // is nothing there, so clamping to the request would store a span over
        // messages that do not exist. The range check runs before the clamp.
        let verdict = validate_span(&requested, 0, 200, 40);
        assert_eq!(
            verdict,
            SpanVerdict::OutOfRange {
                reported_end: 200,
                message_count: 40
            }
        );
    }

    /// The hash is the idempotency key: the same span over the same messages
    /// collides (a retry is a no-op), and an edited span does not.
    #[test]
    fn the_hash_collides_for_a_retry_and_not_for_an_edited_span() {
        let msgs = vec![
            user("what is the plan"),
            assistant("the plan is"),
            user("why"),
            assistant("because"),
        ];
        let a = fold_hash("s1", 0, 3, &msgs);
        let b = fold_hash("s1", 0, 3, &msgs);
        assert_eq!(a, b, "a retry of the same fold is the same fold");

        let mut edited = msgs.clone();
        edited[2] = user("why exactly");
        assert_ne!(
            a,
            fold_hash("s1", 0, 3, &edited),
            "edited content is a new fold"
        );

        assert_ne!(
            a,
            fold_hash("s2", 0, 3, &msgs),
            "a different session is a new fold"
        );
    }
    /// The parser tolerates the shapes a model actually returns.
    ///
    /// The prompt asks for plain prose, but a fenced answer or a preamble is
    /// common enough that a strict parser would discard a fold that DID reach the
    /// model — paying for the call and keeping nothing.
    #[test]
    fn the_parser_accepts_prose_a_fence_and_a_preamble() {
        let plain =
            parse_compartment("Decided on the chunked writer. The lock is held across the write.");
        assert_eq!(
            plain.as_deref(),
            Some("Decided on the chunked writer. The lock is held across the write.")
        );

        let fenced = parse_compartment("```text\nThe lock is held across the write.\n```");
        assert_eq!(
            fenced.as_deref(),
            Some("The lock is held across the write.")
        );

        // A fenced block with no closing fence still yields the content.
        let unclosed = parse_compartment("```\nThe lock is held.\n");
        assert_eq!(unclosed.as_deref(), Some("The lock is held."));

        // Whitespace is collapsed, because a compartment renders inline.
        let multiline = parse_compartment("first line\n\n  second   line");
        assert_eq!(multiline.as_deref(), Some("first line second line"));
    }

    /// An empty or whitespace-only reply is a FAILURE, not an empty compartment.
    ///
    /// Storing an empty body would leave a row that occupies a span and says
    /// nothing, and the anchor would advance past messages that were never
    /// summarized — the one outcome worse than not folding.
    #[test]
    fn an_empty_reply_is_no_compartment() {
        assert_eq!(parse_compartment(""), None);
        assert_eq!(parse_compartment("   \n\n  "), None);
        assert_eq!(parse_compartment("```\n\n```"), None);
    }
}

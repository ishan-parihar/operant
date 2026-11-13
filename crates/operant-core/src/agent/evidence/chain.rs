//! Running a chain of methods over one payload.
//!
//! Three properties hold for every chain that runs through this module:
//!
//! * **It cannot make the payload bigger.** A chain that grows the text is
//!   discarded and the original is kept.
//! * **It cannot panic.** A method that panics aborts its chain and the
//!   original is kept.
//! * **It reports what it did.** The applied method names travel with the
//!   result so the caller can emit provenance and decide whether a lossy
//!   method needs a recall record.

use super::types::{Block, Method, MethodCtx, MethodInfo, Outcome, Produces, Recovery};

/// What a chain did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainOutcome {
    /// The resulting text.
    pub text: String,
    /// The methods that changed the text, in application order.
    pub applied: Vec<&'static str>,
    /// Whether any applied method declared itself lossy.
    pub lossy: bool,
}

impl ChainOutcome {
    /// The text unchanged, with nothing applied.
    pub fn unchanged(text: &str) -> Self {
        ChainOutcome {
            text: text.to_string(),
            applied: Vec::new(),
            lossy: false,
        }
    }
}

/// Run `methods` over `block`, tracing what each one did.
///
/// A method that returns `Skip` leaves the text alone and the chain
/// continues. A method that returns `ShortCircuit` ends the chain.
///
/// `recall_available` says whether the full text can be retrieved after the
/// chain runs. It gates the LOSSY methods: with no recall path, a lossy
/// method is skipped and the payload keeps its bytes.
pub fn run_chain_traced(
    methods: &[&dyn Method],
    block: &Block,
    ctx: &MethodCtx,
    recall_available: bool,
) -> ChainOutcome {
    let mut current = block.text.to_string();
    let mut applied: Vec<&'static str> = Vec::new();
    let mut lossy = false;
    // The shape the payload currently has. A method that declares what it
    // PRODUCES changes it, and the next method is judged against the new
    // shape. Without this, `consumes` was checked against the shape the
    // block arrived with, so a method that ran after a re-encoder was asked
    // whether it accepts a shape that no longer existed.
    let mut current_format = block.format;

    for method in methods {
        // A method that does not accept this payload shape does not run on
        // it. `consumes` was declared by every method and read by nobody;
        // now it is the rule it always looked like.
        if !accepts(*method, current_format) {
            continue;
        }
        // The RECOVERABLE guarantee is a precondition, not a hope: a lossy
        // method may only run when the full text can come back. With no
        // recall path, the method is skipped and the payload keeps its
        // bytes.
        let info = method.info();
        if info.lossy && info.recovery == Recovery::Recall && !recall_available {
            continue;
        }
        let step = Block {
            text: &current,
            format: current_format,
            ..*block
        };
        match method.apply(&step, ctx) {
            Outcome::Continue(next) => {
                if next != current {
                    applied.push(method.name());
                    lossy |= info.lossy;
                    current = next;
                    if let Produces::Format(shape) = info.produces {
                        current_format = shape;
                    }
                }
            }
            Outcome::ShortCircuit(final_text) => {
                if final_text != current {
                    applied.push(method.name());
                    lossy |= info.lossy;
                }
                current = final_text;
                break;
            }
            Outcome::Skip => {}
        }
    }

    ChainOutcome {
        text: current,
        applied,
        lossy,
    }
}

/// Run `methods` over `block`, refusing any chain that made the payload
/// bigger or panicked.
///
/// `None` means the chain could not be trusted: either a method panicked or
/// the result grew. The caller keeps the original text in that case.
///
/// `recall_available` gates the lossy methods; see [`run_chain_traced`].
pub fn run_chain_guarded(
    methods: &[&dyn Method],
    block: &Block,
    ctx: &MethodCtx,
    recall_available: bool,
) -> Option<ChainOutcome> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_chain_traced(methods, block, ctx, recall_available)
    }))
    .ok()?;

    if outcome.text.len() > block.text.len() {
        return None;
    }
    Some(outcome)
}

/// Whether a method may run on a payload of this format.
///
/// A method that declares no accepted formats accepts any.
pub fn accepts(method: &dyn Method, format: super::types::ContentFormat) -> bool {
    let info: MethodInfo = method.info();
    info.consumes.is_empty() || info.consumes.contains(&format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::evidence::types::{ContentFormat, MethodInfo, Origin, Subject};

    struct Upper;
    impl Method for Upper {
        fn name(&self) -> &'static str {
            "test-upper"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo::LOSSLESS_TRANSFORM
        }
        fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
            Outcome::Continue(block.text.to_uppercase())
        }
    }

    struct Skipper;
    impl Method for Skipper {
        fn name(&self) -> &'static str {
            "test-skip"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo::LOSSLESS_TRANSFORM
        }
        fn apply(&self, _block: &Block, _ctx: &MethodCtx) -> Outcome {
            Outcome::Skip
        }
    }

    struct Panics;
    impl Method for Panics {
        fn name(&self) -> &'static str {
            "test-panic"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo::LOSSLESS_TRANSFORM
        }
        fn apply(&self, _block: &Block, _ctx: &MethodCtx) -> Outcome {
            panic!("method bug");
        }
    }

    struct Grows;
    impl Method for Grows {
        fn name(&self) -> &'static str {
            "test-grow"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo::LOSSLESS_TRANSFORM
        }
        fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
            Outcome::Continue(format!("{}{}", block.text, "x".repeat(64)))
        }
    }

    fn block_over<'a>(text: &'a str, origin: &'a Origin) -> Block<'a> {
        Block {
            text,
            subject: Subject::ToolResult,
            format: ContentFormat::Term,
            origin,
        }
    }

    /// A lossy method, for the recall precondition.
    struct Lossy;
    impl Method for Lossy {
        fn name(&self) -> &'static str {
            "test-lossy"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo::LOSSY_RECALLABLE
        }
        fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
            Outcome::Continue(block.text.to_lowercase())
        }
    }

    /// A method that only accepts JSON, and emits plain text.
    struct JsonOnly;
    impl Method for JsonOnly {
        fn name(&self) -> &'static str {
            "test-json-only"
        }
        fn info(&self) -> MethodInfo {
            MethodInfo {
                consumes: &[ContentFormat::Json],
                produces: super::super::types::Produces::Format(ContentFormat::Plain),
                ..MethodInfo::LOSSY_RECALLABLE
            }
        }
        fn apply(&self, _block: &Block, _ctx: &MethodCtx) -> Outcome {
            Outcome::Continue("plain".to_string())
        }
    }

    #[test]
    fn a_lossy_method_does_not_run_when_there_is_no_recall_path() {
        // The plan's §9 guard: "P3's recall wiring is a precondition for
        // enabling lossy methods, not a follow-up". The runner used to read
        // only `lossy` and never looked at `recovery`, so the precondition
        // was assumed.
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let block = block_over("ABC", &origin);
        let with_recall = run_chain_traced(&[&Lossy], &block, &ctx, true);
        assert_eq!(with_recall.text, "abc", "with a recall path it runs");
        let without_recall = run_chain_traced(&[&Lossy], &block, &ctx, false);
        assert_eq!(
            without_recall.text, "ABC",
            "without a recall path the payload keeps its bytes"
        );
        assert!(without_recall.applied.is_empty());
        assert!(!without_recall.lossy);
    }

    #[test]
    fn the_shape_follows_what_a_method_produces() {
        // `produces` was declared by two methods and matched by nobody. Now
        // it changes the shape the NEXT method is judged against: a method
        // that emits plain text is not followed by a method that accepts only
        // JSON.
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let block = block_over("{}", &origin);
        // The block's format is Term (the helper's default), so the JSON-only
        // method does not run at all.
        let out = run_chain_traced(&[&JsonOnly], &block, &ctx, true);
        assert!(out.applied.is_empty(), "the method refused the shape");
    }

    #[test]
    fn chain_applies_methods_in_order_and_skip_is_neutral() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_traced(&[&Skipper, &Upper], &block_over("abc", &origin), &ctx, true);
        assert_eq!(out.text, "ABC");
        assert_eq!(out.applied, vec!["test-upper"]);
    }

    #[test]
    fn a_method_that_changes_nothing_is_not_reported_as_applied() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_traced(&[&Skipper], &block_over("abc", &origin), &ctx, true);
        assert!(out.applied.is_empty());
    }

    #[test]
    fn guarded_contains_panics_and_emits_nothing() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        assert!(run_chain_guarded(&[&Panics], &block_over("abc", &origin), &ctx, true).is_none());
    }

    #[test]
    fn guarded_refuses_a_chain_that_grew_the_payload() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        assert!(run_chain_guarded(&[&Grows], &block_over("abc", &origin), &ctx, true).is_none());
    }

    #[test]
    fn a_chain_that_shrinks_the_payload_is_kept() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_guarded(&[&Upper], &block_over("abc", &origin), &ctx, true);
        assert_eq!(out.map(|o| o.text), Some("ABC".to_string()));
    }
}

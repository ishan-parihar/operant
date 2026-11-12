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

use super::types::{Block, Method, MethodCtx, MethodInfo, Outcome};

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
pub fn run_chain_traced(methods: &[&dyn Method], block: &Block, ctx: &MethodCtx) -> ChainOutcome {
    let mut current = block.text.to_string();
    let mut applied: Vec<&'static str> = Vec::new();
    let mut lossy = false;

    for method in methods {
        let step = Block {
            text: &current,
            ..*block
        };
        match method.apply(&step, ctx) {
            Outcome::Continue(next) => {
                if next != current {
                    applied.push(method.name());
                    lossy |= method.info().lossy;
                    current = next;
                }
            }
            Outcome::ShortCircuit(final_text) => {
                if final_text != current {
                    applied.push(method.name());
                    lossy |= method.info().lossy;
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
pub fn run_chain_guarded(
    methods: &[&dyn Method],
    block: &Block,
    ctx: &MethodCtx,
) -> Option<ChainOutcome> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_chain_traced(methods, block, ctx)
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

    #[test]
    fn chain_applies_methods_in_order_and_skip_is_neutral() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_traced(&[&Skipper, &Upper], &block_over("abc", &origin), &ctx);
        assert_eq!(out.text, "ABC");
        assert_eq!(out.applied, vec!["test-upper"]);
    }

    #[test]
    fn a_method_that_changes_nothing_is_not_reported_as_applied() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_traced(&[&Skipper], &block_over("abc", &origin), &ctx);
        assert!(out.applied.is_empty());
    }

    #[test]
    fn guarded_contains_panics_and_emits_nothing() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        assert!(run_chain_guarded(&[&Panics], &block_over("abc", &origin), &ctx).is_none());
    }

    #[test]
    fn guarded_refuses_a_chain_that_grew_the_payload() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        assert!(run_chain_guarded(&[&Grows], &block_over("abc", &origin), &ctx).is_none());
    }

    #[test]
    fn a_chain_that_shrinks_the_payload_is_kept() {
        let (origin, ctx) = (Origin::default(), MethodCtx::new("test"));
        let out = run_chain_guarded(&[&Upper], &block_over("abc", &origin), &ctx);
        assert_eq!(out.map(|o| o.text), Some("ABC".to_string()));
    }
}

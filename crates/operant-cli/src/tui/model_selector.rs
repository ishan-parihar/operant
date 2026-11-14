//! One grammar for "which model, in which role, at what effort".
//!
//! (2026-11-15, model plan M4) Before this, `/model <x>` treated its argument as
//! a FILTER STRING for the picker: typing `/model @smol:xhigh` narrowed the list to
//! nothing and set nothing. There was no way to say "the recap role's model, at
//! xhigh" in one string, so role and effort each needed a separate control.
//!
//! The grammar is oh-my-pi's, and it is one string rather than three controls:
//!
//! ```text
//!   @role[:level]              the role's model, optionally at a level
//!   role[:level]               same, when `role` is a known role name
//!   [provider/]modelId[:level] a specific model, optionally at a level
//! ```
//!
//! Two of oh-my-pi's rules are load-bearing here and are the reason this is not
//! a `split_once(':')`:
//!
//! 1. **The literal-id guard.** Real model ids END IN `:max` (`glm-4.7:max`), so an
//!    ungated split silently reinterprets a real model as a model plus a level.
//!    The whole selector is checked against the live catalogue before anything is
//!    split off it.
//! 2. **A role alias is a prefix, not a whole token.** `@smol:xhigh` has to split
//!    at the colon AFTER the alias, so the caller passes a minimum colon index.

use crate::tui::model_effort::EffortLevel;
use operant_core::model_roles::ModelSlot;

/// What a selector resolved to. Every field is optional because a selector may
/// name only one of the three axes; `default` names none of them and means "open
/// the picker", which is what a bare `/model` has always done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedSelector {
    /// A configured model slot (`recap`, `vision`, …). When set, the caller
    /// assigns that slot rather than moving the conversation.
    pub role: Option<ModelSlot>,
    /// A model id, `provider/`-qualified or bare.
    pub model: Option<String>,
    /// A reasoning level.
    pub effort: Option<EffortLevel>,
}

impl ResolvedSelector {
    /// Whether the selector named anything at all.
    pub fn is_empty(&self) -> bool {
        self.role.is_none() && self.model.is_none() && self.effort.is_none()
    }
}

/// Why a selector could not be resolved. Every variant carries the offending
/// text, because "unknown model" without the text is not actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// `@word` where `word` is not a configured role.
    UnknownRole(String),
    /// A `:suffix` that is not a reasoning level.
    UnknownEffort(String),
    /// A bare word that is neither a role nor a model the catalogue lists.
    ///
    /// Only raised when a catalogue was supplied. An empty catalogue means the
    /// fetch has not happened yet or the provider is unmapped, and refusing on no
    /// evidence would block a legitimate id the list simply does not know.
    UnknownModel { typed: String, hint: Option<String> },
}

/// Split a trailing `:level` off `pattern`, at or after `min_colon`.
///
/// Returns the pattern unchanged when there is no colon at or after `min_colon`,
/// or when the text after the LAST such colon is not a reasoning level — so
/// `provider/id` and a bare id both come back whole.
pub fn split_thinking_suffix(pattern: &str, min_colon: usize) -> (&str, Option<&str>) {
    let Some(at) = pattern.rfind(':') else {
        return (pattern, None);
    };
    if at < min_colon {
        return (pattern, None);
    }
    let (head, tail) = pattern.split_at(at);
    let tail = &tail[1..];
    if EffortLevel::from_wire(tail).is_some() {
        (head, Some(tail))
    } else {
        (pattern, None)
    }
}

/// Resolve one selector against the models the catalogue currently lists.
///
/// `available` is the LIVE list: the fetched endpoint catalogue plus the
/// configured `[agent] model` and `fallback_models`. Pass it empty when no fetch
/// has resolved — the resolver then accepts any `provider/id` and any bare word
/// rather than guessing at a list it does not have.
pub fn resolve(selector: &str, available: &[String]) -> Result<ResolvedSelector, SelectorError> {
    let raw = selector.trim();
    if raw.is_empty() {
        return Ok(ResolvedSelector::default());
    }

    // The literal-id guard: a real id may END in a level (`glm-4.7:max`), so the
    // whole selector wins over any way of splitting it.
    if available.iter().any(|m| m == raw) {
        return Ok(ResolvedSelector {
            model: Some(raw.to_string()),
            ..ResolvedSelector::default()
        });
    }

    // `@role[:level]` — the `@` says "this is a role", so no catalogue lookup is
    // needed to tell it from a model id.
    if let Some(rest) = raw.strip_prefix('@') {
        let (word, effort) = split_thinking_suffix(rest, 0);
        let role =
            ModelSlot::parse(word).ok_or_else(|| SelectorError::UnknownRole(word.to_string()))?;
        return Ok(ResolvedSelector {
            role: Some(role),
            effort: parse_effort(effort)?,
            model: None,
        });
    }

    let (head, effort) = split_thinking_suffix(raw, 0);
    let effort = parse_effort(effort)?;

    // A colon whose tail is not a level is either an EXTENDED id (`x:free`,
    // `x:beta`) or a mistyped level. The two are told apart by what sits before the
    // colon: when THAT is a model the catalogue lists, the operator appended a
    // suffix to a real model, and `gpt-5:free` is not a thing — so the tail was
    // meant as a level and the useful error names it. When it is not listed, the
    // whole string is left alone as an extended id the list does not carry.
    if effort.is_none()
        && let Some((before, tail)) = raw.rsplit_once(':')
        && !tail.is_empty()
        && available.iter().any(|m| m == before)
    {
        return Err(SelectorError::UnknownEffort(tail.to_string()));
    }

    // A bare word may be a role name (`/model slow`). A `provider/id` is never a
    // role, so the `/` test comes first and the two namespaces cannot collide.
    if !head.contains('/') {
        if let Some(role) = ModelSlot::parse(head) {
            return Ok(ResolvedSelector {
                role: Some(role),
                effort,
                model: None,
            });
        }
    }

    if head.trim().is_empty() {
        return Err(SelectorError::UnknownModel {
            typed: raw.to_string(),
            hint: None,
        });
    }

    // A bare word that is neither a role nor listed is the one case worth
    // refusing: the operator almost certainly meant a model and got the name
    // wrong, and the catalogue is in hand to say so.
    if !head.contains('/') && !available.is_empty() && !available.iter().any(|m| m == head) {
        return Err(SelectorError::UnknownModel {
            typed: head.to_string(),
            hint: closest(head, available),
        });
    }

    Ok(ResolvedSelector {
        role: None,
        model: Some(head.to_string()),
        effort,
    })
}

/// Parse the `:level` half. `None` means there was no suffix.
fn parse_effort(suffix: Option<&str>) -> Result<Option<EffortLevel>, SelectorError> {
    match suffix {
        None => Ok(None),
        Some(raw) => EffortLevel::from_wire(raw)
            .map(Some)
            .ok_or_else(|| SelectorError::UnknownEffort(raw.to_string())),
    }
}

/// The closest listed id to `typed`, for the "did you mean" hint.
///
/// A plain prefix or substring match, deliberately: this runs on one keystroke and
/// a Levenshtein over a 9,000-row catalogue is not worth the accuracy on a list
/// the operator can see.
fn closest(typed: &str, available: &[String]) -> Option<String> {
    let lower = typed.to_ascii_lowercase();
    available
        .iter()
        .find(|m| {
            let m = m.to_ascii_lowercase();
            m.contains(&lower) || lower.contains(&m)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Vec<String> {
        vec![
            "gpt-5".to_string(),
            "claude-opus-4-8".to_string(),
            "glm-4.7:max".to_string(),
        ]
    }

    #[test]
    fn a_literal_id_ending_in_a_level_is_not_split() {
        // The guard. `glm-4.7:max` is a real id, so it must come back whole — an
        // ungated split turns it into `glm-4.7` at effort `max`.
        let got = resolve("glm-4.7:max", &listed()).expect("a listed id resolves");
        assert_eq!(
            got,
            ResolvedSelector {
                model: Some("glm-4.7:max".to_string()),
                ..ResolvedSelector::default()
            },
            "a listed id must not be split into a model and a level"
        );
    }

    #[test]
    fn a_model_and_a_level_split_when_the_head_is_a_model() {
        let got = resolve("gpt-5:xhigh", &listed()).expect("resolves");
        assert_eq!(got.model.as_deref(), Some("gpt-5"));
        assert_eq!(got.effort, Some(EffortLevel::XHigh));
    }

    #[test]
    fn a_provider_qualified_id_is_accepted_verbatim() {
        let got = resolve("openai/gpt-5", &[]).expect("resolves");
        assert_eq!(got.model.as_deref(), Some("openai/gpt-5"));
        assert!(got.effort.is_none());
    }

    #[test]
    fn a_role_alias_takes_an_effort_after_the_alias() {
        // The alias-prefix rule: the colon that ends `@smol` is the one to split
        // at, not the first colon in the string.
        let got = resolve("@smol:xhigh", &[]).expect("resolves");
        assert_eq!(got.role, Some(ModelSlot::Recap));
        assert_eq!(got.effort, Some(EffortLevel::XHigh));
        assert!(got.model.is_none(), "a role selector names no model");
    }

    #[test]
    fn a_bare_role_name_resolves_to_the_slot() {
        for (word, slot) in [
            ("slow", ModelSlot::Memory),
            ("vision", ModelSlot::Vision),
            ("default", ModelSlot::Conversation),
            ("advisor", ModelSlot::MoaReference),
        ] {
            let got = resolve(word, &[]).expect("a role name resolves");
            assert_eq!(got.role, Some(slot), "{word} must resolve to {slot:?}");
            assert!(got.model.is_none(), "{word} is a role, not a model");
        }
    }

    #[test]
    fn an_unknown_role_and_an_unknown_level_are_distinct_errors() {
        assert_eq!(
            resolve("@nope", &[]),
            Err(SelectorError::UnknownRole("nope".to_string()))
        );
        assert_eq!(
            resolve("gpt-5:ultra", &listed()),
            Err(SelectorError::UnknownEffort("ultra".to_string()))
        );
    }

    #[test]
    fn an_extended_id_suffix_is_left_alone_when_the_head_is_not_listed() {
        // OpenRouter really does name ids `vendor/model:free` and `:beta`. Those
        // must survive: the head is not a listed id, so the suffix is part of the
        // name rather than a mistyped level.
        let got = resolve("qwen/qwen3:free", &listed()).expect("an extended id resolves");
        assert_eq!(got.model.as_deref(), Some("qwen/qwen3:free"));
        assert!(got.effort.is_none());
    }

    #[test]
    fn a_misspelled_model_is_refused_with_a_hint_when_the_catalogue_is_known() {
        let err = resolve("gpt-", &listed()).expect_err("not a listed model");
        match err {
            SelectorError::UnknownModel { typed, hint } => {
                assert_eq!(typed, "gpt-");
                assert_eq!(
                    hint.as_deref(),
                    Some("gpt-5"),
                    "the hint must name a model the operator can actually pick"
                );
            }
            other => panic!("expected UnknownModel, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_catalogue_never_refuses() {
        // A provider whose fetch has not resolved yet must not block a legitimate
        // id it simply cannot see.
        let got = resolve("some-brand-new-model", &[]).expect("an empty list accepts");
        assert_eq!(got.model.as_deref(), Some("some-brand-new-model"));
    }

    #[test]
    fn an_empty_selector_names_nothing() {
        assert!(resolve("", &listed()).expect("empty resolves").is_empty());
        assert!(
            resolve("   ", &listed())
                .expect("blank resolves")
                .is_empty()
        );
    }

    #[test]
    fn split_thinking_suffix_respects_the_minimum_colon() {
        // `@smol:xhigh`: index 0 is the `@`, so a floor of 1 must not split at 0.
        assert_eq!(
            split_thinking_suffix("@smol:xhigh", 1),
            ("@smol", Some("xhigh"))
        );
        assert_eq!(
            split_thinking_suffix("@smol:xhigh", 6),
            ("@smol:xhigh", None)
        );
        // A colon whose tail is not a level is not a separator.
        assert_eq!(
            split_thinking_suffix("provider/id", 0),
            ("provider/id", None)
        );
    }
}

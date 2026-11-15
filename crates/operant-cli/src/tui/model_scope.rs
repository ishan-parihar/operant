//! The models page's scope axis: which slice of the catalogue the body shows.
//!
//! (model plan M7, the sidebar) Everything is one flat inline list today —
//! configured models, fetched catalogue, and role rows, in insertion order. The
//! scope axis is the missing dimension: the operator asks "what have I used
//! lately?", "what is assigned to a role?", "what does this provider have?" —
//! and today all three are the same scroll.
//!
//! The scopes follow omp's `model-hub.ts:1-12` seam: recently used, roles, all
//! models, and one entry per provider. This module is the DATA half — the list of
//! scopes and the row selection for each — so the render half is a layout change
//! rather than a second implementation of what a scope means.

use crate::tui::operant_app::tui_fns::{PickerAction, PickerEntry};

/// One entry in the scope sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelScope {
    /// The operator's most recently used models, newest first.
    Recent,
    /// The role rows: one per model slot the config routes to.
    Roles,
    /// Every model, which is today's flat list.
    All,
    /// One provider's models.
    Provider(String),
}

impl ModelScope {
    /// The sidebar's label for this scope.
    pub fn label(&self) -> String {
        match self {
            ModelScope::Recent => "Recent".to_string(),
            ModelScope::Roles => "Roles".to_string(),
            ModelScope::All => "All models".to_string(),
            ModelScope::Provider(name) => name.clone(),
        }
    }
}

/// The sidebar's rows: the three fixed scopes, then one per provider that has
/// at least one row in the body.
///
/// The order is the seam's order (`model-hub.ts:1-12`), not alphabetical: the
/// fixed scopes are the questions the operator asks most, and providers follow in
/// first-appearance order so the sidebar does not reshuffle as the catalogue
/// loads.
pub fn scopes(entries: &[PickerEntry]) -> Vec<ModelScope> {
    let mut out = vec![ModelScope::Recent, ModelScope::Roles, ModelScope::All];
    for entry in entries {
        for option in &entry.options {
            let name = option.provider.trim();
            if name.is_empty() {
                continue;
            }
            let scope = ModelScope::Provider(name.to_string());
            if !out.contains(&scope) {
                out.push(scope);
            }
        }
    }
    out
}

/// Whether a row belongs to a scope.
///
/// A role row belongs to `Roles` and to no provider scope; a model row belongs to
/// every provider it carries an option for, which is how an OpenRouter row that
/// resolves to a vendor model appears under both names.
pub fn contains(scope: &ModelScope, entry: &PickerEntry, recent: &[&str]) -> bool {
    match scope {
        ModelScope::All => true,
        ModelScope::Roles => matches!(entry.action, PickerAction::ModelSlot(_)),
        // A role row is an ASSIGNMENT, not a model: it selects nothing when the
        // operator asks a provider "what do you have?". Making it fall through
        // is how a role row shows up in a provider's model list twice, once as
        // its own slot and once as an Anthropic row.
        ModelScope::Provider(name) => {
            !matches!(entry.action, PickerAction::ModelSlot(_))
                && entry
                    .options
                    .iter()
                    .any(|option| option.provider.trim() == name)
        }
        // `Recent` is the recency window, and the caller supplies it: the
        // entries this picker holds are catalogue rows while the recency file
        // holds model names, so the join happens against a list the caller read
        // rather than a global this module reached for.
        //
        // Empty when nothing is recorded, which is the correct state on a fresh
        // install rather than a defect: a scope that invented an order would be
        // worse than one that honestly shows nothing.
        ModelScope::Recent => recent
            .iter()
            .any(|model| model.eq_ignore_ascii_case(&entry.name)),
    }
}

/// The indices of the rows a scope selects, in body order.
pub fn select(scope: &ModelScope, entries: &[PickerEntry], recent: &[&str]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| contains(scope, entry, recent))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::operant_app::tui_fns::{PickerAction, PickerOption};

    fn model_row(name: &str, providers: &[&str], slot: bool) -> PickerEntry {
        PickerEntry {
            name: name.to_string(),
            options: providers
                .iter()
                .map(|provider| PickerOption {
                    provider: (*provider).to_string(),
                    api_method: "chat".to_string(),
                    available: true,
                    detail: String::new(),
                    estimated_reference_cost_micros: None,
                })
                .collect(),
            action: if slot {
                PickerAction::ModelSlot(operant_core::model_roles::ModelSlot::Vision)
            } else {
                PickerAction::Model
            },
            selected_option: 0,
            is_current: false,
            is_default: false,
            is_favorite: false,
            recommended: false,
            recommendation_rank: 0,
            usage_score: 0,
            old: false,
            created_date: None,
            effort: None,
        }
    }

    #[test]
    fn the_sidebar_lists_the_fixed_scopes_and_every_provider_with_a_row() {
        let entries = vec![
            model_row("gpt-4", &["OpenAI"], false),
            model_row("claude", &["Anthropic"], false),
            model_row("vision", &["Anthropic"], true),
        ];
        let scopes = scopes(&entries);
        assert_eq!(
            scopes,
            vec![
                ModelScope::Recent,
                ModelScope::Roles,
                ModelScope::All,
                ModelScope::Provider("OpenAI".to_string()),
                ModelScope::Provider("Anthropic".to_string()),
            ],
            "the fixed scopes lead, then providers in first-appearance order"
        );
        // A provider already seen is not listed twice, even when a second row
        // (here a role row) resolves through it.
        assert_eq!(
            scopes
                .iter()
                .filter(|s| matches!(s, ModelScope::Provider(p) if p == "Anthropic"))
                .count(),
            1
        );
    }

    #[test]
    fn a_role_row_is_a_roles_scope_row_and_belongs_to_no_provider() {
        let entries = vec![model_row("vision", &["Anthropic"], true)];
        assert_eq!(
            select(&ModelScope::Roles, &entries, &[]),
            vec![0],
            "the role row is the Roles scope"
        );
        assert!(
            select(
                &ModelScope::Provider("Anthropic".to_string()),
                &entries,
                &[]
            )
            .is_empty(),
            "a role row is an assignment, not a provider's model list"
        );
    }

    #[test]
    fn a_model_row_belongs_to_every_provider_it_resolves_through() {
        // An OpenRouter row that resolves to a vendor model appears under both
        // names: the operator looking for either finds it.
        let entries = vec![model_row("vendor/model", &["OpenRouter", "vendor"], false)];
        assert_eq!(
            select(
                &ModelScope::Provider("OpenRouter".to_string()),
                &entries,
                &[]
            ),
            vec![0]
        );
        assert_eq!(
            select(&ModelScope::Provider("vendor".to_string()), &entries, &[]),
            vec![0]
        );
        assert!(select(&ModelScope::Provider("missing".to_string()), &entries, &[]).is_empty());
    }

    #[test]
    fn recent_selects_the_models_the_recency_list_names() {
        // (panels plan S3) The scope used to select nothing, with the honest
        // comment that the picker carried no recency window. It has one now, so
        // the scope must JOIN against it rather than stay empty — an empty
        // `Recent` on a machine that has switched models is the defect.
        let entries = vec![
            model_row("gpt-4", &["OpenAI"], false),
            model_row("claude-opus", &["Anthropic"], false),
            model_row("unrecorded", &["OpenAI"], false),
        ];
        let recent = vec!["claude-opus", "gpt-4"];
        assert_eq!(
            select(&ModelScope::Recent, &entries, &recent),
            vec![0, 1],
            "both recorded models are selected, in body order"
        );
        // A model the list does not name is not selected, so `Recent` is a
        // ranking rather than a second `All`.
        assert!(
            !select(&ModelScope::Recent, &entries, &recent).contains(&2),
            "an unrecorded model must not appear in Recent"
        );
    }

    #[test]
    fn recent_with_nothing_recorded_selects_nothing() {
        // The correct state on a fresh install, not a defect. A scope that
        // invented an order would be worse than one that shows nothing.
        let entries = vec![model_row("gpt-4", &["OpenAI"], false)];
        assert!(select(&ModelScope::Recent, &entries, &[]).is_empty());
    }

    #[test]
    fn all_is_every_row_and_recent_claims_none_rather_than_inventing_an_order() {
        let entries = vec![
            model_row("gpt-4", &["OpenAI"], false),
            model_row("vision", &["Anthropic"], true),
        ];
        assert_eq!(select(&ModelScope::All, &entries, &[]), vec![0, 1]);
        assert!(
            select(&ModelScope::Recent, &entries, &[]).is_empty(),
            "the picker carries no recency window, so the scope selects nothing \
             rather than inventing an order"
        );
    }
}

//! `Ctrl+]` in the model picker: move along the scope axis.
//!
//! (model plan M7) `model_scope.rs` decides what a scope MEANS; this is the
//! control that moves between them, and the row selection that follows. Without
//! it the scope model would be data with no reader — the phantom surface this
//! plan exists to prevent.

use super::*;
use crate::tui::model_scope::{ModelScope, scopes, select};

impl App {
    /// The scope after `current` in the sidebar's own order, wrapping.
    pub fn next_model_scope(&self) -> ModelScope {
        let entries = self
            .inline_interactive
            .as_ref()
            .map(|picker| picker.entries.as_slice())
            .unwrap_or(&[]);
        let all = scopes(entries);
        let at = all.iter().position(|scope| scope == &self.model_scope);
        match at {
            Some(index) => all[(index + 1) % all.len()].clone(),
            // The current scope is no longer in the list (the catalogue moved
            // and its last provider emptied). Fall to the first scope rather
            // than pinning the operator to a scope with no rows.
            None => all.first().cloned().unwrap_or(ModelScope::All),
        }
    }

    /// Advance the scope and re-point the picker's rows at what it selects.
    ///
    /// Returns the scope now in force, so the caller can name it. The filter is
    /// kept: a scope narrows the BODY, it does not clear what the operator typed.
    pub fn cycle_model_scope(&mut self) -> ModelScope {
        let scope = self.next_model_scope();
        let scope = self.apply_model_scope(scope);
        // The control announces itself wherever it is invoked, so a caller
        // cannot move the scope without the operator being told what it is.
        self.set_status_notice(format!("scope: {}", scope.label()));
        scope
    }

    /// Put a scope in force: the picker's `filtered` becomes the scope's
    /// selection, intersected with the operator's filter text.
    pub fn apply_model_scope(&mut self, scope: ModelScope) -> ModelScope {
        // (audit finding) The scope axis is a MODELS-page axis. The picker
        // handler serves the effort picker too, and an effort row is neither a
        // role nor a provider's model, so applying a scope there selected zero
        // rows and blanked the list for two presses. A control on the wrong
        // surface must refuse rather than empty it.
        if !matches!(
            self.inline_interactive.as_ref().map(|picker| picker.kind),
            Some(crate::tui::operant_app::tui_fns::PickerKind::Model)
        ) {
            return scope;
        }
        self.model_scope = scope.clone();
        let needle = self
            .inline_interactive
            .as_ref()
            .map(|picker| picker.filter.to_lowercase())
            .unwrap_or_default();
        let Some(picker) = self.inline_interactive.as_mut() else {
            return scope;
        };
        // The recency list the `Recent` scope joins against. Read once per
        // scope change, not per row: the file does not change while the picker
        // is open, and a read per row would put a syscall in a filter.
        let recent = crate::tui::model_recent::RecentModels::load();
        let recent_models = recent.models();
        let mut rows = select(&scope, &picker.entries, &recent_models);
        if !needle.is_empty() {
            rows.retain(|index| {
                let entry = &picker.entries[*index];
                entry.name.to_lowercase().contains(&needle)
                    || entry
                        .options
                        .iter()
                        .any(|option| option.provider.to_lowercase().contains(&needle))
            });
        }
        picker.filtered = rows;
        picker.selected = 0;
        scope
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::tests::make_app;
    use crate::tui::model_scope::ModelScope;
    use crate::tui::operant_app::tui_fns::{
        InlineInteractiveState, PickerAction, PickerEntry, PickerKind, PickerOption,
    };

    fn entry(name: &str, provider: &str, slot: bool) -> PickerEntry {
        PickerEntry {
            name: name.to_string(),
            options: vec![PickerOption {
                provider: provider.to_string(),
                api_method: "chat".to_string(),
                available: true,
                detail: String::new(),
                estimated_reference_cost_micros: None,
            }],
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

    fn app_with(entries: Vec<PickerEntry>, filter: &str) -> App {
        let mut app = make_app();
        app.inline_interactive = Some(InlineInteractiveState {
            kind: PickerKind::Model,
            entries,
            filtered: Vec::new(),
            selected: 0,
            column: 0,
            filter: filter.to_string(),
            loading: false,
            preview: false,
        });
        // The live picker's rows are always a scope selection, so start at `All`
        // the way an open picker does rather than with an empty body.
        app.apply_model_scope(crate::tui::model_scope::ModelScope::All);
        app
    }

    fn names(app: &App) -> Vec<String> {
        app.inline_interactive
            .as_ref()
            .expect("picker")
            .filtered
            .iter()
            .map(|index| {
                app.inline_interactive.as_ref().unwrap().entries[*index]
                    .name
                    .clone()
            })
            .collect()
    }

    #[test]
    fn all_selects_every_row_and_the_cycle_moves_on_from_there() {
        let mut app = app_with(
            vec![
                entry("gpt-4", "OpenAI", false),
                entry("vision", "Anthropic", true),
            ],
            "",
        );
        assert_eq!(app.model_scope, ModelScope::All);
        assert_eq!(names(&app).len(), 2, "All is today's flat list");

        // The sidebar's documented order is recently used · roles · all ·
        // providers (`model-hub.ts:1-12`), and the default is `All`, so the
        // walk from the default reaches the providers first and wraps to Recent
        // last. That is the seam's order, not an accident of the cycle.
        let first = app.cycle_model_scope();
        assert_eq!(first, ModelScope::Provider("OpenAI".to_string()));
        assert_eq!(names(&app), vec!["gpt-4".to_string()]);
        assert_eq!(
            app.status_message.as_deref(),
            Some("scope: OpenAI"),
            "the scope names itself: {:?}",
            app.status_message
        );

        assert_eq!(
            app.cycle_model_scope(),
            ModelScope::Provider("Anthropic".to_string())
        );
        // Anthropic holds only the role row, and a role row is not a provider's
        // model list, so this provider scope is empty.
        assert!(names(&app).is_empty());

        assert_eq!(app.cycle_model_scope(), ModelScope::Recent);
        assert!(
            names(&app).is_empty(),
            "Recent selects nothing while the picker carries no recency window"
        );

        assert_eq!(app.cycle_model_scope(), ModelScope::Roles);
        assert_eq!(
            names(&app),
            vec!["vision".to_string()],
            "Roles is the role rows"
        );

        // And the walk wraps back to the default.
        assert_eq!(app.cycle_model_scope(), ModelScope::All);
        assert_eq!(names(&app).len(), 2);
    }

    #[test]
    fn a_scope_narrows_the_body_without_clearing_the_filter() {
        let mut app = app_with(
            vec![
                entry("gpt-4", "OpenAI", false),
                entry("gpt-4o", "OpenAI", false),
                entry("vision", "Anthropic", true),
            ],
            "gpt",
        );
        let scope = app.apply_model_scope(ModelScope::Provider("OpenAI".to_string()));
        assert_eq!(scope, ModelScope::Provider("OpenAI".to_string()));
        assert_eq!(
            names(&app),
            vec!["gpt-4".to_string(), "gpt-4o".to_string()],
            "the scope intersects the operator's filter rather than replacing it"
        );
    }

    #[test]
    fn a_role_row_is_not_reachable_through_a_provider_scope() {
        let mut app = app_with(vec![entry("vision", "Anthropic", true)], "");
        app.apply_model_scope(ModelScope::Provider("Anthropic".to_string()));
        assert!(
            names(&app).is_empty(),
            "a role row is an assignment, not a provider's model list"
        );
    }
}

#[cfg(test)]
mod scope_kind_tests {
    use crate::tui::app::tests::make_app;
    use crate::tui::model_scope::ModelScope;
    use crate::tui::operant_app::tui_fns::{InlineInteractiveState, PickerKind};

    /// (audit finding) The scope axis is a models-page axis. The picker handler
    /// serves the effort picker too, and an effort row is neither a role nor a
    /// provider's model, so an unguarded scope selection emptied the effort list.
    #[test]
    fn a_scope_never_applies_to_the_effort_picker() {
        let mut app = make_app();
        app.inline_interactive = Some(InlineInteractiveState {
            kind: PickerKind::Effort,
            entries: Vec::new(),
            filtered: vec![0, 1, 2],
            selected: 1,
            column: 0,
            filter: String::new(),
            loading: false,
            preview: false,
        });
        let scope = app.apply_model_scope(ModelScope::Roles);
        assert_eq!(scope, ModelScope::Roles, "the scope is still reported");
        let picker = app.inline_interactive.as_ref().expect("picker");
        assert_eq!(
            picker.filtered,
            vec![0, 1, 2],
            "the effort picker's rows must survive a stray scope press"
        );
        assert_eq!(picker.selected, 1, "the cursor must not move either");
        assert!(
            !matches!(app.model_scope, ModelScope::Roles),
            "and the App must not adopt a scope the surface cannot show: {:?}",
            app.model_scope
        );
    }
}

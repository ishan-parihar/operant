//! `Shift+Tab` in the model picker: cycle the conversation model through the
//! models the ROLES are configured with.
//!
//! (model plan M6, the role-cycle key) omp binds `Ctrl+P` / `Shift+Ctrl+P` to
//! cycle the configured role models over `cycleOrder` (default
//! `["smol","default","slow"]`), skipping unavailable roles, with no picker
//! (`model-controls.ts:410-424`). Operant's eight slots are the natural cycle
//! list.
//!
//! The operator asked for `Shift+Tab`, and that is what this takes. It was
//! `cycle_favorite_model` in the picker, so favourite-cycling moves to
//! `Ctrl+Shift+N` — the sibling of `Ctrl+N`, which adds and removes a favourite.
//! Nothing is lost: the two chords stay adjacent and each keeps one job.
//!
//! What cycling MEANS here, and why it is not the role rule broken: the cycle
//! changes the CONVERSATION model to the model configured for the next role. That
//! is the `Conversation` slot legitimately switching the live agent, using another
//! slot's configured model as a named preset — "use my cheap model", "use my deep
//! model". It never writes the role's own slot, and an unassigned role is skipped
//! rather than offered as an empty stop.

use super::*;
use operant_core::model_roles::ModelSlot;

impl App {
    /// The role presets the cycle visits, in order, each with its configured
    /// model. Unassigned roles are skipped, not offered as empty stops.
    pub fn role_cycle(&self) -> Vec<(ModelSlot, String)> {
        let mut out = Vec::new();
        for slot in operant_core::model_roles::ALL_SLOTS {
            // `Conversation` is the live model, not a preset for it.
            if slot == ModelSlot::Conversation {
                continue;
            }
            let Some(model) = slot.assigned_models(&self.config).into_iter().next() else {
                continue;
            };
            out.push((slot, model));
        }
        out
    }

    /// Advance to the next role's configured model and apply it as the
    /// conversation model. Returns the role whose model was applied.
    pub fn cycle_role_models(&mut self) -> Option<(ModelSlot, String)> {
        let cycle = self.role_cycle();
        if cycle.is_empty() {
            self.set_status_notice(
                "no role has a model assigned — set one with /model @role".to_string(),
            );
            return None;
        }
        let current = self.model_name.clone();
        // The position AFTER the live model, so a model that happens to equal a
        // role's still advances rather than pinning.
        let at = cycle
            .iter()
            .position(|(_, model)| *model == current)
            .map(|index| index + 1)
            .unwrap_or(0);
        let (slot, model) = cycle[at % cycle.len()].clone();
        self.pending_model = Some(model.clone());
        self.set_status_notice(format!(
            "Model: {model} (from the {} role, applying…)",
            slot.name()
        ));
        Some((slot, model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::model_roles::ModelSlot;

    fn app_with_roles() -> App {
        let mut app = crate::tui::app::tests::make_app();
        app.config.agent.model = "live-model".to_string();
        // Two assigned role presets and one deliberately unassigned, so the
        // "skip unavailable roles" rule is exercised rather than assumed.
        app.config.recap.model = operant_core::config::AuxiliaryModelConfig {
            model: Some("cheap-model".to_string()),
            ..Default::default()
        };
        app.config.auxiliary_models.memory = Some(operant_core::config::AuxiliaryModelConfig {
            model: Some("deep-model".to_string()),
            ..Default::default()
        });
        app
    }

    #[test]
    fn the_cycle_lists_assigned_roles_and_skips_the_rest() {
        let app = app_with_roles();
        let cycle = app.role_cycle();
        let names: Vec<&str> = cycle.iter().map(|(slot, _)| slot.name()).collect();
        assert_eq!(
            names,
            vec!["smol", "slow"],
            "assigned roles in slot order; Conversation is the live model, not a preset"
        );
        assert!(
            cycle.iter().all(|(_, model)| !model.is_empty()),
            "an unassigned role is skipped, never offered as an empty stop: {cycle:?}"
        );
    }

    #[test]
    fn the_cycle_applies_the_next_role_model_and_names_its_role() {
        let mut app = app_with_roles();
        let (slot, model) = app
            .cycle_role_models()
            .expect("two assigned roles make a cycle");
        assert_eq!((slot, model.as_str()), (ModelSlot::Recap, "cheap-model"));
        assert_eq!(app.pending_model.as_deref(), Some("cheap-model"));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|notice| notice.contains("smol") && notice.contains("applying")),
            "the notice must name the role the model came from: {:?}",
            app.status_message
        );

        // Advancing again moves to the next role, not back to the first. The
        // drain applies `pending_model` and refreshes `model_name`; with no agent
        // here the test does the drain's one line so the second press sees what
        // the first one applied.
        app.model_name = model.clone();
        let (next_slot, next_model) = app.cycle_role_models().expect("still a cycle");
        assert_eq!(
            (next_slot, next_model.as_str()),
            (ModelSlot::Memory, "deep-model")
        );
    }

    #[test]
    fn no_assigned_role_says_so_instead_of_a_silent_no_op() {
        let mut app = crate::tui::app::tests::make_app();
        app.config.recap.model = operant_core::config::AuxiliaryModelConfig::default();
        app.config.auxiliary_models.memory = None;
        app.config.auxiliary_models.vision = None;
        app.config.auxiliary_models.compression = None;
        app.config.moa.aggregator = None;
        app.config.moa.references.clear();
        app.config.agent.fallback_models.clear();
        assert!(app.role_cycle().is_empty());
        assert!(app.cycle_role_models().is_none());
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|notice| notice.contains("no role has a model assigned")),
            "an empty cycle must be named, not silently ignored: {:?}",
            app.status_message
        );
    }
}

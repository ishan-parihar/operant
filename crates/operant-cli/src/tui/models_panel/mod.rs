//! The models panel: one row per model slot, with the scope axis beside it.
//!
//! (panels plan S3, shipped) The inline picker is a three-column list over the
//! model catalogue. It answers "which model?" well and "what is each ROLE
//! configured with?" badly — the slot rows it carries are assignments buried in
//! a model list, with no unassign, no assigned-vs-auto distinction, and a scope
//! axis you can only reach with a key chord.
//!
//! This module is the panel's DATA half: the rows, the scope selection, the
//! armed-slot assignment, and the one write path an assignment takes. The render
//! half is `render.rs`, and the key layer lives on `App` (`handle_models_panel_key`),
//! exactly as `session_hub/` is split — so what a row MEANS is testable without a
//! terminal.
//!
//! ## Why this is not a second model picker
//!
//! The inline picker persists nothing and switches the conversation. This panel
//! ASSIGNS slots: it writes `operant.toml` through the slot's own `persist_key()`,
//! and it touches the live agent only for the one slot whose
//! `switches_conversation()` is true. Those are different jobs, and omp keeps
//! them on separate surfaces for the same reason (`model-picker.ts` vs
//! `model-hub.ts`).
//!
//! ## What it deliberately does not do
//!
//! No priority chains. A slot with no assignment is `Auto`, which names the
//! consumer's documented fallback; inventing a resolver here would be a second
//! ranker beside `model_catalog::resolve`.

pub mod render;
pub use render::render_models_panel;

use crate::tui::model_scope::ModelScope;
use operant_core::config::AppConfig;
use operant_core::model_roles::{ALL_SLOTS, ModelSlot};

/// Whether a slot holds a model, and if so which.
///
/// Three states, not two: a slot that falls back to a default is not the same
/// as one with nothing behind it, and the operator's next move differs — leave
/// it alone, versus "this slot has no model and no fallback".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotState {
    /// The slot holds this model.
    Assigned(String),
    /// Unassigned, and the consumer has a documented fallback.
    Auto,
    /// Unassigned with nothing behind it.
    Empty,
}

/// One row of the Roles view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRow {
    pub slot: ModelSlot,
    pub state: SlotState,
    /// Whether assigning this slot also switches the running agent.
    ///
    /// `Conversation` is the only true, and the panel marks it: it is the one
    /// asymmetry in the slot set and the one the operator most needs to see
    /// before pressing Enter.
    pub switches_conversation: bool,
    /// The model ids this slot holds, in the order its consumer reads them.
    ///
    /// `MoaReference` is append-shaped, so a slot can carry more than one; the
    /// row shows the count rather than pretending to a single value.
    pub models: Vec<String>,
}

impl RoleRow {
    /// The status glyph: `●` assigned, `○` auto, `·` empty.
    pub fn marker(&self) -> &'static str {
        match &self.state {
            SlotState::Assigned(_) => "●",
            SlotState::Auto => "○",
            SlotState::Empty => "·",
        }
    }

    /// The right-hand note: the model, `auto`, or `—`.
    pub fn note(&self) -> String {
        match &self.state {
            SlotState::Assigned(model) if self.models.len() > 1 => {
                format!("{model} (+{} more)", self.models.len() - 1)
            }
            SlotState::Assigned(model) => model.clone(),
            SlotState::Auto => "auto".to_string(),
            SlotState::Empty => "—".to_string(),
        }
    }
}

/// Build the Roles view: one row per slot in [`ALL_SLOTS`] order.
///
/// Unlike [`operant_core::model_roles::slot_summaries`], an unassigned slot is
/// KEPT. That function feeds a "what can I switch to" list, where an empty row
/// is a dead end; this feeds an assignment surface, where an unassigned row is
/// the whole point — it is the thing the operator came to fill.
pub fn role_rows(cfg: &AppConfig) -> Vec<RoleRow> {
    ALL_SLOTS
        .into_iter()
        .map(|slot| {
            let models = slot.assigned_models(cfg);
            let state = if !models.is_empty() {
                SlotState::Assigned(models[0].clone())
            } else if slot_has_fallback(slot) {
                SlotState::Auto
            } else {
                SlotState::Empty
            };
            RoleRow {
                slot,
                state,
                switches_conversation: slot.switches_conversation(),
                models,
            }
        })
        .collect()
}

/// Whether an unassigned slot still has something behind it.
///
/// Operant has no priority chain, so this is a narrow claim: `Fallback` reads
/// the conversation model when its own slot is unset, and every other slot's
/// consumer has a plain default. Naming it keeps `Auto` honest rather than
/// decorative — a slot marked `auto` must actually fall back to something.
fn slot_has_fallback(slot: ModelSlot) -> bool {
    matches!(slot, ModelSlot::Fallback)
}

/// The panel's state. Owned by `App`; rendered full-frame while `visible`.
#[derive(Debug, Clone)]
pub struct ModelsPanel {
    pub visible: bool,
    /// Which scope the sidebar has selected.
    pub scope: ModelScope,
    /// The sidebar's cursor. Kept separate from `scope` so a rebuild that
    /// reorders the scope list does not silently move the operator's selection.
    pub scope_cursor: usize,
    /// The scopes the sidebar lists, in order. Rebuilt at open and after a
    /// catalogue refresh — the provider scopes come from the rows themselves.
    pub scopes: Vec<ModelScope>,
    /// One row per slot, in [`ALL_SLOTS`] order. Rebuilt at open and after a
    /// write, so a reopen never shows a stale assignment.
    pub roles: Vec<RoleRow>,
    /// The model ids the non-Roles scopes list, in catalogue order.
    pub models: Vec<String>,
    /// Every model the endpoint and the cache know about, kept so a scope
    /// switch can rebuild `models` without re-reading either source — and so
    /// the sidebar can show a scope's real size rather than the size of
    /// whatever scope happens to be selected.
    pub catalogue: Vec<String>,
    /// The recency list, newest first. Kept for the same reason: `Recent` is
    /// an intersection of this and `catalogue`, and the count must not change
    /// when the operator moves the cursor.
    pub recent: Vec<String>,
    /// The row the cursor is on, within the active scope.
    pub selected: usize,
    /// `Some(slot)` while the operator is choosing a model FOR that slot.
    ///
    /// The assignment flow is two steps — pick the slot, then pick the model —
    /// and this is what makes the second step know which slot it is filling.
    pub assigning: Option<ModelSlot>,
    /// The candidates the pick step offers, filtered by the armed slot's
    /// `accepts()` predicate. Computed once at arm time so the list cannot
    /// shift under the cursor.
    pub pick: Vec<String>,
    /// The cursor within `pick`.
    pub pick_cursor: usize,
    /// Why the last assignment was refused. A refusal the operator cannot read
    /// is a dead end, so the reason is carried rather than dropped.
    pub refusal: Option<String>,
    /// A note about the last write (what landed, where).
    pub notice: Option<String>,
}

impl Default for ModelsPanel {
    fn default() -> Self {
        Self {
            visible: false,
            scope: ModelScope::All,
            scope_cursor: 0,
            scopes: Vec::new(),
            roles: Vec::new(),
            models: Vec::new(),
            catalogue: Vec::new(),
            recent: Vec::new(),
            selected: 0,
            assigning: None,
            pick: Vec::new(),
            pick_cursor: 0,
            refusal: None,
            notice: None,
        }
    }
}

impl ModelsPanel {
    /// Open the panel on a scope.
    ///
    /// The entry point declares the operator's question: `/model roles` means
    /// "what is each role on?", so it opens on `Roles`; `/models` means "what
    /// can I switch to?", so it opens on `All`.
    ///
    /// `catalogue` is every model the endpoint and the cache know about;
    /// `recent` is the recency list, newest first. Both are read once at open
    /// rather than per frame, which is what keeps the render path syscall-free.
    pub fn open(
        &mut self,
        scope: ModelScope,
        cfg: &AppConfig,
        recent: &[String],
        catalogue: &[String],
    ) {
        self.visible = true;
        self.roles = role_rows(cfg);
        self.catalogue = catalogue.to_vec();
        self.recent = recent.to_vec();
        self.scopes = scope_list(catalogue);
        self.scope_cursor = self.scopes.iter().position(|s| *s == scope).unwrap_or(0);
        self.scope = scope;
        self.models = models_for(&self.scope, &self.catalogue, &self.recent);
        self.selected = 0;
        self.assigning = None;
        self.pick.clear();
        self.pick_cursor = 0;
        self.refusal = None;
        self.notice = None;
    }

    /// Re-read the slot rows after a write, keeping the operator's place.
    pub fn refresh(&mut self, cfg: &AppConfig) {
        self.roles = role_rows(cfg);
        self.selected = self.selected.min(self.row_count().saturating_sub(1));
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.assigning = None;
        self.pick.clear();
    }

    /// The sidebar rows.
    pub fn scope_rows(&self) -> &[ModelScope] {
        &self.scopes
    }

    /// How many rows a scope would show, for the sidebar's count column.
    ///
    /// Computed from the stored catalogue and recency rather than from
    /// `models`, because `models` only holds the ACTIVE scope: a count read
    /// from it would change every time the operator moved the sidebar cursor,
    /// which is how a count becomes a lie.
    pub fn scope_count(&self, scope: &ModelScope) -> usize {
        match scope {
            ModelScope::Roles => self.roles.len(),
            ModelScope::All => self.catalogue.len(),
            ModelScope::Recent => {
                models_for(&ModelScope::Recent, &self.catalogue, &self.recent).len()
            }
            ModelScope::Provider(_) => models_for(scope, &self.catalogue, &self.recent).len(),
        }
    }

    /// The row count the active scope shows.
    pub fn row_count(&self) -> usize {
        if self.assigning.is_some() {
            return self.pick.len();
        }
        match self.scope {
            ModelScope::Roles => self.roles.len(),
            _ => self.models.len(),
        }
    }

    /// Move the row cursor, clamped to the row count.
    pub fn move_selection(&mut self, delta: isize) {
        let total = self.row_count();
        if total == 0 {
            self.set_cursor(0);
            return;
        }
        let next = self.cursor() as isize + delta;
        self.set_cursor(next.clamp(0, total as isize - 1) as usize);
    }

    fn cursor(&self) -> usize {
        if self.assigning.is_some() {
            self.pick_cursor
        } else {
            self.selected
        }
    }

    fn set_cursor(&mut self, value: usize) {
        if self.assigning.is_some() {
            self.pick_cursor = value;
        } else {
            self.selected = value;
        }
    }

    /// Move the sidebar cursor and switch scope.
    ///
    /// The scope list is circular in one direction only at the ends: a sidebar
    /// that wraps is a sidebar whose bottom row is one keypress from its top,
    /// which is how an operator lands somewhere they did not choose. Clamp.
    pub fn move_scope(&mut self, delta: isize) {
        let total = self.scopes.len();
        if total == 0 {
            return;
        }
        let next = self.scope_cursor as isize + delta;
        let next = next.clamp(0, total as isize - 1) as usize;
        if next == self.scope_cursor {
            return;
        }
        self.scope_cursor = next;
        self.scope = self.scopes[next].clone();
        self.models = models_for(&self.scope, &self.catalogue, &self.recent);
        self.selected = 0;
        self.assigning = None;
        self.pick.clear();
    }

    /// The slot the cursor is on, when the Roles scope is showing.
    pub fn selected_slot(&self, cfg: &AppConfig) -> Option<ModelSlot> {
        let _ = cfg;
        self.roles.get(self.selected).map(|row| row.slot)
    }

    /// The model the cursor is on in a model scope (All / Provider / Recent).
    pub fn selected_model(&self) -> Option<&str> {
        self.models.get(self.selected).map(String::as_str)
    }

    /// The model the cursor is on in the pick step.
    pub fn selected_pick(&self) -> Option<&str> {
        self.pick.get(self.pick_cursor).map(String::as_str)
    }

    /// The slot to unassign under the cursor.
    ///
    /// Returns `None` — with a notice — when the slot is already unassigned, so
    /// `x` on an empty row says so rather than writing a no-op key. The removal
    /// itself is `operant_core::remove_config_key`, called by the key layer:
    /// the panel names WHAT, the seam decides HOW.
    ///
    /// The SLOT, not its persist key: the key layer needs both halves of the
    /// write — the file (remove the leaf) and the live config (the field falls
    /// back to its default) — and a caller holding only the string can do the
    /// first and silently skip the second. That was the defect this signature
    /// replaced: `x` removed the key, reported "cleared", and left
    /// `app.config` still holding the model, so the row kept its `●` and the
    /// process kept using the model until a restart.
    pub fn clear_slot(&mut self) -> Option<ModelSlot> {
        let row = self.roles.get(self.selected)?;
        if row.models.is_empty() {
            self.notice = Some(format!("{} is already unassigned", row.slot.name()));
            return None;
        }
        Some(row.slot)
    }

    /// Begin assigning a model to the selected slot.
    ///
    /// `catalogue` is filtered by the slot's `accepts()` predicate BEFORE the
    /// list is stored, so the pick step cannot offer a model the consumer would
    /// refuse — the refusal belongs at the moment of choosing, not after Enter.
    /// Unknown metadata is `Unverified`, not a refusal: the catalogue covers
    /// mapped providers only, and a missing row must not lock the operator out
    /// of their own config.
    pub fn begin_assign(&mut self, cfg: &AppConfig, catalogue: &[String]) -> Option<ModelSlot> {
        let slot = self.selected_slot(cfg)?;
        let provider =
            crate::provider::provider_from_url(cfg.client.base_url.trim_end_matches('/'))
                .map(|p| p.name)
                .unwrap_or_default();
        self.pick = candidates_for(slot, catalogue, |model| {
            operant_core::models_dev::cached_capabilities(provider, model)
        });
        self.pick_cursor = 0;
        self.assigning = Some(slot);
        self.refusal = None;
        Some(slot)
    }

    pub fn cancel_assign(&mut self) {
        self.assigning = None;
        self.pick.clear();
        self.pick_cursor = 0;
    }
}

/// The candidates a slot may be assigned, in catalogue order.
///
/// A model the slot's `accepts()` REFUSES is dropped; `Unverified` is kept.
/// The capability lookup is a parameter rather than a direct call so the rule
/// is testable without the operator's real models.dev cache — a test that read
/// that cache would assert against whatever provider happened to be configured
/// on the machine running it.
pub fn candidates_for<F>(slot: ModelSlot, catalogue: &[String], capabilities: F) -> Vec<String>
where
    F: Fn(&str) -> Option<operant_core::models_dev::ModelCapabilities>,
{
    catalogue
        .iter()
        .filter(|model| {
            !matches!(
                slot.accepts(capabilities(model).as_ref()),
                operant_core::model_roles::RoleAcceptance::Refuses(_)
            )
        })
        .cloned()
        .collect()
}

/// The sidebar: the fixed scopes, then one per provider that has a model.
///
/// `Recent` is always listed even when empty — a scope that vanished when it
/// had nothing would teach the operator that the sidebar reshuffles under them.
/// The render marks it `(0)` so the emptiness is visible rather than implied.
pub fn scope_list(catalogue: &[String]) -> Vec<ModelScope> {
    let mut out = vec![ModelScope::Recent, ModelScope::Roles, ModelScope::All];
    for model in catalogue {
        // The catalogue rows carry `provider/model` or a bare id; the provider
        // prefix is what the sidebar groups on, and a bare id groups under the
        // endpoint's own name, which the caller supplies as "".
        if let Some((provider, _)) = model.split_once('/') {
            let scope = ModelScope::Provider(provider.to_string());
            if !out.contains(&scope) {
                out.push(scope);
            }
        }
    }
    out
}

/// The models a scope lists.
pub fn models_for(scope: &ModelScope, catalogue: &[String], recent: &[String]) -> Vec<String> {
    match scope {
        ModelScope::Roles => Vec::new(),
        ModelScope::All => catalogue.to_vec(),
        ModelScope::Provider(name) => catalogue
            .iter()
            .filter(|model| model.split_once('/').is_some_and(|(p, _)| p == name))
            .cloned()
            .collect(),
        ModelScope::Recent => recent
            .iter()
            .filter(|model| catalogue.iter().any(|known| known == *model))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(agent_model: &str) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.agent.model = agent_model.to_string();
        cfg
    }

    #[test]
    fn every_slot_has_a_row_including_the_unassigned_ones() {
        // The distinction from `slot_summaries`, which drops empty slots. An
        // assignment surface must show the holes — they are what the operator
        // came to fill.
        let rows = role_rows(&AppConfig::default());
        assert_eq!(rows.len(), ALL_SLOTS.len());
        for slot in ALL_SLOTS {
            assert!(
                rows.iter().any(|row| row.slot == slot),
                "{} has no row",
                slot.name()
            );
        }
    }

    #[test]
    fn the_conversation_slot_is_the_only_one_marked_live() {
        let rows = role_rows(&cfg_with("gpt-4"));
        let live: Vec<&str> = rows
            .iter()
            .filter(|row| row.switches_conversation)
            .map(|row| row.slot.name())
            .collect();
        assert_eq!(
            live,
            vec![ModelSlot::Conversation.name()],
            "exactly one slot switches the running agent"
        );
    }

    #[test]
    fn an_assigned_slot_shows_its_model_and_a_filled_marker() {
        let rows = role_rows(&cfg_with("gpt-4"));
        let row = rows
            .iter()
            .find(|row| row.slot == ModelSlot::Conversation)
            .expect("the conversation row");
        assert_eq!(row.state, SlotState::Assigned("gpt-4".to_string()));
        assert_eq!(row.marker(), "●");
        assert_eq!(row.note(), "gpt-4");
    }

    #[test]
    fn an_unassigned_slot_with_a_fallback_reads_auto_not_empty() {
        // `Fallback` reads the conversation model when its own slot is unset, so
        // `auto` is the truth. Every other unassigned slot has a plain default
        // and no chain, which is `Empty` rather than a decorative `auto`.
        let rows = role_rows(&AppConfig::default());
        let fallback = rows
            .iter()
            .find(|row| row.slot == ModelSlot::Fallback)
            .expect("the fallback row");
        assert_eq!(fallback.state, SlotState::Auto);
        assert_eq!(fallback.marker(), "○");
        assert_eq!(fallback.note(), "auto");

        let vision = rows
            .iter()
            .find(|row| row.slot == ModelSlot::Vision)
            .expect("the vision row");
        assert_eq!(vision.state, SlotState::Empty);
        assert_eq!(vision.marker(), "·");
        assert_eq!(vision.note(), "—");
    }

    #[test]
    fn clearing_an_unassigned_slot_says_so_rather_than_writing() {
        let mut panel = ModelsPanel::default();
        panel.open(ModelScope::Roles, &AppConfig::default(), &[], &[]);
        // The cursor starts on `Conversation`, which IS assigned under default;
        // move to a slot that is not.
        let vision_row = role_rows(&AppConfig::default())
            .iter()
            .position(|row| row.slot == ModelSlot::Vision)
            .expect("the vision row");
        panel.selected = vision_row;
        assert_eq!(
            panel.clear_slot(),
            None,
            "an unassigned slot has no key to clear"
        );
        assert!(panel.notice.is_some(), "and the operator is told why");
    }

    #[test]
    fn clearing_an_assigned_slot_names_the_slot_whose_key_lands() {
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        panel.open(ModelScope::Roles, &cfg, &[], &[]);
        // The SLOT, not the key: the caller needs both halves of the write.
        // Handing back only the string is what let the clear update the file
        // and skip the live config.
        let slot = panel
            .clear_slot()
            .expect("the conversation slot is assigned");
        assert_eq!(slot, ModelSlot::Conversation);
        assert_eq!(slot.persist_key(), ModelSlot::Conversation.persist_key());
    }

    #[test]
    fn the_scope_decides_which_rows_the_panel_shows() {
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        panel.open(ModelScope::Roles, &cfg, &[], &[]);
        assert_eq!(panel.row_count(), ALL_SLOTS.len());
        // A model scope has no role rows: the catalogue is the picker's, not a
        // second copy of it here.
        panel.open(ModelScope::All, &cfg, &[], &["gpt-4".into()]);
        assert_eq!(panel.row_count(), 1);
        assert_eq!(panel.selected_model(), Some("gpt-4"));
    }

    #[test]
    fn assignment_is_a_two_step_flow_that_remembers_the_slot() {
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        panel.open(ModelScope::Roles, &cfg, &[], &["gpt-4".into()]);
        let slot = panel
            .begin_assign(&cfg, &["gpt-4".into()])
            .expect("a slot to assign");
        assert_eq!(slot, ModelSlot::Conversation);
        assert_eq!(panel.assigning, Some(slot));
        assert_eq!(panel.row_count(), 1, "the pick step lists the candidates");
        panel.cancel_assign();
        assert_eq!(panel.assigning, None, "Esc leaves nothing half-armed");
        assert!(panel.pick.is_empty());
    }

    #[test]
    fn the_pick_step_keeps_a_model_whose_capabilities_are_unknown() {
        // `Unverified` is not a refusal: the catalogue covers mapped providers
        // only, so "no row" is the common case and must not lock the operator
        // out of their own config. This test passes a lookup that finds NOTHING,
        // which is the offline default.
        let kept = candidates_for(
            ModelSlot::Vision,
            &["gpt-4".into(), "claude".into()],
            |_| None,
        );
        assert_eq!(
            kept,
            vec!["gpt-4".to_string(), "claude".to_string()],
            "unknown metadata must not drop a candidate"
        );
    }

    #[test]
    fn the_pick_step_drops_a_model_the_slot_refuses() {
        // The other half: a KNOWN refusal must not reach the list. Vision needs
        // image input, so a text-only model is dropped at the moment of
        // choosing rather than failing on the first image.
        let caps = |model: &str| {
            let mut caps = operant_core::models_dev::ModelCapabilities {
                context_window: None,
                max_output: None,
                supports_tools: true,
                supports_vision: false,
                supports_reasoning: false,
                supports_structured_output: false,
                input_modalities: vec!["text".to_string()],
                output_modalities: vec!["text".to_string()],
                cost_input_per_million: None,
                cost_output_per_million: None,
            };
            if model != "text-only" {
                caps.input_modalities.push("image".to_string());
                caps.supports_vision = true;
            }
            Some(caps)
        };
        let kept = candidates_for(
            ModelSlot::Vision,
            &["text-only".into(), "visionary".into()],
            caps,
        );
        assert_eq!(
            kept,
            vec!["visionary".to_string()],
            "a model the slot refuses is dropped, and the rest are kept"
        );
    }

    #[test]
    fn arming_a_slot_populates_the_pick_list_from_the_catalogue_it_was_given() {
        // The end-to-end arm: the list the picker draws is exactly the filtered
        // catalogue, so "Enter lists nothing" cannot pass as a working flow.
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        let catalogue = vec!["gpt-4".to_string(), "claude".to_string()];
        panel.open(ModelScope::Roles, &cfg, &[], &catalogue);
        let slot = panel
            .begin_assign(&cfg, &catalogue)
            .expect("a slot to assign");
        assert_eq!(slot, ModelSlot::Conversation);
        assert!(
            !panel.pick.is_empty(),
            "the pick step must hold the filtered catalogue, not an empty list"
        );
        assert_eq!(panel.pick_cursor, 0, "the cursor starts at the top");
    }

    #[test]
    fn the_cursor_stays_inside_the_row_count() {
        let mut panel = ModelsPanel::default();
        panel.open(ModelScope::Roles, &AppConfig::default(), &[], &[]);
        let total = panel.row_count();
        panel.move_selection(-5);
        assert_eq!(panel.selected, 0, "the cursor clamps at the top");
        panel.move_selection(999);
        assert_eq!(panel.selected, total - 1, "and at the bottom");
        panel.move_selection(1);
        assert_eq!(panel.selected, total - 1);
    }

    #[test]
    fn the_sidebar_clamps_at_both_ends_and_switches_scope() {
        let mut panel = ModelsPanel::default();
        panel.open(
            ModelScope::Roles,
            &cfg_with("gpt-4"),
            &[],
            &["openai/gpt-4".into(), "anthropic/claude".into()],
        );
        let scopes = panel.scopes.clone();
        assert!(scopes.len() >= 4, "fixed scopes plus two providers");
        let roles_at = scopes.iter().position(|s| *s == ModelScope::Roles).unwrap();
        panel.scope_cursor = roles_at;
        panel.move_scope(-1);
        assert_eq!(panel.scope, ModelScope::Recent, "up from Roles");
        panel.scope_cursor = 0;
        panel.move_scope(-1);
        assert_eq!(
            panel.scope_cursor, 0,
            "the sidebar does not wrap at the top"
        );
        panel.move_scope(9999);
        assert_eq!(
            panel.scope_cursor,
            scopes.len() - 1,
            "and clamps at the bottom"
        );
    }

    #[test]
    fn recent_selects_only_models_the_catalogue_knows() {
        // A recency entry for a model the endpoint no longer serves would be a
        // row that cannot be applied. Recent is an INTERSECTION, not a log.
        let catalogue = vec!["gpt-4".to_string(), "claude".to_string()];
        let recent = vec!["gpt-4".to_string(), "retired-model".to_string()];
        assert_eq!(
            models_for(&ModelScope::Recent, &catalogue, &recent),
            vec!["gpt-4".to_string()],
            "a retired model is dropped from Recent"
        );
    }

    #[test]
    fn the_panel_reopens_on_the_scope_its_entry_point_declared() {
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        panel.open(ModelScope::Roles, &cfg, &[], &[]);
        assert_eq!(panel.scope, ModelScope::Roles);
        assert_eq!(panel.scope_cursor, 1, "the sidebar starts on Roles");
        panel.open(ModelScope::All, &cfg, &[], &["gpt-4".into()]);
        assert_eq!(panel.scope, ModelScope::All);
        assert_eq!(panel.scope_cursor, 2);
    }
}

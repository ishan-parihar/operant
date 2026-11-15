//! The models panel: one row per model slot, with the scope axis beside it.
//!
//! (panels plan S3) The inline picker is a three-column list over the model
//! catalogue. It answers "which model?" well and "what is each ROLE configured
//! with?" badly — the slot rows it carries are assignments buried in a model
//! list, with no unassign, no assigned-vs-auto distinction, and a scope axis
//! you can only reach with a key chord.
//!
//! This module is the panel's DATA half: the rows, the scope selection, and the
//! one write path an assignment takes. The render half is a separate file, and
//! the key layer is a third, exactly as `session_hub/` is split — so what a row
//! MEANS is testable without a terminal.
//!
//! ## Why this is not a second model picker
//!
//! The inline picker persists nothing and switches the conversation. This panel
//! ASSIGNS slots: it writes `operant.toml` through the slot's own
//! `persist_key()`, and it touches the live agent only for the one slot whose
//! `switches_conversation()` is true. Those are different jobs, and omp keeps
//! them on separate surfaces for the same reason.
//!
//! ## What it deliberately does not do
//!
//! No priority chains. A slot with no assignment is `Auto`, which names the
//! consumer's documented fallback; inventing a resolver here would be a second
//! ranker beside `model_catalog::resolve`.

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
    ///
    /// (panels plan S3) Read by the render half, which is the next unit. Until
    /// it lands these two are test-only, and the allow says so rather than
    /// inviting a caller to invent a surface that paints them twice.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the render half is the next unit")
    )]
    pub fn marker(&self) -> &'static str {
        match &self.state {
            SlotState::Assigned(_) => "●",
            SlotState::Auto => "○",
            SlotState::Empty => "·",
        }
    }

    /// The right-hand note: the model, `auto`, or `—`.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the render half is the next unit")
    )]
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
    /// The slot the cursor is on, when the Roles scope is showing.
    pub selected: usize,
    /// `Some(slot)` while the operator is choosing a model FOR that slot.
    ///
    /// The assignment flow is two steps — pick the slot, then pick the model —
    /// and this is what makes the second step know which slot it is filling.
    pub assigning: Option<ModelSlot>,
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
            selected: 0,
            assigning: None,
            refusal: None,
            notice: None,
        }
    }
}

impl ModelsPanel {
    /// Open the panel on a scope.
    ///
    /// The entry point declares the operator's question: `/agents` means "what
    /// is each role on?", so it opens on `Roles`; `/model` means "which model?",
    /// so it opens on `All`.
    pub fn open(&mut self, scope: ModelScope) {
        self.visible = true;
        self.scope = scope;
        self.selected = 0;
        self.assigning = None;
        self.refusal = None;
        self.notice = None;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.assigning = None;
    }

    /// The rows the current scope selects.
    ///
    /// Only `Roles` has rows here: the provider and `All` scopes list MODELS,
    /// which come from the catalogue the inline picker already loads. This panel
    /// owns the role axis, and duplicating the catalogue would be a second copy
    /// of a fact the picker already holds.
    pub fn rows(&self, cfg: &AppConfig) -> Vec<RoleRow> {
        match self.scope {
            ModelScope::Roles => role_rows(cfg),
            _ => Vec::new(),
        }
    }

    /// Move the cursor, clamped to the row count.
    pub fn move_selection(&mut self, delta: isize, total: usize) {
        if total == 0 {
            self.selected = 0;
            return;
        }
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, total as isize - 1) as usize;
    }

    /// The slot the cursor is on.
    pub fn selected_slot(&self, cfg: &AppConfig) -> Option<ModelSlot> {
        self.rows(cfg).get(self.selected).map(|row| row.slot)
    }

    /// Clear the selected slot.
    ///
    /// Returns the config key that was cleared, so the caller can report where
    /// the write landed. `MoaReference` is append-shaped and is cleared by
    /// removing its entries rather than by writing an empty string, which the
    /// loader would read as "no change".
    pub fn clear_slot(&mut self, cfg: &AppConfig) -> Option<&'static str> {
        let slot = self.selected_slot(cfg)?;
        if slot.assigned_models(cfg).is_empty() {
            self.notice = Some(format!("{} is already unassigned", slot.name()));
            return None;
        }
        Some(slot.persist_key())
    }

    /// Begin assigning a model to the selected slot.
    pub fn begin_assign(&mut self, cfg: &AppConfig) -> Option<ModelSlot> {
        let slot = self.selected_slot(cfg)?;
        self.assigning = Some(slot);
        self.refusal = None;
        Some(slot)
    }

    pub fn cancel_assign(&mut self) {
        self.assigning = None;
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
        panel.open(ModelScope::Roles);
        let cfg = AppConfig::default();
        // The cursor starts on `Conversation`, which IS assigned under default;
        // move to a slot that is not.
        let vision_row = role_rows(&cfg)
            .iter()
            .position(|row| row.slot == ModelSlot::Vision)
            .expect("the vision row");
        panel.selected = vision_row;
        assert_eq!(
            panel.clear_slot(&cfg),
            None,
            "an unassigned slot has no key to clear"
        );
        assert!(panel.notice.is_some(), "and the operator is told why");
    }

    #[test]
    fn clearing_an_assigned_slot_names_the_key_that_lands() {
        let mut panel = ModelsPanel::default();
        panel.open(ModelScope::Roles);
        let cfg = cfg_with("gpt-4");
        let key = panel
            .clear_slot(&cfg)
            .expect("the conversation slot is assigned");
        assert_eq!(key, ModelSlot::Conversation.persist_key());
    }

    #[test]
    fn the_scope_decides_which_rows_the_panel_shows() {
        let mut panel = ModelsPanel::default();
        let cfg = cfg_with("gpt-4");
        panel.open(ModelScope::Roles);
        assert_eq!(panel.rows(&cfg).len(), ALL_SLOTS.len());
        // A model scope has no role rows: the catalogue is the picker's, not a
        // second copy of it here.
        panel.open(ModelScope::All);
        assert!(panel.rows(&cfg).is_empty());
    }

    #[test]
    fn assignment_is_a_two_step_flow_that_remembers_the_slot() {
        let mut panel = ModelsPanel::default();
        panel.open(ModelScope::Roles);
        let cfg = cfg_with("gpt-4");
        let slot = panel.begin_assign(&cfg).expect("a slot to assign");
        assert_eq!(slot, ModelSlot::Conversation);
        assert_eq!(panel.assigning, Some(slot));
        panel.cancel_assign();
        assert_eq!(panel.assigning, None, "Esc leaves nothing half-armed");
    }

    #[test]
    fn the_cursor_stays_inside_the_row_count() {
        let mut panel = ModelsPanel::default();
        panel.open(ModelScope::Roles);
        let cfg = AppConfig::default();
        let total = panel.rows(&cfg).len();
        panel.move_selection(-5, total);
        assert_eq!(panel.selected, 0, "the cursor clamps at the top");
        panel.move_selection(999, total);
        assert_eq!(panel.selected, total - 1, "and at the bottom");
        panel.move_selection(1, 0);
        assert_eq!(panel.selected, 0, "an empty scope has no cursor");
    }
}

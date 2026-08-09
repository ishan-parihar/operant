// model_picker/state.rs — ModelPickerState struct + methods + Default.
//
// Extracted from the model_picker.rs monolith.

use super::*;

pub struct ModelPickerState {
    pub visible: bool,
    pub selected_idx: usize,
    pub models: Vec<ModelEntry>,
    pub title: String,
    /// Live filter typed by the user.
    pub filter: String,
    /// Current effort level for models that support extended thinking.
    pub effort_level: EffortLevel,
    /// Whether fast mode is currently active.
    pub fast_mode: bool,
    /// The currently locked fast-mode model, if fast mode is active.
    pub fast_mode_model: Option<String>,
    /// `true` once the dynamic model list has been loaded from the API.
    pub models_loaded: bool,
    /// `true` while the background fetch is in flight.
    pub loading_models: bool,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl ModelPickerState {
    pub(crate) fn default_models() -> Vec<ModelEntry> {
        vec![
            model_entry(
                "claude-3-5-sonnet-20241022",
                "Claude 3.5 Sonnet",
                "Latest Sonnet model",
            ),
            model_entry(
                "claude-3-5-haiku-20241022",
                "Claude 3.5 Haiku",
                "Fast and capable",
            ),
        ]
    }

    /// Create a new picker with the default model list (not yet visible).
    pub fn new() -> Self {
        Self {
            visible: false,
            selected_idx: 0,
            models: Self::default_models(),
            title: "Select model".to_string(),
            filter: String::new(),
            effort_level: EffortLevel::Normal,
            fast_mode: false,
            fast_mode_model: None,
            models_loaded: false,
            loading_models: false,
        }
    }

    /// Open the overlay.
    ///
    /// `current_model` is highlighted as active; `current_effort` and
    /// `fast_mode` are carried over from app state so the user sees the live
    /// values.
    #[allow(dead_code)] // Prepared for model picker initialization
    pub fn open(&mut self, current_model: &str) {
        self.open_with_state(current_model, EffortLevel::Normal, false);
    }

    /// Open the overlay with full state context.
    #[allow(dead_code)] // Prepared for model picker with effort/fast mode state
    pub fn open_with_state(&mut self, current_model: &str, effort: EffortLevel, fast_mode: bool) {
        self.open_with_title("Select model", current_model, effort, fast_mode);
    }

    pub fn open_with_title(
        &mut self,
        title: impl Into<String>,
        current_model: &str,
        effort: EffortLevel,
        fast_mode: bool,
    ) {
        for m in &mut self.models {
            m.is_current = m.id == current_model;
        }
        self.selected_idx = self.models.iter().position(|m| m.is_current).unwrap_or(0);
        self.title = title.into();
        self.filter.clear();
        self.effort_level = effort;
        self.fast_mode = fast_mode;
        self.fast_mode_model = fast_mode.then_some(current_model.to_string());
        self.visible = true;
    }

    /// Close the overlay without selecting.
    pub fn close(&mut self) {
        self.visible = false;
        self.filter.clear();
    }

    pub fn is_selected_fast_mode_model(&self, model_id: &str) -> bool {
        self.fast_mode_model.as_deref() == Some(model_id)
    }

    /// Move selection up one row (wraps to last if at top).
    pub fn select_prev(&mut self) {
        let count = self.filtered_models().len();
        cycle_prev(&mut self.selected_idx, count);
    }

    /// Move selection down one row (wraps to first if at bottom).
    pub fn select_next(&mut self) {
        let count = self.filtered_models().len();
        cycle_next(&mut self.selected_idx, count);
    }

    pub fn select_first(&mut self) {
        self.selected_idx = 0;
    }

    pub fn select_last(&mut self) {
        let count = self.filtered_models().len();
        self.selected_idx = count.saturating_sub(1);
    }

    /// Cycle effort level forward (→ key).
    pub fn effort_next(&mut self) {
        let filtered = self.filtered_models();
        let id = filtered
            .get(self.selected_idx)
            .map(|m| m.id.as_str())
            .unwrap_or("");
        let supports_max = model_supports_max_effort(id);
        self.effort_level = self.effort_level.next(supports_max);
    }

    /// Cycle effort level backward (← key).
    pub fn effort_prev(&mut self) {
        let filtered = self.filtered_models();
        let id = filtered
            .get(self.selected_idx)
            .map(|m| m.id.as_str())
            .unwrap_or("");
        let supports_max = model_supports_max_effort(id);
        self.effort_level = self.effort_level.prev(supports_max);
    }

    /// Confirm the current selection.
    ///
    /// Returns `(model_id, effort)` where `effort` is `None` for models that
    /// do not support extended thinking.  Closes the picker.
    ///
    /// Returns the selected model; the caller is responsible for persisting it
    /// in the correct provider-aware format.
    pub fn confirm(&mut self) -> Option<(String, Option<EffortLevel>)> {
        let filtered = self.filtered_models();
        let custom = self.filter.trim();
        if filtered.is_empty() {
            if custom.is_empty() {
                return None;
            }
            let id = custom.to_string();
            self.close();
            return Some((id, None));
        }
        let entry = filtered.get(self.selected_idx)?;
        let id = entry.id.clone();
        let effort = if model_supports_effort(&id) {
            Some(self.effort_level)
        } else {
            None
        };
        // If user chose a model other than the fast-mode model while fast mode is
        // active, the caller should turn off fast mode (mirrors TS behaviour).
        self.close();
        Some((id, effort))
    }

    /// Append a character to the filter string and reset the selection.
    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected_idx = 0;
    }

    /// Remove the last character from the filter string.
    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.selected_idx = 0;
    }

    /// Return models that match the current filter (case-insensitive).
    pub fn filtered_models(&self) -> Vec<&ModelEntry> {
        if self.filter.is_empty() {
            return self.models.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.models
            .iter()
            .filter(|m| {
                m.id.to_lowercase().contains(needle.as_str())
                    || m.display_name.to_lowercase().contains(needle.as_str())
                    || m.description.to_lowercase().contains(needle.as_str())
            })
            .collect()
    }

    /// Replace the model list with dynamically loaded entries.
    ///
    /// Called by the app event loop when the background fetch completes.
    /// Resets `loading_models` and sets `models_loaded`.
    pub fn set_models(&mut self, entries: Vec<ModelEntry>) {
        self.models = entries;
        self.loading_models = false;
        self.models_loaded = true;
        // Keep selected_idx in bounds.
        let count = self.filtered_models().len();
        if count > 0 && self.selected_idx >= count {
            self.selected_idx = count - 1;
        }
    }
}

impl Default for ModelPickerState {
    fn default() -> Self {
        Self::new()
    }
}

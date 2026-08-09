// model_picker/tests.rs — Unit tests for the model picker.
//
// Extracted from the model_picker.rs monolith.

use super::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn make_picker_with_current(current: &str) -> ModelPickerState {
    let mut p = ModelPickerState::new();
    p.open(current);
    p
}

// 1. Default model list is non-empty and contains expected IDs.
#[test]
fn default_models_are_populated() {
    let models = ModelPickerState::default_models();
    assert!(!models.is_empty(), "default model list must not be empty");
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"claude-3-5-sonnet-20241022"));
    assert!(ids.contains(&"claude-3-5-haiku-20241022"));
}

// 2. open() marks exactly one model as current.
#[test]
fn open_marks_current_model() {
    let mut p = ModelPickerState::new();
    p.open("claude-3-5-sonnet-20241022");
    let current_count = p.models.iter().filter(|m| m.is_current).count();
    assert_eq!(current_count, 1);
    assert!(
        p.models
            .iter()
            .find(|m| m.id == "claude-3-5-sonnet-20241022")
            .unwrap()
            .is_current
    );
}

#[test]
fn open_with_title_updates_dialog_title() {
    let mut p = ModelPickerState::new();
    p.open_with_title("Anthropic", "claude-sonnet-4-6", EffortLevel::Normal, false);
    assert_eq!(p.title, "Anthropic");
}

#[test]
fn open_with_fast_mode_tracks_locked_model() {
    let mut p = ModelPickerState::new();
    p.open_with_state("gpt-4o-mini", EffortLevel::Normal, true);
    assert_eq!(p.fast_mode_model.as_deref(), Some("gpt-4o-mini"));
    assert!(p.is_selected_fast_mode_model("gpt-4o-mini"));
    assert!(!p.is_selected_fast_mode_model("gpt-4o"));
}

// 3. open() with an unknown model ID marks none as current and sets idx=0.
#[test]
fn open_unknown_model_selects_first() {
    let mut p = ModelPickerState::new();
    p.open("unknown-model");
    assert_eq!(p.selected_idx, 0);
    assert!(p.models.iter().all(|m| !m.is_current));
}

// 4. select_next() wraps around to 0 after the last entry.
#[test]
fn select_next_wraps() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    let total = p.filtered_models().len();
    p.selected_idx = total - 1;
    p.select_next();
    assert_eq!(p.selected_idx, 0);
}

// 5. select_prev() wraps around to last after idx 0.
#[test]
fn select_prev_wraps() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    p.selected_idx = 0;
    p.select_prev();
    let total = p.filtered_models().len();
    assert_eq!(p.selected_idx, total - 1);
}

// 6. filter reduces visible entries.
#[test]
fn filter_reduces_results() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    for c in "sonnet".chars() {
        p.push_filter_char(c);
    }
    let all = p.models.len();
    let filtered = p.filtered_models();
    assert!(
        filtered.len() < all,
        "filter should reduce the result count"
    );
    assert!(!filtered.is_empty(), "at least one sonnet model must match");
    for m in &filtered {
        let haystack = format!("{} {} {}", m.id, m.display_name, m.description).to_lowercase();
        assert!(
            haystack.contains("sonnet"),
            "model '{}' does not match filter",
            m.id
        );
    }
}

// 7. pop_filter_char removes last char.
#[test]
fn pop_filter_char_removes_last() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    p.push_filter_char('h');
    p.push_filter_char('a');
    p.push_filter_char('i');
    assert_eq!(p.filter, "hai");
    p.pop_filter_char();
    assert_eq!(p.filter, "ha");
}

// 8. confirm() returns selected model ID and closes the picker.
#[test]
fn confirm_returns_id_and_closes() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    p.selected_idx = 0;
    let first_id = p.filtered_models()[0].id.clone();
    let result = p.confirm();
    assert_eq!(result.map(|(id, _)| id), Some(first_id));
    assert!(!p.visible, "picker should be closed after confirm");
}

// 9. confirm() on empty filter list uses custom model when filter is set.
#[test]
fn confirm_empty_filter_returns_none() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    p.filter = "zzznomatch999".to_string();
    p.selected_idx = 0;
    let result = p.confirm();
    assert_eq!(result.map(|(id, _)| id), Some("zzznomatch999".to_string()));
}

// 10. close() clears filter and hides overlay.
#[test]
fn close_clears_state() {
    let mut p = make_picker_with_current("claude-opus-4-6");
    p.push_filter_char('x');
    p.close();
    assert!(!p.visible);
    assert!(p.filter.is_empty());
}

// 11. effort cycling works for effort-supporting models.
#[test]
fn effort_cycles_correctly() {
    let mut p = make_picker_with_current("claude-sonnet-4-6");
    // sonnet-4-6 supports effort but not max
    assert_eq!(p.effort_level, EffortLevel::Normal);
    p.effort_next();
    assert_eq!(p.effort_level, EffortLevel::High);
    p.effort_next();
    // no max for sonnet → wraps to Low
    assert_eq!(p.effort_level, EffortLevel::Low);
}

// 12. Opus supports max effort.
#[test]
fn opus_supports_max_effort() {
    assert!(model_supports_max_effort("claude-opus-4-6"));
    assert!(!model_supports_max_effort("claude-sonnet-4-6"));
    assert!(!model_supports_max_effort("claude-haiku-4-5"));
}

// 13. Non-effort models return None from effective_effort.
#[test]
fn haiku_has_no_effort() {
    let mut p = make_picker_with_current("claude-3-5-haiku-20241022");
    p.selected_idx = p
        .models
        .iter()
        .position(|m| m.id == "claude-3-5-haiku-20241022")
        .unwrap();
    assert!(!model_supports_effort("claude-3-5-haiku-20241022"));
    let effort = p.confirm();
    assert!(effort.is_some_and(|(_, e)| e.is_none()));
}

// 14. render_model_picker does not panic for a default-area call.
#[test]
fn render_does_not_panic() {
    let mut p = ModelPickerState::new();
    p.open("claude-3-5-sonnet-20241022");
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    render_model_picker(&p, area, &mut buf);
}

// 15. render does nothing when not visible.
#[test]
fn render_noop_when_hidden() {
    let p = ModelPickerState::new();
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    render_model_picker(&p, area, &mut buf);
    for cell in buf.content() {
        assert_eq!(
            cell.symbol(),
            " ",
            "buffer should be empty when picker is hidden"
        );
    }
}

// 16. models_for_provider_from_registry returns the bundled snapshot's
//     entries for each well-known provider.  Specific model IDs aren't
//     asserted here because the snapshot is regenerated periodically;
//     instead we check the family / provider-namespace shape.
#[test]
fn models_for_provider_anthropic() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let models = models_for_provider_from_registry("anthropic", &registry);
    assert!(!models.is_empty(), "anthropic must yield models");
    assert!(
        models.iter().any(|m| m.id.starts_with("claude")),
        "anthropic should expose at least one claude-* model"
    );
}

#[test]
fn models_for_provider_openai() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let models = models_for_provider_from_registry("openai", &registry);
    assert!(!models.is_empty());
    // Must NOT contain Claude models
    assert!(!models.iter().any(|m| m.id.contains("claude")));
    // Should contain at least one gpt-* or o-series id
    assert!(
        models
            .iter()
            .any(|m| m.id.starts_with("gpt-") || m.id.starts_with("o3") || m.id.starts_with("o4")),
        "openai should expose at least one gpt/o-series model"
    );
}

#[test]
fn models_for_provider_unknown_returns_default() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let models = models_for_provider_from_registry("some-unknown-provider", &registry);
    assert!(!models.is_empty());
    assert_eq!(models[0].id, "default");
}

// 17. default_model_for_provider returns prefixed models for non-anthropic.
#[test]
fn default_model_for_provider_openai() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let m = default_model_for_provider("openai", &registry);
    assert!(
        m.starts_with("openai/"),
        "openai default must be prefixed: {m}"
    );
}

#[test]
fn default_model_for_provider_anthropic_bare() {
    // Anthropic models are bare (no prefix) for backwards compat.
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let m = default_model_for_provider("anthropic", &registry);
    assert!(!m.contains('/'), "anthropic default must be bare: {m}");
    assert!(
        m.starts_with("claude"),
        "anthropic default must be a claude variant: {m}"
    );
}

#[test]
fn default_model_for_provider_unknown_falls_back() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    assert_eq!(
        default_model_for_provider("some-self-hosted-thing", &registry),
        "some-self-hosted-thing/default"
    );
}

// 18. set_models replaces the model list.
#[test]
fn set_models_replaces_list() {
    let registry = crate::tui::adapter_types::ModelRegistry::new();
    let mut p = ModelPickerState::new();
    let openai_models = models_for_provider_from_registry("openai", &registry);
    p.set_models(openai_models);
    let ids: Vec<&str> = p.models.iter().map(|m| m.id.as_str()).collect();
    assert!(!ids.iter().any(|id| id.contains("claude")));
}

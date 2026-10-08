// Vendored from jcode (crates/operant-tui-core/src/keybind.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only the platform Alt/Option label helpers the ported renderers use:
// MACOS_OPTION_SYMBOL (:8), alt_label (:11), alt_label_for_platform (:15),
// alt_chord (:19), alt_chord_lower (:24).
// [port-decision] gated, not ported: the per-config keybinding registry
// (operant-tui/src/tui/keybind.rs, 814 LOC — effort_switch_keys_label :267,
// load_new_terminal_key :629, load_open_resume_key :663,
// side_panel_toggle_key_label :450, diagram_pane_visibility_key_label :454,
// EFFORT_HELP :438 — plus operant-tui-core's parse/format machinery,
// ~400 LOC reachable set incl. ToggleBinding/ToggleKeys and a keybindings
// config section). Exceeds the ~120 LOC leaf budget; callers are help-overlay
// labels only, gated at their call sites; re-activate at cutover.
pub const MACOS_OPTION_SYMBOL: &str = "⌥";

/// Platform label for the Alt/Option modifier: `⌥` on macOS, `Alt` elsewhere.
pub fn alt_label() -> &'static str {
    alt_label_for_platform(cfg!(target_os = "macos"))
}

pub fn alt_label_for_platform(is_macos: bool) -> &'static str {
    if is_macos { MACOS_OPTION_SYMBOL } else { "Alt" }
}

/// Build a title-case Alt chord label, e.g. `Alt+N` or `⌥+N`.
pub fn alt_chord(keys: &str) -> String {
    format!("{}+{}", alt_label(), keys)
}

/// Build a lowercase Alt chord label for compact inline hints, e.g. `alt+n`
/// or `⌥+n`.
pub fn alt_chord_lower(keys: &str) -> String {
    let label = alt_label();
    if label == MACOS_OPTION_SYMBOL {
        format!("{label}+{keys}")
    } else {
        format!("{}+{keys}", label.to_ascii_lowercase())
    }
}

// [port-decision] batch-4: the two toggle-key labels the ported renderers
// call un-gated (ui_pinned.rs:588, ui_diagram_pane.rs:834). Bodies from
// crates/operant-tui/src/tui/keybind.rs:450-:458, re-rooted: upstream's
// operant_tui_core::keybind::alt_chord is this same module's alt_chord.

/// Upstream keybind.rs:450. The side-panel toggle is the fixed Alt+M chord.
pub(crate) fn side_panel_toggle_key_label() -> String {
    alt_chord("M")
}

/// Upstream keybind.rs:454, degraded arm: upstream reads the per-config
/// keybinding registry (load_toggle_keys -> cfg.keybindings
/// .diagram_pane_visibility_toggle) and falls back to Alt+Shift+M; the
/// registry is deliberately not ported (see the gate notes at
/// ui_overlays.rs:501-:632), so this returns the upstream default chord.
/// Re-activate the registry read when the keybinding registry ports.
pub(crate) fn diagram_pane_visibility_key_label() -> String {
    alt_chord("Shift+M")
}

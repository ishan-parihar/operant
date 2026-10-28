// (plan 01, S1b) prints while the TUI owns the screen corrupt the diff —
// cells written outside ratatui's buffer never repaint until something
// else touches them (the "heals on scroll" ghost-row shape). Route through
// tracing or a status notice; the only exemptions are the #[expect] sites
// with a reason (logger self-failure, where tracing is dead) and cfg(test).
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod adapter_types;
pub mod provider;
pub mod vendor;

pub mod color_depth;
pub mod debug;

pub mod agents_view;
pub mod app;
pub mod ask_user_dialog;
// The background-task row surface is wired: `App` carries the registry
// (`app/mod.rs`), `App::new` constructs it (`app/init.rs`), and the render pass
// refreshes and paints it (`render/mod.rs`). `tests::render_pass_still_wires_the_registry`
// pins that last call site, so the rows cannot silently stop being shown again.
pub mod background_tasks;
pub mod banner;
pub mod bypass_permissions_dialog;
pub mod clipboard;
pub mod context_viz;
pub mod custom_provider_dialog;
pub mod dialog_select;
pub mod dialogs;
pub mod diff_viewer;
pub mod export_dialog;
pub mod figures;
pub mod fold_search;
pub mod image_paste;
pub mod image_render;
pub mod input;
pub mod input_history;
pub mod permissions_picker;
pub mod rewind_selector;
// W1 seam of docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md: jcode's message
// data model (`DisplayMessage`, prepared chat frames, viewport anchors,
// tool-name display helpers), ported verbatim from jcode @ 0a9dc7805. It
// lands before its W3 renderer consumers, so the module is dead until the
// transcript renderers arrive; `expect` rather than `allow` so it fails
// loudly the moment W3 wires in. The module's own tests read its items, so
// the expectation is cfg-gated off in test builds (same as copy_targets).
pub mod external_editor;
pub mod file_discovery;
pub mod journey_view;
pub mod keybindings;
pub mod keyboard_enhancement;
pub mod mcp_view;
pub mod messages;
pub mod model_effort;
pub mod notifications;
pub mod operant_anim;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lands before its cutover wiring (App-seam adaptation)"
    )
)]
pub mod operant_app;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lands before its W3 transcript-renderer consumers"
    )
)]
pub mod operant_markdown;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "lands before its W3 renderer consumers")
)]
pub mod operant_model;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "lands before its W3/W5 renderer consumers")
)]
pub mod operant_render;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lands before its W3 transcript-renderer consumers"
    )
)]
pub mod operant_render_core;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "lands before its cutover wiring (App-seam adaptation)"
    )
)]
pub mod operant_ui;
pub mod osc8;
pub mod overlays;
pub mod pinned_images;
pub mod plugins_hub;
pub mod prompt_input;
pub mod redraw;
pub mod render;
pub mod rustle;
pub mod session_hub;
pub mod settings_screen;
pub mod skills_view;
pub mod slash_usage;
pub mod space;
pub mod stats_dialog;
pub mod terminal_setup;
// (iter-211: feedback_survey module deleted — no telemetry backend, YAGNI)
pub mod free_mode_dialog;
pub mod import_config_dialog;
pub mod key_input_dialog;
pub mod memory_file_selector;
pub mod theme_colors;
pub mod theme_screen;
pub mod usage_overlay;

pub use adapter_types::LaunchMode;
pub use adapter_types::TuiApp;

// Deliverable 1.10 of docs/PLAN-TUI-OVERHAUL.md: the release-mode
// characterisation of the per-frame substitution pass. Test-only and
// `#[ignore]`d inside, so it costs the normal build and test run nothing.
#[cfg(test)]
mod substitution_cost;

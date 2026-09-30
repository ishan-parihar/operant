pub mod adapter_types;
pub mod provider;
pub mod vendor;

pub mod bridge_state;
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
pub mod device_auth_dialog;
pub mod dialog_select;
pub mod dialogs;
pub mod diff_viewer;
pub mod effort_picker;
pub mod export_dialog;
pub mod figures;
pub mod image_paste;
pub mod image_render;
pub mod input;
pub mod input_history;
pub mod journey_view;
pub mod keybindings;
pub mod mcp_view;
pub mod messages;
pub mod model_picker;
pub mod notifications;
pub mod osc8;
pub mod overlays;
pub mod pinned_images;
pub mod plugins_hub;
pub mod prompt_input;
pub mod redraw;
pub mod render;
pub mod rustle;
pub mod session_branching;
pub mod session_browser;
pub mod settings_screen;
pub mod skills_view;
pub mod slash_usage;
pub mod space;
pub mod stats_dialog;
pub mod terminal_setup;
pub mod transcript_turn;
pub mod virtual_list;
// (iter-211: feedback_survey module deleted — no telemetry backend, YAGNI)
pub mod free_mode_dialog;
pub mod hooks_config_menu;
pub mod import_config_dialog;
pub mod key_input_dialog;
pub mod latex;
pub mod memory_file_selector;
pub mod mermaid;
pub mod theme_colors;
pub mod theme_screen;
pub mod usage_overlay;
pub mod voice_mode_notice;

pub use adapter_types::LaunchMode;
pub use adapter_types::TuiApp;

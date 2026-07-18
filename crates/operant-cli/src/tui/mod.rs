#![allow(dead_code)] // TUI module: many items prepared for future use in the evolving UI framework

pub mod adapter_types;
pub mod provider;

pub mod debug;

pub mod agents_view;
pub mod app;
pub mod ask_user_dialog;
pub mod banner;
pub mod bypass_permissions_dialog;
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
pub mod input;
pub mod input_history;
pub mod journey_view;
pub mod mcp_view;
pub mod message_copy;
pub mod messages;
pub mod model_picker;
pub mod notifications;
pub mod osc8;
pub mod overlays;
pub mod plugins_hub;
pub mod prompt_input;
pub mod render;
pub mod rustle;
pub mod session_branching;
pub mod session_browser;
pub mod settings_screen;
pub mod skills_view;
pub mod slash_usage;
pub mod stats_dialog;
pub mod tasks_overlay;
pub mod transcript_turn;
pub mod virtual_list;
// (iter-211: feedback_survey module deleted — no telemetry backend, YAGNI)
pub mod free_mode_dialog;
pub mod hooks_config_menu;
pub mod import_config_dialog;
pub mod key_input_dialog;
pub mod memory_file_selector;
pub mod theme_screen;
pub mod voice_mode_notice;

pub use adapter_types::LaunchMode;
pub use adapter_types::TuiApp;

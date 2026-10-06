// Vendored from jcode (crates/jcode-tui/src/tui/mod.rs:110-205), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 (batch-4 sweep
// tail) — the kitty/tmux keyboard-enhancement request/reset block plus
// `reapply_terminal_modes_to`. Re-root: crate::logging ->
// crate::tui::jcode_app::logging.

use crate::tui::jcode_app::logging;

fn keyboard_enhancement_flags() -> crossterm::event::KeyboardEnhancementFlags {
    use crossterm::event::KeyboardEnhancementFlags;

    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

/// Intentionally avoid REPORT_ALL_KEYS_AS_ESCAPE_CODES for now. When that flag is enabled,
/// terminals such as kitty/Alacritty/Warp can report printable keys as a base key plus
/// modifiers instead of the final text produced by the active keyboard layout. Crossterm does
/// not yet expose kitty's associated text / alternate key data, so we cannot safely reconstruct
/// shifted symbols for every keyboard layout. Prefer the terminal-delivered printable character
/// and only synthesize ASCII letter casing in the input fallback.
///
/// Returns whether the requests were written, not whether the terminal supports them.
pub fn enable_keyboard_enhancement() -> bool {
    let result = enable_keyboard_enhancement_to(&mut std::io::stdout(), inside_tmux()).is_ok();
    logging::info(&format!(
        "Keyboard enhancement request: {}",
        if result { "sent" } else { "FAILED" }
    ));
    result
}

fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

fn enable_keyboard_enhancement_to(
    writer: &mut impl std::io::Write,
    inside_tmux: bool,
) -> std::io::Result<()> {
    use crossterm::event::PushKeyboardEnhancementFlags;
    request_tmux_extended_keys_to(writer, inside_tmux)?;
    crossterm::execute!(
        writer,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    )
}

/// Reset tmux extended keys and pop Kitty keyboard reporting.
pub fn disable_keyboard_enhancement() {
    let _ = disable_keyboard_enhancement_to(&mut std::io::stdout(), inside_tmux());
}

fn disable_keyboard_enhancement_to(
    writer: &mut impl std::io::Write,
    inside_tmux: bool,
) -> std::io::Result<()> {
    if inside_tmux {
        writer.write_all(b"\x1b[>4;0m")?;
    }
    crossterm::execute!(writer, crossterm::event::PopKeyboardEnhancementFlags)
}

fn request_tmux_extended_keys_to(
    writer: &mut impl std::io::Write,
    inside_tmux: bool,
) -> std::io::Result<()> {
    if inside_tmux {
        // `extended-keys on` needs modifyOtherKeys opt-in, not a Kitty push.
        writer.write_all(b"\x1b[>4;2m")?;
    }
    Ok(())
}

fn reapply_keyboard_enhancement_to(
    writer: &mut impl std::io::Write,
    inside_tmux: bool,
) -> std::io::Result<()> {
    request_tmux_extended_keys_to(writer, inside_tmux)?;
    write!(writer, "\x1b[={}u", keyboard_enhancement_flags().bits())
}

/// Reassert terminal modes that terminals may clear while the TUI remains alive.
///
/// Kitty keyboard enhancement uses its `set` form rather than the stack-based
/// `push`, keeping the shutdown pop balanced. Enabling focus reporting may itself
/// produce a focus event, so focus-event handlers must pass `focus_change = false`.
pub(crate) fn reapply_terminal_modes_to(
    writer: &mut impl std::io::Write,
    mouse_capture: bool,
    keyboard_enhanced: bool,
    focus_change: bool,
) -> std::io::Result<()> {
    use crossterm::QueueableCommand;
    use crossterm::event::{EnableBracketedPaste, EnableFocusChange, EnableMouseCapture};

    writer.queue(EnableBracketedPaste)?;
    if focus_change {
        writer.queue(EnableFocusChange)?;
    }
    if mouse_capture {
        writer.queue(EnableMouseCapture)?;
        // Crossterm toggles Win32 console mouse input on Windows, but ConPTY
        // hosts such as VS Code also need the VT tracking modes reasserted.
        #[cfg(windows)]
        writer.write_all(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h")?;
    }
    if keyboard_enhanced {
        reapply_keyboard_enhancement_to(writer, inside_tmux())?;
    }
    writer.flush()
}

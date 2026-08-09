#![allow(dead_code)] // Foundation modules for future multi-crate extraction — wired in Phase 2I
//! Terminal setup/teardown with panic hook for the ratatui-based TUI.
//!
//! Without a panic hook, any panic in rendering code leaves the terminal in
//! raw mode with mouse capture enabled — the user sees garbage input until
//! they run `reset`. This module installs a hook that restores the terminal
//! before printing the panic message.

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether mouse capture was enabled during setup. Used by the panic hook
/// and restore path to know whether to emit `DisableMouseCapture`.
static MOUSE_CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Set up the terminal for TUI mode (raw mode + alternate screen + mouse capture).
///
/// Also installs a panic hook that restores the terminal before printing the
/// panic message. Only restores on the main thread to avoid destroying a live
/// TUI display when a background tokio task panics.
pub fn setup_terminal(mouse_capture: bool) -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    MOUSE_CAPTURE_ACTIVE.store(mouse_capture, Ordering::Relaxed);

    // Chain on top of any existing hook (e.g. from a previous call or test harness).
    let main_thread_id = std::thread::current().id();
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if std::thread::current().id() == main_thread_id {
            let _ = disable_raw_mode();
            let _ = restore_terminal_cleanup();
            let _ = execute!(io::stdout(), crossterm::cursor::Show);
        }
        original_hook(panic_info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen, EnableMouseCapture,)?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to its original state.
pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    restore_terminal_cleanup()?;
    terminal.show_cursor()?;
    Ok(())
}

/// Restore terminal capabilities (alternate screen, mouse capture).
/// Used by both restore_terminal and the panic hook.
fn restore_terminal_cleanup() -> io::Result<()> {
    if MOUSE_CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
    execute!(io::stdout(), LeaveAlternateScreen,)?;
    Ok(())
}

/// Set the terminal window title via OSC escape sequence.
pub fn set_terminal_title(title: &str) {
    let _ = execute!(io::stdout(), crossterm::terminal::SetTitle(title),);
}

/// Whether the current terminal supports the OSC 9;4 progress sequence.
pub fn supports_progress_osc() -> bool {
    use std::io::IsTerminal;
    if !io::stdout().is_terminal() {
        return false;
    }
    if std::env::var_os("TMUX").is_some() {
        return false;
    }
    if let Ok(term) = std::env::var("TERM")
        && (term.starts_with("screen") || term.starts_with("tmux"))
    {
        return false;
    }
    if std::env::var_os("WT_SESSION").is_some() || std::env::var_os("ConEmuPID").is_some() {
        return true;
    }
    matches!(
        std::env::var("TERM_PROGRAM").unwrap_or_default().as_str(),
        "iTerm.app" | "WezTerm" | "ghostty"
    )
}

/// Emit the OSC 9;4 progress sequence. `active = true` shows an indeterminate
/// "busy" indicator; `false` clears it.
pub fn set_terminal_progress(active: bool) {
    if !supports_progress_osc() {
        return;
    }
    use std::io::Write;
    let seq: &[u8] = if active {
        b"\x1b]9;4;3;0\x07"
    } else {
        b"\x1b]9;4;0;0\x07"
    };
    let mut out = io::stdout();
    let _ = out.write_all(seq);
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_osc_detection_does_not_panic() {
        // Just verify it runs without panicking in a test environment.
        let _ = supports_progress_osc();
    }
}

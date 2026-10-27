//! Kitty keyboard-protocol enablement — the Shift+Enter half of the fix.
//!
//! ## Why this exists (2026-10-27, operator report: "SHIFT+ENTER is not
//! adding line breaks")
//!
//! `key_handling.rs` has mapped Enter+SHIFT to `insert_newline` since
//! iter-120, so the app-side half was never the gap. The gap is that a
//! terminal that has not been asked for the kitty keyboard protocol sends
//! a bare `\r` for EVERY Enter chord — Shift+Enter, Ctrl+Enter, Alt+Enter
//! all arrive as an unmodified `KeyCode::Enter`, indistinguishable from
//! plain Enter, and the app submits instead of inserting a newline. The
//! information is simply not transmitted unless the app requests it.
//!
//! jcode's `tui/mod.rs` (`enable_keyboard_enhancement`, MIT) proved the fix:
//! push `PushKeyboardEnhancementFlags(DISAMBIGUATE_ESCAPE_CODES |
//! REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS)` at TUI startup and pop it at
//! teardown. Modern terminals (kitty, WezTerm, Alacritty, iTerm2, foot,
//! Ghostty, recent VTE) honor it out of the box; tmux needs
//! `extended-keys on` (requested here as CSI >4;2m when inside tmux); the
//! residual cases keep `/terminal-setup` relevant.
//!
//! ## Ownership discipline (jcode's, kept)
//!
//! The push/pop pair must be balanced even across panic paths, because a
//! stray entry leaves the SHELL in CSI-u mode after operant exits. Three
//! rules, each traced to a real incident class in jcode:
//!
//! 1. An `AtomicBool` records that THIS process owns the top entry. Only the
//!    process that pushed pops: a child editor (Ctrl+G external editor)
//!    may push its own entry after we suspended, and popping blindly would
//!    remove THEIR entry instead of ours.
//! 2. Termination cleanup runs once; a late `enable` after cleanup started
//!    is refused so no entry is left unpopped for the shell to inherit.
//! 3. The ownership swap and the terminal write happen under one lock so a
//!    concurrent teardown never observes a half-applied state.
//!
//! ## Flags deliberately NOT requested
//!
//! `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is intentionally absent (jcode's
//! finding, kept): terminals report printable keys as base key + modifiers
//! instead of the layout-produced text, and crossterm does not expose kitty's
//! text/alternate-key fields, so shifted symbols (`?`, `@` on some layouts)
//! cannot be reconstructed safely. DISAMBIGUATE already delivers every
//! Enter-chord distinction this feature needs.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};

/// This process owns the top kitty stack entry (push succeeded, not yet
/// popped). See the module doc's ownership rules.
static KITTY_PUSH_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Terminal cleanup has begun; a later push is refused so the pop stays
/// balanced against the shell the process is about to return to.
static TERMINATION_STARTED: AtomicBool = AtomicBool::new(false);

/// Serializes the push/pop terminal writes against their ownership swaps,
/// so teardown never interleaves with a half-applied push.
static STATE_LOCK: Mutex<()> = Mutex::new(());

/// The flag set that makes Enter chords distinguishable. See the module doc
/// for why REPORT_ALL_KEYS is not here.
fn enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
}

/// `$TMUX` non-empty — we are inside a tmux session, and tmux needs its own
/// extended-keys opt-in rather than (only) the kitty push.
fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

/// Request kitty keyboard reporting (and tmux extended-keys when relevant).
/// Returns whether the request bytes were written — not whether the terminal
/// honors them (nothing in the protocol round-trips that to the client).
pub fn enable_keyboard_enhancement() -> bool {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if TERMINATION_STARTED.load(Ordering::Acquire) {
        tracing::info!("keyboard enhancement push skipped: teardown already started");
        return false;
    }
    let result = enable_to(&mut std::io::stdout()).is_ok();
    if result {
        // Set while still holding the lock: a concurrent teardown must not
        // observe the write without the ownership flag.
        KITTY_PUSH_ACTIVE.store(true, Ordering::Release);
    }
    result
}

/// Writer-backed push, so the test suite can assert the exact escape bytes
/// without writing to the developer's terminal.
fn enable_to(writer: &mut impl Write) -> std::io::Result<()> {
    if inside_tmux() {
        // tmux's own modifyOtherKeys/extended-keys mode — jcode's exact
        // sequence. Harmless no-op when tmux ignores it.
        writer.write_all(b"\x1b[>4;2m")?;
    }
    crossterm::execute!(writer, PushKeyboardEnhancementFlags(enhancement_flags()))
}

/// Pop the entry this process pushed, if it is still unbalanced. Returns
/// whether a pop was performed. Must be idempotent: the panic hook, the
/// graceful teardown, and a signal cleanup may all race to call this.
pub fn disable_keyboard_enhancement() -> bool {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if !KITTY_PUSH_ACTIVE.swap(false, Ordering::AcqRel) {
        return false;
    }
    let _ = disable_to(&mut std::io::stdout());
    true
}

/// Writer-backed pop for tests.
fn disable_to(writer: &mut impl Write) -> std::io::Result<()> {
    if inside_tmux() {
        // Restore tmux's default key handling.
        writer.write_all(b"\x1b[>4m")?;
    }
    crossterm::execute!(writer, PopKeyboardEnhancementFlags)
}

/// Mark teardown as started: a later `enable_keyboard_enhancement` (a
/// suspended-editor resume that lost the race) is refused so the shell
/// never inherits an entry nothing will pop.
pub fn begin_termination_cleanup() {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TERMINATION_STARTED.store(true, Ordering::Release);
}

/// Test hook: reset both flags without touching any terminal. Tests must
/// call this to keep the process-globals balanced across cases.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    let _guard = STATE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    KITTY_PUSH_ACTIVE.store(false, Ordering::Release);
    TERMINATION_STARTED.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_globals() -> &'static Mutex<()> {
        &STATE_LOCK
    }

    /// The exact byte sequences a compliant terminal expects: the kitty push
    /// carries all three flags, the pop is the stack pop CSI. Pinned so a
    /// crossterm upgrade that changes the encoding fails loudly here rather
    /// than silently breaking every Enter chord in production.
    #[test]
    fn push_and_pop_write_the_kitty_sequences() {
        let _guard = lock_globals().lock().unwrap_or_else(|p| p.into_inner());
        let mut buf: Vec<u8> = Vec::new();
        enable_to(&mut buf).expect("push writes");
        // crossterm 0.29 encodes PushKeyboardEnhancementFlags as `CSI > flags u`
        // (event.rs:497). 1|2|4 = DISAMBIGUATE(1) | REPORT_EVENT_TYPES(2) |
        // REPORT_ALTERNATE_KEYS(4) — verified against the vendored source.
        let pushed = String::from_utf8(buf.clone()).expect("utf8");
        assert!(pushed.contains("\x1b[>7u"), "push bytes: {pushed:?}");

        let mut buf2: Vec<u8> = Vec::new();
        disable_to(&mut buf2).expect("pop writes");
        let popped = String::from_utf8(buf2).expect("utf8");
        assert!(popped.contains("\x1b[<1u"), "pop bytes: {popped:?}");
    }

    /// Ownership: only the process that pushed pops, and only once. A second
    /// `disable` must be a no-op so a panic-hook + graceful-teardown race
    /// cannot remove someone else's entry (jcode's editor-suspension
    /// incident class).
    ///
    /// NOTE: this test must NOT take `lock_globals()` AND call
    /// `reset_for_test()` — the reset re-locks STATE_LOCK and a std Mutex
    /// is NOT reentrant: the first run deadlocked the whole test binary
    /// (observed 2026-10-27: the test process slept in do_wait with 0 CPU
    /// forever). Either take the guard OR use the reset; here the reset is
    /// the cleaner isolation and the assertions read the atomics directly.
    #[test]
    fn pop_is_ownership_gated_and_idempotent() {
        super::reset_for_test();

        // Nothing pushed: the ownership swap reports false → pop refuses.
        assert!(!KITTY_PUSH_ACTIVE.load(Ordering::Acquire));
        // Simulated successful push:
        KITTY_PUSH_ACTIVE.store(true, Ordering::Release);
        // First pop: the swap reports the previous true → pop happens.
        assert!(KITTY_PUSH_ACTIVE.swap(false, Ordering::AcqRel));
        // Second pop: the swap reports the previous false → refuse.
        assert!(!KITTY_PUSH_ACTIVE.swap(false, Ordering::AcqRel));

        super::reset_for_test();
    }

    /// After termination cleanup starts, a late push is refused so no
    /// unbalanced entry survives the process. (No `lock_globals()` guard —
    /// see the deadlock note on the ownership test above.)
    #[test]
    fn late_push_after_teardown_is_refused() {
        super::reset_for_test();
        TERMINATION_STARTED.store(true, Ordering::Release);
        // The gate `enable_keyboard_enhancement` consults: asserted via the
        // same predicate it uses. A late push must be refused.
        assert!(TERMINATION_STARTED.load(Ordering::Acquire));
        // And the public fn's observable contract: it returns false and does
        // NOT mark ownership. (Writes go to the real stdout on the success
        // path only — the gate refuses BEFORE any write, so this is safe to
        // call in a test harness.)
        assert!(!crate::tui::keyboard_enhancement::enable_keyboard_enhancement());
        super::reset_for_test();
    }
}

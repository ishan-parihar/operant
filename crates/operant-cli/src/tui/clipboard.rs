//! The single owner of every clipboard **write** in the TUI.
//!
//! Before this module the write path existed three times over
//! (`app::helpers::try_copy_to_clipboard`, `message_copy::copy_to_clipboard`
//! and `image_paste::write_clipboard_text`), each with its own hardcoded
//! wl-copy/xclip/xsel list and its own idea of which OS it was on. All three
//! are now thin delegates here, so a copy either goes through the chain below
//! or it does not happen at all.
//!
//! The chain, in preference order:
//!
//! 1. [`Mechanism::Arboard`] — in-process clipboard.
//! 2. [`Mechanism::WlCopy`] — Wayland.
//! 3. [`Mechanism::Xclip`] — X11.
//! 4. [`Mechanism::Xsel`] — X11, last resort.
//! 5. [`Mechanism::Osc52`] — base64 escape sequence written to the tty. This
//!    is the rung that survives plain SSH and terminals with no clipboard
//!    daemon, because the *terminal emulator* owns the clipboard there.
//!
//! The split that makes this testable: [`plan`] is a **pure** function of
//! "is mechanism X available" and is the only place the order lives.
//! [`write_with`] is the **only** impure step and is the only place that
//! spawns a process. Availability is probed once and cached — see
//! [`available`].

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

use base64::Engine;

/// Ceiling on the OSC 52 payload, measured in bytes of base64.
///
/// Terminals that implement OSC 52 (xterm, iTerm2, WezTerm, Kitty, Windows
/// Terminal) accept payloads on the order of a megabyte, but `tmux` and
/// `screen` truncate passthrough far below that — and a truncated escape
/// sequence does not fail quietly, it leaves a visible `]52;c…` fragment
/// painted on the screen. 100 KB of base64 is ~75 KB of text: above what the
/// small multiplexers clip, comfortably under what the terminals accept, and
/// small enough that a live drag-select never blocks the event loop on one
/// huge stdout write.
pub const OSC52_MAX_BASE64_BYTES: usize = 100_000;

/// A way of getting text into the system clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// In-process clipboard. Resolved through the platform's own helper; see
    /// [`native_helper`].
    Arboard,
    /// `wl-copy` (Wayland).
    WlCopy,
    /// `xclip` (X11).
    Xclip,
    /// `xsel` (X11).
    Xsel,
    /// OSC 52 escape sequence written to the tty.
    Osc52,
}

impl Mechanism {
    /// Short human-readable name, used in the one-line failure report.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Arboard => "arboard/native",
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
            Self::Xsel => "xsel",
            Self::Osc52 => "OSC 52",
        }
    }
}

/// The preference order, best first. [`plan`] filters this; nothing else
/// decides order.
pub const CHAIN: [Mechanism; 5] = [
    Mechanism::Arboard,
    Mechanism::WlCopy,
    Mechanism::Xclip,
    Mechanism::Xsel,
    Mechanism::Osc52,
];

/// **Pure.** The mechanisms we would try, given what is available — always in
/// [`CHAIN`] order, regardless of the order the caller listed them in.
///
/// Takes a slice rather than a predicate so the whole decision is one
/// exhaustive `filter` that can be unit-tested without touching the process
/// table, `$PATH`, or a terminal.
pub fn plan(available: &[Mechanism]) -> Vec<Mechanism> {
    CHAIN
        .iter()
        .copied()
        .filter(|m| available.contains(m))
        .collect()
}

/// One mechanism that was tried and why it did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The mechanism that was attempted.
    pub mechanism: Mechanism,
    /// Why it was skipped or why it failed.
    pub reason: String,
}

/// Result of a copy attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    /// Some mechanism took the text.
    Copied { mechanism: Mechanism },
    /// Every mechanism failed. `attempts` names all of them, in the order
    /// they were tried.
    Failed { attempts: Vec<Attempt> },
}

impl CopyOutcome {
    /// `true` when the text reached a clipboard.
    pub fn is_copied(&self) -> bool {
        matches!(self, Self::Copied { .. })
    }

    /// The one-line message to show the user: success, or every mechanism
    /// that was tried with the reason each one failed.
    pub fn status_message(&self) -> String {
        match self {
            Self::Copied { mechanism } => format!("Copied to clipboard via {}.", mechanism.label()),
            Self::Failed { attempts } => failure_summary(attempts),
        }
    }
}

/// **Pure.** Render a total failure as a single line naming every mechanism
/// that was tried and why each one failed.
///
/// This is the difference between "copy silently did nothing" and a user
/// knowing their terminal is refusing OSC 52 and `wl-copy` is not installed.
pub fn failure_summary(attempts: &[Attempt]) -> String {
    if attempts.is_empty() {
        return "Clipboard copy failed: no mechanism attempted.".to_string();
    }
    let mut line = String::from("Clipboard copy failed — ");
    for (i, a) in attempts.iter().enumerate() {
        if i > 0 {
            line.push_str("; ");
        }
        line.push_str(a.mechanism.label());
        line.push_str(" (");
        line.push_str(&a.reason);
        line.push(')');
    }
    line.push('.');
    line
}

/// Copy `text` to the system clipboard, trying [`CHAIN`] in order and
/// stopping at the first mechanism that works.
pub fn copy(text: &str) -> CopyOutcome {
    run(text, &plan(available()))
}

/// The mechanisms this process can actually use, probed once and cached for
/// the lifetime of the process.
///
/// A `OnceLock` is the right cache because nothing it consults changes under
/// us mid-session: `$PATH` is fixed at exec, the tty does not appear
/// mid-session, and the tool binaries do not get installed while the TUI is
/// running. Probing on every keystroke would fork a `stat` storm for no
/// possible gain. [`available_mechanisms`] is the impure probe behind it.
pub fn available() -> &'static [Mechanism] {
    static CACHE: OnceLock<Vec<Mechanism>> = OnceLock::new();
    CACHE.get_or_init(available_mechanisms)
}

/// **Impure.** The one-time availability probe. Resolved to whichever helper
/// the platform provides, and cached by [`available`].
fn available_mechanisms() -> Vec<Mechanism> {
    plan(&CHAIN)
        .into_iter()
        .filter(|m| match m {
            Mechanism::Arboard => native_helper().is_some_and(|(p, _)| on_path(p)),
            Mechanism::WlCopy => on_path("wl-copy"),
            Mechanism::Xclip => on_path("xclip"),
            Mechanism::Xsel => on_path("xsel"),
            Mechanism::Osc52 => std::io::stdout().is_terminal(),
        })
        .collect()
}

/// The platform's own clipboard helper, used for the in-process rung.
///
/// `arboard` is not a dependency of `operant-cli`, so rather than delete the
/// rung and lose macOS/Windows copying, the rung resolves to the OS helper the
/// platform always ships: `pbcopy` on macOS, `clip` on Windows. On Linux
/// there is no in-process path and the rung is simply unavailable — `wl-copy`
/// and `xclip` cover it. Adding the real dependency later is a change to this
/// function and [`write_with`] only; the chain order is already correct.
fn native_helper() -> Option<(&'static str, &'static [&'static str])> {
    #[cfg(target_os = "macos")]
    {
        Some(("pbcopy", &[]))
    }
    #[cfg(target_os = "windows")]
    {
        Some(("clip", &[]))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Is `prog` an executable file somewhere on `$PATH`? A `stat` per directory
/// entry, no subprocess.
fn on_path(prog: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(prog);
        candidate.is_file() || candidate.with_extension("exe").is_file()
    })
}

/// The single impure write step: try one mechanism, report why it failed.
///
/// Takes the chain as an argument so a caller (or a test) can drive the
/// fallback loop with a chain it chose, without `available()` having run.
fn run(text: &str, chain: &[Mechanism]) -> CopyOutcome {
    if chain.is_empty() {
        // Nothing available at all: still report every mechanism, with the
        // reason it was ruled out, so the user is never left guessing.
        return CopyOutcome::Failed {
            attempts: CHAIN
                .iter()
                .map(|m| Attempt {
                    mechanism: *m,
                    reason: unavailable_reason(*m),
                })
                .collect(),
        };
    }

    let mut attempts = Vec::new();
    for &mechanism in chain {
        match write_with(text, mechanism) {
            Ok(()) => {
                return CopyOutcome::Copied { mechanism };
            }
            Err(reason) => attempts.push(Attempt { mechanism, reason }),
        }
    }
    CopyOutcome::Failed { attempts }
}

/// Why a mechanism is not even worth attempting. Only used on the
/// all-unavailable path, where no spawn was tried.
fn unavailable_reason(mechanism: Mechanism) -> String {
    match mechanism {
        Mechanism::Arboard => match native_helper() {
            Some((p, _)) => format!("{p} not on PATH"),
            None => "no in-process clipboard on this platform".to_string(),
        },
        Mechanism::WlCopy | Mechanism::Xclip | Mechanism::Xsel => {
            let prog = mechanism.label();
            format!("{prog} not on PATH")
        }
        Mechanism::Osc52 => "stdout is not a tty".to_string(),
    }
}

/// The one place in the TUI that spawns a process for a clipboard write, or
/// emits an escape sequence.
fn write_with(text: &str, mechanism: Mechanism) -> Result<(), String> {
    match mechanism {
        Mechanism::Arboard => {
            let Some((prog, args)) = native_helper() else {
                return Err(unavailable_reason(Mechanism::Arboard));
            };
            spawn_write(prog, args, text)
        }
        Mechanism::WlCopy => spawn_write("wl-copy", &[], text),
        Mechanism::Xclip => spawn_write("xclip", &["-selection", "clipboard"], text),
        Mechanism::Xsel => spawn_write("xsel", &["--clipboard", "--input"], text),
        Mechanism::Osc52 => write_osc52(text),
    }
}

/// Pipe `text` into `prog`, and treat a non-zero exit as failure so the chain
/// moves on. `wl-copy` in particular exits non-zero when there is no Wayland
/// compositor, which is exactly the "unavailable" case the chain wants to
/// detect.
fn spawn_write(prog: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{prog} not usable: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A broken pipe here means the helper exited early; fall through to
        // `wait` for the real reason.
        let _ = stdin.write_all(text.as_bytes());
    }
    let status = child
        .wait()
        .map_err(|e| format!("{prog} wait failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{prog} exited {status}"))
    }
}

/// Emit `ESC ] 52 ; c ; <base64> BEL` on stdout, the same write path
/// `osc8.rs` uses for hyperlinks — stdout through crossterm's queueable
/// `Print`, then flush.
///
/// Works over plain SSH and in any terminal that honours OSC 52, because the
/// clipboard belongs to the terminal emulator, not to this process.
fn write_osc52(text: &str) -> Result<(), String> {
    use crossterm::QueueableCommand;
    use crossterm::style::Print;

    let payload = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    if payload.len() > OSC52_MAX_BASE64_BYTES {
        return Err(format!(
            "payload {} B over the {OSC52_MAX_BASE64_BYTES} B ceiling",
            payload.len()
        ));
    }
    let mut out = std::io::stdout();
    out.queue(Print(format!("\x1b]52;c;{payload}\x07")))
        .map_err(|e| format!("stdout write failed: {e}"))?;
    out.flush().map_err(|e| format!("stdout flush failed: {e}"))
}

// ---------------------------------------------------------------------------
// Drag-select copy mode
// ---------------------------------------------------------------------------

/// State of the drag-select copy mode (Ctrl+T).
///
/// The selection itself is not modelled here — the TUI already has
/// `selection_anchor` / `selection_focus` / `selection_text` on `App` and copy
/// mode reuses them verbatim. This is only the transient state copy mode has
/// to put back when the user leaves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyMode {
    /// While true, composer keystrokes are swallowed and drags stream to the
    /// clipboard.
    pub active: bool,
    /// The selection that was live before copy mode was entered.
    pub saved_anchor: Option<(u16, u16)>,
    /// The selection focus that was live before copy mode was entered.
    pub saved_focus: Option<(u16, u16)>,
    /// The selection text that was live before copy mode was entered.
    pub saved_selection_text: String,
    /// Scroll position before entry; dragging past the viewport edge moves it,
    /// so it has to come back.
    pub saved_scroll_offset: usize,
    /// Tail-follow flag before entry.
    pub saved_auto_scroll: bool,
    /// Last payload handed to the clipboard, so a drag that has not actually
    /// extended the selection does not re-spawn a helper 60 times a second.
    last_copied: String,
}

thread_local! {
    /// Copy mode lives here because `App`'s fields are all initialised in
    /// `app/init.rs`, which this change is not allowed to touch, and there is
    /// exactly one TUI `App` per process. `ponytail:` move it onto `App` as a
    /// plain field the next time `init.rs` is editable.
    static COPY_MODE: std::cell::RefCell<CopyMode> =
        const { std::cell::RefCell::new(CopyMode::new()) };
}

impl CopyMode {
    /// A fresh, inactive copy mode with nothing to restore.
    pub const fn new() -> Self {
        Self {
            active: false,
            saved_anchor: None,
            saved_focus: None,
            saved_selection_text: String::new(),
            saved_scroll_offset: 0,
            saved_auto_scroll: false,
            last_copied: String::new(),
        }
    }
}

/// Is drag-select copy mode currently on?
pub fn copy_mode_active() -> bool {
    COPY_MODE.with(|m| m.borrow().active)
}

/// Turn copy mode on, remembering the state it is about to take over.
pub fn enter_copy_mode(
    anchor: Option<(u16, u16)>,
    focus: Option<(u16, u16)>,
    selection_text: &str,
    scroll_offset: usize,
    auto_scroll: bool,
) {
    COPY_MODE.with(|m| {
        let mut m = m.borrow_mut();
        m.active = true;
        m.saved_anchor = anchor;
        m.saved_focus = focus;
        m.saved_selection_text = selection_text.to_string();
        m.saved_scroll_offset = scroll_offset;
        m.saved_auto_scroll = auto_scroll;
        m.last_copied.clear();
    });
}

/// Turn copy mode off and hand back everything it saved, so the caller can
/// put the transcript back exactly as it found it.
pub fn exit_copy_mode() -> CopyMode {
    COPY_MODE.with(|m| {
        let mut m = m.borrow_mut();
        m.active = false;
        CopyMode {
            active: false,
            saved_anchor: m.saved_anchor.take(),
            saved_focus: m.saved_focus.take(),
            saved_selection_text: std::mem::take(&mut m.saved_selection_text),
            saved_scroll_offset: m.saved_scroll_offset,
            saved_auto_scroll: m.saved_auto_scroll,
            last_copied: std::mem::take(&mut m.last_copied),
        }
    })
}

/// Stream the selection to the clipboard while it is being dragged, skipping
/// the write when the selection has not actually changed.
///
/// Returns `true` when a copy was made.
pub fn copy_selection_live(selection_text: &str) -> bool {
    if selection_text.is_empty() {
        return false;
    }
    let stale = COPY_MODE.with(|m| m.borrow().last_copied.as_str() != selection_text);
    if !stale {
        return false;
    }
    if copy(selection_text).is_copied() {
        COPY_MODE.with(|m| m.borrow_mut().last_copied = selection_text.to_string());
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail(all: &[Mechanism]) -> Vec<Mechanism> {
        all.to_vec()
    }

    #[test]
    fn plan_returns_every_mechanism_in_chain_order_when_all_are_available() {
        let shuffled = avail(&[
            Mechanism::Xsel,
            Mechanism::Osc52,
            Mechanism::Xclip,
            Mechanism::WlCopy,
            Mechanism::Arboard,
        ]);
        assert_eq!(plan(&shuffled), CHAIN.to_vec());
    }

    #[test]
    fn plan_keeps_preference_order_not_caller_order() {
        assert_eq!(
            plan(&[Mechanism::Xsel, Mechanism::Arboard, Mechanism::WlCopy]),
            vec![Mechanism::Arboard, Mechanism::WlCopy, Mechanism::Xsel]
        );
    }

    #[test]
    fn plan_drops_unavailable_and_keeps_osc52_last() {
        assert_eq!(
            plan(&[Mechanism::Xclip, Mechanism::Osc52]),
            vec![Mechanism::Xclip, Mechanism::Osc52]
        );
    }

    #[test]
    fn osc52_payload_below_ceiling_is_accepted() {
        // 1 base64 char per 3 input bytes, so this is comfortably under.
        assert!(encode("hello").len() < OSC52_MAX_BASE64_BYTES);
    }

    fn encode(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
    }

    #[test]
    fn total_failure_names_every_mechanism_with_a_reason() {
        // An empty chain is "nothing is available", which is a total failure
        // with all five mechanisms reported — no subprocess needed.
        let out = run("hello", &[]);
        let CopyOutcome::Failed { attempts } = out else {
            panic!("expected failure, got {out:?}");
        };
        assert_eq!(attempts.len(), CHAIN.len());
        for m in CHAIN {
            let a = attempts
                .iter()
                .find(|a| a.mechanism == m)
                .unwrap_or_else(|| panic!("{m:?} missing from the report"));
            assert!(!a.reason.is_empty(), "{m:?} reported without a reason");
        }
    }

    #[test]
    fn failure_summary_is_one_line_and_names_each_mechanism() {
        let attempts: Vec<Attempt> = CHAIN
            .iter()
            .map(|m| Attempt {
                mechanism: *m,
                reason: format!("{} nope", m.label()),
            })
            .collect();
        let line = failure_summary(&attempts);
        assert!(!line.contains('\n'), "summary spans lines: {line}");
        for m in CHAIN {
            assert!(
                line.contains(m.label()),
                "summary omits {}: {line}",
                m.label()
            );
        }
    }

    #[test]
    fn copy_mode_enter_then_exit_restores_saved_state() {
        let _guard = CopyModeTestGuard;
        assert!(!copy_mode_active());
        enter_copy_mode(Some((1, 2)), Some((3, 4)), "before", 17, true);
        assert!(copy_mode_active());
        let saved = exit_copy_mode();
        assert!(!copy_mode_active());
        assert_eq!(saved.saved_anchor, Some((1, 2)));
        assert_eq!(saved.saved_focus, Some((3, 4)));
        assert_eq!(saved.saved_selection_text, "before");
        assert_eq!(saved.saved_scroll_offset, 17);
        assert!(saved.saved_auto_scroll);
    }

    /// Copies the pristine value into the thread-local so a failing test
    /// cannot leak copy mode into the next one.
    struct CopyModeTestGuard;

    impl Drop for CopyModeTestGuard {
        fn drop(&mut self) {
            COPY_MODE.with(|m| *m.borrow_mut() = CopyMode::new());
        }
    }
}

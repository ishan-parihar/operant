// keybindings/report.rs — Presentation layer over the binding registry.
//
// The registry is the single source of truth for *what the bindings are*.
// This module turns it into the two reports the TUI surfaces:
//
//   /keys    — the active catalogue, with OS-conflict flags
//   /hotkeys — the same catalogue split by whether you have ever pressed it
//
// Nothing here mutates the registry, and nothing here dispatches keys: the
// if-chain in `app/key_handling.rs` remains the dispatcher. This is the
// read-only reporting half of the split.

use super::{BindingContext, KeyBinding, KeyBindingRegistry};
use crate::tui::slash_usage::UsageStore;
use std::collections::HashSet;

/// Chords a terminal, window manager or VM layer commonly intercepts before
/// the TUI ever sees them.
///
/// Static and short *on purpose*: a large speculative list is worse than a
/// small accurate one, because it trains the reader to ignore the flags. Each
/// entry names a mechanism that is real, not a guess. Chords listed here but
/// absent from the default table cost nothing — `/keys` only reports flags for
/// chords the table actually binds.
const OS_CONFLICTS: &[(&str, &str)] = &[
    (
        "Ctrl+c",
        "SIGINT — the tty line discipline usually acts before the app sees it",
    ),
    ("Ctrl+z", "SIGTSTP — suspends the process"),
    (
        "Ctrl+d",
        "tty EOF (VEOF) — closes the input stream instead of paging",
    ),
    (
        "Ctrl+s",
        "XOFF — terminal software flow control pauses output",
    ),
    (
        "Ctrl+q",
        "XON — terminal software flow control resumes output",
    ),
    (
        "Ctrl+b",
        "X11 keysym Backward — readline/Emacs layers intercept it",
    ),
    (
        "Ctrl+f",
        "X11 keysym Forward — readline/Emacs layers intercept it",
    ),
    (
        "Ctrl+p",
        "X11 keysym Previous — readline/Emacs layers intercept it",
    ),
    (
        "Ctrl+n",
        "X11 keysym Next — readline/Emacs layers intercept it",
    ),
    (
        "Ctrl+k",
        "X11 keysym Clear/Line Kill — some terminals intercept it",
    ),
    (
        "Ctrl+u",
        "X11 keysym Line Delete — some terminals intercept it",
    ),
    (
        "Ctrl+w",
        "X11 keysym WordRubout — some terminals intercept it",
    ),
    ("Ctrl+y", "X11 keysym Undo — some terminals intercept it"),
    (
        "Ctrl+h",
        "ASCII 8 / X11 BackSpace — the terminal or readline layer takes backspace before the app sees it",
    ),
    (
        "Ctrl+l",
        "readline clear-screen — most terminals repaint instead of delivering the key",
    ),
    ("F1", "many terminals bind their own help viewer to F1"),
];

/// Why `chord` is likely swallowed by the terminal / window manager, or
/// `None` when it looks safe.
///
/// This is the single conflict predicate; callers that only need a bool read
/// `os_conflict_reason(..).is_some()` rather than getting a second function.
pub fn os_conflict_reason(chord: &str) -> Option<&'static str> {
    if let Some((_, why)) = OS_CONFLICTS.iter().find(|(c, _)| *c == chord) {
        return Some(why);
    }
    // `Alt+<letter>` is the classic window-menu accelerator on most terminals.
    // Exactly five bytes, so `Alt+Ctrl+x` and friends are not caught by this.
    if chord.len() == 5 && chord.starts_with("Alt+") {
        return Some("Alt+<letter> is a window-menu accelerator on many terminals");
    }
    None
}

/// Namespaced key into the shared usage store, so chord counters can never
/// collide with the slash-command counters in the same file.
pub fn usage_key(chord: &str) -> String {
    format!("key:{chord}")
}

/// One row of `/keys` or `/hotkeys`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingView {
    /// Canonical chord text, e.g. `Ctrl+Shift+P`, `Alt+v`, `F1`.
    pub chord: String,
    /// Context the binding is active in.
    pub context: BindingContext,
    /// `KeyAction` variant name — the registry-level meaning of the chord.
    pub action: String,
    /// Human description from the default table.
    pub description: String,
    /// Times this chord has been pressed this install. `0` = never used.
    pub count: u32,
}

/// `/hotkeys` output: muscle memory vs undiscovered.
#[derive(Debug, Clone, Default)]
pub struct KeyUsageReport {
    /// Chords pressed at least once, most-used first.
    pub used: Vec<BindingView>,
    /// Chords never pressed — the undiscovered surface.
    pub never_used: Vec<BindingView>,
    /// `(chord, reason)` for bound chords the terminal likely intercepts.
    pub conflicts: Vec<(String, String)>,
    /// Distinct chords in the report (used + never_used).
    pub total: usize,
}

impl KeyBindingRegistry {
    /// Every binding in the table, each listed exactly once with the context
    /// it applies in, in catalogue order.
    ///
    /// Deliberately not built on `get_bindings`: that appends the global set
    /// to *every* context, so a catalogue built on it would list the globals
    /// once per context.
    pub fn catalogue(&self) -> Vec<(BindingContext, &KeyBinding)> {
        let mut out: Vec<(BindingContext, &KeyBinding)> = self
            .bindings
            .iter()
            .flat_map(|(ctx, bs)| bs.iter().map(move |b| (*ctx, b)))
            .collect();
        // Context-less bindings are global by definition; the default table
        // uses explicit contexts, so this is normally empty.
        out.extend(
            self.global_bindings
                .iter()
                .map(|b| (BindingContext::Global, b)),
        );
        out.sort_by_key(|(ctx, _)| {
            BindingContext::ALL
                .iter()
                .position(|c| c == ctx)
                .unwrap_or(usize::MAX)
        });
        out
    }

    /// The catalogue as report rows, annotated with press counts from
    /// `usage`.
    pub fn bindings_with_usage(&self, usage: &UsageStore) -> Vec<BindingView> {
        self.catalogue()
            .into_iter()
            .map(|(context, b)| BindingView {
                count: usage.frequency_rank(&usage_key(&b.chord())),
                chord: b.chord(),
                context,
                action: format!("{:?}", b.action),
                description: b.description.clone().unwrap_or_default(),
            })
            .collect()
    }

    /// Split the catalogue into chords the user has pressed and chords they
    /// never have, most-used first, plus the OS-conflict shortlist.
    ///
    /// Counted per *chord*, not per row: `Ctrl+p` is bound twice in the table
    /// (global command palette, prompt history-prev) but it is one key the
    /// user presses, so it appears once here. `/keys` still lists every row.
    pub fn usage_report(&self, usage: &UsageStore) -> KeyUsageReport {
        let mut report = KeyUsageReport::default();
        let mut seen: HashSet<String> = HashSet::new();
        for view in self.bindings_with_usage(usage) {
            if let Some(why) = os_conflict_reason(&view.chord)
                && !report
                    .conflicts
                    .iter()
                    .any(|(chord, _)| *chord == view.chord)
            {
                report.conflicts.push((view.chord.clone(), why.to_string()));
            }
            if !seen.insert(view.chord.clone()) {
                continue;
            }
            if view.count == 0 {
                report.never_used.push(view);
            } else {
                report.used.push(view);
            }
        }
        // Most-pressed first, so `/hotkeys` leads with the real muscle memory.
        report.used.sort_by_key(|v| std::cmp::Reverse(v.count));
        report.total = report.used.len() + report.never_used.len();
        report
    }
}

impl BindingContext {
    /// Short label used in the `/keys` listing.
    pub fn label(self) -> &'static str {
        match self {
            BindingContext::Global => "global",
            BindingContext::Prompt => "prompt",
            BindingContext::Transcript => "transcript",
            BindingContext::Dialog => "dialog",
            BindingContext::VimNormal => "vim-normal",
            BindingContext::VimInsert => "vim-insert",
            BindingContext::VimVisual => "vim-visual",
            BindingContext::VimVisualLine => "vim-visual-line",
            BindingContext::VimVisualBlock => "vim-visual-block",
            BindingContext::VimCommand => "vim-command",
            BindingContext::VimSearch => "vim-search",
            BindingContext::Completion => "completion",
        }
    }
}

impl KeyBinding {
    /// Canonical display form of this binding's chord.
    ///
    /// crossterm reports a capital letter as `Char('I')` *plus* the SHIFT
    /// modifier, so SHIFT is only spelled out for keys that are not already
    /// upper case — otherwise every vim chord would render `Shift+X`.
    pub fn chord(&self) -> String {
        let mut s = String::new();
        if self
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            s.push_str("Ctrl+");
        }
        if self.modifiers.contains(crossterm::event::KeyModifiers::ALT) {
            s.push_str("Alt+");
        }
        let key_is_upper =
            matches!(self.key, crossterm::event::KeyCode::Char(c) if c.is_uppercase());
        if self
            .modifiers
            .contains(crossterm::event::KeyModifiers::SHIFT)
            && !key_is_upper
        {
            s.push_str("Shift+");
        }
        if self
            .modifiers
            .contains(crossterm::event::KeyModifiers::SUPER)
        {
            s.push_str("Super+");
        }
        s.push_str(&match self.key {
            crossterm::event::KeyCode::Char(c) => c.to_string(),
            crossterm::event::KeyCode::Enter => "Enter".to_string(),
            crossterm::event::KeyCode::Esc => "Esc".to_string(),
            crossterm::event::KeyCode::Tab => "Tab".to_string(),
            crossterm::event::KeyCode::Up => "Up".to_string(),
            crossterm::event::KeyCode::Down => "Down".to_string(),
            crossterm::event::KeyCode::Left => "Left".to_string(),
            crossterm::event::KeyCode::Right => "Right".to_string(),
            crossterm::event::KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        });
        s
    }
}

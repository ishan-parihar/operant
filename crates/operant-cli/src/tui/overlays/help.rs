// overlays/help.rs — Full-screen help overlay (? / F1 / /help).
//
// Extracted from the overlays.rs monolith. Renders the two-column
// keyboard-shortcut + slash-command reference panel and owns the
// HelpOverlay / HelpEntry state types.

// ============================================================================
// HelpOverlay
// ============================================================================

/// What the overlay currently shows. 2026-10-14 batch, plan 10: /keys and
/// /hotkeys open the SAME overlay machinery as /help (scroll, filter,
/// Esc), but with the binding catalogue as content — rendered at the top
/// (scroll 0), never as a bottom-dumped transcript message.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum HelpOverlayContent {
    #[default]
    Commands,
    Keys,
    Hotkeys,
}

/// State for the full-screen help overlay (? / F1 / /help).
#[derive(Debug, Default)]
pub struct HelpOverlay {
    pub visible: bool,
    pub scroll_offset: u16,
    /// The largest scroll offset the CONTENT allows (its height minus the
    /// viewport), recorded by the render path on every frame.
    /// (iter-1795) The key handler used to cap the scroll at a hard-coded
    /// 50 lines — "a generous upper bound" — which was generous for the
    /// /keys report and far too small for the /help catalogue: the
    /// "More commands" list, the Skills section, the whole KEYBOARD
    /// section and the Input section were UNREACHABLE. The render path
    /// knows the real number, so it records it here (a Cell, because the
    /// TuiState seam is read-only) and the handler clamps to it.
    pub max_scroll: std::cell::Cell<usize>,
    /// The overlay's visible height in rows, recorded by the render path on
    /// every frame — the PAGE step for PageUp/PageDown.
    /// (iter-1855) The /help catalogue is long (the commands, the Skills
    /// section, the whole KEYBOARD section, the Input section), and the only
    /// way through it was one line at a time — the arrows, or the wheel. The
    /// page keys are what a reader expects of a long list, and the render
    /// path knows the real height (a Cell, because the TuiState seam is
    /// read-only), so it records it here.
    pub page_height: std::cell::Cell<usize>,
    /// Live search filter — only commands matching this substring are shown.
    pub filter: String,
    /// Dynamically populated entries from the command registry.
    pub commands: Vec<HelpEntry>,
    /// Which surface this overlay shows right now.
    pub content: HelpOverlayContent,
}

/// A single command entry shown in the help overlay.
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub name: String,
    pub category: String,
}

impl HelpOverlay {
    pub fn new() -> Self {
        let overlay = Self::default();
        // "Never recorded" — the render path has not drawn the content yet, so
        // the height is unknown and the scroll must not be clamped to a
        // guessed zero (iter-1795). The renderer clamps the DISPLAY anyway.
        overlay.max_scroll.set(usize::MAX);
        overlay
    }

    /// Populate (or replace) the command entries from the command registry.
    /// Entries are sorted by category then name.
    pub fn populate_from_commands(&mut self, entries: Vec<HelpEntry>) {
        self.commands = entries;
        // Sort stable by category, then name for consistent display.
        self.commands
            .sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            // Opening via ?/F1//help always shows the command catalogue;
            // Keys/Hotkeys content is set only by open_keys/open_hotkeys.
            self.content = HelpOverlayContent::Commands;
        } else {
            // Reset state when closing
            self.scroll_offset = 0;
            self.filter.clear();
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.scroll_offset = 0;
        self.filter.clear();
        self.content = HelpOverlayContent::Commands;
    }

    /// Open showing the full binding catalogue (plan 10) — at the top.
    pub fn open_keys(&mut self) {
        self.visible = true;
        self.scroll_offset = 0;
        self.filter.clear();
        self.content = HelpOverlayContent::Keys;
    }

    /// Open showing used/never-used chords (plan 10) — at the top.
    pub fn open_hotkeys(&mut self) {
        self.visible = true;
        self.scroll_offset = 0;
        self.filter.clear();
        self.content = HelpOverlayContent::Hotkeys;
    }

    /// Open the command catalogue already FILTERED on a term (the
    /// `/help <command>` form the overlay's own row and its footer
    /// advertise). (iter-1785) The argument used to be dropped on the
    /// floor — the arm toggled the overlay whatever it was given.
    pub fn open_filtered(&mut self, filter: &str) {
        self.visible = true;
        self.scroll_offset = 0;
        self.filter = filter.trim().to_lowercase();
        self.content = HelpOverlayContent::Commands;
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll up by `lines` rows — the PAGE step (PageUp).
    pub fn scroll_up_by(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Scroll down one line, up to `max` — the largest offset the content
    /// allows (`HelpOverlay::max_scroll`, recorded by the render path).
    /// `max` is INCLUSIVE: the offset may reach it (the renderer clamps
    /// the same way), and the comparison is overflow-safe.
    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset < max {
            self.scroll_offset += 1;
        }
    }

    /// Scroll down by `lines` rows — the PAGE step (PageDown) — up to `max`,
    /// with the same inclusive clamp as `scroll_down`.
    pub fn scroll_down_by(&mut self, lines: u16, max: u16) {
        let target = self.scroll_offset.saturating_add(lines);
        self.scroll_offset = target.min(max);
    }

    /// The page step: the overlay's visible height, recorded by the render
    /// path (`HelpOverlay::page_height`). A page is at least one line, so a
    /// page key always moves even before the first frame has drawn.
    pub fn page_step(&self) -> u16 {
        self.page_height.get().max(1).min(u16::MAX as usize) as u16
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.scroll_offset = 0;
    }

    pub fn pop_filter_char(&mut self) {
        self.filter.pop();
        self.scroll_offset = 0;
    }
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

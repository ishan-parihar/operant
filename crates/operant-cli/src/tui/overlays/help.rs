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
    /// Live search filter — only commands matching this substring are shown.
    pub filter: String,
    /// Dynamically populated entries from the command registry.
    pub commands: Vec<HelpEntry>,
    /// Which surface this overlay shows right now.
    pub content: HelpOverlayContent,
}

/// A single command entry shown in the help overlay.
#[expect(
    dead_code,
    reason = "aliases/description are read only by the help filter in tests"
)]
#[derive(Debug, Clone)]
pub struct HelpEntry {
    pub name: String,
    /// Comma-separated aliases, e.g. "h, ?"
    pub aliases: String,
    pub description: String,
    pub category: String,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self::default()
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

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, max: u16) {
        if self.scroll_offset + 1 < max {
            self.scroll_offset += 1;
        }
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

//! The session hub — one full-frame page for sessions, branches, and live
//! subagents (plan 2026-10-28 iter-B/C).
//!
//! Geometry and interaction vendored from jcode's session picker
//! (`jcode-tui/src/tui/session_picker.rs`): vertical constraint stack
//! (search row, then `Min(8)`), horizontal 40/60 split (list | preview),
//! tokenized search with per-char highlight, a filter-mode cycle, and
//! cached-then-refresh loading over the plan-03 mpsc pattern. What was
//! replaced for operant: jcode loads session FILES through its own `Session`
//! type — operant loads `SessionEntryV2` rows from SQLite
//! (`Database::list_sessions_rich`, iter-A) and appends subagent rows from
//! the async-delegation registry (iter-C). jcode's external-CLI importers,
//! server grouping, onboarding band, and crash banner had no operant
//! counterpart and were not ported.
//!
//! The page replaces the 70x20 `session_browser` modal (audit RT-B): the old
//! modal is deleted, and `/session` `/resume` `/sessions` route here.

use std::time::Instant;

use operant_core::session::listing::{RowKind, RowStatus, SessionEntryV2};

mod render;
pub use render::render_session_hub;

#[cfg(test)]
mod tests;

/// Filter mode cycled with `s` (jcode's cycle, reduced to what operant has —
/// no external-CLI modes, no server modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubFilterMode {
    All,
    /// Working directory matches the current process cwd.
    CurrentDir,
    /// Only rows carrying an employee tag (seat-bound sessions).
    ByEmployee,
    /// Live rows only: the streaming session + pending delegations.
    Active,
    Archived,
}

impl HubFilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::CurrentDir,
            Self::CurrentDir => Self::ByEmployee,
            Self::ByEmployee => Self::Active,
            Self::Active => Self::Archived,
            Self::Archived => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::CurrentDir => "current dir",
            Self::ByEmployee => "by employee",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

/// What Enter on the selected row asks the App to do.
#[derive(Clone, Debug)]
pub enum HubSelection {
    /// Switch the agent to this session (the existing
    /// `pending_switch_session` flow — branches ARE sessions).
    SwitchSession(String),
    /// Focus a live subagent row (iter-C's focus view).
    FocusSubagent(String),
}

/// Page state. Owned by `App`; rendered full-frame while `visible`.
pub struct SessionHubState {
    pub visible: bool,
    /// The loaded unified rows (sessions + branches + subagents).
    pub rows: Vec<SessionEntryV2>,
    /// Indices into `rows` that pass the current filter + search.
    visible_idx: Vec<usize>,
    pub selected: usize,
    pub filter_mode: HubFilterMode,
    /// Search tokens (AND). Empty = no filtering, no highlight.
    pub search_query: String,
    /// While true, keystrokes edit `search_query` (jcode's search mode).
    pub search_active: bool,
    /// Loading placeholder while the background load runs.
    pub loading: bool,
    /// Preview scroll offset (rows), clamped by render.
    pub preview_scroll: u16,
    /// omp's `armCloseTap` handoff: seeded by the ←← opener so ONE further
    /// Left closes the page (omp agent-hub.ts:626-636). `None` = gesture
    /// state not armed.
    pub armed_close_tap: Option<Instant>,
    /// The id of the subagent row currently focused (omp's chat view),
    /// rendered in place of the message preview.
    pub focused_subagent: Option<String>,
}

impl Default for SessionHubState {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionHubState {
    pub fn new() -> Self {
        Self {
            visible: false,
            rows: Vec::new(),
            visible_idx: Vec::new(),
            selected: 0,
            filter_mode: HubFilterMode::All,
            search_query: String::new(),
            search_active: false,
            loading: true,
            preview_scroll: 0,
            armed_close_tap: None,
            focused_subagent: None,
        }
    }

    /// Open with fresh rows (the caller reloads in the background and
    /// replaces `rows` when the mpsc delivers). `armed_close_tap` carries the
    /// omp handoff when the page was opened by the ←← gesture.
    pub fn open(&mut self, armed_close_tap: Option<Instant>) {
        self.visible = true;
        self.loading = true;
        self.selected = 0;
        self.search_active = false;
        self.preview_scroll = 0;
        self.focused_subagent = None;
        self.armed_close_tap = armed_close_tap;
        self.rebuild_visible();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.focused_subagent = None;
        self.armed_close_tap = None;
    }

    /// Replace the backing rows (background load landed) and keep the
    /// selection on the same row id when possible.
    pub fn replace_rows(&mut self, rows: Vec<SessionEntryV2>) {
        let keep = self.selected_row().map(|r| r.id.clone());
        self.rows = rows;
        self.loading = false;
        self.rebuild_visible();
        if let Some(id) = keep
            && let Some(pos) = self.visible_idx.iter().position(|&i| self.rows[i].id == id)
        {
            self.selected = pos;
        } else {
            self.selected = 0;
        }
        self.clamp_selection();
    }

    /// The rows that survive filter + search, as indices into `rows`.
    pub fn visible_rows(&self) -> Vec<&SessionEntryV2> {
        self.visible_idx.iter().map(|&i| &self.rows[i]).collect()
    }

    pub fn selected_row(&self) -> Option<&SessionEntryV2> {
        self.visible_idx.get(self.selected).map(|&i| &self.rows[i])
    }

    pub fn cycle_filter(&mut self) {
        self.filter_mode = self.filter_mode.next();
        self.selected = 0;
        self.rebuild_visible();
    }

    /// Tokenize the search query the way jcode's matcher does: split on
    /// whitespace, lowercase, drop empties. AND semantics across tokens.
    pub fn search_tokens() -> fn(&str) -> Vec<String> {
        |q: &str| {
            q.split_whitespace()
                .map(|t| t.to_lowercase())
                .filter(|t| !t.is_empty())
                .collect()
        }
    }

    /// Re-derive `visible_idx` from `rows` + filter + search.
    pub fn rebuild_visible(&mut self) {
        let tokens = if self.search_active || !self.search_query.is_empty() {
            Self::search_tokens()(&self.search_query)
        } else {
            Vec::new()
        };
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        self.visible_idx = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| match self.filter_mode {
                HubFilterMode::All => true,
                HubFilterMode::CurrentDir => r
                    .cwd
                    .as_deref()
                    .zip(cwd.as_deref())
                    .is_some_and(|(a, b)| a == b),
                HubFilterMode::ByEmployee => r.employee_id.is_some(),
                HubFilterMode::Active => {
                    matches!(r.status, RowStatus::Live)
                        || matches!(
                            &r.kind,
                            RowKind::Subagent {
                                status: "pending",
                                ..
                            }
                        )
                }
                HubFilterMode::Archived => matches!(r.status, RowStatus::Archived),
            })
            .filter(|(_, r)| {
                // AND-token search over a lowercase haystack: title, id,
                // employee, goal, source, cwd (jcode's search_index shape).
                if tokens.is_empty() {
                    return true;
                }
                let mut hay = format!(
                    "{} {} {}",
                    r.id,
                    r.source,
                    r.employee_id.clone().unwrap_or_default()
                );
                if let Some(t) = &r.title {
                    hay.push(' ');
                    hay.push_str(&t.to_lowercase());
                }
                if let Some(c) = &r.cwd {
                    hay.push(' ');
                    hay.push_str(&c.to_lowercase());
                }
                if let RowKind::Subagent { goal, .. } = &r.kind {
                    hay.push(' ');
                    hay.push_str(&goal.to_lowercase());
                }
                tokens.iter().all(|t| hay.contains(t.as_str()))
            })
            .map(|(i, _)| i)
            .collect();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        if self.selected >= self.visible_idx.len() {
            self.selected = self.visible_idx.len().saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.visible_idx.len() {
            self.selected += 1;
            self.preview_scroll = 0;
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        if !self.visible_idx.is_empty() {
            self.preview_scroll = 0;
        }
    }

    pub fn scroll_preview(&mut self, delta: i16) {
        self.preview_scroll = self.preview_scroll.saturating_add_signed(delta);
    }

    /// What Enter does on the current row.
    pub fn confirm_selection(&mut self) -> Option<HubSelection> {
        let row = self.selected_row()?;
        let selection = match &row.kind {
            RowKind::Subagent { .. } => Some(HubSelection::FocusSubagent(row.id.clone())),
            // Sessions AND branches retarget through the existing flow.
            _ => Some(HubSelection::SwitchSession(row.id.clone())),
        };
        if let Some(HubSelection::FocusSubagent(id)) = &selection {
            self.focused_subagent = Some(id.clone());
        }
        selection
    }

    /// A plain Left while the page is open: with the omp close-tap armed,
    /// ONE Left closes (agent-hub.ts:2467); otherwise it arms.
    /// Returns true when the page should close.
    pub fn left_tap_close(&mut self) -> bool {
        // omp agent-hub.ts:626-636: when the open is armed, the ←← that opened
        // the hub already counts as the first tap — ONE further Left closes.
        // The arm is consumed either way, so a slow second Left re-arms
        // fresh (LEFT_TAP_CLOSE_WINDOW) instead of closing on a stale seed.
        match self.armed_close_tap.take() {
            Some(at) if at.elapsed() < LEFT_TAP_CLOSE_WINDOW => {
                self.close();
                true
            }
            _ => {
                self.armed_close_tap = Some(Instant::now());
                false
            }
        }
    }
}

/// omp's close window (`LEFT_TAP_WINDOW_MS`, agent-hub.ts:200-201).
const LEFT_TAP_CLOSE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

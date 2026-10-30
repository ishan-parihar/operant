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

/// N idle session rows for hosts and tests that need a populated hub list
/// without a database (iter-1503).
///
/// Idle `RowKind::Session` rows with no employee and no usage: the neutral
/// case a caller overrides. Gated to `#[cfg(test)]` because nothing in the
/// shipped binary calls it yet — when a real caller appears (the dashboard's
/// session pane, a CLI that previews the hub headlessly), the gate widens to
/// `#[cfg(any(test, feature = "test-fixtures"))]` in that same change. Until
/// then the release binary must not carry an unused constructor.
#[cfg(test)]
pub fn probe_rows(n: usize) -> Vec<SessionEntryV2> {
    (0..n)
        .map(|i| SessionEntryV2 {
            id: format!("sess_{i}"),
            parent_id: None,
            title: Some(format!("session {i}")),
            updated_at: chrono::Utc::now(),
            employee_id: None,
            source: "local".to_string(),
            message_count: 1,
            cost_usd: 0.0,
            tokens: 0,
            cwd: None,
            status: RowStatus::Idle,
            kind: RowKind::Session,
        })
        .collect()
}

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
    /// The current session and its forks/children — the Ctrl+B branch
    /// view (audit R1 2026-10-28: replaces the session_branching
    /// overlay, which listed exactly this set).
    CurrentBranch,
    Archived,
}

impl HubFilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::CurrentDir,
            Self::CurrentDir => Self::ByEmployee,
            Self::ByEmployee => Self::Active,
            Self::Active => Self::CurrentBranch,
            Self::CurrentBranch => Self::Archived,
            Self::Archived => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::CurrentDir => "current dir",
            Self::ByEmployee => "by employee",
            Self::Active => "active",
            Self::CurrentBranch => "this branch's tree",
            Self::Archived => "archived",
        }
    }
}

/// One rendered message in the preview pane (jcode's `PreviewMessage`,
/// jcode-tui-session-picker/src/lib.rs:167-173, reduced to the fields operant
/// renders: role + flattenable content + a tool-call summary).
#[derive(Clone, Debug)]
pub struct PreviewMessage {
    pub role: String,
    /// Flattened plain text (already trimmed of tool-call JSON noise).
    pub content: String,
    /// Short tool-call labels, one per call, joined when rendered.
    pub tool_calls: Vec<String>,
}

/// Which pane has keyboard focus (jcode's `PaneFocus`,
/// session_picker/navigation.rs:191-196). The hub's Up/Down/PageUp/PageDown
/// step WITHIN the focused pane, so a preview needs a focus to exist at all
/// — this is what Tab toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFocus {
    /// Session list (left pane) — Up/Down step sessions.
    Sessions,
    /// Preview (right pane) — Up/Down scroll the preview.
    Preview,
}

impl PaneFocus {
    pub fn toggle(self) -> Self {
        match self {
            Self::Sessions => Self::Preview,
            Self::Preview => Self::Sessions,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sessions => "list",
            Self::Preview => "preview",
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
    /// Which pane the keyboard is acting on (jcode's PaneFocus).
    pub focus: PaneFocus,
    /// Last rendered maximum preview scroll offset (total wrapped lines minus
    /// the preview inner height), so key steps clamp against the real bound
    /// instead of scrolling forever into empty space (jcode's
    /// `preview_max_scroll`).
    pub preview_max_scroll: u16,
    /// Per-session loaded preview: (session id, trailing messages, how many
    /// user prompts were dropped from the head). Loaded off a worker thread
    /// and polled per frame (jcode's `apply_session_preview` +
    /// `build_messages_preview_with_offset`).
    pub preview: Option<(String, Vec<PreviewMessage>, usize)>,
    /// True while a preview load is in flight for the selected row.
    pub preview_loading: bool,
    /// Session ids whose preview load failed, so a broken session does not
    /// get re-loaded on every selection change (jcode's
    /// `preview_load_failures`).
    pub preview_failures: Vec<String>,
    /// omp's `armCloseTap` handoff: seeded by the ←← opener so ONE further
    /// Left closes the page (omp agent-hub.ts:626-636). `None` = gesture
    /// state not armed.
    pub armed_close_tap: Option<Instant>,
    /// The id of the subagent row currently focused (omp's chat view),
    /// rendered in place of the message preview.
    pub focused_subagent: Option<String>,
    /// The row the `CurrentBranch` filter treats as the branch root (the
    /// agent's live session id, fed at open). `None` = no anchor.
    pub branch_root: Option<String>,
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
            focus: PaneFocus::Sessions,
            preview_max_scroll: 0,
            preview: None,
            preview_loading: false,
            preview_failures: Vec::new(),
            armed_close_tap: None,
            focused_subagent: None,
            branch_root: None,
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
        self.focus = PaneFocus::Sessions;
        self.focused_subagent = None;
        self.armed_close_tap = armed_close_tap;
        self.rebuild_visible();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.focused_subagent = None;
        self.armed_close_tap = None;
    }

    /// Push the current delegation-registry state into the rows ALREADY in
    /// the open hub, for the subagent rows only.
    ///
    /// The defect this closes (audit 2026-10-30, the Wave-3 false premise):
    /// the hub's rows are loaded when it OPENS, so while the operator sits
    /// watching it a row keeps reading `pending` no matter how long ago the
    /// delegation finished. The live tail sits one band away
    /// (`background_tasks.rs`, refreshed every frame) and is correct the
    /// whole time — the two were never connected.
    ///
    /// Deliberately narrow. DB rows are never touched (a re-read would move
    /// them under the operator's cursor), rows the registry does not mention
    /// are left exactly as they are, and the selection, scroll offset and
    /// `rebuild_visible` inventory are all preserved. Only a row whose
    /// delegation ACTUALLY changed status is rewritten, so a settled hub
    /// costs a lock read and nothing else.
    ///
    /// Returns true when a row visibly changed, which is the caller's cue
    /// that the frame repainted new information.
    pub fn refresh_subagent_rows(&mut self) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        // `updated_at` is the field the age/preview path reads; status and
        // RowStatus are what the row line draws. A terminal delegation's
        // `completed_time` is its `updated_at`, so an age read from it is
        // correct rather than "time since dispatch" for a finished row.
        let live = operant_core::tools::async_delegation::list_records();
        let mut changed = false;
        for row in self.rows.iter_mut() {
            let RowKind::Subagent {
                status,
                dispatch_time,
                ..
            } = &mut row.kind
            else {
                continue;
            };
            let Some(rec) = live.iter().find(|r| r.delegation_id == row.id) else {
                continue;
            };
            let (next, row_status, updated) = match rec.status {
                operant_core::tools::async_delegation::AsyncDelegationStatus::Pending => {
                    ("pending", RowStatus::Live, None)
                }
                operant_core::tools::async_delegation::AsyncDelegationStatus::Completed => {
                    ("completed", RowStatus::Idle, rec.completed_time)
                }
                operant_core::tools::async_delegation::AsyncDelegationStatus::Failed => {
                    ("failed", RowStatus::Idle, rec.completed_time)
                }
            };
            let settled_at = updated.unwrap_or(rec.dispatch_time);
            if *status == next && row.updated_at.timestamp() as u64 == settled_at {
                continue;
            }
            *status = next;
            row.status = row_status;
            row.updated_at = chrono::DateTime::from_timestamp(settled_at as i64, 0)
                .unwrap_or_else(chrono::Utc::now);
            *dispatch_time = rec.dispatch_time;
            changed = true;
        }
        if changed {
            // Sort unaffected: id-keyed rows keep their order; only the
            // rendered status/age move, and the selection is by row id.
            self.rebuild_visible();
        }
        changed
    }

    /// Replace the backing rows (background load landed) and keep the
    /// selection on the same row id when possible.
    pub fn replace_rows(&mut self, rows: Vec<SessionEntryV2>) {
        let keep = self.selected_row().map(|r| r.id.clone());
        // (audit R1) the CurrentBranch filter needs an anchor. Without one
        // (the operator opened by Ctrl+B, not by a session switch), take the
        // live row — the only session the App could be streaming into.
        if self.branch_root.is_none() {
            self.branch_root = rows
                .iter()
                .find(|r| matches!(r.status, RowStatus::Live))
                .map(|r| r.id.clone());
        }
        self.rows = rows;
        self.loading = false;
        self.rebuild_visible();
        if let Some(id) = keep
            && let Some(pos) = self.visible_idx.iter().position(|&i| self.rows[i].id == id)
        {
            self.selected = pos;
        } else {
            self.selected = 0;
            // The selection moved: the cached preview belongs to a row that
            // is no longer shown (the keep-id lookup failed).
            self.note_preview_miss();
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
        self.note_preview_miss();
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
                HubFilterMode::CurrentBranch => match self.branch_root.as_deref() {
                    Some(root) => r.id == root || r.parent_id.as_deref() == Some(root),
                    None => true,
                },
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
            self.note_preview_miss();
        }
    }

    /// True when the keyboard acts on the preview pane (jcode's focus split).
    pub fn preview_focused(&self) -> bool {
        matches!(self.focus, PaneFocus::Preview)
    }

    /// Tab: swap which pane the step keys act on.
    pub fn toggle_focus(&mut self) {
        self.focus = self.focus.toggle();
    }

    /// jcode's `focus_previous/next_step`: the SAME keys move the selection
    /// or scroll the preview depending on which pane has focus.
    pub fn focus_up(&mut self) {
        match self.focus {
            PaneFocus::Sessions => self.select_prev(),
            PaneFocus::Preview => self.scroll_preview(-PREVIEW_SCROLL_STEP),
        }
    }

    pub fn focus_down(&mut self) {
        match self.focus {
            PaneFocus::Sessions => self.select_next(),
            PaneFocus::Preview => self.scroll_preview(PREVIEW_SCROLL_STEP),
        }
    }

    pub fn focus_page_up(&mut self) {
        match self.focus {
            PaneFocus::Sessions => {
                for _ in 0..SESSION_PAGE_STEP_COUNT {
                    self.select_prev();
                }
            }
            PaneFocus::Preview => self.scroll_preview(-PREVIEW_PAGE_SCROLL),
        }
    }

    pub fn focus_page_down(&mut self) {
        match self.focus {
            PaneFocus::Sessions => {
                for _ in 0..SESSION_PAGE_STEP_COUNT {
                    self.select_next();
                }
            }
            PaneFocus::Preview => self.scroll_preview(PREVIEW_PAGE_SCROLL),
        }
    }

    /// Home/End follow the focused pane too (jcode keeps them pane-local:
    /// first/last session in the list, top/bottom of the preview).
    pub fn focus_home(&mut self) {
        match self.focus {
            PaneFocus::Sessions => self.select_first(),
            PaneFocus::Preview => self.preview_scroll = 0,
        }
    }

    pub fn focus_end(&mut self) {
        match self.focus {
            PaneFocus::Sessions => self.select_last(),
            PaneFocus::Preview => self.preview_scroll = self.preview_max_scroll,
        }
    }

    /// Jump to the first row (Home — R5 of the 2026-10-28 audit: the pane
    ///'s nav matches the other overlays').
    pub fn select_first(&mut self) {
        if !self.visible_idx.is_empty() {
            self.selected = 0;
            self.preview_scroll = 0;
            self.note_preview_miss();
        }
    }

    /// Set by every selection move: the loaded preview belongs to the row
    /// the operator LEFT, so the next frame asks for a fresh load (jcode's
    /// `needs_preview = messages_preview.is_empty()` check in
    /// `ensure_selected_preview_loading`).
    pub fn note_preview_miss(&mut self) {
        self.preview = None;
        self.preview_loading = false;
        self.preview_scroll = 0;
    }

    /// Drop the cached preview for the selected row so the next frame reloads
    /// it. The affordance the render path offers when a preview never landed
    /// (a DB miss, a load that was dropped when the selection moved): without
    /// it the operator's only recovery is closing and reopening the hub.
    pub fn request_preview_reload(&mut self) {
        let Some(id) = self.selected_row().map(|r| r.id.clone()) else {
            return;
        };
        if self.preview.as_ref().map(|(pid, _, _)| pid) == Some(&id) {
            self.preview = None;
        }
        self.preview_failures.retain(|fid| fid != &id);
        self.preview_scroll = 0;
    }

    /// Apply a background preview load, but ONLY if it is still the selected
    /// row — a load that landed after the operator moved on belongs to a
    /// row nobody is looking at (the stale-write guard jcode gets for free
    /// from comparing `pending.session_id`).
    pub fn apply_preview(
        &mut self,
        session_id: &str,
        messages: Vec<PreviewMessage>,
        hidden_prompts: usize,
    ) -> bool {
        if self.selected_row().map(|r| r.id.as_str()) != Some(session_id) {
            return false;
        }
        self.preview = Some((session_id.to_string(), messages, hidden_prompts));
        self.preview_loading = false;
        self.preview_scroll = 0;
        self.preview_failures.retain(|id| id != session_id);
        true
    }

    pub fn note_preview_failure(&mut self, session_id: &str) {
        self.preview_loading = false;
        if !self.preview_failures.iter().any(|id| id == session_id) {
            self.preview_failures.push(session_id.to_string());
        }
    }

    /// True when the selected row wants a preview load (no cached one,
    /// nothing in flight, not a known failure).
    pub fn preview_needs_load(&self) -> bool {
        if self.loading || self.preview_loading {
            return false;
        }
        let Some(row) = self.selected_row() else {
            return false;
        };
        let cached = self
            .preview
            .as_ref()
            .is_some_and(|(id, _, _)| id == &row.id);
        !cached && !self.preview_failures.iter().any(|id| id == &row.id)
    }

    /// Record the scroll bound the renderer measured this frame, so key
    /// steps clamp against the real geometry (jcode's `preview_max_scroll`).
    pub fn set_preview_max_scroll(&mut self, max: u16) {
        self.preview_max_scroll = max;
        if self.preview_scroll > max {
            self.preview_scroll = max;
        }
    }

    pub fn set_preview_loading(&mut self, loading: bool) {
        self.preview_loading = loading;
    }

    /// True when the preview pane has real transcript content (not just
    /// metadata) — the render/verification predicate.
    pub fn has_preview_content(&self) -> bool {
        self.preview
            .as_ref()
            .is_some_and(|(_, msgs, _)| !msgs.is_empty())
    }

    /// Window a session's messages to the preview: keep the trailing
    /// `PREVIEW_MESSAGE_LIMIT` and count the user prompts that fell off the
    /// head, so the pane can number prompts globally (jcode's
    /// `build_messages_preview_with_offset`).
    pub fn window_messages(messages: Vec<PreviewMessage>) -> (Vec<PreviewMessage>, usize) {
        let skip = messages.len().saturating_sub(PREVIEW_MESSAGE_LIMIT);
        let hidden_prompts = messages[..skip]
            .iter()
            .filter(|m| m.role == "user" && !m.content.trim().is_empty())
            .count();
        (messages.into_iter().skip(skip).collect(), hidden_prompts)
    }

    /// Jump to the last row (End).
    pub fn select_last(&mut self) {
        if !self.visible_idx.is_empty() {
            self.selected = self.visible_idx.len() - 1;
            self.preview_scroll = 0;
            self.note_preview_miss();
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        if !self.visible_idx.is_empty() {
            self.preview_scroll = 0;
            self.note_preview_miss();
        }
    }

    pub fn scroll_preview(&mut self, delta: i16) {
        let next = self.preview_scroll.saturating_add_signed(delta);
        // Clamp at the scroll bound the LAST frame measured. Without it a
        // preview shorter than the pane scrolls into blank space and Home /
        // End have nothing sensible to land on.
        self.preview_scroll = next.min(self.preview_max_scroll);
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

/// jcode's preview nav constants (session_picker/navigation.rs:197-199).
const PREVIEW_SCROLL_STEP: i16 = 3;
const PREVIEW_PAGE_SCROLL: i16 = PREVIEW_SCROLL_STEP * 3;
/// jcode's `SESSION_PAGE_STEP_COUNT`: how many sessions one PageUp/Down
/// covers while the LIST has focus (jcode steps 3, not a viewport page —
/// the picker list is short enough that a viewport page overshoots).
const SESSION_PAGE_STEP_COUNT: usize = 3;
/// jcode's `PREVIEW_MESSAGE_LIMIT` (session_picker/loading.rs:32): how many
/// trailing messages the preview pane keeps. The rest of the session stays
/// in the store; the pane names how many prompts it dropped.
pub const PREVIEW_MESSAGE_LIMIT: usize = 400;

/// Flatten one stored message row into a preview row.
///
/// The preview renders ONE line per message, so a message whose whole body is
/// tool-call JSON would render as noise. The cold loader that fills a reopened
/// session drops non user/assistant roles for the same reason; here the role is
/// kept (the pane shows what the session holds) but the noise is not: tool
/// calls become short labels off the parsed names, and a message with no
/// visible text becomes a placeholder row rather than an empty line.
pub fn preview_message_from_row(
    row: operant_core::database::MessageData,
) -> Option<PreviewMessage> {
    let role = row.role;
    let content = row.content.unwrap_or_default();
    let tool_calls = row
        .tool_calls
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.as_array().cloned())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let name = call.get("name").and_then(|n| n.as_str())?;
                    Some(name.to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if content.trim().is_empty() && tool_calls.is_empty() {
        return None;
    }
    Some(PreviewMessage {
        role,
        content,
        tool_calls,
    })
}

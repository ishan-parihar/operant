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
mod row_style;
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
    /// The session's state is the operator's question: "what needs me, what is
    /// running, what is done, what is parked" — codex's `TASK_FILTERS`
    /// (codex-rs/tui/src/app/agent_center/mod.rs:12-18: All | Needs you |
    /// Working | Ready | Inactive), ported onto operant's four `RowStatus`
    /// values.
    ///
    /// This ladder is what the operator asked for: "separate tabs for various
    /// filters based on the status of the agentic-loop". Before it, the tab
    /// row mixed a STATUS question with two SCOPE questions (which directory,
    /// which branch) and one IDENTITY question (is it employee-tagged at
    /// all), so no tab answered "what needs me" and the status of the
    /// agentic loop had to be read off every row by eye.
    NeedsYou,
    /// Live rows: the streaming session + pending delegations.
    Working,
    /// Finished and quiet: nothing to do about them.
    Ready,
    /// Archived — parked deliberately, so not "inactive" in the sense of
    /// idle, but out of the operator's way.
    Inactive,
    /// Working directory matches the current process cwd.
    ///
    /// A SCOPE question, not a status one, and it stays in the ladder because
    /// it is the one scope the operator reaches for most often.
    CurrentDir,
    /// The current session and its forks/children — the Ctrl+B branch
    /// view (audit R1 2026-10-28: replaces the session_branching
    /// overlay, which listed exactly this set).
    CurrentBranch,
}

impl HubFilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::NeedsYou,
            // The four status tabs come first, in the order the operator asks
            // the question: what needs me, what is running, what is done,
            // what is parked.
            Self::NeedsYou => Self::Working,
            Self::Working => Self::Ready,
            Self::Ready => Self::Inactive,
            // Then the two scope views.
            Self::Inactive => Self::CurrentDir,
            Self::CurrentDir => Self::CurrentBranch,
            Self::CurrentBranch => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::NeedsYou => "needs you",
            Self::Working => "working",
            Self::Ready => "ready",
            Self::Inactive => "inactive",
            Self::CurrentDir => "current dir",
            Self::CurrentBranch => "this branch's tree",
        }
    }

    /// The compact form the tab bar falls back to when the long labels do not
    /// fit. Codex's `toolbar_for_width` degrades the same way rather than
    /// letting the last pill fall off the edge — the count at the far right is
    /// the one the operator needs (it is the filter they have not tried yet).
    /// Every form is unambiguous at the lead: `cur`/`emp`/`live`/`int`/`br`/
    /// `arch` all start differently, so the compact row can be read.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::NeedsYou => "need",
            Self::Working => "work",
            Self::Ready => "ready",
            Self::Inactive => "idle",
            Self::CurrentDir => "cur",
            Self::CurrentBranch => "br",
        }
    }
}

/// How the visible rows are grouped, cycled with `g` (codex's `AgentsOverview`
/// `Group:` control). Grouping is ORTHOGONAL to filtering: the filter decides
/// which rows exist, grouping decides the order they are read in. Codex's four
/// axes are Project/Directory/Status/Model; operant's are its own data's axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubGroupMode {
    /// The order the rows arrived in — the global `updated_at DESC` the load
    /// already applies. The default, and the only mode that is not a regrade.
    Auto,
    /// Grouped by the session's working directory (`row.cwd`). Codex's
    /// `Project` mode keys on git repo identity (common_dir + relative_cwd) so
    /// worktrees of one repo collapse; operant does not resolve git identity
    /// here, so `Directory` is the honest name rather than a promise the data
    /// does not keep.
    Directory,
    /// Grouped by the employee tag — "who is executing this". The rows carrying
    /// no employee tag (personal sessions) form their own group rather than
    /// being dropped.
    Employee,
    /// Grouped by `RowStatus`, which is the question an operator opens the hub
    /// to answer: what was interrupted, what is live.
    Status,
}

impl HubGroupMode {
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Directory,
            Self::Directory => Self::Employee,
            Self::Employee => Self::Status,
            Self::Status => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "all",
            Self::Directory => "directory",
            Self::Employee => "employee",
            Self::Status => "status",
        }
    }

    /// The compact form, for the header when the employee chip has taken the
    /// room the long one used. The header measures 46 of the pane's 46 inner
    /// columns, so a chip displaces the group clause rather than being added
    /// beside it.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Auto => "all",
            Self::Directory => "dir",
            Self::Employee => "emp",
            Self::Status => "stat",
        }
    }

    /// The key that cycles it, for the header's affordance. Codex shows `g`
    /// next to the mode it is changing for exactly this reason.
    pub const fn key(self) -> &'static str {
        "g"
    }
}

/// Sort key for the visible rows, cycled with `o` (Stage E).
///
/// Codex's `ThreadSortKey` has exactly two usable keys — `CreatedAt` and
/// `UpdatedAt` (its `RecencyAt`/`SectionPosition` collapse onto `UpdatedAt`).
/// Operant's projection (`SessionEntryV2`, `session/listing.rs`) carries NO
/// creation timestamp — `updated_at` is `ended_at` falling back to
/// `started_at`, and that is the whole temporal axis a listed row has — so
/// `Created` is NOT in the ladder: a sort key that silently sorts by a
/// different field is worse than no key at all.
///
/// What operant's rows DO carry that answers a real question is usage:
/// `cost_usd`, `message_count` and `tokens`. So the ladder is
/// `Recent | Cost | Messages | Tokens` rather than codex's two, and codex's
/// `Model` axis is not here at all (no hub row carries a model field, so every
/// row would tie and the key would be decoration).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubSortMode {
    /// The load's own order (`updated_at DESC`), preserved so a fresh hub
    /// renders exactly as it did before Stage E. The default, and the only
    /// mode that is not a re-grade of the row's numbers.
    Auto,
    /// Most expensive first — "where is the money going".
    Cost,
    /// Busiest first — "where was the work".
    Messages,
    /// Largest first — "which sessions consumed the context window".
    Tokens,
}

impl HubSortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Cost,
            Self::Cost => Self::Messages,
            Self::Messages => Self::Tokens,
            Self::Tokens => Self::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "recent",
            Self::Cost => "cost",
            Self::Messages => "messages",
            Self::Tokens => "tokens",
        }
    }

    /// The key that cycles it, for the header's affordance.
    pub const fn key(self) -> &'static str {
        "o"
    }
}

/// Does this row pass this filter mode, with the process context the modes
/// need. ONE predicate for the visible set AND the tab bar's counts: two copies
/// of "what does `active` mean" would drift, and the count the operator reads
/// would then lie about what the list will show.
pub(super) fn row_matches_filter(
    mode: HubFilterMode,
    r: &SessionEntryV2,
    branch_root: Option<&str>,
    cwd: Option<&str>,
) -> bool {
    match mode {
        HubFilterMode::All => true,
        // "Needs you" is the session that was streaming when its owning
        // process went away: the only state that is an unfinished obligation
        // rather than a fact about the work. It had no tab at all before
        // (audit 2026-11-13, finding 4) — 22 sessions on the operator's
        // database, reachable only by reading 500 rows by eye.
        HubFilterMode::NeedsYou => matches!(r.status, RowStatus::Interrupted),
        // "Working" is the live session plus the delegations still pending.
        HubFilterMode::Working => {
            matches!(r.status, RowStatus::Live)
                || matches!(
                    &r.kind,
                    RowKind::Subagent {
                        status: "pending",
                        ..
                    }
                )
        }
        // "Ready" is finished and quiet.
        HubFilterMode::Ready => matches!(r.status, RowStatus::Idle),
        // "Inactive" is parked on purpose.
        HubFilterMode::Inactive => matches!(r.status, RowStatus::Archived),
        HubFilterMode::CurrentDir => r.cwd.as_deref().zip(cwd).is_some_and(|(a, b)| a == b),
        HubFilterMode::CurrentBranch => match branch_root {
            Some(root) => r.id == root || r.parent_id.as_deref() == Some(root),
            None => true,
        },
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
    /// How the visible rows are grouped. Orthogonal to `filter_mode`: the
    /// filter decides which rows exist, this decides the order they are read
    /// in, and the renderer draws a heading per group.
    pub group_mode: HubGroupMode,
    /// How the visible rows are ordered *within* their reading order (Stage E,
    /// `o`). Orthogonal to both the filter and the grouping: the sort key is a
    /// comparator over the flat `visible_idx`, so grouping stays a view and
    /// every navigation path is untouched. `Auto` (the default) is the order
    /// `rows` already arrived in — `updated_at DESC` from the database — so a
    /// fresh hub renders exactly as it did before Stage E.
    pub sort_mode: HubSortMode,
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
    /// Stage E archive confirm: `Some(id)` while the operator has pressed `a`
    /// on a row and is being asked to confirm before it is parked.
    pub pending_archive: Option<String>,
    /// The id of the session THIS process is streaming into, fed by the App
    /// from the agent's own session handle.
    ///
    /// Why this field exists: `RowStatus::Live` is documented as "the session
    /// this process is actively streaming into right now"
    /// (`operant_core::session::listing`), and `HubFilterMode::Working` selects
    /// it — but nothing ever SET it on a session row. The only production
    /// assignment of `Live` was inside the `RowKind::Subagent` arm of
    /// `refresh_subagent_rows`, so the "active" tab read 0 rows always, the ⚡
    /// glyph never appeared on a session, and the branch-root auto-detection
    /// below (which looks for a `Live` row) always missed (audit 2026-11-13,
    /// finding 2). The database cannot know it either — the comment on
    /// `list_sessions_rich` says "the hub sets `Live` on the current session id
    /// after loading", and that hub code did not exist. This is it.
    ///
    /// `None` before the agent's first turn mints an id: `agent.session_id()`
    /// is `None` at adapter construction (session_switch.rs:56-65 records the
    /// same fact), so a hub opened on a fresh start has no live session and
    /// every row stays Idle — which is the truth.
    live_session_id: Option<String>,
    /// The EMPLOYEE this list is restricted to, by identity — the second axis.
    ///
    /// The operator asked for "a separate filter for the agentic-os employee
    /// sessions, so that they can be filtered based on the employee/agent
    /// identity". The ladder's old `ByEmployee` mode was the boolean version
    /// of that and could not answer it: "by employee" says only THAT a
    /// session is employee-tagged, never WHICH employee, so with three
    /// distinct employees on the operator's database it was a filter that
    /// could not tell them apart.
    ///
    /// It is a SEPARATE axis, not a seventh tab, and that is deliberate:
    /// codex keeps its status filters (TASK_FILTERS) and its organisation
    /// axes (AgentsOverviewGrouping) apart for the same reason — the first is
    /// a QUESTION about the state of the work, the second is a way of looking
    /// at it, and folding them into one list of tabs makes both unreadable.
    ///
    /// `None` = every employee, which is the default: a filter that hid rows
    /// before the operator asked for one is a filter nobody can trust.
    employee_filter: Option<String>,
    /// Hide every tool call in the preview (`t`).
    ///
    /// The operator asked for "an option to hide all the tool-calls, so that
    /// the prompts, and the end of turn output remains". This is an operant
    /// feature, NOT a codex port: codex has only `RawReasoningVisibility`
    /// (codex-rs/tui/src/thread_transcript.rs:62-65), which hides the
    /// REASONING traces — codex has no way to hide tool calls at all.
    ///
    /// What it hides: the `[tool]` labels attached to an assistant turn, and
    /// the tool RESULT messages themselves. What it leaves: the prompts the
    /// operator typed and the assistant's end-of-turn output — which is
    /// exactly the pair the operator named.
    pub hide_tool_calls: bool,
    /// The session's FIRST user prompt, kept beside the tail window.
    ///
    /// The operator asked for "scrolling mechanism through the user-prompt,
    /// like first-message/last-message". The last message is already
    /// reachable — the preview's window IS the tail — but the FIRST prompt was
    /// discarded: the load read every message and `window_messages` kept the
    /// last 400, so the opening line of the conversation was read and thrown
    /// away on every load (audit finding 6).
    ///
    /// It is kept here rather than in a new column because it costs nothing:
    /// the load already has the messages, and the field is the answer to a
    /// question the operator asks of every session they open.
    preview_first_prompt: Option<(String, String)>,
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
            group_mode: HubGroupMode::Auto,
            sort_mode: HubSortMode::Auto,
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
            pending_archive: None,
            live_session_id: None,
            employee_filter: None,
            hide_tool_calls: false,
            preview_first_prompt: None,
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
        self.pending_archive = None;
        self.armed_close_tap = armed_close_tap;
        self.rebuild_visible();
    }

    /// Open with the cursor already on `session_id`, when the row set holds it.
    ///
    /// # Why this exists
    ///
    /// `open()` resets `selected = 0` and rebuilds by `updated_at DESC`, so the
    /// session the operator was just interrupted in is usually near the top but
    /// NOT at it — and once other sessions run it is not near the top at all.
    /// The interrupted-session notice (plan 2026-11-02) names that session in
    /// words; this is the *keyboard* path to the same target, so the first
    /// `/resume` or the first arrow press lands on the row the operator was
    /// already told about instead of an arbitrary one.
    ///
    /// # Why it degrades rather than fails
    ///
    /// The row set depends on the active filter and search, so the marked
    /// session may not be in `visible_idx` at all. Row 0 is the right answer
    /// then — it is exactly `open()`'s behaviour — and an error here would be
    /// worse than no cursor placement.
    ///
    /// Deliberately does NOT reorder the rows: the interrupted-first rank is a
    /// separate change owned by the session-organiser plan's Stage C, and
    /// pre-empting it here would put two agents in one ordering rule.
    pub fn open_selecting(&mut self, armed_close_tap: Option<Instant>, session_id: &str) {
        self.open(armed_close_tap);
        // The same lookup `set_rows` performs for its keep-id branch, so a miss
        // behaves identically: fall through to the clamped row-0 selection
        // rather than showing a preview for a row that is no longer selected.
        if let Some(pos) = self
            .visible_idx
            .iter()
            .position(|&i| self.rows[i].id == session_id)
        {
            self.selected = pos;
            self.preview_scroll = 0;
        }
        self.clamp_selection();
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

    /// Tell the hub which session this process is streaming into, so the row
    /// can be marked `RowStatus::Live`.
    ///
    /// Called by the App's session-list drain, whose whole job is "the rows
    /// landed, tell the hub about it" — the one moment the hub holds a full row
    /// set it is about to filter, group and render, and therefore the one place
    /// a `Live` mark cannot be missed by a path that never rebuilds the
    /// visible set.
    ///
    /// No-op on `None`, which is the truth for a hub opened before the agent's
    /// first turn mints an id.
    /// Cycle the EMPLOYEE filter (`e`): every employee → each in turn → back.
    ///
    /// The list is derived from the rows the operator is actually looking at,
    /// not from the seat registry: an employee with no loaded session would
    /// otherwise appear in the cycle and select nothing, which reads as a
    /// broken filter. Sorted, so the cycle's order is stable between loads.
    ///
    /// One employee is `None`-filtered back to "all", and "all" is always the
    /// first stop — an axis the operator cannot step onto without knowing its
    /// shape is an axis they will avoid.
    pub fn cycle_employee(&mut self) {
        let mut employees: Vec<String> = self
            .rows
            .iter()
            .filter_map(|r| r.employee_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        employees.sort();
        self.employee_filter = match self.employee_filter.take() {
            None => employees.first().cloned(),
            Some(current) => {
                let pos = employees.iter().position(|e| e == &current);
                match pos {
                    Some(i) if i + 1 < employees.len() => Some(employees[i + 1].clone()),
                    _ => None,
                }
            }
        };
        self.rebuild_visible();
    }

    /// Flip the tool-call visibility in the preview (`t`).
    ///
    /// A pure presentation flag: the transcript is NOT reloaded, because the
    /// tool calls are still in the cached messages and are still what the
    /// operator may want back with one more keypress. Re-fetching would be
    /// both slower and a worse answer.
    pub fn toggle_tool_calls(&mut self) {
        self.hide_tool_calls = !self.hide_tool_calls;
    }

    /// The employee this list is restricted to, for the header's chip.
    pub fn employee_filter(&self) -> Option<&str> {
        self.employee_filter.as_deref()
    }

    pub fn set_live_session_id(&mut self, id: Option<String>) {
        self.live_session_id = id;
    }

    /// Mark the row the agent is streaming into as `RowStatus::Live`.
    ///
    /// One assignment, applied to `self.rows` before every filter/group/sort
    /// derivation, so the mark cannot be stale for any of them. It overrides
    /// `Interrupted` deliberately: a turn START clears the interrupted marker
    /// (`turn_context`'s `clear_session_interrupted` runs before any work can
    /// be interrupted), so a session that is live now is not a session that
    /// was cut off — and `RowStatus::Interrupted`'s own doc comment ranks it
    /// below Archived precisely because it means "resumable accident".
    fn mark_live_row(&mut self) {
        let Some(live) = self.live_session_id.as_ref() else {
            return;
        };
        for row in self.rows.iter_mut() {
            if &row.id == live {
                row.status = RowStatus::Live;
            }
        }
    }

    /// Replace the backing rows (background load landed) and keep the
    /// selection on the same row id when possible.
    pub fn replace_rows(&mut self, rows: Vec<SessionEntryV2>) {
        let keep = self.selected_row().map(|r| r.id.clone());
        // (audit R1) the CurrentBranch filter needs an anchor. Without one
        // (the operator opened by Ctrl+B, not by a session switch), take the
        // live row — the only session the App could be streaming into.
        // This search used to always miss: no session row was ever marked
        // Live, so Ctrl+B silently fell through to `None` and the branch
        // filter showed every session in the store (audit 2026-11-13,
        // finding 2).
        if self.branch_root.is_none() {
            self.branch_root = rows
                .iter()
                .find(|r| matches!(r.status, RowStatus::Live))
                .map(|r| r.id.clone())
                .or_else(|| self.live_session_id.clone());
        }
        self.rows = rows;
        // The live mark is applied AFTER `rows` is assigned and BEFORE
        // `rebuild_visible`, so the filter counts, the grouping keys and the
        // status column all see the same row.
        self.mark_live_row();
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

    /// Which group this row falls into under `mode`. The key is a `String` and
    /// not a `Copy` enum because two of the four axes (Directory, Employee) are
    /// open sets read from the row's data, so the key IS the data.
    fn group_key(&self, row: &SessionEntryV2) -> String {
        match self.group_mode {
            HubGroupMode::Auto => String::new(),
            HubGroupMode::Directory => row.cwd.clone().unwrap_or_else(|| "(no cwd)".to_string()),
            HubGroupMode::Employee => row
                .employee_id
                .clone()
                .unwrap_or_else(|| "(personal)".to_string()),
            HubGroupMode::Status => row.status.api_label().to_string(),
        }
    }

    /// The visible rows as ordered groups, each group's rows kept in their
    /// existing relative order.
    ///
    /// This is the whole Stage C surface: `visible_idx` stays a FLAT list so
    /// every existing navigation path (Up/Down/select_next/clamp_selection and
    /// the callers that read `visible_rows()`) is untouched, and grouping is a
    /// VIEW over it rather than a change to the index model. Codex's
    /// `AgentsOverviewGroup` does the same — group headings are a rendering
    /// concern, not a second index.
    ///
    /// `Auto` returns ONE group with an empty label, which the renderer draws
    /// with no heading at all: the default must not add chrome.
    pub fn visible_groups(&self) -> Vec<(String, Vec<usize>)> {
        if self.group_mode == HubGroupMode::Auto {
            return vec![(String::new(), self.visible_idx.clone())];
        }
        // First-appearance order keeps the groups stable across refreshes, which
        // is what codex's `AgentsOverviewGroup` guarantees too: re-deriving the
        // groups from a re-loaded row set must not shuffle the section the
        // operator is reading.
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for &i in &self.visible_idx {
            let key = self.group_key(&self.rows[i]);
            if !groups.contains_key(&key) {
                order.push(key.clone());
            }
            groups.entry(key).or_default().push(i);
        }
        order
            .into_iter()
            .map(|k| {
                let rows = groups.remove(&k).unwrap_or_default();
                (k, rows)
            })
            .collect()
    }

    /// Cycle the grouping. The selection is preserved BY ROW ID rather than by
    /// position: grouping reorders the list, so position-based preservation
    /// would move the cursor to a different session and the operator would
    /// watch the highlight jump sideways while pressing `g`.
    pub fn cycle_group(&mut self) {
        let keep = self.selected_row().map(|r| r.id.clone());
        self.group_mode = self.group_mode.next();
        // Grouping now IS a reordering: contiguous sections (see
        // apply_group_order). The row-id restore below is what keeps the
        // cursor on the row the operator was reading — position-based
        // preservation would move the highlight to a different session.
        self.apply_group_order();
        if let Some(id) = keep
            && let Some(pos) = self.visible_idx.iter().position(|&i| self.rows[i].id == id)
        {
            self.selected = pos;
        }
        self.note_preview_miss();
    }

    /// Cycle the sort key. Like `cycle_group`, the selection is preserved BY
    /// ROW ID: a sort reorders the flat visible set, so position-based
    /// preservation would teleport the cursor to a different session and the
    /// operator would lose the row they were reading.
    pub fn cycle_sort(&mut self) {
        let keep = self.selected_row().map(|r| r.id.clone());
        self.sort_mode = self.sort_mode.next();
        self.rebuild_visible();
        if let Some(id) = keep
            && let Some(pos) = self.visible_idx.iter().position(|&i| self.rows[i].id == id)
        {
            self.selected = pos;
        }
        self.note_preview_miss();
    }

    /// The row the pending archive confirm names, as an index into `visible_idx`
    /// — the render position, so a heading can be marked on the screen.
    ///
    /// The confirm itself lives on `pending_archive` (the peer's iter-1671
    /// mechanism): ONE source of truth for "which row is being archived".
    pub(super) fn pending_archive_pos(&self) -> Option<usize> {
        let id = self.pending_archive.as_deref()?;
        self.visible_idx.iter().position(|&i| self.rows[i].id == id)
    }

    /// How many of ALL the rows each filter mode would show. This is the live
    /// count codex's tab bar carries (`agent_center`'s `render_filled_tab_bar`
    /// over `TASK_FILTERS`): the operator must see that `active` has 0 rows
    /// BEFORE committing to it, or the filter is a guess. With 3,266 rows in
    /// the local DB, cycling all six modes to find out was the entire problem.
    ///
    /// Search is deliberately excluded: while a search is active these answer
    /// "what would I see if I switched filters", which is the only question a
    /// static tab bar can answer. Mixing the live search in would make the
    /// numbers jump per keystroke and teach the operator to ignore them.
    pub fn filter_counts(&self) -> Vec<(HubFilterMode, usize)> {
        // The order comes from `next()` walking its own cycle, so the tab bar
        // can never disagree with the order `s` cycles in.
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        let mut modes = Vec::with_capacity(7);
        let mut mode = HubFilterMode::All;
        loop {
            modes.push(mode);
            mode = mode.next();
            if mode == HubFilterMode::All {
                break;
            }
        }
        modes
            .into_iter()
            .map(|mode| {
                let n = self
                    .rows
                    .iter()
                    .filter(|r| {
                        row_matches_filter(mode, r, self.branch_root.as_deref(), cwd.as_deref())
                    })
                    // The counts answer "how many rows would this tab show",
                    // so the employee axis has to be in them too: a count that
                    // ignored the axis would promise 22 rows and deliver 0.
                    .filter(|r| match self.employee_filter.as_deref() {
                        Some(emp) => r.employee_id.as_deref() == Some(emp),
                        None => true,
                    })
                    .count();
                (mode, n)
            })
            .collect()
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
            .filter(|(_, r)| {
                row_matches_filter(
                    self.filter_mode,
                    r,
                    self.branch_root.as_deref(),
                    cwd.as_deref(),
                )
            })
            .filter(|(_, r)| {
                // The employee axis is ANDed with the status ladder, never
                // merged into it: the two answer different questions ("which
                // state is this work in" and "whose is it"), and one predicate
                // feeds both the visible set and the tab counts so they can
                // never disagree.
                match self.employee_filter.as_deref() {
                    Some(emp) => r.employee_id.as_deref() == Some(emp),
                    None => true,
                }
            })
            .filter(|(_, r)| {
                // AND-token search over a lowercase haystack: title, id,
                // employee, goal, source, cwd (jcode's search_index shape).
                if tokens.is_empty() {
                    return true;
                }
                // id/source/employee lowercase with the rest: the tokens are
                // lowercased, so an exact-case id search found nothing while
                // title/cwd/goal searches were case-insensitive — per-field
                // behavior nobody could predict.
                let mut hay = format!(
                    "{} {} {}",
                    r.id.to_lowercase(),
                    r.source.to_lowercase(),
                    r.employee_id.clone().unwrap_or_default().to_lowercase()
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
        // Stage E: the sort key reorders the FLAT visible set, exactly the way
        // grouping reorders the reading order. It is applied last, so the
        // filter and the search never see a sorted view, and it is a stable
        // sort — a `sort_by_key` on equal keys leaves arrival order intact, so
        // `Auto` (and every tie under the other keys) still reads
        // `updated_at DESC` from the load.
        //
        // `Auto` sorts by nothing: the load's own order IS the `updated_at
        // DESC` answer, and re-sorting on it would only shuffle equal
        // timestamps.
        //
        // Every key sorts descending: the first row under each is the extreme
        // the operator asked about ("most expensive", "busiest", "largest").
        // `sort_by` is stable, so a tie leaves the arrival order — and therefore
        // the load's `updated_at DESC` — intact.
        match self.sort_mode {
            HubSortMode::Auto => {}
            HubSortMode::Cost => self.visible_idx.sort_by(|&a, &b| {
                self.rows[a]
                    .cost_usd
                    .partial_cmp(&self.rows[b].cost_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .reverse()
            }),
            HubSortMode::Messages => self.visible_idx.sort_by(|&a, &b| {
                self.rows[a]
                    .message_count
                    .cmp(&self.rows[b].message_count)
                    .reverse()
            }),
            HubSortMode::Tokens => self
                .visible_idx
                .sort_by(|&a, &b| self.rows[a].tokens.cmp(&self.rows[b].tokens).reverse()),
        }
        self.apply_group_order();
        self.clamp_selection();
    }

    /// Reorder the flat `visible_idx` into contiguous group sections whenever
    /// a grouping is active (a no-op under `Auto`). Grouping used to be a
    /// pure reading: the interposer drew a heading at each group's
    /// first-appearance flat position, so a group whose rows were
    /// non-contiguous in the flat order — the NORMAL case under
    /// `updated_at DESC` ties or Status grouping — rendered some of its rows
    /// AFTER a later group's heading, visually inside the wrong section, and
    /// every heading-offset consumer had to model the interleaving (the old
    /// take_while modelled it wrong; see group_heading_rows_above).
    /// Contiguous sections make the drawn layout and the arithmetic coincide
    /// by construction. Within a section the pre-grouping order is preserved
    /// (visible_groups partitions in visible_idx order), so the sort key
    /// still reads inside each section; groups keep first-appearance order,
    /// which is what keeps the sections stable across refreshes.
    fn apply_group_order(&mut self) {
        if self.group_mode == HubGroupMode::Auto {
            return;
        }
        let ordered: Vec<usize> = self
            .visible_groups()
            .into_iter()
            .flat_map(|(_, idxs)| idxs)
            .collect();
        debug_assert_eq!(ordered.len(), self.visible_idx.len());
        self.visible_idx = ordered;
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
    /// Record the session's FIRST user prompt, so the preview can show it
    /// beside the tail window.
    ///
    /// The id is carried with it for the same reason `apply_preview` carries
    /// one: the operator moves the selection while the load is in flight, and
    /// a prompt that arrives for a row the operator has left must not be
    /// painted on the row they are now looking at.
    pub fn set_first_prompt(&mut self, session_id: &str, prompt: Option<String>) {
        match prompt {
            Some(p) => self.preview_first_prompt = Some((session_id.to_string(), p)),
            None => {
                if self
                    .preview_first_prompt
                    .as_ref()
                    .is_some_and(|(id, _)| id == session_id)
                {
                    self.preview_first_prompt = None;
                }
            }
        }
    }

    /// The first prompt of the session whose preview is showing.
    pub fn first_prompt_for(&self, session_id: &str) -> Option<&str> {
        self.preview_first_prompt
            .as_ref()
            .filter(|(id, _)| id == session_id)
            .map(|(_, p)| p.as_str())
    }

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
    /// The FIRST user prompt in a message list.
    ///
    /// "First" means the first thing the operator TYPED: a system line is not
    /// a prompt, and an empty one is not a prompt either. The preview shows
    /// this above the tail window, so the operator can see how the
    /// conversation opened without scrolling a 400-message window back to the
    /// top.
    pub fn first_user_prompt(messages: &[PreviewMessage]) -> Option<String> {
        messages
            .iter()
            .find(|m| m.role == "user" && !m.content.trim().is_empty())
            .map(|m| m.content.trim().to_string())
    }

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

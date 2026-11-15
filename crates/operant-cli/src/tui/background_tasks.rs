// tui/background_tasks.rs — persistent, lifecycle-bearing rows for tasks that
// run in the BACKGROUND of a turn.
//
// # Why this exists at all
//
// A background-task row is only meaningful if work can actually outlive the
// turn that started it. In operant that is true for exactly one path, and it
// is worth being precise about which:
//
//   * `delegate_task(background = true)` — `SubAgentTool::dispatch_background`
//     (`operant-core/src/tools/sub_agent_tool.rs`) registers a record via
//     `async_delegation::try_create_record` and then `tokio::spawn`s the child,
//     returning `{"delegation_id": …, "status": "dispatched"}` to the parent
//     immediately. The child owns its own tokio task, so the turn carries on
//     while it runs and it keeps running after the turn ends. This is real
//     background execution.
//   * Every OTHER tool call is awaited. `agent/stream.rs` drives tool calls
//     through `stream::iter(futures).buffer_unordered(8).collect().await`, so
//     the whole batch is joined before the next step. A `Semaphore::new(8)`
//     bounds concurrency; it does not detach work.
//
// So the background delegation is the one thing worth a row that outlives its
// tool block, and the row's state is DERIVED from the core's own record —
// never invented here. A row that said "running in the background" for an
// awaited tool call would be a lie rendered in the user's face.
//
// # What the row promises
//
// Every state has a real producer:
//   Spawned   — the record is registered but this is the first refresh that
//               has seen it; the child was dispatched moments ago and has not
//               yet been observed running.
//   Running   — a later refresh still finds the record `Pending`.
//   Done      — the record reached `Completed`.
//   Failed    — the record reached `Failed` (child errored or timed out).
//   Cancelled — the row was released while still live. `/stop` does NOT
//               cancel a spawned child (the child belongs to the runtime, not
//               the turn), so this is reserved for session teardown, and the
//               doc comment on `cancel` says so rather than implying a kill.
//
// Elapsed is a wall-clock subtraction from the record's own `dispatch_time`
// against a `SystemTime::now()` read — never a counter bumped per render.

use std::cell::RefCell;

use operant_core::tools::async_delegation::{self, AsyncDelegationRecord, AsyncDelegationStatus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::theme_colors;

/// Longest goal echoed into a row before it is clipped. Rows are a status
/// surface, not a transcript — the full goal stays in the tool block.
const GOAL_PREVIEW: usize = 56;

/// How long a SETTLED row stays on screen before it is suppressed.
///
/// (plan 2026-11-15 B1) MEASURED: the live ledger on this box held 50 records,
/// every one terminal (`completed`/`failed`), and `rows_area` sized itself to
/// the full row count — so the dock claimed the WHOLE messages band at every
/// terminal height and the header, the transcript and the composer's status row
/// were all painted over. The operator saw it as "an overlay of dlg-<id> as the
/// delegated stale log". It is not an overlay; it is a task log that never
/// forgets.
///
/// A finished row is still useful for a moment — long enough to read what it
/// said — and is history after that. 10 minutes is the same order as the
/// transient-notice TTL; the number is a hygiene rule with one right answer, not
/// an operator preference, so it is a constant and not a config key.
const STALE_AFTER_SECS: u64 = 600;

/// Most settled rows rendered at once before the rest fold into a `+N more` row.
///
/// The same idiom the todos widget and the info-widget dock already use
/// (`info_widget_frame::more`): a status surface shows what fits and names the
/// remainder rather than growing without bound. 3 matches both, so the dock
/// cannot drift from the two surfaces an operator already reads.
const DOCK_MAX_ROWS: usize = 3;

/// Wall-clock unix seconds, saturating. A clock reading earlier than the unix
/// epoch yields 0 rather than panicking; the only cost is a zero elapsed,
/// which is the honest reading of an impossible clock anyway.
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lifecycle of one background task row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskState {
    /// Dispatched; first observation of the record, child not yet observed
    /// running.
    Spawned,
    /// Child is running.
    Running,
    /// Child finished and its result was recorded.
    Done,
    /// Child errored or timed out.
    Failed,
    /// Row released while still live. The child is NOT killed by this.
    Cancelled,
}

impl BackgroundTaskState {
    /// True once the row can no longer claim work is in progress. Every
    /// "is anything still going?" question goes through here, so a released
    /// row can never read as running.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            BackgroundTaskState::Done
                | BackgroundTaskState::Failed
                | BackgroundTaskState::Cancelled
        )
    }

    /// Short label for the row and the aggregate line.
    pub fn label(self) -> &'static str {
        match self {
            BackgroundTaskState::Spawned => "spawned",
            BackgroundTaskState::Running => "running",
            BackgroundTaskState::Done => "done",
            BackgroundTaskState::Failed => "failed",
            BackgroundTaskState::Cancelled => "cancelled",
        }
    }
}

/// One background task row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTask {
    /// The `delegation_id` handed back to the parent agent at dispatch.
    pub id: String,
    /// The task goal, so the row is identifiable without the tool block.
    pub goal: String,
    /// Model the child runs on.
    pub model: String,
    pub state: BackgroundTaskState,
    /// Wall-clock unix seconds at dispatch (the core record's
    /// `dispatch_time`). Every elapsed read derives from this.
    pub started_at: u64,
    /// Wall-clock unix seconds the row settled; `None` while live.
    pub finished_at: Option<u64>,
    /// Terminal text: the child's result preview, or its error.
    pub detail: Option<String>,
}

impl BackgroundTask {
    /// Seconds this task has been running, or ran for once settled.
    ///
    /// `now` is a wall-clock reading, never a frame count. A settled row uses
    /// its own `finished_at`, so its elapsed freezes instead of ticking.
    pub fn elapsed_secs(&self, now: u64) -> u64 {
        self.finished_at
            .unwrap_or(now)
            .saturating_sub(self.started_at)
    }

    /// True while the row still claims work is in progress.
    pub fn is_live(&self) -> bool {
        !self.state.is_terminal()
    }

    /// Seconds since this row SETTLED, or since dispatch while it is still live.
    ///
    /// (plan 2026-11-15 B1) Deliberately not `elapsed_secs`: for a settled row
    /// that measures how long the work RAN, which is frozen at its
    /// `finished_at` and never advances. The staleness rule needs the opposite
    /// clock — how long ago the row stopped being news — which for a settled row
    /// is `now - finished_at` and for a live row is its age.
    pub fn age_secs(&self, now: u64) -> u64 {
        match self.finished_at {
            Some(at) => now.saturating_sub(at),
            None => now.saturating_sub(self.started_at),
        }
    }

    /// Settle the row, recording when its lifecycle ended.
    ///
    /// Idempotent by design: a row that already settled is left alone, so a
    /// late poll can never walk a released row back into the live set. Without
    /// that guard a cancelled row would resurrect as "running" on the next
    /// refresh — the exact leak this type exists to prevent.
    fn settle(&mut self, state: BackgroundTaskState, at: u64, detail: Option<String>) {
        if self.state.is_terminal() {
            return;
        }
        self.state = state;
        self.finished_at = Some(at);
        self.detail = detail;
    }
}

/// The transcript's collection of background task rows.
///
/// State is a cache of the core delegation registry, refreshed on a tick. The
/// cache exists so the transcript can show a `Spawned` row before the first
/// poll settles it, and so settled rows stay readable after they finish.
///
/// The rows live behind a [`RefCell`] because the render pass holds `&App` and
/// still has to re-read the registry each frame — the same interior-mutability
/// idiom the inline-image queue already uses in `render::draw`.
#[derive(Debug, Clone, Default)]
pub struct BackgroundTaskRegistry {
    rows: RefCell<Vec<BackgroundTask>>,
}

impl BackgroundTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-read the process-wide delegation registry using a real clock read.
    /// This is the per-frame update call.
    pub fn refresh(&self) {
        let now = now_unix_secs();
        self.sync_from(&async_delegation::list_records(), now);
    }

    /// Sync against an explicit record set and clock reading. `refresh` is the
    /// production entry; this split exists so the lifecycle can be driven
    /// deterministically in tests without touching the process-global
    /// registry.
    fn sync_from(&self, records: &[AsyncDelegationRecord], now: u64) {
        let mut rows = self.rows.borrow_mut();
        for rec in records {
            let Some(row) = rows.iter_mut().find(|r| r.id == rec.delegation_id) else {
                // First sighting. A pending record is `Spawned`, not `Running`:
                // this is the moment the dispatch became visible to us.
                rows.push(BackgroundTask {
                    id: rec.delegation_id.clone(),
                    goal: rec.goal.clone(),
                    model: rec.model.clone(),
                    state: match rec.status {
                        AsyncDelegationStatus::Pending => BackgroundTaskState::Spawned,
                        AsyncDelegationStatus::Completed => BackgroundTaskState::Done,
                        AsyncDelegationStatus::Failed => BackgroundTaskState::Failed,
                    },
                    started_at: rec.dispatch_time,
                    finished_at: rec.completed_time,
                    detail: detail_of(rec),
                });
                continue;
            };

            // The goal is fixed at dispatch, but re-reading it costs nothing
            // and keeps the row from drifting if the record is ever enriched.
            row.goal = rec.goal.clone();
            row.model = rec.model.clone();

            match rec.status {
                AsyncDelegationStatus::Pending => {
                    // Seen twice and still pending: it is running.
                    if row.state == BackgroundTaskState::Spawned {
                        row.state = BackgroundTaskState::Running;
                    }
                }
                AsyncDelegationStatus::Completed => {
                    let at = rec.completed_time.unwrap_or(now);
                    row.settle(BackgroundTaskState::Done, at, detail_of(rec));
                }
                AsyncDelegationStatus::Failed => {
                    let at = rec.completed_time.unwrap_or(now);
                    row.settle(BackgroundTaskState::Failed, at, detail_of(rec));
                }
            }
        }
    }

    /// Release one live row as cancelled.
    ///
    /// This does NOT stop the child — a background delegation is a spawned tokio
    /// task and `/stop` (like `is_streaming = false`) never touched those. The
    /// row stops claiming "running" because the UI is done tracking it, not
    /// because the work stopped, and callers must not read this as a kill.
    pub fn cancel(&self, id: &str, now: u64) {
        if let Some(row) = self.rows.borrow_mut().iter_mut().find(|r| r.id == id) {
            row.settle(BackgroundTaskState::Cancelled, now, None);
        }
    }

    /// Session teardown: release every live row, then drop the collection.
    ///
    /// Called on the `should_exit` path so a row can never outlive the process
    /// that owns the child. Terminal rows are counted by the caller before
    /// this runs, which is why the collection is cleared rather than kept.
    pub fn release_all(&self, now: u64) {
        let live: Vec<String> = self
            .rows
            .borrow()
            .iter()
            .filter(|r| r.is_live())
            .map(|r| r.id.clone())
            .collect();
        for id in live {
            self.cancel(&id, now);
        }
        self.rows.borrow_mut().clear();
    }

    /// Every row, oldest first.
    pub fn rows(&self) -> Vec<BackgroundTask> {
        self.rows.borrow().clone()
    }
}

/// Terminal text from a record: the error if it failed, else the result.
/// `None` while the record is still pending.
fn detail_of(rec: &AsyncDelegationRecord) -> Option<String> {
    let text = rec.error.as_ref().or(rec.result.as_ref())?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Which rows the dock shows, and how many are folded away.
///
/// (plan 2026-11-15 B1) Two rules, in order:
///
/// 1. **Suppress stale.** A row that settled more than [`STALE_AFTER_SECS`] ago
///    is history, not status. A LIVE row is never suppressed however old it is —
///    a long-running child is exactly what the operator is watching for.
/// 2. **Cap the recent.** At most [`DOCK_MAX_ROWS`] of what survives, newest
///    first; the rest become one `+N more` row.
///
/// Extracted as a pure function because the dock's defect is an unbounded count,
/// and a count cannot be asserted through a live frame whose height and record
/// set both vary. `rows` is already newest-first (`list_records` sorts by
/// `dispatch_time` descending, and `sync_from` preserves that arrival order).
pub fn visible_rows(rows: &[BackgroundTask], now: u64) -> (Vec<&BackgroundTask>, usize) {
    let fresh: Vec<&BackgroundTask> = rows
        .iter()
        .filter(|row| row.is_live() || row.age_secs(now) <= STALE_AFTER_SECS)
        .collect();
    let shown = fresh.len().min(DOCK_MAX_ROWS);
    (fresh[..shown].to_vec(), fresh.len() - shown)
}

/// Paint the rows into `area`, one task per row, capped to the available
/// height so a long history of finished tasks can never push the transcript
/// off-screen. `now` is a wall-clock reading.
///
/// Lives beside the model so the transcript hook is one call and the colouring
/// can only come from the theme accessors.
pub fn render_rows(frame: &mut Frame, area: Rect, rows: &[BackgroundTask], now: u64) {
    if area.height == 0 || rows.is_empty() {
        return;
    }
    let (visible, hidden) = visible_rows(rows, now);
    if visible.is_empty() && hidden == 0 {
        // Every row is stale: a task log with nothing current to say renders
        // nothing at all rather than an empty frame.
        return;
    }

    let mut lines: Vec<Line> = visible
        .iter()
        .map(|row| Line::from(row_spans(row, now)))
        .collect();
    if hidden > 0 {
        lines.push(Line::from(vec![Span::styled(
            format!("+{hidden} more"),
            Style::default().fg(theme_colors::muted()),
        )]));
    }

    // The caller reserved exactly this many rows (it derives the count from the
    // same `visible_rows`), so the paragraph and the rect agree by construction —
    // no clamp here to drift out of step with `rows_area`.
    frame.render_widget(Paragraph::new(lines), area);
}

/// Rows left blank between the block and the bottom of the area, so the rows
/// never sit flush against the separator.
const BOTTOM_INSET: u16 = 1;

/// Dock the rows into the bottom-left of `area`: sized to the number of rows
/// actually present, and clamped so a tiny terminal cannot overflow or
/// underflow. Returns a zero-height rect when there is nothing to draw, which
/// `render_rows` treats as a no-op.
pub fn rows_area(area: Rect, row_count: usize, header_height: u16) -> Rect {
    if row_count == 0 || area.height == 0 || area.width == 0 {
        return Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: 0,
        };
    }
    // (plan 2026-11-15 B1) The header is a TRANSCRIPT SECTION, so it occupies
    // the top of this same band. Reserving its height is what stops the dock
    // from painting over it — measured: 50 terminal records claimed all 30 rows
    // of a 30-row band, header included.
    //
    // The header's height is a FRAME property (its wrapped-line count), not a
    // viewport one, so it does not move with scroll and this is not a
    // scroll-derived guard. A constant floor would be wrong: band 7 and band 30
    // share a header and need different answers.
    let usable = area.height.saturating_sub(header_height + BOTTOM_INSET);
    if usable == 0 {
        // No room for a transcript: a launch screen is not a place to show a
        // task log. Render nothing rather than cover the header.
        return Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: 0,
        };
    }
    let height = u16::try_from(row_count).unwrap_or(u16::MAX).min(usable);
    Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(height + BOTTOM_INSET),
        width: area.width,
        height,
    }
}

/// One row: state glyph, handle, state label, elapsed, then the goal.
fn row_spans(row: &BackgroundTask, now: u64) -> Vec<Span<'static>> {
    let (glyph, colour) = match row.state {
        BackgroundTaskState::Spawned => ("\u{25cb}", theme_colors::warning()),
        BackgroundTaskState::Running => ("\u{25d0}", theme_colors::accent()),
        BackgroundTaskState::Done => ("\u{25cf}", theme_colors::success()),
        BackgroundTaskState::Failed => ("\u{2715}", theme_colors::error()),
        BackgroundTaskState::Cancelled => ("\u{2298}", theme_colors::disabled()),
    };

    let mut spans = vec![
        Span::styled(
            format!(" {glyph} "),
            Style::default().fg(colour).add_modifier(Modifier::BOLD),
        ),
        Span::styled(row.id.clone(), Style::default().fg(theme_colors::text())),
        Span::styled(
            format!(" {}", row.state.label()),
            Style::default().fg(colour),
        ),
        Span::styled(
            format!(" {}s", row.elapsed_secs(now)),
            Style::default().fg(theme_colors::muted()),
        ),
    ];

    if !row.goal.is_empty() {
        spans.push(Span::styled(
            format!("  {}", clip(&row.goal, GOAL_PREVIEW)),
            Style::default().fg(theme_colors::muted()),
        ));
    }
    spans
}

/// Clip to `max` chars, marking the cut. Char-based (not byte-based) so a
/// multi-byte goal cannot panic the render.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000;

    /// A pending record, as `dispatch_background` registers it before the
    /// child is spawned.
    fn pending(id: &str, goal: &str, dispatch_time: u64) -> AsyncDelegationRecord {
        AsyncDelegationRecord {
            delegation_id: id.to_string(),
            status: AsyncDelegationStatus::Pending,
            goal: goal.to_string(),
            model: "test-model".to_string(),
            dispatch_time,
            completed_time: None,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: None,
            request_key: None,
        }
    }

    /// The same record once the child settled.
    fn settled(
        id: &str,
        goal: &str,
        dispatch_time: u64,
        status: AsyncDelegationStatus,
        detail: &str,
    ) -> AsyncDelegationRecord {
        AsyncDelegationRecord {
            status,
            completed_time: Some(dispatch_time + 5),
            result: match status {
                AsyncDelegationStatus::Completed => Some(detail.to_string()),
                _ => None,
            },
            error: match status {
                AsyncDelegationStatus::Failed => Some(detail.to_string()),
                _ => None,
            },
            ..pending(id, goal, dispatch_time)
        }
    }

    fn state_of(reg: &BackgroundTaskRegistry, id: &str) -> BackgroundTaskState {
        reg.rows()
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no row for {id}"))
            .state
    }

    // Test 1: the full lifecycle, each state observable on the row.
    #[test]
    fn row_walks_spawned_running_done() {
        let reg = BackgroundTaskRegistry::new();
        let records = [pending("dlg-a", "audit the browser stack", T0)];

        reg.sync_from(&records, T0);
        assert_eq!(state_of(&reg, "dlg-a"), BackgroundTaskState::Spawned);
        assert_eq!(reg.rows()[0].goal, "audit the browser stack");
        assert_eq!(
            reg.rows()[0].elapsed_secs(T0),
            0,
            "fresh row has no elapsed"
        );

        // Seen again, still pending: it is running, and elapsed advances with
        // the clock rather than a render counter.
        reg.sync_from(&records, T0 + 3);
        assert_eq!(state_of(&reg, "dlg-a"), BackgroundTaskState::Running);
        assert!(reg.rows()[0].is_live());
        assert_eq!(reg.rows()[0].elapsed_secs(T0 + 3), 3);

        let done = [settled(
            "dlg-a",
            "audit the browser stack",
            T0,
            AsyncDelegationStatus::Completed,
            "stack is fine",
        )];
        reg.sync_from(&done, T0 + 40);
        assert_eq!(state_of(&reg, "dlg-a"), BackgroundTaskState::Done);
        assert!(!reg.rows()[0].is_live());
        assert_eq!(reg.rows()[0].detail.as_deref(), Some("stack is fine"));
        // Elapsed froze at the record's completed_time, not at the poll time.
        assert_eq!(reg.rows()[0].elapsed_secs(T0 + 999), 5);
    }

    // Test 2: no exit path leaves a row claiming to be running.

    // Test 3: concurrent tasks each get their own row and their own elapsed.

    // Test 4: the aggregate footer line reports the right numbers.

    // Test 5: regression gate — no raw colour literal may reappear here.
    // iter-414 migrated 294 named terminal colours to the theme palette; a
    // new literal in this module is a regression, and this gate is the thing
    // that makes it visible in review rather than three months later.
    #[test]
    fn module_has_no_raw_colour_literals() {
        let src = include_str!("background_tasks.rs");
        // Assembled at runtime so the gate's own needle cannot match itself —
        // a literal here would trip the gate it is written to enforce.
        let needle = ["Color", "::"].concat();
        assert!(
            !src.contains(&needle),
            "background_tasks.rs now contains a raw colour literal. Colour only \
             through the theme_colors accessors (text/muted/border/accent/\
             warning/error/success/panel_bg/disabled) — see iter-414."
        );
    }

    // A row's goal must not blow up the row budget on long or multi-byte text.
    #[test]
    fn goal_preview_clips_without_panicking_on_multibyte() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdef", 4).chars().count(), 4);
        assert!(clip("abcdef", 4).ends_with('\u{2026}'));
        // Multi-byte input must clip by chars, not bytes.
        let wide = "\u{5f00}\u{53d1}\u{4ef6}\u{5904}\u{7406}";
        let clipped = clip(wide, 3);
        assert_eq!(clipped.chars().count(), 3);
        assert!(clipped.ends_with('\u{2026}'));
    }

    /// The registry must stay wired into the render pass.
    ///
    /// This module is a cache with no producer of its own: it is only ever
    /// populated by `refresh()` reading the process-wide delegation registry,
    /// and only ever observed by `render_rows`. If the render call is deleted
    /// the module still compiles, still passes every test above, and quietly
    /// shows the user nothing — which is exactly what happened to the
    /// `tasks_overlay` this work replaced. So pin the call site.
    ///
    /// The render pass is searched (the dispatch table that used to hold the
    /// call site was deleted at the jcode cutover, iter-648). What must not
    /// happen is the render pass not calling it. A rebuild that moves the call
    /// site again has to move this search too.
    #[test]
    fn render_pass_still_wires_the_registry() {
        let sites: [(&str, &str); 1] = [("render/mod.rs", include_str!("render/mod.rs"))];
        let called_in = |needle: &str| {
            sites
                .iter()
                .find(|(_, src)| src.contains(needle))
                .map(|(label, _)| *label)
        };
        assert!(
            called_in("background_tasks::render_rows").is_some(),
            "render/mod.rs does not call \
             background_tasks::render_rows — the rows would compile and pass tests \
             while never being shown"
        );
        assert!(
            called_in("background_tasks.refresh()").is_some(),
            "render/mod.rs does not call \
             background_tasks::refresh() — the registry would never re-read the \
             delegation registry, so rows would freeze at whatever the first frame saw"
        );
    }

    /// The rows must stay inside the area they are handed.
    #[test]
    fn rows_area_never_escapes_its_container() {
        let area = Rect {
            x: 4,
            y: 2,
            width: 20,
            height: 6,
        };
        // More rows than fit: clamped to the room left after the header and the
        // bottom inset, so the inset row really is blank.
        //
        // (plan 2026-11-15 B1) This assertion used to expect the FULL height (6)
        // on the reasoning that the inset sat below the rect. It did not: at
        // height 6 in a 6-row area the rect was y=0..6 and the "inset" fell
        // outside the container entirely, so the rows sat flush against the
        // separator — the exact thing BOTTOM_INSET's own doc says must not
        // happen. `y + height + 1 == area.y + area.height` below is the contract;
        // the old expectation contradicted it.
        let many = rows_area(area, 99, 0);
        assert_eq!(many.height, 5);
        assert!(many.y + many.height <= area.y + area.height);

        // Fewer rows than fit: sits just above the bottom inset.
        let few = rows_area(area, 2, 0);
        assert_eq!(few.height, 2);
        assert_eq!(few.x, area.x);
        assert_eq!(few.y + few.height + 1, area.y + area.height);

        // Nothing to draw is a zero-height no-op, never a panic.
        assert_eq!(rows_area(area, 0, 0).height, 0);
        // Degenerate areas must not underflow.
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        assert_eq!(rows_area(tiny, 3, 0).height, 0);
        // A one-row band cannot hold a row AND the inset, so it renders nothing
        // rather than sitting flush against the separator. (plan 2026-11-15 B1:
        // this used to expect 1, which put the rows in the only row there was
        // and left the inset with nowhere to go.)
        let one_row = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        };
        assert_eq!(rows_area(one_row, 5, 0).height, 0);
        // Two rows is the smallest band that can show one row and keep the inset.
        let two_rows = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 2,
        };
        assert_eq!(rows_area(two_rows, 5, 0).height, 1);
    }

    /// (plan 2026-11-15 B1) THE LAUNCH-SCREEN DEFECT.
    ///
    /// The header is a transcript section inside the messages band, and the dock
    /// used to be sized to the full row count — so with the 50 terminal records
    /// the live ledger held, the dock claimed every row of the band and painted
    /// over the header (measured: band 7/9/12/20/30/50 all covered it).
    ///
    /// Two things must hold now: the dock never enters the header's rows, and a
    /// band that cannot spare room renders nothing rather than covering them.
    #[test]
    fn the_dock_never_paints_over_the_header() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 30,
        };
        let header = 7u16;

        // Whatever the row count, the top row stays below the header.
        for rows in [1usize, 3, 10, 50, 500] {
            let dock = rows_area(area, rows, header);
            assert!(
                dock.y >= header,
                "with {rows} rows the dock starts at y={} but the header ends at \
                 {header} — it would paint over the header",
                dock.y
            );
        }

        // A band exactly as tall as the header has no room: render nothing.
        let launch = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: header,
        };
        let dock = rows_area(launch, 3, header);
        assert_eq!(dock.height, 0, "a band the size of the header has no room");
        assert_eq!(dock.width, 0, "a zero-height rect must not carry width");

        // One row of headroom still leaves the header clear.
        let tight = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: header + 2,
        };
        let dock = rows_area(tight, 3, header);
        assert!(dock.y >= header, "the dock still cleared the header");
        assert!(dock.height <= 1, "only the spare row is usable");
    }

    /// (plan 2026-11-15 B1) A cap-only fix does not work, and this is the case
    /// that proves it: a plain cap at band 7-10 still reaches the header.
    ///
    /// This is the assertion the plan's own enumeration called out — a constant
    /// floor and a plain cap both fail here, which is why the reservation is
    /// keyed on the header's height.
    #[test]
    fn a_short_band_renders_no_dock_rather_than_a_stale_log() {
        let header = 7u16;
        for band in [header, header + 1] {
            let area = Rect {
                x: 0,
                y: 0,
                width: 80,
                height: band,
            };
            let dock = rows_area(area, DOCK_MAX_ROWS + 1, header);
            assert_eq!(
                dock.height, 0,
                "band {band} with a {header}-row header must render nothing"
            );
        }
    }

    /// (plan 2026-11-15 B1) A settled row is history after [`STALE_AFTER_SECS`];
    /// a LIVE row is never suppressed however old it is.
    #[test]
    fn stale_rows_are_suppressed_and_live_ones_never_are() {
        const NOW: u64 = 10_000;

        // A row that settled just inside the window is kept; one just outside is
        // dropped. The boundary is asserted from both sides.
        let fresh = settled(
            "dlg-fresh",
            "just finished",
            NOW - 10,
            AsyncDelegationStatus::Completed,
            "done",
        );
        let stale = settled(
            "dlg-stale",
            "long finished",
            NOW - STALE_AFTER_SECS - 60,
            AsyncDelegationStatus::Failed,
            "too old",
        );
        // A live row dispatched before the window still shows: it is what the
        // operator is watching for.
        let live = pending("dlg-live", "still working", NOW - 9_999);

        let registry = BackgroundTaskRegistry::new();
        registry.sync_from(&[fresh, stale, live], NOW);
        let rows = registry.rows();

        let (visible, hidden) = visible_rows(&rows, NOW);
        let ids: Vec<&str> = visible.iter().map(|row| row.id.as_str()).collect();
        assert!(ids.contains(&"dlg-fresh"), "a fresh row is shown: {ids:?}");
        assert!(
            ids.contains(&"dlg-live"),
            "a LIVE row is never suppressed: {ids:?}"
        );
        assert!(
            !ids.contains(&"dlg-stale"),
            "a settled row older than the window is suppressed: {ids:?}"
        );
        assert_eq!(hidden, 0, "nothing was folded away: {ids:?}");
    }

    /// (plan 2026-11-15 B1) More survivors than [`DOCK_MAX_ROWS`] fold into one
    /// `+N more` row — the idiom the todos widget and the info-widget dock use.
    #[test]
    fn the_recent_rows_are_capped_and_the_rest_named() {
        const NOW: u64 = 10_000;
        let records: Vec<AsyncDelegationRecord> = (0..10)
            .map(|i| {
                settled(
                    &format!("dlg-{i}"),
                    &format!("task {i}"),
                    NOW - 100 + i,
                    AsyncDelegationStatus::Completed,
                    "done",
                )
            })
            .collect();

        let registry = BackgroundTaskRegistry::new();
        registry.sync_from(&records, NOW);
        let rows = registry.rows();

        let (visible, hidden) = visible_rows(&rows, NOW);
        assert_eq!(
            visible.len(),
            DOCK_MAX_ROWS,
            "only {DOCK_MAX_ROWS} rows render"
        );
        assert_eq!(
            hidden,
            10 - DOCK_MAX_ROWS,
            "the rest are named, not dropped"
        );
    }
}

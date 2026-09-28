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

/// Aggregate counts for the footer line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackgroundTaskCounts {
    /// `Spawned` + `Running` — work claimed to be in progress.
    pub running: usize,
    pub finished: usize,
    pub failed: usize,
    /// Released while live. Kept apart from `failed`: nothing went wrong, the
    /// row was retired.
    pub cancelled: usize,
}

impl BackgroundTaskCounts {
    /// True when there is nothing at all to report, so the footer can stay
    /// silent on an idle session.
    pub fn is_empty(&self) -> bool {
        self.running == 0 && self.finished == 0 && self.failed == 0 && self.cancelled == 0
    }

    /// The aggregate footer label, e.g. `2 running · 1 done · 1 failed`.
    /// `cancelled` is appended only when non-zero so the common case reads
    /// exactly the three numbers that matter.
    pub fn label(&self) -> String {
        let mut parts = vec![
            format!("{} running", self.running),
            format!("{} done", self.finished),
            format!("{} failed", self.failed),
        ];
        if self.cancelled > 0 {
            parts.push(format!("{} cancelled", self.cancelled));
        }
        parts.join(" \u{00b7} ")
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

    /// Aggregate counts for the footer line.
    pub fn counts(&self) -> BackgroundTaskCounts {
        let mut counts = BackgroundTaskCounts::default();
        for row in self.rows.borrow().iter() {
            match row.state {
                BackgroundTaskState::Spawned | BackgroundTaskState::Running => counts.running += 1,
                BackgroundTaskState::Done => counts.finished += 1,
                BackgroundTaskState::Failed => counts.failed += 1,
                BackgroundTaskState::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }
}

/// Terminal text from a record: the error if it failed, else the result.
/// `None` while the record is still pending.
fn detail_of(rec: &AsyncDelegationRecord) -> Option<String> {
    let text = rec.error.as_ref().or(rec.result.as_ref())?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
    let start = rows.len().saturating_sub(usize::from(area.height));
    let lines: Vec<Line> = rows[start..]
        .iter()
        .map(|row| Line::from(row_spans(row, now)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Rows left blank between the block and the bottom of the area, so the rows
/// never sit flush against the separator.
const BOTTOM_INSET: u16 = 1;

/// Dock the rows into the bottom-left of `area`: sized to the number of rows
/// actually present, and clamped so a tiny terminal cannot overflow or
/// underflow. Returns a zero-height rect when there is nothing to draw, which
/// `render_rows` treats as a no-op.
pub fn rows_area(area: Rect, row_count: usize) -> Rect {
    if row_count == 0 || area.height == 0 || area.width == 0 {
        return Rect {
            x: area.x,
            y: area.y,
            width: 0,
            height: 0,
        };
    }
    let height = u16::try_from(row_count)
        .unwrap_or(u16::MAX)
        .min(area.height);
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
    #[test]
    fn error_and_cancellation_release_the_row() {
        // A failing child settles as Failed and stops being live.
        let reg = BackgroundTaskRegistry::new();
        reg.sync_from(&[pending("dlg-fail", "risky task", T0)], T0);
        reg.sync_from(&[pending("dlg-fail", "risky task", T0)], T0 + 2);
        assert_eq!(state_of(&reg, "dlg-fail"), BackgroundTaskState::Running);

        reg.sync_from(
            &[settled(
                "dlg-fail",
                "risky task",
                T0,
                AsyncDelegationStatus::Failed,
                "child timed out",
            )],
            T0 + 20,
        );
        assert_eq!(state_of(&reg, "dlg-fail"), BackgroundTaskState::Failed);
        assert!(!reg.rows()[0].is_live());
        assert_eq!(reg.counts().running, 0);
        assert_eq!(reg.counts().failed, 1);
        assert_eq!(reg.rows()[0].detail.as_deref(), Some("child timed out"));

        // An explicit cancel retires a live row.
        let reg = BackgroundTaskRegistry::new();
        reg.sync_from(&[pending("dlg-cancel", "long job", T0)], T0);
        reg.sync_from(&[pending("dlg-cancel", "long job", T0)], T0 + 7);
        assert!(reg.rows()[0].is_live());

        reg.cancel("dlg-cancel", T0 + 9);
        assert_eq!(state_of(&reg, "dlg-cancel"), BackgroundTaskState::Cancelled);
        assert!(!reg.rows()[0].is_live(), "cancelled row must not read live");
        assert_eq!(reg.rows()[0].elapsed_secs(T0 + 500), 9);
        assert_eq!(reg.counts().running, 0, "cancelled is not running");
        assert_eq!(reg.counts().cancelled, 1);

        // A late poll must NOT resurrect a cancelled row: that is the leak
        // the settle() idempotence guard exists to prevent.
        reg.sync_from(&[pending("dlg-cancel", "long job", T0)], T0 + 11);
        assert_eq!(state_of(&reg, "dlg-cancel"), BackgroundTaskState::Cancelled);
        assert_eq!(reg.counts().running, 0);

        // Session teardown releases everything live and clears the rows.
        let reg = BackgroundTaskRegistry::new();
        reg.sync_from(
            &[pending("dlg-1", "one", T0), pending("dlg-2", "two", T0)],
            T0,
        );
        assert_eq!(reg.counts().running, 2);
        reg.release_all(T0 + 4);
        assert!(reg.rows().is_empty(), "teardown must not leave rows behind");
        assert!(reg.counts().is_empty());
    }

    // Test 3: concurrent tasks each get their own row and their own elapsed.
    #[test]
    fn concurrent_tasks_get_independent_rows_and_elapsed() {
        let reg = BackgroundTaskRegistry::new();
        let records = [
            pending("dlg-1", "first", T0),
            pending("dlg-2", "second", T0 + 30),
            pending("dlg-3", "third", T0 + 60),
        ];

        reg.sync_from(&records, T0 + 100);
        // Second sighting promotes all three out of Spawned.
        reg.sync_from(&records, T0 + 100);

        assert_eq!(reg.rows().len(), 3, "each task gets its own row");
        for row in reg.rows() {
            assert_eq!(row.state, BackgroundTaskState::Running);
        }

        let elapsed = |id: &str| {
            reg.rows()
                .iter()
                .find(|r| r.id == id)
                .unwrap()
                .elapsed_secs(T0 + 100)
        };
        assert_eq!(elapsed("dlg-1"), 100);
        assert_eq!(elapsed("dlg-2"), 70);
        assert_eq!(elapsed("dlg-3"), 40);

        // Settling one does not disturb the others.
        reg.sync_from(
            &[
                settled(
                    "dlg-2",
                    "second",
                    T0 + 30,
                    AsyncDelegationStatus::Completed,
                    "ok",
                ),
                pending("dlg-1", "first", T0),
                pending("dlg-3", "third", T0 + 60),
            ],
            T0 + 101,
        );
        assert_eq!(state_of(&reg, "dlg-2"), BackgroundTaskState::Done);
        assert_eq!(state_of(&reg, "dlg-1"), BackgroundTaskState::Running);
        assert_eq!(state_of(&reg, "dlg-3"), BackgroundTaskState::Running);
        assert_eq!(reg.counts().running, 2);
        assert_eq!(reg.counts().finished, 1);
    }

    // Test 4: the aggregate footer line reports the right numbers.
    #[test]
    fn aggregate_counts_and_label_are_correct() {
        let reg = BackgroundTaskRegistry::new();
        assert!(reg.counts().is_empty());
        assert_eq!(
            BackgroundTaskCounts::default().label(),
            "0 running \u{00b7} 0 done \u{00b7} 0 failed"
        );

        let records = [
            pending("dlg-1", "a", T0),
            pending("dlg-2", "b", T0),
            pending("dlg-3", "c", T0),
        ];
        reg.sync_from(&records, T0);
        reg.sync_from(&records, T0 + 1);
        assert_eq!(reg.counts().running, 3);

        // One done, one failed, one still running.
        reg.sync_from(
            &[
                settled("dlg-1", "a", T0, AsyncDelegationStatus::Completed, "done"),
                settled("dlg-2", "b", T0, AsyncDelegationStatus::Failed, "boom"),
                pending("dlg-3", "c", T0),
            ],
            T0 + 10,
        );
        let counts = reg.counts();
        assert_eq!(counts.running, 1);
        assert_eq!(counts.finished, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.cancelled, 0);
        assert_eq!(
            counts.label(),
            "1 running \u{00b7} 1 done \u{00b7} 1 failed"
        );

        // Cancelled rows are reported apart from failed, not folded in.
        reg.cancel("dlg-3", T0 + 11);
        let counts = reg.counts();
        assert_eq!(counts.running, 0);
        assert_eq!(counts.failed, 1, "a cancel is not a failure");
        assert_eq!(
            counts.label(),
            "0 running \u{00b7} 1 done \u{00b7} 1 failed \u{00b7} 1 cancelled"
        );
    }

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
    #[test]
    fn render_pass_still_wires_the_registry() {
        let render = include_str!("render/mod.rs");
        assert!(
            render.contains("background_tasks::render_rows"),
            "render/mod.rs no longer calls background_tasks::render_rows — the rows \
             would compile and pass tests while never being shown"
        );
        assert!(
            render.contains("background_tasks.refresh()"),
            "render/mod.rs no longer calls background_tasks::refresh() — the registry \
             would never re-read the delegation registry, so rows would freeze at \
             whatever the first frame saw"
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
        // More rows than fit: clamped to the full height, offset by the inset.
        let many = rows_area(area, 99);
        assert_eq!(many.height, 6);
        assert!(many.y + many.height <= area.y + area.height);

        // Fewer rows than fit: sits just above the bottom inset.
        let few = rows_area(area, 2);
        assert_eq!(few.height, 2);
        assert_eq!(few.x, area.x);
        assert_eq!(few.y + few.height + 1, area.y + area.height);

        // Nothing to draw is a zero-height no-op, never a panic.
        assert_eq!(rows_area(area, 0).height, 0);
        // Degenerate areas must not underflow.
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        assert_eq!(rows_area(tiny, 3).height, 0);
        let one_row = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 1,
        };
        assert_eq!(rows_area(one_row, 5).height, 1);
    }
}

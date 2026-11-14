//! `operant continue` — put the operator back into a session they just closed.
//!
//! # WHY THIS EXISTS
//!
//! Three surfaces existed and none of them connected:
//!
//! * the TUI could switch sessions from inside itself (the `/session` hub);
//! * `operant sessions resume <id>` could recap a session from the shell;
//! * nothing could actually REOPEN one.
//!
//! The session id was reachable only through `operant sessions list` — a lookup
//! the operator has to remember to make while holding a half-finished task in
//! their head, which is exactly the state they are in when they close the TUI
//! and come back an hour later.
//!
//! So the TUI now leaves a LEFTOVER: the exact command that reopens what it
//! was working on. Codex does the same when its TUI closes ("To continue this
//! session, run: `codex resume <id>`" — codex-rs/tui/src/app/exit_summary.rs),
//! and this module owns both halves of that contract: the line the leftover
//! prints, and the command the line names.

use anyhow::{Context, Result};
use operant_core::config::AppConfig;
use operant_core::database::Database;
use operant_core::session_interrupted::SessionInterrupted;

/// The session `operant continue` resolved to.
#[derive(Debug)]
struct TargetSession {
    id: String,
    /// Best-effort human label. The id is opaque (`sess_3f9a…`), so the title
    /// is the only part of the leftover an operator can recognise.
    title: Option<String>,
}

/// The command line that continues `session_id`.
///
/// One function because TWO surfaces print it (the TUI exit leftover and the
/// no-TTY fallback), and a drift between them would leave the operator holding
/// a command that does not exist.
pub fn continue_command_line(session_id: &str) -> String {
    format!("operant continue {session_id}")
}

/// The copy-pasteable way back to a session, as one annotation.
///
/// `state` says what the session IS at this surface — "Session saved" after
/// the TUI closed, "Last session was interrupted" when the next launch finds
/// the marker, "Current session" while the TUI is open. The command is the
/// same in all three; only the sentence above it changes, and it HAS to
/// change, because this annotation outlives the status line that introduced
/// it and has to make sense on its own.
///
/// The label falls back to the session id, never to nothing: an id the
/// operator cannot see is an id they cannot type.
pub fn format_continue_hint(state: &str, session_id: &str, title: Option<&str>) -> String {
    let label = title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(session_id);
    format!(
        "{state} — {label}\nTo continue this session, run:\n  {}",
        continue_command_line(session_id)
    )
}

/// The headline for a leftover that was printed WITHOUT a teardown deciding
/// what the exit was.
///
/// A PANIC is the one exit that reaches the leftover without going through
/// `ExitKind`: `run_exit_teardown` never runs on the unwind path, so
/// there is no deliberate/interrupted flag to read. The sentence still must
/// not claim a save that never happened — `Session ended` says the process
/// is gone (which is true) and nothing about whether the session was cut off
/// (which the panic path does not know).
pub const FORCED_EXIT_STATE: &str = "Session ended";

/// Print a leftover the teardown never got to write, on the way out of a
/// panic.
///
/// The normal exit leftover lives in `TuiApp::run`'s teardown, which an
/// unwind skips entirely. A session that was streaming when the process
/// died is exactly the one an operator most needs to get back into, so the
/// same copy-pasteable block is printed here, after the terminal has been
/// restored and the alternate screen popped — the same two conditions the
/// teardown's own print is exempted on.
///
/// Best-effort by construction: the terminal restore is allowed to fail (the
/// panic hook ignores it too), and nothing here may panic — a panicking
/// panic hook aborts the process and takes the message with it.
pub fn print_exit_leftover(session_id: Option<&str>, title: Option<&str>) {
    use std::io::Write;
    let Some(session_id) = session_id else {
        // No id means no turn ever ran (the agent mints the id on the first
        // turn), so there is no session to continue and nothing to print.
        return;
    };
    let block = format_continue_hint(FORCED_EXIT_STATE, session_id, title);
    let mut stdout = std::io::stdout().lock();
    // A closed pipe is not a reason to abort the unwind: the operator is
    // already looking at a panic, and losing the leftover beats replacing
    // their terminal with a broken-pipe abort.
    let _ = writeln!(stdout);
    let _ = writeln!(stdout, "{block}");
    let _ = stdout.flush();
}

/// The headline for a leftover the operator asked for while the TUI is open.
///
/// `/continue` is the third surface, and its sentence must not borrow either
/// of the other two: nothing was closed (this is the open TUI) and nothing was
/// interrupted. It names WHAT the session is, and the command below it is the
/// same one every other surface prints.
pub const CURRENT_SESSION_STATE: &str = "Current session";

/// What the exit was, for the two places that have to agree on it.
///
/// `deliberate` is the exit's own flag, read from `App::exit_is_deliberate`.
/// It decides which sentence the leftover carries — and the DECIDING belongs
/// here rather than at the print site, because the print site is not the only
/// reader: the boot surface re-prints the same leftover, and a boot that
/// guessed "interrupted" while the teardown said "saved" would show the
/// operator two contradictory sentences about one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// A deliberate close (`/exit`, `/quit`): the session was saved.
    Deliberate,
    /// A Ctrl+C or a killed terminal: the session was cut off mid-flight.
    Interrupted,
}

impl ExitKind {
    /// Classify a teardown's exit from the flag the `/exit` arm sets.
    pub fn from_deliberate(deliberate: bool) -> Self {
        if deliberate {
            Self::Deliberate
        } else {
            Self::Interrupted
        }
    }

    /// The sentence the leftover opens with.
    pub fn state(self) -> &'static str {
        match self {
            Self::Deliberate => "Session saved",
            Self::Interrupted => "Session was interrupted",
        }
    }
}

/// The one exit leftover, for BOTH surfaces that print it.
///
/// The teardown prints it into the terminal's scrollback; the next launch
/// re-prints the same block as a transcript annotation for the session the
/// last one was in. They must be ONE string: two copies of the sentence are
/// two things to keep in step, and an operator who closes the TUI and
/// restarts it is exactly the operator who would see both.
pub fn exit_leftover(kind: ExitKind, session_id: &str, title: Option<&str>) -> String {
    format_continue_hint(kind.state(), session_id, title)
}

/// The short title for a session's first prompt — the SAME rule the store's
/// auto-title uses (`agent/turn_context.rs::interrupted_title`, 60 chars,
/// ellipsised at a character boundary).
///
/// Duplicated rather than shared because `turn_context` is `pub(crate)` to
/// `operant-core`: if the rule changes there, change it here, and the two
/// labels stay the same sentence.
pub fn title_from_first_prompt(prompt: &str) -> String {
    const MAX: usize = 60;
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(MAX).collect();
    format!("{}…", head.trim_end())
}

/// Resolve the session `operant continue` should attach to.
///
/// Three answers, in order of how well they know the operator's intent:
///
/// 1. the id the operator typed — authoritative, and an unknown id is an error
///    rather than a guess;
/// 2. the interrupted-session marker — the session the last TUI was working on
///    when it went away, which is what "continue" means with no argument;
/// 3. the most recently ACTIVE session — the fallback that keeps the command
///    usable when the marker was already consumed by another surface. "Active"
///    means the newest MESSAGE, not the newest start: a crashed session never
///    ended, so ranking by started/ended would bury it under sessions the
///    operator finished hours ago.
///
/// The marker is passed IN rather than read here: reading it inside would make
/// this function depend on a file in the operator's home directory, and a
/// function whose answer depends on ambient state cannot be tested.
fn resolve_target_session(
    db: &Database,
    id: Option<&str>,
    marker: Option<&SessionInterrupted>,
) -> Result<TargetSession> {
    if let Some(id) = id {
        let sessions = db
            .list_sessions(10_000)
            .context("Failed to list sessions")?;
        let found = sessions
            .into_iter()
            .find(|s| s.id == id)
            .context(format!("Session not found: {id}"))?;
        return Ok(TargetSession {
            id: found.id,
            title: found.title,
        });
    }
    if let Some(marker) = marker {
        return Ok(TargetSession {
            id: marker.session_id.clone(),
            title: Some(marker.title.clone()).filter(|t| !t.trim().is_empty()),
        });
    }
    // By the last MESSAGE, not by started/ended: a session that never ended
    // (a crash, a SIGKILL, a closed terminal) must answer "what was I working
    // on" — `list_sessions` would rank it by when it started and answer with a
    // session the operator already finished. See the DB method's note.
    let most_recent = db
        .most_recent_session_by_activity()
        .context("Failed to read the most recent session")?
        .context("No sessions found — start one with `operant chat`.")?;
    Ok(TargetSession {
        id: most_recent.id,
        title: most_recent.title,
    })
}

/// `operant continue [SESSION_ID]` — attach the TUI to a previous session.
///
/// With no id this continues the session the last TUI was working on (the
/// interrupted-session marker), falling back to the most recent session.
pub async fn handle_continue_command(
    config: &AppConfig,
    id: Option<String>,
    no_mouse: bool,
    dangerously_skip_permissions: bool,
) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let marker = operant_core::session_interrupted::read_marker();
    let target = resolve_target_session(&db, id.as_deref(), marker.as_ref())?;

    // Two processes writing one session's rows is the race the presence
    // registry exists to prevent, so refuse rather than fork. The registry is
    // only truthful when the live process claims its session — which the TUI
    // now does as soon as the agent has an id (see the TUI's presence sync).
    if operant_core::active_sessions::session_is_live(&target.id) {
        anyhow::bail!(
            "Session {} is still running in another process — close it before continuing it here.",
            target.id
        );
    }

    if config.tui.rich_output && crate::tui_available() {
        crate::tui::TuiApp::enter(
            config.clone(),
            None,
            crate::tui::LaunchMode::Resume(target.id),
            no_mouse,
            dangerously_skip_permissions,
        )
        .await?
        .run()
        .await?;
    } else {
        // No TTY: the TUI cannot enter raw mode here, so print what the
        // operator needs instead of failing with a crossterm error.
        crate::warn_tui_fallback(config.tui.rich_output);
        // "Session", not "Session saved": nothing was closed here — the
        // session is at rest in the store, and the command below is what
        // continues it from a terminal that has one.
        println!(
            "{}",
            format_continue_hint("Session", &target.id, target.title.as_deref())
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "operant_continue_cli_{}_{}.db",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_file(&path);
        Database::init(path).expect("Failed to create test database")
    }

    /// The leftover is a COPY-PASTE surface: the operator is expected to run
    /// the exact line it contains. So the test asserts the line is the real
    /// command, not merely that the id appears somewhere in the text.
    #[test]
    fn the_leftover_prints_the_exact_continue_command() {
        let leftover = exit_leftover(
            ExitKind::Deliberate,
            "sess_abc123",
            Some("Fix the importer"),
        );
        assert!(
            leftover.contains("operant continue sess_abc123"),
            "the leftover must name the command verbatim: {leftover:?}"
        );
        assert!(
            leftover.contains("Fix the importer"),
            "the title is what the operator recognises: {leftover:?}"
        );
    }

    /// A session with no title still has to produce a runnable command — the
    /// id alone is enough to continue it.
    #[test]
    fn an_untitled_session_still_prints_the_command() {
        let leftover = exit_leftover(ExitKind::Deliberate, "sess_abc123", None);
        assert!(
            leftover.contains("operant continue sess_abc123"),
            "the leftover must name the command verbatim: {leftover:?}"
        );
        let blank = exit_leftover(ExitKind::Deliberate, "sess_abc123", Some("   "));
        assert!(
            blank.contains("operant continue sess_abc123"),
            "a blank title must not swallow the command: {blank:?}"
        );
    }

    /// The exit headline must tell the operator WHICH kind of exit it was: a
    /// deliberate close saved the session, a Ctrl+C interrupted it. The
    /// session row says `interrupted` too — two records that disagree about
    /// the same exit are worse than either alone.
    #[test]
    fn the_exit_headline_distinguishes_a_deliberate_close_from_an_interruption() {
        assert_eq!(ExitKind::from_deliberate(true).state(), "Session saved");
        assert_eq!(
            ExitKind::from_deliberate(false).state(),
            "Session was interrupted"
        );
        // The panic path's headline is its own sentence: it cannot know
        // whether the session was cut off, so it must not claim either of the
        // two the teardown can prove.
        assert_ne!(FORCED_EXIT_STATE, ExitKind::Deliberate.state());
        assert_ne!(FORCED_EXIT_STATE, ExitKind::Interrupted.state());
        let interrupted = exit_leftover(
            ExitKind::Interrupted,
            "sess_abc123",
            Some("Fix the importer"),
        );
        assert!(
            interrupted.starts_with("Session was interrupted"),
            "an interrupted exit must not read as a clean save: {interrupted:?}"
        );
    }

    /// An id the operator typed is authoritative: a typo must say so rather
    /// than silently continuing a DIFFERENT session.
    #[test]
    fn an_unknown_id_is_an_error_not_a_fallback() {
        let db = test_db();
        db.save_session(
            "sess_real",
            Some("real"),
            "test",
            "2026-01-01",
            "2026-01-01",
        )
        .expect("save_session");
        let err = resolve_target_session(&db, Some("sess_typo"), None)
            .expect_err("a typo must not resolve");
        assert!(
            err.to_string().contains("Session not found"),
            "the error must name the problem: {err}"
        );
    }

    /// With no id the marker wins over recency: the marker names the session
    /// the operator was actually working on, which is the whole point of
    /// "continue".
    #[test]
    fn the_marker_beats_the_most_recent_session() {
        let db = test_db();
        db.save_session("sess_old", Some("old"), "test", "2026-01-01", "2026-01-01")
            .expect("save_session");
        db.save_session("sess_new", Some("new"), "test", "2026-01-02", "2026-01-02")
            .expect("save_session");
        let marker = SessionInterrupted::new("sess_old", "interrupted work");
        let target = resolve_target_session(&db, None, Some(&marker)).expect("resolve");
        assert_eq!(target.id, "sess_old");
        assert_eq!(target.title.as_deref(), Some("interrupted work"));
    }

    /// With neither an id nor a marker the most recently ACTIVE session is the
    /// answer — and "active" is the newest MESSAGE, not the newest start.
    ///
    /// The shape is the case this ordering exists for: `sess_crashed` STARTED
    /// first and never ended (a crash, a SIGKILL, a closed terminal), while
    /// `sess_finished` started later and ended normally. Ranking by
    /// started/ended buries the crashed session under one the operator already
    /// finished, so "continue with no id" answers the wrong session — exactly
    /// when the operator most needs it.
    #[test]
    fn without_an_id_or_a_marker_the_most_recently_active_session_is_used() {
        let db = test_db();
        // Started days earlier, never ended, and its last turn is the NEWEST
        // thing in the store.
        db.save_session(
            "sess_crashed",
            Some("crashed"),
            "test",
            "2026-01-01",
            "2026-01-01",
        )
        .expect("save_session");
        db.save_message(
            "sess_crashed",
            "user",
            "the last thing I asked",
            "2026-01-09T10:00:00Z",
        )
        .expect("save_message");
        // Started later and finished normally, but its last turn is OLDER.
        db.save_session(
            "sess_finished",
            Some("finished"),
            "test",
            "2026-01-05",
            "2026-01-05",
        )
        .expect("save_session");
        db.save_message(
            "sess_finished",
            "user",
            "an older turn",
            "2026-01-06T09:00:00Z",
        )
        .expect("save_message");

        let target = resolve_target_session(&db, None, None).expect("resolve");
        assert_eq!(target.id, "sess_crashed");
        assert_eq!(target.title.as_deref(), Some("crashed"));
    }

    /// The title rule the store's auto-title uses: the first 60 characters,
    /// ellipsised. Two labels that must read as one sentence cannot drift.
    #[test]
    fn the_title_rule_matches_the_stores_auto_title() {
        assert_eq!(title_from_first_prompt("  padded  "), "padded");
        let long = "x".repeat(400);
        let cut = title_from_first_prompt(&long);
        assert!(cut.chars().count() <= 61, "{cut}");
        assert!(cut.ends_with('…'));
    }

    /// An empty database has no session to continue, and the error must say
    /// what to do instead.
    #[test]
    fn an_empty_database_says_how_to_start_one() {
        let db = test_db();
        let err = resolve_target_session(&db, None, None).expect_err("nothing to continue");
        assert!(
            err.to_string().contains("operant chat"),
            "the error must name the way out: {err}"
        );
    }

    /// The leftover is not only the teardown's to print: a PANIC unwinds past
    /// `run_exit_teardown` entirely, so the panic hook is the only thing left
    /// that can offer the session back. The hook is installed in another module
    /// with no test seam of its own (the process-global hook cannot be
    /// exercised here), so this pins the WIRING — the hook must actually call
    /// the printer — the same source-contract way `tui::mod`'s seam tests do.
    #[test]
    fn the_panic_hook_prints_the_leftover_for_a_session_it_cannot_tear_down() {
        let source = include_str!("tui/adapter_types/tui_app.rs");
        let hook = source
            .split_once("std::panic::set_hook(")
            .expect("the panic hook must be installed in tui_app.rs");
        assert!(
            hook.1.contains("print_exit_leftover"),
            "a panic must leave the same copy-pasteable way back the teardown prints, or the session streaming when the panic hit is the one session with no way back"
        );
        // And the print must be AFTER the alternate screen is popped: before it,
        // the terminal erases the leftover with the frame it just restored.
        let hook_body = hook.1.split("prev_hook(info);").next().unwrap_or("");
        let pop = hook_body
            .find("LeaveAlternateScreen")
            .expect("the hook must pop the alternate screen");
        let print = hook_body
            .find("print_exit_leftover")
            .expect("the hook must print the leftover");
        assert!(
            pop < print,
            "the leftover must be printed after LeaveAlternateScreen, or the terminal erases it with the frame"
        );
    }
}

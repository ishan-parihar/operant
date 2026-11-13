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

/// The headline the exit leftover carries.
///
/// A deliberate close (`/exit`) SAVED the session; a Ctrl+C INTERRUPTED it —
/// the session was cut off mid-flight. The leftover has to say which: the
/// session row says `interrupted`, and two records that disagree about the
/// same exit are worse than either alone. An operator deciding whether to
/// continue a half-finished turn reads the sentence, not the row.
pub fn exit_state(deliberate: bool) -> &'static str {
    if deliberate {
        "Session saved"
    } else {
        "Session was interrupted"
    }
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
        let sessions = db.list_sessions(10_000).context("Failed to list sessions")?;
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
        let leftover = format_continue_hint(
            exit_state(true),
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
        let leftover = format_continue_hint(exit_state(true), "sess_abc123", None);
        assert!(
            leftover.contains("operant continue sess_abc123"),
            "the leftover must name the command verbatim: {leftover:?}"
        );
        let blank = format_continue_hint(exit_state(true), "sess_abc123", Some("   "));
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
        assert_eq!(exit_state(true), "Session saved");
        assert_eq!(exit_state(false), "Session was interrupted");
        let interrupted = format_continue_hint(
            exit_state(false),
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
        db.save_session("sess_real", Some("real"), "test", "2026-01-01", "2026-01-01")
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
        db.save_session("sess_crashed", Some("crashed"), "test", "2026-01-01", "2026-01-01")
            .expect("save_session");
        db.save_message("sess_crashed", "user", "the last thing I asked", "2026-01-09T10:00:00Z")
            .expect("save_message");
        // Started later and finished normally, but its last turn is OLDER.
        db.save_session("sess_finished", Some("finished"), "test", "2026-01-05", "2026-01-05")
            .expect("save_session");
        db.save_message("sess_finished", "user", "an older turn", "2026-01-06T09:00:00Z")
            .expect("save_message");

        let target = resolve_target_session(&db, None, None).expect("resolve");
        assert_eq!(target.id, "sess_crashed");
        assert_eq!(target.title.as_deref(), Some("crashed"));
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
}

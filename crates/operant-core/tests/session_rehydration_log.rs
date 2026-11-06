//! The session-rehydration warning says what it dropped — and only when it dropped.
//!
//! The resumed-session load is bounded (a message-count backstop and a byte
//! budget), so a long transcript is loaded from its newest end. When that
//! happens the store logs a warning, because a transcript that starts
//! mid-session is a fact the operator needs.
//!
//! The count it logged was the KEPT count, not the dropped one: `dropped` was
//! computed as `total - start`, and `start` is already `total - kept`, so the
//! two cancelled and the warning fired on EVERY load — claiming a mid-session
//! start for sessions that lost nothing (measured on the live database:
//! "rows_total: 643, rows_kept: 643, rows_dropped: 643").

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};

use operant_core::database::Database;
use operant_core::session::{SessionKey, SessionStore, SessionStoreConfig};

// ── Tracing capture (the pattern from tests/tool_failure_logging.rs) ──

#[derive(Clone)]
struct LogSink(Arc<Mutex<Vec<String>>>);

struct EventLine {
    sink: Arc<Mutex<Vec<String>>>,
    buf: Vec<u8>,
}

impl std::io::Write for EventLine {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        if !text.trim().is_empty() {
            self.sink
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(text);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
    type Writer = EventLine;

    fn make_writer(&'a self) -> Self::Writer {
        EventLine {
            sink: Arc::clone(&self.0),
            buf: Vec::new(),
        }
    }
}

fn install_log_capture() -> Arc<Mutex<Vec<String>>> {
    static SINK: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(LogSink(Arc::clone(&sink)))
                .with_ansi(false)
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        Arc::clone(&sink)
    })
    .clone()
}

/// Seed `count` user rows into a fresh session and return its store.
fn seeded_store(
    db: &Arc<Database>,
    session_id: &str,
    count: usize,
    rehydrate_tail: usize,
) -> SessionStore {
    db.save_session(session_id, None, "test", "t0", "t0")
        .expect("save session");
    for i in 0..count {
        db.save_message(
            session_id,
            "user",
            &format!("message {i}"),
            &format!("t{i:04}"),
        )
        .expect("save message");
    }
    SessionStore::with_config(
        Arc::clone(db),
        SessionStoreConfig {
            warm_capacity: 4,
            rehydrate_tail,
            rehydrate_bytes: 1024 * 1024,
        },
    )
}

fn lines_for(capture: &Arc<Mutex<Vec<String>>>, session_id: &str) -> Vec<String> {
    let lines = capture.lock().unwrap_or_else(|e| e.into_inner()).clone();
    lines
        .into_iter()
        .filter(|line| line.contains(session_id))
        .collect()
}

#[test]
fn a_truncated_rehydration_logs_the_rows_it_actually_dropped() {
    let capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("rehydrate.sqlite")).unwrap());
    let session_id = "rehydrate-truncated";
    let store = seeded_store(&db, session_id, 20, 5);

    let loaded = store.acquire(&SessionKey::new(session_id));
    assert_eq!(loaded, 5, "the count bound must have kept exactly 5 rows");

    let lines = lines_for(&capture, session_id);
    let warning = lines
        .iter()
        .find(|line| line.contains("session rehydration dropped rows"))
        .unwrap_or_else(|| panic!("the truncated load must be logged; lines: {lines:#?}"));
    assert!(
        warning.contains("rows_total=20"),
        "the log must name the whole transcript: {warning}"
    );
    assert!(
        warning.contains("rows_kept=5"),
        "the log must name the kept window: {warning}"
    );
    assert!(
        warning.contains("rows_dropped=15"),
        "the log must name the rows it ACTUALLY dropped (20 - 5), not the rows \
         it kept: {warning}"
    );
}

#[test]
fn a_session_that_fits_is_not_reported_as_truncated() {
    let capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("rehydrate-full.sqlite")).unwrap());
    let session_id = "rehydrate-complete";
    let store = seeded_store(&db, session_id, 6, 50);

    let loaded = store.acquire(&SessionKey::new(session_id));
    assert_eq!(loaded, 6, "the whole transcript must be resident");

    let lines = lines_for(&capture, session_id);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("session rehydration dropped rows")),
        "a session that lost nothing must not be reported as truncated; lines: {lines:#?}"
    );
}

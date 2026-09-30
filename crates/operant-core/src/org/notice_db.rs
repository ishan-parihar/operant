//! Notice board — SQLite storage (`docs/WAVE1-DECISIONS.md` §3.3).
//!
//! ## Where this table lives, and why it is not a new file
//!
//! The board's only table is `notices`, created in the **existing**
//! `operant_kanban.db`. §3.3 says so explicitly, and §1.2 restates it as the
//! "sibling DB next to the domain DB" convention (`database.db` for sessions,
//! `operant_cron.db` for cron, `operant_kanban.db` for kanban + notice board +
//! worklog + employee registry). §1.2's stated reason is the repo's own scar:
//! a second location for one concept is what `BUGS.md` R5-1 (the `memory/`
//! split-brain) already paid for once.
//!
//! ## How the schema is applied without a migration conflict
//!
//! The schema comes from [`NoticeBoard::NOTICES_SCHEMA`], executed with
//! `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS` — the same
//! declarative reconciliation shape `database.rs` documents, and the same
//! idempotence `migrations.rs` asks each entry to have.
//!
//! It deliberately does **not** go through `crate::migrations::migrate`.
//! `PRAGMA user_version` is file-wide: `operant_kanban.db` sits at version 1
//! (kanban's single `MIGRATIONS` entry), and appending a notice entry here
//! would move the board's version, which would then make kanban's one-entry
//! family *look* downgraded the next time kanban opens the same file and
//! hard-fail with "refusing to downgrade". That guard is `migrations.rs`'s
//! own documented INVARIANT, added after R39-7. The board therefore owns no
//! version counter and bumps no PRAGMA. The cost is that the board has no
//! versioned upgrade path; at one additive table in a fresh column space that
//! is the cheaper trade.
//!
//! ## GC is batched, and the cap is the contract
//!
//! The organism's GC needed `max_weekly_summaries_per_tick=4`
//! (`axe_lib.py:753-758`) because summarizing every elapsed week in one pass
//! is unbounded work. Every step here is capped by
//! [`RetentionLimits`](crate::org::notice::RetentionLimits), selects at most
//! `cap + 1` candidate ids so "did I hit the cap" is one extra row rather
//! than a `COUNT(*)`, and reports
//! [`GcReport::more_work_pending`](crate::org::notice::GcReport) so the
//! caller reschedules instead of looping.
//!
//! ## Where this file diverges from §3.3, and why
//!
//! - `from_dept` is stored `NULL`-or-value and read back through
//!   `COALESCE(from_dept, 'unknown')`, per §3.3 and AD-013 §2.
//! - §3.3 step 3 prunes summaries with `tags = '["weekly-summary"]'`. This
//!   filters with `tags LIKE '%"weekly-summary"%'` instead of an exact string
//!   compare: the organism's own summary rows carry a second tag
//!   (`dept:task-grid`), so the literal in the spec would never match a
//!   summary row with more than one tag. The predicate still means exactly
//!   "carries the summary tag", which is the intent.
//! - The organism's per-tag retention overrides (`axe_lib.py:109-115` —
//!   `status` 7d, `request`/`ack`/`result` 30d, `fix-pattern` 24h) are **not**
//!   implemented. §3.3's SQL has no such table and §3.3 is binding. This is a
//!   real behavior gap, not an oversight: a `status` notice survives 14 days
//!   here where the organism keeps it 7.
//! - `metadata` is an added column. §3.3's GC step 2 requires a metadata
//!   block on summary rows (`week_start` / `week_end` / `entry_count` /
//!   distributions / `outcomes`) but §3.3's `CREATE TABLE` has nowhere to put
//!   it — the organism carries it as a JSONL field. Column, not sidecar, per
//!   the no-second-store rule.

use crate::error::Error;
use crate::org::notice::{
    ACK_TAG, BROADCAST_SELECTOR, GcReport, InboxQuery, Notice, PostNotice, RAW_RETENTION_DAYS,
    Recipient, RetentionLimits, SUMMARY_RETENTION_WEEKS, WEEK_SECONDS, WEEKLY_SUMMARY_TAG, rfc3339,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Columns selected by every read, in [`Notice`] field order (modulo
/// `from_dept`, which is `COALESCE`d).
const NOTICE_COLUMNS: &str = "id, created_at, epoch, sender, \
     COALESCE(from_dept, 'unknown') AS from_dept, recipients, subject, body, \
     correlation_id, ack_required, acked_by, acked_at, ttl_expires_at, tags, \
     thread_id, pinned, reason, metadata";

/// Ordinals of the columns in [`NOTICE_COLUMNS`].
///
/// `row_to_notice` used to hard-code `row.get(14)` for `tags` when `tags` is
/// column 13, which shifted every field after it and made every read of a row
/// with a NULL `thread_id` fail. Named constants keep the list and the mapper
/// reading the same sequence, so a future column insert is one edit here plus
/// one in the `SELECT`, not a silent one-character drift in the mapper.
mod col {
    pub(super) const ID: usize = 0;
    pub(super) const CREATED_AT: usize = 1;
    pub(super) const EPOCH: usize = 2;
    pub(super) const SENDER: usize = 3;
    pub(super) const FROM_DEPT: usize = 4;
    pub(super) const RECIPIENTS: usize = 5;
    pub(super) const SUBJECT: usize = 6;
    pub(super) const BODY: usize = 7;
    pub(super) const CORRELATION_ID: usize = 8;
    pub(super) const ACK_REQUIRED: usize = 9;
    pub(super) const ACKED_BY: usize = 10;
    pub(super) const ACKED_AT: usize = 11;
    pub(super) const TTL_EXPIRES_AT: usize = 12;
    pub(super) const TAGS: usize = 13;
    pub(super) const THREAD_ID: usize = 14;
    pub(super) const PINNED: usize = 15;
    pub(super) const REASON: usize = 16;
    pub(super) const METADATA: usize = 17;
}

/// Sender identity GC stamps on a weekly summary. The organism uses
/// `axe-summary-system`; operant has no `axe`, so this is the operant spelling
/// of the same role.
const SUMMARY_SENDER: &str = "operant-summary-system";

/// The notice board store.
pub struct NoticeBoard {
    conn: Arc<Mutex<Connection>>,
}

impl NoticeBoard {
    /// Open the notice board in the org database, deriving the path from the
    /// main app database exactly like packet A's
    /// [`org_db_path`](crate::org::employee_db::org_db_path) does.
    ///
    /// This is the production entry point, and it is deliberately *the only*
    /// one that names a file: the path is not a parameter, so a caller
    /// cannot invent a second store by passing a different one. The
    /// `operant_kanban.db` constant in [`crate::org::notice`] documents the
    /// name that [`org_db_path`](crate::org::employee_db::org_db_path)
    /// produces; the derivation is delegated rather than duplicated so the
    /// two subsystems cannot drift into opening different files.
    pub fn for_app(database_path: &Path) -> Result<Self, Error> {
        Self::init(crate::org::employee_db::org_db_path(database_path))
    }

    /// Open (or create) the notice board in an existing sqlite file.
    ///
    /// `path` is expected to be the sibling kanban DB
    /// ([`NOTICE_BOARD_DB_FILE`](crate::org::notice::NOTICE_BOARD_DB_FILE)),
    /// but nothing here enforces that — the board is a table, not a file, and
    /// tests legitimately point it at a temp path.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("notice board: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("notice board: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach the board to a connection somebody else already owns.
    ///
    /// This is the path the rest of operant should use. The board is a table
    /// inside a database another subsystem owns; opening a second
    /// `Connection` to the same file just to create one table would trade a
    /// separate-store problem for a separate-lock problem.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let board = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        board.ensure_schema()?;
        Ok(board)
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` (the shape
    /// `KanbanDb::conn` returns) so the board shares the kanban handle
    /// instead of opening a second writer on the same file.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let board = Self { conn };
        board.ensure_schema()?;
        Ok(board)
    }

    /// The underlying connection, for callers that need to share one
    /// connection with kanban or the employee registry.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, converting mutex poisoning into a recoverable
    /// error instead of panicking (same pattern as `kanban/db.rs` and
    /// `cronjobs/db.rs`).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("notice board db mutex poisoned".to_string()))
    }

    /// §3.3's schema, plus the `metadata` column (see module docs).
    ///
    /// Applied declaratively and idempotently — see the module docs for why
    /// this is not a `crate::migrations::migrate` entry.
    pub const NOTICES_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS notices (
            id              TEXT PRIMARY KEY,      -- 'n_' || uuid v4 (organism: AXE-AD-013)
            created_at      TEXT NOT NULL,         -- RFC3339; indexed for GC + inbox queries
            epoch           INTEGER NOT NULL,      -- unix seconds; the inbox-bridge query key
            sender          TEXT NOT NULL,         -- employee_id, or 'user', or 'system'
            from_dept       TEXT,                  -- sender provenance; NULL => 'unknown' (AD-013 §2)
            recipients      TEXT NOT NULL,         -- JSON array of typed selectors
            subject         TEXT,
            body            TEXT NOT NULL,
            correlation_id  TEXT,                  -- uuid; request -> ack -> result chain
            ack_required    INTEGER NOT NULL DEFAULT 0,
            acked_by        TEXT,                  -- JSON array of employee_ids that acked
            acked_at        TEXT,
            ttl_expires_at  TEXT,                  -- RFC3339; NULL => never expires
            tags            TEXT NOT NULL DEFAULT '[]',  -- JSON array
            thread_id       TEXT,                  -- explicit thread anchor
            pinned          INTEGER NOT NULL DEFAULT 0, -- retention pin (axe retention-pins.jsonl)
            reason          TEXT NOT NULL,         -- the --reason of the posting write
            metadata        TEXT                   -- JSON object; GC weekly-summary block
        );

        CREATE INDEX IF NOT EXISTS idx_notices_created    ON notices(created_at);
        CREATE INDEX IF NOT EXISTS idx_notices_correlation ON notices(correlation_id);
        CREATE INDEX IF NOT EXISTS idx_notices_thread      ON notices(thread_id);
        CREATE INDEX IF NOT EXISTS idx_notices_epoch       ON notices(epoch);
    "#;

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::NOTICES_SCHEMA)
            .map_err(|e| Error::Agent(format!("notice board: schema: {e}")))
    }

    /// Total rows on the board. Diagnostics and test affordance.
    pub fn count(&self) -> Result<i64, Error> {
        let conn = self.lock_conn()?;
        conn.query_row("SELECT COUNT(*) FROM notices", [], |r| r.get(0))
            .map_err(|e| Error::Agent(format!("notice board: count: {e}")))
    }

    // ----------------------------------------------------------------- post

    /// Post a notice. Returns the stored row.
    ///
    /// An empty recipient list normalises to broadcast, matching the organism
    /// (`axe_lib.py:191-217`: `None`/empty → broadcast).
    pub fn post(&self, post: &PostNotice) -> Result<Notice, Error> {
        let now = chrono::Utc::now();
        let recipients = if post.recipients.is_empty() {
            vec![Recipient::Broadcast]
        } else {
            post.recipients.clone()
        };
        let ttl_expires_at = match (post.ttl_expires_at, post.ttl) {
            // An explicit instant wins, and is passed through verbatim. This
            // is how a replay, a migration backfill, or a test produces a row
            // whose TTL is already past.
            (Some(instant), _) => Some(rfc3339(instant)),
            // `Duration` is bounded well inside chrono's range, so a failure
            // here would mean a caller built an absurd TTL; fall back to "no
            // expiry" rather than panicking in a message write.
            (None, Some(ttl)) => match chrono::Duration::from_std(ttl) {
                Ok(delta) => Some(rfc3339(now + delta)),
                Err(_) => None,
            },
            (None, None) => None,
        };

        let notice = Notice {
            id: format!("n_{}", uuid::Uuid::new_v4()),
            created_at: rfc3339(now),
            epoch: now.timestamp(),
            sender: post.sender.clone(),
            from_dept: post.from_dept.clone(),
            recipients,
            subject: post.subject.clone(),
            body: post.body.clone(),
            correlation_id: post.correlation_id.clone(),
            ack_required: post.ack_required,
            acked_by: Vec::new(),
            acked_at: None,
            ttl_expires_at,
            tags: post.tags.clone(),
            thread_id: post.thread_id.clone(),
            pinned: post.pinned,
            reason: post.reason.clone(),
            metadata: post.metadata.clone(),
        };
        let guard = self.lock_conn()?;
        insert_into(&guard, &notice)?;
        Ok(notice)
    }

    // ---------------------------------------------------------------- inbox

    /// Read a reader's inbox.
    ///
    /// Resolves typed recipients **at read time**: a notice reaches the
    /// reader when any of its stored selectors is one of the reader's own
    /// (`agent:` / `dept:` / `team:` / `role:`), or when it is a broadcast.
    /// A notice posted to `dept:infra` before the reader joined still reaches
    /// them — that is the whole point of storing selectors verbatim.
    ///
    /// Expired notices (`ttl_expires_at` already past) are filtered out.
    /// `pending_only` narrows further to `ack_required` notices the reader
    /// has not acked.
    pub fn inbox(
        &self,
        reader: &dyn crate::org::notice::NoticeInboxMatcher,
        query: &InboxQuery,
    ) -> Result<Vec<Notice>, Error> {
        let selectors: Vec<String> = reader.selectors().iter().map(Recipient::selector).collect();
        let mut q = query.clone();
        if q.selectors.is_none() {
            q.selectors = Some(selectors);
        }
        self.query_inbox(&q)
    }

    /// Inbox query with the reader's selectors supplied directly.
    ///
    /// This is the primitive; [`NoticeBoard::inbox`] is the ergonomic wrapper.
    /// Passing `selectors: None` reads every notice on the board regardless
    /// of recipient — the "what has happened org-wide" view.
    pub fn query_inbox(&self, query: &InboxQuery) -> Result<Vec<Notice>, Error> {
        let conn = self.lock_conn()?;
        let now = rfc3339(chrono::Utc::now());
        let mut sql = format!("SELECT {NOTICE_COLUMNS} FROM notices WHERE 1 = 1");
        let mut binds: Vec<String> = Vec::new();

        if let Some(after) = query.after_epoch {
            sql.push_str(" AND epoch >= ?");
            binds.push(after.to_string());
        }

        // Recipient matching is a json_each scan over the stored array: the
        // selector set is tiny (an employee is in one dept, a few teams, a few
        // roles) and the epoch index still drives the outer scan.
        if let Some(selectors) = &query.selectors {
            let mut any = String::from(
                "EXISTS (SELECT 1 FROM json_each(notices.recipients) je WHERE je.value = 'broadcast')",
            );
            for (i, sel) in selectors.iter().enumerate() {
                any.push_str(&format!(
                    " OR EXISTS (SELECT 1 FROM json_each(notices.recipients) je WHERE je.value = ?{})",
                    i + 1
                ));
                binds.push(sel.clone());
            }
            sql.push_str(&format!(" AND ({any})"));
        }

        if query.pending_only {
            sql.push_str(" AND ack_required = 1");
            if let Some(reader_id) = &query.reader_id {
                // "Not yet acked by me" — an array-overlap check.
                sql.push_str(
                    " AND NOT EXISTS (SELECT 1 FROM json_each(notices.acked_by) a WHERE a.value = ?)",
                );
                binds.push(reader_id.clone());
            }
        }

        sql.push_str(" ORDER BY epoch ASC, id ASC");
        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| Error::Agent(format!("notice board: inbox prepare: {e}")))?;
        let refs: Vec<&dyn rusqlite::ToSql> =
            binds.iter().map(|b| b as &dyn rusqlite::ToSql).collect();
        let rows = stmt
            .query_map(refs.as_slice(), row_to_notice)
            .map_err(|e| Error::Agent(format!("notice board: inbox query: {e}")))?;

        // TTL is applied after the read rather than in SQL so the comparison
        // goes through the same fixed-width RFC3339 writer, instead of
        // relying on SQLite text comparison over a mixed-precision column.
        let mut out = Vec::new();
        for row in rows {
            let notice = row.map_err(|e| Error::Agent(format!("notice board: inbox row: {e}")))?;
            if !notice.is_expired_at(&now) {
                out.push(notice);
            }
        }
        Ok(out)
    }

    // ------------------------------------------------------------------ ack

    /// Acknowledge a notice on behalf of `employee_id`.
    ///
    /// Per §3.3 an ack is **two writes in one transaction**: a notice post
    /// carrying the parent's `correlation_id` and tag `ack` (the organism's
    /// request → ack → result protocol, AD-013 §3-4), plus a direct `UPDATE`
    /// on the parent row so inbox queries do not have to join. A repeated
    /// ack by the same employee is idempotent — no second ack notice, no
    /// duplicate `acked_by` entry.
    pub fn ack(&self, notice_id: &str, employee_id: &str) -> Result<Notice, Error> {
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction()
            .map_err(|e| Error::Agent(format!("notice board: ack begin: {e}")))?;

        let parent = tx
            .query_row(
                &format!("SELECT {NOTICE_COLUMNS} FROM notices WHERE id = ?1"),
                params![notice_id],
                row_to_notice,
            )
            .optional()
            .map_err(|e| Error::Agent(format!("notice board: ack lookup: {e}")))?
            .ok_or_else(|| Error::Agent(format!("notice board: no such notice: {notice_id}")))?;

        if parent.is_acked_by(employee_id) {
            // Idempotent re-ack: no second ack notice. Nothing was written, so
            // dropping the transaction (which rolls back) is correct.
            return Ok(parent);
        }

        let mut acked_by = parent.acked_by.clone();
        acked_by.push(employee_id.to_string());
        let now = chrono::Utc::now();
        let now_str = rfc3339(now);

        tx.execute(
            "UPDATE notices SET acked_by = ?2, acked_at = ?3 WHERE id = ?1",
            params![notice_id, encode_strings(&acked_by)?, now_str],
        )
        .map_err(|e| Error::Agent(format!("notice board: ack update: {e}")))?;

        let ack_notice = Notice {
            id: format!("n_{}", uuid::Uuid::new_v4()),
            created_at: now_str.clone(),
            epoch: now.timestamp(),
            sender: employee_id.to_string(),
            from_dept: parent.from_dept.clone(),
            // Addressed back to whoever asked.
            recipients: vec![Recipient::Agent(parent.sender.clone())],
            subject: Some(format!(
                "ack: {}",
                parent
                    .subject
                    .clone()
                    .unwrap_or_else(|| notice_id.to_string())
            )),
            body: format!("{employee_id} acknowledged notice {notice_id}"),
            correlation_id: parent.correlation_id.clone(),
            ack_required: false,
            acked_by: Vec::new(),
            acked_at: None,
            ttl_expires_at: None,
            tags: vec![ACK_TAG.to_string()],
            thread_id: parent.thread_id.clone(),
            pinned: false,
            reason: format!("ack of notice {notice_id}"),
            metadata: Some(serde_json::json!({ "ack_of": notice_id })),
        };
        insert_in_tx(&tx, &ack_notice)?;

        tx.commit()
            .map_err(|e| Error::Agent(format!("notice board: ack commit: {e}")))?;
        Ok(Notice {
            acked_by,
            acked_at: Some(now_str),
            ..parent
        })
    }

    /// Notices in `reader`'s inbox that still need an ack from `reader_id`.
    pub fn pending_acks(
        &self,
        reader: &dyn crate::org::notice::NoticeInboxMatcher,
        reader_id: &str,
    ) -> Result<Vec<Notice>, Error> {
        self.inbox(
            reader,
            &InboxQuery {
                pending_only: true,
                reader_id: Some(reader_id.to_string()),
                ..Default::default()
            },
        )
    }

    // --------------------------------------------------------- thread / chain

    /// Read a thread: every notice anchored to `thread_id`, oldest first.
    ///
    /// TTL is deliberately **not** applied here. A thread read is an audit
    /// read — the same read that explains "what happened to this request" must
    /// still work after a TTL lapse, and the board is an audit trail
    /// (`axe_lib.py:173-174`). TTL governs delivery to an inbox, not history.
    pub fn thread(&self, thread_id: &str, limit: Option<i64>) -> Result<Vec<Notice>, Error> {
        let mut sql = format!(
            "SELECT {NOTICE_COLUMNS} FROM notices WHERE thread_id = ?1 ORDER BY created_at ASC, id ASC"
        );
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let guard = self.lock_conn()?;
        read_all(&guard, &sql, params![thread_id])
    }

    /// Read a whole request → ack → result chain by `correlation_id`, oldest
    /// first. This is the cross-thread view §3.3's `correlation_id` exists
    /// for: the ack written by [`NoticeBoard::ack`] and any result notice
    /// land in the same chain even when they carry different `thread_id`s.
    pub fn correlation_chain(&self, correlation_id: &str) -> Result<Vec<Notice>, Error> {
        let guard = self.lock_conn()?;
        read_all(
            &guard,
            &format!(
                "SELECT {NOTICE_COLUMNS} FROM notices WHERE correlation_id = ?1 \
                 ORDER BY created_at ASC, id ASC"
            ),
            params![correlation_id],
        )
    }

    /// Fetch one notice by id.
    pub fn get(&self, id: &str) -> Result<Option<Notice>, Error> {
        self.lock_conn()?
            .query_row(
                &format!("SELECT {NOTICE_COLUMNS} FROM notices WHERE id = ?1"),
                params![id],
                row_to_notice,
            )
            .optional()
            .map_err(|e| Error::Agent(format!("notice board: get {id}: {e}")))
    }

    // ------------------------------------------------------------------- GC

    /// Run one batched retention pass.
    ///
    /// Per §3.3, per tick:
    ///
    /// 1. summarize at most
    ///    [`crate::org::notice::DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK`]
    ///    completed weeks into synthetic `weekly-summary` rows;
    /// 2. prune `weekly-summary` rows older than
    ///    [`SUMMARY_RETENTION_WEEKS`];
    /// 3. delete raw rows older than [`RAW_RETENTION_DAYS`] that are not
    ///    pinned and are not part of a still-live correlation chain.
    ///
    /// **Step order is a deliberate fix, not a transcription.** §3.3 lists the
    /// steps as delete / summarize / prune. Run literally, the first delete
    /// removes the very rows step 2 summarizes, so `entry_count` is always 0
    /// and the board keeps its raw entries while silently manufacturing a
    /// row of empty summaries every tick — those summaries would then be
    /// pruned at 16 weeks having recorded nothing. Summarizing *first* is what
    /// AXE-AD-006 describes ("weekly summaries are generated for any
    /// completed week whose raw entries are all ≥14 days old" — generated
    /// from those entries, then the entries are pruned). The trade is that a
    /// tick which dies between step 1 and step 3 leaves raw rows alongside
    /// their summary; the next tick's `INSERT OR IGNORE` on the deterministic
    /// summary id absorbs that, so it converges.
    ///
    /// Each step is capped by `limits`; each selects at most `cap + 1`
    /// candidate ids so "did I hit the cap" costs one extra row rather than a
    /// `COUNT(*)`. A step that hit its cap sets
    /// [`GcReport::more_work_pending`], and the caller runs GC again next
    /// tick rather than inside this call.
    ///
    /// Idempotent: a pass with nothing to do returns a zeroed report, and the
    /// deterministic summary ids (`n_weekly_<week_start>`) plus
    /// `INSERT OR IGNORE` make a retried pass write no duplicates.
    ///
    /// Call site per §3.3 is step 0c of every tick — the top of
    /// `process_due_jobs`, not inside the per-job path.
    pub fn retention_gc(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limits: RetentionLimits,
    ) -> Result<GcReport, Error> {
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction()
            .map_err(|e| Error::Agent(format!("notice board: gc begin: {e}")))?;

        let raw_cutoff = rfc3339(now - chrono::Duration::days(RAW_RETENTION_DAYS));
        // The liveness bound is one day *older* than the raw cutoff: a sibling
        // counts as live while it is still inside the 14-day retention window,
        // i.e. while its `created_at` is `>= now - 14 days`. Anything strictly
        // older than that has no live sibling left and so is collectable.
        //
        // §3.3 reuses the raw cutoff for both roles, which is what makes a
        // chain stall: an old member of a chain whose sibling is still live is
        // un-collectable *and* makes its week ineligible for summarization,
        // because the sibling sits inside that week and the summarize query
        // filters the whole week on `created_at < raw_cutoff`. The week never
        // qualifies, the summary is never written, the members are never
        // collectable, and the chain is pinned forever. One extra day of slack
        // breaks the cycle while leaving the chain-whole guarantee intact.
        let liveness_cutoff = rfc3339(now - chrono::Duration::days(RAW_RETENTION_DAYS + 1));
        let summary_cutoff_epoch = now.timestamp() - SUMMARY_RETENTION_WEEKS * WEEK_SECONDS;

        // Step 1 — batched weekly summaries. A week qualifies once its whole
        // 7-day window is past the raw retention window (AXE-AD-006: "any
        // completed week whose raw entries are all ≥14 days old"), so the
        // same `raw_cutoff` drives both step 1 and step 3.
        let (summaries_written, summaries_more) =
            gc_summarize(&tx, &raw_cutoff, limits.max_weekly_summaries_per_tick)?;

        // Step 2 — batched prune of expired summaries.
        let (summaries_deleted, summaries_delete_more) =
            gc_delete_expired_summaries(&tx, summary_cutoff_epoch, limits.max_rows_per_pass)?;

        // Step 3 — batched raw delete.
        let (raw_deleted, raw_more) =
            gc_delete_raw(&tx, &raw_cutoff, &liveness_cutoff, limits.max_rows_per_pass)?;

        let report = GcReport {
            raw_deleted,
            summaries_written,
            summaries_deleted,
            more_work_pending: raw_more || summaries_more || summaries_delete_more,
        };

        tx.commit()
            .map_err(|e| Error::Agent(format!("notice board: gc commit: {e}")))?;
        Ok(report)
    }
}

// ------------------------------------------------------------------- helpers

fn insert_into(conn: &Connection, notice: &Notice) -> Result<(), Error> {
    conn.execute(
        "INSERT INTO notices (
            id, created_at, epoch, sender, from_dept, recipients, subject, body,
            correlation_id, ack_required, acked_by, acked_at, ttl_expires_at,
            tags, thread_id, pinned, reason, metadata
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            notice.id,
            notice.created_at,
            notice.epoch,
            notice.sender,
            notice.from_dept,
            encode_recipients(&notice.recipients)?,
            notice.subject,
            notice.body,
            notice.correlation_id,
            i64::from(notice.ack_required),
            encode_strings(&notice.acked_by)?,
            notice.acked_at,
            notice.ttl_expires_at,
            encode_strings(&notice.tags)?,
            notice.thread_id,
            i64::from(notice.pinned),
            notice.reason,
            notice.metadata.as_ref().map(|m| m.to_string()),
        ],
    )
    .map_err(|e| Error::Agent(format!("notice board: insert {}: {e}", notice.id)))?;
    Ok(())
}

fn insert_in_tx(tx: &rusqlite::Transaction<'_>, notice: &Notice) -> Result<(), Error> {
    insert_into(tx, notice)
}

fn encode_recipients(recipients: &[Recipient]) -> Result<String, Error> {
    let selectors: Vec<String> = recipients.iter().map(Recipient::selector).collect();
    serde_json::to_string(&selectors)
        .map_err(|e| Error::Agent(format!("notice board: encode recipients: {e}")))
}

fn encode_strings(values: &[String]) -> Result<String, Error> {
    serde_json::to_string(values)
        .map_err(|e| Error::Agent(format!("notice board: encode json: {e}")))
}

fn decode_strings(raw: Option<String>, field: &str) -> Result<Vec<String>, Error> {
    match raw {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => serde_json::from_str(&s)
            .map_err(|e| Error::Agent(format!("notice board: decode {field}: {e} ({s})"))),
    }
}

fn decode_recipients(raw: String) -> Result<Vec<Recipient>, Error> {
    let selectors = decode_strings(Some(raw), "recipients")?;
    // Stored selectors are canonical, so a parse failure means a hand-edited
    // row; degrade to the selector verbatim rather than failing the read.
    Ok(selectors
        .into_iter()
        .map(|sel| Recipient::parse(&sel).unwrap_or(Recipient::Agent(sel)))
        .collect())
}

fn row_to_notice(row: &Row<'_>) -> rusqlite::Result<Notice> {
    let recipients: String = row.get(col::RECIPIENTS)?;
    let acked_by: Option<String> = row.get(col::ACKED_BY)?;
    let tags: String = row.get(col::TAGS)?;
    let metadata: Option<String> = row.get(col::METADATA)?;

    // JSON columns are decoded here. A malformed value degrades to the empty
    // form rather than failing the whole inbox read, and each decode cannot
    // panic — which is what lets `row_to_notice` be a plain
    // `FnMut(&Row) -> rusqlite::Result<Notice>` mapper.
    let recipients = decode_recipients(recipients).unwrap_or_else(|_| vec![Recipient::Broadcast]);
    let acked_by = decode_strings(acked_by, "acked_by").unwrap_or_default();
    let tags = decode_strings(Some(tags), "tags").unwrap_or_default();
    let metadata = metadata
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());

    Ok(Notice {
        id: row.get(col::ID)?,
        created_at: row.get(col::CREATED_AT)?,
        epoch: row.get(col::EPOCH)?,
        sender: row.get(col::SENDER)?,
        // `from_dept` is read through `nullable_text` even though the SELECT
        // `COALESCE`s it: a row written before the COALESCE existed, or any
        // row whose `from_dept` is the literal string NULL-adjacent empty,
        // must not fail the whole inbox read. §3.3's "NULL => 'unknown'" is a
        // read-time guarantee, so the read is the place to keep it.
        from_dept: nullable_text(row, col::FROM_DEPT)?,
        recipients,
        subject: row.get(col::SUBJECT)?,
        body: row.get(col::BODY)?,
        correlation_id: row.get(col::CORRELATION_ID)?,
        ack_required: row.get::<_, i64>(col::ACK_REQUIRED)? != 0,
        acked_by,
        acked_at: row.get(col::ACKED_AT)?,
        ttl_expires_at: row.get(col::TTL_EXPIRES_AT)?,
        tags,
        thread_id: nullable_text(row, col::THREAD_ID)?,
        pinned: row.get::<_, i64>(col::PINNED)? != 0,
        reason: row.get(col::REASON)?,
        metadata,
    })
}

/// Read a nullable text column as `Option<String>`.
///
/// Used for `from_dept`: `row.get::<_, String>` on a SQL `NULL` is a
/// conversion error, not a `None`.
fn nullable_text(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<String>> {
    row.get::<_, Option<String>>(idx)
}

/// Run a read query and map every row through [`row_to_notice`].
fn read_all<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<Notice>, Error> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| Error::Agent(format!("notice board: prepare: {e}")))?;
    let rows = stmt
        .query_map(params, row_to_notice)
        .map_err(|e| Error::Agent(format!("notice board: query: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::Agent(format!("notice board: row: {e}")))?);
    }
    Ok(out)
}

// ------------------------------------------------------------------- GC SQL

/// Delete up to `cap` expired, unpinned raw rows that are not part of a
/// still-live correlation chain — §3.3 step 1's "a chain is kept whole or
/// not at all".
///
/// Returns `(deleted, hit_cap)`. The candidate select is `LIMIT cap + 1`, so
/// "we stopped because we hit the cap" is detected by having one more id
/// than the budget allows, with no `COUNT(*)`.
fn gc_delete_raw(
    tx: &rusqlite::Transaction<'_>,
    raw_cutoff: &str,
    liveness_cutoff: &str,
    cap: i64,
) -> Result<(i64, bool), Error> {
    if cap <= 0 {
        return Ok((0, false));
    }
    // Four deliberate differences from §3.3's literal SQL, each a defect in
    // the literal:
    //
    // 1. `tags NOT LIKE '%"weekly-summary"%'`. A summary row is stamped with
    //    the *week it summarizes*, so it is always older than the 14-day raw
    //    window by construction. §3.3's raw delete has no such exclusion,
    //    which means the raw delete eats every summary row on the very next
    //    tick and step 2's 16-week prune can never fire. Summaries have their
    //    own retention and their own prune step.
    // 2. `AND EXISTS (... a weekly summary for this week ...)`. This is the
    //    serious one. §3.3 runs the raw delete unconditionally, so with a
    //    `max_weekly_summaries_per_tick` cap the delete destroys raw rows for
    //    weeks the summarize step has not reached yet — the cap then has
    //    nothing left to summarize, and those weeks are lost outright. With
    //    the default cap of 4, five weeks of backlog lose the fifth week
    //    permanently on the first tick. A summary can only be built from rows
    //    that still exist, so a week is collectable only *after* its summary
    //    is written. The direction matters: this must be `EXISTS`, not
    //    `NOT EXISTS` — `NOT EXISTS` inverts it into "delete only weeks that
    //    have no summary", which never collects anything at all, because the
    //    summarize step runs first in the same transaction and always leaves
    //    the summary present.
    //
    //    The outer `n.epoch` is qualified deliberately: an unqualified `epoch`
    //    inside the subquery resolves to the inner `s.epoch` under SQLite's
    //    name resolution, which would compare each summary row's own week
    //    rather than the candidate row's.
    // 3. `HAVING COUNT(*) > 0`, not §3.3's `COUNT(*) > 1`. The subquery is
    //    `GROUP BY correlation_id`, so every group holds exactly one row and
    //    `> 1` is never true — the subquery is always empty, the exclusion
    //    never fires, and chains are never protected at all.
    // 4. The `GROUP BY`/`HAVING` is retained only for readability; the
    //    correlated form below is equivalent and cheaper to plan.
    let ids = select_ids(
        tx,
        "SELECT id FROM notices n
          WHERE n.created_at < ?1
            AND n.pinned = 0
            AND n.tags NOT LIKE '%\"' || ?3 || '\"%'
            AND EXISTS (SELECT 1 FROM notices s
                         WHERE s.id = 'n_weekly_' || ((n.epoch / ?4) * ?4))
            AND n.correlation_id NOT IN (
                  SELECT correlation_id FROM notices
                   WHERE correlation_id IS NOT NULL
                     AND created_at >= ?5
                )
          ORDER BY n.created_at ASC
          LIMIT ?2",
        params![
            raw_cutoff,
            cap.saturating_add(1),
            WEEKLY_SUMMARY_TAG,
            WEEK_SECONDS,
            liveness_cutoff
        ],
    )?;
    let hit_cap = ids.len() as i64 > cap;
    Ok((delete_ids(tx, &ids, cap)?, hit_cap))
}

/// Step 2 — summarize up to `cap` completed weeks into synthetic rows.
///
/// Returns `(written, more_pending)`. Weeks whose summary already exists are
/// skipped **without** consuming the per-tick budget; otherwise a board with
/// `cap` already-summarized weeks at the front of the queue would stall
/// forever and never reach the weeks behind them.
fn gc_summarize(
    tx: &rusqlite::Transaction<'_>,
    raw_cutoff: &str,
    cap: i64,
) -> Result<(i64, bool), Error> {
    if cap <= 0 {
        return Ok((0, false));
    }

    // Two stages, and the order matters for the cap to mean anything.
    //
    // Stage 1 picks the candidate weeks with `LIMIT cap + 1`, so a board with
    // a long backlog never groups more than `cap + 1` weeks of raw rows. The
    // earlier version grouped *every* eligible week and only then stopped
    // writing at `cap` — bounded output over an unbounded scan, which is
    // exactly the failure mode `max_weekly_summaries_per_tick` exists to
    // prevent. §3.3's "summarize at most 4 completed weeks per tick" is a
    // statement about work done, not about rows written.
    let candidates = select_summarizable_weeks(tx, raw_cutoff, cap)?;
    let more_pending = candidates.len() as i64 > cap;
    let candidates: Vec<i64> = candidates.into_iter().take(cap as usize).collect();
    if candidates.is_empty() {
        return Ok((0, false));
    }

    // Stage 2 aggregates only those weeks. Each week is its own query so the
    // scan is bounded by `cap` weeks rather than by the whole backlog, and so
    // a week with a hundred thousand notices does not pull the other three
    // candidate weeks out of memory with it.
    let mut weeks: Vec<WeekSummary> = Vec::with_capacity(candidates.len());
    for week_start in &candidates {
        let week = summarize_one_week(tx, *week_start, raw_cutoff)?;
        if let Some(week) = week {
            weeks.push(week);
        }
    }

    let mut written = 0i64;
    for week in &weeks {
        // The write is `INSERT OR IGNORE` on a deterministic id, so a retried
        // pass — or a concurrent tick — cannot duplicate the row. The
        // candidate select above already skipped weeks that have a summary,
        // so a zero here means a concurrent tick won the race, not that this
        // pass wasted budget.
        written += insert_in_tx_ignore(tx, &week_summary_notice(week)?)? as i64;
    }
    Ok((written, more_pending))
}

/// The `cap + 1` oldest weeks that have eligible raw rows and no summary yet.
///
/// Returns at most `cap + 1` ids, so the caller can tell "there were exactly
/// `cap`" from "there were more" by length alone.
///
/// The `NOT EXISTS` on the deterministic summary id is what keeps a
/// partially-drained backlog from stalling. Without it the first `cap` weeks
/// would keep coming back — already summarized, so zero rows written — and the
/// weeks behind them would never be reached, so a caller looping on
/// `more_work_pending` would spin forever.
///
/// The outer `n.epoch` is qualified deliberately. An unqualified `epoch`
/// inside the subquery resolves to the *inner* `s.epoch` under SQLite's name
/// resolution, so the predicate would ask "does some summary row's own week
/// have a summary" instead of "does the candidate row's week have a summary".
/// Because every summary row sits in exactly its own summarized week, the
/// inner form is true for the first summary ever written and false for every
/// row after it, which silently reduces this select to zero candidates and
/// deadlocks the whole GC the moment the board has a single summary.
fn select_summarizable_weeks(
    tx: &rusqlite::Transaction<'_>,
    raw_cutoff: &str,
    cap: i64,
) -> Result<Vec<i64>, Error> {
    let mut stmt = tx
        .prepare(
            "SELECT (n.epoch / ?1) * ?1 AS week_start
               FROM notices n
              WHERE n.created_at < ?2
                AND n.tags NOT LIKE '%\"' || ?3 || '\"%'
                AND NOT EXISTS (SELECT 1 FROM notices s
                                 WHERE s.id = 'n_weekly_' || ((n.epoch / ?1) * ?1))
              GROUP BY week_start
              ORDER BY week_start ASC
              LIMIT ?4",
        )
        .map_err(|e| Error::Agent(format!("notice board: gc week select: {e}")))?;
    let rows = stmt
        .query_map(
            params![
                WEEK_SECONDS,
                raw_cutoff,
                WEEKLY_SUMMARY_TAG,
                cap.saturating_add(1)
            ],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| Error::Agent(format!("notice board: gc week select: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::Agent(format!("notice board: gc week read: {e}")))?);
    }
    Ok(out)
}

/// Aggregate one week. `None` when the week turned out to have no eligible
/// rows after all (a concurrent delete, or rows that only matched the week
/// predicate and not the age predicate).
fn summarize_one_week(
    tx: &rusqlite::Transaction<'_>,
    week_start: i64,
    raw_cutoff: &str,
) -> Result<Option<WeekSummary>, Error> {
    let mut stmt = tx
        .prepare(
            "SELECT COUNT(*), tags, sender
               FROM notices n
              WHERE (n.epoch / ?1) * ?1 = ?2
                AND n.created_at < ?3
                AND n.tags NOT LIKE '%\"' || ?4 || '\"%'
              GROUP BY tags, sender",
        )
        .map_err(|e| Error::Agent(format!("notice board: gc week summarize: {e}")))?;
    let rows = stmt
        .query_map(
            params![WEEK_SECONDS, week_start, raw_cutoff, WEEKLY_SUMMARY_TAG],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| Error::Agent(format!("notice board: gc week summarize: {e}")))?;

    let mut week: Option<WeekSummary> = None;
    for row in rows {
        let (count, tags_json, sender) =
            row.map_err(|e| Error::Agent(format!("notice board: gc week row: {e}")))?;
        let tags = decode_strings(Some(tags_json), "tags")?;
        let acc = week.get_or_insert_with(|| WeekSummary {
            week_start,
            ..WeekSummary::default()
        });
        acc.absorb(sender, tags, count);
    }
    Ok(week)
}

/// The deterministic id of a week's summary row.
fn summary_id(week_start: i64) -> String {
    format!("n_weekly_{week_start}")
}

/// Build the synthetic `weekly-summary` row for one week.
fn week_summary_notice(week: &WeekSummary) -> Result<Notice, Error> {
    let week_end = week.week_start + WEEK_SECONDS;
    let created_at = chrono::DateTime::from_timestamp(week.week_start, 0)
        .map(rfc3339)
        .ok_or_else(|| {
            Error::Agent(format!(
                "notice board: gc summarize: week {} is not a representable timestamp",
                week.week_start
            ))
        })?;
    Ok(Notice {
        id: summary_id(week.week_start),
        created_at,
        epoch: week.week_start,
        sender: SUMMARY_SENDER.to_string(),
        from_dept: None,
        recipients: vec![Recipient::Broadcast],
        subject: None,
        body: format!(
            "WEEKLY SUMMARY (epoch {week_start}-{week_end}):\n  Total notices: {count}\n  Top tags: {tags}\n  Top agents: {agents}",
            week_start = week.week_start,
            week_end = week_end,
            count = week.entry_count,
            tags = top_n(&week.tag_distribution, 5),
            agents = top_n(&week.agent_distribution, 5),
        ),
        correlation_id: None,
        ack_required: false,
        acked_by: Vec::new(),
        acked_at: None,
        ttl_expires_at: None,
        tags: vec![WEEKLY_SUMMARY_TAG.to_string()],
        thread_id: None,
        pinned: false,
        reason: format!("weekly summary for epoch {}", week.week_start),
        metadata: Some(serde_json::json!({
            "week_start": week.week_start,
            "week_end": week_end,
            "entry_count": week.entry_count,
            "tag_distribution": week.tag_distribution,
            "agent_distribution": week.agent_distribution,
            "outcomes": { "successes": 0, "failures": 0, "blocks": 0 },
        })),
    })
}

/// Step 3 — prune `weekly-summary` rows past the summary window, capped.
fn gc_delete_expired_summaries(
    tx: &rusqlite::Transaction<'_>,
    summary_cutoff_epoch: i64,
    cap: i64,
) -> Result<(i64, bool), Error> {
    if cap <= 0 {
        return Ok((0, false));
    }
    let ids = select_ids(
        tx,
        "SELECT id FROM notices
          WHERE tags LIKE '%\"' || ?1 || '\"%'
            AND epoch < ?2
          ORDER BY epoch ASC
          LIMIT ?3",
        params![
            WEEKLY_SUMMARY_TAG,
            summary_cutoff_epoch,
            cap.saturating_add(1)
        ],
    )?;
    let hit_cap = ids.len() as i64 > cap;
    Ok((delete_ids(tx, &ids, cap)?, hit_cap))
}

/// `SELECT id ...` into a `Vec<String>`.
fn select_ids<P: rusqlite::Params>(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    params: P,
) -> Result<Vec<String>, Error> {
    let mut stmt = tx
        .prepare(sql)
        .map_err(|e| Error::Agent(format!("notice board: gc select: {e}")))?;
    let rows = stmt
        .query_map(params, |r| r.get::<_, String>(0))
        .map_err(|e| Error::Agent(format!("notice board: gc select: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| Error::Agent(format!("notice board: gc select row: {e}")))?);
    }
    Ok(out)
}

/// Delete at most `cap` of `ids`, returning the number deleted.
fn delete_ids(tx: &rusqlite::Transaction<'_>, ids: &[String], cap: i64) -> Result<i64, Error> {
    let mut deleted = 0i64;
    for id in ids.iter().take(cap as usize) {
        deleted += tx
            .execute("DELETE FROM notices WHERE id = ?1", params![id])
            .map_err(|e| Error::Agent(format!("notice board: gc delete: {e}")))?
            as i64;
    }
    Ok(deleted)
}

/// `INSERT OR IGNORE` — returns rows actually inserted (0 = already there).
fn insert_in_tx_ignore(tx: &rusqlite::Transaction<'_>, notice: &Notice) -> Result<usize, Error> {
    tx.execute(
        "INSERT OR IGNORE INTO notices (
            id, created_at, epoch, sender, from_dept, recipients, subject, body,
            correlation_id, ack_required, acked_by, acked_at, ttl_expires_at,
            tags, thread_id, pinned, reason, metadata
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            notice.id,
            notice.created_at,
            notice.epoch,
            notice.sender,
            notice.from_dept,
            encode_recipients(&notice.recipients)?,
            notice.subject,
            notice.body,
            notice.correlation_id,
            i64::from(notice.ack_required),
            encode_strings(&notice.acked_by)?,
            notice.acked_at,
            notice.ttl_expires_at,
            encode_strings(&notice.tags)?,
            notice.thread_id,
            i64::from(notice.pinned),
            notice.reason,
            notice.metadata.as_ref().map(|m| m.to_string()),
        ],
    )
    .map_err(|e| Error::Agent(format!("notice board: gc summarize insert: {e}")))
}

/// Per-week accumulator for the weekly summary.
#[derive(Debug, Default)]
struct WeekSummary {
    week_start: i64,
    entry_count: i64,
    tag_distribution: HashMap<String, i64>,
    agent_distribution: HashMap<String, i64>,
}

impl WeekSummary {
    fn absorb(&mut self, sender: String, tags: Vec<String>, count: i64) {
        self.entry_count += count;
        for tag in tags {
            *self.tag_distribution.entry(tag).or_insert(0) += count;
        }
        *self.agent_distribution.entry(sender).or_insert(0) += count;
    }
}

/// `k(n)` pairs, highest count first — the organism's "Top tags:" line.
fn top_n(map: &HashMap<String, i64>, k: usize) -> String {
    let mut pairs: Vec<(&String, &i64)> = map.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    pairs
        .into_iter()
        .take(k)
        .map(|(key, count)| format!("{key}({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The broadcast selector, re-exported so callers matching against a board's
/// `recipients` do not have to reach into the domain module for one string.
pub const NOTICE_BROADCAST_SELECTOR: &str = BROADCAST_SELECTOR;

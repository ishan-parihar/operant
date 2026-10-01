//! DM threads — the §10 loop guard (`docs/ORG-AUTHORITY-ARCHITECTURE.md`).
//!
//! ## What this is
//!
//! §10.1 is the owner's specification: two employees DM for a *fixed* number
//! of turns — three. Each message spawns the other agent, each party may use
//! tools freely inside its turn, and **both parties must be aware of the
//! limit**. [`TurnBudget`] is the shared, decremented counter that makes that
//! hold, and [`DmThreadDb`] persists it.
//!
//! ## Why the budget lives on the thread, not on the notice
//!
//! §10.4 is the whole reason this file exists as a *store* rather than a field:
//! "A→B→A is bounded at 3 turns because the budget is shared and decremented,
//! not per-pair. That is why the budget belongs on the *thread* and not on the
//! *notice*: a per-notice cap is defeated by opening a new notice, which is
//! exactly the regression the audit called out."
//!
//! So the API makes the wrong shape hard to reach:
//!
//! - The spend path is [`DmThreadDb::spend_turn`], and it takes a **thread id**.
//!   There is no `spend_turn_for_notice(notice_id, ...)` and no method that
//!   takes a `Recipient` — a caller holding only a notice has nothing to pass.
//! - A turn is defined in §10.2 as "one message + the recipient's full tool
//!   loop to produce its reply", so the initiator's own first message spends a
//!   turn ([`DmThreadDb::open`] seeds nothing; the first
//!   [`DmThreadDb::spend_turn`] spends turn 1). Nothing is free.
//!
//! The regression test `loop_guard_terminates_abab_and_is_not_per_pair` pins
//! this: A→B→A→B alternation stops at the budget, and it stays stopped.
//!
//! ## What this does and does not bound — stated plainly
//!
//! **Bounded:** every message on a thread. After `turn_budget` spends the state
//! is [`ThreadState::Exhausted`], the thread is terminal, and a further message
//! on its `correlation_id` is *rejected with a visible recorded reason* (§10.3)
//! rather than silently dropped. Re-opening the *same* thread is impossible:
//! `open` mints a new `thread_id`.
//!
//! **Not bounded, by design:** a *new* thread between the same pair gets a
//! fresh budget — §10.3 says explicitly that "an employee can be DM'd again in
//! a new thread with a fresh budget". That is correct: the limit is a
//! conversation-length limit, not a relationship limit, and a hard per-pair cap
//! would make the first ever DM between two employees a one-shot.
//!
//! The consequence is stated rather than hidden: **runaway is bounded by the
//! number of threads, not by this module.** Two agents alternating can still
//! post forever — each hop lands in a fresh thread with 3 turns. What bounds
//! that case is the caller: the DM runner that mints threads must cap how many
//! open threads one pair may hold (see `loop_guard_does_not_bound_fresh_threads`
//! for the shape of that hole and the counter the caller keeps).
//!
//! This module cannot close that hole itself without contradicting §10.3. It
//! can only make it impossible to *accidentally* run unbounded on one thread,
//! and that is what it does.
//!
//! ## Where the table lives, and why it is not a new file
//!
//! `dm_threads` sits in the **existing** `operant_kanban.db`, the same file the
//! notice board and worklog use — see
//! [`crate::org::notice_db`] for the no-second-store argument (§1.2) and the
//! same scar (`BUGS.md` R5-1).
//!
//! ## How the schema is applied without a migration conflict
//!
//! `CREATE TABLE IF NOT EXISTS`, plus [`crate::org::schema::ensure_column`] for
//! each column so a `dm_threads` table written by an earlier packet gains
//! columns here instead of failing to open. It deliberately does **not** go
//! through `crate::migrations::migrate`: `PRAGMA user_version` is file-wide,
//! `operant_kanban.db` sits at 1 because the kanban family claims it, and
//! appending an org entry would make kanban *look* downgraded and hard-fail
//! with "refusing to downgrade". [`crate::org::schema`] documents this; §12
//! resolves it. The cost is no versioned upgrade path, which at one additive
//! table in a fresh column space is the cheaper trade.
//!
//! ## Post-closed messages: reject, never drop (Q6)
//!
//! §10.3's hard requirement. [`DmThreadDb::guard_post`] is the write barrier a
//! DM caller runs before it posts to the notice board; it returns
//! [`PostDecision::Rejected`] carrying a reason that names the thread's state
//! when the thread is terminal. There is no code path here that returns
//! "no opinion" for a thread that is gone — a `correlation_id` with no thread
//! row is [`PostDecision::NoThread`], which is *permitted* (not every
//! `correlation_id` is a DM), but a thread that exists and is `Closed` or
//! `Exhausted` is always rejected.
//!
//! ## Mutation rule
//!
//! Every mutation takes a `reason`, and a blank one is rejected at this
//! boundary rather than defaulted. A closed thread with an empty audit trail
//! is exactly the invisible-mutation failure §10.3 is written against.

use crate::error::Error;
use crate::org::notice::{Recipient, rfc3339};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

/// §10.2: `turn_budget = 3`. A turn is one message plus the recipient's full
/// tool loop, so a three-turn thread is three exchanges, not six messages.
pub const DEFAULT_TURN_BUDGET: u32 = 3;

/// Columns selected by every read, in [`DmThread`] field order.
const THREAD_COLUMNS: &str =
    "thread_id, participants, turn_budget, turns_used, state, correlation_id, opened_at, closed_at";

/// Ordinals of the columns in [`THREAD_COLUMNS`].
///
/// Named ordinals so the SELECT and the mapper cannot drift apart the way a
/// hand-counted `row.get(6)` does — the failure mode `notice_db.rs`'s `mod col`
/// documents.
mod col {
    pub(super) const THREAD_ID: usize = 0;
    pub(super) const PARTICIPANTS: usize = 1;
    pub(super) const TURN_BUDGET: usize = 2;
    pub(super) const TURNS_USED: usize = 3;
    pub(super) const STATE: usize = 4;
    pub(super) const CORRELATION_ID: usize = 5;
    pub(super) const OPENED_AT: usize = 6;
    pub(super) const CLOSED_AT: usize = 7;
}

/// One DM thread's terminal-or-not state.
///
/// [`ThreadState::Exhausted`] is distinct from [`ThreadState::Closed`] on
/// purpose: "the three turns are spent" and "a human (or a peer) ended this
/// early" are different facts and the transcript should say which happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThreadState {
    /// Accepting messages, budget remaining.
    Open,
    /// Ended by an explicit [`DmThreadDb::close`].
    Closed,
    /// Budget reached zero. Terminal, and set by [`DmThreadDb::spend_turn`].
    Exhausted,
}

impl ThreadState {
    /// The canonical stored form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Exhausted => "exhausted",
        }
    }

    /// Parse a stored value. An unrecognised value is a named error rather
    /// than a silent default to `Open` — defaulting would resurrect a terminal
    /// thread, which is the one outcome §10.3 forbids.
    pub fn parse(raw: &str) -> crate::error::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "exhausted" => Ok(Self::Exhausted),
            other => Err(Error::Agent(format!(
                "dm_thread: unknown thread state '{other}' (expected open|closed|exhausted)"
            ))),
        }
    }

    /// True when no further message may be posted to this thread.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Exhausted)
    }
}

impl std::fmt::Display for ThreadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThreadState {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A thread-level shared turn budget.
///
/// Shared, not per-party: `A→B→A→B` decrements one counter, so the pair gets
/// three exchanges total rather than three each. See the module docs for why
/// this is the loop guard §10.4 asks for.
///
/// `used` is `total`-bounded by construction — [`TurnBudget::spend`] is the only
/// mutator and it refuses at zero rather than wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnBudget {
    /// Turns the thread allows in total, across both parties.
    pub total: u32,
    /// Turns spent so far.
    pub used: u32,
}

impl TurnBudget {
    /// A fresh budget with nothing spent.
    pub fn new(total: u32) -> Self {
        Self { total, used: 0 }
    }

    /// Turns left for the thread.
    pub fn remaining(&self) -> u32 {
        self.total.saturating_sub(self.used)
    }

    /// Spend one turn.
    ///
    /// Errors at zero and does **not** mutate. Returning `Err` rather than
    /// going negative is the whole point: a negative counter would make
    /// "3 of -2 remaining" a reachable state and a caller looping on
    /// `remaining() > 0` would never see the end.
    pub fn spend(&mut self) -> Result<(), String> {
        if self.remaining() == 0 {
            return Err(format!(
                "dm_thread: turn budget exhausted (0 of {} remaining) — \
                 the thread is closed for both parties (§10.2)",
                self.total
            ));
        }
        self.used += 1;
        Ok(())
    }

    /// True when the thread has no turns left.
    pub fn is_exhausted(&self) -> bool {
        self.remaining() == 0
    }

    /// The §6.1 volatile-suffix line, via [`awareness_line`].
    pub fn awareness_line(&self) -> String {
        awareness_line(self.remaining(), self.total)
    }
}

impl Default for TurnBudget {
    /// §10.2's `turn_budget = 3`.
    fn default() -> Self {
        Self::new(DEFAULT_TURN_BUDGET)
    }
}

/// The awareness line injected into **both** parties' volatile prompt suffixes
/// (§6.1, §10.3).
///
/// Pure and total so it is testable without a store: awareness is structural
/// here, not a request the model is asked to remember. The wording carries both
/// numbers and the binding fact, because "3 turns remain" without "you cannot
/// exceed this" reads as a suggestion.
///
/// ```text
/// This DM has 2 of 3 turns remaining. Both parties are bound by this limit.
/// ```
pub fn awareness_line(remaining: u32, total: u32) -> String {
    format!(
        "This DM has {remaining} of {total} turns remaining. Both parties are bound by this limit."
    )
}

/// One DM thread row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmThread {
    /// `'dm_' || uuid v4`. Minted per [`DmThreadDb::open`]; there is no way to
    /// re-open an existing thread under its own id, so a thread-level budget
    /// cannot be escaped by "starting over" on the same row.
    pub thread_id: String,
    /// JSON array of exactly two employee ids, as `Recipient::Agent`
    /// selectors. §10.1 is a two-party protocol; the store enforces the arity.
    pub participants: Vec<Recipient>,
    /// §10.2's budget, normally [`DEFAULT_TURN_BUDGET`].
    pub turn_budget: u32,
    /// Turns spent, never greater than `turn_budget`.
    pub turns_used: u32,
    pub state: ThreadState,
    /// The board thread this DM's notices carry. Joining key for the
    /// transcript and for the post-closed rejection (§10.3).
    pub correlation_id: Option<String>,
    /// RFC3339, fixed millisecond precision (see [`rfc3339`]).
    pub opened_at: String,
    /// RFC3339. `None` while `state == Open`.
    pub closed_at: Option<String>,
}

impl DmThread {
    /// The live budget as a [`TurnBudget`] value.
    pub fn budget(&self) -> TurnBudget {
        TurnBudget {
            total: self.turn_budget,
            used: self.turns_used,
        }
    }

    /// Turns left on this thread.
    pub fn remaining(&self) -> u32 {
        self.budget().remaining()
    }

    /// The other party, given one participant's employee id.
    ///
    /// `None` when `self` is not a participant — which is also what a
    /// three-way or malformed row yields, so a caller cannot get a plausible
    /// wrong answer.
    ///
    /// The membership check is not decoration: "the first participant that is
    /// not me" would answer `Some(<me>)` for a caller who *is* a participant,
    /// because the asker is normally first in the array. So this asks both
    /// questions — am I in it, and who is the other one — rather than only the
    /// second.
    pub fn counterparty(&self, employee_id: &str) -> Option<String> {
        if !self.involves(employee_id) {
            return None;
        }
        self.participants.iter().find_map(|p| match p {
            Recipient::Agent(id) if id != employee_id => Some(id.clone()),
            _ => None,
        })
    }

    /// `true` when `employee_id` is one of the two participants.
    pub fn involves(&self, employee_id: &str) -> bool {
        self.participants
            .iter()
            .any(|p| matches!(p, Recipient::Agent(id) if id == employee_id))
    }

    /// True when this thread will accept no further message.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

/// What the write barrier says about posting one message.
///
/// This is §10.3's "rejected with a visible recorded reason" as a value, so the
/// decision is inspectable by the caller (and testable) instead of being an
/// error string that gets swallowed upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostDecision {
    /// The thread is open and has budget. Spend before posting.
    Allowed {
        thread_id: String,
        remaining_after: u32,
    },
    /// The thread exists and is terminal. `reason` names the state and is
    /// recorded on the rejecting notice.
    Rejected { thread_id: String, reason: String },
    /// No thread row for this `correlation_id`.
    ///
    /// Permitted rather than rejected: `correlation_id` is a general board join
    /// key (`notice.rs`), and a request → ack → result chain that happens not
    /// to be a DM must still post. The distinction that matters is between
    /// "there is no DM here" and "the DM here is over" — only the second is
    /// a rejection.
    NoThread,
}

impl PostDecision {
    /// `true` when the message may be posted.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. } | Self::NoThread)
    }

    /// The rejection reason, when there is one.
    pub fn rejection_reason(&self) -> Option<&str> {
        match self {
            Self::Rejected { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

/// The DM thread store.
pub struct DmThreadDb {
    conn: Arc<Mutex<Connection>>,
}

impl DmThreadDb {
    /// Open the store in an existing sqlite file, creating the parent
    /// directory first.
    ///
    /// Production callers pass the sibling kanban DB (the same file the notice
    /// board uses) or a shared handle — see the module docs.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("dm thread: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("dm thread: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach to a connection somebody else already owns.
    ///
    /// This is the path the rest of operant should use: `dm_threads` is a table
    /// inside a database another subsystem owns, and opening a second
    /// `Connection` to the same file would trade a separate-store problem for
    /// a separate-lock problem.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    /// Attach to an already-shared handle (the shape
    /// [`crate::org::notice_db::NoticeBoard::conn`] returns), so DM threads,
    /// notices and worklog all share one writer on the kanban file.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("dm thread db mutex poisoned".to_string()))
    }

    /// §10.2's `dm_threads` schema.
    ///
    /// Applied declaratively and idempotently — see the module docs for why
    /// this is not a `crate::migrations::migrate` entry.
    pub const DM_THREADS_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS dm_threads (
            thread_id      TEXT PRIMARY KEY,   -- 'dm_' || uuid v4
            participants   TEXT NOT NULL DEFAULT '[]',  -- JSON array of 2 employee ids
            turn_budget    INTEGER NOT NULL DEFAULT 3,
            turns_used     INTEGER NOT NULL DEFAULT 0,
            state          TEXT NOT NULL DEFAULT 'open',  -- open|closed|exhausted
            correlation_id TEXT,
            opened_at      TEXT NOT NULL,
            closed_at      TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_dm_threads_correlation ON dm_threads(correlation_id);
        CREATE INDEX IF NOT EXISTS idx_dm_threads_state       ON dm_threads(state);
    "#;

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::DM_THREADS_SCHEMA)
            .map_err(|e| Error::Agent(format!("dm thread: schema: {e}")))?;
        // Additive reconciliation for a `dm_threads` table written by an
        // earlier packet, so this module opens a file it did not fully
        // define instead of failing every later query. `ensure_column` is a
        // no-op when the column is present, which is the normal case.
        crate::org::schema::ensure_columns(
            &conn,
            "dm_threads",
            &[
                ("participants", "TEXT NOT NULL DEFAULT '[]'"),
                ("turn_budget", "INTEGER NOT NULL DEFAULT 3"),
                ("turns_used", "INTEGER NOT NULL DEFAULT 0"),
                ("state", "TEXT NOT NULL DEFAULT 'open'"),
                ("correlation_id", "TEXT"),
                ("opened_at", "TEXT"),
                ("closed_at", "TEXT"),
            ],
        )
        .map_err(|e| Error::Agent(format!("dm thread: ensure columns: {e}")))?;
        Ok(())
    }

    /// Open a DM thread between `a` and `b`.
    ///
    /// Rejects, at the store boundary rather than at a CLI:
    ///
    /// - a blank participant id — an employee nobody can be;
    /// - the same employee twice — a self-DM with a "both parties" budget has
    ///   one party, so the shared-decimal reasoning §10.4 relies on does not
    ///   apply and the row would be a lie about who is bound;
    /// - `turn_budget == 0` — a thread that is closed before it opens. It is
    ///   the failure mode this module exists to prevent, so it is not
    ///   constructible.
    ///
    /// `correlation_id` defaults to the new thread id, which is the natural
    /// board anchor for a DM that has no parent request.
    pub fn open(&self, a: &str, b: &str, turn_budget: u32) -> Result<DmThread, Error> {
        self.open_with_correlation(a, b, turn_budget, None)
    }

    /// [`DmThreadDb::open`], anchoring the thread to an existing board
    /// `correlation_id`.
    ///
    /// This is how a DM that continues someone else's request chain records
    /// which chain it belongs to — the join §10.2's `correlation_id` column is
    /// for.
    pub fn open_with_correlation(
        &self,
        a: &str,
        b: &str,
        turn_budget: u32,
        correlation_id: Option<String>,
    ) -> Result<DmThread, Error> {
        let a = a.trim();
        let b = b.trim();
        if a.is_empty() || b.is_empty() {
            return Err(Error::Agent(
                "dm thread: both participants must be non-blank employee ids".to_string(),
            ));
        }
        if a == b {
            return Err(Error::Agent(format!(
                "dm thread: {a} cannot DM themselves — §10.1 is a two-party \
                 protocol and a one-party thread has no second agent to spawn"
            )));
        }
        if turn_budget == 0 {
            return Err(Error::Agent(
                "dm thread: turn_budget must be at least 1 — a zero-budget thread \
                 is closed before it opens, which is the failure this guard exists \
                 to prevent (§10.2)"
                    .to_string(),
            ));
        }

        let thread = DmThread {
            thread_id: format!("dm_{}", uuid::Uuid::new_v4()),
            participants: vec![
                Recipient::Agent(a.to_string()),
                Recipient::Agent(b.to_string()),
            ],
            turn_budget,
            turns_used: 0,
            state: ThreadState::Open,
            correlation_id: Some(
                correlation_id.unwrap_or_else(|| format!("dm_{}", uuid::Uuid::new_v4())),
            ),
            opened_at: rfc3339(chrono::Utc::now()),
            closed_at: None,
        };

        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO dm_threads (
                thread_id, participants, turn_budget, turns_used, state,
                correlation_id, opened_at, closed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                thread.thread_id,
                encode_participants(&thread.participants)?,
                i64::from(thread.turn_budget),
                i64::from(thread.turns_used),
                thread.state.as_str(),
                thread.correlation_id,
                thread.opened_at,
                thread.closed_at,
            ],
        )
        .map_err(|e| Error::Agent(format!("dm thread: insert {}: {e}", thread.thread_id)))?;
        Ok(thread)
    }

    /// Spend one turn on `thread_id` and return the resulting budget.
    ///
    /// This is **the** loop guard (§10.4). It takes a thread id and nothing
    /// else — there is no notice-shaped entry point — so the ceiling cannot be
    /// sidestepped by posting under a new notice, which is exactly the
    /// regression the audit called out.
    ///
    /// At zero remaining the write is refused *and* the row is not touched, so
    /// the failure is visible to the caller instead of producing a negative
    /// counter. When the spend brings the thread to zero the same transaction
    /// flips `state` to [`ThreadState::Exhausted`] and stamps `closed_at`, so a
    /// thread cannot sit at `open` with nothing left to spend.
    pub fn spend_turn(&self, thread_id: &str) -> Result<TurnBudget, Error> {
        let mut conn = self.lock_conn()?;
        let tx = conn
            .transaction()
            .map_err(|e| Error::Agent(format!("dm thread: spend begin: {e}")))?;

        let current = fetch_in_tx(&tx, thread_id)?;
        let mut budget = current.budget();
        if current.state.is_terminal() {
            return Err(Error::Agent(format!(
                "dm thread: cannot spend on {thread_id}: thread state is '{}' — \
                 the conversation ended and further notices on correlation_id '{}' \
                 are rejected (§10.3)",
                current.state,
                current.correlation_id.as_deref().unwrap_or("<none>")
            )));
        }
        // `TurnBudget::spend` refuses at zero without mutating; the string it
        // returns is the visible recorded reason the audit asked for.
        budget
            .spend()
            .map_err(|e| Error::Agent(format!("dm thread: {thread_id}: {e}")))?;

        let state = if budget.is_exhausted() {
            ThreadState::Exhausted
        } else {
            ThreadState::Open
        };
        let closed_at = (state.is_terminal()).then(|| rfc3339(chrono::Utc::now()));
        tx.execute(
            "UPDATE dm_threads
                SET turns_used = ?2, state = ?3,
                    closed_at = COALESCE(?4, closed_at)
              WHERE thread_id = ?1",
            params![thread_id, i64::from(budget.used), state.as_str(), closed_at,],
        )
        .map_err(|e| Error::Agent(format!("dm thread: spend update {thread_id}: {e}")))?;
        tx.commit()
            .map_err(|e| Error::Agent(format!("dm thread: spend commit: {e}")))?;
        Ok(budget)
    }

    /// Close a thread early, with a recorded reason.
    ///
    /// A blank or whitespace-only `reason` is rejected here, at the store
    /// boundary, with no default substituted — the same rule Wave 1 enforces on
    /// the notice board and the kanban. A terminal thread is refused: `Closed`
    /// and `Exhausted` are both end states, and downgrading `Exhausted` to
    /// `Closed` would erase the fact that the budget, rather than a person, ran
    /// out.
    pub fn close(&self, thread_id: &str, reason: &str) -> Result<DmThread, Error> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::Agent(
                "dm thread: close requires a non-empty reason — a thread that ended \
                 with no recorded cause is indistinguishable from one that was \
                 lost (§10.3)"
                    .to_string(),
            ));
        }
        let conn = self.lock_conn()?;
        let current = require(&conn, thread_id)?;
        if current.state.is_terminal() {
            return Err(Error::Agent(format!(
                "dm thread: cannot close {thread_id}: state is already '{}' — \
                 close is for ending an open thread early (reason: {reason})",
                current.state
            )));
        }
        conn.execute(
            "UPDATE dm_threads SET state = ?2, closed_at = ?3 WHERE thread_id = ?1",
            params![
                thread_id,
                ThreadState::Closed.as_str(),
                rfc3339(chrono::Utc::now())
            ],
        )
        .map_err(|e| Error::Agent(format!("dm thread: close update {thread_id}: {e}")))?;
        // The write lock is released before the read-back, so a long-lived
        // `MutexGuard` is not held across the second query.
        drop(conn);
        self.get(thread_id)?
            .ok_or_else(|| Error::Agent(format!("dm thread: vanished after close: {thread_id}")))
    }

    /// Fetch one thread.
    pub fn get(&self, thread_id: &str) -> Result<Option<DmThread>, Error> {
        let conn = self.lock_conn()?;
        fetch(&conn, thread_id)
    }

    /// Every still-open thread `employee_id` participates in, oldest first.
    ///
    /// The DM runner's "what am I in the middle of" query, and the input to
    /// the awareness injection: for each open thread the volatile suffix gets
    /// [`awareness_line`] for **both** parties.
    pub fn open_threads_for(&self, employee_id: &str) -> Result<Vec<DmThread>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {THREAD_COLUMNS} FROM dm_threads
                  WHERE state = 'open'
                    AND EXISTS (SELECT 1 FROM json_each(dm_threads.participants) je
                                 WHERE je.value = ?1)
                  ORDER BY opened_at ASC, thread_id ASC"
            ))
            .map_err(|e| Error::Agent(format!("dm thread: open_threads_for prepare: {e}")))?;
        let selector = Recipient::Agent(employee_id.to_string()).selector();
        let rows = stmt
            .query_map(params![selector], row_to_thread)
            .map_err(|e| Error::Agent(format!("dm thread: open_threads_for query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|e| Error::Agent(format!("dm thread: open_threads_for row: {e}")))?,
            );
        }
        Ok(out)
    }

    /// Turns left on `thread_id`, or `0` for a thread that does not exist.
    ///
    /// `0` — not a panic, not an error — is deliberate: "how much can I still
    /// say here" is a prompt-injection read that must never fail a run. The
    /// DM runner needs the number; it does not need to distinguish "spent" from
    /// "no such thread", and treating a missing thread as "nothing more to say"
    /// is the safe direction anyway (§10.3).
    pub fn remaining(&self, thread_id: &str) -> Result<u32, Error> {
        Ok(self.get(thread_id)?.map(|t| t.remaining()).unwrap_or(0))
    }

    /// §10.3's write barrier: may one more message go out on this thread?
    ///
    /// `correlation_id` is the join key, so this is what a DM caller runs
    /// before posting to the notice board. A thread that exists and is terminal
    /// is [`PostDecision::Rejected`] with a reason naming the state — the
    /// "visible recorded reason", never a silent drop.
    pub fn guard_post(&self, correlation_id: &str) -> Result<PostDecision, Error> {
        let Some(thread) = self.find_by_correlation(correlation_id)? else {
            return Ok(PostDecision::NoThread);
        };
        if thread.is_terminal() {
            return Ok(PostDecision::Rejected {
                reason: format!(
                    "DM thread {} is {} ({} of {} turns spent, correlation_id {}): \
                     the conversation is over and this message is rejected, not dropped (§10.3)",
                    thread.thread_id,
                    thread.state,
                    thread.turns_used,
                    thread.turn_budget,
                    correlation_id
                ),
                thread_id: thread.thread_id,
            });
        }
        let remaining_after = thread.remaining();
        Ok(PostDecision::Allowed {
            thread_id: thread.thread_id,
            remaining_after,
        })
    }

    /// The thread anchored to `correlation_id`, if any.
    ///
    /// Oldest wins, so a board that ends up with two rows for one chain
    /// resolves to the one that started it rather than to whichever was
    /// inserted last.
    pub fn find_by_correlation(&self, correlation_id: &str) -> Result<Option<DmThread>, Error> {
        let conn = self.lock_conn()?;
        conn.query_row(
            &format!(
                "SELECT {THREAD_COLUMNS} FROM dm_threads
                  WHERE correlation_id = ?1
                  ORDER BY opened_at ASC, thread_id ASC
                  LIMIT 1"
            ),
            params![correlation_id],
            row_to_thread,
        )
        .optional()
        .map_err(|e| Error::Agent(format!("dm thread: find_by_correlation: {e}")))
    }

    /// Total rows. Diagnostics and test affordance.
    pub fn count(&self) -> Result<i64, Error> {
        let conn = self.lock_conn()?;
        conn.query_row("SELECT COUNT(*) FROM dm_threads", [], |r| r.get(0))
            .map_err(|e| Error::Agent(format!("dm thread: count: {e}")))
    }
}

// ------------------------------------------------------------------- helpers

/// Read one thread, `None` when there is no such row.
fn fetch(conn: &Connection, thread_id: &str) -> Result<Option<DmThread>, Error> {
    conn.query_row(
        &format!("SELECT {THREAD_COLUMNS} FROM dm_threads WHERE thread_id = ?1"),
        params![thread_id],
        row_to_thread,
    )
    .optional()
    .map_err(|e| Error::Agent(format!("dm thread: get {thread_id}: {e}")))
}

/// [`fetch`], but a missing thread is a named error rather than `None`.
///
/// Mutations must not act on an absent row, so they read through this instead
/// of [`fetch`] — "no such thread" then reaches the caller with the thread id
/// in the message instead of a bare `None` to unwrap somewhere further up.
fn require(conn: &Connection, thread_id: &str) -> Result<DmThread, Error> {
    fetch(conn, thread_id)?
        .ok_or_else(|| Error::Agent(format!("dm thread: no such thread: {thread_id}")))
}

fn fetch_in_tx(tx: &rusqlite::Transaction<'_>, thread_id: &str) -> Result<DmThread, Error> {
    tx.query_row(
        &format!("SELECT {THREAD_COLUMNS} FROM dm_threads WHERE thread_id = ?1"),
        params![thread_id],
        row_to_thread,
    )
    .optional()
    .map_err(|e| Error::Agent(format!("dm thread: spend lookup {thread_id}: {e}")))?
    .ok_or_else(|| Error::Agent(format!("dm thread: no such thread: {thread_id}")))
}

fn encode_participants(participants: &[Recipient]) -> Result<String, Error> {
    let selectors: Vec<String> = participants.iter().map(Recipient::selector).collect();
    serde_json::to_string(&selectors)
        .map_err(|e| Error::Agent(format!("dm thread: encode participants: {e}")))
}

fn row_to_thread(row: &Row<'_>) -> rusqlite::Result<DmThread> {
    let participants: String = row.get(col::PARTICIPANTS)?;
    let state: String = row.get(col::STATE)?;
    Ok(DmThread {
        thread_id: row.get(col::THREAD_ID)?,
        participants: decode_participants(&participants),
        turn_budget: row.get::<_, i64>(col::TURN_BUDGET)?.max(0) as u32,
        turns_used: row.get::<_, i64>(col::TURNS_USED)?.max(0) as u32,
        state: parse_state_for_row(&state),
        correlation_id: row.get(col::CORRELATION_ID)?,
        opened_at: row.get(col::OPENED_AT)?,
        closed_at: row.get(col::CLOSED_AT)?,
    })
}

/// `row_to_thread` is a plain `FnMut(&Row) -> rusqlite::Result<_>` mapper, so it
/// cannot fail a read on a malformed JSON array. It also must not default an
/// unrecognised `state` to `Open`: that would resurrect a terminal thread, the
/// one outcome §10.3 forbids. An unknown value therefore reads as `Exhausted`
/// — fail-closed, and visible to the caller through `state`.
fn parse_state_for_row(raw: &str) -> ThreadState {
    ThreadState::parse(raw).unwrap_or(ThreadState::Exhausted)
}

/// Decode the participant array.
///
/// A malformed or non-`agent:` entry degrades to `Recipient::Broadcast`, which
/// then matches nobody in `open_threads_for` — a read degrades to "this thread
/// is not yours" rather than failing the whole query.
fn decode_participants(raw: &str) -> Vec<Recipient> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let Ok(selectors) = serde_json::from_str::<Vec<String>>(raw) else {
        return vec![Recipient::Broadcast];
    };
    selectors
        .into_iter()
        .map(|sel| Recipient::parse(&sel).unwrap_or(Recipient::Broadcast))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::notice::Recipient;

    fn temp_db() -> (tempfile::TempDir, DmThreadDb) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = DmThreadDb::init(dir.path().join("operant_kanban.db")).expect("init");
        (dir, db)
    }

    fn thread(db: &DmThreadDb) -> DmThread {
        db.open("emp-a", "emp-b", DEFAULT_TURN_BUDGET)
            .expect("open")
    }

    /// A budget with `used` turns already spent — for expectations that name a
    /// position on the counter rather than replaying the spends.
    trait Tap {
        fn tap(self, used: u32) -> TurnBudget;
    }
    impl Tap for TurnBudget {
        fn tap(self, used: u32) -> TurnBudget {
            TurnBudget {
                total: self.total,
                used,
            }
        }
    }

    // --------------------------------------------------------- TurnBudget

    #[test]
    fn default_budget_is_three() {
        assert_eq!(TurnBudget::default().total, 3);
        assert_eq!(DEFAULT_TURN_BUDGET, 3, "§10.2 fixes turn_budget = 3");
    }

    #[test]
    fn budget_starts_with_the_full_total_remaining() {
        let b = TurnBudget::new(3);
        assert_eq!(b.remaining(), 3);
        assert_eq!(b.used, 0);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn spend_decrements_remaining_by_one() {
        let mut b = TurnBudget::new(3);
        b.spend().expect("first spend");
        assert_eq!(b.remaining(), 2);
        assert_eq!(b.used, 1);
        assert!(!b.is_exhausted());
        b.spend().expect("second spend");
        assert_eq!(b.remaining(), 1);
        assert!(!b.is_exhausted());
    }

    #[test]
    fn spend_errors_at_zero_and_does_not_go_negative() {
        let mut b = TurnBudget::new(2);
        b.spend().expect("first");
        b.spend().expect("second");
        assert!(b.is_exhausted());
        let err = b.spend().expect_err("must refuse at zero");
        assert!(err.contains("exhausted"), "unhelpful error: {err}");
        // The counter must be untouched by the refused spend: no wrap, no
        // decrement below zero, so "remaining" stays a meaningful number.
        assert_eq!(b.used, 2, "a refused spend must not mutate");
        assert_eq!(b.remaining(), 0);
        // And it stays refused, idempotently.
        assert!(b.spend().is_err());
        assert_eq!(b.remaining(), 0);
    }

    #[test]
    fn budget_with_zero_total_is_exhausted_from_the_start() {
        let b = TurnBudget::new(0);
        assert!(b.is_exhausted());
        assert_eq!(b.remaining(), 0);
        let mut b = b;
        assert!(b.spend().is_err());
    }

    #[test]
    fn awareness_line_names_remaining_and_total() {
        let line = awareness_line(2, 3);
        assert!(line.contains('2'), "must state remaining: {line}");
        assert!(line.contains('3'), "must state total: {line}");
        assert!(
            line.to_lowercase().contains("both parties"),
            "must state that the limit binds both: {line}"
        );
        // The same rendering comes out of the budget's own method, so the DM
        // runner can inject either form without re-deriving the wording.
        let mut spent = TurnBudget::new(3);
        spent.spend().expect("one spend");
        assert_eq!(line, spent.awareness_line());
    }

    // -------------------------------------------------------- ThreadState

    #[test]
    fn thread_state_round_trips_through_display_and_fromstr() {
        for (state, text) in [
            (ThreadState::Open, "open"),
            (ThreadState::Closed, "closed"),
            (ThreadState::Exhausted, "exhausted"),
        ] {
            assert_eq!(state.to_string(), text);
            assert_eq!(ThreadState::from_str(text).expect("parses"), state);
            assert_eq!(ThreadState::as_str(&state), text);
        }
        // Case and padding tolerance, matching the notice selector rules.
        assert_eq!(
            ThreadState::from_str(" OPEN ").expect("parses"),
            ThreadState::Open
        );
        assert_eq!(
            ThreadState::from_str("Closed").expect("parses"),
            ThreadState::Closed
        );
    }

    #[test]
    fn unknown_thread_state_is_a_named_error_not_a_silent_open() {
        // Defaulting an unknown state to `Open` would resurrect a terminal
        // thread — the one outcome §10.3 forbids.
        let err = ThreadState::from_str("finished").expect_err("must refuse");
        assert!(err.to_string().contains("unknown thread state"), "{err}");
    }

    #[test]
    fn terminal_states_are_terminal() {
        assert!(!ThreadState::Open.is_terminal());
        assert!(ThreadState::Closed.is_terminal());
        assert!(ThreadState::Exhausted.is_terminal());
    }

    // ------------------------------------------------------------- open()

    #[test]
    fn fresh_thread_has_three_remaining() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        assert_eq!(t.turn_budget, 3);
        assert_eq!(t.turns_used, 0);
        assert_eq!(t.remaining(), 3);
        assert_eq!(t.state, ThreadState::Open);
        assert_eq!(t.closed_at, None);
        assert!(t.thread_id.starts_with("dm_"), "{}", t.thread_id);
        assert_eq!(
            t.participants,
            vec![
                Recipient::Agent("emp-a".into()),
                Recipient::Agent("emp-b".into())
            ]
        );
        assert_eq!(db.remaining(&t.thread_id).expect("remaining"), 3);
    }

    #[test]
    fn open_rejects_the_same_participant_twice() {
        let (_dir, db) = temp_db();
        let err = db.open("emp-a", "emp-a", 3).expect_err("must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("cannot DM themselves"),
            "unhelpful error: {msg}"
        );
        assert_eq!(db.count().expect("count"), 0, "no row may be written");
    }

    #[test]
    fn open_rejects_a_blank_participant() {
        let (_dir, db) = temp_db();
        assert!(db.open("emp-a", "   ", 3).is_err());
        assert!(db.open("", "emp-b", 3).is_err());
        assert_eq!(db.count().expect("count"), 0);
    }

    #[test]
    fn open_rejects_a_zero_budget() {
        let (_dir, db) = temp_db();
        let err = db.open("emp-a", "emp-b", 0).expect_err("must refuse");
        assert!(
            err.to_string().contains("at least 1"),
            "unhelpful error: {err}"
        );
        assert_eq!(db.count().expect("count"), 0, "no row may be written");
    }

    #[test]
    fn open_with_correlation_anchors_the_thread_to_an_existing_chain() {
        let (_dir, db) = temp_db();
        let t = db
            .open_with_correlation("emp-a", "emp-b", 3, Some("corr-77".into()))
            .expect("open");
        assert_eq!(t.correlation_id.as_deref(), Some("corr-77"));
        assert_eq!(
            db.find_by_correlation("corr-77")
                .expect("find")
                .map(|f| f.thread_id),
            Some(t.thread_id.clone())
        );
    }

    // ---------------------------------------------------------- spend_turn()

    #[test]
    fn spending_decrements_the_thread_budget() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        let after = db.spend_turn(&t.thread_id).expect("spend 1");
        assert_eq!(after.used, 1);
        assert_eq!(after.remaining(), 2);
        assert_eq!(db.remaining(&t.thread_id).expect("remaining"), 2);
        let read = db.get(&t.thread_id).expect("get").expect("present");
        assert_eq!(read.turns_used, 1);
        assert_eq!(read.state, ThreadState::Open);
        assert_eq!(read.closed_at, None);
    }

    #[test]
    fn spending_past_the_budget_errors_rather_than_going_negative() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        for expected_used in 1..=3 {
            let b = db.spend_turn(&t.thread_id).expect("spend");
            assert_eq!(b.used, expected_used);
        }
        let err = db
            .spend_turn(&t.thread_id)
            .expect_err("the fourth turn must be refused");
        let msg = err.to_string();
        assert!(msg.contains("exhausted"), "unhelpful error: {msg}");
        // The counter is untouched by the refusal.
        let read = db.get(&t.thread_id).expect("get").expect("present");
        assert_eq!(read.turns_used, 3, "a refused spend must not mutate");
        assert_eq!(read.remaining(), 0, "never negative");
        assert_eq!(db.remaining(&t.thread_id).expect("remaining"), 0);
    }

    #[test]
    fn thread_auto_transitions_to_exhausted_at_zero_and_stamps_closed_at() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        db.spend_turn(&t.thread_id).expect("1");
        let mid = db.get(&t.thread_id).expect("get").expect("present");
        assert_eq!(mid.state, ThreadState::Open);
        assert_eq!(mid.closed_at, None, "an open thread has no closed_at");

        db.spend_turn(&t.thread_id).expect("2");
        db.spend_turn(&t.thread_id).expect("3");

        let end = db.get(&t.thread_id).expect("get").expect("present");
        assert_eq!(end.state, ThreadState::Exhausted);
        assert_eq!(end.turns_used, end.turn_budget);
        assert_eq!(end.remaining(), 0);
        assert!(end.closed_at.is_some(), "exhaustion stamps closed_at");
        assert!(end.is_terminal());
    }

    #[test]
    fn remaining_is_correct_after_n_spends() {
        let (_dir, db) = temp_db();
        let t = db.open("emp-a", "emp-b", 5).expect("open");
        assert_eq!(db.remaining(&t.thread_id).expect("r"), 5);
        for (i, expected) in [4, 3, 2, 1, 0].into_iter().enumerate() {
            db.spend_turn(&t.thread_id).expect("spend");
            assert_eq!(
                db.remaining(&t.thread_id).expect("r"),
                expected,
                "after {} spend(s)",
                i + 1
            );
        }
    }

    #[test]
    fn spending_on_an_unknown_thread_is_a_named_error() {
        let (_dir, db) = temp_db();
        let err = db.spend_turn("dm_nope").expect_err("must fail");
        assert!(err.to_string().contains("no such thread"), "{err}");
    }

    // -------------------------------------------------------------- close()

    #[test]
    fn close_on_an_exhausted_thread_errors() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        for _ in 0..3 {
            db.spend_turn(&t.thread_id).expect("spend");
        }
        assert_eq!(
            db.get(&t.thread_id).expect("get").expect("present").state,
            ThreadState::Exhausted
        );
        let err = db
            .close(&t.thread_id, "winding up after the budget ran out")
            .expect_err("must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("exhausted"),
            "error must name the state: {msg}"
        );
        // The refusal must not downgrade Exhausted to Closed — the difference
        // between "the budget ended this" and "a person ended this" is the
        // point of two states.
        assert_eq!(
            db.get(&t.thread_id).expect("get").expect("present").state,
            ThreadState::Exhausted
        );
    }

    #[test]
    fn close_ends_an_open_thread_and_records_closed_at() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        db.spend_turn(&t.thread_id).expect("spend 1");
        let closed = db
            .close(&t.thread_id, "escalated to the head of department")
            .expect("close");
        assert_eq!(closed.state, ThreadState::Closed);
        assert!(closed.closed_at.is_some());
        assert_eq!(closed.turns_used, 1, "close does not spend a turn");
        assert!(!db.spend_turn(&t.thread_id).is_ok());
    }

    #[test]
    fn close_requires_a_non_blank_reason() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        for bad in ["", "   ", "\t\n  "] {
            let err = db.close(&t.thread_id, bad).expect_err("must refuse");
            assert!(
                err.to_string().contains("non-empty reason"),
                "unhelpful error for {bad:?}: {err}"
            );
        }
        // And the thread is untouched — a refused mutation writes nothing.
        assert_eq!(
            db.get(&t.thread_id).expect("get").expect("present").state,
            ThreadState::Open
        );
    }

    #[test]
    fn close_on_an_unknown_thread_is_a_named_error() {
        let (_dir, db) = temp_db();
        let err = db.close("dm_nope", "because").expect_err("must fail");
        assert!(err.to_string().contains("no such thread"), "{err}");
    }

    // ------------------------------------------- posting to a closed thread

    /// §10.3 / Q6: **a message on a closed thread is rejected with a visible
    /// recorded reason, never silently dropped.** This is the no-silent-drop
    /// test, stated as such.
    #[test]
    fn posting_to_a_closed_thread_is_rejected_with_a_reason_naming_the_state() {
        let (_dir, db) = temp_db();
        let t = db
            .open_with_correlation("emp-a", "emp-b", 3, Some("corr-1".into()))
            .expect("open");
        db.spend_turn(&t.thread_id).expect("spend");
        db.close(&t.thread_id, "resolved by the head of department")
            .expect("close");

        let decision = db.guard_post("corr-1").expect("guard");
        assert!(
            !decision.is_allowed(),
            "a message on a closed thread must not be postable"
        );
        let reason = decision
            .rejection_reason()
            .expect("a rejection must carry a recorded reason");
        assert!(
            reason.contains("closed"),
            "the reason must name the closed state: {reason}"
        );
        assert!(
            reason.contains("not dropped"),
            "the reason must state it was rejected rather than dropped: {reason}"
        );
        assert!(
            reason.contains("corr-1"),
            "the reason must name the chain: {reason}"
        );
    }

    #[test]
    fn posting_to_an_exhausted_thread_is_rejected_with_a_reason_naming_the_state() {
        let (_dir, db) = temp_db();
        let t = db
            .open_with_correlation("emp-a", "emp-b", 3, Some("corr-2".into()))
            .expect("open");
        for _ in 0..3 {
            db.spend_turn(&t.thread_id).expect("spend");
        }
        let decision = db.guard_post("corr-2").expect("guard");
        let reason = decision.rejection_reason().expect("reason");
        assert!(
            reason.contains("exhausted"),
            "must name the state: {reason}"
        );
        assert!(
            reason.contains("3 of 3"),
            "must state the accounting: {reason}"
        );
    }

    #[test]
    fn posting_on_an_open_thread_is_allowed_with_the_remaining_count() {
        let (_dir, db) = temp_db();
        db.open_with_correlation("emp-a", "emp-b", 3, Some("corr-3".into()))
            .expect("open");
        let decision = db.guard_post("corr-3").expect("guard");
        assert!(decision.is_allowed());
        assert_eq!(decision.rejection_reason(), None);
        assert_eq!(
            decision,
            PostDecision::Allowed {
                thread_id: {
                    db.find_by_correlation("corr-3")
                        .expect("find")
                        .expect("present")
                        .thread_id
                },
                remaining_after: 3,
            }
        );
    }

    #[test]
    fn a_correlation_id_with_no_thread_is_permitted_not_rejected() {
        // `correlation_id` is a general board join key; a request -> ack ->
        // result chain that is not a DM must still be able to post. What must
        // never happen is "there is no thread" being used to *quiet* a thread
        // that exists and is over — hence a separate variant.
        let (_dir, db) = temp_db();
        assert_eq!(
            db.guard_post("corr-none").expect("guard"),
            PostDecision::NoThread
        );
        assert!(db.guard_post("corr-none").expect("guard").is_allowed());
    }

    #[test]
    fn a_closed_thread_stays_rejected_for_every_later_attempt() {
        let (_dir, db) = temp_db();
        let t = db
            .open_with_correlation("emp-a", "emp-b", 3, Some("corr-4".into()))
            .expect("open");
        db.close(&t.thread_id, "done").expect("close");
        for attempt in 1..=4 {
            let decision = db.guard_post("corr-4").expect("guard");
            assert!(
                decision.rejection_reason().is_some(),
                "attempt {attempt} must still be rejected"
            );
        }
    }

    // --------------------------------------------------- participants / reads

    #[test]
    fn open_threads_for_returns_only_open_threads_for_that_employee() {
        let (_dir, db) = temp_db();
        let a_b = thread(&db);
        let a_c = db.open("emp-a", "emp-c", 3).expect("open");
        let closed = db.open("emp-a", "emp-d", 3).expect("open");
        db.close(&closed.thread_id, "no longer needed")
            .expect("close");

        let for_a = db.open_threads_for("emp-a").expect("query");
        let ids: Vec<&str> = for_a.iter().map(|t| t.thread_id.as_str()).collect();
        assert_eq!(ids.len(), 2, "closed thread must not be listed");
        assert!(ids.contains(&a_b.thread_id.as_str()));
        assert!(ids.contains(&a_c.thread_id.as_str()));

        assert!(db.open_threads_for("emp-b").expect("query").len() == 1);
        assert!(
            db.open_threads_for("emp-zzz").expect("query").is_empty(),
            "a non-participant sees none"
        );
    }

    #[test]
    fn counterparty_and_involvement_are_symmetric() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        assert_eq!(t.counterparty("emp-a").as_deref(), Some("emp-b"));
        assert_eq!(t.counterparty("emp-b").as_deref(), Some("emp-a"));
        assert_eq!(t.counterparty("emp-c"), None);
        assert!(t.involves("emp-a") && t.involves("emp-b"));
        assert!(!t.involves("emp-c"));
    }

    /// Comparing the read against the value `open` returned is wrong: that is
    /// a snapshot taken *before* [`DmThreadDb::spend_turn`] ran, so its
    /// `turns_used` is legitimately 0. Every other field must match, which is
    /// what this pins — the id, both participants, the budget, the correlation
    /// anchor, the timestamps.
    #[test]
    fn a_stored_thread_round_trips_through_the_schema() {
        let (_dir, db) = temp_db();
        let t = db.open("emp-a", "emp-b", 7).expect("open");
        db.spend_turn(&t.thread_id).expect("spend");
        let read = db.get(&t.thread_id).expect("get").expect("present");
        assert_eq!(read.turns_used, 1, "the spend is persisted");
        assert_eq!(
            DmThread {
                turns_used: t.turns_used,
                ..read.clone()
            },
            t,
            "every other field must survive the round trip"
        );
        assert_eq!(read.turn_budget, 7);
        assert_eq!(read.remaining(), 6);
        assert_eq!(read.budget(), TurnBudget::new(7).tap(1));
    }

    #[test]
    fn remaining_on_an_unknown_thread_is_zero_not_a_panic() {
        let (_dir, db) = temp_db();
        assert_eq!(db.remaining("dm_nope").expect("remaining"), 0);
    }

    #[test]
    fn schema_is_idempotent_across_repeated_opens() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant_kanban.db");
        for _ in 0..3 {
            DmThreadDb::init(path.clone()).expect("reopen must reconcile");
        }
        let db = DmThreadDb::init(path).expect("init");
        assert_eq!(db.count().expect("count"), 0);
        // A thread written before the reopen survived it.
        let t = thread(&db);
        db.spend_turn(&t.thread_id).expect("spend");
        let cols = crate::org::schema::table_columns(&db.lock_conn().expect("lock"), "dm_threads")
            .expect("columns");
        for want in [
            "thread_id",
            "participants",
            "turn_budget",
            "turns_used",
            "state",
            "correlation_id",
            "opened_at",
            "closed_at",
        ] {
            assert!(cols.contains(&want.to_string()), "missing column {want}");
        }
    }

    // ------------------------------------------------- the loop guard itself

    /// The §10.4 regression: A→B→A alternation terminates at the shared
    /// budget, and it stays terminated.
    /// With a shared, thread-level budget the alternation below cannot draw on
    /// two separate allowances — there is no per-speaker allowance to draw on,
    /// because the spend path takes a thread id and nothing else.
    #[test]
    fn loop_guard_terminates_abab_and_is_not_per_pair() {
        let (_dir, db) = temp_db();
        let t = thread(&db);
        // Six messages were going to be exchanged. Three are allowed.
        let attempted_messages = 6;

        let mut posted = 0usize;
        let mut first_refusal: Option<String> = None;
        for _ in 0..attempted_messages {
            match db.spend_turn(&t.thread_id) {
                Ok(_) => posted += 1,
                Err(e) => {
                    first_refusal = Some(e.to_string());
                    break;
                }
            }
        }

        assert_eq!(posted, 3, "A→B→A must stop after 3 shared turns");
        let refusal = first_refusal.expect("the 4th turn must be refused");
        assert!(refusal.contains("exhausted"), "unhelpful: {refusal}");

        // Per-pair would have allowed 6 (3 each); the point of the test is that
        // it did not.
        assert_ne!(posted, 6, "the budget must be shared, not per-pair");
        assert!(posted < attempted_messages);

        // And the thread is terminal and stays that way.
        assert_eq!(
            db.get(&t.thread_id).expect("get").expect("present").state,
            ThreadState::Exhausted
        );
        for _ in 0..5 {
            assert!(db.spend_turn(&t.thread_id).is_err(), "must stay refused");
        }
    }

    /// What the loop guard does **not** bound, pinned as a test so the next
    /// reader does not mistake it for coverage.
    ///
    /// §10.3 says an employee can be DM'd again in a *new* thread with a fresh
    /// budget — a new conversation is legitimately a new budget. So alternating
    /// on fresh threads is unbounded *here*, and the counter is the **number of
    /// threads**, which the DM runner that mints them must cap. This test
    /// documents the hole; [`awareness_line`] is what makes the remaining turns
    /// visible in both prompts, and the thread cap the caller's is.
    #[test]
    fn loop_guard_does_not_bound_fresh_threads_and_says_so() {
        let (_dir, db) = temp_db();
        let mut turns_spent = 0usize;
        for hop in 0..10 {
            let t = db
                .open("emp-a", "emp-b", DEFAULT_TURN_BUDGET)
                .expect("open");
            assert_eq!(
                db.open_threads_for("emp-a").expect("q").len(),
                1,
                "an unspent thread is open and must be listed — this is the shape \
                 a runaway leaves behind: one open thread at a time"
            );
            while db.spend_turn(&t.thread_id).is_ok() {
                turns_spent += 1;
            }
            assert_eq!(
                db.open_threads_for("emp-a").expect("q").len(),
                0,
                "each exhausted thread drops out of the open list, so it leaves a \
                 terminal row rather than a growing open backlog"
            );
            assert_eq!(hop + 1, db.count().expect("count") as usize);
        }
        // Ten threads × three turns: each bounded, the total unbounded. This
        // is by design (§10.3); the bound is the caller's thread cap.
        assert_eq!(turns_spent, 30);
        // And what the caller can see per thread is the awareness line, which
        // is what §6.1 injects into both parties' volatile suffixes.
        let t = db
            .open("emp-a", "emp-b", DEFAULT_TURN_BUDGET)
            .expect("open");
        let line = awareness_line(t.remaining(), t.turn_budget);
        assert!(line.contains("3"), "{line}");
    }
}

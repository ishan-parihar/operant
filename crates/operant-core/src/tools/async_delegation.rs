//! Async (background) delegation registry — hermes `tools/async_delegation.py`
//! parity.
//!
//! Backs `delegate_task(background=true)`: the parent agent dispatches a
//! subagent that runs on a spawned tokio task and returns a handle
//! immediately, so the user and the model can keep working while the child
//! runs. When the child finishes, the record transitions to
//! completed/failed and an `AgentEvent::AsyncDelegation` is pushed onto the
//! parent's event channel so the CLI/TUI can surface it. The agent polls
//! progress with `delegate_task(query="<id>")`.
//!
//! This module owns ONLY the async lifecycle + records; the actual child
//! build + run is delegated back to `SubAgentTool::call`, so depth limits,
//! tool inheritance, credential config, and result shaping stay in one
//! place (hermes: "the actual child build + run is delegated back to
//! delegate_tool._run_single_child").

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many concurrent background delegations are allowed (hermes
/// `_DEFAULT_MAX_ASYNC_CHILDREN = 3` parity).
pub const DEFAULT_MAX_ASYNC_CHILDREN: usize = 3;
/// How many terminal (completed/failed) records to retain before pruning.
pub const MAX_RETAINED_TERMINAL: usize = 50;

/// Lifecycle status of a background delegation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsyncDelegationStatus {
    /// Dispatched, child still running.
    Pending,
    /// Child finished and the result was recorded.
    Completed,
    /// Child errored or timed out.
    Failed,
}

/// A single background delegation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncDelegationRecord {
    /// Opaque handle returned to the parent at dispatch time.
    pub delegation_id: String,
    pub status: AsyncDelegationStatus,
    /// The task goal (used in status queries so the parent can re-orient).
    pub goal: String,
    /// Model the child ran on.
    pub model: String,
    /// Unix seconds at dispatch.
    pub dispatch_time: u64,
    pub completed_time: Option<u64>,
    /// Final child answer (Completed only).
    pub result: Option<String>,
    /// Error text (Failed only).
    pub error: Option<String>,
    /// The session the child actually ran in.
    ///
    /// MEASURED 2026-11-14 (audit finding 10): a delegated child mints its own
    /// `sess_<uuid>` and nothing joined it to this record, so the operator could
    /// see that a delegation had run and could not open what it did. `serde(default)`
    /// so a ledger written before this field still loads.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The session that dispatched this delegation — the parent edge.
    ///
    /// MEASURED 2026-11-14 (audit finding 2): the record carried the child's
    /// session but not the parent's, so `records_snapshot` set
    /// `parent_id: None` for every subagent row and the delegation tree could
    /// not be rebuilt. Both references carry this edge as primary structure —
    /// jcode walks `report_back_to_session_id` for ancestry, codex writes a
    /// graph edge at spawn. Without it the hub shows a flat list in which a
    /// worker is indistinguishable from a top-level session, and nothing can
    /// answer "what did this session spawn?".
    ///
    /// `serde(default)` for the same reason as `session_id`: a ledger written
    /// before the edge existed still loads.
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// An optional caller-chosen key that makes a re-dispatch of the SAME work
    /// return the existing handle instead of starting a second child.
    ///
    /// MEASURED 2026-11-14 (audit finding 6): dispatch had no idempotency at
    /// all. `new_id()` mints a fresh `dlg-<epoch>-<n>` on every call, so a parent
    /// that lost the handle — context compaction, a dropped tool result, a
    /// torn-down turn — and re-dispatched the same goal got a SECOND worker, a
    /// second token spend and a second ledger row, with nothing joining them.
    /// jcode's `swarm_mutation_state` is the reference shape: request keys plus
    /// persisted final responses, so a replayed mutation replays its answer
    /// instead of repeating the work.
    ///
    /// `serde(default)` so a ledger written before the field loads.
    #[serde(default)]
    pub request_key: Option<String>,
}

/// What admitting a background delegation did.
///
/// The caller MUST distinguish these: a reused admission means the work is
/// already in flight, so running a child for it would double-spend exactly what
/// the key exists to prevent. Returning a bare id would lose that and make the
/// deduplication a silent no-op (audit finding 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationAdmission {
    /// A new record exists; the caller owns settling this id.
    Created(String),
    /// A live record with the same request key already exists; the caller must
    /// NOT start another child — poll this id instead.
    Reused(String),
}

impl DelegationAdmission {
    /// The handle either way, so a caller that only needs the id can read it.
    pub fn id(&self) -> &str {
        match self {
            Self::Created(id) | Self::Reused(id) => id,
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, AsyncDelegationRecord>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, AsyncDelegationRecord>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Where the ledger persists. `None` until a process opts in.
///
/// MEASURED 2026-11-14 (audit finding 3): the registry is a process-local
/// `OnceLock<Mutex<HashMap>>` and a restart forgot every delegation. A record
/// that was `Pending` when the process died then stayed `Pending` for the life
/// of the new one — the hub's `Working` count and the background-task widget
/// kept counting work whose child had died with the old process, and no surface
/// could tell the difference. Persistence is opt-in so a test process (and any
/// embedding that does not want a sidecar) never touches the operator's disk.
static LEDGER_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Opt in to a durable ledger at `path`, and recover whatever the previous
/// process left there. Returns how many records were settled as interrupted.
///
/// MEASURED 2026-11-14 (audit finding 4): this returned `bool` and discarded the
/// count, and `recover_interrupted` computed it only to throw it away. So the
/// rows each said "the process restarted while this delegation was running", but
/// nothing ever said HOW MANY — an operator who left six delegations running saw
/// six unexplained failures and no statement of what happened. jcode appends an
/// idempotent note and notifies the owner on the next attach. The count is the
/// minimum: it exists, it is returned, and the caller reports it once.
///
/// `0` covers both "nothing to recover" and "this process did not win the path"
/// — a process that did not win recovery must not report recovered work, so the
/// caller's action is the same in both cases.
pub fn enable_persistence(path: PathBuf) -> usize {
    if LEDGER_PATH.set(path).is_err() {
        return 0;
    }
    recover_interrupted()
}

/// Rewrite the ledger from the in-memory registry.
///
/// (session-hub plan T5) The write MERGES the memory into the file rather than
/// replacing the file wholesale. Before this, a process whose registry held a few
/// test records wrote ONLY those — replacing the operator's real history with
/// them on `enable_persistence`. The merge keeps every file-only record and lets
/// memory win on the records THIS process actually holds (the authority: it ran
/// them), so a delegating process, a resumed process and the next one all observe
/// the same ledger.
///
/// Best-effort, as before: a write failure must never block a delegation.
/// An exclusive advisory lock on the ledger, held across one read-merge-write.
///
/// (audit finding) `persist()` reads the file, merges, and writes it back. With
/// two processes persisting at once — the TUI and the gateway share one home —
/// both could read the same base and the second write would drop the first's new
/// record, which is exactly the cross-process case T5's merge exists for. The
/// lock is what makes the merge mean anything.
///
/// Unix-only, like the rest of this module's file handling (`fs_secrets` is
/// already Unix-shaped): `flock` on a sibling `.lock` file, so the ledger's own
/// bytes are never the lock target.
struct LedgerLock {
    file: std::fs::File,
}

impl LedgerLock {
    fn acquire(path: &std::path::Path) -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let lock_path = path.with_extension("lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .ok()?;
        // Blocking exclusive: a second writer waits its turn rather than
        // overwriting a record it never read.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            return None;
        }
        Some(Self { file })
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn persist() {
    let Some(path) = LEDGER_PATH.get() else {
        return;
    };
    // Serialize the read-merge-write across processes. A writer that cannot take
    // the lock skips this persist rather than risking a lost record; the next
    // status change persists again.
    let Some(_lock) = LedgerLock::acquire(path) else {
        return;
    };
    // Merge base: the current file, which may predate this process.
    let prior: Vec<AsyncDelegationRecord> = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<AsyncDelegationRecord>>(&text).ok())
        .unwrap_or_default();
    let records = merge_records(prior, list_records());
    write_ledger_atomic(path, &records);
}

/// Whether the last persist of a record reached the file.
///
/// (iter-2457, F4) `persist()` returns `()` and swallows every failure, which is
/// right for a best-effort write and wrong for a TERMINAL one: `mark_completed` is
/// the last write a record ever receives, so a persist that did not land means the
/// file still says `Pending` — and on the next start `settle_interrupted` rewrites
/// that row to `Failed` with "the process restarted while this delegation was
/// running; its child died with that process". A delegation that SUCCEEDED is then
/// reported as a failure, and its result existed only in memory.
///
/// WHAT ACTUALLY FAILS HERE, measured and not guessed: `LedgerLock::acquire` uses
/// a BLOCKING `flock(LOCK_EX)` with no `LOCK_NB`, so a CONTENDED lock does not
/// return `None` — it waits, and then succeeds. Contention is therefore not the
/// failure this guards. What does reach the `else` branch is `acquire` failing to
/// OPEN or LOCK the sibling `.lock` file at all: a read-only or full filesystem, a
/// home that vanished mid-run, an fd-table exhaustion. Those are rare and
/// environmental, which is exactly why the fix is to REPORT the failure rather
/// than to retry it hard — the retry below exists only to cross a transient open
/// error, and the value is the `bool` the caller can act on.
fn persist_terminal() -> bool {
    match LEDGER_PATH.get() {
        Some(path) => persist_terminal_at(path),
        // Persistence disabled: there is no file to lose, so nothing to report.
        None => true,
    }
}

/// [`persist_terminal`] against an explicit path.
///
/// Split out so a test can drive the real retry-and-write path against a ledger
/// it owns: `LEDGER_PATH` is a `OnceLock`, so only the FIRST caller in a process
/// can opt in, and a test that depends on winning that race is a test that fails
/// when the suite runs in a different order.
fn persist_terminal_at(path: &std::path::Path) -> bool {
    // Short, bounded backoff across a transient open/lock failure. This is NOT a
    // contention workaround — `acquire`'s `flock` blocks, so a busy lock is waited
    // out inside `acquire` and never reaches here. Keep it small: the failure this
    // crosses is environmental, and a caller waiting on a dead filesystem should
    // find out quickly rather than after a long sleep ladder.
    const ATTEMPTS: u32 = 3;
    for attempt in 0..ATTEMPTS {
        if let Some(_lock) = LedgerLock::acquire(path) {
            let prior: Vec<AsyncDelegationRecord> = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| serde_json::from_str::<Vec<AsyncDelegationRecord>>(&text).ok())
                .unwrap_or_default();
            let records = merge_records(prior, list_records());
            write_ledger_atomic(path, &records);
            return true;
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(5 << attempt));
        }
    }
    tracing::warn!(
        "delegation ledger: could not take the ledger lock after {} attempts; \
         the terminal state is still in memory and will be re-attempted",
        ATTEMPTS
    );
    false
}

/// Replace `path` with `records` atomically.
///
/// (audit finding) `write_secret_file` truncates in place, so a crash mid-write
/// would leave a half-written ledger that no longer parses — and an unparseable
/// ledger reads as "no records at all", which is the whole file lost rather than
/// the one record being written. Writing beside the target and renaming over it
/// is atomic on the same filesystem, so a reader sees the old ledger or the new
/// one, never a torn one. The mode is set on the temp before the rename, so the
/// ledger is never briefly world-readable either.
fn write_ledger_atomic(path: &std::path::Path, records: &[AsyncDelegationRecord]) {
    let Ok(json) = serde_json::to_string_pretty(records) else {
        return;
    };
    let tmp = path.with_extension("tmp");
    if crate::fs_secrets::write_secret_file(&tmp, json.as_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Merge the current in-memory records into the persisted set.
///
/// (session-hub plan T5) A file-only record is preserved: another process, or an
/// earlier one, wrote it and this process's memory is not the authority for it.
/// A memory record replaces the file row with the same id — this process ran that
/// delegation, so its state is newer than the file's. Retention is applied over
/// the merged set, then the whole ledger is sorted newest-dispatch-first so a
/// second process can list a session's children by file alone.
/// Cut the oldest terminal rows down to [`MAX_RETAINED_TERMINAL`], in place.
///
/// ONE implementation of the retention rule. It was four: `merge_records`,
/// `recover_interrupted` and `prune` each spelled the ordering out by hand, and
/// `recover_interrupted` sorted by time ALONE — so a restart that recovered two
/// delegations finished in the same second kept whichever the file happened to
/// list first, while the other paths keep the higher id. A total order is what
/// makes "the oldest N are cut" mean the same thing on every path.
///
/// That duplication has already shipped a defect: iter-2406 exists because
/// `merge_records` truncated the WHOLE set while `prune` filtered `Pending`
/// first, so a still-running child was dropped from the ledger while it ran.
///
/// `Pending` rows never count against the cap and are never cut. A pending row
/// is work in flight; the cap bounds HISTORY, and dropping a live row to make
/// room for a finished one makes a restart forget work the operator is still
/// waiting on.
fn enforce_terminal_cap(records: &mut Vec<AsyncDelegationRecord>) {
    let mut terminal: Vec<(u64, String)> = records
        .iter()
        .filter(|r| r.status != AsyncDelegationStatus::Pending)
        .map(|r| {
            (
                r.completed_time.unwrap_or(r.dispatch_time),
                r.delegation_id.clone(),
            )
        })
        .collect();
    if terminal.len() <= MAX_RETAINED_TERMINAL {
        return;
    }
    // Oldest first, so the front of the list is what the cap cuts.
    terminal.sort_by(|(time, id), (other_time, other_id)| {
        time.cmp(other_time).then_with(|| id.cmp(other_id))
    });
    let cut: std::collections::HashSet<String> = terminal
        .iter()
        .take(terminal.len() - MAX_RETAINED_TERMINAL)
        .map(|(_, id)| id.clone())
        .collect();
    records.retain(|r| !cut.contains(&r.delegation_id));
}

fn merge_records(
    prior: Vec<AsyncDelegationRecord>,
    current: Vec<AsyncDelegationRecord>,
) -> Vec<AsyncDelegationRecord> {
    let mut records = prior;
    for record in current {
        if let Some(slot) = records
            .iter_mut()
            .find(|existing| existing.delegation_id == record.delegation_id)
        {
            *slot = record;
        } else {
            records.push(record);
        }
    }
    // Retention is the same cap that applies on resume: a ledger that only
    // grows unbounded is the defect the cap exists to prevent.
    //
    // TERMINAL rows only, because that is what the cap is documented to bound
    // (`MAX_RETAINED_TERMINAL`) and what `prune()` enforces on the in-memory
    // path. Truncating the WHOLE merged set dropped a still-running delegation
    // whenever the terminal rows around it were newer: a live child vanished
    // from the ledger while it ran, so the subagent navigator and the hub could
    // not show it and a restart forgot work the operator was waiting on.
    // MEASURED 2026-11-15 (seventh-pass audit, finding 1): `prune()` would keep
    // every row of an all-Pending ledger; this path truncated it to the cap.
    enforce_terminal_cap(&mut records);
    records.sort_by(|a, b| {
        b.dispatch_time
            .cmp(&a.dispatch_time)
            .then_with(|| b.delegation_id.cmp(&a.delegation_id))
    });
    records
}

/// Adopt the previous process's ledger and settle whatever it left running.
///
/// A record that was `Pending` is a record whose child was a tokio task in a
/// process that no longer exists. Keeping it `Pending` is the lie; the honest
/// transition is `Failed` with the reason, so the row stops claiming work is in
/// progress and the parent can re-dispatch if it still wants the work.
///
/// Returns how many records were settled, so a caller can say so once.
pub fn recover_interrupted() -> usize {
    let Some(path) = LEDGER_PATH.get() else {
        return 0;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0; // no ledger, or unreadable: nothing to recover
    };
    let Ok(records) = serde_json::from_str::<Vec<AsyncDelegationRecord>>(&text) else {
        return 0; // a corrupt ledger must not stop a process from starting
    };
    let (mut recovered, settled) = settle_interrupted(records);
    // Retention still applies across a restart: a ledger that only grows is the
    // same unbounded-map defect persistence would otherwise introduce.
    //
    // Via the shared helper, so recovery cuts the same rows the live path would.
    // It used to sort by time alone: two delegations that finished in the same
    // second were cut by whichever order the file listed them in, while
    // `prune` and `merge_records` both break that tie on the id.
    enforce_terminal_cap(&mut recovered);
    if let Ok(mut map) = registry().lock() {
        *map = recovered
            .into_iter()
            .map(|rec| (rec.delegation_id.clone(), rec))
            .collect();
    }
    persist();
    settled
}

/// Settle every `Pending` row in a loaded ledger and say how many were settled.
///
/// Count the TRANSITION, not the survivors. MEASURED 2026-11-15 (third-pass
/// audit, finding 2): the count used to be read back out of the registry by
/// matching the error text `starts_with("the process restarted")`. But the
/// ledger is reloaded whole on every start, so a row settled by an EARLIER
/// restart still carries that text and was counted again — the operator was told
/// 2, then 3, then 4 for one piece of work, and re-dispatching what the warning
/// named did the work N times. A row this call did not touch is not a row this
/// call recovered, however it is worded.
///
/// Split out so the count is testable without the process-global ledger path
/// (`LEDGER_PATH` is a `OnceLock`: only the first caller in a process can set it,
/// so a test that drives the real path would race whichever test got there
/// first).
fn settle_interrupted(records: Vec<AsyncDelegationRecord>) -> (Vec<AsyncDelegationRecord>, usize) {
    let mut settled = 0usize;
    let recovered: Vec<AsyncDelegationRecord> = records
        .into_iter()
        .map(|mut rec| {
            if rec.status == AsyncDelegationStatus::Pending {
                rec.status = AsyncDelegationStatus::Failed;
                rec.completed_time = Some(now_secs());
                rec.error = Some(
                    "the process restarted while this delegation was running; its child died \
                     with that process, so the work must be re-dispatched if it is still wanted"
                        .to_string(),
                );
                settled += 1;
            }
            rec
        })
        .collect();
    (recovered, settled)
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
///
/// MEASURED 2026-11-15 (third-pass audit, finding 9): this was written three
/// times in one surface — here, in `foreground_swarm`, and inline in
/// `App::swarm_gallery_members`, whose own comment apologised for the copy
/// ("`now_secs` is private to the registry module ... so read the clock here
/// rather than exporting a second time source"). All three bodies were
/// identical, and all three age the SAME records from the SAME `dispatch_time`
/// / `started_at`, so a clock that disagreed between them would make the hub's
/// ages inconsistent with the registry's. One export, three callers.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// RAII drain of the process-global delegation registry.
///
/// The registry has no clear in production by design (a running gateway's
/// delegations must survive), but a TEST that seeds it leaves records behind
/// for the rest of the process. This guard empties the registry when it drops,
/// which is what stops one test's seeded subagents from rendering into another
/// test's frame. See `__test_reset_registry` for the RT-207 incident.
///
/// Construction does NOT clear — it clears on DROP — so the seeding test still
/// sees its own records while it runs.
pub struct RegistryDrain;

impl Default for RegistryDrain {
    fn default() -> Self {
        Self
    }
}

impl RegistryDrain {
    pub fn new() -> Self {
        Self
    }
}

impl Drop for RegistryDrain {
    fn drop(&mut self) {
        __test_reset_registry();
    }
}

/// Serialize tests that touch the process-global registry.
///
/// Audit 2026-11-15 (third pass): this was private to the `tests` module, so a
/// test in ANOTHER module that seeds the registry had nothing to hold and raced
/// these — seeding rows there silently broke `prune_keeps_the_newest_by_
/// completion_order` and `records_list_is_sorted_newest_first`, which both count
/// rows absolutely. This lock is the join point; `RegistryDrain` is the cleanup.
pub fn test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Empty the process-global registry. TEST-ONLY on purpose.
///
/// The registry is `OnceLock<Mutex<HashMap>>` with no clear in production, and
/// tests that seed it leave records behind for the rest of the process. That is
/// not a hypothetical: RT-207 was a real red tip in `operant-cli` where
/// `gateway_commands::subagents_command_tests`
/// (`a_completed_delegation_shows_its_result_and_a_failed_one_its_error` seeds
/// two records via `mark_completed`/`mark_failed`) poisoned the TUI's
/// `highlight_covers_only_the_text_a_copy_carries` and
/// `live_tails_render_once_while_a_tool_is_reserved`, which both panic on
/// `Option::unwrap()` after failing to find their own text in a frame polluted
/// by the leaked subagent rows.
///
/// Mutation-proven: with the registry cleared before the reader, the minimal
/// repro (that one seeding test + the reader) goes GREEN, and the same pair is
/// RED without the clear.
///
/// `#[cfg(test)]` is NOT enough here — the polluting tests live in the
/// `operant-cli` binary's test build, so this must be visible to a binary test
/// target. Gated instead by name and by there being no production caller.
#[doc(hidden)]
pub fn __test_reset_registry() {
    if let Ok(mut g) = registry().lock() {
        g.clear();
    }
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("dlg-{}-{n}", now_secs())
}

fn insert_pending(
    map: &mut HashMap<String, AsyncDelegationRecord>,
    goal: &str,
    model: &str,
    parent_session_id: Option<&str>,
    request_key: Option<&str>,
) -> String {
    let id = new_id();
    map.insert(
        id.clone(),
        AsyncDelegationRecord {
            delegation_id: id.clone(),
            status: AsyncDelegationStatus::Pending,
            goal: goal.to_string(),
            model: model.to_string(),
            dispatch_time: now_secs(),
            completed_time: None,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: parent_session_id.map(str::to_string),
            request_key: request_key.map(str::to_string),
        },
    );
    id
}

/// Register a pending background delegation and return its handle id.
///
/// No parent edge: a caller that knows which session dispatched the work wants
/// [`try_create_record`], which carries it.
pub fn create_record(goal: &str, model: &str) -> String {
    if let Ok(mut map) = registry().lock() {
        insert_pending(&mut map, goal, model, None, None)
    } else {
        // Registry poisoned: fall back to an id that simply won't resolve.
        new_id()
    }
}

/// Atomically check the in-flight cap and register a new pending record
/// under ONE lock acquisition, so concurrent dispatches can't both pass a
/// check-then-act race (the count can never exceed `max_pending`).
/// Returns `Some(id)` when a slot was free, `None` when the cap is reached.
///
/// `parent_session_id` is the dispatching session — the edge the hub needs to
/// place a worker under its parent rather than among top-level sessions
/// (audit finding 2). `None` when the caller genuinely has no session.
pub fn try_create_record(
    goal: &str,
    model: &str,
    max_pending: usize,
    parent_session_id: Option<&str>,
    request_key: Option<&str>,
) -> Result<DelegationAdmission, TryCreateError> {
    let mut map = registry()
        .lock()
        .map_err(|_| TryCreateError::RegistryPoisoned)?;

    // Audit finding 6: a replayed dispatch returns the existing handle. Matched
    // on PENDING only — a completed record with the same key is finished work,
    // and re-running it is a legitimate new request, not a double-spend.
    // Checked under the same lock as the insert, so two concurrent dispatches
    // with one key cannot both miss and both create.
    if let Some(key) = request_key {
        let live = map.values().find(|r| {
            r.status == AsyncDelegationStatus::Pending && r.request_key.as_deref() == Some(key)
        });
        if let Some(existing) = live {
            return Ok(DelegationAdmission::Reused(existing.delegation_id.clone()));
        }
    }

    let pending = map
        .values()
        .filter(|r| r.status == AsyncDelegationStatus::Pending)
        .count();
    if pending >= max_pending {
        return Err(TryCreateError::TooManyPending { max_pending });
    }
    Ok(DelegationAdmission::Created(insert_pending(
        &mut map,
        goal,
        model,
        parent_session_id,
        request_key,
    )))
}

/// Why a background delegation was not admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TryCreateError {
    /// The in-flight cap is reached.
    TooManyPending { max_pending: usize },
    /// The registry lock was poisoned; no id can be trusted.
    RegistryPoisoned,
}

/// Join a delegation to the session its child ran in.
///
/// Written before the terminal transition, and separately from it, because the
/// two facts arrive from different places: the id is on the child's answer, the
/// outcome is the task's. Idempotent, and a no-op for an unknown id.
pub fn mark_session(id: &str, session_id: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.session_id = Some(session_id.to_string());
    }
    // NOT terminal, so a plain persist is right here: a terminal transition always
    // follows for a delegation that finishes, and that write re-reads the registry,
    // so the session id rides along on it. Best-effort is correct for this one.
    persist();
}

/// Transition a record to completed and stash the child's final answer.
pub fn mark_completed(id: &str, result: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.status = AsyncDelegationStatus::Completed;
        rec.completed_time = Some(now_secs());
        rec.result = Some(result.to_string());
    }
    prune();
    // TERMINAL: this is the last write the record receives, so a persist that
    // could not take the lock must not leave the file saying `Pending` — a
    // restart would then report this successful delegation as a failure (F4).
    persist_terminal();
}

/// Transition a record to failed and stash the error text.
pub fn mark_failed(id: &str, error: &str) {
    if let Ok(mut map) = registry().lock()
        && let Some(rec) = map.get_mut(id)
    {
        rec.status = AsyncDelegationStatus::Failed;
        rec.completed_time = Some(now_secs());
        rec.error = Some(error.to_string());
    }
    prune();
    // TERMINAL, for the same reason as `mark_completed`.
    persist_terminal();
}

/// Run a blocking delegation-ledger transition off the async runtime.
///
/// (iter-2457, F3) The three `mark_*` functions are synchronous because TUI code
/// and tests call them from non-async contexts, but the background-completion path
/// calls them from inside `tokio::spawn`. Each one takes a `flock` with no timeout
/// and does a full read-merge-write of a machine-wide JSON file, so calling them
/// inline from that task blocks a runtime worker for the length of the critical
/// section — and the lock is contended routinely because the TUI and the gateway
/// share one ledger. Use this from async callers; sync callers keep calling the
/// `mark_*` functions directly.
pub async fn mark_completed_off_runtime(id: &str, result: &str) {
    let id = id.to_string();
    let result = result.to_string();
    let _ = tokio::task::spawn_blocking(move || mark_completed(&id, &result)).await;
}

/// The `mark_failed` counterpart of [`mark_completed_off_runtime`].
pub async fn mark_failed_off_runtime(id: &str, error: &str) {
    let id = id.to_string();
    let error = error.to_string();
    let _ = tokio::task::spawn_blocking(move || mark_failed(&id, &error)).await;
}

/// The `mark_session` counterpart of [`mark_completed_off_runtime`].
pub async fn mark_session_off_runtime(id: &str, session_id: &str) {
    let id = id.to_string();
    let session_id = session_id.to_string();
    let _ = tokio::task::spawn_blocking(move || mark_session(&id, &session_id)).await;
}

/// The handle of the pending delegation already carrying `request_key`.
///
/// Audit 2026-11-15 (third pass, finding 1): `dispatch_background` peeked at the
/// breadth budget BEFORE it asked the registry about the key, so a replay was
/// refused whenever the process was full — and the caller, told to "retry", then
/// re-dispatched under a NEW key, which is the double-spend the key exists to
/// prevent. A replay needs no slot, so it must not be gated on one. This is the
/// read half of the dedup `try_create_record` performs atomically; the caller
/// consults it first, and `try_create_record` remains the authority (two
/// concurrent dispatches with one key still cannot both create).
pub fn pending_for_request_key(request_key: &str) -> Option<String> {
    registry()
        .lock()
        .ok()?
        .values()
        .find(|r| {
            r.status == AsyncDelegationStatus::Pending
                && r.request_key.as_deref() == Some(request_key)
        })
        .map(|r| r.delegation_id.clone())
}

/// Fetch a record clone (for `delegate_task(query="<id>")` polls).
pub fn get_record(id: &str) -> Option<AsyncDelegationRecord> {
    registry().lock().ok()?.get(id).cloned()
}

/// Records newest dispatch first, optionally only the pending ones.
///
/// One implementation, not two. MEASURED 2026-11-15 (third-pass audit, finding
/// 10): `list_records` and `live_records` had byte-identical bodies except for
/// a single `.filter()`, and the 4-line `sort_by` was copied verbatim — so a
/// change to the ordering had to be made twice and would silently diverge if it
/// was made once.
fn records_filtered(live_only: bool) -> Vec<AsyncDelegationRecord> {
    let mut records: Vec<AsyncDelegationRecord> = registry()
        .lock()
        .map(|map| {
            map.values()
                .filter(|r| !live_only || r.status == AsyncDelegationStatus::Pending)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    records.sort_by(|a, b| {
        b.dispatch_time
            .cmp(&a.dispatch_time)
            .then_with(|| b.delegation_id.cmp(&a.delegation_id))
    });
    records
}

/// All records, newest dispatch first.
pub fn list_records() -> Vec<AsyncDelegationRecord> {
    records_filtered(false)
}

/// Only the records still pending, newest dispatch first.
///
/// The per-frame callers (`App::swarm_gallery_members(live_only)`, the hub's
/// `Working` count) ask for live rows and used to clone EVERY record first —
/// including each finished child's full answer — then throw the finished ones
/// away (audit finding 13). Filtering before the clone is the whole fix.
pub fn live_records() -> Vec<AsyncDelegationRecord> {
    records_filtered(true)
}

/// The registry as `SessionEntryV2` subagent rows — the unified listing's
/// join point for live delegations (plan 2026-10-28 iter-C). Pending rows
/// first (the ones an operator would act on), then terminal records within
/// the registry's own retention window.
pub fn records_snapshot() -> Vec<crate::session::listing::SessionEntryV2> {
    use crate::session::listing::{RowKind, RowStatus, SessionEntryV2};

    list_records()
        .into_iter()
        .map(|r| {
            let (status, updated_at) = match r.status {
                AsyncDelegationStatus::Pending => (
                    "pending",
                    chrono::DateTime::from_timestamp(r.dispatch_time as i64, 0)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now),
                ),
                AsyncDelegationStatus::Completed => (
                    "completed",
                    chrono::DateTime::from_timestamp(
                        r.completed_time.unwrap_or(r.dispatch_time) as i64,
                        0,
                    )
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                ),
                AsyncDelegationStatus::Failed => (
                    "failed",
                    chrono::DateTime::from_timestamp(
                        r.completed_time.unwrap_or(r.dispatch_time) as i64,
                        0,
                    )
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now),
                ),
            };
            SessionEntryV2 {
                id: r.delegation_id,
                // Audit finding 2: the parent edge. This was `None` for every
                // row, so a subagent could not be placed under the session that
                // spawned it and the tree could not be rebuilt.
                parent_id: r.parent_session_id,
                title: None,
                updated_at,
                // A delegation's runtime is dispatch -> completion. The registry
                // is process-local and prunes with its worker, so these are the
                // live values rather than persisted ones.
                started_at: Some(
                    chrono::DateTime::from_timestamp(r.dispatch_time as i64, 0)
                        .unwrap_or_else(chrono::Utc::now),
                ),
                ended_at: r.completed_time.map(|t| {
                    chrono::DateTime::from_timestamp(t as i64, 0).unwrap_or_else(chrono::Utc::now)
                }),
                employee_id: None,
                source: "subagent".to_string(),
                message_count: 0,
                cost_usd: 0.0,
                tokens: 0,
                cwd: None,
                // Same-process presence: a pending delegation IS live work.
                status: if r.status == AsyncDelegationStatus::Pending {
                    RowStatus::Live
                } else {
                    RowStatus::Idle
                },
                kind: RowKind::Subagent {
                    goal: r.goal,
                    model: r.model,
                    dispatch_time: r.dispatch_time,
                    status,
                },
            }
        })
        .collect()
}

/// Drop the oldest terminal records beyond the retention cap so a long-lived
/// process doesn't accumulate unbounded delegation history (hermes
/// `_MAX_RETAINED_COMPLETED = 50` parity).
///
/// Ordered by `completed_time`, with the id as tiebreak. `dispatch_time` has
/// one-second resolution and several delegations routinely finish inside the same
/// second, so a dispatch-time sort made the SURVIVOR of a prune nondeterministic —
/// which handle stayed resolvable depended on HashMap iteration order. MEASURED
/// 2026-11-14.
fn prune() {
    if let Ok(mut map) = registry().lock() {
        let terminal: Vec<String> = map
            .iter()
            .filter(|(_, r)| r.status != AsyncDelegationStatus::Pending)
            .map(|(id, _)| id.clone())
            .collect();
        if terminal.len() > MAX_RETAINED_TERMINAL {
            // The rule lives in `enforce_terminal_cap`; this only adapts it to a
            // map. The copy that used to sit here is the one iter-2406 fixed in
            // `merge_records` and NOT here, which is how a duplicated rule drifts.
            let mut rows: Vec<AsyncDelegationRecord> = terminal
                .iter()
                .filter_map(|id| map.get(id).cloned())
                .collect();
            enforce_terminal_cap(&mut rows);
            let survivors: std::collections::HashSet<String> =
                rows.into_iter().map(|r| r.delegation_id).collect();
            // Pending rows were never candidates to cut, so they must survive the
            // retain explicitly — `survivors` holds terminal rows only, and a
            // bare `retain` against it would drop every live delegation.
            map.retain(|id, r| {
                r.status == AsyncDelegationStatus::Pending || survivors.contains(id)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share the process-global registry and run in parallel — any
    /// absolute-count assertion would race. Serialize them with a lock.
    ///
    /// One lock for the whole crate, not one per module: a test in another
    /// module that seeds the registry must hold the SAME lock or the two race
    /// (audit 2026-11-15). `super::test_lock` is that lock.
    use super::test_lock;

    #[test]
    fn create_then_complete_round_trip() {
        let _guard = test_lock().lock().unwrap();
        let id = create_record("analyze module", "gpt-4.1");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Pending);
        assert_eq!(rec.goal, "analyze module");
        assert!(rec.result.is_none());

        mark_completed(&id, "{\"summary\":\"done\"}");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Completed);
        assert_eq!(rec.result.as_deref(), Some("{\"summary\":\"done\"}"));
        assert!(rec.completed_time.is_some());
    }

    #[test]
    fn create_then_fail_records_error() {
        let _guard = test_lock().lock().unwrap();
        let id = create_record("risky task", "gpt-4.1");
        mark_failed(&id, "child timed out");
        let rec = get_record(&id).unwrap();
        assert_eq!(rec.status, AsyncDelegationStatus::Failed);
        assert_eq!(rec.error.as_deref(), Some("child timed out"));
        assert!(rec.result.is_none());
    }

    #[test]
    fn get_missing_record_returns_none() {
        let _guard = test_lock().lock().unwrap();
        assert!(get_record("dlg-does-not-exist").is_none());
    }

    /// Pruning must keep exactly the newest records by the pruner's own key.
    ///
    /// The key is `(completed_time, id)` — a total order, so the result is
    /// deterministic by construction (MEASURED 2026-11-14: the old sort keyed on
    /// `dispatch_time` alone, and `completed_time` was not consulted at all, so
    /// the survivor of a same-second prune depended on HashMap iteration order).
    #[test]
    fn prune_keeps_the_newest_by_completion_order() {
        let _guard = test_lock().lock().unwrap();
        // Start from an empty registry: records left by other tests would take
        // slots under the cap and change which of OURS survive.
        __test_reset_registry();
        let mut all: Vec<(u64, String)> = Vec::new();
        for i in 0..MAX_RETAINED_TERMINAL + 5 {
            let id = create_record(&format!("job {i}"), "m");
            mark_completed(&id, "ok");
            let finished = get_record(&id).and_then(|r| r.completed_time).unwrap_or(0);
            all.push((finished, id));
        }
        all.sort();

        let mut expected: Vec<String> = all
            .iter()
            .skip(all.len() - MAX_RETAINED_TERMINAL)
            .map(|(_, id)| id.clone())
            .collect();
        expected.sort();
        let mut survivors: Vec<String> = list_records()
            .into_iter()
            .map(|r| r.delegation_id)
            .collect();
        survivors.sort();
        assert_eq!(
            survivors, expected,
            "the newest {MAX_RETAINED_TERMINAL} must survive"
        );
        __test_reset_registry();
    }

    #[test]
    fn records_list_is_sorted_newest_first() {
        let _guard = test_lock().lock().unwrap();
        let first = create_record("first", "m");
        let second = create_record("second", "m");
        let list = list_records();
        // Records created in the same second share dispatch_time; assert the
        // ordering invariant (newest first) + membership, not a strict pair
        // order that would be flaky on same-second creation.
        let times: Vec<u64> = list.iter().map(|r| r.dispatch_time).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]));
        assert!(list.iter().any(|r| r.delegation_id == first));
        assert!(list.iter().any(|r| r.delegation_id == second));
        // cleanup so other tests' count assertions stay isolated
        mark_completed(&first, "done");
        mark_completed(&second, "done");
    }

    /// The two listers are one implementation with a filter, so they must agree
    /// on membership and ordering.
    ///
    /// MEASURED 2026-11-15 (third-pass audit, finding 10): `list_records` and
    /// `live_records` were byte-identical but for one `.filter()`, with the sort
    /// copied verbatim. A change to the ordering made once would have silently
    /// diverged the two. This pins the shared property: live rows are a subset of
    /// all rows, in the same relative order.
    #[test]
    fn the_live_list_is_the_all_list_filtered_not_a_second_ordering() {
        let _guard = test_lock().lock().unwrap();
        let _drain = RegistryDrain::new();
        // Scope every assertion to THIS test's records. Not every test in the
        // module drains the registry, so a leaked row from another test would
        // break a bare `assert_eq!(all.len(), 4)` — which is exactly what
        // happened when this ran alongside the rest of the suite.
        let a = create_record("live-a", "m");
        let b = create_record("live-b", "m");
        let c = create_record("live-c", "m");
        let d = create_record("live-d", "m");
        let mine: std::collections::HashSet<String> =
            [&a, &b, &c, &d].into_iter().cloned().collect();

        // Force DISTINCT dispatch times. `create_record` stamps `now_secs()`, so
        // three records created in the same second share one timestamp and a
        // reversed sort is indistinguishable from the real one — which is exactly
        // how a divergent copy would hide.
        if let Ok(mut map) = registry().lock() {
            if let Some(r) = map.get_mut(&a) {
                r.dispatch_time = 100;
            }
            if let Some(r) = map.get_mut(&b) {
                r.dispatch_time = 200;
            }
            if let Some(r) = map.get_mut(&c) {
                r.dispatch_time = 300;
            }
            if let Some(r) = map.get_mut(&d) {
                r.dispatch_time = 400;
            }
        }

        // Two finish; TWO stay pending, so the live list has an observable
        // order rather than a single row that any sort would leave alone.
        mark_completed(&a, "done");
        mark_failed(&b, "boom");

        let all: Vec<AsyncDelegationRecord> = list_records()
            .into_iter()
            .filter(|r| mine.contains(&r.delegation_id))
            .collect();
        let live: Vec<AsyncDelegationRecord> = live_records()
            .into_iter()
            .filter(|r| mine.contains(&r.delegation_id))
            .collect();

        assert_eq!(all.len(), 4, "all four of THIS test's records: {all:?}");
        assert_eq!(live.len(), 2, "the two pending ones: {live:?}");
        assert_eq!(
            live[0].delegation_id, d,
            "newest first: d (400) before c (300): {live:?}"
        );
        assert_eq!(live[1].delegation_id, c);

        // Relative order is preserved: filtering must not reorder.
        let order = |rows: &[AsyncDelegationRecord]| -> Vec<String> {
            rows.iter().map(|r| r.delegation_id.clone()).collect()
        };
        let all_ids = order(&all);
        let live_ids = order(&live);
        let expected: Vec<String> = all_ids
            .iter()
            .filter(|id| live_ids.contains(id))
            .cloned()
            .collect();
        assert_eq!(
            live_ids, expected,
            "the live list must be the all list filtered in place, not re-sorted"
        );

        __test_reset_registry();
    }

    /// Audit finding 3: a restart used to forget every delegation, and a record
    /// that was `Pending` when the process died then read as running forever.
    /// Audit finding 10: the row must name the session that ran it, and the
    /// terminal transition must not drop the join.
    #[test]
    fn a_delegation_records_the_session_its_child_ran_in() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let id = create_record("audit the pool", "m");
        assert!(
            get_record(&id).unwrap().session_id.is_none(),
            "a fresh record has no session yet"
        );
        mark_session(&id, "sess_42");
        assert_eq!(
            get_record(&id).unwrap().session_id.as_deref(),
            Some("sess_42")
        );
        mark_completed(&id, "done");
        assert_eq!(
            get_record(&id).unwrap().session_id.as_deref(),
            Some("sess_42"),
            "completing must not drop the join"
        );
        __test_reset_registry();
    }

    /// The field is additive: a ledger written before the join existed must
    /// still load, or the first start after an upgrade would silently discard
    /// every delegation the operator had.
    #[test]
    fn a_ledger_written_before_the_session_join_still_loads() {
        let old = r#"{"delegation_id":"d1","status":"completed","goal":"g","model":"m",
            "dispatch_time":1,"completed_time":2,"result":"r","error":null}"#;
        let rec: AsyncDelegationRecord =
            serde_json::from_str(old).expect("a pre-join ledger must load");
        assert!(rec.session_id.is_none());
        // Audit finding 2: the parent edge is additive too. A ledger written
        // before the edge existed must load, or the first start after this
        // upgrade would discard every delegation the operator had.
        assert!(rec.parent_session_id.is_none());
    }

    /// Audit finding 6: a replayed dispatch returns the existing handle, and
    /// the caller is told NOT to start a second child. This is the property
    /// that prevents the double-spend — a bare id would make the deduplication
    /// a silent no-op, because the caller would run the work again under it.
    #[test]
    fn a_replayed_request_key_returns_the_existing_delegation() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();

        let first =
            try_create_record("audit the pool", "m", 3, None, Some("job-42")).expect("a free slot");
        assert_eq!(
            first,
            DelegationAdmission::Created(first.id().to_string()),
            "the first dispatch creates"
        );

        // The replay: same key, same work, must NOT create a second record and
        // must NOT be handed back as something to run.
        let replay = try_create_record("audit the pool", "m", 3, None, Some("job-42"))
            .expect("a replay is admitted — as a reuse");
        assert_eq!(
            replay,
            DelegationAdmission::Reused(first.id().to_string()),
            "the replay must return the ORIGINAL id and be marked Reused"
        );
        assert_eq!(
            list_records().len(),
            1,
            "a replay must not add a ledger row: {:?}",
            list_records()
        );

        // A different key is different work.
        let other =
            try_create_record("audit the pool", "m", 3, None, Some("job-43")).expect("a free slot");
        assert!(matches!(other, DelegationAdmission::Created(_)));
        assert_ne!(other.id(), first.id());
        assert_eq!(list_records().len(), 2);

        // A FINISHED record with the same key does not block a new request: the
        // first one is done, and re-running finished work is legitimate.
        mark_completed(first.id(), "done");
        let again = try_create_record("audit the pool", "m", 3, None, Some("job-42"))
            .expect("a completed key is free to re-run");
        assert!(
            matches!(again, DelegationAdmission::Created(_)),
            "a completed record must not block a fresh dispatch: {again:?}"
        );
        __test_reset_registry();
    }

    /// The in-flight cap still applies, and a replay does not consume a slot.
    #[test]
    fn a_replay_is_admitted_even_when_the_cap_is_full() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let a = try_create_record("a", "m", 1, None, Some("k1")).expect("the only slot");
        assert!(matches!(a, DelegationAdmission::Created(_)));
        // A different key cannot get a slot.
        assert_eq!(
            try_create_record("b", "m", 1, None, Some("k2")),
            Err(TryCreateError::TooManyPending { max_pending: 1 })
        );
        // But replaying the admitted key does not need one: the work is already
        // in flight, so refusing the replay would push the caller to re-dispatch
        // under a new key and double-spend.
        assert_eq!(
            try_create_record("a", "m", 1, None, Some("k1")),
            Ok(DelegationAdmission::Reused(a.id().to_string()))
        );
        __test_reset_registry();
    }

    /// Audit finding 2: a delegation records which session dispatched it, and
    /// the session-hub row carries it as the parent edge.
    #[test]
    fn a_delegation_records_the_session_that_dispatched_it() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let admission = try_create_record("audit the pool", "m", 3, Some("sess_parent42"), None)
            .expect("a free slot");
        let id = admission.id().to_string();
        assert_eq!(
            get_record(&id).unwrap().parent_session_id.as_deref(),
            Some("sess_parent42"),
            "the record must name the dispatching session"
        );

        // The hub row is where the operator sees it: without this the row was
        // flat and a worker was indistinguishable from a top-level session.
        let row = records_snapshot()
            .into_iter()
            .find(|r| r.id == id)
            .expect("the delegation has a hub row");
        assert_eq!(
            row.parent_id.as_deref(),
            Some("sess_parent42"),
            "the hub row must carry the parent edge"
        );

        // A caller with no session still works, and says so rather than
        // inventing one.
        let orphan = create_record("no session", "m");
        assert!(get_record(&orphan).unwrap().parent_session_id.is_none());
        __test_reset_registry();
    }

    #[test]
    fn a_ledger_left_by_a_dead_process_settles_its_pending_records() {
        let _guard = test_lock().lock().unwrap();
        let dir = std::env::temp_dir().join(format!("deleg-ledger-{}", std::process::id()));
        let path = dir.join("delegations.json");
        std::fs::create_dir_all(&dir).unwrap();
        let pending = AsyncDelegationRecord {
            delegation_id: "dlg-old-1".to_string(),
            status: AsyncDelegationStatus::Pending,
            goal: "work that died with the process".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: None,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: Some("dlg-parent-1".to_string()),
            request_key: None,
        };
        let done = AsyncDelegationRecord {
            delegation_id: "dlg-old-2".to_string(),
            status: AsyncDelegationStatus::Completed,
            goal: "work that finished".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: Some(110),
            result: Some("ok".to_string()),
            error: None,
            session_id: None,
            parent_session_id: Some("dlg-parent-1".to_string()),
            request_key: None,
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&[pending, done]).unwrap(),
        )
        .unwrap();

        // Audit finding 4: the call returns HOW MANY it settled, not just
        // whether it ran. One pending record in the ledger means one recovery.
        assert_eq!(
            enable_persistence(path.clone()),
            1,
            "exactly the one pending record must be reported as recovered"
        );

        let recovered = get_record("dlg-old-1").expect("the pending record is recovered");
        assert_eq!(recovered.status, AsyncDelegationStatus::Failed);
        assert!(
            recovered
                .error
                .as_deref()
                .is_some_and(|e| e.starts_with("the process restarted")),
            "the reason must name the restart: {:?}",
            recovered.error
        );
        assert!(
            recovered.completed_time.is_some(),
            "a settled row must stop claiming it is still running"
        );
        // A finished record is history, not a casualty: it survives untouched.
        assert_eq!(
            get_record("dlg-old-2").unwrap().status,
            AsyncDelegationStatus::Completed
        );

        // Durable, not just in memory: the ledger on disk now says Failed, so a
        // THIRD process would not resurrect the pending row.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("dlg-old-1"),
            "ledger was not rewritten: {text}"
        );
        assert!(
            !text.contains("\"status\": \"pending\""),
            "the settled ledger must not still claim pending"
        );
        // The process-global path stays set for the rest of the test binary;
        // deleting the directory under it would only make later best-effort
        // writes fail, so the per-pid temp dir is left for the OS to reap.
    }

    /// A row an EARLIER restart already settled must not be counted again.
    ///
    /// MEASURED 2026-11-15 (third-pass audit, finding 2): the count was read
    /// back out of the registry by matching the error text
    /// `starts_with("the process restarted")`, and the ledger is reloaded whole
    /// on every start — so a previously-settled row still matched and was
    /// recounted. Proven by execution before the fix: a ledger holding one
    /// already-settled row plus one still pending reported **2**, where the
    /// truth is 1.
    ///
    /// Drives `settle_interrupted` rather than `enable_persistence`:
    /// `LEDGER_PATH` is a `OnceLock`, so only the first test in the process can
    /// set it, and a test that drove the real path would race whichever of these
    /// got there first.
    ///
    /// MUTATION PROVEN: counting by error-text prefix over the settled set makes
    /// this fail with `left: 2, right: 1`.
    #[test]
    fn a_row_settled_by_an_earlier_restart_is_not_counted_again() {
        // Settled by a PREVIOUS process's restart: it already carries the
        // restart wording, which is exactly what the old counter matched on.
        let settled_before = AsyncDelegationRecord {
            delegation_id: "dlg-prev-1".to_string(),
            status: AsyncDelegationStatus::Failed,
            goal: "settled by an earlier restart".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: Some(110),
            result: None,
            error: Some(
                "the process restarted while this delegation was running; its child died with \
                 that process, so the work must be re-dispatched if it is still wanted"
                    .to_string(),
            ),
            session_id: None,
            parent_session_id: None,
            request_key: None,
        };
        let still_pending = AsyncDelegationRecord {
            delegation_id: "dlg-now-1".to_string(),
            status: AsyncDelegationStatus::Pending,
            goal: "left running by the last process".to_string(),
            model: "m".to_string(),
            dispatch_time: 100,
            completed_time: None,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: None,
            request_key: None,
        };

        let (recovered, settled) = settle_interrupted(vec![settled_before, still_pending]);

        // ONE row is newly settled by this call. The other was settled before
        // this process existed, so it is history, not a casualty.
        assert_eq!(
            settled, 1,
            "only the pending row is recovered by THIS start"
        );
        assert_eq!(recovered.len(), 2, "both rows still load");
        assert_eq!(
            recovered[0].status,
            AsyncDelegationStatus::Failed,
            "the previously-settled row must stay settled"
        );
        assert_eq!(
            recovered[1].delegation_id, "dlg-now-1",
            "the pending row is the one that transitions"
        );
    }
}

// ── the durable ledger merge (session-hub plan T5, 2026-11-15) ──────────────
//
// Two guarantees, one test each: a record written by ANOTHER process survives
// this process's persist, and a record written by THIS process carries its
// dispatching session's parent edge into the file so the next process can list
// that session's children from the file alone.
#[cfg(test)]
mod ledger_merge_tests {
    use super::*;

    fn record(
        id: &str,
        status: AsyncDelegationStatus,
        goal: &str,
        dispatch: u64,
        completed: Option<u64>,
        parent: Option<&str>,
    ) -> AsyncDelegationRecord {
        AsyncDelegationRecord {
            delegation_id: id.to_string(),
            status,
            goal: goal.to_string(),
            model: "m".to_string(),
            dispatch_time: dispatch,
            completed_time: completed,
            result: None,
            error: None,
            session_id: None,
            parent_session_id: parent.map(str::to_string),
            request_key: None,
        }
    }

    /// Retention bounds TERMINAL rows only: a live delegation is not history,
    /// and dropping one loses work the operator is still waiting on.
    ///
    /// `prune()` (the in-memory path) filters to non-Pending before counting;
    /// `merge_records` (the persist path) truncated the WHOLE merged set, so a
    /// ledger of 51 live delegations lost the oldest on the persist that should
    /// have recorded it. The two paths disagreeing was the defect: liveness is
    /// what decides whether a row may be cut.
    ///
    /// The terminal set must EXCEED the cap, or the branch under test never
    /// runs: a first draft used exactly `MAX_RETAINED_TERMINAL` rows, and with
    /// the whole retention block deleted from the source this test still passed.
    /// A guard that passes with the fix removed is not a guard.
    /// MEASURED 2026-11-15 (eighth-pass audit, finding 1).
    #[test]
    fn a_pending_record_is_never_cut_by_the_merge_retention_cap() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        // MORE than a cap-ful of terminal rows, all NEWER than the pending one,
        // so the retention branch is entered and the pending row is the oldest
        // thing in the set by the truncate key.
        let prior: Vec<AsyncDelegationRecord> = (0..(MAX_RETAINED_TERMINAL as u64 + 5))
            .map(|i| {
                record(
                    &format!("term-{i:03}"),
                    AsyncDelegationStatus::Completed,
                    "finished work",
                    1000 + i,
                    Some(1000 + i),
                    None,
                )
            })
            .collect();
        // This process is still running one delegation. It has no
        // completed_time, so the truncate key falls back to its dispatch_time.
        let live = record(
            "live-pending",
            AsyncDelegationStatus::Pending,
            "still running",
            900,
            None,
            None,
        );
        let merged = merge_records(prior, vec![live]);
        // Drop the guard BEFORE asserting: an assert that panics while holding
        // the process-global lock poisons it, and every later test in the
        // module then fails with PoisonError instead of its own result.
        drop(_guard);
        assert!(
            merged.iter().any(|r| r.delegation_id == "live-pending"),
            "a Pending record must survive the merge cap: it is not terminal history"
        );
        assert_eq!(
            merged
                .iter()
                .filter(|r| r.status != AsyncDelegationStatus::Pending)
                .count(),
            MAX_RETAINED_TERMINAL,
            "the terminal rows were still capped, so the branch really ran"
        );
    }

    /// Pending rows do not count against a cap that bounds TERMINAL rows, even
    /// when the terminal rows are over the cap and the branch is running.
    ///
    /// The first draft of this test used an all-Pending ledger, which cannot
    /// fail: the branch is gated on `terminal.len()`, and an all-Pending set has
    /// `terminal.len() == 0`, so nothing is ever cut and the assertion holds for
    /// any implementation — including one with no retention at all. The shape
    /// that can fail is a ledger where the terminal rows DO exceed the cap and
    /// the Pending rows are numerous: an implementation that counts the whole set
    /// (the iter-2406 defect) truncates the live rows away.
    /// MEASURED 2026-11-15 (eighth-pass audit, finding 1).
    #[test]
    fn pending_rows_do_not_count_against_the_terminal_cap() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        // Terminal rows over the cap, so the retention branch is entered.
        let mut prior: Vec<AsyncDelegationRecord> = (0..(MAX_RETAINED_TERMINAL as u64 + 5))
            .map(|i| {
                record(
                    &format!("term-{i:03}"),
                    AsyncDelegationStatus::Completed,
                    "finished work",
                    2000 + i,
                    Some(2000 + i),
                    None,
                )
            })
            .collect();
        // Plus live rows, interleaved OLDER than the terminal ones, so an
        // implementation that counts the whole set cuts these first.
        for i in 0..5u64 {
            prior.push(record(
                &format!("live-{i:03}"),
                AsyncDelegationStatus::Pending,
                "running",
                1000 + i,
                None,
                None,
            ));
        }
        let merged = merge_records(prior, Vec::new());
        drop(_guard);
        let live: Vec<&str> = merged
            .iter()
            .filter(|r| r.status == AsyncDelegationStatus::Pending)
            .map(|r| r.delegation_id.as_str())
            .collect();
        assert_eq!(
            live.len(),
            5,
            "no live row may be cut, however far the terminal rows exceed the cap"
        );
        assert_eq!(
            merged
                .iter()
                .filter(|r| r.status != AsyncDelegationStatus::Pending)
                .count(),
            MAX_RETAINED_TERMINAL,
            "the terminal rows were still capped, so the branch really ran"
        );
    }

    /// The cap still cuts what it was built for: the OLDEST terminal rows go,
    /// newest survive, and the live rows are untouched.
    #[test]
    fn the_merge_cap_still_cuts_the_oldest_terminal_rows() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let mut prior: Vec<AsyncDelegationRecord> = (0..(MAX_RETAINED_TERMINAL as u64 + 5))
            .map(|i| {
                record(
                    &format!("term-{i:03}"),
                    AsyncDelegationStatus::Completed,
                    "finished work",
                    100 + i,
                    Some(100 + i),
                    None,
                )
            })
            .collect();
        // One live row that predates every terminal row: it must still survive.
        prior.push(record(
            "live-old",
            AsyncDelegationStatus::Pending,
            "still running",
            1,
            None,
            None,
        ));
        let merged = merge_records(prior, Vec::new());
        drop(_guard);
        let ids: Vec<&str> = merged.iter().map(|r| r.delegation_id.as_str()).collect();
        assert!(
            ids.contains(&"live-old"),
            "the live row must survive even though it is the oldest by the truncate key"
        );
        assert_eq!(
            merged
                .iter()
                .filter(|r| r.status != AsyncDelegationStatus::Pending)
                .count(),
            MAX_RETAINED_TERMINAL,
            "terminal rows are still capped"
        );
        assert!(
            !ids.contains(&"term-000"),
            "the oldest terminal row is the one the cap cuts"
        );
        assert!(
            ids.contains(&"term-005"),
            "the newest terminal rows survive the cap"
        );
    }

    /// Retention is a TOTAL order: rows finished in the same second are cut by
    /// id, on every path, not by the order the input happened to hold them.
    ///
    /// MEASURED 2026-11-15 (ninth-pass audit, finding 1): `recover_interrupted`
    /// sorted by `completed_time` ALONE while `merge_records` and `prune` both
    /// broke the tie on the id. So a restart that recovered two delegations
    /// finished in the same second kept whichever the file listed first, and the
    /// live path would have kept the other — the same ledger, two different
    /// survivors, depending on which code path did the cutting. One shared
    /// `enforce_terminal_cap` closes it.
    ///
    /// The fixture is cap+5 rows ALL sharing one timestamp, supplied in
    /// DESCENDING id order, so "keep input order" and "keep the highest id"
    /// disagree: a time-only sort is stable, so it would cut `cut-054` first,
    /// while the rule cuts `cut-000`.
    ///
    /// An earlier draft of this test supplied the rows ASCENDING and passed with
    /// the tiebreak deleted — a stable sort on equal keys preserves input order,
    /// so both rules cut the same row and the fixture could not tell them apart.
    /// A tiebreak guard has to make input order lie.
    #[test]
    fn the_cap_breaks_a_timestamp_tie_on_the_id_on_every_path() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let same_second = 4_000u64;
        let rows = MAX_RETAINED_TERMINAL as u64 + 5;
        // Descending: the highest id is handed over FIRST, so an implementation
        // that keeps input order keeps the wrong end of the range.
        let prior: Vec<AsyncDelegationRecord> = (0..rows)
            .rev()
            .map(|i| {
                record(
                    &format!("cut-{i:03}"),
                    AsyncDelegationStatus::Completed,
                    "finished in the same second",
                    same_second,
                    Some(same_second),
                    None,
                )
            })
            .collect();
        let merged = merge_records(prior, Vec::new());
        drop(_guard);
        assert_eq!(
            merged.len(),
            MAX_RETAINED_TERMINAL,
            "the cap still applies to a same-second set"
        );
        let ids: Vec<&str> = merged.iter().map(|r| r.delegation_id.as_str()).collect();
        assert!(
            ids.contains(&format!("cut-{:03}", rows - 1).as_str()),
            "the highest id survives a same-second tie"
        );
        assert!(
            !ids.contains(&"cut-000"),
            "the lowest id is cut, not whichever row the input listed first"
        );
    }

    #[test]
    fn a_second_processs_file_records_survive_this_processs_persist() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        // The file as a DEAD process left it: two completed delegations, one
        // under each of two dispatchers' sessions.
        let dead = vec![
            record(
                "dlg-dead-1",
                AsyncDelegationStatus::Completed,
                "work of the dead process",
                100,
                Some(110),
                Some("sess-parent-1"),
            ),
            record(
                "dlg-dead-2",
                AsyncDelegationStatus::Completed,
                "more of the dead process",
                101,
                Some(111),
                Some("sess-parent-2"),
            ),
        ];
        let merged = merge_records(dead.clone(), Vec::new());
        assert_eq!(
            merged.len(),
            2,
            "a persist with an empty registry must not discard the dead process's records"
        );
        assert!(
            merged
                .iter()
                .all(|r| dead.iter().any(|d| d.delegation_id == r.delegation_id)),
        );

        // This process's OWN record carries its parent edge into the same ledger.
        let mine = record(
            "dlg-live-1",
            AsyncDelegationStatus::Completed,
            "work of the live process",
            200,
            Some(201),
            Some("sess-parent-3"),
        );
        let merged = merge_records(dead, vec![mine.clone()]);
        assert_eq!(
            merged.len(),
            3,
            "merge keeps the file records and adds this process's own"
        );
        assert!(
            merged.iter().any(|r| r.delegation_id == "dlg-live-1"),
            "this process's record must reach the merged ledger"
        );
        assert!(
            merged.iter().any(|r| r.delegation_id == "dlg-dead-1"),
            "the dead process's record must survive the merge"
        );

        // A record the file already knows about is REPLACED by the newer state,
        // never duplicated.
        let mine_recompleted = record(
            "dlg-dead-1",
            AsyncDelegationStatus::Failed,
            "work of the dead process",
            100,
            Some(112),
            Some("sess-parent-1"),
        );
        let merged = merge_records(merged, vec![mine_recompleted]);
        assert_eq!(
            merged
                .iter()
                .filter(|r| r.delegation_id == "dlg-dead-1")
                .count(),
            1,
            "a record present in both sets is merged, not duplicated"
        );
        __test_reset_registry();
    }

    /// The second process reads the file alone and lists its session's children
    /// by the parent edge — the whole point of persisting `parent_session_id`.
    #[test]
    fn a_resumed_process_lists_a_sessions_children_from_the_file_alone() {
        let _guard = test_lock().lock().unwrap();
        __test_reset_registry();
        let ledger = vec![
            record(
                "dlg-resume-1",
                AsyncDelegationStatus::Completed,
                "child of the resumed session",
                100,
                Some(110),
                Some("sess-parent-1"),
            ),
            record(
                "dlg-resume-2",
                AsyncDelegationStatus::Failed,
                "another child of the same session",
                101,
                Some(111),
                Some("sess-parent-1"),
            ),
            record(
                "dlg-resume-3",
                AsyncDelegationStatus::Completed,
                "child of a DIFFERENT session",
                102,
                Some(112),
                Some("sess-parent-2"),
            ),
        ];
        // Persist = serde write; then a fresh process reads the file into its
        // own registry — exactly what `recover_interrupted` does through the
        // OnceLock, without the process-global path fight between tests.
        let json = serde_json::to_string_pretty(&ledger).unwrap();
        let reloaded: Vec<AsyncDelegationRecord> =
            serde_json::from_str(&json).expect("the file alone must carry the whole ledger");

        let children_of_parent_1: Vec<&AsyncDelegationRecord> = reloaded
            .iter()
            .filter(|r| r.parent_session_id.as_deref() == Some("sess-parent-1"))
            .collect();
        assert_eq!(
            children_of_parent_1.len(),
            2,
            "the resumed session's two children must be listable from the file alone"
        );
        assert!(
            !children_of_parent_1
                .iter()
                .any(|r| r.parent_session_id.as_deref() == Some("sess-parent-2")),
            "two dispatchers' records must not cross"
        );
        // The pending row stays pending on resume when it was a live record
        // whose child died with the process — recover_interrupted's contract,
        // pinned at the file's edge.
        let pending_on_disk = record(
            "dlg-resume-4",
            AsyncDelegationStatus::Pending,
            "work that died with the process",
            103,
            None,
            Some("sess-parent-1"),
        );
        let reloaded = serde_json::from_str::<Vec<AsyncDelegationRecord>>(
            &serde_json::to_string_pretty(&[pending_on_disk]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            reloaded[0].status,
            AsyncDelegationStatus::Pending,
            "a Pending row is not settled by a plain read — that is what the \
             interrupted-records path is for"
        );
        __test_reset_registry();
    }

    /// (audit finding) The persist is an atomic replace under a lock: the ledger
    /// parses after a write, and no torn `.tmp` sibling is left behind.
    #[test]
    fn a_persist_leaves_a_parseable_ledger_and_no_temp_file() {
        let _guard = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("operant-ledger-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("delegations.json");
        // A prior file so the merge has something to preserve.
        let prior = AsyncDelegationRecord {
            delegation_id: "dlg-file-only".to_string(),
            status: AsyncDelegationStatus::Completed,
            goal: "from an earlier process".to_string(),
            model: "m".to_string(),
            dispatch_time: now_secs(),
            completed_time: Some(now_secs()),
            result: Some("ok".to_string()),
            error: None,
            session_id: Some("sess-file".to_string()),
            parent_session_id: None,
            request_key: None,
        };
        std::fs::write(
            &path,
            serde_json::to_string_pretty(std::slice::from_ref(&prior)).unwrap(),
        )
        .unwrap();

        let id = create_record("a fresh delegation", "m");
        mark_completed(&id, "done");
        let merged = merge_records(vec![prior], list_records());
        write_ledger_atomic(&path, &merged);

        let text = std::fs::read_to_string(&path).expect("the ledger is readable");
        let records: Vec<AsyncDelegationRecord> =
            serde_json::from_str(&text).expect("the ledger must parse after a write");
        assert!(
            records.iter().any(|r| r.delegation_id == "dlg-file-only"),
            "the file-only record must survive the write"
        );
        assert!(records.iter().any(|r| r.delegation_id == id));
        assert!(
            !dir.join("delegations.tmp").exists(),
            "the atomic write must not leave its temp file behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// iter-2457 (F4): a TERMINAL transition must reach the file, or a restart
    /// reports success as failure.
    ///
    /// The defect: `persist()` returned `()` and swallowed every failure, including
    /// a persist that never landed. That is correct for a best-effort write and
    /// wrong for a TERMINAL one, because `mark_completed` is the last write a
    /// record ever receives. If it does not land, the file still says `Pending`,
    /// and on the next start `settle_interrupted` rewrites that row to `Failed`
    /// with "the process restarted while this delegation was running; its child
    /// died with that process" — a successful delegation reported as a failure, its
    /// result existing only in the memory of a dead process.
    ///
    /// SCOPE, stated so nobody reads this as more than it proves: the failure
    /// `LedgerLock::acquire` can actually return is an open/lock error on the
    /// `.lock` file (read-only or full filesystem, a home that vanished), NOT
    /// contention — `acquire`'s `flock` is blocking with no `LOCK_NB`, so a busy
    /// lock is waited out inside `acquire`. This test therefore pins the property
    /// that matters and that I can drive: the terminal status reaches the FILE and
    /// the recovery pass leaves it alone. MUTATION-CHECKED against retries 3->1:
    /// the test still passes, which is correct — the retry count is not what this
    /// pins, and I say so rather than implying the retry is tested.
    ///
    /// This drives the REAL terminal persist (`persist_terminal_at`, the retry
    /// path `mark_completed`/`mark_failed` use) against a ledger the test owns, then
    /// runs the REAL recovery pass over the file it left behind. `persist_terminal_at`
    /// takes the path explicitly because `LEDGER_PATH` is a `OnceLock`: only the
    /// first caller in a process can opt in, so a test that needed to win that race
    /// would pass alone and fail in a suite.
    #[test]
    fn a_terminal_transition_reaches_the_file_and_survives_a_restart() {
        let _guard = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        __test_reset_registry();
        let dir =
            std::env::temp_dir().join(format!("operant-ledger-terminal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("delegations.json");
        std::fs::write(&path, "[]").unwrap();

        for (terminal, payload, field) in [
            (
                AsyncDelegationStatus::Completed,
                "the real answer",
                "result",
            ),
            (AsyncDelegationStatus::Failed, "the child died", "error"),
        ] {
            // `create_record` mints its own id; use the one it returns rather than
            // assuming a shape, so the test cannot pass by inventing a row.
            let id = create_record("work that finished", "m");
            // Drive the record to its terminal state through the same setter the
            // production path uses, so the test cannot pass by writing the field
            // the assertion reads.
            match terminal {
                AsyncDelegationStatus::Completed => mark_completed(&id, payload),
                _ => mark_failed(&id, payload),
            }
            assert!(
                persist_terminal_at(&path),
                "the terminal persist must report that it wrote"
            );

            let on_disk: Vec<AsyncDelegationRecord> =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let row = on_disk
                .iter()
                .find(|r| r.delegation_id == id)
                .unwrap_or_else(|| panic!("{id} must be in the ledger file"));
            assert_eq!(
                row.status, terminal,
                "{id}: the FILE must carry the terminal status, or a restart will                  rewrite it and report a false failure (field under test: {field})"
            );
            let stored = if field == "result" {
                row.result.as_deref()
            } else {
                row.error.as_deref()
            };
            assert_eq!(stored, Some(payload), "{id}: the payload must be persisted");

            // And the recovery pass must leave a terminal row alone.
            let (rows, settled) = settle_interrupted(on_disk);
            assert_eq!(settled, 0, "{id}: a terminal record must not be settled");
            assert!(rows.iter().any(|r| r.delegation_id == id));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

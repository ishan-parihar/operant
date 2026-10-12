//! The per-channel dispatch breaker (hermes `services/dispatch_breaker.py`
//! #526 parity, v6 outline §2 P1-A).
//!
//! The Telegram saga this exists for: a dead token (401) followed by the
//! gateway re-issuing the same send on every cycle — 401, 400, 503, 404, in a
//! loop, for hours, hammering a platform that has already told us to stop.
//! The breaker makes the second failure consequential: after
//! [`OPEN_THRESHOLD`] consecutive failures for one (channel, class) pair the
//! breaker **opens** and every further send to that channel in that class is
//! fast-failed at the seam — the adapter is never called, the platform never
//! sees the request, and the reason is logged once per open, not once per
//! cycle.
//!
//! Design points carried from the blueprint:
//!
//! - **Separate namespaces per concern** — transport-unreachable, auth-dead,
//!   and wedged (everything else) keep separate counters. A flaky network
//!   must not open the breaker for an auth-dead token, and an auth death
//!   must not be explained as "the network is down".
//! - **Capped exponential cooldown** — `base * 2^n` clamped to
//!   [`COOLDOWN_CAP`]; the first open waits [`COOLDOWN_BASE`] before the
//!   half-open probe, a repeatedly-failing channel backs off to the cap and
//!   stays there.
//! - **One probe in flight** — the half-open open window allows exactly one
//!   send through; the claim is atomic (`UPDATE … WHERE state` in SQLite), so
//!   concurrent senders on the same channel cannot race each other into N
//!   probes. When the probe succeeds the breaker closes; when it fails the
//!   breaker re-opens with the next cooldown.
//! - **Fail-open on infra down** — every method returns `Result`; a store
//!   error at the seam returns `Allow` (log + send) rather than an outage of
//!   our own taking message delivery down with it. The breaker is a guard,
//!   not a new single point of failure.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Error;

/// Consecutive failures for one (channel, class) key before the breaker
/// opens. Parity with hermes' consecutive-failure trip.
pub const OPEN_THRESHOLD: u32 = 3;

/// First cooldown after an open (seconds).
pub const COOLDOWN_BASE_SECS: u64 = 30;

/// Cooldowns never grow past this (blueprint's 300s clamp).
pub const COOLDOWN_CAP_SECS: u64 = 300;

/// How long a half-open probe owns its channel. A probe that reports
/// back (success → closed, failure → re-open) releases it immediately;
/// this deadline only matters when the sender dies mid-flight, so a
/// crashed probe cannot wedge the channel shut forever.
pub const PROBE_DEADLINE_SECS: u64 = 30;

/// Failure classes — the namespaces that never contaminate each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureClass {
    /// Network-layer unreachability: timeouts, refusals, DNS, resets.
    Transport,
    /// Credentials are dead: 401/403/unauthorized/forbidden/bad-token.
    /// The retry is pointless; the fix is operator action.
    Auth,
    /// Everything else: 4xx (bad request), 5xx, malformed replies.
    Wedged,
}

impl FailureClass {
    /// Stable storage key (the table's `class` column).
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureClass::Transport => "transport",
            FailureClass::Auth => "auth",
            FailureClass::Wedged => "wedged",
        }
    }

    /// Parse the stored key back. Unknown strings map to `Wedged` — an
    /// unrecognized row can only ever have been written by a future
    /// version, and treating it as the catch-all is the safe reading.
    pub fn from_str_key(s: &str) -> Self {
        match s {
            "transport" => FailureClass::Transport,
            "auth" => FailureClass::Auth,
            _ => FailureClass::Wedged,
        }
    }
}

/// The classifier's input: the platform adapter's error, already rendered.
/// Substring markers, matched case-insensitively, in the anti-marker order
/// hermes' `failure_classifier.py` uses — a death signal is never
/// misclassified as something operationally fixable.
pub fn classify_error(raw: &str) -> FailureClass {
    let lower = raw.to_ascii_lowercase();
    for marker in [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "invalid token",
        "token expired",
        "bad token",
        "chat not found",
        "bot was blocked",
    ] {
        if lower.contains(marker) {
            return FailureClass::Auth;
        }
    }
    for marker in [
        "timeout",
        "timed out",
        "connection refused",
        "connection reset",
        "network",
        "dns",
        "temporarily unavailable",
        "service unavailable",
        "503",
        "502",
        "429",
        "too many requests",
    ] {
        if lower.contains(marker) {
            return FailureClass::Transport;
        }
    }
    FailureClass::Wedged
}

/// What the breaker says about a send at the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchVerdict {
    /// Sends pass. (The common case — and the pre-probe state.)
    Allow,
    /// The breaker is open and the cooldown is still running: do not send.
    Reject,
    /// The breaker is open, the cooldown has elapsed, and THIS caller won
    /// the (atomic) race to be the single half-open probe: send, and the
    /// outcome re-closes or re-opens.
    Probe,
}

/// Breaker state for one (channel, class) key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// The breaker's state row, as the seam and tests see it.
#[derive(Debug, Clone)]
pub struct KeyState {
    pub state: BreakerState,
    /// Consecutive failures recorded for this key.
    pub failures: u32,
    /// Which cooldown tier this open is on (0 = first open).
    pub cooldown_tier: u32,
    /// Unix seconds after which an open key admits its probe.
    pub open_until: u64,
}

impl Default for KeyState {
    fn default() -> Self {
        Self {
            state: BreakerState::Closed,
            failures: 0,
            cooldown_tier: 0,
            open_until: 0,
        }
    }
}

/// Row projection written under a single SQL transaction.
struct Row {
    state: BreakerState,
    failures: u32,
    cooldown_tier: u32,
    open_until: u64,
}

/// The per-channel dispatch breaker: SQLite-backed so the state survives
/// restarts (a hot loop must not get a fresh breaker by bouncing the
/// gateway) and atomic under concurrent senders.
pub struct DispatchBreaker {
    conn: std::sync::Arc<std::sync::Mutex<Connection>>,
}

impl DispatchBreaker {
    /// Open (or create) the breaker store at `path`. Failures bubble as
    /// `Err` — callers fail-open at the seam rather than refusing to boot.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        let conn = Connection::open(&path).map_err(|e| Error::Agent(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS dispatch_breaker (
                 platform    TEXT NOT NULL,
                 channel_id  TEXT NOT NULL,
                 class       TEXT NOT NULL,
                 state       TEXT NOT NULL,
                 failures    INTEGER NOT NULL DEFAULT 0,
                 tier        INTEGER NOT NULL DEFAULT 0,
                 open_until  INTEGER NOT NULL DEFAULT 0,
                 probe_lock  INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (platform, channel_id, class)
             );",
        )
        .map_err(|e| Error::Agent(e.to_string()))?;
        Ok(Self {
            conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
        })
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Cooldown for a tier: base × 2^tier, capped.
    fn cooldown_for(tier: u32) -> u64 {
        let shifted = COOLDOWN_BASE_SECS
            .checked_shl(tier.min(20))
            .unwrap_or(COOLDOWN_CAP_SECS);
        shifted.clamp(COOLDOWN_BASE_SECS, COOLDOWN_CAP_SECS)
    }

    /// The seam's pre-send question: consult every class row for this
    /// (platform, channel). A closed/cool row never blocks. An open row
    /// whose cooldown has elapsed admits exactly one probe (atomic claim);
    /// an open row still cooling rejects. With more than one class open,
    /// the first claimable row wins the probe race; the rest reject.
    pub fn check(&self, platform: &str, channel_id: &str) -> DispatchVerdict {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return DispatchVerdict::Allow, // poison = fail-open
        };
        let now = Self::now();
        let mut stmt = match conn.prepare(
            "SELECT class, state, open_until, probe_lock \
             FROM dispatch_breaker WHERE platform = ?1 AND channel_id = ?2",
        ) {
            Ok(s) => s,
            Err(_) => return DispatchVerdict::Allow,
        };
        let rows: Vec<(String, String, u64, bool)> = stmt
            .query_map(params![platform, channel_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .and_then(|mapped| mapped.collect())
            .unwrap_or_default();

        let mut blocked = false;
        for (class_key, state, open_until, probe_lock) in &rows {
            match state.as_str() {
                "open" => {
                    blocked = true;
                    // Cooldown still running, or a probe is already
                    // claimed: this class keeps rejecting.
                    if now < *open_until || *probe_lock {
                        continue;
                    }
                    // Atomic claim: exactly one concurrent caller flips
                    // 0→1 and becomes the probe; the rest lose the race
                    // and fall through to Reject below.
                    let claimed = conn
                        .execute(
                            "UPDATE dispatch_breaker \
                             SET state = 'half_open', probe_lock = 1, open_until = ?4 \
                             WHERE platform = ?1 AND channel_id = ?2 AND class = ?3 \
                                   AND probe_lock = 0 AND state = 'open'",
                            params![platform, channel_id, class_key, now + PROBE_DEADLINE_SECS],
                        )
                        .unwrap_or(0);
                    if claimed == 1 {
                        return DispatchVerdict::Probe;
                    }
                }
                // A probe owns this channel: everyone else waits for its
                // outcome. Half-open is only recoverable by success
                // (closes) or failure (re-opens) — never by a parallel
                // send slipping past the breaker.
                "half_open" => {
                    blocked = true;
                    // A probe that never reported (the sender died
                    // mid-flight) would wedge the channel forever past
                    // its deadline. Past the deadline the lock is stale
                    // and this caller takes the probe over.
                    if now >= *open_until {
                        let taken = conn
                            .execute(
                                "UPDATE dispatch_breaker \
                                 SET probe_lock = 1, open_until = ?4 \
                                 WHERE platform = ?1 AND channel_id = ?2 AND class = ?3 \
                                       AND state = 'half_open' AND open_until <= ?5",
                                params![
                                    platform,
                                    channel_id,
                                    class_key,
                                    now + PROBE_DEADLINE_SECS,
                                    now
                                ],
                            )
                            .unwrap_or(0);
                        if taken == 1 {
                            tracing::info!(
                                platform,
                                channel_id,
                                "dispatch breaker probe deadline passed — taking the probe over"
                            );
                            return DispatchVerdict::Probe;
                        }
                    }
                }
                _ => {}
            }
        }
        if blocked {
            DispatchVerdict::Reject
        } else {
            DispatchVerdict::Allow
        }
    }

    fn write_state(
        &self,
        platform: &str,
        channel_id: &str,
        class: FailureClass,
        row: &Row,
        probe_lock: bool,
    ) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|_| Error::Agent("lock".into()))?;
        conn.execute(
            "INSERT INTO dispatch_breaker
                 (platform, channel_id, class, state, failures, tier, open_until, probe_lock)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (platform, channel_id, class) DO UPDATE SET
                 state = excluded.state,
                 failures = excluded.failures,
                 tier = excluded.tier,
                 open_until = excluded.open_until,
                 probe_lock = excluded.probe_lock",
            params![
                platform,
                channel_id,
                class.as_str(),
                state_str(row.state),
                row.failures,
                row.cooldown_tier,
                row.open_until,
                probe_lock,
            ],
        )
        .map_err(|e| Error::Agent(e.to_string()))?;
        Ok(())
    }

    /// Record a SUCCESSFUL send for a class: failures reset to zero, the
    /// state closes, the probe lock releases, the cooldown tier restarts.
    /// Called per send-success at the seam (all three classes reset — one
    /// healthy send is evidence the channel is alive; hermes'
    /// `record_success` behaves the same).
    pub fn record_success(&self, platform: &str, channel_id: &str) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|_| Error::Agent("lock".into()))?;
        conn.execute(
            "UPDATE dispatch_breaker
             SET state = 'closed', failures = 0, tier = 0, open_until = 0, probe_lock = 0
             WHERE platform = ?1 AND channel_id = ?2",
            params![platform, channel_id],
        )
        .map_err(|e| Error::Agent(e.to_string()))?;
        Ok(())
    }

    /// Record a FAILED send for one class: the failure counter walks, the
    /// breaker opens at the threshold, an open key re-opening (a failed
    /// probe) steps the cooldown tier.
    pub fn record_failure(
        &self,
        platform: &str,
        channel_id: &str,
        class: FailureClass,
        rendered_error: &str,
    ) -> Result<(), Error> {
        let conn = self.conn.lock().map_err(|_| Error::Agent("lock".into()))?;
        let current: Option<(String, u32, u32, u64)> = conn
            .query_row(
                "SELECT state, failures, tier, open_until FROM dispatch_breaker \
                 WHERE platform = ?1 AND channel_id = ?2 AND class = ?3",
                params![platform, channel_id, class.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| Error::Agent(e.to_string()))?;
        drop(conn);

        let (state, failures, tier, _open_until) = current.unwrap_or(("closed".into(), 0, 0, 0));
        let now = Self::now();

        // A failed probe (half_open) re-opens immediately at the next
        // cooldown tier — same treatment as a leaked failure on an open
        // row, but the probe's failure is the designed signal, so the log
        // says so.
        if state == "open" || state == "half_open" {
            let cooldown_tier = tier + 1;
            let re_row = Row {
                state: BreakerState::Open,
                failures: failures + 1,
                cooldown_tier,
                open_until: now + Self::cooldown_for(cooldown_tier),
            };
            let from_probe = state == "half_open";
            self.write_state(platform, channel_id, class, &re_row, false)?;
            if from_probe {
                tracing::warn!(
                    platform,
                    channel_id,
                    class = class.as_str(),
                    tier = cooldown_tier,
                    error = %rendered_error,
                    "dispatch breaker probe FAILED — re-opened, cooldown steps up"
                );
            } else {
                tracing::info!(
                    platform,
                    channel_id,
                    class = class.as_str(),
                    tier = cooldown_tier,
                    error = %rendered_error,
                    "dispatch breaker re-opened (failure while open) — backing off"
                );
            }
            return Ok(());
        }

        let mut next = Row {
            state: BreakerState::Closed,
            failures: failures + 1,
            cooldown_tier: tier,
            open_until: 0,
        };

        if next.failures >= OPEN_THRESHOLD {
            next.state = BreakerState::Open;
            next.cooldown_tier = 0;
            next.open_until = now + Self::cooldown_for(0);
            self.write_state(platform, channel_id, class, &next, false)?;
            tracing::warn!(
                platform,
                channel_id,
                class = class.as_str(),
                failures = next.failures,
                cooldown_secs = Self::cooldown_for(0),
                error = %rendered_error,
                "dispatch breaker OPEN for this channel+class — sends fast-fail until the probe"
            );
            return Ok(());
        }

        self.write_state(platform, channel_id, class, &next, false)
    }

    /// Test/introspection helper: the current state for one key.
    pub fn state_of(
        &self,
        platform: &str,
        channel_id: &str,
        class: FailureClass,
    ) -> Result<KeyState, Error> {
        let conn = self.conn.lock().map_err(|_| Error::Agent("lock".into()))?;
        let row: Option<(String, u32, u32, u64)> = conn
            .query_row(
                "SELECT state, failures, tier, open_until FROM dispatch_breaker \
                 WHERE platform = ?1 AND channel_id = ?2 AND class = ?3",
                params![platform, channel_id, class.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| Error::Agent(e.to_string()))?;
        Ok(row
            .map(|(s, f, t, u)| KeyState {
                state: match s.as_str() {
                    "open" => BreakerState::Open,
                    "half_open" => BreakerState::HalfOpen,
                    _ => BreakerState::Closed,
                },
                failures: f,
                cooldown_tier: t,
                open_until: u,
            })
            .unwrap_or_default())
    }
}

fn state_str(state: BreakerState) -> &'static str {
    match state {
        BreakerState::Closed => "closed",
        BreakerState::Open => "open",
        BreakerState::HalfOpen => "half_open",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "operant-dispatch-breaker-{}-{}.sqlite",
            tag,
            std::process::id()
        ))
    }

    fn breaker(tag: &str) -> DispatchBreaker {
        let path = temp_path(tag);
        let _ = std::fs::remove_file(&path);
        DispatchBreaker::init(path).expect("breaker store opens")
    }

    #[test]
    fn classifier_separates_auth_from_transport_from_wedged() {
        // The blueprints' anti-marker order: a death signal is never
        // misclassified as something operationally fixable.
        assert_eq!(classify_error("HTTP 401 Unauthorized"), FailureClass::Auth);
        assert_eq!(classify_error("403 forbidden"), FailureClass::Auth);
        assert_eq!(classify_error("401 unauthorized"), FailureClass::Auth);
        assert_eq!(
            classify_error("bad token"),
            FailureClass::Auth,
            "anti-marker: an auth marker wins before transport"
        );
        assert_eq!(
            classify_error("connection timeout after 30s"),
            FailureClass::Transport
        );
        assert_eq!(
            classify_error("connection refused"),
            FailureClass::Transport
        );
        assert_eq!(classify_error("HTTP 503"), FailureClass::Transport);
        assert_eq!(
            classify_error("rate limited (429)"),
            FailureClass::Transport
        );
        assert_eq!(classify_error("HTTP 400 bad request"), FailureClass::Wedged);
        assert_eq!(classify_error("malformed reply"), FailureClass::Wedged);
        assert_eq!(classify_error(""), FailureClass::Wedged);
    }

    #[test]
    fn cooldown_is_capped_exponential() {
        assert_eq!(DispatchBreaker::cooldown_for(0), 30);
        assert_eq!(DispatchBreaker::cooldown_for(1), 60);
        assert_eq!(DispatchBreaker::cooldown_for(2), 120);
        assert_eq!(DispatchBreaker::cooldown_for(3), 240);
        assert_eq!(
            DispatchBreaker::cooldown_for(4),
            COOLDOWN_CAP_SECS,
            "tier 4 would be 480 — clamped to the 300s cap"
        );
        assert_eq!(DispatchBreaker::cooldown_for(40), COOLDOWN_CAP_SECS);
    }

    #[test]
    fn three_consecutive_failures_open_the_breaker() {
        let b = breaker("open");
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Allow);

        for i in 0..OPEN_THRESHOLD - 1 {
            b.record_failure("telegram", "chat-1", FailureClass::Auth, "401 unauthorized")
                .expect("record");
            assert_eq!(
                b.check("telegram", "chat-1"),
                DispatchVerdict::Allow,
                "below the threshold the channel still sends (failure {i})"
            );
        }
        b.record_failure("telegram", "chat-1", FailureClass::Auth, "401 unauthorized")
            .expect("record");
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Reject,
            "the third consecutive failure opens the breaker"
        );
        let state = b
            .state_of("telegram", "chat-1", FailureClass::Auth)
            .unwrap();
        assert_eq!(state.state, BreakerState::Open);
        assert_eq!(state.failures, OPEN_THRESHOLD);
        assert_eq!(state.cooldown_tier, 0);
    }

    #[test]
    fn success_closes_a_hot_channel() {
        let b = breaker("close");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure(
                "telegram",
                "chat-1",
                FailureClass::Transport,
                "connection refused",
            )
            .expect("record");
        }
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Reject);
        b.record_success("telegram", "chat-1").expect("record");
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Allow,
            "one healthy send clears every class row for the channel"
        );
        assert_eq!(
            b.state_of("telegram", "chat-1", FailureClass::Transport)
                .unwrap()
                .failures,
            0
        );
    }

    #[test]
    fn classes_never_contaminate_each_other() {
        let b = breaker("classes");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-1", FailureClass::Auth, "401")
                .unwrap();
        }
        // The AUTH class is open…
        assert_eq!(
            b.state_of("telegram", "chat-1", FailureClass::Auth)
                .unwrap()
                .state,
            BreakerState::Open
        );
        // …but a WEDGED send on the same channel still passes: the
        // namespaces are separate counters by design (the channel-level
        // verdict is still Reject while ANY class is open).
        assert_eq!(
            b.state_of("telegram", "chat-1", FailureClass::Wedged)
                .unwrap()
                .state,
            BreakerState::Closed
        );
        // Reset the auth class via success, and a transport failure starts
        // its own fresh ladder — the auth history does not carry over.
        b.record_success("telegram", "chat-1").unwrap();
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Allow);
        b.record_failure("telegram", "chat-1", FailureClass::Transport, "503")
            .unwrap();
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Allow,
            "one transport failure after a success is far from the threshold"
        );
    }

    #[test]
    fn per_channel_isolation() {
        let b = breaker("channels");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-hot", FailureClass::Auth, "401")
                .unwrap();
        }
        assert_eq!(b.check("telegram", "chat-hot"), DispatchVerdict::Reject);
        assert_eq!(
            b.check("telegram", "chat-cool"),
            DispatchVerdict::Allow,
            "a hot channel does not block its siblings"
        );
        assert_eq!(
            b.check("discord", "chat-hot"),
            DispatchVerdict::Allow,
            "and the platform is part of the key"
        );
    }

    #[test]
    fn cooldown_elapsed_admits_exactly_one_probe() {
        let b = breaker("probe");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-1", FailureClass::Auth, "401")
                .unwrap();
        }
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Reject,
            "cooldown running"
        );

        // Force the cooldown window open (tests must not sleep 30s).
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Probe,
            "cooldown elapsed: the first caller becomes the single probe"
        );
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Reject,
            "the second caller is rejected — exactly one probe flies"
        );
    }

    #[test]
    fn failed_probe_reopens_with_stepped_cooldown() {
        let b = breaker("reopen");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-1", FailureClass::Auth, "401")
                .unwrap();
        }
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0, state = 'half_open' \
                 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        b.record_failure("telegram", "chat-1", FailureClass::Auth, "401 again")
            .unwrap();
        let state = b
            .state_of("telegram", "chat-1", FailureClass::Auth)
            .unwrap();
        assert_eq!(state.state, BreakerState::Open, "a failed probe re-opens");
        assert_eq!(state.cooldown_tier, 1, "the cooldown tier steps up");
        // And it is closed again to probing after the (tier-1) window.
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Probe);
    }

    #[test]
    fn stale_probe_deadline_lets_a_new_caller_take_over() {
        let b = breaker("stale");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-1", FailureClass::Auth, "401")
                .unwrap();
        }
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        // First caller takes the probe; the second is rejected while it
        // is in flight.
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Probe);
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Reject);
        // The probe never reported (sender died). Past the deadline the
        // lock is stale and a new caller probes again — a crashed probe
        // must not wedge the channel shut forever.
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            b.check("telegram", "chat-1"),
            DispatchVerdict::Probe,
            "the deadline passed: a caller takes the stale probe over"
        );
    }

    #[test]
    fn successful_probe_closes_and_resets() {
        let b = breaker("probe-ok");
        for _ in 0..OPEN_THRESHOLD {
            b.record_failure("telegram", "chat-1", FailureClass::Transport, "timeout")
                .unwrap();
        }
        {
            let conn = b.conn.lock().unwrap();
            conn.execute(
                "UPDATE dispatch_breaker SET open_until = 0 WHERE platform = 'telegram' AND channel_id = 'chat-1'",
                [],
            )
            .unwrap();
        }
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Probe);
        b.record_success("telegram", "chat-1").unwrap();
        let state = b
            .state_of("telegram", "chat-1", FailureClass::Transport)
            .unwrap();
        assert_eq!(state.state, BreakerState::Closed);
        assert_eq!(state.failures, 0);
        assert_eq!(state.cooldown_tier, 0);
        assert_eq!(b.check("telegram", "chat-1"), DispatchVerdict::Allow);
    }
}

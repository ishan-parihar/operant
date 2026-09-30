//! Notice board — domain types (`docs/WAVE1-DECISIONS.md` §3.3).
//!
//! The board is the inter-agent message substrate: a durable, append-mostly
//! store of notices that employees post to typed recipients and read back
//! from their own inbox. It lives in the **existing** `operant_kanban.db`
//! file — see [`crate::org::notice_db`] for the storage layer and the
//! no-second-store argument.
//!
//! ## Typed recipients (AD-013 selector set)
//!
//! Selectors are stored **verbatim** as a JSON array and resolved **at read
//! time**, so a notice posted to `dept:platform-infra` reaches an employee
//! onboarded an hour later. This is a deliberate divergence from the
//! organism's JSONL, where a `dept:` notice is a flat tag and the inbox
//! bridge has to query it separately (`AXE-AD-013` §5).
//!
//! ## Timestamps
//!
//! Every timestamp this module writes goes through [`rfc3339`], which pins a
//! fixed millisecond precision. GC and TTL compare `created_at` /
//! `ttl_expires_at` **lexicographically as strings**, so equal-width fields
//! are what make that comparison equal to a chronological one. `chrono`'s
//! default `AutoSi` precision varies (0/3/6/9 fractional digits) which would
//! make the ordering subtly input-dependent.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The SQLite file the notice board's table lives in.
///
/// Per §1.2 this is the sibling kanban DB, not a new file and not the
/// sessions `database.db`. It is a `const` rather than a function on purpose:
/// it exists so the "no second store" invariant is greppable.
pub const NOTICE_BOARD_DB_FILE: &str = "operant_kanban.db";

/// Raw-entry retention, matching the organism's `RAW_RETENTION_DAYS`
/// (`axe_lib.py:91`).
pub const RAW_RETENTION_DAYS: i64 = 14;

/// Weekly-summary retention, matching `SUMMARY_RETENTION_WEEKS`
/// (`axe_lib.py:92`).
pub const SUMMARY_RETENTION_WEEKS: i64 = 16;

/// Seconds in the summary bucketing window. Week starts are
/// `epoch / WEEK_SECONDS * WEEK_SECONDS` — unix epoch was a Thursday, but a
/// fixed 7-day bucket is all the organism's summaries rely on
/// (`week_start` / `week_end` pairs 604800 apart).
pub const WEEK_SECONDS: i64 = 7 * 86_400;

/// The tag that marks a GC-generated weekly summary row.
pub const WEEKLY_SUMMARY_TAG: &str = "weekly-summary";

/// The tag the organism's request → ack → result protocol uses for acks
/// (AD-013 §3-4). An ack notice carries this tag plus the parent's
/// `correlation_id`.
pub const ACK_TAG: &str = "ack";

/// Value substituted for a missing `from_dept` at read time. AD-013 §2
/// defaults legacy rows to `unknown`; storing `NULL` and `COALESCE`-ing here
/// reproduces that without a backfill.
pub const UNKNOWN_DEPT: &str = "unknown";

/// Format a timestamp the way every notice board write stores it.
///
/// Fixed millisecond precision keeps lexicographic string comparison of
/// `created_at` / `ttl_expires_at` equivalent to chronological comparison.
pub fn rfc3339(ts: chrono::DateTime<chrono::Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A typed recipient selector.
///
/// Stored verbatim as a string (`agent:<id>`, `dept:<slug>`, …) so adding a
/// fifth kind needs no schema change and no migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Recipient {
    /// One employee, by id.
    Agent(String),
    /// All active employees in a department. Resolved at read time.
    Dept(String),
    /// All members of a team. Reserved for Wave 3 — no membership resolver
    /// exists yet, so a `Team` notice currently reaches nobody.
    Team(String),
    /// Employees holding a capability. Resolved at read time.
    Role(String),
    /// Every reader. The default when a notice names no recipient.
    Broadcast,
}

impl Recipient {
    /// The canonical stored form. Round-trips through [`Recipient::parse`].
    pub fn selector(&self) -> String {
        match self {
            Self::Agent(id) => format!("agent:{id}"),
            Self::Dept(slug) => format!("dept:{slug}"),
            Self::Team(slug) => format!("team:{slug}"),
            Self::Role(cap) => format!("role:{cap}"),
            Self::Broadcast => BROADCAST_SELECTOR.to_string(),
        }
    }

    /// Normalise one raw recipient, following the organism's rules
    /// (`axe_lib.py:191-217`):
    ///
    /// - empty or `broadcast` → [`Recipient::Broadcast`]
    /// - a bare string with no `:` → `agent:<s>`
    /// - a string containing `:` → already typed, split at the first `:`
    ///
    /// Unlike the organism we reject an unknown kind instead of passing it
    /// through: a selector the matcher cannot resolve would silently deliver
    /// to nobody, and a typo should fail at post time where the author can
    /// still see it.
    pub fn parse(raw: &str) -> crate::error::Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(BROADCAST_SELECTOR) {
            return Ok(Self::Broadcast);
        }
        match trimmed.split_once(':') {
            None => Ok(Self::Agent(trimmed.to_string())),
            Some((kind, value)) => {
                if value.is_empty() {
                    return Err(crate::error::Error::Agent(format!(
                        "notice: recipient selector '{raw}' has an empty value"
                    )));
                }
                match kind {
                    "agent" => Ok(Self::Agent(value.to_string())),
                    "dept" => Ok(Self::Dept(value.to_string())),
                    "team" => Ok(Self::Team(value.to_string())),
                    "role" => Ok(Self::Role(value.to_string())),
                    other => Err(crate::error::Error::Agent(format!(
                        "notice: unknown recipient kind '{other}' in selector '{raw}' \
                         (expected agent|dept|team|role|broadcast)"
                    ))),
                }
            }
        }
    }

    /// Normalise a slice of raw recipients, preserving order.
    pub fn parse_all<'a, I: IntoIterator<Item = &'a str>>(
        raw: I,
    ) -> crate::error::Result<Vec<Self>> {
        raw.into_iter().map(Self::parse).collect()
    }
}

/// The canonical broadcast selector string.
pub const BROADCAST_SELECTOR: &str = "broadcast";

/// One notice row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// `'n_' || uuid v4`. GC-generated summaries use a deterministic
    /// `n_weekly_<week_start>` id instead — see [`notice_db`].
    pub id: String,
    /// RFC3339, fixed millisecond precision. Indexed for GC and inbox.
    pub created_at: String,
    /// Unix seconds. The inbox-bridge cursor key; indexed.
    pub epoch: i64,
    /// Employee id, or `user`, or `system`.
    pub sender: String,
    /// Sender provenance. `None` in storage; the read path `COALESCE`s a
    /// missing value to [`UNKNOWN_DEPT`], reproducing AD-013 §2's legacy
    /// default without a backfill.
    pub from_dept: Option<String>,
    /// Typed selectors, canonical form. `["broadcast"]` for a broadcast.
    pub recipients: Vec<Recipient>,
    pub subject: Option<String>,
    pub body: String,
    /// Join key for the request → ack → result chain.
    pub correlation_id: Option<String>,
    pub ack_required: bool,
    /// Employee ids that have acked. Empty until someone does.
    pub acked_by: Vec<String>,
    pub acked_at: Option<String>,
    /// RFC3339. `None` = never expires.
    pub ttl_expires_at: Option<String>,
    pub tags: Vec<String>,
    /// Explicit thread anchor. Corresponds to the organism's `thread_to`.
    pub thread_id: Option<String>,
    /// Retention pin. Pinned rows are never GC'd — the organism sources
    /// pins from worklog links, open chains and `retention-pins.jsonl`
    /// (`axe_lib.py:761-763`), and §3.3 forbids the sidecar, so the pin is
    /// a column.
    pub pinned: bool,
    /// The `--reason` of the posting write.
    pub reason: String,
    /// Free-form structured metadata. Carries the GC weekly-summary block
    /// (`week_start` / `week_end` / `entry_count` / distributions).
    pub metadata: Option<serde_json::Value>,
}

impl Notice {
    /// `true` when this notice carries the weekly-summary tag.
    pub fn is_weekly_summary(&self) -> bool {
        self.tags.iter().any(|t| t == WEEKLY_SUMMARY_TAG)
    }

    /// `true` when this notice carries the ack tag.
    pub fn is_ack(&self) -> bool {
        self.tags.iter().any(|t| t == ACK_TAG)
    }

    /// `true` when `employee_id` is in this notice's `acked_by` array.
    pub fn is_acked_by(&self, employee_id: &str) -> bool {
        self.acked_by.iter().any(|a| a == employee_id)
    }

    /// `true` when this notice's TTL has elapsed as of `now_str`.
    ///
    /// A `None` TTL never expires. `now_str` must come from [`rfc3339`] so
    /// the comparison is lexicographic-over-equal-width and therefore
    /// chronological — see that function's doc comment.
    pub fn is_expired_at(&self, now_str: &str) -> bool {
        self.ttl_expires_at
            .as_deref()
            .is_some_and(|exp| exp < now_str)
    }
}

/// Parameters for posting a notice.
///
/// `ttl` is a *duration* rather than an absolute instant so a caller cannot
/// write a notice that expired before it was born; the board turns it into
/// `ttl_expires_at` at post time. `ttl_expires_at` is the escape hatch for the
/// one caller that legitimately needs an instant in the past — a replay, a
/// migration backfill, a test that must produce a genuinely expired row. It
/// wins over `ttl` when both are set.
pub struct PostNotice {
    pub sender: String,
    pub from_dept: Option<String>,
    pub recipients: Vec<Recipient>,
    pub subject: Option<String>,
    pub body: String,
    pub correlation_id: Option<String>,
    pub ack_required: bool,
    pub ttl: Option<Duration>,
    pub ttl_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tags: Vec<String>,
    pub thread_id: Option<String>,
    pub pinned: bool,
    pub reason: String,
    pub metadata: Option<serde_json::Value>,
}

impl PostNotice {
    /// A notice with the required fields set and everything else defaulted.
    pub fn new(
        sender: impl Into<String>,
        recipients: Vec<Recipient>,
        body: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            sender: sender.into(),
            from_dept: None,
            recipients,
            subject: None,
            body: body.into(),
            correlation_id: None,
            ack_required: false,
            ttl: None,
            ttl_expires_at: None,
            tags: Vec::new(),
            thread_id: None,
            pinned: false,
            reason: reason.into(),
            metadata: None,
        }
    }
}

/// Inbox query parameters.
#[derive(Debug, Clone, Default)]
pub struct InboxQuery {
    /// Only notices with `epoch >= this`. This is the incremental-fetch
    /// cursor the `idx_notices_epoch` index exists for. `None` = from the
    /// beginning of time.
    pub after_epoch: Option<i64>,
    /// When set, return only notices addressed to this reader's selectors.
    /// `None` = every notice on the board regardless of recipient.
    pub selectors: Option<Vec<String>>,
    /// When true, return only notices with `ack_required` set that the
    /// reader has not acked.
    pub pending_only: bool,
    /// The reader, for `pending_only` (to exclude notices it already acked).
    pub reader_id: Option<String>,
    pub limit: Option<i64>,
}

/// How much retention GC is allowed to do in one tick.
///
/// The organism's own GC needed this: `NoticeBoard.retention_gc` carries
/// `max_weekly_summaries_per_tick=4` (`axe_lib.py:753-758`) because a pass
/// that summarised every elapsed week at once is the unbounded-work failure
/// the parameter exists to prevent. §3.3 says to carry it; we also cap the
/// row deletes for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionLimits {
    /// Maximum raw rows deleted per pass, and maximum weekly-summary rows
    /// pruned per pass. Defaults to [`DEFAULT_MAX_ROWS_PER_PASS`].
    pub max_rows_per_pass: i64,
    /// Maximum weeks summarized per pass. Defaults to
    /// [`DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK`] — the organism's
    /// `max_weekly_summaries_per_tick=4` (`axe_lib.py:753-758`).
    ///
    /// Setting this to `0` disables summarization, and that also disables raw
    /// collection entirely: a week is only collectable once its summary has
    /// been written, so nothing is ever collected. That is deliberate. §3.3
    /// deletes raw rows unconditionally, which collects quickly but also
    /// destroys any week the cap has not reached yet — with a 5-week backlog
    /// and the default cap of 4, the fifth week is lost on the first tick.
    /// Holding rows back is recoverable; losing them is not.
    pub max_weekly_summaries_per_tick: i64,
}

impl Default for RetentionLimits {
    fn default() -> Self {
        Self {
            max_rows_per_pass: DEFAULT_MAX_ROWS_PER_PASS,
            max_weekly_summaries_per_tick: DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK,
        }
    }
}

/// Default per-pass row-deletion cap for raw entries and expired summaries.
pub const DEFAULT_MAX_ROWS_PER_PASS: i64 = 500;

/// Default per-pass weekly-summary cap, carried from the organism's
/// `max_weekly_summaries_per_tick=4` (`axe_lib.py:753-758`).
pub const DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK: i64 = 4;

/// What one GC pass did.
///
/// `more_work_pending` is the batching contract: when a step hits its cap it
/// sets this, and the caller runs GC again next tick rather than the pass
/// growing without bound. It can be `true` while every count is `0` — a
/// summary pass that found only already-summarized weeks reports that, so the
/// caller keeps scheduling rather than concluding the backlog is drained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Raw rows deleted (past `RAW_RETENTION_DAYS`, unpinned, no live chain).
    pub raw_deleted: i64,
    /// Weekly-summary rows inserted.
    pub summaries_written: i64,
    /// Weekly-summary rows pruned (past `SUMMARY_RETENTION_WEEKS`).
    pub summaries_deleted: i64,
    /// A step hit its cap and has more work queued for the next tick.
    pub more_work_pending: bool,
}

/// Resolves a reader's typed selectors so the inbox query can match them.
///
/// Kept as a trait rather than an `Employee` parameter on purpose: the
/// employee registry is a separate packet, and the board should not be
/// coupled to its type. The harness provider adapts an employee to this.
pub trait NoticeInboxMatcher {
    /// The canonical selectors that address this reader. A notice reaches
    /// the reader when any of its recipients is in this set, or when the
    /// notice is a broadcast.
    fn selectors(&self) -> Vec<Recipient>;
}

/// A plain, testable [`NoticeInboxMatcher`] built from the four membership
/// facts. `team` membership stays empty until Wave 3 ships a resolver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoticeIdentity {
    pub employee_id: String,
    pub dept: Option<String>,
    pub teams: Vec<String>,
    pub roles: Vec<String>,
}

impl NoticeIdentity {
    pub fn new(employee_id: impl Into<String>) -> Self {
        Self {
            employee_id: employee_id.into(),
            dept: None,
            teams: Vec::new(),
            roles: Vec::new(),
        }
    }

    pub fn in_dept(mut self, dept: impl Into<String>) -> Self {
        self.dept = Some(dept.into());
        self
    }

    pub fn on_teams<I: IntoIterator<Item = String>>(mut self, teams: I) -> Self {
        self.teams = teams.into_iter().collect();
        self
    }

    pub fn with_roles<I: IntoIterator<Item = String>>(mut self, roles: I) -> Self {
        self.roles = roles.into_iter().collect();
        self
    }
}

impl NoticeInboxMatcher for NoticeIdentity {
    fn selectors(&self) -> Vec<Recipient> {
        let mut out = vec![Recipient::Agent(self.employee_id.clone())];
        if let Some(dept) = &self.dept {
            out.push(Recipient::Dept(dept.clone()));
        }
        out.extend(self.teams.iter().cloned().map(Recipient::Team));
        out.extend(self.roles.iter().cloned().map(Recipient::Role));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_recipient_normalises_like_the_organism() {
        // axe_lib.py:191-217 — empty/None and "broadcast" are broadcast,
        // a bare string becomes agent:, a string with ':' is passed through.
        let b = Recipient::Broadcast;
        assert_eq!(Recipient::parse("").expect("parses"), b);
        assert_eq!(Recipient::parse("   ").expect("parses"), b);
        assert_eq!(Recipient::parse("broadcast").expect("parses"), b);
        assert_eq!(Recipient::parse("BROADCAST").expect("parses"), b);
        assert_eq!(
            Recipient::parse("axe-test2").expect("parses"),
            Recipient::Agent("axe-test2".into())
        );
        assert_eq!(
            Recipient::parse("dept:task-grid").expect("parses"),
            Recipient::Dept("task-grid".into())
        );
        assert_eq!(
            Recipient::parse("agent:emp-1").expect("parses"),
            Recipient::Agent("emp-1".into())
        );
        assert_eq!(
            Recipient::parse("role:reviewer").expect("parses"),
            Recipient::Role("reviewer".into())
        );
        assert_eq!(
            Recipient::parse("team:core").expect("parses"),
            Recipient::Team("core".into())
        );
    }

    #[test]
    fn notice_recipient_selector_round_trips() {
        for raw in [
            "broadcast",
            "agent:emp-1",
            "dept:platform-infra",
            "team:core",
            "role:reviewer",
        ] {
            let parsed = Recipient::parse(raw).expect("parses");
            assert_eq!(parsed.selector(), raw, "round trip failed for {raw}");
        }
    }

    #[test]
    fn notice_recipient_rejects_unknown_kind() {
        assert!(Recipient::parse("guild:ops").is_err());
        assert!(Recipient::parse("dept:").is_err());
    }

    #[test]
    fn notice_identity_expands_membership_facts() {
        let id = NoticeIdentity::new("emp-1")
            .in_dept("platform-infra")
            .on_teams(vec!["core".to_string()])
            .with_roles(vec!["reviewer".to_string()]);
        let sels: Vec<String> = NoticeInboxMatcher::selectors(&id)
            .iter()
            .map(Recipient::selector)
            .collect();
        assert_eq!(
            sels,
            vec![
                "agent:emp-1".to_string(),
                "dept:platform-infra".to_string(),
                "team:core".to_string(),
                "role:reviewer".to_string(),
            ]
        );
    }

    #[test]
    fn notice_rfc3339_is_fixed_width_so_string_order_is_chronological() {
        // The GC and TTL filters compare these lexicographically; equal width
        // is what makes that safe.
        let at = |s: &str| {
            rfc3339(
                chrono::DateTime::parse_from_rfc3339(s)
                    .expect("parses")
                    .to_utc(),
            )
        };
        let a = at("2026-09-30T11:48:55Z");
        let b = at("2026-09-30T11:48:55.500Z");
        // Millisecond precision, `Z` offset. Fixed width, so string order is
        // chronological order.
        assert_eq!(a, "2026-09-30T11:48:55.000Z");
        assert_eq!(b, "2026-09-30T11:48:55.500Z");
        assert!(a < b, "equal-width RFC3339 must sort chronologically");
        assert!(at("2026-09-30T11:48:55.999+00:00") < at("2026-09-30T11:48:56Z"));
        // A cross-second boundary still orders correctly: the width is equal
        // on both sides, so the differing digit decides.
        assert!(at("2026-09-30T11:48:55.999Z") < at("2026-09-30T11:48:56.000Z"));
    }
}

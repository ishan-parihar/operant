//! Employee registry types and the `employee_id` derivation.
//!
//! Pure: no I/O, no sqlite. The store lives in [`super::employee_db`].
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.1, §3.1.1, §3.1.2.

use serde::{Deserialize, Serialize};

/// Prefix every `employee_id` carries.
pub const EMPLOYEE_ID_PREFIX: &str = "emp-";

/// How many characters of the cron job id are folded into an
/// `employee_id`. The organism's own convention is 12 hex characters of the
/// cron job id (`employees.json`: `emp-a02e3f692fb0` whose
/// `home_crons[0].cron_id` is `a02e3f692fb0`) — see §3.1.2.
pub const EMPLOYEE_ID_HEX_LEN: usize = 12;

/// `role` written on every backfilled row. The organism uses `automator`
/// for its cron-derived employees (`employees.json` sample: `emp-a02e3f692fb0`
/// has `"role": "automator"`).
pub const DEFAULT_ROLE: &str = "automator";

/// `status` written when the cron job's `state` is the normal scheduled
/// state. Every other cron `state` is carried through verbatim rather than
/// collapsed, so `paused` stays visible in the registry.
pub const STATUS_ACTIVE: &str = "active";

/// The fallback `reason` recorded on a backfilled row when the caller supplies
/// none — i.e. a library caller, not the CLI.
///
/// §3.1 requires every row to record why it exists. When an operator *is* in
/// the loop, the row must carry **their** reason, verbatim: the CLI's
/// `--reason` is a mandatory validated argument, and stamping a constant
/// instead would make the audit trail name a party that did not make the
/// write. That was a real defect — `org sync` validated and echoed the
/// operator's reason, then persisted a constant. See
/// `EmployeeDb::backfill_from_cron_jobs`.
pub const BACKFILL_REASON: &str = "org sync: backfill from cron job";

/// The cron `state` value that maps to [`STATUS_ACTIVE`].
pub const CRON_STATE_SCHEDULED: &str = "scheduled";

/// Long-lived agent kind, from WAVE1-DECISIONS §3.1.2 / Q3.
///
/// **There is deliberately no `#[default]` variant.** Absence is
/// `Option<AgentType>::None`, and `None` is distinct from all three
/// variants. That distinctness is the whole point: it is what makes
/// "absent keeps today's behavior" true (Q3). Adding a `Default` here —
/// or collapsing `None` into any variant — would silently change the
/// behavior of every existing cron job on the first tick.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    /// A long-lived operational role — infrastructure, monitoring, a service
    /// that is always on.
    Service,
    /// A role that lives for one working session: research, drafting, a
    /// scheduled piece of work with a beginning and an end.
    Session,
    /// A standing remediation role. A fixer is *hired*, like a janitor or an
    /// engineer: it has a permanent identity and a persistent seat, and it
    /// is on call rather than being consumed by the work.
    Fixer,
}

impl AgentType {
    /// The wire form stored in `employees.agent_type`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Session => "session",
            Self::Fixer => "fixer",
        }
    }

    /// Parse a stored `agent_type` string. `None` for a NULL column and
    /// for an unrecognized value — absence and "we don't know this" must
    /// both read as absent, never as a guess at a variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "service" => Some(Self::Service),
            "session" => Some(Self::Session),
            "fixer" => Some(Self::Fixer),
            _ => None,
        }
    }
}

/// One employee row. Mirrors the `employees` table in §3.1 exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Employee {
    pub employee_id: String,
    pub name: String,
    pub role: String,
    /// NULL until Wave 3 builds the department registry. Operant has no
    /// department concept yet and `None` is the honest value.
    pub department: Option<String>,
    /// JSON array of leaf skill names. May legitimately be empty — a job
    /// with no skills backfills to `[]` and stays that way. See
    /// [`Employee::missing_required_fields`].
    pub skills: Vec<String>,
    /// `None` = absent. See [`AgentType`].
    pub agent_type: Option<AgentType>,
    /// JSON `Persona`, NULL until Wave 2.
    pub persona: Option<serde_json::Value>,
    /// The employee's charter: the system prompt a session executing as
    /// this employee runs under (ORGANISM-ARCHITECTURE §2 — "each employee
    /// carries its own `system_prompt` charter"). `None` on every
    /// cron-backfilled row: the charter is cast-seed data, and a cron job
    /// owns its prompt elsewhere (`employee_cron_jobs` is the link back).
    pub system_prompt: Option<String>,
    pub status: String,
    /// Why this row exists. Never empty.
    pub reason: String,
    /// RFC3339.
    pub created_at: String,
    /// RFC3339.
    pub updated_at: String,
}

impl Employee {
    /// The three Wave 1 required identity fields, in the order §3.2 lists
    /// them. Returns the names of the ones that are missing.
    ///
    /// This is the predicate the fail-closed gate (packet B) consumes. It
    /// lives here, in the registry, because a field's requiredness is a
    /// property of the record — not of the gate that happens to read it.
    ///
    /// Note what is *not* in the set: `agent_type`, `department`,
    /// `persona`. §3.2 narrows the organism's 5-field contract
    /// (AXE-AD-012) to these three precisely so the gate is passable
    /// against the live DB: measured over 102 real jobs, requiring all
    /// five would block 34, requiring these three blocks 6.
    pub fn missing_required_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.employee_id.trim().is_empty() {
            missing.push("employee_id");
        }
        if self.name.trim().is_empty() {
            missing.push("name");
        }
        if self.skills.is_empty() {
            missing.push("skills");
        }
        missing
    }

    /// True when the row satisfies the Wave 1 required set.
    pub fn is_valid(&self) -> bool {
        self.missing_required_fields().is_empty()
    }
}

/// One `employee_cron_jobs` row. The employee's link back to the cron row
/// that already owns the prompt/model/delivery — the registry does not
/// duplicate those (§3.1.1).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmployeeCronJob {
    pub employee_id: String,
    pub cron_job_id: String,
    /// Human display string, e.g. `"*/15 * * * *"`.
    pub schedule: Option<String>,
    pub enabled: bool,
}

/// Derive an `employee_id` from a cron job id.
///
/// §3.1.2: `employee_id = "emp-" + job.id[..min(12, len)]`.
///
/// # Why the slice is byte-safe but not char-aligned
///
/// The rule indexes bytes. For any non-ASCII id that could land mid
/// codepoint, so this uses a char-boundary-aware truncation. Every id
/// operant actually produces is ASCII (`cron_<8 hex>`, see
/// `CronDb::create_job` at `cronjobs/db.rs:181`), so the two paths are
/// identical in practice; the char-aware form exists so a hand-created or
/// imported non-ASCII id degrades to a shorter id instead of panicking.
///
/// **Known deviation from the organism's convention, deliberate here:**
/// operant's live job ids are `cron_` + 8 hex (13 chars), so the literal
/// 12-byte slice yields e.g. `emp-cron_3236529` — the prefix is not hex
/// and the last hex digit is dropped. The organism's ids *are* bare
/// 12-hex, so there the same rule yields `emp-a02e3f692fb0` exactly. The
/// rule is implemented as written because it is deterministic, stable
/// under re-runs, and reproduces the organism's shape on hex ids. See the
/// packet-A report for the full argument.
pub fn derive_employee_id(cron_job_id: &str) -> String {
    let mut out = String::from(EMPLOYEE_ID_PREFIX);
    let mut taken = 0usize;
    for ch in cron_job_id.chars() {
        if taken >= EMPLOYEE_ID_HEX_LEN {
            break;
        }
        let len = ch.len_utf8();
        if taken + len > EMPLOYEE_ID_HEX_LEN {
            // The 12-byte budget cannot hold a whole codepoint. Take as
            // much as fits on the boundary rather than panicking.
            break;
        }
        out.push(ch);
        taken += len;
    }
    out
}

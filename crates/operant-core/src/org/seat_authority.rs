//! P2 of `docs/PERMISSION-SCOPING-PLAN.md` §5: the escalation core — the
//! seams that turn `decide()`'s `Escalate` verdict into a durable ask, and
//! an approval back into standing authority.
//!
//! Two halves, one file:
//!
//! - [`SeatAuthority`] is the **run-path** half the agent guard consults
//!   (wave-2 slice E threaded only the policy source; this widens the same
//!   Option-field seam to carry the grant ledger and the request queue, so
//!   the consult can complete precedence rule 6 — `has_grant` becomes REAL,
//!   read from [`GrantDb::list_for_grantee`] — and an escalation can be
//!   persisted). It performs no blocking waits and never prompts.
//! - [`SeatApprover`] is the **resolution** half. An approver's verdict on a
//!   queued ask mints a TTL'd grant through the EXISTING
//!   [`crate::tools::issue_grant`] — which bites when the approver's scope
//!   does not cover the grant, and that refusal is recorded as the
//!   request's resolution (Denied), never bypassed — then resolves the
//!   queue row. §8's delivery model holds: the tool runs only when a grant
//!   stands, never at request time.
//!
//! Routing is `Hierarchy::manager_of` over `HierarchyEdgesDb` entries +
//! `EmployeeDb` rows; an absent edge — or a store that cannot be read —
//! falls back to the `'operator'` literal (§6 Q5: the human operator is the
//! terminal authority) instead of collapsing the branch.

use std::sync::Arc;

use tracing::warn;

use crate::org::authority::{AuthorityScope, Grant, GrantDb, resolve_scope};
use crate::org::employee::Employee;
use crate::org::employee_db::EmployeeDb;
use crate::org::hierarchy::Hierarchy;
use crate::org::hierarchy_edges::HierarchyEdgesDb;
use crate::org::notice::rfc3339;
use crate::org::pending_requests::{PendingRequest, PendingRequestDb, Resolution, Status};
use crate::org::seat_policy::{SeatDecision, SeatPolicySource, decide};
use crate::tools::{SeatDirectory, issue_grant};

/// The terminal approver when no hierarchy edge names one (§6 Q5). A literal,
/// not an employee id: it deliberately fails [`issue_grant`]'s grantor-seat
/// check, so an approval that would otherwise be attributed to nobody is
/// refused loudly instead of minting authority from thin air.
pub const OPERATOR_FALLBACK: &str = "operator";

// =====================================================================
// SeatAuthority — the run-path half
// =====================================================================

/// The composed seat-policy seam: one handle bundling everything the run
/// guard needs to consult the genome, so the agent carries ONE `Option`
/// instead of three and the precedence consult cannot read a ledger the
/// dispatcher writes to a different file.
///
/// Wave-2 slice E threaded `Option<Arc<dyn SeatPolicySource>>`; this is that
/// seam widened (the builder is `with_seat_authority`). `None` — or a seat
/// with no policy row — keeps today's ungoverned behaviour byte-for-byte.
pub struct SeatAuthority {
    policy: Arc<dyn SeatPolicySource>,
    grants: Arc<GrantDb>,
    requests: Arc<PendingRequestDb>,
}

impl SeatAuthority {
    pub fn new(
        policy: Arc<dyn SeatPolicySource>,
        grants: Arc<GrantDb>,
        requests: Arc<PendingRequestDb>,
    ) -> Self {
        Self {
            policy,
            grants,
            requests,
        }
    }

    /// Consult the genome for one tool call. `None` = ungoverned (no source
    /// or no policy row): the caller must reproduce today's behaviour
    /// exactly. `blocked` stays `false` here for the reason slice E recorded:
    /// the smart approval gate above the consult has already denied blocked
    /// tools, and the hardline floor must fire before any seat mode can see.
    ///
    /// `has_grant` is REAL here (plan §4 rule 6 completed): an unexpired,
    /// unrevoked grant row naming THIS tool exactly, via
    /// [`GrantDb::list_for_grantee`] (which already refuses lapsed rows, so
    /// `true` means *currently standing*). A ledger that cannot be read is
    /// `false` — fail closed, the seat falls back to its mode default.
    pub fn consult(&self, seat: &str, tool: &str, dangerous: bool) -> Option<SeatDecision> {
        let policy = self.policy.policy_for(seat)?;
        let has_grant = self.standing_grant_for(seat, tool).is_some();
        Some(decide(Some(&policy), tool, false, dangerous, has_grant))
    }

    /// The standing grant covering `tool` for `seat`, if any.
    ///
    /// Exact capability match only — `content.*` style namespace globs belong
    /// to the policy lists, not the ledger: a grant is authority someone
    /// explicitly minted for a named tool, and pattern-matching it would let
    /// one approval buy a namespace.
    pub fn standing_grant_for(&self, seat: &str, tool: &str) -> Option<Grant> {
        match self.grants.list_for_grantee(seat) {
            Ok(grants) => grants.into_iter().find(|g| g.capability == tool),
            Err(e) => {
                warn!(
                    seat = %seat,
                    tool = %tool,
                    error = %e,
                    "seat authority: grant ledger unreadable — treating the seat as grantless (fail closed)"
                );
                None
            }
        }
    }

    /// Persist one escalation ask, deduplicated.
    ///
    /// §8.3: re-escalation *references* the standing ask, it never
    /// duplicates it. A pending row for the same (seat, tool) returns its
    /// own id with a "still pending" note; otherwise a fresh row is
    /// enqueued with `why` as the judgeable note. The returned
    /// [`QueuedAsk::escalation`] is `None` only when the queue write itself
    /// failed — the run still denies/clamps, loudly, rather than pretending
    /// the ask was recorded.
    pub fn escalate(&self, seat: &str, tool: &str, why: &str) -> QueuedAsk {
        let existing = match self.requests.pending() {
            Ok(rows) => rows
                .into_iter()
                .find(|r| r.employee_id == seat && r.tool == tool),
            Err(e) => {
                warn!(seat = %seat, tool = %tool, error = %e, "seat authority: pending queue unreadable — a duplicate ask is possible");
                None
            }
        };
        if let Some(row) = existing {
            let note = format!(
                "is still pending as {} since {}",
                row.request_id, row.requested_at
            );
            return QueuedAsk {
                escalation: Some(SeatEscalation {
                    request_id: row.request_id,
                    employee_id: seat.to_string(),
                    tool: row.tool,
                    why: why.to_string(),
                }),
                note,
            };
        }
        match self.requests.enqueue(seat, tool, why) {
            Ok(request_id) => QueuedAsk {
                escalation: Some(SeatEscalation {
                    request_id,
                    employee_id: seat.to_string(),
                    tool: tool.to_string(),
                    why: why.to_string(),
                }),
                note: String::new(),
            },
            Err(e) => {
                warn!(seat = %seat, tool = %tool, error = %e, "seat authority: enqueue failed — the ask was NOT persisted");
                QueuedAsk {
                    escalation: None,
                    note: format!("could not be persisted ({e})"),
                }
            }
        }
    }
}

/// What [`SeatAuthority::escalate`] produced: the queue handle to attach to
/// a permission prompt (so an approval can resolve the row it approved),
/// plus the reference sentence for logs and deny texts.
#[derive(Debug, Clone)]
pub struct QueuedAsk {
    pub escalation: Option<SeatEscalation>,
    pub note: String,
}

/// The queue-side identity of one escalation, threaded from the run guard
/// into `ToolPermissionRequest` so the dispatcher's approval can mint and
/// resolve the ask it was actually asked about. `None` on a request means
/// today's ungoverned prompt — no queue row, no minting, byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatEscalation {
    pub request_id: String,
    /// The REQUESTING seat (the employee whose `decide()` escalated).
    pub employee_id: String,
    /// The tool the seat wants to run.
    pub tool: String,
    /// `decide()`'s why — the judgeable note the approver rules on, and the
    /// grant's recorded reason when approved.
    pub why: String,
}

// =====================================================================
// SeatApprover — the resolution half
// =====================================================================

/// Resolves queued asks: an approval mints a TTL'd grant through
/// [`issue_grant`] and records it; a denial records a denial; the 60s
/// interactive lapse records an expiry. Every resolution names the
/// resolving seat.
///
/// Grant shape per the owner defaults (§11 Q3/Q8): an org lead (CEO+) mints
/// a standing `Org` grant (`expires_at = None`); a department head mints a
/// `Department` grant pinned to the requesting seat's department, TTL
/// `grant_ttl_days` (default 7). Anyone below head rank is not in the
/// granting topology — the mint is attempted at the department shape and
/// `issue_grant`'s containment check refuses it, because a refusal on the
/// record is worth more than a quiet widening of who may grant.
/// The shape [`SeatApprover::mint_for`] confers for (approver, seat):
/// which scope, which pinned department, which expiry — derived from the
/// hierarchy, not requested by the caller. Read it through
/// [`SeatApprover::preview_mint`]; the mint itself is the only writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintShape {
    /// `Org` for a CEO+ approver, `Department` otherwise.
    pub scope: AuthorityScope,
    /// The grantee's department for a departmental mint; `None` when `Org`.
    pub target_dept: Option<String>,
    /// `None` = standing (CEO+ only); otherwise RFC3339.
    pub expires_at: Option<String>,
}

pub struct SeatApprover {
    grants: Arc<GrantDb>,
    requests: Arc<PendingRequestDb>,
    employees: Arc<EmployeeDb>,
    edges: Arc<HierarchyEdgesDb>,
    grant_ttl_days: i64,
}

impl SeatApprover {
    pub fn new(
        grants: Arc<GrantDb>,
        requests: Arc<PendingRequestDb>,
        employees: Arc<EmployeeDb>,
        edges: Arc<HierarchyEdgesDb>,
        grant_ttl_days: i64,
    ) -> Self {
        Self {
            grants,
            requests,
            employees,
            edges,
            grant_ttl_days,
        }
    }

    /// The seat that rules on `employee_id`'s asks: their manager, or the
    /// `'operator'` literal when no edge names one. A store error is a
    /// warned fallback, never a branch collapse — routing must degrade to
    /// the terminal authority, not to an error nobody answers.
    pub fn approver_of(&self, employee_id: &str) -> String {
        let loaded = self
            .employees
            .list_employees()
            .and_then(|employees| Ok((employees, self.edges.entries()?)));
        match loaded {
            Ok((employees, entries)) => Hierarchy::new(&employees, &entries)
                .manager_of(employee_id)
                .map(str::to_string)
                .unwrap_or_else(|| OPERATOR_FALLBACK.to_string()),
            Err(e) => {
                warn!(
                    seat = %employee_id,
                    error = %e,
                    "seat approver: hierarchy unreadable — routing to the operator literal"
                );
                OPERATOR_FALLBACK.to_string()
            }
        }
    }

    /// Approve one queued ask: mint, then resolve. `Ok` carries the minted
    /// grant id.
    ///
    /// When [`issue_grant`] refuses — the approver's scope does not cover
    /// the grant, or a seat is vacant/invalid — the refusal is RECORDED as
    /// the resolution (Denied, with the error text logged and returned) and
    /// the caller must tell the waiting run `Deny`. There is no path that
    /// widens authority around the check: the check failing IS the outcome.
    pub fn approve(&self, ask: &SeatEscalation) -> Result<String, String> {
        let approver = self.approver_of(&ask.employee_id);
        match self.mint(ask, &approver) {
            Ok(grant_id) => {
                match self.requests.resolve(
                    &ask.request_id,
                    Resolution::Approved {
                        grant_id: grant_id.clone(),
                    },
                    &approver,
                ) {
                    Ok(true) => {}
                    // Unknown id (already swept?) or a resolve error: the
                    // grant STANDS — authority exists whether or not the
                    // audit row moved, and the next consult runs on the
                    // ledger, not the queue. Loud, not fatal.
                    Ok(false) => warn!(
                        request = %ask.request_id,
                        grant = %grant_id,
                        "seat approver: approval minted but the queue row is already gone"
                    ),
                    Err(e) => warn!(
                        request = %ask.request_id,
                        grant = %grant_id,
                        error = %e,
                        "seat approver: approval minted but the queue row could not be resolved"
                    ),
                }
                Ok(grant_id)
            }
            Err(refusal) => {
                warn!(
                    request = %ask.request_id,
                    approver = %approver,
                    refusal = %refusal,
                    "seat approver: issue_grant refused the approval — recording the ask as denied"
                );
                if let Err(e) =
                    self.requests
                        .resolve(&ask.request_id, Resolution::Denied, &approver)
                {
                    warn!(request = %ask.request_id, error = %e, "seat approver: the denial could not be recorded either");
                }
                Err(refusal)
            }
        }
    }

    /// Deny one queued ask, recording the resolving seat.
    pub fn deny(&self, ask: &SeatEscalation) -> Result<(), String> {
        let approver = self.approver_of(&ask.employee_id);
        self.requests
            .resolve(&ask.request_id, Resolution::Denied, &approver)
            .map(|_| ())
            .map_err(|e| format!("deny {request}: {e}", request = ask.request_id))
    }

    /// Record the interactive 60s lapse (§8.3: the attempt is denied, the
    /// row survives for audit). The resolver is the operator literal —
    /// nobody answered.
    pub fn expire(&self, ask: &SeatEscalation) -> Result<(), String> {
        self.requests
            .resolve(&ask.request_id, Resolution::Expired, OPERATOR_FALLBACK)
            .map(|_| ())
            .map_err(|e| format!("expire {request}: {e}", request = ask.request_id))
    }

    /// P3's proactive mint: the operator grants a capability to a seat
    /// WITHOUT a queued escalation, speaking with the authority of that
    /// seat's approver-of-record — the same seat [`Self::approve`] names
    /// as the resolver, so `/grant` and `/approve` confer identical
    /// authority through identical checks. An unmanaged seat (approver
    /// falls back to the `'operator'` literal, who holds no scope) fails
    /// closed here, exactly as it does at [`Self::approve`]: the remedy is
    /// a hierarchy edge or a policy allow-row, never a widened mint.
    /// `ttl_days` overrides `[genome] grant_ttl_days` for this mint only
    /// (the org-lead standing rule is untouched by the override).
    pub fn grant_direct(
        &self,
        seat: &str,
        tool: &str,
        ttl_days: Option<i64>,
        why: &str,
    ) -> Result<String, String> {
        let approver = self.approver_of(seat);
        self.mint_for(seat, tool, why, &approver, ttl_days)
    }

    /// The seat's live grants (not revoked, not lapsed) — `/permissions`
    /// reads this. The audit view including lapsed/revoked rows is
    /// [`GrantDb::history_for_grantee`].
    pub fn grants_for(&self, seat: &str) -> Result<Vec<Grant>, String> {
        self.grants
            .list_for_grantee(seat)
            .map_err(|e| format!("the grant ledger is unreadable: {e}"))
    }

    /// The seat's still-pending escalations — `/permissions` reads this.
    pub fn pending_for(&self, seat: &str) -> Result<Vec<PendingRequest>, String> {
        let rows = self
            .requests
            .list_for_employee(seat)
            .map_err(|e| format!("the escalation queue is unreadable: {e}"))?;
        Ok(rows
            .into_iter()
            .filter(|r| r.status == Status::Pending)
            .collect())
    }

    /// Revoke a grant by id (P3 `/revoke`). `Ok(false)` = no such grant;
    /// a revoked grant stops consulting immediately because
    /// [`GrantDb::list_for_grantee`]'s live filter excludes it.
    pub fn revoke_grant(&self, grant_id: &str, reason: &str) -> Result<bool, String> {
        self.grants
            .revoke(grant_id, reason)
            .map_err(|e| format!("the grant ledger is unreadable: {e}"))
    }

    /// Mint the grant an approval confers. All refusal paths return before
    /// any write, exactly as [`issue_grant`] promises.
    fn mint(&self, ask: &SeatEscalation, approver: &str) -> Result<String, String> {
        self.mint_for(&ask.employee_id, &ask.tool, &ask.why, approver, None)
    }

    /// The mint body shared by queued approvals and P3's direct grants.
    /// `ttl_days_override` falls back to `[genome] grant_ttl_days`.
    ///
    /// `pub`: two out-of-approver callers must both reach the contained
    /// mint — the CLI's `org grant give` (F1, ORGANISM-ARCHITECTURE §6)
    /// and the cast seeder (§7; it cannot use [`Self::grant_direct`]: that
    /// verb derives the approver from the hierarchy, and the org lead has
    /// no manager edge, so it would route to the `'operator'` literal and
    /// fail closed as a vacant grantor). Every mint — gateway `/grant`,
    /// `/approve`, the CLI, and the seeder — passes the same grantor-scope
    /// ceiling, TTL shape, and vacant/unmanaged-seat fail-closed.
    /// [`crate::org::authority::GrantDb::insert`] is `pub(crate)`; this is
    /// the only cross-crate write path onto the ledger.
    pub fn mint_for(
        &self,
        seat: &str,
        tool: &str,
        why: &str,
        approver: &str,
        ttl_days_override: Option<i64>,
    ) -> Result<String, String> {
        let (grantor_scope, shape) = self.mint_shape(approver, seat, ttl_days_override)?;
        let grant = Grant::new(
            approver,
            seat,
            tool,
            shape.scope,
            shape.target_dept,
            why,
            shape.expires_at,
        );
        issue_grant(&grant, grantor_scope, self.employees.as_ref(), &self.grants)
            .map_err(|e| format!("{e}"))
    }

    /// The shape [`Self::mint_for`] confers for (approver, seat), derived
    /// by the same code the mint runs — the two cannot drift. F1's CLI
    /// route compares the operator's `--scope` / `--target-dept` /
    /// `--expires-at` against this BEFORE minting, so a request the
    /// approver would not mint is refused with no row written — the same
    /// all-refusals-first promise [`issue_grant`] makes.
    pub fn preview_mint(
        &self,
        approver: &str,
        seat: &str,
        ttl_days_override: Option<i64>,
    ) -> Result<MintShape, String> {
        Ok(self.mint_shape(approver, seat, ttl_days_override)?.1)
    }

    /// (grantor scope, mint shape) — one derivation shared by the mint
    /// and the preview.
    fn mint_shape(
        &self,
        approver: &str,
        seat: &str,
        ttl_days_override: Option<i64>,
    ) -> Result<(AuthorityScope, MintShape), String> {
        let employees = self
            .employees
            .list_employees()
            .map_err(|e| format!("the employee registry is unreadable: {e}"))?;
        let entries = self
            .edges
            .entries()
            .map_err(|e| format!("the hierarchy edges are unreadable: {e}"))?;
        let hierarchy = Hierarchy::new(&employees, &entries);
        let org_lead = hierarchy.is_org_lead(approver);
        let grantor_scope = resolve_scope(hierarchy.is_dept_head(approver), org_lead);

        // Shape: CEO+ mints a standing Org grant; a head mints a
        // department-pinned one. `issue_grant` enforces that the grantor
        // actually holds the scope being granted.
        let (scope, target_dept) = if org_lead {
            (AuthorityScope::Org, None)
        } else {
            (
                AuthorityScope::Department,
                hierarchy.department_of(seat).map(str::to_string),
            )
        };

        let ttl_days = ttl_days_override.unwrap_or(self.grant_ttl_days);
        let expires_at = if org_lead {
            // Standing grants are CEO+ only (§11 Q3).
            None
        } else if ttl_days > 0 {
            Some(rfc3339(
                chrono::Utc::now() + chrono::Duration::days(ttl_days),
            ))
        } else {
            // 0/negative TTL: never-standing for non-CEO approvals — the
            // grant is recorded already-lapsed, so the approval is auditable
            // while conferring nothing.
            Some(rfc3339(chrono::Utc::now()))
        };

        Ok((
            grantor_scope,
            MintShape {
                scope,
                target_dept,
                expires_at,
            },
        ))
    }
}

/// [`SeatDirectory`] over the employee registry, so [`issue_grant`]'s
/// vacant-seat and validity checks read the live store rather than a
/// snapshot the caller had to remember to refresh.
impl SeatDirectory for EmployeeDb {
    fn employee(&self, employee_id: &str) -> Option<Employee> {
        match self.get_employee(employee_id) {
            Ok(found) => found,
            Err(e) => {
                warn!(
                    seat = %employee_id,
                    error = %e,
                    "employee registry read failed — treating the seat as vacant (fail closed)"
                );
                None
            }
        }
    }
}

// =====================================================================
// tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::employee_db::EmployeeDb;
    use crate::org::hierarchy_edges::HierarchyEdgesDb;
    use crate::org::pending_requests::Status;
    use crate::org::seat_policy::SeatMode;
    use crate::org::seat_policy_db::SeatPolicyDb;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// All five stores one tempdir file — the way the gateway shares one
    /// `database.db` — plus the raw shared connection so tests can seed
    /// rows through the front door where a store has no public insert (the
    /// employee registry's only write path is backfill, which hardcodes
    /// `department: None`; position math needs a department, so the seed
    /// writes that one column directly).
    struct Stores {
        _dir: tempfile::TempDir,
        _path: PathBuf,
        conn: Arc<std::sync::Mutex<Connection>>,
        policies: Arc<SeatPolicyDb>,
        grants: Arc<GrantDb>,
        requests: Arc<PendingRequestDb>,
        employees: Arc<EmployeeDb>,
        edges: Arc<HierarchyEdgesDb>,
    }

    fn stores() -> Stores {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("org.sqlite");
        let conn = Arc::new(std::sync::Mutex::new(
            Connection::open(&path).expect("open org sqlite"),
        ));
        Stores {
            policies: Arc::new(
                SeatPolicyDb::from_shared_connection(Arc::clone(&conn)).expect("policies"),
            ),
            grants: Arc::new(GrantDb::from_shared_connection(Arc::clone(&conn)).expect("grants")),
            requests: Arc::new(
                PendingRequestDb::from_shared_connection(Arc::clone(&conn)).expect("requests"),
            ),
            employees: Arc::new(
                EmployeeDb::from_shared_connection(Arc::clone(&conn)).expect("employees"),
            ),
            edges: Arc::new(
                HierarchyEdgesDb::from_connection(Connection::open(&path).expect("edges conn"))
                    .expect("edges"),
            ),
            _dir: dir,
            _path: path,
            conn,
        }
    }

    /// Seed one employee row with a department (the column `backfill` cannot
    /// write). Direct SQL through the store's own connection — documented
    /// seeding, not a store bypass: the test asserts through the public
    /// APIs afterwards.
    fn seed_employee(s: &Stores, id: &str, name: &str, dept: Option<&str>) {
        s.conn
            .lock()
            .expect("conn")
            .execute(
                "INSERT OR REPLACE INTO employees (
                     employee_id, name, role, department, skills, agent_type,
                     persona, status, reason, created_at, updated_at
                 ) VALUES (?1, ?2, 'tester', ?3, '[\"probe\"]', NULL, NULL,
                           'active', 'test seed', ?4, ?4)",
                rusqlite::params![id, name, dept, rfc3339(chrono::Utc::now())],
            )
            .expect("seed employee");
    }

    fn authority(s: &Stores) -> SeatAuthority {
        SeatAuthority::new(
            Arc::clone(&s.policies) as Arc<dyn SeatPolicySource>,
            Arc::clone(&s.grants),
            Arc::clone(&s.requests),
        )
    }

    fn approver(s: &Stores, ttl_days: i64) -> SeatApprover {
        SeatApprover::new(
            Arc::clone(&s.grants),
            Arc::clone(&s.requests),
            Arc::clone(&s.employees),
            Arc::clone(&s.edges),
            ttl_days,
        )
    }

    fn lockdown(s: &Stores, seat: &str) {
        s.policies
            .upsert(
                seat,
                &crate::org::seat_policy::SeatPolicy {
                    mode: SeatMode::Lockdown,
                    allow: Vec::new(),
                    deny: Vec::new(),
                },
            )
            .expect("upsert lockdown policy");
    }

    // -------------------------------------------------- enqueue dedupe

    /// §8.3: the second ask for the same (seat, tool) references the first
    /// row instead of duplicating it. Without the dedupe scan this test
    /// sees two rows and fails — that is what keeps it from passing
    /// vacuously.
    #[test]
    fn escalate_dedupes_the_same_seat_and_tool() {
        let s = stores();
        let a = authority(&s);
        let first = a.escalate("seat-1", "bash", "lockdown seat wants bash");
        let second = a.escalate("seat-1", "bash", "lockdown seat wants bash again");

        assert!(
            first.note.is_empty(),
            "a fresh enqueue reports no reference note"
        );
        assert!(
            second.note.starts_with("is still pending as pr_"),
            "the deduped ask must reference the standing row: {:?}",
            second.note
        );
        assert_eq!(
            first.escalation.as_ref().expect("first ask").request_id,
            second.escalation.as_ref().expect("second ask").request_id,
            "both asks must name the SAME row"
        );
        let pending = s.requests.pending().expect("pending");
        assert_eq!(pending.len(), 1, "exactly one row, never a duplicate");
        assert_eq!(pending[0].employee_id, "seat-1");
        assert_eq!(pending[0].tool, "bash");

        // A different tool for the same seat is a NEW ask, not a dedupe hit.
        let other = a.escalate("seat-1", "file_write", "lockdown seat wants file_write");
        assert!(other.note.is_empty());
        assert_eq!(s.requests.pending().expect("pending").len(), 2);
    }

    // ---------------------------------------------- consult + real ledger

    /// Rule 6 with a REAL ledger: a standing grant naming the tool exactly
    /// turns a lockdown Escalate into a Run; a grant naming a different
    /// capability does not (exact match, never a pattern).
    #[test]
    fn consult_reads_standing_grants_from_the_ledger() {
        let s = stores();
        lockdown(&s, "seat-2");
        let a = authority(&s);

        assert!(
            matches!(
                a.consult("seat-2", "bash", true),
                Some(SeatDecision::Escalate(_))
            ),
            "lockdown escalates bash with no grant"
        );

        s.grants
            .insert(&Grant::new(
                "emp-manager",
                "seat-2",
                "file_write",
                AuthorityScope::Department,
                None,
                "approved ask",
                None,
            ))
            .expect("insert unrelated grant");
        assert!(
            matches!(
                a.consult("seat-2", "bash", true),
                Some(SeatDecision::Escalate(_))
            ),
            "a grant for file_write must NOT cover bash — capability match is exact"
        );

        s.grants
            .insert(&Grant::new(
                "emp-manager",
                "seat-2",
                "bash",
                AuthorityScope::Department,
                None,
                "approved ask",
                None,
            ))
            .expect("insert covering grant");
        assert!(
            matches!(
                a.consult("seat-2", "bash", true),
                Some(SeatDecision::Run(_))
            ),
            "a standing grant naming bash exactly must Run under lockdown"
        );
    }

    // ------------------------------------------------------- approver routing

    /// manager_of routes the ask; an absent edge degrades to the 'operator'
    /// literal rather than collapsing the branch.
    #[test]
    fn approver_of_routes_to_the_manager_or_the_operator_literal() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-manager", "M", Some("platform"));
        s.edges
            .upsert_edge("emp-worker", "emp-manager", "test routing")
            .expect("upsert edge");
        let ap = approver(&s, 7);

        assert_eq!(ap.approver_of("emp-worker"), "emp-manager");
        assert_eq!(
            ap.approver_of("emp-manager"),
            OPERATOR_FALLBACK,
            "no edge above the manager — the operator literal, not an error"
        );
        assert_eq!(
            ap.approver_of("emp-nobody"),
            OPERATOR_FALLBACK,
            "an unknown seat still routes to the operator literal"
        );
    }

    // ------------------------------------------------------------- minting

    /// The happy approval: head-of-department approver mints a
    /// department-pinned, TTL'd grant; the queue row resolves Approved
    /// naming both the grant and the resolver; the seat's next consult Runs.
    #[test]
    fn approve_mints_a_ttl_department_grant_and_resolves_the_row() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-manager", "M", Some("platform"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        // The manager is a department head (a same-dept report) WITH a
        // manager above them — the HoD tier: not an org lead, so the
        // approval mints a TTL'd department grant, never a standing one.
        s.edges
            .upsert_edge("emp-worker", "emp-manager", "test approval")
            .expect("edge w->m");
        s.edges
            .upsert_edge("emp-manager", "emp-ceo", "test approval")
            .expect("edge m->c");
        lockdown(&s, "emp-worker");
        let a = authority(&s);
        let ask = a
            .escalate("emp-worker", "bash", "lockdown seat wants bash")
            .escalation
            .expect("queued ask");

        let ap = approver(&s, 7);
        let grant_id = ap.approve(&ask).expect("the head approval must mint");

        let grants = s.grants.list_for_grantee("emp-worker").expect("ledger");
        let grant = grants
            .iter()
            .find(|g| g.grant_id == grant_id)
            .expect("the minted grant stands in the ledger");
        assert_eq!(grant.grantor, "emp-manager");
        assert_eq!(grant.capability, "bash");
        assert_eq!(grant.scope, AuthorityScope::Department);
        assert_eq!(grant.target_dept.as_deref(), Some("platform"));
        assert!(
            grant.expires_at.is_some(),
            "a HoD approval is TTL'd (7 days), never standing"
        );
        assert!(
            grant.reason.contains("lockdown seat wants bash"),
            "the grant records the judgeable ask as its reason"
        );

        // The queue row is resolved by the approver, naming the grant.
        let pending = s.requests.pending().expect("pending");
        assert!(pending.is_empty(), "the ask is no longer pending");
        let rows: Vec<(String, Option<String>, Option<String>)> = {
            let conn = s.conn.lock().expect("conn");
            let mut stmt = conn
                .prepare("SELECT status, resolved_by, grant_id FROM pending_requests")
                .expect("prepare");
            let mapped = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .expect("query");
            mapped.filter_map(|r| r.ok()).collect::<Vec<_>>()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, Status::Approved.as_str());
        assert_eq!(rows[0].1.as_deref(), Some("emp-manager"));
        assert_eq!(rows[0].2.as_deref(), Some(grant_id.as_str()));

        // The §5 acceptance: the next consult does not re-ask.
        assert!(matches!(
            a.consult("emp-worker", "bash", true),
            Some(SeatDecision::Run(_))
        ));
    }

    /// issue_grant biting: an approver whose own reports sit in ANOTHER
    /// department holds only Peers scope (no same-dept report → not a
    /// head), and Peers cannot contain a Department grant. The refusal is
    /// RECORDED as the ask's resolution — denied, no grant, never a bypass.
    #[test]
    fn approve_refused_by_issue_grant_records_the_denial() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-cross-manager", "PM", Some("sales"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        // PM has a manager (and no same-DEPARTMENT report), so §2.1 makes
        // them a Peer, not a head; the worker reports to PM across the
        // department line.
        s.edges
            .upsert_edge("emp-cross-manager", "emp-ceo", "test topology")
            .expect("edge pm->ceo");
        s.edges
            .upsert_edge("emp-worker", "emp-cross-manager", "test topology")
            .expect("edge w->pm");

        let a = authority(&s);
        let ask = a
            .escalate("emp-worker", "bash", "cross-managed seat wants bash")
            .escalation
            .expect("queued ask");

        let ap = approver(&s, 7);
        let refusal = ap
            .approve(&ask)
            .expect_err("a peer cannot mint a Department grant");
        assert!(
            refusal.contains("does not contain"),
            "the refusal must be issue_grant's own scope text: {refusal}"
        );

        let conn = s.conn.lock().expect("conn");
        let (status, grant_id): (String, Option<String>) = conn
            .query_row("SELECT status, grant_id FROM pending_requests", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("read the resolved row");
        drop(conn);
        assert_eq!(status, Status::Denied.as_str());
        assert_eq!(grant_id, None, "no grant was minted");
        assert!(
            s.grants
                .list_for_grantee("emp-worker")
                .expect("ledger")
                .is_empty(),
            "the refused approval left the ledger empty"
        );
    }

    /// A vacant approver seat (the 'operator' literal, or an unknown id) is
    /// refused the same way: authority is never minted from thin air.
    #[test]
    fn approve_with_no_edge_fails_closed_through_the_operator_literal() {
        let s = stores();
        seed_employee(&s, "emp-orphan", "O", Some("platform"));
        let a = authority(&s);
        let ask = a
            .escalate("emp-orphan", "bash", "orphan seat wants bash")
            .escalation
            .expect("queued ask");

        let ap = approver(&s, 7);
        let refusal = ap
            .approve(&ask)
            .expect_err("the operator literal holds no seat");
        assert!(
            refusal.contains("grantor"),
            "the refusal must name the vacant grantor: {refusal}"
        );
        let conn = s.conn.lock().expect("conn");
        let status: String = conn
            .query_row("SELECT status FROM pending_requests", [], |row| row.get(0))
            .expect("read the resolved row");
        drop(conn);
        assert_eq!(status, Status::Denied.as_str());
    }

    /// CEO (org lead) approvals are standing and org-wide — the one tier the
    /// owner allows `expires_at = None`.
    #[test]
    fn org_lead_approvals_are_standing_org_grants() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        s.edges
            .upsert_edge("emp-worker", "emp-ceo", "test topology")
            .expect("edge");
        // The CEO has a same-dept report and no manager — the org-lead tier.
        let a = authority(&s);
        let ask = a
            .escalate("emp-worker", "bash", "ceo-managed seat wants bash")
            .escalation
            .expect("queued ask");

        let ap = approver(&s, 7);
        let grant_id = ap.approve(&ask).expect("the CEO approval mints");
        let grants = s.grants.list_for_grantee("emp-worker").expect("ledger");
        let grant = grants
            .iter()
            .find(|g| g.grant_id == grant_id)
            .expect("standing grant");
        assert_eq!(grant.scope, AuthorityScope::Org);
        assert_eq!(
            grant.expires_at, None,
            "CEO+ approvals are the standing tier"
        );
    }

    /// ttl 0/negative = never-standing for non-CEO approvals: the grant is
    /// recorded already-lapsed — the approval is auditable, confers
    /// nothing, and the seat escalates again on the next consult.
    #[test]
    fn zero_ttl_approvals_record_already_lapsed_grants() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-manager", "M", Some("platform"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        s.edges
            .upsert_edge("emp-worker", "emp-manager", "test topology")
            .expect("edge w->m");
        s.edges
            .upsert_edge("emp-manager", "emp-ceo", "test topology")
            .expect("edge m->c");
        lockdown(&s, "emp-worker");
        let a = authority(&s);
        let ask = a
            .escalate("emp-worker", "bash", "lockdown seat wants bash")
            .escalation
            .expect("queued ask");

        let ap = approver(&s, 0);
        let grant_id = ap.approve(&ask).expect("the mint itself succeeds");
        assert!(
            s.grants
                .list_for_grantee("emp-worker")
                .expect("ledger")
                .iter()
                .all(|g| g.grant_id != grant_id),
            "the lapsed grant must not stand in the live ledger view"
        );
        assert!(
            matches!(
                a.consult("emp-worker", "bash", true),
                Some(SeatDecision::Escalate(_))
            ),
            "a lapsed grant confers nothing — the seat re-escalates"
        );
        let conn = s.conn.lock().expect("conn");
        let status: String = conn
            .query_row("SELECT status FROM pending_requests", [], |row| row.get(0))
            .expect("read the resolved row");
        drop(conn);
        assert_eq!(
            status,
            Status::Approved.as_str(),
            "the approval is recorded"
        );
    }

    /// F1's CLI route calls `mint_for` with the operator's `--grantor` as
    /// F1's CLI route calls `mint_for` with the operator's `--grantor` as
    /// the approver, not the queue's approver-of-record. The runnable twin
    /// of the cmd_org test that cannot link on a mid-port branch: a
    /// below-head approver is refused by the scope ceiling, and the
    /// refusal writes nothing.
    #[test]
    fn mint_for_with_a_below_head_approver_refuses_and_writes_nothing() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-manager", "M", Some("platform"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        s.edges
            .upsert_edge("emp-worker", "emp-manager", "test topology")
            .expect("edge w->m");
        s.edges
            .upsert_edge("emp-manager", "emp-ceo", "test topology")
            .expect("edge m->c");

        let ap = approver(&s, 7);
        let refusal = ap
            .mint_for(
                "emp-worker",
                "bash",
                "worker tries to grant a peer",
                "emp-worker",
                None,
            )
            .expect_err("a below-head approver must not mint");
        assert!(
            refusal.contains("does not contain"),
            "the refusal must name the scope ceiling: {refusal}"
        );
        assert!(
            s.grants
                .list_for_grantee("emp-worker")
                .expect("ledger")
                .is_empty(),
            "a refused mint must write nothing"
        );
    }

    /// The preview the CLI asserts its flags against is the SAME derivation
    /// the mint runs — scope and department cannot drift between the two.
    #[test]
    fn preview_mint_matches_the_minted_grant() {
        let s = stores();
        seed_employee(&s, "emp-worker", "W", Some("platform"));
        seed_employee(&s, "emp-manager", "M", Some("platform"));
        seed_employee(&s, "emp-ceo", "C", Some("platform"));
        s.edges
            .upsert_edge("emp-worker", "emp-manager", "test topology")
            .expect("edge w->m");
        s.edges
            .upsert_edge("emp-manager", "emp-ceo", "test topology")
            .expect("edge m->c");

        let ap = approver(&s, 7);

        // The HoD shape: department scope pinned to the grantee's
        // department, TTL'd.
        let head_shape = ap
            .preview_mint("emp-manager", "emp-worker", None)
            .expect("head preview");
        assert_eq!(head_shape.scope, AuthorityScope::Department);
        assert_eq!(head_shape.target_dept.as_deref(), Some("platform"));
        assert!(head_shape.expires_at.is_some(), "HoD shape is TTL'd");

        // The org-lead shape: standing, unpinned.
        let ceo_shape = ap
            .preview_mint("emp-ceo", "emp-worker", None)
            .expect("org-lead preview");
        assert_eq!(ceo_shape.scope, AuthorityScope::Org);
        assert_eq!(ceo_shape.target_dept, None);
        assert_eq!(ceo_shape.expires_at, None, "org-lead shape is standing");

        // And the mint writes exactly the previewed shape.
        let grant_id = ap
            .mint_for(
                "emp-worker",
                "bash",
                "preview must match the mint",
                "emp-manager",
                None,
            )
            .expect("the HoD mint");
        let grants = s.grants.list_for_grantee("emp-worker").expect("ledger");
        let grant = grants
            .iter()
            .find(|g| g.grant_id == grant_id)
            .expect("the minted grant");
        assert_eq!(grant.scope, head_shape.scope);
        assert_eq!(grant.target_dept, head_shape.target_dept);
        assert!(grant.expires_at.is_some(), "the mint is TTL'd like the preview");
    }
}

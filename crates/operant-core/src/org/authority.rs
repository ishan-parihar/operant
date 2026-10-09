//! Scoped authority — the lattice, the grant table, and the designation.
//! (`docs/ORG-AUTHORITY-ARCHITECTURE.md` §1, §2, §2.5.)
//!
//! ## Why this module exists
//!
//! §2.1 records the owner's direction: *hierarchy scopes the range of
//! authority*. So hierarchy is the **primary** grant and is sufficient for
//! everything inside a department; an explicit capability grant is required
//! **only** to cross a department boundary — because that is the only place
//! hierarchy gives no answer. Everything here exists to make that one sentence
//! executable, and §2.3 insists it is enforced in exactly four places or it is
//! "decoration".
//!
//! ## The lattice, and the tension in it
//!
//! [`AuthorityScope`] is an ordered lattice, not a flag set: an action is
//! permitted when the actor's scope **contains** the target's scope, and
//! containment is tree containment. `Descendants` contains `DirectReports`
//! contains `Department`; `Org` contains everything; `Self` contains only
//! itself.
//!
//! ## Where this table lives, and why it is not a new file
//!
//! `authority_grants` goes in the **existing** `operant_kanban.db`, per §13's
//! data-model summary. `notice_db` makes the same choice for `notices` and
//! documents the cost: a second location for one concept is what `BUGS.md`
//! R5-1 (the `memory/` split-brain) already paid for once.
//!
//! ## How the schema is applied without a migration conflict
//!
//! `CREATE TABLE IF NOT EXISTS`, plus [`crate::org::schema::ensure_column`]
//! for any column add — never `crate::migrations::migrate` and never
//! `PRAGMA user_version`. `user_version` is file-wide: `operant_kanban.db` sits
//! at version 1 (kanban's single `MIGRATIONS` entry), and appending an authority
//! entry here would move the file's version and make kanban's one-entry family
//! *look* downgraded the next time kanban opens the same file, hard-failing
//! with "refusing to downgrade". That guard is `migrations.rs`'s own documented
//! INVARIANT (added after R39-7), and §12 reaches the same conclusion. The
//! grants table therefore owns no version counter and bumps no PRAGMA.
//!
//! ## Expiry is enforced at READ time, not by a sweeper
//!
//! §2.5: "Grants expire by default: a cross-department capability that nobody
//! renewed should not silently persist for a year." The honest way to enforce
//! that is to make an expired grant **absent from
//! [`GrantDb::list_for_grantee`]**, so no call site has to remember to also
//! check the clock. A background sweeper that deletes lapsed rows would make
//! the audit trail lossy — a grant that was relied on and then quietly vanished
//! from the record — so expiry is a *read filter* over a row that is still
//! there. [`Grant::is_lapsed_at`] is the single predicate both the read path
//! and any caller-side check use.
//!
//! ## Why the resolution functions take their inputs as parameters
//!
//! [`resolve_scope`], [`can_post_to`], and [`can_accept_decision`] are pure.
//! They do not read `employees` or `departments`: `department_db` is a
//! separate packet, and a pure predicate that took a `&dyn` lookup would be
//! harder to test than one that takes two `bool`s. The caller — the one that
//! already has the rows — resolves `head_employee_id` into the flags and calls
//! in. The enforcement sites in §2.3 stay uniform because they all consume the
//! same [`ScopeCheck`].

use crate::error::Error;
use crate::org::notice::{Recipient, rfc3339};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

// =====================================================================
// AuthorityScope
// =====================================================================

/// How far an employee's authority reaches. Ordered by **tree containment**,
/// not by magnitude of privilege: `a <= b` reads "`a` is contained by `b`".
///
/// §2.2's table, verbatim, with the honest caveat about `Peers` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
// Every wire name is pinned explicitly rather than left to `rename_all`:
// the narrowest rung is spelled `Own` in Rust (see below) and would otherwise
// serialise as `"own"`, breaking the `"self"` name §2.2 specifies.
#[serde(rename_all = "snake_case")]
pub enum AuthorityScope {
    /// Own record only: read/write own profile, own worklog, own subjective log.
    ///
    /// §2.2 names this rung `Self`. It is spelled `Own` here because `Self` is
    /// a Rust keyword that cannot be a variant — not even as a raw identifier
    /// (`r#Self` is rejected by the parser). The *stored and parsed* name is
    /// still `"self"` ([`AuthorityScope::as_str`]), so §2.2's wire format, the
    /// `authority_grants.scope` column, and any CLI `--scope self` are all
    /// unaffected by the Rust-side spelling.
    #[serde(rename = "self")]
    Own,
    /// Same department: read peers' logs. **No writes** — the whole point of
    /// the rung between `Self` and `Department` is that reading a colleague
    /// does not imply writing for them.
    Peers,
    /// Own department: post to `dept:<me>`, read the assignment board,
    /// acknowledge.
    Department,
    /// Own reports: assign work, edit their persona drafts, approve their DMs.
    DirectReports,
    /// Whole subtree: department-level structural changes inside it.
    Descendants,
    /// Whole organisation: cross-department grants, board creation, CEO loop.
    Org,
}

impl AuthorityScope {
    /// Every scope, narrowest first. Used by round-trip and lattice tests, and
    /// by any caller that needs to render a scope menu.
    pub const ALL: [Self; 6] = [
        Self::Own,
        Self::Peers,
        Self::Department,
        Self::DirectReports,
        Self::Descendants,
        Self::Org,
    ];

    /// The stored/parsed spelling of this scope, matching §2.2's names.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Own => "self",
            Self::Peers => "peers",
            Self::Department => "department",
            Self::DirectReports => "direct_reports",
            Self::Descendants => "descendants",
            Self::Org => "org",
        }
    }

    /// The containment test: "`other` is inside the reach of a holder of
    /// `self`".
    ///
    /// This is `other <= *self`, and that is the whole lattice. It is written
    /// as a method rather than left to the `PartialOrd` derive because every
    /// enforcement site in §2.3 should read as a containment question, and
    /// because a future variant that is *not* ordered by containment will be
    /// forced to override this rather than silently inherit the wrong answer.
    pub fn contains(&self, other: Self) -> bool {
        other <= *self
    }

    /// `true` when this scope reaches across a department boundary on its own.
    ///
    /// §2.1: hierarchy is sufficient *inside* a department; crossing one is the
    /// only place an explicit grant is required. §2.4 makes this load-bearing
    /// beyond the board — a non-crossing scope is offered its own department's
    /// tool surface with no grant recorded.
    pub fn is_cross_department(&self) -> bool {
        matches!(self, Self::Org)
    }

    /// The one-line authority sentence used by [`Designation::render`].
    ///
    /// Derived from the scope rather than hand-written per employee, so a scope
    /// change cannot leave a stale claim in a designation.
    pub fn authority_sentence(&self) -> &'static str {
        match self {
            Self::Own => {
                "Authority: may act on own record only; may not read a peer's log or write \
                 for another employee."
            }
            Self::Peers => {
                "Authority: may read peers' logs within own department; may not write for \
                 another employee."
            }
            Self::Department => {
                "Authority: may act within own department; may not alter staffing outside it."
            }
            Self::DirectReports => {
                "Authority: may act within own department, including direct reports' records; \
                 may not alter staffing outside it."
            }
            Self::Descendants => {
                "Authority: may make structural changes within own subtree; may not act \
                 outside it without an explicit recorded grant."
            }
            Self::Org => {
                "Authority: organisation-wide; cross-department actions require an explicit \
                 recorded grant."
            }
        }
    }
}

impl fmt::Display for AuthorityScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse failure for [`AuthorityScope::from_str`].
///
/// Carries the offending input so a CLI `--scope` typo prints what was typed,
/// not just what was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeParseError {
    /// The rejected input, verbatim.
    pub raw: String,
}

impl fmt::Display for ScopeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "authority: '{}' is not an authority scope (expected self|peers|department|\
             direct_reports|descendants|org)",
            self.raw
        )
    }
}

impl std::error::Error for ScopeParseError {}

impl FromStr for AuthorityScope {
    type Err = ScopeParseError;

    /// Parse §2.2's scope names, case-insensitively.
    ///
    /// Rejects anything else rather than falling back to a default. A grant or
    /// a designation that silently widened or narrowed on a typo is exactly
    /// the failure §2.3 exists to prevent, so an unknown spelling is an error
    /// at the boundary.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "self" => Ok(Self::Own),
            "peers" => Ok(Self::Peers),
            "department" => Ok(Self::Department),
            "direct_reports" | "directreports" | "direct-reports" => Ok(Self::DirectReports),
            "descendants" | "descendant" => Ok(Self::Descendants),
            "org" => Ok(Self::Org),
            _ => Err(ScopeParseError { raw: s.to_string() }),
        }
    }
}

// =====================================================================
// ScopeCheck
// =====================================================================

/// The single answer to "may this actor do this to this target".
///
/// §2.3 names four enforcement sites and says an authority model that is not
/// enforced in all four "is decoration". Making the answer one type is what
/// keeps them uniform: a caller cannot return `Ok(())` from one site and a bare
/// `bool` from another, and every denial carries a human-readable `reason` for
/// the objective log rather than a bare `false`.
///
/// `required_scope` is the scope the *actor would need* in order to pass. It is
/// carried even on a grant-backed allow, because a denial's log line is what an
/// operator reads to find out which grant to issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeCheck {
    /// The verdict.
    pub allowed: bool,
    /// Human-readable justification, suitable for the objective log. Always
    /// populated — an unexplained deny is indistinguishable from a bug.
    pub reason: String,
    /// The scope the actor would need to proceed without a grant.
    pub required_scope: AuthorityScope,
}

impl ScopeCheck {
    /// Allowed. The reason states *why*, so an allow is as auditable as a deny.
    pub fn allow(reason: impl Into<String>, required_scope: AuthorityScope) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
            required_scope,
        }
    }

    /// Denied, with the scope that would have permitted it.
    pub fn deny(reason: impl Into<String>, required_scope: AuthorityScope) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
            required_scope,
        }
    }

    /// `true` when this check permits the action.
    ///
    /// A thin alias over the field, so call sites read as a question.
    pub fn is_allowed(&self) -> bool {
        self.allowed
    }

    /// Convert to a `Result` for a caller whose only question is "may I
    /// proceed". The [`Error`] carries the check's own `reason` and
    /// `required_scope`, so nothing is lost at the boundary.
    pub fn into_result(self) -> crate::error::Result<()> {
        if self.allowed {
            Ok(())
        } else {
            Err(Error::Agent(format!(
                "authority: denied (requires scope {}): {}",
                self.required_scope, self.reason
            )))
        }
    }
}

// =====================================================================
// Resolution — pure functions, no I/O
// =====================================================================

/// §2.2: "An ordinary employee is `Peers` + `Self`. A department head adds
/// `Department` + `DirectReports` + `Descendants`. The CEO is `Org`."
///
/// The inputs are flags rather than `&Employee`/`&Department` because the
/// department table belongs to another packet; the caller that already loaded
/// the rows resolves `head_employee_id` into `is_dept_head` and calls in. What
/// comes back is the **ceiling** of the scope, not a flag set — see the note
/// on [`AuthorityScope::Peers`] about what that ceiling does and does not claim.
///
/// `is_org_lead` is checked first: §8.3 makes the CEO an ordinary staff row
/// whose `Org` scope comes from an explicit grant, so an org lead is not
/// necessarily a department head and must not be inferred as one.
pub fn resolve_scope(is_dept_head: bool, is_org_lead: bool) -> AuthorityScope {
    if is_org_lead {
        AuthorityScope::Org
    } else if is_dept_head {
        AuthorityScope::Descendants
    } else {
        AuthorityScope::Peers
    }
}

/// §2.3.1 — board writes. The first of the four enforcement sites.
///
/// The rules, in order:
///
/// - `Recipient::Agent(me)` — always allowed. Addressing yourself is not a
///   cross-boundary act.
/// - `Recipient::Dept(slug)` where `slug == actor_dept` — allowed at **any**
///   scope. This is the ratchet: §3.2's department tier is the ordinary way a
///   department talks, and gating it behind a grant would push every notice
///   into `broadcast` and erase the segregation the tiers exist to provide.
/// - `Recipient::Dept(other)` — denied unless a live grant covers it. This is
///   the only place hierarchy gives no answer, so it is the only place a grant
///   is required (§2.1).
/// - `Broadcast`, `Role`, `Team` — allowed at any scope. Per §3.2 these are the
///   *global* tier, visible to everyone by construction, so addressing one is
///   an escalation that its recipients already see; refusing it here would
///   gate a public channel on a private capability, which is not what §2.1
///   says. Recorded as a deviation from a literal "posting outside your scope
///   needs a grant" reading of §2.3.1, and stated here rather than buried.
pub fn can_post_to(
    actor: AuthorityScope,
    actor_dept: Option<&str>,
    target: &Recipient,
    grants: &[Grant],
) -> ScopeCheck {
    let target_dept = match target {
        // Self-addressing needs no scope at all, so `required_scope` is
        // `Self`: that is the least authority the action required.
        Recipient::Agent(id) => {
            return ScopeCheck::allow(
                format!("direct addressing of self ({id}) is always permitted"),
                AuthorityScope::Own,
            );
        }
        Recipient::Dept(slug) => slug.as_str(),
        Recipient::Broadcast => {
            return ScopeCheck::allow(
                "broadcast is the global tier, visible to everyone by construction",
                AuthorityScope::Own,
            );
        }
        Recipient::Role(cap) => {
            return ScopeCheck::allow(
                format!("role:{cap} is a global capability selector, visible to its holders"),
                AuthorityScope::Own,
            );
        }
        // §3.2 has no team tier resolver; a `team:` notice currently reaches
        // nobody. Allowing the write keeps the store honest about what was
        // posted rather than failing on a selector the board already accepts.
        Recipient::Team(slug) => {
            return ScopeCheck::allow(
                format!("team:{slug} has no membership resolver yet and reaches no department"),
                AuthorityScope::Own,
            );
        }
    };

    if actor_dept == Some(target_dept) {
        return ScopeCheck::allow(
            format!("department tier: posting to own dept ({target_dept})"),
            AuthorityScope::Department,
        );
    }

    let matched = grants.iter().find(|g| g.covers_department(target_dept));
    match matched {
        Some(g) => ScopeCheck::allow(
            format!(
                "cross-department post to dept:{target_dept} covered by grant {} \
                 ({} / {}, {})",
                g.grant_id, g.capability, g.scope, g.reason
            ),
            actor,
        ),
        None => ScopeCheck::deny(
            format!(
                "no live grant covers dept:{target_dept}; §2.1 requires an explicit \
                 capability grant to cross a department boundary"
            ),
            AuthorityScope::Org,
        ),
    }
}

/// §2.3.3 — decision acceptance. Allowed iff `actor.contains(decision_scope)`.
///
/// The third enforcement site, and the one §8 pairs with the decision object:
/// a `Department`-scoped actor may not accept a decision targeted at `Org`,
/// because accepting it would bind a subtree it cannot see.
pub fn can_accept_decision(actor: AuthorityScope, decision_scope: AuthorityScope) -> ScopeCheck {
    if actor.contains(decision_scope) {
        ScopeCheck::allow(
            format!("{actor} contains the decision's target scope {decision_scope}"),
            decision_scope,
        )
    } else {
        ScopeCheck::deny(
            format!(
                "{actor} does not contain the decision's target scope {decision_scope}; \
                 §8 requires the accepting actor's scope to contain it"
            ),
            actor,
        )
    }
}
/// §2.3-analog — delegation posture. The per-seat policy row names HOW
/// much of its work a seat may hand to another agent; the D-2 ruling's
/// surface (the seat's policy row, and only that surface) extended to
/// delegation. `Independent` is the default absence — today's behaviour,
/// byte-for-byte — so wiring a posture is always the opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationPosture {
    /// The seat may never hand work off. Every `delegate` call is denied.
    Forbidden,
    /// The seat may delegate within its department, or cross-department on
    /// a live grant covering the target — the same tiers `can_post_to`
    /// applies to notices (§2.3.1), because delegation is work, and work
    /// follows the same boundaries as words.
    Bounded,
    /// Today's behaviour: the tool-config gates (depth, allowlists,
    /// readonly) own the verdict; governance is not consulted further.
    Independent,
}

impl DelegationPosture {
    /// Stored/parsed spelling on the policy row.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Bounded => "bounded",
            Self::Independent => "independent",
        }
    }
}

impl std::str::FromStr for DelegationPosture {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "forbidden" => Ok(Self::Forbidden),
            "bounded" => Ok(Self::Bounded),
            "independent" => Ok(Self::Independent),
            other => Err(format!(
                "unknown delegation posture '{other}' (expected forbidden/bounded/independent)"
            )),
        }
    }
}

impl fmt::Display for DelegationPosture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// §2.3-analog — the delegation predicate. PURE: posture + resolved
/// departments + live grants in, verdict out. The DB companions
/// ([`crate::org::seat_authority::SeatAuthority::consult_delegation`]) do
/// the resolution; this is the rule, stated once.
///
/// - `Forbidden` — deny, always. The policy row is the operator's word.
/// - `Independent` — allow, always: the tool-config gates own the rest.
/// - `Bounded` — the `can_post_to` tiers applied to work: same department
///   allowed, cross-department needs a live grant covering the target's
///   department, and an unregistered target is a refusal (the registry is
///   the only source of identity — fail-closed, like
///   [`resolve_actor_scope`]).
pub fn can_delegate(
    posture: DelegationPosture,
    actor_dept: Option<&str>,
    target_label: &str,
    target_dept: Option<&str>,
    grants: &[Grant],
) -> ScopeCheck {
    match posture {
        DelegationPosture::Forbidden => ScopeCheck::deny(
            "the seat's policy row forbids delegation (posture: forbidden)",
            AuthorityScope::Own,
        ),
        DelegationPosture::Independent => ScopeCheck::allow(
            "delegation posture is independent — tool-config gates own the verdict",
            AuthorityScope::Own,
        ),
        DelegationPosture::Bounded => {
            let Some(target_dept) = target_dept else {
                return ScopeCheck::deny(
                    format!(
                        "bounded delegation requires a registered target; \
                         '{target_label}' is not in the employee registry \
                         (fail-closed)"
                    ),
                    AuthorityScope::Department,
                );
            };
            if actor_dept == Some(target_dept) {
                return ScopeCheck::allow(
                    format!("department tier: delegating within own dept ({target_dept})"),
                    AuthorityScope::Department,
                );
            }
            let matched = grants.iter().find(|g| g.covers_department(target_dept));
            match matched {
                Some(g) => ScopeCheck::allow(
                    format!(
                        "cross-department delegation to '{target_label}' (dept:{target_dept}) \
                         covered by grant {} ({})",
                        g.grant_id, g.capability
                    ),
                    AuthorityScope::Org,
                ),
                None => ScopeCheck::deny(
                    format!(
                        "no live grant covers dept:{target_dept}; bounded delegation \
                         crosses departments only on an explicit grant (§2.1 tiers \
                         applied to work)"
                    ),
                    AuthorityScope::Org,
                ),
            }
        }
    }
}

// =====================================================================
// Consult companions (DB-touching)
// =====================================================================
//
// The three predicates above stay pure; these two functions are the
// impure companions every consult caller needs first — resolve WHO the
// actor is (scope + department) and WHAT grants it holds right now.
// They live here so the CLI seams and the scheduler-side socialization
// writer consult through the exact same resolution logic and cannot
// drift about what an actor's designation means.

/// Resolve an actor label into `(scope, department)` for a §2.3 consult.
///
/// `'user'`/`'system'` are the operator root — the standing Org-scope grant
/// is the root of every delegation (owner ruling), so they consult as
/// [`AuthorityScope::Org`] and are not subject to the predicates. An
/// employee resolves through their designation: org-lead = a live
/// capability-`org` grant ([`PREMIERE_GRANT_CAPABILITY`]); department head
/// = any department naming them as head ([`resolve_scope`]'s precedence).
///
/// An **unknown actor is a refusal** (fail-closed): the registry is the
/// only source of identity, and an unregistered label must not reach a
/// write path.
pub fn resolve_actor_scope(
    conn: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    actor: &str,
) -> Result<(AuthorityScope, Option<String>), crate::error::Error> {
    if actor == "user" || actor == "system" {
        return Ok((AuthorityScope::Org, None));
    }
    let employees = super::employee_db::EmployeeDb::from_shared_connection(conn.clone())?;
    let employee = employees.get_employee(actor)?.ok_or_else(|| {
        crate::error::Error::Agent(format!(
            "unknown actor '{actor}' — only registered employees, 'user', or 'system' \
             can act here (fail-closed)"
        ))
    })?;
    let grants = live_grants_for(conn, actor)?;
    let is_org_lead = grants
        .iter()
        .any(|g| g.capability == super::cast::PREMIERE_GRANT_CAPABILITY);
    let departments =
        super::department_db::DepartmentDb::from_shared_connection(conn.clone())?.list()?;
    let is_dept_head = departments
        .iter()
        .any(|d| d.head_employee_id() == Some(actor));
    Ok((
        resolve_scope(is_dept_head, is_org_lead),
        employee.department.clone(),
    ))
}

/// The live grants an actor holds right now — the only currency
/// [`can_post_to`] accepts for a cross-department post. Revoked and
/// expired rows are filtered here, once, so no consult caller can forget.
pub fn live_grants_for(
    conn: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    actor: &str,
) -> Result<Vec<Grant>, crate::error::Error> {
    let now = chrono::Utc::now().to_rfc3339();
    Ok(GrantDb::from_shared_connection(conn.clone())?
        .list_for_grantee(actor)?
        .into_iter()
        .filter(|g| g.is_live_at(&now))
        .collect())
}

// =====================================================================
// Grant
// =====================================================================

/// §2.5's `authority_grants` row.
///
/// One row is one individually recorded and individually revocable
/// cross-department capability. §2.5 is explicit that `reason` is required and
/// non-blank, that `revocation_reason` is required if `revoked_at` is set, and
/// that expiry is first-class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// `'ag_' || uuid v4`.
    pub grant_id: String,
    /// Employee id of the grantor. §15.4: a grant that extends an
    /// `Org`-touching scope requires the grantor to hold `Org`.
    pub grantor: String,
    /// Employee id of the grantee.
    pub grantee: String,
    /// e.g. `platform_infra.tooling`, `content.read`.
    pub capability: String,
    /// The scope being extended.
    pub scope: AuthorityScope,
    /// The specific department the grant is good for; `None` = any.
    pub target_dept: Option<String>,
    /// Required, non-blank. Same rule as every other mutation in Wave 1.
    pub reason: String,
    /// RFC3339, via [`rfc3339`] so string comparison is chronological.
    pub granted_at: String,
    /// RFC3339. `None` = never expires — which §2.5 says should be the
    /// exception, not the default. The caller chooses; the store does not
    /// invent an expiry, because inventing one would silently revoke a
    /// deliberate standing grant.
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    /// Required if `revoked_at` is set.
    pub revocation_reason: Option<String>,
}

impl Grant {
    /// Mint a grant with a fresh id and a `granted_at` of now.
    ///
    /// The `expires_at` is the caller's to set, for the reason given on the
    /// field. §2.5's "grants expire by default" is enforced by *requiring the
    /// caller to think about it*, and by [`GrantDb::list_for_grantee`] refusing
    /// to return a lapsed one — not by the constructor quietly picking a TTL
    /// the grantor never agreed to.
    pub fn new(
        grantor: impl Into<String>,
        grantee: impl Into<String>,
        capability: impl Into<String>,
        scope: AuthorityScope,
        target_dept: Option<String>,
        reason: impl Into<String>,
        expires_at: Option<String>,
    ) -> Self {
        Self {
            grant_id: format!("ag_{}", uuid::Uuid::new_v4()),
            grantor: grantor.into(),
            grantee: grantee.into(),
            capability: capability.into(),
            scope,
            target_dept,
            reason: reason.into(),
            granted_at: rfc3339(chrono::Utc::now()),
            expires_at,
            revoked_at: None,
            revocation_reason: None,
        }
    }

    /// `true` when this grant has lapsed as of `now_str`.
    ///
    /// `None` never expires. `now_str` must come from [`rfc3339`] — fixed
    /// millisecond width is what makes the lexicographic `<` chronological.
    pub fn is_lapsed_at(&self, now_str: &str) -> bool {
        self.expires_at.as_deref().is_some_and(|exp| exp <= now_str)
    }

    /// `true` when this grant has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    /// `true` when this grant currently confers anything at all.
    pub fn is_live_at(&self, now_str: &str) -> bool {
        !self.is_revoked() && !self.is_lapsed_at(now_str)
    }

    /// `true` when this grant is good for `dept`.
    ///
    /// `None` on [`Grant::target_dept`] means "any department" per §2.5.
    pub fn covers_department(&self, dept: &str) -> bool {
        match self.target_dept.as_deref() {
            None => true,
            Some(target) => target == dept,
        }
    }
}

// =====================================================================
// GrantDb
// =====================================================================

/// The `authority_grants` store, in the shared `operant_kanban.db`.
///
/// Mirrors [`crate::org::notice_db::NoticeBoard`]: same table-not-a-file
/// choice, same `init` / `from_connection` / `conn` shape, same
/// `Error::Agent(format!("...: {e}"))` wrapping, same "no unwrap, mutex
/// poisoning is a recoverable error" discipline.
pub struct GrantDb {
    conn: Arc<Mutex<Connection>>,
}

impl GrantDb {
    /// §2.5's DDL, applied declaratively and idempotently.
    ///
    /// See the module docs for why this is not a
    /// [`crate::migrations::migrate`] entry.
    pub const GRANTS_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS authority_grants (
            grant_id          TEXT PRIMARY KEY,   -- 'ag_' || uuid v4
            grantor           TEXT NOT NULL,      -- must hold Org to grant Org-touching scope (§15.4)
            grantee           TEXT NOT NULL,
            capability        TEXT NOT NULL,      -- e.g. 'platform_infra.tooling'
            scope             TEXT NOT NULL,      -- AuthorityScope as_str()
            target_dept       TEXT,               -- NULL = any department
            reason            TEXT NOT NULL,      -- required, non-blank (enforced in insert)
            granted_at        TEXT NOT NULL,      -- RFC3339, fixed millisecond precision
            expires_at        TEXT,               -- RFC3339; NULL = never
            revoked_at        TEXT,
            revocation_reason TEXT                -- required when revoked_at is set
        );

        CREATE INDEX IF NOT EXISTS idx_authority_grants_grantee ON authority_grants(grantee);
    "#;

    /// Open (or create) the grants table in an existing sqlite file.
    ///
    /// Tests point this at a temp path; production points it at
    /// [`crate::org::notice::NOTICE_BOARD_DB_FILE`]. Nothing here enforces
    /// which — the grants table is a table, not a file.
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("authority grants: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("authority grants: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach the store to a connection somebody else already owns.
    ///
    /// This is the path production should use: `authority_grants` is a table
    /// inside a database the kanban family owns, and opening a second
    /// `Connection` to the same file just to create one table trades a
    /// separate-store problem for a separate-lock problem. Same reasoning as
    /// [`crate::org::notice_db::NoticeBoard::from_connection`].
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` (the shape
    /// [`crate::org::notice_db::NoticeBoard::conn`] returns) so the grants
    /// share that handle instead of opening a second writer.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, turning poisoning into a recoverable error rather
    /// than a panic (same as `kanban/db.rs` and `cronjobs/db.rs`).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("authority grants db mutex poisoned".to_string()))
    }

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::GRANTS_SCHEMA)
            .map_err(|e| Error::Agent(format!("authority grants: schema: {e}")))
    }

    /// Write one grant.
    ///
    /// **The blank-`reason` rejection lives here, at the store boundary**, not
    /// at the CLI: §2.5's rule is that every grant is a mutation and every
    /// mutation carries a reason, so a caller that forgot `--reason` must fail
    /// here rather than write an unattributable row. `revocation_reason` is
    /// checked on the same boundary because a grant cannot arrive already
    /// revoked — [`GrantDb::revoke`] is the only way to set `revoked_at` — but
    /// the invariant is worth one cheap check rather than an assumption.
    ///
    /// `pub(crate)` (F1, ORGANISM-ARCHITECTURE §6): the only writers are
    /// [`crate::tools::issue_grant`] — itself reached only through
    /// [`crate::org::seat_authority::SeatApprover::mint_for`] — and
    /// same-crate tests. Cross-crate callers mint through the approver; a
    /// direct insert from outside this crate is a compile error, not a
    /// convention to remember.
    pub(crate) fn insert(&self, grant: &Grant) -> Result<(), Error> {
        require_reason("grant reason", &grant.reason)?;
        if grant.revoked_at.is_some() {
            require_reason(
                "revocation reason",
                grant.revocation_reason.as_deref().ok_or_else(|| {
                    Error::Agent(
                        "authority grants: revoked grant without a revocation_reason".to_string(),
                    )
                })?,
            )?;
        }

        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO authority_grants (
                grant_id, grantor, grantee, capability, scope, target_dept, reason,
                granted_at, expires_at, revoked_at, revocation_reason
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                grant.grant_id,
                grant.grantor,
                grant.grantee,
                grant.capability,
                grant.scope.as_str(),
                grant.target_dept,
                grant.reason,
                grant.granted_at,
                grant.expires_at,
                grant.revoked_at,
                grant.revocation_reason,
            ],
        )
        .map_err(|e| Error::Agent(format!("authority grants: insert {}: {e}", grant.grant_id)))?;
        Ok(())
    }

    /// Every **live** grant held by `grantee`: not revoked, not lapsed.
    ///
    /// Expiry and revocation are filtered here, at the single read all callers
    /// use, so no enforcement site has to remember to also consult the clock —
    /// see the module docs on why this is a read filter rather than a sweeper.
    /// The filters are applied in SQL for revocation (a `NULL` test on the
    /// column) and after the read for expiry (a lexicographic compare over
    /// equal-width RFC3339), matching what `notice_db` does for TTL.
    pub fn list_for_grantee(&self, grantee: &str) -> Result<Vec<Grant>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT grant_id, grantor, grantee, capability, scope, target_dept, reason, \
                        granted_at, expires_at, revoked_at, revocation_reason \
                   FROM authority_grants \
                  WHERE grantee = ?1 AND revoked_at IS NULL \
                  ORDER BY granted_at ASC, grant_id ASC",
            )
            .map_err(|e| Error::Agent(format!("authority grants: list prepare: {e}")))?;
        let rows = stmt
            .query_map(params![grantee], row_to_grant)
            .map_err(|e| Error::Agent(format!("authority grants: list query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let grant =
                row.map_err(|e| Error::Agent(format!("authority grants: list row: {e}")))?;
            if !grant.is_lapsed_at(&rfc3339(chrono::Utc::now())) {
                out.push(grant);
            }
        }
        Ok(out)
    }

    /// Every row held by `grantee`, lapsed and revoked included.
    ///
    /// The audit view. §2.5's revocation reason is only worth recording if
    /// something can read it back, and this is what reads it back.
    pub fn history_for_grantee(&self, grantee: &str) -> Result<Vec<Grant>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT grant_id, grantor, grantee, capability, scope, target_dept, reason, \
                        granted_at, expires_at, revoked_at, revocation_reason \
                   FROM authority_grants \
                  WHERE grantee = ?1 \
                  ORDER BY granted_at ASC, grant_id ASC",
            )
            .map_err(|e| Error::Agent(format!("authority grants: history prepare: {e}")))?;
        let rows = stmt
            .query_map(params![grantee], row_to_grant)
            .map_err(|e| Error::Agent(format!("authority grants: history query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Agent(format!("authority grants: history row: {e}")))?);
        }
        Ok(out)
    }

    /// Revoke a grant, recording who did it and why.
    ///
    /// `revocation_reason` must be non-blank: §2.5 makes it required whenever
    /// `revoked_at` is set, for the same reason a grant needs a reason — an
    /// unattributable revocation is indistinguishable from a bug, and it is
    /// the one change that removes somebody's capability.
    ///
    /// Returns `true` when a row was updated, `false` when the id is unknown
    /// or already revoked. Revoking twice is not an error: it is a no-op that
    /// leaves the first revocation's reason intact, which is the one worth
    /// keeping.
    ///
    /// `pub(crate)` (F1): the only production caller is
    /// [`crate::org::seat_authority::SeatApprover::revoke_grant`]; cross-crate
    /// revocation goes through the approver.
    pub(crate) fn revoke(&self, grant_id: &str, revocation_reason: &str) -> Result<bool, Error> {
        require_reason("revocation reason", revocation_reason)?;
        let conn = self.lock_conn()?;
        let updated = conn
            .execute(
                "UPDATE authority_grants SET revoked_at = ?2, revocation_reason = ?3 \
                  WHERE grant_id = ?1 AND revoked_at IS NULL",
                params![grant_id, rfc3339(chrono::Utc::now()), revocation_reason],
            )
            .map_err(|e| Error::Agent(format!("authority grants: revoke {grant_id}: {e}")))?;
        Ok(updated > 0)
    }

    /// Fetch one grant by id, live or not.
    pub fn get(&self, grant_id: &str) -> Result<Option<Grant>, Error> {
        self.lock_conn()?
            .query_row(
                "SELECT grant_id, grantor, grantee, capability, scope, target_dept, reason, \
                        granted_at, expires_at, revoked_at, revocation_reason \
                   FROM authority_grants WHERE grant_id = ?1",
                params![grant_id],
                row_to_grant,
            )
            .optional()
            .map_err(|e| Error::Agent(format!("authority grants: get {grant_id}: {e}")))
    }

    /// Total rows, including lapsed and revoked. Diagnostics and test
    /// affordance.
    pub fn count(&self) -> Result<i64, Error> {
        self.lock_conn()?
            .query_row("SELECT COUNT(*) FROM authority_grants", [], |r| r.get(0))
            .map_err(|e| Error::Agent(format!("authority grants: count: {e}")))
    }
}

// ------------------------------------------------------------------- helpers

/// Reject a blank or whitespace-only justification at the store boundary.
///
/// This is the §2.5 rule and the Wave-1 mutation rule in one place, so every
/// grant write is covered without each call site repeating the check (and
/// forgetting it once).
fn require_reason(field: &str, value: &str) -> Result<(), Error> {
    // iter-637: the guard lives in org::require_non_blank; this keeps the
    // authority message verbatim.
    super::require_non_blank(value, || {
        format!("authority grants: {field} is required and must not be blank")
    })
}

/// Ordinals of the grant columns, so the SELECT list and the mapper cannot
/// drift apart silently. Same defence as `notice_db`'s `mod col`.
mod col {
    pub(super) const GRANT_ID: usize = 0;
    pub(super) const GRANTOR: usize = 1;
    pub(super) const GRANTEE: usize = 2;
    pub(super) const CAPABILITY: usize = 3;
    pub(super) const SCOPE: usize = 4;
    pub(super) const TARGET_DEPT: usize = 5;
    pub(super) const REASON: usize = 6;
    pub(super) const GRANTED_AT: usize = 7;
    pub(super) const EXPIRES_AT: usize = 8;
    pub(super) const REVOKED_AT: usize = 9;
    pub(super) const REVOCATION_REASON: usize = 10;
}

fn row_to_grant(row: &Row<'_>) -> rusqlite::Result<Grant> {
    let raw_scope: String = row.get(col::SCOPE)?;
    // A scope spelling this build does not know is a row written by a newer
    // one, or a hand-edited row. Neither may be silently widened: degrade to
    // `Self`, the narrowest scope, so an unreadable grant grants nothing.
    let scope = raw_scope
        .parse::<AuthorityScope>()
        .unwrap_or(AuthorityScope::Own);
    Ok(Grant {
        grant_id: row.get(col::GRANT_ID)?,
        grantor: row.get(col::GRANTOR)?,
        grantee: row.get(col::GRANTEE)?,
        capability: row.get(col::CAPABILITY)?,
        scope,
        target_dept: row.get(col::TARGET_DEPT)?,
        reason: row.get(col::REASON)?,
        granted_at: row.get(col::GRANTED_AT)?,
        expires_at: row.get(col::EXPIRES_AT)?,
        revoked_at: row.get(col::REVOKED_AT)?,
        revocation_reason: row.get(col::REVOCATION_REASON)?,
    })
}

// =====================================================================
// Designation
// =====================================================================

/// §1's designation — derived, never stored.
///
/// §1's argument for deriving it is worth repeating because it is the whole
/// reason this is a function rather than a column: "an agent reading a board
/// must be able to trust that a designation is current. A derived string cannot
/// say 'Head of Platform' for someone who was moved yesterday." A stored string
/// can, and will.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Designation {
    /// The employee's display name.
    pub name: String,
    /// The role label. `Head of Department` for a filled head seat.
    pub role: String,
    /// The employee's department. `None` renders the `(undept)` marker that
    /// §3.1 requires for a NULL department.
    pub department: Option<String>,
    /// The `employee_id` this person reports to, rendered as a designation.
    pub reports_to: Option<String>,
    /// Count of live direct reports.
    pub direct_reports: usize,
    /// The employee's resolved scope.
    pub scope: AuthorityScope,
}

impl Designation {
    /// The designation for a **vacant** seat — §1's standing no-fabrication
    /// rule.
    ///
    /// §1: "If a department declares it requires a Platform Infrastructure head
    /// and the seat is empty, the designation for that seat renders as
    /// `'VACANT — Platform Infrastructure (unstaffed)'` rather than being
    /// omitted." It carries no name, no direct reports, and — critically — no
    /// authority sentence, because rendering one would advertise a capability
    /// nobody holds.
    pub fn vacant(department: &str) -> Self {
        Self {
            name: Self::VACANT_MARKER.to_string(),
            role: "Head of Department".to_string(),
            department: Some(department.to_string()),
            reports_to: None,
            direct_reports: 0,
            // Narrowest scope: an unstaffed seat confers nothing. A vacant seat
            // that resolved to anything wider is a fabricated capability.
            scope: AuthorityScope::Own,
        }
    }

    /// The marker §1 specifies for an unstaffed seat.
    pub const VACANT_MARKER: &'static str = "VACANT";

    /// `true` when this designation describes an unstaffed seat.
    pub fn is_vacant(&self) -> bool {
        self.name.starts_with(Self::VACANT_MARKER)
    }

    /// Render the one-line designation every other agent reads (§1).
    ///
    /// Example, filled head seat:
    /// `"Rowan Vale — Head of Department, Platform Infrastructure. Reports to:
    /// Chief Executive. Direct reports: 4. Authority: may act within own
    /// department; may not alter staffing outside it."`
    ///
    /// Example, vacant seat:
    /// `"VACANT — Platform Infrastructure (unstaffed). Authority: none; no employee
    /// holds this seat."`
    pub fn render(&self) -> String {
        if self.is_vacant() {
            return self.render_vacant();
        }

        let mut out = format!("{} — {}", self.name, self.role);
        if let Some(dept) = &self.department {
            out.push_str(&format!(", {dept}"));
        } else {
            // §3.1: a NULL department is surfaced, never silently tolerated.
            out.push_str(" (undept)");
        }
        out.push('.');
        if let Some(manager) = &self.reports_to {
            out.push_str(&format!(" Reports to: {manager}."));
        }
        out.push_str(&format!(" Direct reports: {}.", self.direct_reports));
        out.push(' ');
        out.push_str(self.scope.authority_sentence());
        out
    }

    fn render_vacant(&self) -> String {
        let dept = self.department.as_deref().unwrap_or("unknown");
        format!(
            "{marker} — {dept} (unstaffed). Authority: none; no employee holds this seat. \
             Acting authority, if any, is an explicit recorded grant, not this seat.",
            marker = Self::VACANT_MARKER,
        )
    }
}

// =====================================================================
// tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_db() -> GrantDb {
        GrantDb::from_connection(Connection::open_in_memory().expect("in-memory")).expect("open")
    }

    fn live_grant(grantee: &str, dept: Option<&str>) -> Grant {
        Grant::new(
            "emp-ceo",
            grantee,
            "platform_infra.tooling",
            AuthorityScope::Department,
            dept.map(str::to_string),
            "cross-department tooling access for the cutover",
            None,
        )
    }

    // ------------------------------------------------------------- lattice

    #[test]
    fn lattice_orders_all_six_scopes_by_containment() {
        // Ascending narrowest-first, exactly §2.2's table order.
        let ordered = AuthorityScope::ALL;
        for (i, narrow) in ordered.iter().enumerate() {
            for wider in &ordered[i + 1..] {
                assert!(narrow <= wider, "{narrow} should be contained by {wider}");
                assert!(
                    wider.contains(*narrow),
                    "{wider}.contains({narrow}) should hold"
                );
                assert!(
                    !narrow.contains(*wider),
                    "{narrow}.contains({wider}) should not hold"
                );
            }
        }
    }

    #[test]
    fn self_contains_only_itself_and_org_contains_everything() {
        assert!(AuthorityScope::Own.contains(AuthorityScope::Own));
        assert!(!AuthorityScope::Own.contains(AuthorityScope::Peers));
        for scope in AuthorityScope::ALL {
            assert!(
                AuthorityScope::Org.contains(scope),
                "Org must contain {scope}"
            );
        }
    }

    #[test]
    fn department_contains_peers_but_not_direct_reports() {
        assert!(AuthorityScope::Department.contains(AuthorityScope::Peers));
        assert!(AuthorityScope::Department.contains(AuthorityScope::Own));
        assert!(!AuthorityScope::Department.contains(AuthorityScope::DirectReports));
        assert!(AuthorityScope::DirectReports.contains(AuthorityScope::Department));
        assert!(AuthorityScope::Descendants.contains(AuthorityScope::DirectReports));
    }

    #[test]
    fn scope_display_and_from_str_round_trip() {
        for scope in AuthorityScope::ALL {
            assert_eq!(scope.to_string(), scope.as_str());
            assert_eq!(
                scope
                    .as_str()
                    .parse::<AuthorityScope>()
                    .expect("round trip"),
                scope
            );
            assert_eq!(
                scope
                    .as_str()
                    .to_uppercase()
                    .parse::<AuthorityScope>()
                    .expect("ci"),
                scope,
                "parsing must be case-insensitive"
            );
            assert_eq!(
                format!("  {scope}  ")
                    .parse::<AuthorityScope>()
                    .expect("trimmed"),
                scope,
                "parsing must trim surrounding whitespace"
            );
        }
    }

    #[test]
    fn direct_reports_parses_its_several_spellings() {
        for spelling in ["direct_reports", "directreports", "direct-reports"] {
            assert_eq!(
                spelling.parse::<AuthorityScope>().expect("spelling"),
                AuthorityScope::DirectReports
            );
        }
    }

    #[test]
    fn serde_uses_the_section_2_2_names_not_the_rust_variant_names() {
        // Guards the `Own` -> `Self` spelling difference at the boundary, so a
        // future `rename_all` change cannot silently rename a stored scope.
        let expected = [
            "self",
            "peers",
            "department",
            "direct_reports",
            "descendants",
            "org",
        ];
        for (scope, name) in AuthorityScope::ALL.iter().zip(expected) {
            assert_eq!(
                serde_json::to_string(scope).expect("serialize"),
                format!("\"{name}\""),
                "wire name for {scope:?}"
            );
            assert_eq!(
                serde_json::from_str::<AuthorityScope>(&format!("\"{name}\""))
                    .expect("deserialize"),
                *scope
            );
        }
    }

    #[test]
    fn unknown_scope_is_rejected_not_defaulted() {
        let err = "supervisor"
            .parse::<AuthorityScope>()
            .expect_err("must reject");
        assert!(err.to_string().contains("supervisor"), "unhelpful: {err}");
    }

    #[test]
    fn only_org_scope_crosses_a_department_boundary() {
        assert!(AuthorityScope::Org.is_cross_department());
        for scope in [
            AuthorityScope::Own,
            AuthorityScope::Peers,
            AuthorityScope::Department,
            AuthorityScope::DirectReports,
            AuthorityScope::Descendants,
        ] {
            assert!(!scope.is_cross_department(), "{scope} must not cross depts");
        }
    }

    #[test]
    fn resolve_scope_follows_the_section_2_2_precedence() {
        assert_eq!(
            resolve_scope(false, false),
            AuthorityScope::Peers,
            "an ordinary employee is Peers + Self, so Peers is the ceiling"
        );
        assert_eq!(
            resolve_scope(true, false),
            AuthorityScope::Descendants,
            "a department head adds Descendants"
        );
        assert_eq!(
            resolve_scope(false, true),
            AuthorityScope::Org,
            "§8.3's CEO is an ordinary row whose Org scope is explicit"
        );
        assert_eq!(
            resolve_scope(true, true),
            AuthorityScope::Org,
            "an org lead outranks a department head"
        );
    }

    // ------------------------------------------------------- can_post_to

    #[test]
    fn post_to_own_department_is_allowed_without_a_grant() {
        let check = can_post_to(
            AuthorityScope::Peers,
            Some("platform_infra"),
            &Recipient::Dept("platform_infra".to_string()),
            &[],
        );
        assert!(check.is_allowed(), "reason: {}", check.reason);
        assert_eq!(check.required_scope, AuthorityScope::Department);
    }

    #[test]
    fn post_to_self_is_always_allowed() {
        let check = can_post_to(
            AuthorityScope::Own,
            None,
            &Recipient::Agent("emp-1".to_string()),
            &[],
        );
        assert!(check.is_allowed(), "reason: {}", check.reason);
    }

    #[test]
    fn cross_department_post_is_denied_without_a_grant() {
        let check = can_post_to(
            AuthorityScope::Descendants,
            Some("platform_infra"),
            &Recipient::Dept("content".to_string()),
            &[],
        );
        assert!(!check.is_allowed(), "a grant is the only crossing");
        assert_eq!(
            check.required_scope,
            AuthorityScope::Org,
            "the reason must name the scope that would permit it"
        );
        assert!(
            check.reason.contains("content"),
            "unhelpful: {}",
            check.reason
        );
    }

    #[test]
    fn cross_department_post_is_allowed_with_a_matching_grant() {
        let grants = vec![live_grant("emp-head", Some("content"))];
        let check = can_post_to(
            AuthorityScope::Descendants,
            Some("platform_infra"),
            &Recipient::Dept("content".to_string()),
            &grants,
        );
        assert!(check.is_allowed(), "reason: {}", check.reason);
        assert!(check.reason.contains("ag_"), "the grant must be cited");
    }

    #[test]
    fn a_grant_scoped_to_another_department_does_not_carry() {
        let grants = vec![live_grant("emp-head", Some("content"))];
        let check = can_post_to(
            AuthorityScope::Descendants,
            Some("platform_infra"),
            &Recipient::Dept("research".to_string()),
            &grants,
        );
        assert!(!check.is_allowed(), "target_dept must actually match");
    }

    #[test]
    fn a_grant_with_no_target_dept_covers_any_department() {
        let grants = vec![live_grant("emp-head", None)];
        let check = can_post_to(
            AuthorityScope::Peers,
            Some("platform_infra"),
            &Recipient::Dept("anything_at_all".to_string()),
            &grants,
        );
        assert!(
            check.is_allowed(),
            "NULL target_dept means any: {}",
            check.reason
        );
    }

    #[test]
    fn global_selectors_are_allowed_at_any_scope() {
        for target in [
            Recipient::Broadcast,
            Recipient::Role("content.read".to_string()),
            Recipient::Team("infra".to_string()),
        ] {
            let check = can_post_to(AuthorityScope::Own, Some("platform_infra"), &target, &[]);
            assert!(
                check.is_allowed(),
                "{} should be open: {}",
                target.selector(),
                check.reason
            );
        }
    }

    #[test]
    fn a_departmentless_actor_cannot_reach_a_named_department() {
        // `actor_dept: None` never equals a `Some(slug)`, so an unstaffed
        // department tag (§3.1) does not accidentally become a free pass into
        // the first department that happens to be addressed.
        let check = can_post_to(
            AuthorityScope::Peers,
            None,
            &Recipient::Dept("platform_infra".to_string()),
            &[],
        );
        assert!(!check.is_allowed(), "reason: {}", check.reason);
    }

    // ------------------------------------------------ can_accept_decision

    #[test]
    fn decision_acceptance_follows_containment() {
        let cases = [
            (AuthorityScope::Org, AuthorityScope::Descendants, true),
            (AuthorityScope::Org, AuthorityScope::Org, true),
            (
                AuthorityScope::Descendants,
                AuthorityScope::Department,
                true,
            ),
            (
                AuthorityScope::Descendants,
                AuthorityScope::Descendants,
                true,
            ),
            (AuthorityScope::Descendants, AuthorityScope::Org, false),
            (
                AuthorityScope::Department,
                AuthorityScope::DirectReports,
                false,
            ),
            (AuthorityScope::Peers, AuthorityScope::Department, false),
            (AuthorityScope::Own, AuthorityScope::Own, true),
        ];
        for (actor, decision, expected) in cases {
            let check = can_accept_decision(actor, decision);
            assert_eq!(
                check.allowed, expected,
                "{actor} accepting a {decision} decision: {}",
                check.reason
            );
            if !check.allowed {
                assert!(!check.reason.is_empty(), "a deny must explain itself");
            }
        }
    }

    #[test]
    fn a_denied_decision_carries_a_failing_into_result() {
        let check = can_accept_decision(AuthorityScope::Department, AuthorityScope::Org);
        let err = check.clone().into_result().expect_err("must refuse");
        assert!(format!("{err}").contains("org"), "context lost: {err}");
        assert_eq!(check.required_scope, AuthorityScope::Department);
    }

    // ------------------------------------------------------------- GrantDb

    #[test]
    fn insert_then_list_round_trips() {
        let db = mem_db();
        let grant = live_grant("emp-1", Some("content"));
        db.insert(&grant).expect("insert");

        let listed = db.list_for_grantee("emp-1").expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], grant, "every field must round-trip");
        assert_eq!(listed[0].scope, AuthorityScope::Department);
        assert!(listed[0].grant_id.starts_with("ag_"), "id prefix per §2.5");
    }

    #[test]
    fn blank_reason_is_rejected_at_the_store_boundary() {
        let db = mem_db();
        for reason in ["", "   ", "\t\n "] {
            let grant = Grant::new(
                "emp-ceo",
                "emp-1",
                "content.read",
                AuthorityScope::Department,
                None,
                reason,
                None,
            );
            let err = db
                .insert(&grant)
                .expect_err("blank reason must be rejected");
            assert!(
                format!("{err}").contains("reason"),
                "unhelpful error: {err}"
            );
            assert_eq!(db.count().expect("count"), 0, "nothing may be written");
        }
    }

    #[test]
    fn blank_revocation_reason_is_rejected() {
        let db = mem_db();
        let grant = live_grant("emp-1", None);
        db.insert(&grant).expect("insert");
        for reason in ["", "  "] {
            let err = db
                .revoke(&grant.grant_id, reason)
                .expect_err("blank revocation must be rejected");
            assert!(
                format!("{err}").contains("revocation reason"),
                "unhelpful error: {err}"
            );
        }
        assert!(
            db.list_for_grantee("emp-1")
                .expect("list")
                .first()
                .is_some_and(|g| !g.is_revoked()),
            "a refused revocation must not have taken effect"
        );
    }

    #[test]
    fn revoking_removes_the_grant_from_the_live_list_but_keeps_it_in_history() {
        let db = mem_db();
        let grant = live_grant("emp-1", Some("content"));
        db.insert(&grant).expect("insert");

        assert!(
            db.revoke(&grant.grant_id, "cutover complete")
                .expect("revoke")
        );
        assert!(
            db.list_for_grantee("emp-1").expect("list").is_empty(),
            "a revoked grant confers nothing"
        );

        let history = db.history_for_grantee("emp-1").expect("history");
        assert_eq!(history.len(), 1, "the audit trail must survive revocation");
        assert_eq!(
            history[0].revocation_reason.as_deref(),
            Some("cutover complete")
        );
        assert!(history[0].revoked_at.is_some());
    }

    #[test]
    fn revoking_twice_is_a_noop_that_keeps_the_first_reason() {
        let db = mem_db();
        let grant = live_grant("emp-1", None);
        db.insert(&grant).expect("insert");

        assert!(db.revoke(&grant.grant_id, "first reason").expect("revoke"));
        assert!(
            !db.revoke(&grant.grant_id, "second reason").expect("revoke"),
            "a second revoke must not overwrite the first reason"
        );
        let history = db.history_for_grantee("emp-1").expect("history");
        assert_eq!(
            history[0].revocation_reason.as_deref(),
            Some("first reason")
        );
    }

    #[test]
    fn revoking_an_unknown_grant_reports_no_rows() {
        let db = mem_db();
        assert!(
            !db.revoke("ag_does_not_exist", "no such grant")
                .expect("revoke"),
            "an unknown id must not be reported as a revocation"
        );
    }

    #[test]
    fn an_expired_grant_is_absent_from_the_live_list() {
        let db = mem_db();
        let mut expired = live_grant("emp-1", None);
        expired.expires_at = Some(rfc3339(chrono::Utc::now() - chrono::Duration::days(1)));
        let live = live_grant("emp-2", None);
        db.insert(&expired).expect("insert expired");
        db.insert(&live).expect("insert live");

        let listed = db.list_for_grantee("emp-1").expect("list");
        assert!(listed.is_empty(), "a lapsed grant must confer nothing");

        // §2.5: "grants expire by default". The row is still readable, so the
        // lapse is explainable after the fact.
        let history = db.history_for_grantee("emp-1").expect("history");
        assert_eq!(history.len(), 1);
        assert!(history[0].is_lapsed_at(&rfc3339(chrono::Utc::now())));
        assert_eq!(db.count().expect("count"), 2, "expiry is not a delete");
    }

    #[test]
    fn a_future_expiry_is_still_live() {
        let db = mem_db();
        let mut grant = live_grant("emp-1", None);
        grant.expires_at = Some(rfc3339(chrono::Utc::now() + chrono::Duration::days(30)));
        db.insert(&grant).expect("insert");
        assert_eq!(db.list_for_grantee("emp-1").expect("list").len(), 1);
    }

    #[test]
    fn a_null_expiry_never_lapses() {
        let db = mem_db();
        let grant = live_grant("emp-1", None);
        db.insert(&grant).expect("insert");
        let listed = &db.list_for_grantee("emp-1").expect("list")[0];
        assert!(
            !listed.is_lapsed_at(&rfc3339(chrono::Utc::now())),
            "None means never expires"
        );
    }

    #[test]
    fn grants_are_scoped_to_their_grantee() {
        let db = mem_db();
        db.insert(&live_grant("emp-1", None)).expect("insert a");
        db.insert(&live_grant("emp-2", None)).expect("insert b");
        let listed = db.list_for_grantee("emp-1").expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].grantee, "emp-1");
    }

    #[test]
    fn init_creates_the_parent_directory_and_the_table() {
        let dir = std::env::temp_dir().join(format!(
            "operant_authority_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("nested").join("operant_kanban.db");
        let db = GrantDb::init(path.clone()).expect("init");
        db.insert(&live_grant("emp-1", None)).expect("insert");
        assert!(Path::new(&path).exists(), "the file must be created");
        assert_eq!(db.count().expect("count"), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_is_idempotent_on_a_second_open() {
        let dir = std::env::temp_dir().join(format!(
            "operant_authority_shared_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("operant_kanban.db");
        let first = GrantDb::init(path.clone()).expect("first init");
        first.insert(&live_grant("emp-1", None)).expect("insert");
        let second = GrantDb::init(path.clone()).expect("second init");
        assert_eq!(
            second.list_for_grantee("emp-1").expect("list").len(),
            1,
            "CREATE TABLE IF NOT EXISTS must not disturb existing rows"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_scope_string_degrades_to_the_narrowest_scope() {
        // Never widen on a value this build cannot parse.
        let db = mem_db();
        let mut grant = live_grant("emp-1", None);
        grant.scope = AuthorityScope::Org;
        db.insert(&grant).expect("insert");
        db.conn()
            .lock()
            .expect("lock")
            .execute(
                "UPDATE authority_grants SET scope = 'supervisor' WHERE grant_id = ?1",
                params![grant.grant_id],
            )
            .expect("hand-edit");

        let listed = db.list_for_grantee("emp-1").expect("list");
        assert_eq!(listed.len(), 1, "the row is still readable");
        assert_eq!(
            listed[0].scope,
            AuthorityScope::Own,
            "an unknown scope must grant the least, not the most"
        );
    }

    // --------------------------------------------------------- Designation

    fn filled_designation() -> Designation {
        Designation {
            name: "Rowan Vale".to_string(),
            role: "Head of Department".to_string(),
            department: Some("Platform Infrastructure".to_string()),
            reports_to: Some("Chief Executive".to_string()),
            direct_reports: 4,
            scope: AuthorityScope::Descendants,
        }
    }

    #[test]
    fn a_filled_designation_renders_name_role_manager_reports_and_authority() {
        let rendered = filled_designation().render();
        assert!(rendered.starts_with("Rowan Vale — Head of Department, Platform Infrastructure."));
        assert!(
            rendered.contains("Reports to: Chief Executive."),
            "{rendered}"
        );
        assert!(rendered.contains("Direct reports: 4."), "{rendered}");
        assert!(rendered.contains("Authority:"), "{rendered}");
    }

    #[test]
    fn a_vacant_seat_renders_visibly_vacant_and_claims_no_capability() {
        let rendered = Designation::vacant("Platform Infrastructure").render();
        assert!(
            rendered.starts_with("VACANT — Platform Infrastructure (unstaffed)."),
            "§1 requires this exact shape: {rendered}"
        );
        assert!(
            rendered.contains("no employee holds this seat"),
            "{rendered}"
        );

        // §1's no-fabrication rule, made checkable: a vacant seat must not
        // advertise a scope sentence, and must not carry a person's name.
        assert!(!rendered.contains("Head of Department"), "{rendered}");
        assert!(!rendered.contains("Authority: may"), "{rendered}");
        assert_eq!(Designation::vacant("x").scope, AuthorityScope::Own);
    }

    #[test]
    fn vacancy_is_detected_by_its_marker_not_by_a_name_comparison() {
        assert!(Designation::vacant("content").is_vacant());
        assert!(!filled_designation().is_vacant());
        assert_eq!(Designation::VACANT_MARKER, "VACANT");
    }

    #[test]
    fn a_null_department_renders_the_undept_marker() {
        // §3.1: a NULL department is surfaced, never silently tolerated.
        let designation = Designation {
            department: None,
            ..filled_designation()
        };
        let rendered = designation.render();
        assert!(rendered.contains("(undept)"), "{rendered}");
        assert!(!rendered.contains(", \n"), "{rendered}");
    }

    #[test]
    fn a_designation_without_a_manager_omits_the_reports_to_clause() {
        let designation = Designation {
            reports_to: None,
            ..filled_designation()
        };
        let rendered = designation.render();
        assert!(!rendered.contains("Reports to:"), "{rendered}");
        assert!(rendered.contains("Direct reports: 4."), "{rendered}");
    }

    #[test]
    fn the_authority_sentence_follows_the_scope_not_a_hand_written_claim() {
        for scope in AuthorityScope::ALL {
            let designation = Designation {
                scope,
                ..filled_designation()
            };
            let rendered = designation.render();
            assert!(
                rendered.contains(scope.authority_sentence()),
                "scope {scope} must render its own authority sentence: {rendered}"
            );
        }
    }

    // ── Delegation posture + can_delegate (P0 governance slice) ───────────

    #[test]
    fn delegation_posture_round_trips_through_its_spellings() {
        for p in [
            DelegationPosture::Forbidden,
            DelegationPosture::Bounded,
            DelegationPosture::Independent,
        ] {
            let parsed: DelegationPosture = p.as_str().parse().expect(p.as_str());
            assert_eq!(parsed, p);
        }
        assert!("nosuch".parse::<DelegationPosture>().is_err());
    }

    #[test]
    fn can_delegate_forbidden_denies_everywhere() {
        let check = can_delegate(
            DelegationPosture::Forbidden,
            Some("platform"),
            "emp-peer",
            Some("platform"),
            &[],
        );
        assert!(!check.is_allowed());
        assert!(check.reason.contains("forbids delegation"));
    }

    #[test]
    fn can_delegate_independent_allows_without_governance() {
        let check = can_delegate(
            DelegationPosture::Independent,
            None,
            "literally-anything",
            None,
            &[],
        );
        assert!(check.is_allowed());
    }

    #[test]
    fn can_delegate_bounded_allows_own_department_without_a_grant() {
        let check = can_delegate(
            DelegationPosture::Bounded,
            Some("platform"),
            "emp-peer",
            Some("platform"),
            &[],
        );
        assert!(check.is_allowed());
        assert!(check.reason.contains("department tier"));
    }

    #[test]
    fn can_delegate_bounded_cross_department_needs_a_live_grant() {
        let denied = can_delegate(
            DelegationPosture::Bounded,
            Some("platform"),
            "emp-finance",
            Some("finance"),
            &[],
        );
        assert!(!denied.is_allowed());
        assert!(denied.reason.contains("no live grant"));

        let grants = vec![live_grant("emp-platform", Some("finance"))];
        let allowed = can_delegate(
            DelegationPosture::Bounded,
            Some("platform"),
            "emp-finance",
            Some("finance"),
            &grants,
        );
        assert!(allowed.is_allowed());
        assert!(allowed.reason.contains("covered by grant"));
    }

    #[test]
    fn can_delegate_bounded_fails_closed_on_unregistered_target() {
        let check = can_delegate(
            DelegationPosture::Bounded,
            Some("platform"),
            "some-config-alias",
            None,
            &[],
        );
        assert!(!check.is_allowed());
        assert!(check.reason.contains("fail-closed"));
    }
}

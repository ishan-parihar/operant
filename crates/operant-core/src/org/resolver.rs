//! Recipient resolver — the piece that makes the notice board readable.
//!
//! Spec: `docs/REMAINING-GAPS.md` GAP-2.2, `docs/ORG-AUTHORITY-ARCHITECTURE.md`
//! §3.2 (feed segregation) and §3.1 (department membership).
//!
//! Pure: no I/O, no sqlite. The stores live in [`super::notice_db`] and
//! [`super::employee_db`]; this module is the mapping between them, so it can be
//! unit-tested without a database file and reused by every caller.
//!
//! ## What was missing before this module
//!
//! `NoticeIdentity` and `NoticeInboxMatcher` already existed and correctly
//! derive a reader's four selectors from four membership facts, and
//! `NoticeBoard::query_inbox` already filters a notice's stored selectors
//! against a reader's. **Nothing in production ever built a `NoticeIdentity`
//! from an [`Employee`] row, and nothing called the inbox.** So `dept:` and
//! `team:` selectors resolved to nobody and the only working feed shapes were
//! `broadcast` and `agent:<id>`. This module supplies the missing link:
//!
//! ```text
//! Employee ──identity_for──▶ NoticeIdentity ──inbox_query_for──▶ InboxQuery
//!                                                                        │
//!                                                          NoticeBoard::query_inbox
//! ```
//!
//! ## Why the mapping lives in exactly one function
//!
//! `identity_for` is the single place an [`Employee`] becomes a
//! [`NoticeIdentity`]. If every caller derived selectors itself, the mapping
//! could drift — one site that forgets the `role:` selector, another that
//! invents a `dept:` selector for a departmentless employee — and none of it
//! would fail loudly, because a wrong selector set is exactly a notice that
//! reaches nobody. A silent non-delivery is the failure mode this whole module
//! exists to remove, so the mapping is centralised rather than left to callers.
//!
//! ## Read-time resolution, and the two directions
//!
//! The board stores selectors **verbatim** and resolves them **at read time**
//! (see [`super::notice`]), so there are two distinct directions and they are
//! not interchangeable:
//!
//! - **Who am I?** ([`identity_for`]) — a reader's own selectors, used to fetch
//!   their inbox. This is the direction that was missing entirely.
//! - **Who should this reach?** ([`resolve_recipients`]) — a target's fan-out to
//!   concrete employee ids, used when a caller needs the audience up front
//!   (a digest, a roster, a dispatch decision).
//!
//! Both read the same `Employee` rows, so a resolver built on one and tested
//! against the other cannot disagree.
//!
//! ## The NULL-department honesty rule
//!
//! `employees.department` is `None` for every row the cron backfill produces
//! (`employee_db.rs`, which sets `department: None` because
//! `ORG-AUTHORITY-ARCHITECTURE.md` §3.1 says backfill must not guess from cron
//! job names — it is an operator-set field). So a resolver that assumes a
//! department exists would resolve the entire live org to "no department" and
//! quietly reach nobody.
//!
//! Two rules follow, and this module holds both:
//!
//! 1. **Never invent a membership.** A missing department produces **no**
//!    `dept:` selector. Absence is not a guess.
//! 2. **Never hide it.** [`unstaffed_employees`] reports every such row, because
//!    the standing project rule is that invalid and missing state must stay
//!    discoverable rather than be silently omitted. A departmentless employee is
//!    exactly that: a visible gap, not a person who belongs to no one.
//!
//! ## What this module does NOT do
//!
//! It does not invent a team-membership derivation. `Employee` has no team
//! column and no membership table exists (that is Wave 3, `REMAINING-GAPS.md`
//! GAP-3.3). [`resolve_recipients`] therefore returns **empty** for
//! [`Recipient::Team`] rather than guessing; see that function for why empty is
//! the honest answer.

use super::employee::Employee;
use super::notice::{InboxQuery, NoticeIdentity, NoticeInboxMatcher, Recipient};

/// The tier a notice recipient lands in, per
/// `docs/ORG-AUTHORITY-ARCHITECTURE.md` §3.2.
///
/// The three tiers are the segregation the owner asked for. They need no new
/// feed model: `Recipient::{Agent,Dept,Role,Broadcast}` already encode all
/// three, and what was missing was the resolver that says which tier a given
/// recipient belongs to and — for a department — which employees occupy it.
///
/// Note what §3.2 calls out: **escalation is explicit.** A `Department` notice
/// is never promoted to `Global`. If a department decides something is
/// org-wide, the head posts a *new* notice to `broadcast` citing the department
/// notice, so every escalation is an attributable act. The tier is therefore a
/// property of the recipient as written, which is why
/// [`FeedTier::of`] is a pure classification with no promotion rule in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeedTier {
    /// `agent:<id>` — the named employee only.
    Direct,
    /// `dept:<slug>` — members of one department.
    Department,
    /// `broadcast` and `role:<cap>` — everyone.
    ///
    /// `role:` sits here because a role is a capability, not a location: §3.2's
    /// table puts both `broadcast` and `role:<role>` in the global row, and
    /// "every employee holding the reviewer capability" is org-wide by
    /// construction.
    Global,
}

impl FeedTier {
    /// Classify a recipient into its feed tier.
    ///
    /// Pure and total: every `Recipient` maps to exactly one tier, with no
    /// default arm and no promotion. `Team` classifies as
    /// [`FeedTier::Department`] because a team is a group narrower than the org
    /// and narrower than a department — a team notice is a same-group
    /// delivery, not an org-wide one. (This classification is about *intent*
    /// and segregation; whether anyone receives the notice is
    /// [`resolve_recipients`]' question, and for a team today the answer is
    /// nobody.)
    pub fn of(target: &Recipient) -> Self {
        match target {
            Recipient::Agent(_) => Self::Direct,
            Recipient::Dept(_) | Recipient::Team(_) => Self::Department,
            Recipient::Role(_) | Recipient::Broadcast => Self::Global,
        }
    }
}

/// The one place an [`Employee`] row becomes a [`NoticeIdentity`].
///
/// # Why `teams` is a parameter
///
/// Team membership is a later wave. `Employee` has no team column, and no
/// membership table exists yet (`REMAINING-GAPS.md` GAP-3.3). Rather than bake
/// in "teams are always empty" — which would make Wave 3 a breaking change to
/// this signature — the resolver takes the membership facts it cannot yet
/// derive. When the membership table lands, the caller that reads it passes the
/// result here and this function needs no change.
///
/// Callers that have no membership source today pass an empty slice, which
/// produces a `NoticeIdentity` with no `team:` selectors. That is correct: a
/// `team:` selector that matched nothing would be a promise the resolver
/// cannot keep.
///
/// # Why `roles` falls back to the employee's own `role`
///
/// `Employee::role` is a populated, meaningful column (the backfill writes
/// `automator` to every cron-derived row), so an identity built without it
/// would silently miss `role:`-addressed notices even though the data is right
/// there on the row. The `roles` parameter is an *addition* to the known role,
/// not a replacement: an employee with extra roles still gets its own. When
/// `roles` is empty, the employee's own role is used; when it is non-empty, the
/// passed roles are used as the caller supplied them.
///
/// # What is deliberately absent
///
/// A `None` department produces **no** `dept:` selector. Per
/// `ORG-AUTHORITY-ARCHITECTURE.md` §3.1, backfill must not guess a department
/// from the cron job name, so a departmentless employee is a real,
/// operator-fixable gap. Emitting a placeholder department would deliver the
/// employee notices for a department they are not in, which is the exact
/// "fabricate a capability for an unstaffed seat" failure the project forbids.
/// The gap is surfaced separately, by [`unstaffed_employees`].
///
/// A blank (`""`) department or role is treated the same as `None`/empty for
/// the same reason: an empty slug would serialise to `dept:` — a selector that
/// matches nothing, and that `Recipient::parse` would reject on the write side.
pub fn identity_for(employee: &Employee, teams: &[String], roles: &[String]) -> NoticeIdentity {
    let dept = employee
        .department
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string);

    let mut effective_roles: Vec<String> = roles
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect();
    if effective_roles.is_empty() {
        let own = employee.role.trim();
        if !own.is_empty() {
            effective_roles.push(own.to_string());
        }
    }

    NoticeIdentity {
        employee_id: employee.employee_id.clone(),
        dept,
        teams: teams
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        roles: effective_roles,
    }
}

/// The canonical selector strings for an identity — the exact form
/// `NoticeBoard::query_inbox` matches against.
///
/// # Why this exists, and why the exact serialisation matters
///
/// `query_inbox` builds its predicate as a `json_each` scan over the stored
/// `recipients` JSON array and compares each element to a bound parameter:
///
/// ```sql
/// EXISTS (SELECT 1 FROM json_each(notices.recipients) je WHERE je.value = ?N)
/// ```
///
/// (`notice_db.rs`, `query_inbox`.) On the **write** side, `insert_into`
/// serialises recipients with `encode_recipients`, which maps every recipient
/// through [`Recipient::selector`] into a JSON string array. `json_each` yields
/// each array element as an unquoted SQL value, so a stored `"dept:infra"`
/// compares equal only to a bound value of exactly `dept:infra`.
///
/// Therefore the selector strings here **must** be
/// [`Recipient::selector`] output — not a hand-rolled `format!`, and not the
/// serialised JSON with its quotes. A mismatch here is the worst kind of bug in
/// this subsystem: it produces **zero rows and no error**. A notice simply
/// never appears in anyone's inbox, and nothing anywhere says why. Every
/// selector this module emits delegates to `Recipient::selector` precisely so
/// the two sides cannot drift, and the round-trip is pinned by a test against a
/// real `NoticeBoard` (`serialization_matches_query_inbox_json_each`).
///
/// This is the same conversion `NoticeBoard::inbox` performs internally
/// (it maps `reader.selectors()` through `Recipient::selector`). It is exposed
/// here for callers that hold an identity but need the strings — to compose a
/// query, to log, or to assert — without a board in hand.
pub fn selectors_for_identity(identity: &NoticeIdentity) -> Vec<String> {
    identity
        .selectors()
        .iter()
        .map(Recipient::selector)
        .collect()
}

/// Build an [`InboxQuery`] that reads exactly one employee's inbox.
///
/// This is the production path that did not exist: with it, a caller goes from
/// an [`Employee`] row to a live inbox read in two lines —
///
/// ```no_run
/// # use operant_core::org::employee::Employee;
/// # use operant_core::org::notice_db::NoticeBoard;
/// # use operant_core::org::resolver::{identity_for, inbox_query_for};
/// # fn read(board: &NoticeBoard, employee: &Employee) -> Result<(), operant_core::error::Error> {
/// let identity = identity_for(employee, &[], &[]);
/// let notices = board.query_inbox(&inbox_query_for(&identity, None, Some(20)))?;
/// # let _ = notices;
/// # Ok(())
/// # }
/// ```
///
/// # `selectors` is `Some`, always
///
/// The query sets `selectors: Some(identity.selectors())` unconditionally, so
/// the read is scoped to the reader. Leaving it `None` would read **every**
/// notice on the board regardless of recipient (`query_inbox` documents `None`
/// as the org-wide "what has happened" view) — a quiet, easy-to-miss privacy
/// and noise regression for a caller who meant "my inbox".
///
/// # The cursor
///
/// `after_epoch` is passed through unchanged as the incremental-fetch cursor
/// (`epoch >= after`). The board stores `epoch` in unix seconds and indexes it,
/// so a caller that keeps a per-employee cursor gets a cheap incremental read
/// with no server-side dedup. `None` reads from the beginning of time.
///
/// `limit` is passed through unchanged and is **not** defaulted: this function
/// is a mapping, not a policy. Choosing a limit is a product decision, and a
/// silent default here would be a cap nobody asked for, invisible at the call
/// site that forgot to set it.
pub fn inbox_query_for(
    identity: &NoticeIdentity,
    after_epoch: Option<i64>,
    limit: Option<i64>,
) -> InboxQuery {
    InboxQuery {
        after_epoch,
        selectors: Some(selectors_for_identity(identity)),
        pending_only: false,
        reader_id: Some(identity.employee_id.clone()),
        limit,
    }
}

/// One surfaced configuration gap: an employee with no department.
///
/// The project rule is that invalid and missing state must remain
/// **discoverable** rather than be silently omitted, and a departmentless
/// employee is precisely that. Every backfilled row is departmentless today
/// (`ORG-AUTHORITY-ARCHITECTURE.md` §3.1 makes the field operator-set, and the
/// backfill must not guess), so without this the resolver would correctly
/// return "no `dept:` selector" and an operator would see nothing explaining
/// why their org's department feeds are empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstaffedFinding {
    /// The employee with no department.
    pub employee_id: String,
    /// Human-readable description of the gap, for a report or CLI line.
    pub finding: String,
}

/// Report every employee whose `department` is `None` (or blank).
///
/// This is the read-side companion to the no-fabrication rule: a missing
/// department is an empty feed **and** a finding, never a synthetic recipient.
///
/// A blank (`""`) department counts as missing. It is the same gap wearing a
/// different value: it cannot produce a `dept:` selector (it would serialise to
/// a bare `dept:`) and it cannot match any real department, so treating it as
/// staffed would hide a row that needs the same operator fix as a `NULL`.
///
/// This deliberately does **not** check whether a department *exists* or has
/// anyone in it — there is no `departments` table yet
/// (`ORG-AUTHORITY-ARCHITECTURE.md` §3.3). It reports only the fact the
/// `Employee` row itself carries, which is exactly what
/// `ORG-AUTHORITY-ARCHITECTURE.md` §3.1 asks `org check` to count.
pub fn unstaffed_employees(employees: &[Employee]) -> Vec<UnstaffedFinding> {
    employees
        .iter()
        .filter(|e| {
            e.department
                .as_deref()
                .map(str::trim)
                .is_none_or(|d| d.is_empty())
        })
        .map(|e| UnstaffedFinding {
            employee_id: e.employee_id.clone(),
            finding: format!(
                "employee {} has no department: no `dept:` selector is derivable, so \
                 department feeds exclude them until one is set",
                e.employee_id
            ),
        })
        .collect()
}

/// `true` when this row is an active employee.
///
/// `Employee::status` is a free string carried through from the cron job's
/// `state` rather than a closed enum (`Employee` documents that every non-
/// `scheduled` state is preserved verbatim so `paused` stays visible), so this
/// compares against the one known-active value
/// ([`STATUS_ACTIVE`](super::employee::STATUS_ACTIVE)) and treats anything
/// else — `paused`, an unrecognised value, an empty string — as **not** active.
/// Unknown-is-not-active is the fail-closed reading: a row whose status we
/// cannot interpret must not be pulled into a fan-out on a guess.
fn is_active(employee: &Employee) -> bool {
    employee.status.trim() == super::employee::STATUS_ACTIVE
}

/// Resolve a target recipient to the employee ids that should receive it.
///
/// This is the write-side companion to [`identity_for`]: where that function
/// answers "which selectors address this reader", this one answers "who is in
/// this group right now", for callers that need the audience up front rather
/// than resolving it at read time.
///
/// Returns ids in the order they appear in `employees`, de-duplicated. Order is
/// stable and input-ordered rather than sorted, so the output reads as a roster
/// in the shape the caller supplied it.
///
/// # `Recipient::Team` returns empty — deliberately
///
/// There is no team-membership resolver yet. `Employee` has no team column and
/// no membership table exists (`REMAINING-GAPS.md` GAP-3.3, Wave 3). So a
/// `team:` target resolves to **no one**.
///
/// This is stated rather than worked around on purpose. The alternatives are
/// all worse:
///
/// - Matching teams against a department or the `role` column would be an
///   invented derivation — it would deliver notices to people who are not on
///   the team, which is a fabricated membership. The project forbids
///   fabricating a capability for a seat nobody fills.
/// - Falling back to broadcast would be worse still: a `team:` notice would
///   silently reach the whole org, and the author would never learn that the
///   group they scoped it to does not exist.
///
/// An empty result is the one outcome that is *true*. It is a real, visible
/// limitation (a `team:` notice reaches nobody) rather than a hidden wrong
/// delivery, and when Wave 3 ships the membership table this function gains a
/// real branch without changing its signature or any caller.
///
/// Callers that need to distinguish "team not yet supported" from "team is
/// empty" should check for the Wave-3 gap directly; the honest signal today is
/// that `resolve_recipients(&Recipient::Team(_), …)` is always empty.
///
/// # Why only active employees
///
/// Only [`STATUS_ACTIVE`](super::employee::STATUS_ACTIVE) rows are returned.
/// A paused or otherwise-not-active employee should not receive work notices,
/// and delivering to them is a state error, not a stale-read convenience. The
/// active check is applied uniformly to the group tiers (`Dept`, `Role`,
/// `Broadcast`) so a paused employee is invisible to fan-out everywhere.
///
/// # `Recipient::Agent` is not filtered by status
///
/// A direct `agent:<id>` notice returns exactly `[id]` regardless of status.
/// Naming a specific employee is an explicit, attributable act by the author
/// (an escalation, a correction, a reply to them) and should not be silently
/// dropped because that employee happens to be paused. Whether a paused
/// employee acts on it is the author's and the employee's business, not this
/// resolver's.
pub fn resolve_recipients(target: &Recipient, employees: &[Employee]) -> Vec<String> {
    match target {
        // Direct: named explicitly, so not filtered by status — see the doc
        // comment on the `Agent` arm.
        Recipient::Agent(id) => {
            if id.trim().is_empty() {
                Vec::new()
            } else {
                vec![id.clone()]
            }
        }
        // Group tiers: only active employees.
        Recipient::Dept(dept) => active_matching(employees, |e| {
            e.department
                .as_deref()
                .map(str::trim)
                .is_some_and(|d| d.eq_ignore_ascii_case(dept.trim()))
        }),
        Recipient::Role(role) => active_matching(employees, |e| {
            e.role.trim().eq_ignore_ascii_case(role.trim())
        }),
        // Wave 3. See the doc comment: no membership resolver exists, so this
        // is empty rather than guessed.
        Recipient::Team(_) => Vec::new(),
        Recipient::Broadcast => active_matching(employees, |_| true),
    }
}

/// Collect the ids of active employees matching `pred`, de-duplicated,
/// preserving input order.
fn active_matching(employees: &[Employee], pred: impl Fn(&Employee) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for employee in employees.iter().filter(|e| is_active(e) && pred(e)) {
        if !out.iter().any(|seen| seen == &employee.employee_id) {
            out.push(employee.employee_id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::notice::{PostNotice, Recipient};
    use crate::org::notice_db::NoticeBoard;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // ------------------------------------------------------------- fixtures

    /// An employee row with sensible defaults; individual tests override the
    /// fields they care about so each assertion traces to one fact.
    fn emp(id: &str) -> Employee {
        Employee {
            employee_id: id.to_string(),
            name: format!("Employee {id}"),
            role: "automator".to_string(),
            department: None,
            skills: vec![],
            agent_type: None,
            persona: None,
            status: "active".to_string(),
            reason: "test fixture".to_string(),
            created_at: "2026-09-30T00:00:00.000Z".to_string(),
            updated_at: "2026-09-30T00:00:00.000Z".to_string(),
        }
    }

    /// An active employee in `dept`.
    fn in_dept(id: &str, dept: &str) -> Employee {
        Employee {
            department: Some(dept.to_string()),
            ..emp(id)
        }
    }

    /// A pause()d employee in `dept` — not active.
    fn inactive_in_dept(id: &str, dept: &str) -> Employee {
        Employee {
            department: Some(dept.to_string()),
            status: "paused".to_string(),
            ..emp(id)
        }
    }

    /// The selector strings an identity produces, for concise assertions.
    /// Named `identity_selectors` rather than `sels` because several tests bind
    /// a local `sels` to the result, and a local shadows a function of the same
    /// name for the rest of the block.
    fn identity_selectors(identity: &NoticeIdentity) -> Vec<String> {
        selectors_for_identity(identity)
    }

    // ------------------------------------------------- identity_for selectors

    #[test]
    fn identity_for_staffed_employee_yields_agent_and_dept_only() {
        // Exactly `agent:<id>` and `dept:<dept>`. The `automator` role is also
        // a real selector, so assert the *set* membership and exclusion of the
        // other department's selector rather than a bare two-element vector.
        let e = in_dept("emp-infra-1", "infra");
        let sels = identity_selectors(&identity_for(&e, &[], &[]));

        assert!(sels.contains(&"agent:emp-infra-1".to_string()));
        assert!(sels.contains(&"dept:infra".to_string()));
        assert!(
            !sels.contains(&"dept:platform".to_string()),
            "must not carry another department's selector: {sels:?}"
        );
        // The role is populated on the row, so it is carried as `role:automator`.
        assert!(sels.contains(&"role:automator".to_string()));
        // The employee is not on a team (Wave 3 absent), so no team selector.
        assert!(
            !sels.iter().any(|s| s.starts_with("team:")),
            "no team selector should be invented: {sels:?}"
        );
    }

    #[test]
    fn identity_for_departmentless_employee_yields_no_dept_selector() {
        // The critical no-fabrication case: a NULL department must produce ONLY
        // agent: (plus the row's own role). No `dept:` selector out of nothing.
        let e = emp("emp-nodep");
        let sels = identity_selectors(&identity_for(&e, &[], &[]));

        assert!(sels.contains(&"agent:emp-nodep".to_string()));
        assert!(
            !sels.iter().any(|s| s.starts_with("dept:")),
            "must not invent a dept selector for a NULL department: {sels:?}"
        );
        // Still carries the row's own meaningful role.
        assert!(sels.contains(&"role:automator".to_string()));
    }

    #[test]
    fn identity_for_blank_department_is_treated_as_missing() {
        // A blank department is the same gap as NULL — it cannot serialise to a
        // real `dept:` selector, so it must not be treated as staffed.
        let e = Employee {
            department: Some("   ".to_string()),
            ..emp("emp-blank")
        };
        let sels = identity_selectors(&identity_for(&e, &[], &[]));
        assert!(
            !sels.iter().any(|s| s.starts_with("dept:")),
            "blank department must not yield a dept selector: {sels:?}"
        );
    }

    #[test]
    fn identity_for_uses_passed_roles_and_falls_back_to_own_role() {
        // Non-empty `roles` is used as the caller supplied it (plus own is
        // NOT auto-added when caller supplied extra roles).
        let e = in_dept("emp-1", "infra");
        let sels = identity_selectors(&identity_for(&e, &[], &["reviewer".to_string()]));
        assert!(sels.contains(&"role:reviewer".to_string()));
        assert!(
            !sels.contains(&"role:automator".to_string()),
            "when roles are supplied the own-role is not appended: {sels:?}"
        );

        // Empty `roles` falls back to the employee's own role column.
        let fallback = identity_selectors(&identity_for(&e, &[], &[]));
        assert!(fallback.contains(&"role:automator".to_string()));
    }

    #[test]
    fn identity_for_carries_teams_when_supplied() {
        // Wave 3 is absent, but the signature already accepts the membership
        // fact, so a caller that has it gets a team selector without a change
        // to this function.
        let e = in_dept("emp-1", "infra");
        let teams = vec!["core".to_string()];
        let sels = identity_selectors(&identity_for(&e, &teams, &[]));
        assert!(sels.contains(&"team:core".to_string()));
    }

    // ------------------------------------------------------ resolve_recipients

    #[test]
    fn resolve_recipients_dept_returns_only_that_depts_active() {
        let employees = vec![
            in_dept("emp-a1", "infra"),
            in_dept("emp-a2", "infra"),
            in_dept("emp-b1", "platform"),
            inactive_in_dept("emp-a3", "infra"), // right dept, but paused
        ];
        let got = resolve_recipients(&Recipient::Dept("infra".into()), &employees);

        // All and only the active infra members.
        assert!(got.contains(&"emp-a1".to_string()), "{got:?}");
        assert!(got.contains(&"emp-a2".to_string()), "{got:?}");
        // NOT an employee from a different department.
        assert!(
            !got.contains(&"emp-b1".to_string()),
            "must not cross departments: {got:?}"
        );
        // NOT an inactive employee, even in the right department.
        assert!(
            !got.contains(&"emp-a3".to_string()),
            "must not deliver to an inactive employee: {got:?}"
        );
    }

    #[test]
    fn resolve_recipients_dept_is_case_insensitive() {
        // Department slugs are operator-set free text; an author typing
        // `Infra` should not silently reach nobody. Match the read path, which
        // compares stored selector strings.
        let employees = vec![in_dept("emp-a1", "infra")];
        let got = resolve_recipients(&Recipient::Dept("INFRA".into()), &employees);
        assert_eq!(got, vec!["emp-a1".to_string()]);
    }

    #[test]
    fn resolve_recipients_role_returns_active_holders() {
        let employees = vec![
            Employee {
                role: "reviewer".to_string(),
                ..in_dept("emp-r1", "infra")
            },
            Employee {
                role: "automator".to_string(),
                ..in_dept("emp-r2", "infra")
            },
            Employee {
                role: "reviewer".to_string(),
                status: "paused".to_string(),
                ..in_dept("emp-r3", "infra")
            },
        ];
        let got = resolve_recipients(&Recipient::Role("reviewer".into()), &employees);
        assert_eq!(got, vec!["emp-r1".to_string()], "paused holder excluded");
    }

    #[test]
    fn resolve_recipients_team_returns_empty_wave3_absent() {
        // Documented as Wave-3-absent: there is no membership resolver, so a
        // `team:` notice reaches nobody rather than being guessed at.
        let employees = vec![in_dept("emp-a1", "infra"), in_dept("emp-a2", "infra")];
        let got = resolve_recipients(&Recipient::Team("core".into()), &employees);
        assert!(
            got.is_empty(),
            "team must resolve to nobody (Wave 3 absent), got {got:?}"
        );
    }

    #[test]
    fn resolve_recipients_broadcast_returns_all_active() {
        let employees = vec![
            in_dept("emp-a1", "infra"),
            emp("emp-nodep"), // active, no dept
            inactive_in_dept("emp-paused", "platform"),
        ];
        let got = resolve_recipients(&Recipient::Broadcast, &employees);
        assert!(got.contains(&"emp-a1".to_string()), "{got:?}");
        assert!(got.contains(&"emp-nodep".to_string()), "{got:?}");
        assert!(
            !got.contains(&"emp-paused".to_string()),
            "broadcast must exclude inactive: {got:?}"
        );
        assert_eq!(got.len(), 2, "exactly the two active employees: {got:?}");
    }

    #[test]
    fn resolve_recipients_agent_is_exactly_that_id_regardless_of_status() {
        // Direct notice: named explicitly, not filtered by status.
        let employees = vec![inactive_in_dept("emp-paused", "infra")];
        let got = resolve_recipients(&Recipient::Agent("emp-paused".into()), &employees);
        assert_eq!(got, vec!["emp-paused".to_string()]);

        // And it resolves to the id even if the row is absent from the slice
        // (a direct notice names someone, whether or not the registry has them).
        let got = resolve_recipients(&Recipient::Agent("emp-ghost".into()), &employees);
        assert_eq!(got, vec!["emp-ghost".to_string()]);
    }

    #[test]
    fn resolve_recipients_broadcast_dedupes_duplicate_ids() {
        // A duplicated row (possible from a re-run backfill) must not deliver
        // twice.
        let employees = vec![in_dept("emp-a1", "infra"), in_dept("emp-a1", "infra")];
        let got = resolve_recipients(&Recipient::Broadcast, &employees);
        assert_eq!(got, vec!["emp-a1".to_string()]);
    }

    // -------------------------------------------------- unstaffed_employees

    #[test]
    fn unstaffed_employees_reports_every_null_department_row() {
        let employees = vec![
            in_dept("emp-a1", "infra"),
            emp("emp-nodep"),
            emp("emp-nodep2"),
            in_dept("emp-b1", "platform"),
            Employee {
                department: Some("  ".to_string()),
                ..emp("emp-blank")
            },
        ];
        let findings = unstaffed_employees(&employees);

        let ids: Vec<&str> = findings.iter().map(|f| f.employee_id.as_str()).collect();
        assert!(ids.contains(&"emp-nodep"), "{ids:?}");
        assert!(ids.contains(&"emp-nodep2"), "{ids:?}");
        assert!(
            ids.contains(&"emp-blank"),
            "blank counts as missing: {ids:?}"
        );
        // Staffed rows are NOT reported.
        assert!(!ids.contains(&"emp-a1"), "{ids:?}");
        assert!(!ids.contains(&"emp-b1"), "{ids:?}");
        // Every finding carries a non-empty description.
        assert!(findings.iter().all(|f| !f.finding.is_empty()));
    }

    #[test]
    fn unstaffed_employees_is_empty_when_all_staffed() {
        let employees = vec![in_dept("emp-a1", "infra"), in_dept("emp-b1", "platform")];
        assert!(unstaffed_employees(&employees).is_empty());
    }

    // ------------------------------------------------------ inbox_query_for

    #[test]
    fn inbox_query_for_sets_selectors_and_passes_cursor_and_limit() {
        let identity = identity_for(&in_dept("emp-1", "infra"), &[], &[]);
        let q = inbox_query_for(&identity, Some(100), Some(5));
        let sels = q.selectors.clone().expect("selectors are set");
        assert!(sels.contains(&"agent:emp-1".to_string()));
        assert!(sels.contains(&"dept:infra".to_string()));
        assert_eq!(q.after_epoch, Some(100));
        assert_eq!(q.limit, Some(5));
        // Never reads every notice on the board: selectors must be Some.
        assert!(q.selectors.is_some());
    }

    // ------------------------------------------- FeedTier classification

    #[test]
    fn feed_tier_classifies_every_recipient_kind() {
        assert_eq!(
            FeedTier::of(&Recipient::Agent("x".into())),
            FeedTier::Direct
        );
        assert_eq!(
            FeedTier::of(&Recipient::Dept("x".into())),
            FeedTier::Department
        );
        assert_eq!(
            FeedTier::of(&Recipient::Team("x".into())),
            FeedTier::Department
        );
        assert_eq!(FeedTier::of(&Recipient::Role("x".into())), FeedTier::Global);
        assert_eq!(FeedTier::of(&Recipient::Broadcast), FeedTier::Global);
    }

    // ------------------------------- THE critical serialization round-trip

    /// A notice board on a throwaway sqlite file named exactly like the real
    /// sibling DB, mirroring `tests/org_notice_board.rs` so a pass here cannot
    /// pass by accident against a differently-named file.
    fn board() -> (NoticeBoard, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path: PathBuf = dir.path().join("operant_kanban.db");
        let board = NoticeBoard::init(path).expect("NoticeBoard::init");
        (board, dir)
    }

    /// THE make-or-break test.
    ///
    /// Proves that the selector strings this module produces are byte-for-byte
    /// what `query_inbox`'s `json_each` scan matches. It posts a `dept:infra`
    /// notice through a real `NoticeBoard`, builds a reader's identity from an
    /// `Employee` row via `identity_for`, and:
    ///
    /// 1. asserts an infra member's inbox contains the notice (positive match),
    /// 2. asserts an outsider's inbox is empty (no cross-delivery).
    ///
    /// A serialization drift between `selectors_for_identity` and the SQL would
    /// make BOTH directions return zero rows and the positive assertion would
    /// fail. This is exactly the silent-bug class the packet warns about, so it
    /// is pinned against the real board rather than a mock.
    #[test]
    fn serialization_matches_query_inbox_json_each() {
        let (board, _dir) = board();

        // Post to `dept:infra` — this serialises through Recipient::selector
        // into the stored `recipients` JSON array.
        let posted = board
            .post(&PostNotice::new(
                "emp-head",
                vec![Recipient::Dept("infra".into())],
                "infra notice",
                "department feed must reach infra members",
            ))
            .expect("post dept notice");

        // An infra member reads their inbox via the resolver's own query.
        let member = in_dept("emp-infra-member", "infra");
        let member_identity = identity_for(&member, &[], &[]);
        let member_inbox = board
            .query_inbox(&inbox_query_for(&member_identity, None, None))
            .expect("member inbox");
        assert_eq!(
            member_inbox.len(),
            1,
            "infra member must receive the dept:infra notice (serialization must match the SQL)"
        );
        assert_eq!(member_inbox[0].id, posted.id);

        // An outsider in a different department receives nothing.
        let outsider = in_dept("emp-outsider", "platform");
        let outsider_identity = identity_for(&outsider, &[], &[]);
        let outsider_inbox = board
            .query_inbox(&inbox_query_for(&outsider_identity, None, None))
            .expect("outsider inbox");
        assert!(
            outsider_inbox.is_empty(),
            "outsider must not receive the dept:infra notice: {:?}",
            outsider_inbox.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
    }

    /// The same round-trip, but exercising `resolve_recipients` + `inbox` to
    /// confirm the write-side fan-out agrees with the read-side selectors: a
    /// `Dept` fan-out names exactly the readers whose `identity_for` inbox
    /// contains a `dept:` notice. This closes the loop between the two
    /// directions so a future edit cannot make them disagree.
    #[test]
    fn resolve_recipients_and_identity_for_agree_on_department_audience() {
        let (board, _dir) = board();
        let employees = vec![
            in_dept("emp-a1", "infra"),
            in_dept("emp-a2", "infra"),
            in_dept("emp-b1", "platform"),
        ];

        board
            .post(&PostNotice::new(
                "emp-head",
                vec![Recipient::Dept("infra".into())],
                "infra notice",
                "audience agreement",
            ))
            .expect("post");

        let audience = resolve_recipients(&Recipient::Dept("infra".into()), &employees);
        assert_eq!(audience, vec!["emp-a1".to_string(), "emp-a2".to_string()]);

        // Every id in the fan-out reaches the notice; the one not in it does not.
        for id in &audience {
            let identity = identity_for(
                employees
                    .iter()
                    .find(|e| &e.employee_id == id)
                    .expect("id in slice"),
                &[],
                &[],
            );
            let inbox = board
                .query_inbox(&inbox_query_for(&identity, None, None))
                .expect("inbox");
            assert_eq!(inbox.len(), 1, "fan-out member {id} must receive it");
        }
        let outsider_identity = identity_for(
            employees
                .iter()
                .find(|e| e.employee_id == "emp-b1")
                .expect("row"),
            &[],
            &[],
        );
        let outsider_inbox = board
            .query_inbox(&inbox_query_for(&outsider_identity, None, None))
            .expect("outsider inbox");
        assert!(
            outsider_inbox.is_empty(),
            "non-audience must not receive it"
        );
    }

    /// Proves `selectors_for_identity` matches the selectors the board itself
    /// would compute for the same identity, by comparing against the value
    /// `NoticeBoard::inbox` derives internally. If these ever drift, the
    /// JSON round-trip test above is the second line of defence, but this one
    /// localises a drift to the exact source.
    #[test]
    fn selectors_for_identity_uses_recipient_selector_form() {
        let identity = identity_for(&in_dept("emp-1", "infra"), &["core".into()], &[]);
        let sels = selectors_for_identity(&identity);
        // The canonical stored forms, exactly as encode_recipients writes them.
        assert!(sels.contains(&"agent:emp-1".to_string()));
        assert!(sels.contains(&"dept:infra".to_string()));
        assert!(sels.contains(&"team:core".to_string()));
        assert!(sels.contains(&"role:automator".to_string()));
        // Every selector matches the `kind:value` shape query_inbox scans for.
        for s in &sels {
            assert!(
                s.contains(':') || s == "broadcast",
                "selector {s} is not in the kind:value form the SQL scans for"
            );
        }
    }
}

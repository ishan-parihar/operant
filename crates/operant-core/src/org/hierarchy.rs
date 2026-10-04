//! The reporting tree — `reports_to`, position, peers, and cycle findings.
//! (`docs/REMAINING-GAPS.md` GAP-3.1, `docs/ORG-AUTHORITY-ARCHITECTURE.md` §1–§2.)
//!
//! Pure: no I/O, no sqlite. The rows live in [`super::employee_db`] and the
//! head seat lives in [`super::department_db`]; this module is the *query
//! surface* over a set of edges, so it is unit-testable without a database file
//! and readable on its own.
//!
//! ## Why hierarchy is separate from [`Employee`]
//!
//! GAP-3.1's acceptance asks for `reports_to` "on `Employee`". This packet
//! delivers the same capability as an explicit edge type instead, for one
//! reason: `Employee` is constructed from struct literals in eight files
//! across this crate and its tests, **none of which use
//! `..Default::default()`** — the type derives `Default` nowhere — so adding a
//! field breaks every one of them. Two of those files, `employee_db.rs`'s
//! backfill and row mapper, are outside this packet's ownership, as are the
//! `org` modules other agents are editing right now. Adding the field would
//! leave `operant-core` not compiling, which is damage, not a change.
//!
//! So the reporting line lives on [`HierarchyEntry`], and the queries take a
//! slice of edges plus the employees they name. `Employee` is untouched. When a
//! later packet adds the column, `HierarchyEntry` is what reads and projects
//! it, and no query here changes — because no query here ever assumed the edge
//! was a field.
//!
//! ## §2.1 made executable: a head outranks a peer with no grant recorded
//!
//! "Hierarchy is the **primary** grant: it is sufficient for everything inside
//! a department; an explicit capability grant is required **only** to cross a
//! department boundary." Head-versus-peer is therefore answerable from position
//! alone — no `authority_grants` row is read, because there is nothing to read:
//! [`Position`] is derived purely from the edges.
//!
//! [`Position`] is a three-way answer, not a bool, because the honest third
//! state is the one this project exists to protect:
//!
//! - [`Position::Head`] — somebody in this employee's own department reports
//!   *to* them, so they outrank their peers by construction.
//! - [`Position::Peer`] — they report to somebody in their own department.
//! - [`Position::Detached`] — the position could not be resolved: they are
//!   undeparted, or their reporting line is malformed. This is *not*
//!   "top of the org" and *not* "a head". Conflating the two would hand
//!   `Peers`-and-`Department` authority to any row the backfill happens not to
//!   have linked yet — the fabricated capability §3.3 forbids. It is reported
//!   as [`AUTHORITY_UNRESOLVED`], with the reason named in the finding text.
//!
//! ## The three honesty rules this module holds
//!
//! 1. **`None` is a state.** A missing `reports_to` is listed by
//!    [`Hierarchy::unassigned`] and reported as [`UNASSIGNED_MANAGER`]. It is
//!    never defaulted to a guess and never given a placeholder manager.
//! 2. **A missing department stays visible.** An employee with
//!    `department: None` is never bucketed into a default department: they
//!    cannot be a head and cannot have peers. Both outcomes are surfaced by
//!    [`Hierarchy::findings`] — [`UNDEPARTMENTED`] for the former,
//!    [`UNASSIGNED_MANAGER`] for the latter — so the row stays discoverable
//!    rather than invisible.
//! 3. **A cycle is found, never followed.** [`Hierarchy::cycles`] returns the
//!    malformed edges as data. The walk is iterative and guarded by a visited
//!    set, so a hand-edited `a → b → a` costs a finding, not a hang and not a
//!    stack overflow.
//!
//! ## What this module does NOT do
//!
//! It does not decide whether a head may act. That is
//! [`super::authority::resolve_scope`], fed by the two `bool` flags
//! [`Hierarchy::is_dept_head`] and [`Hierarchy::is_org_lead`] produce. This
//! module resolves *position*; that function resolves *authority*. Keeping them
//! apart is what makes both testable without a grants table.
//!
//! It does not persist the edges either. There is no `hierarchy_reports` table
//! and no DDL here, because the DDL and the backfill belong to
//! [`super::employee_db`] — a file this packet does not own. The gap is
//! reported, not worked around.

use std::collections::{BTreeMap, BTreeSet};

use super::employee::Employee;

/// The recorded value of `reports_to` for an employee who has no manager.
///
/// §3.1 makes `department` an operator-set tag rather than something derived;
/// the reporting line gets the same treatment. It is a *recorded* top, which is
/// how [`Hierarchy::findings`] tells it apart from an employee nobody has
/// assigned yet.
pub const REPORTS_TO_NONE: &str = "<none>";

/// A position that could not be resolved from the reporting tree: a recorded
/// top of the tree, a cycle with no top, a self-report, a broken pointer, or a
/// row nobody has assigned yet.
///
/// Reporting it is the point. §2.1 makes hierarchy the *primary* grant, so
/// "hierarchy gives no answer" has to be a nameable, countable state rather than
/// a silent false — otherwise the operator cannot tell "this employee has no
/// authority" from "this employee's reporting line was never filled in".
pub const AUTHORITY_UNRESOLVED: &str = "authority_unresolved";
/// See [`UNASSIGNED_MANAGER`].
pub const UNASSIGNED_MANAGER: &str = "unassigned_manager";
/// See [`UNDEPARTMENTED`].
pub const UNDEPARTMENTED: &str = "undeparted";
/// See [`DANGLING_MANAGER`].
pub const DANGLING_MANAGER: &str = "dangling_manager";
/// See [`SELF_MANAGED`].
pub const SELF_MANAGED: &str = "self_managed";

// =====================================================================
// HierarchyEntry
// =====================================================================

/// One edge of the reporting tree: "this employee reports to that one".
///
/// Separate from [`Employee`] for the reason in the module docs. `peers` is
/// deliberately **absent**: the organism's `AgentSpec` carries an explicit peer
/// list (GAP-3.1), but here peers are a *derived* set — everyone reporting to
/// the same manager — so storing them would be a second copy of the edges that
/// can drift. [`Hierarchy::peers_of`] is the one derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyEntry {
    /// The employee this entry describes.
    pub employee_id: String,
    /// The `employee_id` they report to, or [`REPORTS_TO_NONE`].
    ///
    /// `None` here is *malformed*, not top-of-org: it means nobody built the
    /// entry through [`HierarchyEntry::new`] or
    /// [`HierarchyEntry::top_of_org`]. It stays visible — it simply reports no
    /// manager, exactly like [`REPORTS_TO_NONE`].
    pub reports_to: Option<String>,
    /// One-line reason this edge exists. Never empty.
    ///
    /// §3.1 already requires a reason on every `employees` row for the same
    /// reason: a reporting line is an operator's act, so the record must name
    /// whoever made it. A backfill with no operator behind it would be
    /// indistinguishable from a real assignment.
    pub reason: String,
}

impl HierarchyEntry {
    /// An employee who reports to `manager`.
    pub fn new(employee_id: &str, manager: &str, reason: &str) -> Self {
        HierarchyEntry {
            employee_id: employee_id.to_string(),
            reports_to: Some(manager.to_string()),
            reason: reason.to_string(),
        }
    }

    /// An employee with no reporting line: top of the org, or not yet assigned.
    ///
    /// The two are deliberately the same value — §2.1 gives hierarchy no answer
    /// about what sits above a top-of-org employee either, so distinguishing
    /// them would invent a distinction the design does not have.
    pub fn top_of_org(employee_id: &str, reason: &str) -> Self {
        HierarchyEntry {
            employee_id: employee_id.to_string(),
            reports_to: Some(REPORTS_TO_NONE.to_string()),
            reason: reason.to_string(),
        }
    }

    /// The manager `employee_id` reports to, or `None` for [`REPORTS_TO_NONE`],
    /// a blank, or a malformed `reports_to: None`.
    ///
    /// Every reader goes through here, so the sentinel cannot be mistaken for a
    /// real manager in one query while being filtered out of another.
    pub fn manager(&self) -> Option<&str> {
        let raw = self.reports_to.as_deref()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == REPORTS_TO_NONE {
            None
        } else {
            Some(trimmed)
        }
    }

    /// `true` when this entry carries no reporting line.
    pub fn is_unassigned(&self) -> bool {
        self.manager().is_none()
    }
}

// =====================================================================
// Position
// =====================================================================

/// Where an employee sits, relative to their own department.
///
/// Derived from edges and departments only. No grant is read, because §2.1 says
/// none is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Somebody in this employee's own department reports **to** them, so they
    /// outrank their peers by position alone (§2.1).
    Head,
    /// They report to somebody in their own department.
    Peer,
    /// The position could not be resolved: the employee is undeparted, or their
    /// reporting line is malformed (a cycle, a self-report, or a manager that is
    /// not a known row).
    ///
    /// Certainly not top-of-org and certainly not a head — see the module docs
    /// on the fabricated-capability failure that conflating them would cause.
    Detached,
}

impl Position {
    /// `true` only for [`Position::Head`].
    ///
    /// The predicate [`super::authority::resolve_scope`] wants as
    /// `is_dept_head`. Named to match that parameter so a call site reads
    /// without a comment.
    pub fn is_head(&self) -> bool {
        matches!(self, Self::Head)
    }
}

// =====================================================================
// Findings
// =====================================================================

/// One visible gap in the reporting tree, as a value a caller renders.
///
/// The shape mirrors [`super::department_db::StaffingFinding`]: a value, not a
/// log line, because `org check`'s exit contract (§11.4) counts these and the
/// TUI shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyFinding {
    /// The employee the finding is about.
    pub employee_id: String,
    /// Machine-readable kind: one of the module's `*` constants.
    pub kind: String,
    /// Human-readable sentence naming the specific gap.
    pub detail: String,
}

impl HierarchyFinding {
    fn new(employee_id: &str, kind: &str, detail: String) -> Self {
        HierarchyFinding {
            employee_id: employee_id.to_string(),
            kind: kind.to_string(),
            detail,
        }
    }
}

/// One malformed edge. `from` is the employee whose `reports_to` is `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyCycle {
    pub from: String,
    pub to: String,
}

/// Why a reporting line does not resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MalformedKind {
    /// Following managers returns to a node already visited: a genuine loop,
    /// which has no top and therefore no resolvable position for anybody in it.
    Cycle,
    /// The employee reports to themselves. A one-node loop.
    SelfManaged,
    /// The manager is not a known employee, so the reporting line resolves to
    /// nothing. Reported rather than treated as a clean top, because a typo is a
    /// gap to fix and a top is a position — they are not the same fact.
    DanglingManager,
}

impl MalformedKind {
    /// The [`HierarchyFinding`] kind this defect is reported under.
    pub fn finding_kind(&self) -> &'static str {
        match self {
            Self::Cycle => AUTHORITY_UNRESOLVED,
            Self::SelfManaged => SELF_MANAGED,
            Self::DanglingManager => DANGLING_MANAGER,
        }
    }
}

// =====================================================================
// Hierarchy
// =====================================================================

/// A resolved reporting tree over a fixed set of [`Employee`] rows and
/// [`HierarchyEntry`] edges.
///
/// Every index is built once in [`Hierarchy::new`], including the malformed-edge
/// map, so resolving a whole org's positions does not re-walk the edges per
/// employee. Cheap to clone for a per-request copy.
///
/// A **departmentless** employee (`department: None` or blank) can never be
/// [`Position::Head`] and can never have peers: there is no department to be
/// head *of*. That is not a silent drop — [`Hierarchy::findings`] reports every
/// such employee under [`UNDEPARTMENTED`], so the row stays visible.
#[derive(Debug, Clone)]
pub struct Hierarchy<'a> {
    employees: &'a [Employee],
    /// Department per `employee_id`. Only employees **with** a non-blank
    /// department are keys, so `get(..) == None` *is* the undeparted state.
    department: BTreeMap<&'a str, &'a str>,
    /// Manager per `employee_id`. An employee with no entry, or with an entry
    /// whose [`HierarchyEntry::manager`] is `None`, is simply absent — the
    /// "unassigned" read is a map miss, not a sentinel string.
    manager: BTreeMap<&'a str, &'a str>,
    /// Direct reports per manager `employee_id`. **Includes** edges naming a
    /// manager who is not a known employee — that is the dangling-pointer case
    /// [`DANGLING_MANAGER`] exists to report, and dropping it here would hide
    /// the finding.
    reports: BTreeMap<&'a str, Vec<&'a str>>,
    /// First entry per `employee_id`. Later duplicates are ignored rather than
    /// overwritten: when two operators disagree about a manager, the *first*
    /// recorded line is the one on the record, and the disagreement is not
    /// silently resolved in favour of the later write.
    entry: BTreeMap<&'a str, &'a HierarchyEntry>,
    /// Malformed edges, keyed by the employee whose line is broken.
    malformed: BTreeMap<&'a str, MalformedKind>,
}

impl<'a> Hierarchy<'a> {
    /// Resolve the tree for `employees` under `entries`.
    ///
    /// Never fails. A malformed graph produces findings and degenerate
    /// positions, never an error the caller has to handle to make progress.
    pub fn new(employees: &'a [Employee], entries: &'a [HierarchyEntry]) -> Self {
        let mut department = BTreeMap::new();
        for employee in employees {
            // A blank department is the same gap as NULL for the reason
            // `resolver::identity_for` gives: it cannot name a real department,
            // so storing it would put the employee in a department named "".
            if let Some(dept) = employee
                .department
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
            {
                department.insert(employee.employee_id.as_str(), dept);
            }
        }

        let mut manager = BTreeMap::new();
        let mut reports: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut entry = BTreeMap::new();
        for record in entries {
            let id = record.employee_id.as_str();
            if entry.contains_key(id) {
                continue;
            }
            entry.insert(id, record);
            if let Some(mgr) = record.manager() {
                manager.insert(id, mgr);
                reports.entry(mgr).or_default().push(id);
            }
        }

        let mut malformed = BTreeMap::new();
        for id in entry.keys().copied() {
            if let Some(kind) = classify_edge(id, &manager, &department) {
                malformed.insert(id, kind);
            }
        }

        Hierarchy {
            employees,
            department,
            manager,
            reports,
            entry,
            malformed,
        }
    }

    /// The department `employee_id` is in, or `None` when undeparted or unknown.
    pub fn department_of(&self, employee_id: &str) -> Option<&'a str> {
        self.department.get(employee_id).copied()
    }

    /// The manager `employee_id` reports to.
    ///
    /// `None` is the honest answer for all three real states: top of the org,
    /// not yet assigned, and a manager that is not a known row.
    pub fn manager_of(&self, employee_id: &str) -> Option<&'a str> {
        self.manager.get(employee_id).copied()
    }

    /// The direct reports of `employee_id`, in the order their entries appear.
    ///
    /// Empty for anybody nobody reports to.
    pub fn direct_reports_of(&self, employee_id: &str) -> Vec<&'a str> {
        self.reports
            .get(employee_id)
            .map_or(Vec::new(), |r| r.clone())
    }

    /// Every peer of `employee_id`: every other employee reporting to the *same*
    /// manager, in the same department.
    ///
    /// Empty for an employee with no manager or no department, and exact for
    /// everyone else:
    ///
    /// - No manager ⇒ empty. A peer set needs a shared manager; without one
    ///   there is nobody to be a peer *of*, and manufacturing a default bucket
    ///   would silently merge unrelated rows into one group.
    /// - No department ⇒ empty, because [`Position::Head`] and
    ///   [`Position::Peer`] are both defined relative to a department.
    /// - A sibling in **another** department is excluded. §2.2 scopes `Peers` to
    ///   "same department", so counting a cross-department sibling would widen
    ///   the scope past its own definition.
    /// - The employee is not their own peer.
    pub fn peers_of(&self, employee_id: &str) -> Vec<&'a str> {
        let Some(manager) = self.manager_of(employee_id) else {
            return Vec::new();
        };
        let Some(dept) = self.department_of(employee_id) else {
            return Vec::new();
        };
        self.reports.get(manager).map_or(Vec::new(), |siblings| {
            siblings
                .iter()
                .copied()
                .filter(|id| *id != employee_id)
                .filter(|id| self.department_of(id) == Some(dept))
                .collect()
        })
    }

    /// Where `employee_id` sits relative to its own department.
    ///
    /// This is §2.1 made executable: [`Position::Head`] comes from somebody in
    /// the same department reporting to this employee, and that fact alone
    /// outranks a peer — no `authority_grants` row is consulted, because §2.1
    /// says none is required.
    ///
    /// A malformed line resolves to [`Position::Detached`] **before** the head
    /// test runs. A two-node cycle makes each node the other's report, so the
    /// head test alone would call both heads and hand them authority off a
    /// corrupted graph. The brief requires a cycle to produce a finding rather
    /// than a hang; granting authority off it would be the same failure in the
    /// opposite direction.
    pub fn position_of(&self, employee_id: &str) -> Position {
        let Some(dept) = self.department_of(employee_id) else {
            // Undeparted: cannot be head of anything, and is not thereby a peer
            // either. Absence of a department is not a default one.
            return Position::Detached;
        };
        if self.malformed.contains_key(employee_id) {
            return Position::Detached;
        }
        let has_same_dept_report = self
            .reports
            .get(employee_id)
            .is_some_and(|rs| rs.iter().any(|r| self.department_of(r) == Some(dept)));
        if has_same_dept_report {
            Position::Head
        } else if self.manager.contains_key(employee_id) {
            Position::Peer
        } else {
            // Nobody reports to them and they report to nobody: unresolved.
            Position::Detached
        }
    }

    /// `is_dept_head` for [`super::authority::resolve_scope`].
    ///
    /// No grant is read and none is needed: §2.1 makes hierarchy the primary
    /// grant inside a department.
    pub fn is_dept_head(&self, employee_id: &str) -> bool {
        self.position_of(employee_id).is_head()
    }

    /// `is_org_lead` for [`super::authority::resolve_scope`].
    ///
    /// §2.1 grants org scope to the top of the reporting tree, and the top is
    /// the department head who reports to nobody. A head *with* a manager is a
    /// department head, not an org lead.
    pub fn is_org_lead(&self, employee_id: &str) -> bool {
        self.is_dept_head(employee_id) && !self.manager.contains_key(employee_id)
    }

    /// Every employee with no reporting line, in input order.
    ///
    /// A recorded top (`REPORTS_TO_NONE`) and a never-assigned row both appear
    /// here — neither has a manager. [`Hierarchy::findings`] is what tells them
    /// apart, by kind.
    pub fn unassigned(&self) -> Vec<&'a str> {
        self.employees
            .iter()
            .filter(|e| !self.manager.contains_key(e.employee_id.as_str()))
            .map(|e| e.employee_id.as_str())
            .collect()
    }

    /// Every malformed edge, classified: cycles, self-reports, and dangling
    /// pointers. Empty for a well-formed tree.
    pub fn malformed_edges(&self) -> Vec<(HierarchyCycle, MalformedKind)> {
        self.malformed
            .iter()
            .filter_map(|(from, kind)| {
                let to = self.manager.get(*from).copied()?;
                Some((
                    HierarchyCycle {
                        from: (*from).to_string(),
                        to: to.to_string(),
                    },
                    *kind,
                ))
            })
            .collect()
    }

    /// Every cycle, as malformed edges — one per participant, so each employee
    /// whose line is wrong is named exactly once.
    ///
    /// A self-report ([`MalformedKind::SelfManaged`]) and a broken pointer
    /// ([`MalformedKind::DanglingManager`]) are *not* cycles and are excluded
    /// here; use [`Hierarchy::malformed_edges`] to see all three kinds.
    pub fn cycles(&self) -> Vec<HierarchyCycle> {
        self.malformed_edges()
            .into_iter()
            .filter(|(_, kind)| matches!(kind, MalformedKind::Cycle))
            .map(|(edge, _)| edge)
            .collect()
    }

    /// Every visible gap in the reporting tree, in `employees` input order.
    ///
    /// The read-side half of the module's three honesty rules. `org check`
    /// (§11.4) renders exactly this set, which is where "missing must stay
    /// discoverable" is kept true in practice.
    ///
    /// Each employee produces at most two findings: [`UNDEPARTMENTED`] (they
    /// have no department, which is independent of their reporting line) and at
    /// most one finding about the line itself.
    pub fn findings(&self) -> Vec<HierarchyFinding> {
        let mut out = Vec::new();
        for employee in self.employees {
            let id = employee.employee_id.as_str();

            if self.department_of(id).is_none() {
                out.push(HierarchyFinding::new(
                    id,
                    UNDEPARTMENTED,
                    format!(
                        "employee {id} has no department: department is an operator-set tag and \
                         none is derivable, so this employee cannot be resolved as a department \
                         head and has no peer set"
                    ),
                ));
            }

            if let Some(kind) = self.malformed.get(id) {
                out.push(self.malformed_finding(id, kind));
                continue;
            }

            // A recorded manager is a healthy line; nothing to report.
            if self.manager.contains_key(id) {
                continue;
            }

            let recorded_top = self
                .entry
                .get(id)
                .and_then(|e| e.reports_to.as_deref())
                .is_some_and(|raw| raw.trim() == REPORTS_TO_NONE);

            if recorded_top {
                out.push(HierarchyFinding::new(
                    id,
                    AUTHORITY_UNRESOLVED,
                    format!(
                        "employee {id} reports to {REPORTS_TO_NONE}: it is the top of this \
                         reporting tree and reports to nobody, so hierarchy grants it nothing \
                         further — an org-wide scope needs an explicit recorded grant"
                    ),
                ));
            } else {
                out.push(HierarchyFinding::new(
                    id,
                    UNASSIGNED_MANAGER,
                    format!(
                        "employee {id} has no reporting line: reports_to is unset, so it has no \
                         manager, no peer set, and no resolved position — assign one, or record \
                         {REPORTS_TO_NONE} if it really is the top"
                    ),
                ));
            }
        }
        out
    }

    /// The one-sentence explanation for a malformed line.
    fn malformed_finding(&self, id: &str, kind: &MalformedKind) -> HierarchyFinding {
        let to = self.manager.get(id).copied().unwrap_or(id);
        let detail = match kind {
            MalformedKind::Cycle => format!(
                "employee {id} sits on a reports_to cycle through {to}: a loop has no top, so no \
                 position resolves for anyone in it"
            ),
            MalformedKind::SelfManaged => format!(
                "employee {id} reports to itself: a self-reporting line is not a position in any \
                 tree"
            ),
            MalformedKind::DanglingManager => format!(
                "employee {id} reports to {to}, which is not a known employee, so the reporting \
                 line does not resolve"
            ),
        };
        HierarchyFinding::new(id, kind.finding_kind(), detail)
    }
}

/// Walk one employee's reporting line and report the first defect, if any.
///
/// # Why this terminates
///
/// Following managers is non-terminating on `a → b → a`, so the walk carries a
/// `visited` set and returns the instant it revisits a node. Each iteration
/// consumes a node not previously visited, so the loop is bounded by
/// `manager.len()` iterations *by construction* — no counter to get wrong. It is
/// iterative, not recursive, so there is no stack to overflow.
///
/// # Why one start node is enough to find every defect
///
/// A defect is a property of an edge, and every edge belongs to exactly one
/// employee. `a → a` never reaches a second node, so only `a`'s own walk can see
/// it; `a → ghost` closes no edge, so the walk ends in `None` and would be
/// reported as a clean top were it not caught here. Both are caught here, which
/// is why the whole tree is scanned rather than trusting one representative.
fn classify_edge<'a>(
    start: &'a str,
    manager: &BTreeMap<&'a str, &'a str>,
    department: &BTreeMap<&'a str, &'a str>,
) -> Option<MalformedKind> {
    let mut visited: BTreeSet<&'a str> = BTreeSet::new();
    let mut previous: Option<&'a str> = None;
    let mut current: &'a str = start;
    loop {
        if !visited.insert(current) {
            // `previous` is the edge that closed the ring, since `previous`'s
            // manager is `current`. For a self-report the ring closes on the
            // first iteration and `previous` is `None`, which is handled below.
            return Some(match previous {
                Some(_) => MalformedKind::Cycle,
                None => MalformedKind::SelfManaged,
            });
        }
        let next = manager.get(current).copied()?;
        if next == current {
            return Some(MalformedKind::SelfManaged);
        }
        // A manager who is not a known employee closes no edge, so the walk
        // would end in `None` — a "clean top" that is really a typo.
        let next_is_known = department.contains_key(next) || manager.contains_key(next);
        if !next_is_known {
            return Some(MalformedKind::DanglingManager);
        }
        previous = Some(current);
        current = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::employee::STATUS_ACTIVE;

    // ------------------------------------------------------------ fixtures

    /// An employee row with sensible defaults, so a test states only what it is
    /// about. Mirrors the fixtures in `resolver.rs` and `department_db.rs`.
    fn employee(id: &str, department: Option<&str>) -> Employee {
        Employee {
            employee_id: id.to_string(),
            name: format!("Employee {id}"),
            role: "automator".to_string(),
            department: department.map(str::to_string),
            skills: vec!["ops".to_string()],
            agent_type: None,
            persona: None,
            system_prompt: None,
            status: STATUS_ACTIVE.to_string(),
            reason: "test fixture".to_string(),
            created_at: "2026-10-01T00:00:00Z".to_string(),
            updated_at: "2026-10-01T00:00:00Z".to_string(),
        }
    }

    fn platform() -> &'static str {
        "Platform Infrastructure"
    }

    /// A normal chain plus a departmentless row:
    ///
    /// ```text
    /// emp-ceo         (Platform Infrastructure, recorded top)
    ///   └─ emp-head   (Platform Infrastructure)
    ///        ├─ emp-a  (Platform Infrastructure)
    ///        └─ emp-b  (Platform Infrastructure)
    /// emp-sole        (Data Services, its own recorded top)
    /// emp-orphan      (no department, recorded top)
    /// ```
    fn acyclic_tree() -> (Vec<Employee>, Vec<HierarchyEntry>) {
        let employees = vec![
            employee("emp-ceo", Some(platform())),
            employee("emp-head", Some(platform())),
            employee("emp-a", Some(platform())),
            employee("emp-b", Some(platform())),
            employee("emp-sole", Some("Data Services")),
            employee("emp-orphan", None),
        ];
        let entries = vec![
            HierarchyEntry::top_of_org("emp-ceo", "founding seat"),
            HierarchyEntry::new("emp-head", "emp-ceo", "platform lead"),
            HierarchyEntry::new("emp-a", "emp-head", "runbook work"),
            HierarchyEntry::new("emp-b", "emp-head", "oncall work"),
            HierarchyEntry::top_of_org("emp-sole", "single-person department"),
            HierarchyEntry::top_of_org("emp-orphan", "not yet placed"),
        ];
        (employees, entries)
    }

    /// The two `bool` flags [`super::authority::resolve_scope`] is called with,
    /// so a test asserts that *position* is what feeds authority.
    fn scope_flags(h: &Hierarchy, id: &str) -> (bool, bool) {
        (h.is_dept_head(id), h.is_org_lead(id))
    }

    fn kinds_for<'a>(findings: &'a [HierarchyFinding], id: &str) -> Vec<&'a str> {
        findings
            .iter()
            .filter(|f| f.employee_id == id)
            .map(|f| f.kind.as_str())
            .collect()
    }

    // ------------------------------------ (1) head outranks peer, no grant

    #[test]
    fn a_head_outranks_a_peer_by_position_with_no_grant_recorded() {
        let (employees, entries) = acyclic_tree();
        let h = Hierarchy::new(&employees, &entries);

        // `entries` is the whole input and carries no capability record, so no
        // `authority_grants` row could have contributed. Position is the only
        // thing that could produce these.
        assert_eq!(h.position_of("emp-head"), Position::Head);
        assert_eq!(h.position_of("emp-a"), Position::Peer);
        assert_eq!(h.position_of("emp-b"), Position::Peer);

        assert_eq!(
            scope_flags(&h, "emp-head"),
            (true, false),
            "a head with a manager is a department head, not an org lead"
        );
        assert_eq!(
            scope_flags(&h, "emp-a"),
            (false, false),
            "a peer must resolve no authority flags at all"
        );
        assert_eq!(
            scope_flags(&h, "emp-ceo"),
            (true, true),
            "the top of the tree, with a direct report, is the org lead"
        );
    }

    // ----------------------------------------- (2) None stays a real state

    #[test]
    fn reports_to_none_is_reported_as_a_real_state_not_silently_resolved() {
        let (employees, entries) = acyclic_tree();
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(
            h.manager_of("emp-ceo"),
            None,
            "REPORTS_TO_NONE must never read back as a manager"
        );
        assert!(
            h.unassigned().contains(&"emp-ceo"),
            "a recorded top has no manager, and must be listed rather than resolved"
        );
        assert_eq!(
            kinds_for(&h.findings(), "emp-ceo"),
            vec![AUTHORITY_UNRESOLVED],
            "a recorded top is reported as unresolved, not silently accepted: {:?}",
            h.findings()
        );
        assert!(
            !h.findings().iter().any(|f| f.kind == UNASSIGNED_MANAGER),
            "a recorded top is not an unassigned row: {:?}",
            h.findings()
        );
    }

    #[test]
    fn a_row_with_no_entry_at_all_is_reported_as_unassigned() {
        let employees = vec![employee("emp-lonely", Some(platform()))];
        let h = Hierarchy::new(&employees, &[]);

        assert_eq!(h.unassigned(), vec!["emp-lonely"]);
        assert_eq!(h.manager_of("emp-lonely"), None);
        assert_eq!(
            kinds_for(&h.findings(), "emp-lonely"),
            vec![UNASSIGNED_MANAGER],
            "a missing entry must surface as UNASSIGNED_MANAGER: {:?}",
            h.findings()
        );
        assert_ne!(
            h.findings()[0].kind,
            AUTHORITY_UNRESOLVED,
            "a never-assigned row is a missing assignment, not a resolved top"
        );
    }

    #[test]
    fn the_sentinel_is_recognised_even_when_written_with_padding() {
        let entry = HierarchyEntry {
            employee_id: "emp-a".to_string(),
            reports_to: Some(format!("  {REPORTS_TO_NONE}  ")),
            reason: "recorded top".to_string(),
        };
        assert_eq!(
            entry.manager(),
            None,
            "whitespace must not make it a manager"
        );
        assert!(entry.is_unassigned());
    }

    // -------------------------------------- (3) departmentless stays visible

    #[test]
    fn an_employee_with_no_department_stays_visible_in_findings() {
        let (employees, entries) = acyclic_tree();
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(
            kinds_for(&h.findings(), "emp-orphan"),
            vec![UNDEPARTMENTED, AUTHORITY_UNRESOLVED],
            "the missing department must be named, and the missing manager separately: {:?}",
            h.findings()
        );
        assert_eq!(
            h.department_of("emp-orphan"),
            None,
            "no default department may be invented"
        );
        assert!(
            h.peers_of("emp-orphan").is_empty(),
            "a departmentless row has no peer set"
        );
        assert_eq!(
            h.position_of("emp-orphan"),
            Position::Detached,
            "no department means no head and no peer — not Head by default"
        );
        assert_eq!(scope_flags(&h, "emp-orphan"), (false, false));
    }

    #[test]
    fn a_departmentless_employee_is_never_promoted_to_head_by_siblings() {
        // Without the department requirement, emp-z's report would make emp-x a
        // "head" — a fabricated capability for a seat nobody staffs.
        let employees = vec![
            employee("emp-x", None),
            employee("emp-y", None),
            employee("emp-z", None),
        ];
        let entries = vec![
            HierarchyEntry::new("emp-x", "emp-y", "chained"),
            HierarchyEntry::new("emp-z", "emp-x", "chained"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(h.position_of("emp-x"), Position::Detached);
        assert_eq!(scope_flags(&h, "emp-x"), (false, false));
        assert!(
            h.peers_of("emp-z").is_empty(),
            "undeparted siblings are excluded from a peer set, not merged into one"
        );
    }

    #[test]
    fn a_head_needs_a_report_from_their_own_department() {
        // The one shape the head test's department clause exists for: this
        // employee HAS a direct report, and that report sits in another
        // department. Without the department requirement they resolve as Head
        // and hand out `DirectReports` authority over a colleague they do not
        // manage — the cross-department widening §2.1 requires a grant for.
        //
        // The employee also has a real manager, so the `Detached` branch is not
        // what is under test here: the only thing separating the correct answer
        // from `Head` is the department match on the incoming edge.
        let employees = vec![
            employee("emp-ceo", Some(platform())),
            employee("emp-lead", Some(platform())),
            employee("emp-outsider", Some("Data Services")),
            employee("emp-mate", Some(platform())),
        ];
        let entries = vec![
            HierarchyEntry::top_of_org("emp-ceo", "founding seat"),
            HierarchyEntry::new("emp-lead", "emp-ceo", "platform lead"),
            HierarchyEntry::new("emp-outsider", "emp-lead", "crosses a boundary"),
            HierarchyEntry::new("emp-mate", "emp-outsider", "works the outsider's team"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        // The edge exists: `emp-outsider` really is a direct report of emp-lead.
        assert_eq!(h.direct_reports_of("emp-lead"), vec!["emp-outsider"]);
        assert_eq!(
            h.manager_of("emp-lead"),
            Some("emp-ceo"),
            "they have a manager, so this is not an unresolved position"
        );
        assert_eq!(
            h.position_of("emp-lead"),
            Position::Peer,
            "a report from another department makes this a peer, not a head"
        );
        assert_eq!(
            scope_flags(&h, "emp-lead"),
            (false, false),
            "no cross-department authority may be conferred by an incoming edge"
        );

        // The control: the same shape with the report inside the department
        // flips to Head. Without this, the assertion above would also pass
        // under a `position_of` that returned `false` unconditionally.
        let same_dept = vec![
            HierarchyEntry::top_of_org("emp-ceo", "founding seat"),
            HierarchyEntry::new("emp-lead", "emp-ceo", "platform lead"),
            HierarchyEntry::new("emp-mate", "emp-lead", "same department now"),
        ];
        let h2 = Hierarchy::new(&employees, &same_dept);
        assert_eq!(h2.direct_reports_of("emp-lead"), vec!["emp-mate"]);
        assert_eq!(
            h2.position_of("emp-lead"),
            Position::Head,
            "the same shape inside the department IS a head"
        );
        assert_eq!(scope_flags(&h2, "emp-lead"), (true, false));
    }

    // ------------------------------------------------ (4) two-node cycle

    #[test]
    fn a_two_node_cycle_is_detected_rather_than_followed() {
        let employees = vec![
            employee("emp-a", Some(platform())),
            employee("emp-b", Some(platform())),
        ];
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-b", "a reports to b"),
            HierarchyEntry::new("emp-b", "emp-a", "b reports to a"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        let mut pairs: Vec<(String, String)> = h
            .cycles()
            .iter()
            .map(|c| (c.from.clone(), c.to.clone()))
            .collect();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("emp-a".to_string(), "emp-b".to_string()),
                ("emp-b".to_string(), "emp-a".to_string()),
            ],
            "both participants of a two-node cycle must be reported: {:?}",
            h.cycles()
        );
        assert_eq!(
            kinds_for(&h.findings(), "emp-a"),
            vec![AUTHORITY_UNRESOLVED]
        );
        assert_eq!(
            kinds_for(&h.findings(), "emp-b"),
            vec![AUTHORITY_UNRESOLVED]
        );
        assert_eq!(
            scope_flags(&h, "emp-a"),
            (false, false),
            "a cycle must not resolve into authority"
        );
    }

    // ------------------------------------------------- (5) longer cycle

    #[test]
    fn a_four_node_cycle_is_detected() {
        let employees: Vec<Employee> = ["emp-a", "emp-b", "emp-c", "emp-d"]
            .iter()
            .map(|id| employee(id, Some(platform())))
            .collect();
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-b", "ring"),
            HierarchyEntry::new("emp-b", "emp-c", "ring"),
            HierarchyEntry::new("emp-c", "emp-d", "ring"),
            HierarchyEntry::new("emp-d", "emp-a", "ring"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        let cycles = h.cycles();
        assert_eq!(
            cycles.len(),
            4,
            "one edge per participant, no more and no fewer: {cycles:?}"
        );
        for id in ["emp-a", "emp-b", "emp-c", "emp-d"] {
            assert!(
                cycles.iter().any(|c| c.from == id),
                "{id} is a participant and must be named: {cycles:?}"
            );
        }
        for id in ["emp-a", "emp-b", "emp-c", "emp-d"] {
            assert_eq!(
                scope_flags(&h, id),
                (false, false),
                "no member of a ring may resolve authority"
            );
        }
    }

    #[test]
    fn a_self_reporting_line_is_detected_as_its_own_kind() {
        let employees = vec![employee("emp-a", Some(platform()))];
        let entries = vec![HierarchyEntry::new("emp-a", "emp-a", "oops")];
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(
            h.malformed_edges(),
            vec![(
                HierarchyCycle {
                    from: "emp-a".to_string(),
                    to: "emp-a".to_string(),
                },
                MalformedKind::SelfManaged,
            )]
        );
        assert!(
            h.cycles().is_empty(),
            "a self-report is a one-node loop, not a ring: {:?}",
            h.cycles()
        );
        assert_eq!(kinds_for(&h.findings(), "emp-a"), vec![SELF_MANAGED]);
    }

    #[test]
    fn a_manager_who_is_not_a_known_employee_is_reported_not_followed() {
        let employees = vec![employee("emp-a", Some(platform()))];
        let entries = vec![HierarchyEntry::new("emp-a", "emp-ghost", "typo")];
        let h = Hierarchy::new(&employees, &entries);

        assert!(
            h.cycles().is_empty(),
            "a broken pointer closes no edge and is not a cycle: {:?}",
            h.cycles()
        );
        assert_eq!(
            h.malformed_edges(),
            vec![(
                HierarchyCycle {
                    from: "emp-a".to_string(),
                    to: "emp-ghost".to_string(),
                },
                MalformedKind::DanglingManager,
            )]
        );
        assert_eq!(
            kinds_for(&h.findings(), "emp-a"),
            vec![DANGLING_MANAGER],
            "a typo is a gap to fix, not a top: {:?}",
            h.findings()
        );
        assert_eq!(
            h.position_of("emp-a"),
            Position::Detached,
            "an unresolvable line confers nothing"
        );
    }

    // --------------------------------------- (6) acyclic chain resolution

    #[test]
    fn a_normal_acyclic_chain_resolves_correctly() {
        let (employees, entries) = acyclic_tree();
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(h.manager_of("emp-head"), Some("emp-ceo"));
        assert_eq!(h.manager_of("emp-a"), Some("emp-head"));
        assert_eq!(h.direct_reports_of("emp-head"), vec!["emp-a", "emp-b"]);
        assert_eq!(h.direct_reports_of("emp-ceo"), vec!["emp-head"]);
        assert!(
            h.malformed_edges().is_empty(),
            "an acyclic tree reports no malformed edge at all: {:?}",
            h.malformed_edges()
        );
        assert_eq!(
            h.findings().len(),
            4,
            "two recorded tops, plus the departmentless row's two findings: {:?}",
            h.findings()
        );
        assert!(
            h.findings()
                .iter()
                .all(|f| !f.employee_id.starts_with("emp-a") && !f.employee_id.starts_with("emp-b")),
            "a healthy reporting line raises no finding: {:?}",
            h.findings()
        );
    }

    #[test]
    fn org_lead_is_the_top_of_the_tree_not_every_head() {
        let (employees, entries) = acyclic_tree();
        let h = Hierarchy::new(&employees, &entries);

        assert!(h.is_org_lead("emp-ceo"), "the top of the tree");
        assert!(
            !h.is_org_lead("emp-head"),
            "a department head still has a manager, so is not the org lead"
        );
        assert!(
            !h.is_org_lead("emp-sole"),
            "a lone top with no direct report is not a head, so resolves no flags"
        );
    }

    // ------------------------------------------------------ (7) exact peers

    #[test]
    fn peers_are_exactly_the_expected_set() {
        let employees = vec![
            employee("emp-head", Some(platform())),
            employee("emp-a", Some(platform())),
            employee("emp-b", Some(platform())),
            employee("emp-c", Some("Data Services")),
            employee("emp-lonely", Some(platform())),
            employee("emp-ghostless", None),
        ];
        let entries = vec![
            HierarchyEntry::top_of_org("emp-head", "top"),
            HierarchyEntry::new("emp-a", "emp-head", "x"),
            HierarchyEntry::new("emp-b", "emp-head", "x"),
            HierarchyEntry::new("emp-c", "emp-head", "crosses a department boundary"),
            HierarchyEntry::top_of_org("emp-lonely", "no manager"),
            HierarchyEntry::new("emp-ghostless", "emp-head", "no department"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(
            h.peers_of("emp-a"),
            vec!["emp-b"],
            "emp-c shares a manager but not a department; emp-head is a manager, \
             not a peer; emp-ghostless is undeparted, so excluded"
        );
        assert!(
            !h.peers_of("emp-a").contains(&"emp-a"),
            "an employee is not their own peer"
        );
        assert!(
            h.peers_of("emp-head").is_empty(),
            "a head has no manager, so it has no peers"
        );
        assert!(
            h.peers_of("emp-lonely").is_empty(),
            "no manager means no peer set"
        );
        assert!(
            h.peers_of("emp-ghostless").is_empty(),
            "no department means no peer set"
        );
    }

    #[test]
    fn peers_are_symmetric_for_a_pair() {
        let employees = vec![
            employee("emp-a", Some(platform())),
            employee("emp-b", Some(platform())),
        ];
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-head", "x"),
            HierarchyEntry::new("emp-b", "emp-head", "x"),
        ];
        let h = Hierarchy::new(&employees, &entries);
        assert_eq!(h.peers_of("emp-a"), vec!["emp-b"]);
        assert_eq!(h.peers_of("emp-b"), vec!["emp-a"]);
    }

    // --------------------------------------------------------- the guards

    #[test]
    fn a_first_entry_wins_and_later_contradictions_are_not_applied() {
        let employees = vec![
            employee("emp-a", Some(platform())),
            employee("emp-mgr", Some(platform())),
            employee("emp-other", Some(platform())),
        ];
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-mgr", "first"),
            HierarchyEntry::new("emp-a", "emp-other", "second, contradictory"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(
            h.manager_of("emp-a"),
            Some("emp-mgr"),
            "the first recorded line is the one on the record"
        );
        assert_eq!(h.direct_reports_of("emp-mgr"), vec!["emp-a"]);
        assert!(h.direct_reports_of("emp-other").is_empty());
    }

    #[test]
    fn a_blank_department_is_missing_not_a_department_named_empty() {
        let employees = vec![
            employee("emp-a", Some("   ")),
            employee("emp-b", Some("")),
            employee("emp-c", Some(platform())),
        ];
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-c", "x"),
            HierarchyEntry::new("emp-b", "emp-c", "x"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        assert_eq!(h.department_of("emp-a"), None);
        assert_eq!(h.department_of("emp-b"), None);
        assert!(
            h.peers_of("emp-a").is_empty(),
            "a blank department must not become a peer bucket"
        );
        assert_eq!(kinds_for(&h.findings(), "emp-a"), vec![UNDEPARTMENTED]);
    }

    #[test]
    fn a_chain_terminating_at_a_recorded_top_reports_no_defect() {
        // The longest healthy path a real org has: employee → ... → top. The
        // walk must end in `None`, not mistake the top for a dangling manager.
        let employees: Vec<Employee> = ["emp-a", "emp-b", "emp-c", "emp-top"]
            .iter()
            .map(|id| employee(id, Some(platform())))
            .collect();
        let entries = vec![
            HierarchyEntry::new("emp-a", "emp-b", "x"),
            HierarchyEntry::new("emp-b", "emp-c", "x"),
            HierarchyEntry::new("emp-c", "emp-top", "x"),
            HierarchyEntry::top_of_org("emp-top", "x"),
        ];
        let h = Hierarchy::new(&employees, &entries);

        assert!(h.malformed_edges().is_empty(), "{:?}", h.malformed_edges());
        assert_eq!(h.position_of("emp-top"), Position::Head);
        assert_eq!(scope_flags(&h, "emp-top"), (true, true));
        assert_eq!(h.position_of("emp-a"), Position::Peer);
    }
}

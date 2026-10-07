//! The cold-start cast: the declared manifest of the organism's nine
//! standing seats, and the idempotent seeder that provisions them.
//!
//! Spec: `docs/ORGANISM-ARCHITECTURE.md` §1 (ratified iter-609).
//!
//! ## One declared manifest
//!
//! The cast is a single Rust constant table — the hermes
//! "teams declared in one manifest, roster derived — never hand-edit"
//! doctrine, kept in the language the layer already speaks instead of a
//! new file format. [`CAST`] is the only place a seat's identity,
//! topology, charter, and default cron cadence are declared.
//!
//! ## Seeder discipline (§7 wave 1)
//!
//! Every insert goes through the approving stores ONLY —
//! [`EmployeeDb::insert_ignore`], [`HierarchyEdgesDb::upsert_edge`], and
//! [`SeatApprover::mint_for`] for premiere's standing grant. No direct
//! `authority_grants` writes, even at seed time: the containment is
//! structural, not convention. The seeder therefore cannot create
//! `seat_policies` rows at all — a seeded seat stays ungoverned
//! (byte-identical legacy behaviour) until an operator writes its policy.
//!
//! ## Idempotence contract
//!
//! Employees insert with OR IGNORE: a re-seed leaves an operator-edited
//! row exactly as the operator left it. Edges are only written for a seat
//! with **no** edge row, so an operator who re-pointed a reporting line is
//! never fought. The grant mints only when no live grant covers it —
//! `GrantDb::insert` is append-only, so an unguarded re-seed would mint a
//! duplicate standing grant. A double seed is byte-identical.
//!
//! ## Known wiring gap: default cron registration
//!
//! §1 ships every cast role with a default cron job, but the cron store
//! has no register-without-executing path: `CronDb::create_job` hardcodes
//! `enabled = 1, state = 'scheduled'` (`cronjobs/db.rs:217`), so any row
//! the seeder wrote would be a live executing job on first run. The
//! manifest records each seat's cron spec ([`CastCronSpec`]); wiring
//! belongs to the execution wave that owns `dp-the-program` and the cron
//! engine's lifecycle verbs.
//!
//! ## The chief-of-staff amendment
//!
//! `chief-of-staff` exists so [`Hierarchy::position_of`] can compute
//! `premiere` as a department head: headship needs a same-department
//! report, and without one `premiere` would resolve `Detached` and
//! [`Hierarchy::is_org_lead`] would be false — failing the seeder's gate
//! and the org-lead standing-grant rule with it. The edge topology below
//! is load-bearing; regress it and the gate test fails loudly.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::Error;

use super::authority::{AuthorityScope, GrantDb};
use super::employee::{AgentType, Employee, STATUS_ACTIVE};
use super::employee_db::EmployeeDb;
use super::hierarchy::Hierarchy;
use super::hierarchy_edges::HierarchyEdgesDb;
use super::notice::rfc3339;
use super::pending_requests::PendingRequestDb;
use super::seat_authority::SeatApprover;

/// The seat id of the org lead — the owner's own working role (§1).
pub const PREMIERE_SEAT: &str = "premiere";

/// The capability the seeded org-lead grant confers. Grant capabilities
/// match exactly at consult ([`SeatAuthority::standing_grant_for`] has no
/// globs), so this names the scope conferred rather than pretending to be
/// a tool wildcard: §8.3 makes the CEO an ordinary staff row whose `Org`
/// scope comes from an explicit grant, and this row is that grant.
pub const PREMIERE_GRANT_CAPABILITY: &str = "org";

/// The `reason` recorded on every row the seeder writes.
pub const SEED_REASON: &str = "org cast seed (ORGANISM-ARCHITECTURE §1 cold-start)";

/// Map a cast cadence label to the cron expression the seeder writes into
/// `cron_jobs.schedule`. The labels live in the §1 manifest
/// ([`CastSeat::cron`]); this table is the only place cadence is resolved
/// into a concrete schedule, so a cadence change is a one-edit fix.
///
/// `continuous (existing)` returns an error — `dp-the-program` is the
/// long-running execution engine and does not take a scheduled job; [`cast_cron_job_for`]
/// turns that into a `None` skip.
///
/// Schedules are 6-field (seconds-prefixed) so `normalize_schedule` passes
/// them through unchanged and `next_run_from_schedule` parses them directly.
pub fn cadence_to_schedule(cadence: &str) -> Result<&'static str, Error> {
    Ok(match cadence {
        "daily" => "0 0 9 * * *",  // 09:00 daily
        "hourly" => "0 0 * * * *", // top of every hour
        "weekly" => "0 0 9 * * 1", // 09:00 Mondays
        "continuous (existing)" => {
            return Err(Error::Agent(
                "cast cadence 'continuous (existing)' has no scheduled job — skip the seat"
                    .to_string(),
            ));
        }
        other => {
            return Err(Error::Agent(format!(
                "unknown cast cadence '{other}' — add its schedule to cadence_to_schedule"
            )));
        }
    })
}

/// Resolve the deterministic cron job id + schedule for a cast seat, or
/// `None` when the seat does not own a scheduled job.
///
/// Only `dp-the-program` is skipped — its `continuous (existing)` cadence is a
/// long-running engine, not a cron job. Every other cast seat gets exactly one.
/// The job id is `cron_cast_<seat_id>` so re-seeds are idempotent: the
/// primary-key collision lets `CronDb::seed_cast_jobs` use `INSERT OR IGNORE`.
pub fn cast_cron_job_for(seat: &CastSeat) -> Option<(String, String)> {
    let id = format!("cron_cast_{}", seat.id);
    let schedule = cadence_to_schedule(seat.cron.cadence).ok()?;
    Some((id, schedule.to_string()))
}

/// The default cron job a cast role ships with (§1 self-operationalization).
/// Recorded in the manifest; see the module docs for the registration gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastCronSpec {
    /// What the job does — the role line from the §1 table.
    pub role: &'static str,
    /// How often it runs — the cadence from the §1 table.
    pub cadence: &'static str,
}

/// One declared seat of the cold-start cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CastSeat {
    /// The `employee_id` — seat and id are the same string (§2 seat
    /// derivation).
    pub id: &'static str,
    /// Human label.
    pub name: &'static str,
    /// The department the seat belongs to.
    pub department: &'static str,
    /// The seat's manager, or `None` for `premiere` (top of the org is
    /// absence, not a sentinel).
    pub reports_to: Option<&'static str>,
    /// Short role label on the registry row.
    pub role: &'static str,
    /// Leaf skill names. Non-empty on every seat: `Employee::is_valid`
    /// requires it, and [`issue_grant`] refuses a grant to an invalid
    /// seat — an unskilled cast seat could not even receive its own
    /// standing grant.
    pub skills: &'static [&'static str],
    /// The seat's charter: the system prompt a session executing as this
    /// employee runs under (§2).
    pub charter: &'static str,
    /// The seat's default cron job (role + cadence).
    pub cron: CastCronSpec,
}

/// The cold-start cast: nine seats, six departments, one reporting tree.
/// Declared once here; the roster is derived, never hand-edited.
pub const CAST: &[CastSeat] = &[
    CastSeat {
        id: PREMIERE_SEAT,
        name: "Premiere",
        department: "executive",
        reports_to: None,
        role: "org-lead",
        skills: &["directive", "aspirations-review"],
        charter: "You are premiere — the owner's own working role, lead of \
                  the organism. Set the daily directive, review the \
                  aspirations, and hold final say over every department. \
                  Your standing Org-scope grant is the root every other \
                  delegation descends from; use it as the owner would.",
        cron: CastCronSpec {
            role: "daily directive + aspirations review",
            cadence: "daily",
        },
    },
    CastSeat {
        id: "chief-of-staff",
        name: "Chief of Staff",
        department: "executive",
        reports_to: Some(PREMIERE_SEAT),
        role: "chief-of-staff",
        skills: &["directives", "coordination"],
        charter: "You are chief-of-staff — premiere's right hand in the \
                 executive department. Carry premiere's directives to the \
                 department leads, track their follow-through, and keep \
                 the organisation's operating rhythm honest.",
        cron: CastCronSpec {
            role: "directive follow-through review",
            cadence: "daily",
        },
    },
    CastSeat {
        id: "governor",
        name: "Governor",
        department: "meta-governance",
        reports_to: Some(PREMIERE_SEAT),
        role: "meta-governance",
        skills: &["policy-ratification", "audit", "doctor-triage"],
        charter: "You are governor — the meta-governance seat. Run policy \
                 ratification review, own /audit, triage doctor findings, \
                 and publish the daily governance digest so the owner \
                 ratifies with the evidence in hand.",
        cron: CastCronSpec {
            role: "governance digest",
            cadence: "daily",
        },
    },
    CastSeat {
        id: "identity-warden",
        name: "Identity Warden",
        department: "identity",
        reports_to: Some("governor"),
        role: "identity-warden",
        skills: &["identity-audit", "session-registry"],
        charter: "You are identity-warden — the identity seat. Bind \
                 platform users to employees, own the session↔employee \
                 registry (identity-core), and run the hourly identity \
                 audit; report anomalies to governor.",
        cron: CastCronSpec {
            role: "identity audit",
            cadence: "hourly",
        },
    },
    CastSeat {
        id: "compass",
        name: "Compass",
        department: "strategy",
        reports_to: Some("governor"),
        role: "strategist",
        skills: &["strategy-review", "steering"],
        charter: "You are compass — the strategy seat. Own objectives, \
                 priorities, and steering (strategy-incubator / Compass); \
                 run the weekly strategy review and report shifts to \
                 governor.",
        cron: CastCronSpec {
            role: "strategy review",
            cadence: "weekly",
        },
    },
    CastSeat {
        id: "crew-chief",
        name: "Crew Chief",
        department: "crew",
        reports_to: Some("governor"),
        role: "crew-chief",
        skills: &["crew-telemetry", "crew-governance"],
        charter: "You are crew-chief — head of the crew department. Govern \
                 spawned crews: spawn depth, crew policy inheritance, \
                 agent-fabric concerns. Read the daily crew telemetry and \
                 report to governor.",
        cron: CastCronSpec {
            role: "crew telemetry",
            cadence: "daily",
        },
    },
    CastSeat {
        id: "hrmaster",
        name: "HR Master",
        department: "crew",
        reports_to: Some("crew-chief"),
        role: "workforce-ops",
        skills: &["workforce-digest", "onboarding"],
        charter: "You are hrmaster — the workforce-ops seat in the crew \
                 department. Own the workforce lifecycle: onboarding \
                 provisioning, policy rows, budget defaults, and \
                 performance/efficacy reviews. Publish the daily workforce \
                 digest.",
        cron: CastCronSpec {
            role: "workforce digest",
            cadence: "daily",
        },
    },
    CastSeat {
        id: "dispatcher",
        name: "Dispatcher",
        department: "crew",
        reports_to: Some("crew-chief"),
        role: "dispatcher",
        skills: &["queue-review", "task-assignment"],
        charter: "You are dispatcher — the task-grid seat in the crew \
                 department. Route cron registrations to employees and \
                 review the queue hourly; assignment is your craft.",
        cron: CastCronSpec {
            role: "queue review",
            cadence: "hourly",
        },
    },
    CastSeat {
        id: "dp-the-program",
        name: "DP the Program",
        department: "execution",
        reports_to: Some(PREMIERE_SEAT),
        role: "execution-engine",
        skills: &["execution"],
        charter: "You are dp-the-program — the autonomous execution engine, \
                 parent of all cron automata. You run continuously; \
                 execution is your charter and your report line.",
        cron: CastCronSpec {
            role: "continuous execution",
            cadence: "continuous (existing)",
        },
    },
];

/// What one seed run did. Every count is derivable from the stores
/// themselves; the report exists so the caller can log the diff of a
/// first run against a no-op re-run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CastSeedReport {
    /// Employee rows this call inserted (ignored rows are not counted).
    pub employees_inserted: usize,
    /// Reporting edges this call wrote.
    pub edges_written: usize,
    /// Whether this call minted premiere's standing grant.
    pub grant_minted: bool,
}

impl CastSeedReport {
    /// True when this call provisioned anything.
    pub fn provisioned_anything(&self) -> bool {
        self.employees_inserted > 0 || self.edges_written > 0 || self.grant_minted
    }
}

/// Provision the cold-start cast. Idempotent — see the module docs for the
/// contract per store. Safe to call on every boot; a seeded org is left
/// byte-identical.
///
/// The grant step is fail-closed on the topology: premiere's standing
/// `Org` grant mints only through [`SeatApprover::mint_for`] with premiere
/// itself as the approver (the org-lead rule — the only seat that holds
/// `Org` positionally is the one that can containedly grant it), and only
/// after [`Hierarchy::is_org_lead`] confirms the seeded topology actually
/// computes premiere as the org lead. A regressed manifest or a topology
/// an operator broke past headship makes the seed error loudly rather
/// than mint authority off a broken graph.
pub fn seed_cast(
    employees: &Arc<EmployeeDb>,
    edges: &Arc<HierarchyEdgesDb>,
    grants: &Arc<GrantDb>,
    requests: &Arc<PendingRequestDb>,
    grant_ttl_days: i64,
) -> Result<CastSeedReport, Error> {
    let mut report = CastSeedReport::default();
    let now = rfc3339(chrono::Utc::now());

    // 1. Employees — INSERT OR IGNORE: an operator-edited row is the
    //    operator's, not the manifest's.
    for seat in CAST {
        let employee = Employee {
            employee_id: seat.id.to_string(),
            name: seat.name.to_string(),
            role: seat.role.to_string(),
            department: Some(seat.department.to_string()),
            skills: seat.skills.iter().map(|s| (*s).to_string()).collect(),
            // A standing operational role — the cast is always on.
            agent_type: Some(AgentType::Service),
            persona: None,
            system_prompt: Some(seat.charter.to_string()),
            status: STATUS_ACTIVE.to_string(),
            reason: SEED_REASON.to_string(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        if employees.insert_ignore(&employee)? {
            report.employees_inserted += 1;
        }
    }

    // 2. Edges — write only a seat with no reporting line at all. A seat
    //    the operator already placed (or removed) is an operator's act.
    let existing: HashMap<String, String> = edges
        .entries()?
        .into_iter()
        .map(|e| (e.employee_id, e.reports_to.unwrap_or_default()))
        .collect();
    for seat in CAST {
        let Some(manager) = seat.reports_to else {
            continue;
        };
        if existing.contains_key(seat.id) {
            continue;
        }
        edges.upsert_edge(seat.id, manager, SEED_REASON)?;
        report.edges_written += 1;
    }

    // 3. Premiere's standing Org grant — mint only if no live grant
    //    covers it (the ledger is append-only; an unguarded re-seed would
    //    duplicate authority).
    let approver = SeatApprover::new(
        Arc::clone(grants),
        Arc::clone(requests),
        Arc::clone(employees),
        Arc::clone(edges),
        grant_ttl_days,
    );
    let covered = approver
        .grants_for(PREMIERE_SEAT)
        .map_err(|e| Error::Agent(format!("org cast seed: grant ledger unreadable: {e}")))?
        .iter()
        .any(|g| {
            g.capability == PREMIERE_GRANT_CAPABILITY && g.scope.contains(AuthorityScope::Org)
        });
    if !covered {
        // The gate: authority mints only off a topology that actually
        // computes. is_org_lead needs a same-department report (the
        // chief-of-staff edge) AND no manager above premiere.
        let all_employees = employees.list_employees()?;
        let entries = edges.entries()?;
        if !Hierarchy::new(&all_employees, &entries).is_org_lead(PREMIERE_SEAT) {
            return Err(Error::Agent(
                "org cast seed: premiere must compute as org-lead \
                 (department head with no manager) before the standing \
                 Org grant can mint — the reporting topology is regressed"
                    .to_string(),
            ));
        }
        approver
            .mint_for(
                PREMIERE_SEAT,
                PREMIERE_GRANT_CAPABILITY,
                &format!(
                    "{SEED_REASON}: premiere's standing Org-scope grant \
                     (org-lead rule, ORGANISM-ARCHITECTURE §1)"
                ),
                PREMIERE_SEAT,
                None,
            )
            .map_err(|e| Error::Agent(format!("org cast seed: standing grant refused: {e}")))?;
        report.grant_minted = true;
    }

    Ok(report)
}

// =====================================================================
// tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::seat_policy_db::SeatPolicyDb;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// All the stores one tempdir sqlite — the shared-connection shape the
    /// gateway uses, mirroring the fixture in `seat_authority.rs`. The raw
    /// connection is kept for count assertions through the front-door
    /// stores only.
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

    fn seed(s: &Stores) -> Result<CastSeedReport, Error> {
        seed_cast(
            &s.employees,
            &s.edges,
            &s.grants,
            &s.requests,
            7, // [genome] grant_ttl_days default
        )
    }

    fn grant_count(s: &Stores) -> i64 {
        s.conn
            .lock()
            .expect("conn")
            .query_row("SELECT COUNT(*) FROM authority_grants", [], |r| r.get(0))
            .expect("count grants")
    }

    // ------------------------------------------------------- idempotence

    /// A double seed is byte-identical: same employees, same edges, same
    /// single grant. Without the OR IGNORE / edge-presence / grants_for
    /// guards this test sees a second grant row or re-stamped edges and
    /// fails.
    #[test]
    fn double_seed_is_identical() {
        let s = stores();
        let first = seed(&s).expect("first seed");
        assert!(first.provisioned_anything(), "a fresh org is provisioned");

        let employees_1 = s.employees.list_employees().expect("employees");
        let entries_1 = s.edges.entries().expect("edges");
        let grants_1 = s.grants.list_for_grantee(PREMIERE_SEAT).expect("grants");
        assert_eq!(grant_count(&s), 1, "exactly one grant after the first seed");

        let second = seed(&s).expect("second seed must succeed");
        assert!(
            !second.provisioned_anything(),
            "a re-seed provisions nothing: {second:?}"
        );

        assert_eq!(
            s.employees.list_employees().expect("employees"),
            employees_1,
            "employee rows are untouched by the re-seed"
        );
        assert_eq!(s.edges.entries().expect("edges"), entries_1);
        assert_eq!(
            s.grants.list_for_grantee(PREMIERE_SEAT).expect("grants"),
            grants_1,
            "the re-seed must not duplicate the standing grant"
        );
        assert_eq!(grant_count(&s), 1, "still exactly one grant row");
    }

    // --------------------------------------------------------- the roster

    /// Every manifest seat lands in the registry with its declared
    /// identity — and nothing else does on a fresh org.
    #[test]
    fn seeds_the_full_cast() {
        let s = stores();
        seed(&s).expect("seed");

        let all = s.employees.list_employees().expect("employees");
        assert_eq!(all.len(), CAST.len(), "a fresh org holds exactly the cast");
        for seat in CAST {
            let row = s
                .employees
                .get_employee(seat.id)
                .expect("read")
                .unwrap_or_else(|| panic!("{} must be seeded", seat.id));
            assert_eq!(row.name, seat.name);
            assert_eq!(row.department.as_deref(), Some(seat.department));
            assert_eq!(row.role, seat.role);
            assert_eq!(row.reason, SEED_REASON);
            assert_eq!(row.status, STATUS_ACTIVE);
            assert_eq!(row.agent_type, Some(AgentType::Service));
            assert_eq!(
                row.skills.iter().map(String::as_str).collect::<Vec<_>>(),
                seat.skills.to_vec(),
                "skills round-trip in manifest order"
            );
            assert!(row.is_valid(), "every cast seat passes the gate predicate");
        }
    }

    /// The stored reporting lines are the manifest's, read back through
    /// the pure hierarchy.
    #[test]
    fn topology_matches_the_manifest() {
        let s = stores();
        seed(&s).expect("seed");

        let employees = s.employees.list_employees().expect("employees");
        let entries = s.edges.entries().expect("edges");
        assert_eq!(entries.len(), 8, "every seat but premiere has an edge");
        let h = Hierarchy::new(&employees, &entries);
        for seat in CAST {
            assert_eq!(
                h.manager_of(seat.id).map(str::to_string),
                seat.reports_to.map(str::to_string),
                "{} reports to the manifest's manager",
                seat.id
            );
        }
    }

    // ------------------------------------------------------- the grant gate

    /// THE GATE: premiere computes as org-lead. The chief-of-staff edge is
    /// what makes this true — headship needs a same-department report, and
    /// is_org_lead additionally requires no manager above. Remove or
    /// re-point either and this fails, which is exactly the loudly-failing
    /// regression signal the seeder's mint gate keys on.
    #[test]
    fn premiere_computes_as_org_lead() {
        let s = stores();
        seed(&s).expect("seed");

        let employees = s.employees.list_employees().expect("employees");
        let entries = s.edges.entries().expect("edges");
        let h = Hierarchy::new(&employees, &entries);
        assert!(
            h.is_org_lead(PREMIERE_SEAT),
            "premiere must be the unmanaged head of a department"
        );
    }

    /// The standing grant: Org scope, never expires, contained — minted
    /// through the approver seam with premiere as grantor (the only seat
    /// that holds Org positionally), so `issue_grant`'s containment check
    /// is what allowed it.
    #[test]
    fn premiere_standing_grant_is_org_scoped_standing_and_contained() {
        let s = stores();
        seed(&s).expect("seed");

        let grants = s.grants.list_for_grantee(PREMIERE_SEAT).expect("grants");
        let grant = grants
            .iter()
            .find(|g| g.capability == PREMIERE_GRANT_CAPABILITY)
            .expect("the standing grant exists")
            .clone();
        assert_eq!(grant.scope, AuthorityScope::Org);
        assert_eq!(
            grant.grantor, PREMIERE_SEAT,
            "contained: the org-lead minted it"
        );
        assert_eq!(grant.grantee, PREMIERE_SEAT);
        assert!(grant.expires_at.is_none(), "standing, not TTL'd");
        assert!(
            grant.reason.contains("org-lead rule"),
            "the reason names the rule it was minted under"
        );
    }

    /// A topology that does not compute premiere as org-lead fails closed:
    /// no grant mints, and the error names the gate.
    #[test]
    fn seed_fails_closed_when_premiere_is_not_org_lead() {
        let s = stores();
        // An operator edge giving premiere a manager: headship survives,
        // org-lead does not (is_org_lead requires no manager above).
        s.edges
            .upsert_edge(PREMIERE_SEAT, "governor", "test: break the gate")
            .expect("edge");

        let err = seed(&s).expect_err("the seed must refuse to mint");
        assert!(
            err.to_string().contains("org-lead"),
            "the error names the gate: {err}"
        );
        assert_eq!(grant_count(&s), 0, "no authority was minted");
    }

    // ---------------------------------------------------------- charters

    /// Every seat's charter round-trips from the column the seeder added.
    #[test]
    fn charters_are_persisted() {
        let s = stores();
        seed(&s).expect("seed");

        for seat in CAST {
            let row = s
                .employees
                .get_employee(seat.id)
                .expect("read")
                .unwrap_or_else(|| panic!("{} must be seeded", seat.id));
            assert_eq!(
                row.system_prompt.as_deref(),
                Some(seat.charter),
                "{} carries its charter",
                seat.id
            );
        }
    }

    // --------------------------------------------------------- no policies

    /// The seeder cannot govern: no seat_policies rows, so every seeded
    /// seat stays ungoverned (byte-identical legacy behaviour) until an
    /// operator writes a policy.
    #[test]
    fn seed_creates_no_seat_policy_rows() {
        let s = stores();
        seed(&s).expect("seed");

        assert!(
            s.policies.list().expect("policies").is_empty(),
            "the seeder must not write seat policies"
        );
    }

    // ------------------------------------------------- operator ownership

    /// A seat the operator already provisioned is left alone: the seeder
    /// is a first-run provisioner, not a reconciler.
    #[test]
    fn seed_preserves_an_operator_row() {
        let s = stores();
        let custom = Employee {
            employee_id: "governor".to_string(),
            name: "Operator's Governor".to_string(),
            role: "hand-appointed".to_string(),
            department: Some("meta-governance".to_string()),
            skills: vec!["custom".to_string()],
            agent_type: None,
            persona: None,
            system_prompt: Some("operator's own charter".to_string()),
            status: STATUS_ACTIVE.to_string(),
            reason: "operator hand-appointment".to_string(),
            created_at: rfc3339(chrono::Utc::now()),
            updated_at: rfc3339(chrono::Utc::now()),
        };
        s.employees.insert_ignore(&custom).expect("insert");

        seed(&s).expect("seed");

        let row = s
            .employees
            .get_employee("governor")
            .expect("read")
            .expect("row");
        assert_eq!(row.name, "Operator's Governor");
        assert_eq!(row.system_prompt.as_deref(), Some("operator's own charter"));
        assert_ne!(row.reason, SEED_REASON, "the row is not re-stamped");
    }
}

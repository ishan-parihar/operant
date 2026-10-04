//! Wave 1 — the fail-closed identity gate (packet B).
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.2, "Backfill + fail-closed rule".
//! Acceptance: `docs/ORGANISM-OS-UPGRADE-OUTLINE.md` §4 Wave 1 (the "Gate"
//! line) — *"a job missing any of the identity fields **blocks the tick**
//! (not a warning)."*
//!
//! ## What this module is
//!
//! A **pure** decision function. Given the employee record resolved for a cron
//! job, it decides whether that job may tick. It performs no I/O, reads no
//! config, and holds no state, so every branch of the fail-closed contract is
//! reachable from a unit test without a database, an agent, or a network.
//!
//! The sqlite read that *resolves* the record is deliberately not here. It
//! arrives as a resolved `Option<&Employee>` so this gate does not depend on
//! when `EmployeeDb` landed, and so the tick path stays testable.
//!
//! ## Fail-closed
//!
//! The three ways a lookup can come back empty are all blocks, not passes:
//!
//! 1. no employee row for the job at all,
//! 2. an employee row whose `employee_id` is blank or does not match the id
//!    derived from the job (the join failed — see §3.1.1, where `employee_id`
//!    is `format!("emp-{}", &job.id[..12])`),
//! 3. an employee row that parses but is missing required fields.
//!
//! Case 2 is the one that is easy to get wrong and expensive when it is: a
//! lookup that "succeeds" against the wrong row is exactly the drift
//! `operant org check` exists to catch, and passing it would let a job run
//! under a borrowed identity.
//!
//! ## Dark-mergeability
//!
//! The gate is **not reachable unless a caller installs an [`IdentityGate`] on
//! the scheduler**. There is no config key, no default-on construction, and no
//! path from today's boot code to a gate instance. With nothing installed the
//! tick path takes an early return that is byte-identical to the pre-gate
//! code — same jobs, same order, same dispatch. See
//! `docs/harness-kernel.md:7` for the precedent this follows: *"dark-mergeable:
//! the default config keeps it disabled, and the legacy boot path is
//! byte-stable when it is off."* A gate that blocked every existing job on
//! upgrade would be a production outage, not a feature.
//!
//! ## The message contract
//!
//! Every block renders through [`GateBlock::message`], which is the single
//! source of the greppable `blocked by org gate:` prefix. The prefix is
//! machine-readable in the same spirit as the existing `blocked by security
//! policy:` marker, and the message names **the offending job and every
//! missing field**. `BUGS.md` R41-1 is the bug this shape exists to prevent:
//! an error "naming a symptom the operator cannot act on" (`compile()`
//! returning "pool manifest has empty `name`" for a file that had no
//! top-level `name` at all). An operator reading this message must be able to
//! name the job, name the fields, and know the command that fixes it — with no
//! reference to this source file.

use super::employee::Employee;
use super::employee_db::EmployeeDb;

/// The machine-readable prefix on every gate block. Callers and operators
/// grep for this; do not reword it without updating the CLI surfaces.
pub const GATE_BLOCK_PREFIX: &str = "blocked by org gate";

/// The set of the three required identity fields, in §3.2's order.
///
/// Exposed as a slice rather than three constants so a test can drive the
/// "one case per field" battery without hand-maintaining a parallel list.
pub const REQUIRED_IDENTITY_FIELDS: [&str; 3] = ["employee_id", "name", "skills"];

/// Resolves the employee record for a cron job, or `None` if the job has no
/// employee.
///
/// Implemented by whatever store owns the registry (packet A's `EmployeeDb`
/// against `operant_kanban.db`). A trait rather than a direct `EmployeeDb`
/// call so this gate compiles and is testable independently of the store, and
/// so an install can back the gate with a different source without editing it.
pub trait EmployeeLookup {
    /// The employee record for `cron_job_id`, or `None` when the job has no
    /// employee row. Returning `Err` is not an option: an unreachable store
    /// must fail *closed*, and the honest way to do that is to return `None`
    /// and let the gate block. A store that cannot answer has not proven the
    /// job's identity.
    fn lookup_employee(&self, cron_job_id: &str) -> Option<Employee>;
}

/// A decision the gate made, with the reason attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// The job's identity is complete. It may tick.
    Allow { employee_id: String },
    /// The job's identity is incomplete. It may **not** tick, and it will not
    /// be retried — the next attempt is its next scheduled tick.
    Block(GateBlock),
}

/// Why the gate stopped a job, in a form the operator can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateBlock {
    /// No employee row exists for this job at all.
    ///
    /// §3.1.2: a `CronJob` lacking required identity fields "gets **no
    /// row**" — never a partial one, because "a half-employee that cannot
    /// carry `skills` is worse than a missing employee, because the missing one
    /// is visible."
    NoEmployeeRecord { job_id: String },
    /// An employee row exists but the join to this job did not hold — the
    /// row's `employee_id` is not the id derived from `job_id` (§3.1.1). The
    /// job would otherwise run under a borrowed identity.
    IdentityMismatch {
        job_id: String,
        found_employee_id: String,
    },
    /// An employee row exists and joins correctly, but required fields are
    /// absent. `missing` holds the field names in §3.2's order.
    MissingFields {
        job_id: String,
        employee_id: String,
        missing: Vec<&'static str>,
    },
}

impl GateBlock {
    /// The actionable, greppable block message.
    ///
    /// Names the job, the missing fields, and the fix. Format is fixed by
    /// §3.2's "Exact block message" and must stay greppable.
    pub fn message(&self) -> String {
        match self {
            Self::NoEmployeeRecord { job_id } => format!(
                "{GATE_BLOCK_PREFIX}: cron job '{job_id}' has no valid employee record \
                 (missing: employee record); fix with `operant org sync` or \
                 `operant org backfill` and re-run"
            ),
            Self::IdentityMismatch {
                job_id,
                found_employee_id,
            } => format!(
                "{GATE_BLOCK_PREFIX}: cron job '{job_id}' has no valid employee record \
                 (missing: employee_id; the registry row '{found_employee_id}' does not match \
                 the id derived from this job); fix with `operant org sync` or \
                 `operant org backfill` and re-run"
            ),
            Self::MissingFields {
                job_id,
                employee_id,
                missing,
            } => format!(
                "{GATE_BLOCK_PREFIX}: cron job '{job_id}' has no valid employee record \
                 (missing: {}; employee_id: {employee_id}); fix with `operant org sync` or \
                 `operant org backfill` and re-run",
                missing.join(", ")
            ),
        }
    }

    /// The job this block is about, for callers that log or report by id.
    pub fn job_id(&self) -> &str {
        match self {
            Self::NoEmployeeRecord { job_id }
            | Self::IdentityMismatch { job_id, .. }
            | Self::MissingFields { job_id, .. } => job_id,
        }
    }

    /// The names of the required identity fields this block is about, in
    /// §3.2's order. Never empty — including for [`Self::NoEmployeeRecord`],
    /// where the absent thing *is* the record, reported as `employee record`.
    pub fn missing_fields(&self) -> Vec<&'static str> {
        match self {
            Self::NoEmployeeRecord { .. } => vec!["employee record"],
            Self::IdentityMismatch { .. } => vec!["employee_id"],
            Self::MissingFields { missing, .. } => missing.clone(),
        }
    }
}

/// The gate. Constructed only by a caller that has decided to enforce, and
/// handed to the scheduler; its absence is the "org layer off" state.
pub struct IdentityGate {
    lookup: Box<dyn EmployeeLookup + Send + Sync>,
}

impl std::fmt::Debug for IdentityGate {
    /// Hand-written: the boxed lookup is not required to be `Debug`, and a
    /// gate that panics in `{:?}` inside an error path helps nobody.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdentityGate { lookup: <opaque> }")
    }
}

impl IdentityGate {
    /// Build a gate over `lookup`.
    pub fn new(lookup: Box<dyn EmployeeLookup + Send + Sync>) -> Self {
        Self { lookup }
    }

    /// Decide whether `job_id` may tick.
    ///
    /// Fails closed at every step: no record blocks, a non-joining record
    /// blocks, an incomplete record blocks. `expected_employee_id` is the id
    /// §3.1.1 derives from the job (`emp-<first 12 of job id>`); passing it
    /// rather than re-deriving inside the gate keeps one derivation
    /// authoritative — packet A's `derive_employee_id`.
    pub fn check(&self, job_id: &str, expected_employee_id: &str) -> GateDecision {
        let Some(employee) = self.lookup.lookup_employee(job_id) else {
            return GateDecision::Block(GateBlock::NoEmployeeRecord {
                job_id: job_id.to_string(),
            });
        };

        // The join must hold. A row that is present but belongs to a
        // different job is not this job's identity.
        if employee.employee_id.trim() != expected_employee_id.trim() {
            return GateDecision::Block(GateBlock::IdentityMismatch {
                job_id: job_id.to_string(),
                found_employee_id: employee.employee_id,
            });
        }

        let missing = employee.missing_required_fields();
        if !missing.is_empty() {
            return GateDecision::Block(GateBlock::MissingFields {
                job_id: job_id.to_string(),
                employee_id: employee.employee_id,
                missing,
            });
        }

        GateDecision::Allow {
            employee_id: employee.employee_id,
        }
    }
}

/// An [`EmployeeLookup`] backed by the real [`EmployeeDb`] registry.
///
/// Bridges the two key shapes: the trait is keyed by **cron job id** (that is
/// what the tick holds) while the store is keyed by **employee id**. The
/// bridge is §3.1.1's derivation, `emp-<first 12 of job id>`, and it is the
/// *only* place that translation happens.
///
/// **Fails closed on a store error.** `get_employee` returns `Result`, but a
/// database that cannot answer has not proven the job's identity, so an `Err`
/// collapses to `None` — which the gate reports as `NoEmployeeRecord`. The
/// block message names the job and the fix. Swallowing the underlying error
/// is deliberate: the alternative is a fail-*open* path where a locked or
/// corrupt DB silently unlocks every job on the fleet, and the gate's whole
/// purpose is to be the thing that does not fail open. The error is not lost
/// entirely — the surrounding tick logs the block at `error!` with the job id.
pub struct EmployeeDbLookup {
    db: std::sync::Arc<EmployeeDb>,
}

impl EmployeeDbLookup {
    /// Wrap a shared registry handle.
    pub fn new(db: std::sync::Arc<EmployeeDb>) -> Self {
        Self { db }
    }
}

impl EmployeeLookup for EmployeeDbLookup {
    fn lookup_employee(&self, cron_job_id: &str) -> Option<Employee> {
        let employee_id = super::employee::derive_employee_id(cron_job_id);
        match self.db.get_employee(&employee_id) {
            Ok(found) => found,
            Err(e) => {
                tracing::error!(
                    "org gate: employee lookup failed for cron job {cron_job_id} \
                     (employee_id {employee_id}); treating as no employee record: {e}"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn employee(id: &str) -> Employee {
        Employee {
            employee_id: id.to_string(),
            name: "Nightly digest".to_string(),
            role: "automator".to_string(),
            department: None,
            skills: vec!["summarize".to_string()],
            agent_type: None,
            persona: None,
            system_prompt: None,
            status: "active".to_string(),
            reason: "test".to_string(),
            created_at: "2026-09-30T00:00:00Z".to_string(),
            updated_at: "2026-09-30T00:00:00Z".to_string(),
        }
    }

    struct Fixed(Option<Employee>);

    impl EmployeeLookup for Fixed {
        fn lookup_employee(&self, _cron_job_id: &str) -> Option<Employee> {
            self.0.clone()
        }
    }

    fn gate_with(emp: Option<Employee>) -> IdentityGate {
        IdentityGate::new(Box::new(Fixed(emp)))
    }

    #[test]
    fn complete_employee_allows() {
        let gate = gate_with(Some(employee("emp-cron_1234abcd")));
        assert_eq!(
            gate.check("cron_1234abcd", "emp-cron_1234abcd"),
            GateDecision::Allow {
                employee_id: "emp-cron_1234abcd".to_string()
            }
        );
    }

    #[test]
    fn no_employee_record_blocks() {
        let gate = gate_with(None);
        let GateDecision::Block(block) = gate.check("cron_1234abcd", "emp-cron_1234abcd") else {
            panic!("absent employee record must block, fail closed");
        };
        assert_eq!(block.job_id(), "cron_1234abcd");
        assert_eq!(block.missing_fields(), vec!["employee record"]);
        assert!(block.message().contains("cron_1234abcd"));
        assert!(block.message().starts_with(GATE_BLOCK_PREFIX));
    }

    #[test]
    fn identity_mismatch_blocks() {
        // A row that exists but belongs to a different job.
        let gate = gate_with(Some(employee("emp-someone_else")));
        let GateDecision::Block(block) = gate.check("cron_1234abcd", "emp-cron_1234abcd") else {
            panic!("a row that does not join must block");
        };
        assert_eq!(block.missing_fields(), vec!["employee_id"]);
        assert!(block.message().contains("emp-someone_else"));
    }

    /// One case per required field, as §3.2 lists them.
    #[test]
    fn each_required_field_missing_blocks_individually() {
        for field in REQUIRED_IDENTITY_FIELDS {
            let mut emp = employee("emp-cron_1234abcd");
            match field {
                "employee_id" => emp.employee_id = "   ".to_string(),
                "name" => emp.name = "  ".to_string(),
                "skills" => emp.skills = Vec::new(),
                other => panic!("unhandled required field {other}"),
            }
            // The blank-employee_id case would otherwise trip the join check
            // first; for the field-under-test to be the *reported* miss, the
            // join target must equal the blanked value.
            let expected = if field == "employee_id" {
                "   ".to_string()
            } else {
                "emp-cron_1234abcd".to_string()
            };
            let gate = gate_with(Some(emp));
            let GateDecision::Block(block) = gate.check("cron_1234abcd", &expected) else {
                panic!("missing {field} must block");
            };
            assert_eq!(
                block.missing_fields(),
                vec![field],
                "blocking on the wrong field for {field}"
            );
            let msg = block.message();
            assert!(msg.contains(field), "message must name {field}: {msg}");
            assert!(msg.starts_with(GATE_BLOCK_PREFIX), "unanchored: {msg}");
        }
    }

    #[test]
    fn message_names_job_and_fix() {
        // A *mismatched* employee id is the only variant whose message carries
        // the job id together with both remediation commands. A complete
        // employee whose id already matches legitimately returns `Allow`, so
        // passing one here would assert a block that can never occur.
        let gate = gate_with(Some(employee("emp-cron_1234abcd")));
        let GateDecision::Block(block) = gate.check("cron_deadbeef", "emp-cron_deadbeef") else {
            panic!("expected block");
        };
        let msg = block.message();
        for needle in ["cron_deadbeef", "operant org sync", "operant org backfill"] {
            assert!(msg.contains(needle), "message must contain {needle}: {msg}");
        }
    }
}

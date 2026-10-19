//! The Holon-run seat authority (phase C: policy.json becomes the live fence).
//!
//! When `HOLON_RUN_DIR` is set and `policy.json` sits in it — i.e. the Holon
//! kernel claimed this run and handed us its compiled reach — the agent mounts
//! a [`SeatAuthority`] whose [`SeatPolicySource`] is that document. Tool calls
//! then flow through the loop's existing consult site:
//!
//! - observe: lockdown with the observer allow list; a mutating tool escalates,
//!   and an unattended run queues + denies it this run (the loop's F2 semantics).
//! - act: yolo with the document's deny list; the hardline blocklist still
//!   fires first (infrastructure, not policy).
//!
//! No Holon run (no `HOLON_RUN_DIR`) = no authority = today's behavior,
//! byte-identical — invariant 5. A Holon run whose `policy.json` is missing is
//! refused, not waved through (OS-3): the kernel always writes the document.

use std::sync::Arc;

use operant_core::org::seat_authority::SeatAuthority;
use operant_core::org::seat_policy::{SeatPolicy, SeatPolicySource};

use crate::policy::PolicyDoc;

/// The seat id a Holon run runs as: the kernel exports `HOLON_EMPLOYEE_ID`
/// (O-02); the legacy `HOLON_EMPLOYEE` and the run id keep the consult
/// self-consistent on older kernels.
pub fn holon_seat() -> String {
    crate::run_env::holon_employee()
        .or_else(|| std::env::var("HOLON_RUN_ID").ok())
        .unwrap_or_else(|| "holon-run".to_string())
}

/// The policy document for THIS run, when this is a Holon run at all.
///
/// `None` = not a Holon run (personal mode, byte-identical). A Holon run with
/// no `policy.json` is an `Err` (OS-3): fail closed, never an unfenced agent.
pub fn run_policy() -> Option<crate::Result<PolicyDoc>> {
    policy_in(crate::run_env::holon_run_dir())
}

/// The policy for a run directory: `None` outside a Holon run, else the
/// document `PolicyDoc::load` reads (a missing one is `PolicyMissing`).
fn policy_in(run_dir: Option<std::path::PathBuf>) -> Option<crate::Result<PolicyDoc>> {
    run_dir.map(|dir| PolicyDoc::load(&dir))
}

/// One run's policy, exposed to the agent loop's consult as this seat's row.
struct RunPolicySource {
    seat: String,
    policy: PolicyDoc,
}

impl SeatPolicySource for RunPolicySource {
    fn policy_for(&self, employee_id: &str) -> Option<SeatPolicy> {
        (employee_id == self.seat).then(|| self.policy.seat_policy())
    }
}

/// Build the seat authority for a Holon run, or `Ok(None)` when this is not
/// one. Databases live under the isolated operant home (`HERMES_HOME`) — the
/// same home the adapter installer prepared — so escalations from Holon runs
/// never touch the operator's own org stores.
pub fn holon_authority(_home: &std::path::Path) -> crate::Result<Option<Arc<SeatAuthority>>> {
    let Some(loaded) = run_policy() else {
        return Ok(None);
    };
    let policy = loaded?;
    // G+1: the authority is policy-only now — the grant ledger and the
    // durable ask queue retired to Holon (an approved plan IS the grant).
    Ok(Some(Arc::new(SeatAuthority::new(Arc::new(
        RunPolicySource {
            seat: holon_seat(),
            policy,
        },
    )))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_holon_run_without_a_policy_document_is_refused_not_unfenced() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            policy_in(Some(dir.path().to_path_buf())),
            Some(Err(crate::Error::PolicyMissing(_)))
        ));
    }

    #[test]
    fn outside_a_holon_run_there_is_no_policy() {
        assert!(policy_in(None).is_none());
    }
}

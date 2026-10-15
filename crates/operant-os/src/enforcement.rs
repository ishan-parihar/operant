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
//! No Holon run (no env var, no document) = no authority = today's behavior,
//! byte-identical — invariant 5.

use std::path::PathBuf;
use std::sync::Arc;

use operant_core::org::seat_authority::SeatAuthority;
use operant_core::org::seat_policy::{SeatPolicy, SeatPolicySource};

use crate::policy::PolicyDoc;

/// The seat id a Holon run runs as: the kernel exports `HOLON_EMPLOYEE`.
/// Falls back to the run id so the consult always names *something*.
pub fn holon_seat() -> String {
    std::env::var("HOLON_EMPLOYEE")
        .or_else(|_| std::env::var("HOLON_RUN_ID"))
        .unwrap_or_else(|_| "holon-run".to_string())
}

/// The policy document for THIS run, when this is a Holon run at all.
///
/// `None` = not a Holon run (personal mode, byte-identical) or a run the
/// kernel launched before phase C (no policy.json yet) — same no-op.
pub fn run_policy() -> Option<crate::Result<PolicyDoc>> {
    let dir: PathBuf = std::env::var_os("HOLON_RUN_DIR")?.into();
    if !dir.join("policy.json").exists() {
        return None;
    }
    Some(PolicyDoc::load(&dir))
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

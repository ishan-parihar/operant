//! Escalation core — the post-supersession keep-shape (docs/agentic-os/,
//! G+1 by owner order 2026-10-15).
//!
//! What survives Holon's supersession of the organism (the full history is
//! in `docs/ORGANISM-ARCHITECTURE.md` + `docs/ORG-AUTHORITY-ARCHITECTURE.md`):
//!
//! - [`SeatAuthority::consult`] — the policy-driven verdict for one tool
//!   call. Org mode: the policy source is Holon's `policy.json` (phase C);
//!   personal mode: the operator's `seat_policies` rows. There is no grant
//!   ledger anymore — an approved plan IS the grant (compiled to
//!   `policy.json` by Holon), so `has_grant` is structurally false.
//! - [`QueuedAsk`] / [`SeatEscalation`] — the escalation identity threaded
//!   from the run guard into the permission request, so an interactive
//!   approver can resolve the ask it was actually asked about.
//! - [`SeatApprover`] — resolution only: an interactive approval/denial
//!   resolves the in-flight ask. The durable ask queue, the grant minting
//!   and the hierarchy topology retired to Holon (board + approvals;
//!   org-lead authority by grade). A lapse expires unresolved, exactly as
//!   the unattended path always did.
//!
//! In org mode an `Escalate` verdict never reaches this file's queue at all
//! when no interactive channel exists — the run guard turns it into a
//! refusal (`result.refusals[]`, phase C).

use std::sync::Arc;

use crate::org::seat_policy::{SeatDecision, SeatPolicySource, decide};

// =====================================================================
// SeatAuthority — the consult half
// =====================================================================

/// Consults the policy source for one seat's tool call. The grant ledger,
/// the durable ask queue and the employee registry the delegation tier
/// used are gone with the organism — this is the policy engine plus the
/// escalation identity, nothing else.
pub struct SeatAuthority {
    policy: Arc<dyn SeatPolicySource>,
}

impl SeatAuthority {
    pub fn new(policy: Arc<dyn SeatPolicySource>) -> Self {
        Self { policy }
    }

    /// Consult the policy for one tool call. `None` = ungoverned (no source
    /// or no policy row): the caller must reproduce the ungoverned
    /// behaviour exactly. `blocked` stays `false` here for the reason slice
    /// E recorded: the smart approval gate above the consult has already
    /// denied blocked tools, and the hardline floor must fire before any
    /// seat mode can see.
    ///
    /// There is no grant to consult anymore — an approved plan is the grant
    /// (Holon compiles it to `policy.json`), so `has_grant` is structurally
    /// `false` and the verdict is the policy's alone.
    pub fn consult(&self, seat: &str, tool: &str, dangerous: bool) -> Option<SeatDecision> {
        let policy = self.policy.policy_for(seat)?;
        Some(decide(Some(&policy), tool, false, dangerous, false))
    }

    /// The escalation identity for an `Escalate` verdict — what the run
    /// guard threads into the permission request. No durable row is
    /// written; the queue that made asks survive restarts retired to
    /// Holon's approvals.
    pub fn escalate(&self, seat: &str, tool: &str, why: &str) -> QueuedAsk {
        QueuedAsk {
            escalation: Some(SeatEscalation {
                request_id: format!("pr_{}", uuid::Uuid::new_v4()),
                employee_id: seat.to_string(),
                tool: tool.to_string(),
                why: why.to_string(),
            }),
            note: why.to_string(),
        }
    }
}

// =====================================================================
// The escalation identity
// =====================================================================

/// What `escalate()` hands the run guard: the identity of the ask plus the
/// note to surface. `escalation = None` marks the classic ungoverned
/// prompt — byte-identical to the pre-organism flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedAsk {
    pub escalation: Option<SeatEscalation>,
    pub note: String,
}

/// The queue-side identity of one escalation, threaded from the run guard
/// into `ToolPermissionRequest` so the dispatcher's approval can resolve
/// the ask it was actually asked about. `None` on a request means the
/// ungoverned prompt — no escalation, byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatEscalation {
    pub request_id: String,
    /// The REQUESTING seat (the employee whose `decide()` escalated).
    pub employee_id: String,
    /// The tool the seat wants to run.
    pub tool: String,
    /// `decide()`'s why — the judgeable note the approver rules on.
    pub why: String,
}

// =====================================================================
// SeatApprover — resolution only
// =====================================================================

/// Resolves in-flight escalations for the interactive (gateway) path.
/// Approval and denial resolve the ask; there is no grant to mint and no
/// durable row to record — Holon's board and approvals own the durable
/// half now. The resolver's name is still recorded on the resolution
/// string an operator sees.
pub struct SeatApprover {
    approver_id: String,
}

impl SeatApprover {
    pub fn new(approver_id: impl Into<String>) -> Self {
        Self {
            approver_id: approver_id.into(),
        }
    }

    /// The seat every resolution is attributed to.
    pub fn approver_of(&self, _employee_id: &str) -> String {
        self.approver_id.clone()
    }

    /// Resolve an escalation by approving it. The permission channel has
    /// already unblocked the waiting tool call; this records the verdict
    /// the caller surfaces.
    pub fn approve(&self, ask: &SeatEscalation) -> Result<String, String> {
        Ok(format!(
            "approved by {}: {} may run {} ({})",
            self.approver_id, ask.employee_id, ask.tool, ask.why
        ))
    }

    /// Resolve an escalation by denying it.
    pub fn deny(&self, ask: &SeatEscalation) -> Result<(), String> {
        let _ = ask;
        Ok(())
    }

    /// A lapsed ask (60s interactive window, or an unattended run) expires
    /// unresolved — the waiting tool call already saw its denial.
    pub fn expire(&self, ask: &SeatEscalation) -> Result<(), String> {
        let _ = ask;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::seat_policy::{SeatMode, SeatPolicy, SeatPolicySource};

    struct StaticSource(Option<SeatPolicy>);
    impl StaticSource {
        fn lockdown() -> Self {
            Self(Some(SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: Vec::new(),
                deny: Vec::new(),
            }))
        }
        fn empty() -> Self {
            Self(None)
        }
    }
    impl SeatPolicySource for StaticSource {
        fn policy_for(&self, _seat: &str) -> Option<SeatPolicy> {
            self.0.clone()
        }
    }

    #[test]
    fn lockdown_escalates_a_mutating_tool_with_no_grant_to_save_it() {
        let authority = SeatAuthority::new(Arc::new(StaticSource::lockdown()));
        match authority.consult("emp_1", "file_write", false) {
            Some(SeatDecision::Escalate(_)) => {}
            other => panic!("lockdown must escalate a mutating tool: {other:?}"),
        }
    }

    #[test]
    fn an_escalation_carries_the_ask_identity() {
        let authority = SeatAuthority::new(Arc::new(StaticSource::lockdown()));
        let ask = authority.escalate("emp_1", "file_write", "not in allow list");
        let esc = ask.escalation.expect("escalate always carries one");
        assert_eq!(esc.employee_id, "emp_1");
        assert_eq!(esc.tool, "file_write");
    }

    #[test]
    fn an_ungoverned_seat_consults_to_none() {
        let authority = SeatAuthority::new(Arc::new(StaticSource::empty()));
        assert!(
            authority
                .consult("emp_nobody", "file_write", false)
                .is_none()
        );
    }

    #[test]
    fn an_approver_resolves_without_minting() {
        let approver = SeatApprover::new("premiere");
        let ask = SeatEscalation {
            request_id: "req_1".into(),
            employee_id: "emp_1".into(),
            tool: "file_write".into(),
            why: "test".into(),
        };
        assert!(approver.approve(&ask).is_ok());
        assert!(approver.deny(&ask).is_ok());
        assert!(approver.expire(&ask).is_ok());
    }
}

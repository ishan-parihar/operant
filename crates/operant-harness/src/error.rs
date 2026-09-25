//! Kernel error type.

use crate::claim::Claim;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("provider `{0}` is already mounted")]
    AlreadyMounted(String),

    #[error("provider `{0}` is not mounted")]
    NotFound(String),

    #[error("claim `{claim}` already owned by provider `{owner}`")]
    ClaimConflict { claim: Claim, owner: String },

    #[error("no seam named `{0}` is registered with the kernel")]
    MissingSeam(String),

    /// The seam IS registered but cannot service installs right now (e.g. the
    /// backing runtime is off). Unlike [`HarnessError::MissingSeam`] this is
    /// seam-originated provenance, so the kernel stores the entry as Pending
    /// unconditionally instead of applying the genuine-missing heuristic.
    #[error("seam `{0}` is registered but not currently serviceable")]
    SeamUnavailable(String),

    #[error("activation of provider `{id}` failed: {message}")]
    ActivationFailed { id: String, message: String },

    #[error("provider `{id}` cannot replace `{target}`: id mismatch")]
    ReplaceIdMismatch { id: String, target: String },

    #[error(
        "replacement `{id}` would leave requirement `{requirement}` unsatisfied \
         (previous owner is being replaced)"
    )]
    ReplaceBreaksRequirement { id: String, requirement: Claim },

    /// Composition-layer error (Phase 3): bad row, bad patch, patch target
    /// missing, etc. Always carries a human-readable message.
    #[error("composition: {0}")]
    CompositionError(String),
}

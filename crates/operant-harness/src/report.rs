//! Serializable introspection of a resolved harness tree.

use serde::{Deserialize, Serialize};

use crate::claim::Claim;
use crate::provider::{ProviderSource, ProviderState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntryInfo {
    pub id: String,
    pub source: ProviderSource,
    pub state: ProviderState,
    pub generation: u64,
    /// Sequence number assigned at activation — deterministic dump ordering.
    pub seq: u64,
    pub provides: Vec<Claim>,
    pub requires: Vec<Claim>,
    /// Number of live undo handles held by the kernel for this provider.
    pub effects: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimInfo {
    #[serde(flatten)]
    pub claim: Claim,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DumpTree {
    pub semantics_version: u32,
    pub providers: Vec<ProviderEntryInfo>,
    pub claims: Vec<ClaimInfo>,
}

impl DumpTree {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Result of a mount attempt.
#[derive(Debug, Clone)]
pub enum MountReport {
    /// Activated now; lists this provider plus any Pending entries rescued by
    /// late binding during the same call.
    Mounted { activated: Vec<String> },
    /// Requirements unmet; parked until dependencies appear.
    Pending { missing: Vec<Claim> },
}

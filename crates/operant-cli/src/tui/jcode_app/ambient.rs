// Vendored from jcode (crates/jcode-app-core/src/ambient.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only AmbientStatus (:70), the type AmbientWidgetData carries. See
// jcode_app/mod.rs for scope.
//! [port-excision] the ambient runner/queue machinery is not ported.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};


/// Ambient mode status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum AmbientStatus {
    #[default]
    Idle,
    Running {
        detail: String,
    },
    Scheduled {
        next_wake: DateTime<Utc>,
    },
    Paused {
        reason: String,
    },
    Disabled,
}

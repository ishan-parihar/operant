#![deny(missing_docs)]

//! Configuration schema, secrets, and related types for Operant.

pub mod api_error;
/// Autonomy levels and agent control settings.
pub mod autonomy;
/// Comment rewriting / formatting utilities.
pub mod comment_writer;
/// Cost tracking configuration types.
pub mod cost;
/// Sensitive-domain URL gating for tool use.
pub mod domain_matcher;
/// Property helpers shared by the derive-generated code.
pub mod helpers;
pub mod migration;
/// Device pairing configuration.
pub mod pairing;
/// Platform-specific configuration types.
pub mod platform;
/// Security policy, autonomy, and tool-execution gating types.
pub mod policy;
pub mod provider_aliases;
/// Provider section (`[providers]`) types.
pub mod providers;
pub mod scattered_types;
/// The full config schema (`Config` and all section types).
pub mod schema;
/// Secret encryption / storage helpers.
pub mod secrets;
/// Derive-support traits (`HasPropKind`, `ChannelConfig`, `OnboardUi`, ...).
pub mod traits;
/// Typed-value coercion used by the property CRUD surface.
pub mod typed_value;
pub mod validation_warnings;
pub mod workspace;

/// Shim module so `Configurable` derive macro's generated `crate::config::*` paths resolve.
/// The macro was written assuming it runs inside the root crate where `mod config` exists.
pub mod config {
    pub use crate::helpers::*;
    pub use crate::traits::*;
}

/// Shim module so `Configurable` derive macro's generated `crate::security::*` paths resolve.
pub mod security {
    pub use crate::policy::SecurityPolicy;
    pub use crate::secrets::SecretStore;
}

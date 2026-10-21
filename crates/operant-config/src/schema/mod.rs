//! Configuration schema root. The former 19.8K-line schema.rs was
//! split into domain modules; this file re-exports every public item so
//! `schema::Name` paths are byte-for-byte compatible with the monolith.
//!
//! # THIS IS NOT THE LIVE CONFIG
//!
//! The config the binary reads is `operant_core::config::AppConfig`
//! (`crates/operant-core/src/config.rs`) — the CLI parses the operator's
//! `config.toml` into it and every runtime path reads it.
//!
//! This schema is a SPEC: `Config::load_or_init` has no production caller
//! (only this crate's own tests call it), and the one production-typed
//! consumer — `operant-providers::provider_runtime_options_from_config` —
//! is itself called only from tests. 123 of the 145 structs here have no
//! production reader outside this crate (645 fields);
//! `python3 scripts/audit-parallel-config.py` re-derives the counts.
//!
//! So a setting added here is NOT live, and a setting the operator needs
//! goes into the live `AppConfig`. The retirement (or the adoption of this
//! surface as the config spine) is planned in
//! `docs/plans/plan-2026-10-21-config-schema-retirement.md`; the evidence is
//! in `docs/audit/2026-10-21-PARALLEL-CONFIG-SURFACE.md`; the open item is
//! RT-141 in `docs/audit/2026-06-19-BUGS.md`. The guard test
//! `tests/schema_is_not_loaded.rs` fails if this surface starts being loaded
//! by accident.

mod agent_cfg;
mod backup;
mod channels_cfg;
mod config_impl;
mod core;
mod cost;
mod delegate;
mod gateway;
mod hardware;
mod helpers;
mod mcp;
mod media;
mod memory_store;
mod ops_cfg;
mod platform_cfg;
mod providers_cfg;
mod proxy;
mod runners;
mod scheduler;
mod security_cfg;
mod skills;
mod sop;
mod stt;
mod tts;
mod tunnels;
mod web_tools;
mod workspace_cfg;

pub use agent_cfg::*;
pub use backup::*;
pub use channels_cfg::*;
pub use config_impl::*;
pub use core::*;
pub use cost::*;
pub use delegate::*;
pub use gateway::*;
pub use hardware::*;
pub use helpers::*;
pub use mcp::*;
pub use media::*;
pub use memory_store::*;
pub use ops_cfg::*;
pub use platform_cfg::*;
pub(crate) use providers_cfg::*;
pub use proxy::*;
pub use runners::*;
pub use scheduler::*;
pub use security_cfg::*;
pub use skills::*;
pub use sop::*;
pub use stt::*;
pub use tts::*;
pub use tunnels::*;
pub use web_tools::*;
pub use workspace_cfg::*;

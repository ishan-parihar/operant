//! Configuration schema root. The former 19.8K-line schema.rs was
//! split into domain modules; this file re-exports every public item so
//! `schema::Name` paths are byte-for-byte compatible with the monolith.
//!
//! # THE LIVE CONFIG IS `AppConfig`
//!
//! The config the binary reads is `operant_core::config::AppConfig`
//! (`crates/operant-core/src/config.rs`) — the CLI parses the operator's
//! `operant.toml` into it and every runtime path reads it.
//!
//! This module used to hold a SECOND, much larger config surface: the
//! aggregate `Config` (~30 sections) with its own loader
//! (`Config::load_or_init`), its own `config.toml` resolution and its own
//! migration. Nothing loaded it — only this crate's own tests did — so it
//! was retired (RT-141, iter-1138): the aggregate, its loader, its
//! migration and its 7K-line test file are GONE.
//!
//! What remains here are the TYPES the live config composes — the section
//! structs `AppConfig` embeds (`ModelProviderConfig`, `ProxyConfig`,
//! `ObservabilityConfig`, …) and the helpers that read them. Those are
//! live. The section structs that no live config carries are the next
//! step of the retirement; `python3 scripts/audit-parallel-config.py`
//! re-derives the counts.
//!
//! So a setting the operator needs goes into the live `AppConfig`. The
//! retirement plan is `docs/plans/plan-2026-10-21-config-schema-retirement.md`;
//! the evidence is in `docs/audit/2026-10-21-PARALLEL-CONFIG-SURFACE.md`;
//! the open item is RT-141 in `docs/audit/2026-06-19-BUGS.md`. The guard
//! test `tests/schema_is_not_loaded.rs` fails if the aggregate comes back.

mod agent_cfg;
mod backup;
mod channels_cfg;
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
mod prop_kinds;
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

pub use agent_cfg::*;
pub use backup::*;
pub use channels_cfg::*;
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

//! Configuration schema root. The former 19.8K-line schema.rs was
//! split into domain modules; this file re-exports every public item so
//! `schema::Name` paths are byte-for-byte compatible with the monolith.

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

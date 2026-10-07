//! Orchestrator — the channels crate's session-protocol surface.
//!
//! History: this module was the full multi-channel dispatch orchestrator
//! (`start_channels` → `run_message_dispatch_loop` → `dispatch_worker` →
//! `process_channel_message`). The shipped CLI gateway never called any of
//! it — `gateway_runner.rs` + `operant-gateway` own the live message paths
//! (since iter-628), and LTO stripped the whole cluster from every deployed
//! binary. iter-664 (see `docs/plan-2026-10-07-agentic-core-verified-remaining-work.md`
//! §2, Option A) deleted the dead cluster; the audit trail for why every
//! removed module was unreachable lives in that doc.
//!
//! What remains:
//! - [`acp_server`] — the Agent Client Protocol server, served live by
//!   `operant-gateway::acp`.
//! - [`mqtt`] — the MQTT SOP listener, feature-gated (`channel-mqtt`), wired
//!   via `DaemonSubsystems::mqtt_start`.
//!
//! The one live helper the dead cluster exported, `strip_tool_call_tags`,
//! moved verbatim to `crate::telegram::helpers`.

#[cfg(feature = "channel-acp-server")]
pub mod acp_server;
#[cfg(feature = "channel-mqtt")]
pub mod mqtt;

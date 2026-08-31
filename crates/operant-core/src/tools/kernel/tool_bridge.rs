//! Phase 2.5 tool bridge: drains kernel-initiated tool calls from the
//! sidecar's reader channel and executes them host-side under full approval
//! parity (see runtime::BridgeExecutor for the security invariants).
//!
//! One persistent drainer task (spawned once by [`super::register`]) survives
//! sidecar crashes: when a handle is replaced, the old channel closes
//! (`recv()` → None) and the drainer locks the NEXT handle's channel. The
//! per-handle mutex guarantees exactly one drainer owns any live channel.
//!
//! Per-cell accounting: the cell budget resets at each `exec` request;
//! concurrent cells share one budget (documented v1 limitation).

use std::sync::Arc;

use serde_json::json;

use super::runtime::KernelRuntime;
use super::sidecar::{Inbound, SidecarWriter};
use crate::tools::ToolContext;

pub(super) fn spawn_drainer(rt: Arc<KernelRuntime>) {
    tokio::spawn(async move {
        loop {
            let Some(handle) = rt.handle_snapshot().await else {
                #[cfg(test)]
                eprintln!("[pk-bridge] waiting for sidecar handle");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            };
            let mut rx = handle.bridge_rx.lock().await;
            while let Some(call) = rx.recv().await {
                let Inbound::BridgeCall {
                    bridge_id,
                    session_key,
                    name,
                    args,
                } = call;
                let Some(executor) = rt.executor() else {
                    reply(
                        &handle.writer,
                        &bridge_id,
                        Err("tool bridge disabled ([tools.kernel.tool_bridge])".into()),
                    )
                    .await;
                    continue;
                };
                if rt.cell_calls() >= executor.max_calls_per_exec as u64 {
                    reply(
                        &handle.writer,
                        &bridge_id,
                        Err(format!(
                            "max_calls_per_exec ({}) exceeded; partial results returned",
                            executor.max_calls_per_exec
                        )),
                    )
                    .await;
                    continue;
                }
                rt.record_bridge_call();
                #[cfg(test)]
                eprintln!(
                    "[pk-bridge] dispatching {} for session {}",
                    name, session_key
                );
                let ctx = ToolContext::default()
                    .with_metadata("session_id", session_key.clone())
                    .with_metadata("origin", "kernel_bridge");
                tracing::debug!(target: "kernel", tool = %name, session = %session_key, "bridged call");
                let outcome = executor.execute_bridged(&name, args, ctx).await;
                #[cfg(test)]
                eprintln!(
                    "[pk-bridge] outcome for {}: {:?}",
                    bridge_id,
                    outcome.as_ref().map(|_| "ok")
                );
                reply(&handle.writer, &bridge_id, outcome).await;
                #[cfg(test)]
                eprintln!("[pk-bridge] reply written for {}", bridge_id);
            }
            // Channel closed ⇒ handle replaced/dropped; loop re-snapshots.
        }
    });
}

async fn reply(
    writer: &SidecarWriter,
    bridge_id: &str,
    outcome: Result<serde_json::Value, String>,
) {
    let frame = match outcome {
        Ok(result) => json!({"reply_for": bridge_id, "ok": true, "result": result}),
        Err(error) => json!({"reply_for": bridge_id, "ok": false, "error": error}),
    };
    if let Err(e) = writer.send_line(&frame.to_string()).await {
        tracing::warn!(target: "kernel", "bridge reply write failed: {e}");
    }
}

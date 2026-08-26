//! Prime Kernel (plan 015): persistent stateful Python kernel + continual
//! harness, hosted in the `pk-sidecar` subprocess over NDJSON JSON-RPC stdio.
//!
//! Ownership mirrors upstream prime-agent's split: policy lives in this Rust
//! host (approval gating, allowlists, caps, timeouts), state lives in the
//! sidecar (kernel namespaces + vendored live `rlm` harness store).
//!
//! Replaces-not-duplicates: the kernel is the default interactive Python path
//! once `route_python_to_kernel` flips; the harness store carries ONLY the two
//! kinds operant lacks (`prompt`, `subagent`) — skills/memories stay owned by
//! curator/skills/MEMORY.md lanes.

pub mod harness_tools;
pub mod kernel_tool;
pub mod runtime;
pub mod sidecar;
pub mod tool_bridge;
use std::sync::{Arc, OnceLock};

use serde_json::Value;

pub use runtime::PkRuntime;

/// Process-wide runtime handle installed by [`register`] so non-tool callers
/// (e.g. `code_execution`'s python routing cutover) can reach the sidecar
/// without plumbing an Arc through every constructor.
static GLOBAL_RUNTIME: OnceLock<Arc<PkRuntime>> = OnceLock::new();

pub fn global_runtime() -> Option<&'static Arc<PkRuntime>> {
    GLOBAL_RUNTIME.get()
}

/// Feed-forward injection lane (phase 4): bounded `[continual harness]` text
/// for the per-turn volatile prompt suffix. Local(session)-scope entries come
/// first, then global lessons; empty state renders NOTHING so prompts stay
/// byte-stable when the kernel is off or the store is empty.
pub async fn injection_block(session_id: Option<&str>, max_chars: usize) -> Option<String> {
    let rt = global_runtime()?;
    if !rt.settings().enabled {
        return None;
    }
    const EMPTY_MARKERS: [&str; 2] = ["(empty harness)", "(empty"];
    let mut block = String::from("[continual harness]\n");
    let mut budget = max_chars.max(200);
    for scope in ["local", "global"] {
        let params = serde_json::json!({
            "scope": scope,
            "session_key": session_id.unwrap_or("default"),
        });
        let Ok(v) = rt.request("harness_overview", params).await else {
            continue;
        };
        let Some(text) = v.get("overview").and_then(Value::as_str) else {
            continue;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() || EMPTY_MARKERS.iter().any(|m| trimmed.starts_with(m)) {
            continue;
        }
        let section = format!("({scope}) {trimmed}\n");
        let taken: String = section.chars().take(budget).collect();
        let cut = taken.trim_end();
        if cut.is_empty() || cut == "(local)" || cut == "(global)" {
            break;
        }
        budget = budget.saturating_sub(cut.chars().count() + 1);
        block.push_str(cut);
        block.push('\n');
        if budget <= 40 {
            break;
        }
    }
    if block == "[continual harness]\n" {
        return None;
    }
    Some(block)
}

/// Register the three model-facing tools under the gated `prime_kernel`
/// toolset. No-op unless `[tools.prime_kernel] enabled = true`.
pub async fn register(
    registry: &crate::tools::ToolRegistry,
    settings: &crate::config::PrimeKernelSettings,
) -> anyhow::Result<Option<Arc<PkRuntime>>> {
    if !settings.enabled {
        return Ok(None);
    }
    let rt = PkRuntime::new(settings.clone());
    let _ = GLOBAL_RUNTIME.set(rt.clone());
    if settings.tool_bridge.enabled {
        // Executor holds a ToolRegistry clone sharing the live tool map, so
        // tools registered after this point stay visible to kernel programs.
        rt.install_executor(registry.clone());
        tool_bridge::spawn_drainer(rt.clone());
    }
    registry
        .register(kernel_tool::PkKernelExecTool::new(rt.clone()))
        .await?;
    registry
        .register(harness_tools::PkHarnessGetTool::new(rt.clone()))
        .await?;
    registry
        .register(harness_tools::PkRefineTool::new(rt))
        .await?;
    Ok(GLOBAL_RUNTIME.get().cloned())
}

#[cfg(test)]
mod tests {
    //! Live sidecar integration tests: spawn the real Python subprocess and
    //! exercise ping / exec persistence / transparent restart / harness CRUD.
    //! Skipped naturally when python>=3.11 is absent (spawn error surfaces).

    use super::*;
    use crate::config::PrimeKernelSettings;
    use serde_json::json;

    fn test_settings(state_root: &std::path::Path) -> PrimeKernelSettings {
        let s = PrimeKernelSettings {
            enabled: true,
            state_dir: Some(state_root.to_path_buf()),
            // Keep the idle reaper out of test timing.
            sidecar_idle_secs: 0,
            request_timeout_secs: 30,
            ..PrimeKernelSettings::default()
        };
        // PK_SIDECAR_DIR resolution: in-tree builds fall back to the
        // compile-time CARGO_MANIFEST_DIR path (../../pk-sidecar), which is
        // exactly this repo layout — no env mutation needed (edition-2024
        // set_var is unsafe and racy under parallel tests).
        s
    }

    #[tokio::test]
    async fn ping_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = PkRuntime::new(test_settings(dir.path()));
        let v = rt.request("ping", json!({})).await.expect("ping");
        assert_eq!(v["pong"], json!(true));
        assert_eq!(v["has_prime_runtime"], json!(true));
    }

    #[tokio::test]
    async fn exec_state_persists_across_requests() {
        let dir = tempfile::tempdir().unwrap();
        let rt = PkRuntime::new(test_settings(dir.path()));
        rt.request(
            "exec",
            json!({"session_key": "t1", "code": "x = 41\nprint('set')"}),
        )
        .await
        .expect("exec 1");
        let v = rt
            .request("exec", json!({"session_key": "t1", "code": "print(x + 1)"}))
            .await
            .expect("exec 2");
        assert_eq!(v["stdout"], json!("42\n"));
        // (The "persistent": true marker is added by PkKernelExecTool; the
        // raw exec protocol returns kernel fields only.)
    }

    #[tokio::test]
    async fn transparent_restart_after_sidecar_exit() {
        let dir = tempfile::tempdir().unwrap();
        let rt = PkRuntime::new(test_settings(dir.path()));
        // Ask the sidecar to stop its loop; child exits.
        rt.request("shutdown", json!({})).await.expect("shutdown");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // Next request must transparently respawn and still work.
        let v = rt
            .request(
                "exec",
                json!({"session_key": "r1", "code": "print('back')"}),
            )
            .await
            .expect("post-restart exec");
        assert_eq!(v["stdout"], json!("back\n"));
    }

    #[tokio::test]
    async fn probe_direct_bridged_exec() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = test_settings(dir.path());
        settings.tool_bridge.enabled = true;
        let rt = PkRuntime::new(settings);
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        registry
            .register(crate::tools::datetime_tool::DateTimeTool)
            .await
            .unwrap();
        rt.install_executor(registry);
        let res = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            rt.executor().unwrap().execute_bridged(
                "datetime",
                serde_json::json!({"action": "now"}),
                crate::tools::ToolContext::default(),
            ),
        )
        .await;
        match res {
            Ok(Ok(v)) => eprintln!("PROBE OK: {v}"),
            Ok(Err(e)) => eprintln!("PROBE ERR: {e}"),
            Err(_) => eprintln!("PROBE HANG: execute_bridged exceeded 8s"),
        }
    }

    /// Phase 2.5: kernel programs reach the host registry ONLY through the
    /// allowlist + approval-parity gate.
    #[tokio::test]
    async fn tool_bridge_allowlisted_call_roundtrip_and_denial() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = test_settings(dir.path());
        settings.tool_bridge.enabled = true;
        let rt = PkRuntime::new(settings);

        // Executor over a registry holding one allowlisted tool (datetime).
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(10));
        registry
            .register(crate::tools::datetime_tool::DateTimeTool)
            .await
            .unwrap();
        rt.install_executor(registry);
        tool_bridge::spawn_drainer(rt.clone());

        rt.reset_cell_budget();
        let v = rt
            .request(
                "exec",
                json!({"session_key": "b1", "code": "r = await operant_tool('datetime', {'action': 'now'})\nprint('bridge_ok', isinstance(r, dict))"}),
            )
            .await
            .expect("bridged exec");
        assert!(
            v["stdout"].as_str().unwrap().contains("bridge_ok True"),
            "{v}"
        );
        assert_eq!(rt.cell_calls(), 1);

        // Non-allowlisted tool returns an error VALUE (cell still succeeds).
        rt.reset_cell_budget();
        let v = rt
            .request(
                "exec",
                json!({"session_key": "b1", "code": "r = await operant_tool('terminal', {'command': 'id'})\nprint('denied', '_bridge_error' in r)"}),
            )
            .await
            .expect("denied exec");
        assert!(v["stdout"].as_str().unwrap().contains("denied True"), "{v}");
    }

    #[tokio::test]
    async fn harness_apply_and_rollback_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = PkRuntime::new(test_settings(dir.path()));
        let applied = rt
            .request(
                "refine_apply",
                json!({
                    "scope": "local",
                    "trigger": "test",
                    "evidence": "integration",
                    "edits": [{"action": "create", "kind": "prompt",
                               "title": "Terse", "content": "Be terse."}],
                }),
            )
            .await
            .expect("apply");
        let event_id = applied["refinement_id"].as_str().unwrap().to_string();
        let rolled = rt
            .request(
                "refine_rollback",
                json!({"event_id": event_id, "scope": "local"}),
            )
            .await
            .expect("rollback");
        assert!(rolled["rolled_back"].is_string());
        // Entry gone after rollback:
        let ov = rt
            .request("harness_overview", json!({"scope": "local"}))
            .await
            .unwrap();
        assert!(!ov["overview"].as_str().unwrap().contains("Terse"));
    }
}

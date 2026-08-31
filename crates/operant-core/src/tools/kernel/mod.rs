//! Persistent Kernel (plan 015): persistent stateful Python kernel + continual
//! harness, hosted in the `kernel-sidecar` subprocess over NDJSON JSON-RPC stdio.
//!
//! Ownership mirrors upstream persistent kernel split: policy lives in this Rust
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

pub use runtime::KernelRuntime;

/// Process-wide runtime handle installed by [`register`] so non-tool callers
/// (e.g. `code_execution`'s python routing cutover) can reach the sidecar
/// without plumbing an Arc through every constructor.
static GLOBAL_RUNTIME: OnceLock<Arc<KernelRuntime>> = OnceLock::new();

pub fn global_runtime() -> Option<&'static Arc<KernelRuntime>> {
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
    let sid = session_id.unwrap_or("default");
    // Per-turn cache: avoid double sidecar roundtrip within 5s for same session+budget.
    if let Some(cached) = rt.cached_injection(sid, max_chars).await {
        return Some(cached);
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
    // Phase 6b: executable skills section (SKILL.toml [reference]).
    if let Some(exe) = executable_skills_section(max_chars.saturating_sub(block.len()))
        && block.len() + exe.len() < max_chars
    {
        block.push_str(&exe);
    }
    if block == "[continual harness]\n" {
        return None;
    }
    rt.store_injection(sid, max_chars, block.clone()).await;
    Some(block)
}

/// Executable skills (plan 016, phase 6b): scan <skills_dir>/*/SKILL.toml
/// for `[reference]` and emit a bounded "executable skills:" section.
/// Best-effort, sync filesystem scan — budgeted so prompts stay bounded.
fn executable_skills_section(budget: usize) -> Option<String> {
    if budget < 80 {
        return None;
    }
    let cfg = crate::config::runtime_config();
    if !cfg.tools.kernel.pyskill.enabled {
        return None;
    }
    let allowlist = &cfg.tools.kernel.pyskill.allowed_imports;
    // Skills dir: runtime config root_dir if set, else platform default.
    let skills_dir = if cfg.skills.root_dir.as_os_str().is_empty() {
        crate::platform::operant_skills_dir()
    } else {
        cfg.skills.root_dir.clone()
    };
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return None;
    };
    let mut lines: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let toml_path = path.join("SKILL.toml");
        if !toml_path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&toml_path) else {
            continue;
        };
        let Ok(val) = content.parse::<toml::Value>() else {
            continue;
        };
        let Some(tbl) = val.get("reference").and_then(|v| v.as_table()) else {
            continue;
        };
        let Some(import) = tbl.get("import").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(callable) = tbl.get("callable").and_then(|v| v.as_str()) else {
            continue;
        };
        if !is_import_allowed(import, allowlist) {
            continue;
        }
        let call_pattern = tbl
            .get("call_pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let skill_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let line = if call_pattern.is_empty() {
            format!("  - {skill_name}: {import}::{callable}")
        } else {
            format!("  - {skill_name}: {call_pattern}  # from {import}::{callable}")
        };
        lines.push(line);
        if lines.len() >= 10 {
            break;
        }
    }
    if lines.is_empty() {
        return None;
    }
    let mut section = String::from("(executable skills)\n");
    for line in &lines {
        if section.len() + line.len() + 2 > budget {
            break;
        }
        section.push_str(line);
        section.push('\n');
    }
    Some(section)
}

fn is_import_allowed(import: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return false;
    }
    for pat in allowlist {
        if pat == "*" {
            return true;
        }
        if pat.ends_with(".*") {
            let prefix = &pat[..pat.len() - 2];
            if import == prefix || import.starts_with(&format!("{prefix}.")) {
                return true;
            }
        } else if pat.contains('*') || pat.contains('?') {
            // Simple glob: convert to fnmatch-style check via glob crate fallback
            // For now, handle `*` as substring wildcard.
            let star_parts: Vec<&str> = pat.split('*').collect();
            let mut ok = true;
            let mut pos = 0usize;
            for (i, part) in star_parts.iter().enumerate() {
                if part.is_empty() {
                    continue;
                }
                if i == 0 {
                    if !import.starts_with(*part) {
                        ok = false;
                        break;
                    }
                    pos = part.len();
                } else if i == star_parts.len() - 1 {
                    if !import[pos..].ends_with(*part) {
                        ok = false;
                        break;
                    }
                } else if let Some(idx) = import[pos..].find(*part) {
                    pos += idx + part.len();
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                return true;
            }
        } else if import == pat {
            return true;
        }
    }
    false
}

/// Register the three model-facing tools under the gated `kernel`
/// toolset. No-op unless `[tools.kernel] enabled = true`.
pub async fn register(
    registry: &crate::tools::ToolRegistry,
    settings: &crate::config::KernelSettings,
) -> anyhow::Result<Option<Arc<KernelRuntime>>> {
    if !settings.enabled {
        return Ok(None);
    }
    let rt = KernelRuntime::new(settings.clone());
    let _ = GLOBAL_RUNTIME.set(rt.clone());
    if settings.tool_bridge.enabled {
        // Executor holds a ToolRegistry clone sharing the live tool map, so
        // tools registered after this point stay visible to kernel programs.
        rt.install_executor(registry.clone());
        tool_bridge::spawn_drainer(rt.clone());
    }
    registry
        .register(kernel_tool::KernelExecTool::new(rt.clone()))
        .await?;
    registry
        .register(harness_tools::KernelStateTool::new(rt.clone()))
        .await?;
    registry
        .register(harness_tools::KernelRefineTool::new(rt))
        .await?;
    Ok(GLOBAL_RUNTIME.get().cloned())
}

#[cfg(test)]
mod tests {
    //! Live sidecar integration tests: spawn the real Python subprocess and
    //! exercise ping / exec persistence / transparent restart / harness CRUD.
    //! Skipped naturally when python>=3.11 is absent (spawn error surfaces).

    use super::*;
    use crate::config::KernelSettings;
    use serde_json::json;

    fn test_settings(state_root: &std::path::Path) -> KernelSettings {
        let s = KernelSettings {
            enabled: true,
            state_dir: Some(state_root.to_path_buf()),
            // Keep the idle reaper out of test timing.
            sidecar_idle_secs: 0,
            request_timeout_secs: 30,
            ..KernelSettings::default()
        };
        // KERNEL_SIDECAR_DIR resolution: in-tree builds fall back to the
        // compile-time CARGO_MANIFEST_DIR path (../../kernel-sidecar), which is
        // exactly this repo layout — no env mutation needed (edition-2024
        // set_var is unsafe and racy under parallel tests).
        s
    }

    #[tokio::test]
    async fn ping_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let rt = KernelRuntime::new(test_settings(dir.path()));
        let v = rt.request("ping", json!({})).await.expect("ping");
        assert_eq!(v["pong"], json!(true));
        assert_eq!(v["has_runtime"], json!(true));
    }

    #[tokio::test]
    async fn exec_state_persists_across_requests() {
        let dir = tempfile::tempdir().unwrap();
        let rt = KernelRuntime::new(test_settings(dir.path()));
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
        // (The "persistent": true marker is added by KernelExecTool; the
        // raw exec protocol returns kernel fields only.)
    }

    #[tokio::test]
    async fn transparent_restart_after_sidecar_exit() {
        let dir = tempfile::tempdir().unwrap();
        let rt = KernelRuntime::new(test_settings(dir.path()));
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
        let rt = KernelRuntime::new(settings);
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
        let rt = KernelRuntime::new(settings);

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
        let rt = KernelRuntime::new(test_settings(dir.path()));
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

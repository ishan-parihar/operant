//! Shared runtime for all kernel tools: settings snapshot, sidecar lifecycle,
//! bridged-executor installation, and metrics.

use std::collections::HashSet;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU32, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::config::{KernelSettings, KernelToolBridge};
use crate::tools::{ToolContext, ToolRegistry};

use super::sidecar::{SidecarHandle, request as sc_request, spawn_idle_reaper};

/// Host-side executor for kernel-initiated tool calls (phase 2.5).
///
/// Security invariant: every bridged call re-enters `check_tool_approval`
/// with the TARGET tool's name/args before execution — a kernel program can
/// never launder permissions a direct call would not have. `requires_approval`
/// verdicts are DENIED inside cells (no human-in-loop channel mid-cell);
/// denied calls return an error VALUE to the kernel program.
pub struct BridgeExecutor {
    registry: ToolRegistry,
    approval_mode: Option<String>,
    pub allowlist: HashSet<String>,
    pub max_calls_per_exec: usize,
    pub per_call_timeout_secs: u64,
}

impl BridgeExecutor {
    pub async fn execute_bridged(
        &self,
        name: &str,
        args: Value,
        ctx: ToolContext,
    ) -> Result<Value, String> {
        if !self.allowlist.contains(name) {
            return Err(format!(
                "tool '{name}' is not allowlisted for kernel programs \
                 ([tools.kernel.tool_bridge.allowlist])"
            ));
        }
        let verdict =
            crate::approval::check_tool_approval(name, &args, self.approval_mode.as_deref());
        match verdict.verdict.as_str() {
            "allowed" => {}
            other => {
                return Err(format!(
                    "tool '{name}' approval verdict '{other}'{} — denied inside kernel programs",
                    verdict
                        .reason
                        .as_ref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ));
            }
        }
        let fut = self.registry.execute(name, "pk-bridge", args, ctx);
        let timeout = Duration::from_secs(self.per_call_timeout_secs.max(1));
        match tokio::time::timeout(timeout, fut).await {
            Ok(Ok(res)) => Ok(serde_json::from_str::<Value>(&res.content)
                .unwrap_or(Value::String(res.content.clone()))),
            Ok(Err(e)) => Err(format!("tool '{name}' failed: {e}")),
            Err(_) => Err(format!(
                "tool '{name}' timed out after {}s",
                timeout.as_secs()
            )),
        }
    }
}

pub struct KernelRuntime {
    settings: KernelSettings,
    handle: AsyncMutex<Option<Arc<SidecarHandle>>>,
    executor: OnceLock<BridgeExecutor>,
    failures: AtomicU32,
    bridge_calls_total: AtomicU64,
    cell_calls: AtomicU64,
    restarts: AtomicU64,
    /// Per-turn injection cache: (session_id, max_chars, block, timestamp).
    /// Eliminates double sidecar roundtrip when injection_block is called
    /// multiple times within one turn (e.g. concurrent tool calls).
    injection_cache: AsyncMutex<Option<(String, usize, String, Instant)>>,
}

impl KernelRuntime {
    pub(super) fn handle_cell(&self) -> &AsyncMutex<Option<Arc<SidecarHandle>>> {
        &self.handle
    }
    pub fn new(settings: KernelSettings) -> Arc<Self> {
        let rt = Arc::new(Self {
            settings,
            handle: AsyncMutex::new(None),
            executor: OnceLock::new(),
            failures: AtomicU32::new(0),
            bridge_calls_total: AtomicU64::new(0),
            cell_calls: AtomicU64::new(0),
            restarts: AtomicU64::new(0),
            injection_cache: AsyncMutex::new(None),
        });
        spawn_idle_reaper(rt.clone());
        // Fire-and-forget session GC at startup (best-effort, never blocks boot).
        let gc_rt = rt.clone();
        tokio::spawn(async move {
            gc_rt.gc_sessions().await;
        });
        rt
    }

    pub fn settings(&self) -> &KernelSettings {
        &self.settings
    }

    /// Install the bridged-call executor. Called once at registration;
    /// later registry mutations stay visible because ToolRegistry clones
    /// share one internal tool map.
    pub fn install_executor(&self, registry: ToolRegistry) {
        let tb: &KernelToolBridge = &self.settings.tool_bridge;
        let _ = self.executor.set(BridgeExecutor {
            registry,
            approval_mode: None, // Smart default; explicit modes arrive via config later.
            allowlist: tb.allowlist.iter().cloned().collect(),
            max_calls_per_exec: tb.max_calls_per_exec,
            per_call_timeout_secs: tb.per_call_timeout_secs,
        });
    }

    pub fn executor(&self) -> Option<&BridgeExecutor> {
        self.executor.get()
    }

    pub fn record_bridge_call(&self) {
        self.bridge_calls_total.fetch_add(1, Ordering::Relaxed);
        self.cell_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Reset the per-cell bridged-call budget (called before each exec).
    pub fn reset_cell_budget(&self) {
        self.cell_calls.store(0, Ordering::Relaxed);
    }

    pub fn cell_calls(&self) -> u64 {
        self.cell_calls.load(Ordering::Relaxed)
    }

    pub(super) fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) async fn drop_handle(&self) {
        let mut guard = self.handle.lock().await;
        if guard.is_some() {
            self.restarts.fetch_add(1, Ordering::Relaxed);
        }
        *guard = None; // Arc drop → Child drop → kill_on_drop tears down group
    }

    pub(super) async fn handle_snapshot(&self) -> Option<Arc<SidecarHandle>> {
        self.handle.lock().await.clone()
    }

    pub fn failures(&self) -> u32 {
        self.failures.load(Ordering::Relaxed)
    }

    pub fn bridge_calls(&self) -> u64 {
        self.bridge_calls_total.load(Ordering::Relaxed)
    }

    pub fn restarts(&self) -> u64 {
        self.restarts.load(Ordering::Relaxed)
    }

    /// One request/response round-trip to the live sidecar.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        // Invalidate injection cache on any harness write so next turn sees fresh state.
        let is_write = matches!(method, "refine_apply" | "refine_record" | "refine_rollback" | "harness_set" | "harness_delete");
        let res = sc_request(self, method, params).await;
        if is_write && res.is_ok() {
            self.invalidate_injection_cache().await;
        }
        res
    }

    /// Per-turn injection cache helpers (TTL 5s, keyed by session_id + max_chars).
    pub(crate) async fn cached_injection(&self, session_id: &str, max_chars: usize) -> Option<String> {
        let guard = self.injection_cache.lock().await;
        if let Some((cached_sid, cached_max, block, ts)) = guard.as_ref()
            && cached_sid == session_id
            && *cached_max == max_chars
            && ts.elapsed() < Duration::from_secs(5)
        {
            return Some(block.clone());
        }
        None
    }

    pub(crate) async fn store_injection(&self, session_id: &str, max_chars: usize, block: String) {
        let mut guard = self.injection_cache.lock().await;
        *guard = Some((session_id.to_string(), max_chars, block, Instant::now()));
    }

    pub(crate) async fn invalidate_injection_cache(&self) {
        let mut guard = self.injection_cache.lock().await;
        *guard = None;
    }

    /// Session GC: prune per-session harness dirs older than ttl_hours.
    /// Scans <state_dir>/sessions/<id>/ and removes stale session subdirs.
    /// Best-effort: logs warnings, never fails the caller.
    pub async fn gc_sessions(&self) {
        let ttl_hours = self.settings.session_gc_ttl_hours;
        if ttl_hours == 0 {
            return;
        }
        let Some(state_dir) = self.settings.state_dir.as_ref() else {
            return;
        };
        let sessions_dir = state_dir.join("sessions");
        let ttl = Duration::from_secs(ttl_hours * 3600);
        let now = std::time::SystemTime::now();
        let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let Ok(age) = now.duration_since(modified) else {
                continue;
            };
            if age > ttl {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!(target: "kernel", "gc_sessions: failed to remove {}: {e}", path.display());
                } else {
                    tracing::info!(target: "kernel", "gc_sessions: pruned stale session {}", path.display());
                }
            }
        }
    }
}

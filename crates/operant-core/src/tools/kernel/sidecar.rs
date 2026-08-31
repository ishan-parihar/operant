//! NDJSON framed client + process supervision for the kernel-sidecar.
//!
//! Protocol (see kernel-sidecar/kernel_sidecar/server.py):
//!   Rust → py : {"id","method","params"}            one object per line
//!   py → Rust : {"id","ok":true,"result"} | {"id","ok":false,"error"}
//!   py → Rust : {"bridge_id","method":"tool_call",params}   (bridged call out)
//!   Rust → py : {"reply_for","ok","result"|"error"}          (our answer)
//!
//! Supervision: spawn-on-demand, transparent restart on crash (≤1 retry per
//! request, bounded consecutive failures fail closed), idle auto-exit reaper,
//! whole-process-group teardown on drop (parity with code_execution.rs).

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use super::runtime::KernelRuntime;

const MAX_CONSECUTIVE_FAILURES: u32 = 2;

/// In-flight request map: request id → oneshot responder.
type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum Inbound {
    BridgeCall {
        bridge_id: String,
        session_key: String,
        name: String,
        args: Value,
    },
}

#[derive(Clone)]
pub struct SidecarWriter {
    stdin: Arc<AsyncMutex<ChildStdin>>,
}

impl SidecarWriter {
    pub async fn send_line(&self, line: &str) -> Result<(), String> {
        let mut w = self.stdin.lock().await;
        let bytes = line.as_bytes();
        let mut buf = Vec::with_capacity(bytes.len() + 1);
        buf.extend_from_slice(bytes);
        buf.push(b'\n');
        if buf.len() > MAX_LINE_BYTES {
            return Err("sidecar frame exceeds MAX_LINE_BYTES".into());
        }
        w.write_all(&buf).await.map_err(|e| e.to_string())?;
        w.flush()
            .await
            .map_err(|e| format!("sidecar flush failed: {e}"))
    }
}

pub struct SidecarHandle {
    pub writer: SidecarWriter,
    child: Arc<AsyncMutex<Child>>,
    pending: PendingMap,
    /// Bridged tool-call requests awaiting host execution (phase 2.5).
    #[allow(dead_code)] // consumed by tool_bridge drain loop (phase 2.5 wiring)
    pub bridge_rx: AsyncMutex<mpsc::UnboundedReceiver<Inbound>>,
    last_activity: Arc<AtomicU64>,
}

impl SidecarHandle {
    fn touch(&self) {
        self.last_activity.store(now_secs(), Ordering::Relaxed);
    }

    fn idle_secs(&self) -> u64 {
        now_secs().saturating_sub(self.last_activity.load(Ordering::Relaxed))
    }

    async fn is_alive(&self) -> bool {
        matches!(self.child.lock().await.try_wait(), Ok(None))
    }

    async fn kill(&self) {
        // kill_on_drop covers normal drops; explicit kill makes the idle-reaper
        // path deterministic even if some Arc clone lingers.
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
    }

    pub(super) fn pending_map(&self) -> &PendingMap {
        &self.pending
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[allow(clippy::too_many_arguments)]
fn spawn_reader(
    stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    bridge_tx: mpsc::UnboundedSender<Inbound>,
    last_activity: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let frame: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            last_activity.store(now_secs(), Ordering::Relaxed);

            if let Some(bridge_id) = frame.get("bridge_id").and_then(Value::as_str) {
                let params = frame.get("params").cloned().unwrap_or(Value::Null);
                let _ = bridge_tx.send(Inbound::BridgeCall {
                    bridge_id: bridge_id.to_string(),
                    session_key: params
                        .get("session_key")
                        .and_then(Value::as_str)
                        .unwrap_or("default")
                        .to_string(),
                    name: params
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    args: params.get("args").cloned().unwrap_or(json!({})),
                });
                continue;
            }
            if let Some(id) = frame.get("id").and_then(Value::as_str) {
                let ok = frame.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let payload = if ok {
                    Ok(frame.get("result").cloned().unwrap_or(json!({})))
                } else {
                    Err(frame
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown sidecar error")
                        .to_string())
                };
                if let Ok(mut map) = pending.lock()
                    && let Some(tx) = map.remove(id)
                {
                    let _ = tx.send(payload);
                }
            }
        }
        // EOF: sidecar exited — fail all waiters so callers surface instantly.
        if let Ok(mut map) = pending.lock() {
            for (_, tx) in map.drain() {
                let _ = tx.send(Err("sidecar exited unexpectedly".into()));
            }
        }
    });
}

fn resolve_python(explicit: Option<&std::path::PathBuf>) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return p.clone();
    }
    crate::platform::find_python().unwrap_or_else(|| std::path::PathBuf::from("python3"))
}

/// Directory containing the `kernel_sidecar` python package. Env override wins so
/// installed binaries can point at a checkout; build-time fallback keeps dev
/// runs working from any cwd.
fn sidecar_cwd() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("KERNEL_SIDECAR_DIR").or_else(|_| std::env::var("PK_SIDECAR_DIR")) {
        return std::path::PathBuf::from(dir);
    }
    match option_env!("CARGO_MANIFEST_DIR") {
        Some(manifest) => std::path::PathBuf::from(manifest).join("../../kernel-sidecar"),
        None => std::path::PathBuf::from("kernel-sidecar"),
    }
}

pub(super) async fn ensure_handle(rt: &KernelRuntime) -> Result<Arc<SidecarHandle>, String> {
    {
        let guard = rt.handle_cell().lock().await;
        if let Some(h) = guard.as_ref()
            && h.is_alive().await
        {
            return Ok(h.clone());
        }
    }
    let mut guard = rt.handle_cell().lock().await;
    if let Some(h) = guard.as_ref()
        && h.is_alive().await
    {
        return Ok(h.clone());
    }
    let handle = spawn_handle(rt)?;
    *guard = Some(handle.clone());
    Ok(handle)
}

fn spawn_handle(rt: &KernelRuntime) -> Result<Arc<SidecarHandle>, String> {
    let settings = rt.settings();
    let python = resolve_python(settings.python.as_ref());
    let mut cmd = tokio::process::Command::new(&python);
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.current_dir(sidecar_cwd());
    cmd.arg("-m").arg("kernel_sidecar.server");
    if let Some(root) = settings.state_dir.as_ref() {
        cmd.arg("--state-root").arg(root);
    }
    if let Some(vendor_root) = settings.vendor_dir.as_ref()
        && let Some(parent) = vendor_root.parent()
    {
        cmd.env("KERNEL_SIDECAR_REPO_ROOT", parent);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("kernel-sidecar spawn failed ({}): {e}", python.display()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "kernel-sidecar missing stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "kernel-sidecar missing stdout".to_string())?;

    let writer = SidecarWriter {
        stdin: Arc::new(AsyncMutex::new(stdin)),
    };
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (bridge_tx, bridge_rx) = mpsc::unbounded_channel::<Inbound>();
    let last_activity = Arc::new(AtomicU64::new(now_secs()));
    spawn_reader(stdout, pending.clone(), bridge_tx, last_activity.clone());
    Ok(Arc::new(SidecarHandle {
        writer,
        child: Arc::new(AsyncMutex::new(child)),
        pending,
        bridge_rx: AsyncMutex::new(bridge_rx),
        last_activity,
    }))
}

/// One request/response round-trip with crash-transparent single retry.
pub(super) async fn request(rt: &KernelRuntime, method: &str, params: Value) -> Result<Value, String> {
    let budget = Duration::from_secs(rt.settings().request_timeout_secs.max(1));
    let mut consecutive_failures = 0u32;
    loop {
        let handle = ensure_handle(rt).await?;
        handle.touch();
        let id = format!("r{}", next_id());
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = handle.pending_map().lock() {
            map.insert(id.clone(), tx);
        }
        let frame = json!({"id": id, "method": method, "params": params});
        let write_ok = handle.writer.send_line(&frame.to_string()).await.is_ok();
        let result = if write_ok {
            match tokio::time::timeout(budget, rx).await {
                Ok(Ok(res)) => res,
                Ok(Err(_)) | Err(_) => {
                    if let Ok(mut map) = handle.pending_map().lock() {
                        map.remove(&id);
                    }
                    Err(format!(
                        "sidecar request '{method}' timed out after {}s",
                        budget.as_secs()
                    ))
                }
            }
        } else {
            Err("failed writing to sidecar".into())
        };

        match result {
            Ok(v) => return Ok(v),
            Err(e) => {
                consecutive_failures += 1;
                rt.record_failure();
                if e.contains("timed out") || consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    return Err(e);
                }
                tracing::warn!(target: "kernel", method, "sidecar crashed; restarting");
                rt.drop_handle().await;
            }
        }
    }
}

fn next_id() -> u64 {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Idle reaper: stops the child after `sidecar_idle_secs` without traffic.
pub(super) fn spawn_idle_reaper(rt: Arc<KernelRuntime>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let idle_limit = rt.settings().sidecar_idle_secs;
            if idle_limit == 0 {
                continue; // disabled
            }
            if let Some(h) = rt.handle_snapshot().await
                && h.idle_secs() >= idle_limit
                && h.is_alive().await
            {
                tracing::info!(target: "kernel", "idle timeout reached; stopping sidecar");
                h.kill().await;
                rt.drop_handle().await;
            }
        }
    });
}

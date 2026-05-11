use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{debug, info, warn};
use uuid::Uuid;

const MAX_OUTPUT_CHARS: usize = 200_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProcessStatus {
    Running,
    Exited,
    Killed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSession {
    pub id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
    pub started_at: f64,
    pub status: ProcessStatus,
    pub exit_code: Option<i32>,
    pub output_buffer: String,
    pub notify_on_complete: bool,
}

impl ProcessSession {
    fn new(command: String, cwd: Option<String>) -> Self {
        Self {
            id: format!("proc_{}", &Uuid::new_v4().to_string().replace('-', "")[..12]),
            command,
            cwd,
            pid: None,
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64(),
            status: ProcessStatus::Running,
            exit_code: None,
            output_buffer: String::new(),
            notify_on_complete: false,
        }
    }
}

#[derive(Debug)]
struct TrackedProcess {
    session: ProcessSession,
    output: Arc<RwLock<String>>,
}

impl TrackedProcess {
    fn snapshot(&self) -> ProcessSession {
        let mut s = self.session.clone();
        if let Ok(out) = self.output.try_read() {
            s.output_buffer = out.clone();
        }
        s
    }

    fn into_final(mut self) -> ProcessSession {
        if let Ok(out) = self.output.try_read() {
            self.session.output_buffer = out.clone();
        }
        self.session
    }
}

#[derive(Debug, Clone)]
pub struct ProcessRegistry {
    running: Arc<RwLock<HashMap<String, TrackedProcess>>>,
    finished: Arc<RwLock<HashMap<String, ProcessSession>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            running: Arc::new(RwLock::new(HashMap::new())),
            finished: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn spawn(&self, command: String, cwd: Option<String>) -> std::io::Result<ProcessSession> {
        let mut session = ProcessSession::new(command.clone(), cwd.clone());
        let sid = session.id.clone();

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.arg("/c").arg(&command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(&command);
            c
        };
        if let Some(ref dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let output = Arc::new(RwLock::new(String::new()));
        let out_reader = output.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            if let Some(mut s) = stdout {
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(text) = String::from_utf8(buf[..n].to_vec()) {
                                let mut out = out_reader.write().await;
                                out.push_str(&text);
                                if out.len() > MAX_OUTPUT_CHARS {
                                    *out = out[out.len().saturating_sub(MAX_OUTPUT_CHARS)..].to_string();
                                }
                                drop(out);
                                sleep(Duration::from_millis(1)).await;
                            }
                        }
                        Err(e) => {
                            debug!(pid = ?pid, error = %e, "stdout read error");
                            break;
                        }
                    }
                }
            }
            if let Some(mut s) = stderr {
                loop {
                    match s.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(text) = String::from_utf8(buf[..n].to_vec()) {
                                let mut out = out_reader.write().await;
                                out.push_str(&text);
                                if out.len() > MAX_OUTPUT_CHARS {
                                    *out = out[out.len().saturating_sub(MAX_OUTPUT_CHARS)..].to_string();
                                }
                                drop(out);
                                sleep(Duration::from_millis(1)).await;
                            }
                        }
                        Err(e) => {
                            debug!(pid = ?pid, error = %e, "stderr read error");
                            break;
                        }
                    }
                }
            }
        });

        let sid2 = sid.clone();
        let running = self.running.clone();
        let finished = self.finished.clone();

        tokio::spawn(async move {
            let exit_status = child.wait().await;
            let exit_code = exit_status.ok().and_then(|s| s.code());
            debug!(pid = ?pid, exit_code = ?exit_code, "Process exited");
            let mut run_map = running.write().await;
            let mut fin_map = finished.write().await;
            if let Some(tp) = run_map.remove(&sid2) {
                let mut fs = tp.into_final();
                fs.exit_code = exit_code;
                fs.status = ProcessStatus::Exited;
                fin_map.insert(sid2.clone(), fs);
                info!(session = %sid2, "Process finished");
            }
        });

        {
            let mut run_map = self.running.write().await;
            session.pid = pid;
            let tracked = TrackedProcess { session, output };
            let snapshot = tracked.snapshot();
            run_map.insert(sid, tracked);
            Ok(snapshot)
        }
    }

    pub async fn poll(&self, session_id: &str) -> Option<ProcessSession> {
        {
            let run_map = self.running.read().await;
            if let Some(tp) = run_map.get(session_id) {
                return Some(tp.snapshot());
            }
        }
        {
            let fin_map = self.finished.read().await;
            if let Some(session) = fin_map.get(session_id) {
                return Some(session.clone());
            }
        }
        None
    }

    pub async fn wait(&self, session_id: &str, timeout_secs: Option<u64>) -> Option<ProcessSession> {
        let start = Instant::now();
        let timeout = timeout_secs.map(Duration::from_secs).unwrap_or(Duration::from_secs(300));
        loop {
            {
                let fin_map = self.finished.read().await;
                if let Some(session) = fin_map.get(session_id) {
                    return Some(session.clone());
                }
            }
            {
                let run_map = self.running.read().await;
                if !run_map.contains_key(session_id) {
                    let fin_map = self.finished.read().await;
                    return fin_map.get(session_id).cloned();
                }
            }
            if start.elapsed() > timeout {
                warn!(session = %session_id, "Wait timeout");
                return self.poll(session_id).await;
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn kill(&self, session_id: &str) -> Result<(), String> {
        let mut run_map = self.running.write().await;
        if let Some(tp) = run_map.remove(session_id) {
            let mut session = tp.into_final();
            if let Some(pid) = session.pid {
                #[cfg(unix)]
                {
                    let result = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .output();
                    if let Err(e) = result {
                        warn!(session = %session_id, error = %e, "kill failed");
                    }
                }
                #[cfg(windows)]
                {
                    let result = std::process::Command::new("taskkill")
                        .arg("/PID").arg(pid.to_string())
                        .arg("/F")
                        .output();
                    if let Err(e) = result {
                        warn!(session = %session_id, error = %e, "taskkill failed");
                    }
                }
            }
            session.status = ProcessStatus::Killed;
            let mut fin_map = self.finished.write().await;
            fin_map.insert(session_id.to_string(), session);
            Ok(())
        } else {
            let fin_map = self.finished.read().await;
            if fin_map.contains_key(session_id) {
                Err("Process already exited".to_string())
            } else {
                Err(format!("Process '{}' not found", session_id))
            }
        }
    }

    pub async fn list(&self) -> Vec<ProcessSession> {
        let mut sessions = Vec::new();
        {
            let run_map = self.running.read().await;
            sessions.extend(run_map.values().map(|tp| tp.snapshot()));
        }
        {
            let fin_map = self.finished.read().await;
            sessions.extend(fin_map.values().cloned());
        }
        sessions.sort_by(|a, b| b.started_at.partial_cmp(&a.started_at).unwrap_or(std::cmp::Ordering::Equal));
        sessions
    }

    pub async fn running_count(&self) -> usize {
        self.running.read().await.len()
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_poll() {
        let registry = ProcessRegistry::new();
        let session = registry.spawn("echo hello world".to_string(), None).await.unwrap();
        assert!(session.id.starts_with("proc_"));
        assert!(session.pid.is_some());
        sleep(Duration::from_millis(500)).await;
        let polled = registry.poll(&session.id).await;
        assert!(polled.is_some());
        let finished = registry.wait(&session.id, Some(5)).await;
        assert!(finished.is_some());
        assert_eq!(finished.unwrap().status, ProcessStatus::Exited);
    }

    #[tokio::test]
    async fn test_kill_process() {
        let registry = ProcessRegistry::new();
        let session = registry.spawn("sleep 30".to_string(), None).await.unwrap();
        registry.kill(&session.id).await.unwrap();
        let polled = registry.poll(&session.id).await;
        assert!(polled.is_some());
        assert_eq!(polled.unwrap().status, ProcessStatus::Killed);
    }

    #[tokio::test]
    async fn test_list_processes() {
        let registry = ProcessRegistry::new();
        registry.spawn("echo first".to_string(), None).await.unwrap();
        registry.spawn("echo second".to_string(), None).await.unwrap();
        sleep(Duration::from_millis(500)).await;
        let sessions = registry.list().await;
        assert!(sessions.len() >= 2);
    }
}

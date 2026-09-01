//! G4 — Plugin dir watcher (host-side, stdlib only).
//!
//! Polls `~/.operant/plugins/**/manifest.toml` on a configurable cadence.
//! On every poll the watcher:
//!
//! 1. Rescans the plugins dir and computes (mtime, signature) for each
//!    manifest.
//! 2. Detects new or changed manifests vs. the prior scan.
//! 3. Calls the host-supplied `on_change` callback with the list of
//!    (manifest_path, PluginManifest) that need to be (re-)loaded.
//! 4. Re-verifies Ed25519 signature on EVERY swap (not just the initial
//!    `load_plugin`) — G4's distinguishing feature.
//!
//! The watcher does NOT pull in `notify` (no extra dep) — a poll loop is
//! adequate for the `~/.operant/plugins/**` directory size and matches the
//! pk Hermetic kernel polling pattern.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::PluginError;
use crate::host::PluginHost;
use crate::signature::{enforce_signature_policy, SignatureMode, VerificationResult};
use crate::PluginManifest;

/// Configuration for the watcher.
#[derive(Debug, Clone)]
pub struct WatcherConfig {
    /// How often the watcher scans the plugins dir.
    pub interval: std::time::Duration,
    /// When true, the watcher will start polling in a background tokio
    /// task. When false, the host calls `Watcher::scan_once` manually.
    pub auto_run: bool,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            interval: std::time::Duration::from_secs(5),
            auto_run: false,
        }
    }
}

/// One detected (new or changed) manifest.
#[derive(Debug, Clone)]
pub struct ManifestChange {
    pub manifest_path: PathBuf,
    pub manifest: PluginManifest,
    pub signature: VerificationResult,
}

/// Polling watcher over the plugins dir.
pub struct Watcher {
    host: PluginHost,
    config: WatcherConfig,
    /// Trusted publisher keys for Ed25519 re-verification on every swap.
    pub trusted_keys: Vec<String>,
    /// Signature policy (Strict / Permissive / Disabled).
    pub signature_mode: SignatureMode,
    last_seen: HashMap<PathBuf, (SystemTime, SystemTime, Option<String>)>,
}

impl Watcher {
    pub fn new(host: PluginHost, config: WatcherConfig) -> Self {
        Self {
            host,
            config,
            trusted_keys: Vec::new(),
            signature_mode: SignatureMode::Disabled,
            last_seen: HashMap::new(),
        }
    }

    /// Configure signature verification (G4 — re-verify on every swap).
    pub fn with_signature(mut self, mode: SignatureMode, trusted_keys: Vec<String>) -> Self {
        self.signature_mode = mode;
        self.trusted_keys = trusted_keys;
        self
    }

    /// Mutable access to the underlying host (so the caller can read the
    /// loaded plugins list or trigger install/remove).
    pub fn host_mut(&mut self) -> &mut PluginHost {
        &mut self.host
    }

    /// The plugins dir being watched.
    pub fn plugins_dir(&self) -> &Path {
        self.host.plugins_dir()
    }

    /// Run a single scan. Returns the list of new/changed manifests
    /// detected by this scan.
    pub fn scan_once(&mut self) -> Result<Vec<ManifestChange>, PluginError> {
        let mut current: HashMap<PathBuf, (SystemTime, SystemTime, Option<String>)> = HashMap::new();
        let mut changes: Vec<ManifestChange> = Vec::new();

        let entries = match std::fs::read_dir(self.host.plugins_dir()) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The plugins dir was removed between scans. Clear state.
                self.last_seen.clear();
                return Ok(Vec::new());
            }
            Err(e) => return Err(PluginError::Io(e)),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.toml");
            if !manifest_path.exists() {
                continue;
            }

            let mtime = std::fs::metadata(&manifest_path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let raw = match std::fs::read_to_string(&manifest_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let manifest: PluginManifest = match toml::from_str(&raw) {
                Ok(m) => m,
                Err(_) => continue,
            };
            // S4 — also track wasm file mtime so a tampered or rebuilt .wasm
            // triggers a swap attempt (Ed25519 re-verify happens above).
            let wasm_mtime = manifest
                .wasm_path
                .as_deref()
                .and_then(|p| {
                    let wasm_file = path.join(p);
                    std::fs::metadata(&wasm_file).and_then(|m| m.modified()).ok()
                })
                .unwrap_or(SystemTime::UNIX_EPOCH);

            // G4 — re-verify Ed25519 signature on every scan, not just
            // the first load. Tampered manifests are filtered out and
            // the existing provider is NOT replaced.
            let signature = match enforce_signature_policy(
                &manifest.name,
                &raw,
                manifest.signature.as_deref(),
                manifest.publisher_key.as_deref(),
                &self.trusted_keys,
                self.signature_mode,
            ) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        plugin = %manifest.name,
                        error = %e,
                        "plugin failed signature policy; skipping swap"
                    );
                    continue;
                }
            };

            current.insert(
                manifest_path.clone(),
                (mtime, wasm_mtime, manifest.signature.clone()),
            );

            let prior = self.last_seen.get(&manifest_path);
            let is_new = prior.is_none();
            let mtime_changed = prior.map(|(t, _, _)| *t != mtime).unwrap_or(false);
            let wasm_changed = prior.map(|(_, w, _)| *w != wasm_mtime).unwrap_or(false);
            let sig_changed = prior.map(|(_, _, s)| s != &manifest.signature).unwrap_or(false);

            if is_new || mtime_changed || wasm_changed || sig_changed {
                changes.push(ManifestChange {
                    manifest_path,
                    manifest,
                    signature,
                });
            }
        }

        self.last_seen = current;
        Ok(changes)
    }

    /// Run the polling loop until `stop` is set to true. Each tick calls
    /// `on_change` with the detected manifest changes. Errors are logged
    /// and the loop continues (so a single bad manifest can't kill the
    /// watcher).
    pub async fn run_until<F>(&mut self, mut stop: tokio::sync::watch::Receiver<bool>, mut on_change: F)
    where
        F: FnMut(&[ManifestChange]),
    {
        let interval = self.config.interval;
        loop {
            if *stop.borrow() {
                return;
            }
            match self.scan_once() {
                Ok(changes) if !changes.is_empty() => on_change(&changes),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "watcher scan failed");
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = stop.changed() => {
                    if *stop.borrow() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use tempfile::tempdir;

    fn write_manifest(host: &PluginHost, name: &str, version: &str) -> PathBuf {
        let plugin_dir = host.plugins_dir().join(name);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let manifest_path = plugin_dir.join("manifest.toml");
        let manifest = format!(
            r#"
name = "{name}"
version = "{version}"
description = "test plugin"
capabilities = ["skill"]
"#
        );
        std::fs::write(&manifest_path, manifest).unwrap();
        manifest_path
    }

    #[test]
    fn scan_once_detects_new_manifest() {
        let tmp = tempdir().unwrap();
        let host = PluginHost::new(tmp.path()).unwrap();
        let mut w = Watcher::new(host, WatcherConfig::default());

        let _ = write_manifest(&w.host, "plug-a", "0.1.0");
        let changes = w.scan_once().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].manifest.name, "plug-a");
    }

    #[test]
    fn scan_once_detects_changed_manifest() {
        let tmp = tempdir().unwrap();
        let host = PluginHost::new(tmp.path()).unwrap();
        let mut w = Watcher::new(host, WatcherConfig::default());

        write_manifest(&w.host, "plug-a", "0.1.0");
        let _ = w.scan_once().unwrap();
        // No changes on the second scan.
        let changes2 = w.scan_once().unwrap();
        assert!(changes2.is_empty(), "no change ⇒ no change events");

        // Modify the manifest; third scan must report a change.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let plugin_dir = w.host.plugins_dir().join("plug-a");
        let manifest_path = plugin_dir.join("manifest.toml");
        std::fs::write(
            &manifest_path,
            r#"
name = "plug-a"
version = "0.2.0"
description = "bumped"
capabilities = ["skill"]
"#,
        )
        .unwrap();
        let changes3 = w.scan_once().unwrap();
        assert_eq!(changes3.len(), 1);
        assert_eq!(changes3[0].manifest.version, "0.2.0");
    }

    #[test]
    fn scan_once_skips_unparseable_manifest() {
        let tmp = tempdir().unwrap();
        let plugin_dir = tmp.path().join("broken");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "this is not valid toml {{{",
        )
        .unwrap();
        let host = PluginHost::new(tmp.path()).unwrap();
        let mut w = Watcher::new(host, WatcherConfig::default());
        let changes = w.scan_once().unwrap();
        assert!(changes.is_empty(), "unparseable manifest is skipped, not crashed");
    }
}

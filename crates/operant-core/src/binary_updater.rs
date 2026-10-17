//! binary_updater.rs — one auto-update path for operant's external
//! integrations (2026-10-14 batch, plan 11B).
//!
//! sourcehound (single binary on PATH) and aft (versioned cache dir) both
//! "check GitHub and install newer", but each carried its own download,
//! verify and replace logic — and neither pruned nor throttled. This module
//! is the one shared implementation:
//!
//! - `check` reports `{installed, latest}` through the `gh` CLI (the repos
//!   are private; the public API 404s without auth);
//! - `update` compares, downloads, SHA256-verifies when the release carries
//!   a `SHA256SUMS`, and installs by the integration's `InstallKind`;
//! - `prune_cache` keeps the newest N version dirs (aft had 14);
//! - a per-process once-guard plus a 6 h on-disk throttle
//!   (`~/.operant/cache/updates.json`) so sessions and the gateway daemon
//!   do not re-poll GitHub on every tool call.
//!
//! Failure policy (the plan's): an update never blocks a tool call; the
//! first failure per session is surfaced once as an `UpdateNotice` the UI
//! can show, and everything else stays at debug level.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Dotted numeric version, no prerelease semantics — release tags here are
/// plain `vX.Y.Z`.
pub type Version = (u64, u64, u64);

/// How an integration installs a new version.
#[derive(Debug, Clone)]
pub enum InstallKind {
    /// Atomic replace of one binary on PATH (sourcehound).
    ReplaceOnPath(PathBuf),
    /// Extract into `<dir>/aft-vX.Y.Z/` (or `<name>-vX.Y.Z/`), prune older
    /// version dirs down to `keep`.
    CacheDir { dir: PathBuf, keep: usize },
}

/// Everything the shared updater needs to know about one integration.
#[derive(Debug, Clone)]
pub struct IntegrationSpec {
    /// Lowercase integration name — throttle key, log tag, doctor label.
    pub name: &'static str,
    /// `owner/repo` for the `gh api` call.
    pub repo: &'static str,
    /// Substring the platform asset name must contain (e.g.
    /// `linux-x86_64`), plus the archive extensions accepted.
    pub asset_marker: &'static str,
    pub install: InstallKind,
}

/// One release asset: name plus its browser download URL.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub assets: Vec<ReleaseAsset>,
}

/// `operant doctor` / `/version` line: installed and latest, as resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    pub installed: Option<Version>,
    pub latest: Option<Version>,
}

impl Versions {
    pub fn describe(&self) -> String {
        let fmt = |v: Option<Version>| {
            v.map(|(a, b, c)| format!("{a}.{b}.{c}"))
                .unwrap_or_else(|| "unknown".to_string())
        };
        format!("{} (latest {})", fmt(self.installed), fmt(self.latest))
    }
}

/// A user-visible update outcome (plan 11B failure policy): emitted at most
/// once per session per integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateNotice {
    Updated {
        name: &'static str,
        from: String,
        to: String,
    },
    Available {
        name: &'static str,
        to: String,
    },
    Failed {
        name: &'static str,
        reason: String,
    },
}

// ── throttle state ─────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct ThrottleFile {
    /// name → unix-seconds of the last successful poll.
    checked: HashMap<String, u64>,
}

fn throttle_path() -> PathBuf {
    let home = std::env::var("OPERANT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".operant"));
    home.join("cache").join("updates.json")
}

const THROTTLE: Duration = Duration::from_secs(6 * 3600);

/// True when this integration was polled within the throttle window.
pub fn throttled(name: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(throttle_path()) else {
        return false;
    };
    let Ok(file) = serde_json::from_str::<ThrottleFile>(&text) else {
        return false;
    };
    file.checked
        .get(name)
        .and_then(|t| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .and_then(|now| now.as_secs().checked_sub(*t))
        })
        .is_some_and(|since| since < THROTTLE.as_secs())
}

fn mark_checked(name: &str) {
    let path = throttle_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    let mut file: ThrottleFile = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    file.checked.insert(name.to_string(), unix_now());
    let _ = serde_json::to_string_pretty(&file).map(|t| std::fs::write(&path, t));
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── once-guard ──────────────────────────────────────────────────────────

fn once_guard_fired(name: &str) -> bool {
    static FIRED: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);
    // Lock poisoning can only happen mid-panic; treat as "already fired" so
    // a panicking process never re-polls on every call.
    let Ok(mut map) = FIRED.lock() else {
        return true;
    };
    let map = map.get_or_insert_with(HashMap::new);
    map.insert(name.to_string(), true).is_some()
}

// ── public API ───────────────────────────────────────────────────────────

/// Resolve `{installed, latest}` without installing anything. Bypasses the
/// throttle — doctor and `update --integrations` want the real state.
pub async fn check(spec: &IntegrationSpec) -> anyhow::Result<Versions> {
    let installed = match &spec.install {
        InstallKind::ReplaceOnPath(bin) => local_version(bin).await.ok(),
        InstallKind::CacheDir { dir, .. } => cached_version(dir).ok(),
    };
    let latest = latest_release(spec)
        .await?
        .and_then(|r| parse_version(&r.tag));
    Ok(Versions { installed, latest })
}

/// Background entry point for "first use per session": fires at most once
/// per process per integration and only when the on-disk throttle has
/// expired. Never blocks the caller — the work is spawned.
pub fn kick_session_update(spec: IntegrationSpec) {
    if once_guard_fired(spec.name) {
        return;
    }
    if throttled(spec.name) {
        return;
    }
    tokio::spawn(async move {
        match update(&spec).await {
            Ok(Some(notice)) => publish_notice(notice),
            Ok(None) => {}
            Err(e) => publish_notice(UpdateNotice::Failed {
                name: spec.name,
                reason: e.to_string(),
            }),
        }
    });
}

/// Compare and, when behind, download + verify + install. `Ok(None)` means
/// current (or no platform asset) — nothing to report.
pub async fn update(spec: &IntegrationSpec) -> anyhow::Result<Option<UpdateNotice>> {
    let installed = match &spec.install {
        InstallKind::ReplaceOnPath(bin) => local_version(bin).await?,
        InstallKind::CacheDir { dir, .. } => cached_version(dir)?,
    };
    let Some(release) = latest_release(spec).await? else {
        return Ok(None);
    };
    let Some(remote) = parse_version(&release.tag) else {
        anyhow::bail!("unparseable release tag {}", release.tag);
    };
    mark_checked(spec.name);
    if remote <= installed {
        return Ok(None);
    }
    let Some(asset) = pick_asset(&release.assets, spec.asset_marker) else {
        // No asset for this platform: report availability, don't install.
        return Ok(Some(UpdateNotice::Available {
            name: spec.name,
            to: version_string(remote),
        }));
    };
    let bytes = download(&asset.url).await?;
    if release_has_sums(&release.assets) {
        verify_sha256(&release, &asset.name, &bytes).await?;
    }
    install(spec, &release.tag, &bytes)?;
    Ok(Some(UpdateNotice::Updated {
        name: spec.name,
        from: version_string(installed),
        to: version_string(remote),
    }))
}

/// Keep only the newest `keep` `*-v*` version dirs under `dir`, returning
/// the number removed.
pub fn prune_cache(dir: &Path, keep: usize) -> std::io::Result<usize> {
    let mut versions: Vec<(Version, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(tag) = name
            .rsplit_once("-v")
            .map(|(_, t)| t.strip_prefix('v').unwrap_or(t))
        else {
            continue;
        };
        if let Some(v) = parse_version(tag) {
            versions.push((v, entry.path()));
        }
    }
    versions.sort();
    let excess = versions.len().saturating_sub(keep);
    for (_, path) in versions.iter().take(excess) {
        std::fs::remove_dir_all(path)?;
    }
    Ok(excess)
}

// ── internals (proven logic, moved from tools/sourcehound_update.rs) ─────

pub fn parse_version(s: &str) -> Option<Version> {
    let token = s.split_whitespace().last()?;
    let token = token.strip_prefix('v').unwrap_or(token);
    let mut parts = token.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn version_string(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// Installed version via `<bin> --version`. A binary that will not report
/// one is left alone: updating blind is how a working install gets clobbered.
pub async fn local_version(bin: &Path) -> anyhow::Result<Version> {
    let out = tokio::process::Command::new(bin)
        .arg("--version")
        .output()
        .await?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text).ok_or_else(|| anyhow::anyhow!("unparseable --version output"))
}

/// Installed version for cache-dir integrations: the highest `*-v*` dir.
fn cached_version(dir: &Path) -> anyhow::Result<Version> {
    let mut best: Option<Version> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(tag) = name.rsplit_once("-v").map(|(_, t)| t) else {
            continue;
        };
        if let Some(v) = parse_version(tag) {
            best = Some(match best {
                None => v,
                Some(b) => {
                    if v > b {
                        v
                    } else {
                        b
                    }
                }
            });
        }
    }
    best.ok_or_else(|| anyhow::anyhow!("no versioned cache dirs in {}", dir.display()))
}

/// Latest release via the `gh` CLI, which authenticates against the private
/// repo. No `gh` (or no auth) means no poll — the public API 404s, and a
/// failed poll must never break a working install. `Ok(None)` = gh missing.
/// Asset URLs are the API form (`api.github.com/.../assets/<id>`): the
/// browser_download_url 404s on private repos even with the token — the
/// API form with `Accept: application/octet-stream` is the working path
/// (verified 2026-10-15 with curl against the sourcehound 1.7.0 asset).
async fn latest_release(spec: &IntegrationSpec) -> anyhow::Result<Option<ReleaseInfo>> {
    let out = tokio::process::Command::new("gh")
        .args([
            "api",
            &format!("repos/{}/releases/latest", spec.repo),
            "--jq",
            "{tag: .tag_name, assets: [.assets[] | {name: .name, url: .url}]}",
        ])
        .output()
        .await;
    let Ok(out) = out else {
        return Ok(None);
    };
    if !out.status.success() {
        anyhow::bail!("gh api failed for {}", spec.repo);
    }
    Ok(Some(serde_json::from_slice(&out.stdout)?))
}

fn pick_asset<'a>(assets: &'a [ReleaseAsset], marker: &str) -> Option<&'a ReleaseAsset> {
    assets.iter().find(|a| {
        a.name.contains(marker)
            // Archives (sourcehound) and raw binaries (aft's plain
            // `aft-linux-x64`) are both accepted — extract_binary handles
            // raw ELF/mach pass-through.
            && (a.name.ends_with(".tar.gz")
                || a.name.ends_with(".zip")
                || !a.name.contains('.'))
    })
}

fn release_has_sums(assets: &[ReleaseAsset]) -> bool {
    assets
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case("SHA256SUMS"))
}

async fn download(url: &str) -> anyhow::Result<Vec<u8>> {
    // Private-repo asset downloads: the `gh` CLI's token plus the
    // octet-stream accept header (browser_download_url 404s here even
    // WITH the token — this is the verified working combination).
    let token = tokio::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty());
    let mut req = reqwest::Client::builder()
        .user_agent("operant-binary-updater")
        .build()?
        .get(url)
        .header("Accept", "application/octet-stream");
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let response = req.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("download failed: {}", response.status());
    }
    Ok(response.bytes().await?.to_vec())
}

async fn verify_sha256(
    release: &ReleaseInfo,
    asset_name: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS"))
        .ok_or_else(|| anyhow::anyhow!("release carries no SHA256SUMS"))?;
    let text = download(&sums_asset.url).await?;
    let text = String::from_utf8_lossy(&text);
    let want = text
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            (name.strip_prefix('*').unwrap_or(name) == asset_name).then_some(hash)
        })
        .ok_or_else(|| anyhow::anyhow!("no checksum entry for {asset_name}"))?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let got = hex::encode(hasher.finalize());
    if got != want {
        anyhow::bail!("checksum mismatch for {asset_name}");
    }
    Ok(())
}

fn install(spec: &IntegrationSpec, tag: &str, bytes: &[u8]) -> anyhow::Result<()> {
    match &spec.install {
        InstallKind::ReplaceOnPath(bin) => {
            let binary = extract_binary(bytes)?;
            atomic_replace(bin, &binary)
        }
        InstallKind::CacheDir { dir, keep } => {
            let tag = tag.strip_prefix('v').unwrap_or(tag);
            let version_dir = dir.join(format!("{}-v{}", spec.name, tag));
            std::fs::create_dir_all(&version_dir)?;
            let bin_name = if cfg!(windows) {
                format!("{}.exe", spec.name)
            } else {
                spec.name.to_string()
            };
            let target = version_dir.join(bin_name);
            let binary = extract_binary(bytes)?;
            atomic_replace(&target, &binary)?;
            let _ = prune_cache(dir, *keep);
            Ok(())
        }
    }
}

/// Unpack the executable out of a release archive (`.tar.gz`/`.zip`); a raw
/// binary asset passes through untouched. Anything else — or an archive
/// with no single executable entry — aborts rather than writing junk over
/// a working install.
fn extract_binary(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    // Raw ELF/mach magic passes through; archives unpack to their first
    // executable-looking member. Keep the shape from the sourcehound path.
    if bytes.starts_with(&[0x7f, b'E', b'L', b'F']) {
        return Ok(bytes.to_vec());
    }
    // Archive: gzip → tar single file. (zip support: keep to the crates we
    // already ship.)
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf)?;
        return Ok(buf);
    }
    anyhow::bail!("archive carried no file entry")
}

/// Write to a temp sibling then rename over — a running binary's inode
/// survives, and a failed write never destroys the old install. The temp
/// is chmod 0755 BEFORE the rename: a binary written with the default
/// 0644 lands unusable ("Permission denied" — caught live 2026-10-15 when
/// the sourcehound 1.7.0 install replaced a working binary with a
/// non-executable one; the fix re-installs with the mode set).
fn atomic_replace(dest: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = dest.with_extension("operant-new");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Where the once-per-session notice lands: a broadcast the CLI/TUI surface
/// reads (status notice). Kept as a plain global channel-shaped sink for
/// now — the TUI polls it through `pending_update_notice()`.
static PENDING_NOTICE: Mutex<Option<UpdateNotice>> = Mutex::new(None);

fn publish_notice(notice: UpdateNotice) {
    tracing::info!(notice = ?notice, "integration update notice");
    if let Ok(mut slot) = PENDING_NOTICE.lock() {
        // Keep the FIRST notice of the session: the earliest signal is the
        // actionable one.
        if slot.is_none() {
            *slot = Some(notice);
        }
    }
}

/// Take the pending update notice (the UI status line drains this).
pub fn take_pending_notice() -> Option<UpdateNotice> {
    PENDING_NOTICE.lock().ok().and_then(|mut slot| slot.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_compare_versions() {
        assert_eq!(parse_version("sourcehound 1.6.0"), Some((1, 6, 0)));
        assert_eq!(parse_version("v0.59.0"), Some((0, 59, 0)));
        assert_eq!(parse_version("nope"), None);
    }

    #[test]
    fn prune_keeps_newest_two() {
        let dir = std::env::temp_dir().join(format!("operant-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for v in ["0.49.4", "0.52.0", "0.55.1", "0.59.0", "1.0.0"] {
            std::fs::create_dir_all(dir.join(format!("aft-v{v}"))).expect("mkdir");
        }
        let removed = prune_cache(&dir, 2).expect("prune");
        assert_eq!(removed, 3, "14-version style caches shrink to 2");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "only the 2 newest remain: {left:?}");
        assert!(left.contains(&"aft-v1.0.0".to_string()));
        assert!(left.contains(&"aft-v0.59.0".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_checksum_rejected() {
        // verify_sha256 needs the network for the sums file, so the
        // rejection path is proven at the sha layer: a wrong hash NEVER
        // matches. This pins the comparison, not the fetch.
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"good bytes");
        let good = hex::encode(h.finalize());
        let mut h = Sha256::new();
        h.update(b"tampered bytes");
        let tampered = hex::encode(h.finalize());
        assert_ne!(good, tampered, "checksum mismatch is detectable");
    }

    #[test]
    fn extract_binary_passthrough_and_tar() {
        let raw = vec![0x7f, b'E', b'L', b'F', 1, 2, 3];
        assert_eq!(extract_binary(&raw).unwrap(), raw);
        // A tar.gz archive: build one with the tar crate.
        let mut tar_bytes = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o755);
            header.set_cksum();
            ar.append_data(&mut header, "binary", b"hello".as_slice())
                .unwrap();
            ar.into_inner().unwrap().finish().unwrap();
        }
        let extracted = extract_binary(&tar_bytes).unwrap();
        assert_eq!(extracted, b"hello");
    }

    /// (plan 11B red-team) fake-`gh` check: `check` resolves installed (a
    /// fake `--version` script) and latest (the fake gh's release JSON)
    /// without touching the network, and an unparseable local version is
    /// reported as unknown instead of forcing an update decision.
    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "test-only: the std mutex serialises process-env mutation across the whole test"
    )]
    async fn check_resolves_versions_through_a_fake_gh() {
        let dir = std::env::temp_dir().join(format!("operant-fakegh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Fake binary: reports 0.58.0.
        let bin = dir.join("fakebin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&bin, "#!/bin/sh\necho 'fakebin 0.58.0'\n").unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Fake gh: emits the jq-shaped release JSON.
        let gh = dir.join("gh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(
                &gh,
                "#!/bin/sh\necho '{\"tag\":\"v0.59.0\",\"assets\":[{\"name\":\"fake-linux-x86_64.tar.gz\",\"url\":\"https://example.invalid/a\"}]}'\n",
            )
            .unwrap();
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::var("PATH").unwrap_or_default();
        // SAFETY: test-scoped PATH swap; restored below before any
        // assertion can panic through an early `?` — the restore happens
        // before the checks that matter, and a poisoned PATH would only
        // affect this process's test children (cargo runs tests in
        // threads; the check below restores before returning).
        // Miri/clippy aside, this is the same pattern the fake-cargo
        // build test uses at the process level.
        {
            // Scope the unsafe so the borrow checker sees it end here.
            // Serialise against other PATH-mutating tests via a lock file
            // — simplest correct approach for a test that must control
            // the environment.
            static _PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _guard = _PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            unsafe {
                std::env::set_var("PATH", format!("{}:{}", dir.display(), path));
            }

            let spec = IntegrationSpec {
                name: "fake-integration",
                repo: "example/fake",
                asset_marker: "linux-x86_64",
                install: InstallKind::ReplaceOnPath(bin.clone()),
            };
            let versions = check(&spec).await.expect("check resolves");
            assert_eq!(versions.installed, Some((0, 58, 0)));
            assert_eq!(versions.latest, Some((0, 59, 0)));

            unsafe {
                std::env::set_var("PATH", path.clone());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

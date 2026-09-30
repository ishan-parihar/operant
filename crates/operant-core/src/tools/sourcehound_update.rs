//! Background auto-update polling for the sourcehound binary (AFT-shape).
//!
//! The first sourcehound tool call per session kicks one non-blocking check:
//! installed `--version` against the latest GitHub release tag, resolved via
//! the `gh` CLI (which carries auth for the private repo — the public API
//! 404s). A newer tag plus a known platform asset means download,
//! SHA256SUMS-verify, and atomic replace. Every failure degrades to a debug
//! log: the current call always uses the installed binary, and a platform
//! with no known asset is skipped, never guessed at.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;

/// Latest-release shape we read (via `gh api ... --jq`, so only these two
/// fields ever need parsing).
#[derive(Debug, Deserialize)]
struct ReleaseInfo {
    tag: String,
    assets: Vec<ReleaseAsset>,
}

/// One release asset: name plus its download URL.
#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    url: String,
}

/// Kick the once-per-session background update check for `bin`.
///
/// Cheap by construction: an `AtomicBool` swap, a PATH lookup at most once,
/// and the network work happens on a spawned task the caller never awaits.
pub fn kick_update_check() {
    static KICKED: AtomicBool = AtomicBool::new(false);
    if KICKED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(bin) = super::sourcehound::find_sourcehound_binary() else {
        return;
    };
    tokio::spawn(async move {
        if let Err(e) = check_and_update(&bin).await {
            tracing::debug!(error = %e, "sourcehound auto-update check failed (non-fatal)");
        }
    });
}

/// Compare-installed-against-latest and replace when behind. Any error aborts
/// the update, never the caller.
async fn check_and_update(bin: &Path) -> anyhow::Result<()> {
    let local = local_version(bin).await?;
    let release = latest_release().await?;
    let remote = parse_version(&release.tag)
        .ok_or_else(|| anyhow::anyhow!("unparseable release tag {}", release.tag))?;
    if !is_newer(&local, &remote) {
        return Ok(());
    }
    let Some(asset) = pick_asset(&release.assets) else {
        tracing::info!(
            tag = %release.tag,
            "newer sourcehound release available but no asset matches this platform — skipping"
        );
        return Ok(());
    };
    tracing::info!(tag = %release.tag, asset = %asset.name, "updating sourcehound binary");
    let bytes = download(&asset.url).await?;
    verify_sha256(&release, &asset.name, &bytes).await?;
    let binary = extract_binary(&asset.name, &bytes)?;
    replace_binary(bin, &binary).await?;
    Ok(())
}

/// Dotted numeric version, no prerelease semantics — release tags here are
/// plain `vX.Y.Z`.
type Version = (u64, u64, u64);

/// Parse `sourcehound 1.6.0` / `1.6.0` / `v1.6.0` into a triple. Anything else
/// is `None`, and an unparseable local version skips the update rather than
/// forcing one.
fn parse_version(s: &str) -> Option<Version> {
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

/// True when `remote` is strictly ahead of `local`. Equal means current —
/// never re-download what is installed.
fn is_newer(local: &Version, remote: &Version) -> bool {
    remote > local
}

/// Installed version via `<bin> --version`. A binary that will not report one
/// is left alone: updating blind is how a working install gets clobbered.
async fn local_version(bin: &Path) -> anyhow::Result<Version> {
    let out = tokio::process::Command::new(bin)
        .arg("--version")
        .output()
        .await?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text).ok_or_else(|| anyhow::anyhow!("unparseable --version output"))
}

/// Latest release via the `gh` CLI, which authenticates against the private
/// repo. No `gh` (or no auth) means no poll — the public API 404s, and a
/// failed poll must never break a working install.
async fn latest_release() -> anyhow::Result<ReleaseInfo> {
    let out = tokio::process::Command::new("gh")
        .args([
            "api",
            "repos/ishan-parihar/sourcehound/releases/latest",
            "--jq",
            "{tag: .tag_name, assets: [.assets[] | {name: .name, url: .browser_download_url}]}",
        ])
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!("gh api failed");
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

/// Asset markers per platform. Only pairs verified against real published
/// asset names are listed — today that is linux/x86_64
/// (`sourcehound-linux-x86_64.tar.gz`). Anything else returns `None` and the
/// update is skipped with an info log, not guessed at.
fn pick_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let marker: &str = match (os, arch) {
        ("linux", "x86_64") => "linux-x86_64",
        _ => return None,
    };
    assets.iter().find(|a| {
        a.name.contains(marker) && (a.name.ends_with(".tar.gz") || a.name.ends_with(".zip"))
    })
}

/// Fetch URL bytes with the shared reqwest shape.
async fn download(url: &str) -> anyhow::Result<Vec<u8>> {
    let response = reqwest::Client::builder()
        .user_agent("operant-sourcehound-updater")
        .build()?
        .get(url)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("download failed: {}", response.status());
    }
    Ok(response.bytes().await?.to_vec())
}

/// Verify `bytes` against the release's SHA256SUMS entry for `asset_name`.
/// The sums file is fetched alongside (same auth surface as the asset); a
/// missing entry or a mismatch aborts the update.
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

/// Unpack the `sourcehound` executable out of a release archive. Release
/// assets are `.tar.gz` (`.zip` accepted for forward-compat); a raw binary
/// asset passes through untouched. Anything else — or an archive with no
/// `sourcehound` entry — aborts the update rather than writing junk over a
/// working install.
fn extract_binary(asset_name: &str, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if asset_name.ends_with(".tar.gz") {
        let gz = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(gz);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().into_owned();
            let name = path.rsplit('/').next().unwrap_or(&path);
            if name == "sourcehound" || name == "sourcehound.exe" {
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut out)?;
                return Ok(out);
            }
        }
        anyhow::bail!("archive {asset_name} contains no sourcehound binary");
    }
    if asset_name.ends_with(".zip") {
        anyhow::bail!("zip assets not supported yet — skipping");
    }
    Ok(bytes.to_vec())
}

/// Write-temp + rename over `bin`, executable. Rename-over-running is fine on
/// Unix (inode swap); a platform that refuses it errors into the silent-debug
/// path, never into the caller.
async fn replace_binary(bin: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = bin.with_extension("new");
    tokio::fs::write(&tmp, bytes).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&tmp).await?.permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&tmp, perms).await?;
    }
    tokio::fs::rename(&tmp, bin).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The comparison is load-bearing for the whole updater: invert it and
    /// equal versions re-download forever while newer ones are skipped.
    #[test]
    fn newer_comparison_has_the_right_direction() {
        assert!(is_newer(&(1, 6, 0), &(1, 6, 1)));
        assert!(is_newer(&(1, 6, 0), &(1, 7, 0)));
        assert!(is_newer(&(1, 6, 0), &(2, 0, 0)));
        assert!(!is_newer(&(1, 6, 0), &(1, 6, 0)));
        assert!(!is_newer(&(1, 7, 0), &(1, 6, 9)));
    }

    /// Every shape the parser must accept or reject.
    #[test]
    fn version_parse_accepts_known_shapes_only() {
        assert_eq!(parse_version("sourcehound 1.6.0"), Some((1, 6, 0)));
        assert_eq!(parse_version("1.6.0"), Some((1, 6, 0)));
        assert_eq!(parse_version("v1.6.0"), Some((1, 6, 0)));
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("sourcehound"), None);
        assert_eq!(parse_version("1.6"), None);
        assert_eq!(parse_version("1.6.0.1"), None);
        assert_eq!(parse_version("nightly-2026"), None);
    }

    /// Asset selection on this machine's platform: the known linux asset is
    /// picked, and anything else (including a Windows-only release) skips.
    #[test]
    fn asset_pick_selects_only_the_known_marker() {
        let assets = vec![
            ReleaseAsset {
                name: "SHA256SUMS".into(),
                url: "https://example.com/SHA256SUMS".into(),
            },
            ReleaseAsset {
                name: "sourcehound-linux-x86_64.tar.gz".into(),
                url: "https://example.com/sh.tgz".into(),
            },
        ];
        let picked = pick_asset(&assets);
        if std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64" {
            assert_eq!(
                picked.map(|a| a.name.as_str()),
                Some("sourcehound-linux-x86_64.tar.gz")
            );
        } else {
            assert!(picked.is_none());
        }
        let win_only = vec![ReleaseAsset {
            name: "sourcehound-windows-x86_64.zip".into(),
            url: "https://example.com/sh.zip".into(),
        }];
        // On linux/x86_64 a Windows-only asset must not match; on other
        // platforms nothing matches either way.
        if std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64" {
            assert!(pick_asset(&win_only).is_none());
        }
    }
}

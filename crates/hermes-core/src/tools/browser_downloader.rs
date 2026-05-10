//! Browser Binary Downloader and Verifier for Hermes-RS
//! 
//! This module handles the automatic downloading and verification of the
//! Lightpanda browser binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use serde::Deserialize;

use crate::error::{Error, Result};

#[derive(Deserialize, Clone)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Clone)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub struct BrowserDownloader;

impl BrowserDownloader {
    /// Returns the default installation path for the browser binary
    pub fn default_bin_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".hermes")
            .join("bin")
            .join("browser")
    }

    /// Downloads the latest Lightpanda browser binary for the current platform
    pub async fn download_binary() -> Result<PathBuf> {
        let bin_path = Self::default_bin_path();
        
        if bin_path.exists() && Self::verify_binary(&bin_path).await.is_ok() {
            return Ok(bin_path);
        }

        tracing::info!("Downloading Lightpanda browser binary...");

        let release_url = "https://api.github.com/repos/lightpanda-io/browser/releases/latest";
        let client = reqwest::Client::builder()
            .user_agent("Hermes-RS-Downloader")
            .build()?;

        let release: GithubRelease = client
            .get(release_url)
            .send()
            .await?
            .json()
            .await?;

        let asset = Self::find_matching_asset(&release.assets)?;
        tracing::info!("Downloading asset: {}", asset.name);

        let response = client
            .get(&asset.browser_download_url)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(Error::Agent(format!("Failed to download binary: {}", response.status())));
        }

        if let Some(parent) = bin_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::File::create(&bin_path).await?;
        let mut content = response.bytes().await?;
        file.write_all(&content).await?;
        file.flush().await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin_path).await?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin_path, perms).await?;
        }

        if Self::verify_binary(&bin_path).await.is_err() {
            return Err(Error::Agent("Downloaded binary failed verification".to_string()));
        }

        tracing::info!("Browser binary successfully installed to {}", bin_path.display());
        Ok(bin_path)
    }

    /// Verifies that the binary exists and can be executed
    pub async fn verify_binary(path: &Path) -> Result<()> {
        if !path.exists() {
            return Err(Error::Config(format!("Binary not found at {}", path.display())));
        }

        let output = Command::new(path)
            .arg("--version")
            .output()
            .map_err(|e| Error::Agent(format!("Failed to execute binary: {}", e)))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Agent("Binary execution failed".to_string()))
        }
    }

    fn find_matching_asset(assets: &[GithubAsset]) -> Result<GithubAsset> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        
        tracing::debug!("Matching asset for OS: {}, Arch: {}", os, arch);
        
        assets.iter()
            .find(|a| a.name.contains(os) && a.name.contains(arch))
            .cloned()
            .ok_or_else(|| Error::Agent(format!("Could not find matching binary for {} on {}", arch, os)))
    }
}

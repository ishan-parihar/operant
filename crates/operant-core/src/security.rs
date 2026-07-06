//! Security utilities for Operant-RS
//!
//! Backend utility modules used by `skills_guard` and other internal components.
//! These are **not** registered tools — they are plain utility functions.
//!
//! Modules:
//!
//! - **tirith_security** — Subprocess wrapper for the `tirith` policy-as-code
//!   scanner.  Automatically downloads the binary from GitHub releases on first
//!   use.  Respects `fail_open` / `fail_closed` configuration.
//!
//! - **url_safety** — SSRF protection via DNS resolution + IP class checks.
//!   Checks resolved addresses against private, loopback, link-local, CGNAT,
//!   benchmarking, and cloud-metadata ranges.  Fail-closed on DNS errors.
//!
//! - **osv_check** — Queries [api.osv.dev](https://api.osv.dev) for MAL-type
//!   vulnerability advisories on PyPI packages.  Fail-open (returns empty on
//!   any error).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{Error, Result};

// ============================================================================
// Shared constants
// ============================================================================

/// Hostnames that are always blocked regardless of IP resolution.
const BLOCKED_HOSTNAMES: &[&str] = &["metadata.google.internal", "metadata.goog"];

// ============================================================================
// url_safety — SSRF protection
// ============================================================================

/// Check whether a URL resolves to a public (safe) address.
///
/// The URL's hostname is resolved to IP addresses and each is checked
/// against private, loopback, link-local, CGNAT (RFC 6598), benchmarking
/// (RFC 2544), and cloud-metadata ranges.
///
/// Returns:
/// - `Ok(true)` — all resolved IPs are public and safe.
/// - `Ok(false)` — at least one resolved IP is in a blocked range, or the
///   hostname itself is on the always-blocked list.
/// - `Err` — DNS resolution failed (fail-closed).
pub async fn check_url_safety(url: &str) -> Result<bool> {
    let parsed = url::Url::parse(url)
        .map_err(|e| Error::InvalidUrl(format!("failed to parse URL '{url}': {e}")))?;

    let hostname = parsed
        .host_str()
        .map(|h| h.trim().to_lowercase().trim_end_matches('.').to_string())
        .ok_or_else(|| Error::InvalidUrl("URL has no hostname".into()))?;

    if hostname.is_empty() {
        return Ok(false);
    }

    // Always-blocked hostnames fire regardless of IP resolution.
    if BLOCKED_HOSTNAMES.contains(&hostname.as_str()) {
        warn!(hostname = %hostname, "blocked request to internal hostname");
        return Ok(false);
    }

    // If the hostname is a literal IP, check it directly without DNS.
    if let Ok(ip) = IpAddr::from_str(&hostname) {
        return Ok(!is_blocked_ip(ip));
    }

    // Resolve the hostname to socket addresses.
    let addr_str = format!("{hostname}:0");
    let addrs = tokio::net::lookup_host(&addr_str).await.map_err(|e| {
        warn!(hostname = %hostname, error = %e, "DNS resolution failed for URL safety check");
        Error::Io(e)
    })?;

    for addr in addrs {
        if is_blocked_ip(addr.ip()) {
            warn!(hostname = %hostname, ip = %addr.ip(), "blocked request to blocked address");
            return Ok(false);
        }
    }

    Ok(true)
}

/// `true` when *ip* belongs to a private, internal, or blocked range.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || is_reserved_ipv4(v4)
                || is_cgnat(v4)
                || is_benchmarking(v4)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
                || is_unique_local_ipv6(v6)
        }
    }
}

/// 240.0.0.0/4 — Reserved for future use (RFC 1112).
fn is_reserved_ipv4(ip: Ipv4Addr) -> bool {
    const RESERVED_START: u32 = 0xF000_0000; // 240.0.0.0
    const RESERVED_END: u32 = 0xFFFF_FFFF; // 255.255.255.255
    let val = u32::from_be_bytes(ip.octets());
    (RESERVED_START..=RESERVED_END).contains(&val)
}

/// fd00::/8 — Unique Local Addresses (ULA, RFC 4193).
fn is_unique_local_ipv6(ip: Ipv6Addr) -> bool {
    ip.octets()[0] == 0xfd
}

/// 100.64.0.0/10 — Carrier-Grade NAT (RFC 6598).
///
/// Not covered by `Ipv4Addr::is_private()` or `is_global()`.
fn is_cgnat(ip: Ipv4Addr) -> bool {
    const CGNAT_START: u32 = 0x6440_0000; // 100.64.0.0
    const CGNAT_END: u32 = 0x647F_FFFF; // 100.127.255.255
    let val = u32::from_be_bytes(ip.octets());
    (CGNAT_START..=CGNAT_END).contains(&val)
}

/// 198.18.0.0/15 — Benchmarking (RFC 2544).
///
/// Not covered by `Ipv4Addr::is_private()` or `is_global()`.
fn is_benchmarking(ip: Ipv4Addr) -> bool {
    const BENCH_START: u32 = 0xC612_0000; // 198.18.0.0
    const BENCH_END: u32 = 0xC613_FFFF; // 198.19.255.255
    let val = u32::from_be_bytes(ip.octets());
    (BENCH_START..=BENCH_END).contains(&val)
}

// ============================================================================
// osv_check — OSV malware advisory checker
// ============================================================================

/// A malware advisory from the OSV database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsvAdvisory {
    /// OSV advisory ID (e.g. `MAL-2023-1234`).
    pub id: String,
    /// Human-readable summary of the advisory.
    #[serde(default)]
    pub summary: String,
    /// Optional CVSS severity score string.
    #[serde(default)]
    pub severity: Option<String>,
}

/// Query the OSV (Open Source Vulnerabilities) API for MAL-type advisories
/// on a PyPI package.
///
/// Sends a POST to `https://api.osv.dev/v1/query` with the package name and
/// version.  Only entries whose ID starts with `MAL-` are returned (regular
/// CVEs are ignored).
///
/// **Fail-open**: network errors, timeouts, or parse failures return an empty
/// vec — the caller should treat "unknown" as "allow" for this pre-flight
/// malware check.
pub async fn check_osv(package_name: &str, version: &str) -> Result<Vec<OsvAdvisory>> {
    match do_check_osv(package_name, version).await {
        Ok(vulns) => Ok(vulns),
        Err(e) => {
            debug!(error = %e, package = %package_name, "OSV check failed (allowing)");
            Ok(vec![])
        }
    }
}

/// Internal implementation — propagates errors for `check_osv` to catch.
async fn do_check_osv(package_name: &str, version: &str) -> Result<Vec<OsvAdvisory>> {
    let payload = serde_json::json!({
        "package": {
            "name": package_name,
            "ecosystem": "PyPI",
        },
        "version": version,
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(Error::Network)?;

    let resp = client
        .post("https://api.osv.dev/v1/query")
        .json(&payload)
        .header("User-Agent", "operant-core-osv-check/1.0")
        .send()
        .await
        .map_err(Error::Network)?;

    let data: serde_json::Value = resp.json().await.map_err(Error::Network)?;

    let vulns = data
        .get("vulns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|v| {
                    v.get("id")
                        .and_then(|id| id.as_str())
                        .map(|s| s.starts_with("MAL-"))
                        .unwrap_or(false)
                })
                .map(|v| OsvAdvisory {
                    id: v
                        .get("id")
                        .and_then(|id| id.as_str())
                        .unwrap_or("")
                        .to_string(),
                    summary: v
                        .get("summary")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    severity: v
                        .get("severity")
                        .and_then(|s| s.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|s| s.get("score"))
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(vulns)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- url_safety tests --------------------------------------------------

    #[test]
    fn test_is_cgnat() {
        assert!(is_cgnat(Ipv4Addr::new(100, 64, 0, 0)));
        assert!(is_cgnat(Ipv4Addr::new(100, 127, 255, 255)));
        assert!(is_cgnat(Ipv4Addr::new(100, 100, 100, 100)));
        assert!(!is_cgnat(Ipv4Addr::new(100, 128, 0, 0)));
        assert!(!is_cgnat(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_is_benchmarking() {
        assert!(is_benchmarking(Ipv4Addr::new(198, 18, 0, 0)));
        assert!(is_benchmarking(Ipv4Addr::new(198, 19, 255, 255)));
        assert!(!is_benchmarking(Ipv4Addr::new(198, 20, 0, 0)));
        assert!(!is_benchmarking(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn test_is_blocked_ip_private() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn test_is_blocked_ip_loopback() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(127, 255, 255, 255))));
    }

    #[test]
    fn test_is_blocked_ip_link_local() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
    }

    #[test]
    fn test_is_blocked_ip_cgnat() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200))));
    }

    #[test]
    fn test_is_blocked_ip_benchmark() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1))));
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(198, 19, 0, 1))));
    }

    #[test]
    fn test_is_blocked_ip_public() {
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(208, 67, 222, 222))));
    }

    #[test]
    fn test_is_blocked_ip_v6_loopback() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn test_is_blocked_ip_v6_private() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xfd00, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn test_is_blocked_ip_v6_public() {
        assert!(!is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

}

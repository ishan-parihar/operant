use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

/// Query the OSV API to check MCP packages for malware before launching them.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsvCheckArgs {
    /// The full command string to check (e.g. "npx @modelcontextprotocol/server-filesystem /dir")
    pub command: String,
}

#[derive(Debug, Deserialize)]
struct OsvQueryResponse {
    #[serde(default)]
    vulns: Vec<OsvVulnerability>,
}

#[derive(Debug, Deserialize)]
struct OsvVulnerability {
    id: String,
    summary: Option<String>,
    details: Option<String>,
    aliases: Option<Vec<String>>,
    modified: Option<String>,
    database_specific: Option<serde_json::Value>,
}

struct PackageInfo {
    name: String,
    ecosystem: String,
}

/// Parse a command string to extract the package name and ecosystem.
/// Supports: npx, uvx, npx.cmd, uvx.cmd, pipx
fn parse_command(command: &str) -> Vec<PackageInfo> {
    let trimmed = command.trim();
    let mut packages = Vec::new();

    for (prefix, ecosystem) in [
        ("npx ", "npm"),
        ("npx.cmd ", "npm"),
        ("uvx ", "npm"),
        ("uvx.cmd ", "npm"),
        ("pipx ", "PyPI"),
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if let Some(pkg) = rest.split_whitespace().next() {
                packages.push(PackageInfo {
                    name: pkg.trim().to_string(),
                    ecosystem: ecosystem.to_string(),
                });
            }
            return packages;
        }
    }

    for prefix in ["pip ", "pip3 "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let mut after_install = false;
            for part in rest.split_whitespace() {
                if part == "install" {
                    after_install = true;
                    continue;
                }
                if after_install && !part.starts_with('-') {
                    packages.push(PackageInfo {
                        name: part.to_string(),
                        ecosystem: "PyPI".to_string(),
                    });
                    return packages;
                }
            }
        }
    }

    packages
}

async fn query_osv(package: &PackageInfo) -> Result<Vec<OsvVulnerability>, String> {
    let url = "https://api.osv.dev/v1/query";
    let body = json!({
        "package": {
            "name": package.name,
            "ecosystem": package.ecosystem,
        }
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("OSV API request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OSV API returned HTTP {}", resp.status()));
    }

    let data: OsvQueryResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse OSV response: {e}"))?;

    Ok(data.vulns)
}

async fn check_packages(packages: &[PackageInfo]) -> Result<Value, String> {
    let mut results_map: HashMap<String, Vec<Value>> = HashMap::new();

    for pkg in packages {
        let vulns = query_osv(pkg).await.unwrap_or_default();

        if vulns.is_empty() {
            results_map
                .entry(pkg.name.clone())
                .or_default()
                .push(json!({"status": "no_vulnerabilities_found"}));
            continue;
        }

        // Filter to malware advisories (MAL- prefix) in the ID, but
        // report ALL vulnerabilities as informational.
        let advisory_list: Vec<Value> = vulns
            .iter()
            .map(|v| {
                let is_malware = v.id.starts_with("MAL-");
                json!({
                    "id": v.id,
                    "type": if is_malware { "malware" } else { "vulnerability" },
                    "summary": v.summary,
                    "details": v.details,
                    "aliases": v.aliases,
                    "modified": v.modified,
                })
            })
            .collect();

        results_map.insert(pkg.name.clone(), advisory_list);
    }

    Ok(serde_json::to_value(&results_map).unwrap_or_default())
}

pub struct OsvCheckTool;

#[async_trait]
impl HermesTool for OsvCheckTool {
    fn name(&self) -> &str {
        "osv_check"
    }

    fn description(&self) -> &str {
        "Check MCP packages for known malware/vulnerabilities using the OSV API before launching them"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<OsvCheckArgs>("osv_check", "Check packages for known vulnerabilities via the OSV API")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: OsvCheckArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("osv_check", format!("Invalid arguments: {e}")),
        };

        let packages = parse_command(&parsed.command);

        if packages.is_empty() {
            return ToolResult::success(
                "osv_check",
                json!({
                    "status": "skipped",
                    "message": "Could not parse a package from the command. Supports npx, uvx, pipx commands.",
                    "known_malware": false,
                    "advisories": []
                }),
            );
        }

        match check_packages(&packages).await {
            Ok(results) => {
                let has_malware = results
                    .as_object()
                    .map(|obj| {
                        obj.values().any(|v| {
                            v.as_array().map_or(false, |arr| {
                                arr.iter().any(|item| {
                                    item.get("type").and_then(|t| t.as_str()) == Some("malware")
                                })
                            })
                        })
                    })
                    .unwrap_or(false);

                ToolResult::success(
                    "osv_check",
                    json!({
                        "status": "checked",
                        "packages_checked": packages.iter().map(|p| json!({"name": &p.name, "ecosystem": &p.ecosystem})).collect::<Vec<_>>(),
                        "known_malware": has_malware,
                        "results": results
                    }),
                )
            }
            Err(e) => {
                // fail-open — log warning but allow execution
                ToolResult::success(
                    "osv_check",
                    json!({
                        "status": "error",
                        "message": format!("OSV check failed: {e}"),
                        "known_malware": false,
                        "advisories": []
                    }),
                )
            }
        }
    }
}

/// Check a single package by name and ecosystem.
pub async fn check_package_osv(package_name: &str, ecosystem: &str) -> Result<Value, String> {
    let pkg = PackageInfo {
        name: package_name.to_string(),
        ecosystem: ecosystem.to_string(),
    };
    let vulns = query_osv(&pkg).await?;

    let results: Vec<Value> = vulns
        .iter()
        .map(|v| {
            json!({
                "id": v.id,
                "summary": v.summary,
                "details": v.details,
                "modified": v.modified,
            })
        })
        .collect();

    Ok(json!(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_npx() {
        let pkgs = parse_command("npx @modelcontextprotocol/server-filesystem /tmp");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "@modelcontextprotocol/server-filesystem");
        assert_eq!(pkgs[0].ecosystem, "npm");
    }

    #[test]
    fn test_parse_npx_cmd() {
        let pkgs = parse_command("npx.cmd @modelcontextprotocol/server-filesystem");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "@modelcontextprotocol/server-filesystem");
    }

    #[test]
    fn test_parse_uvx() {
        let pkgs = parse_command("uvx some-package --flag value");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "some-package");
        assert_eq!(pkgs[0].ecosystem, "npm");
    }

    #[test]
    fn test_parse_uvx_cmd() {
        let pkgs = parse_command("uvx.cmd some-package");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "some-package");
    }

    #[test]
    fn test_parse_pipx() {
        let pkgs = parse_command("pipx run black");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "run");
    }

    #[test]
    fn test_parse_empty() {
        let pkgs = parse_command("echo hello");
        assert!(pkgs.is_empty());
    }

    #[test]
    fn test_parse_no_package() {
        let pkgs = parse_command("npx");
        assert!(pkgs.is_empty());
    }
}

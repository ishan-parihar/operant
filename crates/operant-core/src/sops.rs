//! Standard operating procedures — the operator's playbooks.
//!
//! An SOP is a TOML file in `~/.operant/sops/<name>.toml`:
//!
//! ```toml
//! name = "incident-response"
//! description = "What to do when a service goes down"
//! steps = """
//! 1. Check the dashboard health endpoint.
//! 2. Page the on-call engineer if the outage is user-visible.
//! """
//! ```
//!
//! Before iter-1126 the directory was read by `operant sop` alone: the
//! operator could list and show the files, but the AGENT could not see
//! them, so a playbook the harness could have followed sat unused.

use std::path::{Path, PathBuf};

/// One discovered SOP.
#[derive(Debug, Clone)]
pub struct Sop {
    /// The SOP name (the file stem).
    pub name: String,
    /// The one-line description (`description` in the file, or the first
    /// non-empty line).
    pub description: String,
    /// The file it came from.
    pub path: PathBuf,
    /// The full file content.
    pub content: String,
}

/// The directory the SOP files live in.
pub fn sops_dir() -> PathBuf {
    crate::platform::operant_home().join("sops")
}

/// Discover the SOPs in a directory. Files that are not TOML are skipped;
/// a file that does not parse still shows up (with a placeholder
/// description) so the operator can see and fix it.
pub fn discover_sops(dir: &Path) -> Vec<Sop> {
    let mut sops = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return sops;
    };
    let mut files: Vec<_> = entries.filter_map(Result::ok).collect();
    files.sort_by_key(std::fs::DirEntry::file_name);
    for entry in files {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let description = content
            .parse::<toml::Value>()
            .ok()
            .and_then(|v| v.get("description").cloned())
            .map(|v| match v {
                toml::Value::String(s) => s,
                other => other.to_string(),
            })
            .unwrap_or_else(|| "(no description)".to_string());
        sops.push(Sop {
            name,
            description,
            path,
            content,
        });
    }
    sops.sort_by(|a, b| a.name.cmp(&b.name));
    sops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sops_are_discovered_sorted_with_their_description() {
        let dir = std::env::temp_dir().join(format!("operant-sops-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("b-second.toml"),
            "description = \"the second one\"\n",
        )
        .expect("write");
        std::fs::write(dir.join("a-first.toml"), "name = \"x\"\n").expect("write");
        std::fs::write(dir.join("ignored.md"), "not a sop").expect("write");

        let sops = discover_sops(&dir);
        assert_eq!(sops.len(), 2, "{sops:?}");
        assert_eq!(sops[0].name, "a-first");
        assert_eq!(sops[1].description, "the second one");
        // A file with no description still lists, with a placeholder.
        assert_eq!(sops[0].description, "(no description)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

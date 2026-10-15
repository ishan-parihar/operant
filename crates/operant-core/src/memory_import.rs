//! memory_import.rs — one-time MEMORY.md/USER.md → memory-wire importer
//! (2026-10-14 batch, plan 11C3).
//!
//! When BuiltinProvider dies, the file-backed memories a user accumulated
//! must NOT. On startup, if `MEMORY.md` / `USER.md` exist in the operant
//! home, every block is parsed with the SAME §-format parser that wrote
//! them and retained into the memory-wire provider's default bank (tagged
//! `[imported, <block_type>]` so the origin is queryable). The file is
//! then renamed to `*.imported` — the rename IS the idempotency marker: a
//! second start sees no `MEMORY.md` and does nothing. The renamed file is
//! kept (never deleted): it is the user's data and the rollback path.

use std::path::Path;

/// Run the import for a directory if the legacy files exist. Returns how
/// many entries were retained (0 when nothing to import). The retain
/// callback returns whether the entry LANDED; on any failure the file is
/// NOT renamed — a later start retries, so no data can be lost to a
/// half-dead store.
pub fn import_legacy_files(
    dir: &Path,
    mut retain: impl FnMut(String, Vec<String>) -> bool,
) -> std::io::Result<usize> {
    let mut count = 0;
    // MEMORY.md — every block is a long-term memory.
    let memory_md = dir.join("MEMORY.md");
    if memory_md.exists() {
        let store = crate::memory::MemoryStore::new(dir.to_path_buf());
        let blocks = store.read_memories()?;
        let mut all_ok = !blocks.is_empty();
        for block in blocks.values() {
            let ok = retain(
                block.content.clone(),
                vec![
                    "imported".to_string(),
                    format!("type:{}", block.block_type),
                    format!("id:{}", block.id),
                ],
            );
            if ok {
                count += 1;
            } else {
                all_ok = false;
            }
        }
        if all_ok {
            rename_imported(&memory_md)?;
        }
    }
    // USER.md — profiles are facts about the user; one retain per profile
    // fact, tagged with the profile name.
    let user_md = dir.join("USER.md");
    if user_md.exists() {
        let store = crate::memory::MemoryStore::new(dir.to_path_buf());
        let profiles = store.read_profiles()?;
        let mut all_ok = !profiles.is_empty();
        let mut any = false;
        for (name, profile) in profiles {
            for fact in profile.facts {
                any = true;
                let text = format!(
                    "About {}: {}",
                    profile.name.as_deref().unwrap_or(&name),
                    fact.content
                );
                let ok = retain(
                    text,
                    vec![
                        "imported".to_string(),
                        "user_profile".to_string(),
                        format!("profile:{name}"),
                    ],
                );
                if ok {
                    count += 1;
                } else {
                    all_ok = false;
                }
            }
            for (key, value) in profile.preferences {
                any = true;
                let text = format!(
                    "About {}: prefers {key} = {value}",
                    profile.name.as_deref().unwrap_or(&name)
                );
                let ok = retain(
                    text,
                    vec!["imported".to_string(), "user_preference".to_string()],
                );
                if ok {
                    count += 1;
                } else {
                    all_ok = false;
                }
            }
        }
        if all_ok && any {
            rename_imported(&user_md)?;
        }
    }
    Ok(count)
}

fn rename_imported(path: &Path) -> std::io::Result<()> {
    let imported = path.with_extension("md.imported");
    std::fs::rename(path, imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_and_renames_idempotently() {
        let dir = std::env::temp_dir().join(format!("operant-memimport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Write a MEMORY.md in the § format MemoryStore itself produces.
        std::fs::write(
            dir.join("MEMORY.md"),
            "§ [id: fact1 | type: fact | importance: 80]\nUser prefers concise answers",
        )
        .unwrap();
        std::fs::write(dir.join("USER.md"), "").unwrap();

        let mut retained: Vec<(String, Vec<String>)> = Vec::new();
        let n = import_legacy_files(&dir, |content, tags| {
            retained.push((content, tags));
            true
        })
        .unwrap();
        assert_eq!(n, 1, "the one MEMORY.md block is retained");
        assert!(
            retained[0].0.contains("concise answers"),
            "content preserved: {:?}",
            retained[0].0
        );
        assert!(retained[0].1.contains(&"imported".to_string()));
        // Renamed: idempotent second run imports nothing.
        assert!(dir.join("MEMORY.md.imported").exists());
        assert!(!dir.join("MEMORY.md").exists());
        let n2 = import_legacy_files(&dir, |_, _| panic!("no second retain")).unwrap();
        assert_eq!(n2, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The no-data-loss contract: a failing retain leaves the file in place
    /// so the next start retries.
    #[test]
    fn failed_retain_leaves_file_for_retry() {
        let dir = std::env::temp_dir().join(format!("operant-memimport2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("MEMORY.md"),
            "§ [id: fact1 | type: fact | importance: 80]\nUser prefers concise answers",
        )
        .unwrap();
        let n = import_legacy_files(&dir, |_, _| false).unwrap();
        assert_eq!(n, 0, "nothing counted as landed");
        assert!(dir.join("MEMORY.md").exists(), "file kept for retry");
        assert!(!dir.join("MEMORY.md.imported").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

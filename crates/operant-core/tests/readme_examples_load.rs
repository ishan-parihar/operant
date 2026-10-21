#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The README's config examples must LOAD.
//!
//! The README is the operator's first contact with the config surface: it
//! carries a copy-paste `[client]`/`[agent]`/`[memory]` block that people
//! paste into `~/.operant/operant.toml`. If a key in that block does not
//! exist, the loader REJECTS THE WHOLE FILE (`deny_unknown_fields`), so a
//! wrong key in the README hands every operator who copies it a config
//! that will not load.
//!
//! That is not hypothetical: the README's example carried
//! `fallbacks = [ { model = "gpt-4o-mini" } ]` while the live field is
//! `fallback_models` (a list of strings) — the example did not parse, and
//! it was the only TOML block in the file, so every copy of it failed.
//!
//! This test parses every ```toml block in the README through the REAL
//! loader (`parse_config_str` — the same function `load_app_config` uses)
//! and fails on the first block that does not.

use std::path::{Path, PathBuf};

/// The repository root (this crate lives at `<root>/crates/operant-core`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-core is two levels under the repository root")
        .to_path_buf()
}

/// Every ```toml fenced block in the README.
fn toml_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```toml\n") {
        let after = &rest[start + "```toml\n".len()..];
        let Some(end) = after.find("```") else { break };
        blocks.push(after[..end].to_string());
        rest = &after[end + 3..];
    }
    blocks
}

#[test]
fn every_readme_config_example_loads() {
    let readme = repo_root().join("README.md");
    let text = std::fs::read_to_string(&readme).expect("the README is readable");
    let blocks = toml_blocks(&text);
    assert!(
        !blocks.is_empty(),
        "the README has no ```toml block any more — this test would be vacuous, \
         so either the examples moved or the test should go"
    );

    for (i, block) in blocks.iter().enumerate() {
        operant_core::config::parse_config_str(block, std::path::Path::new("memory://readme"))
            .unwrap_or_else(|e| {
                panic!("README config example {i} does not load: {e}\n--- the block ---\n{block}")
            });
    }
}

#[test]
fn the_readme_does_not_advertise_a_retired_config_key() {
    // `grant_ttl_days` was retired with the grant ledger (RT-136): an
    // approved plan IS the grant, so the TTL governed nothing. The live
    // loader still ACCEPTS the key (the field is still there, and every
    // existing config carries it), so a parse check cannot catch it — the
    // README must not advertise it as a working setting.
    let readme = repo_root().join("README.md");
    let text = std::fs::read_to_string(&readme).expect("the README is readable");
    assert!(
        !text.contains("grant_ttl_days"),
        "the README advertises `grant_ttl_days`, a key that governs nothing since \
         the grant ledger was retired (RT-136). An operator who copies it learns \
         a promise the binary cannot keep."
    );
}

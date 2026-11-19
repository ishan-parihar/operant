#![allow(clippy::expect_used, clippy::unwrap_used)]

//! F7 claimed `operant-cli/src/config.rs` carries a SECOND live config system:
//! a `config.yaml` / `config.local.yaml` loader feeding `delegation.*` and
//! `security.redact_secrets` live from `main.rs`. Measured at iter-2691, the
//! premise is false, and the way it is false is the kind that invites a
//! confident wrong answer: the fields exist, they are READ from `main.rs`, and
//! they look exactly like a parallel schema.
//!
//! What is actually true:
//!
//! * `CliConfig::load()` resolves `config_file` / `local_config_file` and never
//!   opens either one. There is no `serde_yaml` read of those paths anywhere in
//!   the workspace; the only `serde_yaml::Value` import serves an unrelated
//!   helper. The paths survive so `operant doctor` can *detect* a legacy
//!   `config.yaml` in the home and print "(Python compatibility)".
//! * `delegation.*` and `security.redact_secrets` are `CliConfig` fields with
//!   in-code defaults, populated by no file. `main.rs` reads them to seed the
//!   delegation tool's limits and `redaction::set_redact_enabled`; neither has
//!   an `AppConfig` home because they configure the PROCESS, not the agent.
//! * `to_app_config()` maps the API / agent / display fields onto `AppConfig`,
//!   which makes `CliConfig` the CLI's adapter onto the one system.
//!
//! So this guard pins the two claims a future reader will otherwise re-derive
//! the hard way: the YAML paths are NOT read, and the doc says so. If someone
//! ever DOES wire the YAML in, this test fails and tells them to update the
//! doc -- which is the point, because the doc is the deliverable here.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|p| p.join("AGENTS.md").exists() && p.join("crates").exists())
        .expect("workspace root (AGENTS.md + crates/)")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The legacy YAML paths must not be parsed.
///
/// The check is for a READ of the resolved path, not for the field's existence:
/// `config_file` is assigned in `load()` and is legitimately present, so a bare
/// name match would fail on the resolver. What must be absent is any file read
/// of it -- `read_to_string(config_file)`, `File::open(config_file)`, or a
/// `serde_yaml::from_reader` over either path.
#[test]
fn the_legacy_yaml_paths_are_never_parsed() {
    let cfg = read("crates/operant-cli/src/config.rs");
    for forbidden in [
        "read_to_string(&config.config_file)",
        "read_to_string(config.config_file)",
        "File::open(&config.config_file)",
        "File::open(&config.local_config_file)",
        "from_reader(config.config_file",
        "from_reader(&config.config_file",
        "from_str(&config.config_file",
    ] {
        assert!(
            !cfg.contains(forbidden),
            "crates/operant-cli/src/config.rs now reads the legacy YAML path (`{forbidden}`). F7 was closed on the finding that this does NOT happen -- the paths are resolved and stored for doctor's legacy detection only. If you are wiring the YAML in for real, this test is the tripwire: update docs/library/config.md and AGENTS.md in the SAME change."
        );
    }
}

/// `CliConfig::load()` must stay the documented shape: defaults, .env, env overrides.
///
/// The three steps are the doc's claim. Pinning them keeps the adapter honest if
/// someone later adds a fourth source without saying so.
#[test]
fn cli_config_load_reads_only_env_and_dotenv() {
    let cfg = read("crates/operant-cli/src/config.rs");
    // Split at the definition so the doc comment above it cannot satisfy this.
    let start = cfg
        .find("pub fn load() -> ConfigResult<Self>")
        .expect("CliConfig::load exists");
    let body = &cfg[start..];
    let end = body.find("\n    }").expect("load() body closes");
    let body = &body[..end];
    for needle in ["load_dotenv_file", "apply_operant_env_overrides"] {
        assert!(
            body.contains(needle),
            "CliConfig::load() no longer calls `{needle}`; docs/library/config.md says the loader reads the .env file and the OPERANT_*/HERMES_* overrides, and nothing else."
        );
    }
    assert!(
        !body.contains("from_yaml") && !body.contains("serde_yaml::from"),
        "CliConfig::load() now parses YAML, which contradicts the documented precedence (TOML file, .env, process env)."
    );
}

/// The docs must say the CLI layer is an adapter, not a second system.
#[test]
fn the_docs_name_the_cli_adapter() {
    for rel in ["docs/library/config.md", "AGENTS.md"] {
        let text = read(rel);
        assert!(
            text.contains("CliConfig"),
            "{rel} does not mention CliConfig. F7's whole finding was that a reader greps config.rs, sees config.yaml, and concludes there is a second config system. Naming the adapter is the fix."
        );
    }
    let lib = read("docs/library/config.md");
    assert!(
        lib.contains("never opened") || lib.contains("never parsed"),
        "docs/library/config.md no longer states that the legacy YAML paths are never parsed. That sentence is the entire F7 correction."
    );
}

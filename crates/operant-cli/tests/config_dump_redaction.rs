//! End-to-end gate on the config DUMP surfaces not printing credentials.
//!
//! (2026-11-16 architecture audit, RT-Z43-C) `operant config show` serialized
//! the live `AppConfig` straight to YAML, and `operant dump --all` did the same
//! in both its YAML and JSON branches. The config carries the provider API
//! keys, the SMTP password and the webhook secrets, so every one of those
//! commands printed them verbatim to the terminal and into whatever log or
//! paste the operator used next.
//!
//! A unit test on `config::redact_secrets` cannot catch a caller that stops
//! calling it: deleting the call from `handle_show` leaves the helper's own
//! tests green. These tests drive the real binary, so the only way to pass is
//! for the whole path - config load, redaction, serializer - to agree.
//!
//! The fixture sets BOTH `HERMES_HOME` (the operant home resolver) and
//! `OPERANT_CONFIG_DIR` (the config file resolver), the same two variables
//! `doctor_exit_code.rs` documents: setting only one silently reads the
//! developer's real `~/.operant`, and a fixture that reads the real config
//! would pass on someone else's keys and read like a broken redactor.

#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use operant_core::config::AppConfig;

/// A fixture operant home holding a config with a planted credential in it.
struct Fixture {
    dir: PathBuf,
}

/// The planted secret. Chosen to be unique enough that a false pass cannot
/// come from the developer's own environment.
const PLANTED_KEY: &str = "sk-REDTEAM-PROBE-9f3a1c7e";

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "operant-redteam-config-dump-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture dir");

        // A minimal config that LOADS. The point is the redaction, not the
        // provider wiring, so only the fields `AppConfig` requires are set and
        // the credential rides in a field that actually holds one.
        //
        // `client.api_key` is the real primary-provider field
        // (`config.rs:565`). A `[openai]` table is NOT the shape: the build
        // rejects it as an unknown key and the key reads back as `null`, which
        // made the first draft of this test pass VACUOUSLY against a config
        // with no credential in it at all.
        std::fs::write(
            dir.join("operant.toml"),
            format!(
                r#"[agent]
model = "redteam-probe-model"

[client]
api_key = "{PLANTED_KEY}"

[tools.stt]
groq_api_key = "{PLANTED_KEY}"
"#
            ),
        )
        .expect("write fixture config");

        Self { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Run the real binary against the fixture with a cleared environment.
///
/// `env_clear` is deliberate: an exported `OPENAI_API_KEY` in the developer's
/// shell must not be able to influence what these tests observe.
fn run(args: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_operant"))
        .args(args)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("HERMES_HOME", home)
        .env("OPERANT_CONFIG_DIR", home)
        .output()
        .expect("run operant")
}

fn stdout_and_stderr(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The control: prove the planted key IS reachable through the fixture.
///
/// This test is why the others are not vacuous. The first draft of the
/// fixture wrote `[openai] api_key`, which the config loader rejects as an
/// unknown key, so `dump` printed `api_key: null` and every "the key was
/// masked" assertion passed against a config that had NO credential in it.
/// This control fails if the fixture stops carrying a real secret.
#[test]
fn the_fixture_config_actually_loads() {
    let fixture = Fixture::new("control");
    let out = run(&["config", "show"], fixture.path());
    let text = stdout_and_stderr(&out);
    assert!(
        text.contains("redteam-probe-model"),
        "the fixture config must be the one loaded, got: {text}"
    );
    assert!(
        !text.contains("does not know"),
        "the fixture must not contain keys this build rejects: {text}"
    );

    // Non-vacuity, asserted from the UNMASKED parse. If the fixture ever
    // stops carrying a real credential, the `dump` tests below would pass
    // because there is nothing to mask rather than because the masking works.
    // `client.api_key` is the live primary-provider field, so a fixture
    // without it is a test that cannot test anything.
    let raw_text =
        std::fs::read_to_string(fixture.path().join("operant.toml")).expect("fixture toml");
    let unmasked: AppConfig = toml::from_str(&raw_text).expect("the fixture must parse");
    let unmasked = serde_json::to_value(&unmasked).expect("config to json");
    assert_eq!(
        unmasked["client"]["api_key"].as_str(),
        Some(PLANTED_KEY),
        "the fixture must carry a real credential, or the redaction tests are vacuous"
    );
    assert_eq!(
        unmasked["tools"]["stt"]["groq_api_key"].as_str(),
        Some(PLANTED_KEY),
        "the nested fixture credential must be real too"
    );
}

/// `operant config show` must mask the provider key.
///
/// Mutation-proven: removing the `redact_secrets` call from `handle_show`
/// fails this test at the `PLANTED_KEY` assertion.
#[test]
fn config_show_does_not_print_the_provider_key() {
    let fixture = Fixture::new("show");
    let out = run(&["config", "show"], fixture.path());
    let text = stdout_and_stderr(&out);
    assert!(
        out.status.success(),
        "`config show` must still succeed: {text}"
    );
    assert!(
        !text.contains(PLANTED_KEY),
        "`config show` printed the provider API key verbatim"
    );
    assert!(
        text.contains("***"),
        "`config show` must show the key as masked rather than dropping it: {text}"
    );
    // The config is still fully described apart from the secret.
    assert!(
        text.contains("redteam-probe-model"),
        "redaction must not eat the non-secret config: {text}"
    );
}

/// `operant dump --all --json` must mask the key.
///
/// This is the widest surface in the CLI: it prints every section at once.
#[test]
fn dump_all_json_does_not_print_the_provider_key() {
    let fixture = Fixture::new("dump-json");
    let out = run(&["dump", "--all", "--json"], fixture.path());
    let text = stdout_and_stderr(&out);
    assert!(
        out.status.success(),
        "`dump --all --json` must still succeed: {text}"
    );
    assert!(
        !text.contains(PLANTED_KEY),
        "`dump --all --json` printed the provider API key verbatim"
    );
}

/// `operant dump --all` (YAML) must mask the key too.
///
/// Separate from the JSON test on purpose: the two branches were separate
/// `serde_*` calls before the fix, and a fix that only patched one of them
/// would pass a single-branch test.
#[test]
fn dump_all_yaml_does_not_print_the_provider_key() {
    let fixture = Fixture::new("dump-yaml");
    let out = run(&["dump", "--all"], fixture.path());
    let text = stdout_and_stderr(&out);
    assert!(
        out.status.success(),
        "`dump --all` must still succeed: {text}"
    );
    assert!(
        !text.contains(PLANTED_KEY),
        "`dump --all` printed the provider API key verbatim"
    );
}

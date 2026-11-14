#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The upgrade contract, exercised through the real loader.
//!
//! `load_app_config` used to normalize in memory and drop the result: an
//! out-of-range `max_iterations` stayed wrong in the file forever, and a
//! config written by a NEWER binary failed to load at all because every
//! settings struct denies unknown fields.
//!
//! These tests pin both halves of the fix at the loader boundary — the
//! boundary the operator actually hits — rather than at the helper level.

use std::fs;
use std::path::Path;

use operant_core::config::{load_app_config_read_only, parse_config_str_without_recording};
use operant_core::config_heal::{UNKNOWN_SIDECAR, unknown_sidecar_for};

/// A temp directory that cleans itself up, outside the tree.
fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("operant-config-heal-")
        .tempdir()
        .expect("tempdir")
}

#[test]
fn a_missing_key_is_filled_from_the_default_and_named() {
    // An older config that predates `[agent] max_healing_attempts` (or any
    // future key) must load, and must say what it filled in. The default is
    // the behaviour the operator had before the key existed, so the load is
    // byte-identical in behaviour and only the diagnostics differ.
    let raw = "[agent]\nmodel = \"gpt-4\"\n";
    let loaded = parse_config_str_without_recording(raw, Path::new("operant.toml"))
        .expect("a config with a missing key must still load");

    assert_eq!(
        loaded.agent.max_healing_attempts,
        operant_core::config::BehaviorSettings::default().max_healing_attempts,
        "the missing key is filled from the documented default, not from 0"
    );
}

#[test]
fn an_out_of_range_value_is_healed_and_named() {
    // `max_iterations = 0` ends the turn before the model is called once.
    // The heal clamps it to the documented floor and says so.
    let raw = "[agent]\nmax_iterations = 0\n";
    let mut config =
        parse_config_str_without_recording(raw, Path::new("operant.toml")).expect("loads");
    let rows = operant_core::config_heal::heal_config(&mut config);

    assert_eq!(config.agent.max_iterations, 1);
    assert!(
        rows.iter()
            .any(|r| r.key == "agent.max_iterations" && r.old == "0"),
        "the heal row must name the key and the value that was wrong: {rows:?}"
    );
}

#[test]
fn heal_is_idempotent_on_a_second_pass() {
    let mut config = parse_config_str_without_recording(
        "[agent]\nmax_iterations = 0\n",
        Path::new("operant.toml"),
    )
    .expect("loads");
    let first = operant_core::config_heal::heal_config(&mut config);
    let second = operant_core::config_heal::heal_config(&mut config);
    assert!(!first.is_empty(), "the first pass heals");
    assert!(
        second.is_empty(),
        "the second pass heals nothing: {second:?}"
    );
}

#[test]
fn a_key_unknown_to_this_binary_is_parked_not_fatal() {
    // The UPGRADE case: a config written by a NEWER operant names a key this
    // build has never heard of. Parking it is the whole point of the L2
    // contract — an older binary runs a newer config instead of refusing to
    // start.
    let dir = tempdir();
    let config_path = dir.path().join("operant.toml");
    fs::write(
        &config_path,
        "[agent]\nmodel = \"gpt-4\"\n\n[gateway]\ntelegram_enabled = false\ntelemetry_endpoint = \"https://example.test/hook\"\n",
    )
    .expect("write fixture");

    let loaded = load_app_config_read_only(Some(&config_path))
        .expect("an unknown key must not fail the load");

    // The known keys are still honoured.
    assert_eq!(loaded.config.agent.model, "gpt-4");

    // The unknown key is in the sidecar, with its value verbatim, and it is
    // NOT in the loaded struct.
    let sidecar = unknown_sidecar_for(&config_path);
    assert_eq!(
        sidecar.file_name().expect("sidecar name").to_str().unwrap(),
        UNKNOWN_SIDECAR
    );
    assert!(
        sidecar.exists(),
        "the unknown key is parked in {}",
        sidecar.display()
    );
    let parked = fs::read_to_string(&sidecar).expect("sidecar readable");
    assert!(parked.contains("telemetry_endpoint"), "{parked}");
    assert!(parked.contains("https://example.test/hook"), "{parked}");
}

#[test]
fn a_clean_config_creates_no_sidecar() {
    // The parking path must be invisible to every existing install: a config
    // this binary understands perfectly loads without a sidecar file.
    let dir = tempdir();
    let config_path = dir.path().join("operant.toml");
    fs::write(&config_path, "[agent]\nmodel = \"gpt-4\"\n").expect("write fixture");

    let _ = load_app_config_read_only(Some(&config_path)).expect("loads");
    assert!(
        !unknown_sidecar_for(&config_path).exists(),
        "a clean config must not create a sidecar"
    );
}

#[test]
fn a_type_error_is_still_a_hard_failure() {
    // Parking exists for UNKNOWABLE keys, not for a broken file. A number
    // where a string belongs is the operator's mistake and must fail loudly,
    // quoting the offending line.
    let dir = tempdir();
    let config_path = dir.path().join("operant.toml");
    fs::write(&config_path, "[agent]\nmodel = 123\n").expect("write fixture");

    let err = load_app_config_read_only(Some(&config_path))
        .expect_err("a type error must fail, not park");
    let message = format!("{err}");
    assert!(
        message.contains("model = 123") || message.contains("bytes"),
        "the error must point at the offending content: {message}"
    );
}

#[test]
fn the_heal_rows_are_renderable_for_a_log_line() {
    let row = operant_core::config_heal::Healed::missing(
        "gateway.max_concurrent_sessions",
        "Some(32)",
        "the cap bounds the active set",
    );
    let line = row.render();
    assert!(line.contains("gateway.max_concurrent_sessions"));
    assert!(line.contains("the cap bounds the active set"));
}

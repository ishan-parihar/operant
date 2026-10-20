//! Probe: does the rustls client configuration build?
//!
//! `rustls::ClientConfig::builder()` resolves the crypto provider from
//! the crate FEATURES, and PANICS when they are ambiguous — "Could not
//! automatically determine the process-level CryptoProvider from Rustls
//! crate features". They ARE ambiguous in this workspace: `hyper-rustls`
//! (reached through `reqwest`) turns on `aws-lc-rs` by default, and
//! `tokio-rustls` turns on `ring` — so both are on whenever the
//! workspace is built with `--all-features`, and the workspace-wide
//! feature unification brings `aws-lc-rs` in even when a crate's own
//! dependency does not.
//!
//! The Email adapter's TLS setup and the proxy's TLS setup both hit that
//! panic, so the Email adapter was 100% non-functional and every proxied
//! WebSocket connection failed.
//!
//! The fix names the provider EXPLICITLY. This test pins the fix: it
//! builds the configuration the way the code does, and asserts it works.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "a test target: the expect IS the assertion"
)]

use operant_config::schema::client_config;

#[test]
fn the_rustls_client_config_builds_with_the_named_provider() {
    // The named-provider path: no feature inference, no panic.
    //
    // BUILDING IS THE ASSERTION: the failure mode is a PANIC inside
    // `builder()`, not a wrong value. So the test is "this call returns".
    let _config = client_config(rustls::RootCertStore::empty())
        .expect("the client config must build with the provider named");
}

#[test]
fn the_inferred_provider_path_is_still_ambiguous() {
    // This asserts the DEFECT is still present in the FEATURES — i.e. that
    // the named-provider fix is load-bearing and not a no-op.
    //
    // If a future dependency change makes the features unambiguous, this
    // test fails and the `client_config` helper becomes belt-and-braces
    // rather than the thing that prevents a panic. That is worth knowing:
    // it says the fix can be simplified, not that it broke.
    let result = std::panic::catch_unwind(|| {
        let _ = rustls::ClientConfig::builder();
    });
    assert!(
        result.is_err(),
        "the inferred provider was expected to be ambiguous; if it no longer \\
         is, the named-provider fix can be simplified"
    );
}

//! Probe: does the rustls client configuration build?
//!
//! `rustls::ClientConfig::builder()` resolves the crypto provider from
//! the crate FEATURES, and PANICS when they are ambiguous — "Could not
//! automatically determine the process-level CryptoProvider from Rustls
//! crate features". They WERE ambiguous in this workspace: `hyper-rustls`
//! (reached through `reqwest`) turns on `aws-lc-rs` by default, and
//! `tokio-rustls` turned on `ring` — so both were on whenever the
//! workspace was built with `--all-features`, and the workspace-wide
//! feature unification brought `aws-lc-rs` in even when a crate's own
//! dependency did not.
//!
//! The Email adapter's TLS setup and the proxy's TLS setup both hit that
//! panic, so the Email adapter was 100% non-functional and every proxied
//! WebSocket connection failed.
//!
//! TWO fixes, because the feature set was the accident and the call site
//! was the reliance:
//!
//! 1. `tokio-rustls` now declares `default-features = false` with `ring`
//!    explicitly, so `ring` is the only provider and the features are
//!    unambiguous.
//! 2. The call site names the provider EXPLICITLY, so it does not depend
//!    on a feature-unification accident.
//!
//! This test pins both: it builds the configuration the way the code does
//! (the named-provider path), and it asserts that the INFERRED path now
//! works too — so a future dependency change that reintroduces a second
//! provider fails here.

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
fn the_inferred_provider_path_also_works_now() {
    // The features are no longer ambiguous, so the inference resolves on
    // its own: `tokio-rustls` was declaring its DEFAULT features, which
    // pulled in `aws-lc-rs` alongside the `ring` that `hyper-rustls`
    // turns on — two providers, and `builder()` panics on that.
    // `tokio-rustls` now declares `default-features = false` with `ring`
    // explicitly, so `ring` is the only provider.
    //
    // The named-provider fix is therefore BELT-AND-BRACES rather than
    // load-bearing. That is the better state: the code does not depend on
    // a feature-unification accident, and this test says so.
    //
    // The test FAILS if a future dependency change reintroduces a second
    // provider — which is the signal that the named provider in
    // `client_config` is what prevents the panic.
    let result = std::panic::catch_unwind(|| {
        let _ = rustls::ClientConfig::builder();
    });
    assert!(
        result.is_ok(),
        "the inferred provider was expected to be unambiguous now; if it is \\
         not, a second provider is back and the named-provider fix is what \\
         prevents the panic"
    );
}

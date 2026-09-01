//! G12 — `Source` shim / `ProviderSource` unit-style compat test.
//!
//! The pk worktree's pk code imports `operant_harness::Source` (no
//! payload). G11 made `ProviderSource` carry an `Option<String>`
//! payload. To preserve the pk-facing API, `Source` is now a type
//! alias for `ProviderSource`, and `wasm()` / `pool()` provide
//! unit-style constructors. This test pins that compat so the
//! `operant-pk` crate keeps compiling without changes.

use operant_harness::{ProviderSource, Source};

#[test]
fn source_alias_is_provider_source() {
    // The pk-style unit source must still be constructible.
    let _unit: Source = ProviderSource::Native;
    let _unit_wasm: Source = ProviderSource::wasm();
    let _unit_pool: Source = ProviderSource::pool();
}

#[test]
fn unit_style_matches_explicit_none() {
    assert_eq!(ProviderSource::wasm(), ProviderSource::Wasm { path: None });
    assert_eq!(ProviderSource::pool(), ProviderSource::Pool { name: None });
}

#[test]
fn display_round_trip() {
    assert_eq!(ProviderSource::Native.to_string(), "native");
    assert_eq!(ProviderSource::Wasm { path: None }.to_string(), "wasm");
    assert_eq!(
        ProviderSource::Wasm {
            path: Some("/x.wasm".to_string())
        }
        .to_string(),
        "wasm(/x.wasm)"
    );
    assert_eq!(ProviderSource::Pool { name: None }.to_string(), "pool");
    assert_eq!(
        ProviderSource::Pool {
            name: Some("research-engine".to_string())
        }
        .to_string(),
        "pool(research-engine)"
    );
    assert_eq!(ProviderSource::ConfigRow.to_string(), "config-row");
}

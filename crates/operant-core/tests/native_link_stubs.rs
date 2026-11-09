//! RT-416: the native link contract for the espeak-ng audio-backend stubs.
//!
//! espeak-ng's cmake build DETECTS a system libsonic (it is detected, not
//! configured), so on any host with libsonic installed it compiles its sonic
//! speed-up path (`USE_LIBSONIC`) and the archive references seven `sonic*`
//! symbols. espeak-rs-sys's build script forwards only the libraries it
//! built itself (`speechPlayer`/`espeak-ng`/`ucd`) — never the system
//! dependency espeak-ng linked PRIVATE — so nothing emits `-lsonic` and the
//! final link fails with undefined symbols. operant-cli did not build.
//!
//! The repair is the stub file: the audio backend is stubbed anyway, so the
//! speed-up has nothing to speed up, and stubbing it needs neither the
//! system library nor its header on any host.
//!
//! This contract pins the repair's shape. It is a source contract because
//! the defect's observable is a LINK failure (the test binary cannot be
//! produced at all when the stubs are gone), so the behavioural evidence is
//! the link itself; the contract is what keeps the repair from being
//! silently deleted.

use std::path::Path;

/// The exact set of `sonic*` symbols espeak-ng references under
/// `USE_LIBSONIC` (espeak-rs-sys 0.1.9, pinned in Cargo.lock).
/// Derived from the dependency's sources: `sonicCreateStream`,
/// `sonicDestroyStream`, `sonicFlushStream`, `sonicGetSpeed`,
/// `sonicReadShortFromStream`, `sonicSetSpeed`, `sonicWriteShortToStream`.
const SONIC_SYMBOLS: &[&str] = &[
    "sonicCreateStream",
    "sonicDestroyStream",
    "sonicFlushStream",
    "sonicGetSpeed",
    "sonicReadShortFromStream",
    "sonicSetSpeed",
    "sonicWriteShortToStream",
];

fn crate_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn the_audio_stub_defines_every_sonic_symbol_espeak_references() {
    let stubs = std::fs::read_to_string(crate_dir().join("espeak_audio_stubs.c"))
        .expect("the audio stub file is in the crate root");
    for symbol in SONIC_SYMBOLS {
        // The definition, not a mention: the stub must DECLARE it. A mention
        // in a comment would satisfy a naive grep — so require the name to be
        // followed by an opening parenthesis (a call or a definition), which
        // a prose mention does not have.
        let declaration = format!("{symbol}(");
        assert!(
            stubs.contains(&declaration),
            "the stub file must define {symbol} — espeak-ng references it under \
             USE_LIBSONIC and nothing emits -lsonic"
        );
    }
}

#[test]
fn the_audio_stub_needs_neither_the_sonic_header_nor_the_library() {
    let stubs = std::fs::read_to_string(crate_dir().join("espeak_audio_stubs.c"))
        .expect("the audio stub file is in the crate root");
    assert!(
        !stubs.contains("#include <sonic.h>"),
        "the stub must not include sonic.h — the whole point is that the host \
         needs neither the header nor the library"
    );
    assert!(
        !stubs.contains("#include \"sonic.h\""),
        "the stub must not include sonic.h"
    );
}

#[test]
fn the_build_script_compiles_the_stub_file() {
    let build_rs =
        std::fs::read_to_string(crate_dir().join("build.rs")).expect("the build script exists");
    assert!(
        build_rs.contains("espeak_audio_stubs.c"),
        "the build script must compile the stub file — the symbols are only in \
         the final link if it does"
    );
    assert!(
        build_rs.contains("espeak_audio_stubs"),
        "the build script must link the compiled stub library"
    );
}

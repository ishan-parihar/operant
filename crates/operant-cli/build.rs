//! operant-cli build script (2026-10-14 batch, plan 11A).
//!
//! Exports the short memory-wire rev (7 hex chars) from the workspace
//! Cargo.lock so `--version` can embed it as a compile-time literal via
//! `concat!(env!(...))` — clap's `version` attribute only accepts
//! `'static`-like strings, and the project forbids Box::leak, so the honest
//! owner of this data is the build itself. The FULL rev comes from
//! operant-core's build.rs (`operant_core::MEMORY_WIRE_REV`) for doctor and
//! `operant version --detailed`.
//!
//! Deliberate duplication: build scripts cannot share workspace library code
//! without build-deps; this parser is 25 lines against a stable lock format,
//! mirrored on operant-core/build.rs.
//!
//! ## No RUNPATH any more (iter-2513)
//!
//! This build script used to emit a RUNPATH so the loader could find
//! `libonnxruntime.so.1`, which Kokoro (the previous TTS default) linked
//! dynamically. Without it the binary died before `main`:
//!
//! ```text
//! error while loading shared libraries: libonnxruntime.so.1: cannot open shared object file
//! ```
//!
//! The whole apparatus — the RUSTFLAGS copy in `dev-env.sh`, the library copy
//! in `install.sh`, the `ORT_LIB_LOCATION` pin, the API-24 probe — existed for
//! that one dependency. iter-2513 replaced Kokoro with Phonon, whose `ptts`
//! runtime is pure Rust, and `readelf -d` on the rebuilt binary shows no
//! libonnxruntime NEEDED entry at all. The emission is therefore GONE, not
//! disabled: a RUNPATH naming a directory nothing needs is a false claim in
//! the ELF header. If a future dependency links a shared library again, emit
//! the RUNPATH next to it, in the same change.
//!
//! One fact about `target/debug/operant` survives, because it is not about
//! linking at all: `cargo test` HARDLINKS the test harness over that path, so
//! after a test build it holds the harness, not the CLI. Rebuild with
//! `scripts/build.sh` before testing the binary.

fn main() {
    let rev = memory_wire_lock_rev().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!(
        "cargo:rustc-env=OPERANT_MEMORY_WIRE_REV_SHORT={}",
        &rev[..rev.len().min(7)]
    );
    // No rpath is emitted any more. Kokoro, the previous TTS default, linked
    // `libonnxruntime.so.1` dynamically and needed a RUNPATH pointing at
    // `~/.local/lib/operant` so an installed binary could find it (iter-425).
    // iter-2513 replaced Kokoro with Phonon, whose `ptts` runtime is pure Rust
    // with no shared ONNX dependency: `readelf -d target/release/operant` now
    // shows only libgcc_s/libm/libc/ld-linux. Adding an rpath back without a
    // corresponding NEEDED entry would be a lie in the ELF header, so this
    // call is gone rather than neutered.
}

/// The locked git rev of the `memory-wire` package: the 40-hex tail of its
/// `source = "git+…#<rev>"` line in the workspace Cargo.lock. See the twin
/// in operant-core/build.rs for the format argument (name → version → source
/// block order; the first flat rewrite returned on the version line and the
/// `--version` smoke test caught it reading "unknown").
fn memory_wire_lock_rev() -> Option<String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let root = std::path::Path::new(&manifest).parent()?.parent()?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).ok()?;
    lock.lines()
        .skip_while(|l| !l.starts_with("name = \"memory-wire\""))
        .take_while(|l| !l.starts_with("[[package]]"))
        .find_map(|l| {
            l.strip_prefix("source = ")
                .and_then(|rest| rest.trim_matches('"').rsplit('#').next())
                .filter(|rev| rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
        })
}

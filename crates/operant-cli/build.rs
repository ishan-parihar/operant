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
//! ## The ONNX Runtime RUNPATH
//!
//! The binary links `libonnxruntime.so.1` dynamically whenever the ONNX
//! Runtime is present, and the loader finds a shared library through the
//! RUNPATH — an absolute directory baked into the executable. WITHOUT one the
//! binary dies before `main`, so it cannot explain itself:
//!
//! ```text
//! error while loading shared libraries: libonnxruntime.so.1: cannot open shared object file
//! ```
//!
//! `scripts/dev-env.sh` adds that RUNPATH through `RUSTFLAGS`, so only a
//! build that SOURCED it got one. Every other build path in this tree — a
//! plain `cargo build`, and the `cargo build --release -p operant-cli` that
//! `scripts/install.sh` runs — produced an artifact that could not start, and
//! the failure named the shared object rather than the build command.
//! Measured twice in one session: once by `cargo build`, once by a
//! `cargo test` relink of the same path.
//!
//! The RUNPATH is therefore emitted HERE, by the build script, so every build
//! of the binary carries it. The directory is the RUNTIME copy
//! (`~/.local/lib/operant`), not the checkout: an rpath into a disposable
//! worktree dies with the worktree, and `install.sh` populates the runtime
//! directory with the shared object the loader is looking for.
//!
//! Two more facts about the same path:
//!
//! * `cargo test` HARDLINKS the test harness over `target/debug/operant`, so
//!   after a test build that path holds the harness, not the CLI. Rebuild
//!   with `scripts/build.sh` before testing the binary.
//! * The flag is Linux-only (`-Wl,-rpath` is a GNU ld spelling the MSVC
//!   linker does not accept) and it is gated on the TARGET's OS, not the
//!   host's, so a cross-link never receives it.

fn main() {
    let rev = memory_wire_lock_rev().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!(
        "cargo:rustc-env=OPERANT_MEMORY_WIRE_REV_SHORT={}",
        &rev[..rev.len().min(7)]
    );
    emit_onnx_runtime_rpath();
}

/// Emit the RUNPATH that lets the binary find `libonnxruntime.so.1`.
///
/// Emitted unconditionally on Linux: the runtime directory is where the
/// shared object is INSTALLED, so it may not exist yet at build time — and
/// on a machine where ONNX is linked statically the entry is simply unused.
fn emit_onnx_runtime_rpath() {
    // The TARGET's OS, not the host's: the flag must not reach a link for a
    // platform whose linker does not speak it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let dir = std::env::var("OPERANT_RUNTIME_LIB_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|h| !h.is_empty())
                .map(|home| format!("{home}/.local/lib/operant"))
        })
        .unwrap_or_default();
    if dir.is_empty() {
        return;
    }
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{dir}");
    println!("cargo:rerun-if-env-changed=OPERANT_RUNTIME_LIB_DIR");
    println!("cargo:rerun-if-env-changed=HOME");
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

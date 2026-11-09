fn main() {
    // No `rustc-link-lib=sonic` is emitted here, deliberately. History: the
    // flag was first gated to Linux (the only sonic artifact was a Linux ELF
    // `local/lib/libsonic.a`, which ld64 rejected on macOS), then deleted
    // outright — because untracking that archive (iter-499) removed the only
    // copy any fresh checkout could link, and `Native (Linux x86_64)` failed
    // with `unable to find library -lsonic`.
    //
    // RT-416 corrected the premise that deletion rested on. "Nothing
    // references a sonic symbol" was true only where espeak-ng's cmake build
    // does NOT detect a system libsonic: the detection is automatic, so on
    // any host with libsonic installed, espeak-ng compiles its sonic
    // speed-up path (USE_LIBSONIC) and the binary needs -lsonic — which
    // espeak-rs-sys's build script never forwards (it emits only the
    // libraries it built itself). The link then fails with undefined
    // sonic* symbols, and operant-cli does not build at all.
    //
    // The fix is the stub file below, not the flag: espeak's audio backend
    // is stubbed here anyway, so the speed-up has nothing to speed up, and
    // stubbing it needs neither the system library nor its header on any
    // host. The flag stays out — a host with a real audio backend and a real
    // libsonic can then link the real one by adding the flag AND dropping
    // the stub, which is a deliberate change, not a default.
    //
    // The payload dir is a machine-local prerequisite: repo-local `local/lib`
    // (gitignored) or $OPERANT_NATIVE_LIB_DIR. The search path MUST come from
    // build.rs — cargo's build.rustflags don't reach rustdoc, so doctests
    // (which link with CWD in a temp dir) only see link-search emitted here.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let dir = std::env::var("OPERANT_NATIVE_LIB_DIR")
        .unwrap_or_else(|_| format!("{manifest}/../../local/lib"));
    if std::path::Path::new(&dir).exists() {
        println!("cargo:rustc-link-search=native={dir}");
    }
    // Compile stub implementations for espeak-ng audio backend symbols
    // since espeak-rs-sys's cmake build may not find pulseaudio/portaudio dev libs
    // and the audio backend .cpp files don't get compiled
    println!("cargo:rerun-if-changed=espeak_audio_stubs.c");
    cc::Build::new()
        .file("espeak_audio_stubs.c")
        .compile("espeak_audio_stubs");

    // memory-wire provenance (2026-10-14 batch, plan 11A): bake the exact git
    // rev of the memory-wire dependency into the binary, read from the
    // workspace Cargo.lock at compile time. The lock is the single authority
    // for what was compiled, so `operant --version` and `operant doctor` can
    // report drift from upstream main without guessing. "unknown" (not a
    // build failure) when the lock is absent — cargo always resolves one
    // before compiling, so this is a vendored-dist edge case only.
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    println!(
        "cargo:rustc-env=OPERANT_MEMORY_WIRE_REV={}",
        memory_wire_lock_rev().unwrap_or_else(|| "unknown".to_string())
    );
}

/// Extract the locked git rev of the `memory-wire` package from the workspace
/// Cargo.lock: the 40-hex tail of its `source = "git+…#<rev>"` line. The
/// lock's block order is name → version → source, and dependency lists are
/// quoted strings (never `name = …`), so the first block whose name line
/// matches is unambiguous. An earlier flat rewrite returned on the first
/// non-source line inside the block (version precedes source) and so always
/// read "unknown" — the `--version` smoke test caught it.
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

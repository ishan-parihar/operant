fn main() {
    // No `rustc-link-lib=sonic` is emitted here, deliberately. History: the
    // flag was first gated to Linux (the only sonic artifact was a Linux ELF
    // `local/lib/libsonic.a`, which ld64 rejected on macOS), then deleted
    // outright — because untracking that archive (iter-499) removed the only
    // copy any fresh checkout could link, and `Native (Linux x86_64)` failed
    // with `unable to find library -lsonic`. Nothing references a sonic
    // symbol: espeak-rs-sys links its own bundled speechPlayer/espeak-ng/ucd
    // (plus per-platform extras), libsonic is an audio-BACKEND dependency,
    // and the backend is stubbed out below. Proven by deletion twice: the
    // Linux link is clean with the flag gone AND with no sonic artifact on
    // disk at all. If an environment ever builds the real audio backend, the
    // symbols come back — re-add the flag then, not before.
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
}

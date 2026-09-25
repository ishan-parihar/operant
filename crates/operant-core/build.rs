fn main() {
    // Link against system sonic library for espeak-rs-sys (TTS dependency).
    // The payload dir is a machine-local prerequisite: repo-local `local/lib`
    // (gitignored) or $OPERANT_NATIVE_LIB_DIR. The search path MUST come from
    // build.rs — cargo's build.rustflags don't reach rustdoc, so doctests
    // (which link with CWD in a temp dir) only see link-search emitted here.
    println!("cargo:rustc-link-lib=sonic");
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

fn main() {
    // Link against system sonic library for espeak-rs-sys (TTS dependency).
    // The payload dir is a machine-local prerequisite: repo-local `local/lib`
    // (gitignored) or $OPERANT_NATIVE_LIB_DIR. The search path MUST come from
    // build.rs — cargo's build.rustflags don't reach rustdoc, so doctests
    // (which link with CWD in a temp dir) only see link-search emitted here.
    // Linux only.
    //
    // The only sonic artifact this repo carries is a Linux x86-64 ELF archive
    // (`local/lib/libsonic.a` — verified ELF present, Mach-O absent), so
    // requiring `-lsonic` on macOS made ld64 reject it outright:
    //   ld: archive member '/' not a mach-o file in '.../local/lib/libsonic.a'
    // which failed `Native (macOS ARM64)` in the release-gating native job.
    //
    // Nothing here actually needs it. espeak-rs-sys never links sonic itself: it
    // links its own bundled speechPlayer/espeak-ng/ucd, then adds
    // Foundation + c++ on macOS, msvcrtd on Windows and stdc++ on Linux. libsonic
    // is an audio-BACKEND dependency, and the audio backend is stubbed out below,
    // so no sonic symbol is ever referenced. Verified rather than argued:
    // deleting this line entirely still links operant-cli cleanly on Linux
    // (exit 0, full TTS stack).
    //
    // Kept for Linux rather than dropped globally, deliberately. If an
    // environment ever builds the real audio backend instead of the stubs, the
    // symbols would come back, and a conditional that is wrong in one direction
    // is recoverable while a missing link flag is not.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=sonic");
    }
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

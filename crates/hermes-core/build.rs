fn main() {
    // Link against system sonic library for espeak-rs-sys (TTS dependency)
    println!("cargo:rustc-link-lib=sonic");
    // Compile stub implementations for espeak-ng audio backend symbols
    // since espeak-rs-sys's cmake build may not find pulseaudio/portaudio dev libs
    // and the audio backend .cpp files don't get compiled
    println!("cargo:rerun-if-changed=espeak_audio_stubs.c");
    cc::Build::new()
        .file("espeak_audio_stubs.c")
        .compile("espeak_audio_stubs");
}

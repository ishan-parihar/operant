fn main() {
    // The `channels-vendor` cfg is intentionally NOT declared in Cargo.toml:
    // the channel adapters it gates (lark, line, matrix, nostr, wechat,
    // voice-call/wake, whatsapp-web) reference vendor SDKs (matrix-sdk,
    // nostr-sdk, wa-rs, prost, cpal, …) that have never been wired into this
    // workspace and are not reachable from the CLI. Gating them keeps
    // `--all-features` green while the code stays in-tree for future wiring.
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"channels-vendor\"))");
}

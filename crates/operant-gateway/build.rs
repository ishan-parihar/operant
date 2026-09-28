//! Gateway build script.
//!
//! The `embedded-web` feature embeds the compiled frontend via `include_dir!`,
//! which **panics at compile time** if the directory is absent. The dist
//! directory is a build artifact that is not part of the repository, so we only
//! mark it available when it actually exists and let the runtime filesystem
//! fallback (`gateway.web_dist_dir`) serve the dashboard otherwise.
//!
//! The path matters. This used to check `<repo-root>/web/dist`, a location that
//! has never existed in this layout: the Vite project lives in
//! `crates/operant-cli/web/` and `vite.config.ts` sets
//! `outDir: "../src/dashboard"`, so the build lands in
//! `crates/operant-cli/src/dashboard/`. Because the check pointed somewhere the
//! build never writes, `embedded_web_dist_available` was never set and the
//! `embedded-web` feature silently degraded to the filesystem fallback on every
//! machine and every build — a feature that looks wired and embeds nothing.
//!
//! Keep this in sync with the `include_dir!` in `src/static_files.rs`; if the two
//! disagree the cfg is set but the embed panics, or the embed is skipped.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../operant-cli/src/dashboard");
    // Re-run this script whenever the dist directory changes.
    println!("cargo:rerun-if-changed=../operant-cli/src/dashboard");
    // Register the custom cfg so the `unexpected_cfgs` lint stays quiet.
    println!("cargo:rustc-check-cfg=cfg(embedded_web_dist_available)");

    if dist.join("index.html").is_file() {
        println!("cargo:rustc-cfg=embedded_web_dist_available");
    }
}

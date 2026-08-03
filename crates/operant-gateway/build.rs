//! Gateway build script.
//!
//! The `embedded-web` feature embeds the compiled frontend (`web/dist/`) via
//! `include_dir!`, which **panics at compile time** if the directory is absent.
//! The dist directory is a build artifact that is not part of the repository,
//! so we only mark it available when it actually exists and let the runtime
//! filesystem fallback (`gateway.web_dist_dir`) serve the dashboard otherwise.
//!
//! This keeps `cargo build --all-features` green on machines that have not
//! built the frontend, while still embedding the dist in release deployments
//! that include it.

use std::path::Path;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/dist");
    // Re-run this script whenever the dist directory changes.
    println!("cargo:rerun-if-changed=../../web/dist");
    // Register the custom cfg so the `unexpected_cfgs` lint stays quiet.
    println!("cargo:rustc-check-cfg=cfg(embedded_web_dist_available)");

    if dist.join("index.html").is_file() {
        println!("cargo:rustc-cfg=embedded_web_dist_available");
    }
}

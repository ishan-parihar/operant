fn main() {
    // The `hardware-vendor` cfg is intentionally NOT declared in Cargo.toml:
    // the vendor SDKs it gates (nusb, probe-rs, tokio-serial, aardvark-sys,
    // rppal) have never been wired in this workspace, so `--all-features`
    // must not enable them. Registering the cfg here keeps rustc quiet about
    // the (intentional) undeclared feature while the code stays in-tree for
    // future wiring. `probe` / `peripheral-rpi` are likewise legacy cfgs kept
    // for reference only.
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"hardware\", \"hardware-vendor\", \"probe\", \"peripheral-rpi\"))");
}

# kokoro-tiny 0.1.0 (patched: atty removed)

Vendored copy of [`kokoro-tiny`](https://crates.io/crates/kokoro-tiny) 0.1.0, used through
`[patch.crates-io]` in the workspace `Cargo.toml`. The TTS engine (`TtsEngine`) is the library
that `operant-core` uses.

## What changed

- `Cargo.toml`: the `atty` dependency is removed.
- `src/main.rs`: the one `atty::is(Stdin)` check becomes `std::io::IsTerminal::is_terminal(&std::io::stdin())`.
  `atty` is needed for nothing else in the crate.
- `src/lib.rs`: `save_flac` and its `"flac"` match arm are deleted. They called `flac::StreamWriter`, but the
  crate has no `flac` dependency and never declared a `flac-format` feature, so the branch could not compile if
  enabled. Clippy flagged it (`unexpected_cfgs`). The unused `Source` import from `rodio` is also removed.

## Why

`atty` 0.2.14 carries RUSTSEC-2021-0145 (unsound: an unaligned read on Windows) and RUSTSEC-2024-0375
(unmaintained). The standard library has provided `IsTerminal` since Rust 1.70, so the check does not
need a crate. The library itself never used `atty`; the dependency was for the binary only.

## Upkeep

Drop this patch when `kokoro-tiny` releases a version without `atty`. Its `ndarray-npy` dependency is
patched separately (`vendor/ndarray-npy`, RT-18 / AD-032).

Decision record: `docs/decisions/AD-033-*.md`.

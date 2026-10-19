# ndarray-npy 0.9.1 (patched: zip 2 → 9)

Vendored copy of [`ndarray-npy`](https://crates.io/crates/ndarray-npy) 0.9.1, used through
`[patch.crates-io]` in the workspace `Cargo.toml`. It is the reader behind
`kokoro-tiny`'s voice file (`voices-v1.0.bin`, an `.npz` archive).

## Why it is patched

`kokoro-tiny` 0.1.0 and its `main` branch both pin `ndarray-npy` to a version that
requires `zip` 2 (0.9.x) or `zip` 6 (0.10.x). Every `zip` release below 9.0.0 is in
RUSTSEC-2026-0367, -0368 and -0369, and those advisories have no `unaffected` range.
No published `kokoro-tiny` or `ndarray-npy` release reaches `zip` 9.

## What changed

- `Cargo.toml`: the `zip` requirement is `"9"` instead of `"2"`. Nothing else in the manifest changed.
- `src/npz.rs`: `ZipFile::name()` returns `ZipResult<Cow<str>>` in zip 9, so the one call site in
  `names()` takes `?` and derefs. No behaviour changes.

The crate's own 39 integration tests pass against zip 9.0.5
(`cargo test --manifest-path vendor/ndarray-npy/Cargo.toml --tests`).

## Upkeep

Drop this patch when `kokoro-tiny` releases a version that depends on `ndarray-npy` 0.10 or
later with `zip` 9 or newer, and `cargo audit` stays clean without it. The live check is
`cargo test -p operant-core kokoro_speaks -- --ignored` (needs `~/.cache/kokoros`).

Decision record: `docs/decisions/AD-032-*.md` (BUGS RT-18).

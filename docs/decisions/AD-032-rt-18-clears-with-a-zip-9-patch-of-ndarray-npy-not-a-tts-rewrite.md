---
ID: "AD-032"
Title: "RT-18 clears with a zip-9 patch of ndarray-npy, not a TTS-engine rewrite"
Status: "Active"
Date: "2026-10-19"
Description: "The three zip advisories are cleared by vendoring ndarray-npy 0.9.1 with zip 9; kokoro-micro was rejected because it still resolves zip 6"
---

# AD-032: RT-18 clears with a zip-9 patch of ndarray-npy, not a TTS-engine rewrite

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

RUSTSEC-2026-0367, -0368 and -0369 affect every `zip` crate below 9.0.0. Their `patched` range is `>= 9.0.0`, with no `unaffected` range. The path into the graph is `kokoro-tiny` 0.1.0 → `ndarray-npy` 0.9.1 → `zip` 2.4.2.

RT-18 recorded two owner options: (1) move to `kokoro-micro` 1.3.0, which needs a TTS rewrite, or (2) drop TTS from the default features. It recommended option 1.

## Decision

Keep TTS. Vendor `ndarray-npy` 0.9.1 at `vendor/ndarray-npy`, bump only its `zip` requirement to 9, and patch the workspace to it (`[patch.crates-io]`). The API `kokoro-tiny` uses is unchanged, apart from one call site in `npz.rs` (`ZipFile::name()` now returns a `Result`).

The recommendation in RT-18 was wrong. Measured against the lock file:

- `kokoro-micro` 1.3.0 depends on `ndarray-npy` 0.10.0, which depends on `zip` 6.0.0. That is inside the advisory range, so option 1 does not clear it.
- `kokoro-tiny` `main` (0ca44ce, version 0.2.0) also depends on `ndarray-npy` 0.10.0 and so also reaches `zip` 6.0.0. No upstream revision clears it.
- Option 2 removes a shipped feature to clear an advisory whose vulnerable functions this code does not call.

## Consequences

- `zip` 9.0.5 is the only `zip` in the lock. `cargo audit --no-fetch` lists no `zip` advisory.
- The vendored crate carries a 4-line change and a README (`vendor/ndarray-npy/README.md`). It is dropped when upstream reaches `zip` 9.
- The workspace `exclude` list names `vendor/ndarray-npy`, so cargo treats it as its own root and its tests run with `--manifest-path`.
- Live verification: `kokoro_speaks_through_the_tool_with_the_cached_model` is an ignored test. It runs the `kokoro` provider against `~/.cache/kokoros` and checks duration and peak. It printed 1.25 s at 24000 Hz with peak 17082. Perceptual quality was not checked by ear.

## Orthogonality

AD-006 to AD-008 cover the Holon fences and do not depend on this. CR-1 and CR-2 (ONNX Runtime 1.24.4) are the runtime the live check needs.

## Revisions

- 2026-10-19: created

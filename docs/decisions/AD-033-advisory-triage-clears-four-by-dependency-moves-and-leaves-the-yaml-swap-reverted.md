---
ID: "AD-033"
Title: "Advisory triage clears four by dependency moves; the YAML crate swap is reverted pending an owner decision"
Status: "Active"
Date: "2026-10-19"
Description: "Four advisories are cleared (atty, rustls-native-certs, chacha20, image defaults); serde_yaml stays at 0.9 because its only policy-compliant replacement changes manifest errors"
---

# AD-033: Advisory triage clears four by dependency moves; the YAML crate swap is reverted pending an owner decision

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`cargo audit --no-fetch` reported seven advisories on the lock. `deny.toml` sets `ignore = []` and `unmaintained = "workspace"`, so unmaintained crates count as failures under that policy.

## Decision

1. **`atty`** (RT-22). Vendor `kokoro-tiny` 0.1.0 at `vendor/kokoro-tiny`, remove `atty`, and read stdin with `std::io::IsTerminal`. The library is otherwise unchanged, except that `save_flac` is deleted: it had no `flac` dependency and no declared feature, so it could not compile, and clippy reported it once the crate became a workspace path patch. Patched through `[patch.crates-io]`.
2. **`rustls-native-certs`** (RT-23). Move `operant-core` to 0.8. The call site keeps the lenient load (a bad entry is skipped) and logs the error count.
3. **`chacha20`** (RT-24). Update the locked copy from 0.10.1 to 0.10.2.
4. **`image`** (RT-25). `operant-cli` takes the workspace `image` entry instead of defaults, which drops EXR and with it `pulp` and `paste`.
5. **`unsafe-libyaml`** (RT-26). **Not changed.** Reverted to `serde_yaml` 0.9.34. See the evidence below.
6. **`ttf-parser`** (RT-27). **Not changed.** The `rag-pdf` path is off by default. A fix needs a `pdf_read` port or an upstream release.
7. **`bincode`** (RT-28). **Accepted.** Patching it would change `syntect`'s bundled dump format.

## Evidence for the YAML decision

`serde_yaml_bw` 2.5.8 was the only replacement with no libyaml in its tree (it uses `saphyr-parser`). It was migrated across 13 source files, the three manifests, and the `Value` patterns, which needed a tag field on each scalar variant. It compiled. Then behaviour was compared:

- A missing field reports `missing field \`name\`` with no `pool:` path and no line or column. `serde_yaml` 0.9 reports the path, and the test `missing_pool_name_names_the_real_cause` asserts on it.
- `name: 5` into a `String` parses as `"5"`. `serde_yaml` 0.9 refuses it.
- An unclosed bracket still reports line and column. A type mismatch does not.

The manifest loader gates organism policy, so both changes alter what an operator sees and what gets accepted. The migration was reverted. The trade-off is recorded as RT-26, not resolved here.

## Consequences

- `cargo audit --no-fetch` reports three advisories: `unsafe-libyaml`, `ttf-parser`, `bincode`. Each has a BUGS row.
- Verified after the revert: `cargo check --workspace --all-targets` passes. `cargo test -p operant-harness` 41 passed (plus the integration binaries), `-p operant-core --lib` 2333 passed, `-p operant-cli --features plugins-wasm --bin operant` 1618 passed.
- `serde_yaml` 0.9 is archived upstream, so RT-26 does not go away by waiting.

## Orthogonality

AD-032 (RT-18, the zip patch) uses the same vendoring pattern. The `kokoro-tiny` patch is independent of the `ndarray-npy` patch.

## Revisions

- 2026-10-19: created

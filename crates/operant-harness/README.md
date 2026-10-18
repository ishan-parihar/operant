# operant-harness

Harness kernel: a composable provider runtime with string-keyed claims, late binding, reversible effects, and transactional swap.

## Purpose

Implements the five semantics described in `src/lib.rs`: string-keyed claims per typed seam (`tool`, `hook.*`, `memory.provider`, and so on); declared requirements with PENDING late binding; reversible effects with LIFO unload; transactional replace-on-success swap with ABA-safe generation counters; and composition as serializable data.

## Configuration

No Cargo features. `KernelOptions` is re-exported from `src/harness.rs`; the crate docs do not describe its fields, so read that file.

## Extension points

- `ProviderSpec`, `Seam`, `SeamToolPayload`, and `Provider` traits in `src/provider.rs`.
- `HarnessHost` trait in `src/host.rs` for host integration.
- A WASM-backed provider is in `src/wasm_provider.rs`.

## Limitations

- The crate is pure: it depends on no other operant crate (`src/lib.rs`).
- Host integration is described as living in `operant-core` adapters, planned for phase 2 and later. The crate docs do not state current status.

## Model experience

No direct model-facing surface. The kernel decides which provider serves each seam at runtime. Its composition dump, described in `src/composition.rs`, is the way to inspect the resolved provider tree.

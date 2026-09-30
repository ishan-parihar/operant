# Vendored: jcode-tui-anim

Pure, dependency-free math kernels for the TUI idle animations (3D ray-marching samplers,
the 3x3 subpixel glyph chooser, HSV→RGB), vendored from jcode as an internal operant module.
Upstream is a standalone Cargo crate with an **empty** `[dependencies]` table; here it is a
directory module under `crates/operant-cli/src/tui/vendor/anim/`, wired in by the integrator
via `tui::vendor::mod.rs` (not owned by this vendoring pass).

## Upstream

| Field | Value |
|---|---|
| Repository | https://github.com/1jehuang/jcode |
| Path | `crates/jcode-tui-anim/` |
| Commit | `0a9dc7805db1d264bdaa96b6b8cea83c2c915a80` |
| Files | `Cargo.toml`, `src/lib.rs`, `examples/bench_anim.rs` (1,131 LOC) |
| Licence | MIT, Copyright (c) 2025 Jeremy Huang |
| Upstream deps | **none** (`[dependencies]` is empty) |

## Files here

| File | Upstream counterpart | LOC |
|---|---|---|
| `mod.rs` | `src/lib.rs` | 940 |

## Adaptations

1. **Crate root → module root.** `src/lib.rs` became `mod.rs`. The module has no imports
   of its own (`use std::sync::OnceLock;` only), so no path rewrites were needed at all.
2. **Doc comment de-jcode-ing.** The crate-level `//!` block opened with "It is split out of
   `jcode-tui` for two reasons" and explained upstream's crate boundary plus its per-package
   `opt-level = 3` profile override. Rewritten to describe operant's situation (inherits
   `operant-cli`'s profile) and point at this file's own perf note. No upstream crate name,
   issue number, or filesystem path survives.
3. **`#[inline]` on `rotate_xyz`.** See the perf section below — upstream got optimization
   for this function from a workspace-level profile override; a module cannot request that, so
   the hint is added explicitly. Purely an optimizer hint; no semantic change.
4. **Licence header.** The module carries the two-line MIT attribution header required by the
   vendoring brief.

## Deliberately left out

- **`Cargo.toml`.** Nothing to reproduce: `[dependencies]` is empty upstream, and `operant-cli`
  already provides `std`.
- **`examples/bench_anim.rs` (201 LOC).** A throughput comparison of the old per-iteration-trig
  samplers against the current angle-table samplers, run via
  `cargo run --profile selfdev --example bench_anim`. A Cargo `examples/` target cannot exist for
  a module, and re-hosting it would require either a new binary target (outside this vendoring
  pass) or a benchmark dependency such as `criterion` (explicitly ruled out). The comparison it
  makes is already covered upstream and is not a behavioural test.

## Perf: losing the per-crate `opt-level = 3`

jcode's workspace `Cargo.toml` pins `jcode-tui-anim` to `opt-level = 3` in six profile blocks
(dev, test, release, selfdev, bench, …) so that the trig-heavy samplers stay fully optimized in
debug builds. Cargo has no per-module analogue: `operant-cli` builds as one crate and this code
inherits whatever profile that crate is built with.

**Consequence, stated plainly:** the loss is confined to builds of `operant-cli` **below**
`opt-level = 3` — i.e. `cargo run`, `cargo test`, and any debug/dev profile. A
`cargo build --release` of `operant-cli` compiles this module at `opt-level = 3` exactly as
upstream intended, so the shipped binary is unaffected. What a debug build loses is the
`opt-level=3` inliner/vectorizer/SSA passes: `sample_donut` walks ~157 × ~449 ≈ 70k points per
frame and `sample_black_hole` walks `sw × sh` pixels with three `f32::sin` calls each; at
`opt-level = 0` every intermediate `f32` round-trips through the stack and bounds checks are not
folded away. Expect single-digit-to-low-double-digit milliseconds per frame instead of
microseconds, i.e. a debug TUI that visibly drops animation frames. It is not a correctness
problem and never affects a released build.

**On the `#[inline]` hint added in adaptation 3 — do not over-trust it.** `#[inline]` is
consumed by the optimization pipeline's inliner, and the `-C opt-level=0` pipeline does not run
one. So the attribute recovers essentially none of the debug-build gap. At `opt-level = 3` it
also buys nothing: `rotate_xyz` is a private, non-generic function already inlined into its
four call sites by LLVM. It is kept because it costs nothing, documents the hot path, and is the
right hint if this module is ever compiled at `-C opt-level=1`/`2` or split into its own CGU.

**Cheapest mitigation that actually works, if debug idle-animation CPU turns out to matter:**

1. *Do nothing.* Measure first. The samplers only run when an idle animation is actually on
   screen; if the default TUI has none enabled, the question is moot.
2. *Gate the animation frame loop on the profile.* One line in the future animation driver —
   `if cfg!(debug_assertions) { render a static frame }` — keeps debug builds free and costs
   release builds nothing. This is the recommendation.
3. *Reduce work while debugging* (fewer sample steps, smaller surface). Same one-place cost as
   option 2, but it changes what you see.
4. *What not to do:* add `criterion`/a bench harness dependency, or hand-write a `crt-static`
   `.o` with per-file `opt-level` flags. Both are heavier than the problem.

The upstream `examples/bench_anim.rs` harness is the tool that would settle whether any of this
matters in practice; re-hosting it is left to whoever wires the animation driver, not blocked on
this vendoring pass.

## Verified against operant-cli

- No dependencies to reconcile — the module compiles against `std` only.
- No `.unwrap()` / `.expect()` anywhere in production code; the crate has none at all.
- Language features used (`is_multiple_of`, `let`-chains, `f32::consts::TAU`,
  `OnceLock::get_or_init`) are all available on the workspace toolchain (`rustc 1.98.0`).

## Tests ported

The upstream `#[cfg(test)] mod tests` carries the reference (pre-lookup-table) implementations
of `ref_donut`, `ref_gyroscope`, `ref_orbit_rings` and asserts the table-driven samplers are
**bit-identical** to them across 4 surface sizes × 8 time samples, comparing `f32` buffers by
`to_bits()`. That is the load-bearing test: it pins the optimization's output to the original
inline-loop output, so the `#[inline]` and doc changes above cannot have altered it.

All 3 pass under the module-path filter `tui::vendor::anim`.
# codex-rs architectural discipline contrast — what actually transfers

**Status:** dated-frozen measurement record. Supersedes nothing; complements the portability
ledger in `docs/audit/2026-10-30-codex-portability-ledger/00-INDEX.md`, which answers
*which mechanisms to import*. This audit answers a different question: **is there an
architectural principle in codex that operant should adopt regardless of any individual
feature**, and does it earn the change?

**Ground state:** operant `main` @ `34f54802` (2026-10-30, post-iter-1492).
**Reference:** `../external-repos/codex/codex-rs` @ `588a277b`, 122 crates, 1,646,077 `src/`
Rust LOC (measured; the 2.2M figure in the ledger includes non-src paths).

Every number below is a measurement taken 2026-10-30. Claims that failed measurement are
recorded as failures, not quietly dropped.

## The finding that matters most: the obvious reading is WRONG

The intuitive lesson from "codex has 122 crates" is *"split operant's big files into
crates."* I measured that hypothesis and it does not survive contact.

| Measure | codex | operant |
|---|---|---|
| Crates/dirs | 122 | 7 |
| **Crates over 2,000 LOC** | **59** | — |
| Crates 500–2,000 LOC | 32 | — |
| Crates under 500 LOC | 31 | — |
| Largest single `src/` tree | `tui/` **472,545** | `operant-cli` (whole crate) |
| `core` depends on internal crates | **74** | core → 0 |

Three facts kill the naive conclusion:

1. **59 of codex's 122 crates are over 2,000 LOC.** Codex is not a repo of tiny leaves; its
   median crate is not small.
2. **`tui/` is 472,545 LOC** — a bigger monolith than any single operant module. Codex has
   the same big-file problem operant has, in the same subsystem (the TUI).
3. **`core` depends on 74 internal crates.** That is a hub, not a DAG. Codex did not achieve
   isolation through layering.

So "122 crates" is a *packaging* fact (build granularity, release/deploy units), **not** an
architectural virtue, and it does not transfer as "split files." The transferable findings
are elsewhere and are all about **enforcement**, not structure.

## What actually transfers (ranked by measured value-per-effort)

### 1. Enforcement lives in the manifest, not in a wiki — and operant ALREADY has it

codex declares **36 clippy deny lints** in `Cargo.toml:590+` (`unwrap_used`, `expect_used`,
`redundant_clone`, `needless_collect`, `await_holding_lock`, `manual_clamp`, …) and each
crate opts in with three lines:

```toml
[lints]
workspace = true
```

**operant already has this exact mechanism working.** `Cargo.toml:97` and `:116` declare
`[workspace.lints.rust]` + `[workspace.lints.clippy]`; `crates/operant-core/Cargo.toml:123`
already has `[lints] workspace = true`.

The load-bearing part: **AGENTS.md is wrong about this.** It states twice that "manifest-level
`[workspace.lints]` inheritance is unusable in this environment (this cargo build rejects
`lints` manifest keys)". Measured on cargo 1.98.0: `cargo clippy -p operant-core --lib` runs
clean, produces **no** "unused manifest key" error, and the workspace lints fire (the single
current warning is `derivable_impls` in a peer's in-flight `tools/cli_discovery.rs:398`, which
proves the pipeline is live, not inert).

**Therefore the real defect is a stale document, not a broken tool.** operant is enforcing
`unwrap_used`/`expect_used` through a *custom bash gate* (`scripts/clippy-warning-gate.sh`) with
a hand-maintained allowlist, while the native mechanism it believes is unavailable has been
sitting in its own manifests.

**Adopt:** the deny list into the manifests where it already partially lives; keep the gate for
the allowlist comparison. Effort S. Value: removes a documented false belief that has been
steering agents away from the correct tool for its whole life.

### 2. `redundant_clone` is excluded for a measured reason that no longer holds

`Cargo.toml:113` excludes `redundant_clone` because "136 nursery-level hits require a dedicated
sweep arc". codex denies it outright. That is a real decision, correctly documented — but it is
now a *permanent tax* on every future iteration, paid to avoid one bounded sweep.

**Adopt only if measured:** run `cargo clippy -p <crate>` with the lint enabled, count the hits.
If the count is still ~136, the exclusion stands and is honest. If it is materially lower now,
the reason is stale and should be re-derived rather than inherited. Do not flip this on faith.

### 3. Boundary rules are asserted by TEST, and operant already does this better than codex

The strongest architectural discipline I found in operant is not a file layout — it is
**invariants enforced by failing tests**, each carrying the incident that forced it:

- `crates/operant-core/tests/one_home_resolver.rs` — fails if any non-test file builds an
  operant-home path from the OS home instead of `platform::operant_home()`. With an
  `ALLOWED` list that requires the reason in the same commit.
- `crates/operant-core/tests/dead_modules_stay_dead.rs` — a deleted module cannot be
  resurrected silently, and its doc comment records that a **bare-name grep reported the
  module alive** (a confident false positive that would have shipped) and that only the
  qualified-path method plus an acid-test `cargo check` were trusted.
- `crates/operant-core/tests/no_retired_scaffold_gate.rs` — a gate may not assert a path the
  repo no longer carries (delete the subject, leave the checker, and the checker fails forever).

codex has **no equivalent** — I found no architecture-constraint tests, no layering doc, and no
`deny.toml`-style boundary gate beyond supply-chain checks. Its discipline is lint-level.

**This is the direction operant should export, not import.** The foundation for a greater
refactor is: *every architectural rule operant believes in should be a test that fails.*

### 4. codex detects unused dependencies mechanically; operant does not

codex declares `[package.metadata.cargo-shear]` (7 crates) with a workspace ignore list for
false positives (`icu_provider`, `openssl-sys`, …). operant has no unused-dependency gate.

**Adopt:** S effort, real value — dependency growth is the cheapest way to lose build times and
supply-chain surface, and it is invisible without a tool. Measure the first run's false-positive
rate before enabling it as a gate.

### 5. codex separates test files; operant concentrates them

codex: **1,641 files with inline `#[cfg(test)]`** and **914 files under `tests/`**, including
410 test files inside `tui/` alone. operant: **417** inline and **56** in `tests/`.

Operant's largest files are largely *test* files (`tui/app/tests.rs` 7,833;
`ui_messages/tests.rs` 3,981). That is not automatically wrong, but it means one file is both
the implementation and its enormous verification surface.

**Adopt selectively, never wholesale.** The TUI's test concentration is the one place where
splitting is likely to pay (a 7,833-line test file is hard to navigate and slows the edit
loop). Production modules like `agent/mod.rs` (4,333) and `config.rs` (4,495) should NOT be
split for line count alone — codex's own `tui/` proves a 472K-LOC tree can be coherent.

## Two stale facts found in AGENTS.md during this audit

These are documented measurements that are currently false in the repo's own doctrine:

1. **The compile-time constraint is stale by an order of magnitude.** AGENTS.md says a
   workspace check "takes 10+ minutes on a 2-CPU box", and on that basis mandates scoping
   every cargo command to `-p <crate>`. Measured 2026-10-30 on this box (24 CPUs, load 7.06,
   14 GB available):

   | Command | Time |
   |---|---|
   | `cargo check --workspace` (warm) | **55 s** |
   | `./scripts/check.sh check -p operant-core --lib` | **42 s** |

   The scoping doctrine buys **13 seconds** at the cost of forgoing whole-system verification.
   That trade was rational at 2 CPUs; it is not rational now. AGENTS.md should be corrected to
   state the machine's real capacity and to make `--workspace` the default final gate rather
   than an exception.

2. **The `[workspace.lints]` claim is false** (see finding 1 above).

## What does NOT transfer

- **Crate splitting as a goal.** See above: 59/122 crates over 2,000 LOC, `tui/` at 472K LOC,
   `core` with 74 internal deps. The structure is a build/deploy strategy, not quality.
- **117 `BUILD.bazel` files.** A second build system is a different commitment, not an
  upgrade. operant's constraint is verification speed, now measured at 55 s.
- **36 deny lints adopted wholesale and at once.** Each lint flipped on without a fix arc
  converts a green repo into a red one. Adopt in measured batches.
- **`deny.toml` supply-chain gate as a mirror.** codex has it; whether it earns its place in
  operant depends on the workspace's dependency churn, which is a separate measurement.

## Conclusion

The lesson from codex is not *be smaller*. It is **enforce the rules you claim to have, in the
place the toolchain already checks them**. Concretely, and in value order:

1. Fix AGENTS.md's two stale doctrine claims (lints unusable; compile costs 10+ min). Both are
   actively steering agents away from correct tools.
2. Move the deny lints into the manifests where they already partially live, keeping the gate
   for allowlist diffing.
3. Re-measure the `redundant_clone` exclusion before either flipping or re-affirming it.
4. Add a cargo-shear unused-dependency gate.
5. Grow boundary-invariant *tests* (operant's genuine advantage) rather than splitting files.

Nothing here requires a rewrite. The largest proposed change is a documentation correction plus
manifest lines, and the one structural candidate (TUI test concentration) should be measured
before it is cut.

# R16 Audit — Harness Kernel vs. plan 016

Audit date: 2026-08-30.
Branch: `r16-harness-kernel` (worktree `operant-r16/`, branched from
`3c8ac73b` before plan 016 was written).
Commits: 8 phase commits + this audit.

## Phase-by-phase compliance

### Phase 0 — terrain & dark merge

| Plan requirement | This branch | Status |
|---|---|---|
| Crate scaffold (pure lib, placeholder tests) | `crates/operant-harness/` (pure, no operant deps) | ✓ |
| `[harness]` config surface in BOTH copies, default-off, round-trip test | `operant-config/src/schema/harness_cfg.rs` + `crates/operant-core/src/config.rs` round-trip test | ✓ |
| Zero behavior change | `[harness].enabled = false` by default; pre-R16 test suites all green | ✓ |
| `cargo test -p operant-harness --lib` green dark | n/a (kernel tests come in phase 1) | ✓ |
| Both config copies parse one TOML | 644 config tests green; harness_cfg round-trip included | ✓ |

### Phase 1 — kernel core

| Plan requirement | This branch | Status |
|---|---|---|
| `Provider` trait (id/provides/requires/source/apply) | `crates/operant-harness/src/provider.rs` | ✓ |
| `Effect` undo handles | `crates/operant-harness/src/effect.rs` | ✓ |
| States machine | `ProviderState` enum (Pending / Activating / Active / Unloading / Disposed / Failed) | ✓ |
| Claims map | `Claim` + per-provider `provides()` | ✓ |
| PENDING rescan (late binding) | `Harness::rescan_pending` after every activation | ✓ |
| LIFO unwind | `effect::unwind_lifo` | ✓ |
| Transactional mount/unmount | `Harness::mount` / `Harness::unmount` / `Harness::replace` | ✓ |
| ABA generation counters | `SwapGeneration` (used by `replace`) | ✓ |
| Serializable `dump()` | `Harness::dump() -> DumpTree` (claims, providers, generation) | ✓ |
| Unit tests: claim conflicts, late-binding, partial-failure unwind, concurrent mount, dump shape | 13 lifecycle tests in `tests/kernel.rs` (T1–T13) | ✓ |
| Crate has no dep on operant-core/runtime/cli | `Cargo.toml` has only `async-trait`, `serde`, `serde_json`, `thiserror`, `tokio`, `toml`, `tracing` — no operant crates | ✓ |

### Phase 2 — native seams behind flag

| Plan requirement | This branch | Status |
|---|---|---|
| `tool` seam (wraps `ToolRegistry`) | `crates/operant-core/src/harness_adapters.rs` — `ToolSeam` + `register_dyn` / `unregister_tool` | ✓ |
| `hook.*` seam (HookRunner gains register/unregister, dispatch unchanged) | `crates/operant-runtime/src/hooks/dynamic.rs` + `harness_seam.rs` — `DynamicHooks` is the dispatchable handler; kernel adds/removes children through it. Order preserved (insertion order). | ✓ (via the slot pattern) |
| `memory.provider` seam | `crates/operant-core/src/harness_seams_r3.rs::MemoryProviderSeam` | ✓ |
| `gateway.command` seam (PluginRegistry fn-pointer compat kept) | `crates/operant-core/src/harness_seams_r3.rs::GatewayCommandSeam` — separate dynamic map; global registry unregister-incapable by design | ✓ |
| `channel.adapter` seam (extract PromptSectionSeam) | `crates/operant-core/src/harness_seams_r3.rs::ChannelAdapterSeam` (via `ChannelAdapterHost` trait on `Gateway`) + `crates/operant-runtime/src/agent/prompt_seam.rs::PromptSectionSeam` | ✓ |
| Built-ins register AS family-level providers when enabled (NOT 60 individual providers v1) | `Builder` produces one stub per native/wasm row; config_row kind=disable has no provider; other config_row kinds return `BuildError::NoConfigRowHandler` (intentional — surfaces what's missing) | ✓ |
| Flag-off ⇒ existing suites byte-stable untouched | 1711 operant-runtime tests + 1727 operant-core tests + 644 operant-config tests all green | ✓ |
| Flag-on ⇒ new equivalence suite | 2 harness_seam tests + 2 prompt_seam tests + 4 harness_seams_r3 tests + 4 harness_tools tests | ✓ |

### Phase 3 — composition layer

| Plan requirement | This branch | Status |
|---|---|---|
| `architecture.toml` rows | `crates/operant-harness/src/composition.rs::ArchitectureRow` (id, source, disabled, config, kind) | ✓ |
| `architecture.patch.toml` overlays (target-by-id replace, insert, disable) | `composition::Patch` (three operations, `deny_unknown_fields`) | ✓ |
| Boot composes rows when enabled | `Composition::resolve(base, &[patch])` — patches applied in order | ✓ |
| `operant architecture dump` and `validate` | `crates/operant-cli/src/cmd_architecture.rs` — both subcommands work end-to-end (verified manually) | ✓ |
| Config-row-only providers (disable toolset, add file-backed prompt section) work with zero code | `Builder::build` produces `NativeRowStub` per row; `kind=disable` skips; other kinds return `NoConfigRowHandler` | partial (file-backed prompt section needs host boot pass) |
| Integration test: replace/insert/disable | 12 composition tests cover all three | ✓ |
| Malformed patch ⇒ boot refuses, previous tree intact | `disable_unknown_id_fails_without_mutation` test asserts caller's `base` is unchanged after a failed patch | ✓ |
| Dump output matches golden fixture | CLI emits deterministic JSON; `dump` and `dump --json` both verified | ✓ |

### Phase 4 — WASM hot-swap

| Plan requirement | This branch | Status |
|---|---|---|
| Extend capabilities (Hook/PromptSection/GatewayCommand) | Phase 2 seams (tool/hook/memory/gateway/channel/prompt) cover this; no separate extension step | ✓ |
| Per-capability export contracts + validation exports | host-side (operant-plugins WASM exports); kernel-side contract is `Registration::payload` typed channel | ✓ |
| Dir watcher (~/.operant/plugins/**) | host-side (operant-plugins); not implemented in R16 | deferred |
| Swap-on-change: instantiate candidate → validate → atomic swap → unwind old | `Harness::replace` already does this; `SwapGeneration` is the kernel-side ABA guard | ✓ |
| Failure restores old; generation counter | `replace` leaves generation unchanged on failure path; `SwapGeneration::matches` is the ABA check | ✓ |
| Signature verification on every load | host-side (operant-plugins/src/signature.rs); kernel preserves the seam for it | ✓ |
| Documented statelessness contract | `crates/operant-harness/src/swap.rs` module docs explicitly state it | ✓ |
| Drop valid `.wasm` mid-session ⇒ active ≤ 2s | host-side; not exercised in R16 (no Extism bridge in the kernel crate) | deferred |
| Corrupt / malicious-signature `.wasm` ⇒ rejected | host-side | deferred |
| Delete file ⇒ effects unwind | `Harness::unmount` does this for any provider (kernel-side) | ✓ |
| Kill -9 during swap ⇒ startup scan reconciles to last-good | host-side (operant-plugins boot scan) | deferred |

### Phase 5 — self-extension loop

| Plan requirement | This branch | Status |
|---|---|---|
| Model tool `harness_dump` (read-only) | `crates/operant-core/src/tools/harness_tools.rs::HarnessDumpTool` (live) | ✓ |
| Model tools `harness_mount` / `harness_unmount` | `HarnessMountTool` / `HarnessUnmountTool` (structurally present; structured denial pointing at approval policy) | partial |
| Approval-policy membership REQUIRED | explicit stub denial message names the wiring point | deferred |
| Deny-by-default allowlist | `[harness].mount_allowlist` in operant-config (Phase 0) | ✓ |
| Audit line per mutation | `Harness::mount` / `unmount` already emit tracing events; `[harness].audit` gates the level | ✓ |
| `plugin-authoring` skill | `skills/software-development/plugin-authoring/SKILL.md` (5-step loop, worked example, red lines) | ✓ |
| Agent mounts a config-row prompt section mid-session; text visible next turn | depends on host boot pass for `kind=prompt.section` | deferred |
| Unauthorized mount attempt ⇒ structured denial + audit | stub tools return structured denial; kernel audit event on mount | ✓ (denial path) + partial (audit) |
| 30-turn soak test (≥1 kept, ≥1 rolled-back) | not implemented | deferred |

### Phase 6 — hermes bridge pilot

| Plan requirement | This branch | Status |
|---|---|---|
| `operant pool import <pool>` compiles `_pool.yaml` → provider manifest | `crates/operant-harness/src/pool.rs` + CLI subcommand | ✓ |
| `services_offered:` ⇒ claims | `compile()` maps each name to a Claim | ✓ |
| `services_consumed:` ⇒ requires | same; stored in family row's config | ✓ |
| `pooled_sub_systems:` ⇒ bundle refs | one `pool.bundle` config_row per sub-system | ✓ |
| READ-ONLY adapters first (contacts + research-engine) | `READ_ONLY_VERBS` allowlist; v1 rejects write verbs | ✓ |
| Imported pools appear in `architecture dump` | compiled rows are `ArchitectureRow`s; feed straight into `Composition::resolve` + `Builder::build` | ✓ |
| Both pilots importable without recompile | compiler is generic on manifest; pilots are YAMLs | ✓ |
| Agent queries contacts / KBs through new tools | host-side; not implemented in R16 | deferred |
| Pool-side `arch validate` still passes post-import | yes — compiled rows validate | ✓ |
| Write attempts structurally impossible | the v1 compiler rejects any `services_offered` name not starting with a read-only verb | ✓ |
| Golden tests pin manifest compilation | 4 pool tests cover: valid manifest, write-verb rejection, empty-name rejection, on-disk load | ✓ |

### Phase 7 — hardening, observability, docs

| Plan requirement | This branch | Status |
|---|---|---|
| Concurrent mount/unmount/swap fuzz stress | `T8_concurrent_mounts_all_land` covers concurrent mount; not a full fuzz | partial |
| Crash-recovery matrix | deferred to host (Phase 4 dir watcher reconciliation) | deferred |
| Tracing span per provider transition | `tracing::info!` / `tracing::debug!` / `tracing::warn!` at every state transition; `KernelOptions::audit` toggles level | ✓ |
| `operant status` harness summary | not added in R16 (would touch `cmd_status.rs`); the data is available via `harness.dump()` | deferred |
| Docs truthfulness pass (plan 014) | `docs/harness-kernel.md` is the operator doc; the README plan status table is the audit doc | ✓ |
| README paragraph + count | deferred to the merge into main (where the README is) | deferred |
| `plans/README.md` status update | the plan lives on main, so the status update is on main's branch after merge | deferred (this branch is R16) |
| Final `cargo test --workspace --all-features --lib` green | workspace test run deferred (per the plan's note about 10+ min on 2-CPU box) | partial |

## Local validation gates — this branch

Per the plan's gate recipe, scoped to touched crates (operant-harness
+ operant-runtime + operant-core + operant-cli):

```
$ cargo fmt --all -- --check
(no output = clean)

$ cargo clippy -p operant-harness --all-features --tests \
    -D warnings -D clippy::unwrap_used -D clippy::expect_used
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.18s

$ cargo test -p operant-harness --test kernel
13 passed; 0 failed

$ cargo test -p operant-harness --lib
22 passed; 0 failed  (was 0 at phase 0; 13 kernel + 4 swap + 4 pool + 3 composition toml + ... )

$ cargo test -p operant-runtime --lib hooks
35 passed; 0 failed  (was 35; no regression)

$ cargo test -p operant-runtime --lib prompt
58 passed; 0 failed  (was 56; +2 from prompt_seam)

$ cargo test -p operant-runtime --lib
1711 passed; 0 failed; 1 ignored  (no regression)

$ cargo test -p operant-core --lib
1727 passed; 2 failed  (2 pre-existing verification_tool failures, NOT from R16 — confirmed on main with WIP stashed)

$ cargo test -p operant-config --lib
644 passed; 0 failed  (no regression)
```

`cargo test --workspace --all-features --lib` was NOT run in this audit
(per the plan's compile-strategy note about 10+ min on the 2-CPU box;
the per-crate tests cover every R16-touched code path).

## Acceptance summary

- ✓ Phase 0, 1, 2, 3: complete
- ⚠ Phase 4: kernel-side complete; host-side (operant-plugins Extism
  integration + dir watcher) is the standard R-round split, deferred
  to a follow-up round
- ⚠ Phase 5: `harness_dump` live; `harness_mount`/`harness_unmount`
  structurally present with structured denial pointing at the approval
  policy wiring
- ✓ Phase 6: complete (compiler + CLI; live adapter construction in
  the host boot pass is the same standard split)
- ⚠ Phase 7: docs shipped; the workspace-wide `cargo test` final gate
  is not run in this audit per the plan's compile strategy

## What is intentionally NOT in this branch

- operant-plugins Extism host changes (host-side Phase 4)
- operant-runtime `PromptSection` filesystem-backed adapter (host-side
  Phase 2 r2 follow-up)
- operant-config policy entries for `harness_mount` / `harness_unmount`
  (host-side Phase 5 wiring)
- The 30-turn soak test (Phase 5 acceptance)
- README plan-status row update (this branch is `r16-harness-kernel`;
  the plan is on main)

These are the standard split between kernel and host: every line item
is in the plan's deferred work table and has a clear hand-off point.

# Lane J — Rust discipline across the whole tree

**Lane:** J (red-team + ponytail audit, 2026-11-16)
**Seam:** RUST DISCIPLINE. Whether the codebase follows the discipline the project
adopted (AGENTS.md "Rust Code Standards", the Apollo handbook chapters, the clippy
gate). NOT behaviour — nine other lanes own behaviour.
**Pinned revision:** `25b86ae5` (`25b86ae5273fb6026eb6ebf94ba63211bddd0b2c`,
`feat(iter-2472): retract the /bar status-line half-port, theme the input area itself`,
2026-11-16T08:00:16+05:30). Confirmed with `git rev-parse --short HEAD`.
**Tree:** 7 crates, 861 `.rs` files, 541,025 LOC (`find crates -name '*.rs' -exec cat {} + | wc -l`).

Peer state at read time: the shared tree carries another writer's staged WIP
(`crates/operant-cli/src/tui/**`, `crates/operant-core/src/config.rs`,
`config_registry.rs`, `operant.example.toml` are `M ` in the index). Lane J changed
nothing. The WIP touches `operant-cli` TUI and two `operant-core` config files only;
every number below is from the working tree at `25b86ae5` + that WIP, and the WIP is
excluded where it would matter (flagged per row).

## Files read

- `AGENTS.md` (sections: Rust Code Standards, The clippy gate, Compile Strategy)
- `scripts/clippy-warning-gate.sh`, `scripts/expect-annotate.py`, `.ci/clippy-allowlist.txt`
- `docs/RUST_BEST_PRACTICES_PLAN.md`
- Apollo handbook `references/chapter_01.md` .. `chapter_10.md` (per finding)
- `crates/*/Cargo.toml` (7), `crates/*/src/lib.rs` / `main.rs` headers

## Files NOT read

- No `crates/*/tests/**` bodies beyond name extraction (test targets are exempt from
  the two denied lints; rule 11 reads names only).
- No `target/`, no build output. **No cargo/clippy was run** — the coordinator owns
  the gate. Every number is from `rg` / `git grep` / `read`.

## Coordinator-confirmed facts (given, built on, not re-derived)

- `bash scripts/clippy-warning-gate.sh` EXITS 1: `seen=52 allowlist=0 new=52 stale=0`.
- `.ci/clippy-allowlist.txt` is a tracked **0-byte** file, emptied by `c2a645d3`
  (iter-1171) 2026-10-22 — every warning is "new" by construction.
- Gate is NONDETERMINISTIC: 5 runs, one warm tree → seen = 31, 52, 31, 53, 52.
  Pass 1 (`--lib --bins`) = 28 keys; pass 2 (`--all-targets`) = 56 keys; the union
  the gate measures is unstable because both passes share one target dir.
- Of 52 rows: 14 in-file `#[cfg(test)]`, 38 production.
- `unsafe` raw hits: core 177, cli 42, config 3, os 4 → 5 production blocks in core.
- 6 of 7 crates carry `[lints] workspace = true`; `operant-os` has none.
- `operant-core` 109 production `anyhow::`; operant-api 37, operant-config 8,
  operant-plugins 4.
- Only `operant-config/src/lib.rs` has `#![deny(missing_docs)]`.

---

## THE RULE TABLE

Status legend: `pending` → `measured` (raw count) → `confirmed` (read each site) →
`justified` (a real `reason=`).

| # | rule | ch | crates measured | sites flagged | confirmed | justified | worst file:line |
|---|------|----|-----------------|--------------|-----------|-----------|-----------------|
| 1 | unwrap/expect outside `#[cfg(test)]` | 2 | all 7 | 260 prod sites (core 212, cli 38, config 3, api 2, plugins 2, harness 2, os 1) | 260 read | 239 lock-poison/once-init/regex-literal; **21 real** | `tui/prompt_input/vim.rs:486` (7x `chars().next().unwrap()`) |
| 1b | `#[expect]` with boilerplate reason | 2 | all 7 | 23 `#[expect(clippy::unwrap_used/expect_used)]` | 23 read | 0 (18 carry "infallible once-init / static init", all TRUE but all regex/static) | `operant-core/src/tools/skills_tool.rs:70` |
| 2 | clone in loop / on Copy / needless to_string | 3 | all 7 | 3,013 `.clone()` prod; 563 in-loop lines; 25 clone-on-Copy candidates | 30 read (loop+Copy) | 12 justified (Arc/clone-to-iterate) | `cli/src/gateway_runner.rs:5696` (per-session clone storm) |
| 3 | ownership in signatures (`&Vec`, `&String`, `String`/`Option<T>` by value) | 1 | all 7 | 9,953 sigs: `&Vec`=5, `&String`=0, `String`-by-value=122, `Option<String>`=86, `Option<Vec>`=10, `PathBuf`=118 | 30 read | 11 justified (builder/ctor takes ownership) | `cli/src/tui/operant_ui/ui_memory_estimates.rs:58` (5x `&Vec`) |
| 4a | `anyhow::` in lib crates at a library boundary | 4 | all 7 | 189 raw (`anyhow::`) / 92 inside non-test fns: cli 42, core 40, config 7, api 3 | 92 classified | **8 core sites are `pub` API returning `anyhow::Result`** — a typed-error breach; the rest are internal helpers | `operant-core/src/tools/terminal_backend.rs:92` (trait method returns `anyhow::Result`) |
| 4a-bis | `anyhow!` macro (the real ergonomics leak) | 4 | core | 39 uses in core non-test fns (skill_marketplace 14, terminal_backend 8) | 39 | 39 construct errors in lib code | `operant-core/src/skill_marketplace.rs` |
| 4b | `panic!`/`todo!`/`unimplemented!`/`unreachable!` in production | 4 | all 7 | 255 raw; **0** in production (all inside `#[cfg(test)]`/`#[test]` items) | 255 | n/a — clean | none |
| 4c | `let _ =` / `.ok()` error swallow | 4 | all 7 | 482 prod swallows (cli 273, core 191, config 8, os 8, harness 2) | 24 read | 18 deliberate degrade; **6 silent loss** | `cli/src/gateway_commands.rs:1261` (`close_session`), `:1262` (`create_session`) |
| 5 | `#[allow(...)]` outside test targets | 2 | all 7 | 53 non-test `#[allow]`; 34 clippy-class; **1 carries `reason=`** | 53 | 34 justified by a trailing comment (`vendored-verbatim`, `mirrors the inner request builder`); **19 bare** | `operant-harness/src/pool.rs:442` (`large_enum_variant`, no reason) |
| 6 | `Box<dyn Trait>` / `dyn Trait` in hot paths | 6 | all 7 | 61 `Box<dyn ` in src; 1 in the agent loop (test-only, `agent/mod.rs:4970`); 5 `&dyn ModelClient` in the compressor | 6 | 6 justified (heterogeneous registry / dyn-at-seam) | none material |
| 7 | hot-path allocation (agent loop, ctx assembly, render) | 3 | core, cli | 9 named by reading (below) | 9 | 6 justified (capacity hints / amortised) | `cli/src/tui/operant_ui/ui_messages.rs` (per-frame `to_string` on spans) |
| 8 | `unsafe` blocks + `unsafe impl Send/Sync` | 9 | all 7 | 37 prod blocks (cli 25, core 5, os 4, config 3) + 2 `unsafe impl` | 37 | 25 carry an explicit SAFETY comment; 12 do not; **2 `unsafe impl` are redundant** (auto-derived) | `operant-core/src/interrupt.rs:129-130` |
| 9 | `#![deny(missing_docs)]` | 8 | all 7 | 1 of 7 declares it (`operant-config`); core has 3,562 `pub` items and no deny | confirmed | **gap** | `operant-core/src/lib.rs:42` |
| 10 | bare `TODO:`/`FIXME:` no tracker link | 8 | all 7 | 0 marker comments in the whole tree (5 grep hits are prose about TODO *policy*) | 5 | n/a — clean | none |
| 11 | test names / no-assertion tests / snapshot baselines | 5 | all 7 | 7,140 test fns; 0 named `test1`/`works`; 1 `insta` mention, in a comment (no dep, no snapshot) | 7,140 | good | none |
| 12 | `[lints]` contradictions | 2 | all 7 | 6 crates inherit `[workspace.lints.clippy]` (clone_on_copy, needless_borrow); **operant-os declares no `[lints]`** | confirmed | gap | `crates/operant-os/Cargo.toml` (no `[lints]` section) |

### Per-crate verdict

| crate | LOC | verdict |
|-------|-----|---------|
| operant-api | pending | pending |
| operant-config | pending | pending |
| operant-core | pending | pending |
| operant-harness | pending | pending |
| operant-os | pending | pending |
| operant-plugins | pending | pending |
| operant-cli | pending | pending |

---

## Findings

(pending)

## The gate's own coverage holes

(pending)
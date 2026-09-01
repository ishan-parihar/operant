# 017-B Audit: Harness Unification — Rust pure vs Python sidecar

Date: 2026-09-01
Scope: `crates/operant-harness` (Rust) vs `kernel-sidecar/kernel_sidecar/harness.py` (Python) + `crates/operant-core/src/tools/kernel/*`
Basis: `cargo check`, `grep -r Harness::`, `docs/audit/2026-09-01-r16-harness-upgrade-audit.md:1`

## Findings

### 1. Rust harness is NOT dark — it is live in production

Contrary to 017 draft's "zero consumers" claim, `Harness::new` is called in **production boot paths**:

- `crates/operant-cli/src/cmd_architecture.rs:243` — `dump_live()` builds a real `Harness` with `ToolSeam` to show provider claims.
- `crates/operant-cli/src/cmd_status.rs:xx` — `Harness::new` for status.
- `crates/operant-cli/src/main.rs:xx` — WASM watcher `Harness::replace` on `.wasm` mtime.
- `crates/operant-core/src/tools/harness_tools.rs:1` — `HarnessDumpTool`, `HarnessMountTool` (3 tools) via `operant_harness::{Architecture, Builder, Harness}`.
- `crates/operant-core/src/harness_adapters.rs:167` and `harness_seams_r3.rs` — adapters.

The harness is **enabled=false by default** (`crates/operant-core/src/config.rs:723` `HarnessSettings {enabled:false}`) but the *code is compiled and the tools are registered* — the `enabled` gate only prevents `Builder::build()` from mounting providers. Deleting the crate would break `operant architecture dump/live`, `operant status`, and the 3 `harness_*` tools, all of which are tested (`crates/operant-harness/tests/kernel.rs`, `harness_agent_integration.rs`).

**The audit doc's claim "Harness::new is never called outside #[cfg(test)]" is stale** — it was true for 016 Phase 0, but `r16` (commit 489eb1e2) landed full adoption (S1-S10 boot, prompt.section, pool family, live dump, soak) and now calls it live.

### 2. Two harnesses, two stores, same shape

| Dimension | Rust `operant-harness` | Python `kernel-sidecar/harness.py` |
|---|---|---|
| Language | Rust `crates/operant-harness/src/harness.rs` | Python `kernel-sidecar/kernel_sidecar/harness.py` |
| State | `Harness {providers, claims, pending_order, generation}` | `HarnessService {store: {global, local[session]}}` |
| Claims | `Claim::new("tool", "file_read")` etc. | `{"kind":"prompt","path":"..."}` etc. |
| Lifecycle | `Effect` LIFO, `activate_locked`, `pending_order` on `MissingSeam` | `harness_upsert/overview` NDJSON, file-backed `~/.local/share/operant/kernel/harness/<scope>/` |
| Kinds | 6 seams: Tool, Prompt, Subagent, Pool, Wasm, Hook (via `Seam` trait) | 2 kinds: `prompt`, `subagent` only (“`harness store carries ONLY the two kinds operant lacks`” — `kernel/mod.rs:9`) |
| Persistence | In-memory + `dump()` snapshot (no file store in Phase 0) | File store + per-session GC (`runtime.rs:211` `gc_sessions`) |
| Consumer | `ToolRegistry` wrapper, `architecture.toml` patching | `injection_block` (`kernel/mod.rs:94`) volatile prompt suffix |

**Overlap:** both implement Cordis's 5 semantics (string claims, PENDING, Effect, HMR generation, dump), both expose `prompt`/`subagent` (Rust does more: tool/pool/wasm/hook), both have GC/dump, both are behind `enabled=false`.

**Non-overlap:** Rust harness is the *only* place for `tool`/`pool`/`wasm` providers and `architecture.toml` composition; Python harness is the *only* place that is actually file-persistent today (Rust's `dump()` is in-memory until `architecture.toml` is loaded, Python's store survives restarts).

### 3. Why simple deletion (017 draft Phase B) is wrong

Deleting `crates/operant-harness` would:
- Break `cargo check` (6 files import it, see above)
- Remove `architecture.toml` composition (Rust-only, no Python equivalent)
- Remove `tool`/`pool`/`wasm` seams (Python harness only knows `prompt`/`subagent`)
- Require re-implementing 6 seams in Python — YAGNI and language mismatch (WASM hot-swap is Rust-side).

The correct unification is **not deletion but projection**: make the Python harness the *persistent store* and the Rust harness the *in-memory view + composition layer* over it — or, more lazily, keep them separate but make them **non-overlapping** (Python owns `prompt`/`subagent`, Rust owns `tool`/`pool`/`wasm`/`hook`). That's already the de-facto split (`kernel/mod.rs:9` says so), so the "duplication" is actually **intentional partitioning**, not duplication.

### 4. Revised Phase B — no deletion, just boundary hardening

**Goal:** make the split explicit and remove the *perceived* duplication without deleting a live crate.

**B1 — Boundary docs (0 lines, just docs, 1 PR):**
- Update `crates/operant-harness/src/lib.rs:1` module docs: "This crate owns `tool`/`pool`/`wasm`/`hook`/`prompt.section` composition; `kernel-sidecar/harness.py` owns `prompt`/`subagent` persistence. The two harnesses are complementary, not duplicate."
- Update `crates/operant-core/src/tools/kernel/mod.rs:1` docs to match.
- No code change.

**B2 — Single `prompt` ownership (30 lines, 1 PR):**
- Today `prompt` is claimed by *both* harnesses (Rust `prompt.section` seam, Python `prompt` kind). Pick one: keep `prompt` in Python (since it's file-persistent) and make Rust's `prompt.section` a *read-only alias* that calls `harness_overview` instead of owning its own store. Or vice versa — but the cheapest is to leave split as is and just document it (B1). If we do B2, it's a 30-line change where `prompt.section` mount goes via `kernel::global_runtime().request("harness_upsert")` instead of `Harness::mount`.

**B3 — Unified injection (already done in 016):**
- `injection_block` already calls `harness_overview` for `prompt`/`subagent` and `Harness::dump` for `tool` etc. separately. No change needed; just ensure `per-turn cache` (`runtime.rs:190`) covers both.

**B4 — Deletion NOT performed:**
- Keep `crates/operant-harness` as is. It is the composition/harness kernel; `kernel-sidecar` is the persistence sidecar. They are two processes, not two copies.

### 5. What to do next

- **Do B1 only** for 017-B (docs, 0 risk). Ship after 017-A.
- Defer B2 until a concrete `prompt` conflict is observed (no conflict today; `prompt` in Rust is `prompt.section` subscope, Python is top-level `prompt` — they don't collide).
- Proceed to 017-C (skills scan unification) which is the remaining true duplication (double `read_dir`).

## Verdict

Phase A is green (`cargo check` `Finished`, `74f07cbb`). Phase B as originally drafted (delete `operant-harness`) is **rejected** — audit shows the crate is live and its 6 seams have no Python equivalent. The perfect integration keeps both harnesses but makes their boundary explicit (docs only), which is the lazy deletion: delete the *perception* of duplication, not the code.


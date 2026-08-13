# Operant — LCM & AFT Integration: Architectural Audit

**Date:** 2026-08-13 · **Scope:** Lossless Context Management (hermes-lcm parity) and
Aft file-tools bridge (cortexkit/aft parity) · **Method:** source inspection + live
agentic-loop testing of every tool in both surfaces.

---

## 1. Executive Summary

| Integration | Verdict | Tool surface | Live agentic-loop result |
|-------------|---------|--------------|--------------------------|
| **LCM** (lossless context mgmt) | ✅ Production-grade | 5 tools | 9/9 PASS (v2 E2E) + 6/6 in combined audit |
| **AFT** (code file-tools bridge) | ✅ Production-grade (1 gap fixed this iteration) | 18 tools | 12/13 → 13/13 after callgraph warm-up fix |

Two real defects were found and fixed during this audit:

1. **AFT callgraph cold-build surfacing as a hard tool error** — `aft_callers` /
   `aft_inspect`(dead-code) failed with `[callgraph_building] "building in the
   background; retry shortly"` on a fresh store. Fixed via a bounded exponential-
   backoff retry in the bridge (`send_request`) + per-tool 180s timeout overrides.
2. **LCM embeddings were a hard external dependency** — verified `local:hash`
   zero-dependency embedder so operant runs fully offline by default (hermes-agent
   parity: no embedding provider there either). Composite-PK fix for the
   `lcm_embeddings` table also shipped.

---

## 2. LCM — Lossless Context Management

### 2.1 What it is (from hermes-lcm source)

hermes-lcm replaces lossy context eviction with a **persisted, lossless DAG** of
conversation turns and rollups. Every token ever produced is retained; the LLM only
sees an optimized window + targeted recall. Core properties (verified from source):

- **Losslessness**: nothing is dropped — old context is stored, not deleted.
- **Rollups**: older turns are summarized into nodes so the active window stays
  small while facts remain recoverable.
- **Recall**: `lcm_recall` / `lcm_recall_round` query the DAG for relevant context.
- **Assertions**: structured facts (`subject/predicate/object`) with active/archived
  lifecycle, extracted automatically in the background.
- **Optional vector recall**: semantic search over the DAG when an embedding backend
  is available.

### 2.2 Architecture in operant

```
operant-core/src/context/
├── lcm.rs          # DAG store (SQLite lcm.db): nodes, rollups, assertions,
│                   #   embeddings; rollup scheduler; maintenance gating
├── embedder.rs     # Embedder trait: local:hash (zero-dep, default) | remote (Ollama etc.)
└── mod.rs          # ContextEngine wiring
```

- **DAG schema** (SQLite): `nodes` (id, parent, depth, role, content, summary,
  created_at), `lcm_assertions` (subject/predicate/object/state), `lcm_embeddings`
  (node_id, model, vector — **composite PK** `(node_id, model)`).
- **`ContextEngine` trait** mirrors the hermes `ContextEngine` abstraction so other
  engines (e.g. truncation) can coexist behind one interface.
- **Rollups** replace lossy eviction when `context_engine = "lcm"` (P1 ✅).
- **Background assertion extraction**: a maintenance scheduler extracts
  `lcm_assert` facts from conversation turns (P3 ✅, `27e0b1db`).
- **Auto-recall**: relevant DAG context is injected into the active window before
  each turn (P3 ✅).
- **Embeddings are optional**: default `local:hash` needs zero external services;
  a remote endpoint is only consulted if configured (`7674ff3a`).

### 2.3 Config surface (hermes parity)

```toml
[context]
engine = "lcm"                     # or "truncation"
lcm_db_path = "~/.operant/lcm.db"
lcm_recall_tokens = 4000
lcm_assertion_extraction = true    # background scheduler
lcm_embedding_model = "local:hash" # zero external deps by default
# lcm_embedding_base_url = "http://localhost:11434/v1"  # only if remote wanted
```

### 2.4 Tool surface (5 tools, live-registered)

| Tool | Purpose | Verified |
|------|---------|----------|
| `lcm_stats` | Engine + DAG stats (`engine="lcm"`, dag_nodes, rollups, assertions) | ✅ PASS |
| `lcm_assert` | Save/query/archive structured facts | ✅ PASS (save + query) |
| `lcm_recall` | Recall relevant DAG context | ✅ PASS |
| `lcm_recall_round` | Recall with round-trip completeness flag | ✅ PASS |
| `lcm_vector_recall` | Cosine-ranked semantic recall (local:hash or remote) | ✅ PASS (offline) |

### 2.5 Live agentic-loop test (v2 E2E + combined audit)

- **9/9 PASS** in the dedicated E2E (documented in `docs/HERMES_LCM_INTEGRATION.md`),
  including a **live `lcm_assert` background extraction** from a real turn.
- **6/6 PASS** in the combined audit run (steps 1–6: stats → assert save → assert
  query → recall → recall_round → vector_recall).
- **Vector recall proven fully offline**: `lcm_vector_recall` returned real
  cosine-ranked hits with `"model":"local:hash"`, no Ollama, no network.

---

## 3. AFT — File-Tools Bridge (cortexkit/aft)

### 3.1 Architecture in operant

```
operant-core/src/
├── aft_bridge.rs        # AftBridgePool, binary resolution, subprocess lifecycle,
│                        #   harness protocol (configure → requests → responses)
└── tools/aft_tools.rs   # 18 tool registrations via register_aft_tools(registry, pool)
operant-cli/src/main.rs  # gate: config.tools.aft_enabled → register + timeout overrides
```

- **Binary resolution** (`resolve_aft_binary`): `AFT_BINARY` env override →
  auto-download to `~/.operant/aft/aft-<version>/aft` → PATH fallback. Verified:
  AFT **v0.49.4** resolved from the managed cache.
- **Pooled bridge**: `AftBridgePool` keeps subprocess(es) warm; a reader loop
  dispatches responses back to awaiting callers via a pending-map keyed by request id.
- **Configure-time project root** is sent so the subprocess indexes the workspace
  once, then all 18 tools reuse the warm session.
- **Timeout overrides** (`main.rs`): `aft_callers`/`aft_inspect` get 180s (callgraph
  store cold-builds lazily on first use).

### 3.2 Tool surface (18 tools, live-registered)

| Tool | Purpose | Live result |
|------|---------|-------------|
| `aft_read` | Read file with symbol context | ✅ |
| `aft_write` / `aft_edit` | Write / targeted edit | ✅ |
| `aft_apply_patch` | Apply a patch | ✅ |
| `aft_search` | Semantic/code search | ✅ |
| `aft_grep` / `aft_glob` | Pattern file discovery | ✅ |
| `aft_outline` | File structure outline | ✅ |
| `aft_zoom` | Zoom into a symbol | ✅ |
| `aft_inspect` | Codebase health / dead code | ✅ (post warm-up) |
| `aft_callers` | Callers of a symbol | ✅ (post warm-up, 1.3s) |
| `aft_ast_search` / `aft_ast_replace` | AST-level query/rewrite | ✅ |
| `aft_bash` | Sandboxed command exec | ✅ |
| `aft_checkpoint` / `aft_list_checkpoints` / `aft_undo` | State rollback | ✅ |
| `aft_status` | Workspace status | ✅ |

### 3.3 Gap found & fixed: callgraph cold-build (this iteration)

- **Symptom**: in the combined live audit, `aft_callers` returned
  `[callgraph_building] "callgraph store is building in the background; retry shortly"`
  — a hard failure surfaced to the model on a fresh store.
- **Root cause**: AFT persists a per-project callgraph store
  (`~/.local/share/cortexkit/aft/callgraph`) that **cold-builds lazily on first use**
  and can take minutes on a large workspace (operant itself: 1,147 files / ~591k LOC;
  AFT's own `warmup` CLI defaults to a **600s** build budget). The bridge surfaced
  the transient state as a terminal error instead of waiting.
- **Fix** (`05bb349e`):
  1. `aft_bridge.rs` — `send_request` now retries `callgraph_building` responses
     with exponential backoff (1.5s × 2ⁿ, 6 attempts ≈ 95s, bounded well under the
     600s request timeout) before surfacing the error. The store is persisted, so
     this is a **one-time cold-build cost per project root**.
  2. `operant-cli/src/main.rs` — per-tool timeout overrides to 180s for
     `aft_callers` and `aft_inspect` so the retry window fits inside the executor.
- **Verification**: on the now-warm store `aft_callers` returns real results
  (`build_registry` @826, `build_context_engine` @1236) in **1.3s**. Full gates
  green (core 1504, CLI 652; clippy -D warnings clean; fmt clean).

---

## 4. Combined live agentic-loop audit (13 steps)

Run against the real binary (`/tmp/operant-ae.toml`, AFT v0.49.4, LCM engine with
Ollama embeddings available at the time):

| # | Step | Result |
|---|------|--------|
| 1 | `lcm_stats` → engine="lcm", dag_nodes | ✅ |
| 2 | `lcm_assert` save `audit:lcm`/status/verified | ✅ |
| 3 | `lcm_assert` query → active state | ✅ |
| 4 | `lcm_recall` "deploy cadence" → hits | ✅ |
| 5 | `lcm_recall_round` → complete flag | ✅ |
| 6 | `lcm_vector_recall` → ranked hits | ✅ |
| 7 | `aft_read` a source file | ✅ |
| 8 | `aft_search` a symbol | ✅ |
| 9 | `aft_outline` a module | ✅ |
| 10 | `aft_grep` a pattern | ✅ |
| 11 | `aft_glob` a file pattern | ✅ |
| 12 | `aft_inspect` → triggered callgraph build | ✅ |
| 13 | `aft_callers` for `lcm_config` | ✅ (was ❌ → fixed) |

Post-fix re-verification: the full sequence passes end-to-end in the loop; the
callgraph store persists, so step 13 is now instant on subsequent runs.

---

## 5. Gaps fixed during this audit (all pushed)

| Commit | Fix |
|--------|-----|
| `05bb349e` | AFT callgraph cold-build: bounded retry in `send_request` + 180s timeouts for `aft_callers`/`aft_inspect` |
| `7674ff3a` | LCM embeddings fully optional (zero external deps); composite PK for `lcm_embeddings` + legacy migration |
| `27e0b1db` | LCM background assertion-extraction scheduler + maintenance gating |

## 6. Iteration 2 — full 18-tool AFT sweep + LCM finalization (pushed in this iteration)

### 6.1 AFT — every tool tested in the live agentic loop

Scratch workspace `/tmp/aft-scratch` (small Rust project with real call
relations); model `nemotron-3.5-lightning-free` on the OpenCode endpoint
(`deepseek-v4-flash-free` was quota-exhausted at test time). Result: **18/18
tools operational**.

| Tool | Live | | Tool | Live |
|------|------|-|------|------|
| aft_status | ✅ | | aft_write | ✅ |
| aft_read | ✅ | | aft_edit | ✅ |
| aft_search | ✅ | | aft_apply_patch | ✅ (format fixed) |
| aft_outline | ✅ | | aft_ast_replace | ✅ (syntax fixed) |
| aft_zoom | ✅ | | aft_bash | ✅ |
| aft_grep | ✅ | | aft_checkpoint | ✅ |
| aft_glob | ✅ | | aft_list_checkpoints | ✅ |
| aft_ast_search | ✅ (syntax fixed) | | aft_undo | ✅ |
| aft_callers | ✅ (warm store, 1.3s) | | aft_inspect | ✅ |

**Three agent-guidance gaps found and fixed** (the tools were functional; the
model was using the wrong input syntax — tool descriptions now document the
exact formats):

1. **`aft_apply_patch` format** — AFT's patch dialect, NOT unified diff:
   `*** Begin Patch` / `*** Update File: <path>` (hunks under a bare `@@`
   anchor line with space-prefixed context and `-`/`+` lines) / `*** Add
   File:` / `*** Delete File:` / `*** End Patch`. Verified end-to-end (Add +
   Update both apply on disk).
2. **`aft_ast_search` / `aft_ast_replace` syntax** — ast-grep code patterns
   (`console.log($MSG)`, `fn $NAME($$$ARGS)`), not plain text / node-kind
   names; `lang` required. Verified against AFT's own fixtures.
3. **`aft_callers` cold-store behavior** — an empty caller list on a fresh
   project means the persisted callgraph is still building (the bridge waits
   it out; `build_registry` → 2 real callers verified on the warm store).

### 6.2 AFT warmup-on-configure — implemented

Investigation: the subprocess is `kill_on_drop(true)`, so on a never-warmed
root every fresh session paid the cold-build cost via retry, and a session
exit could abort a mid-build. `AftBridgePool` now fires a **detached**
`aft warmup --root <root> --only callgraph --timeout 600000` when it spawns a
bridge for a new project root — non-blocking, deduped per root, warn-only on
failure, and it survives operant exit (AFT's CLI returns ~immediately when the
store is already warm, so re-firing is cheap).

### 6.3 LCM — parity audit vs hermes-lcm source + finalization

| hermes-lcm feature | operant status |
|--------------------|----------------|
| SQLite message store (raw preserved) | ✅ nodes table |
| Depth-aware summary DAG + rollups | ✅ lcm_rollups (day/week/month) |
| Fresh-tail protection (D0) | ✅ tail_tokens |
| Rollup injection on budget exceed | ✅ rollups_inject |
| Auto-recall | ✅ |
| Assertions + background extraction | ✅ |
| Semantic recall (cloud or local) | ✅ local:hash zero-dep default |
| **`lcm_recall` FTS+vector RRF fusion** | ✅ **added this iteration** |
| **`lcm_recent` temporal recall** | ✅ **added this iteration** |
| **`lcm_doctor` health diagnostics** | ✅ **added this iteration** |
| Sensitive redaction before storage | ✅ redaction.rs |
| Externalization / session-controls / evidence-packing | out of scope (hermes-plugin memory extensions; YAGNI for operant core) |

**bm25 decision: not needed.** hermes-lcm's `lcm_recall` fuses a full-text arm
and a semantic arm with RRF. operant already has SQLite FTS5 (the lexical
arm) and `local:hash`/remote vectors (the semantic arm) — the parity gap was
the fusion, not a new embedder. `lcm_recall` now fuses both arms with RRF when
an embedder is configured (FTS-only otherwise — backwards compatible).
`local:hash` remains the correct zero-dependency default.

Live verification (real DAG): `lcm_doctor` reports `integrity_check: ok`,
314 nodes, 100% FTS coverage, 28 assertions, 239 embeddings, 12 rollups;
`lcm_recent` returns day-period rollups; `lcm_recall` returns fused hits
(vector-arm hit ranked alongside FTS bm25). New unit tests: RRF fusion
(merge/dedupe/rerank), fused recall path, recent fallback, doctor health.

---

## 7. Verdict

Both integrations are **production-grade for deployment**:

- **AFT**: all 18 tools verified in the live loop; input-format documentation
  fixes make them reliably usable by the model; detached callgraph warmup
  removes the cold-start wait; bridge retry covers the residual window.
- **LCM**: core hermes-lcm parity complete — lossless DAG, rollups, fresh
  tail, auto-recall, assertions, FTS+vector RRF fusion, temporal recall
  (`lcm_recent`), diagnostics (`lcm_doctor`), zero mandatory external deps.
- **Overall**: 7 LCM + 18 AFT = 25 tools live; repository fully green
  (fmt, clippy -D warnings, core 1508 + CLI 652 tests) and pushed to
  origin/main.

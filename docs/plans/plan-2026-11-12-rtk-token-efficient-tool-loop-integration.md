# RTK into Operant's tooling architecture: a token-efficient evidence layer

Dated-frozen: published `2026-11-12`. Scope: a design plan.

## Landing status (corrected 2026-11-13 against `origin/main` @ `3a9c489d`)

The 2026-11-12 version of this table was wrong in five places. It was corrected on
2026-11-13 from the code, and every corrected row cites the file and line it was
corrected against. The evidence layer landed across three commits, not one:

- `9d163951` (**iter-2120**) — the layer itself: types, chain runner, classifier,
  router, recall contract, the method set and chains, `[compression]` in
  `operant.example.toml`, and the tests.
- `5244ac70` (**iter-2123**) — the layer was DELETED again (the files went), and
  the request-time trim, the context ledger and the per-session usage records
  landed with it.
- `9f723e88` (**iter-2139**) — the layer landed on the tool-result chokepoint,
  with the `rtk` bridge, the terminal rewrite, the fleet ranking,
  `operant doctor --tools-tokens`, and `evidence_recall` resolving the marker.

| Part | State | Commit |
|---|---|---|
| P0 — instrument the tool-result chokepoint | **LANDED** — per-tool token accounting is part of the layer's own `Compressed` report (`tokens_before`/`tokens_after`/`applied`), `crates/operant-core/src/agent/evidence/mod.rs:95-116` | `9d163951` (iter-2120), re-landed in `9f723e88` (iter-2139) |
| P1 — the evidence layer (types, chain runner, classifier, router, recall contract) | **LANDED** — `crates/operant-core/src/agent/evidence/` | `9d163951` (iter-2120), re-landed in `9f723e88` (iter-2139) |
| P2 — the method set and the per-format chains | **LANDED** — 9 lossless + 6 lossy methods, plus the code filter | `9d163951` (iter-2120), re-landed in `9f723e88` (iter-2139) |
| P3 — lossless recovery | **LANDED, but not as this plan described it** — there is NO bounded ring and NO disk spill: both were deliberately deleted, so one fact has one owner. The full text lives in `messages.full_content` (`crates/operant-core/src/database.rs:78`) and the `evidence_recall` tool reads it back (`crates/operant-core/src/tools/evidence_recall.rs:96` → `crates/operant-core/src/database.rs:1289`) | `9f723e88` (iter-2139) |
| P4 — tool-family coverage | **PARTIAL** — the coverage test iterates `builtin_tool_names()` (`crates/operant-core/src/tools/builtin.rs:281`), a hand-written list that the live registry is tied to only by a COUNT assertion (`crates/operant-core/src/tools/builtin.rs:406-413`). The kernel tools (`kernel_exec`, `kernel_state`, `kernel_refine`, registered from `crates/operant-core/src/tools/kernel/`) are in neither list and are unclassified, so their results are never compressed. The test that iterates the list is `crates/operant-core/tests/evidence_classification.rs:75` | `9d163951` (iter-2120), re-landed in `9f723e88` (iter-2139) |
| P5 — the shell path | **LANDED** — `compression.shell = "aft" \| "native" \| "rtk"`, defaulting to `aft` (AFT already compresses bash in-process) | `9d163951` (iter-2120), the `rtk` backend wired in `9f723e88` (iter-2139) |
| P6 — observability | **LANDED** — the `Compressed` report carries the per-result saving, the methods applied, and the subject/format | `9d163951` (iter-2120), re-landed in `9f723e88` (iter-2139) |
| P0 — a *fleet-wide* baseline of operant's own token distribution | **LANDED** — `Database::tool_token_ranking` (`crates/operant-core/src/database.rs:1338`), its test (`crates/operant-core/src/database.rs:5310`), and the report `operant doctor --tools-tokens` (the flag: `crates/operant-cli/src/cmd_doctor/mod.rs:60`; the report: `crates/operant-cli/src/cmd_doctor/checks_tools.rs:1155`) | `9f723e88` (iter-2139) |
| The `rtk` binary as a live backend | **LANDED** — the bridge is `crates/operant-core/src/rtk_bridge.rs` (`find_rtk`, `rewrite_command`), and the terminal tool runs the rewrite when `compression.shell = "rtk"` (`crates/operant-core/src/tools/terminal_tool.rs:151-167`) | `9f723e88` (iter-2139) |

**The one claim this plan deliberately does NOT make:** a percentage. §11's numbers are AFT's measured
benchmark and RTK's documented per-rule design bounds — neither is operant's own. The layer now
*measures* every result it touches, so the number becomes available the first time it runs on a real
session; until then, any percentage would be a guess dressed as a measurement.

Evidence base: a first-hand clone of [`rtk-ai/rtk`](https://github.com/rtk-ai/rtk) @ `0b92106f` plus the
installed `rtk` binary; the AFT checkout at `external-repos/plugins/aft` (`cortexkit/aft` @ `9dcb4954`);
and Operant itself @ `53d9712b`. Claim tags used below: **[V]** verified at a cited location ·
**[D]** documented by the upstream project · **[R]** reasoned from those · **[G]** guess.
The full register is `_kernel/rtk-integration/claims.md`.

---

## 0. Result first

**The naive plan — "make operant's terminal tool call `rtk <command>`" — is the wrong plan, and this
document explains why with evidence.** AFT, which operant already bridges, contains a **12,380-line
compression subsystem** (`crates/aft/src/compress/`) that covers the same ground as RTK, in-process, for
the bash path [V]. Adding RTK there would double-compress, add a process spawn per command, and split the
filter policy into two tables that will drift.

**What RTK genuinely contributes to operant is its abstraction, not its filters.** `rtk-engine`
describes a tool result as `(Subject, ContentFormat) → method chain`, with a per-method contract
(lossy? recoverable? idempotent? cache-stable? deterministic? cost) and a `never-worse` guarantee [V].
That is exactly the layer operant is missing: operant shrinks tool results **by size**, never by
**shape** [V]. AFT compresses shape, but only for bash [V].

Three measured facts set the priority order:

1. **`aft_read` returns up to 2 000 lines of raw source — ~10–25 K tokens per call — and nobody
   compresses it.** Not AFT, not RTK, and not operant's 4 096-char cap, which cuts it to a head/tail
   sliver that is usually the wrong sliver [V]. Code filtering is the direct answer and it does not exist
   anywhere in the stack.
2. **AFT's compression is whitelist-first, and its generic tier saves ~0 tokens on unrecognised
   commands** — `git status --short`, `git log --oneline`, `helm list` each saved exactly 1 token in
   AFT's own benchmark [V]. That measured gap *is* RTK's lane.
3. **AFT and RTK independently ship the same recall UX** — a printed pointer to the retained artifact
   plus chunked retrieval, surviving restarts [V]. Copy that shape; don't design a second one.

And one architectural finding settles the shell question: **RTK's rewrite table and AFT's decline table
are the same policy in opposite directions** — both redirect `grep`, `cat`, `ls`, `find` and `rg` away
from raw execution, RTK to a filtering wrapper and AFT to a semantic tool (§5.2) [V]. Two independent
projects reached the same conclusion; AFT's version is the better one for a code workspace, and the two
tables must never both be installed.

**The plan, in one line:** build an *evidence layer* at operant's tool-result chokepoint
(`crates/operant-core/src/agent/run.rs:2694-2757`) that classifies every tool payload into a
`(Subject, ContentFormat)` pair, routes the dominant payload field through a format-appropriate
compression chain, applies a never-worse guard, and records a lossless-recall backstop — and let AFT and
RTK each keep the surface they already own.

---

## 1. The meta-architecture: where an agentic loop actually spends tokens

This is the framing the request asked for. An agentic tool loop has four layers between the model and
the world:

```
1. INTENT layer     what the model sees it can do (schemas, descriptions, tool inventory)
2. EXECUTION layer  where the work happens (shells, parsers, indexes, subprocesses, MCP servers)
3. EVIDENCE layer   the result, turned into text, before it enters the context   ← the weak layer
4. CURATOR layer    what survives in the context across turns (compaction, eviction, recall)
```

Token cost accumulates in all four, and each is optimised differently:

| Layer | Lever | Operant today | Grade |
|---|---|---|---|
| Intent | smaller schemas, lazy listing | `tools/tool_search/` — Tier 0/1/2 progressive disclosure for MCP [V] | **good** |
| Execution | amortised subprocesses, indexes, warm state | `AftBridgePool` (one `aft` per root), `daemon_pool`, `pooled_client` [V] | **good** |
| Evidence | format-aware compression, field routing, per-result budget | `guard_tool_output_spend` + `truncate_tool_result` — **size-based only** [V] | **the gap** |
| Curator | lossless DAG, rollups, targeted recall | LCM (`context/lcm.rs`) + `llm_compressor` + `manage_context` [V] | **good** |

Three of four layers are well-built. The evidence layer is a 4,096-character blunt cap and a 5%-of-window
share guard [V]. Every tool result that enters the context is either in full, half, or truncated — never
*shaped*.

**Consequence:** a `web_fetch` of 900k of HTML, a `grep` returning 40 000 matches, a `code_execution`
stdout with 2 000 progress lines, and a `file_read` of a 40 000-line data file all get the same
treatment: a head/tail cut with a `[... N characters truncated ...]` marker. The model loses the part of
the result that answered its question, and the transcript carries the part that didn't.

**RTK and AFT are both evidence-layer tools.** That is the axis on which they should be judged — and on
that axis they cover *different halves* of operant's surface.

---

## 2. What operant already has (and why it is insufficient)

Three mechanisms, all cited:

| Mechanism | File:line | What it does | Limit |
|---|---|---|---|
| `guard_tool_output_spend` | `agent/compress.rs:709-755` | Withholds the bulk of a result over `max_tool_result_share` (default **0.05**, `config.rs:959-961`) of `window − 4096`, behind a marker reporting real size, share, and estimated input price | Fires only above 5% of the window; no notion of what the result *is* |
| `truncate_tool_result` | `agent/mod.rs:1148-1189` | Absolute **4,096**-char cap; drops `audio`/`data` fields; truncates the largest JSON string fields so metadata survives | A byte cut, not a semantic one |
| `fast_trim_tool_results` | `agent/compress.rs:117-225` | Retro-active re-trim of *old* tool messages at compaction (**2 000** chars, 2/3 head + 1/3 tail, envelope-preserving) | Runs after the cost was paid |

The critical property they **share**: all three decide *how much* to keep and none decides *what* to
keep. `guard_tool_output_spend` measures the real size of the result to report it [V] — which is
evidence that the chokepoint is already the right place to add shape-awareness.

Three further relevant facts:

- `ToolResult.content` is the **serialized JSON** of the tool's payload (`tools.rs:186-188`; the cap
  parses it back as JSON at `agent/mod.rs:1157`). A compressor at the chokepoint therefore sees an
  envelope `{…}`, not a bare string [V] **[R]** ⇒ the compressor must route into the envelope's
  dominant payload field, or it will compress `{"exit_code":…,"stdout":…}` as generic JSON and lose the
  distinction between "the thing to compress" and "the thing to keep".
- `ToolResult` has **no metadata/provenance field** (`tools.rs:158-179`). Compression provenance must
  travel either in the compressed text as a marker or through a side channel [V] **[R]** ⇒ prefer the
  marker: it is the only channel the model can act on.
- `redaction::redact_sensitive_text_if_enabled` already runs **immediately before** the spend guard
  (`run.rs:2745`). **[R]** Compression must run *after* it, so no lossy transform or recall store ever
  holds pre-redaction bytes.

---

## 3. What AFT already does (verified, not assumed)

`cortexkit/aft` is a Rust binary with tree-sitter code tools and a safety/checkpoint model. Operant
bridges it over NDJSON, registering 20 tools through one pooled subprocess per project root
(`aft_bridge.rs:9-28`; `tools/aft_tools.rs:36-90`).

The finding that reshapes the plan: **AFT already ships a 12,380-line compression subsystem**
(`crates/aft/src/compress/`), 5-tiered (`compress/mod.rs:1-25`):

1. **Specific compressors** per tool — `git` (966), `go` (652), `tree` (643), `vitest` (454),
   `biome` (434), `eslint` (397), `playwright` (536), `mypy` (290), `cargo` (323), `next` (311),
   `pytest` (278), `ruff` (282), `tsc` (261), `prettier` (223), `npm` (219), `ls` (204), `find` (176),
   `pnpm` (175).
2. **Output-shape sniffers** that recognise their own private summary even through wrappers.
3. **Package-manager compressors** (`npm`/`pnpm`/`bun`, 543) claiming by head token.
4. **19 declarative TOML filters** for the long tail: `make`, `df`, `du`, `wc`, `gradle`,
   `xcodebuild`, `terraform`, `helm`, `docker`, `kubectl`, `gh`, `ansible-playbook`, `aws`, `curl`,
   `deno`, `pip`, `psql`, `uv`, `wget` (`compress/builtin_filters.rs:14-57`).
5. **Generic fallback** — ANSI strip + consecutive-line dedup, with a final output cap.

with drop-class caps (errors 20 / warnings 10 / list 20 / inventory 50, `compress/caps.rs:3-6`),
per-compressor ceilings (`git` status 1 024 / diff 5 files × 20 hunks × 30 lines, `tsc` 30 errors/file,
`prettier` 250, `mypy` 300, `next` 500, `playwright`/`tree` 400 + 20 JSON failures), and a **real Claude
tokenizer** (`crates/aft-tokenizer`, vendored BPE — not a bytes/4 heuristic).

Three facts bound the overlap, and all three cut the same way:

- **AFT's compression is invoked only from the bash background registry**
  (`crates/aft/src/bash_background/registry.rs:8573, 8663`). It compresses *shell command output* and
  nothing else [V].
- **AFT's compression is whitelist-first.** Tier 5 is *only* ANSI-strip + consecutive-dedup
  (`compress/generic.rs:3`), with per-tier hard caps (`bash_background/output.rs:3-5`: final 16 KiB,
  head 6 / tail 10). Its own benchmark measures **0 tokens saved on unrecognised commands** — `git status
  --short`, `git log --oneline`, `docker-compose`-like and `helm list` each saved exactly 1 token
  [V — AFT's recon report, from `benchmarks/compression-tokens/`]. That empty generic tier **is** RTK's
  lane, and it is a measured fact, not an inference.
- **AFT's `bash_rewrite` is a decline/redirect surface, not a rewrite-to-compressor**:
  `ControlRole{Accept, Decline, Native, Sandbox}`; `RULE_INVENTORY = grep, rg, find, cat, cat_append,
  sed, ls`; the decision classes are literally named `dc.grep.decline.v1`, `dc.rg.decline.v1`,
  `dc.find.decline.v1`, `dc.cat.decline.v1`
  (`crates/aft/src/bash_rewrite/catalog.rs:9-80`). AFT's answer to "the model is about to `grep`" is
  **"don't — use `aft_grep`"**, i.e. redirect to its own semantic tool, not to a filter.

### Two things AFT already does right that this plan should copy rather than invent

**(a) AFT's full-output recall.** A compressed rendering prints a pointer to the retained artifact —
`[JSON output <kb> KB; full output: read "<path>"]` (`bash_background/output.rs:300-306`) — and
`bash_status` accepts `output_offset` and returns `output_chunk_base64` + `output_next_offset`
(`commands/bash_status.rs:31, 111-127`). Tasks persist at `harness_dir()/bash-tasks/<hash(session)>`
and survive restarts. This is exactly the `Recovery::Recall` contract, already working, in the sibling
project. P3 below should adopt this shape rather than design a new one.

**(b) AFT's chunked caps, and the reason they exist.** AFT replaced a uniform 4 KiB head+tail preview
with a 16 KiB / 6+10 KiB split, and the design comment states why: the old uniform preview *"injected
~1K tokens of mostly build noise per completed task into reminders"* (`bash_background/output.rs:9-18`).
Operant's `truncate_tool_result` is the same 4 096-char uniform cut (`agent/mod.rs:1150`), and it is the
single most-repeated line of evidence-layer cost in the codebase. P2 should read that comment before
sizing anything.

**Therefore:** AFT owns *code understanding* (semantic search, outline, zoom, callers, inspect — 20
tools) and *bash output compression*. RTK owns *generic command-output filtering*, and — uniquely —
a *format-agnostic* compression taxonomy for non-shell results.

---

## 4. What RTK actually is (verified)

`rtk-ai/rtk`, Rust, clap, version 0.49.0 in-tree (`Cargo.toml:26-31`); the installed binary here reports
`1.0.0`. Six-phase lifecycle — parse → route → execute → filter → print → track
(`docs/contributing/ARCHITECTURE.md:57-140`), ~5–15 ms overhead per command, <10 ms startup, SQLite
tracking with 90-day retention (`~/.local/share/rtk/history.db`).

Licence is the first thing to check before any integration:

- CLI: **Apache-2.0**.
- `rtk-engine` (the compression core): **BUSL-1.1**, Change Date **2030-09-09** → Apache-2.0.
  The Additional Use Grant permits production use, free, *"including as a component of the `rtk`
  command-line binary and the `rtk-proxy` daemon **distributed by RTK AI Labs**"*, and does **not**
  permit offering the licensed work, or a derivative, to third parties as a paid or hosted product or
  service [V — `engine/LICENSE:1-27`].

What is genuinely adoptable, and licensed-clean because it is an *idea*:

| RTK concept | Location | What operant gains |
|---|---|---|
| `Subject` vocabulary: `Read Write Edit Shell Search Web Subagent Mcp ToolResult …` | `engine/src/block.rs:31-73` | One taxonomy for all ~65 tool surfaces |
| `ContentFormat` vocabulary: `Json Web Matches Lockfile Term` | `engine/src/block.rs:133-151` | The "what shape is this payload" question, asked once |
| `Method` + `MethodInfo` contract | `engine/src/method.rs:3-68` | Every compressor declares `lossy`, `recovery`, `idempotent`, `cache_stable`, `deterministic`, `cost`, `saves_on`, `cost_tokens` |
| 15 generic methods | `engine/src/registry.rs` | ansi, progress, dedup, base64-elide, minify-json, md-slim, strip-line-numbers, shape, rle (lossless) + toon, grep-group, lockfile, web-md, cap, truncate (lossy, recoverable) |
| 5 format chains | `engine/src/methods/mod.rs:40-64` | `Json=[base64-elide,toon]`, `Web=[ansi,base64-elide,web-md,toon,md-slim]`, `Matches=[grep-group]`, `Lockfile=[lockfile]`, `Term=[ansi,progress,dedup,base64-elide,cap]` |
| `run_chain_guarded` | `engine/src/chain.rs:24-67` | Panic containment + a `never_worse` guard that rejects growth *and* no-ops |
| `HarnessProfile` + degradation ladder | `engine/src/harness.rs:1005-1027` | A formal model of what a harness *can* do to a result — see §9 |
| Declarative pipeline rules | `engine/src/pipeline/schema.rs:5-175` | `when` conditions + a method chain + guards, as data, not code |
| `Recovery::Recall` | `engine/src/method.rs:33-39` | Lossy methods must declare how the full text comes back — **and RTK's implementation is a working reference** (below) |
| RTK's recall store, as a *design* reference | `src/core/retriever.rs:1-258` | Content-addressed SQLite at `~/.local/share/rtk/recall.db`; blob = the **full raw child stdout+stderr**, gzip; hash = `SHA256(command ‖ 0x00 ‖ content)` truncated to 16 hex (⇒ command-scoped); caps 10 MiB/entry, 200 entries, 30-day retention; retrieval `rtk recall <hash>` with unique-prefix resolution, `--full/--from/--lines/--grep/--list`, and **by default returning only the elided portion** [V] |

What is **not** adoptable, and why:

- **No MCP server mode** [V] — RTK integrates as hooks/plugins/a proxy, never as an MCP server. Now
  confirmed exhaustively: zero MCP/`rmcp` references across all five manifests, 93 `Commands` variants with
  no MCP subcommand, and every "mcp" hit is *consumer*-side (`Subject::Mcp`, the `mcp__.*` matcher, the
  `hook.*.mcp` pipeline targets). RTK *consumes* MCP output via PostToolUse and never serves MCP. **Any MCP
  compression in operant is ours to build** [V].
  - **And operant already owns the surface to build it on** — but *not* via `rmcp`. The `rmcp` 3.5.0 entry
    in `Cargo.lock:4298-4303` reaches operant only **transitively through `memory-wire` 0.10.0**
    (`Cargo.lock:2795, 2805`); **no operant source imports `rmcp`** [V]. Operant's MCP substrate is its
    own hand-rolled client: `crates/operant-core/src/mcp.rs`, **3 944 lines** — JSON-RPC 2.0 over stdio and
    SSE, protocol-version negotiation (`2025-06-18` / `2024-11-05`, `mcp.rs:22-29`), OAuth, banner/drain
    limits, auto-restart [V]. MCP tools are registered as `McpNamespacedTool` (`mcp.rs:2156`) implementing
    `OperantTool` (`mcp.rs:2295`) into the ordinary `ToolRegistry` (`mcp.rs:2602-2605`) — so **MCP results
    already flow through the same `execute_tools` → `run.rs` chokepoint as every other tool** [V]. The
    evidence layer therefore reaches MCP servers with no MCP-specific work at all; the only thing RTK
    could never supply is an MCP *server* front-end, and operant has no reason to build one.
  - Caveat worth carrying forward: `sourcehound_mcp` is **explicitly held out** of
    `crates/operant-core/Cargo.toml:105-108` (vendored `obscura-render` breaks against registry `taffy
    0.12.2`, 36 errors), with the recorded fallback being *"subprocess over their MCP server"* [V]. So the
    sourcehound surface reaches operant as a subprocess, not as in-process MCP — which makes it a
    **subprocess-shaped** payload, and a good first target for the `Term`/`Json` chains.
- **No generic stdin compressor** [V] — `rtk json` requires a `<FILE>`; there is no way to hand it an
  already-in-memory tool result. This alone disqualifies it from the admission chokepoint.
- **The `rtk` crate is not a library**: "rtk is an application, not a library API: every module here is
  private", with `[lib]` existing only so a plugin host can call `rtk::entry()` [V].
- **`rtk-plugin` is a dead end** — BUSL-1.1, `publish = false`, process-global name-unique static
  registration, and delivery is `rtk plugins build`, which **rebuilds and installs a custom `rtk` binary**
  (`rtk` and `rtk-oss` binaries cannot coexist) [V]. There is no dynamic loading and no FFI boundary.
- **Token estimation is `bytes/4`** [V] — AFT's tokenizer is strictly better and is already vendored.

### Measured footprint, and what the published numbers are worth

Measured on the installed v1.0.0 binary, 30 runs [V]: **startup 2.2 ms** (`rtk --version` median; the
documented `<10 ms` budget is met on the clean path), **resident 8.4–9.6 MB**, **binary 14.9 MiB** — about
3× the documented `<5 MB` ceiling, which the repo's own `CLAUDE.md` concedes is tracked against a measured
breakdown rather than a fixed ceiling. Startup is cheap enough for a hook path; the binary is not small.

RTK's coverage is large — **93 top-level subcommands**, 96 rewrite rules → 94 targets, 62 TOML filters —
and its per-rule `savings_pct` spans **20.0 → 99.0**. ⚠️ **Those percentages are static metadata in the
rule table, not measurements** [V]. The only measured floor is RTK's own contribution rule: every filter
must cut **≥20%** on real fixtures. Treat 20–99% as per-family *design upper bounds*, never as a forecast
— the README says so itself: *"a 90% cut in command output is not a 90% cut in cost"* [D].

One cross-project invariant is worth naming explicitly, because it is stated independently in three
places inside RTK and again inside AFT and operant: **fail-open**. Filters fall back to raw execution with
the exit code propagated unchanged; `posthook` never emits unchanged or larger output and swallows every
error; a missing classification fact means "never matches → passthrough" [V]. The evidence layer must be
built on that same floor.

---

## 5. The overlap table (the analytical core)

Per tool family, who covers it today and what the evidence layer adds:

| Tool family (operant surface) | Model-visible cost driver | AFT covers? | RTK terminal covers? | Needed chain (RTK taxonomy) |
|---|---|---|---|---|
| `terminal` / `aft_bash` shell output | huge stdout, progress, dedup | **yes**, in-process, 21 compressors | yes, but a second copy | `Term = [ansi, progress, dedup, base64-elide, cap]` — only when AFT is off |
| `grep`/`rg`/`file_search`/`aft_grep` match sets | thousands of matches | `aft_grep` yes (redirect) | yes | `Matches = [grep-group]` |
| `file_read` / `aft_read` | whole files | no (returns content) | `rtk read` (code filter) | `Read` + code-filter (comments → bodies) |
| `web_fetch` / `web_scrape` / `web_search` | HTML, markdown, JSON APIs | no | no | `Web = [ansi, base64-elide, web-md, toon, md-slim]` |
| `http_request` | JSON envelopes | no | no | `Json = [base64-elide, toon]` |
| MCP tool results (all servers) | arbitrary | no | no | classifier-selected, per MCP server — **and already reaches the chokepoint**: `McpNamespacedTool` implements `OperantTool` and registers into the ordinary registry (`mcp.rs:2295, 2602-2605`) |
| sourcehound surface | subprocess payload, not in-process MCP (`sourcehound_mcp` is held out, `operant-core/Cargo.toml:105-108`) | no | no | `Term`/`Json` by shape |
| `code_execution` stdout | test/build noise | no | partial (its filters) | `Term` + `Json` if structured |
| `sub_agent` / `delegate` | child transcripts | no | no | `Read`/`Json` by shape |
| `file_write`/`patch`/`aft_edit` | small | n/a | n/a | exempt |
| TTS / vision / transcribe / image / video | base64 blobs | no | `base64-elide` partial | must be **exempt** (already guarded: `data:image/:` skip, `compress.rs:145`) |
| `browser` (CDP) | accessibility-tree dumps | no | no | `Term`/`Web` by shape |
| `sourcehound` composites | JSON reports | no | no | `Json` |
| `memory_search`/`session_search`/`kanban`/`sop`/`tool_backend` | small-measured | no | no | `Json` |

**Read the table this way.** Column 4 is nearly empty. That empty column is the entire reason this plan
exists: **RTK uniquely covers the non-bash, non-code tool result**, which in operant is the overwhelming
majority of the model-visible token budget.

### 5.1 Where the tokens actually are (measured in AFT, cited in the recon report)

AFT's own payload shapes give the only hard numbers available today [V]:

| Family | Measured payload | Compression applied by anyone |
|---|---|---|
| `aft_read` | default limit **2 000 lines** (`commands/read.rs:30`), ≈10–25 K tokens | **none** — raw file text |
| `aft_bash` | 16 KiB final cap ≈4 K tokens; 50 KiB raw passthrough | AFT's whitelist (see §3) |
| `aft_inspect` | whole-project aggregate, heaviest single result | tier-2 cache, no size reduction |
| `aft_zoom` | symbol body + context + callgraph annotations | none — verbose by design |
| `aft_grep` | 100 rows/page ≈150–250 tokens | none |
| `aft_search` / `aft_outline` / `aft_glob` | top-10 / one line per symbol / 100 paths | already compact |

**`aft_read` is the headline.** A single 2 000-line file read is ~10–25 K tokens of raw source text, and
*nobody compresses it* — not AFT, not RTK, and certainly not operant's 4 096-char cap, which cuts it to
a head/tail sliver that is usually the wrong sliver. Code filtering (RTK's `FilterLevel` — none →
minimal: strip comments → aggressive: strip bodies, `docs/contributing/ARCHITECTURE.md:308-328`) is a
direct, lossless-tiered answer to exactly this family and it does not exist anywhere in the stack yet.
That is the first thing P2 should ship.

### 5.2 The finding that settles the shell question: two projects, one policy, opposite directions

RTK ships a **command-rewrite table** — 96 rules → 94 distinct `rtk_cmd` targets
(`src/discover/rules.rs:59`; matcher `src/discover/registry.rs:768`). AFT ships a **command-decline table** —
7 rules (`crates/aft/src/bash_rewrite/catalog.rs:9-80`). Align them:

| The model wants to run | RTK rewrites it to | AFT's decision |
|---|---|---|
| `cat f` / `head f` / `tail f` | `rtk read` | **decline** → `aft_read` |
| `grep …` | `rtk grep` | **decline** → `aft_grep` |
| `rg …` | `rtk rg` | **decline** → `aft_grep` |
| `ls` | `rtk ls` | **decline** → `aft_glob` |
| `find` | `rtk find` | **decline** → `aft_glob` |
| `ast-grep` | `rtk ast-grep` | (AFT has `aft_ast_search`) |

**These are the same policy expressed in opposite directions.** Two independent projects concluded that
the model must not run raw `grep`/`cat`/`ls`/`find` — RTK redirects it to a *filtering* wrapper of the
same command, AFT redirects it to its own *semantic* tool. The agreement is the strongest available
evidence for the mechanism. For a code workspace AFT's direction is strictly better, because it returns a
ranked outline rather than filtered lines.

**⇒ Never install both tables in operant. When AFT is present, AFT wins; RTK's rewrite table applies only
on a host with no AFT.** Two tables rewriting the same command is how a policy rots.

RTK's rewrite contract also carries a sharp edge that operant must respect if it ever adopts the table:
`rtk rewrite` exits `0` (rewrite + auto-allow permitted), `1` (no rewrite — excluded, or unattestable
input like `$(…)`, redirects, multi-line blocks), `2` (deny) and `3` (rewrite, but **defer to the host**).
The invariant is security-pinned: a command with no matching rule must exit **3, never 0** — an unruled
command is never auto-allowed [V]. Operant has its own approval surface (`tool_guardrails.rs`,
`approval.rs`, `write_approval.rs`, and the Holon policy fence at `terminal_tool.rs:66`), so **operant
must never honour RTK's exit-0 "auto-allow" verdict**; only exit-3 is compatible, and operant then runs
its own gate on the rewritten command.

---

## 6. The decision: three rules

**Rule 1 — Do not shell out to `rtk` for shell commands, and never install RTK's rewrite table beside
AFT's decline table.** AFT already compresses bash in-process with purpose-built parsers and *declines*
the very commands RTK's table rewrites (`grep`, `cat`, `ls`, `find`, `rg`) — the two tables are the same
policy in opposite directions (§5.2) [V]. Installing both would double-compress, cost a process spawn
(~5–15 ms, [D]) per command, and fork the filter policy.

**Rule 2 — Do not link `rtk-engine` without an owner ruling.** The Additional Use Grant is written
around *distributing* the licensed work inside RTK's own binaries; operant linking it into a third-party
product is outside the plain reading, and the grant explicitly excludes offering it to third parties as a
paid or hosted product [V] **[R]**. *(Unknown U2: whether operant is such a product. Code cannot settle
this — it needs the operator.)*

**Rule 2a — The licence-clean library option is AFT, not RTK.** AFT is **MIT** and publishes its Rust
API as the crate **`agent-file-tools`** (`README.md:14`; `crates/aft/src/lib.rs` declares `pub mod` for
every subsystem, including `compress`) [V]. So operant can depend on AFT's compressors, its Claude
tokenizer, its output caps and its filter registry under a permissive licence, with no BUSL question at
all. RTK's `rtk-engine` has no equivalent affordance. Where the two overlap, **prefer AFT's crate**.
The house precedent supports this too: operant has already ported CortexKit's `magic-context` techniques
natively into `context_management.rs` (and recorded the decision as AD-029) rather than bridging it
[V — AFT recon §6].

**Rule 3 — Build the evidence layer natively, using RTK as the specification.**

Clean-room in operant's own crate, Apache-2.0/compatible, with the two things that actually need to be
shared code paths already present in-repo:

- the format chains (RTK's 5 chains are ~15 lines of policy, `engine/src/methods/mod.rs:40-64`);
- the never-worse guard and panic containment (`engine/src/chain.rs:24-67`, ~25 lines);
- AFT's tokenizer (`crates/aft-tokenizer/src/lib.rs`) as the measuring instrument, vendored via the
  existing bridge or re-derived.

Estimated clean-room cost: **700–1000 lines** of Rust with a method registry, a classifier, five chains,
three lossless transforms and the recall wiring — against **12 380 lines** of AFT shell compressors that
would otherwise be duplicated, and a licence ruling that would otherwise block Option B.

---

## 7. The target architecture

```
tool call
   │
   ▼
execute_tools()                              agent/stream.rs:783          [unchanged]
   │
   ▼
for result in tool_results {                 agent/run.rs:2694            [existing]
   │
   ├─ 1. body = success ? content : error   run.rs:2717-2721             [unchanged]
   ├─ 2. tool-name-recovery prefix          run.rs:2729-2744             [unchanged]
   ├─ 3. redact_sensitive_text_if_enabled   run.rs:2745                  [unchanged, order fixed]
   │
   ├─ 4. ★ EVIDENCE LAYER                   NEW — insert after 2745      [plan §8 P1]
   │        classify(tool_name, content)      → (Subject, ContentFormat)
   │        route(json, policy)               → [(field path, payload)]
   │        for each payload: run_chain(...)  → never-worse + panic-guarded
   │        record provenance marker          → "compressed: grep-group (lossy)"
   │        (lossy only) spill full text      → recall store, keyed by tool_call_id
   │
   ├─ 5. guard_tool_output_spend(...)       run.rs:2746-2748             [unchanged, now measured on the compressed size]
   └─ 6. truncate_tool_result(...)          run.rs:2752-2756             [unchanged, last resort]
}
```

Two ordering decisions worth stating explicitly, because both are load-bearing:

- **Redaction before compression (3 → 4).** Compression is lossy; a recall store is lossless. If either
  ever saw pre-redaction bytes, the redactor's guarantee would be silently voided. The redactor already
  runs first [V]; keep it.
- **Compression before the spend guard (4 → 5).** The guard's job is the *residual* case: the result was
  still enormous after shaping. Measure it on the shaped size, so its marker reports the real residual
  cost, not the pre-compression one. **[R]** The original size should still be reported in the marker —
  capture it before step 4.

`guard_tool_output_spend` then becomes the *last* line of defence, which is what its own doc comment
already says it is for (`compress.rs:699-708`), and `truncate_tool_result` stays the absolute floor.

---

## 8. The plan, phased

Each phase has one observable exit criterion. No phase is merged without it.

### P0 — Instrument, don't change behaviour (1 iteration)

**Goal:** make "token-efficient" measurable, so P1+ can be falsified rather than asserted.

Add a counter at `run.rs:2745` keyed by tool name: `chars`, `estimate_tokens`, `content_format`
(classifier stub), `guard_fired`, `cap_fired`, `recall_written`. Emit as a `tracing` span plus a
session aggregate.

- Exit: a report that ranks operant's tool families by model-visible tokens **on real sessions**.
- Verify: `cargo test --workspace`; a new `operant doctor --tools-tokens` (or extend `doctor.rs`) prints
  the ranking; `scripts/check.sh check --workspace`.
- Cost: ~120 lines. No behavior change.
- **This phase is not optional.** Unknown U3 (the actual distribution) blocks any honest saving claim,
  and the AFT/RTK numbers below are *their* benchmarks, not ours.

### P1 — The evidence layer, behind a flag (1–2 iterations)

**New module** `crates/operant-core/src/agent/evidence.rs`:

```
Subject, ContentFormat, Method, MethodInfo, Block, Provenance, Outcome, Resolution   (RTK's model, clean-room)
classify(tool_name: &str, content: &str) -> Option<(Subject, ContentFormat)>          (the policy table)
route(json: Value, tool_name: &str) -> Vec<(String, Value)>                            (envelope → payload fields)
run_chain(methods: &[&dyn Method], block: &Block) -> String                            (never-worse guarded)
compress(tool_name: &str, content: &str, cfg: &EvidenceConfig) -> String               (the entry point)
```

Wired at `run.rs` between 2745 and 2746. Flag-gated by `compression.enabled = false` by default, so the
landing is behaviour-neutral and A/B-measurable.

- Exit: with the flag on, unit tests prove (a) a `web_fetch` HTML payload shrinks ≥40% with `md-slim`
  lossless, (b) a grep result set groups by file, (c) a JSON MCP result TOON-encodes, (d) nothing grows
  (`never_worse`), (e) a panicking method falls back to the original text, (f) `data:image/` payloads are
  untouched.
- Verify: `cargo test --workspace`, clippy `-D warnings`, plus a mutation check — disable one method in
  the chain and confirm its golden test fails.

### P2 — Method set and chains

Chains exactly as RTK defines them (§4 table). Priorities:

1. **Lossless first** — `ansi`, `progress` (strip), `dedup`, `minify-json`, `md-slim`,
   `strip-line-numbers`, `base64-elide`, `shape`, `rle`. These are the safe default: no recall needed,
   no information the model can ask for is destroyed.
2. **Recoverable-lossy second** — `grep-group` (Matches), `lockfile` (Lockfile), `cap` (Term, tail-biased
   because errors live at the end), `truncate` (last resort), `web-md`, `toon`. Each requires the recall
   backstop from P3 before it may be enabled by default.
3. **Per-tool field routing** — for `terminal`/`code_execution`, the payload is
   `{exit_code, stdout, stderr, runtime}` (`terminal_tool.rs:98-107`, error envelope at `115-122`);
   route `stdout` through the `Term` chain and leave `stderr`, `exit_code` and `runtime` intact **[R]**.

- Exit: golden fixtures per (Subject, ContentFormat) pair, plus the invariant that `json` payloads stay
  parseable after a chain runs.

### P3 — Lossless recovery (the safety net that makes lossy legal)

RTK's `Recovery::Recall` contract demands a lossy method declare how the full text comes back
(`engine/src/method.rs:33-39`) [V].

**Three independent precedents agree on the answer**, which is why this phase is cheap:

| Project | Mechanism | Retrieval |
|---|---|---|
| RTK | content-addressed SQLite `~/.local/share/rtk/recall.db`; blob = full raw child stdout+stderr, gzip; hash = `SHA256(command‖0x00‖content)` truncated to 16 hex; 10 MiB/entry, 200 entries, 30-day retention [V] | `rtk recall <hash>` with unique-prefix resolution; **default returns only the elided portion** |
| AFT | retained artifact on disk + a printed pointer `[JSON output <kb> KB; full output: read "<path>"]` (`bash_background/output.rs:300-306`) [V] | `bash_status` chunked: `output_offset` → `output_chunk_base64` + `output_next_offset` (`commands/bash_status.rs:31, 111-127`), persisting across restarts |
| Operant (backing store) | the **LCM DAG** (`context/lcm.rs` — persisted, lossless, sqlite, FTS5 + `local:hash`/remote vectors) [V — `docs/OPERANT_LCM_AFT_AUDIT.md:48-68`] | `lcm_recall` / `lcm_recall_round`, already model-addressable |

**So the design is settled by precedent, not invention.** Adopt the *shape* — a compressed rendering that
names what was removed and how to get it back, over a content-addressed key, with chunked retrieval and a
retention policy. Operant's key should be `(session_id, tool_call_id)` rather than RTK's
command-scoped hash, because operant already correlates results by `tool_call_id` everywhere
(`run.rs:2717` onwards).

One fidelity rule to pin as a test: RTK builds the stored blob *"from the same formatted values
displayed"* [V], so recall never returns bytes the user could never have seen.

When a lossy method fires: spill the pre-compression text into the LCM DAG and print the pointer:

```
[compressed: cap, term — 12 431 tokens → 3 204. Full output: `lcm_recall "<tool_call_id>"`]
```

- Exit: for every lossy method enabled by default, there is a test that a recall call returns the exact
  original bytes; and no lossy method is enabled while its recall is unwired.
- This is the invariant that lets compression be aggressive without ever trapping the model.

### P4 — Tool-family coverage

Work down the §5 table. For each family: classifier entry, format, chain, exemption list. Explicit
exemptions: any payload carrying `data:image/`, `data:audio/`, or base64 blobs (the existing
`fast_trim` skip at `compress.rs:145` is the precedent, and the terminal tool's `runtime` field must not
be reformatted).

- Exit: every name in `builtin_tool_names()` (`tools/builtin.rs:275-320`), every AFT tool
  (`tools/aft_tools.rs:36-90`) and the MCP bridge (`tools/tool_search.rs`) has a deliberate classification
  or a tested exemption. A test asserts the classifier table covers the full registry, so a newly added
  tool cannot silently fall through.

### P5 — The shell path, decided once

Three operating modes, chosen by config, default to the safest:

- `compression.shell = "aft"` (**default**, and the only mode worth shipping): `terminal` results are
  either routed to `aft_bash` or marked "AFT-owned" so the evidence layer skips them [R].
- `compression.shell = "native"`: the evidence layer applies the `Term` chain to `stdout`. Note the
  incremental value over AFT is *not* ANSI+dedup — AFT's tier-5 already does that — it is `progress`,
  `base64-elide` and `cap`, which AFT does not have [V].
- `compression.shell = "rtk"` (**opt-in only**, for hosts without AFT): invokes the `rtk` binary as the
  executor for the terminal tool (`rtk <command>`), inheriting its never-worse and passthrough guarantees
  [D]. This is the one defensible place to run RTK: it is a *backend substitution*, not a second filter on
  top of AFT's. Two constraints: the binary is **14.9 MiB** on disk and ~9 MB resident [V], so it is a
  once-per-host cost, not a per-call one; and **do not use `rtk rewrite` as the decision path** — its
  exit-0 "auto-allow" verdict must never bypass operant's own approval gate (§5.2). If a rewrite table is
  wanted, port only the *mapping*, and run operant's gate on the rewritten command.

There is a fourth option the AFT licence now makes attractive **[R]**: depend on the MIT crate
`agent-file-tools` and call AFT's `compress::` dispatch **in-process** — 20 compressors, the TOML filter
registry, the Claude tokenizer and the chunked output caps, with no BUSL and no process spawn. For any
host that already bridges AFT (which is every operant host with `aft_enabled`), this is strictly better
than both `native` and `rtk`. It is also the option that matches operant's existing precedent of porting
CortexKit techniques natively rather than bridging them.

- Exit: the modes are tested; mode selection is documented in `operant.example.toml`; and there is
  a test that AFT and the evidence layer never both process the same result.

### P6 — Observability and the operator surface

Extend the P0 report into `operant doctor --tools-tokens` and a per-turn summary: per family, tokens in /
out / saved, method hit rates, `never_worse` rejections, recall hits, and the residual spend-guard fires.
Config lands in `operant.example.toml` and `README.md`/`CHANGELOG.md` per the repo's own rules.

- Exit: the report runs on a real session and shows a per-family saving; the operator can disable any
  method without a rebuild.

### Config schema (corrected 2026-11-13 against `EvidenceConfig`, `crates/operant-core/src/agent/evidence/mod.rs:63-76`)

The 2026-11-12 version of this block documented four keys that do not exist, and
spelled the family table wrong. `EvidenceConfig` has exactly these fields:
`enabled`, `min_size`, `shell`, and `families.<subject>.enabled` /
`families.<subject>.chain`.

```toml
[compression]
enabled = false              # the layer is OFF by default; this is the opt-in
shell = "aft"                # "aft" | "native" | "rtk" — who owns shell output
min_size = 512               # BYTES; below this, compression is a no-op

[compression.families.web]   # one table per subject; the key is the subject name
enabled = true               # when present, this decides — it is not limited by the master switch
chain = "web"                # ONE named chain: json, web, matches, lockfile, term,
                             # code, code-aggressive, plain, lossless
```

**Keys this plan named that were never built:**

- `never_worse` — never a key. The guarantee is hardcoded (`crates/operant-core/src/agent/evidence/mod.rs:230-233`), not configurable.
- `policy = "default"` — never built. There is no method-chain preset key; the chain comes from the format, and a family's `chain` overrides it.
- `tool_result_retrim_chars` — never built under `[compression]`, and not a TOML key at all. It is a field of `PreflightConfig` (`crates/operant-core/src/agent/compress.rs:45`, default 2000), which derives no serde — the code does not read it from a config file.
- `[compression.family.web]` — the spelling is `families` (plural), not `family`, and `chain` is a STRING naming one chain, not a list of chain names.

`EvidenceConfig` carries no `deny_unknown_fields`, so an operator who copies the
old block gets a config that loads cleanly with those keys silently dropped.

---

## 9. Failure modes and their guards

| Failure | Why it happens | Guard |
|---|---|---|
| Compression destroys the failure detail the model needed | chains run on the whole payload | `cap` is **tail-biased** (AFT and RTK both place errors at the end) [V]; the router never routes `stderr`/`error` fields; failure paths are exempt below `min_size` |
| Compression breaks a tool's JSON contract | chains re-encode | only chains declaring `produces: Format(..)` may re-encode; a test asserts the result still parses as JSON. Operant already has this discipline (`agent/mod.rs:1157-1189`) |
| Compression breaks the `tool_call_id` envelope | the envelope is JSON too | the router operates **inside** the envelope, on `content` — never on the envelope (precedent: `truncate_tool_content`, `compress.rs:160-170`) |
| Compression redacts wrong / leaks | order at the chokepoint | redactor runs before the compressor [V]; the recall store is written post-redaction only |
| Prompt-cache thrash | compressing mid-conversation changes prefixes | compress **before** the result joins the message list — which the chokepoint satisfies [R]; RTK makes the same guarantee for its own hook path [D] |
| Double compression (AFT + evidence layer) | both active | P5 marks AFT-owned results so the layer skips them; a test asserts exclusivity |
| Recall store grows unboundedly | every lossy result spills | spill only above `min_size`; reuse LCM's existing maintenance/retention |
| A new tool silently loses its output | no classifier entry | P4's coverage test over the live registry fails loudly |
| Compression helps latency but hides the answer | shaping is lossy | P3's recall wiring is a **precondition** for enabling lossy methods, not a follow-up |

---

## 10. Rejected alternatives (with the reason each was rejected)

| Alternative | Why rejected |
|---|---|
| `rtk <cmd>` from the terminal tool (the obvious plan) | AFT already compresses bash in-process with 12 380 lines and redirects grep/cat/ls to semantic tools [V]; double compression, +5–15 ms/command, two filter tables that drift |
| `rtk-engine` as a Cargo dependency | BUSL Additional Use Grant is written around RTK's own distributed binaries and excludes offering it to third parties as a paid/hosted product [V]; needs an owner ruling (unknown U2). Clean-room is cheaper and unblocked |
| Drive the `rtk` binary per tool result at the chokepoint | no stdin compressor exists [V]; one process spawn per result; hundreds of results per turn |
| `rtk-proxy` (provider-API interceptor) | Operant *is* the API client and already builds its own requests (`fitted_request`, `compress.rs:621-697`); a second hop would break the prompt cache, add a BUSL daemon, and require RTK to understand every provider operant speaks |
| RTK as an MCP server | RTK has no MCP server mode — confirmed exhaustively (§4) [V] |
| Install RTK's rewrite table beside AFT's decline table | the same policy twice, in opposite directions (§5.2); two tables rewriting one command is how a policy rots [V] |
| Use `rtk rewrite`'s exit-0 "auto-allow" to skip operant's approval | would bypass operant's own guardrails, approval surface and Holon policy fence (`terminal_tool.rs:66`) — RTK's own invariant is that an unruled command exits 3, never 0 [V] |
| Ship an `rtk-plugin` from operant | BUSL-1.1, `publish=false`, no dynamic loading, and delivery is a **rebuilt `rtk` binary** that cannot coexist with the installed one (§4) [V] — dead as an integration path |
| Adopt RTK's pipeline TOML file verbatim | it is built for hook positions (`before_tool`/`after_tool`/`request`) operant does not have; adopt the *shape* (rule = when + chain + guards) and express it in operant's own config |
| Rewrite AFT's shell compressors into the evidence layer | out of scope and duplicated work; AFT already owns it and is the better owner |
| Make the evidence layer a tool the model calls | adds a model round-trip per result; the cost it saves is paid back in tool-call tokens |

---

## 11. Token economics

**Every number in this section is a target, not a measurement.** Operant's own distribution is unknown
(U3) — which is what P0 exists to fix. The figures below are **AFT's measured benchmark numbers**
(`benchmarks/compression-tokens/REPORT.md`, Claude `ai-tokenizer` counts, 26 realistic bash fixtures,
11 324 tokens saved overall ≈ 435 tokens/fixture) and **RTK's published per-command reductions** [D].
They bound what is plausible; they are not operant projections.

**AFT's measured wins** — the whitelist working as designed:

| Fixture | Tokens in → out | Reduction |
|---|---|---|
| `docker ps` | 2 426 → 131 | −95% |
| `biome check .` | 262 → 16 | −94% |
| `git push` | 195 → 31 | −84% |
| `cargo test` | 434 → 77 | −82% |
| `journalctl -u aft-worker -n 80` | 5 694 → 1 334 | −77% |
| `tree -a -L3` | 1 706 → 938 | −45% |
| `find . -maxdepth 3` | 2 550 → 1 707 | −33% |
| `ls -la` | 3 771 → 2 904 | −23% |
| `git status --short`, `git log --oneline`, `docker-compose`-like, `helm list` | **0 tokens saved** | **0%** |

**AFT's measured failures are the opening for this plan.** The bottom four rows are not a
one-off: they are the *generic* tier, which is ANSI-strip + consecutive-dedup only
(`compress/generic.rs:3`). A 23% cut on `ls -la` and a 33% cut on `find` are the generic tier's ceiling.
RTK's Term chain adds `progress` (strip), `base64-elide` and `cap` on top of ANSI+dedup, and its
`Matches`/`Json`/`Web` chains do not exist in AFT at all.

Proposed per-family targets, and their basis:

| Family | Mechanism today | After P2–P4 | Basis |
|---|---|---|---|
| `aft_read` / `file_read` | 4 096-cap head/tail on ~10–25 K tokens of raw source | **code filtering** (none → minimal: strip comments → aggressive: strip bodies), lossless tiers | **[R]** — RTK's code-filter strategy [D]; the single largest measured gap (§5.1) |
| `web_fetch`/`web_scrape`/`web_search` | 4 096-cap | `web-md` + `md-slim` (lossless) then `toon` on JSON APIs | [R] — HTML/markdown boilerplate dominates; RTK's Web chain [V] |
| `grep`/`file_search`/`aft_grep` | 4 096-cap | `grep-group` by file with per-file counts | [R]; RTK's Matches chain [V] |
| MCP results | 4 096-cap | classifier-selected `Json`/`Web` chain per server | [R] |
| `code_execution`/`terminal` (native mode) | 4 096-cap | `Term` chain: ansi + progress + dedup + base64-elide + cap | [R]; RTK's Term chain [V]; AFT's generic tier is only ansi+dedup [V] |
| `terminal`/`aft_bash` (default mode) | **AFT's whitelist, unchanged** | unchanged — plus RTK-style generic tier **only for commands AFT's whitelist misses** | [V] AFT owns the known set; the 0%-saving rows are exactly the gap |
| `sub_agent`, `sourcehound`, `browser` CDP | 4 096-cap | classifier-selected chain | [R] |
| Per-result worst case | `max_tool_result_share` = 5% of window [V] | same bound, reached far less often | [R] |

**The honest headline:** the plan does not promise a percentage, because there is no operant baseline. It
promises a *mechanism* — shape-aware, contract-declared, never-worse, recoverably-lossy compression at the
single point where a tool result enters the context — and it makes the percentage measurable in P0. The
one number worth betting on is the one with the widest gap between what is spent and what is needed:
**`aft_read` at ~10–25 K tokens per call, currently uncompressed.**

---

## 12. What I did not check

- *Resolved during this work:* **RTK's recall-store internals** — it is a content-addressed SQLite store
  at `~/.local/share/rtk/recall.db`, gzip blobs of the full raw child stdout+stderr, command-scoped
  16-hex hash, 10 MiB/entry × 200 entries × 30-day retention, chunked retrieval defaulting to the elided
  portion (§4, P3) [V].
- *Resolved during this work:* **operant's MCP substrate.** The `rmcp` crate in `Cargo.lock` is
  **transitive through `memory-wire` only** and is imported nowhere in operant's source; operant's real
  MCP client is its own 3 944-line `crates/operant-core/src/mcp.rs`, whose namespaced tools already
  register into the ordinary `ToolRegistry` (§4) [V]. The evidence layer therefore reaches MCP with no
  MCP-specific work.
- RTK's `rewrite` table contents and `hook` JSON schema — verified at file:line (§5.2), but **not**
  exercised against a live `rtk` binary; the 96-rule table is read from source, not from `rtk rewrite`
  output.
- Whether operant is a "paid or hosted product or service" under RTK's grant (U2) — an operator decision
  that decides whether `rtk-engine` may ever be linked. The recommended path (clean-room + AFT's MIT
  crate) does not depend on the answer.
- RTK's release cadence between the repo's `CHANGELOG.md` (stops at 0.49.0, 2026-09-11) and the installed
  binary (1.0.0) — the discrepancy is verified, what sits between is unknown.
- RTK's issue-tracker themes and third-party adopter counts — the GitHub API was rate-limited from this
  host; the only adoption evidence is RTK's own marketing assets.
- No Operant source was modified; the peer WIP present in the working tree was left untouched.
  **Corrected 2026-11-13:** this was true when the plan was written and is false now. The
  evidence layer landed in `9d163951` (iter-2120), was deleted again in `5244ac70` (iter-2123),
  and landed on the chokepoint in `9f723e88` (iter-2139). See the landing table at the top.

### Two operational notes for whoever picks this up

1. **The tree currently does not compile, for reasons unrelated to this plan.** A concurrent sibling agent
   is mid-build and its working tree has a compile error at `crates/operant-core/src/tools/web_tools.rs:274`
   — `read_bounded_body` is private in `crates/operant-core/src/tools/http_tool.rs:287`. This is peer WIP
   and was deliberately not touched. **P0 cannot be measured until that build is green.**
   **Corrected 2026-11-13:** the tree compiles, and P0 IS measured — `operant doctor --tools-tokens`
   ranks the tool families (`crates/operant-cli/src/cmd_doctor/checks_tools.rs:1155`). The note above
   is kept as the 2026-11-12 snapshot.
2. **`.tmpwork/` is not a safe place for deliverables.** It was wiped mid-task by a concurrent agent using
   it for a Rust build, taking a cloned source tree and a research report with it. Durable artefacts
   belonging to this plan live in `_kernel/rtk-integration/`; use `.tmpwork/` only for throwaway scratch.

---

## 13. The one-paragraph answer to the actual question

Operant's agentic loop is strong at intent (lazy tool disclosure), execution (amortised bridges) and
curation (lossless LCM), and weak at evidence — the moment a tool result becomes text. That weakness is
a size-only 4,096-char cap and a 5%-of-window share guard. RTK's contribution is not its filters, which
AFT already duplicates and exceeds in-process for the shell path; it is RTK's *abstraction*: classify a
result by `(Subject, ContentFormat)`, run a format-appropriate method chain behind a `never-worse` guard,
and declare every lossy step recoverable. Building that evidence layer natively at
`agent/run.rs:2745` — after redaction, before the spend guard — covers every tool family AFT and RTK do
not touch (file reads at ~10–25 K tokens each and currently uncompressed, grep result sets, web fetch,
MCP JSON, subagent output, browser dumps) and needs no BUSL dependency. AFT keeps bash and code
understanding, and its MIT crate `agent-file-tools` is the licence-clean way to borrow its compressors
and tokenizer in-process; RTK supplies the taxonomy and the occasional generic-tier backend for hosts
without AFT; operant gets a token-efficient tool loop that can be measured in P0 and falsified in every
phase after it.

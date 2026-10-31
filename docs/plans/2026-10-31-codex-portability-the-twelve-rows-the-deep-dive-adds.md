# codex portability — the twelve rows the deep-dive adds

Dated-frozen: published `2026-10-31`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Mechanism

**Supersedes nothing. Scopes the residue of**
`docs/audit/2026-10-31-codex-architecture-deepen/00-INDEX.md`.

The 2026-10-30 ledger audited four families and closed them; the 2026-10-31 plan executed its
residue (4 shipped, 2 refuted with measurement). This plan is the build list for the **eight
families nobody had measured**, ranked by value-per-line.

**The rule this plan applies:** import only when the portable unit is small enough that copying
beats rebuilding, the gap is measurable, and the dependencies are already carried. That rule turned
a 122-crate, 2.2M-LOC reference into **twelve rows totalling ~2,900 LOC of operant change and zero
new production dependencies**.

### What is already done (do not re-do)

Verified shipped, not "in progress": the sandbox phantom-comment fix (iter-1504), tool-search
relevance ranking (iter-1510), tool-call duration/trace payload (iter-1506/2cd1a0e2),
`ToolExposure` (tools.rs), the context-budget tail line (iter-1545). The MCP schema byte cap and
token-budget compaction are REFUTED — see the index's honesty notes.

### The rows, in execution order

#### 1. Row 4+10 — the command-safety interpreter unwrap (S, ~40 LOC, 0 deps)

**Codex source:** `shell-command/src/bash.rs:106-119` (`extract_bash_command`, 14 LOC) and
`is_dangerous_command.rs:146-155` (sudo/env unwrap, ~20 LOC).

**Operant today:** `blast_radius.rs:22` names indirection through an interpreter as deliberately
unmodelled, and `segments()` (`:596-624`) therefore sees the verb `bash` for `bash -c "rm -rf /etc"`
and returns `None`. The flag-permutation hole is ALREADY closed — `has_short_flag` (`:200-205`) is
the same flag-cluster logic codex has, and `screen()` runs from `approval.rs:662`.

**This is the one high-severity gap the contrast found in the safety surface, and it is 34 lines.**

- **Acceptance:** `bash -c`/`sh -c`/`zsh -lc "…"` unwraps to the inner script and the inner command
  is classified; `sudo`/`env`/`trap` wrappers are skipped to the verb beneath them; a script that
  cannot be unwrapped **fails closed** (classified as the conservative tier, not `None`).
- **Mutation guard:** revert the unwrap and a test driving `bash -c "rm -rf /etc"` must move from a
  refusal to `None` — fail by name.
- **Do NOT add tree-sitter.** Three deps to close what 34 lines of `String` work closes.

#### 2. Row 7 — process hardening: no core dumps, no ptrace (S, ~45 LOC, 0 deps)

**Codex source:** `process-hardening/src/lib.rs:65-75` (prctl) and `:99-115` (RLIMIT_CORE).

**Operant today:** `libc = "0.2"` is already a dependency of `operant-cli`
(`crates/operant-cli/Cargo.toml:73`). No `setrlimit`/`prctl` call exists anywhere in the tree.

**Why it matters now:** the repo already paid for the file side of this exact threat (auth.json and
.env at 0600, RT-154). A core dump is the same credential through a different channel.

- **Acceptance:** `RLIMIT_CORE = {0,0}` and `PR_SET_DUMPABLE = 0` are set at the top of `main.rs`;
  a test asserts the rlimit applies (fork-free: read it back with `getrlimit`).
- **Deliberate divergence, to be recorded in the code:** codex `exit()`s when a hardening call
  fails. operant must **log and continue** — a container runtime that forbids `PR_SET_DUMPABLE`
  would otherwise make the binary unusable for a non-security reason.
- **DO NOT port the `LD_*` strip.** Measured: `ldd target/release/operant` resolves
  `libonnxruntime.so.1` from `~/.local/lib/operant/`, so `LD_LIBRARY_PATH` is load-bearing for the
  Kokoro TTS default. The narrow safe version (stripping only `LD_PRELOAD`) needs its own
  measurement first.

#### 3. Row 8 — exclusive turn attribution (S, ~60 LOC, 0 deps)

**Codex source:** `utils/critical-path/src/lib.rs` (181) + `core/src/turn_timing/report.rs:4-25` +
the three-state `TurnCriticalPath` in `analytics/src/turn_critical_path.rs:1-2`.

**Operant today:** `llm_duration` (`run.rs:1234`), per-tool durations (`stream.rs:1638-1768`), and
turn total (`run.rs:1705`) are all measured and emitted — but they are **inclusive and overlapping**.
No field exists for `UserWait` or for the residual.

- **Acceptance:** a per-turn struct with `Inference/Tools/UserWait/Overhead` as **exclusive**
  intervals summing to the turn total, exposed on the existing `<turn_context>` tail (the same
  mechanism the context-budget line rides at `run.rs:2965-2978`).
- **The state model is the actual import:** `{Disabled, Incomplete, Complete}` — never emit a
  duration that was not captured. RT-199 was a `Duration::from_millis(0)` that lived undetected
  because a `u64` cannot distinguish zero from unmeasured.
- **Mutation guard:** the naive implementation **double-counts** — summing per-tool durations plus
  inter-tool gaps plus LLM latency exceeds the turn interval, so the operator sees >100%. Force
  that by making one category count twice, and the sum check must fail.
- **Mutation guard 2:** make the report a `u64` and assert it loses the `Incomplete` state.

#### 4. Row 5+6 — the native patch engine (M, ~405 LOC, 0 deps)

**Codex source:** `apply-patch/src/seek_sequence.rs` (164), `text_file.rs` (121) — both portable
**as-is, zero imports** — and `file_update.rs:60-160` (`compute_replacements`, ~120 core LOC after
stripping `similar` + exec-server).

**Operant today:** the `*** Begin Patch` grammar already reaches the model through
`AftApplyPatchTool` (`aft_tools.rs:615-668`), which path-scans for the write guard then forwards to
the subprocess. When the bridge is dead, native `patch` (`patch_tool.rs:20-29`) is **one
whole-file find/replace** and `matches().count()` then `content.replace()` rewrites every
occurrence (defect D1). `normalize_whitespace` (`:34-39`) is one tolerance tier, no EOF anchor, no
Unicode normalization, and `:95-99` is a no-op branch (defect D2).

- **Acceptance:**
  - `seek_sequence` + `text_file` + `compute_replacements` port as the native engine.
  - `patch` takes an internal multi-chunk path so "edit 5 places" is one call, not five.
  - **D1 fixed:** a `find` matching more than once is refused with the occurrence count, unless an
    explicit occurrence index is given. Regression test: a two-occurrence `find` with no index must
    fail loudly, not rewrite both.
  - **D2 fixed:** the dead branch is deleted and the behaviour it pretended to have is tested.
  - CRLF fidelity preserved (the `LineEnding` model), which `.lines()`/`replace` drops today.
- **Order discipline:** fix the defects **inside** the port, not after it, so the mutation guards
  prove both at once.
- **Do NOT port `parser.rs`/`streaming_parser.rs`/`invocation.rs`.** The grammar is already
  agent-reachable through aft; a second agent-facing surface is two ways to say one thing.
  Streaming is dead behind a test-only feature flag.

#### 5. Row 3 — the mermaid renderer (M, ~1,600 LOC, 0 new deps) — CONDITIONAL

**Codex source:** `mermaid/` 1,498 non-test LOC, one dependency (`unicode-width`, already at
`crates/operant-cli/Cargo.toml:47`), plus `tui/src/markdown_render/mermaid.rs` (91) as the
integration template.

**Operant today:** `native_image_protocol_available() -> false`
(`markdown_mermaid_fallback.rs:41`) gates every mermaid block into a plain highlighted code block
labelled `mermaid`. The branch sites exist (`markdown_render_full.rs:423`, `:993`). The W9 cut was
of a **10,215-LOC compiler** — 6.8× larger than what is actually available.

- **Acceptance:** the renderer ports; `should_render_mermaid_block` gains a native-text path; every
  `RenderError` degrades to the original highlighted source **with a specific reason**
  (unsupported vs too-wide vs over-limit) — that is codex's own contract, not a nicety.
- **BLOCKED ON ONE OPERATOR ANSWER:** W9 said "only if diagrams-in-transcript are a real
  requirement". No transcript count exists in this repo. **This row does not start until that
  question is answered.** It is listed so the W9 reasoning is not inherited unchallenged, not to
  pre-empt the operator.
- **D3 fixed by this row:** the four unreachable `RenderResult::Error` sites and the
  pinned_images producer gap get a real path or get deleted.

#### 6. Rows 1+2+9+12 — hook decisions (L, ~550 LOC, 0 deps) — ARCHITECTURE DECISION

**Codex source:** `hooks/src/engine/output_parser.rs` (617, use ~250), `matcher.rs` (49),
per-event input structs (~150), `hooks/src/lib.rs:29-44` (~40),
`discovery.rs:390-451` (~60).

**Operant today:** `shell_hooks.rs` has 4 of 14 events, 4 flat env vars, **no stdin**, **no
matcher**, and stdout logged at debug only. `HookHandler` returns `()` and `emit()` is
fire-and-forget (`gateway_pipeline.rs:154-162`, `:211-236`). Of codex's 8 output decisions,
operant implements **0**.

- **Acceptance:**
  - The hook command receives the event as a JSON document on stdin.
  - `decision: block` + mandatory `reason` can refuse a tool call; `updatedInput` can rewrite its
    arguments; `additionalContext` injects a developer message. Those three are the ones that
    change the run — start with them.
  - A matcher scopes a hook to named tools/triggers.
  - **`emit()` gains a SEPARATE drainable path**, not a signature change — the kernel
    `HookSlot` bridge depends on the existing fire-and-forget arm.
- **This row is last and it needs the operator's agreement**, because it changes what a hook can
  *do*, and an operator-configurable command that can rewrite tool arguments is a security
  decision, not a porting one.

#### 7. Row 11 — skill tool dependencies (S, ~40 LOC, 0 deps)

**Codex source:** `skills/src/model.rs:81-95` (15 LOC).

**Operant today:** `prerequisites_env`/`prerequisites_commands` + `is_available` (`skills.rs:18-27`,
`:218-245`) gate on env and commands. Tool availability exists on the registry
(`tools.rs:325`, called at `:640`, `:655`, `:686`, `:728`). Nothing declares a tool.

- **Acceptance:** a `prerequisites_tools` frontmatter field; invoking a skill whose declared tool
  is unavailable is refused with the tool named; a regression test invokes a skill declaring a
  nonexistent tool and asserts the refusal names it.
- **Do NOT take `transport`/`command`/`url`/`oauth_callback_port`** — those exist for the
  connectors crate, which this ledger has already rejected.

### Explicitly NOT in this plan

| Not doing | Why | Row |
|---|---|---|
| LLM memory extraction (2 phases) | a premise divergence: costs tokens + determinism to buy curation operant deliberately does not want | 04-memory |
| tree-sitter shell parsing | 3 deps for a boundary 34 lines of `String` work closes | 02 |
| The `LD_*` env strip | measured: breaks the Kokoro default. Only `LD_PRELOAD` could be safe, and that needs its own measurement | 07 |
| `$skill` sigil / `skill://` URIs | `/skill name` already works — two syntaxes for one action is a redundancy | 03 |
| Streaming patch parser | dead behind a test-only feature flag; operant streams tool *results*, not arguments | 05 |
| Cross-file patch rollback | codex has none either. Apply in order, stop at the first failure, `atomic_write` per file | 05 |
| Realtime voice / WebRTC | operant's interactive voice was deleted with zero callers (iter-1420) | 00 |
| Connectors / app catalog | 5,491 LOC for a problem operant's 3-transport MCP client already answers | 00 |

## Gates / success criteria

(fixed before the run)

Per row, one iteration, each with a **mutation-proven** guard: the mutation must break the
behaviour the test claims and restoring it must be green.

Repo gates: `cargo fmt --all`; `./scripts/check.sh check -p <crate>`; the touched crate's tests;
`bash scripts/clippy-warning-gate.sh`; release build + deploy to both install paths;
`git log origin/main -1` confirms the push.

**Known red gate carried into every step:** RT-203 (opened iter-1523) is an unidentified
`operant-core --lib` parallel-load flake. It is delta-proven not-ours. If the gate goes red on an
unrelated test name, capture it and continue; do not fix a test this plan does not own.

## Cost

| Row | Added | Deleted | New deps |
|---|---|---|---|
| 1+10 interpreter unwrap | ~40 | 0 | 0 |
| 7 process hardening | ~45 | 0 | 0 |
| 8 turn attribution | ~60 | 0 | 0 |
| 5+6 patch engine | ~405 | ~90 | 0 |
| 3 mermaid (conditional) | ~1,600 | ~15 | 0 |
| 1+2+9+12 hooks | ~550 | 0 | 0 |
| 11 skill deps | ~40 | 0 | 0 |
| **total** | **~2,740** | **~105** | **0** |

Seven iterations, one deploy each, **zero new dependencies**. The largest row (mermaid) is the one
gated on an operator answer.

# codex-architecture-deepen — the execution outline (what we execute next)

Dated-frozen: published `2026-10-31`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Contents

- [What this document is](#what-this-document-is)
  - [What re-measurement changed (read this first)](#what-re-measurement-changed-read-this-first)
  - [What this document deliberately does NOT decide](#what-this-document-deliberately-does-not-decide)
- [The execution order](#the-execution-order)
- [Row 1 — the command-safety interpreter unwrap](#row-1--the-command-safety-interpreter-unwrap)
- [Row 2 — process hardening: no core dumps, no ptrace](#row-2--process-hardening-no-core-dumps-no-ptrace)
- [Row 3 — exclusive turn attribution](#row-3--exclusive-turn-attribution)
- [Row 4 — the native patch engine (+ D1, D2)](#row-4--the-native-patch-engine--d1-d2)
- [Row 5 — mermaid (BLOCKED — one operator answer needed)](#row-5--mermaid-blocked--one-operator-answer-needed)
- [Row 6 — hook decisions (BLOCKED — operator agreement on a security posture)](#row-6--hook-decisions-blocked--operator-agreement-on-a-security-posture)
- [Row 7 — skill tool prerequisites](#row-7--skill-tool-prerequisites)
- [Per-iteration gate (unchanged, applies to all five)](#per-iteration-gate-unchanged-applies-to-all-five)
- [Cost, re-measured](#cost-re-measured)
- [What this outline refuses to do](#what-this-outline-refuses-to-do)

## What this document is

The build list `docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md` was
written the same day as the audit that produced it, from the audit's measurements. This is the
**execution outline**: every row of that plan re-measured against the live tree today, with the
exact edit sites, the test each iteration must add, and the mutation guard that proves it.

**Why a second document instead of editing the plan:** the plan is dated-frozen history of what the
audit ranked. This file carries the newer verified state. Where the two disagree, this file is
right; the frozen plan keeps its original ranking.

### What re-measurement changed (read this first)

Three claims in the frozen plan did not survive contact with the tree. Each correction is recorded
rather than deleted, so the next reader can see what was wrong and why:

| # | Frozen plan said | Measured today | Effect |
|---|---|---|---|
| C1 | "operant has **4 of 14 events**"; "codex's **8 output decisions**, operant implements 0" | codex has **11** events (`HookEventNameWire`, `hooks/src/schema.rs:103-126`). The **8 decisions** are real but the taxonomy is not flat: 2 live on the universal output (`continue`, `stopReason`) + 2 universal fields (`suppressOutput`, `systemMessage`) + 6 on `pre-tool-use.command.output` (`decision`, `reason`, `permissionDecision`, `permissionDecisionReason`, `updatedInput`, `additionalContext`). operant implements 0 of all 12. | Row 7's scope statement is **corrected and slightly enlarged**. The "4 of 14" phrasing was directionally right and numerically wrong. |
| C2 | Row 7's discovery source is "`discovery.rs:390-451` (~60 LOC)" | `hooks/src/engine/discovery.rs` is **1,963 LOC** total, and the file carries config-rule keys, plugin trust and plugin fingerprints (`plugin_trust.rs`, `plugin_fingerprint.rs` are separate files). The 60-LOC slice was a line-range guess from an outline view. | Row 7's **`hooks.json` discovery is re-scoped down to operant's existing `shell_hooks.rs` directory loader** (which already parses a hooks dir) rather than importing discovery.rs. Recorded in the row. |
| C3 | Row 4's defect D1: "`matches().count()` then `content.replace()` rewrites every occurrence" | The code uses `content.matches(&args.find).count()` — the same thing — and the count is **reported back** as `replacements` in the tool result (`patch_tool.rs:172-176`, `:203`). So the behavior is *visible to the model*, not silent. | **D1 is downgraded from defect to design question.** The row keeps the multi-occurrence **refusal** (codex's contract) but the regression test now asserts the *refusal*, not "silent corruption". D1's severity note in the audit stays as-written (med) with this headnote. |

**One row was re-verified and CONFIRMED with no change:** the interpreter unwrap.
`segments()` (`blast_radius.rs:597-622`) splits on `\n`, `;`, `&&`, `||`, `|` and never looks inside
a quoted argument, so `bash -c "rm -rf /etc"` is one segment whose first word is `bash`, which
`rate_segment` does not know — `BlastRadius::None`. The module doc at `:22` names this exact case
as deliberately unmodelled. This is the real hole and it is unfixed today.

### What this document deliberately does NOT decide

Rows 5 (mermaid) and 7 (hooks) are operator decisions, not porting decisions. This outline marks
them BLOCKED and says in one line each what the operator is being asked. Everything else is
executable now.

---

## The execution order

Ordered by risk-adjusted value-per-line: the two cheap safety rows first (highest value per line,
lowest blast radius of *change*), then attribution, then the patch engine, then the two gated rows.

| # | Row | Plan ref | LOC | Deps | Crate | Status |
|---|---|---|---|---|---|---|
| 1 | Interpreter unwrap (command safety) | §1 (Row 4+10) | ~40 | 0 | `operant-core` | **READY** |
| 2 | Process hardening (no core dump, no ptrace) | §2 (Row 7) | ~45 | 0 | `operant-cli` | **READY** |
| 3 | Exclusive turn attribution | §3 (Row 8) | ~60 | 0 | `operant-core` | **READY** |
| 4 | Native patch engine + D1/D2 | §4 (Row 5+6) | ~405 (+90 del) | 0 | `operant-core` | **READY** |
| 5 | Mermaid renderer | §5 (Row 3) | ~1,600 | 0 | `operant-cli` | **BLOCKED — operator** |
| 6 | Hook decisions + events + matcher | §6 (Rows 1+2+9+12) | ~550 | 0 | `operant-core` | **BLOCKED — operator** |
| 7 | Skill tool prerequisites | §7 (Row 11) | ~40 | 0 | `operant-core` | **READY** |

Iterations: 5 executable rows → 5 commits → 5 deploys. The two blocked rows do not start.

**Nothing here adds a production dependency.** Verified: `libc` is already `operant-cli`'s
(`Cargo.toml:73`), `unicode-width` is already `operant-cli`'s (`:47`), and `which` is already
`operant-core`'s (used by `skills.rs:243` for `prerequisites_commands`).

---

## Row 1 — the command-safety interpreter unwrap

**Closes:** D5, the audit's one *high*-severity finding.

**Codex source (measured):** `shell-command/src/bash.rs:106-119` — `extract_bash_command`, 14 LOC;
`is_dangerous_command.rs:146-155` — sudo/env unwrap, ~20 LOC.

**operant today (re-verified line by line):**
- `blast_radius.rs:22` — the module doc names the interpreter gap.
- `blast_radius.rs:597-622` — `segments()` splits on `\n`/`;`/`&&`/`||`/`|`. It does not tokenize
  quoted strings, so `bash -c "rm -rf /etc"` stays one segment.
- `rate_segment` therefore sees the verb `bash`, matches no verb arm, and returns `None`.
- `classify()` (`:587-593`) maxes across segments → `None`.
- `screen()` (`:811`) returns `None` immediately (`:813-815`) — **no gate at all.**
- The hardline blocklist's `rm` rules at `approval.rs:209` are literal strings (`PACKAGE_RISK`), so
  `bash -c "rm -rf /etc"` misses that layer too. This is D4, and `approval.rs:662`'s `screen()` call
  is the only defense that was supposed to catch it.

**The edit.** One new private function in `blast_radius.rs`, called at the top of `segments()`:

- `fn unwrap_interpreter(cmd: &str) -> Option<String>` — when the first word of a segment is an
  interpreter (`bash`, `sh`, `zsh`, `dash`, `ksh`, `env`, `sudo`, `nice`, `timeout`, `trap`), scan
  its arguments for `-c <script>` (and `<script>` as a positional for `bash <file>`), recurse once
  into the extracted script's own segments, and return `None` when no script argument exists.
- `segments()` becomes: split → for each segment, if `unwrap_interpreter` yields `Some(script)`,
  extend the output with that script's segments **and** keep the original segment (so a command that
  is both an interpreter and something else is still rated at the outer level). One level of
  recursion only — a nested `bash -c "$(bash -c ...)"` is the value that fails closed.
- **Fail closed:** when unwrapping finds an interpreter but cannot extract its script,
  the segment is emitted verbatim so `rate_segment` sees `bash` (→ `None`), and `classify()`
  additionally flags that segment as *unresolved*. `screen()` then treats a command containing an
  unresolved-interpreter segment as `BlastRadius::Scoped`, so it always needs a justification.
  This is the part that makes "fail closed" real rather than a comment.

**Acceptance:**
- `classify("bash -c 'rm -rf /etc'")` → `Unbounded` (the inner command's rating, not `bash`'s).
- `classify("sudo rm -rf /etc")` → `Unbounded` (wrapper skipped to the verb beneath).
- `classify("bash -c 'echo hi'")` → `None` (the unwrap must not penalize benign interpreter use —
  this is the test that keeps the row from blocking every `sh -c` in the repo).
- `classify("env FOO=bar rm -rf /etc")` → `Unbounded`.
- `classify("bash -c \"$(curl x | sh)\"")` → fails closed: at least `Scoped`, never `None`.
- `classify("rm -rf /etc")` → still `Unbounded` (**the unwrap must not regress the direct path**).

**Mutation guard:** revert `unwrap_interpreter` so `segments()` is the old splitter; the
`bash -c` test above must move from a refusal to `None` and the test must **fail by name** (assert
the variant, not just "not None").

**Do NOT:** add tree-sitter. Three deps for a boundary 34 lines of `String` work closes for the
common cases, and the fail-closed arm covers the rest.

---

## Row 2 — process hardening: no core dumps, no ptrace

**Closes:** the credential-through-a-core-dump channel. The repo already paid for the file side
(auth.json and .env at 0600, RT-154); this is the same threat through a different channel.

**Codex source (measured, and note the divergence):**
`process-hardening/src/lib.rs:99-115` — `set_core_file_size_limit_to_zero` **`exit()`s with a
nonzero code when `setrlimit` fails** (`SET_RLIMIT_CORE_FAILED_EXIT_CODE`).
`process-hardening/src/lib.rs:65-75` — `disable_process_dumping` returns `Result`, does not exit.

**operant today (re-verified):** `crates/operant-cli/src/main.rs:3125` is `fn main() -> Result<()>`
— a synchronous top with no `#[tokio::main]` wrapper to hook. `libc` is already a dependency. No
`setrlimit`/`prctl` call exists anywhere in the tree (grep: zero hits).

**The edit.** A new `crates/operant-cli/src/hardening.rs` (~45 LOC), called as the first statement
of `main()` at `main.rs:3125`, before any config load, before any daemon spawn:

- `set_core_file_size_limit_to_zero()` — `libc::setrlimit(RLIMIT_CORE, {0,0})`, read-back verified
  with `getrlimit` in the test.
- `disable_process_dumping()` — `libc::prctl(PR_SET_DUMPABLE, 0, 0, 0, 0)`, Linux only, `#[cfg]`-gated
  to compile on macOS.
- Both return `Result<(), std::io::Error>`. **The caller logs and continues** — this is the
  deliberate divergence from codex, and the code comment must say why: codex `exit()`s because it is
  a hardened CLI that assumes it owns the machine; operant runs in container runtimes that forbid
  `PR_SET_DUMPABLE` (Yama restrictions, seccomp, unprivileged containers) and an operator whose
  agent binary refuses to start for a non-security reason is a worse outcome than a dumpable process.

**DO NOT port `remove_env_vars_with_prefix(b"LD_")`.** Measured and recorded in
RG-019: `ldd target/release/operant` resolves `libonnxruntime.so.1` from
`~/.local/lib/operant/`, so `LD_LIBRARY_PATH` is load-bearing for the Kokoro TTS default. The narrow
`LD_PRELOAD`-only variant is the only one that could be safe and it needs its own measurement —
out of scope here.

**Acceptance:** both calls are made at the top of `main()`; a test asserts `getrlimit(RLIMIT_CORE)`
returns `{0,0}` (fork-free — read it back in-process, no `fork`/`exec`); a `PR_SET_DUMPABLE` failure
returns `Err` rather than exiting, proven by a test that a deliberately-invalid call path returns
the error and the function does not abort.

**Mutation guard:** make `disable_process_dumping` `unwrap()` the result instead of propagating it;
the test that feeds a failing path must panic (proving the error path is live, not stubbed).

---

## Row 3 — exclusive turn attribution

**Closes:** the RT-199 class of bug — a `u64` duration that cannot distinguish "took zero time"
from "nobody measured this".

**Codex source (measured):** `utils/critical-path/src/lib.rs` (181) + `core/src/turn_timing/report.rs:4-25`
+ the three-state `TurnCriticalPath` in `analytics/src/turn_critical_path.rs:1-2`.

**operant today (re-verified):**
- `llm_duration` is measured per LLM call (`agent/run.rs:1234`) and emitted on both `LlmResponse`
  (`:1236-1244`) and `RequestLatency` (`:1245`).
- Per-tool duration: `tool_duration_for(&result.tool_call_id)` (`run.rs:2209`).
- Turn total: `turn_start.elapsed()` on `AgentEnd` (`run.rs:1705`).
- **All three are inclusive and overlapping.** Nothing computes `UserWait` or the residual
  (`turn_total - llm - tools`), and nothing declares a category unmeasured.

**The edit.** One new struct in `operant-core` (~60 LOC) + one emission site:

- `enum TurnSpan { Inference, Tools, UserWait, Overhead }` with a
  `fn as_str(&self) -> &'static str`.
- `struct TurnBreakdown { total: Duration, spans: [Duration; 4] }` built by *exclusive* arithmetic:
  `total` from `turn_start.elapsed()`, `inference` from the sum of the per-call `llm_duration`s,
  `tools` from the sum of `tool_duration_for`s, `user_wait` from the estop/pause hold, and
  `overhead = total - inference - tools - user_wait` computed with `saturating_sub` so a
  measurement gap can never make the residual negative.
- `enum TurnAttribution { Disabled, Incomplete { … }, Complete { breakdown } }` — **the state model
  is the actual import.** `Disabled` = capture is off; `Incomplete` = one of the four spans never
  got a timestamp (and the missing span is named); `Complete` = all four measured.
- Exposed on the existing `<turn_context>` volatile tail, in the same block where the
  context-budget disclosure rides (`run.rs:2967-3010`) — so the prompt cache is untouched (the block
  is below the latest user message) and no new plumbing is needed.

**Acceptance:** the four spans sum to the turn total with no overlap; a turn with no tools reports
`Incomplete` for the tools span rather than `Duration::ZERO`; the tail block renders one line per
span and is omitted entirely under `Disabled`.

**Mutation guard 1 (the important one):** the naive implementation double-counts. Make
`inference` and `tools` both include the inter-tool gap and the sum exceeds `total` — the test
asserting `sum == total` must fail. This is the guard that proves exclusivity is real.

**Mutation guard 2:** make the report a `u64` and assert the `Incomplete` state is lost — i.e. a
test that matches on `TurnAttribution::Incomplete` fails to compile or fails to fire, which is the
RT-199 lesson made executable.

---

## Row 4 — the native patch engine (+ D1, D2)

**Closes:** the collapse of editing to one whole-file find/replace when the AFT bridge is dead.

**Codex source (measured today):** `apply-patch/src/seek_sequence.rs` (164) +
`apply-patch/src/text_file.rs` (121) — **both portable as-is, zero imports**. `file_update.rs:60-160`
is `compute_replacements` (~120 core LOC after stripping the `similar` fuzzy-lib dependency and the
exec-server integration).

**operant today (re-verified):**
- `AftApplyPatchTool` (`tools/aft_tools.rs:615-668`) forwards the `*** Begin Patch` grammar to the
  subprocess, after a path scan for the write guard. The grammar is already agent-reachable.
- `PatchTool` (`tools/patch_tool.rs`) is the native fallback: **one** whole-file
  find-and-replace, one tier of whitespace-normalized fuzzy match, no occurrence count check, and
  `:95-99` is a dead branch that does nothing.
- `normalize_whitespace` (`:34-44`) normalizes `\r\n` away entirely — so CRLF is *destroyed* on every
  fuzzy match. That is the concrete bug the import fixes.

**The edit.**
1. New `operant-core/src/tools/patch/` — `seek_sequence.rs` (ported as-is) + `text_file.rs` (ported
   as-is, this is where `LineEnding`/CRLF fidelity lives) + `compute_replacements.rs` (ported,
   de-`similar`-ified).
2. `PatchTool`'s `execute` gains a `patches: Vec<Chunk>` argument path so "edit 5 places" is one
   call. The old single-find/replace path stays as the one-chunk case.
3. **D1, inside the port:** a `find` matching more than once is refused with the occurrence count,
   unless an explicit occurrence index is supplied. (Note C3 above: today the count is *reported*;
   codex's contract is a *refusal*. The regression test asserts the refusal message names the count.)
4. **D2, inside the port:** delete `:95-99` and add a test for the behavior it pretended to have
   (delete-a-line-with-empty-replacement, which `fuzzy_replace` actually does correctly at `:86-100`
   — the branch was dead *on top of* a working implementation).

**Order discipline:** the defects are fixed inside the port, not after it, so the mutation guards
prove both at once.

**Acceptance:** multi-chunk applies in one call; CRLF preserved; a two-occurrence find without an
index is refused with the count; a one-occurrence find still succeeds; D2's dead branch is gone and
its intended behavior is covered.

**Mutation guards:** (a) drop the occurrence check and the two-occurrence test must fail; (b) remove
the `LineEnding` model and let the writer join with `"\n"` and the CRLF fidelity test must fail.

**Do NOT port** `parser.rs`/`streaming_parser.rs`/`invocation.rs` — the grammar is already
agent-reachable via AFT, and a second agent-facing surface is two ways to say one thing. Do NOT port
cross-file rollback — codex has none either.

---

## Row 5 — mermaid (BLOCKED — one operator answer needed)

The largest row in the plan (~1,600 LOC) and the only one whose precondition was never measured.

**The measured surprise, restated:** operant's W9 cut was of a **10,215-LOC diagram compiler**. The
codex subsystem actually available is **1,515 non-test LOC** with **one** dependency
(`unicode-width`, already at `operant-cli/Cargo.toml:47`). The gate
`native_image_protocol_available() -> false`
(`tui/operant_markdown/markdown_mermaid_fallback.rs:38`) is a permanently-false function, so every
mermaid block is a plain highlighted code block labelled `mermaid`. The branch sites exist
(`markdown_render*`, `mod.rs:170`).

**The question the operator must answer, in one line:** *are diagrams in the transcript a real
requirement for you?* There is no transcript count in this repo — no evidence either way, which is
exactly why W9 deferred it and why this outline refuses to pre-empt that call.

**If the answer is yes**, the row as planned stands: port the renderer, give
`should_render_mermaid_block` a native-text path, and make every `RenderError` degrade to the
original highlighted source **with a specific reason** (unsupported / too-wide / over-limit) — that
is codex's own contract, not a nicety. That row also fixes D3 (the four unreachable
`RenderResult::Error` sites and the `pinned_images` producer gap).

**If the answer is no**, the row becomes a *cleanup* iteration instead: delete the
permanently-false gate and the dead deferred-registration apparatus rather than building a renderer
nobody asked for. Either answer produces an iteration; no answer produces none.

---

## Row 6 — hook decisions (BLOCKED — operator agreement on a security posture)

The row that changes what a hook can *do*.

**The measured gap, corrected (see C1):** codex has **11** hook events
(`hooks/src/schema.rs:103-126`) and **12 output fields across 3 groups** — universal
(`continue`, `stopReason`, `suppressOutput`, `systemMessage`) plus pre-tool-use
(`decision`, `reason`, `permissionDecision`, `permissionDecisionReason`, `updatedInput`,
`additionalContext`). operant implements **0** of the 12.

**operant today (re-verified):**
- 4 events (`shell_hooks.rs:31`: `agent_start`, `agent_end`, `turn_start`, `turn_end`) against
  codex's 11.
- `HookHandler` is `Arc<dyn Fn(HookEvent, HookContext) -> Pin<Box<Future<Output = ()>>>>`
  (`gateway_pipeline.rs:150-159`) — **the return type is `()`, so a handler structurally cannot
  influence the decision.**
- 4 flat env vars, **no stdin**, **no matcher** — a hook cannot learn which tool is being called,
  and cannot refuse one it never heard the name of.
- stdout is logged at debug level only (`run_hook_command`, `:190`).
- `emit()` is fire-and-forget (`gateway_pipeline.rs:211-238`).

**What the operator is being asked, in one line:** *an operator-configurable shell command that can
rewrite a tool call's arguments and inject a developer message is a security posture, not a port —
do you want operant to have that power, and if so at what matcher granularity?*

**If yes**, the row as planned stands, with the correction that the **11-event and 12-field
counts** replace "4 of 14" and "8 decisions" everywhere. The three decisions that actually change a
run — `decision: block`, `updatedInput`, `additionalContext` — ship first; the matcher scopes a hook
to named tools; `emit()` gains a **separate drainable path** (not a signature change — the kernel
`HookSlot` bridge depends on the existing fire-and-forget arm, per `gateway_pipeline.rs:84-87`'s
`Hooks(String)` catch-all comment).

**Discovery correction (C2):** do not import `discovery.rs` — it is 1,963 LOC carrying config-rule
keys, plugin trust and plugin fingerprints. operant already has a hooks-directory loader
(`load_shell_hooks`, `shell_hooks.rs:92`); extend that instead.

---

## Row 7 — skill tool prerequisites

**Closes:** a skill can be invoked whose declared tool does not exist, and the failure arrives as a
confusing tool error rather than a named refusal.

**Codex source (measured):** `skills/src/model.rs:81-95` — 15 LOC.

**operant today (re-verified):** `SkillFrontmatter` (`skills.rs:15-30`) has
`prerequisites_env` + `prerequisites_commands`; `is_available` (`:218-245`) gates on platform (not
exposed in frontmatter), env vars, and `which::which(cmd)`. Tool availability **already exists** on
the registry and is already called in four places — `is_available` is simply not one of them.

**The edit.** Add `prerequisites_tools: Option<Vec<String>>` to `SkillFrontmatter`, a loop in
`is_available`, and a refusal path at the skill-invocation site that **names the missing tool**.

**Acceptance:** a skill declaring `prerequisites_tools = ["nonexistent_tool"]` is refused at
invocation with the tool named; a skill declaring an available tool still runs.

**Mutation guard:** remove the loop and the refusal test must fail with the tool name present in the
message.

**Do NOT take** `transport`/`command`/`url`/`oauth_callback_port` — those belong to the connectors
crate, already rejected in this ledger.

---

## Per-iteration gate (unchanged, applies to all five)

1. `cargo fmt --all`
2. `./scripts/check.sh check -p <crate>` — scoped, never `--workspace` mid-iteration
3. the touched crate's tests, filtered to the changed area
4. `bash scripts/clippy-warning-gate.sh`
5. release build + deploy to **both** install paths
6. `git log origin/main -1` shows the commit
7. the mutation guard is run and observed to fail **before** the fix is confirmed (a guard that
   never saw red is not a guard)

**Known red gate carried into every step:** RT-203 (opened iter-1523) — an unidentified
`operant-core --lib` parallel-load flake, delta-proven not-ours. If the gate goes red on an unrelated
test name, capture it and continue; do not fix a test this plan does not own.

## Cost, re-measured

| Row | Added | Deleted | New deps |
|---|---|---|---|
| 1 interpreter unwrap | ~40 | 0 | 0 |
| 2 process hardening | ~45 | 0 | 0 |
| 3 turn attribution | ~60 | 0 | 0 |
| 4 patch engine | ~405 | ~90 | 0 |
| 7 skill tool deps | ~40 | 0 | 0 |
| **executable total** | **~590** | **~90** | **0** |
| 5 mermaid (blocked) | ~1,600 | ~15 | 0 |
| 6 hooks (blocked) | ~550 | 0 | 0 |

**Five iterations, ~590 LOC, zero new dependencies** to execute. The two blocked rows add ~2,150
LOC and are worth nothing until the operator answers their one question each.

## What this outline refuses to do

- **Ship the blocked rows "while we're in there."** A row gated on an operator answer is not
  unblocked by a convenient commit boundary.
- **Reorder for convenience.** The interpreter unwrap is first because it is the one *high* severity
  finding and 40 lines; mermaid is last because it is 1,600 lines gated on a question nobody asked.
- **Add tree-sitter, `similar`, or the connectors crate** — each was measured and refused, with the
  disqualifier recorded in the audit.
- **Re-litigate the DO-NOT families** — memory extraction, voice/WebRTC, connectors, code-mode/V8,
  the app-server protocol tier. The disqualifiers are in
  `docs/audit/2026-10-31-codex-architecture-deepen/00-INDEX.md`.

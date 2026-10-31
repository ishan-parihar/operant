# 03 — Session and turn: storage, metadata, compaction, shell

**Subsystem family:** what a turn knows, what survives it, how context is reclaimed, and
how a command runs.

**Verdict key:** `IMPORT-DIRECT` · `IMPORT-PATTERN` · `SKIP`.

---

## Row 1 — token-budget compaction without a model call

- **Codex:** `core/src/compact_token_budget.rs:1` (69 LOC). The file's own doc comment (`compact_token_budget.rs:14-17`): "Token-budget compaction skips model/server summarization and installs a fresh context window instead. It is still modeled as compaction so compact hooks and `ContextCompaction` turn items observe the same lifecycle as local or remote compaction."
- **Operant today:** operant has **three** compaction mechanisms, all documented in `crates/operant-core/src/session/autocompact.rs:3-20`:
  1. `context_management` — tiered eviction + decay curve, runs **during** a request, reactive, per-request, lossy.
  2. `agent::llm_compressor` — `compress_context_overflow`, runs when a provider reports overflow, also reactive.
  3. `session::autocompact` — scheduled, runs **after** a scheduled execution finishes, gated on a 4,000-token floor.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** S
- **Value:** the missing third option.

**Why it matters.** Both of operant's *reactive* mechanisms spend a **model call** to
reclaim context. The LLM compressor (`crates/operant-core/src/agent/llm_compressor.rs`,
1,214 LOC) asks the model to summarise the history; the eviction path drops messages.
codex has a third option that neither of operant's has: when the budget is exceeded,
**install a fresh context window** — drop the history, keep the turn's live state, cost
**zero** model calls and zero latency.

That is not always the right answer (losing the history loses the reasoning), but it is
right when the history is already a summary of a summary. operant's scheduled
autocompact already discovered this problem and solved it with a **floor**
(`autocompact.rs:21-33`: below 4,000 tokens, lossy compaction is strictly bad because
the next run "summarises a summary, compounding the loss every job"). codex solves the
same problem by having a compaction mode that does not summarise at all.

**The import is small.** codex's unit is 69 lines, but most of it is the hook lifecycle
(`run_pre_compact_hooks` / `run_post_compact_hooks`), which operant does not have as
compaction events. The **portable core** is: a `CompactionTrigger` enum
(`Manual` / `Auto`) and a `start_new_context_window` path — "keep the system prompt and
the current turn, drop the rest".

**Ponytail note:** operant already has the *pieces* — `manage_context_reporting`
(`context_management.rs:1283`) returns an `EvictionOutcome`, and
`estimate_total_tokens` (`context_management.rs:85`) measures the current size. The
missing piece is a **mode** that says "evict hard, do not summarise" and a trigger that
uses it when the history is already lossy. That is a config flag and a branch.

---

## Row 2 — the model's own context budget

- **Codex:** `core/src/tools/handlers/get_context_remaining.rs:1` (109 LOC) + `..._spec.rs:1` (39 LOC). The output type carries `tokens_left: Option<i64>` and renders a `TokenBudgetRemainingContext` fragment (`get_context_remaining.rs:15-33`).
- **Operant today:** the accessors **exist** but nothing exposes them to the model. `crates/operant-core/src/context_management.rs:85` — `pub fn estimate_total_tokens(messages: &[Message]) -> usize`; `context_management.rs:1283` — `manage_context_reporting(...) -> (Vec<Message>, EvictionOutcome)` with an `EvictionOutcome` struct (`context_management.rs:802`). `rg -n "remaining" crates/operant-core/src/agent/` finds only retry-budget and cooldown timers — **nothing** about the context budget reaching the model.
- **Verdict:** `IMPORT-DIRECT`
- **Effort:** M (see the re-grade below)
- **Value:** operant's eviction is silent, and operant already has the arithmetic.

**Re-graded effort — this row was wrong the first time.** Both docs recorded an
effort for this row on incomplete evidence, and doc 03 asserted **S** while `01-tool-surface.md`
said **M**. Neither checked the call path. The ground truth:

- `agent/stream.rs:679` — `pub(crate) async fn execute_tools(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>>`. **`messages` is not a parameter.**
- `agent/stream.rs:24` — `fn tool_context(&self) -> ToolContext` builds from `self`, and `ToolContext` (`crates/operant-core/src/tools.rs:287-298`) carries only `session_id` + `metadata`. There is **no message list** on it.
- `agent/run.rs:1813` calls `self.execute_tools(tool_calls)`, while `messages` is a **local variable** in `run.rs` at that point (evicted at `run.rs:2880`). It is not on `self`.

So `estimate_total_tokens` being `pub` is necessary but **not sufficient**: the tool cannot
reach the live messages today. Closing the gap needs either a `remaining_tokens` value
threaded into `execute_tools`/`tool_context`, or a shared budget cell the agent updates
after each compaction event. That is new plumbing, not just a new tool file. **M is
correct; the earlier "S, operant already has the number" was overconfident and is
withdrawn.**

**Why it matters.** See `01-tool-surface.md` Row 2. The model cannot tell that operant
dropped its context, so it cannot compensate. This is the highest value-per-line row in
the session family precisely because operant already has the arithmetic and only lacks
the channel.

**Same blocker as Row 4 below.** Both this row and Row 4 (turn diff tracking) are blocked
on the same thing: a `ToolContext` that carries live per-turn state. If one enrichment is
done, do it once and unblock both — see Row 4's note on why a global would be worse.

---

## Row 3 — turn metadata

- **Codex:** `core/src/turn_metadata.rs:1` (626 LOC).
- **Operant today:** `crates/operant-core/src/agent/turn_context.rs:1` — operant has a turn context type, and `crates/operant-core/src/agent/events.rs:1` carries `AgentEvent`s.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant already has turn context and turn events. codex's 626-LOC file is
  shaped by codex's `Session`/`StepContext` model.

**Why SKIP:** this is not a gap, it is a difference in how the same thing is modelled.
Importing it would mean restructuring operant's turn model for no new capability.

---

## Row 4 — turn diff tracking

- **Codex:** `core/src/turn_diff_tracker.rs:1` (403 LOC).
- **Operant today:** `ABSENT` as a core mechanism. `rg -n "turn_diff|diff_tracker" crates/` returns only **removal markers**: `crates/operant-cli/src/tui/app/prompt.rs:400` ("iter-209: attach_turn_diff_state + refresh_turn_diff_from_history" — removed), `crates/operant-cli/src/tui/app/agent_events.rs:257,293,387` ("refresh_turn_diff_from_history removed"), and `crates/operant-cli/src/tui/diff_viewer/parse.rs:33` ("iter-209: build_turn_diff deleted"). operant has a **TUI diff viewer** but no core-level tracking of what a turn changed on disk.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** L
- **Value:** real, but it needs Row 5 of `01-tool-surface.md` first.

**Why it matters.** operant deleted its turn-diff tracking in iter-209 and has a
`diff_viewer` that therefore reconstructs state some other way. codex's tracker records
what changed during a turn so the model (and the UI) can see the turn's effects.

**Why L, and why it waits.** The tracker has to be handed to the tool, and operant's
`ToolContext` (`crates/operant-core/src/tools.rs:290`) cannot carry it — it is a
`HashMap<String, String>` plus a dormant session id. So this row is blocked on the
`ToolContext` enrichment. Importing the tracker without the context change would mean
routing it through a global, which is worse than not having it.

---

## Row 5 — executed tool-call records

- **Codex:** `core/src/tools/executed_tool_calls.rs:1` (758 LOC).
- **Operant today:** `crates/operant-core/src/trajectory.rs:1` — operant records a trajectory. `crates/operant-core/src/agent/events.rs:1` emits tool events.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant already records what ran.

**Why SKIP:** codex's 758-LOC unit is tied to its `Session` rollout format. operant's
trajectory is the equivalent and it exists. The **narrower** gap — per-call dispatch
tracing with payload sizes — is Row 3 of `01-tool-surface.md` and is much smaller.

---

## Row 6 — instruction-file discovery (`AGENTS.md`)

- **Codex:** `core/src/agents_md.rs:1` (562 LOC) + `core/src/agents_md_manager.rs:1` (204 LOC).
- **Operant today:** operant has `AGENTS.md` handling but it is **scattered**, not a
  manager. `rg -n "AGENTS\.md" crates/` finds:
  - `crates/operant-cli/src/tui/memory_file_selector.rs:56-58` — three levels: user
    (`~/.operant/AGENTS.md`), project (`{project_root}/AGENTS.md`), local
    (`{project_root}/.operant/AGENTS.md`).
  - `crates/operant-cli/src/commands.rs:391` — `CommandDef::new("init", "Initialize AGENTS.md for this project")`.
  - `crates/operant-harness/src/pool.rs:1915` — `required_files: [AGENTS.md]` for a harness pool.
  - `crates/operant-os/tests/e2e.rs:549` — a test that plants "a legacy `AGENTS.md`" and asserts "a Holon body must not carry the legacy tree's AGENTS.md".
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant already discovers and injects `AGENTS.md` at three levels.

**Why SKIP:** this is a case where reading codex's larger, more formalised module could
suggest operant's implementation is unsound. It is not — operant's three-level scheme
is documented and tested. codex's 766-LOC pair adds tree-walking and caching that
operant does not need.

**One thing worth copying:** codex's `agents_md_manager.rs` is a **single owner** for
the file. operant's handling is spread across a TUI selector, a command, the harness
pool, and the org layer. If that list keeps growing, a manager is the fix. Not now —
three call sites is not yet a seam worth cutting.

---

## Row 7 — persistent shell session

- **Codex:** `core/src/unified_exec/` — `process.rs` (PTY lifecycle + output buffering),
  `process_manager.rs` (orchestration: approvals, sandbox selection, reuse),
  `process_state.rs` (27 LOC, shared exit/failure state for local and remote processes),
  `stdin_approval.rs` (297 LOC, approval for writing to a process's stdin),
  `shell_snapshot.rs`, `snapshots/`, `async_watcher.rs`, `oneshot.rs`.
  The module doc at `core/src/unified_exec/mod.rs:1-27` lays out the flow: request →
  orchestrator (approval → sandbox selection) → runtime transform → PTY spawn → on
  denial, retry without sandbox (no re-prompt, thanks to caching).
- **Operant today:** `crates/operant-core/src/tools/terminal_tool.rs`, `terminal_backend.rs`,
  `code_execution.rs`, `crates/operant-core/src/tools/kernel/` (the Python kernel).
  operant has a **persistent Python kernel** (`docs/library/kernel.md`) but its shell
  tool is not the same shape.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** L
- **Value:** one genuine sub-feature worth taking, at small size.

**The sub-feature worth taking: `process_state.rs` (27 LOC).** codex factors the
"what happened to this process" state (exit code, signal, failure reason) into one type
shared between local and remote processes, so that an approval decision or a result
rendering does not have to know which side ran it. operant's `code_execution.rs` and
`terminal_backend.rs` are separate paths; if operant ever gets a remote execution tier
(the codex `exec-server/` shape), this is the seam that keeps the two from diverging.

**The rest of `unified_exec/` is SKIP for now.** 7 modules, a PTY abstraction, a
snapshot system, an async watcher, and a stdin-approval flow — for a capability
operant's shell tool already provides in a simpler form.

**`stdin_approval.rs` (297 LOC) is the interesting half.** codex asks the operator
before writing to a running process's stdin, because stdin to a live process is an
**out-of-band channel** that bypasses the normal command-text approval guard. operant's
approval guard reads the command string (`approval.rs:621` —
`extract_command_from_args`); a tool that writes to an already-running shell's stdin
would slip past it.

**Checked, and operant does NOT have that bypass.** `crates/operant-core/src/tools/terminal_tool.rs:45`
is a one-shot `execute` — there is no persistent shell session to write into.
`crates/operant-core/src/tools/code_execution.rs:56` routes Python through the
persistent kernel, whose `stdin` (`crates/operant-core/src/tools/kernel/sidecar.rs:46`)
is a **NDJSON JSON-RPC channel to the sidecar process**, not a shell's stdin: the
model sends a JSON-RPC request, not shell text. `rg -n "os.system|subprocess|shell=True|exec\(|eval\("`
over the kernel and code-execution sources returns no shell-execution path.

So the guard bypass codex has to defend against is **not present in operant**. The
`stdin_approval.rs` pattern is therefore **SKIP** — it defends against a shape operant
does not have. Recording it because if operant ever gains a persistent shell session,
this is the file to read first.

---

## Row 8 — thread-store (per-thread key/value)

- **Codex:** `thread-store/` crate — `store.rs`, `live_thread.rs`, `queue_store.rs`, `thread_sections.rs`, `thread_attachments.rs`, `projects.rs`, `in_memory.rs`, `types.rs`, `error.rs`.
- **Operant today:** `crates/operant-core/src/session/mod.rs:1`, `listing.rs`, `autocompact.rs`; `crates/operant-core/src/database.rs:1` (4,176 LOC). operant's session store is **SQLite** and it is a **conversation** store, not a per-thread key/value store.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** operant has no use for a thread-scoped KV store today.

**Why SKIP:** codex's thread-store holds a thread's *sections* (a thread is a document
with named regions), its *attachments*, and a *project* grouping. operant's session is
a message transcript with a title, a tool-state record and checkpoints
(`docs/library/sessions.md`). Those are different shapes serving different purposes.
Adding a KV store because codex has one is exactly the complexity ponytail forbids.

---

## Family summary

| Row | Mechanism | Codex LOC | Verdict | Effort |
|---|---|---|---|---|
| 1 | token-budget compaction (no model call) | 69 | IMPORT-PATTERN | S |
| 2 | model's own context budget | 148 | IMPORT-DIRECT | **M** (re-graded: needs plumbing) |
| 3 | turn metadata | 626 | SKIP | — |
| 4 | turn diff tracking | 403 | IMPORT-PATTERN (blocked on ToolContext) | L |
| 5 | executed tool-call records | 758 | SKIP | — |
| 6 | instruction-file discovery | 766 | SKIP | — |
| 7 | persistent shell session | ~7 modules | IMPORT-PATTERN (only `process_state.rs`, 27 LOC) | L |
| 8 | thread-store KV | 1 crate | SKIP | — |

**Best in family:** Row 2. operant already computes the number
(`estimate_total_tokens`, `context_management.rs:85`) and already has the eviction
outcome (`EvictionOutcome`, `context_management.rs:802`). It lacks only the channel to
the model. That is a ~150-line port that closes a real capability gap.

---

**OUTCOME — iter-1545 (Row 2, and 01-tool Row 2).** Shipped as a **tail line, not a tool**,
exactly as the audit re-graded it. One block appended to the volatile `<turn_context>` at
`crates/operant-core/src/agent/run.rs` immediately before the tail is attached
(`run.rs`'s "Context budget disclosure" comment marks it), reading two values already in scope
there — `estimated_tokens` (`run.rs:2721`) and `effective_budget` (`run.rs:2717`) — plus
`ladder_fired` (`run.rs:2723`), which reports whether the preflight ladder compacted this turn.

No `ToolContext` change, no `get_context_remaining` tool (the 148-LOC codex unit both ledger
docs pointed at), no `manage_context_reporting`. The block **charges itself before quoting its
numbers**: it is built once to measure, its own token size is added back to `used`, then built
again for the record, so the figure the model reads includes the block it is reading. The
first draft charged `turn_context_tokens` instead and the compiler warned it dead — every read
of that binding happens earlier in the function (2641, 2719, 2721, 2796, 2806, 2837, 2895), so
this turn could not observe it.

Guard: `agent::tests::the_tail_reports_the_context_budget_and_compaction_state`
(`agent/mod.rs`), which asserts the block is present and carries real numbers, that a larger
history reports a larger `used`, and that the frozen prefix stays byte-identical across two
turns with different budgets. **Mutation-proven RED:** deleting the disclosure block fails it by
name. operant-core 2685/0, fmt + clippy clean (0 warnings), operant-cli compiles.

# A2 — Tool registry and tool implementations (architecture red-team)

Audited revision: **origin/main @ 2d431ac0**.
Read-only. No working-tree file was read, no cargo command was run, no git write command was issued.
Every citation is `git show origin/main:<path>` + line number.

Slice: `crates/operant-core/src/tools.rs`, `crates/operant-core/src/tools/**`,
`crates/operant-core/src/aft_bridge.rs`, `crates/operant-core/src/tool_guardrails.rs`.

## Contents

1. [Direct-execute inventory](#direct-execute-inventory)
2. [Verified](#verified)
3. [Asserted](#asserted)
4. [Lean cuts](#lean-cuts)
5. [Rust discipline](#rust-discipline)
6. [Finding summary](#finding-summary)
7. [Coverage](#coverage)

---

## DIRECT-EXECUTE INVENTORY

The prior round's finding (RT-3) was Telegram voice enrichment constructing `TranscriptionTool`
and calling `OperantTool::execute` directly, so `operant tools disable transcribe_audio` had no
effect. This is the complete call-site inventory for that class, tree-wide.

Method: `git grep '\.execute(' origin/main` over all crates (548 non-test hits), filtered to
receivers that are `dyn OperantTool` rather than a `rusqlite` connection or a `CommandRegistry`.
Every candidate was then classified by reading its enclosing function and the nearest preceding
`#[cfg(test)]`.

### Production call sites (the only four that matter)

| # | Site | Tool | Consults disabled set? | Verdict |
|---|------|------|------------------------|---------|
| 1 | `crates/operant-cli/src/gateway_runner/enrichment.rs:143` (`enrich_voice`) | `TranscriptionTool` | **YES** — `direct_call_is_disabled(tool.name())` at :136 | SAFE (guarded) |
| 2 | `crates/operant-cli/src/gateway_runner.rs:1962` (gateway `/search` command) | `SessionSearchTool` | **NO** | **BYPASS — see FINDING A2-1** |
| 3 | `crates/operant-core/src/mcp.rs:2458` (`McpNamespacedTool::execute`) | delegates to inner `McpTool::execute` | n/a — this is a trait impl body, entered only via `registry.execute` | SAFE (delegation) |
| 4 | `crates/operant-cli/src/plugin_tools.rs:69` (`PluginToolAdapter::execute`) | plugin inner | n/a — trait impl body, entered only via `registry.execute` | SAFE (delegation) |

### Registry-mediated nested dispatch (safe by construction)

| Site | Mechanism | Verdict |
|------|-----------|---------|
| `crates/operant-core/src/tools.rs:502` (`ToolExecutor::execute_with_timeout`) | the single production executor, called only from `ToolRegistry::execute` :1213/:1235 | SAFE — registry core |
| `crates/operant-core/src/tools/tool_search.rs:652` (`ToolCallTool::execute`) | `self.registry.execute(&name, &id, arguments, context)` — the deferred-tool bridge. Also gates on `registry.is_available(&name)` at :625, which reads `disabled_names`/`disabled_toolsets`. A regression test `tool_call_cannot_bypass_disabled_tools` (:991) pins it. | SAFE |
| `crates/operant-core/src/tools/kernel/runtime.rs:62` (pk-bridge) | `self.registry.execute(name, "pk-bridge", args, ctx)` | SAFE |
| `crates/operant-core/src/agent/stream.rs:1763, 1833, 1920` | the agent loop's three dispatch arms (single tool / serial batch / 8-worker pool) | SAFE |
| `crates/operant-core/src/agent/prompting.rs:872` (background review) | `registry_for_review.execute(tool_name, …)` after a whitelist check at :846 | SAFE |
| `crates/operant-cli/src/mcp_serve.rs:405` | `registry.execute(tool_name, …)` after `registry.contains` at :390 | SAFE |
| `crates/operant-core/src/tools/sub_agent_tool.rs:1040-1051` | child agent gets its own `ToolRegistry`, built by `register_child_tools` (:1469) which inherits `parent_disabled_tools`/`parent_disabled_toolsets` (:603-608) and filters at :1424/:1437 | SAFE — child cannot regain a parent-disabled tool |

### Test-only direct executes (all 100+ remaining sites)

Every other `.execute(` on an `OperantTool` in the tree is inside a `#[cfg(test)] mod tests`
block. Verified by locating the nearest preceding `#[cfg(test)]` for each: `tools.rs`,
`browser_cdp_tool`, `browser_dialog_tool`, `browser_tool`, `checkpoint_tool`, `clarify_tool`,
`code_execution`, `config_tool`, `cron_tool`, `ctx_tools`, `datetime_tool`, `debug_helpers`,
`discord_tool`, `evidence_recall`, `file_state`, `file_tools`, `harness_tools`,
`home_assistant_tool`, `http_tool`, `kernel/mod`, `lcm_tools`, `mcp_tool`, `memory_tools`,
`notification_tool`, `reaction_tool`, `send_message_tool`, `skills_tool`, `sourcehound`,
`sourcehound_composites`, `terminal_tool`, `todo_tool`, `tool_backend_helpers`, `tool_search`,
`transcription_tool`, `tts_tool`, `verification_tool`, `video_analysis_tool`, `web_tools`,
`working_diff_tool`, `gateway_commands.rs:4406/4480`, `plugin_tools.rs` (test module),
`memory_provider_tools.rs:155` (test), `pool_adapter.rs:110` (test).

`crates/operant-core/examples/simple_agent.rs` and `crates/operant-os/src/tools.rs:151` are
example/adapter code, not production paths.

### FINDING A2-1 — HIGH — gateway `/search` bypasses the disabled-tool gate

`crates/operant-cli/src/gateway_runner.rs:1944-1963`. The gateway's `/search` / `/find` channel
command constructs `SessionSearchTool` directly and calls `OperantTool::execute` on it with
`ToolContext::default()`. There is no `direct_call_is_disabled` check and no registry hop, so
`operant tools disable session_search` leaves the channel command searching.

Impact is narrower than the RT-3 voice case, because the tool is invoked with a fixed argument
set (`{limit: 10}` or `{query, limit: 10}`) rather than free-form model output, and the command
is operator-typed rather than model-chosen. But it is the same defect shape: a disable that is
honored everywhere except one path.

Evidence that this is the only such site: the two `OperantTool::execute` uses in
`gateway_commands.rs` (:4406, :4480) are inside `#[test]` functions, and the enrichment module's
one arm is the guarded one.

Note the sibling `/memory`, `/skill` and `/todos` commands are the *same* shape but do not
construct tools — they read the tool's backing store directly (the `TodoTool` state file, the
skill directory), so the disabled set is not the right gate for them and no finding attaches.

---

## VERIFIED

**The registry's disabled set is a real enforcement boundary.** `ToolRegistry::execute`
(`tools.rs:1113-1186`) reads `disabled_names` and `disabled_toolsets` under read locks, drops the
guards before any `.await`, and refuses a disabled tool by name OR by toolset (:1162-1176) with a
message that names the undo command. `is_disabled` (:842-849) is the single definition shared by
the schema filters and dispatch. This closes audit finding Z17.

**`direct_call_is_disabled` is correctly scoped.** `enrichment.rs:213-215` delegates to
`tool_state::disabled_tool_names(&runtime_config())` — the SAME effective set the registry build
applies (TOML `[tools] disabled_tools` unioned with the CLI state file), so the direct path and
the registry path cannot disagree. The mutation test
`a_disabled_transcription_tool_stops_the_voice_arm` (`enrichment.rs:231`) writes the real
`tool_state.json` and also asserts scope (disabling one tool must not disable another, :258-260).

**The AFT dedup disabled-set names are correct.** `main.rs:1471-1478` disables
`file_read`/`file_write`/`patch`/`terminal`/`file_search`/`file_list` — all six are real
`name()` values, verified against the registering modules. This was the reachable default case
Z17 named, and it is clean.

**`is_available()` closes the bridge path too.** `tool_search.rs:625` checks
`registry.is_available(&name)` before dispatch, and `is_available` (`tools.rs:992-1004`) reads
both disabled sets AND `is_available()`. A disabled deferred tool cannot be reached around the ban
via `tool_call`.

**Child agents inherit the parent's bans.** `sub_agent_tool.rs:603-608` stores the parent's
disabled tools/toolsets; `register_child_tools` (:1469) filters at :1424 (toolsets) and :1437
(builtin) so a delegated child can never regain a tool the parent disabled.

**Sub-agent MCP sharing is safe.** `builtin.rs:309-311` passes the parent's `McpManager` to
children, so children reach the same live MCP children rather than spawning their own.

**No duplicate tool names.** Extracted every `name()` return across `crates/operant-core/src/tools/**`
(100 distinct names) and ran `uniq -d`: zero collisions. Two tools cannot fight over one registry key.

**No duplicate capability that warrants deletion.** The pairs that look redundant are not:
`file_search` (ripgrep over a directory) vs `file_list` (directory listing) vs `file_read`
(contents) are three distinct primitives; `skill_view` (read a skill) vs `skill_manage` (write
one) split read from write deliberately, and `skill_manage` is pool-only so a vendor skill can
never be written in place; `web_fetch`/`web_scrape`/`web_extract`/`web_crawl` are one engine's
four verbs; `memory_search` vs `session_search` search different stores (facts vs sessions).

**Path traversal is guarded in the file tools that take a model-supplied path.**
`file_tools.rs:315-383` `validate_path` canonicalizes, rejects the `DENIED_PATH_PATTERNS` list
(:37-65: SSH keys, `.aws/credentials`, `.operant/auth.json`, `/etc/shadow`, keychain, …),
rejects registered credential files via `credential_files::protected_path_reason` (:368), and
rejects `..` in the canonical path (:375). It is called by `FileReadTool` (:418),
`FileWriteTool`, `FileSearchTool`, `FileListTool` and by `PatchTool` (`patch_tool.rs`, 2 call
sites — the patch tool previously bypassed it entirely, now shares the same gate).

**The Holon write fence fails closed.** `file_tools.rs:233-264` `holon_write_fence` resolves an
unresolvable path through its nearest existing ancestor and REFUSES if it cannot (:246-254) —
RT-445's exact hole (a write whose parent did not exist yet used to pass) is closed.

**`todo_tool` is path-safe against a model-supplied session id.**
`todo_tool.rs:61-67` hashes the session id (`todo_state_path`, :107-113) instead of using it as
a filename, so `../../keys` cannot steer the write out of the todos directory. The id is stored
inside the JSON so a file is still traceable.

**The 8-worker pool serializes what can interfere.** `stream.rs:1815-1821` routes a batch to the
serial arm when `batch_allows_parallel_execution` returns false.
`agent/mod.rs:1232-1244` returns false for >1 filesystem mutation or any permission-gated call;
`is_file_mutation_tool` (:1167-1179) and `is_permission_gated_tool` (:1189-1219) are pure
functions of the batch's names and unit-testable without an agent. Permission prompts stay in
sequential phase 1 (`stream.rs:1721-1726`) so a user never sees two dialogs at once.

**Process-global tool state is lock-guarded, not racy.** `file_state.rs:31` (`FILE_STATES`),
`todo_tool.rs:52` (`TODO_STORE`) + `:58` (`LOADED_SESSIONS`),
`foreground_swarm.rs:53` (`LIVE`), `async_delegation.rs:126` (delegation registry),
`checkpoint_tool.rs:30` (`CHECKPOINT_MANAGER`), `kernel/mod.rs:28` (`GLOBAL_RUNTIME`),
`sourcehound.rs:173` (client). Each is a `LazyLock`/`OnceLock` + `Mutex`, and the multi-line
comments at `todo_tool.rs:416-427` show the lock-scope reasoning was done (the store lock is
scoped to the store access; summaries are built outside it; `load_session` is called before the
lock because it takes the same lock and would deadlock a plain `Mutex`).

**`file_state`'s session keying is real, not aspirational.** `stream.rs:33-57` `tool_context`
sets BOTH the `session_id` field and the `session_id` metadata key (:47-49), and
`ToolContext::resolved_session_id` (`tools.rs:401-405`) reads the field with a metadata fallback.
`file_state.rs:174-178` and `evidence_recall.rs:108-110` use `resolved_session_id()`. The
RT-538 defect (every session sharing one `("", path)` entry) is closed.

**`normalize_arg_keys` is a genuine class fix.** `tools.rs:450-468` adds the snake_case twin of
any camelCase key at the single dispatch seam, so a model following the advertised
`#[serde(rename_all = "camelCase")]` schema no longer hits "missing required parameter" on half
the parameterized tools. An explicit snake key always wins; a key with no uppercase letter is
untouched.

---

## ASSERTED

**`vision_tool.rs:109-130` `resolve_file_path` does not call `validate_path`.** It strips
`file://`, expands `~` from `$HOME`, and reads the file if it exists. A model asking for
`file:///etc/shadow` would get an image-decode failure rather than a refusal, and
`file://~/.ssh/id_rsa` would be read as bytes. The practical blast radius is small — the bytes
are fed to an image decoder that rejects non-images, and the tool's output is a text description,
not the file — but the path is not fenced, unlike every other model-supplied path in the file
tools. ASSERTED, not traced to a live wrong output.

**`file_state` `check`/`watch`/`diff` do not call `validate_path`.** `file_state.rs:176-178`
pass `&args.path` straight to `handle_check`/`handle_watch`/`handle_diff` →
`get_file_metadata` → `fs::metadata`/`fs::read` (:117-143, :59-90). A model can hash and stat
any file on the box, including one `file_read` would refuse. This is a metadata-read primitive,
not a content-read primitive, and it is session-keyed; but the deny list does not apply.
ASSERTED.

**`working_diff_tool` takes a `path` and does not fence it.** `:67-84` uses the supplied path as
`cwd` for `git` and reports diffs. It runs `git` in an arbitrary directory, so the exposure is
whatever `git` will read in a repo the caller names — no `validate_path`. ASSERTED.

**`code_execution` is explicitly not a sandbox.** `code_execution.rs:6-17` says so in the module
doc and names the hermes parity gap (R12-1). Code runs with the operant process's own
privileges. This is a documented, accepted limitation rather than a defect, but it means every
path guard in this slice is advisory against a model that can call `code_execution`.

**`debug_env` masks only by substring.** `debug_helpers.rs:20-25` masks keys containing
`KEY`/`TOKEN`/`SECRET`/`PASSWORD`/`PASS`/`CREDENTIAL`. A variable holding a credential under a
name that matches none of those (e.g. `GH_PAT`, `AWS_SESSION`, `OPENROUTER`) is printed in
full. ASSERTED.

**`tools.rs:1200-1209` clones the interceptor list on every tool call when any interceptor is
registered.** `interceptors.read().await.clone()` plus a full `plugin_interceptors` clone, then
`args.clone()` per pre-execute event (:1221) and again per post-execute event (:1250). The
empty-registry fast path (:1210-1216) is clone-free and is the default, so this only costs
operators who installed an observer plugin. ASSERTED as a cost, not a bug.

---

## LEAN CUTS

**`MemoryRecallTool` is dead code — delete it.** `crates/operant-core/src/tools/memory_tools.rs:347-402`
defines `pub struct MemoryRecallTool` with `name() == "memory_recall"`, and
`builtin.rs:39` re-exports it. It is **never registered**: `register_memory_tools`
(`builtin.rs:16-21`) registers only `MemoryStoreTool` and `MemorySearchTool`.
`builtin_tool_aliases` (`tools.rs:614-623`) maps `memory_recall` → `memory_search`, so the name
the dead tool advertises is an alias to a DIFFERENT tool. Non-test references: the two re-exports
(`tools.rs:95`, `builtin.rs:39`) and `crates/operant-core/tests/integration_test.rs:22,422`,
which constructs it to assert its schema. Nothing in any registry build path touches it.

Verified: `git grep 'MemoryRecallTool' origin/main -- crates/` returns exactly those five lines.

**Nothing else in the slice is dead.** `lcm_tools.rs` (1711 lines) and `ctx_tools.rs`
(609 lines) are NOT dead, contrary to a first pass: both are conditionally registered from
`main.rs` (`register_ctx_tools` at :1515, `register_lcm_tools` at :1581). `foreground_swarm.rs`
is live (`sub_agent_tool.rs:1169/1207`, `tui/app/mod.rs:1488`). `harness_tools.rs` is live
(`main.rs:1967`, `kernel/mod.rs:516`). `cli_roster`/`cli_discovery`/`cli_help_parser` are
library helpers used by `cmd_context.rs` and `cli_roster.rs:366`.

---

## RUST DISCIPLINE

Read the five charter chapters (01 idioms/borrowing, 03 perf, 04 errors, 06 generics/dispatch,
09 Send-Sync) before this pass.

**`unwrap`/`expect` in production: compliant.** Every production site carries an `#[expect]` with
a reason. Sampled and confirmed:
- `skills_tool.rs:70` `#[expect(clippy::expect_used, reason = "infallible once-init / static init")]`
  over five `Regex::new(...).expect("static regex literal is invalid — authoring bug")` sites.
- `kanban_tool.rs:47-51` `#[expect(..., reason = "result is a json! value built in this arm, so it always serializes")]`
  over three `serde_json::to_string_pretty(...).expect(...)` sites.
- `file_state.rs:210-213` / `:235-238` `#[expect(clippy::expect_used, reason = "poisoned lock: panic is the intended recovery")]`.
- `stream.rs:1742-1745` `#[expect(..., reason = "unreachable: the `pending.len() == 1` guard above leaves exactly one element")]`.
- `tts_tool.rs:928`, `mcp_tool.rs:145`, `session_search_tool.rs:267`, `tts_command_provider.rs:80`
  all carry reasons.

Zero bare `.unwrap()` / `.expect()` in non-test code across the slice.

**`anyhow` in a library crate: clean.** `operant-core` tool modules return
`Result<T, ToolResult>` / `Result<T, crate::error::Error>` / `Result<T, String>` (the fence
functions). `anyhow` appears only in `operant-cli` binaries (`gateway_runner/enrichment.rs:7`,
`cmd_tools.rs`), which is correct per the repo rule.

**`.clone()` in loops: no smoking gun.** The loop-adjacent clones found are all
`Arc`/`String` clones of values that must outlive the iteration (`sub_agent_tool.rs:1169`
worker-id into a `static` registry, :1214-1215 answer/error text into result vecs,
`skills_tool.rs:693` a path into a dangling-pointer list). Each is ownership transfer, not a
redundant clone.

**`String`/`Vec<T>` params that should be `&str`/`&[T]`: none found in the tool trait surface.**
`OperantTool::execute(&self, args: Value, context: ToolContext)` takes owned `Value` because it
is consumed. The helpers use `&str`/`&[T]` correctly (`validate_path(raw_path: &str)`,
`holon_write_fence(raw_path: &str)`, `cap_content(content: &str)`,
`dedupe_by_id(items: Vec<TodoItem>)` — owned because it returns a new vec).

**Needless `.collect()`: two minor ones, both deliberate or cheap.**
- `tools.rs:454-459` `normalize_arg_keys` collects the rename list before mutating the map —
  required, since the map is borrowed by the iterator.
- `tools.rs:1200-1209` `handlers` collect exists to drop the read guards before the first
  `.await`; the comment says so. Correct.

**`large_enum_variant`: none in the slice.** `ToolExposure`, `ToolInterceptPoint`,
`PipelineAction` are all small. `WorkerStatus` (`sub_agent_tool.rs:1194`) is boxed at the
boundary where it is stored.

**`#[allow]` where `#[expect]` + reason belongs: one instance.**
`pool_adapter.rs:100` carries `#![allow(clippy::unwrap_used, clippy::expect_used)]` at the top of
the `#[cfg(test)]` module. Test targets are exempt from the gate anyway, so the attribute is
redundant but harmless.

**Send/Sync: clean.** `ToolRegistry`'s shared state is all `Arc<RwLock<…>>` (tokio) or
`Arc<std::sync::RwLock<…>>` for the boot-only `aliases` map; `ToolExecutor::overrides` is
`Arc<std::sync::Mutex<HashMap>>` with a documented reason for choosing `std` over `tokio`
(`tools.rs:413-419`: written once at boot from async context, read for a copy on every execution
with no await while held). No guard is held across an `.await` in any path I read.

---

## FINDING SUMMARY

| id | rank | one line |
|----|------|----------|
| A2-1 | HIGH | gateway `/search` constructs `SessionSearchTool` and executes it directly, bypassing the disabled-tool gate (`gateway_runner.rs:1962`) |
| A2-2 | MEDIUM | `tts` is advertised in `builtin_tool_names()` but no tool has that name (the real tool is `text_to_speech`), so `operant tools disable tts` matches nothing |
| A2-3 | MEDIUM | `lcm_*` and `ctx_*` tools are registered live from `main.rs` but absent from `builtin_tool_names()`, so the advertised surface under-reports the live surface |
| A2-4 | LOW | `vision_tool`, `file_state` and `working_diff_tool` take model-supplied paths without `validate_path` |
| A2-5 | LOW | `MemoryRecallTool` is dead code (registered nowhere; its name is an alias to `memory_search`) |
| A2-6 | LOW | `debug_env` masks by substring only; credentials under non-matching names print in full |
| A2-7 | LOW | `code_execution` is not a sandbox (documented, accepted) — every path guard here is advisory against it |

---

## COVERAGE

Read in full or in the relevant regions:
`tools.rs` (2104 L — trait, `ToolExecutor`, `ToolRegistry`, all schema paths, `execute`,
aliases, the 200-line test module), `tools/builtin.rs` (621 L — every `register` call,
`builtin_tool_names`, the count tests), `tools/file_tools.rs` (1015 L — `DENIED_PATH_PATTERNS`,
`atomic_write`, `holon_command_fence`, `early_canonical`, `validate_path`, `FileReadTool`),
`tools/todo_tool.rs` (1487 L — store, `todo_state_path`, `load_session`, `persist_session`, the
merge path), `tools/file_state.rs` (587 L — `compute_hash`, `collect_dir_files`, all three
handlers), `tools/tool_search.rs` (1428 L — `assemble_tools` contract, `ToolCallTool::execute`,
the disabled-bypass test), `tools/transcription_tool.rs` (419 L), `tool_guardrails.rs` (1886 L —
thresholds, `observe`, `observe_result`, `observe_output`, message builders),
`tools/sub_agent_tool.rs` (4175 L — registry build, `register_child_tools`, worker fan-out),
`tools/async_delegation.rs` (2047 L — registry, ledger, persistence), `tools/memory_tools.rs`
(1365 L — all three tools), `tools/debug_helpers.rs` (259 L), `tools/harness_tools.rs` (495 L),
`tools/ctx_tools.rs` (609 L), `tools/lcm_tools.rs` (1711 L — registration surface + timeout
override), `tools/vision_tool.rs` (876 L — `resolve_file_path`, `resize_image_for_vision`),
`tools/working_diff_tool.rs` (319 L), `tools/code_execution.rs` (723 L — module doc, subprocess
build), `tools/kernel/mod.rs` (636 L), `aft_bridge.rs` (1518 L — module doc, tool mapping table),
`agent/stream.rs` (the three dispatch arms + `tool_context` + `preflight_tool_calls` seam),
`agent/mod.rs:1150-1275` (the concurrency predicates), `agent/prompting.rs:583/840-900`
(background review dispatch), `mcp.rs:2225-2470` (both MCP tool impls),
`pool_adapter.rs` (full), `harness_adapters.rs:50-130` (tool seam install),
`crates/operant-cli/src/main.rs:1300-1650` (registry build, AFT dedup, ctx/lcm registration),
`crates/operant-cli/src/cmd_tools.rs:56-266` (list/disable/defer),
`crates/operant-cli/src/gateway_runner.rs:1900-2000`, `gateway_runner/enrichment.rs` (full),
`crates/operant-cli/src/mcp_serve.rs:380-420`, `crates/operant-cli/src/plugin_tools.rs:40-100`,
`crates/operant-cli/src/memory_provider_tools.rs`, `crates/operant-core/src/tools/tool_state.rs`
(`disabled_tool_names`, `apply_disabled_tools`).

Not read in this slice (owned by other workers): the gateway adapters, the TUI, the org/holon
layer, `context/`, `cronjobs/`, `database.rs`, the dashboard.

Method notes: no working-tree file was read — every code read was
`git show origin/main:<path>`. The one file read from disk was
`docs/audit/2026-11-16-arch-redteam/CHARTER.md`, which is untracked and is the audit's own
control document, not a file under audit.

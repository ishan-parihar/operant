# BUGS.md — Operant Audit Fixes

## Round 2 (2026-08-06)

### R2-1 — LLM context compressor dead-wired (FIXED c394c517)
`with_llm_compressor` was never called in any binary (cli/runtime/gateway/core all zero callers). The compressor was always `None`, so `compress_context_overflow` always fell back to deterministic decay/evict.
- **Fix**: wired `with_llm_compressor` in both agent factories (`create_runtime_agent` + `create_agent_without_events`) in `operant-cli/src/main.rs`. Compressor self-guards on threshold/cooldown + deterministic fallback. Gated on `config.agent.context_compression` (R4 follow-up: see below).

### R2-2 — Real token usage never drives compression (FIXED c394c517)
Both compression gates used the char/4 `estimate_total_tokens` heuristic instead of actual API `usage.prompt_tokens`.
- **Fix**: added `last_reported_prompt_tokens: AtomicUsize` on `OperantAgent`, recorded in `emit_usage_and_cost`. Both gates now use `estimate_current_tokens` which prefers the reported value via `prefer_reported(reported, fallback)`.

### R2-3 — Memory bifurcation / dead JSON store (FIXED c394c517)
Agent-callable memory tools (`memory_store`/`memory_search`/`memory_recall`) wrote to a naive substring JSON store (`~/.operant/memory/tool_memories.json`) that was never injected into prompts. The real injected store (`MemoryManager`/MEMORY.md) was decoupled.
- **Fix**: added `ACTIVE_MEMORY_MANAGER` global hook in `memory_tools.rs`; tools now delegate to the agent's active `MemoryManager` via `set_active_memory_manager` (wired in `load_memory_manager` in main.rs). JSON store is fallback only when hook is unset (tests).
- **Live-verified**: `memory_store` writes land in MEMORY.md (2 hits for test key), `tool_memories.json` stays empty (0).

## Round 2 Follow-Up (2026-08-06)

### R2-1 follow-up — Compressor config-gate (FIXED 7960e614)
R2-1 wired `with_llm_compressor` unconditionally with `..Default::default()` (enabled=true), overriding user's `context_compression = false`.
- **Fix**: gate `enabled` on `config.agent.context_compression`, use `config.agent.context_compression_threshold` for `threshold_percent`.

## Round 4 (2026-08-06)

### R4-1 — Empty-response retry ladder (FIXED 7960e614)
Free-tier providers intermittently return empty assistant responses (no text, no reasoning, no tool calls). Operant silently accepted these as final answers instead of retrying.
- **Fix**: added `empty_content_retries` counter in the per-iteration tool loop (`operant-core/src/agent/mod.rs:1137`); retries up to `max_retries` (3) by appending the empty assistant turn as a nudge (mirrors hermes-agent `conversation_loop.py` empty-retry loop).
- **Live-verified**: `WARN Empty assistant response — retrying (1/3)` fired on a real empty turn, task recovered (10 files created).

## Round 3 (2026-08-06)

### R3 — Credential pool dead-wired (FIXED 1da115a9)
`with_credential_pool` had zero callers; the pool was never built/attached, so `try_rotate_credential` always returned `None`. Multi-key rotation (a hermes runtime feature) was unreachable even when `credential_pool.enabled=true`.
- **Fix**: shared attach helper in both agent factories — seeds from provider env var + `client.additional_api_keys`, attaches when `config.credential_pool.enabled` and pool non-empty.
- **Live-verified**: "Attached credential pool, provider: openai, creds: 2" + "Credential rotated — client API key updated, rotation_count: 1" on auth failure.

## Round 4 (2026-08-06)

### R4-1 — Empty-response retry ladder (FIXED 7960e614)
Free-tier providers intermittently return empty assistant responses (no text, no reasoning, no tool calls). Operant silently accepted these as final answers instead of retrying.
- **Fix**: `empty_content_retries` counter in the per-iteration tool loop (mod.rs:1137); retries up to `max_retries` by appending the empty turn as a nudge (hermes-agent `conversation_loop.py` empty-retry parity).
- **Live-verified**: `WARN Empty assistant response — retrying (1/3)` fired on a real empty turn from free-tier model; task recovered.

## Known/Deferred

### Bug #1 — Session aggregate counters dead (FIXED 9eb8f4a4)
`sessions` table counters (message_count, tool_call_count) stayed 0 despite persisted rows. `save_message`/`save_message_full` never incremented them.
- **Fix**: rewrote both writers to increment counters in a transaction mirroring hermes (`hermes_state.py:6361`). Live-verified: fresh session shows `message_count=2`.

### Bug #2 — `session_events` table fully dead (YAGNI-flagged)
`record_event` on `session_events` has zero runtime callers; never read by CLI; hermes has no such table. Dead scaffolding — removal skipped pending user direction.

## Round 5 (2026-08-08) — R5 memory-store split-brain (FIXED)

### R5-1 — CLI `memory` subcommands point at a phantom store (FIXED)
The agent's memory tools persist to `~/.operant/MEMORY.md`/`USER.md` (`load_repo_memory_manager` → `storage_dir = operant_home()`), but the CLI `operant memory *` commands built their `MemoryManager` with `operant_home().join("memory")` (`~/.operant/memory/`) — a directory the agent never reads. Result: `operant memory list/search/get/stats` saw nothing the agent stored, and `operant memory store` wrote into the void. Three divergent locations existed for one concept (agent → root `MEMORY.md`, CLI → `~/.operant/memory/`, doctor → `~/.operant/memories/`).
- **Fix (R5-1)**: `cmd_memory.rs::memory_manager()` now uses `operant_home()` — the same store as the agent. `cmd_stats` file-size check fixed to the same dir.
- **Live-verified**: `operant memory search teal` and `operant memory get live_test_color` now return the agent-stored entries; `operant memory stats` reports the real store (212 entries, 44 KB).

### R5-1b — CLI `memory store`/`import` silently dropped writes (FIXED)
`MemoryManager::store()` marks the manager dirty instead of writing synchronously (batch-write optimization for distillation loops). The CLI write commands called `store()` and then the process exited — the dirty flag was never flushed, so the write was lost. Only `delete`/`clear` flushed.
- **Fix**: `cmd_store` and `cmd_import` now call `save_to_disk()` after mutating (matches the agent's `memory_store` tool which already flushes).
- **Live-verified**: `operant memory store cli_roundtrip_key ...` → entry present in `~/.operant/MEMORY.md` (`grep` count 1).

### R5-1c — CLI `memory prune` was a preview-only no-op (FIXED)
`operant memory prune` printed candidates and then instructed the user to use `clear` — it never pruned, contradicting its stated purpose.
- **Fix**: added `MemoryManager::remove_block(id)` (marks dirty, mirrors `store`); `cmd_prune` now removes eligible blocks and flushes. Unit test `test_remove_block_removes_and_flushes` added.

### R5-2 — Doctor probed a phantom `memories/` subdir (FIXED)
`operant doctor` checked `~/.operant/memories/MEMORY.md` and warned "memories/ not found" even when the agent's real store at `~/.operant/MEMORY.md` was healthy.
- **Fix**: `checks_config.rs` probes root `MEMORY.md`/`USER.md` directly; removed `memories` from the subdir existence loop.
- **Live-verified**: doctor now reports `✓ MEMORY.md exists (44179 chars)` / `✓ USER.md exists`.

### R5-3 — Audit verdict: operant-runtime `RuntimeAgent` stack is dead-linked legacy (FLAGGED)
`crates/operant-runtime/src/agent/` (`agent.rs`, `loop_.rs`, `classifier`, `context_analyzer`, `context_compressor`, `memory_loader`, `loop_detector`, `history_pruner`, `dispatcher`) has **zero non-test callers** across all crates; the CLI/TUI/gateway use `operant-core::OperantAgent`. `operant-runtime` is pulled in only as an optional dep via the default `agent-runtime` feature (its personality/tools/security/cron submodules ARE used by the gateway). This is legacy scaffolding — not a live divergence, since the shipped binary never executes it. Recommendation: remove the dead agent modules (keep the gateway-used submodules) in a dedicated cleanup round. Its unit tests (including a well-tested `loop_detector`) are the only thing exercising the code today.

### R5-5 — Pre-existing workspace clippy debt (DEFERRED, not from this round)
`cargo clippy --all-targets -- -D warnings` fails on ~40 pre-existing lints in untouched files (`collapsible_if` in `accessibility.rs`/`background_review.rs`/`insights.rs`/`message_safety.rs`/`turn_context.rs`/`turn_finalizer.rs`/`mod.rs`/`fallback.rs`; `needless_mut` in `database.rs:2101`; `sort_by_key` + `manual_div_ceil` in `insights.rs`/`llm_compressor.rs`; `format_push_string` in `gemini_oauth.rs:274`). The R5 round fixed the one lint blocking the touched path (`question_mark` in `operant-tool-call-parser/src/lib.rs:922`). None of the R5 files carry lints. A dedicated lint-cleanup round (like the `#[expect(dead_code)]` migration already in-flight in the working tree) should sweep the rest.

### R9-1 — Browser `navigate`/`snapshot` had no SSRF guard (FIXED)
The `browser` tool's `navigate` (and `snapshot`, which reloads the current URL) passed URLs straight to every provider — including **local browser binaries** (lightpanda/obscura/igs) that fetch the URL directly. With no URL-safety check, the agent could be prompted to navigate to cloud metadata (`169.254.169.254`), localhost services, or internal addresses — the same SSRF class closed for `http_request`/`web_fetch` in R6. Hermes guards every browser navigation with `tools/url_safety.is_safe_url` (fail-closed). The `accessibility_tree` command connects only to a configured CDP URL (operator-controlled env var), so it was not in scope.
- **Fix**: `BrowserTool::execute` now runs `ssrf_verdict(url)` for `navigate`/`snapshot` commands before dispatching to any provider — one guard covering all 7 providers (igs/lightpanda/obscura/camofox/browserbase/browser-use/firecrawl).
- **Tests**: 3 new unit tests (cloud-metadata IP, loopback, metadata hostname) — browser_tool suite now 14 tests, all pass.
- **Live-verified** on deployed binary: `browser navigate http://169.254.169.254/latest/meta-data/` → tool returns `"URL blocked: points to private/internal address (SSRF protection)"`.
- **Clippy**: pre-existing `collapsible_if` in the touched file was fixed while in the file (`if matches!(...) && let Some(url) = …`).
- **Scope note (review-verified)**: `browser_cdp` (`browser_cdp_tool.rs`) only drives a *pre-connected operator-owned* CDP session via `BROWSER_CDP_URL` — it has no model-supplied navigate URL (no `Page.navigate` path taking model input), and `browser_camofox_state` carries no URL fields. `browser_downloader` fetches only fixed GitHub release URLs (binary auto-download, not agent-controlled). So the R9 guard covers the only model-URL-driven browser entry point.

## Round 10 (2026-08-08)

### R10-1 — `--force` could override a *dangerous* skills_guard verdict (FIXED)
`should_allow_install` treated `force` as a universal override (`Verdict::Block => if force { Some(true) }`) — so a skill from a **community** or **trusted** source carrying a **dangerous** verdict (critical findings: exfiltration, destructive, injection, embedded credentials…) could be installed with `--force`. Hermes's `tools/skills_guard.py::should_allow_install` explicitly refuses this: `if force and not (verdict == "dangerous" and trust_level in ("community", "trusted"))` — dangerous verdicts from community/trusted sources are hard-blocked and *cannot* be force-overridden; `--force` only bypasses non-dangerous blocks and the agent-created "ask" decision.
- **Fix**: `should_allow_install` now computes `dangerous_hard_block` (dangerous verdict + community/trusted trust level) and returns `Some(false)` with "…--force does not override a dangerous verdict." even when `force` is set. Non-dangerous blocks and agent-created ask decisions remain force-overridable (hermes-identical).
- **CLI parity**: `cmd_skills.rs` blocked-message no longer unconditionally advises "re-run with --force" (detects the hard-block reason and omits the hint); `skill_marketplace.rs` dangerous-block error no longer promises a `--force` override that doesn't exist. (Follow-up per review: the severity breakdown now includes critical findings — previously a critical-only skill printed "0 high, 0 medium, 0 low".)
- **Tests**: `test_force_overrides_dangerous_for_{community,trusted}` rewritten → `test_force_cannot_override_dangerous_for_{community,trusted}` asserting the hard block; `test_force_overrides_dangerous_for_agent_created` unchanged (ask→force still allowed, matching hermes).
- **Live-verified** on deployed binary: `skills install` of a file containing `rm -rf /` with `--force` → blocked, reason contains "--force does not override a dangerous verdict."; a caution-verdict skill with `--force` → force-installed; a benign skill → installed. Test skills removed from the hub afterwards.
- **Also fixed while in file**: pre-existing `collapsible_if` lints in `skill_marketplace.rs` (cache-hit + cache-save let-chains) and `skills_guard.rs` (`content_hash`).
- **Review note (kanban dispatcher)**: `Dispatcher::claim_task` (pending_tasks read → INSERT run → UPDATE task) is not one atomic claim; under *concurrent* workers two could race to claim the same task. All callers are single-process CLI subcommands (`cmd_kanban.rs`), and hermes-agent-ultra's kanban claim is an in-memory object mutation — no hermes divergence; future-hardening only.

## Round 11 (2026-08-08)

### R11-1 — SSH terminal backend built remote commands by string concatenation (FIXED)
`SshBackend::execute_command` prefixed the remote command with raw `cd {cwd} && ` and `export {k}="{v}" && `. Both components are model-controlled (`working_dir`/`env_vars` tool args), and neither was shell-quoted: `$()`/backticks/`;`/`&&` inside a `working_dir` or env value would execute on the remote host (double quotes do NOT stop `$()`/backtick expansion). Hermes's remote execution defends with an allowlist `_validate_workdir` (shell metacharacters rejected outright) + `shlex.quote` on every cwd/env/path (`hermes-agent/tools/terminal_tool.py`, `tools/environments/ssh.py`).
- **Fix**: extracted `SshBackend::build_remote_command` — `cwd` is `shell_words::quote`d, env values are `shell_words::quote`d (single-quote style, no expansion), env names are validated against `[A-Za-z_][A-Za-z0-9_]*` (rejecting separator injection via the name), and the command itself stays shell-quoted in shell mode. The whole remote command is still wrapped in `bash -c {quoted}`.
- **Tests**: 3 new — env value with `$(rm -rf …)` stays single-quoted, cwd with `;` payload is quoted (`cd '/tmp; touch /tmp/pwn'`), env-name injection (`X; touch …`, `1BAD`) is rejected. Docker backend verified unaffected (env/cwd passed as `-e`/`-w` argv elements, no shell).

### R11-2 — Unimplemented terminal backends silently fell back to LOCAL execution (FIXED)
`TerminalBackend` declares `Modal`/`Daytona`/`VercelSandbox`/`Singularity` (serde `snake_case`), but `create_backend`'s catch-all arm logged a `warn!` and returned `LocalBackend`. A hermes-style `terminal_backend = "modal"` config — expected to run commands in a cloud sandbox — would silently execute **unsandboxed on the host** (warn-level log only). Hermes implements these backends; operant does not, so the safe behavior is to refuse.
- **Fix**: `create_backend` now returns `anyhow::Result<Box<dyn TerminalBackend>>` and **fails closed** on unimplemented backends with "Terminal backend '{k}' is not implemented in the Rust port — refusing to fall back to unsandboxed local execution. Use \"local\", \"docker\", or \"ssh\"." The terminal tool surfaces this as a tool error.
- **Tests**: `create_backend_refuses_unimplemented_backends` (all 4 variants refused with the fail-closed message) + `create_backend_default_is_local`.
- **Live-verified**: temporarily set `terminal_backend = "modal"` in `~/.operant/operant.toml` → agent run reports the refusal message; config restored to `local`.
- **Also fixed while in file**: pre-existing `collapsible_if` in `DockerBackend::find_docker`; removed now-unused `warn` import.

## Round 12 (2026-08-08)

### R13-1 — WebSocket chat / node-discovery / SSE surfaces were never routed (FIXED)
`handle_ws_chat` (ws.rs, 1383 lines), `handle_ws_nodes` (nodes.rs), and `handle_sse_events`/`handle_events_history` (sse.rs) are fully implemented with auth + approval machinery, but the gateway router only nested `/api/*` — **no route ever registered `/ws/chat`, `/ws/nodes`, `/api/events`, or `/api/events/history`**. Every connect got a 404; the entire WS chat + dynamic node-discovery + SSE event-stream features were unreachable dead code. Git history shows `handle_ws_chat` was never referenced in the router — dead-wiring from the start, not a regression.
- **Fix**: `api::router` now registers `/ws/chat` + `/ws/nodes` at top level (matching the handlers' documented URLs), and `build_api_routes` registers `/events` + `/events/history` inside the `/api` nest. 2 new router-level tests: (a) all four routes respond non-404 (regression guard), (b) with pairing enabled the WS routes are gated (never 200/404).
- **Live-verified**: gateway boots clean and the routes resolve; SSE stream responds 200.

### R13-2 — axum 0.7 `:id` route syntax panics router build on axum 0.8.9 (FIXED, latent crash)
The crate pins axum 0.8 (which requires `{param}` capture syntax; `:param` panics at router construction with "Path segments must not start with `:`"), but `build_api_routes` still used `/cron/:id`, `/cron/:id/run`, `/cron/:id/runs`, `/memory/:key`, the test helper `/api/cron/:id/run`, and `dashboard_server.rs` used `/assets/:filename`. Any real invocation of the HTTP gateway or the dashboard crashed at startup (uncovered while writing the R13-1 route test — the panic fired at router build).
- **Fix**: all colon captures converted to `{id}`/`{key}`/`{filename}`; axum 0.8 handles the rest. Verified live: `operant dashboard server` now boots and serves `/api/health`=200, `/`=200; the gateway boots clean.

### R13-3 — HTTP gateway (`run_gateway`) is dead-wired: zero callers in the shipped binary (FLAGGED)
`crates/operant-gateway` is compiled in via the default `gateway` feature, but `run_gateway` (the axum HTTP server with all `/api/*` + WS + SSE routes) has **zero non-test callers** anywhere in the workspace. `operant gateway run` starts the channel/adapters gateway (`gateway_runner`), and `operant dashboard` uses a separate small axum server in `dashboard_server.rs`. The entire HTTP surface (cron API, pairing, WS chat, node discovery, SSE) is unreachable from the shipped binary — dead code kept alive only by its own unit tests. Recommendation: wire `run_gateway` to a CLI subcommand (e.g. `operant web`) or remove it in a dedicated cleanup round.

### R13-4 — `[memory] audit_enabled` is dead config: `AuditedMemory` never applied (FLAGGED)
`audit_enabled`/`audit_retention_days` exist on the memory config, and `hygiene.rs` prunes `memory/audit.db` entries older than the retention window — but the only thing that *creates* and *writes* that table is the `AuditedMemory` decorator, which has **zero callers** outside its own module. The live path uses the file-backed `MemoryManager`, never the `Memory`-trait backends the decorator wraps. Net effect: setting `audit_enabled = true` does nothing (hygiene finds no audit.db and skips). Recommendation: apply the decorator in the memory-backend factory when `audit_enabled` is set, or drop the config to avoid the silent no-op.

### R13-5 — Response cache is dead config in the live agent (FLAGGED)
`[memory] response_cache_enabled` / `response_cache_ttl_minutes` / `response_cache_max_entries` (schema) and the CLI's `openrouter.response_cache`/`response_cache_ttl` are parsed, but the only consumer of `ResponseCache` is the **dead-linked `operant-runtime` agent stack** (`agent.rs`). The live `OperantAgent` (operant-core) never reads or writes the cache — the config silently does nothing. Hermes has a real OpenRouter response cache (`auxiliary_client.py`, `HERMES_OPENROUTER_CACHE`) wired into its live client path. Recommendation: wire `ResponseCache` into the live agent's provider layer (with the existing hot-cache/SQLite implementation) or drop the dead config.

### R12-1 — `code_execution` claimed sandboxing but runs on the host; misleading docs (FIXED-docs / FLAGGED)
`code_execution.rs`'s module doc claimed "secure code execution in a sandboxed environment", and the interactive permission prompt said "This executes code in a sandbox" — but the tool writes a temp file and runs python3/node/bash/rustc **directly on the host** (no bubblewrap/firejail/nsjail/unshare/container; mitigations are timeout + `kill_on_drop` + the approval gate). Hermes's `code_execution_tool.py` runs code in a real sandboxed subprocess (AF_UNIX transport, env hardening #27303, allowlisted in-sandbox tools) with docker/modal/daytona/vercel_sandbox backends. A model could be led to believe risky code was isolated when it was running with the process's own permissions.
- **Fix (docs)**: module doc now states plainly that execution is NOT sandboxed (runs with the operant process's permissions), lists the actual mitigations, and points at this entry; the permission prompt now says "runs code on your system with the operant process's permissions (not sandboxed)".
- **FLAGGED**: full sandbox parity (hermes's sandboxed subprocess protocol) is future work, not a bug-fix-round change. Minor hygiene note: temp script files (UUID-named in the system temp dir) are not removed on the timeout path.

### R12-2 — `checkpoint` tool was dead-wired AND committed into the user's repo (FIXED)
Two stacked bugs, found via live testing (the tool failed in a real git repo with changes):
1. **Dead by default, un-enableable**: `CheckpointConfig::default()` had `enabled: false` with a comment "enable via config" — but **no config path existed** (no `[checkpoints]` section in the schema/config, no `configure()`/`set_enabled()` caller anywhere). Every `checkpoint ensure` call returned "Failed to create checkpoint (may be disabled or no changes)" and the auto-checkpoint-before-mutation path never fired. The tool was advertised to the model while permanently inert — the same dead-wiring pattern as R2-1/R2-3/R3.
2. **User-repo pollution**: `take_checkpoint` ran `git add -A` + `git commit` **inside the user's working repository** (staging everything + creating "checkpoint" commits in the user's history). Hermes's `checkpoint_manager.py` is explicitly *not* a tool and snapshots into an isolated shadow store via `GIT_DIR` + `GIT_WORK_TREE` + `GIT_INDEX_FILE` — "no git state leaks into the user's project directory".
- **Fix**: (a) wired `[checkpoints]` config — new `CheckpointsSettings` on `AppConfig` (`enabled`, `base_dir`, `max_snapshots`; default disabled, serde-defaulted so existing configs load unchanged) + both CLI agent factories call `configure_checkpoints(config)`; (b) rewrote the git layer as a **shadow store**: per-workdir bare repo at `~/.operant/checkpoints/store/<sha256-prefix>` with `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` env isolation, default excludes (`.git`, node_modules, target, …), and an internal commit identity (`GIT_AUTHOR_*`) so the user's git identity is never required. Snapshots now work in **any** directory (project git repo no longer needed) and never touch the user's repo. `ensure`'s error message now says to set `[checkpoints] enabled = true`. `CheckpointManager` config moved behind a mutex (the global lives in a `OnceLock`).
- **Followup (review)**: `max_snapshots` was dead config — enabling checkpoints meant unbounded store growth. `take_checkpoint` now prunes via `git update-ref` (moves the store's branch ref to `HEAD~excess`, keeping the newest N commits) — safe on a bare store, never touches the user's index/worktree. `ensure_checkpoint` also threads the caller-supplied `reason` through instead of hardcoding "auto checkpoint". Unit test `test_max_snapshots_caps_commits` added.
- **Followup (review round 2)**: `update-ref` only hid dropped commits — their objects stayed in the store forever, so growth was only half-capped. After a successful ref move the store now runs `git gc --prune=now --quiet` (with the same `GIT_DIR` env, so it operates on the store, never the user's repo), and a failed prune now `warn!`s instead of being silent.
- **Tests**: `test_shadow_checkpoint_roundtrip` (snapshot → list → mutate → snapshot → restore, asserting no `.git` leaks into the workdir), `test_store_name_is_stable_and_distinct`, `test_checkpoints_disabled_by_default`.
- **Live-verified**: with `[checkpoints] enabled = true`, `checkpoint ensure` returns `{"message":"Checkpoint created in .","success":true}`, the store appears at `~/.operant/checkpoints/store/<hash>`, and the user's repo remains at exactly 1 commit with the working change intact (no pollution). Config restored afterwards.

### R8-1 — Web-search providers ignored HTTP error statuses (FIXED)
None of the four web-search providers (`DDGProvider`, `SearXNGProvider`, `TavilyProvider`, `ExaProvider`) checked `resp.status()` before parsing the body. A 401 (bad/expired API key), 429 (rate limit), or 5xx error envelope would be fed to `serde_json` as if it were results — producing a confusing `ParseResponse` error or a *silent empty result list* (the `web_search` tool reports `0 results` for an auth failure). Hermes's Tavily/Exa plugins call `response.raise_for_status()` so failures surface as typed errors.
- **Fix**: all four providers now check `!resp.status().is_success()` and return `Error::Provider { status, body, retry_after }` with the HTTP code + raw error body — mirroring hermes `raise_for_status`. (Captured `status` before consuming the body to avoid a move error.)
- **Live-verified**: `web_search` still executes and returns results with the deployed binary.

### R7-1 — Web-search query encoding was not URL-safe (FIXED)
The DDG and SearXNG providers built their search URLs with a naive `query.split(' ').join("+")` — spaces became `+` but reserved characters (`&`, `?`, `#`, `=`, non-ASCII/CJK) were passed through raw. A query like `C++ & Rust` would inject `& Rust` as a *new query parameter* (or truncate the URL at `?`), producing wrong or mangled search requests. Hermes percent-encodes query components (`urllib.parse.urlencode`/`quote_plus`).
- **Fix**: added `web_providers::urlencode()` using the `url` crate's form-urlencoded byte serializer (spaces→`+`, reserved + non-ASCII percent-encoded); wired into `DDGProvider` and `SearXNGProvider` (which previously duplicated the same broken helper, now removed).
- **Tests**: 3 new unit tests (spaces→`+`, reserved chars `%2B`/`%26`/`%3F`/`%23`/`%3D`, CJK byte-encoding).
- **Live-verified**: `web_search` for `C++ and Rust memory safety` executes with no URL-parse/injection error.

### R7-2 — IGS auto-preference overrode explicit provider config (FIXED)
`web_tools.rs` computed `want_igs = settings.preferred_provider == "igs" || igs_available` — so if a user explicitly configured `preferred_provider = "tavily"`/`"exa"`/`"searxng"`, an installed `igs` binary silently hijacked the search anyway. Hermes's `web_search_registry` resolves the explicit `web.search_backend` config first and only auto-selects among *available* backends when the config key is unset.
- **Fix**: `want_igs` now matches hermes semantics — IGS is used only when `preferred_provider` is `"igs"`/`"auto"`/unset (and igs is available); any explicit non-igs choice is respected.
- **Tests**: existing provider-selection tests still pass; behavior verified live (provider correctly resolved to DuckDuckGo in the default config).

### R6-1 — `http_request` and IGS web tools had no SSRF protection (FIXED)
Operant ships a shared SSRF oracle (`security::check_url_safety` — DNS resolution + blocked ranges: loopback, link-local, RFC 1918, CGNAT, benchmark, reserved, cloud-metadata hostnames `metadata.google.internal`/`metadata.goog`, fail-closed on DNS errors). Before R6 it guarded exactly ONE caller: `vision_tool`. Meanwhile `http_request` (arbitrary method/headers/body to any URL) and the IGS-backed `web_scrape`/`web_extract` had **no** guard at all, and `web_fetch` used a weaker local `check_ssrf` (no DNS resolution, no metadata-hostname blocklist, no benchmark/reserved ranges). Hermes Python applies its `tools/url_safety.is_safe_url` oracle fail-closed to every URL-fetching tool (browser_tool.py etc.) — so this was a real parity + security gap: an agent could be prompted to fetch `http://169.254.169.254/latest/meta-data/` (cloud credentials) or hit internal services.
- **Fix**: added `security::ssrf_verdict(url) -> (bool, String)` helper; wired it into `http_request`, `web_fetch` (replacing the weaker local `check_ssrf`, which was removed), `web_scrape`, `web_extract`, and `xai_http_request` (post base-URL assembly). `xai_http` base-url unit test updated: `0.0.0.0` is now correctly blocked pre-flight.
- **Tests**: 8 new unit tests (cloud-metadata IP, loopback, metadata hostname for `http_request`; metadata IP, RFC 1918, public-IP allowed for `web_fetch`; metadata IP for `web_scrape` + `web_extract`) — all pass. Full `operant-core` lib: 1194 passed; `operant-cli`: 631 passed.
- **Redirect hardening (review follow-up)**: `http_request`, `web_fetch`, and `xai_http_request` now build their reqwest clients with `redirect::Policy::none()` — the SSRF guard validates only the initial URL, and silently following a redirect to a private/metadata address would have bypassed it. The 3xx response (with `Location` header) is returned to the model, which can re-issue against the new URL (which then goes through the guard). `vision_tool`'s pre-existing `download_image` still follows up to 10 redirects without re-checking — same latent bypass, flagged for a follow-up round. Known limitation documented: DNS-rebinding TOCTOU (check and connect resolve separately) remains, matching the hermes-Python guard's behavior.
- **Live-verified** on deployed binary: prompted the agent to `http_request` `http://169.254.169.254/latest/meta-data/` → tool returned `"URL blocked: points to private/internal address (SSRF protection)"` and the metadata endpoint was never contacted.
- **Clippy**: the two pre-existing `collapsible_if` lints in the touched `http_tool.rs`/`web_tools.rs` body blocks were fixed while in the file (`.filter(|b| !b.is_empty())`).

### R5-4 — Live core-loop validation (PASSED)
Ran the shipped binary (v0.1.4) against `~/.operant/operant.toml` (kilo.ai gateway, `nvidia/nemotron-3-ultra-550b-a55b:free`, 128k window, free-tier rate limits): a 4-iteration / 3-tool-turn task exercised `file_read` + `memory_store` + `memory_recall` end-to-end.
- **Verified**: file read correct; memory entry persisted to `~/.operant/MEMORY.md` and recalled by key; assistant+tool messages persisted to `database.db` (`sess_2bb86894…`); turn diagnostics logged (`Turn ended: reason=text_response … api_calls=4/12 … tool_turns=3`).
- Rate limiter (`check_rate_limit`), streaming usage halves, credential-pool attach, empty-response retry ladder, and evolution-trigger split (R1) all confirmed present in the exercised path.
- **Not live-tested**: LLM-compressor overflow path (requires context >80% of the 128k window on the free tier; gate verified by code review — `should_compress` checks `config.enabled`, and unit test `prefer_reported` covers the real-usage gate).

## Round 14 (2026-08-08) — deeper-scan audit (binary live-test + wiring)

### R14-1 — datetime tool rendered every month ≥ February one behind (FIXED)
`days_to_date`'s month loop only assigned `month` on iterations that did NOT break,
so the final (breaking) month was never recorded: Aug 8 2026 rendered as
"2026-07-08" (live-verified — unix timestamp 1786151776 is correct, the formatted
string was one month behind). The unit test only covered the epoch
(`1970-01-01`), which is why it shipped.
- **Fix**: replaced the hand-rolled civil-date math with `chrono`
  (`DateTime::<Utc>::from_timestamp`) — chrono was already a workspace + crate
  dependency, so no new deps. `parse_date`/`is_leap_year` (used by the
  `timestamp` tool) unchanged.
- **Tests**: 4 new regressions — modern date (1786151776 → "2026-08-08 01:16:16"),
  leap day (1709208000 → "2024-02-29 12:00:00"), year end (2019686399 →
  "2033-12-31 23:59:59"), nanoseconds (`%f`). All 19 datetime tests pass.
- **Live-verified** on the rebuilt binary: `operant test datetime` →
  `"formatted":"2026-08-08 01:29:26"` (correct).

### R14-2 — `operant memory delete` silently no-oped on memory entries (FIXED)
`cmd_delete` called `mm.delete_session(id)` — the *session* namespace — while
memory entries live in the *block* namespace (MEMORY.md). Deleting a memory id
printed "Session '...' deleted." but the entry stayed on disk (live-verified:
`operant memory delete audit_r14_live_test` → the entry remained in MEMORY.md).
- **Fix**: `cmd_delete` now tries `remove_block(id)` first (the R5-1c primitive
  `memory prune` already uses), falls back to the legacy session namespace, and
  flushes either way.
- **Live-verified**: `operant memory delete audit_r14_live_test` →
  "Memory entry 'audit_r14_live_test' deleted." and the entry is gone from
  MEMORY.md (grep count 0).

### R14 observations (no code change)
- **Live core-loop test**: with the real `~/.operant` config, `operant run`
  executed 5 tool calls end-to-end (datetime, file_write, file_read,
  memory_store, skills_list); file landed, memory entry landed in MEMORY.md,
  session recorded (218→219), trajectory saved + listable. A second run
  completed 10/10 datetime calls cleanly (exit 0).
- **Reflection wiring**: skill nudge (`advance_skill_trigger`, interval 10,
  mod.rs:1851) and memory review (per-turn, mod.rs:1133) are wired into the
  live `OperantAgent` loop with unit-tested counters; the 10-call run landed
  exactly on the counter boundary (needs the 11th iteration to fire), so the
  nudge itself was not re-observed this round (prior round R2 live-verified it
  at iteration 10).
- **MaxIterationsExceeded UX**: when the iteration budget is exhausted and the
  grace call fails, `run` returns a hard error and the CLI exits 1 with no
  visible output (observed on a 12-append run the free-tier model could not
  finish). Recommendation (not applied): surface the partial session state +
  a closing summary on budget exhaustion instead of silent exit — hermes
  produces a closing summary.
- **Config hygiene**: `~/.operant/operant.toml` still had `[memory] provider =
  "tdg"` (legacy/removed — silently downgraded to BuiltinProvider, so the
  memory-review reflection gate IS live). Changed to `"builtin"` (backup
  kept); doctor no longer flags it.
- **In-flight tree**: the `#[allow(dead_code)]` → `#[expect(dead_code,
  reason=...)]` migration (26 files) was uncommitted; it compiles clean and is
  committed alongside this round.
### R14 review-followups (2026-08-08)
- `%f` zero-padding regression assertion added (`nsecs=5` → `"000000005"`) locking
  parity with the old `{:09}` formatter.
- `operant memory delete` now reports "No memory entry or session with id
  '...' found." when the id exists in neither namespace (was a misleading
  "Session deleted." success message).
- Known follow-up (not fixed): the `timestamp` tool's `parse_date` splits on
  `+`/`:` so a timezone suffix like `+05:30` can be misread as extra time
  components. Out of scope for the R14 format-side fix; revisit if the
  timestamp tool's parse path is ever hardened.

### R14-3 — `webhooks_secret` config was never wired to signature verification (FIXED)

The webhook platform (live path: `operant-core/src/gateway/mod.rs`, `WebhookAdapter`)
had complete HMAC-SHA256 / Slack / Stripe signature verification, and the TOML
schema exposed `webhooks_secret` — but the CLI **never read it**:
`GatewaySettings` (core) lacked the field, `GatewayConfig` lacked it,
`start_gateway`/`build_adapters` never called `.with_secret(...)`, and
`with_secret` had **zero live callers**. Any webhook request was accepted
unsigned even when a secret was configured.

- **Fix**: added `webhooks_secret: Option<String>` to `GatewaySettings` +
  `GatewayConfig`, wired `start_gateway` → `build_adapters` →
  `.with_secret(...)`, and updated all literals. Signature verification now
  actually engages when a secret is present.
- **Test**: `test_webhook_hmac_live_server` boots the real axum server on an
  ephemeral port with a secret and asserts: unsigned request → **401**, wrong
  signature → **401**, correct `sha256=` HMAC → **200** and the payload is
  forwarded to the channel. Live-verified passing.

### R14-4 — Slack `_signing_secret` is a dead field; SlackAdapter gets `None` (FLAGGED)

`gateway/mod.rs` defines `_signing_secret` (underscore-prefixed — Rust treats it
as intentionally-unused, so no dead-code warning fires), and the CLI wires
`SlackAdapter::new(token, None)`. Slack's own webhook signature verification
(implemented in the same file) can therefore never be reached through the Slack
adapter. Same root pattern as R14-3; left FLAGGED because wiring it requiresa `slack_signing_secret` schema field + adapter plumbing (higher churn), and the
webhook adapter now honors its secret.

### R14-5 — `WhatsAppAdapter::with_phone_number_id` is dead-wired (FLAGGED)

`gateway/mod.rs` exposes `with_phone_number_id(...)` but it has **zero live
callers** and no `whatsapp_phone_number_id` schema/config field — the
`WhatsAppAdapter` is always constructed with `None`. This only disables
per-number routing hints (inbound messages still arrive with `platform =
"whatsapp"`), so it's cosmetic; flagged as the same dead-config pattern as
R14-3/R14-4 for a future sweep of `with_*` setters vs callers.

### R15-1 — the operant-channels crate (55K lines) is dead-wired (FLAGGED, major)

`operant-channels/src/orchestrator/mod.rs::start_channels` — the 14K-line
orchestrator with the full dispatch machinery (debouncer, per-sender
interruption, session manager, media pipeline, link enricher, reply-intent
precheck, per-platform listeners for 20+ channels) — has **zero non-test
callers** anywhere in the workspace. `operant gateway run` / `channel start`
use `gateway_runner::start_gateway`, which runs only the 7 operant-core
`PlatformAdapter`s (telegram, discord, slack, whatsapp, email, sms, webhooks).
The only live path INTO the channels crate is `operant acp server` →
`operant-gateway::acp` → `AcpServer`. All the other platform implementations
(irc, mattermost, imessage, matrix, lark, wechat, nostr, qq, twitter, reddit,
notion, linq, wati, nextcloud, mochat, wecom, dingtalk, bluesky, clawdtalk,
line, signal, gmail_push, etc.) are compiled but unreachable in the shipped
binary. Same dead-wiring class as R13-3 (HTTP `run_gateway`) and the dead
RuntimeAgent — the codebase carries parallel implementations and only one path
per surface is wired. Not fixed: rewiring the CLI to `start_channels` is a
large architectural change with its own risks; documented so a future round
can decide which stack is canonical.

### R16-1 — cron `repeat_times` was never enforced — finite-repeat jobs ran forever (FIXED)

`CronDb::mark_job_run` incremented `repeat_completed` on every run but **no code
checked it against `repeat_times`** — a job configured to run N times ran
indefinitely. Hermes (`cron/jobs.py`) enforces the limit: when
`completed >= times` it disables the job, sets `state="completed"`, and clears
`next_run_at` (retaining the record so `last_status`/`last_error` stay
inspectable). Operant had the schema field + the counter but not the check.

- **Fix** (scheduler.rs): `run_job` now computes `repeat_limit_reached(
  repeat_times, repeat_completed)` (pure function, unit-tested) and, on the
  final run, writes the terminal completion shape (`enabled=false`,
  `state="completed"`, `next_run_at=null`) via `update_job` instead of
  scheduling the next run. Delivery of the final response still happens.
- **Fix** (cmd_cron.rs): `cron create` exposed a `--repeat N` flag (previously
  the CLI always stored `repeat_times: None`, making the field unreachable);
  negative/zero is treated as infinite, matching `None` semantics.
- **Tests**: 3 unit tests for `repeat_limit_reached` (reached / not-reached /
  infinite semantics) + 2 DB tests (counter increments; terminal completion
  shape disables the job and it leaves `get_due_jobs`).

### R16-2 — rust-best-practices scan (PASSED / notes)

- Applied the skill's disciplines to the round's edits: extracted a pure
  testable helper instead of inline logic, used `update_job` (no new SQL),
  kept error propagation via `?` / `anyhow::Context`, no `unwrap` outside
  tests, and `#[expect]` over `#[allow]` where the existing code already used
  it.
- Scan results: `cargo clippy --all-features` on operant-core reports 130
  pre-existing style errors (125 `collapsible_if` etc.) but they live in
  vendor-feature code paths not compiled in the shipped binary; the default-
  feature build is clean. ~1786 `unwrap`/`expect` occurrences outside tests
  are almost entirely the `lock().expect("…poisoned")` idiom (a poisoned
  mutex is unrecoverable — a legitimate use), plus tested invariants; no
  user-input `parse().unwrap()` or index-unwrap patterns found in the live
  agent/gateway path.

### R15-2 — `operant channel` subcommands lied or dead-ended (FIXED)

- **`channel start`** printed "Use `operant daemon` to start channels" — but
  **no `daemon` subcommand exists** in the CLI (verified: `error:
  unrecognized subcommand 'daemon'`). → Now calls
  `gateway_runner::start_gateway` directly (same path as `operant gateway
  run`). Live-verified: boots the gateway.
- **`channel send`** printed a fake `"status":"sent"` (JSON) / "Sending..."
  (text) without delivering anything (`// TODO: wire to actual gateway
  sender`). → Now routes through `gateway_runner::send_channel_message`,
  which uses the running gateway when present or a one-shot gateway built
  from config otherwise, and surfaces real errors (unknown platform, nothing
  enabled). Two new tests pin the honest-error paths.
- **`channel bind-telegram`** claimed "Bound Telegram identity" + "The agent
  will now respond" while doing nothing (no allowlist persistence existed for
  it, and the live gateway has no per-user allowlist). → Now reports
  `not-applied` and points to `[gateway] admins` (the live enforcement
  mechanism), in both JSON and text output.

### R17 — gateway `/yolo` wrote a dead metadata key; approvals were never skipped (FIXED)

- **Gateway `/yolo`** toggled a `yolo_mode` session-metadata key that **no
  consumer read**: `grep` across all crates shows `yolo_mode`/`reasoning_override`
  have zero readers outside the command handler itself, and the gateway
  permission receiver (`gateway_runner.rs`, spawned in `start_gateway`)
  prompted on every tool permission request regardless — yet `/yolo`
  advertised "skips approval prompts for destructive operations." The TUI's
  `/yolo` genuinely flips `PermissionMode::BypassPermissions`, which the TUI
  permission flow consumes — the gateway path was a false promise.
  → **Fix**: new live global `gateway_runner::YOLO_CHANNELS` (keyed
  `"{platform}:{channel_id}"`); the permission receiver now checks
  `yolo_enabled()` before prompting and auto-sends
  `ToolPermissionResponse::AllowSession` for YOLO channels. `/yolo` handler
  syncs the set in addition to the metadata, and now supports `/yolo
  [on|off|status]`. New unit test pins the set semantics (per-channel,
  per-platform, clear).
- **Audited the other gateway metadata overrides** (`reasoning_override`,
  `fast_mode`, `footer_enabled`, `voice_enabled`, `personality`,
  `codex_runtime`): all are display-only toggles with no agent-run consumer,
  which is **parity-consistent** — the TUI's `/fast` and `/reasoning` are also
  display-only state toggles, and `voice_enabled` is only consumed by the TUI
  voice-notice UI. Only `/yolo` promised safety-relevant behavior (skipping
  approval prompts) while being dead, so it got the live wiring.
- **Reviewer follow-up (same commit series)**: `/yolo` is now `admin_only:
  true` — previously any channel user could flip it (the registry flagged it
  `false`), auto-approving destructive tool executions for the whole channel;
  the dispatch gate at `gateway_commands.rs:558` now blocks non-admins. Also
  `/yolo status`/toggle now read the live `YOLO_CHANNELS` set (single source
  of truth) instead of the persisted `yolo_mode` metadata — the metadata is
  kept as a record only, so after a gateway restart (set is in-memory) status
  reports the honest OFF instead of claiming ON while prompts resume.
  Steady-state auto-approve log downgraded warn→info.

### R18 — ACP server: lied about state, violated JSON-RPC framing, ignored a flag (FIXED)

- **`status` always reported `idle`** — `AcpCliHandler::agent_state` returned
  `AgentState::Idle` unconditionally, so a client polling status during a
  long-running `command` was told the agent was idle while it was mid-run.
  → **Fix**: new `operant_core::acp::AgentStateTracker` (cloneable, shared
  `Arc<Mutex<AgentState>>`); `execute_command` sets `Running` before the
  `spawn_blocking` run and restores `Idle`/`Error` on completion. `status`
  now reports real state. Unit-tested.
- **JSON-RPC 2.0 framing violations**: (a) a request without an `id` (a
  JSON-RPC notification) failed to *deserialize* and got `-32700 Parse error`
  instead of being honored as a notification with no response; (b) the
  `jsonrpc` version member was never validated — `"1.0"` or missing was
  accepted silently; (c) object/array `id`s were echoed back. → **Fix**:
  `id`/`jsonrpc` are `#[serde(default)]`, new `validate_request` returns
  `-32600 Invalid Request` for wrong version / bad id type, and the stdio
  loop suppresses responses for notifications. Live-verified: notification
  ping produced no output; `"jsonrpc":"1.0"` → `-32600`.
- **`--accept-hooks` was accepted and silently ignored** (`accept_hooks: _`)
  while the server implements no ACP hooks. → **Fix**: the flag now fails
  loudly with a pointer to what implementing hooks requires. Live-verified.
- **Flagged (documented divergence)**: `operant acp server` is a *custom
  operant-native 4-method protocol* (ping/status/command/stop) over stdio,
  not the ACP wire protocol — hermes ships a real 5,832-line ACP adapter
  (`hermes-agent/acp_adapter/`: server.py 2510, session.py 684, tools.py
  1347, permissions.py, provenance.py) with sessions, prompts, permissions
  and provenance. Operant's biggest gap vs hermes: **each `command` spawns a
  fresh agent with no session continuity** (hermes keeps a session
  (`session.py`)). The gateway's ACP-over-WebSocket endpoint
  (`operant-gateway/src/acp.rs`) does use the channels-crate `AcpServer` —
  the one live caller found so far (narrows the R15 dead-wiring note to the
  channels *orchestrator* specifically).
- **Reviewer follow-up (same commit series)**: (1) explicit `"id": null` was
  conflated with a notification (`Option<Value>` collapsed JSON null → None),
  so a spec-valid null-id request was silently dropped — now a  presence-aware `deserialize_with` keeps `Some(Value::Null)`, and the loop only suppresses
  responses for truly omitted ids. (2) `execute_command` early-returned on a
  `spawn_blocking` join failure (`?` before state restore), wedging the
  tracker at `Running` forever — the join error now flows through the same
  restore path and sets `Error` state. Live-verified: explicit null-id ping
  → `{"id":null,"result":"pong"}`; omitted-id notification → no output.

### R19 — MEMORY_SNAPSHOT.md round-trip data loss + fragile hydration FTS (FIXED)

- **Export → hydrate round trip silently corrupted content**: `parse_snapshot`
  treats any line starting with `*Created:` or exactly `---` as decorative
  metadata (dropped), and any line starting with `### 🔑 `` as a new key —
  but `export_snapshot` wrote core-memory content verbatim. A core memory
  whose content contained such a line (markdown `---`, a `*Created:`-style
  note, or a `### 🔑 ``-looking line) was silently truncated or split into a
  phantom key on cold-boot hydration — data loss in the agent's "soul"
  round trip. → **Fix**: export escapes colliding content lines with a
  leading `\`; parse unescapes only lines whose escaped form matches a
  collision pattern (a literal leading backslash in content is preserved).
  New test proves the round trip preserves such content unchanged (fails
  against the old parser).
- **Hydration FTS index depended on a rowid coincidence and swallowed
  errors**: `hydrate_from_snapshot` inserts directly into the
  external-content `memories_fts` without an explicit rowid and without the
  sync triggers (its schema block creates none), and wraps the insert in
  `let _ =` (errors silently ignored). FTS search joins
  `memories_fts f JOIN memories m ON m.rowid = f.rowid`, so index
  correctness relied on FTS auto-rowids coinciding with the content table's
  insertion-order rowids. → **Fix**: after hydrating, rebuild the index
  (`INSERT INTO memories_fts(memories_fts) VALUES('rebuild')`), matching
  sqlite.rs's own reindex approach — consistency is guaranteed regardless
  of rowid assignment, and failures surface loudly instead of being
  swallowed. New test locks in FTS-searchability of hydrated memories.
- **Audited, no findings**: the gateway SSE surface (`sse.rs` — auth,
  `KeepAlive`, lagged-receiver skip, history replay, e2e wiring test) and
  the hygiene prunes (`prune_conversation_rows`/`prune_audit_entries` write
  and prune use the same `Local::now().to_rfc3339()` format; the FTS
  `memories_ad` delete trigger cascades prunes; archive/purge helpers are
  collision-safe and char-boundary-safe) are solid.
- **Reviewer follow-up (same commit series)**: the first escape/unescape
  draft had two correctness gaps in the round-trip contract — (1) a literal
  `\`-prefixed collision line in content (`\---`) was *not* escaped by
  export but *was* unescaped by parse (the checks were not inverses),
  silently dropping the backslash; (2) the export check used `trim_start()`
  while the parser's skip rules use `trim()`, so a `---  ` line with trailing
  whitespace escaped export but was dropped on parse. → Fixed with a shared
  `escape_content_line` helper: export escapes any line whose trimmed form
  starts with `\` or collides, and parse strips exactly one `\` from any
  backslash-leading line — now lossless in both directions (the test's
  escaped-doc simulation also uses the helper, so it cannot drift). The
  strengthened test covers `\---` and mid-content `---  `. (Pre-R19
  snapshots were already corrupted by the original bug; the new format is
  fully lossless.)

### R20 — skill `.usage.json` telemetry write race (FIXED)

- **`.usage.json` was written non-atomically and unlocked.** The main agent
  and the background-review daemon both call `skill_manage` concurrently
  (the review is `tokio::spawn`-ed and keeps running while the next turn
  starts), and multiple operant processes can share a skills dir — so two
  interleaved read-modify-write cycles lost updates or corrupted the file.
  A corrupt `.usage.json` silently unpins skills (`is_pinned` defaults to
  false on parse failure) and zeroes telemetry. Hermes serializes the same
  file with a `.json.lock` (`skill_usage.py`) and never writes it in place.
  → **Fix**: a process-wide `USAGE_TELEMETRY_LOCK` (std Mutex, poison-
  tolerant) around the read-modify-write, plus `atomic_write_json`
  (write `.usage.json.tmp` then rename). New concurrency test: 8 threads ×
  25 `patch` records on a shared tool — the file stays valid JSON and all
  200 `patch_count`s survive. (Lost updates across *separate processes*
  remain possible without an OS file lock, but corruption is no longer
  possible; matches the realistic single-process agent+review case.)
- **Audited, parity-consistent (no fix)**: `use_count` is seeded at 0 and
  never bumped on actual skill usage — but hermes's `record_used`
  (`skill_usage.py:870`) has **zero callers** too, so this is a shared
  dormant-telemetry gap rather than an operant divergence. The learning
  graph's "used skills" stat is therefore always 0 in both implementations.
- **Reviewer verification (closed)**: `skills/.usage.json` has exactly ONE
  writer — `record_usage` (now locked+atomic). The second telemetry
  implementation (`skill_usage.rs` `SkillUsageTracker`) writes a *different*
  file (`.curator/usage.json`, already temp+rename atomic) and is used only
  by `operant curator`; the marketplace/CLI install paths write no usage
  file. No writer bypasses the lock. **Flagged (duplication, not fixed)**:
  operant carries two parallel usage-tracking implementations with different
  schemas and files (`skills/.usage.json` vs `.curator/usage.json`) — the
  same parallel-implementation class flagged in R15 (channels crate) and R3
  (RuntimeAgent); consolidating them is a larger refactor.
- **Audited, solid**: the skill-upgrade pipeline (background-review daemon:
  whitelisted memory/skill tools, write-origin guard, read-before-modify
  guard, protected/hub-installed skill protection from R10, frozen-prefix
  prompt-cache parity, digest replay for routed models) is complete and
  matches hermes's background_review.py pattern.

## Round 21 (2026-08-08)

### R21 — Curator archival pipeline dead: skill_manage never fed the usage tracker (FIXED)
`operant curator` reads `.curator/usage.json` via `SkillUsageTracker`, and `run_review` archives skills filtered by `agent_created` — but **nothing ever populated that file**: `mark_agent_created`/`bump_*` had zero production callers, so `agent_created_records()` was permanently empty and the entire archive/stale pipeline was dead code. Hermes wires this in `skill_manager_tool.py` (`record_created(name, agent_created=is_background_review())` on create, `bump_patch` on patch/edit/write_file/remove_file, `forget` on delete).
- **Fix**: bridged real agent activity into the curator tracker from `SkillManageTool::record_usage` — the existing choke point — under the same `USAGE_TELEMETRY_LOCK`, so the main agent and background-review daemon can't lose curator records. `create` → `record_created(name, is_background_review())` (review-created skills become agent-managed candidates; ordinary creates stay tracked but are never auto-archived), `patch/edit/write_file/remove_file` → `bump_patch` (advances `last_used`), `delete` → `remove`. Added `UsageTelemetry::record_created` + `SkillUsageTracker::{record_created, bump_patch}`. +4 tests (bridge create/patch/delete, corrupt-file tolerance, record semantics, tracker round-trip).

### R21-b — Curator `state.json` non-atomic write (FIXED)
`save_state_inner` wrote `state.json` with a plain `fs::write` — the R20 bug class. A crash mid-save could truncate it, hard-failing `load_state` forever.
- **Fix**: atomic temp + rename, matching the tracker's own save pattern.

### R21-c — Corrupt `.curator/usage.json` bricked skill_manage/curator (FIXED)
`UsageTelemetry::load` propagated JSON parse errors, so one corrupt sidecar hard-failed `operant curator` and (with the new bridge) would silently disable the bridge. Telemetry is disposable — hermes falls back on corrupt telemetry.
- **Fix**: corrupt file now falls back to an empty store with a warning; IO errors still propagate. The bridge then self-heals the file on the next successful save. +2 tests.

### Audited, parity-consistent (no fix)
- **View recording unwired on both sides**: hermes's `skill_view` → `record_view` path is equally dormant (its `record_used` has zero callers), so not wiring `bump_view` is parity, not divergence. Views already feed the review daemon via `mark_review_skill_read`.

### R21-followup — Cross-process telemetry race (FIXED, reviewer-caught)
Review found the bridge's serialization was in-process only: `USAGE_TELEMETRY_LOCK` is a `std::sync::Mutex` (process-local), but `operant curator` runs in a **separate process** whose pin/unpin/restore/archive tracker writes could interleave with the agent's bridge — last-writer-wins on `.curator/usage.json`. The R20 comment claiming "other processes sharing this skills dir" were serialized was false.
- **Fix**: `with_exclusive_file_lock` (std `File::lock` — kernel-managed `flock`, auto-released on crash, no stale lockfiles; hermes's `.json.lock` parity) now wraps the `.usage.json` read-modify-write AND the curator-tracker transaction in `record_usage`. New `SkillUsageTracker::with_exclusive_lock` reloads fresh state from disk inside the lock before mutating, then saves — so neither side clobbers the other's newer writes. `operant curator pin/unpin/restore/archive` now use the same transaction. +1 test (8 threads × separate tracker instances exercising the flock path — all 4 skills survive). Also switched the corrupt-telemetry warning to `tracing::warn!` and documented the non-review `provenance=None` semantics.
- **Flagged (not fixed)**: `curator run` re-loads the tracker at start and saves at end; it does not hold the file lock across LLM consolidation (that would block the agent's skill_manage for minutes). Residual risk is bounded to a lost `last_used`/state update when run_review overlaps an agent write — a full fix (lock around the whole run_review transaction) is a larger refactor.

## Round 22 (2026-08-08)

### R22 — WhatsApp outbound permanently broken: phone_number_id had no wiring path (FIXED)
`WhatsAppAdapter::send_message` requires `phone_number_id` (it is the Graph API URL segment — `graph.facebook.com/v18.0/{phone_number_id}/messages`), and the adapter's own error message claimed `config.gateway.whatsapp_phone_number_id` exists — but **nothing could ever set it**: the live `GatewayConfig`/`GatewaySettings` had only `whatsapp_token`, the adapter factory never called `with_phone_number_id`, and the wizard skipped it. Every WhatsApp send failed with "phone_number_id not configured" (or 404, per the adapter's own "Bug #10" note). Hermes's `whatsapp_cloud.py` reads `phone_number_id` from config — clear parity divergence.
- **Fix**: added `whatsapp_phone_number_id` to `GatewaySettings` (TOML) + `GatewayConfig` (runtime) + `OPERANT_WHATSAPP_PHONE_NUMBER_ID` env override; wired `with_phone_number_id` in the adapter factory and `gateway_config_from_app`; the `operant gateway setup` wizard now prompts for it; the adapter error message now names the real config path; `config_json()` exposes `phone_number_id_configured` so `operant gateway status`/doctor surfaces the misconfig instead of 404ing at send time. +1 factory-level regression test (asserts the field reaches the adapter both set and unset).
- **Note**: WhatsApp webhook verify-token handshake currently reuses the shared `webhooks_secret` (the Meta GET handshake compares `hub.verify_token` to it) — workable if the Meta dashboard token matches, but a per-WhatsApp `verify_token` (present in the dead schema below) would be cleaner; left as-is to keep scope.

### Flagged (not fixed) — the 14,094-line channels orchestrator is dead-linked
`operant-channels::orchestrator` — `start_channels`, `process_channel_message` (~1,200 lines), the dispatch loop, 30+ platform modules (matrix, signal, wechat, irc, nostr, lark, …) — has **zero callers** across the workspace (only `AcpServer` is used, by the gateway). The live channel system is the gateway's 7 adapters (telegram/discord/slack/whatsapp/email/sms/webhook) driven by `gateway_runner.rs`. The orchestrator core is compiled unconditionally (`pub mod orchestrator;`), so it ships in the binary as dead weight; its complete-but-unlinked `channels.whatsapp` schema (phone_number_id/verify_token/app_secret/session_path) wires to nothing. This is the largest parallel-implementation instance (R3/R15/R20 class). **Recommendation**: gate the orchestrator behind its `channel-*` features and audit whether any feature is enabled by default; if not, stop shipping it (or retire it in favor of the gateway adapters).

## Round 23 (2026-08-08)

### R23 — Gateway path returned empty answers: runtime agent lacked the empty-response retry (FIXED)
The gateway runs on `operant_runtime::agent::Agent` (`process_message` → `run_tool_call_loop`, ~21k lines — a second live agent implementation parallel to operant-core's `OperantAgent`; resolves the R3 "dead RuntimeAgent" item: the runtime `Agent` *is* the gateway agent, while the CLI `run` path uses `OperantAgent`). Its final-response branch was `if tool_calls.is_empty() { … return Ok(text) }` with **no empty-response check** — when the model returned no text, no reasoning, and no tool calls (common on the rate-limited free tier), the gateway sent the user an empty answer immediately. `OperantAgent` has the R4 retry ladder (up to `max_retries`, appends an empty assistant nudge, refunds the iteration) — live-verified on this exact model.
- **Fix**: mirrored R4 in `run_tool_call_loop` — `EMPTY_RESPONSE_MAX_RETRIES = 3`; on an empty final response the loop logs `Empty assistant response — retrying`, pushes an empty assistant turn (nudge), and `continue`s. Requires threading `response_reasoning` through the match tuple so reasoning-only responses (DeepSeek thinking mode) are not retried. +1 test: `StreamingScriptedProvider` serving `["", "", "finally"]` → loop returns `"finally"` with exactly 3 LLM calls. **1704 runtime tests** (+1) + **1232 core** + **635 CLI**, clippy + fmt clean.
- **Followup (fixed)**: retries now **refund their iteration slot** (`real_iterations` accounting) — the caller's `max_iterations` budget is reserved for real work, with the loop bound holding `+EMPTY_RESPONSE_MAX_RETRIES` headroom so the ladder runs even on tiny budgets (`--max-iterations 2`). This also surfaced a latent budget-leak hazard: without the cap, the headroom extended the *real* iteration budget for every caller (a delegate subagent capped at 2 ran 5 passes and tripped loop detection instead of the exhaustion path). Guarded by `execute_agentic_respects_max_iterations` (unchanged, passes) + the R23 retry tests. **1705 runtime tests**, clippy + fmt clean.

### R23 audit (no finding) — web/search/HTTP stack
`web_search` provider selection (explicit config wins, IGS auto-fallback), all five providers (tavily/exa/searxng/ddg/igs), the DDG lite byte-safe parser (graceful malformed-segment recovery + heuristic fallback), `web_fetch`, `http_request`, `xai_http_request` — all already hardened (SSRF fail-closed, redirects disabled to block the SSRF redirect bypass, status checks mirroring hermes `raise_for_status`, method whitelists, timeouts). Webhook POST dispatch likewise (Slack replay-protected HMAC + GitHub/Stripe HMAC + constant-time compare + handshake handling, all prior-audit hardened).

## Round 24 (2026-08-08) — Gateway-path self-evolution missing (FIXED)

### R24 — runtime Agent (`turn_streamed`) had zero post-turn reflection (FIXED)
The gateway and ACP paths run on `operant_runtime::agent::Agent`, but via **`turn_streamed` (agent.rs) — a third live loop distinct from the R23-fixed `run_tool_call_loop`** — and it carried none of OperantAgent's R1 self-evolution: no per-turn memory counter, no memory-review trigger, no skill nudge, no evolution observability. Both references have it on the streaming path: hermes `turn_context.py` advances memory triggers per turn, and hermes-agent-ultra `methods_run_stream.rs` does `c.turns_since_memory += 1; if >= memory_nudge_interval { reset; fire }` per turn plus `"memory" => c.turns_since_memory = 0` when the agent uses memory itself. Gateway sessions therefore never did post-iteration reflection / memory-updating / skill-nudging — the exact feature set this audit directive targets.
- **Fix** (faithful port to both `turn` and `turn_streamed`): new `AgentConfig.memory_nudge_interval` + `creation_nudge_interval` (both default `10`; `0` disables — same names/values as core `BehaviorSettings`); `Agent` counters `turns_since_memory`/`turns_since_skill`; `advance_memory_trigger`/`advance_skill_trigger` over a shared `advance_turn_trigger`; `fire_evolution_triggers` runs at both success boundaries — the memory trigger runs a lightweight LLM memory review (curator prompt over the recent-conversation digest, one non-streaming call, up to 8 facts stored as `memory_review_*` `Core` entries — the gateway analog of core `background_review`/ultra `spawn_background_review`), the skill trigger emits `ObserverEvent::EvolutionNudge { kind: "skill" }`; memory-tool use (`memory_*` names) resets the memory counter (ultra parity). Review failures are swallowed (warn) so a failed review never fails the turn. New `EvolutionNudge` observer event (operant-api) is broadcast to SSE as `{"type":"evolution_nudge",...}` and logged by `LogObserver`.
- **Tests**: +5 runtime (`advance_turn_trigger` fires/resets + 0-disables, `note_memory_tool_use` reset, `turn_fires_memory_review_and_stores_facts_when_interval_elapsed` with scripted provider + recording memory asserting 2 `Core` facts and event `facts_stored=Some(2)`, `skill_trigger_emits_nudge_event_at_interval`); +2 config (TOML `[agent] memory_nudge_interval=3 / creation_nudge_interval=7` parse + absent-key defaults 10). **1711 runtime + 640 config + 204 gateway + 34 api tests**, clippy + fmt clean; live smoke `R24_FINAL` on the deployed binary.
- **R25 follow-up (FIXED)**: the R24 audit proved `turn_streamed` (agent.rs) is a *third* live loop the gateway (`ws.rs:721`) and ACP (`acp_server.rs:661`) actually use — R23's empty-response retry ladder had landed in `run_tool_call_loop` (live via `process_message` at gateway `lib.rs:1045`) but **not** in `turn`/`turn_streamed`, so the WS/ACP paths still returned empty answers on empty model turns. Ported the ladder (same `EMPTY_RESPONSE_MAX_RETRIES = 3`, empty assistant nudge, `real_iterations` refund accounting so retries never eat the `max_tool_iterations` real-work budget) to both `turn()` and `turn_streamed()`. Also fixed the latent feature-gated compile break the new `EvolutionNudge` variant caused in `observability-otel`/`observability-prometheus` (both no-op alternation arms now cover it — verified with `cargo check --features observability-otel,observability-prometheus`), added `LlmRequest`/`LlmResponse` observer events around the review call, and upgraded the two legacy `tests.rs` empty-response tests to the cap semantics (4 empties → empty returned after 3 retries, exact call count asserted via `Arc<dyn Provider>`). **1713 runtime + 640 config + 204 gateway + 34 api tests**, clippy + fmt clean, live smoke `R25_FINAL`.

## Round 26 (2026-08-09) — Sub-agent delegation tool-filtering was dead code (FIXED)

### R26 — children got a fixed toolset, never recursive delegation, ignored parent bans (FIXED)
Deep-scan of the previously unaudited `sub_agent_tool.rs` (delegation) against hermes `tools/delegate_tool.py`. hermes delegates toolsets explicitly: children are built with **the parent's toolsets minus `DELEGATE_BLOCKED_TOOLS`** (`delegate_task`, `clarify`, `memory`, `send_message`, `cronjob`, `kanban`), and **orchestrator role re-grants `delegate_task`** (`_blocked_toolsets_for_role` discards it from the blocklist when `role == "orchestrator"`) — with a `DEFAULT_TOOLSETS` fallback when the parent has no toolset list. The Rust port violated every one of these:
- **`register_child_tools` ignored `_toolsets` entirely** — it hard-coded the same 14-tool registry for every child, so `compute_child_toolsets` (the strip-blocked machinery) was dead code and hermes' `DELEGATE_BLOCKED_TOOLS` semantics never applied.
- **`delegate_task` was never registered for children** — yet the orchestrator system prompt tells the child it "CAN spawn your own subagents", and the CLI's live `DelegationConfig` default is `max_spawn_depth: Some(2)` (intended grandchild support). Orchestrator role was a false promise; recursive delegation was impossible on the core CLI path.
- **Parent tool bans did not propagate** — the CLI applied `disabled_tools`/`disabled_toolsets` to the parent registry *after* registration and passed `vec![]` as parent toolsets; children were built with a fresh registry that inherited **none** of the parent's restrictions.
- **Fix**: `SubAgentTool` now carries the parent's `disabled_tools`/`disabled_toolsets` (new `with_parent_tool_policy` constructor; `register_builtin_tools_with_sub_agent` + CLI `build_registry` pass the real config sets); `compute_child_toolsets` is now live — strips parent-disabled toolsets + hermes child-blocked toolsets, falls back to `"builtin"` when the parent passes none (hermes `DEFAULT_TOOLSETS`), and re-adds `"delegation"` **only** for orchestrator role; `register_child_tools` filters each tool through `register_if_allowed` (parent disabled tool/toolset + child toolset membership) and registers a depth+1 `SubAgentTool` **only when the child is an orchestrator**, so leaf children can never recursively delegate (hermes parity) and orchestrator children finally can.
- **Tests**: +6 core (`compute_child_toolsets` builtin fallback / orchestrator retains delegation / parent-disabled toolset stripped / supplied-list-fully-stripped-yields-empty; end-to-end `child_registry_grants_delegate_task_only_to_orchestrators` — leaf registry has core tools but no `delegate_task`, orchestrator registry has it; `child_registry_honors_parent_disabled_tools` — parent-disabled `terminal` never leaks into the child registry). **1238 core + 1713 runtime + 204 gateway tests**, clippy + fmt clean, release rebuilt + deployed, live smoke `R26_OK`/`R26_FINAL`.
- **R26 review-followup (FIXED)**: reviewer-caught — the builtin fallback could not distinguish "parent supplied no toolset list" (fallback correct) from "parent supplied a list that was fully stripped" (must yield an empty toolset, never a silent re-addition of tools the parent withheld). `compute_child_toolsets` now tracks `parent_supplied` and only applies the builtin fallback when the parent genuinely passed nothing AND did not disable the builtin toolset; a fully-stripped explicit list yields an empty toolset. Locked in with `compute_child_toolsets_supplied_list_fully_stripped_yields_empty` (leaf → empty, orchestrator → `[delegation]` only).

### R27 — `todo` tool lacked read mode / caps / dedupe / merge / post-compression re-injection (hermes `todo_tool.py` parity) (FIXED)
- **`todo` errored on read** — hermes' single `todo` tool reads when `todos` is omitted and writes when provided; the Rust port made `todos` a required field, so a model that wanted to check its own list instead got `Invalid arguments` and could only ever overwrite.
- **No caps on persisted state** — hermes bounds todo state (`MAX_TODO_CONTENT_CHARS = 4000`, `MAX_TODO_ITEMS = 256`) explicitly because "the gateway/API server replays caller-supplied conversation history to rebuild the store, so an oversized forged result is dropped before it is parsed and re-injected"; the Rust port stored unbounded item content/count in the global store.
- **No id-dedupe, no merge mode** — hermes collapses duplicate ids (last occurrence kept in place) and supports `merge: true` (update by id, append new); the Rust port replaced the whole list unconditionally.
- **No post-compression re-injection** — hermes folds the active todo list back into the compressed history (`conversation_compression.py`: `agent._todo_store.format_for_injection()`, header `[Your active task list was preserved across context compression]`) so the model keeps its plan and does not re-do finished work; the Rust agent compressed and dropped the list entirely.
- **Fix**: `todo_tool.rs` now matches hermes semantics — `todos: Option<Vec<TodoItem>>` (omit to read, `[]` to clear), `merge` arg, `_dedupe_by_id`/`_validate`/`_cap_content` ports (invalid status → `pending`, empty content → `(no description)`, truncation marker `… [truncated]`), `MAX_TODO_ITEMS` enforcement; new `todo_injection_for_session` (pending/in_progress only, status markers) + `is_todo_injection_row`; `agent/mod.rs` `compress_context_overflow` now strips any prior snapshot row and appends a fresh one as a trailing user message (both streaming + non-streaming overflow paths — 2 call sites).
- **Tests**: +12 core (`todo` read-mode returns stored list / empty-session read / merge updates-by-id-and-appends / dedupe keeps last occurrence in place / content capped with marker / item list capped at 256 / invalid status normalized to `pending` / injection format header+markers+active-only / injection None when nothing active / cap passthrough / UTF-8 no-split). **1250 core + 1713 runtime + 204 gateway tests**, changed files clippy + fmt clean (agent/mod.rs has pre-existing `collapsible_if` lints at untouched lines — not introduced here), release rebuilt + deployed, live smoke `R27_OK`.

### R27-b — `process` registry: no process cap, kill orphaned child trees (hermes `process_registry.py` parity) (FIXED)
- **`kill` sent TERM to the shell pid only** — hermes kills the whole child tree recursively (`psutil.Process.children(recursive=True)`, children before parent); the Rust port's `kill -TERM <pid>` orphaned `sh -c 'server &'` descendants, which then held the stdout pipe open forever (the subshell-wait trap hermes guards with `_rewrite_compound_background`).
- **No `MAX_PROCESSES` cap** — hermes caps tracked processes at 64 with LRU pruning of finished sessions; the Rust `finished` map grew without bound over a long-lived agent.
- **Fix**: `process_registry.rs` spawns `sh -c` in its own process group (`tokio::process::Command::process_group(0)`, unix) and `kill` now targets `-<pid>` (whole group) with a single-pid fallback; new `MAX_PROCESSES = 64` + `prune_finished` evicts oldest finished sessions from the finish watcher.
- **Tests**: +2 core (`prune_finished` keeps newest 64 / no-op under cap).
- **R27 review-followup (FIXED)**: reviewer-caught — (1) appending a fresh `Message::user(snapshot)` after compression could create a synthetic user/user pair when the compressed tail already ends with a user message; `reinject_todos_after_compression` now folds the snapshot into the trailing user message's content when the tail role is `User` (hermes `conversation_compression.py` merge behavior) and only appends otherwise. (2) session-key divergence: the tool writes under the model-provided `sessionId` (default `"default"`) while re-injection read only the agent's `persistent_session_id` (None on the CLI path, set on gateway paths) — re-injection now checks both keys, preferring whichever holds active todos. (3) `kill` on an already-exited session short-circuited before reaping the still-alive `sh -c 'cmd &'` descendant group — best-effort `kill -TERM -<pid>` group reap on the exited path. (4) global `TODO_STORE` lock scoped to just the store access (summaries built outside). Re-validated: 1250 core + 1713 CLI + 1713 runtime + 204 gateway tests, fmt clean, changed regions clippy clean (remaining agent/mod.rs hits are the same pre-existing `collapsible_if` lints), redeployed, live smoke `R27_FINAL`.

### R27 audit (no finding) — kanban / session_insights
- **kanban** (SQLite `KanbanDb`, actions list/show/complete/block/heartbeat/comment/create/link) is a leaner design than hermes `kanban_tools.py` — hermes adds worker-ownership enforcement (`_enforce_worker_task_ownership`), orchestrator-mode gating, env-heartbeat injection, and URL-attach; those are dispatcher-integration features this port's kanban does not claim, and its core actions are wired correctly against the DB.
- **session_insights** mirrors `agent/insights.py` (days/source filters → `InsightsEngine.generate` + gateway format).

### R28 — `patch` tool bypassed the path-safety gate + file writes were non-atomic (hermes `file_tools.py` / `file_operations.py` parity) (FIXED)
- **`patch` skipped `validate_path` entirely** — `file_read`/`file_write`/`file_search`/`file_list` all canonicalize + deny sensitive paths (SSH keys, `.aws/credentials`, `/etc/shadow`, `.netrc`, …) and reject `..` traversal (iter-125 gate), but `patch` used `PathBuf::from(&args.path)` raw, so the only file tool that *writes* through a find-and-replace was also the only one that could write into a sensitive file or via traversal — `file_write` refused what `patch` happily clobbered.
- **Non-atomic writes** — both `file_write` (non-append) and `patch` used `std::fs::write`, which truncates in place: a crash or kill mid-write corrupts the target and loses the original. hermes `file_operations._atomic_write` writes a temp in the SAME directory and renames over the target (same-filesystem atomic), preserves the existing file's mode (`chmod --reference`), and removes the temp on any failure so a partial `.hermes-tmp` file never lands next to user data. hermes-agent-ultra carries a dedicated `test_atomic_replace_symlinks.py` for exactly this surface.
- **Fix**: `file_tools.rs` gains `atomic_write` (tempfile `NamedTempFile` in the target's dir + `persist()` rename, unix mode preserved via `PermissionsExt`, `sync_all` before rename, temp removed on failure) — `file_write` non-append path routed through it; `patch_tool.rs` now calls the shared `validate_path` (made `pub(crate)`) and writes back atomically. Appends stay as sequential `OpenOptions::append` (no atomic-replace possible). Symlink targets were already safe on the write path since `validate_path` canonicalizes first.
- **Tests**: +3 core (`patch` refuses `.netrc` in a scratch dir, file untouched; `atomic_write` preserves a 0755 executable's mode across replace; `atomic_write` replaces content with no leftover temp files). **1252 core + 1713 CLI + 1713 runtime + 204 gateway tests**, fmt clean, changed regions clippy clean (file_tools has pre-existing `collapsible_if` lints at untouched lines), release rebuilt + deployed, live smoke `R28_OK` with the written file verified on disk.
- **R28 review-followup (FIXED)**: reviewer-caught — `NamedTempFile` creates at hardcoded **0600**, so a *brand-new* target written via `atomic_write` landed at 0600 instead of the previous `std::fs::write` behavior (0666 & ~umask → typically 0644); hermes fixed exactly this with `chmod "=rw"` (#70856). Since `set_permissions` bypasses the umask (fchmod sets the exact mode), `atomic_write` now computes `0o666 & !current_umask()` for new targets, reading the umask race-free from `/proc/self/status` (`Umask:` line) with a 0o022 fallback. +1 test (new file lands at `0666 & !umask`, never 0600). **1253 core + 1713 CLI + 1713 runtime + 204 gateway tests**, fmt clean, changed regions clippy clean, redeployed, live smoke `R28_FINAL` with the written file verified on disk.

### R29 — `mcp_management` tool accepted arbitrary server URLs with no validation (hermes config-declared-servers parity) (FIXED)
- **`add_server` took `server_url` straight from the model** — no scheme or host validation. The agent could register an MCP server at any address (file://, malformed strings, arbitrary hosts) and attach an inline `auth_token` from context. hermes exposes only config-declared `mcp_servers` to the model (server URLs are user-authored in `config.yaml`); the Rust port's `McpManagementTool` is registered whenever an `McpManager` exists (live config `[mcp] autoload = true` → registered), and its `add_server` had zero guards.
- **Silent clobber** — re-adding an already-connected server name overwrote the existing entry without error.
- **Fix**: `mcp_tool.rs` now validates `server_url` via `validate_server_url` (must parse as a URL with an `http`/`https` scheme and a non-empty host; loopback hosts stay allowed — local MCP dev servers are legitimate) and rejects re-adding an already-connected name (`contains` guard). Note the capability itself is no new privilege class — the model already has `terminal` + `http_tool` for arbitrary network/header work — but scheme/host validation is cheap and matches hermes' config-only URL sourcing. OAuth token persistence was already correct (0600 file / 0700 dir in `mcp_oauth.rs`).
- **Tests**: +5 core (`validate_server_url` accepts http/https; rejects file:// / ftp:// / schemeless `localhost:8080`; rejects empty-host and garbage; `add_server` with a bad URL fails before any network connect; valid-format URL to a dead port reaches connect and reports `Failed to add server`). **1258 core + 1713 CLI + 1713 runtime + 204 gateway tests**, fmt + clippy clean, release rebuilt + deployed, live smoke `R29_OK`.
- **R29 review-followup (FIXED)**: reviewer-caught — the tool-level `contains` pre-check had a TOCTOU window (check and insert are separate awaits; concurrent callers could both pass and the second insert silently clobbers). `McpManager::add_server` now rejects an already-connected name itself — a fail-fast read check before the connect AND a second check under the write lock after the connect (the atomic chokepoint), returning `already connected` either way. Safe for all CLI callers (they use fresh names or pre-check `contains`). Re-validated: 1258 core green, fmt + clippy clean, rebuilt + redeployed, live smoke `R29_FINAL`.

### R29 audit (no finding) — browser tool stack
- **`browser` navigate/snapshot** already carry the SSRF verdict (`ssrf_verdict`, fail-closed vs cloud metadata / loopback / internal — hermes `url_safety.is_safe_url` parity, tested); **`browser_cdp`** is env-gated (`BROWSER_CDP_URL`, user-set — the model cannot supply a URL); **`browser_downloader`** fetches only the hardcoded GitHub release URL with post-write `verify_binary` (a partial download fails verification). No unguarded model-controlled URL-fetching surface found.

### R28 audit (no finding) — file_read / file_search / file_list / file_state
- **file_read** partial reads (offset/limit) + `validate_path` gate are correct; **file_search** escapes the pattern (literal match), skips hidden/node_modules/target dirs, caps at 100 results; **file_list** honors recursive/hidden flags; **file_state** check/watch/diff snapshots are consistent. hermes' per-task staleness checks, read-timestamp tracking, cross-profile guard, and per-path write locks are gateway-integration features this port's design doesn't claim — the core file surface is now at parity on the security (gate) + durability (atomic) axes.

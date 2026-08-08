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

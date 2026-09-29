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

### R14-4 — Slack `_signing_secret` is a dead field (WITHDRAWN as misread, fixed iter-347)

**Original finding**: `SlackAdapter` held a `_signing_secret` the CLI always
wired as `None`, and "Slack's own webhook signature verification (implemented
in the same file) can therefore never be reached."

**Correction**: the verification is in `gateway/webhook.rs` (iter-125), not
the Slack adapter, and it IS reachable — Slack-over-HTTP ingress is served by
`WebhookAdapter`, which verifies `x-slack-signature` (HMAC-SHA256 of
`v0:<ts>:<body>`) against `gateway.webhooks_secret`. `SlackAdapter` is
Socket Mode only: authenticated by the bot token over the WebSocket, no
signed requests exist on that transport, so a signing secret there had no
consumer — wiring one would have laundered a dead credential, not closed a
security gap.

**Fix**: dead `_signing_secret` field + constructor parameter removed
(`SlackAdapter::new(token)`; call sites gateway_runner.rs + mod.rs test
updated). No schema field needed: the credential for Slack webhook ingress
is the existing `webhooks_secret`.

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

### R30 — `code_execution` hard-timeout was not a hard timeout (hermes `code_execution_tool.py` parity) (FIXED)
- **Wall-clock unbounded for chatty children** — the old executor wrapped only each individual line-read in `tokio::time::timeout` and applied a separate timeout to the final `wait()`. A child that emitted a line more often than the timeout per line (e.g. `while True: print('x'); sleep(0.01)` with a 60s default) **never timed out** — the loop condition stayed true forever, accumulating unbounded output. The intended "hard timeout" was only soft.
- **Sequential stdout-then-stderr reads → pipe deadlock** — stdout was drained to EOF before stderr was read; a child that filled the stderr pipe while stdout was still streaming blocked forever (and the timeout then only fired per line-read).
- **No output caps** — hermes bounds capture (`MAX_STDOUT_BYTES = 50_000`, `MAX_STDERR_BYTES = 10_000`, head-40%/tail-60% truncation with a marker). The Rust port accumulated unbounded memory.
- **Orphaned grandchildren on timeout** — `kill_on_drop(true)` kills only the direct child. `sh -c 'sleep 30 & wait'` or a python multiprocessing child survived the tool call as an orphan — the exact class of bug fixed for the terminal tool in R27-b, missing here. hermes spawns `start_new_session=True` and kills the whole process group (TERM → 5s grace → KILL).
- **Temp-file hygiene** — scripts were written with `std::fs::write` to predictable `/tmp/operant_code_<uuid>.*` paths (0644, follows symlinks) and the rust project dir leaked on spawn-failure / timeout / compile-failure paths (the compile-fail `return` skipped `remove_dir_all`). The rust path also wrote a dead `Cargo.toml` while compiling with bare `rustc` (default edition 2015).
- **Fix**: all four executors now route through one `run_capped` helper — a **single hard deadline** (`tokio::select!` on `child.wait()` vs the timeout — bounds total wall time, not per-line), **concurrent** capped stdout/stderr reads (no pipe deadlock; pipes drained to EOF so a capped child never blocks), **`process_group(0)` at spawn + whole-group TERM→250ms→KILL** on timeout (negative-pid `kill` binary, single-pid fallback — R27-b pattern) with a final reap, **`NamedTempFile`** (0600 + O_EXCL, auto-removed on every path) for python/js scripts and `tempdir()` for rust projects, head/tail truncation marker, `rustc --edition 2021`, and result JSON gains `timed_out`/`stdout_truncated`/`stderr_truncated`.
- **Tests**: +9 core (happy-path python; 200 KB output capped at ≤50 KB with marker; runaway infinite-loop python times out at the 1 s budget — the old code would hang the suite; `sleep 30 & wait` group-kill regression — a `sleep 30` left running would hang the suite via the open pipe; rust compile-error stage; read_capped head/tail unit tests incl. exact-partition boundary; under-cap untouched). Verified live: no `sleep 30` orphan survives a timed-out run; no `/tmp/operant_code_*`/`operant_rust_*` residue. **1269 core + 1713 CLI + 1713 runtime + 204 gateway tests**, fmt + clippy clean (MSRV-aware — `floor_char_boundary` hand-rolled for 1.88), release rebuilt + deployed, live smoke `R30_OK` (agent ran `code_execution` end-to-end). Full sandbox parity (AF_UNIX transport, env hardening, docker/modal/vercel backends) remains future work — see R12-1.

### R30 — `checkpoint restore` skipped the file-path gate (hermes `_validate_file_path` parity) (FIXED)
- **`restore` passed `file_path` raw to `git checkout <hash> -- <target>`** — no absolute-path or `..`-traversal check. git itself refuses pathspecs outside the worktree, but the failure was a cryptic git error instead of a clear guard, and the tool is the user-facing rollback surface. hermes `checkpoint_manager.py:_validate_file_path` rejects absolute paths and traversal before any git invocation.
- **Fix**: `validate_restore_path` (lexical `resolve_non_strict`, hermes non-strict `Path.resolve()` parity — works for restoring deleted files) rejects empty/absolute/escaping targets before the store check; the tool surfaces the reason.
- **`list_checkpoints` hardcoded `-n 20` and ignored the tool's `limit` argument** — now honors `limit` (clamped 1..100), defaulting to the configured `max_snapshots`.
- **No `_MAX_FILES` guard** — hermes refuses to snapshot directories over 50 000 files (`_dir_file_count` early stop) so `git add -A` can't hang on pathological dirs; the Rust port now has the same guard.
- **Tests**: +5 core (absolute/traversal rejection at both manager and tool level, in-dir normalization accepted, early-stop file counter, list honors limit with a real shadow store).

- **R30 review-followup (FIXED)**: reviewer-caught — (1) `dir_file_count` used `path.is_dir()` which FOLLOWS symlinks: a workdir containing a symlink cycle (`loop -> .`) made the pre-git walk loop forever and hang `take_checkpoint`, and it descended into `.git`/`node_modules`/`target`/`__pycache__`/`.venv`/`venv` (the exact dirs the shadow store's `info/exclude` tells git to skip) — a repo with a >50k-object `.git` would be falsely refused a checkpoint. The walker now uses `entry.file_type()` (no symlink follow — hermes `os.walk(followlinks=False)` parity) and mirrors the exclude list. (2) the post-kill reader join was unbounded — a child wedged in D-state (dead NFS mount) holds the pipes open forever, so the "hard" timeout wasn't fully hard; the join is now bounded (2s) and the reap bounded (1s). (3) `validate_restore_path` was purely lexical — it missed a workdir-internal symlink pointing outside (`link -> /etc`); the longest existing prefix is now canonicalized and containment re-checked (hermes `Path.resolve()` parity; lexical tail covers restoring deleted files). (4) `kill_process_group` sent the single-pid fallback unconditionally, opening a PID-recycle window in the grace period; the fallback now fires only when the group signal fails (ESRCH). Re-validated: 1269 core + 1713 CLI + 1713 runtime + 204 gateway tests, fmt + clippy clean, rebuilt + redeployed, live smoke `R30_FINAL`.

### R30 audit (no finding) — shadow-store internals
- `take_checkpoint`/`restore`/`diff` already run via `GIT_DIR`+`GIT_WORK_TREE`+`GIT_INDEX_FILE` with `GIT_CONFIG_GLOBAL`/`SYSTEM` nulled (no user-config, no repo leakage — R12-2), internal git identity, `update-ref`-based prune + `gc` in-store, and the tool's `working_dir` is model-supplied only for reads (store key = sha256 of the dir). The remaining hermes surface (`_volume_evidence`, migration, cross-worktree sharing, per-project index files) is a storage-efficiency design this port's per-directory bare stores deliberately don't claim.

### R31 — Telegram adapter death on transient startup failure was permanent (FIXED)
- **Symptom**: user messaged the Telegram bot, zero response; service "active (running)" for 9h. Log: `ERROR Failed to start adapter with channel, platform: telegram — Network error … /getMe` at boot (network not yet up when `network-online.target` released), then silence.
- **Root cause**: `Gateway::start`/`Gateway::start_with_channel` logged adapter start failures and moved on. systemd's `Restart=always` never fires because the process stays alive — only one platform inside it is dead. The operant-channels telegram heartbeat/poll-recovery watchdogs spawn only after a successful start, so they could never rescue their own failed startup. No code path ever retried.
- **Fix**: added `Gateway::supervise_adapter_start` (`operant-core/src/gateway/mod.rs`) — a failed adapter start now spawns a background supervisor retrying every 5s→300s capped exponential backoff until the adapter starts or the gateway stops (`running` flag checked each cycle). Safe against double-start: telegram's getMe gate fails before any listen task spawns.
- **Deployment audit** (fresh-install path): (1) `scripts/operant-gateway.service` deleted — orphan template with a hardcoded machine-specific ExecStart, zero references; unit generation lives solely in `cmd_gateway::cmd_install`. (2) `cmd_install`'s generated unit hardened to match the deployed one (`Restart=always`, `StartLimitIntervalSec=0`, `TimeoutStopSec=90`, `KillMode=mixed`, WorkingDirectory for user scope only); previously it emitted weaker `Restart=on-failure` + `RestartSteps/MaxDelaySec` (not valid for `on-failure`) and no rate-limit reset. (3) `scripts/install.sh` now installs+enables the gateway service via the binary itself (`gateway install --force` + daemon-reload + enable) and also copies the fresh binary into `~/.cargo/bin` when present — PATH resolves there first and a stale copy shadows `/usr/local/bin`.
- **Live-verified**: service restarted on the new release build — both adapter paths report `Telegram bot started successfully`, getMe verified, polling active consuming updates (offset 53474081→83), commands registered. Supervisor string confirmed in deployed binary. Full workspace check+test+clippy clean.

### R31 follow-up — remaining silent-death holes in the Telegram adapter (FIXED)
Audit of "self-healing, always running" found three gaps beyond the R31 start-retry:
- **Unbounded HTTP clients** — both `TelegramAdapter::new`/`with_config` built clients with no timeout; a wedged TCP connection (WiFi flap / NAT expiry without RST) hung `getUpdates` forever inside one await: heartbeats stop, bot dead, service still "active". Now `connect_timeout(10s) + timeout(60s)` on both constructors (30s long-poll fits), covering outbound `sendMessage`/media too.
- **Panicked poll task = silent death** — the spawned polling task had no supervision; any panic killed polling permanently while the process lived. The spawn is now wrapped in a panic-supervisor loop that respawns epochs after 5s (clean shutdown still exits; 409 recovery stays inside the epoch's own `'restart` loop).
- **One-shot `setMyCommands`** — fired once at boot with an unbounded client; failing then left the command menu unregistered until next restart. Now 5 attempts × 5s backoff on a timed client (`gateway_runner.rs`).
- **Live-verified**: service restarted on new release build — polling epoch entered, saved offset honored (53474086), commands registered. Workspace check/test/clippy clean (incl. previously-flaky config schema test).

### R32 — gateway streaming/agentic-loop failures surfaced by live tool-test session (FIXED)
Live Telegram tool-test (2026-08-22 06:09–06:31 IST, nvidia free tier) exposed four defects:
- **Silent stall after empty final response** — the model's last iteration returned empty 3/3 retries (provider 502-overload window); `fallback_models=[]` left the fallback-model branch dead, and the gateway's canned fallback text was then **suppressed** because the stream-delivery marker was still set from mid-turn blocks. User saw nothing for 11 minutes. Fix: `AgentEvent::Done` with an empty final block now removes the delivery marker so dispatch always delivers the fallback; the canned text is now honest ("provider returned an empty response… reply 'continue'") instead of falsely claiming completion.
- **Reasoning-only turns ended as empty answers** — a model that emits reasoning but no visible answer/tool call skipped the empty-retry ladder (`has_reasoning` guard) and ended the turn with `response_len=0`. Added hermes `_CODEX_INCOMPLETE_NUDGE` parity: persist the reasoning-only turn, push a "produce your final answer now" user nudge, refund the iteration, continue — bounded by the same retry budget.
- **Interactive tools killed by the generic 30s timeout** — `clarify` timed out at 30s while the user was reading the question (and `approval_request` was exposed to the same cap). Both now get 300s overrides next to the existing lcm/AFT overrides.
- **Log noise masked signal** — every send logged twice (`Message sent to chat` + `message_id` variant) and every LLM iteration logged `Streaming connection established`. First demoted to `debug`; connection-establishment demoted to `debug`.
- Known residual: free-tier SSE streams can drop mid-token; operant accepts the partial block as complete (the observed "tool cate|gories" mid-word message split). Full parity would need finish_reason-less truncation detection — deferred until it recurs on the hardened build.
- Hermes contrast (gateway/stream_consumer.py): single-bubble progressive edit + redundant-edit skip + final seal; operant's per-block editing matches chronologically but lacks `_last_sent_text` dedupe — candidate future polish.
- Live-verified: rebuilt + redeployed + service restarted; workspace tests/clippy clean.

### R33 — degenerate tool-call loop shredded chat output (FIXED)
Live Telegram session (2026-08-22 08:06 IST) exposed the worst-case agentic-loop pathology:
- **Root cause**: the free-tier model entered a repetition loop — **4,397 within-batch duplicate `process` calls skipped**, 1,145 malformed-argument calls repaired to `{}`, repeats up to 17× across iterations, 21+ minutes elapsed. The per-call guardrails (R4) skipped every duplicate but never terminated the turn, so each iteration still cost an LLM call and rendered another text fragment + status bubble.
- **Renderer amplification**: the segment-break design (each writing block = its own message; each tool group = its own message) faithfully shredded alternating text/tool output into dozens of fragments — the "output split by tool messages" symptom.
- **Fixes**:
  1. Turn-level circuit breaker (`agent/mod.rs`): ≥3 consecutive all-failure tool iterations → forced stop-repeating nudge; ≥6 → turn ends with a partial-results notice (hermes repetition-guard parity).
  2. Single-bubble-per-turn rendering (`gateway_runner.rs`): ONE content bubble progressively edited across the whole turn (segment breaks removed), sealed only at >3500 chars (Telegram 4096 edit limit); ONE tool-status bubble per turn edited in place, capped at last 15 lines with a `… +N earlier` header; redundant-edit skip (`_last_sent_text` hermes parity).
  3. IterationComplete no longer resets stream state.
- **Hermes contrast**: hermes `stream_consumer.py` is single-bubble by design with redundant-edit skipping and final seal — operant now matches that shape; hermes `repetition_guard.py` detects repeated-text domination in truncation continuations, operant now has the tool-loop equivalent (text-domination detection remains future work).
- Live-verified: rebuilt + redeployed + restarted; workspace check/test/clippy clean.

### R34 — single-bubble merge broke chronological interleaving (FIXED)
R33's turn-wide single content bubble over-corrected: the initial message was
edited forever and post-tool text (e.g. the delegation result) merged into it
instead of appearing as a fresh message BELOW the tool chrome. Live session
(2026-08-22 09:50 IST, "test all tools") also exposed:
- 90-iteration budget burn on mostly-repeated tool calls (todo×7, cron×5,
  kanban×5) — display showed them as stacked duplicate lines;
- final-answer loss cascade: HTML 400 → plain-text 400 → nothing delivered,
  with no error description logged.

**Hermes reference** (`stream_consumer.py` + `send_progress_messages`): each
contiguous text segment is its own bubble, finalized at tool boundaries
(`MessageStop`/`_NEW_SEGMENT`); when a content bubble lands, `__reset__`
closes the tool-progress bubble so the next tool opens a fresh one BELOW the
content; identical repeated tool lines collapse into `(×N)` (`__dedup__`);
oversized segments seal head chunks at the platform limit.

**Fixes** (hermes parity restored):
1. Segment breaks re-enabled: IterationComplete finalizes the current content
   bubble + resets stream state — post-tool text starts fresh below.
2. `__reset__` linearization: creating/sealing a content bubble closes the
   live tool group; next ToolStart opens a new status bubble below it.
3. ×N dedup: identical consecutive tool lines collapse into counters.
4. Line cap retained (… +N earlier header).
5. Final-send hardening: Telegram 400s now log the API error description;
   plain-text fallback failure retries once more truncated to fit 4096 so
   an oversized/undeliverable answer can never be silently dropped.

Degenerate-loop containment now rests on the R33 circuit breaker + dedup +
line cap instead of merging all output into one bubble.

### R35 — delegation interleaving, mid-word seals, identical-call storm (FIXED)
Second live "test all tools" session (2026-08-22 10:54–11:11 IST) surfaced three
distinct bugs:
1. **Child-event interleaving** — `sub_agent_tool` forwarded the child agent's
   full event stream (Content/ToolStart/IterationComplete/Done) onto the
   parent's channel. The gateway runner treated a CHILD's Done as turn-end
   mid-parent-turn (finalizing/closing bubbles at the wrong time), which is
   what broke chronological rendering around `delegate_task`. Fix: children
   now run HEADLESS (no event forwarding) — hermes parity where a delegation
   is one tool line + its result text; the parent narrates the result.
2. **Mid-word seal splits** ("Recent se" / "ssions (3 shown)") — the 3500-char
   overflow seal cut at an arbitrary byte. Fix: split at the last newline
   within budget (hermes `_safe_limit` rfind parity), char-boundary-safe hard
   cut as fallback (`seal_split_point`, unit-tested).
3. **Identical-call repetition storm** — 408 unrepairable `process` calls were
   repaired to `{}` and each "succeeded", so the R33 all-failure breaker never
   fired: 73 iterations, 480 tool turns, summary regenerated 4×. Fix: new
   cross-iteration guard — same (name + args) signature repeated across
   iterations nudges at 4 and force-ends the turn at 6 (hermes
   repetition-guard parity for the tool loop).

### Dedup pass — over-engineering audit executed (corrected findings)
The full-tree audit's #1 finding ("17.7K-line dead RuntimeAgent stack") did not
survive contact with the call graph: operant-channels' orchestrator (live via
the default `agent-runtime` feature) imports loop_::{run_tool_call_loop, run,
process_message, agent_turn}, and gateway/cron/daemon call them too. The old
"RuntimeAgent is dead-linked" note was stale — RuntimeAgent no longer exists.
Executed cuts (verified against callers before each deletion):
- Deleted orphan crates: operant-eval (492 lines) + robot-kit (473 lines);
  zero dependents, removed from workspace members/dependencies.
- Deleted provably-dead context_analyzer.rs (161 lines; only its own mod decl
  referenced it).
- Removed pin-project workspace dependency (zero usages).
- Extracted loop_.rs's stateless support layer into agent/loop_support.rs
  (~230 lines: tool filtering, task-local scoping, credential scrubbing,
  tool-instruction rendering), with compat re-exports so all existing
  `loop_::` import paths keep working. loop_.rs shrinks 8415 → ~8190 lines;
  further splits (streaming / run / process_message) remain possible but are
  live-code surgery with regression risk — deferred until a reason exists.
Kept deliberately: loop_detector.rs (LIVE circuit breaker inside
run_tool_call_loop), memory_loader/dispatcher traits (AgentBuilder API
surface), thinking/eval modules (heavy internal use).

### Full-scale module split — schema + agent loop monoliths (dedup pass 2)
Executed the rust-best-practices split assessment end-to-end:
1. **schema.rs (19,816 lines) → `config/src/schema/` — 25 domain modules**
   (`agent_cfg, backup, channels_cfg, core, cost, delegate, gateway, hardware,
   helpers, mcp, media, memory_store, plugins_cfg, providers_cfg, proxy,
   runners, scheduler, skills, sop, stt, tts, trace, vi, web_tools,
   workspace_cfg`). Script-driven verbatim extraction
   (`scripts/split_schema.py`); top-level decls widened to pub(crate) so
   cross-domain globs resolve; mod.rs glob-re-exports everything → every
   existing `schema::X` path unchanged. Generic serde `default_*` helpers
   centralized in `helpers.rs`.
2. **loop_.rs (8,196) → `agent/loop_/` directory** — `context.rs`,
   `streaming.rs`, `turn.rs`, `tool_loop.rs`, `run.rs`, `messages.rs`
   extracted verbatim with compat re-exports in loop_ mod; loop_support.rs
   unchanged from pass 1. Lint attributes stripped by the splitter's tail
   trim were detected by clippy and restored (`too_many_arguments` ×2,
   `too_many_lines`); StreamedChatOutcome visibility widened to match its
   now-pub consumer.
3. Verification: workspace check 0/0, clippy -D warnings clean, 36 test
   suites green, release binary rebuilt/deployed, gateway active.
**Deferred deliberately:** orchestrator/mod.rs (14.1K) — same mechanical
pattern applies but it is live code for every messaging platform; do it as a
focused follow-up arc using concern markers already identified (timeout
budgets, interruption handling, runtime-config store, command parsing,
prompt building).

### Full-scale module split — orchestrator/mod.rs (dedup pass 3)
Executed the deferred orchestrator surgery (`scripts/split_orchestrator.py`):
1. **orchestrator/mod.rs (14,106 lines) → 15 concern modules + tests file**
   - `consts.rs` (all tunables), `runtime_types.rs` (ChannelRuntimeContext,
     route/model-cache/runtime-defaults state), `history.rs` (sender history,
     compaction, rollback, overflow detection), `commands.rs` (/stop +
     runtime command parsing/handling), `routing.rs` (provider aliasing,
     defaults loading/reload, route selection, provider construction),
     `prompts.rs` (delivery instructions, system-prompt assembly, help/config
     responses), `memory_ctx.rs` (memory-context building/formatting),
     `sanitize.rs` (reply-intent classification, think-tag/tool-artifact
     stripping), `supervision.rs` (timeout budgets, supervised listeners,
     typing tasks, in-flight caps), `dispatch.rs` (process_channel_message,
     dispatch_worker, dispatch loop), `identity.rs` (telegram identity bind,
     daemon restart), `factory.rs` (build_channel_by_id, send_channel_message),
     `health.rs` (health classification, collect_configured_channels,
     doctor), `startup.rs` (start_channels). Inline `mod tests` (~7.5K lines)
     → `tests.rs`.
   - mod.rs reduced to ~340 lines (observer wiring, deliver_announcement,
     decls/re-exports).
2. Splitter hardening lessons (vs pass 2): brace-counting depth tracking is
   unreliable in string-heavy code (JSON literals poison depth) — replaced
   with indentation-based state machine for field/method/item widening;
   multi-line `use` statements must be captured whole in generated headers;
   classify item kind BEFORE the widening continue.
3. Verification: channels lib 0 warnings/errors; workspace all-targets 0;
   clippy --workspace --all-targets --all-features -D warnings clean;
   36/36 test suites green; fresh release deployed to both install paths;
   gateway restarted (new PID) with Telegram platform up; live `gateway
   status/channels/sessions` verified against the new binary including a
   pre-existing session row (no data regression).
Note: tests/live_parity.rs fails under bare `-p operant-channels --all-targets`
(default features) because it imports the cfg-gated telegram module without a
feature gate — pre-existing, masked by workspace feature unification.

### Full-scale module split — schema/core.rs residual + audit (dedup pass 4)
1. **core.rs (13,118 → 1,600 lines)**: misplaced blocks relocated to their
   proper homes — proxy I/O helpers (AsyncReadWrite/BoxedIo/
   resolve_ws_proxy_url/find_header_end) merged into `proxy.rs` where their
   only consumers live; 13 channel configs (Mattermost, Webhook, Linq,
   NextcloudTalk, Lark, Line, WeCom, WeChat, Twitter, Reddit, Bluesky,
   VoiceDuplex/Wake, Nostr) merged into `channels_cfg.rs`; new modules:
   `platform_cfg.rs` (memory/search/observability/hooks/runtime/reliability/
   routes/cron), `tunnels.rs`, `security_cfg.rs` (security/OTP/sandbox/audit),
   `ops_cfg.rs` (CloudOps/ConversationalAi/SecurityOps), `config_impl.rs`
   (impl Config ~1.9K lines + Default + resolution-source helpers). Inline
   test mod (7.4K) → `core/core_tests.rs`. mod.rs re-exports keep all
   schema:: paths stable.
2. Splitter lesson (scripts/split_core.py): when a target filename equals an
   existing module (proxy.rs), APPEND — do not open("w") (clobbered once,
   restored from git and re-merged). Non-mod.rs child modules resolve to a
   subdirectory named after the parent file (schema/core/core_tests.rs).
3. Audit findings recorded for follow-up arcs:
   - Remaining monoliths: telegram.rs 7.2K; core/agent/mod.rs 6.3K;
     core/gateway/mod.rs 5.8K; slack.rs 5.3K; providers compatible.rs 4.9K;
     loop_ mod.rs residual 4.9K; runtime agent.rs 4.7K; matrix.rs 4.6K.
   - Production `.unwrap()` census (~450 total, most in tests): cron/mod.rs
     51, memory/lucid.rs 32, skills/mod.rs 27, tool-call-parser 22,
     wechat.rs 15, service/mod.rs 8.
   - 8 untracked TODO comments (no issue links).
4. Gates: fmt, workspace check/clippy(-D warnings all-targets+features),
   36/36 suites, fresh release deployed both paths, gateway active.

### Tracked TODO index (rust-best-practices audit, pass 6)
Untracked in-code TODOs retagged `TODO(BUGS.md)`; tracked here per
chapter-1 guidance ("TODOs are not comments"):
- tui/app/commands.rs:1300 — MCP server list placeholder needs live
  core_mcp_manager wiring.
- config/pairing.rs:41 — parking_lot works today; evaluate flume/tokio
  async mutex migration.
- config/pairing.rs:196 — pairing function should become primary without
  task spawn.
- gateway/voice_duplex.rs:70 — wire into session abort mechanism (ref
  upstream PR #5705).
- plugins/wasm_channel.rs:30,41 — WASM plugin send/receive not wired.
- runtime/skills/mod.rs:34 — update registry URL when repo rebranded to
  operant-labs.

### Full-scale module split — loop_ residual, gateway adapters, telegram dir (passes 5–7)
1. **core/agent/mod.rs (6.3K → 2.1K)**: 4.1K-line `impl OperantAgent` split
   into five reopened impl blocks (builders/events/run/prompting/compress/stream).
2. **gateway/mod.rs (5.8K → 1.8K)**: per-platform adapter files (telegram,
   discord, slack, webhook, whatsapp, email, sms, admin) + types.rs.
3. **loop_.rs residual (4.9K → 239)**: inline tests extracted to loop_/tests.rs.
4. **channels/telegram.rs (7.2K) → telegram/ directory**: mod.rs (struct +
   Channel trait impl + Drop + poll watchdogs), helpers.rs (consts,
   attachments, poll-recovery state), channel_impl.rs (inherent methods),
   tests.rs. External path operant_channels::telegram::TelegramChannel stable.
5. **Splitter hardening**: col0 continuation lines inside multi-line string
   literals must NOT reset the widen state machine — whitelist known Rust
   item starters instead of else-resetting. Test-mod extraction: slice after
   the `mod tests {` line, not after the cfg attr.
Gates each pass: fmt, workspace check/clippy(-D warnings), full test suites
(known flakes: iteration_budget Barrier race test, network-dependent DDG
tests - pass in isolation), fresh release deployed both paths, gateway active.

### R36 — gateway booted every adapter TWICE: duplicate outbound delivery (FIXED)
Found via live Telegram loop-testing (2026-08-26): every bot reply arrived as TWO
identical DM messages. Log showed `Starting platform adapter` (from
`Gateway::start`) immediately followed by `Starting platform adapter with channel`
(from `Gateway::start_with_channel`) and two `Telegram bot started successfully`
lines — `gateway_runner.rs` called the legacy `gateway.start()` boot line AND then
`start_with_channel(message_tx)`; both iterate ALL enabled adapters, so telegram
was started twice (two senders; only one poll task survived).
- **Fix**: removed the redundant `gateway.start()` call from the runner (the
  channel path is the sole canonical boot), and made `start_with_channel` set
  `self.running = true` itself — previously only `start()` set the flag, so a
  channel-only boot left `running=false`, which would have made
  `supervise_adapter_start` abandon retries and made `stop()` a no-op.
- Tests: 64 core gateway tests pass; live-verified post-fix that each reply is
  delivered exactly once with a single `bot started successfully` boot line.

### R37 — fast turns still double-delivered: dispatch raced the event consumer's stream marker (FIXED)
Found via live Telegram loop-testing immediately after R36 (2026-08-26): a 2-char
turn still produced TWO identical DM messages. `route_message` returned at
T+0ms and the dispatch loop checked the shared `stream_delivery` map instantly,
but the spawned event-consumer task drains queued events (ToolStart/Usage/
Content…) BEFORE `Done`, so its marker insert landed ~500ms later — dispatch
found no marker and sent the full response itself while the consumer also
delivered it. Slow/streamed turns were unaffected (marker inserted mid-stream).
- **Fix** (`gateway_runner.rs` dispatch arm): replaced the instant marker check
  with a bounded settle-window poll (25ms × 40 = ≤1s) — check-first so already-
  marked turns add zero latency; unmarked (non-streaming) platforms pay the
  full window only before falling back to the legacy full send.
- Live-verified: two consecutive turns each delivered exactly ONE reply with
  `Stream already delivered response … skipping duplicate send` logged.

### R38 — live cron test: no temporal grounding + background review overreach (FIXED)
Found via live Telegram loop-testing (2026-08-26, "schedule a one-time job 2
minutes from now"):
1. **No wall-clock context** — the gateway turn prefix (`build_session_context`)
   told the model the platform and channel but never the current time. The
   free-tier model computed "+2 minutes" as a date a YEAR out and the tool
   dutifully created `next_run_at = 2027-08-26`. **Fix**: `Current time: <UTC>`
   line injected into every gateway turn's session context.
2. **Background review overreach** — at the skill-nudge interval boundary the
   spawned background-review agent saw the pending cron conversation and tried
   to execute the user's task itself (`Background review attempted
   non-whitelisted tool, tool: cron`). The whitelist correctly BLOCKED it, but
   the review then burned 7 API calls / 6 tool turns invisibly attempting the
   task instead of reviewing. The prompt already said "tools only"; it now also
   says NEVER to continue/execute the user's task (both frozen-prefix and
   standalone branches).
3. Working-as-designed confirmations from the same turn: degenerate tool-call
   circuit breaker fired at 3 consecutive failed iterations (R33), ×N dedup
   rendered `⏰ cron... (×3)`, whitelist denial returned a typed error to the
   reviewer.

### R39 — cron jobs never delivered their results: origin fields never recorded (FIXED)
Found via live Telegram loop-testing immediately after R38's time-grounding fix:
the t22b job fired on schedule (`Executing cron job`, `Delivering result` logged,
`last_status=ok`) but NOTHING reached the chat.
- **Root cause chain**: `CronTool::handle_create` hardcoded
  `origin_platform/origin_chat_id/origin_thread_id = None`; prompts only ever see
  PII-redacted channel ids (`chat_636bbfc5f7ee`), so even a model-supplied
  `deliver` string was unusable as a target; at fire time `deliver_result`
  fell into its `debug!`-level "No delivery target" branch and silently dropped
  the result while `last_status` still read ok.
- **Fix**: new `cron_tool::CRON_ORIGIN` process-global (the established
  ACTIVE_MEMORY_MANAGER hook pattern) — the gateway dispatch loop sets it to the
  real `(platform, chat_id, thread_id)` before routing every turn and cron job
  creation reads it into the origin fields; `deliver_result`'s silent drop
  demoted from `debug!` to `warn!` so a broken delivery is visible in INFO logs.

### R48 — plan 005 dead-code audit: real wins + plan-stale items (PARTIAL FIXED)

| Spec item | Verdict | Action |
|---|---|---|
| `crates/operant-channels/src/orchestrator/` (14 files) | LIVE: `acp_server` used by `operant-gateway/acp.rs:13` and `acp_channel.rs:29`; `strip_tool_call_tags` used by `telegram/helpers.rs:462`; `mqtt` doc-referenced; `dispatch` uses `agent::classifier` | **KEEP** — plan stale |
| `crates/robot-kit/`, `crates/operant-eval/` | already removed in earlier rounds | n/a |
| `crates/operant-plugins/src/wasm_channel.rs` | DEAD: zero callers cross-crate (`WasmChannel`/`wasm_channel::` only inside the file); placeholder `Channel` impl that errors "send/listen not yet connected" | **REMOVED** — file deleted + `mod wasm_channel` decl in `operant-plugins/src/lib.rs` removed; build green |
| `session_events` table + `record_event` | LIVE: `record_event` is called 9x in `agent/run.rs` (Observer API) | KEEP — plan stale (called `record_event zero runtime callers` but the runtime agent uses it actively) |
| `WhatsAppAdapter::with_phone_number_id` | LIVE: called at `gateway_runner.rs:635` | KEEP — plan stale (R22 wired it) |
| `whatsapp_web.rs` WIP `#[allow(dead_code)]` blocks (8 sites, lines 672-960) | LIVE channel (`orchestrator/factory.rs:172` mounts it) but the allowed functions are unused helpers; needs surgical per-block audit | **deferred** — mark with a `// plan 005: verify caller` comment and tackle in a follow-up |
| `qq.rs` WIP `#[allow(dead_code)]` blocks (6 sites) | LIVE channel (`orchestrator/factory.rs:196`); same deferred status as whatsapp_web | deferred |
| TUI `commands.rs:1152` "TODO: populate with live MCP server data" | live TUI surface; out of scope for this PR | deferred |
| `operant-runtime/src/agent/{history_pruner,classifier,eval}.rs` | all LIVE: history_pruner has 4 callers in `loop_/`, 1 in orchestrator/startup; classifier has 1 caller in orchestrator/dispatch; eval has 1 caller in `agent.rs:4` | KEEP — plan stale (R5-3 verdict was about the whole `RuntimeAgent` stack being unused, but the agent itself is mounted in ws.rs:343) |

Net win: `wasm_channel.rs` (-49 LOC) + the audit cleared 6 other items as live / already-removed. Binary size unchanged (plugin module was small).

Workspace gate: clippy -D warnings green, core tests 1735 passing, gateway tests 205 passing, plugin crate rebuilt green. Live gateway msg 97893 from `@ip_operant_testing_bot` confirmed.

## Round 39 (2026-09-25) — deployment audit (harness-era)

### R39-1 — lib-test build broken at HEAD 6b25a46e (FIXED 4ade6f51)
`persistence_seam.rs` test module used `Arc` without `use std::sync::Arc` — `cargo test --workspace` could not compile operant-core (lib test) at all. Same commit repointed `.cargo/config.toml` native-lib path from the removed operant-pk worktree to `local/lib`.
- **Verify**: `cargo check -p operant-core --tests` 0 errors; linked suite runs (1787/2).

### R39-2 — doctest linking broken (FIXED 0c722992)
`cargo build.rustflags` never reach rustdoc, so the `approval.rs` doctest linked `-lsonic` with no search path. build.rs now emits `cargo:rustc-link-search=native=<payload>` (repo-local `local/lib`, gitignored, or `$OPERANT_NATIVE_LIB_DIR`), which covers rustc AND rustdoc. `.cargo/config.toml` carries no rustflags.
- **Verify**: `cargo test -p operant-core --doc` green (was 0/1 link failure).

### R39-3 — persistence seam dark-safe test fails at HEAD (FIXED)
`persistence_seam::tests::persistence_seam_pends_when_kernel_off` (persistence_seam.rs:160) asserted `mount` returns `MissingSeam` and entry stays `Pending`, but `activate_locked` wrapped every install error as `ActivationFailed` + `Failed` — the dark-safe "lessons wait, never lost" contract (file header) was unimplemented.
- **Fix**: seam-originated provenance — new `HarnessError::SeamUnavailable(String)` variant; `PersistenceSeam::install` returns it when the kernel runtime is absent or disabled; the harness maps it unconditionally to Pending (no textual heuristic) in `activate_locked`, `mount_locked` (→ `MountReport::Pending` with a `seam/<name>` claim) and `rescan_pending_locked` (kept pending, still rescuable). Landed with the user's C6 machinery (missing-seam heuristic stays for the never-registered case).
- **Verify**: `cargo test -p operant-core --lib` 1789 passed / 0 failed (parallel); harness kernel suite green (29 tests).

### R39-4 — write_approval recency test failed in parallel suite (FIXED, residual noted)
`write_approval::tests::list_pending_orders_by_recency` failed only under parallel execution (`left: 0, right: 2` — another test's `reset()` wiped the global `PENDING` store mid-assert); passed with `--test-threads=1` (1788/1 serial).
- **Fix**: module-level `TEST_LOCK: Mutex<()>` acquired in `reset()` (first call of every test) and held for each test's duration; `set_write_origin_helper` also takes the lock since it mutates the process-global `WRITE_ORIGIN`. The module's own "must run serially" contract is now enforced in-code instead of relying on runner flags.
- **Verify**: `cargo test -p operant-core --lib -- write_approval` 12/0 across 5 consecutive parallel runs; full lib suite 1789/0 parallel.
- **Residual (not blocking)**: `WRITE_ORIGIN` (`write_origin.rs:27`) is process-global and `write_origin.rs`'s own test module runs outside this lock — cross-module origin races remain possible in principle; the observed PENDING/ENABLED failure mode is closed.

### R39-6 — `--all-features` clippy has ~46 unannotated unwrap/expect sites (FIXED iter-331)
`scripts/clippy-warning-gate.sh` runs the workspace clippy with `-D clippy::unwrap_used -D clippy::expect_used`; on default features the tree was green, but with `--all-features` the gate reported 45 errors in `operant-core` (lib) and 1 in `operant-memory` (lib) — feature-gated code paths nobody had annotated. The allowlist held only 4 stale entries.
- **Status**: FIXED (iter-331). Full workspace gate now green under `--workspace --all-targets --all-features`: seen=8 allowlist=8 new=0 stale=0. 46 deny-sites annotated at enclosing-item level across 14 files; real fixes surfaced by the sweep: `context_references.rs` empty-string `unwrap()` → `let-else` (removed a genuine panic path on `operant chat` input), `operant-runtime` `at.lock().unwrap()` → poisoned-guard recovery, `operant-channels` 4× `dm_topic_threads.lock().unwrap()` → `unwrap_or_else(into_inner)`, `live_parity.rs` test-header per repo convention, `vim_command.rs` 7 sites.
- **Gate script defect (fixed iter-337)**: with `set -euo pipefail`, `-D` denials aborted the cargo run at line ~75 before jq ever compared, and the trap deleted `${tmp_json}` — the gate exited 101 with zero diagnostics. Fix: `|| true` on the cargo line + error-level passthrough in the jq filter, so deny violations now reach the allowlist comparison. Verified both ways: green tree → `seen=8 allowlist=8 new=0 stale=0` (exit 0); injected `.unwrap()` in `migrations.rs` → exit 1 with `+ operant_core|clippy::unwrap_used|crates/operant-core/src/migrations.rs` printed. The fixed gate immediately caught a real regression: the `Mutex` import orphaned in `write_approval.rs` by iter-332's TEST_LOCK move (pushed without a post-change gate run), now removed.
- **Residual**: 8 pre-existing non-deny warnings (HEAD debt in pool_adapter.rs, config.rs, cmd_status.rs, main.rs, cmd_architecture.rs, 3 test files) absorbed into the refreshed allowlist via the sanctioned `--update`; prune as they're fixed.

Deployment verdict this round: operant-core lib **1794 passed / 0 failed** (parallel), harness kernel + live-loop suites green, integration suite green, clippy gate green under `--workspace --all-targets --all-features`. The lost CLI-side WIP (labelled "018" after the harness-side commit `c8fc536f`, "fix(iter-018)") was rebuilt in iter-339 as the audit's C3 slice — MetricsSnapshot exposure on `operant status` + `architecture dump --live` — plus the C6 test-contract update. Genuinely open harness gaps from the r16 audit (docs/audit/2026-09-02-r16-harness-adoption-audit-fresh.md §10): **C1** prompt/hook seam wiring in main.rs (highest operator value), **C2** WASM watcher → Extism factory swap, **C5** CLI-half example validation (harness.yml itself was dropped in iter-342).

### R39-5 — clippy gate red: 2 unwrap/expect violations in committed host.rs (FIXED on default features)
- `crates/operant-harness/src/host.rs:73` — `expect()` ("HarnessHost::add_seam requires exclusive ownership") → annotated `#[expect(clippy::expect_used, reason = "invariant: no shared Arc<Harness> clones exist at host setup")]`
- `crates/operant-harness/src/host.rs:102` — `unwrap()` (`ps.pop().unwrap()`) → annotated `#[expect(clippy::unwrap_used, reason = "guarded by !ps.is_empty() directly above")]`
- Four harness test files (`builder_factories.rs`, `host_boot.rs`, `soak.rs`, `source_roundtrip.rs`) were missing the standard `#![allow(clippy::unwrap_used, clippy::expect_used)]` test-target header.
- **Verify**: `cargo clippy -p operant-harness --all-targets -- -D clippy::unwrap_used -D clippy::expect_used` green (0 errors).

### R39-11 — `operant doctor` false `✗ OpenAI (invalid API key)` against a configured local gateway (FIXED iter-336)
The API-connectivity section fired every provider's key at that provider's **default** URL. With the omp omniroute gateway configured (`OPENAI_BASE_URL=http://localhost:20129/v1`, which `provider_from_url` maps to `ollama`), the `OPENAI_API_KEY` override key was sent to `https://api.openai.com/v1/models` → 401 → `✗ OpenAI (invalid API key)` + `✗ OpenAI Codex (invalid API key)` on every run, while the configured endpoint actually answered 200.
- **Fix** (`checks_api.rs`): key-ownership rule — a provider whose key came from `OPENAI_API_KEY` (the var `apply_env_overrides` routes to `client.api_key`) is only probed when the configured base URL maps to that provider, and then at the **configured** URL; otherwise the probe is skipped. A dedicated `Active LLM endpoint` probe covers the configured URL when the generic sweep doesn't (local gateways map to `ollama`, which has no api_key sweep entry).
- **Verify**: 4 unit tests on the decision helper (`omp_gateway_skips_openai_env_key_probes`, `non_active_endpoint_still_probes_unrelated_keys`, `mapped_active_provider_probes_configured_url`, `empty_active_base_keeps_default_behaviour`) 4/0; live `operant doctor` → `✓ Active LLM endpoint`, `✓ Kilo Code`, `✓ OpenCode Zen`, no ✗; live agent turn through small-stack → `SMALLSTACK_OK`.
- **Companion defect (fixed iter-344)**: the key lookup scanned the full `env_vars` list, which mixes credentials with base-URL overrides (`google: [GOOGLE_API_KEY, GEMINI_API_KEY, GEMINI_BASE_URL]`) — with only `GEMINI_BASE_URL` set, doctor would send the URL as the Bearer token. `is_key_env_var` now filters `*_BASE_URL` vars out of the key scan (alternate keys like GEMINI_API_KEY still honored); unit-tested.

### R39-7 — cron CLI and scheduler read different DBs: CLI-created jobs invisible to the runtime (FIXED)
`cmd_cron.rs` opened the SHARED `database_path` (`~/.operant/database.db`, user_version=2, owned by the sessions migration family) while the runtime scheduler (`main.rs:974`) correctly used the dedicated `operant_cron.db`. Two user-facing failures: (a) `operant cron list` hard-failed on the shared-PRAGMA migration guard ("schema for cron is at version 2 but only 1 migrations are declared"); (b) any job created via the CLI was invisible to the scheduler — split-brain between CLI and runtime. Live data: 0 rows in the shared table, 6 real jobs in the dedicated file (all recovered by the fix).
- **Fix**: `cron_db_path(config)` helper in `cmd_cron.rs` mirroring `main.rs` (`db_dir.join("operant_cron.db")`), all 11 `CronDb::init` call sites repointed.
- **Verify**: `operant cron list` → 6 jobs, exit 0; release rebuilt + redeployed; `cargo check -p operant-cli` green.
- **Design note (pre-existing, not fixed here)**: the migration runner keys on SQLite's file-wide `PRAGMA user_version`, so any second migration family pointed at the same file hits the same refusal — cron/kanban/sessions each need their own DB file (kanban already does).

### R39-8 — AFT bridge accepted a dead PATH shim, shadowing the managed cache (FIXED)
`aft_bridge.rs:121` (resolve step 2) accepted any `which("aft")` hit unverified. An orphaned mise shim (`~/.local/share/mise/shims/aft` — mise's registry has no `aft` tool, so the shim errored instantly) was accepted over the working managed cache (`~/.operant/aft/aft-v0.50.1/aft`, step 3), so EVERY `aft_*` tool call died with `failed to write to stdin: Broken pipe` while a healthy binary sat in the cache. A stale PATH entry defeated auto-provisioning entirely.
- **Fix**: probe the PATH candidate once (`aft --version`) before accepting; on failure warn and fall through to cache/download. Orphan shim removed from the environment.
- **Verify**: live tool turn through omp small-stack — `aft_bash` ran `echo tool-path-ok`, agent reported verbatim output `"tool-path-ok\n"`, exit 0.

### R39-9 — `operant memory stats` always reports "Total sessions: 0" (FIXED)
`cmd_memory.rs::cmd_stats` counted `MemoryManager::list_sessions()`, an in-process map populated by `get_or_create_session` during a live agent run and never persisted by `MemoryManager::save_to_disk` (which writes only memories + profiles via `MemoryStore`). Every fresh CLI invocation therefore reported 0 sessions while the canonical session store (`database.db`, surfaced by `operant sessions list`) held 1095+.
- **Root cause**: misleading stat source, not data corruption — the memory store simply has no persisted session registry; sessions are conversation-scoped and owned by `Database`.
- **Fix**: `handle_memory_command` now passes `AppConfig`; `cmd_stats` reads the count via `Database::init(config.database_path).get_session_count()` (same path as `cmd_sessions`), with a comment documenting why the in-process map is not the source.
- **Verify**: `target/debug/operant memory stats` → `Total sessions: 1097` (grew from 1095 between readings — proves it live-reads the DB), `Total memory entries: 713` unchanged, exit 0; `cargo check -p operant-cli --bin operant` green (the 2 remaining warnings are pre-existing 018-WIP, main.rs:1420 + cmd_architecture.rs:197).

### R39-10 — background review poisoned WRITE_ORIGIN for process lifetime + cross-module test races on the same global (FIXED iter-332)
Found while closing R39-4's residual. Two defects on the process-global `WRITE_ORIGIN` slot:
1. **Production leak**: `agent/prompting.rs` bound `let _origin_token = set_write_origin("background_review")` inside the `daemon_pool::spawn("probe-extra", …)` review task — but `WriteOriginToken` has **no Drop impl**, so when the review daemon finished, the global stayed `"background_review"` for the rest of the process. In any long-lived surface (TUI, gateway) every skill write after the first background review was write-guarded as if it came from the review fork. **Fix**: use `WriteOriginGuard::background_review()` (Drop-resets) instead of a bare token.
2. **Test isolation**: `write_approval.rs`'s module-private `TEST_LOCK` serialized its own 12 tests, but `skills_tool.rs::record_usage_bridges_curator_tracker_on_create` sets the same global via the guard with no lock — under cargo's default parallel runner it could interleave with `set_write_origin_helper`'s origin asserts. **Fix**: one owner for the rule — the lock moved to `write_origin.rs` (`origin_test_lock()`, cfg(test)), `write_approval::reset()` delegates to it, and the skills_tool test now takes it. Regression test `origin_test_lock_serializes_concurrent_writers` added; mutation-proven (removing one thread's lock → FAILED at write_origin.rs:210, `"thread_a"` observed while `"thread_b"` set; restored → green, 3/3).
- **Residual (closed iter-338)**: the origin used to remain process-global during the review daemon's lifetime, so a concurrent main-agent turn in that window was also seen as background_review. Fixed with a two-tier design: `write_origin.rs` now has a `tokio::task_local!` tier (`TASK_ORIGIN`) preferred by every read, with the process-global `Arc<RwLock>` as fallback for non-Tokio contexts; the review daemon (`agent/prompting.rs`) wraps its future in `scope_background_review(...)` so `background_review` is visible only inside the review task — concurrent tasks keep reading their own origin. Tool futures (`execute_tools` uses `FuturesUnordered` on the same task, no worker spawns) inherit the scope by construction. Regression test `task_scoped_origin_does_not_leak_to_concurrent_tasks` (deterministic oneshot sync, no sleeps), mutation-proven: reintroducing the global set inside the scope → FAILED with `left: "background_review"` observed by the concurrent task; restored → green, plus 3/3 repeated runs. Remaining known limit: any future code path that moves review work onto a NEW `tokio::spawn`ed task would need explicit scope propagation (task-locals do not cross spawn).
- **Verify**: `cargo test -p operant-core --lib` 1790/0; `--test-threads=1 write_approval` 12/0.

### R39-12 — lost 018 CLI-side WIP rebuilt: MetricsSnapshot exposure on status + dump --live (FIXED iter-339)
The uncommitted 018 CLI-side WIP (main.rs / cmd_architecture.rs / cmd_status.rs / harness_agent_integration.rs) was destroyed by an over-broad checkout during iter-331 prep (disclosed in AGENTS.md history). Reconstructed from the r16 audit's S6 finding and PR-queue item 4 ("expose MetricsSnapshot + pending/failed on status + dump --live"): `MetricsSnapshot` (metrics.rs) was serializable but never left the Harness — operators could not see mount outcomes or alert on PENDING churn.
- **Rebuild**: `architecture dump --live` now attaches `HarnessMetrics`, prints the counter block in text mode and adds a `metrics` key to the JSON dump; `operant status` best-effort boots the configured architecture (shared `pool_builder()` + `boot_metrics_snapshot()`) and exposes `harness.metrics` in `--json` plus a `boot: X ok / Y pending / Z failed` human suffix; `prompt_section_config_row_buildable` updated to the C6 contract landed in `c8fc536f` (MissingSeam → `Ok(MountReport::Pending)` with a `seam/prompt` claim, state Pending — the stale test asserted the pre-C6 error/Failed behavior and had been failing at HEAD since `c8fc536f`, hidden because `cargo test --lib` does not run integration targets).
- **Verify**: `cargo test -p operant-core --test harness_agent_integration` 5/0 (incl. new `metrics_snapshot_exposes_pending_mounts`: pending mount counted + serialized snapshot contains `"mount_pending":1`); live `architecture dump --live architecture.toml.example` → `Metrics: mounts 1 ok / 0 pending / 1 failed, …`; live `operant status` (isolated HOME, harness enabled) → `Harness: enabled (…: 2 rows, 2 active, boot: 1 ok / 0 pending / 1 failed)` and `status --json | jq .harness.metrics` shows all 7 counters.
- **Behavior caveat**: `operant status` now best-effort boots a harness when `[harness].enabled=true` — a new (local-only, no-network, pool rows are passive) cost on a previously read-only command; scripts polling `status` will feel it. The counters describe that boot, not a long-lived agent process — the audit's churn-alerting story still needs a live-process surface (dashboard route) if it ever matters at 250 pools.

## Round 40 (2026-09-27) — production-readiness audit (supply-chain, secret-at-rest, mount-loop, R14-4 withdrawal)

### R40-1 — secret-at-rest files chmod'd after the write (FIXED)
Two writers created secret files with the umask default (0644 under umask 022) and only then tightened to 0600 — a window where the key/token is world-readable on disk. `operant-channels`' matrix writer already had the correct atomic form (`OpenOptions::…mode(0o600)` in one `open(2)`).
- **Sites**: `operant-config/src/secrets.rs` key-file write (master ChaCha key, hex at rest); `operant-channels/src/wechat.rs write_private` (sync cursor).
- **Fix**: both ported to the atomic 0600 `OpenOptions` form (`write_secret_file` / `write_private`), matching `matrix.rs::write_with_owner_only`; redundant post-write `set_permissions` removed (Windows keeps the icacls step — new-file ACLs are user-scoped there).
- **Verify**: `cargo test -p operant-config --lib secrets` 50/0 incl. new `write_secret_file_creates_owner_only_perms` (asserts mode 0o600 regardless of umask). **Caveat, recorded honestly**: `wechat.rs`/`matrix.rs` sit behind the undeclared `channels-vendor` cfg (vendor deps — `matrix_sdk` etc. — are not declared in Cargo.toml, same dark-code pattern as hardware-vendor), so the wechat port is verified by inspection + the identical compiled twin in secrets.rs; it cannot compile in any shipped configuration until the vendor SDK deps are declared.

### R40-2 — runtime npx spawns resolved `@latest` / unpinned (supply-chain; FIXED)
Every cold boot could fetch whatever npm served at that moment and run it with the operator's privileges, on default paths:
- `agent_memory.rs:219` auto-spawn ran `npx -y @agentmemory/agentmemory@latest` (default memory provider, auto-spawn default true);
- `config.rs ensure_default_mcp_servers` and `operant-config/schema/config_impl.rs` both registered `npx -y @agentmemory/mcp` with no version at all (deferred, but spawned on first connect).
- **Fix**: `DEFAULT_AGENTMEMORY_VERSION = "0.9.29"` pinned const in `operant-config` (crate root); `[memory] agentmemory_version` config override (`MemorySettings::agentmemory_package_spec`/`agentmemory_mcp_spec`, whitespace-only falls back to the const); schema world mirrors its documented env pattern with `AGENTMEMORY_VERSION`. Auto-spawn logs the pinned package spec. All three spawn sites now emit `@agentmemory/<pkg>@0.9.29` (or the operator's pin). Auto-spawn itself is untouched (AGENTS.md mandate).
- **Verify**: core `agentmemory_version_pins_package_specs` + updated MCP-injection tests (spec is pinned, never `@latest`/bare) 2/0; `operant-config` ensure_default 3/0; `operant.example.toml` documents the knob. Version 0.9.29 = `npm view` current at fix time.

### R40-3 — C5 mount-cap check rebuilt the full DumpTree per mount, O(n²) across a 250-pool boot (FIXED)
`host.rs` checked `self.harness.dump().await.providers.len()` before each mount; `Harness::dump` clones every provider spec (provides/requires `.to_vec()`), builds claims, and sorts both — per mount, over a growing tree.
- **Fix**: `Harness::provider_count()` — a bare `len()` on the read lock; the cap check uses it.
- **Verify**: `cargo test -p operant-harness --test host_boot` 4/0 incl. new `mount_all_rejects_past_max_active_providers` (cap 1, two rows → `CompositionError` naming the cap and rejected provider, first mount survives) and a `provider_count == dump().providers.len()` parity assertion.

### R40-4 — R14-4 withdrawn as misread; dead `_signing_secret` removed (FIXED)
R14-4 claimed Slack signature verification "implemented in the same file" was unreachable through the Slack adapter. It is in `webhook.rs` (iter-125) and IS reachable: Slack-over-HTTP is served by `WebhookAdapter`, which verifies `x-slack-signature` against `gateway.webhooks_secret`. `SlackAdapter` is Socket Mode only (WS, bot-token auth) — no signed requests exist there, so a wiring iteration would have laundered a dead credential. Dead field + constructor parameter removed (`SlackAdapter::new(token)`); both call sites updated. No schema field needed: the Slack-webhook credential is the existing `webhooks_secret`.

### R40-5 — tagged-release pipeline broken end-to-end (FIXED same round)
`release.yml` read `CHANGELOG.md` at repo root while the file lives in `docs/`; `docs/CHANGELOG.md` had no `## [0.2.0]` section (newest 0.1.4) while `Cargo.toml` says 0.2.0; `build.yml` cloned `../tdg-rust` for a path dependency removed from the workspace. A `v0.2.0` tag push would have run build → release → changelog extraction and exited 1 with nothing published.
- **Fix**: release.yml path → `docs/CHANGELOG.md`; `## [0.2.0]` section added covering iters 331–347; tdg-rust clone step dropped from build.yml (workspace is self-contained — AGENTS.md Path Dependencies).
- **Verify**: workflow YAML parses (`python3 -c yaml.safe_load` on both); changelog section matches the release-notes extraction format; no tag pushed (tagging is the operator's call).

### R40-6 — C1: prompt/hook kernel seams had no live consumer (FIXED iter-349/350)
The r16 audit's highest-value topology gap: `prompt` and `hook` providers could
install into slots nothing read, so only the `tool` family evolved at runtime
("the harness title is still a bus"). Both seam types already existed in
`operant-runtime` (`agent::prompt_seam` / `hooks::harness_seam`) but their only
constructors are tests, and the runtime `Agent` is not the CLI's agent — the
live path is `operant-core`'s `OperantAgent` with its own frozen prefix and its
own event-based `HookRegistry` (whose `with_hook_registry` had **zero**
production callers, so the loop's `emit` calls were no-ops).
- **Fix (iter-349)**: `operant-core::harness_slots` — `PromptSlot` +
  `HookSlot` with their seams, `Effect`-undo uninstall, bounded prompt slot
  (default 32). Sections render into `build_frozen_prefix` (the review fork
  inherits them for free); a new `HookEvent::Hooks("*")` registry pattern lets
  one `bridge_handler` see every live event, and `build_harness_host` returns
  both slots so `AgentCore` hands them to both agent factories. The hook
  registry is constructed only when the harness boots — the disabled-harness
  path stays byte-identical.
- **Verify (iter-349)**: `frozen_prefix_includes_kernel_prompt_sections`,
  `prompt_seam_mount_lands_in_slot_and_unmount_clears_it` (real kernel
  mount/unmount through the seam), `hook_bridge_fires_kernel_hooks_on_live_events`;
  core lib 1801/0; CLI check clean.
- **Gate record (iter-350)**: the post-commit gate caught 2 new lints in the
  C1 diff (`len_without_is_empty` on both slots, `redundant_closure` in the
  test) — both fixed in iter-350, re-verified green (slots 2/0, prefix 3/0).
  The first "fix" used `Arc::new(String::from)` as the test's render closure:
  that renders an empty string, which `render()` filters out, so the
  assertion would have passed vacuously — caught and replaced with a named
  `fn render() -> String` returning a real payload.
- **Not fixed here**: the gate's 3 remaining new warnings
  (`format_in_format_args` commands.rs:167, `manual_is_multiple_of`
  key_handling.rs:1675, `unnecessary_sort_by` keybindings/report.rs:198) are
  in the concurrent agent's uncommitted TUI work. Unlike iter-345 (a
  committed, stale file), editing uncommitted in-flight files would mix
  their code into this tree and conflict with their next push; that gate red
  belongs to their iteration.

### R40-14 — the command-approval blocklist was never applied to `code_execution` (FIXED iter-383, HIGH)
`crates/operant-core/src/approval.rs:656` extracts the command for the
approval gate with:
```rust
"terminal" | "code_execution" | "process" => args.get("command")...
```
But `CodeExecutionArgs` (`crates/operant-core/src/tools/code_execution.rs:42-46`)
is `#[serde(rename_all = "camelCase")]` with fields `code` / `language` /
`timeout` — there is no `command` key. The extractor falls through to
`.unwrap_or(tool_name)`, so the hardline blocklist and dangerous-pattern layer
receive the literal string `"code_execution"`, which matches nothing.
**Net effect: `code_execution` is not gated by the command blocklist at all.**
No test covers this extractor, which is why the mismatch survived. `terminal`
and `process` are unaffected — verify their arg structs before changing the
match.
- **FIXED iter-383**: `code_execution` now has its own arm reading `code`,
  split out of the `terminal | process` arm. `patch` was found broken the same
  way in the same function — grouped with `file_write` and reading
  `args["content"]`, but `PatchArgs` is `path`/`find`/`replace`, so it extracted
  an empty payload and the text it writes was never gated. It now has its own
  arm too. Four regression tests; mutation-proven both directions.
  - **Note for the next reader on the test shape**: the natural end-to-end
    assertion (`verdict != "allowed"` under `mode: Some("manual")`) is a
    TAUTOLOGY — manual mode returns `requires_approval` for every tool before
    the blocklist runs, so it passes with or without the bug. The shipped test
    uses default (smart) mode and asserts `verdict == "blocked"` AND
    `blocked_by == Some("hardline")`, with a benign-payload control so it
    cannot pass by blanket denial. The payload must be the literal
    `rm -rf /*`: the regex alternative requires the WHOLE string to be
    `rm -rf /`, so wrapping it in `os.system(...)` falls through to the
    weaker pattern layer.
  the host with the operant process's own permissions (already recorded as
  R12-1 / BUGS.md:135 — it writes a temp file and invokes python3/node/bash
  directly, with timeout + kill_on_drop + the approval gate as the only
  mitigations). The approval extractor being dead meant the last of those
  mitigations was inert for this tool. Fix this alongside R12-1's unsandboxed
  note, not independently of it.

### R40-15 — secondary provider keys (`api_keys`) were written to config.toml in plaintext (FIXED iter-380, HIGH)
`crates/operant-config/src/schema/core.rs`: `api_key` (line 43) carries
`#[secret]`; `api_keys` (line 50) — the credential-pool list that rotates in on
401/429 — carries only `#[serde(default, skip_serializing_if = ...)]`. So the
primary key is encrypted on save and every pooled secondary key is serialized
in plaintext to `config.toml` on each `save()`. The doc comment on `api_key`
("never commit it to config.toml directly") makes the asymmetry a
claim-must-match-code defect, not just a gap. Compare the hardened reference
implementation, `operant-config/src/secrets.rs:294-301`, which writes with
`OpenOptionsExt::mode(0o600)` at creation.
- **The fix is NOT one attribute (corrected iter-379)** — **this note was
  WRONG and is retracted; see the correction below.**: the derive macro
  documents its supported types at `crates/operant-macros/src/lib.rs:43` —
  "`#[secret]` on a `String` or `Option<String>` field". `api_keys` is a
  `Vec<String>`, so annotating it would be silently ignored. The real work is
  extending the macro's generated `secret_fields` / `set_secret` /
  `encrypt_secrets` / `decrypt_secrets` (`operant-macros/src/lib.rs:847`)
  to handle a collection, and only then annotating the field. Budget it as a
  macro iteration, and the test must assert a pooled key round-trips
  encrypt→save→load as ciphertext — a config round-trip test is NOT enough,
  because `skip_serializing_if` means a leaked key can look correct.
- **CORRECTION (iter-380, supersedes the note above)**: that advice was based
  on a stale doc comment. The macro ALWAYS supported `Vec<String>` — see the
  `is_vec_string` branch at `operant-macros/src/lib.rs:206-237`, which emits
  per-element `secret_fields`, encrypt and decrypt operations. The claim that
  annotating `api_keys` "would be silently ignored" was false. The shipped
  fix was therefore exactly one attribute on `api_keys` plus a doc-comment
  correction at `lib.rs:43` (which is what misled the iter-379 note in the
  first place). No macro change was needed or made.
  - Test: `config_save_encrypts_credential_pool_keys` in
    `schema/core/core_tests.rs`, asserting on RAW file contents (no pooled key
    appears in the clear) and that each element is `SecretStore::is_encrypted`
    and decrypts to its original. A config round-trip is insufficient for the
    reason given above.
  - Mutation-proven both directions: without `#[secret]` the test fails; with
    it, 645/645 in operant-config.

### R40-16 — subprocesses inherited the full parent environment, including API keys (FIXED iter-381, MEDIUM)
`crates/operant-core/src/tools/terminal_backend.rs:95-100` reads
`std::env::vars().collect()` and passes the whole map to the child whenever
`env_vars` is non-empty. When `env_vars` is EMPTY the child still inherits the
parent environment — `std::process::Command` inherits by default and there is
no `env_clear()` on either path. So both branches leak: the non-empty case
explicitly, the empty case implicitly. No allowlist/denylist scrub of
`*_API_KEY` exists anywhere in the file.
- **Unverified sub-claims** (reported by the audit, not read here — do not
  treat as fact): that `LocalBackend` uses bare `child.kill()` with no
  `kill_on_drop`/process-group teardown, and that the Docker/SSH backends
  accept a timeout they never enforce. Note that R12-1/its neighbours already
  document `kill_on_drop` for `code_execution.rs` as a KNOWN accepted gap, so
  these need direct
  reading before they are actioned. (iter-381 read the code: `code_execution.rs`
  DOES use `kill_on_drop(true)` + `process_group(0)` at all five spawn sites.
  The claim was about `terminal_backend.rs`'s LocalBackend, which is a
  different path and is still unverified.)
- **FIXED iter-381**: `sanitized_env()` now clears and rebuilds the child
  environment from the parent minus credential-shaped names, applied at
  LocalBackend plus all five `code_execution.rs` spawn sites (python, node,
  shell, rustc, run). A denylist, not an allowlist, because the tool exists to
  run arbitrary user commands; explicit `env_vars` overrides still win.
  - **Do not reintroduce a bare `AUTH` fragment.** It strips `SSH_AUTH_SOCK`
    and silently breaks every `ssh` / `scp` / `git push`. The fragments are
    deliberately suffix-shaped (`API_KEY`, `_TOKEN`, `TOKEN_`, `SECRET_`,
    `_SECRET`, `PASSWORD`, `PASSWD`, `CREDENTIAL`, `PRIVATE_KEY`,
    `SESSION_KEY`). Caught in review before it shipped; pinned by
    `ordinary_env_vars_are_not_treated_as_secret` and by the end-to-end
    `spawned_command_does_not_inherit_api_keys`, which both fail if `AUTH`
    returns.
  - Docker and SSH backends were NOT leaking and are unchanged: they forward
    only explicit `env_vars` and never inherit the parent environment.
  - Mutation-proven: re-adding bare `AUTH` fails both tests with
    "SSH_AUTH_SOCK must pass through" and "scrub stripped SSH_AUTH_SOCK,
    breaking ssh/scp/git-push". operant-core 1812 pass at that commit.

### R40-12 — AGENTS.md's "7 platforms only" is false: 23 channel features are in the DEFAULT build (OPEN, MEDIUM, CORRECTED iter-376)
`AGENTS.md` states "Supported: 7 platforms", "Do NOT re-add purged platforms",
and records that iter-50 purged 20 phantom platforms (matrix, mattermost,
signal, …). Measured against the committed tree, that is no longer true:
- `crates/operant-cli/Cargo.toml` `default = ["agent-runtime", "gateway"]`,
  and `agent-runtime` enables **23** `channel-*` features — including
  `channel-signal`, `channel-mattermost`, `channel-irc`, `channel-imessage`,
  `channel-dingtalk`, `channel-qq`, `channel-bluesky`, `channel-twitter`,
  `channel-reddit`, `channel-notion`, `channel-linq`, `channel-wati`,
  `channel-nextcloud`, `channel-mochat`, `channel-wecom`, `channel-clawdtalk`.
  (24 `channel-*` features are declared in `operant-cli/Cargo.toml`; the 24th,
  `channel-webhook`, is not in the `agent-runtime` list.)
- These are NOT phantoms resurrected from nowhere: every one is declared in
  `operant-channels/Cargo.toml` and has a real source file (65 files under
  `crates/operant-channels/src` — `bluesky.rs`, `irc.rs`, `matrix.rs`,
  `mattermost.rs`, `nextcloud_talk.rs`, …). So the accurate finding is the
  inverse of "a purge was undone": **23 real, implemented platform adapters
  are unreachable from the shipped binary** because the CLI never references
  `operant-channels` (R40-13). The gap is missing WIRING, not dead code.
- **Operator decision, deliberately not executed here**: removing these
  feature flags would touch a stated Design Preference ("Do NOT change"), and
  the adapters are working code. Correct framing is "wire them or explicitly
  retire them", never "purge the phantoms".
- Corrected from the first statement of this item (iter-374): it reported "22"
  and separately "15" for the same set. The count is 23.

### R40-13 — 4 default-feature crates are in the normal dep graph with no reference from operant-cli/src (OPEN, HIGH, CORRECTED iter-376)
`cargo tree -p operant-cli -e normal --depth 1` lists `operant-channels`,
`operant-runtime`, `operant-gateway` and `operant-tools` as NORMAL (non-dev,
non-build) dependencies of the CLI. Each has **zero** references from
`crates/operant-cli/src` at HEAD. They are compiled into the default binary and
never called.
- **Corrections to the first statement of this item (iter-374)**, which said
  six crates that are "linked" — both parts were wrong:
  - `operant-memory` is NOT among them: the CLI reaches memory through a
    re-export, `operant_core::memory::` — `cmd_memory.rs:12`
    (`use operant_core::memory::{MemoryBlock, MemoryManager}`) and
    `cmd_tui_debug.rs:365`. A bare crate-name grep misses this path.
  - `operant-hardware` is optional and NOT in the default feature set, so it
    is not compiled by default at all.
  - "Linked" was unproven: the installed and `target/release` binary is
    stripped and stale (md5 `39e6c798`, predating the peer's iter-356/357),
    so `nm -C` returns zero symbols for EVERY crate and proves nothing. The
    `cargo tree` normal-graph membership is the supported claim.
- This single root cause explains three older ledger items at once: **R5-3**
  (operant-runtime `RuntimeAgent` unreachable), **R13-3** (`run_gateway` has
  no caller because the CLI never reaches `operant-gateway`), **R15-1**
  (operant-channels unwired).
- Two resolutions, not equivalent, and not an audit's call: (a) wire the
  subsystems so the default features mean something, or (b) drop them from
  `default` and delete the crates if genuinely retired. Per R40-12 the
  adapters are real implementations, so (b) discards working code. Operator
  decision, its own iteration.

### R40-11 — `origin/main` did not compile: iter-357 shipped a reader without its field (FIXED by peer iter-382, HIGH)
`0482fa1b` (peer, `fix(iter-357)`) added
`max_tool_result_share: settings.max_tool_result_share` at
`crates/operant-core/src/agent/mod.rs:170`, sourced from `BehaviorSettings`,
but **the field was never declared on that struct** —
`crates/operant-core/src/config.rs:193` (`BehaviorSettings`) has no
`max_tool_result_share` at HEAD, so every build of a clean checkout fails:
```
error[E0609]: no field `max_tool_result_share` on type `&BehaviorSettings`
  --> crates/operant-core/src/agent/mod.rs:170:45
```
- **Repro, measured**: a clean worktree of HEAD fails
  `cargo check -p operant-cli --bin operant` with the above. To reproduce:
  `git worktree add --detach /tmp/owrepro HEAD && cd /tmp/owrepro && cargo check -p operant-cli --bin operant` (the worktree used for the original measurement, `/tmp/owcheck`, has since been removed; any pristine checkout reproduces it).
  The concurrent agent's UNCOMMITTED
  `crates/operant-core/src/config.rs` does contain the missing field declaration,
  its `#[serde(default = "default_max_tool_result_share")]` attribute, the
  `default_max_tool_result_share()` helper, and the `Default` impl entry.
  **Superseded**: an earlier revision of this entry claimed the shared tree
  stayed green only because of that uncommitted file. That is no longer true —
  the shared tree now carries its OWN 12 errors from the peer's in-flight
  refactor (E0107/E0277/E0308/E0369/E0593/E0631), so NEITHER tree compiles.
  Their uncommitted file remains the *intended* fix, not a working one.
  This is the SECOND time `origin/main` has been left uncompilable by an
  explicit-path commit (first: iter-359, R40-9) — same failure class.
- **BOTH repair sites are peer-dirty (corrected iter-379)**: the plan had
  proposed reverting the reader at `agent/mod.rs:170` to
  `DEFAULT_MAX_TOOL_RESULT_SHARE` as a "HEAD-owned, unblocks today" fix.
  Measured: `git status` shows BOTH `crates/operant-core/src/agent/mod.rs`
  and `crates/operant-core/src/config.rs` are modified by the concurrent
  agent (36 dirty files at last count). So neither the revert nor the field
  declaration can be applied without editing a file the peer holds — the
  R40-10 clobber class. **There is no clean one-line repair available until
  the peer's working tree is committed or cleared.** Options: (a) wait, (b)
  ask the peer to land `config.rs` (their field, with its example-toml line),
  (c) if a repair becomes urgent, coordinate explicitly before touching either
  file rather than racing it.
- **Blast radius beyond compilation**: even once the field lands, the
  config surface must also reach `operant-config`'s schema, or the knob will
  be unreadable/settable-from-nothing — the same two-file thread that
  `agentmemory_version` needed (iter-347). Verify, do not assume.
- **Both independent pre-merge mechanisms would have caught it**: the clean
  clippy gate on HEAD reports `seen=2 / new=1 / stale=7` — the `new` entry is
  `operant_core|E0609|crates/operant-core/src/agent/mod.rs`, the identical
  error. Neither check is a pre-merge RULE in any enforced sense — both are
  local, author-run, and nothing forces either to execute on a push (all four
  workflows are tag-triggered; R40-7). The gap is not a missing check; it is
  a check that nothing runs automatically.

- **Not fixed here on purpose**: the one-line fix lives in a file the
  concurrent agent is actively editing, and the field they wrote is theirs to
  land with their surrounding work (default value + schema + example config
  are all part of the same change). Committing a partial version would be the
  R40-10 mistake in reverse.
- **A config test would NOT have caught it**: `BehaviorSettings` is
  `#[serde(default, deny_unknown_fields)]` with a hand-written `Default`
  (`config.rs:191-193`), so a round-trip test passes as long as the struct is
  self-consistent — the failure was a *reader* reading a field the struct
  never had, which only `cargo check` can see. Do not add a test expecting
  to cover this class.

### R40-7 — CI predicate aligned to the local gate; the main-branch trigger is still fmt-blocked (PARTIAL, iter-369)
All four workflows fire only on `push: tags: ['v*']` (+ dispatch) — no
`branches: [main]`, no `pull_request`.
- **Predicate divergence FIXED (iter-369)**: `ci.yml`'s clippy job called
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  a DIFFERENT and stricter rule than
  `scripts/clippy-warning-gate.sh` + `.ci/clippy-allowlist.txt` — so
  "CI green" and "local gate green" were different statements about the same
  tree, and the gate is the repo's only pre-merge rule. The job now installs
  `jq` and runs the gate script. Two subtleties that made a naive swap fail:
  the workflow sets `RUSTFLAGS: -Dwarnings` globally, and cargo applies that
  ON TOP of the gate's own `-D clippy::unwrap_used -D clippy::expect_used` —
  it would promote every allowlist-absorbed warning to a hard error, so the
  step sets `RUSTFLAGS: ""` (step-level env overrides workflow-level in GitHub
  Actions); and the gate needs `jq`, which is not in the toolchain image.
- **The RUSTFLAGS interaction is worse than a red CI — measured (iter-370)**:
  with the workflow's `RUSTFLAGS: -Dwarnings` inherited, `cargo clippy` aborts
  on the FIRST denied warning, so the gate collects almost nothing:
  `seen=1` against an 8-entry allowlist, `stale=7`, exit 0. It does not fail —
  it passes VACUOUSLY, having linted a fraction of the tree. With
  `RUSTFLAGS=""` it reports the true `seen=8 / new=0 / stale=0`. So the
  step-level clear is what makes the CI verdict mean anything at all; without
  it, a green CI would mean "clippy stopped early", which is the same class
  of false- green as iter-337's gate script dying before its comparison.
- **Still open**: a `branches: [main]` trigger cannot land until the fmt debt
  is cleared, because `ci.yml` runs `cargo fmt --all --check` under
  `-Dwarnings` and the committed tree carries fmt debt (17 files at last
  measure). The sweep and the trigger must land together.
- **Why the sweep is still deferred**: the concurrent agent holds uncommitted
  work in the shared tree — `git status` showed 77 modified files. Their TUI
  lint warnings cleared because they FIXED those lints, not because they
  stopped. A 17-file reformat over their working copy is the same class of
  mistake that destroyed the 018 WIP (R39-12).

### R40-9 — `pub mod harness_slots;` never committed: nine uncompilable commits on origin/main (FIXED iter-359)
iter-349 created `crates/operant-core/src/harness_slots.rs` and wired the C1
agent, but its explicit-path `git add` did not list
`crates/operant-core/src/lib.rs`, so the module declaration stayed uncommitted
in the working tree. Every commit from iter-350 to 358 — on `origin/main` —
failed to compile for anyone who cloned: E0433 `cannot find harness_slots in
crate` at `agent/mod.rs:371` and `agent/builders.rs:292/301`. Found only by
compiling from a CLEAN WORKTREE; every local test run shared the working tree
where the line existed uncommitted, which is why 9 pushes of green local tests
never caught it.
- **Fix**: iter-359 adds exactly the one missing line. Verified with
  `git worktree add` at the fix commit: `cargo check --release -p operant-cli
  --bin operant --features plugins-wasm` → exit 0, zero errors.
- **Durable rule**: after any commit made with `git add <explicit paths>`, the
  staged file list must cover every file the change creates or edits —
  `git diff --cached --name-only` is not enough if the mental list is wrong.
  The reliable check is a clean-worktree compile of HEAD, since a shared
  working tree masks missing declarations entirely.

### R40-10 — peer working-copy clobbered by an unscoped `cp` (iter-359, disclosed)
While isolating the R40-9 fix, I overwrote `crates/operant-core/src/lib.rs`
with a copy derived from HEAD instead of adding one line to the existing file.
That destroyed the concurrent agent's UNCOMMITTED edits in the same file:
* `pub mod terminal_hints;` relocated from just after `pub mod daemon_pool;`
  down to just after `pub mod gateway_pipeline;` (both the `-` and `+` lines
  were visible in the diff)
* de-indentation (one leading space removed) of `pub mod mcp_oauth;`,
  `pub mod memory;`, `pub mod memory_provider;`, `pub mod migrations;`
  (both sides visible)
* `pub mod persistence_seam;` removed from its position between
  `harness_seams_r3` and `interrupt` — the `-` line was visible but the
  re-add fell beyond the 40 lines of diff I had read, so its new position is
  NOT known from my capture. It is NOT a deletion: `persistence_seam` is
  declared exactly once at HEAD (lib.rs:79) and the module itself is intact
  (last touched by `c8fc536f`, iter-018), so treat this as a relocation whose
  destination must be re-established from the peer's own editor history.
All five are cosmetic (module ordering / whitespace); no functional change was
lost. **The content is not recoverable from git** — those edits were never
staged, stashed, or hashed, and no matching dangling blob exists (searched via
`git fsck --lost-found`). They are reconstructible from the `git diff` output
quoted in the session transcript. Root cause: overwriting a file that another
agent holds uncommitted work in, in a shared tree — the same class of mistake
as the over-broad checkout that destroyed the 018 WIP (R39-12). Correct move
would have been `git diff`-ing the file first, applying a targeted edit, and
staging only the intended hunk (`git add -p`).

### R40-8 — C2: WASM hot-swap was kernel-only by absence of a bridge (FIXED iter-353/354/355)
The r16 audit's C2 said P4 was kernel-only until the WASM watcher was wired
into `Harness::replace` with an Extism factory. Every piece it named exists
(`extism 1.21` declared in operant-plugins, `create_plugin`,
`validate_skill_bundle`, `Watcher::run_until`, `Harness::replace`, per-scan
Ed25519 re-verification). What was missing was the mapping from a watcher
`ManifestChange` to a kernel-mountable `ArchitectureRow`, plus the two policy
decisions the audit left open.
- **Bridge (iter-353)**: `plugin_tools.rs` (the only module seeing both
  operant-plugins and operant-harness — operant-plugins must not depend on the
  kernel) converts a verified change into a row whose `config["claims"]` match
  what `WasmProvider` materializes. Claims are PAIRED with their seam and the
  tool name is read from the module's `tool_metadata` export, not mapped from
  `PluginCapability` (Tool/Channel/Memory/Observer/Skill is a coarser
  vocabulary; a guessed name would install a tool the agent cannot call). A
  first draft cross-producted every name under every seam — caught in review,
  now guarded by a test.
- **Watcher (iter-354)**: opt-in via `[harness] watch_wasm` (default false) —
  a permanent poller whose every detected change re-instantiates the module
  must not start as a side effect of `[harness].enabled`. Runs
  `SignatureMode::Strict` unconditionally and refuses an empty
  `[plugins] trusted_publisher_keys` (which would mark every plugin
  `Untrusted` and silently never swap). First-time plugins are MOUNTED, not
  replaced (`Harness::replace` only swaps a mounted provider; there is no
  `get_provider`, so membership comes from `dump()`).
- **Dead code (iter-355)**: the gate caught `assert_verifying_host` — called
  with the `SignatureMode::Strict` constant, so it could never fail, i.e. a
  tautology dressed as a security check — plus the variant only it constructed
  and an unread `KernelRow::claims`. All three deleted; the policy stays where
  it is actually enforced (the `with_signature(Strict, keys)` construction and
  the empty-key refusal) and is pinned by a test instead of a helper nobody
  called.
- **Verify**: bridge tests 5/0 (`--features plugins-wasm`): unverified verdicts
  never become rows, skill/observer unmapped, memory manifest claims only its
  own seam, tool without metadata refused rather than guessed, strict-mode
  policy pinned. `--features plugins-wasm` check clean; default-features check
  clean; bin suite 721/0; gate `seen=8 / new=0 / stale=0`.
- **Not done**: an end-to-end swap test is BLOCKED, not merely outstanding —
  it needs a real signed `.wasm` module plus a trusted publisher key, and the
  repo contains no `.wasm` fixture (searched: zero under
  `crates/operant-plugins`). Authoring one is a new deliverable, not a test
  edit, so it is tracked as its own item rather than faked with a stub module
  that would prove only that the code runs. The unit tests cover the bridge,
  the signature policy, and the claim mapping; the unexercised step is a real
  Extism swap in a real plugins dir.
- **Deployment reality (iter-357)**: C2 is gated behind the `plugins-wasm`
  cargo feature, which is default-OFF. The deployed `/usr/local/bin/operant`
  (default release build) therefore does NOT contain the watcher or the
  bridge — verified by `strings`: `trusted_publisher_keys` is present (the
  AppConfig field is not feature-gated) but `watch_wasm refused` is absent.
  Shipping the hot-swap requires a feature-enabled release build
  (`--features plugins-wasm`). C2 is "landed and compiling", not "deployed".
- **Deploy predates iter-359/360 (no rebuild owed)**: the deployed binary
  (md5 `39e6c798…`) was built from a commit before the missing-module fix and
  the builder-dispatch test. iter-359 is NOT a cosmetic change — it restores
  compilation: at that commit a clean checkout could not produce ANY binary
  (`E0433` in operant-core). The deployed artifact was still built and
  verified, because it was compiled in the shared working tree where the
  declaration was present but uncommitted. iter-360 is test-only. So no
  rebuild is owed, but the reason is that both changes restore/build from
  already-shipped source, not that they are "non-binary": a release build
  from HEAD now succeeds where a clean-checkout build did not.
  Rebuild only when a change alters shipped code paths.
- **Signature policy unified (iter-363), runtime-verified (iter-365)**: the
  tool bridge had hardcoded `PluginHost::new` (Disabled) while the watcher
  hardcoded Strict — two paths, two policies, no operator control. `[plugins]
  signature_mode` + `trusted_publisher_keys` now feed BOTH through
  `with_security` + `parse_signature_mode`, and the swap watcher REFUSES a
  non-strict configured mode with an explicit message instead of silently
  upgrading it. This supersedes the "kept in sync manually" note above.
  iter-363 was compile-verified only (clean-worktree `--features
  plugins-wasm` check); iter-365 adds the missing runtime exercise:
  `configured_mode_reaches_the_host_and_the_key_set_is_honoured` pins the
  whole `parse_signature_mode` mapping, asserts the AppConfig default equals
  the schema world's so the two surfaces cannot drift, and builds a real host.
  It failed on first run — the test assumed `with_security` errors on a
  missing plugins dir, but it creates one and returns Ok (host.rs:41-44);
  the assertion now matches the real contract. Bridge suite 6/0. The tool
  bridge also logs the enforced mode + key count at info, so strict-with-
  empty-keys (which silently loads nothing) is visible rather than inferred.
- **Build interference**: a `--features plugins-wasm` release build in the
  shared tree fails on `tui/image_paste.rs:391` and `tui/image_render.rs:456`
  (E0308 / E0277) — both in the concurrent agent's UNCOMMITTED working copy
  (last committed at their `f956f535`), neither a C2 file. Verification of
  C2's release compile is therefore done in a clean `git worktree` at HEAD
  (exit 0 at iters 359, 360 and 363).

### R40-11 addendum — RESOLVED by the peer, not by this audit (iter-384)
The repair sites were peer-dirty for the whole window this item was open, and
an attempt to land the `DEFAULT_MAX_TOOL_RESULT_SHARE` fallback in an isolated
worktree was **deliberately discarded** once the peer's own fix landed
(`1f6fb2b1`, iter-382): their commit added the field to `BehaviorSettings`
(`config.rs:215`) and kept the intended read at `agent/mod.rs:172`. Reverting
to the constant at that point would have replaced their correct fix AND created
a write-only config knob — a field that loads, persists, and changes nothing,
which is the same defect class as R40-14. `cargo check -p operant-cli --bin
operant` on that tree is green.
- **Still open, minor**: `operant.example.toml` does not mention
  `max_tool_result_share` (grep = 0) even though AGENTS.md requires a template
  line for each new config field. Tracked here rather than as a new item.
- **Process note**: nothing in CI runs on a push to main (all workflows are
  tag-triggered), so a broken `origin/main` is only caught if an agent happens
  to run `cargo check`. This is the second time (first: iter-359) — see R40-7.

### R40-17 — CI now runs on pushes to main; the `doc` job is still red (PARTIAL, iter-391)
**Landed (iter-391)**: `ci.yml` gained `branches: ['main']`, so `fmt` and
`clippy` — the two jobs verified green, and the two that would have caught a
compile break — now run on every push. That is the direct fix for both
R40-11-class incidents going forward.
**Still open**: the `doc` job (below), and `test.yml` / `build.yml`, which stay
tag-only because their 3-OS all-features release matrix and tarpaulin run are
unmeasured. The `doc` job was deliberately left ENABLED rather than excluded
from the workflow, so its failure stays visible instead of being silently
dropped — a partial trigger that quietly omits the red job is exactly the
false green this audit already caught once (the RUSTFLAGS interaction,
iter-370).
The obvious fix for R40-7 is adding `branches: ['main']` to the workflows. **Do
not do that yet** — I built the patch, then ran each job locally, and two of
them are already red, so enabling the trigger turns every push red and trains
everyone to ignore CI.
- `ci.yml` `doc` job (`RUSTDOCFLAGS=-Dwarnings`, `cargo doc --workspace
  --no-deps --all-features`): **red — 26 errors, now 44**. Re-measured at
  iter-391 with `--keep-going`, which is what makes the number trustworthy:
  plain `cargo doc` halts at the first failing crate, so crate-by-crate runs
  reported counts that grew as earlier crates were fixed. Use
  `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps --all-features
  --keep-going` for any future recount. Breakdown of the 44, by class:
  - **unclosed HTML tag (21)** — the largest class, and pure rustdoc
    mechanics: angle-bracket generics and bracketed words in `///` prose are
    parsed as HTML. `HashMap`, `String`, `dyn`, `name`, `key`, `value`, `id`,
    `level`, `hex`, `text`, `prompt`, `filename`, `scope`, `repo`,
    `OperantAgent`, `custom-name`.
  - **unresolved link (22)** — spans all three sub-classes, and they need
    DIFFERENT fixes, so do not batch this blindly:
    (a) *unqualified but real* — qualify the path
    (`Composition::resolve` at `discovery.rs:11`, `chat_provider` at
    `provider_registry.rs:10`). Both were fixed in iter-389/391 but are
    unstaged: those files are peer-mixed.
    (b) *renamed* — `NoopProvider` in the `composition.rs:384` doc names a type
    that no longer exists; the code builds `crate::row::NativeRowStub` at :399
    and the only `NoopProvider` in the repo is a test-local struct in
    `operant-harness/tests/host_boot.rs:14`. Fix by naming `NativeRowStub`.
    (c) *never existed* — `from_provider_tables` at `composition.rs:95,104`:
    no such method anywhere in the crate and nothing calls it; the real boot
    path is `discovery.rs:37` → `Architecture::from_toml`. **Do NOT implement
    it to satisfy rustdoc** — that would invent a feature and add a new
    claim-must-match-code defect. Rewrite the prose to state that only the
    `[[row]]` table-array form is supported.
    Also: `ToolCall`, `refresh_coalesced`, `Duration::ZERO`, `AftBridge::bash`,
    `CommandResult`, `add`, `ProviderRegistry`, `reload_skill_cache`,
    `new_with_writer`, `PLAN`, and bare single letters/numbers (`K`, `V`, `R`,
    `moa`, `1`–`5`, `Esc`, `up`, `enter`) that are markdown-looking
    bracketed text, not links.
  - **public documentation links to a private item (2)** — `PluginRegistry` (×2).
  - **this URL is not a hyperlink (6)** — bare URLs such as
    `http://homeassistant.local:8123`; wrap in `<...>`.
- Two of these were fixed in iter-389 (`PgKnowledgeGraph` → private
  `run_on_os_thread`, and `operant-hardware/src/datasheet.rs` unqualified
  module-doc links). The rest is its own piece of work, not part of a security
  iteration: every one is a doc comment asserting something the code does not
  do, which is the same defect class as R40-14's dead extractor.
- `test.yml` runs `cargo test --workspace --all-features` on a 3-OS matrix
  including `--release`, plus `cargo tarpaulin`. It is red regardless of the
  trigger: `tools::kernel::tests::ping_roundtrip` and
  `harness_apply_and_rollback_roundtrip` require the `vendor/prime-agent`
  submodule, and **no workflow passes `submodules: recursive` to
  `actions/checkout`**. That is ~6 red jobs per push the moment the trigger
  lands. Fix the checkout step first.
  - **Now measured, not inferred (iter-387)**: in a worktree with
    `git submodule update --init --recursive` run, `tools::kernel` is
    **6/6 green** (`ping_roundtrip` and `harness_apply_and_rollback_roundtrip`
    both pass). So these two tests are not independently broken — they are
    correct code that CI cannot run, and `with: submodules: recursive` on the
    checkout step is the whole fix. An earlier version of this entry asserted
    the mechanism without running it; it is now verified both ways (fail
    without the submodule, pass with it).
- `ci.yml` `fmt` and `clippy` jobs were verified green. `release.yml` needs no
  change — it fires on `workflow_run` of `Build`, so it inherits the
  main-branch trigger transitively.
- **This sequence is done** (supersedes the "cheapest honest sequence" advice
  that stood here before iter-391): submodules landed at iter-389, the trigger
  landed at iter-391, and `doc` + `msrv` are explicitly gated off with their
  reasons at iter-392. The advice that a partial trigger "would still be a
  partial fix with a confusing signal, so it is not recommended" was wrong on
  the evidence — the gate comments name the reason, so the signal is legible
  rather than confusing. What is still outstanding is fixing the 44 doc errors
  (R40-19) and re-enabling the two gated jobs, in that order.

### R40-18 — fmt sweep is blocked on the concurrent agent, not on anything else (OPEN, LOW, re-confirmed iter-416)
**Re-confirmed at `31e727db`: 60 hunks across 22 unique files, 22 of 22 dirty in
the shared tree — 100% overlap, unchanged.**
**A measurement trap worth recording, because I fell into it this session.**
Running `cargo fmt --all -- --check` in the *shared* tree reports **0 diffs** —
the peer's uncommitted working copy is already fmt-clean and masks the committed
state. I read that as "the sweep is done" and was wrong by 60 hunks. Measure fmt
debt in a **clean worktree at HEAD**, never in the working tree: the number is a
property of the commit, not of the checkout.
`cargo fmt --all --check` reports 60 dirty hunks at HEAD (was 66, then 22, now
60 again — the concurrent agent's commits add more fmt debt than their landed
commits clear). The sweep is 22 files.
**Re-confirmed blocked at iter-401**: 22 of 22 files are dirty in the shared
tree (37 dirty files total, 100% overlap — identical to the iter-385
measurement). Produced and verified again in an isolated worktree:
`cargo fmt --all --check` clean afterwards and `cargo check -p operant-harness
--lib` still compiles, so it is a pure line-wrap. It remains unstaged because
every one of those files carries the peer's in-flight edits, and a whole-file
`git add` would sweep them in (R40-10). Unlike R40-21 this is not waiting on a
single push — it stays blocked for as long as the peer has these files open.
- **EXPECT A RED `fmt` JOB ON THE PEER'S NEXT PUSH — correct, not a
  regression.** Since iter-392 `fmt` is an active ci.yml job on pushes to
  `main`, and this debt is overwhelmingly the peer's iter-391 work, their next
  push will go red on formatting they own. Do not "fix" it for them and do not
  read it as something this audit broke: their fix is `cargo fmt --all` on their
  own work. Pre-empting it with a 60-file reformat across an active tree is
  exactly what this entry exists to prevent — a diff that size obscures their
  change and collides with their next several commits. Same shape as the
  expected clippy red in R40-21: a check reporting work that is not ours.

### R40-19 — the `doc` CI job is GATED OFF: **74** rustdoc errors under `-Dwarnings` (OPEN, MEDIUM, recounted iter-415 at `c3dfa1ab`)

**Recounted iter-415, on a tree that finally compiles.** The previous figure was
44 (iter-391), explicitly recorded as a floor. The real number is **74**, and the
increase is explained rather than alarming: `cargo doc --keep-going` halts per
crate, and at iter-391 a crate that failed to compile never reported its doc
errors at all. R40-21 made the tree compile, so five crates now report
(`operant-harness`, `operant-core`, `operant-channels`, `operant-gateway`,
`operant-cli`) that previously contributed nothing.

Measured with:
`RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps --all-features --keep-going`
in a worktree at `c3dfa1ab` with submodules initialised.

Breakdown by class: **34** unresolved intra-doc links, **19** unclosed HTML
tags (generics like `HashMap<K, V>` and prose containing `<` in keybinding
contexts), **9** "public documentation for X links to private item", **6** bare
URLs that should be hyperlinks. Five "could not document" lines are the per-crate
aborts above, not additional lint findings.

Independently confirmed clean under the same flags: `operant-config`,
`operant-api`, `operant-macros`, `operant-tool-call-parser`, `operant-infra` —
0 errors each.

Original finding, retained:
`ci.yml`'s doc job (`RUSTDOCFLAGS=-Dwarnings`, `cargo doc --workspace --no-deps
--all-features`) is **disabled, not deferred**. The main-branch trigger landed
at iter-391/392 and `fmt` + `clippy` are live on pushes to `main`; `doc` carries
`if: ${{ false }}` with a comment citing this entry, so it defers nothing and
blocks nothing. It is re-enabled by deleting that one line once the count
reaches zero. (An earlier version of this entry said the trigger was "still
deferred" for this job — that was true when written and stopped being true at
iter-392.)
Two errors were fixed in iter-389
breakdown by class is in that addendum; the two largest are 21 unclosed-HTML-tag
(generics and bracketed prose parsed as HTML) and ~14 single-letter or keybinding
"links", only ~8 of which are genuine claim-must-match-code defects. Measured
with the full `--all-features` graph using `--keep-going`, without which the
count is truncated by the first failing crate:
- `unclosed HTML tag` (9): `String`, `scope`, `repo`, `OperantAgent`, `id`,
  `hex`, `Connection` — angle-bracket generics in doc comments that rustdoc
  parses as HTML.
- `public documentation ... links to private item` (6): `PluginRegistry` (×2),
  `Self::build_headers`, `every_interval`, `Self::download_binary`, `VerbKind`.
- `unresolved link` (8): `ToolCall`, `refresh_coalesced`, `Duration::ZERO`,
  `chat_provider`, `AftBridge::bash`, `add`, and others.
- `this URL is not a hyperlink` (3).
- Two fixes made in iter-389 for `operant-harness` (`Composition::resolve`,
  `NoopProvider`) are correct and verified but were deliberately NOT staged:
  those two files carry the concurrent agent's in-flight edits (a `dyn Fn`
  reflow) alongside my doc changes, so staging would have swept their work in
  (R40-10). They land when the peer's work does — one iteration, two hunks.
- **This is claim-must-match-code work throughout**: every one of these is a
  doc comment asserting something the code does not do, which is the same
  defect class as R40-14's dead extractor. The `NoopProvider` case is the
  sharpest — the code builds `crate::row::NativeRowStub` and the only
  `NoopProvider` in the repo is a test-local struct in
  `operant-harness/tests/host_boot.rs:14`.

### R40-20 — I reverted a peer-dirty file twice; the hunks were `cargo fmt` output, not peer work (iter-394, corrected iter-397)

**Addendum, iter-418 — a disclosure I owe.** Removing the 17 GB
`/tmp/operant-verify` worktree during iter-417 used
`git worktree remove --force`, and I did not first check whether it was clean.
Force is the wrong verb on a machine with a concurrent agent: it discards
uncommitted work without warning, and I had not established whose worktree it
was before running it.

What I established afterwards, and why nothing was lost: it was checked out at
`a6cd2bd5` = **my own** `fix(iter-409)` commit, reachable from `main` and
`github/main`, so its work was already committed. The listing that made it look
like the peer's was `ar`'s verbose output from an unrelated command in the same
shell invocation, not that directory's contents.

Outcome benign, process wrong. The rule this adds: **run
`git -C <path> status --porcelain` before removing any worktree, and never pass
`--force` on a shared machine.** Force is the same class as `git checkout --` on
a peer-dirty file — the R40-10 clobber — aimed at a directory instead of a file.

Also worth fixing, both cost time this round: `scripts/dev-env.sh` hardcodes
`/home/z/my-project/local/...` (another machine's paths) and `~/.cargo/env` does
not exist, so sourcing it aborts the shell. Cargo is at `/usr/bin/cargo`; the
toolchain is `/home/ishanp/.rustup`.
While staging the `operant-harness` doc fixes (iter-393) I ran
`git checkout -- crates/operant-harness/src/composition.rs
crates/operant-harness/src/discovery.rs` to strip the peer's in-flight hunks so
my doc edits would be the only thing staged. That destroyed the peer's
uncommitted work in both files: a rustfmt reflow of `ProviderFactory`'s
`dyn Fn(...)` signature, and a batch of `map_err`/`format!` reflows plus a
`Composition::resolve` call-site change in discovery.rs.
- **CORRECTED (iter-396): nothing hand-written was lost.** I ran plain
  `cargo fmt --all` on HEAD and diffed the result against the preserved
  backups: the `dyn Fn(...)` reflow and every `map_err`/`format!` reflow in
  both files are reproduced byte-for-byte by `cargo fmt`. That is R40-18's
  fmt-sweep output sitting uncommitted in the shared tree, not the concurrent
  agent's own editing — so the peer had nothing to recover, and re-running
  `cargo fmt --all` regenerates it. The restore below was still correct (it
  put the fmt output back), but describing it as recovering "their work"
  overstated it. The disclosure stands for the method, not the damage: a
  `git checkout` on a peer-dirty file is unsafe regardless of what the file
  happens to contain.
- **Recovered**: the pre-revert copies were preserved at
  `/tmp/comp_worktree_version.rs` and `/tmp/disc_worktree_version.rs`, and
  both were restored into the working tree. Their work is present and
  uncommitted, exactly as it was.
- **The lesson, now a standing rule**: to isolate your own hunks in a file
  someone else is editing, do NOT `git checkout` it first. Reverting and
  re-applying is a data-destroying operation on a shared tree, and the
  "isolate my diff" benefit is not worth the risk. Use `git add -p` to select
  hunks (it worked cleanly for the one-hunk case at iter-394) or make the edit
  in a worktree. If a revert has already happened, check for a preserved copy
  BEFORE restoring anything, and say so in the ledger — a silent overwrite of
  a peer's uncommitted work is the R40-10 class, and repeating it is how
  R40-10 happened in the first place.

### R40-21 — `origin/main` does not compile AGAIN: peer's iter-391 omitted a `mod` declaration (CLOSED, iter-414; was OPEN/HIGH, measured iter-396)

**CLOSED iter-414.** The peer pushed their own fix; `pub mod cache_monitor;` is
now at `crates/operant-core/src/agent/clients/mod.rs:3` on `main` (`8fd6d6a9`).
This is the outcome iter-400 defined as "fixed": the declaration landed on
`main`, rather than being added from a clean checkout, which would have been a
duplicate. No action was taken here — the remedy belonged to whoever wrote the
unreferenced module, and adding it myself would have created two competing
declarations.

**Consequence: the three items this blocked are unblocked.** The R40-19 rustdoc
recount, a valid release rebuild, and any further measurement that requires a
compiling tree were all downstream of this. See the addendum at the end of this
entry.

Original finding, retained:
Third occurrence of the R40-9/iter-359 failure class. The peer's
`d68ea308` (iter-391, "cache-miss detection") added
`crates/operant-core/src/agent/clients/cache_monitor.rs` and referenced it
from four places, but **never declared the module**:
- referenced: `agent/builders.rs` (2), `agent/run.rs` (5),
  `agent/clients/anthropic.rs` (3), and the file itself
- not declared: `agent/mod.rs:8-26` lists every sibling
  (`background_review`, `chat_provider`, `error_classifier`, …) and has no
  `mod cache_monitor`
- errors: 8× `E0433: cannot find cache_monitor in clients`, plus
  `E0432: unresolved import super::cache_monitor` and
  `E0433: cannot find cache_monitor in super` — 10 total, `operant-core (lib)`
  fails.
**The fix is one line, and its location is now pinned** so it needs no
re-deriving: add `pub mod cache_monitor;` to
`crates/operant-core/src/agent/clients/mod.rs`, after line 4. `clients` is a
DIRECTORY, not a `clients.rs`, so the declaration does NOT go in
`agent/mod.rs` — that file only has `pub mod clients;` at line 989. The
sibling declarations at `clients/mod.rs:2-4` are `pub mod anthropic;`,
`pub mod openai;`, `pub mod prompt_caching;`, which also settles the visibility:
`pub`, matching its siblings. (The E0433 "cannot find in `super`" variant means
some call sites sit one level up in `agent/`, so the `clients` module must stay
`pub` for them to reach it.)
**Not fixed here on purpose.** It is the concurrent agent's in-flight work;
adding a declaration to their new module is a second unverified edit to
someone else's work, and the visibility question above is inferred from
siblings rather than stated by them. It belongs in their commit.
**Do not start a release build until this is fixed.** The installed binary
(`baae1e9d`) predates the breakage, so `operant --version` and `operant doctor`
still work and `target/release/operant` is a valid build — but any rebuild from
the current tree fails at `operant-core`, costing ~9 minutes to learn nothing.
- **Repro (re-confirmed at origin/main, iter-399)**:
  `git worktree add --detach /tmp/owv origin/main && cd /tmp/owv &&
  git submodule update --init --recursive && cargo check -p operant-core --lib`
  → `could not find cache_monitor in clients` at run.rs, 8 errors,
  `could not compile operant-core (lib)`. The submodule init matters: without
  it the failure is masked by a different crate failing first.
- **The fix already exists — UNCOMMITTED in the concurrent agent's working
  copy.** `crates/operant-core/src/agent/clients/mod.rs` in the shared tree
  contains `pub mod cache_monitor;` as a 1-line uncommitted diff, and it is
  correct. So this is not a missed fix, it is a **fix that was never pushed**:
  `origin/main` is broken while the local tree compiles. Anyone reading the
  ledger should NOT add the declaration — that line is already in the working
  copy, and adding it again from a clean checkout is a duplicate. The action is
  to get that commit pushed (or cherry-picked onto main), not to re-apply it.
- **"Fixed" means `origin/main` carries the line, not that a local tree
  compiles.** A fresh clone is broken until that commit lands, so anyone
  running a clean-worktree compile or a CI run right now sees the failure.
  Two consequences:
  (1) the clippy gate (active on pushes to `main` since iter-392) will be RED
      on the peer's next push **if they push other work first without this
      line**. That is expected, not a regression — and it is the first
      occurrence the gate would have caught automatically, so treat the red as
      the mechanism working, not as a new incident.
  (2) Do NOT add the line yourself even though the tree is broken. A duplicate
      `pub mod cache_monitor;` in a file they are actively editing is a worse
      failure than a broken main, and the fix is one commit from landing.
- **Blocks four downstream items**, all of which are unblocked the moment that
  commit lands and nothing more: the R40-19 doc recount (unmeasurable while
  `operant-core` does not compile), the release rebuild (recorded in
  R40-19's addendum), and anything that needs to compile `operant-core`. State
  it as ONE blocked item rather than four, so the next session does not
  re-derive that the tree is broken.
- **Pattern worth naming**: this is the third time `origin/main` has been
  pushed uncompilable (iter-359 → R40-9, iter-357 → R40-11, now iter-391), and
  a clean-worktree compile at HEAD was the only thing that caught it each time.
  Since iter-391, `ci.yml` runs on pushes to `main` with the clippy gate as a
  job, and on a broken tree that gate reports the compile error as a new
  warning and exits non-zero (measured at iter-373). So the third occurrence is
  the first one CI would have caught automatically — provided the push lands
  after the trigger is live on the remote.

### R40-19 addendum — the recount is DONE: 74, as of iter-415 (supersedes iter-396)

**Resolved iter-415.** R40-21 cleared, so the recount this addendum asked for was
run, exactly as specified: `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace
--no-deps --all-features --keep-going`, in a worktree at `c3dfa1ab`, with
`git submodule update --init --recursive`, libclang available.

**Result: 74 errors.** The prediction this addendum made — that the true number
was higher than 44 because a non-compiling crate never reports its doc errors —
was correct. Five crates now report (`operant-harness`, `operant-core`,
`operant-channels`, `operant-gateway`, `operant-cli`); at iter-391 they
contributed nothing. `operant-config`, `operant-api`, `operant-macros`,
`operant-tool-call-parser` and `operant-infra` are clean.

The "4" recorded at iter-396 was the count for crates that still built while
`operant-core` was broken — not a workspace figure, exactly as this addendum
warned.

Note for the next recount: `ci.yml`'s gate comment still cites 44. Both files now
carry a different number, which is the stale-duplicate-figure failure this
ledger has hit before; update the comment and the entry together, or cite only
the entry.

### R40-22 — three remotes point at the same URL, so tracking refs go stale silently (process note, iter-404/405)
`git remote -v` shows `origin`, `github` and `gitlab` — and all three resolve
to the same `https://github.com/ishan-parihar/operant.git`. The two GitHub ones
track `main` under different local ref names, and neither ref is refreshed by a
push to the other.
**The failure this caused, twice in one session**: `git push` reported
"Everything up-to-date" while `git log origin/main` showed the commit as
absent, and `git merge-base --is-ancestor <mine> origin/main` returned false
for a commit the server actually had. Both looked like a peer force-push
having discarded the work, and both were false alarms. A third alarming
reading — `git show origin/main:BUGS.md` displaying pre-edit content — was the
same stale ref, not a reverted file.
- **Ground truth is the server, not a local ref**:
  `git ls-remote <remote> refs/heads/main`. Use it after any push whose result
  you actually need to confirm.
- **Habit**: `git fetch --prune <remote> main` after pushing, before reading
  `origin/main`. A push that reports success is not proof it landed; this is
  the second time this session a nominally successful operation did not do what
  it reported (the first was the clippy gate's "exit 0" while linting 1 of 8
  warnings — R40-7, iter-370).
- **Correction (iter-405)**: an earlier draft of this note claimed the failing
  `merge-base --is-ancestor` check was explained by the two SHAs being equal.
  That is wrong, and it was tested rather than argued: with identical SHAs
  `git merge-base --is-ancestor HEAD HEAD` exits 0, so equality satisfies
  ancestry. The check failed because the ref it was given was stale, and
  against a freshly fetched ref the same check exits 0. Recorded here because
  the wrong explanation is the more plausible-sounding one and would send the
  next reader to compare SHAs when the actual defect is a stale ref.
- **Also note the sequencing**: the fix that actually worked was
  `git fetch --prune`, not the `git update-ref` shortcut the note originally
  suggested — worth stating plainly, since the note described a remedy that was
  never run.
- **No force-push ever occurred**, and none should: AGENTS.md forbids it, and
  with two agents committing to `main` a force-push would discard peer work.
  Every push this session was a fast-forward, verified with
  `git merge-base --is-ancestor` before pushing and `git ls-remote` after.

### R40-23 — 12 wiremock tests flake under parallel load; mechanism not pinned (iter-449, measured not fixed)

`operant-tools` starts 12 `wiremock` `MockServer::start()` servers across
`web_fetch.rs`, `web_search_tool.rs` and `jira_tool.rs`, all on OS-assigned
ephemeral ports. Under `cargo test` parallelism they intermittently fail with
`Connection refused (os error 111)`, which surfaces as a request that never
reaches its own mock.

Measured, not assumed:

| run mode | runs | result |
|---|---|---|
| default (parallel) | 3 | `1161 ok` / `5 failed` / `1 failed` |
| `--test-threads=1` | 4 | `1161 passed, 0 failed` — every time |

So it is a concurrency defect in those tests, not the environment. Ruled out
port exhaustion: the ephemeral range is 32768–60999 (~28k) and the suite runs in
~5s, so 12 servers cannot plausibly collide on volume.

**Not fixed, and the mechanism is genuinely unidentified.** Two candidates were
not distinguished: a bind/rebind TOCTOU on the ephemeral port (bind `:0`, read
the assigned port, close, rebind — a classic wiremock race), or runtime
starvation where the server task is not being scheduled. Writing a fix for a
race I cannot reproduce on demand is how a test suite ends up permanently serial
and slow, which is a worse outcome than a recorded flake. The measurement above
is the actionable artifact: re-run with `--test-threads=1` to confirm any change
actually helped, because "it passed once" proves nothing against a 1-in-3 rate.

Blocks nothing. `ci.yml` runs no test job, and `release.yml` gates on the Build
workflow, not on tests. Severity is test-hygiene, not correctness.

### R40-24 — `$HOME/.operant/` written into the repo root; creator not identified (iter-449, observed)

An 8.8 MB untracked directory literally named `$HOME` sits in the repo root,
containing `.operant/skills/…`. It appeared during a workspace test run.

Ruled out, by reading rather than guessing: it is **not** in a source tarball
(`git archive` contains 0 matching entries, since it is untracked), so the
release is unaffected; and it is not created by
`operant-tools/src/browser.rs::ensure_browser_env`, which sets `HOME=/tmp` when
the variable is missing rather than writing a literal path. The
`operant-runtime/src/service/mod.rs:1000` `getent passwd` path was checked and
`fields[5]` is correct — index 5 of a passwd entry *is* the home directory.

**Creator not identified.** The fix for the related class of bug did land at
iter-449: four tests in `browser.rs` mutated process-global `HOME` and
`CHROMIUM_FLAGS` with no lock, three of them contradicting each other by design.
That is fixed and verified, but it is not proven to be the cause of this
directory, so the two are recorded separately rather than conflated.

Interim handling: not gitignored yet, because adding an ignore rule without
knowing what writes the path risks hiding a live bug rather than fixing it. The
directory is untracked, so it cannot enter a release. Worth `.gitignore`-ing
once the creator is known.

### R40-25 — 7 channel adapters are compiled into every binary but have no dispatch arm anywhere (iter-451, measured)

`operant-cli's` `default = ["agent-runtime", "gateway"]`, and `agent-runtime`
enables **23** `channel-*` features. Of those, 9 are referenced from
`operant-cli/src`. The remaining 14 looked dead — and **that first measurement
was wrong**, which is the useful part of this entry.

**The wrong measurement, kept on the record.** Grepping `operant-cli/src` for
channel names finds 9, and the naive conclusion is "the other 14 are dead
weight". But `operant-channels/src/orchestrator/factory.rs` is a *config-driven*
dispatcher: it matches on a channel name at runtime and constructs the adapter
without the CLI ever naming it. Seven of the 14 are live through it — `dingtalk`,
`imessage`, `linq`, `mochat`, `twitter`, `wati`, `wecom`. This is the same
failure shape as iter-429's wrong-action keybindings, inverted: **absence of a
reference is not evidence of absence of a capability.** A source-presence test
cannot see runtime dispatch.

**The measurement that does hold.** Re-checked against every dispatch site
(`orchestrator/{mod,factory,runtime_types,prompts}.rs`,
`operant-core/tools/{send_message_tool,reaction_tool}.rs`,
`operant-runtime/daemon/mod.rs`, `operant-cli/{cmd_channel,cmd_setup}.rs`) —
30 unique channel names, cross-checked with hyphens AND underscores normalised,
because a `whatsapp_cloud` arm would have made a live channel look dead. Seven
appear in **no** dispatch arm:

    acp-server  bluesky  clawdtalk  nextcloud  notion  reddit  whatsapp-cloud

Each has a real cfg-gated implementation (2–4 files), so all seven compile into
the shipped binary. `bluesky`/`notion`/`reddit` do have string literals in their
own source (`channel: "bluesky".to_string()`), but those are self-references
inside the adapter, not dispatch sites. And `operant acp` is verified
independent of `channel-acp-server`: `cmd_acp.rs:39` calls
`operant_core::acp::server::run_stdio_server`, a different module entirely.

**CORRECTION — I applied the change, measured it, and reverted it.** The
paragraph above originally claimed the removal "is a real reduction in shipped
code and attack surface". That was false, and measurement is what caught it.

I removed the seven from the `agent-runtime` array in
`crates/operant-cli/Cargo.toml` and rebuilt release. Result: **byte-identical
binary, 51,026,384 bytes before and after, 0-byte delta.** Worse, the build log
showed only **1 crate recompiled versus 12** in the baseline build, and
`operant-channels` was not among them. A real feature change cannot recompile
nothing, so the edit was a no-op.

**Why: `operant-gateway/Cargo.toml` hardcodes the features in `[dependencies]`,
not `[features]`:**

    operant-channels = { workspace = true, features = [
        "channel-signal", "channel-acp-server", "channel-email", ... ] }

Cargo unions features across the whole dependency graph, so those 21 channels
are enabled on `operant-channels` unconditionally no matter what `operant-cli`
asks for. The string probe confirmed the removals never took effect — the
`operant-cli` manifest simply is not where those features come from.

Three things this entry got wrong before measurement, all worth keeping:

  - "compiled into the shipped binary" — they are *compiled*, yes, but the
    `strings` probe found **0 occurrences** of bluesky/notion/reddit/clawdtalk/
    nextcloud in the release binary while telegram/discord/slack were present
    73/66/40 times. The linker already drops them as unreachable. So they cost
    **compile time only** — not binary size, and not runtime attack surface.
  - "the individual `channel-*` features stay declared, so anyone who wants one
    can still enable it" — true, verified (`channel-bluesky = [...]` is still in
    the manifest), but irrelevant, because the hardcoded gateway dependency
    enables them anyway. Enabling is not the problem; nothing can *dis*able them.
  - The fix was framed as a one-line manifest edit. It is a three-crate
    refactor, and I would not have found that without a build.

**This is the real defect, and it is bigger than the seven channels.**
`operant-gateway` has `default = []` and 24 correctly-written forwarding
features (`channel-email = ["operant-channels/channel-email"]`, ...). The
hardcoded `[dependencies]` list makes **all 24 decorative** — they forward
features that are already switched on. The dependency list defeats the entire
forwarding mechanism it sits next to.

Correct fix, deliberately NOT done here: delete the hardcoded list from
`operant-gateway`'s `[dependencies]`, then make `operant-cli`'s `channel-*`
features forward to **both** `operant-channels/channel-*` and
`operant-gateway/channel-*`. `operant-cli`'s `gateway = ["dep:operant-gateway"]`
currently passes no channel flags, so removing the hardcoded list without the
second half would silently drop every channel from the binary. That is a real
refactor of the feature graph across three crates, and it is worth doing
deliberately rather than as a side effect of a dead-code cleanup.

### R40-26 — WITHDRAWN: AGENTS.md's "7 platforms" is ACCURATE. The finding compared two different subsystems (RETRACTED iter-458)
**This entry previously claimed AGENTS.md's platform count was "stale by 9" and
that the code had drifted from a stated design preference. There is no drift,
and the claim was a category error.** Withdrawn in full.

**What AGENTS.md actually claims.** The four sites (lines 190, 262, 488, 623,
929) say `**Supported: 7 platforms** — telegram, discord, slack, whatsapp,
email_smtp, sms_twilio, webhooks`, and the section header at line 188 is
`### Platform Adapters (Gateway)`. That claim is about the **gateway platform
adapters in `crates/operant-core/src/gateway/`**, and it is **exactly right**:

| module | adapter |
|---|---|
| `telegram.rs` | telegram |
| `discord.rs` | discord |
| `slack.rs` | slack |
| `whatsapp.rs` | whatsapp |
| `email.rs` | email_smtp |
| `sms.rs` | sms_twilio |
| `webhook.rs` | webhooks |

Seven adapters, one per claimed platform. (`admin.rs`, `lifecycle.rs` and
`types.rs` are also in that directory but are infrastructure, not platforms.)

**Why the entry was wrong.** I counted `operant-channels` factory arms — 21
unique dispatch arms, 10 of them (`dingtalk`, `imessage`, `lark`, `linq`,
`mochat`, `twitter`, `voice-call`, `wati`, `wechat`, `wecom`) not named
anywhere in `operant-cli/src` — and compared that against the gateway's 7.
**`operant-channels` and `operant-core/src/gateway/` are different subsystems.**
AGENTS.md never mentions `operant-channels` at all: `grep -iE
'channel-?feature|operant-channels|channel adapter' AGENTS.md` returns nothing.
So the document makes no claim that the channel count contradicts, and my
"drift away from a stated design preference" argument had no premise.

**Two further errors inside the original entry**, recorded because they are the
kind that survive review: it said "the nine extra reachable channels" and then
listed **eleven** names; and four of those names — `irc`, `qq`, `signal`,
`mattermost` — are among the 20 phantom platforms that AGENTS.md records as
**purged in iter-50**. So the entry contradicted both itself and the audit it
cited, in a document whose whole purpose is to stop exactly that.

**Nothing to do.** The genuine questions about the channel subsystem — that 23
channel features compile by default, and that 10 are reachable only through the
config-driven factory — are already recorded on their own merits in R40-25, with
the measured proof that removing them from the CLI is a no-op.

#### The transferable lesson

This is the **second** false entry filed in one sitting, and the same error
shape as R40-28: I took a number from one context and reported it against a
claim from another, without checking that the two were the same subject. R40-28
compared *local* tag refs against *remote* state; R40-26 compared *channel*
adapters against a *gateway* claim. In both, a cheap check would have caught it
— `git ls-remote` for the tags, `ls crates/operant-core/src/gateway/` for the
adapters — and in both the plausible-sounding number is what made it convincing
enough to file. **Before filing a "the docs are stale" finding, grep the doc for
the thing the finding is about and confirm the doc actually claims it.**

### R40-27 — Plan G's four "dead code in the shipped binary" claims are all false; verified, not assumed (iter-452)

R40-13 has been carried as "the largest remaining engineering item" and as
"the biggest lever on binary size and attack surface". I finally tested it
instead of restating it. **All four claims fail.** Two of my own measurement
passes were wrong before I got there, and both are recorded here because the
errors are the reusable part.

| claim | verdict | evidence |
|---|---|---|
| `operant-channels` 75k lines dead-linked | **FALSE** | declared at `crates/operant-cli/Cargo.toml:21`, pulled in by `agent-runtime` at `:106` |
| 14,094-line channels orchestrator dead-linked | **FALSE** | `orchestrator/factory.rs` is the *live* config-driven dispatcher — 21 unique channel arms (R40-25) |
| `operant-runtime` 89k-line `RuntimeAgent` legacy stack | **FALSE** | `RuntimeAgent` appears in **zero** `.rs` files. It occurs only in 3 markdown files. The "89k" was my own bad measurement: `find crates -name '*.rs' -path '*runtime*'` sums *every* file whose path contains "runtime", across multiple crates — it never isolated a `RuntimeAgent` |
| 4 default-feature crates in the graph, unreferenced from `operant-cli/src` | **FALSE as dead code** | all 4 are in `operant-cli`'s graph at full depth. "0 references from `operant-cli/src`" means reached *transitively*, not unused |

**My two bad measurements**, both the same mistake: I grepped for a quoted
string (`"operant-channels"`) when Cargo declares a dependency as an unquoted
key (`operant-channels = { ... }`), so a correct dependency reported as absent.
Twice. The channel-feature version of the same error is in R40-25. **When
auditing a manifest, parse it; do not grep it.**

**The methodology error underneath all of it.** "In the dependency graph" and
"reachable" are different properties, and the second is what matters. The
`strings` probe from R40-25 is the honest test: bluesky/notion/reddit appear
**0 times** in the release binary while telegram/discord/slack appear 73/66/40.
The linker already removes unreachable code, so an unreferenced crate or an
undispatchable adapter costs **compile time, not shipped bytes and not runtime
attack surface**. Every "dead code in the shipped binary" claim needs a
`strings`/binary probe to be worth anything; a dependency-graph check cannot
establish it.

Nothing to do here. The AGENTS.md platform-count drift is real and stays filed
as R40-26, which is a design-intent question rather than dead code.

### R40-28 — WITHDRAWN: the "orphan tag" finding was false. Real residue: a stale local tag, and `v0.1.4` has no release (RETRACTED iter-458)
**This entry previously claimed that `v0.1.3` and `v0.1.4` were orphan tags
pointing at commits that are not `main` history, and recommended deleting them.
That was false, and it was wrong twice over before it was caught.** Withdrawn in
full; the real (much smaller) residue is below.

**Why the claim was false.** The remote tag targets are ordinary `main` commits:

| tag | target | in `git log origin/main`? | subject |
|---|---|---|---|
| `v0.1.3` | `f8069a02` | **yes**, 1 line | `chore(release): cut v0.1.3` (2026-04-20) |
| `v0.1.4` | `fddbd265` | **yes**, 1 line | `fix(ci): move tdg-rust clone before rust-cache step` (2026-07-19) |

Both are genuine ancestors of `main`. Had the recommendation been applied, it
would have deleted two valid release markers.

**Two independent measurement errors produced it**, and both are worth
recording because either alone would have been enough:

1. **I inspected local refs and reported them as remote state.** The local
   `v0.1.3`/`v0.1.4` are *divergent* — local `v0.1.3` is `93ff0361`, remote is
   `f8069a02`; identical subject, different commit. The local ones live only on
   `archive/stash-*` branches, so `git branch --contains` and
   `git log origin/main` both reported "not in main" and I believed them. The
   divergence is detectable at fetch time, and I had seen that signal without
   reading it: `git fetch --tags` fails with `! [rejected] v0.1.3 -> v0.1.3
   (would clobber existing tag)`. **A tag that cannot be fetched is a fact
   about the tag, not a warning to be ignored.**
2. **`git describe` was worthless as evidence.** It reported `v0.1.2` because it
   ignores **lightweight** tags by default — and `v0.1.3`/`v0.1.4` are
   lightweight while `v0.1.2`/`v0.2.0` are annotated. That is the entire
   explanation, with no divergent history involved. To include lightweight tags
   you must pass `--tags`, which then correctly reports `v0.2.0-13-gcea14fef`.

I also over-corrected on top of the error: having found the claim needed a
forbidden force-push, I "corrected" it to repairable-via-`git push origin
:refs/tags/...` and recommended deletion. The repairability was true and
irrelevant; the premise was false. **A correct answer to the wrong question is
still a wrong answer**, and I have now shipped two of them in one entry (the
iter-455 unrepairable claim, then the iter-455/456 orphan claim).

#### What is actually true, and worth fixing

1. **A stale local tag shadows the remote, breaking `git fetch --tags`.** This
   clone carries `v0.1.3` -> `93ff0361` and `v0.1.4` -> `e781fcb9`, neither of
   which is on `main`. Any `git fetch --tags` is rejected until they are
   corrected. Fixed at iter-458 with `git fetch origin --tags --force`, which is
   a local-ref operation only and pushes nothing.
2. **`v0.1.4` has a tag but no GitHub release.** `gh release list` shows
   `v0.2.0` (Latest), `v0.1.3`, `v0.1.2` — and no `v0.1.4`. So the tag exists on
   a real `main` commit while the release entry was never published. That is a
   genuine gap and the owner's call: publish it, or drop the tag.
3. **`v0.1.3`/`v0.1.4` are lightweight while `v0.1.2`/`v0.2.0` are annotated.**
   Cosmetic, but it is why `git describe` under-reports, and it costs the tag a
   message and a tagger identity. Not worth rewriting published history for.
### R40-29 — RESOLVED: the example config misstated the browser default and omitted the real one entirely (fixed iter-459)
`crates/operant-core/src/config.rs:1181` defaults `browser.provider` to
`"obscura"`, while `AGENTS.md`, `operant.example.toml` and the deployability
audit all said `igs`. The observation was **true**; my framing of it as an
owner decision was **wrong**, and I had also missed part of the defect.

**`obscura` and `igs` are the same engine, not two options.** `browser_provider.rs:9`
describes `"obscura"` as "Local Obscura binary (shared with IGS)", and
`ObscuraProvider` resolves `OBSCURA_BIN` -> config override -> *the IGS-managed
binary* -> an operant-managed copy, with a test comment noting that downloads
land in IGS's `bin/` "so igs reuses them". `igs` is the CLI wrapper over the same
browser; `obscura` drives it directly over CDP. So AGENTS.md's stated intent —
"Default: IGS ... Do NOT switch the default away from igs" — **is satisfied** by
the code as written. There was no design decision outstanding, and changing the
default to `igs` would have been a behaviour change justified by a misreading.

**The part I missed:** `operant.example.toml` did not merely state the wrong
default, it **omitted `obscura` from the option list entirely**. The one
provider a fresh install actually uses was undocumented, and the list's
`default` marker pointed at a different one.

Fixed at iter-459: the example now marks `obscura` as the default, lists it, and
carries a note that the uncommented `provider = "igs"` line selects the CLI
wrapper explicitly (same engine, different driver) and can be commented out to
inherit the default.

**Why it drifted, and the guard.** `config::tests::example_toml_parses` pins
`agent.model`, `tui.rich_output` and two `autonomous` fields — nothing in
`[browser]`. Fixed without a guard, it would simply re-permit the drift, so
`example_toml_documents_the_real_browser_default_and_providers` now pins two
properties: the value marked `(default)` equals `BrowserSettings::default()`, and
every documented provider is genuinely accepted by `build_browser_provider` (an
unknown name silently falls back to Lightpanda, so a typo is otherwise
invisible). Both assertions mutation-proven independently — restoring the exact
shipped bug, and adding a bogus provider, each fail the guard while the
sibling test still passes.


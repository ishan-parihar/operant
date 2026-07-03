# Plan: Over-Engineering Remediation & Integration Hardening

**Status:** Draft for execution
**Date:** 2026-07-03
**Scope:** `hermes-rs` workspace (`crates/operant-core`, `crates/operant-cli`)
**Reference impl:** `../hermes-agent` (Python)
**Generated from:** `/ponytail-audit` findings + owner decisions

---

## Owner Decisions (locked)

| # | Audit finding | Decision |
|---|---|---|
| 1 | `web/` React dashboard dead | **RETAIN** — out of scope |
| 2 | espeak build orphan + kokoro TTS | **Remove espeak, KEEP kokoro** |
| 3 | `fuzzy_match.rs` (662 LOC, 0 callers) | **RETAIN** — deemed useful |
| 4 | Env stubs `modal/daytona/vercel.rs` | **IMPLEMENT IN FULL** per `hermes-agent` |
| 5 | `gateway/platforms/{discord,slack}.rs` stubs | **FIX** — implement real adapters |
| 6 | Dual config (`core/config.rs` + `cli/config.rs`) | **UNIFY** — CLI follows operant's config |
| 7 | `adapter_types.rs` (1902 LOC, 63 types) | **INVESTIGATE native vs adapter** integration |
| 8 | yagni traits, scratch md, runtime artifacts, dialog consolidation, `CompressionStrategy::Summarize` | **APPLY** the remaining optimizations |

---

## Phase 0 — Safety Net (prerequisite)

**Goal:** establish a baseline before any cut, so every phase is verifiable.

- [ ] P0.1 `cargo fmt --all`
- [ ] P0.2 `cargo check --workspace` → record baseline (must be green)
- [ ] P0.3 `cargo test --workspace` → record baseline pass count
- [ ] P0.4 `cargo clippy --workspace --all-targets --all-features -- -D warnings` → record baseline
- [ ] P0.5 Snapshot `git status` + create branch `remediation/overengineering`

**Verify:** baseline green on `main`; branch created. No code changes.

---

## Phase 1 — Zero-Risk Deletions (verified dead, fully reversible)

These are mechanical, evidence-backed deletions. Each item independently compiles after removal.

### 1.1 Remove espeak build orphan; keep kokoro

**Evidence:** `build.rs` links `sonic` and compiles `espeak_audio_stubs.c` for espeak-ng, but **no espeak crate is a dependency** in any `Cargo.toml`. TTS uses `kokoro-tiny` + `hound` (`tts_tool.rs:16`, `voice.rs`).

**Files:**
- DELETE `crates/operant-core/build.rs` (11 LOC)
- DELETE `crates/operant-core/espeak_audio_stubs.c` (43 LOC)
- EDIT `crates/operant-core/Cargo.toml`: remove `[build-dependencies] cc = "1.0"` block

**Verify:** `cargo build -p operant-core` green; kokoro path (`tts_tool.rs:890 kokoro_local`) still compiles & a `cargo test -p operant-core --lib tools::tts` run passes.

**Net:** −54 LOC, −1 build-dependency (`cc`), removes a spurious `-lsonic` link flag.

### 1.2 Remove two yagni traits (0 implementors)

**Evidence:**
- `MessageFilter` trait + `MessagePipeline` in `gateway_pipeline.rs` (~38 LOC): pipeline always constructed empty → `process()` unconditionally returns `Allow`. Consumer at `gateway_runner.rs:581`.
- `LlmReviewClient` trait + `SkillSummary`/`SkillVerdict` in `curator/review.rs` (~40 LOC): sole use is `_llm_client: Option<&dyn LlmReviewClient>` at `curator/mod.rs:138` (underscore-prefixed = unused).

**Actions:**
- DELETE `crates/operant-core/src/gateway_pipeline.rs`; remove `pub mod gateway_pipeline` from `lib.rs`; remove the dead `MessagePipeline::new()` call at `gateway_runner.rs:581` (replace with direct `Allow`).
- DELETE `crates/operant-core/src/curator/review.rs`; remove `pub mod review` from `curator/mod.rs`; drop the `_llm_client` param at `curator/mod.rs:138` and its call sites.

**Verify:** `cargo check -p operant-core`; `cargo test -p operant-core --lib curator gateway`.

**Net:** −~78 LOC.

### 1.3 Remove `CompressionStrategy::Summarize` placeholder

**Evidence:** `context_compressor.rs:65-68` — `Summarize` just calls `compress_truncate(.., 0.5)`, identical to `Truncate{keep_ratio:0.5}`; only constructed in tests.

**Actions:** delete the enum variant, its match arm, and its test.

**Verify:** `cargo test -p operant-core --lib agent::context_compressor`.

**Net:** −~8 LOC.

### 1.4 Move runtime artifacts to `.gitignore` (do not ship DBs)

**Evidence:** `hermes.db`, `hermes_cron.db`, `hermes_kanban.db`, `telegram_offset.txt` are live runtime state committed to git.

**Actions:**
- `git rm --cached hermes.db hermes_cron.db hermes_kanban.db telegram_offset.txt`
- append to `.gitignore`: `*.db`, `telegram_offset.txt`

**Verify:** `git status` shows removals from index, files persist on disk, fresh clone no longer fetches DBs.

**Net:** −1 runtime DB blob set from VCS.

### 1.5 Remove root AI-scratch markdown

**Evidence:** ~90 KB of transient audit/report output at repo root, not referenced by docs build, README, or CI.

**Files to delete:** `AUDIT_FINAL.md`, `REPORT.md`, `VERIFICATION_REPORT.md`, `AUDIT_CLI_PARITY.md`, `AUDIT.md.old`, `TUI_ADAPTER_BUG_REPORT.md`, `TUI_AUDIT_REPORT.md`, `TUI_MODAL_AUDIT.md`, `TUI_SLASH_COMMAND_AUDIT.md`, `TDG-INTEGRATION-PLAN.md`.

**Keep:** `AGENTS.md`, `README.md`, `CHANGELOG.md`, `BUGS.md`, `TODO.md`, `MEMORY.md`, governance docs (`CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `MAINTAINERS.md`), `CLAUDE.md`.

**Verify:** no `md` links break (`grep -rn "AUDIT_FINAL\|VERIFICATION_REPORT\|..." .` outside deleted set).

**Net:** −~90 KB scratch markdown; cleaner repo root.

### 1.6 Dialog consolidation (shrink) — *deferred to Phase 5*

The 17 `*_dialog.rs` files in `tui/` are a `shrink` candidate but touch live UX. Bundle into Phase 5 (TUI native-integration analysis) where `adapter_types.rs` is also re-evaluated, to avoid double-refactor.

---

## Phase 2 — Unified Config (CLI follows operant's config)

**Problem:** two parallel config worlds coexist:
- **core** `crates/operant-core/src/config.rs` (1,269 LOC, 32 structs) — `AppConfig`, TOML-first, the source of truth for runtime behavior.
- **cli** `crates/operant-cli/src/config.rs` (3,722 LOC, 70+ structs) — `CliConfig` + `OperantConfig`, **YAML**-first with deep-merge + `.env` (mirrors Python `hermes_cli/config.py`), then converted *into* core types via `From<AppConfig>` (`adapter_types.rs:209`).

The CLI config was ported from the Python project's YAML schema and drifts independently from the core TOML schema. Owner decision: **the CLI must follow operant's (core) config.**

### 2.1 Canonicalize the source of truth

- **`operant-core::config::AppConfig` is the single source of truth.** All runtime behavior reads `AppConfig`.
- The CLI keeps its loader (YAML + `.env` + deep-merge is a genuine feature — environment-layered config the Python project depends on), but it **deserializes into `AppConfig` (and core sub-structs)**, not a parallel `CliConfig`/`OperantConfig`.

### 2.2 Migration plan

1. **Audit delta.** Diff the 70+ CLI structs against core's 32. Produce a table: (a) structs that exist in both → keep core's, delete CLI's; (b) structs only in CLI → determine if they're runtime-relevant; if yes, move the struct into `core/config.rs`; if no (pure TUI presentation), relocate to `tui/` presentation types (Phase 5).
   - *Spot-checked overlaps:* `ModelConfig`, `BrowserConfig`, `TerminalConfig`, `MemoryConfigV2` vs core's `MemorySettings`, `TtsConfig` vs `TtsSettings`, `GatewaySettings`, `McpSettings`, etc.
2. **Change the loader target.** In `config.rs:2512 CliConfig::load()`, replace the `CliConfig`/`OperantConfig` deserialize target with `AppConfig`. Keep the YAML→`Value`→deep-merge pipeline; only the final `serde_yaml::from_value::<AppConfig>` changes.
   - This requires `AppConfig` and its sub-structs to derive `Deserialize` for YAML (already serde-derived; YAML vs TOML is a parser choice, the struct is shared). Verify field aliases match the YAML keys used in `operant.example.toml` and the Python `cli-config.yaml.example`.
3. **Eliminate `From<AppConfig> for Config` conversions** at `adapter_types.rs:209` once both sides speak `AppConfig`.
4. **Update `operant.example.toml`** to reflect the unified schema (per `AGENTS.md`: config field changes require example update in the same change).
5. **Update `~/.operant/operant.toml`** loader path; document that YAML config files are still accepted (loader parses YAML), but the schema is `AppConfig`.

### 2.3 Risk & rollback

- **Risk:** field-name mismatches between the Python-YAML schema and core's TOML schema (e.g. `model` vs `agent.model`, `api.url` vs `client.base_url`). Mitigate with `#[serde(alias = "...")]` on core fields to accept both names during transition.
- **Rollback:** gated behind one PR; if runtime behavior regresses, revert the loader-target change (the deep-merge pipeline is untouched).

**Verify per step:** `cargo test --workspace`; a `operant config show` round-trip (load → serialize → reload) is stable; `operant run --query "hi"` starts and reads the expected model/provider.

---

## Phase 3 — Implement Environment Stubs in Full

**Problem:** `crates/operant-core/src/environments/{modal,daytona,vercel}.rs` are stubs that return `"not yet implemented"`. `EnvironmentType::Modal/Vercel/Daytona` are never constructed. Owner decision: **implement in full following `hermes-agent` patterns.**

### 3.1 The contract to satisfy

Reference: `hermes-agent/tools/environments/base.py` — `BaseEnvironment(ABC)`.

```rust
// Rust equivalent (already partially exists in environments/mod.rs)
#[async_trait]
pub trait Environment: Send + Sync {
    async fn execute(&self, command: &str, cwd: Option<&str>, timeout: Option<u32>, stdin: Option<&str>) -> ExecResult;
    async fn cleanup(&self) -> Result<()>;
    // shared: init_session(), get_temp_dir(), _wrap_command() — base behavior
}
// where ExecResult = { output: String, returncode: i32 }  (mirrors base.py:940)
```

All three implementations share the base flow: `init_session()` snapshot → per-call `_before_execute()` (sync files) → `_run_bash()` (backend-specific) → `_wait_for_process()` → `_update_cwd()`.

### 3.2 Daytona (port of `hermes-agent/tools/environments/daytona.py`, 10 KB)

**Reference behavior:**
- SDK: `daytona` Python SDK → in Rust, call the **Daytona REST API** directly over `reqwest` (no Rust SDK exists). Endpoints inferred from SDK: `POST /sandbox` (create from image), `GET /sandbox?labels=` (list), `POST /sandbox/{id}/start`, `POST /sandbox/{id}/stop`, `DELETE /sandbox/{id}`, `POST /sandbox/{id}/toolbox/exec` (run command), `PUT /sandbox/{id}/files` (upload), `GET /sandbox/{id}/files` (download).
- Auth: `DAYTONA_API_KEY` + `DAYTONA_SERVER_URL` + `DAYTONA_TARGET` (standard Daytona CLI env). Read from `AppConfig` (new `DaytonaSettings { api_key, server_url, target, image, cpu, memory, disk, persistent_filesystem }`) — add to `core/config.rs` under a new `EnvironmentsSettings`.
- Persistent sandboxes: name `hermes-{task_id}`, label `hermes_task_id`; on construct, try `get(name)` → `start()`; else `list(labels=…)` → `start()`; else `create()`. Cleanup: `stop()` (persistent) or `delete()` (ephemeral).
- File sync: port `FileSyncManager` (sync `.hermes/` dir up before exec, sync back down on cleanup). Bulk upload via `fs.upload_files` multipart.
- `_stdin_mode = "heredoc"` — embed stdin as heredoc in the command (Daytona exec doesn't pipe stdin).

**Rust file:** `crates/operant-core/src/environments/daytona.rs` — replace the 66-line stub.

### 3.3 Modal (port of `modal.py` + `modal_utils.py`, ~24 KB)

**Reference behavior:**
- SDK: `modal` Python SDK → in Rust, call **Modal REST API** (`https://modal.com/api/v1`): `POST /sandbox/create`, `POST /sandbox/{id}/exec`, `POST /sandbox/{id}/terminate`, file mount via `_modal.Mount.from_local_file`. No official Rust SDK; use `reqwest`.
- Auth: Modal uses `MODAL_TOKEN_ID` + `MODAL_TOKEN_SECRET` (standard `modal token new` output). Add `ModalSettings { token_id, token_secret, image, timeout, cpu, memory, gpu }` to config.
- The Python impl wraps async SDK calls in a `_AsyncWorker` thread — in Rust this is unnecessary (native async); call `reqwest` directly in `async fn`.
- Snapshot restore: `_get_snapshot_restore_candidate` / `_store_direct_snapshot` (JSON store of snapshot IDs per task) — port to a small `~/.operant/modal-snapshots.json`.
- Sandbox lifecycle: `modal.App.lookup("hermes-agent")` → `modal.Sandbox.create(image, mounts, timeout, …)` → `sandbox.exec()` → `sandbox.terminate()`.

**Rust file:** `crates/operant-core/src/environments/modal.rs` — replace the 66-line stub.

### 3.4 Vercel — ⚠️ reference gap

**Critical finding:** `hermes-agent` has **NO Vercel sandbox implementation**. Searches (`tools/environments/vercel*`, repo-wide) return only doc references (`skills/creative/.../vercel.md`). The Rust `vercel.rs` stub (77 LOC) cites "Vercel Rust SDK not yet available" — but there's no Python reference to port from.

**Options (owner to choose before this task starts):**
1. **Implement against Vercel REST API** (the Vercel platform has no first-party "sandbox" product like Modal/Daytona; the closest is Vercel's deployment/execute API, which is not a shell sandbox). This would be net-new work, not a port.
2. **Drop the Vercel variant** — delete `vercel.rs`, remove `EnvironmentType::Vercel`, document that only Modal/Daytona are supported (matching what the reference project actually ships).

**Recommendation:** Option 2. There is no reference contract to honor and no Vercel sandbox product matching the `Environment::execute` shell model. Keeping a speculative variant is the exact pattern this audit targets. *This is the one place the owner instruction "implement in full following hermes-agent" cannot be satisfied literally because hermes-agent does not implement it.*

### 3.5 Wire `EnvironmentType` construction

Once implemented, enable construction paths: whichever dispatcher builds an `Environment` from `EnvironmentType::{Modal, Daytona}` (search `environments/mod.rs` for the match arm that currently has no callers) must read `EnvironmentsSettings` from `AppConfig` and instantiate the real backend. Add an integration test per backend that mocks the REST surface with `mockito` (already a dev-dep).

**Verify per backend:** unit tests for command-wrap/snapshot logic (no network); `mockito`-backed integration tests for create/exec/upload/cleanup; a `#[ignore]` live test gated on env-var presence for real smoke.

---

## Phase 4 — Discord & Slack Adapters (FIX)

**Problem:** `crates/operant-core/src/gateway/platforms/{discord,slack}.rs` (90 LOC each) are `println!`-only stubs. The real adapters used by the gateway live elsewhere (`gateway/mod.rs` has `TelegramAdapter` and references `DiscordAdapter`/`SlackAdapter` per the audit — but those references pointed at stubs). Owner decision: **implement real adapters.**

### 4.1 The contract to satisfy

Current Rust `PlatformAdapter` trait (`gateway/mod.rs:469`):
```rust
pub trait PlatformAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn send_message(&self, message: OutgoingMessage) -> Result<()>;
    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>>;
    fn send_typing(&self, _channel_id: &str) -> Result<()> { /* default */ }
    async fn send_message_return_id(&self, message: OutgoingMessage) -> Result<String> { /* default */ }
    async fn edit_message(&self, ...) -> Result<()> { /* default */ }
    async fn start_with_channel(&self, ...) -> Result<()> { /* default */ }
    async fn send_message_to_channel(&self, ...) -> Result<()> { /* default */ }
    async fn send_voice(&self, ...) -> Result<()> { /* default */ }
    async fn handle(&self, message: IncomingMessage) -> Result<OutgoingMessage> { /* default */ }
}
```

Reference Python `BasePlatformAdapter` (`hermes-agent/gateway/platforms/base.py:2253`) defines capability flags (`supports_code_blocks`, `supports_async_delivery`, `splits_long_messages`, `typed_command_prefix`, `supports_inchannel_continuable`) and the same connect/receive/send lifecycle. Map the capability flags onto the Rust trait as default-associated constants or trait fields.

### 4.2 Discord adapter

**Reference:** `hermes-agent/plugins/platforms/discord/adapter.py` (7,804 LOC) — `DiscordAdapter(BasePlatformAdapter)` using **`discord.py`** (gateway WebSocket, intents, `commands.Bot`).

**Rust approach (no `discord.py` equivalent):**
- Discord exposes a documented HTTPS REST API + gateway WebSocket. Implement with `reqwest` (REST: send message, create thread, react, fetch message) + `tokio-tungstenite` (gateway: IDENTIFY, HEARTBEAT, MESSAGE_CREATE dispatch). This is the standard Rust Discord-bot pattern (what `twilight`, `serenity`, `poise` do internally).
- **Decision point (owner):** vendor a minimal hand-rolled gateway client (~the audit's "native" preference, fewer deps), OR add `twilight-http` + `twilight-gateway` (mature, larger dep tree). Given the audit's "native (dependency doing what the platform does)" tag applies in reverse here (the platform = Discord's protocol), a thin crate is defensible. *Recommend `twilight` to avoid reinventing Discord's gateway resume/heartbeat/ratelimit state machine — that is genuinely hard, not over-engineering.*
- Auth: `DISCORD_BOT_TOKEN` (env or `gateway.discord.bot_token` in `AppConfig::GatewaySettings`). Config keys to add: `bot_token`, `allowed_guilds`, `allowed_channels`, `allow_mentions.{everyone,roles,users,replied_user}` (mirrors Python `_build_allowed_mentions` + `DISCORD_ALLOW_MENTION_*` env).
- Behavior parity targets (MVP): receive MESSAGE_CREATE in allowed channels/DMs → emit `IncomingMessage`; send reply via `POST /channels/{id}/messages`; thread reply via `POST /channels/{id}/messages` with `message_reference`; `send_typing` via `POST /channels/{id}/typing`. Voice (`send_voice`, `VoiceReceiver`) and slash-command sync are out of MVP scope — stub the trait methods with `unimplemented`-free `Ok(())`/errors and track in `TODO.md`.

### 4.3 Slack adapter

**Reference:** `hermes-agent/plugins/platforms/slack/adapter.py` (4,564 LOC) — `SlackAdapter(BasePlatformAdapter)` using **`slack_bolt` Socket Mode** (`AsyncSocketModeHandler`).

**Rust approach:**
- Slack Socket Mode = open a WebSocket to Slack, receive `hello`/eventsEnvelope, post `connections/open` to get the WS URL. Implement with `reqwest` (Web API: `chat.postMessage`, `reactions.add`, `auth.test`) + `tokio-tungstenite` (Socket Mode WS). No mature async Slack Socket Mode crate exists in Rust → hand-rolled thin client is appropriate (this is the "native integration" spirit).
- Auth: `SLACK_BOT_TOKEN` (`xoxb-…`, API calls) + `SLACK_APP_TOKEN` (`xapp-…`, Socket Mode) — mirrors Python `slack/adapter.py:962-963`. Add `SlackSettings { bot_token, app_token, reply_in_thread, reply_broadcast }` to `GatewaySettings`.
- Multi-team support (Python caches `_team_clients`, `_team_bot_user_ids`) — MVP: single-team only; document the limitation.
- Behavior parity targets (MVP): Socket Mode WS connect → receive `message` events → `IncomingMessage`; reply via `chat.postMessage` (threaded if `reply_in_thread`); `send_typing` not supported by Slack (no-op). Block Kit serialization, slash commands, assistant threads → out of MVP scope.

### 4.4 Config & registration

- Add `DiscordSettings`/`SlackSettings` to `AppConfig::GatewaySettings` (Phase 2 unification applies here — add them to the canonical config).
- Register adapters in `Gateway::start` dispatcher (where `TelegramAdapter` is registered) gated on token presence.
- Add `mockito` integration tests for the REST calls; WS handling unit-tested with a fixture frame.

**Verify:** `cargo test -p operant-core --lib gateway`; a `#[ignore]` live smoke (`operant gateway start` with a real token in a test guild/channel) confirmed manually.

---

## Phase 5 — Native CLI Integration Investigation (adapter_types.rs)

**Problem:** `crates/operant-cli/src/tui/adapter_types.rs` (1,902 LOC, 63 types) defines a **parallel type universe** — `Config`, `Settings`, `Message`, `ContentBlock`, `ProviderRegistry`, `AnthropicClient`, `AuthStore`, `ModelRegistry`, etc. — and bridges *into* operant-core via `impl From<operant_core::config::AppConfig> for Config` (line 209) and `impl From<&AppConfig> for Settings`.

This is the **adapter integration** pattern: the TUI was ported from a reference impl (claurst/opencode, per git log "adapter types expanded") and wrapped operant-core's native types in a shim layer. Owner decision: **investigate native integration** — i.e., have the TUI consume operant-core's types directly.

### 5.1 Classification (produce this table first)

For each of the 63 types in `adapter_types.rs`, classify:
- **A. Duplicate of a core type** (e.g. `Config` ≈ `AppConfig`, `Settings` ≈ sub-settings) → delete the adapter type; update TUI call sites to use the core type. (This is the bulk reduction.)
- **B. TUI-presentation-only** (e.g. `StyleInfo`, `Theme`, `KeyBinding`, `KeybindingResolver`, dialog state enums) → legitimate TUI concern; **move out of `adapter_types.rs`** into focused `tui/` modules (theme, keybindings). Not "adapter" types at all.
- **C. Reference-impl speculative** (e.g. `AnthropicClient`, `ProviderRegistry`, `ImportPaths`, `ImportSelection` if import flow isn't wired) → if no live call site, delete; if wired, decide native home.

Preliminary signal: `AnthropicClient` (line 1137) and `ProviderRegistry` (1131) duplicate provider routing that already lives in `operant-core/src/client.rs`/`provider.rs` → likely class A.

### 5.2 Migration mechanics

1. Build the classification table (5.1) — every type mapped to A/B/C with a call-site count (`grep` per type name across `tui/`).
2. **Class A:** for each, find TUI consumers, retype them to the core equivalent, delete the adapter type + its `From` impl. Do one cluster per commit (messages, then config, then providers) so each step compiles.
3. **Class B:** move presentation types into `tui/theme.rs`, `tui/keybindings.rs` (new files); update imports.
4. **Class C:** delete dead, relocate the rest.

### 5.3 Decision gate

This phase is the largest and riskiest. **Do not start until Phases 1–4 land and the baseline is green.** If the classification shows >60% class A (likely), the native-integration refactor is worth it; if most types are class B (legitimately TUI-owned), then `adapter_types.rs` is misnamed but not bloated, and the action shrinks to "rename + split by concern" rather than "delete the layer."

**Verify:** per-cluster `cargo check -p operant-cli` + manual TUI smoke (`operant chat` opens, renders messages, model picker works, settings screen loads). No behavior change is acceptable.

---

## Phase 6 — Documentation & Hygiene

Triggered by behavior/config changes (per `AGENTS.md`):
- [ ] Update `operant.example.toml` for new `EnvironmentsSettings`, `DiscordSettings`, `SlackSettings`, unified config schema.
- [ ] Update `README.md` quickstart if config file location/format changed.
- [ ] Update `CHANGELOG.md` under an unreleased section.
- [ ] Update `TODO.md` ledger for any deferred MVP scope (Discord voice, Slack Block Kit).
- [ ] Update `AGENTS.md` only if autonomous/gateway behavior changed.

---

## Execution Order & Dependencies

```
P0 (baseline) ──┬─► P1 (deletions: 1.1-1.5)     [independent, batch]
                │
                └─► P2 (unified config)          [blocks P3/P4 config additions]
                       │
                       ├─► P3 (envs: daytona, modal; vercel decision)
                       │
                       ├─► P4 (discord, slack)
                       │
                       └─► P5 (adapter_types native)  [largest; last]
                              │
                              └─► P1.6 (dialog consolidation, if still warranted)
                                       │
                                       └─► P6 (docs)
```

Phases 3 & 4 both add config structs → must follow Phase 2. Phase 5 should follow 2–4 so the type system has settled.

## Verification Commands (run after every phase)

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Per `AGENTS.md`: if any command fails, the change is not complete.

---

## Open Questions for Owner (resolve before the named phase starts)

1. **Vercel (Phase 3.4):** drop the variant (recommended) or attempt net-new REST integration with no reference contract?
2. **Discord (Phase 4.2):** `twilight` crate (recommended) or hand-rolled gateway client?
3. **Slack multi-team (Phase 4.3):** single-team MVP acceptable, or block on multi-team parity with the Python adapter?
4. **adapter_types.rs (Phase 5):** proceed with the native-integration refactor at all, or stop at classification if the split is mostly class B?

---

## Net Expected Outcome

| Area | Change |
|---|---|
| Lines removed (Phase 1) | ~−230 LOC + 4 runtime artifacts + ~90 KB scratch md |
| Build deps removed | `cc` (build-dep), espeak link |
| Config systems | 2 → 1 (CLI follows `AppConfig`) |
| Environment stubs | 2→3 real (daytona, modal), 1 dropped (vercel) |
| Platform stubs | 2 fixed (discord, slack) against the real `PlatformAdapter` trait |
| `adapter_types.rs` | −1,900 LOC if class-A dominant; else split-by-concern |

The repo ships only what it runs; every abstraction has ≥2 implementations or is deleted; the CLI and core speak one config language.

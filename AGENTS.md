# AGENTS.md

## Operant Project Context

- Current release line: `0.1.3`
- Runtime config is TOML-first and shared through `crates/operant-core/src/config.rs`
- Rich CLI/TUI uses `ratatui` and lives under `crates/operant-cli/src/tui/`
- Autonomous coding mode lives in `crates/operant-cli/src/autonomous.rs` and is launched through `operant autonomous` or `operant run --autonomous`
- Repo-root `TODO.md` is the task ledger for autonomous mode; keep `Implemented` and `Pending` accurate when autonomous behavior changes
- Autonomous runtime writes repo-local `autonomous-status.toml` state and reloads repeated-failure pause state across restarts; keep that workflow documented when changing autonomous behavior
- The workspace view has `Conversation`, `Reasoning`, `Activity`, and management panels for `MCP`, `Skills`, and `Behavior`
- When config fields change, update `operant.example.toml` in the repo root in the same change
- When user-facing behavior changes, update `README.md`, `CHANGELOG.md`, and screenshots in `assets/` if the UI changed materially
- Tagged releases are created from `CHANGELOG.md`: push `vX.Y.Z`, then GitHub Actions builds artifacts and publishes the GitHub Release from the matching changelog section
- Preferred verification commands:
  - `cargo fmt --all`
  - `cargo check --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## Design Preferences (DO NOT CHANGE)

These are the user's intentional design choices. Do not replace, remove, or
"improve" them. New AI agents working on this repo must respect these defaults:

### Voice / TTS
- **Default: Kokoro** (`tools/tts_tool.rs`)
- Kokoro engine is loaded lazily via `kokoro-tiny::TtsEngine`
- Default voice: `af_sky`
- Config: `config.tts.provider = "kokoro"` in `operant.example.toml`
- Do NOT switch to Edge TTS, OpenAI TTS, or any other provider as the default

### Memory
- **Default: TDG (Teleological Developmental Graph)** via `tdg-rust` path dependency
- TDG is the ONLY real memory backend (`memory_provider.rs`)
- All other providers (Hindsight, RetainDb, Mem0, LocalVector) were removed in iter-30
- `BuiltinProvider` (file-backed MEMORY.md/USER.md) is the zero-dependency fallback
- TDG integration is deep: HybridRetriever + EntityExtractor + auto_wire_edges
- Agent loop auto-calls `sync_turn(user, assistant)` after each turn (iter-33)
- Graph self-organizes — no manual `tdg_create`/`tdg_connect` needed
- Config: `config.memory.provider = "tdg"`

### Browser
- **Default: Obscura/Camofox** (`browser_provider.rs`)
- 5 backends available: lightpanda, camofox, browserbase, browser-use, firecrawl
- Camofox is the preferred default for local automation
- Do NOT switch to a different browser backend as the default

### Platform Adapters (Gateway)
- **Supported: 7 platforms** — telegram, discord, slack, whatsapp, email_smtp, sms_twilio, webhooks
- **Working adapters**: Telegram, Discord, Slack (fully implemented in `gateway/mod.rs`)
- **Stub adapter**: Webhook (needs HTTP server implementation)
- **Config-only**: WhatsApp, Email, SMS (setup wizard + config flags exist, adapter code TBD)
- 20 phantom platforms were purged in iter-50 (matrix, mattermost, signal, etc.)
- Do NOT re-add purged platforms without a real adapter implementation

### Native Tool Integrations
- **AFT (Agent File Tools)**: 15 IDE-grade coding tools via subprocess (`aft_bridge.rs` + `aft_tools.rs`)
  - Auto-downloads from GitHub releases, auto-updates
  - When `aft_enabled=true`, basic file/terminal tools are auto-disabled (no duplication)
  - Feature flag: `aft_enabled` in config
- **IGS (Intelligence Gathering System)**: 14 OSINT/research tools (`igs_tools.rs`)
  - Feature flag: `igs` cargo feature + `igs_enabled` in config
- **LifeOS**: 22 Notion-backed holonic life-management tools (`lifeos_tools.rs`)
  - Feature flag: `lifeos` cargo feature + `lifeos_enabled` in config
  - Requires `NOTION_API_TOKEN` env var

### Context Management
- **Tiered eviction + decay curve** ported from `cortexkit/magic-context` (`context_management.rs`)
- T3 (tool results) evicted first, T2 (reasoning) second, T1 (user/assistant) last
- Recency reserve scales with context window: `budget/4096` clamped to [6, 50]
- Prompt-cache stability: system prompt split into frozen prefix (base + skills) + volatile suffix (memory + workspace)

## Architecture Overview

```
operant/
├── crates/
│   ├── operant-core/          # Core library (no CLI/TUI)
│   │   ├── src/
│   │   │   ├── agent/         # Agent loop, model clients, fallback
│   │   │   │   ├── mod.rs     # OperantAgent — run(), execute_tools(), process_stream()
│   │   │   │   ├── clients/   # OpenAI, Anthropic adapters
│   │   │   │   └── fallback.rs # FallbackModelClient
│   │   │   ├── tools/         # Tool registry + all tool implementations
│   │   │   │   ├── builtin.rs # register_builtin_tools()
│   │   │   │   ├── tdg_tools.rs   # 4 TDG graph memory tools
│   │   │   │   ├── aft_tools.rs   # 15 AFT IDE tools
│   │   │   │   ├── igs_tools.rs   # 14 IGS OSINT tools (feature-gated)
│   │   │   │   └── lifeos_tools.rs # 22 LifeOS tools (feature-gated)
│   │   │   ├── memory_provider.rs # TDG + BuiltinProvider
│   │   │   ├── context_management.rs # Tiered eviction + decay curve
│   │   │   ├── aft_bridge.rs  # AFT subprocess + auto-update
│   │   │   ├── gateway/       # Platform adapters (Telegram/Discord/Slack/Webhook)
│   │   │   ├── mcp.rs         # MCP client (HTTP + Stdio)
│   │   │   └── config.rs      # AppConfig, BehaviorSettings, ToolSettings
│   │   └── Cargo.toml         # Features: anthropic, igs, lifeos
│   └── operant-cli/           # CLI + TUI
│       ├── src/
│       │   ├── main.rs        # CLI entry point, Clap enum, agent setup
│       │   ├── tui/           # 50+ TUI modules (app, render, prompt_input, dialogs)
│       │   ├── gateway_runner.rs # Gateway mode agent handler
│       │   ├── autonomous.rs  # Autonomous coding mode
│       │   ├── config.rs      # CLI config (CliConfig, 3728→2626 LOC after cleanup)
│       │   └── cmd_*.rs       # CLI subcommand handlers
│       └── Cargo.toml         # Features: igs, lifeos
├── Cargo.toml                 # Workspace deps (tdg-rust, igs-rust, lifeos-core)
├── operant.example.toml       # Config template (7 platforms only)
└── AGENTS.md                  # This file
```

## Path Dependencies

Operant depends on three path dependencies that must be cloned alongside it:
- `../../tdg-rust` — TDG graph memory (clone from `ishan-parihar/tdg-rust`)
- `../../igs-rust` — Intelligence Gathering System (clone from `ishan-parihar/igs-rust`)
- `../../lifeos-ops/lifeos-core` — LifeOS Notion tools (clone from `ishan-parihar/lifeos-ops`)

## Build Environment Setup

```bash
# 1. Source the dev env (sets LIBCLANG_PATH, ORT_LIB_LOCATION, etc.)
source /home/z/my-project/operant/scripts/dev-env.sh

# 2. Use the check.sh wrapper (applies RUSTFLAGS + CARGO_INCREMENTAL=0)
/home/z/my-project/operant/scripts/check.sh check -p operant-core --lib
/home/z/my-project/operant/scripts/check.sh test -p operant-core --lib -- <test_filter>

# 3. Or set env manually:
source /home/z/.cargo/env
export LIBCLANG_PATH=/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu
export ORT_LIB_LOCATION=/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export ORT_PREFER_DYNAMIC_LINK=1
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"
export PKG_CONFIG_PATH=/home/z/my-project/local/pkgconfig
export LD_LIBRARY_PATH=/home/z/my-project/local/lib:/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export RUSTFLAGS="-L native=/home/z/my-project/local/lib"
export CARGO_INCREMENTAL=0

# 4. For fresh environments, run provision first:
/home/z/my-project/operant/scripts/provision-build-deps.sh
```

**Compile strategy**: Only compile the specific crate being modified. Use
`-p operant-core --lib` or `-p operant-cli --bin operant` — never
`--workspace` unless doing a final verification. Full workspace compiles
take 10+ minutes on a 2-CPU box.

**Disk constraint**: `target/debug/` can hit 5-8GB. Clean between iterations:
```bash
rm -rf target/debug/deps target/debug/build target/debug/incremental
```

## Known Gaps (from hermes-agent contrast audit)

These are acknowledged gaps vs the hermes-agent reference implementation.
They are NOT bugs — they are features not yet implemented:

1. **No hook system** — `gateway_pipeline.rs` is 38 lines, just a MessageFilter. Hermes has `gateway/hooks.py` with events: `gateway:startup`, `session:start/end/reset`, `agent:start/step/end`, `command:*` (wildcard matching)
2. **Error recovery: 3 classes vs 22** — no context_overflow/billing/content_policy detection. `fallback.rs` only handles Network/RateLimited/5xx. No `should_compress` / `should_rotate_credential` / `should_fallback` classification
3. **MCP: no sampling/elicitation handlers** — stdio + HTTP work, but `sampling/createMessage` and `elicitation/create` server-initiated requests are unhandled. Hermes supports both + SSE transport + dynamic tool discovery (`notifications/tools/list_changed`)
4. **No platform registry** — adapters hardcoded in `build_adapters()` if/elif chain. Hermes has `PlatformRegistry` with deferred loading, `adapter_factory`, `check_fn`, `validate_config`, per-plugin YAML config translation
5. **No credential rotation** — no OAuth refresh / multi-account rotation. Hermes has `agent/credential_pool.py` (2,372 lines) with per-turn tally of consecutive same-entry refreshes
6. **No context compression on overflow** — no automatic summarization when context exceeds window. Hermes has `agent/context_compressor.py` (3,018 lines) + `agent/conversation_compression.py` (1,194 lines)
7. **7 more dead TUI dialogs** — ask_user_dialog, bypass_permissions_dialog, custom_provider_dialog, device_auth_dialog, free_mode_dialog, import_config_dialog, key_input_dialog — all have 0 non-test `show()` calls. (export_dialog and stats_dialog ARE used.)
8. **No `/steer` directive** — no real-time user steering injected during API calls. Hermes drains pending steer directives into the last tool-role message

### Gaps CLOSED in recent iterations:
- ✅ ~~Sequential tool execution~~ — concurrent 8-worker pool (iter-56)
- ✅ ~~7 dead TUI dialogs~~ — elicitation/onboarding/file_injection/invalid_config/memory_update_notification/overage_upsell/desktop_upsell_startup deleted (iter-58)
- ✅ ~~Iteration budget grace call~~ — summarize instead of hard-stop (iter-57)
- ✅ ~~WebhookAdapter stub~~ — real HTTP server with HMAC (iter-54)
- ✅ ~~WhatsApp/Email/SMS adapters~~ — real implementations (iter-54)
- ✅ ~~Anthropic cache_control~~ — breakpoints on system prompt (iter-54)
- ✅ ~~Gateway clear_history every message~~ — session caching fix (iter-49)

## What IS Working (verified functional)

### CLI (39 subcommands — all have real handlers)
run, chat, autonomous, tools, test, config, sessions, mcp, skills, model, completion,
cron, kanban, gateway (16 sub-actions), checkpoints, memory, profile, auth/login/logout,
version, doctor, status, dump, logs, backup, import, uninstall, update, insights,
webhook, debug, plugins, curator, setup, acp, dashboard, trajectory

### Gateway (7 platform adapters — all have real code)
- Telegram: fully implemented (long-poll + Bot API)
- Discord: fully implemented (Gateway WS + REST API)
- Slack: fully implemented (Socket Mode + Web API)
- WhatsApp: implemented (Cloud API outbound + webhook inbound)
- Email: implemented (SMTP outbound + webhook inbound)
- SMS: implemented (Twilio API outbound + Twilio webhook inbound)
- Webhooks: implemented (axum HTTP server + HMAC validation)

### TUI
- Entry point works (TuiApp::enter → ratatui + crossterm)
- Permission dialog system works (iter-20: real prompts instead of auto-approve)
- TUI bridge routes events (bridge.rs: 111 lines)
- Prompt-cache stability: frozen prefix + volatile suffix (iter-39)
- Session caching: clear_history only on session change (iter-49)
- Anthropic cache_control breakpoints (iter-54)

### Web Dashboard
- Dashboard server: axum + TcpListener::bind + axum::serve (dashboard_server.rs)
- Routes: /api/status, /api/boards, /api/health, /api/config, /assets/:filename, /
- Static assets: fonts, JS, CSS served from crates/operant-cli/src/dashboard/

### Memory (TDG — deeply integrated)
- HybridRetriever (FTS5 + trust + recency + embedding scoring)
- EntityExtractor (auto-extracts entities from conversation text)
- auto_wire_edges (auto-connects entities via grammar contracts)
- sync_turn hooks (agent loop auto-calls after each turn)
- BuiltinProvider fallback (file-backed MEMORY.md/USER.md)

### Context Management
- Tiered eviction (T3→T2→T1 oldest-first, iter-37/38)
- Decay curve (H = H50·2^((I-50)/D)/max(p,0.10), iter-38)
- Recency reserve scales with budget (budget/4096 clamped to [6,50], iter-43)
- FTS5 special character escaping (iter-46)

### Native Tool Integrations
- AFT: 15 IDE-grade tools (subprocess + auto-update, iter-40/41)
- IGS: 14 OSINT tools (feature-gated, iter-47)
- LifeOS: 22 Notion tools (feature-gated, iter-47/48)
- AFT dedup: basic file/terminal tools auto-disabled when AFT enabled (iter-51)

## Iteration History (recent)

- iter-54: WebhookAdapter HTTP server + WhatsApp/Email/SMS adapters + Anthropic cache_control
- iter-53: AGENTS.md rewrite + gateway status shows all 7 platforms
- iter-52: Clean operant.example.toml + remove phantom token fields
- iter-51: Wire igs/lifeos registration + AFT tool dedup
- iter-50: Purged 20 phantom platforms — keep only 7 supported
- iter-49: Fixed budget_config regression + yuanbao phantom + gateway session caching
- iter-48: LifeOS API alignment — 22 tools compile clean
- iter-47: Integrated igs-rust + lifeos-ops as native tool modules
- iter-46: FTS5 escaping + aft timeout + decay token-consistency
- iter-45: Deleted 9 more dead operant-core modules (~4.9k LOC)
- iter-44: Fixed test counts + memory flush after iter-24/31/42 changes
- iter-43: Bug fixes + remove rand dep
- iter-42: Deleted 17 dead operant-core modules (~12.6k LOC)
- iter-41: Wired 15 aft tools as OperantTool impls
- iter-40: AFT subprocess bridge with auto-update
- iter-39: Prompt-cache stability — frozen prefix + volatile suffix
- iter-37/38: Ported magic-context tiered eviction + decay curve
- iter-33: TDG hooks — auto-sync_turn in agent loop
- iter-32: Deepened TDG — HybridRetriever + EntityExtractor + auto_wire_edges
- iter-31: Unified TDG pool, gated tools on provider, fixed FTS5 + edge fields
- iter-30: Removed Hindsight/RetainDb/Mem0/LocalVector — TDG-only memory

## MVP Deployment (Current Focus)

**Goal**: Make operant-rs deployable and functional for testing
**Config Defaults**: TDG memory provider, Kokoro TTS (set in operant.example.toml)
**Web Dashboard**: Copied from operant-agent, wired to axum backend

### Quick Start for AI Agents

```bash
# 1. Build the project
cargo build --release

# 2. Run tests
cargo test --workspace

# 3. Test CLI
./target/release/operant chat
./target/release/operant run --query "Hello"
./target/release/operant dashboard

# 4. Test specific module
cargo test -p operant-core --lib database
cargo test -p operant-core --lib agent
```

### Common Development Tasks

**Fix a bug:**
1. Read the error message carefully
2. Find the relevant file in `crates/operant-core/src/` or `crates/operant-cli/src/`
3. Make the fix
4. Run `cargo check --workspace` to verify compilation
5. Run `cargo test --workspace` to verify tests pass
6. Commit with descriptive message

**Add a feature:**
1. Check if similar feature exists in `operant-agent/`
2. Read the Python implementation for reference
3. Implement in Rust following existing patterns
4. Add tests
5. Update documentation

**Port from Python:**
1. Find the Python file in `operant-agent/`
2. Read and understand the implementation
3. Create Rust equivalent in `operant-rs/crates/operant-core/src/`
4. Follow existing Rust patterns (async/await, error handling)
5. Add tests

Behavioral guidelines to reduce common LLM coding mistakes. Merge with project-specific instructions as needed.

**Tradeoff:** These guidelines bias toward caution over speed. For trivial tasks, use judgment.

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

---

## Recursive AI Development Mechanism

**Purpose**: Enable AI agents to fix bugs and improve operant-rs without human bottleneck

### How It Works

1. **AI agent reads this file** (AGENTS.md) to understand the project
2. **AI agent finds a bug** (from BUGS.md, user report, or test failure)
3. **AI agent fixes the bug** following the guidelines above
4. **AI agent runs tests** to verify the fix
5. **AI agent commits the change** with descriptive message
6. **Repeat** until all bugs are fixed

### Issue Tracking

**File**: `operant-rs/BUGS.md`

**Format:**
```markdown
# Open Issues

## Critical (Blocks Deployment)
- [ ] Issue 1: [Description]
- [ ] Issue 2: [Description]

## High (Affects Functionality)
- [ ] Issue 3: [Description]

## Medium (Enhancement)
- [ ] Issue 5: [Description]

## Low (Nice to Have)
- [ ] Issue 6: [Description]
```

### Self-Test Script

**File**: `operant-rs/scripts/self-test.sh`

**Usage:**
```bash
# Run all tests
./scripts/self-test.sh

# Run specific test
cargo test -p operant-core --lib database
```

### Development Loop

```bash
# AI agent development loop
while true; do
  # 1. Pull latest changes
  git pull
  
  # 2. Run tests
  cargo test --workspace
  
  # 3. If tests pass, build
  cargo build --release
  
  # 4. If build succeeds, test manually
  operant run --query "Test query" --max-iterations 1
  
  # 5. If all pass, commit
  git add .
  git commit -m "Fix: [description]"
  git push
  
  # 6. Wait for next issue
  sleep 60
done
```

### Verification Commands

Before claiming any fix is complete, run:
```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

If any command fails, the fix is not complete.

### Configuration

**Default config file**: `operant-rs/operant.example.toml`
**User config file**: `~/.operant/operant.toml`

To set defaults:
1. Copy `operant.example.toml` to `~/.operant/operant.toml`
2. Edit `~/.operant/operant.toml` to set your preferences
3. Set API keys in `~/.operant/.env` (not in config file)

Example config defaults:
```toml
[memory]
provider = "tdg"  # Default memory provider

[tts]
provider = "kokoro"  # Default TTS provider

[agent]
model = "gpt-4"  # Default model
```

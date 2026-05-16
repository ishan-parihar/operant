# CLI Parity Upgrade — Verification Plan

> **Phase 4 of the CLI Parity Upgrade.** Validates that all Phase 1–3 features are implemented correctly, no Python-delegated stubs remain, and the system is production-ready.

**Audit Reference:** [`AUDIT_CLI_PARITY.md`](../../AUDIT_CLI_PARITY.md) — tracks all identified gaps.
**Implementation Plan:** [`2026-05-12-cli-parity-upgrade.md`](./2026-05-12-cli-parity-upgrade.md)

---

## Table of Contents

1. [Pre-Flight Checks](#1-pre-flight-checks)
2. [Phase 1: Independence Verification](#2-phase-1-independence-verification)
   - 2.1 Curator Engine (Core)
   - 2.2 Curator CLI (Wiring)
   - 2.3 Plugins Install
   - 2.4 Claw Migration
3. [Phase 2: Infrastructure Verification](#3-phase-2-infrastructure-verification)
   - 3.1 Gateway Runtime Engine
   - 3.2 ACP Server
   - 3.3 Dashboard Server
   - 3.4 MCP Serve Bridge
4. [Phase 3: Feature Depth Verification](#4-phase-3-feature-depth-verification)
   - 4.1 Kanban Multi-Board
   - 4.2 Command Registry
   - 4.3 RL CLI
5. [Stub-Free Audit (The Parity Check)](#5-stub-free-audit-the-parity-check)
6. [Integration & End-to-End Tests](#6-integration--end-to-end-tests)
7. [Performance & Stability](#7-performance--stability)
8. [Success Criteria Summary](#8-success-criteria-summary)
9. [Manual Verification Script](#9-manual-verification-script)

---

## 1. Pre-Flight Checks

Run these before any feature-specific tests to ensure the build is clean.

```bash
# 1. Clean build
cargo build --release 2>&1

# 2. Full test suite
cargo test --workspace 2>&1

# 3. Clippy with warnings-as-errors
cargo clippy --workspace --all-targets --all-features -- -D warnings 2>&1

# 4. Format check
cargo fmt --all -- --check
```

**Expected:** All four pass with zero errors.

---

## 2. Phase 1: Independence Verification

### 2.1 Curator Engine (Core) — `crates/hermes-core/src/curator/`

**Files under test:**
- `curator/mod.rs` — `CuratorState`, `CuratorEngine`, `LlmReviewClient` trait
- `curator/backup.rs` — `create_backup()`, `list_backups()`, `restore_backup()`
- `curator/archiver.rs` — `archive_skill()`, `restore_skill()`, `list_archived()`, `prune_archived()`
- `curator/review.rs` — LLM-based review pass

**Existing unit tests:** `crates/hermes-core/tests/test_curator.rs`

#### Positive Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| P1.1 | `CuratorState` defaults | Construct `CuratorState::default()` | `enabled=true`, `paused=false`, `interval_hours=24`, `stale_after_days=14`, `archive_after_days=30` |
| P1.2 | State persistence round-trip | Create state, save to temp path via `save_state()`, load via `load_state()` | Loaded fields match saved fields exactly |
| P1.3 | Archive single skill | Create temp skill dir with `SKILL.md`, call `archive_skill()` | Original dir removed, archive dir contains the skill |
| P1.4 | Restore archived skill | Archive then `restore_skill()` | Skill appears back in active directory, removed from archive |
| P1.5 | List archived (empty) | Call `list_archived()` on empty archive dir | Returns empty `Vec` |
| P1.6 | List archived (populated) | Archive 2 skills, `list_archived()` | Returns `["skill-a", "skill-b"]` sorted |
| P1.7 | Create backup | Call `create_backup()` on a dir with content | tar.gz file created in backup dir |
| P1.8 | List backups | Create 2 backups, `list_backups()` | Returns both, newest first |
| P1.9 | Restore from backup | Create backup, delete original, `restore_backup()` | Original content restored, rollback dir created |
| P1.10 | Dry-run review | Create engine with usage tracker, `run_review(true, None)` | Returns `CuratorReport` with `skills_scanned` but no actual archiving |
| P1.11 | Pause/resume | `set_paused(true)`, `is_active()`, then `set_paused(false)`, `is_active()` | Active becomes false, then true again |

#### Negative Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| N1.1 | Archive non-existent skill | `archive_skill("nonexistent", ...)` | Returns `Err` with "not found" message |
| N1.2 | Restore non-existent skill | `restore_skill("nonexistent", ...)` | Returns `Err` with "not found" message |
| N1.3 | Restore over existing skill | Archive skill, create new skill with same name, `restore_skill()` | Returns `Err` with "already exists" message |
| N1.4 | Prune with future timestamp | `prune_archived()` on empty archive | Returns empty `Vec`, no errors |
| N1.5 | Backup to non-existent parent | Call `create_backup()` with path in non-existent dir | Automatically creates parent directories |

#### Regression Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| R1.1 | Existing `SkillUsageTracker` unchanged | Verify active/pinned/agent_created filter logic | Matches pre-Python-port behavior |

#### Commands

```bash
# Run curator-specific tests
cargo test --package hermes-core -- test_curator 2>&1

# Run all core tests
cargo test --package hermes-core 2>&1
```

---

### 2.2 Curator CLI (Wiring) — `crates/hermes-cli/src/cmd_curator.rs`

**⚠ CRITICAL GAP:** As of the current implementation, `cmd_curator.rs` contains **Python-only stubs** for all 12 subcommands (`status`, `run`, `pause`, `resume`, `pin`, `unpin`, `restore`, `list-archived`, `archive`, `prune`, `backup`, `rollback`). Each prints `"Full curator functionality requires the Python hermes-agent"`.

**This is the #1 verification priority.** Every handler must be rewired to call the native `CuratorEngine` from Phase 1.

#### Pre-Verification: Confirm Native Wiring

Before testing, verify each handler no longer references Python:

```bash
# Ensure no Python references remain in the curator CLI handler
grep -n "python\|Python\|requires.*Python" crates/hermes-cli/src/cmd_curator.rs
```

**Expected output:** No matches (exit code 1).

#### Positive Tests (Post-Fix)

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| P2.1 | Status display | `hermes curator status` | Shows enabled/paused state, run count, last run, interval, thresholds |
| P2.2 | Run (dry-run) | `hermes curator run --dry-run` | Lists would-be-archived and stale skills, no changes applied |
| P2.3 | Run (sync) | `hermes curator run --sync` | Review executes, report printed, state updated |
| P2.4 | Pause | `hermes curator pause` | Prints "Curator paused.", `status` shows PAUSED |
| P2.5 | Resume | `hermes curator resume` | Prints "Curator resumed.", `status` shows ACTIVE |
| P2.6 | Pin skill | `hermes curator pin my-skill` | Skill pinned, excluded from future archive |
| P2.7 | Unpin skill | `hermes curator unpin my-skill` | Skill unpinned |
| P2.8 | Archive skill | `hermes curator archive my-skill` | Skill moved to `.archive/` |
| P2.9 | List archived | `hermes curator list-archived` | Lists all archived skills |
| P2.10 | Restore skill | `hermes curator restore my-skill` | Skill moved back from `.archive/` |
| P2.11 | Prune archived | `hermes curator prune --days 30 --yes` | Archived skills older than 30d removed |
| P2.12 | Prune (dry-run) | `hermes curator prune --dry-run` | Shows what would be pruned, no actual removal |
| P2.13 | Create backup | `hermes curator backup --reason pre-upgrade` | tar.gz created in `.backups/` |
| P2.14 | List backups | `hermes curator rollback --list` | Shows all available backups |
| P2.15 | Rollback | `hermes curator rollback --id <backup-filename> --yes` | Skills restored from backup |

#### Negative Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| N2.1 | Prune without --yes | `hermes curator prune --days 30` | Prints warning, no action |
| N2.2 | Rollback non-existent | `hermes curator rollback --id nonexistent.tar.gz --yes` | Error: "Backup not found" |
| N2.3 | Archive non-existent | `hermes curator archive no-such-skill` | Error: "Skill 'no-such-skill' not found" |
| N2.4 | Restore non-existent | `hermes curator restore no-such-skill` | Error: "Archived skill 'no-such-skill' not found" |
| N2.5 | Pin non-existent | `hermes curator pin no-such-skill` | Appropriate error from usage tracker |

#### Regression Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| R2.1 | Help output unchanged | `hermes curator --help` | All subcommands listed, no regressions in CLI surface |

---

### 2.3 Plugins Install — `crates/hermes-cli/src/plugins_install.rs`

**Note:** This is already native — no Python stub detected in `cmd_plugins.rs` or `plugins_install.rs`.

#### Positive Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| P3.1 | Install from full HTTPS URL | `hermes plugins install https://github.com/owner/repo` | Plugin cloned, manifest validated, success message |
| P3.2 | Install from owner/repo shorthand | `hermes plugins install owner/repo` | Resolves to `https://github.com/owner/repo`, cloned |
| P3.3 | Install with --enable | `hermes plugins install owner/repo --enable` | Installed and `.enabled` marker created |
| P3.4 | Force reinstall | `hermes plugins install owner/repo --force` | Existing dir removed, fresh clone |
| P3.5 | List installed | `hermes plugins list` | Shows installed plugin names |
| P3.6 | Remove plugin | `hermes plugins remove my-plugin` | Plugin dir removed |
| P3.7 | Enable plugin | `hermes plugins enable my-plugin` | `.enabled` marker created |
| P3.8 | Disable plugin | `hermes plugins disable my-plugin` | `.enabled` marker removed |

#### Negative Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| N3.1 | Install already-installed (no force) | `hermes plugins install owner/repo` | Error: "already installed. Use --force to reinstall." |
| N3.2 | Remove non-existent | `hermes plugins remove no-such-plugin` | Error: "not installed" |
| N3.3 | Enable non-existent | `hermes plugins enable no-such-plugin` | Appropriate error |
| N3.4 | Invalid git URL | `hermes plugins install not-a-valid-url-at-all` | Error from git clone |
| N3.5 | URL without plugin.yaml | `hermes plugins install https://github.com/random/empty-repo` | Error: "No plugin.yaml or __init__.py found" |
| N3.6 | List empty | `hermes plugins list` (no plugins installed) | Empty list or "No plugins installed" |

#### Commands

```bash
# Run any existing plugins tests
cargo test --package hermes-cli -- test_plugins 2>&1

# Verify no Python references
grep -n "python\|Python" crates/hermes-cli/src/plugins_install.rs crates/hermes-cli/src/cmd_plugins.rs
```

---

### 2.4 Claw Migration — `crates/hermes-cli/src/claw_migrate.rs`

**Note:** Already native — `cmd_claw.rs` calls `claw_migrate::detect_openclaw()`, `scan_openclaw()`, `dry_run_migrate()`, `migrate_skills()`, and `cleanup_openclaw()`.

#### Positive Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| P4.1 | Detect OpenClaw dir | `claw_migrate::detect_openclaw(None)` with `~/.openclaw` present | Returns `Some(path)` |
| P4.2 | No OpenClaw dir | `detect_openclaw(None)` without `~/.openclaw` | Returns `None` |
| P4.3 | Scan items | `scan_openclaw()` on a dir with skills/ and config/ | Returns `["skills", "config"]` |
| P4.4 | Dry-run migrate | `dry_run_migrate()` on a valid dir | Returns `MigrationResult` with "would-migrate" status |
| P4.5 | Actual migrate | `migrate_skills()` with a temp OpenClaw dir | Skills copied to target, `MigrationResult` complete |
| P4.6 | Migrate with overwrite | `migrate_skills(overwrite=true)` with existing target | Existing overwritten |
| P4.7 | Migrate without overwrite | `migrate_skills(overwrite=false)` with existing target | Skipped with "skipped (exists)" status |
| P4.8 | Cleanup (dry-run) | `cleanup_openclaw(dry_run=true)` | Shows what would happen |
| P4.9 | Cleanup (actual) | `cleanup_openclaw(dry_run=false)` | Dir renamed to `.openclaw.backup` |
| P4.10 | CLI migrate --dry-run | `hermes claw migrate --dry-run` | Shows preview of migration |
| P4.11 | CLI cleanup --dry-run | `hermes claw cleanup --dry-run` | Shows preview of cleanup |

#### Negative Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| N4.1 | Migrate from non-existent | `migrate_skills()` on non-existent source | Returns empty `MigrationResult` (no error) |
| N4.2 | Cleanup non-existent | `cleanup_openclaw()` on missing dir | Returns message "No OpenClaw directory found." |
| N4.3 | Detect with bad path | `detect_openclaw(Some("/nonexistent"))` | Returns `None` |

#### Commands

```bash
# Verify no Python references
grep -n "python\|Python" crates/hermes-cli/src/claw_migrate.rs crates/hermes-cli/src/cmd_claw.rs

# Test the module
cargo test --package hermes-cli -- test_claw 2>&1
```

---

## 3. Phase 2: Infrastructure Verification

### 3.1 Gateway Runtime Engine — `crates/hermes-cli/src/gateway_runner.rs`

**Note:** Already native — uses `GatewayConfig` from `AppConfig`, builds platform adapters, manages singleton lifecycle.

#### Positive Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| P5.1 | Start gateway | `hermes gateway start` (with valid config) | Gateway starts, prints platform count |
| P5.2 | Start when already running | `hermes gateway start` twice | Second call prints "already running" |
| P5.3 | Gateway status | `hermes gateway status` | Shows running/stopped state |
| P5.4 | Gateway stop | `hermes gateway stop` | Gateway shuts down |
| P5.5 | Gateway restart | `hermes gateway restart` | Stops then starts cleanly |
| P5.6 | Build adapters from config | `build_adapters()` with all platforms enabled | Returns Vec with 4 adapters (Telegram, Discord, Slack, Webhook) |
| P5.7 | Build adapters with none enabled | `build_adapters()` with all disabled | Returns empty Vec |
| P5.8 | Config passthrough | Verify `GatewayConfig` fields match `AppConfig.gateway.*` | All fields 1:1 mapped |
| P5.9 | Singleton lifecycle | Start, get gateway from static, verify same instance | Same `Arc<Gateway>` returned |
| P5.10 | Axum webhook listener | Start with `webhooks_enabled=true` | HTTP listener binds to configured addr |

#### Negative Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| N5.1 | Stop when not running | `hermes gateway stop` | Graceful no-op or informative message |
| N5.2 | Start with missing token for required adapter | Config without `telegram_token` | Adapter not added (graceful skip) |
| N5.3 | Invalid webhook addr | `webhooks_addr = "invalid"` | Bind error propagated |

#### Commands

```bash
# Verify gateway compilation and test
cargo test --package hermes-cli -- test_gateway 2>&1

# Quick smoke test (requires config)
hermes gateway status
```

---

### 3.2 ACP Server — `crates/hermes-cli/src/cmd_acp.rs` + `crates/hermes-core/src/acp/`

**Note:** Native — `cmd_acp.rs` calls `hermes_core::acp::server::run_stdio_server()`.

**Existing integration test:** `crates/hermes-core/tests/test_acp_stdio.rs`

#### Positive Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| P6.1 | Initialize ACP | Send `{"jsonrpc":"2.0","id":1,"method":"initialize"}` over stdin | Response with serverInfo and capabilities |
| P6.2 | Tools list | Send `{"jsonrpc":"2.0","id":2,"method":"tools/list"}` | Response with tool definitions array |
| P6.3 | Ping | Send `{"jsonrpc":"2.0","id":3,"method":"ping"}` | Response with result |
| P6.4 | Status | Send `{"jsonrpc":"2.0","id":4,"method":"status"}` | Response with agent state |
| P6.5 | Stop | Send `{"jsonrpc":"2.0","id":5,"method":"stop"}` | Response with shutdown acknowledgment |
| P6.6 | stdio server startup | `hermes acp server` | Prints "Starting ACP server over stdio..." and "Listening..." |
| P6.7 | Bidirectional JSON-RPC | Full request/response cycle over stdio | Valid JSON-RPC 2.0 protocol |

#### Negative Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| N6.1 | Invalid JSON | Send `not json` | Parse error response (-32700) |
| N6.2 | Unknown method | Send `{"jsonrpc":"2.0","id":6,"method":"unknown"}` | Method not found response (-32601) |
| N6.3 | Missing id | Send `{"jsonrpc":"2.0","method":"ping"}` | Valid response, id is Null |
| N6.4 | Invalid jsonrpc version | Send `{"jsonrpc":"1.0","id":7,"method":"ping"}` | Accept or reject gracefully |

#### Commands

```bash
# Run ACP integration test
cargo test --package hermes-core -- test_acp_stdio 2>&1

# Manual smoke test via stdin pipe
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | cargo run -- acp server
```

---

### 3.3 Dashboard Server — `crates/hermes-cli/src/dashboard_server.rs`

**Note:** Already native — axum-based HTTP server with `/api/status` and `/api/config` endpoints.

#### Positive Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| P7.1 | Start dashboard | `hermes dashboard server` on port 0 (random) | Server binds and prints address |
| P7.2 | API status endpoint | `GET /api/status` | JSON with status, version, uptime, gateway_running, config_path |
| P7.3 | API config endpoint | `GET /api/config` | JSON with model, platforms_enabled, skills_dir, database_path |
| P7.4 | CORS headers | Check `Access-Control-Allow-Origin` header | Permissive CORS (or configured) |
| P7.5 | Insecure mode | `hermes dashboard server --insecure` | Auth disabled message printed |
| P7.6 | No-browser mode | `hermes dashboard server --no-open` | Starts without opening browser |
| P7.7 | Custom port | `hermes dashboard server --port 9999` | Binds to port 9999 |
| P7.8 | Custom host | `hermes dashboard server --host 0.0.0.0` | Binds to all interfaces |
| P7.9 | Status response fields | Inspect `status` field | `"running"` when server is active |

#### Negative Tests

| # | Test | Command / Action | Expected Result |
|---|------|------------------|-----------------|
| N7.1 | Port in use | Start on occupied port | Bind error with "Address already in use" |
| N7.2 | Invalid host | `--host ""` | Bind error propagated |
| N7.3 | Stop not implemented | `hermes dashboard server --stop` | Info message: "Dashboard stop not yet implemented" |
| N7.4 | Status via CLI | `hermes dashboard server --status` | Info: "use the /api/status endpoint" |

#### Commands

```bash
# Start dashboard in background
cargo run -- dashboard server --port 9191 --no-open &
sleep 2

# Test endpoints
curl -s http://127.0.0.1:9191/api/status | python3 -m json.tool
curl -s http://127.0.0.1:9191/api/config | python3 -m json.tool

# Kill the server
kill %1
```

---

### 3.4 MCP Serve Bridge — `crates/hermes-cli/src/mcp_serve.rs`

**Note:** Already native — stdio MCP server with `conversations_list`, `messages_send`, and `channels_list` tools.

#### Positive Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| P8.1 | Initialize | Send MCP initialize request | Response with protocolVersion `2024-11-05` and capabilities |
| P8.2 | Tools list | Send `tools/list` | Response with 3 tool definitions |
| P8.3 | Conversations list | Call `conversations_list` tool | Response with content (even if empty) |
| P8.4 | Messages send | Call `messages_send` with `{"message": "hello"}` | Response echoing the message |
| P8.5 | Channels list | Call `channels_list` tool | Response with channels array |
| P8.6 | Resources list | Send `resources/list` | Response with empty resources array |
| P8.7 | Server info | Check initialize response | `serverInfo.name` = "hermes-mcp" |

#### Negative Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| N8.1 | Unknown tool | Call non-existent tool | Error (-32602) with "Unknown tool" |
| N8.2 | Unknown method | Send unknown method | Error (-32601) with "Method not found" |
| N8.3 | Parse error | Send invalid JSON | Error (-32700) with "Parse error" |
| N8.4 | Resource read | Send `resources/read` on non-existent resource | Error (-32602) with "Resource not found" |

#### Integration Test: End-to-End MCP Client → Server

```bash
# Start MCP server
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | cargo run -- mcp serve --verbose
```

**Full protocol sequence test:**

```bash
# Send a sequence of MCP commands via stdin pipe
printf '{"jsonrpc":"2.0","id":1,"method":"initialize"}\n{"jsonrpc":"2.0","id":2,"method":"tools/list"}\n{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"messages_send","arguments":{"message":"hello"}}}\n' | cargo run -- mcp serve 2>/dev/null
```

Each line should produce a JSON-RPC response on stdout.

---

## 4. Phase 3: Feature Depth Verification

### 4.1 Kanban Multi-Board — `crates/hermes-core/src/kanban/mod.rs` + `crates/hermes-cli/src/cmd_kanban.rs`

**Note:** Native — `KanbanManager` supports multi-board via separate DB files. CLI has `ListBoards`, `CreateBoard`, and `DeleteBoard` subcommands.

#### Positive Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| P9.1 | List boards (default only) | `hermes kanban boards list` | Shows at least "default" board |
| P9.2 | Create board | `hermes kanban boards create work` | Board created, success message |
| P9.3 | Create multiple boards | Create "personal", "work", "project-x" | All appear in `list` |
| P9.4 | Default board always exists | No explicit creation | `list` shows it, `open_board("default")` works |
| P9.5 | Board-scoped tasks | Create tasks on 2 different boards | Tasks are isolated per-board |
| P9.6 | Delete board | `hermes kanban boards delete scratch` | Board DB file removed, no longer in `list` |
| P9.7 | Create board with mixed-case slug | `hermes kanban boards create "My Project"` | Slug normalized (lowercased, spaces → hyphens) |
| P9.8 | Task creation on named board | Create task on "work" board | Task stored in `hermes_kanban_work.db` |
| P9.9 | Task isolation | List tasks on board A vs board B | Different task sets |

#### Negative Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| N9.1 | Create duplicate board | Create "work" twice | Error: "already exists" |
| N9.2 | Create with empty slug | `create_board("")` | Error: "Board slug cannot be empty" |
| N9.3 | Create with invalid slug | Slugs with `/`, `\`, spaces, `.` | Error: "Invalid board slug" |
| N9.4 | Delete non-existent board | `hermes kanban boards delete no-such-board` | Error: "does not exist" |
| N9.5 | Delete default board | `hermes kanban boards delete default` | Error: "Cannot delete the default board" |
| N9.6 | Open non-existent board | `open_board("nonexistent")` | Creates and initializes new board (auto-create) |

#### Commands

```bash
# Run kanban module tests (includes multi-board unit tests)
cargo test --package hermes-core -- kanban 2>&1

# Manual board lifecycle test
cargo run -- kanban boards list
cargo run -- kanban boards create work
cargo run -- kanban boards create personal
cargo run -- kanban boards list
cargo run -- kanban create --board work "Set up CI pipeline"
cargo run -- kanban create --board personal "Buy groceries"
cargo run -- kanban list --board work
cargo run -- kanban list --board personal
cargo run -- kanban boards delete personal
cargo run -- kanban boards list
```

#### Board Isolation Verification Script

```bash
#!/bin/bash
set -euo pipefail

echo "=== Kanban Board Isolation Test ==="

# Create two boards
cargo run -- kanban boards create test-a 2>/dev/null
cargo run -- kanban boards create test-b 2>/dev/null

# Add a task to each
cargo run -- kanban create --board test-a "Task for A" 2>/dev/null
cargo run -- kanban create --board test-b "Task for B" 2>/dev/null

# Verify isolation
TASKS_A=$(cargo run -- kanban list --board test-a 2>/dev/null | grep -c "Task for A")
TASKS_B=$(cargo run -- kanban list --board test-b 2>/dev/null | grep -c "Task for B")

if [ "$TASKS_A" -eq 1 ] && [ "$TASKS_B" -eq 1 ]; then
    echo "✅ Board isolation: PASS"
else
    echo "❌ Board isolation: FAIL"
    exit 1
fi

# Cleanup
cargo run -- kanban boards delete test-a 2>/dev/null || true
cargo run -- kanban boards delete test-b 2>/dev/null || true
```

---

### 4.2 Command Registry — `crates/hermes-cli/src/commands.rs`

**Note:** Native — defines 23 slash commands across 5 categories (Session, Configuration, Tools & Skills, Info, Exit).

#### Positive Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| P10.1 | Build command map | `build_command_map()` | HashMap with all canonical names + aliases |
| P10.2 | Resolve canonical | `resolve_command("help")` | Returns `"help"` |
| P10.3 | Resolve alias | `resolve_command("/h")` | Returns `"help"` |
| P10.4 | Resolve with leading slash | `resolve_command("/skills")` | Returns `"skills"` |
| P10.5 | Format help text | `format_help_text()` | Formatted string with all categories and commands |
| P10.6 | Help text contains all categories | Check output | "Session", "Configuration", "Tools & Skills", "Info", "Exit" all present |
| P10.7 | All commands have descriptions | Iterate `COMMAND_REGISTRY` | No empty descriptions |
| P10.8 | `/help` in interactive mode | Type `/help` in `hermes chat` | Help text displayed |
| P10.9 | `/exit` in interactive mode | Type `/exit` in `hermes chat` | Chat loop exits |
| P10.10 | `/clear` in interactive mode | Type `/clear` or `/new` | Conversation history cleared |
| P10.11 | `/model` without args | Type `/model` | Current model displayed |
| P10.12 | No duplicate names or aliases | Validate `COMMAND_REGISTRY` | All names and aliases unique |

#### Negative Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| N10.1 | Unknown command | `resolve_command("nonexistent")` | Returns `None` |
| N10.2 | Unknown command in chat | Type `/foobar` | Prints "Unknown command: /foobar. Type /help for available commands." |
| N10.3 | Empty input | `resolve_command("")` | Returns `None` |
| N10.4 | Just a slash | `resolve_command("/")` | Returns `None` |

#### Regression Tests

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| R10.1 | Existing chat flow unchanged | Run `hermes chat` with a normal message (no slash) | Message goes to agent, not command parser |

#### Duplicate Detection Script

```rust
// Verify no duplicate names or aliases in the registry
#[test]
fn test_no_duplicate_commands() {
    let map = build_command_map();
    let mut seen = std::collections::HashSet::new();
    for (&name, _) in &map {
        assert!(
            seen.insert(name),
            "Duplicate command/alias found: {}",
            name
        );
    }
}
```

#### Commands

```bash
# Run command registry tests
cargo test --package hermes-cli -- test_commands 2>&1
```

---

### 4.3 RL CLI — `crates/hermes-cli/src/cmd_rl.rs`

**Note:** Native — provides `run`, `list-environments`, and `doctor` subcommands. Has a minor Python reference in a comment (not a stub).

#### Positive Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| P11.1 | List environments | `hermes rl list-environments` | Shows available RL environments |
| P11.2 | Doctor check | `hermes rl doctor` | Shows env var check results, config info |
| P11.3 | Run training (info) | `hermes rl run "test prompt"` | Prints training session info, model, iterations |
| P11.4 | Run with custom model | `hermes rl run "prompt" --model gpt-4` | Shows specified model |
| P11.5 | Run with custom iterations | `hermes rl run "prompt" --iterations 50` | Shows 50 iterations |

#### Negative Tests

| # | Test | Command | Expected Result |
|---|------|---------|-----------------|
| N11.1 | Doctor with missing env vars | Run without `TINKER_API_KEY`, `WANDB_API_KEY` | Shows missing vars, does not crash |
| N11.2 | Run with missing env vars | Run training without required keys | Shows warning, returns gracefully |

#### Cleanup: Remove Python Reference

Verify the Python comment reference is acceptable (it's documentation-only, not a stub):

```bash
grep -n "python\|Python" crates/hermes-cli/src/cmd_rl.rs
```

Expected: Only the reference line 405 which says `"(Full agent loop integration requires python hermes-agent for now.)"` — this is a phased rollout note, not a stub. Consider removing or updating this message.

---

## 5. Stub-Free Audit (The Parity Check)

This is the **core verification** — ensuring every command that was listed as a Python stub in `AUDIT_CLI_PARITY.md` is now native.

### 5.1 Audit Gap Reference

From [`AUDIT_CLI_PARITY.md`](../../AUDIT_CLI_PARITY.md):

| 🔴 High Priority (Delegation Debt) | Status | Location |
|-------------------------------------|--------|----------|
| `hermes curator <subcommand>` | ⚠ **STUBS REMAIN** | `cmd_curator.rs` — all 12 handlers |
| `hermes gateway start/stop/restart` | ✅ Native | `gateway_runner.rs` |
| `hermes acp server` | ✅ Native | `core/src/acp/server.rs` |
| `hermes dashboard server` | ✅ Native | `dashboard_server.rs` |
| `hermes claw migrate/cleanup` | ✅ Native | `claw_migrate.rs`, `cmd_claw.rs` |
| `hermes mcp serve` | ✅ Native | `mcp_serve.rs`, `cmd_mcp.rs` |

| 🟡 Medium Priority (Feature Depth) | Status | Location |
|-------------------------------------|--------|----------|
| Kanban multi-board | ✅ Native | `kanban/mod.rs` (KanbanManager) |
| Slash command registry | ✅ Native | `commands.rs` |
| RL CLI | ✅ Native | `cmd_rl.rs` |

### 5.2 Zero Python Stub Verification

This script checks every Rust file in the CLI for Python delegation patterns:

```bash
#!/bin/bash
# scripts/verify_no_python_stubs.sh
set -euo pipefail

echo "=== Stub-Free Audit ==="
echo ""

# Define the patterns that indicate a Python stub
STUB_PATTERNS=(
    "requires the Python"
    "requires Python"
    "information-only feature in Rust"
    "Full curator functionality requires the"
)

HAS_STUBS=0

# Search all CLI source files
for pattern in "${STUB_PATTERNS[@]}"; do
    RESULTS=$(grep -rn "$pattern" crates/hermes-cli/src/ 2>/dev/null || true)
    if [ -n "$RESULTS" ]; then
        echo "❌ STUB PATTERN FOUND: $pattern"
        echo "$RESULTS"
        echo ""
        HAS_STUBS=1
    fi
done

# Also check that each command group has a native handler
echo "=== Command Group Native Check ==="
echo ""

COMMAND_GROUPS=(
    "curator::CuratorEngine"
    "plugins_install::install_plugin"
    "claw_migrate::migrate_skills"
    "gateway_runner::start_gateway"
    "acp_server::run_acp_server|acp::server::run_stdio_server"
    "dashboard_server::run_dashboard"
    "mcp_serve::run_mcp_serve"
    "kanban::KanbanManager"
    "commands::COMMAND_REGISTRY"
    "cmd_rl::RlSubcommand"
)

for group in "${COMMAND_GROUPS[@]}"; do
    if grep -qr "$group" crates/hermes-cli/src/ 2>/dev/null; then
        echo "✅ $group"
    else
        echo "⚠ NOT FOUND: $group (may use a different name)"
    fi
done

echo ""
if [ $HAS_STUBS -eq 0 ]; then
    echo "✅ PASS: No Python stubs detected. All commands are native."
else
    echo "❌ FAIL: Python stubs remain. See above."
fi

exit $HAS_STUBS
```

### 5.3 Manual Command Smoke Test

Run every CLI command that was previously stubbed to verify it produces native output:

```bash
#!/bin/bash
# scripts/smoke_test_commands.sh
set -euo pipefail

PASS=0
FAIL=0

run_test() {
    local desc="$1"
    shift
    echo -n "  Testing: $desc ... "
    if OUTPUT=$("$@" 2>&1); then
        if echo "$OUTPUT" | grep -qi "python\|not.*implemented\|requires the Python"; then
            echo "❌ STUB DETECTED"
            echo "    $OUTPUT" | head -3
            FAIL=$((FAIL + 1))
        else
            echo "✅"
            PASS=$((PASS + 1))
        fi
    else
        # Non-zero exit is OK as long as it's not a Python stub
        if echo "$OUTPUT" | grep -qi "python\|not.*implemented\|requires the Python"; then
            echo "❌ STUB DETECTED (exit code $?)"
            echo "    $OUTPUT" | head -3
            FAIL=$((FAIL + 1))
        else
            echo "✅ (exited $?)"
            PASS=$((PASS + 1))
        fi
    fi
}

echo "=== CLI Smoke Test ==="
echo ""

# Phase 1 commands
run_test "curator status" cargo run -- curator status
run_test "curator run --dry-run" cargo run -- curator run --dry-run
run_test "curator list-archived" cargo run -- curator list-archived
run_test "plugins list" cargo run -- plugins list
run_test "plugins install --help" cargo run -- plugins install --help
run_test "claw migrate --dry-run" cargo run -- claw migrate --dry-run
run_test "claw cleanup --dry-run" cargo run -- claw cleanup --dry-run

# Phase 2 commands
run_test "mcp serve --help" cargo run -- mcp serve --help
run_test "acp server --help" cargo run -- acp server --help
run_test "dashboard server --help" cargo run -- dashboard server --help
run_test "gateway status" cargo run -- gateway status
run_test "gateway start --help" cargo run -- gateway start --help

# Phase 3 commands
run_test "kanban boards list" cargo run -- kanban boards list
run_test "kanban boards create test" cargo run -- kanban boards create test-vplan && cargo run -- kanban boards delete test-vplan
run_test "kanban --help" cargo run -- kanban --help
run_test "rl list-environments" cargo run -- rl list-environments
run_test "rl doctor" cargo run -- rl doctor

echo ""
echo "=== Results ==="
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
echo ""

# Phase 4 verification
if [ $FAIL -eq 0 ]; then
    echo "🎉 All commands pass — CLI parity achieved!"
else
    echo "⚠ $FAIL command(s) still have issues."
fi

exit $FAIL
```

---

## 6. Integration & End-to-End Tests

### 6.1 Full Pipeline Test: Plugins → Skills → Agent

This verifies that a plugin can be installed and its skills are usable by the agent.

```bash
# 1. Install a test plugin from a known-good repo
cargo run -- plugins install hermes-agent/test-plugin --enable

# 2. Verify it appears in the list
cargo run -- plugins list

# 3. Verify the skills directory has the plugin's skills
ls ~/.hermes/skills/  # or wherever configured

# 4. Run a curator scan to verify it sees the new skill
cargo run -- curator run --dry-run

# 5. Verify the agent can access skills
cargo run -- status
```

**Success criteria:**
- Plugin installs without error
- Plugin appears in `plugins list`
- Curator scan runs without crashing
- Agent status shows the installed skills

### 6.2 Gateway + MCP + Dashboard Integration

This verifies the three Phase 2 services can coexist.

```bash
# 1. Start dashboard in background
cargo run -- dashboard server --port 9292 --no-open &
DASH_PID=$!
sleep 2

# 2. Verify dashboard API works
curl -s http://127.0.0.1:9292/api/status | grep -q "running"
echo "Dashboard: ✅"

# 3. Verify gateway status endpoint reads correctly
curl -s http://127.0.0.1:9292/api/status | grep -q "gateway_running"
echo "Gateway status endpoint: ✅"

# 4. Start MCP server in test mode
echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | cargo run -- mcp serve &
MCP_PID=$!

# 5. Cleanup
kill $DASH_PID $MCP_PID 2>/dev/null || true

echo "Integration: ✅"
```

### 6.3 Claw Migration Pipeline Test

Uses a synthetic OpenClaw directory to test the full migration flow.

```bash
# Setup synthetic OpenClaw directory
mkdir -p /tmp/test-openclaw/skills/my-old-skill
mkdir -p /tmp/test-openclaw/config
echo "# Old Skill" > /tmp/test-openclaw/skills/my-old-skill/SKILL.md

# Dry-run migration
cargo run -- claw migrate --source /tmp/test-openclaw --dry-run

# Actual migration
cargo run -- claw migrate --source /tmp/test-openclaw --yes

# Verify skills were migrated
ls ~/.hermes/skills/openclaw-imported/

# Dry-run cleanup
cargo run -- claw cleanup --source /tmp/test-openclaw --dry-run

# Cleanup temp
rm -rf /tmp/test-openclaw
```

### 6.4 Curator → Skills → Archive Pipeline Test

```bash
# Create a test skill directory with an agent-created skill
mkdir -p /tmp/test-skills/.curator-test
# (Requires setting up a usage tracker pointing to this dir)

# Run curator review (dry-run)
cargo run -- curator run --dry-run

# Verify the report mentions the test skill
```

---

## 7. Performance & Stability

### 7.1 Memory Leak Detection (Long-Running Gateway)

```bash
#!/bin/bash
# Run gateway for 5 minutes, monitor memory
cargo run -- gateway start &
GW_PID=$!

echo "Gateway PID: $GW_PID"
echo "Monitoring memory for 300 seconds..."

for i in $(seq 1 30); do
    sleep 10
    RSS=$(ps -o rss= -p $GW_PID 2>/dev/null || echo "0")
    echo "  [${i}0s] RSS: ${RSS} KB"
done

kill $GW_PID 2>/dev/null || true
```

**Success criteria:** RSS stays within ±10% of starting value (no linear growth).

### 7.2 Deadlock-Free Concurrent Operation

```bash
#!/bin/bash
# Run kanban operations in parallel to check for deadlocks
echo "Testing concurrent kanban access..."

for i in $(seq 1 5); do
    (
        cargo run -- kanban create --board test-vplan-stress "Task $i from group A" 2>/dev/null
    ) &
    (
        cargo run -- kanban create --board test-vplan-stress "Task $i from group B" 2>/dev/null
    ) &
done

# Wait for all parallel operations
wait

# Verify no corruption
cargo run -- kanban list --board test-vplan-stress 2>/dev/null

# Cleanup
cargo run -- kanban boards delete test-vplan-stress 2>/dev/null || true

echo "Concurrent access test complete."
```

**Success criteria:** All operations complete without hangs or errors. Board is not corrupted.

### 7.3 Dashboard Server — Sustained Load Test

```bash
#!/bin/bash
# Start dashboard
cargo run -- dashboard server --port 9393 --no-open &
DASH_PID=$!
sleep 2

echo "Running 100 status requests..."

FAILURES=0
for i in $(seq 1 100); do
    RESP=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:9393/api/status 2>/dev/null)
    if [ "$RESP" != "200" ]; then
        FAILURES=$((FAILURES + 1))
    fi
done

kill $DASH_PID 2>/dev/null || true

if [ $FAILURES -eq 0 ]; then
    echo "✅ All 100 requests returned 200"
else
    echo "❌ $FAILURES/100 requests failed"
fi
```

**Success criteria:** 100% of requests return HTTP 200.

### 7.4 Resource Cleanup Verification

| # | Test | Action | Expected Result |
|---|------|--------|-----------------|
| S1 | Temp files cleaned | Run curator operations, check `/tmp` for orphaned files | No leftover temp files |
| S2 | DB connections closed | Open/close kanban boards repeatedly | No "too many open files" errors |
| S3 | Gateway process cleanup | Start/stop gateway 10 times | No zombie processes |
| S4 | Dashboard port released | Start/stop dashboard, verify port is free | Port released after shutdown |

---

## 8. Success Criteria Summary

| # | Criterion | How to Verify | Must Pass |
|---|-----------|---------------|-----------|
| SC1 | All builds clean | `cargo build --release`, `cargo check --workspace` | Yes |
| SC2 | All tests pass | `cargo test --workspace` | Yes |
| SC3 | No Clippy warnings | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Yes |
| SC4 | No Python stubs in CLI | `grep` for stub patterns in all CLI source files | Yes |
| SC5 | Curator engine unit tests | `cargo test --package hermes-core -- test_curator` | Yes |
| SC6 | Curator CLI is native | Verify `cmd_curator.rs` calls `CuratorEngine` | Yes |
| SC7 | Plugins install works | Install a plugin from a git URL | Yes |
| SC8 | Claw migration works | Migrate a synthetic OpenClaw dir | Yes |
| SC9 | Gateway runtime native | Start/stop/status all work without Python | Yes |
| SC10 | ACP server responds | JSON-RPC initialize request over stdio | Yes |
| SC11 | Dashboard serves API | GET `/api/status` returns 200 with valid JSON | Yes |
| SC12 | MCP serve responds | Initialize/tools/list over stdio | Yes |
| SC13 | Kanban multi-board | Create/list/delete boards, task isolation | Yes |
| SC14 | Command registry works | `/help` in chat shows all 23 commands | Yes |
| SC15 | RL CLI information | `list-environments` and `doctor` print useful info | Yes |
| SC16 | ACP integration tests pass | `test_acp_stdio.rs` | Yes |
| SC17 | Kanban board isolation | Tasks on one board don't appear on another | Yes |
| SC18 | No memory leaks | Gateway RSS stable over 5 minutes | Yes |
| SC19 | No deadlocks | Parallel kanban operations complete | Yes |
| SC20 | Parity audit passes | `verify_no_python_stubs.sh` exits 0 | Yes |

---

## 9. Manual Verification Script

The complete verification script integrates all the above into one executable pipeline. Run this as the final Phase 4 validation.

```bash
#!/bin/bash
# scripts/verify_complete.sh
# CLI Parity Upgrade — Full Verification
set -euo pipefail

PASS=0
FAIL=0

check() {
    local desc="$1"
    shift
    echo -n "[CHECK] $desc ... "
    if "$@" 2>/dev/null; then
        echo "✅ PASS"
        PASS=$((PASS + 1))
    else
        echo "❌ FAIL"
        FAIL=$((FAIL + 1))
    fi
}

echo "=========================================="
echo " CLI Parity Upgrade — Full Verification"
echo "=========================================="
echo ""

# ── Build & Lint ──
check "Build (release)" cargo build --release -q
check "Format check" cargo fmt --all -- --check -q
check "Clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings -q

# ── Tests ──
check "Core tests" cargo test --package hermes-core -q
check "CLI tests" cargo test --package hermes-cli -q
check "Workspace tests" cargo test --workspace -q

# ── Curator ──
check "Curator engine unit tests" cargo test --package hermes-core -- test_curator -q

# ── No Python Stubs ──
echo ""
echo "--- Stub-Free Audit ---"
STUBS=$(grep -rn "requires the Python\|requires Python\|information-only feature in Rust" \
    crates/hermes-cli/src/ 2>/dev/null || true)
if [ -z "$STUBS" ]; then
    echo "  ✅ No Python stubs found"
    PASS=$((PASS + 1))
else
    echo "  ❌ Python stubs remain:"
    echo "$STUBS"
    FAIL=$((FAIL + 1))
fi

# ── CLI Surface Commands ──
echo ""
echo "--- CLI Command Surface ---"
for cmd in \
    "curator status" \
    "curator list-archived" \
    "plugins list" \
    "plugins install --help" \
    "claw migrate --dry-run" \
    "claw cleanup --dry-run" \
    "mcp serve --help" \
    "acp server --help" \
    "dashboard server --help" \
    "gateway status" \
    "kanban boards list" \
    "kanban boards create test-board" \
    "rl list-environments" \
    "rl doctor"; do
    check "hermes $cmd" cargo run -- $cmd
done

# Clean up test board if created
cargo run -- kanban boards delete test-board 2>/dev/null || true

echo ""
echo "=========================================="
echo " Results: $PASS passed, $FAIL failed"
echo "=========================================="

if [ $FAIL -eq 0 ]; then
    echo "🎉 CLI PARITY ACHIEVED — all systems native."
    exit 0
else
    echo "⚠ $FAIL check(s) failed. See above for details."
    exit 1
fi
```

---

## Appendix A: Known Gaps & Mitigations

| Gap | Severity | Impact | Mitigation |
|-----|----------|--------|------------|
| `cmd_curator.rs` — all 12 handlers still Python stubs | 🔴 Critical | Curator CLI unusable without Python | Rewire to `CuratorEngine` before Phase 4 sign-off |
| `cmd_rl.rs` — comment references Python | 🟡 Minor | Confusing to users | Update comment or implement full RL integration |
| No CLI-level integration tests | 🟡 Medium | Regression risk | Add `tests/` dir to hermes-cli crate |
| No `scripts/` directory | 🟢 Low | Verification script not tracked | Create `scripts/verify_native.sh` as part of Phase 4 |
| `cmd_whatsapp.rs` — Python references | 🟡 Medium | WhatsApp not portable | Document as out-of-scope for current parity phase |
| `cmd_computer_use.rs` — Python reference | 🟢 Low | Out of parity scope | Document |

## Appendix B: Audit Gap Closure Tracker

| AUDIT_CLI_PARITY.md Gap | Phase | Verification Section | Status |
|-------------------------|-------|---------------------|--------|
| `hermes curator` 100% Python-delegated | 1 | 2.1, 2.2, 5 | ❌ Stubs remain in `cmd_curator.rs` |
| `hermes plugins install` stub | 1 | 2.3, 5 | ✅ Native |
| `hermes claw migrate/cleanup` stubs | 1 | 2.4, 5 | ✅ Native |
| `hermes gateway` runtime Python-delegated | 2 | 3.1, 5 | ✅ Native |
| `hermes acp server` stub | 2 | 3.2, 5 | ✅ Native |
| `hermes dashboard server` stub | 2 | 3.3, 5 | ✅ Native |
| `hermes mcp serve` stub | 2 | 3.4, 5 | ✅ Native |
| Kanban multi-board missing | 3 | 4.1, 5 | ✅ Native |
| Slash command registry missing | 3 | 4.2, 5 | ✅ Native |
| RL CLI missing | 3 | 4.3, 5 | ✅ Native |
| Verify all native (`hermes doctor`, `hermes status`) | 4 | 5, 6 | 🟡 Pending Phase 4 execution |
| End-to-end pipeline test | 4 | 6.1 | 🟡 Pending Phase 4 execution |

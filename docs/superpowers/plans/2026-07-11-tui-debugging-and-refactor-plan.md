# TUI Debugging Infrastructure and Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify TUI and core configuration/types, fix stubs, and establish a headless simulator testing framework.

**Architecture:** We will systematically refactor the TUI's adapter types (`adapter_types.rs`) and event handling to use core `AppConfig`, `Message`, and `McpManager` directly. We will add a testing harness for key simulation and remove redundant config files like `settings.json` for model/provider storage.

**Tech Stack:** Rust, Ratatui, Crossterm, Clap, Tokio

## Global Constraints

- **Compiles Cleanly**: Every step must compile without errors on the workspace.
- **Zero Regressions**: Existing tests must remain passing.
- **Iteration Protocol**: Commit and push origin/main after every single iteration.

---

### Task 1: Headless Simulator Assertions

**Files:**
- Modify: `crates/operant-cli/src/cmd_tui_debug.rs`
- Modify: `crates/operant-cli/src/tui/adapter_types.rs`
- Modify: `crates/operant-cli/src/tui/app.rs`

**Interfaces:**
- Consumes: `crossterm::event::KeyEvent`
- Produces: State assertions in the headless simulator

- [ ] **Step 1: Add `--assert` flag to Simulate subcommand**
  Add the `assert` flag argument to `TuiDebugSubcommand::Simulate` variant in `cmd_tui_debug.rs`.
- [ ] **Step 2: Update simulation handler to check assertions**
  Update `debug_simulate` function to evaluate assertions (e.g. comparing overlay visibility states) and exit with code 1 on failure.
- [ ] **Step 3: Run verify check**
  Run: `./scripts/check.sh check -p operant-cli --bin operant`
  Expected: Success
- [ ] **Step 4: Commit Phase 1**
  Run: `git commit -am "feat(iter-219): add state assertion flag to headless simulation"`

---

### Task 2: Config Consolidation

**Files:**
- Modify: `crates/operant-cli/src/tui/adapter_types.rs`
- Modify: `crates/operant-cli/src/tui/app.rs`
- Modify: `crates/operant-cli/src/main.rs`

**Interfaces:**
- Consumes: `operant_core::config::AppConfig`
- Produces: Directly wired configuration loading without double wrappers

- [ ] **Step 1: Delete adapter_types::config::Config**
  Delete `Config` struct and its duplicate fields in `crates/operant-cli/src/tui/adapter_types.rs`.
- [ ] **Step 2: Replace wrapped Config with AppConfig directly**
  Update `TuiApp` fields and implementation in `app.rs` and `main.rs` to refer to `AppConfig` directly instead of the wrapper.
- [ ] **Step 3: Update config fields in settings screens**
  Update the Settings screen render methods to edit settings on `AppConfig` directly.
- [ ] **Step 4: Run build check**
  Run: `./scripts/check.sh check -p operant-cli --bin operant`
  Expected: Success
- [ ] **Step 5: Commit Phase 2**
  Run: `git commit -am "refactor(iter-220): replace duplicate config wrapper with direct AppConfig usage"`

---

### Task 3: settings.json Removal for Model/Provider

**Files:**
- Modify: `crates/operant-cli/src/tui/adapter_types.rs`
- Modify: `crates/operant-cli/src/tui/app.rs`

**Interfaces:**
- Consumes: Persistent settings storage
- Produces: TOML-only configuration for model and provider

- [ ] **Step 1: Remove provider/model fields from settings.json schema**
  Update `adapter_types::config::Settings` to exclude `provider` and `model` fields, leaving only preference-only visual fields (vim, theme).
- [ ] **Step 2: Update setup and connect flows to write directly to operant.toml**
  Update the connection wizard and model picker to write provider and model selections directly to `AppConfig` and save to `operant.toml` TOML file instead of `settings.json`.
- [ ] **Step 3: Run verify check**
  Run: `./scripts/check.sh check -p operant-cli --bin operant`
  Expected: Success
- [ ] **Step 4: Commit Phase 3**
  Run: `git commit -am "feat(iter-221): save provider and model settings exclusively in operant.toml"`

---

### Task 4: Message Type Unification

**Files:**
- Modify: `crates/operant-cli/src/tui/adapter_types.rs`
- Modify: `crates/operant-cli/src/tui/app.rs`
- Modify: `crates/operant-cli/src/tui/messages/mod.rs`

**Interfaces:**
- Consumes: `operant_core::client::Message`
- Produces: Direct usage of core messages in TUI transcript and message list

- [ ] **Step 1: Delete adapter_types::types::Message**
  Delete the duplicate message definitions in `adapter_types.rs`.
- [ ] **Step 2: Update TUI components to use core::Message**
  Replace TUI's internal arrays and rendering routines to construct and use `operant_core::client::Message` and `Role` directly.
- [ ] **Step 3: Run tests to verify correctness**
  Run: `./scripts/check.sh test -p operant-cli --bin operant`
  Expected: Success
- [ ] **Step 4: Commit Phase 4**
  Run: `git commit -am "refactor(iter-222): unify TUI message types with core client message types"`

---

### Task 5: MCP Manager Unification

**Files:**
- Modify: `crates/operant-cli/src/tui/app.rs`
- Modify: `crates/operant-cli/src/tui/mcp_view.rs`
- Modify: `crates/operant-cli/src/tui/adapter_types.rs`

**Interfaces:**
- Consumes: `operant_core::mcp::McpManager`
- Produces: Real MCP tool/server data in the /mcp panel

- [ ] **Step 1: Delete adapter_types::mcp::McpManager stub**
  Remove the fake McpManager structure in `adapter_types.rs`.
- [ ] **Step 2: Connect mcp_view.rs to core_mcp_manager**
  Update `/mcp` panel to read active tools and server statuses directly from `core_mcp_manager` reference in `App`.
- [ ] **Step 3: Run check on workspace**
  Run: `./scripts/check.sh check --workspace`
  Expected: Success
- [ ] **Step 4: Commit Phase 5**
  Run: `git commit -am "feat(iter-223): connect MCP overlay to real core McpManager"`

# AGENTS.md

## Hermes-RS Project Context

- Current release line: `0.1.3`
- Runtime config is TOML-first and shared through `crates/hermes-core/src/config.rs`
- Rich CLI/TUI uses `ratatui` and lives under `crates/hermes-cli/src/tui/`
- Autonomous coding mode lives in `crates/hermes-cli/src/autonomous.rs` and is launched through `hermes autonomous` or `hermes run --autonomous`
- Repo-root `TODO.md` is the task ledger for autonomous mode; keep `Implemented` and `Pending` accurate when autonomous behavior changes
- Autonomous runtime writes repo-local `autonomous-status.toml` state and reloads repeated-failure pause state across restarts; keep that workflow documented when changing autonomous behavior
- The workspace view has `Conversation`, `Reasoning`, `Activity`, and management panels for `MCP`, `Skills`, and `Behavior`
- When config fields change, update `hermes.example.toml` in the repo root in the same change
- When user-facing behavior changes, update `README.md`, `CHANGELOG.md`, and screenshots in `assets/` if the UI changed materially
- Tagged releases are created from `CHANGELOG.md`: push `vX.Y.Z`, then GitHub Actions builds artifacts and publishes the GitHub Release from the matching changelog section
- Preferred verification commands:
  - `cargo fmt --all`
  - `cargo check --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`

## MVP Deployment (Current Focus)

**Goal**: Make hermes-rs deployable and functional for testing
**Config Defaults**: TDG memory provider, Kokoro TTS (set in hermes.example.toml)
**Web Dashboard**: Copied from hermes-agent, wired to axum backend

### Quick Start for AI Agents

```bash
# 1. Build the project
cargo build --release

# 2. Run tests
cargo test --workspace

# 3. Test CLI
./target/release/hermes chat
./target/release/hermes run --query "Hello"
./target/release/hermes dashboard

# 4. Test specific module
cargo test -p hermes-core --lib database
cargo test -p hermes-core --lib agent
```

### Common Development Tasks

**Fix a bug:**
1. Read the error message carefully
2. Find the relevant file in `crates/hermes-core/src/` or `crates/hermes-cli/src/`
3. Make the fix
4. Run `cargo check --workspace` to verify compilation
5. Run `cargo test --workspace` to verify tests pass
6. Commit with descriptive message

**Add a feature:**
1. Check if similar feature exists in `hermes-agent/`
2. Read the Python implementation for reference
3. Implement in Rust following existing patterns
4. Add tests
5. Update documentation

**Port from Python:**
1. Find the Python file in `hermes-agent/`
2. Read and understand the implementation
3. Create Rust equivalent in `hermes-rs/crates/hermes-core/src/`
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

**Purpose**: Enable AI agents to fix bugs and improve hermes-rs without human bottleneck

### How It Works

1. **AI agent reads this file** (AGENTS.md) to understand the project
2. **AI agent finds a bug** (from BUGS.md, user report, or test failure)
3. **AI agent fixes the bug** following the guidelines above
4. **AI agent runs tests** to verify the fix
5. **AI agent commits the change** with descriptive message
6. **Repeat** until all bugs are fixed

### Issue Tracking

**File**: `hermes-rs/BUGS.md`

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

**File**: `hermes-rs/scripts/self-test.sh`

**Usage:**
```bash
# Run all tests
./scripts/self-test.sh

# Run specific test
cargo test -p hermes-core --lib database
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
  hermes run --query "Test query" --max-iterations 1
  
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

**Default config file**: `hermes-rs/hermes.example.toml`
**User config file**: `~/.hermes/hermes.toml`

To set defaults:
1. Copy `hermes.example.toml` to `~/.hermes/hermes.toml`
2. Edit `~/.hermes/hermes.toml` to set your preferences
3. Set API keys in `~/.hermes/.env` (not in config file)

Example config defaults:
```toml
[memory]
provider = "tdg"  # Default memory provider

[tts]
provider = "kokoro"  # Default TTS provider

[agent]
model = "gpt-4"  # Default model
```

# TUI Slash Command Integration Audit

## Executive Summary

The operant TUI has **three disconnected command systems** that need unification:

1. **PROMPT_SLASH_COMMANDS** (58 entries) — autocomplete suggestions in the prompt
2. **intercept_slash_command** (~30 commands) — TUI-side handlers that open UI screens
3. **COMMAND_REGISTRY** (85 entries) — backend command definitions with registered handlers

Only **7 commands** have actual registered handlers in the backend. The rest show "not yet wired" messages or are intercepted by the TUI.

---

## Command System Architecture

### Current Flow
```
User types /command → Enter
  → handle_tui_command(cmd, args)  ← TUI intercept
    → intercept_slash_command(cmd)
      → If handled: opens UI screen, returns true
      → If not handled: returns false
  → command_registry.resolve(cmd)  ← Backend registry
    → If found: command_registry.execute()
      → If handler registered: executes, returns result
      → If no handler: "not yet wired" message
  → Falls through to agent as raw text
```

### The Problem

| System | Count | Handlers | Actual Function |
|--------|-------|----------|-----------------|
| PROMPT_SLASH_COMMANDS | 58 | 0 | Just autocomplete text |
| intercept_slash_command | ~30 | ~28 | Opens UI screens |
| COMMAND_REGISTRY | 85 | 7 | Backend execution |

**Only 7 commands actually work end-to-end:** compact, doctor, init, login, logout, refresh, providers.

---

## Detailed Comparison

### Commands in PROMPT_SLASH_COMMANDS but NOT in COMMAND_REGISTRY (23)

These are shown in autocomplete but have no backend definition:

| Command | TUI Intercept? | Status |
|---------|---------------|--------|
| `advisor` | No | Falls to agent as raw text |
| `agent` | No | Falls to agent |
| `changes` | Yes | Opens diff viewer |
| `compact` | Yes | No-op (returns true, does nothing) |
| `connect` | Yes | Opens connect dialog |
| `context` | Yes | Toggles context viz |
| `cost` | Yes | Opens stats dialog |
| `diff` | Yes | Opens diff viewer |
| `effort` | Yes | Opens effort picker |
| `feedback` | Yes | Opens survey |
| `heapdump` | No | Falls to agent |
| `hooks` | Yes | Opens hooks config |
| `import-config` | Yes | Opens import picker |
| `install-slack-app` | No | Falls to agent |
| `keybindings` | Yes | Opens keybindings file |
| `links` | No | Falls to agent |
| `managed-agents` | No | Falls to agent |
| `output-style` | Yes | Cycles output style |
| `plugin` | No | Falls to agent (registry has `plugins`) |
| `share` | No | Falls to agent |
| `stats` | Yes | Opens stats dialog |
| `theme` | Yes | Opens theme picker |
| `ultrareview` | No | Falls to agent |
| `upgrade` | No | Falls to agent |
| `vim` | Yes | Toggles vim mode |

### Commands in COMMAND_REGISTRY but NOT in PROMPT_SLASH_COMMANDS (48)

These are backend commands that users can't discover:

| Command | Category | Notes |
|---------|----------|-------|
| `start` | Session | gateway_only |
| `new` | Session | aliases: reset |
| `topic` | Session | gateway_only |
| `redraw` | Session | cli_only |
| `history` | Session | cli_only, aliases: h |
| `save` | Session | cli_only, aliases: export |
| `retry` | Session | Retry last message |
| `undo` | Session | Back up N turns |
| `title` | Session | Set session title |
| `handoff` | Session | cli_only |
| `branch` | Session | aliases: fork |
| `compress` | Session | Compress context |
| `rollback` | Session | Restore checkpoints |
| `snapshot` | Session | cli_only, aliases: snap |
| `stop` | Session | Kill background processes |
| `approve` | Session | gateway_only |
| `deny` | Session | gateway_only |
| `background` | Session | aliases: bg, btw |
| `queue` | Session | aliases: q |
| `steer` | Session | Inject message |
| `subgoal` | Session | Manage goal criteria |
| `status` | Session | Show session info |
| `sethome` | Session | gateway_only |
| `sessions` | Session | Browse sessions |
| `provider` | Configuration | Switch provider |
| `env` | Configuration | cli_only |
| `codex-runtime` | Configuration | aliases: codex_runtime |
| `profile` | Info | Show profile |
| `personality` | Configuration | Set personality |
| `statusbar` | Configuration | cli_only, aliases: sb |
| `verbose` | Configuration | cli_only |
| `footer` | Configuration | Toggle footer |
| `yolo` | Configuration | Skip confirmations |
| `reasoning` | Configuration | Toggle reasoning |
| `skin` | Configuration | cli_only |
| `indicator` | Configuration | cli_only |
| `busy` | Configuration | cli_only |
| `tools` | Tools & Skills | cli_only |
| `toolsets` | Tools & Skills | cli_only |
| `skills` | Tools & Skills | cli_only, aliases: skill |
| `bundles` | Tools & Skills | |
| `cron` | Tools & Skills | cli_only |
| `suggestions` | Tools & Skills | aliases: suggest |
| `blueprint` | Tools & Skills | aliases: bp |
| `curator` | Tools & Skills | |
| `kanban` | Tools & Skills | aliases: k |
| `reload` | Tools & Skills | cli_only |
| `reload-mcp` | Tools & Skills | |
| `reload-skills` | Tools & Skills | |
| `browser` | Tools & Skills | cli_only |
| `plugins` | Tools & Skills | cli_only |
| `commands` | Info | gateway_only |
| `restart` | Info | gateway_only |
| `usage` | Info | |
| `credits` | Info | |
| `billing` | Info | |
| `platforms` | Info | cli_only, aliases: gateway |
| `platform` | Info | gateway_only |
| `paste` | Info | cli_only |
| `image` | Info | cli_only |
| `version` | Info | aliases: v |
| `debug` | Info | |
| `whoami` | Info | |
| `gquota` | Info | cli_only |
| `time` | Info | |

### Commands in intercept_slash_command but NOT in PROMPT_SLASH_COMMANDS (3)

These are hidden commands:

| Command | Action |
|---------|--------|
| `search` / `find` | Opens global search overlay |
| `plan` | Toggles plan mode |

### Commands in Python but NOT in Rust (10)

| Python Command | Status in Rust |
|---------------|----------------|
| `billing` | Not implemented |
| `blueprint` | Not implemented |
| `browser` | Not implemented |
| `bundles` | Not implemented |
| `curator` | Not implemented |
| `kanban` | Not implemented |
| `paste` | Not implemented |
| `platforms` | Not implemented |
| `platform` | Not implemented |
| `suggestions` | Not implemented |

---

## Integration Gaps

### Gap 1: Missing Backend Handlers
Most commands in COMMAND_REGISTRY have no registered handlers. Only 7 of 85 commands work:
- `compact`, `doctor`, `init`, `login`, `logout`, `refresh`, `providers`

**Impact:** 78 commands show "not yet wired" messages.

### Gap 2: TUI Intercept vs Backend Disconnect
The TUI intercept handles ~28 commands as UI screens, but the backend doesn't know about them. This creates a split personality:
- TUI opens a dialog for `/model`
- Backend has a `model` command that does something different

### Gap 3: User-Discoverable Commands
Only 58 of 85 backend commands appear in autocomplete. Users can't discover 31 powerful commands like `retry`, `undo`, `rollback`, `compress`, `steer`, `queue`, `background`, `tools`, `skills`, `cron`, `blueprint`, `kanban`.

### Gap 4: Naming Mismatches
- TUI: `plugin` (singular) vs Registry: `plugins` (plural)
- TUI: `settings` vs Registry: `config`
- TUI: `survey` vs Registry: (no equivalent)
- TUI: `upgrade` vs Registry: (no equivalent)

### Gap 5: Missing Python Commands
10 Python commands have no Rust equivalent: billing, blueprint, browser, bundles, curator, kanban, paste, platforms, platform, suggestions.

---

## Refactor Plan

### Phase 1: Unify PROMPT_SLASH_COMMANDS with COMMAND_REGISTRY (1-2 hours)
1. Add missing registry commands to PROMPT_SLASH_COMMANDS
2. Remove commands from PROMPT_SLASH_COMMANDS that aren't in the registry
3. Fix naming mismatches (plugin→plugins, settings→config, survey→feedback)

### Phase 2: Wire TUI Intercepts to Backend (2-3 hours)
1. For each intercepted command, create a backend handler
2. Register all handlers in adapter_types.rs
3. Remove the intercept_slash_command function entirely
4. The outer loop dispatch becomes the single source of truth

### Phase 3: Implement Missing Python Commands (3-4 hours)
1. Port billing, blueprint, browser, bundles, curator, kanban, paste, platforms, suggestions
2. Add to COMMAND_REGISTRY
3. Add handlers

### Phase 4: Clean Up (1 hour)
1. Remove dead code (search/find intercept, plan intercept)
2. Update help overlay to show all commands
3. Test end-to-end

---

## Priority Matrix

| Priority | Issue | Effort | Impact |
|----------|-------|--------|--------|
| P0 | 78 commands show "not yet wired" | High | Users can't use commands |
| P1 | 31 commands invisible in autocomplete | Low | Users can't discover features |
| P2 | TUI intercept vs backend disconnect | Medium | Confusing behavior |
| P3 | Missing Python commands | Medium | Feature parity gap |
| P4 | Naming mismatches | Low | User confusion |

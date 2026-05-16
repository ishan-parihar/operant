# Hermes-RS Setup Wizard: Comprehensive Gap Analysis & Refactoring Plan

**Date**: 2026-05-13  
**Scope**: Full gap analysis of `hermes setup` (Rust) vs `hermes-agent` (Python)  
**Cross-references**:
- `docs/audit/2026-05-13-setup-wizard-audit.md` — Phase 1 audit (provider UX, gateway count, agent settings)
- `docs/audit/2026-05-13-setup-flow-phase2-audit.md` — Phase 2 audit (15 UX gaps, post-setup experience)
- **This doc**: Fresh analysis focusing on page-based UX, post-setup interactive menus, and gaps the prior audits missed

---

## Executive Summary

The Rust setup wizard has **narrowed the feature gap** significantly since the prior audits — gateway platforms went from 2→27, terminal backends exist, provider selection is reasonable. However, the **UX architecture** still fundamentally differs from the Python original in three critical ways:

1. **Page-based vs linear flow**: Python's wizard feels like navigating through distinct pages with clear current-state display. Ours feels like filling out a form.
2. **Post-setup configuration menu**: Python has a **real interactive menu** after setup where users can configure tools per platform, reconfigure MCP, etc. Ours has a placeholder that prints "edit the config file."
3. **Configuration vs questionnaire**: Python's wizard _shows current state and asks "keep or change?"_ at every prompt. Ours still _asks for values_ without consistently displaying current state as defaults.

---

## 1. What Was Fixed Since Previous Audits

| Prior Gap | Status | File |
|-----------|--------|------|
| Gateway platforms (2→27) | ✅ Done | `gateway_platforms.rs` — 27 platforms |
| Terminal backend selection | ✅ Done | `cmd_setup.rs:step_terminal()` — 7 backends |
| Multi-key credential pool | ✅ Done | `cmd_setup.rs:step_fallback_and_strategy()` |
| API key K/R/C pattern | ✅ Done | `cmd_setup.rs:step_api_key()` |
| Config backup before write | ✅ Done | `cmd_setup.rs:persist_config()` — `.bak.timestamp` |
| Post-setup tool summary | ✅ Done | `post_setup.rs:print_tool_summary()` |
| Section dispatch | ✅ Done | `cmd_setup.rs:handle_setup_command()` section matching |
| Current model/provider display | ✅ Done | `cmd_setup.rs:step_provider_and_model()` |
| Config location display | ✅ Done | `post_setup.rs:print_config_location()` |

---

## 2. Still-Gaping: Page-Based UX Architecture (Critical)

### 2.1 The Core Problem

The Python setup wizard feels like navigating **pages**. Each section has:
- A clear section header with visual framing
- Current state shown BEFORE prompts
- "Keep current" as the default option everywhere
- Arrow-key navigable select menus (not just lists)

Our wizard uses **dialoguer** prompts inline with headers, but it lacks the **page boundary feel**.

### 2.2 Specific UX Gaps

| Aspect | Python | Rust | Priority |
|--------|--------|------|----------|
| Section framing | Distinct bordered pages with title box `╔══╗` | Simple `◆ Header` with `──` lines | P1 |
| Prompt consistency | ALL prompts show current as default | Mix of defaults and blanks | P0 |
| "Keep current" in menus | ALWAYS last option, pre-selected | Missing in many select prompts | P0 |
| Fuzzy select for provider | Arrow-key nav through provider list | Works (FuzzySelect) | ✅ |
| Provider status in list | `(●)` active indicator | None | P1 |
| Model fetch status | "Found 14 model(s)" shown clearly | Works but message format differs | P2 |
| Back navigation | Each step can go back | None (must Ctrl+C and restart) | P2 |
| Skip/abort per section | Any prompt can abort the section | Only at provider step via "Leave unchanged" | P1 |

### 2.3 Missing: Provider selection doesn't match Hermes richness

The real Hermes provider list shows (from user's transcript):
```
 (○) Nous Portal (Nous Research subscription)
 (○) OpenRouter (100+ models, pay-per-use)
 (○) LM Studio (local desktop app with built-in model server)
 → (●) OpenCode Go (open models, $10/month subscription)  ← currently active
 (○) custom (direct API)
 (○) Configure auxiliary models...
 (○) Leave unchanged
```

Our wizard shows:
```
Select your LLM provider:
> OpenCode Zen — OpenCode Zen models — portal access
  OpenCode Go — OpenCode Go — $10/month subscription
  Custom endpoint (enter URL manually)
  Leave unchanged
```

**Missing elements**:
- Radio-button style `(○)` / `(●)` indicators
- Active provider marker `← currently active`
- "Configure auxiliary models" as a menu option
- Descriptions in the provider list (we have them but they're flat)

---

## 3. Still-Gaping: Post-Setup Configuration Menu (Critical)

### 3.1 Current State

`post_setup.rs` has a menu loop with 5 options:
```rust
"Configure tools per platform",
"Reconfigure provider & model",
"Configure MCP server tools",
"Open config in editor",
"Done — save and exit",
```

**But options 0-3 all just print messages like "Edit config file directly."** No interactive configuration happens. This is a **facade**.

### 3.2 What The Real Hermes Does

After setup, users get a persistent menu where they can:

1. **Configure tools per platform** — Go through each tool category (CLI, Telegram, Discord, etc.) and toggle/configure API keys per platform
2. **Reconfigure provider & model** — Re-enter the full provider/model/API key flow
3. **Configure MCP server tools** — Add/remove MCP servers, configure transport, probe tools
4. **Open config in editor** — Actually open `$EDITOR` on the config file
5. **Done — save and exit** — Write and exit

### 3.3 Implementation Gap Detail

| Menu Option | Python Behavior | Rust Behavior | Fix Needed |
|-------------|-----------------|---------------|------------|
| Configure tools per platform | Interactive tool-by-tool config with status | `print_info("Edit config file directly.")` | Implement tool configuration sub-wizard |
| Reconfigure provider & model | Calls `step_provider_and_model()` again | `print_info("Run 'hermes setup provider'")` | Actually call the function or chain to the command |
| Configure MCP server tools | Probes MCP servers, allows toggle per tool | `print_info("Edit config file directly.")` | Implement MCP configuration sub-wizard |
| Open config in editor | Opens `$EDITOR` on config file | `print_info("Edit ~/.hermes/config.yaml")` | Actually open `$EDITOR` |
| Done | Saves, prints summary, exits | Breaks loop | ✅ Works |

---

## 4. Missing: Tool Configuration Sub-Wizard

### 4.1 What's Missing

The real Hermes has a full **tool configuration flow** (`tools_config.py` ~1200 lines) that:
1. Lists all tool categories with enable/disable status
2. Per-tool API key config (Tavily, Exa, Fal, etc.)
3. Platform-specific tool permissions (CLI vs Telegram vs Discord)
4. Per-platform tool whitelist/blacklist

Our Rust code has none of this. `cmd_tools.rs` exists but is just a CLI command, not an interactive wizard.

### 4.2 Specific Tool Categories Missing Interactive Config

| Tool Category | Config Needed | Current State |
|---------------|---------------|---------------|
| Web Search | API keys (Tavily, Exa, SearXNG) | Set in hermes.toml manually |
| Vision | Provider/model selection | Set in hermes.toml manually |
| Image Generation | FAL_KEY or equivalent | Not configurable via wizard |
| Browser Automation | Browser binary path | Not configurable |
| Text-to-Speech | Provider + API keys | ✅ Done in step_tts |
| Terminal/Commands | Backend + permissions | ✅ Done in step_terminal |
| MCP | Server configs | Not configurable via wizard |
| RL Training | TINKER_API_KEY | Not configurable |

---

## 5. Missing: Gateway Restart Service Integration

After configuring gateways, the real Hermes:
1. Detects if the gateway service is running
2. Prompts to restart it if it is
3. Prompts to start it if it isn't
4. Offers to install as systemd/launchd service

Our wizard configures the gateway section but **never interacts with the gateway runtime**.

---

## 6. Config File Format Split (Technical Debt)

| Aspect | Python | Rust |
|--------|--------|------|
| Config format | YAML (`config.yaml`) | TOML (`hermes.toml`) |
| CliConfig | Written directly | **Exists** in `config.rs` but **not used by the wizard** |
| .env management | `save_env_value()` / `remove_env_value()` | `env_store.rs` **exists** but wizard barely uses it |
| Secret isolation | API keys in `.env`, config in `.yaml` | API keys in `hermes.toml` (plaintext, in repo!) |

**Problem**: The `config.rs` module has a full YAML-based `CliConfig` (mirroring Python) with >50 structs, but the wizard writes `AppConfig` (TOML) instead. Wizard should write to `CliConfig` (YAML) and use `env_store.rs` for secrets.

---

## 7. Summary of All Remaining Gaps

### P0 (Critical UX — must fix)
| # | Gap | Current | Target | File | Est. |
|---|-----|---------|--------|------|------|
| 1 | "Keep current" as default everywhere | Many prompts lack defaults | ALL prompts show + default | `cmd_setup.rs` | 2h |
| 2 | Provider list lacks radio indicators + status | Flat text | `(○)` / `(●)` with active marker | `cmd_setup.rs` | 2h |
| 3 | Post-setup menu is a facade | Options print "edit config" | Actually interactive | `post_setup.rs` | 6h |

### P1 (Important)
| # | Gap | Current | Target | File | Est. |
|---|-----|---------|--------|------|------|
| 4 | "Configure auxiliary models" in provider list | Only available inline | Menu option in provider select | `cmd_setup.rs` | 2h |
| 5 | Tool configuration sub-wizard post-setup | Not interactive | Per-tool API key/status config | `post_setup.rs` + new | 8h |
| 6 | MCP server config from post-setup | Not interactive | Add/configure MCP servers | `post_setup.rs` + `cmd_mcp.rs` | 4h |
| 7 | Gateway restart prompt | Not present | Detect + prompt restart | `step_gateway()` | 2h |
| 8 | Open config in editor | Just printed | Actually open `$EDITOR` | `post_setup.rs` | 1h |
| 9 | Provider "Back" navigation | None | Back option in menus | `cmd_setup.rs` | 4h |
| 10 | Page-based section framing | Simple headers | Bordered pages per section | `cmd_setup.rs` | 3h |
| 11 | .env secret isolation for API keys | Keys in hermes.toml | Keys in .env via env_store | `cmd_setup.rs` | 4h |

### P2 (Polish)
| # | Gap | Current | Target | File | Est. |
|---|-----|---------|--------|------|------|
| 12 | Provider active indicator | None | `← currently active` | `cmd_setup.rs` | 1h |
| 13 | Launch chat prompt | Not present | After setup complete | `post_setup.rs` | 1h |
| 14 | Post-setup help commands | Not shown | Print available `hermes setup <section>` commands | `post_setup.rs` | 1h |
| 15 | Agent setting alignment | Different variant names | Match Python's exact names (off/new/all/verbose, etc.) | `cmd_setup.rs` + core config | 2h |
| 16 | "Re-run full wizard" post-setup | Not offered | Menu option to re-run | `post_setup.rs` | 1h |

---

## 8. Recommended Implementation Plan

### Phase A — UX Architecture Rewrite (estimated: 1 day)

Fundamentally restructure how the wizard works. The core change: **every prompt shows current state as default, every select has "Keep current" pre-selected, and the flow is re-architected as page-based.**

**Target files**: `cmd_setup.rs`, `prompt_helpers.rs`

**Key changes**:
1. Rewrite `prompt_helpers.rs` to ensure ALL `prompt_text()` calls pass `.default(current_value)`
2. Add "Keep current" as last option to every `prompt_select()` / `prompt_fuzzy_select()` with default index pointing to it
3. Add helper functions for page framing (border, title, section markers)
4. Add `(○)` / `(●)` radio indicators to provider select rendering
5. Add active provider marker

### Phase B — Post-Setup Menu Rewrite (estimated: 2 days)

The post-setup config menu **must actually do things**, not just print messages.

**Target files**: `post_setup.rs`, new `cmd_tools_setup.rs` or similar

**Key changes**:
1. Implement `open_in_editor()` that respects `$EDITOR` / `$VISUAL`
2. Implement tool configuration sub-wizard (iterate tool categories, prompt for API keys)
3. Implement MCP server configuration sub-wizard (add/remove servers, set transport)
4. Chain to `cmd_setup::step_provider_and_model()` for reconfiguration option
5. Add gateway restart detection and prompt
6. Add "Launch chat now?" prompt
7. Print post-setup help commands

### Phase C — Provider UX Enhancement (estimated: 1 day)

**Target files**: `cmd_setup.rs`

**Key changes**:
1. Add "Configure auxiliary models" as a first-class menu option in provider list
2. Add current key status display per provider
3. Implement section abort (go back to previous section)
4. Enhance model discovery to show model count from registry

### Phase D — Config File Migration (estimated: 1 day)

**Target files**: `cmd_setup.rs`, `config.rs`, `env_store.rs`

**Key changes**:
1. Migrate wizard from writing `AppConfig` (TOML) to writing `CliConfig` (YAML)
2. Use `env_store::save_env_value()` for all API key storage
3. Remove API keys from `hermes.toml` output
4. Ensure both config systems stay in sync

---

## 9. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Wizard rewrites break existing configs | Medium | High | Maintain backward compat for hermes.toml format |
| Post-setup menu scope creep | High | Medium | Phase B has clear scope boundary — tool config only |
| YAML config migration breaks CliConfig loading | Low | High | Write integration test: setup → config load → verify |
| `$EDITOR` spawning blocks terminal | Medium | Low | Fork + detach, fall back to print if unset |
| Gateway restart detection too platform-specific | Medium | Low | Start with systemd + launchd only, expand later |

---

## 10. Appendix: Concrete Prompt Transformations

### Current (Rust) → Target (Hermes Python)

**Provider prompt (current)**:
```
◆ Provider & Model
Select your LLM provider, model, and authentication.

  Current model:    deepseek-v4-flash-free
  Active provider:  OpenCode Zen

Select your LLM provider:
> OpenCode Zen — OpenCode Zen models — portal access
  OpenCode Go — OpenCode Go — $10/month subscription
  Custom endpoint (enter URL manually)
  Leave unchanged
```

**Provider prompt (target)**:
```
╔══════════════════════════════════════════════╗
║         Step 1: Provider & Model             ║
╚══════════════════════════════════════════════╝

  Current model:    deepseek-v4-flash-free
  Active provider:  OpenCode Zen  ← currently active

Select your LLM provider:
  (○) Nous Portal (Nous Research subscription)
  (○) OpenRouter (100+ models, pay-per-use)
 →(●) OpenCode Zen (OpenCode Zen models — portal access)  ← currently active
  (○) OpenCode Go (open models, $10/month subscription)
  (○) Custom (direct API)
  (○) Configure auxiliary models...
  (○) Leave unchanged
```

**Post-setup menu (current)**:
```
  ◆ Configuration Menu
Select an option [Done]:
> Configure tools per platform
  Reconfigure provider & model
  Configure MCP server tools
  Open config in editor
  Done — save and exit
```

_(All options except "Done" just print a message)_

**Post-setup menu (target)**:
```
  ◆ What would you like to do next?
    1. Configure tools per platform (5/10 enabled)
    2. Reconfigure provider & model
    3. Configure MCP server tools (3 servers configured)
    4. Open config in editor
    5. Launch Hermes chat
    6. Done — save and exit
```

---

*This audit supersedes specific sections of the prior audits where implementation has advanced. See prior audits for historical context on gaps already resolved.*

# Hermes-RS Setup Wizard: Audit Report & Refactoring Plan

**Date**: 2026-05-13
**Audience**: Engineering
**Scope**: `hermes setup` wizard gap analysis vs `hermes-agent` (Python)

---

## Executive Summary

The Rust setup wizard (`crates/hermes-cli/src/cmd_setup.rs`) currently implements a linear 5-step wizard. The Python version (`hermes_cli/setup.py`) has a richer, modular design with section-specific entry points, per-platform configuration, and significantly deeper provider UX. This audit identifies **10 major gap areas** across setup wizard architecture, UX, provider management, and platform support.

**Estimated effort to match Python**: 2-3 weeks (single developer)

---

## 1. Current State Summary

### What Works (adequate)
- Provider selection (42 providers, but flat names only)
- Model selection (static lists + optional live fetch)
- API key entry + multi-key loop
- Basic TTS step (6 providers, no descriptions)
- Basic Gateway step (Telegram + Discord)
- Terminal display preferences (stream, theme, reasoning)
- Tool settings (iterations, timeout, MCP autoload, rich output)

### Partially Works (needs UI polish)
- Provider descriptions missing (flat names, no `tui_desc`)
- No provider status indicators (active, key status, model count)
- No auxiliary model routing for specialized tasks
- Model discovery is 1-tier (static or live fetch) vs Python's 3-tier
- Gateway lists only 2 platforms instead of 17+

### Missing Entirely
- Terminal backend selection (7 environments)
- Post-setup offer to launch/config more
- Section-specific `hermes setup <section>` commands
- Provider plugin injection system
- Tool progress mode selection
- Context compression settings
- Session reset policy
- Quick vs full mode UX alignment

---

## 2. Detailed Gap Analysis

### 2.1 Provider Selection UX (P0 — Critical UX)

| Aspect | Python | Rust |
|--------|--------|------|
| Display format | `"OpenCode Go (open models, $10/month sub)"` | Just `"OpenCode Go"` |
| Data structure | `ProviderEntry(slug, label, tui_desc)` | `ProviderDef` struct (name, display_name, desc=none) |
| Current provider indicator | `"  ← currently active"` after active provider | None |
| Key status | Shows masked status per provider | Only shows after selection |
| Model count | `"Found 14 model(s) from models.dev registry"` | None |
| Plugin injection | `_inject_builtin_providers()` + plugin system | Static PROVIDERS array only |

**Fix**: Add `description: &'static str` to `ProviderDef`, include in display. Add status indicators.

### 2.2 Model Selection & Discovery (P0 — Critical UX)

| Aspect | Python | Rust |
|--------|--------|------|
| Discovery tiers | 3-tier: models.dev → curated → live API | 2-tier: static → live fetch |
| Display | Provider name + model count, 3 tiers | Static list with optional fetch |
| "Auxiliary" | 9 task slots for specific needs | Not present |
| Active indicator | Shows current model/provider | None |

**Fix**: Implement 3-tier discovery, add auxiliary model concept, show count indicators.

### 2.3 Multi-Key & Auxiliary Models (P1 — Important)

| Aspect | Python | Rust |
|--------|--------|------|
| Slot routing | 9 slots (vision, compression, web_extract, image_gen, embeddings, search, memory, code_execution, reasoning) routed to different providers | Only primary model |
| "Configure auxiliary" | Menu option within provider select | Not present |
| Multi-key | Loop within provider step | Loop within provider step |

**Fix**: Add "Configure auxiliary models" as a first-class option in provider list. Add per-task routing in config.

### 2.4 Terminal Backend Setup (P1 — Important)

| Aspect | Python | Rust |
|--------|--------|------|
| Options | 7 backends: Local, Docker, Modal, SSH, Daytona, Vercel Sandbox, Singularity | None (only display prefs) |
| Per-backend config | SSH keys, Docker image, Modal token, etc. | N/A |
| Backend selection | Interactive list with descriptions | N/A |

**Fix**: Add `terminal_backend` field to AppConfig, implement backend selection with descriptions, add per-backend config prompts.

### 2.5 Gateway Platform Setup (P1 — Important)

| Aspect | Python | Rust |
|--------|--------|------|
| Built-in platforms | 17: Telegram, Discord, Slack, Matrix, Mattermost, WhatsApp, Signal, Email/SMTP, iMessage, Facebook Messenger, WeChat, Line, Viber, Google Business Messages, Twitter/X, Instagram, Webhook | 2: Telegram, Discord |
| Selection UX | Checklist with platform status | Confirm prompts per platform |
| Per-platform setup | Platform-specific `_setup_<platform>()` functions | Generic token prompt only |

**Built-in platforms**: Telegram, Discord, Slack, Matrix, Mattermost, WhatsApp, Signal, Email/SMTP, iMessage, Facebook Messenger, WeChat, Line, Viber, Google Business Messages, Twitter/X, Instagram, Webhook.

**Fix**: Expand gateway list to 17+, add per-platform setup routing, add checklist selection with status.

### 2.6 TTS Configuration (P2 — Nice to have)

| Aspect | Python | Rust |
|--------|--------|------|
| Providers | 9 with descriptions | 6 without descriptions |
| Install flows | NeuTTS/KittenTTS auto-install | None |
| Descriptions | Per-provider `label + tui_desc` | Flat names |

**Fix**: Add descriptions to TTS provider selection, match provider list to Python.

### 2.7 Agent Settings (P2 — Nice to have)

| Aspect | Python | Rust |
|--------|--------|------|
| Max iterations/turns | `max_turns` (default 90) | `max_iterations` (default 20) |
| Tool progress | 4 modes: per-step, final-only, streaming, auto | None |
| Context compression | Configurable | None |
| Session reset | 4 modes: never, on-system-prompt-change, on-tool-change, always | None |
| Defaults | `_apply_default_agent_settings()` | Inline in step_tools |

**Fix**: Add tool_progress, context_compression, session_reset fields. Apply defaults systematically.

### 2.8 Post-Setup Experience (P2 — Nice to have)

| Aspect | Python | Rust |
|--------|--------|------|
| Section-specific cmds | `hermes setup model\|tts\|terminal\|gateway\|tools\|agent` | `hermes setup --quick\|--reconfigure` only |
| Tool summary | Prints available tool count post-setup | None |
| Launch offer | "Ready to start a conversation?" | None |

**Fix**: Add `hermes setup <section>` dispatch, add tool summary, add launch offer.

### 2.9 Quick vs Full Flow Misalignment (P2 — Nice to have)

| Aspect | Python | Rust |
|--------|--------|------|
| Quick mode | Prompts explicitly, applies defaults for skipped sections | `--quick` flag runs steps 1-2 only |
| Default settings | `_apply_default_agent_settings()` applies ToolSettings, GatewaySettings defaults | Not applied for skipped steps |

**Fix**: Ensure quick mode applies sensible defaults for all skipped sections.

### 2.10 Architecture & Code Organization (P3 — Technical debt)

| Aspect | Python | Rust |
|--------|--------|------|
| Section structure | `_setup_<section>()` per function, dispatched by `interactive_setup(section)` | Single linear `run_setup_wizard()` |
| Plugin system | Plugin architecture for providers, gateways | Static arrays only |
| Config formats | YAML-based CliConfig (already exists in Rust but wizard doesn't use it) | Wizard writes TOML AppConfig only |
| Model catalog | `model_catalog.py` with registry, curated lists | `provider.rs` with static lists |

---

## 3. Recommended Implementation Phases

### Phase 1: Foundation (Week 1)

**Focus**: Fix the most visible UX gaps with minimal code changes.

| # | Task | Effort | Impact |
|---|------|--------|--------|
| 1.1 | Add `description` field to ProviderDef (~40 entries) | 2h | High — enables rich provider names |
| 2.1 | Show provider descriptions in FuzzySelect | 1h | High — users see what they're picking |
| 1.3 | Show current provider & key status in setup | 2h | High — context awareness |
| 1.4 | Add "Configure auxiliary models" as option in provider list | 4h | High — matches Python flow |
| 1.5 | Add `tool_progress`, `context_compression`, `session_reset` to AppConfig + wizard | 4h | Medium — agent settings gap |
| 1.6 | Add section dispatch: `hermes setup model\|tts\|terminal\|gateway\|tools\|agent` | 3h | Medium — post-setup UX |

**Phase 1 delivery**: Provider UX parity, auxiliary model support, agent settings, section dispatch.

### Phase 2: Platform Expansion (Week 2)

**Focus**: Terminal backends + Gateway platforms.

| # | Task | Effort | Impact |
|---|------|--------|--------|
| 2.1 | Add TerminalBackend enum + config to AppConfig | 2h | High — missing feature |
| 2.2 | Implement backend selection wizard (7 backends) | 4h | High — core gap |
| 2.3 | Expand gateway list from 2 to 17+ platforms | 3h | High — missing feature |
| 2.4 | Implement checklist-style multi-select for gateways | 3h | Medium — UX match |
| 2.5 | Add per-platform `_setup_<platform>()` routing | 6h | Medium — platform-specific setup |
| 2.6 | Add gateway status indicators (configured/not) | 2h | Medium — UX match |

**Phase 2 delivery**: Terminal backends, full gateway platform coverage.

### Phase 3: Advanced Features (Week 3)

**Focus**: 3-tier model discovery, post-setup, TTS parity.

| # | Task | Effort | Impact |
|---|------|--------|--------|
| 3.1 | Implement models.dev registry lookup | 6h | Medium — 3-tier discovery |
| 3.2 | Add curated model list (3rd tier) | 2h | Medium — model coverage |
| 3.3 | Add auxiliary task slot routing in config | 4h | Medium — advanced routing |
| 3.4 | Add post-setup menu (tool summary, launch offer) | 3h | Low — polish |
| 3.5 | Add TTS provider descriptions + install flows | 2h | Low — TTS parity |
| 3.6 | Migrate wizard to write CliConfig (YAML) instead of AppConfig (TOML) | 8h | Low — tech debt |
| 3.7 | Add plugin injection system for providers/gateways | 8h | Low — architecture |

**Phase 3 delivery**: Advanced model discovery, post-setup UX, TTS parity, architecture cleanup.

---

## 4. Technical Recommendations

### 4.1 ProviderEntry Data Structure

```rust
// Match Python's ProviderEntry pattern
pub struct ProviderEntry {
    pub slug: &'static str,         // "opencode-go"
    pub label: &'static str,        // "OpenCode Go"
    pub tui_desc: &'static str,     // "open models, $10/month subscription"
    // + existing ProviderDef fields
}
```

### 4.2 Auxiliary Model Routing

Add to AppConfig:
```rust
pub struct AuxiliarySettings {
    pub vision: Option<AuxModelConfig>,
    pub compression: Option<AuxModelConfig>,
    pub web_extract: Option<AuxModelConfig>,
    pub image_gen: Option<AuxModelConfig>,
    pub embeddings: Option<AuxModelConfig>,
    pub search: Option<AuxModelConfig>,
    pub memory: Option<AuxModelConfig>,
    pub code_execution: Option<AuxModelConfig>,
    pub reasoning: Option<AuxModelConfig>,
}
```

### 4.3 Section Dispatch

```rust
pub async fn run_setup_wizard(config: &mut AppConfig, section: Option<&str>) {
    match section {
        None => run_full_wizard(config).await,
        Some("model") => step_provider_and_model(config, false).await,
        Some("tts") => step_tts(config, false).await,
        Some("terminal") => step_terminal(config, false).await,
        Some("gateway") => step_gateway(config, false).await,
        Some("tools") => step_tools(config, false).await,
        Some("agent") => step_agent(config, false).await,
        Some(s) => eprintln!("Unknown section: {}", s),
    }
}
```

### 4.4 Gateway Platform Expansion

```rust
pub fn known_gateway_platforms() -> &'static [GatewayPlatform] {
    &[
        GatewayPlatform { key: "telegram", name: "Telegram", icon: "✈" },
        GatewayPlatform { key: "discord", name: "Discord", icon: "" },
        // ... 15 more
    ]
}
```

### 4.5 Model Discovery Architecture

```rust
pub enum ModelDiscovery {
    /// models.dev registry lookup
    Registry(Vec<String>),
    /// Curated list from provider.rs fallback
    Curated(&'static [&'static str]),
    /// Live probe of provider API
    Live(Vec<String>),
}
```

---

## 5. Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Provider description field makes provider.rs too large | Low | Low | Use doc comments, keep descriptions terse |
| Gateway platform setup varies wildly per platform | Medium | Medium | Create PlatformSetup trait with default impl |
| Terminal backends require heavy dependencies (Docker, Modal, etc.) | Medium | High | Keep setup as config-only, no runtime deps |
| Section dispatch breaks existing `--flags` interface | Low | Medium | Maintain backwards compat with existing flags |
| 3-tier model discovery adds latency | Medium | Low | Cache results, timeout aggressively |

---

## Appendix A: Full Platform Lists

### Terminal Backends (7)
1. Local — runs on local machine
2. Docker — runs in Docker container
3. Modal — runs on Modal cloud
4. SSH — runs on remote SSH server
5. Daytona — runs on Daytona
6. Vercel Sandbox — runs on Vercel
7. Singularity — runs on Singularity

### Gateway Platforms (17+)
1. Telegram
2. Discord
3. Slack
4. Matrix
5. Mattermost
6. WhatsApp
7. Signal
8. Email/SMTP
9. iMessage
10. Facebook Messenger
11. WeChat
12. Line
13. Viber
14. Google Business Messages
15. Twitter/X
16. Instagram
17. Webhook

### TTS Providers (9)
1. Edge (free)
2. ElevenLabs (cloud, requires key)
3. OpenAI TTS (cloud)
4. Google Cloud TTS
5. Azure TTS
6. Kokoro (local, free)
7. NeuTTS (local, installable)
8. KittenTTS (local, installable)
9. Piper TTS (local)

---

*Generated from side-by-side testing of hermes-agent (Python) and hermes-rs (Rust).*

# Hermes-RS Setup Wizard: Phase 2 Audit Report

**Date**: 2026-05-13
**Status**: Post-Phase-1 Restructure
**Scope**: Final UX gaps vs Python `hermes-agent` — the "page-based" flow, post-setup menu, .env management, and 15+ missing UX patterns

---

## Executive Summary

Phase 1 restructure narrowed the feature gap but did **not** change the fundamental UX architecture. Python's setup wizard behaves like a **guided configuration tool** — it shows your current state, accepts defaults with Enter, backs up before writing, and offers a post-setup configuration menu. Our Rust wizard is still a **linear questionnaire** — it forces choices, overwrites without backup, and exits without offering next steps.

This audit identifies **15 specific gaps** that make the Rust wizard feel rigid compared to Python's. Only 3 of these are "feature" gaps — the remaining 12 are **UX architecture** issues: how prompts display, how defaults are handled, and what happens after setup.

---

## 1. Critical UX Architecture Gaps

### Gap 1: No "Keep Current" Default Pattern (P0)

| Aspect | Python | Rust (current) |
|--------|--------|----------------|
| Prompt style | Shows current value as default, Enter keeps it | Shows blank field, user must type |
| Empty input | Returns default value silently | Returns empty string, loses value |
| User experience | "Press Enter to keep it" — safe | "Enter new value" — risky |

**Python pattern** (setup.py:197-216):
```python
def prompt(question, default=None, password=False):
    display = f"{question} [{default}]: "  # shows default
    value = input(display)
    return value.strip() or default or ""  # empty = keep default
```

**Python menu pattern** (setup.py:1126-1130):
```choices.append(f"Keep current ({current_label})")```
Last option with default index = keep_current_idx.

**Fix**: Every `dialoguer::Input::new()` must pass the current config value as `.default()`. Every `Select` must have a "Keep current" option at default index. Empty input must preserve the existing value.

---

### Gap 2: API Key [K]eep/[R]eplace/[C]lear UX (P0)

| Aspect | Python | Rust |
|--------|--------|------|
| Existing key shown | `"  OpenCode Go API key: sk-opena... ✓"` | `"currently sk-1234…5678"` in prompt |
| Action prompt | `[K]eep / [R]eplace / [C]lear (default K):` | Blank entry → keeps old; typing → replaces |
| Clear key | Writes empty string, returns `abort=True` | Not possible |
| Keep key | Just press Enter | Must not type anything |

**Python implementation** (main.py:4218-4250):
```python
print(f"  {pconfig.name} API key: {existing_key[:8]}... ✓")
choice = input("  [K]eep / [R]eplace / [C]lear (default K): ").strip().lower()
if choice.startswith("r"):  # prompt for new key
elif choice.startswith("c"):  # clear key, abort
# default: keep
```

**Fix**: After collecting the primary API key, show it masked with `[K]eep/[R]eplace/[C]lear`. R → re-prompt. C → clear and continue. Default → K.

---

### Gap 3: Config Backup Before Write (P0)

| Aspect | Python | Rust |
|--------|--------|------|
| Write strategy | Copies `config.yaml` → `config.yaml.bak.<timestamp>`, then writes | Direct overwrite, no backup |
| Restore hint | Shows `cp {backup} {path}` in success message | None |

**Python implementation** (setup.py:3062-3074):
```python
config_path = get_config_path()
if config_path.exists():
    _backup_path = config_path.with_suffix(f".yaml.bak.{_dt.now().strftime('%Y%m%d_%H%M%S')}")
    shutil.copy2(config_path, _backup_path)
```
Shown after setup:
```python
print_info(f"Previous config backed up to: {_backup_path}")
print_info(f"  cp {_backup_path} {config_path}")
```

**Fix**: In `persist_config()`, create `{file}.bak.{timestamp}` before overwriting.

---

### Gap 4: .env File Management (P1)

| Aspect | Python | Rust |
|--------|--------|------|
| API key storage | `~/.hermes/.env` with atomic write | `hermes.toml` (plain text) |
| Permissions | `_secure_file()` sets restrictive perms | No special handling |
| Key removal | `remove_env_value()` deletes key from .env | Not possible |
| Key query | `get_env_value()` checks os.environ then .env | None |

**Python implementation** (config.py:4363-4431):
```python
def save_env_value(key, value):
    # Validate key name, strip newlines, check non-ASCII
    # Read existing .env, find/replace KEY=line, or append
    # Atomic write via tempfile.mkstemp() + os.replace()
    # Set restrictive file permissions
    # Update os.environ[key] in-process
```

**Fix**: Add `save_secret(key, value)` to cmd_setup.rs that writes `KEY=VALUE` to `.env` with restrictive permissions. The primary config stays as `hermes.toml` for non-secret settings.

---

### Gap 5: Post-Setup Tool Configuration Menu (P1)

| Aspect | Python | Rust |
|--------|--------|------|
| Post-setup menu | Platform menu loop: Configure CLI/global/reconfigure/MCP/Done | None — exits immediately |
| Per-platform toggles | `(8/24 enabled)` status per platform | Generic list, no status |
| Tool reconfig | Pick a configured tool, change provider/keys | Not available |
| MCP tool config | Probes MCP servers, per-server tool toggles | Not available |

**Python implementation** (tools_config.py:2238-2472):
```
Select an option:
  1. Configure CLI  (8/24 enabled)
  2. Configure all platforms (global)
  3. Reconfigure an existing tool's provider or API key
  4. Configure MCP server tools
  5. Done
```

**Fix**: Add a post-setup menu step. For MVP, offer: "Configure tools", "Done". Expand later.

---

### Gap 6: Tool Availability Summary (P1)

| Aspect | Python | Rust |
|--------|--------|------|
| Post-setup output | `"10/11 tool categories available:"` with per-category ✓/✗ | None |
| What it checks | 14 categories: vision, web, browser, TTS, etc. | Nothing |

**Python implementation** (setup.py:360-627):
```
  ✓ Vision (image analysis)
  ✓ Web Search & Extract (tavily)
  ✗ RL Training (missing TINKER_API_KEY)
  ✓ Terminal/Commands
```

**Fix**: After setup, check known tool categories (web, browser, terminal, tts, etc.) against config/env and print availability summary.

---

### Gap 7: Gateway Restart Prompt (P1)

| Aspect | Python | Rust |
|--------|--------|------|
| After gateway config | `"Restart the gateway to pick up changes?"` (default Yes) | None |
| Per-platform restart | systemd / launchd / Windows task | N/A |
| Start if not running | `"Start the gateway service?"` | N/A |
| Install if missing | `"Install as systemd service?"` | N/A |

**Python implementation** (setup.py:2488):
```python
if prompt_yes_no("Restart the gateway to pick up changes?", True):
    if supports_systemd: systemd_restart()
    elif _is_macos: launchd_restart()
    elif _is_windows: gateway_windows.restart()
```

**Fix**: After gateway config, check if gateway is running. If yes, prompt to restart. If no, offer to start.

---

### Gap 8: "Launch Chat Now?" Prompt (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| Post-setup | `"Launch hermes chat now?"` (default Yes) | None |

**Python implementation** (setup.py:3254-3261):
```python
def _offer_launch_chat():
    if not prompt_yes_no("Launch hermes chat now?", True):
        return
    from hermes_cli.relaunch import relaunch
    relaunch(["chat"])
```

**Fix**: After setup complete, prompt "Start a conversation now?" with Yes as default.

---

### Gap 9: Credential Pool Strategy Selection (P1)

| Aspect | Python | Rust |
|--------|--------|------|
| Strategy prompt | 3 labeled choices + description | None |
| Strategies | Fill-first, Round robin, Random | None (just has multi-key) |
| Storage | `config["credential_pool_strategies"]["provider"]` | Not stored |

**Python implementation** (setup.py:880-899):
```python
strategy_labels = [
    "Fill-first / sticky — keep using the first healthy credential until it is exhausted",
    "Round robin — rotate to the next healthy credential after each selection",
    "Random — pick a random healthy credential each time",
]
```

**Fix**: After multi-key collection, if 2+ keys, show strategy selection with descriptions.

---

### Gap 10: Config Location Display (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| Post-setup output | Config file, secrets file, data folder, install dir | None |

**Python** (setup.py:3209-3215):
```
  Config file:  /home/user/.hermes/config.yaml
  Secrets file: /home/user/.hermes/.env
  Data folder:  /home/user/.hermes
  Install dir:  /home/user/.hermes/hermes-agent
```

**Fix**: After setup, print where config was written.

---

### Gap 11: "Current model/Active provider" Display (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| Provider step header | Shows "Current model: deepseek-v4-flash" + "Active provider: OpenCode Go" | Just "Provider set to X" |
| Model count | `"Found 14 model(s) from models.dev registry"` | None |
| Key status | `"OpenCode Go API key: sk-R5OM1... ✓"` | `"API key updated"` |

**Fix**: At start of provider step, print current model + provider + key status.

---

### Gap 12: Gateway Platform Icons (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| Telegram | `📱 Telegram` | `Telegram` |
| Discord | `💬 Discord` | `Discord` |
| Slack | `💼 Slack` | `Slack` |
| Others | Rich emoji per platform | Plain text |

**Fix**: Add emoji/icons to gateway platform display names.

---

### Gap 13: Gateway Platform Count Mismatch (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| Count | 21 platforms | 17 platforms |
| Missing | SMS (Twilio), DingTalk, Feishu/Lark, WeCom, WeCom Callback, QQ Bot, Yuanbao, Google Chat, IRC, Microsoft Teams | — |

**Python platforms**: Telegram, Discord, Slack, Matrix, Mattermost, WhatsApp, Signal, Email/SMTP, SMS (Twilio), DingTalk, Feishu/Lark, WeCom, WeCom Callback, WeChat, BlueBubbles (iMessage), QQ Bot, Yuanbao, Google Chat, IRC, LINE, Microsoft Teams. (Webhooks removed from checklist.)

**Fix**: Expand GatewaySettings to 21 platforms.

---

### Gap 14: Agent Settings Mismatches (P2)

| Setting | Python | Rust |
|---------|--------|------|
| Tool progress | `off`, `new`, `all`, `verbose` | `PerStep`, `FinalOnly`, `Streaming`, `Auto` |
| Context compression | Threshold (0.5-0.95 float) | Boolean on/off |
| Session reset | Inactivity+Dairy, Inactivity only, Daily only, Never | Never, OnSystemPromptChange, OnToolChange, Always |

**Fix**: Align ToolProgressMode variants, change context_compression to float threshold, change SessionResetMode variants.

---

### Gap 15: No Post-Setup Configuration Offer (P2)

| Aspect | Python | Rust |
|--------|--------|------|
| After setup | `hermes setup tools` / `hermes config` / `hermes config edit` / `hermes config set` | None |
| Edit hint | `nano ~/.hermes/config.yaml` | None |

**Python** (setup.py:3265-3295):
```
📝 To edit your configuration:
   hermes setup          Re-run the full wizard
   hermes setup model    Change model/provider
   hermes setup terminal Change terminal backend
   hermes setup gateway  Configure messaging
   hermes setup tools    Configure tool providers
   hermes config         View current settings
   hermes config edit    Open config in your editor
```

**Fix**: Print available commands after setup.

---

## 2. Implementation Effort Estimates

| Gap | Priority | Effort | Type |
|-----|----------|--------|------|
| Gap 1: Keep current defaults | P0 | 2h | UX — prompt wrappers |
| Gap 2: KRC API key | P0 | 2h | UX — API key prompt |
| Gap 3: Config backup | P0 | 1h | Safety — persist_config |
| Gap 4: .env management | P1 | 4h | Architecture — new functions |
| Gap 5: Post-setup tool menu | P1 | 6h | Feature — tools_config equivalent |
| Gap 6: Tool availability summary | P1 | 3h | Feature — status check |
| Gap 7: Gateway restart prompt | P1 | 2h | UX — gateway check |
| Gap 8: Launch chat prompt | P2 | 1h | UX — final prompt |
| Gap 9: Credential pool strategy | P1 | 2h | Feature — strategy enum |
| Gap 10: Config location display | P2 | 0.5h | UX — print paths |
| Gap 11: Current model/provider display | P2 | 1h | UX — show status |
| Gap 12: Gateway platform icons | P2 | 0.5h | Cosmetic |
| Gap 13: Expand gateways 17→21 | P2 | 1h | Config + setup |
| Gap 14: Agent setting variants | P2 | 1h | Config + setup |
| Gap 15: Post-setup help text | P2 | 1h | UX — print help |

**Total**: ~27h (P0=5h, P1=17h, P2=5h)

---

## 3. Recommended Phases

### Phase 2a — Safety & UX Architecture (P0 items, ~5h)
1. Add `.default(current_value)` to ALL dialoguer Input prompts
2. Add "Keep current (X)" as last option with default index to ALL Select/FuzzySelect prompts
3. Implement [K]eep/[R]eplace/[C]lear pattern for API key prompts
4. Add config backup in `persist_config()` before write

### Phase 2b — Post-Setup Experience (P1 items, ~17h)
1. Add `save_secret()` function for `.env` key-value atomic writes
2. Implement post-setup tool configuration menu (tools_command equivalent)
3. Implement tool availability summary check
4. Add gateway restart prompt after gateway config
5. Add credential pool strategy selection after multi-key entry

### Phase 2c — Polish (P2 items, ~5h)
1. Add "Launch chat now?" prompt at end
2. Show config file paths after setup
3. Show current model/provider at start of provider step
4. Add emoji icons to gateway platforms
5. Expand gateway from 17→21 platforms
6. Align agent setting variants with Python
7. Print available post-setup commands

---

## Appendix A: Full Python Flow Reference

### Post-Setup Flow Sequence (Python)
1. Run all setup sections (model → terminal → agent → gateway → tools)
2. `_print_setup_summary()` — tool availability summary
3. Print config location + backup info
4. `_offer_launch_chat()` — "Launch hermes chat now?"
5. Exit

### Post-Setup Menu (Python tools_config.py)
```
Select an option:
  1. Configure CLI  (8/24 enabled)
  2. Configure all platforms (global)
  3. Reconfigure an existing tool's provider or API key
  4. Configure MCP server tools
  5. Done
```

### Gateway Platforms (Python, 21 total)
Telegram, Discord, Slack, Matrix, Mattermost, WhatsApp, Signal, Email/SMTP, SMS (Twilio), DingTalk, Feishu/Lark, WeCom, WeCom Callback, WeChat, BlueBubbles (iMessage), QQ Bot, Yuanbao, Google Chat, IRC, LINE, Microsoft Teams

### Session Reset Modes (Python)
- Inactivity + daily reset (recommended)
- Inactivity only
- Daily only
- Never auto-reset

### Tool Progress Modes (Python)
- `off` — Silent, just the final response
- `new` — Show tool name only when it changes
- `all` — Show every tool call with a short preview
- `verbose` — Full args, results, and debug logs

### Context Compression (Python)
Float threshold (0.5-0.95), summarization triggered when context exceeds threshold.

---

*Generated from side-by-side comparison vs hermes-agent v0.1.3. Implementation in `crates/hermes-cli/src/cmd_setup.rs` (Rust) vs `hermes_cli/setup.py` + `hermes_cli/tools_config.py` (Python).*
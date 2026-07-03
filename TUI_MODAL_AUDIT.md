# TUI Modal/Dialog Audit Report

## Executive Summary

The `/connect` and `/model` modals are **operant-native code** — there is no legacy claurst code in these dialogs. However, there are significant differences from the Python/TypeScript reference implementation that may be causing the user to perceive them as "legacy."

---

## Key Findings

### 1. No `/connect` Command in Python Reference

The Python hermes-agent TUI has **no `/connect` slash command**. The closest equivalent is:
- `hermes setup` — full provider configuration wizard
- `hermes model` — interactive model selection with provider picker

The TypeScript TUI (the actual `hermes --tui`) integrates provider connection directly into the ModelPicker as a 2-step flow:
1. Step 1: Provider selection (with auth status, model count)
2. Step 2: Model selection (with pricing, current model marker)

**The Rust operant TUI has `/connect` as a standalone command** — this is an operant innovation, not legacy.

### 2. Model Picker Differences

| Feature | Python/TS Reference | Rust Operant |
|---------|-------------------|--------------|
| Flow | 2-step: provider → model | Single flat model list |
| Provider grouping | Provider sections in model list | No grouping |
| API key entry | Inline in model picker | Separate dialog |
| Pricing display | In/Out/Cache columns | Not shown |
| Disconnect | Ctrl+D in provider picker | Not visible |
| Effort levels | Not in model picker | Available |
| Current model | Marked with * or "currently in use" | Marked in list |

### 3. Connect Dialog Differences

| Feature | Python/TS Reference | Rust Operant |
|---------|-------------------|--------------|
| Location | Integrated into /model | Standalone /connect command |
| Provider list | Auth status indicators | Categories + badges |
| API key entry | Inline during model selection | Separate key_input_dialog |
| Custom providers | Included in provider list | Separate custom_provider_dialog |

### 4. What's Actually Legacy vs Native

**No claurst legacy code found** in the connect/model dialogs. All "claude" references are:
- Actual Claude model names (claude-3-7, claude-opus-4, etc.)
- Config import from ~/.claude/ (operant feature)
- Billing URL for Anthropic users

**Legacy patterns found elsewhere:**
- `render_legacy_history_search` — old Ctrl+R history search
- `sync_legacy_prompt_fields` — syncing prompt input to older code paths
- `HistoryEntry::legacy()` — entries without timestamps

---

## Identified Issues

### Issue 1: Model Picker Shows Flat List Without Provider Context
The model picker shows all models from all providers in a single flat list. Users may not understand which provider each model belongs to.

**Fix:** Add provider grouping or provider name to each model entry.

### Issue 2: No Inline API Key Entry in Model Picker
The TypeScript reference allows entering API keys directly in the model picker. The Rust version requires a separate `/connect` flow.

**Fix:** Consider integrating API key entry into the model picker for a smoother UX.

### Issue 3: Missing Pricing Information
The TypeScript reference shows In/Out/Cache pricing columns. The Rust version doesn't show pricing.

**Fix:** Add pricing data to model entries from the registry.

### Issue 4: No Disconnect Capability
The TypeScript reference allows disconnecting from a provider (Ctrl+D). The Rust version doesn't have this.

**Fix:** Add disconnect option to the model picker.

### Issue 5: Provider Auth Status Not Shown
The TypeScript reference shows auth status (● current, * active, ○ unauthenticated) in the provider list. The Rust version shows badges (FREE, LOCAL) but not auth status.

**Fix:** Add auth status indicators to provider list.

---

## Recommendations

### Short-term (Quick fixes)
1. Add provider name to model entries in the flat list
2. Show current model at top of list with marker
3. Add auth status indicators to connect dialog

### Medium-term (UX improvements)
1. Add 2-step flow to model picker (provider → model)
2. Add pricing information to model entries
3. Add disconnect capability to model picker

### Long-term (Full parity with reference)
1. Integrate API key entry into model picker
2. Add provider grouping in model list
3. Add persist toggle (session vs global)

---

## Files to Modify

1. `model_picker.rs` — Add provider name, pricing, current model marker
2. `dialog_select.rs` — Add auth status indicators
3. `app.rs` — Update connect dialog selection handler
4. `adapter_types.rs` — Update model registry to include pricing data

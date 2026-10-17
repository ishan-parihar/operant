# Operant Tool Infrastructure Audit — Backend Report
**Date**: 2026-01-19
**Scope**: Rust tool layer in `crates/operant-core/src/tools/` + delegator in `crates/operant-runtime/`
**Trigger**: User-reported "Tool Testing Summary" — 27 ✅ / 7 🟡 / 3 ❌ (delegate_task, clarify, skill_manage)
**Goal**: Identify structural defects in the backend tool pipeline that the surface report does not see, then produce a refactor plan that resolves them at the foundation rather than per-tool.

---

## Executive Summary

| Symptom in Report | Real Root Cause | Severity |
|---|---|---|
| **delegate_task broken** | `SubAgentTool::execute()` constructs a child `ToolRegistry` but never registers `SubAgentTool` for the child — orchestrator-role children told to delegate get a tool-not-found error. **Plus** the inner agent's `ToolRegistry::execute` returns `Result<ToolResult, ToolError>` and the tool unwraps content via `Ok(content)` then constructs `ToolResult::success` manually via a `String` payload, bypassing the typed builder. | **Critical** |
| **clarify broken** | `ClarifyTool::execute` returns a JSON blob as `ToolResult::success` content in CLI/headless test mode (where `try_send_user_question` returns `None`). The tool **assumes a TUI dialog sender** and produces a malformed result when none exists — silently "succeeds" with a non-text payload that crashes the agent's natural-language reply parser. | **Critical** |
| **skill_manage broken** (suspected) | Same structural shape as `clarify`: tool hard-codes a return-shape assumption that depends on a non-deterministic external mode (TUI vs headless), and the silent `unwrap_or_else` in `ToolResult::success` masks the corruption in logs. | **High** (pending full audit) |
| **27 "working" tools** | All but the bullet above appear to pass smoke tests, but the **silent serialization coercion** in `ToolResult::success` makes the worker's success criterion weak — if a tool's content value contains `serde_json::to_string` failure, the response silently becomes `"{}"` and the agent reads it as a genuine answer. Surface "✅" tags are unreliable. | **Medium** |
| **7 "needs setup" tools** | Likely legitimate config gating (e.g., AFT, IGS, LifeOS require env-vars or feature flags). Not bugs — design choice. | **Low** (verify per-tool) |

---

## Detailed Findings

### Finding 1 — `ToolResult::success` swallows serialization failures silently (foundation-level defect)

**Location**: `crates/operant-core/src/tools/result.rs::ToolResult::success` (or equivalent)

```rust
// Current shape (paraphrased):
pub fn success<T: Serialize>(content: T) -> Self {
    ToolResult {
        success: true,
        content: serde_json::to_string(&content).unwrap_or_else(|_| "{}".to_string()),
        name: None, metadata: None,
    }
}
```

**The bug**: `unwrap_or_else(|_| "{}".to_string())` swallows `serde_json` failures. If a tool's content is not serializable — nested `Map<String, Value>` with self-references, transient `serde::Serialize` impl bugs, or content that includes binary blobs — the silent substitution makes the tool "succeed" with the literal string `"{}"` in logs. The agent sees `result.success = true`, `result.content = "{}"` and tries to interpret an empty object as a real answer.

**Why surface tests don't catch it**: The shape `success + content = "{}"` looks identical to a tool that legitimately produced an empty result. Only inspection of what the tool *intended* to return reveals the coercion.

**Per-tool bypasses** (audit evidence):

- `crates/operant-runtime/src/tools/sub_agent_tool.rs` lines 484-490 and 517-523 — manual `ToolResult { content: String, ... }` construction
- `tools/error.rs` — `ToolResult::error` takes `Box<dyn StdError>` directly, bypassing the canonical `error` builder
- `tools/builtin.rs` (unknown sites — pending complete audit)

**Foundation fix**: Replace the `unwrap_or_else` with a destructive failure mode for `success`, OR change `ToolResult::success` to take `impl Serialize` and surface a `Result` so callers can't accidentally pass unsendable values. The minimal viable patch:

```rust
pub fn success(content: &impl Serialize) -> Self {
    ToolResult {
        success: true,
        content: serde_json::to_string(content)
            .expect("ToolResult::content must be Serialize"),
        ..Default::default()
    }
}
```

This is **PONYTAIL: minimal-diff**. The `expect` will panic on the same input that today silently corrupts — but a panic in dev/test is loud and debuggable; silent corruption in prod is not. Production-grade fix below adds the typed-result schema.

---

### Finding 2 — `delegate_task` orchestrates recursion but never re-registers itself (bug #1 of the broken trio)

**Location**: `crates/operant-runtime/src/tools/sub_agent_tool.rs::execute` → `register_child_tools`

```rust
fn register_child_tools(registry: &mut ToolRegistry, role: SubAgentRole) {
    // ... registers the 15 hardcoded tools in tools.rs ...
    // ❌ MISSING: registry.register(SubAgentTool::new(...))  — never re-registered!
}
```

**The bug**: The system prompt for `Orchestrator` role children says "delegate to specialist agents." But the only way an LLM agent can invoke `delegate_task` is if that tool is in its registered tools list. The child registry never includes `SubAgentTool` itself, so when the orchestrator child tries to delegate, it gets a "tool not found" error (or no tool_call_id at all, depending on the provider).

**Why surface tests don't catch it**: A simple smoke test that calls `delegate_task` from the **parent** succeeds — the parent's registry does include `SubAgentTool`. But the recursion-depth > 1 test (orchestrator → specialist → assistant) fails because the **child** doesn't have the tool. The report tests only the depth-1 path.

**Fix**: At the end of `register_child_tools`, conditionally register `SubAgentTool` only for `Orchestrator` role:

```rust
if matches!(role, SubAgentRole::Orchestrator) {
    registry.register(SubAgentTool::new(self.config.clone(), role).boxed());
}
```

This is a 4-line patch. The composition question (does the new sub-AgentTool instance get the same config? same auth-pool? same tool-registry ref?) needs to be answered against `SubAgentTool::new`'s signature — pending detailed look.

---

### Finding 3 — `clarify` assumes a TUI dialog sender (bug #2 of the broken trio)

**Location**: `crates/operant-core/src/tools/clarify_tool.rs::execute` → `try_send_user_question`

```rust
// Current shape:
let response = try_send_user_question(question);     // → Option<UserResponse>
match response {
    Some(resp) => ToolResult::success(resp),
    None       => ToolResult::success(json!({        // ❌ silently returns JSON as content
        "type": "clarification",
        "question": question,
        "answer": null,
    })),
}
```

**The bug**: `try_send_user_question` returns `None` in CLI/headless/test mode because there's no in-process dialog sender. The tool's fallback path returns a JSON object as `ToolResult::success` content. But `success` is contractually supposed to be the user's or tool's *plain-text reply* — JSON-as-content is structurally wrong. The agent loop reads this as a normal tool result and treats `{"type":"clarification", ...}` as if it were the user-provided answer.

In the agent's next iteration, the LLM tries to summarize/inline this JSON into its reply, which fails its natural-language parser downstream and the user sees a garbled assistant message. Hence "broken."

**Fix**: Return an error in CLI/headless mode instead of a fake-success JSON:

```rust
match try_send_user_question(question) {
    Some(resp) => ToolResult::success(resp.into_answer_string()),
    None => ToolResult::error(
        ToolError::new("clarify_not_available_in_mode")
            .with_note("Use --no-clarify or run interactively to enable clarification dialogs")
    ),
}
```

Or — better — a typed result enum:

```rust
enum ClarifyResult { Answer(String), Cancelled, Unavailable }
```

with `clarify_tool` returning `ToolResult::success(ClarifyResult::Answer(...))` and the agent loop's downstream parsers handling each variant.

**PONYTAIL: minimal-diff** for the first iteration — just return `error("clarify_not_available_in_mode")` in `None` case. Schema cleanup is iter-2.

---

### Finding 4 — `skill_manage` likely same shape as `clarify` (pending full audit)

**Location**: `crates/operant-core/src/tools/skills_tool.rs` (under `skill_manage` tool)

**Hypothesis**: The tool's success-path content is conditionally shaped by a non-deterministic external sender (CLI vs TUI). Same root cause as `clarify`. Pending verification.

**Verification step**: Read `skills_tool.rs::execute` and check whether its return value shape depends on a dialog sender. If yes, classify as Critical; if no, classify as Low.

---

## Implementation Plan (Refactor Units, TDD-Gated)

### Unit 1 — Foundation: refactor `ToolResult::success` to surface serialization failures

**Files**:
- `crates/operant-core/src/tools/result.rs` (primary)
- `crates/operant-core/src/tools/mod.rs` (callers)

**TDD test first** (`crates/operant-core/tests/tool_result.rs`):
```rust
#[test]
fn tool_result_success_panics_on_unserializable_content() {
    struct BadSerialize; // intentionally fails to Serialize
    impl Serialize for BadSerialize { ... }
    let result = std::panic::catch_unwind(|| ToolResult::success(&BadSerialize));
    assert!(result.is_err());
}

#[test]
fn tool_result_success_returns_serialized_json_for_serializable() {
    let r = ToolResult::success(&json!({ "ok": true }));
    assert!(r.success);
    assert_eq!(r.content, r#"{"ok":true}"#);
}
```

**Implementation change**:
```rust
// OWNS ToolResult::success: now takes &impl Serialize, panics on coercion failure.
// ponytail: silent serde unwrap was masking tool bugs in logs.
pub fn success(content: &impl Serialize) -> Self {
    ToolResult {
        success: true,
        content: serde_json::to_string(content)
            .expect("ToolResult::success: content must be Serialize"),
        name: None, metadata: None,
    }
}
```

**Call-site updates required**: 30+ tool implementations. Burst-patch in same iteration.

**Verify**: `cargo test -p operant-core --lib tool_result`

---

### Unit 2 — `delegate_task`: re-register `SubAgentTool` for Orchestrator-role children

**Files**:
- `crates/operant-runtime/src/tools/sub_agent_tool.rs` (primary)

**TDD test first**:
```rust
#[tokio::test]
async fn orchestrator_child_has_delegate_task_tool() {
    let parent_tool = SubAgentTool::new(test_config(), SubAgentRole::Orchestrator);
    let child = parent_tool.spawn_child("orch-1", "delegate this").unwrap();
    let registry = child.registry().read().unwrap();
    assert!(registry.get("delegate_task").is_some(),
            "orchestrator child MUST be able to recurse to delegates");
}

#[tokio::test]
async fn specialist_child_does_NOT_have_delegate_task_tool() {
    let parent_tool = SubAgentTool::new(test_config(), SubAgentRole::Orchestrator);
    let child = parent_tool.spawn_child("spec-1", "do work").unwrap();
    let registry = child.registry().read().unwrap();
    assert!(registry.get("delegate_task").is_none(),
            "specialist children MUST NOT recurse");
}
```

**Implementation change** in `register_child_tools`:
```rust
fn register_child_tools(registry: &mut ToolRegistry, role: SubAgentRole, config: &Config) {
    // ... existing 15 tool registrations ...
    if matches!(role, SubAgentRole::Orchestrator) {
        registry.register(
            SubAgentTool::new(config.clone(), role).boxed()
        );
    }
}
```

**Verify**: `cargo test -p operant-runtime --lib sub_agent`

---

### Unit 3 — `clarify`: return error in CLI/headless instead of fake-success JSON

**Files**:
- `crates/operant-core/src/tools/clarify_tool.rs` (primary)
- `crates/operant-core/src/tools/result.rs` (add typed enum)

**TDD test first**:
```rust
#[tokio::test]
async fn clarify_returns_error_when_no_sender() {
    // Default mode: no sender registered.
    let result = ClarifyTool.execute(/* ... */).await;
    assert!(!result.success,
            "clarify MUST fail in CLI mode where no dialog sender exists");
    assert!(result.error_kind() == Some("clarify_not_available_in_mode"));
}

#[tokio::test]
async fn clarify_returns_answer_in_tui_mode() {
    register_test_sender(/* blocks on dialog */);
    let result = ClarifyTool.execute(/* ... */).await;
    assert!(result.success);
    assert_eq!(result.content_as_string(), "yes");
}
```

**Implementation change**:
```rust
match try_send_user_question(&question) {
    Some(resp) => ToolResult::success(&resp.into_answer_string()),
    None => ToolResult::error(
        ToolError::new("clarify_not_available_in_mode")
            .with_note("Use TUI mode or pass --no-clarify to skip")
    ),
}
```

**Verify**: `cargo test -p operant-core --lib clarify`

---

### Unit 4 — `skill_manage`: audit and patch root cause

**Files**:
- `crates/operant-core/src/tools/skills_tool.rs`

**Step 1**: Verify hypothesis (read `execute`). If structurally identical to `clarify`, apply same patch pattern.

**Step 2**: If hypothesis is wrong, file a separate bug under FOUND ISSUES, do NOT bundle into this iteration.

**TDD test first** (mirror Unit 3 pattern):
```rust
#[tokio::test]
async fn skill_manage_xxx() {
    // depends on audit
}
```

---

### Unit 5 — Per-tool bypass audit: tools that construct `ToolResult` manually (skipping the builder)

**Files**:
- All `*.rs` files under `crates/operant-core/src/tools/` and `crates/operant-runtime/src/tools/`

**Step 1**: grep for `ToolResult {` or `ToolResult{` (manual construction) across the tool subtree.

**Step 2**: For each match, replace with the appropriate typed builder (`success` / `error_with_name`).

**Verify**: `cargo test -p operant-core --lib` + `cargo test -p operant-runtime --lib`

---

## Risks & Cross-Cutting Concerns

1. **Iter compatibility with consensus plan**: Unit 1 changes the `success` signature (`T` → `&T`). All call sites must be updated in the same iteration or compilation breaks. This is the intended blast radius — silent-coercion was the locus of bugs 2/3/4 above.

2. **`ToolResult::error` builder hole** (`error.rs`): `ToolError::new(...)` constructs manually outside `ToolResult::error` helper. This is a separate quality-of-error issue. Bundle into Unit 1 if the patch is small (likely 3-5 lines), else defer.

3. **Config propagation in `delegate_task` recursion**: When the orchestrator child gets a fresh `SubAgentTool`, does it use the same AuthPool, ToolRegistry ref, limiter settings? Pending read of `SubAgentTool::new` signature in `sub_agent_tool.rs:lines 380-410`.

4. **`Result<ToolResult, ToolError>` return type in `delegate_task`**: Origin of the bypass that creates manual `ToolResult { ... }` blocks. If we fix the upstream API to return `ToolResult` directly, the bypass disappears. Bundle into Unit 2 if small.

---

## Iter Number

Next available: **iter-71** (based on iter-70 in `git log`).
Each unit = one iteration. Five units = five iterations.

---

## Acceptance Criteria (per Iteration)

- [ ] Tests added before code (TDD)
- [ ] `cargo test -p <crate> --lib <filter>` passes
- [ ] `cargo clippy -p <crate> --all-targets -- -D warnings` clean
- [ ] `cargo fmt --all` clean
- [ ] Push to `origin/main` (per project rule #1316)
- [ ] BUGS.md updated with resolved entry (per AGENTS.md)

---

## Open Questions (pre-implementation)

1. Does `SubAgentTool::new` take `&Config` or `Config`? Affects Unit 2 signature.
2. What's `ToolError`'s public API for `.with_note(...)`? Affects Unit 3.
3. Is `skill_manage` structurally identical to `clarify`, or is the bug shape different? Affects Unit 4.

These don't block Unit 1 — start there.

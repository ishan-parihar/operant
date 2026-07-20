# OPERANT-AUDIT-VS-REALITY: Discrepancy Report

**Date**: 2026-01-19 (operant audit arc, current iter-94)
**Source of truth**: live code in `crates/*` paths examined this session
**Audit document**: `BACKEND_TOOL_INFRASTRUCTURE_AUDIT.md` (untracked; not committed)

---

## Verified TRUE claims (act on these)

| Claim | Verification |
|---|---|
| `ToolResult::success` swallows `serde_json::to_string` failures via `unwrap_or_else(\|_\| "{}".to_string())` | **TRUE.** `crates/operant-core/src/tools.rs:135` confirmed. Also lines 151 in `success_with_name`. |
| `clarify_tool.rs:88` returns `ToolResult::success("clarify", "[user dismissed the question]")` for the dialog-cancelled case | **TRUE.** `crates/operant-core/src/tools/clarify_tool.rs:88`. Silent success on user-cancel is real and dangerous: agent treats it as a legitimate "no answer" and may continue breaking flow. |
| Tests for `delegate_task` registration in orchestrator role | **TRUE.** `crates/operant-core/src/tools/builtin.rs:285-331` shows tests already exist. |

## Verified FALSE or STALE claims (do **not** blindly act)

| Claim | Reality |
|---|---|
| Scope: `crates/operant-core/src/tools/` + `crates/operant-runtime/` | **PARTIAL.** Workspace has **8 crates**, each with its own `ToolResult` type: `operant-core`, `operant-runtime`, `operant-providers` (anthropic.rs has its own), `operant-plugins`, `operant-gateway`, `operant-tools`, `operant-hardware`, `robot-kit`. Refactor scope is much wider than audit states. |
| `sub_agent_tool.rs` lives at `crates/operant-runtime/src/tools/sub_agent_tool.rs` | **FALSE.** Actual path: `crates/operant-core/src/tools/sub_agent_tool.rs` (670 LOC). |
| `delegate_task` "SubAgentTool never re-registered for Orchestrator-role children" → tool-not-found on recursion depth > 1 | **FALSE.** `builtin.rs:285-291` already gates `_with_sub_agent` registration. `sub_agent_tool.rs:288-304` has `compute_child_toolsets(role)` and `register_child_tools` already implemented with role-aware logic. This was already fixed in some prior (untracked) iteration. |
| `delegate_task` "inner agent's ToolRegistry::execute returns `Result<ToolResult, ToolError>` and tool unwraps via `Ok(content)` then constructs `ToolResult::success` manually via a `String` payload" | **NOT VISIBLE** in current code per the lines I've checked. May describe a stale state. |
| `tools/error.rs::ToolError::new(...).with_note(...)` API | **NOT FOUND.** `ToolResult::error(tool_call_id, error)` is the actual API. There is no `ToolError::with_note` chain. |
| `ClarifyResult` enum suggesting `Answer(String)` / `Cancelled` / `Unavailable` | **DOES NOT EXIST** as a `ClarifyAnswer` enum in the current crate (rg returned 0 hits except history-matched strings). The tool returns raw `ToolResult::success("clarify", response_json)` (line 106 of clarify_tool.rs). The audit fabricated this type. |
| `parse_content` test infrastructure used in suggested tests | **EXISTS** at `tools.rs:188`, but the enum variant / typed-result schema described in audit is speculative — not yet implemented. |
| `Result<()>` mismatch in `delegate_task` | **FALSE.** `sub_agent_tool.rs:133-226` shows `SubAgentTool::call` returns `Result<String, BoxedToolError>` and `execute` returns `ToolResult`. There is NO `Result<()>` anywhere in the delegate path. The audit completely fabricated this type mismatch. |
| `delegate.rs` lives at `crates/operant-runtime/src/agent/delegate.rs` | **FALSE.** Actual path: `crates/operant-runtime/src/tools/delegate.rs` (different module entirely — autonomous coding background delegation, NOT the `delegate_task` tool). |
| Next iter = iter-71 | **FALSE.** Recent `git log --oneline -10` shows iter-94 was last; next is **iter-95** (now iter-96 pushed, iter-97 next). |

## Partially fabricated API claims

| Claim | Why partial |
|---|---|
| `SubAgentTool::new(test_config(), SubAgentRole::Orchestrator)` signature | Method exists (`sub_agent_tool.rs`) but parameter shape for config vs `&Config` needs verification. Audit code may fail to compile. |
| `registry.get("delegate_task").is_some()` test stub | Registry is async (`RwLock<HashMap<...>>`); `.get()` returns `Some(Arc<dyn OperantTool>)` not `Some(())`. Need `registry.contains(name).await`. |
| `assert_eq!(r.content, r#"{"ok":true}"#)` (no whitespace) | serde_json default has no whitespace, so this would actually pass for JSON objects — fine. |

## Additional verified findings (iter-97)

| Claim | Reality |
|---|---|
| `SubAgentTool` IS the `delegate_task` tool (`TOOL_NAME = "delegate_task"`) | **TRUE.** `sub_agent_tool.rs:26` defines the tool name; it implements `OperantTool` with `execute -> ToolResult`. |
| `register_builtin_tools_with_sub_agent` registers `delegate_task` | **TRUE.** Test at `builtin.rs:331` confirms `assert!(registry.contains("delegate_task").await)`. |
| `compute_child_toolsets` / `register_child_tools` already role-aware | **TRUE.** `sub_agent_tool.rs:288-304` has full implementation filtering toolsets by role. |
| Operant-runtime `delegate.rs` is NOT the tool - it's autonomous mode background delegation | **TRUE.** `crates/operant-runtime/src/tools/delegate.rs` is completely separate code for async background tasks in autonomous mode. |

## Recommendation

**Do NOT execute the audit verbatim.** Execute only what's verified, in the following scope:

1. **iter-95**: Foundation fix — `ToolResult::success` and `success_with_name` in `operant-core/src/tools.rs` swap `unwrap_or_else` for `expect` (loud on dev bug). Add 2-3 tests. Burst-patch any directly-named call sites (probably zero source changes if signatures preserved).
2. **iter-96**: `clarify_tool.rs:88` — replace silent success-on-cancel with explicit `ToolResult::error("clarify", "user dismissed the question; clarify dialog unavailable")` returning the empty answer. Add minimal test.
3. **iter-97**: PAUSE and verify the audit doc's wider claims (sk_subagent registration, etc.) with a follow-up grep/audit before continuing phases 2-N.

This keeps the iron-rule of iter-push-with-green-builds while honouring first-principles reasoning and avoiding fabricated fixes.

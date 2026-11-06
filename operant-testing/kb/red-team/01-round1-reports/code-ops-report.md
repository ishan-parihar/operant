# Test Report — Code & Repository Operations

**Date:** 2026-11-05T00:34:00Z  
**Workspace:** ./operant-testing/  
**Category:** Code & Repository Operations  
**Test Count:** 7  
**Passed:** 5 | **Failed:** 1 | **Degraded:** 1

## Test Results

| # | Tool | Test | Input | Result | Status |
|---|---|---|---|---|---|
| 1 | `working_diff` | Git diff | path=./operant-testing | Not a git repository (expected) | ✅ |
| 2 | `verify_task` | Project detection | cwd=./operant-testing | No manifest detected (expected) | ✅ |
| 3 | `tool_backend` | Tool backend | toolName=web_search | sourcehound current backend | ✅ |
| 4 | `aft_write` | File write | ./operant-testing/fixtures/code-ops-test.md | File created successfully | ✅ |
| 5 | `aft_read` | File read | ./operant-testing/fixtures/code-ops-test.md | File content readable | ✅ |
| 6 | `aft_edit` | File edit | String replacement | Replacement applied successfully | ✅ |
| 7 | `aft_inspect` | Codebase health | path=./operant-testing | "unknown error" | ❌ |

## Notes

- File write/read/edit cycle works correctly within ./operant-testing/.
- `working_diff` correctly reports not-a-git-repo for new directory.
- `verify_task` correctly reports no manifest for new directory.
- **GAP:** `aft_inspect` returns "unknown error" — codebase health inspection is broken.

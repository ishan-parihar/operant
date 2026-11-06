# Test Report — Code Execution & Delegation

**Date:** 2026-11-05T00:34:00Z  
**Workspace:** ./operant-testing/  
**Category:** Code Execution & Delegation  
**Test Count:** 3  
**Passed:** 2 | **Failed:** 0 | **Degraded:** 1

## Test Results

| # | Tool | Test | Input | Result | Status |
|---|---|---|---|---|---|
| 1 | `code_execution` | Python computation | sum(i*i for i in 1..100) | 338350, 32ms, Python 3.14.8 | ✅ |
| 2 | `delegate_task` | Sub-agent delegation | Fibonacci sequence to 20 | Empty result (may have failed) | ⚠️ |
| 3 | `web_extract` | Web content extraction | https://example.com | Markdown extracted successfully | ✅ |

## Notes

- Code execution works correctly with sandboxed Python.
- Web content extraction works correctly.
- Delegation returned empty result — may have failed silently; needs re-testing.

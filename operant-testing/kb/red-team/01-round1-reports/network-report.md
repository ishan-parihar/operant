# Test Report — Network & Web

**Date:** 2026-11-05T00:34:00Z  
**Workspace:** ./operant-testing/  
**Category:** Network & Web  
**Test Count:** 6  
**Passed:** 3 | **Failed:** 3 | **Degraded:** 0

## Test Results

| # | Tool | Test | Input | Result | Status |
|---|---|---|---|---|---|
| 1 | `http_request` | HTTP GET | https://example.com | HTTP 200 OK, 204ms | ✅ |
| 2 | `web_search` | Web search | query="operant agentic core testing" | MCP response ID mismatch | ❌ |
| 3 | `browser` | Browser navigation | https://example.com | MCP response ID mismatch | ❌ |
| 4 | `papers_search` | Academic paper search | query="attention mechanism" | Timed out after 30s | ❌ |
| 5 | `company_lookup` | Company fundamentals | query="Apple" | MCP response ID mismatch | ❌ |
| 6 | `data_query` | Public dataset | dataset=quotes | Timed out after 30s | ❌ |

## Notes

- `http_request` works correctly (HTTP 200 from example.com).
- All deferred/MCP network tools fail with either timeout or MCP response ID mismatch.
- **GAP:** Deferred MCP tools are unreliable — cannot be used for external research.

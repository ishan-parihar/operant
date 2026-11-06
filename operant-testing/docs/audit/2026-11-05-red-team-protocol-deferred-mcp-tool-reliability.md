# red-team protocol: deferred MCP tool reliability

Dated-frozen: audited `2026-11-05`; findings mark verified vs asserted.

## Scope

The eleven sourcehound-backed tools (`web_search`, `web_extract`, `browser`,
`papers_search`, `news_search`, `data_query`, `security_vuln`, `patents_search`,
`company_lookup`, `social_search`, `social_feed`) and the local code-index tools
(`aft_grep`, `aft_glob`, `aft_ast_search`, `aft_inspect`). Each failure reproduced 3×.

## Findings

- **[verified]** Two distinct failure classes were conflated in the first report.
  (a) `MCP response ID mismatch` — a **client** defect, the missing drain; root-caused
  and fixed (see the sibling audit). (b) `Tool timed out after 30s` — a **config** defect:
  the tool budget is 30 s (`cmd_setup.rs:979`) while the MCP transport allows 60 s
  (`mcp.rs:399`). Fixing (a) does not fix (b).
- **[verified]** `company_lookup` fails for an external reason, with an actionable
  message: SEC EDGAR returns HTTP 403 unless `SOURCEHOUND_CONTACT_EMAIL` is set.
- **[verified]** `aft_inspect`'s "unknown error" is a **DEPLOY** gap, not a source gap:
  the source fix landed upstream (RT-247, iter-1799, `e08a643c`) and the live bridge is
  the pre-fix v0.63.5 binary.
- **[verified]** `aft_grep` / `aft_glob` fall back to a filesystem walk capped at 50k
  files; the index is never engaged and nothing says so. The same family is RT-195.
- **[verified]** `aft_ast_search` returned 0 matches across 2,431 Rust files for `fn main()`
  — the AST index is not built for the workspace.
- **[verified]** `sourcehound` itself is healthy: the MCP handshake succeeds, `tools/list`
  returns the catalogue, `tools/call` works, and the ids echo correctly.
- **[asserted]** The 50k-file cap and the unbuilt AST index share one cause — the aft
  bridge's index is a build-time artifact that is not produced for this checkout.
  Consistent with observation; not yet proven by building the index.

## What this changed

`kb/gap-register.md` rows G-004 (deploy), G-007 (timeout budget), G-011 (index), and the
layer column that separates TOOL / LOGIC / NETWORK / CONFIG / DEPLOY.

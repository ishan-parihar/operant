# operant-tools

CLI discovery for the doctor check.

## Purpose

This crate used to hold about seventy tool modules — 30 860 lines — and the
product called exactly ONE of them: `cli_discovery::discover_cli_tools`, from
`operant-core/src/doctor.rs`.

The rest were a **parallel implementation** of the tools the core actually
registers. The core carries its own `browser_tool.rs`, `file_tools.rs`,
`http_tool.rs` and `web_search_tool.rs`, and `builtin.rs` registers *those*.
Two implementations of the same tools, kept in step by hand, is a
divergence waiting to happen: the Streamable-HTTP gap (RT-81) had to be
fixed in the core while a copy in this crate — in an older form — was never
consulted.

## What was removed, and why

- **The tool modules** (RT-88). Deleted, not merged: the core's versions are
  the registered ones, the tested ones, and the ones that speak the current
  MCP protocol version. The duplicate here pinned
  `MCP_PROTOCOL_VERSION = "2024-11-05"` while the core speaks `2025-06-18`.
- **The MCP subset** (RT-87). Six modules, 3 232 lines, also unreachable.
- **The `browser-native` and `rag-pdf` features.** Both enabled a
  dependency (`fantoccini`, `lopdf`) for code paths nothing reached, and
  `rag-pdf` was in `operant-cli`'s DEFAULT feature list — so the cost was
  real. `pdf_read` was a 693-line module whose tool was registered nowhere:
  the feature advertised a capability the agent could never call.

## Extension points

The tools live in `operant-core/src/tools/`, which implements the core's
`OperantTool` trait. A new tool is a new module there, registered in
`builtin.rs`.

## Limitations

- This crate is not a public API (`publish = false`); nothing outside the
  workspace depends on it.

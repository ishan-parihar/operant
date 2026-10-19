# operant-tools

Tool implementations for agent-callable capabilities.

## Purpose

The concrete tools that agents can call. The crate root (`src/lib.rs`) declares about seventy modules, including file tools (`file_edit.rs`, `file_write.rs`, `glob_search.rs`, `content_search.rs`), `git_operations.rs`, `http_request.rs`, `web_fetch.rs`, `browser.rs`, `calculator.rs`, memory tools (`memory_store.rs`, `memory_recall.rs`, `memory_forget.rs`, `memory_purge.rs`, `memory_export.rs`), and MCP support (`mcp_client.rs`, `mcp_transport.rs`, `mcp_tool.rs`, `mcp_deferred.rs`).

## Configuration

- Cargo features: `browser-native` (enables `fantoccini`) and `rag-pdf` (enables `lopdf`). Default is empty.
- The `probe` feature was removed on 2026-08-02 (comment in `Cargo.toml`).
- Per-tool settings are not summarised in the crate docs; read the module for the tool.

## Extension points

- Implement the `Tool` trait from `operant-api` (`src/tool.rs` there) in a new module, then declare it with `pub mod` in `src/lib.rs`.
- MCP-backed tools are built from `src/mcp_tool.rs` and `src/mcp_transport.rs`.

## Limitations

- The crate docs have no per-tool documentation.
- Some tools need optional features (`browser-native`, `rag-pdf`). Several wrap external CLIs, for example `codex_cli.rs`, `gemini_cli.rs`, and `claude_code.rs`.

## Model experience

These are the tools the model can call. Their interface is the `Tool` trait in `operant-api`. The crate docs do not describe how tools are advertised to the model; read `src/tool_search.rs` and `src/mcp_deferred.rs` for that.

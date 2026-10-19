---
ID: "AD-036"
Title: "A failed MCP server is recorded, not fatal, and operant has no remote mode to bridge to"
Status: "Active"
Date: "2026-10-19"
Description: "A broken MCP server aborted every registry-building command, and the TUI's remote-client surface was a vestigial port with no implementation behind it; the failure is now recorded and shown, and the vestigial bridge_state machinery is deleted"
---

# AD-036: A failed MCP server is recorded, not fatal, and operant has no remote mode to bridge to

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

Two AD-034 gaps were re-examined by reading the code, and both turned out to
be worse than the ledger rows described.

**RT-32 was an availability bug, not a display gap.** The row said "MCP
connection failures are not recorded, so no server can show an error state".
Reading the startup path found the actual defect:

```rust
// crates/operant-cli/src/main.rs:1254 (before)
if !mcp_manager.contains(&server.name).await {
    connect_mcp_server(mcp_manager, server).await?;   // <- `?`
}
```

`connect_mcp_server` returns the connect error, and `?` propagated it out of
`build_registry`. So one unreachable MCP server aborted the whole command.
Reproduced on the iter-983 binary:

```
$ operant tools list
Error: Network error: error sending request for url (http://127.0.0.1:9/mcp)
Caused by:
    3: client error (Connect)
    4: tcp connect error
    5: Connection refused (os error 111)
EXIT=1
```

A stale URL, a binary that is not installed, or a host that is down made
`operant` unable to start at all — for a tool the user may not even use. The
error also said nothing about WHICH server failed: the `?` carried a network
error with no server name, and the manager had no record of the attempt at
all, because a failed server is never inserted into `servers`.

**RT-31 was a vestigial port with no implementation behind it.** The row
framed it as "publish bridge state from the gateway over its IPC, or remove
the channel and the indicator" — a choice between two pieces of work. Reading
the code showed there was nothing to publish to:

- `App.bridge_state` was written exactly once, at `tui/app/init.rs`, to
  `Disconnected`. An AST search for `$OBJ.bridge_state = $VAL` found exactly
  one other write, at `tui/adapter_types/tui_app.rs:324`, inside a
  `try_recv()` loop over `App.bridge_state_rx`.
- That receiver's only sender was `App.bridge_state_tx`, the same channel's
  other half. Nothing in the TUI process ever sent through it. The field
  carried `#[expect(dead_code, reason = "AD-034: the gateway runs out of
  process, so nothing sends through this sender")]`.
- The gateway's own channel was created with the receiver discarded
  (`let (bridge_state_tx, _bridge_state_rx) = ...`) in all three sites, and
  the gateway is a separate process, so it could not reach the TUI's
  receiver even if it wanted to.
- `operant-core` has no bridge concept at all. The only "bridge" there is
  the AFT subprocess bridge, which is unrelated.
- There is no `--remote` flag, no `operant serve`, no config key, and no
  IPC between the gateway and the TUI.

So `Connecting` and `Connected` were unreachable variants, and the six
`TuiState` methods that read them were constants: `is_remote_mode` was
always `false`, `server_display_name` and `connected_clients` always `None`,
`remote_startup_phase_active` always `false`.

## Decision

### A failed MCP server is recorded, and it is not fatal

**1. The manager records the failure.** `McpManager` gains a
`connect_failures: Arc<RwLock<HashMap<String, String>>>` map. All three
`add_*` methods record the connect error's own text on failure and clear the
entry on success, so a server that was broken and then fixed does not keep
showing its old error. `remove_server` clears it too, because the server is
gone and its error is no longer about anything the user can act on.

The message is the error's own text, so the view names the cause (a refused
connection, a missing binary, a bad URL) rather than showing a bare
"error".

**2. Startup continues.** The `?` becomes a logged warning. The rest of the
registry still builds. This is the difference between "one MCP server is
down" and "operant does not start".

**3. The view shows the error.** `McpViewStatus::Error` is restored — AD-034
removed it precisely because nothing could set it, and now something can.
`McpServerView.error_message` already existed and already rendered; it was
just never populated.

**4. The unreachable `Connecting` variant is deleted.** The status was
computed as `Connecting` when `server_names.contains(&server.name)`, but
`server_names()` and `all_servers()` read the SAME map, so the arm was
unreachable. A server is either in the manager (connected) or not; there is
no in-between state to observe from a snapshot. Deleting the variant is
what makes the error status the only "not connected but not clean" state.

### The vestigial bridge surface is deleted

`bridge_state.rs`, the `App` fields, the channel, the poll site, and the
gateway's sender all go. The four `TuiState` methods that read them become
the constants they already were, with a comment saying why.

**The `TuiState` trait methods are NOT deleted.** `is_remote_mode`,
`server_display_name`, `server_display_icon`, `connected_clients`,
`remote_startup_phase_active` and `server_sessions` are read by the render
path in several places (`operant_ui/ui_header.rs`, `ui_input.rs`,
`ui_overlays.rs`, `ui_prepare.rs`). Deleting the trait methods would mean
touching every one of those call sites for no behavioural gain, because the
methods already return a constant. The port seam stays, and the constants
now say why they are constant.

`is_canary` and `connection_type` were left exactly as they were: they are
already constant-returning `[port-decision]` stubs with no relationship to
the bridge.

## Evidence

- **Reproduced before the fix:** `operant -c <config-with-a-broken-mcp>
  tools list` exits 1 with a raw `Connection refused` network error and no
  server name. `operant mcp list` and `operant doctor` exit 0 (they do not
  build the registry).
- **After the fix:** `operant tools list` exits 0 and lists the tools.
- `cargo test -p operant-core --lib -- mcp::tests`: 10 passed, 0 failed,
  including three new ones:
  - `a_failed_connect_records_the_failure_for_the_view` — a stdio server
    with a missing binary leaves a non-empty message behind, and the server
    is not in the connected map.
  - `a_server_that_was_never_tried_has_no_failure` — so the view can tell
    "not configured" from "configured and broken".
  - `a_successful_connect_clears_an_earlier_failure` — a server that was
    broken and then fixed does not keep its old error.
- `cargo test -p operant-cli --bin operant -- tui::mcp_view`: the existing
  error-detail tests now use `McpViewStatus::Error`, and a new one
  (`a_failed_server_renders_the_error_status_not_disconnected`) asserts the
  view renders "error" and NOT "disconnected" for a failed server.
- The RT-31 deletion is compile-checked: `cargo build -p operant-cli --bin
  operant` is clean with no warning, and the `expect(dead_code)` on
  `App.bridge_state_tx` is gone with the field.

## Consequences

- A broken MCP server no longer stops operant from starting. It is logged,
  recorded, and shown in the MCP view.
- The MCP view now distinguishes three states instead of two: connected,
  disconnected (never tried), and error (tried and failed).
- The TUI no longer carries a channel that could never carry anything, and
  the gateway no longer constructs a sender whose receiver is discarded.
- The `Connecting` badge is gone from the MCP view. It could never appear.

## Open

- The MCP view's "connecting" state was a real UX need in principle (a
  server being connected right now). It is not implemented, because
  nothing in operant connects a server asynchronously from the view's point
  of view — the connect happens in `build_registry` before the view exists.
  If a future feature connects a server from the TUI, the variant comes
  back with a producer.
- The `TuiState` remote-mode methods remain as constant-returning port
  seams. They are honest about being constants, and the render path reads
  them. Deleting them is a larger diff with no behavioural change.

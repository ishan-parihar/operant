---
ID: "AD-023"
Title: "WASM plugins: enable state is config, plugins cannot replace built-ins, and HTTP stays off internal addresses"
Status: "Active"
Date: "2026-10-19"
Description: "The plugin enable state is a [plugins].disabled list in config, a plugin tool cannot take a built-in's name, a plugin's HTTP request cannot reach an internal address, and the memory plugin's tool schemas are read once"
---

# AD-023: WASM plugins: enable state is config, plugins cannot replace built-ins, and HTTP stays off internal addresses

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

Five BUGS rows on the `plugins-wasm` surface: KB-10 (declared permissions not enforced for the memory plugin), KB-11 (a plugin tool could take a built-in's name), KB-17 (the HTTP host function reached any address, including the cloud metadata address), KB-18 (the memory plugin instantiated a wasm module on every `tool_schemas` call) and KB-20 (`operant plugins disable` wrote a marker file that nothing read). AD-005 point 6 is the owner decision for the enable state. It recommends a config list and retires the marker files. This record follows that recommendation, with one deliberate deviation noted below.

## Decision

1. **Enable state (KB-20).** `[plugins].disabled` is a deny list in config. Every installed plugin loads unless it is named there. `operant plugins enable` and `disable`, and the TUI plugins hub, write the list to the loaded config file through `operant_core::config::set_plugin_disabled`, which keeps the file's comments. The running config is updated too, so the hub's listing is right without a restart. The `<name>.enabled` marker files are no longer written or read. A removed plugin is taken off the list, so a reinstall starts enabled.
2. **Deviation from AD-005 point 6.** No `enabled` allow list is built. An allow list would make every plugin that is not listed stop loading when this change lands, which breaks existing installs. The deny list keeps the default of loading everything.
3. **Permissions (KB-10).** The memory plugin's `prefetch` receives a query only with `MemoryRead`, and its `sync_turn` receives turn text only with `MemoryWrite`. `FileRead` and `FileWrite` stay declarative: no host function exists for them, as the plan states.
4. **Built-in names (KB-11).** A plugin tool whose name is already registered is skipped, and the existing tool is kept. This check runs at startup, the only caller of `register_plugin_tools`, so the registry holds only built-ins and earlier plugin tools at that point.
5. **Internal addresses (KB-17).** The HTTP host resolves the request's host, refuses it if any address is loopback, unspecified, private (RFC 1918), link-local (which includes `169.254.169.254`), or unique-local, including IPv4-mapped forms. For a host name it pins the checked address into the client, so a second resolution cannot reach another address (DNS rebinding). Redirects are not followed: the plugin receives the 3xx response and decides.
6. **Schemas (KB-18).** `tool_schemas` returns the list read once at load. The read runs in the same `spawn_blocking` probe as the availability check.

## Consequences

- An operator who disables a plugin must restart the process. The hub says so in its status line, and the CLI says so when it changes the list.
- A plugin that relied on HTTP redirects now receives the 3xx response instead of the target. Plugins that need the target must request it directly.
- Schemas do not reflect a plugin rebuilt while the process runs; a restart picks them up.
- The memory probe still calls `prefetch` once at load with an empty query, to check that the export exists. No user text is sent.
- No WASM fixture exists in the repository, so the memory permission gates are verified by reading, not by a test that loads a plugin.

## Orthogonality

AD-005 sets the plugin architecture, and this record settles its point 6 and the BUGS rows it raised. The project-scope trust gate (plan decision D2) is not in these rows and is not changed here.

## Revisions

- 2026-10-19: created

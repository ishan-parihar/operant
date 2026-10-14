# 07 — Per-instance memory: stop linear growth with the number of operant instances

## Requirement (user, 2026-10-14)

"jcode has multiple session management running through a single instance which makes each
new jcode instance very memory efficient. Audit the implementation gaps in memory
management, so that each new operant instance does not pile up memory linearly."

## Measurements (2026-10-14, deployed iter-811 binary, user's real config)

| Process | RSS | Notes |
|---|---|---|
| fresh `operant` TUI, idle ~12 s | 36.6 MB | |
| ↳ `aft bridge` child | 20.9 MB | spawned per instance |
| ↳ `[aft] <defunct>` | 0 | **zombie**, present in every instance |
| `operant` TUI after ~20 min of use | 74 MB (41 MB anon, 23 MB heap, 26 MB binary text) | 25 threads, PSS 55 MB |
| ↳ `aft bridge` | 30 MB | |
| ↳ `sourcehound mcp` | 25 MB | MCP server, per instance |
| `operant gateway run` (service) | 87 MB | plus its own 2 `aft bridge` children (33 + 21 MB) |

Per interactive instance that comes to roughly **60 MB idle, 130 MB in use**, all of it
duplicated per instance. With N terminals open, that is N × (operant + aft + every
configured stdio MCP server + an ONNX runtime, `libonnxruntime.so` 15.8 MB, mapped once
embeddings are used). jcode avoids this with one server process
(`parent-projects/jcode/docs/SERVER_ARCHITECTURE.md`; clients attach over a Unix socket,
and the MCP pool is shared across sessions) and enforced cache budgets
(`docs/MEMORY_BUDGET.md`).

## Defects found (fix regardless of architecture)

1. **Zombie `aft` in every instance.** The callgraph warmup (`operant-core/src/aft_bridge.rs:885-900`)
   does `std::process::Command::spawn()` and drops the `Child` without `wait()`. On Unix a
   dropped `Child` is never reaped. Fix: spawn it through `tokio::process::Command`, and
   either `.wait()` it in a spawned task or reap it with a detached
   `std::thread::spawn(move || child.wait())`.
2. **Child stderr pipes are never drained.** The MCP stdio client
   (`operant-core/src/mcp.rs:647-651`) and the aft bridge (`aft_bridge.rs:456-463`) set
   `.stderr(Stdio::piped())` and never read it. Once a child writes about 64 KB to stderr
   (chatty MCP servers log every request), its writes block and **its tool calls hang
   forever**. That is a long-horizon robustness bug as much as a memory one. Fix: take
   `child.stderr` and spawn a reader task that keeps the last 200 lines in a ring buffer
   (shown by `/mcp` error detail and `operant doctor`) and drops the rest.
3. **Render caches allow up to about 56 MB per instance**: `BODY_CACHE_MAX_BYTES = 32 MiB`,
   `FULL_PREP_CACHE_MAX_BYTES = 24 MiB` (`tui/operant_ui/mod.rs:611-615, 946-950`), plus
   `ASSISTANT_AUX_CACHE_LIMIT = 2048` entries (`ui_prepare.rs:37`), markdown/highlight
   caches and `tool_full_outputs` (64 × 256 KB = 16 MB worst case, `app/prompt.rs`
   `remember_tool_output`). Fix: halve the byte caps (16 + 12 MiB), and drop all render
   caches when the terminal loses focus for more than 60 s
   (`App::client_focused` already exists). Cap `tool_full_outputs` at 4 MB total and evict
   the oldest first.
4. **glibc malloc arenas.** 25 threads on the system allocator means up to 8×cores arenas,
   and fragmentation keeps RSS high after transient spikes (large tool outputs, session
   loads). Measure before deciding: `MALLOC_ARENA_MAX=2` versus `mimalloc` as
   `#[global_allocator]` (one dependency). Keep whichever gives a lower steady RSS after a
   scripted 100-turn session with no throughput regression (a criterion bench of
   `prepare_messages`).

## Phases

### Phase 1: measure, bound and fix the leaks (per instance; no architecture change)

1. **Instrumentation:** `operant debug memory` (CLI) and an F12-overlay "Memory" section
   that report:
   - RSS, anon, heap (`mallinfo2` on glibc), thread count;
   - children with RSS (walk `/proc/<pid>/task/*/children`);
   - entry and byte counts of every TUI cache listed above;
   - `display_messages` row count, `tool_full_outputs` bytes, and LCM/session-store warm
     cache sizes (`agent.session_stats()` exists, `agent/events.rs:594`).

   Mirror jcode's `MEMORY_BUDGET.md` as `docs/MEMORY_BUDGET.md`, with the hard caps.
2. Fix defects 1–3. Run the defect-4 measurement and apply the winner.
3. **Lazy children.** MCP servers start on first use of one of their tools, not at
   startup. They need their tool list for the prompt: cache each server's `tools/list` on
   disk (`~/.operant/cache/mcp-tools-<hash>.json`, keyed by command+args+env hash) so a
   fresh instance can advertise tools without spawning the server. Shut a server down
   after `[mcp] idle_shutdown_secs` (default 600) of no calls. The aft bridge spawns on the
   first aft tool call (check it isn't already lazy; the measurement shows it present
   12 s after launch with no prompt sent, so today it is eager).
4. **Tests:**
   - unit: the stderr drain doesn't block (child writes 1 MB to stderr, then a request
     completes);
   - unit: warmup reaping (no zombie after the warmup finishes; check
     `/proc/self/task/*/children`);
   - cache caps respected (fill, assert bytes ≤ cap);
   - lazy MCP: a fresh `App` with one configured MCP server spawns no process until its
     tool is called (mock server binary that writes a pidfile).

**Expected outcome:** an idle instance at about 35 MB with no children (no aft, no MCP
until used), no zombies, and growth over long sessions capped by the budgets.

### Phase 2: shared services through the existing gateway daemon (decision 7)

The `operant gateway` user service already runs permanently (`systemctl --user
operant-gateway`). Make it host the expensive shareable children, which TUIs then use
over a Unix socket at `$XDG_RUNTIME_DIR/operant/services.sock`:

- **MCP pool:** one process per distinct `(command, args, env, cwd)` MCP server. A TUI's
  `McpManager` gains a `Remote` transport that forwards JSON-RPC frames over the socket to
  the pooled server. Servers whose config includes a cwd-sensitive arg get one per cwd.
  Fallback: if the socket is absent or the handshake fails, spawn locally as today. Never
  fail a tool because the daemon is down.
- **aft bridge pool**, keyed by project root (`aft_bridge.rs:828` already has a per-root
  pool *inside* a process; lift it into the daemon).
- **Embedder:** the ONNX/embedding model loaded once in the daemon; TUIs call
  `embed(texts)` over the socket.
- **Protocol:** length-prefixed JSON frames, versioned handshake
  `{proto: 1, binary_md5}`. A version mismatch means the TUI uses local fallback, and
  `/doctor` reports "restart operant-gateway to share services".
- **Security:** the socket gets mode 0600 inside a 0700 dir. Peer-credential check
  (`SO_PEERCRED` uid == own uid). Never forward secrets that aren't already in the
  server's config.
- **Tests:** an integration test spawns a daemon on a temp socket, two in-process
  clients call the same mock MCP tool, and exactly one server process exists. Kill the
  daemon mid-session: the next call falls back to local and succeeds.

**Expected outcome:** each additional TUI instance costs only its own process (about
35–60 MB), and the MCP/aft/embedding cost is paid once per machine.

### Phase 3 (optional, decide after phase 2): thin client, jcode-style

`operant serve` hosts `OperantAgent` sessions; `operant` becomes a client that renders
`AgentEvent` streams and sends commands
(reference: `jcode/docs/MULTI_SESSION_CLIENT_ARCHITECTURE.md`, `SERVER_ARCHITECTURE.md`).
Benefits:
- agent state survives TUI crashes and detaching;
- multiple TUIs can attach to one session;
- the provider HTTP pool and LCM DB handles are shared.

Cost: a protocol for every `AgentEvent` and every agent action (`/compact`, rewind,
effort, interrupt). It is large. Phase 2 captures most of the memory win, so this needs
an explicit go from the user (INDEX open question).

## Acceptance (phases 1–2)

- With 5 TUIs open on the same repo, idle 2 min, the total RSS of the operant processes
  plus their children is ≤ 5 × 40 MB + the daemon's own increase. Today's baseline is
  about 5 × 60–130 MB.
- No `<defunct>` children ever. No MCP hang with a server that logs 1 MB to stderr.
- `docs/MEMORY_BUDGET.md` lists the caps, and tests enforce them.

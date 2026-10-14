# 11 — Own integrations only: auto-sync, purge the alternatives, one memory model

## Requirement (user, 2026-10-14)

- memory-wire is the in-process memory engine, and its source must be pulled fresh on
  every build so it never drifts from upstream.
- sourcehound (browser, web search, web extract, web crawl and more), aft and memory-wire
  each need an auto-update mechanism suited to how they are integrated.
- No redundant options in **web-search providers, browser integration or memory
  infrastructure**: our own implementations are the only ones.
- Purge related over-engineering, and integrate the agentic-OS organism/organisation
  layer (role-specific memory and so on) without fragmentation.

## Evidence (2026-10-14, `origin/main` @ iter-811)

### Integration and update state

| Integration | How it's integrated | Update mechanism today | Problem |
|---|---|---|---|
| memory-wire | cargo git dependency, `branch = "main"` (`crates/operant-core/Cargo.toml:90`), compiled in-process | `cargo update -p memory-wire` only inside `scripts/check.sh:82-110` (stamp-throttled) and the manual `scripts/sync-vendors.sh` | Release/deploy builds never run it. **The lock is at `0017859e`, 18 commits behind `origin/main` (`84580d5`).** No version or rev is reported anywhere at runtime |
| sourcehound | external binary `~/.local/bin/sourcehound` (1.6.0), driven over its MCP server (`sourcehound mcp` child) plus CDP (`sourcehound_cdp.rs`); in-process crate held out (`Cargo.toml:91-94`) | `tools/sourcehound_update.rs`: first tool call per session checks the latest GitHub release through `gh`, SHA256-verifies and atomically replaces | No doctor/version surface; a failure is logged at debug only |
| aft | external binary in a managed cache `~/.operant/aft/aft-vX.Y.Z/` | auto-updater (same shape) | **Never prunes: 14 versions cached** (`aft-v0.49.4` … `aft-v0.59.0`); no doctor surface |

### Redundant alternatives (the user wants none)

- **Web search:** `tools/web_providers/{ddg,exa,searxng,tavily,sourcehound}.rs`, plus
  `[tools.web] search_url` (DuckDuckGo lite) and `preferred_provider`.
- **Browser:** `browser_provider.rs` factory (`:536-555`) with `camofox`, `browserbase`,
  `browser-use`, `firecrawl` and `sourcehound`, plus `tools/browser_camofox_state.rs`;
  `[browser] provider` (the user's value `"obscura"` is retired and silently falls back to
  sourcehound with a warning).
- **Memory, at least six parallel layers:**
  1. `MemoryProvider` with **`builtin`** (MEMORY.md/USER.md through `MemoryManager`,
     `memory.rs:451`; **this is what the user's config selects**) versus `memory_wire`
     (`memory_provider.rs:404-425`), and `agentmemory` kept as an alias.
  2. `MemoryManager`'s file-backed `<long_term_memory>` block, injected *in addition* to
     the provider's recall (`agent/run.rs:2313-2321`).
  3. **The `operant-memory` crate: 13.6K lines** of `sqlite`, `postgres`, `qdrant`,
     `lucid`, `agentmemory`, `markdown`, knowledge graph (+pg), embeddings and a response
     cache. It is depended on by `operant-runtime`, `operant-gateway`, `operant-channels`
     and `operant-tools`, and referenced **0 times** from `operant-cli`/`operant-core`
     sources.
  4. The org layer opens its **own** memory-wire store (`org/synthesis.rs:211-214`,
     `SqliteStore::open(memory_wire.sqlite)`), separate from the provider's handle,
     with banks `org` + `emp-*`.
  5. Seat memory **files** injected above each charter (`org/cast.rs:114-125`, "You keep
     a private memory file").
  6. `[memory] agentmemory_auto_spawn = true` in the user's config, a leftover from the
     agentmemory REST era.

  LCM (lossless raw history) is *context*, not memory. It stays (plan 06).

### Related over-engineering: crates the binary compiles but barely uses

`operant-cli`'s **default features** (`agent-runtime`, `gateway`) pull in these crates.
The counts are `crate_name::` references from `operant-cli/src` + `operant-core/src`:

| Crate | Lines of Rust | References from cli/core |
|---|---|---|
| operant-runtime | 79,015 | 3 (`gateway_runner.rs:3932`, two comments) |
| operant-channels | 60,542 | 0 (20+ `channel-*` features: telegram, discord, slack, email, signal, irc, imessage, qq, bluesky, twitter, reddit, notion, …) |
| operant-tools | 46,867 | 1 |
| operant-providers | 37,916 | 1 |
| operant-gateway | 16,659 | 0 |
| operant-memory | 13,593 | 0 |
| operant-hardware | 9,570 | 0 |

About 260K lines (a port of another agent runtime) that cost build time, binary size (26 MB
resident text per instance, plan 07) and audit surface. operant's own gateway lives in
`operant-core/src/gateway/` + `operant-cli/src/gateway_runner.rs`, so the Telegram path
may exist twice. **This must be proven by a build, not by grep**: macros, `pub use`
re-exports and trait impls registered at link time can hide uses. BUGS K-4
(`--no-default-features` doesn't build) is the first blocker.

## Plan

### Phase A: one build entry point that keeps memory-wire current (small; do first)

1. `scripts/build.sh [--release] [--offline]` becomes the **only** documented way to
   build (dev, test gates, release, deploy). It:
   1. runs `cargo update -p memory-wire` on every invocation (no stamp throttle);
   2. prints `memory-wire: <old> → <new>`;
   3. then runs `cargo build`/`test` with the passed args.
   - Offline or failing update: a loud warning, continue with the lock, and exit code 0.
     `--strict` turns that into a failure, and the deploy step uses `--strict`.
   - `check.sh` and `sync-vendors.sh` call into it. Drop the throttle stamp.
2. `operant-core/build.rs` reads `Cargo.lock`, extracts the memory-wire `source` rev and
   exports `OPERANT_MEMORY_WIRE_REV`. `operant --version` prints it, and so does
   `operant doctor`. Doctor also runs `git ls-remote https://github.com/ishan-parihar/memory-wire main`
   (5 s timeout) and reports "behind upstream: yes/no".
3. Pull the lock forward now as part of this phase. Run the full gates, because 18
   upstream commits may change APIs; fix compile breaks in `memory_wire.rs` /
   `org/synthesis.rs`.
4. AGENTS.md + CLAUDE.md: "build with `scripts/build.sh`", replacing raw `cargo build`
   for release/deploy. Update the INDEX conventions accordingly.
5. Test: a script test (`scripts/tests/build_sh_test.sh`, run in CI) with a fake
   `cargo` on `PATH` asserts that `update -p memory-wire` precedes `build`.

### Phase B: one binary updater for sourcehound and aft

- Extract the shared logic from `sourcehound_update.rs` and the aft updater into
  `operant-core/src/binary_updater.rs`:
  - `check(name) → {installed, latest}` (through `gh api`, falling back to the public
    API);
  - download, SHA256SUMS verify and atomic replace;
  - **prune to the newest 2 versions** (aft cache today: 14);
  - a per-process once-guard, plus a 6 h on-disk throttle
    (`~/.operant/cache/updates.json`).
- Triggers: the first use per session (as today), plus the gateway daemon checking
  every 6 h.
- `operant doctor` and `/version` list each integration's installed and latest version.
  `operant update --integrations` forces a check now.
- Failure policy: never block a tool call, but surface "update available / update failed
  (reason)" once per session as a status notice (not only a debug log).
- Tests: a fake `gh` and a fake release directory cover the
  verify/replace/prune/throttle paths, and a corrupted checksum is rejected with the old
  binary kept.

### Phase C: purge the alternatives (web search, browser, memory)

Do it in this order, one iteration each. Every removal keeps a **config migration**: a
removed key or value parses with a one-time warning naming the replacement, and never
fails `deny_unknown_fields` loading of an existing user config. Then delete the key from
`operant.example.toml`.

1. **Web search = sourcehound only.**
   - Delete `web_providers/{ddg,exa,searxng,tavily}.rs` and the provider selection in
     `web_tools.rs`.
   - Delete `[tools.web] search_url`, `preferred_provider` and the provider API-key
     fields.
   - `web_search`/`web_extract`/`web_crawl` call sourcehound. If the binary is missing,
     the tool error says `run: operant update --integrations` and doctor flags it.
2. **Browser = sourcehound only.**
   - Delete the `camofox`, `browserbase`, `browser-use` and `firecrawl` providers and
     `browser_camofox_state.rs`, and the factory.
   - `[browser] provider` becomes ignored-with-warning (the user's `"obscura"` included).
   - Keep `sourcehound_cdp.rs` and `browser_cdp_tool.rs` if they are the sourcehound CDP
     path; delete them otherwise.
3. **Memory = memory-wire only.**
   - Delete `BuiltinProvider` and the `agentmemory` aliases. The provider is no longer
     configurable (`[memory] provider` ignored with a warning; `agentmemory_auto_spawn`
     removed).
   - **One-time importer:** on first start after upgrade, if `MEMORY.md`/`USER.md`
     (MemoryManager files) exist, retain each entry into memory-wire (user bank or
     project bank, see phase E), then rename the files to `*.imported`. Idempotent:
     record a marker.
   - Delete the `<long_term_memory>` block (`run.rs:2313-2321`). Memory reaches the
     prompt only through the provider (plan 06 P0/P4).
   - The `memory_store`/`memory_recall`/`memory_search` tools map to memory-wire
     (`retain_tagged`/`recall_with_weights`). Remove duplicates.

### Phase D: the unused-crate audit (prove, then delete)

1. Fix K-4 so `cargo build -p operant-cli --no-default-features` compiles.
2. Build `--no-default-features` and run the gates plus the **gateway smoke test** (the
   systemd gateway with the user's Telegram setup, using a test chat: send `/status` and
   a 1-turn prompt). Whatever breaks names the real dependencies on the heavy crates.
3. For each real dependency, either move the needed code into `operant-core`/`operant-cli`
   (if small and actually ours), or keep the crate and record why in `docs/CRATES.md`.
4. Delete crates with no remaining dependency. Remove their workspace entries and
   features. Expected candidates: `operant-channels` (if operant's own Telegram gateway
   is the live one), `operant-memory` (after phase C3), `operant-hardware`,
   `operant-tool-planning`, and most of `operant-runtime`/`operant-tools`/`operant-providers`.
   **Don't guess:** each deletion is its own iteration, and its commit message lists the
   proof (build plus tests plus gateway smoke).
5. Measure before and after: release binary size, `cargo build --release` time, idle RSS
   (plan 07). Record them in CHANGELOG.

### Phase E: one memory model for solo use and the agentic-OS organisation

The layers in Evidence become one memory-wire `MemoryService` per process (per machine,
once plan 07 phase 2 moves it into the daemon), with **banks** as the only partitioning:

| Bank | Holds | Written by | Read when |
|---|---|---|---|
| `user` | personal preferences and facts across projects | memory tools (default for solo), importer (`USER.md`) | always (weight 0.6) |
| `project:<repo-id>` | project facts and decisions | memory tools (when cwd is a repo), historian facts (plan 06 P3), importer (`MEMORY.md`) | when cwd is that repo (weight 1.0 solo) |
| `org` | org-wide decisions and synthesis digests | `org/synthesis.rs` (chief-of-staff) | in org mode (weight 0.6) |
| `emp:<employee-id>` | **role-specific memory** of a seat/employee | that seat's memory tools; replaces the seat memory *file* | when acting as that seat (weight 1.0) |

1. **A single handle.** `org/synthesis.rs` stops opening its own `SqliteStore`; it
   receives the shared `MemoryService` (the same one `MemoryWireProvider` holds). Two
   writers on one SQLite file inside one process are fragmentation and lock contention.
2. **Identity-scoped recall.** `MemoryWireProvider::prefetch` resolves the bank set from
   the active identity (org seat → `emp:<id>` + `org` + project; solo → project + `user`)
   and fuses the results by weight. Writes default to the most specific bank. A
   `scope: "org" | "project" | "user"` argument on the memory tools lets the agent write
   wider, and org writes require the seat's authority (`org/seat_authority.rs`).
3. **Seat memory files → `emp:` bank.**
   - Migrate each seat file's entries into `emp:<id>` once (with a marker).
   - The charter postscript (`org/cast.rs:114-125`) changes from "you keep a private
     memory file" to "your role memory is recalled into this block; use memory_save to
     add to it".
   - The injection block (`## Your seat memory`) is rendered from the `emp:` bank (pinned
     plus recent plus recalled), within the same budget as before.
4. **Bank naming is one function** (`memory_banks.rs`: `bank_for(identity, scope)`),
   used by the provider, the org layer, the importer and the tools, and covered by tests.
   No other code builds bank ids.
5. Tests:
   - A seat agent's recall includes its `emp:` facts and `org` facts but never another
     seat's `emp:` bank.
   - Solo recall never includes `emp:*`.
   - Synthesis and a concurrent agent turn write without `SQLITE_BUSY` (shared handle).
   - The importers are idempotent.

## Order and size

A (S) → B (M) → C1 (S) → C2 (S) → C3 (M) → E (M) → D (L, many small iterations). Phase D
is last because phases C and E remove most of the reasons the heavy crates exist.

## Acceptance

- Every build path (`scripts/build.sh`, deploy) compiles the current memory-wire `main`.
  `operant --version` shows its rev, and doctor shows "not behind".
- `operant doctor` lists sourcehound, aft and memory-wire with installed and latest
  versions. The aft cache holds at most 2 versions.
- The config, README and example toml have no search-provider, browser-provider or
  memory-provider choice. Existing user configs still load (with migration warnings).
- One memory-wire handle per process. Org and seat memory live in banks, and no seat
  memory files remain after migration.
- Phase D: the crate list in `docs/CRATES.md` matches the workspace, and each kept crate
  has a reason. Binary size, build time and RSS before/after are recorded.

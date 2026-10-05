# AGENTS.md

> **READ THIS FILE FIRST.** Every agent (human or AI) working in this repository
> must read this document top-to-bottom before making any change. It is the
> single source of truth for how to work here.

---

## START HERE — Development Protocol

This repository is operated under a strict **iteration model**. Every change —
no matter how small — is one iteration, and every iteration follows the same
five-step loop. Skipping a step is a protocol violation.

### The Iron Rule

> **ALWAYS PUSH COMMITS AFTER EACH ITERATION.**
>
> After every iteration — without exception — you must:
>   1. commit the change locally, and
>   2. push the commit to `origin/main`.
>
> The user pulls from `origin/main` to test on their local system. **A change
> that lives only in your working tree is not done.** If `git log` on
> `origin/main` does not show your iteration, you have not delivered it.

### Iteration Loop (mandatory, in order)

```bash
# 0. (only at the start of a session) sync to latest
cd /home/z/my-project/operant
git pull --ff-only

# 1. Make the change (surgical, see "Behavioral Guidelines" below).
#    - Touch only what the task requires.
#    - Match existing style.
#    - Do not "improve" adjacent code.

# 2. Verify locally with the dev-env applied.
source /home/z/my-project/operant/scripts/dev-env.sh
cargo fmt --all
./scripts/check.sh check -p operant-core --lib            # fast: crate being edited
./scripts/check.sh check -p operant-cli --bin operant      # if CLI touched
cargo test -p <crate> --lib -- <test_filter>               # tests for the changed area
cargo clippy -p <crate> --all-targets -- -D warnings       # clippy on touched crate

#    Only when the above is green, do a final workspace verify (slow):
#      ./scripts/check.sh check --workspace
#      ./scripts/check.sh test  --workspace

# 3. Stage the change (be deliberate — do not blindly `git add -A`).
git add <specific files>

# 4. Commit with a structured message: <type>(iter-N): <subject>
#    - type: feat | fix | refactor | docs | chore | test | perf | security
#    - iter-N: monotonically increasing iteration number
#              (look it up on origin/main AFTER a `git fetch origin`,
#               then +1 — see "Iteration Numbering" below. Local history
#               is stale and reading it races concurrent agents)
git commit -m "feat(iter-70): short imperative subject" \
           -m "Body: what changed and why. Reference files and line numbers."

# 5. PUSH — non-negotiable.
git push origin main

# 6. Confirm the push landed.
git log origin/main -1 --oneline    # must print your new commit

# 7. Build release binary and deploy to global executable (R&D protocol).
cargo build --release -p operant-cli
# Deploy to BOTH install paths (2026-10-05 lesson): the user's PATH and the
gateway systemd unit resolve ~/.local/bin/operant; /usr/local/bin is the
# convention target. A running daemon holds the old inode open — `cp` fails
# with "Text file busy"; rename-over works: build to a temp name, then `mv`.
cp target/release/operant /usr/local/bin/operant.new && sudo mv /usr/local/bin/operant.new /usr/local/bin/operant
cp target/release/operant ~/.local/bin/operant.new && mv ~/.local/bin/operant.new ~/.local/bin/operant
operant --version   # confirm the deployed binary matches
# Then restart the gateway daemon so it adopts the new inode:
systemctl --user restart operant-gateway
# and verify the RUNNING process actually executes the new binary:
md5sum /proc/$(systemctl --user show -p MainPID --value operant-gateway)/exe
```

### R&D Protocol — Deploy After Every Push

> **After EVERY iteration that touches code (not docs-only):**
> Build the release binary and install it to `/usr/local/bin/operant` so the
> user can immediately test the change. A commit that isn't deployable is not
> done. If the build fails, fix it before moving on.
>
> Docs-only iterations (AGENTS.md, README, CHANGELOG) skip the build step.

### Iteration Numbering

- Find the current iteration number from **`origin/main`, never local history**:

  ```bash
  git fetch origin
  git log origin/main --oneline | rg -o 'iter-[0-9]+' | head -1
  ```

  Local history is stale the moment a concurrent agent pushes, and deriving the
  number from it is a **read-then-write race**: two agents run the lookup at the
  same moment, read the same "last" number, and both compute the same "next".
  This has produced three duplicate labels (iter-516, iter-518, iter-529).
  `origin/main` is the only numbering authority.

- Use the **next** number. Never reuse, never skip.
- Documentation-only iterations still count (e.g. `docs(iter-71): ...`).
- If you do multiple unrelated changes in one session, each is its own iteration
  with its own commit and its own push.

#### Before you push: check nobody took your number

```bash
git fetch origin
git rev-list --left-right --count origin/main...HEAD   # behind=? ahead=?
```

If `ahead` is not exactly 1, or a peer advanced `origin/main` past your base,
or your number is already taken: **`git commit --amend`** with the next free
number, then push.

Run this check **before every push**. Amending an *unpushed* commit is local-only
and safe; amending after a push would require a force-push, which is forbidden —
which is precisely why the check has to happen first.

#### If a duplicate label reaches `origin/main`, do not rewrite history

Two commits sharing a label is a **cosmetic defect, not an incident**. Renumbering
published history is strictly worse than the duplicate: git history is
append-only by design, and an audit that can be rewritten is not an audit. Leave
both commits, note the collision in the next commit body, and carry on.

Force-pushing to "clean up" a number is never the fix.

### Parallel-Agent Coordination Protocol (fleet work — ADDED iter-619, storage doctrine REVISED iter-631 by owner order 2026-10-05)

Multiple agents work this repo at once. The hardest discipline is not
code — it is never destroying a peer's work that lives only in the shared
tree. These rules are load-bearing; every one exists because a specific
incident happened.

1. **The iteration-label authority is `origin/main`, hand-computed.** Fetch,
   read the top label, compute `top+1` yourself (arithmetic in the message,
   never a pipeline that serves the top label back). Push, confirm, then
   `git log origin/main --oneline | grep -c "iter-N"` MUST print 1.
   Duplicate labels are append-only history — you lose the race, you
   land with the next free number.
2. **The main working tree may carry a peer's uncommitted WIP and even
   their local-only WIP commits.** NEVER `git reset --hard`, `git clean`,
   `checkout` over it, or update-ref its branch without preserving first.
   Before ANY ref move: `git branch fleet-hold-<label>-wip <sha>` for every
   unpushed-looking commit (a peer's "restore point" commit on a pushed
   label's back is THEIR recovery tool; orphan it and you ate their work).
3. **Never `git add -A`.** Stage explicit paths. Overlap-check before push:
   `git status --porcelain -- <your paths>` must show nothing of the
   peer's; `git log <your-base>..origin/main --oneline` + diff-name-only vs
   your staged set must show zero file overlap, or coordinate first
   (peers message via `agent://<name>` when slices share files).
4. **Integration happens in the SHARED main tree** (owner order 2026-10-05:
   per-worktree `target/` caches measured 6–62 GB and the box is
   storage-critical — the fleet now builds against ONE shared `target/`):
   work your slice in the main tree, stage explicit paths only (rule 3),
   push from it. A dirty tree does NOT block a `git merge --ff-only
   origin/main` catch-up — git refuses the ff if the incoming range touches
   a dirty tracked file; VERIFY first:
   `git diff --name-only <base> origin/main` ∩ `git status --porcelain`
   must be empty (a `Cargo.lock` intersection is fine — the lock is derived:
   `git checkout -- Cargo.lock`, never stash-merge it, the next build
   regenerates it). Exception: a TEMPORARY worktree is allowed only for a
   clean release build when a peer's dirty WIP blocks the bin compile —
   `df -h` before/after, `git submodule update --init` at creation, and
   `git worktree remove` immediately after the artifact is out.
5. **The shared `target/` is the fleet's single build cache — do not
   casually delete it.** Do NOT `rm -rf target/` (that destroys the
   release binary and the fleet's warm cache). Clean debug debris ONLY
   when free disk drops under 20 GB: `rm -rf target/debug/deps
   target/debug/build target/debug/incremental` (keeps `target/release`).
   Never share a `target/` via symlink while two builds can run
   concurrently — build-script fingerprints race (observed: ring/
   aws-lc-sys/sqlite failing with incoherent errors).
6. **Compute budgets: scoped builds only, always** (`-p <crate>` — see
    Compile Strategy). `df -h /` before any build when free disk < 30 GB.
    Build failures saying `ar: unrecognizsed subcommand 'cq'`: check
    `which ar` — a foreign CLI has twice (~/.local/bin/ar) shadowed
    binutils; rename it to `ar-cli.shadow-N` instead of debugging cargo.
    Kernel-sidecar tests (`tools::kernel::*`) pass ONLY in the main tree:
    a fresh checkout/worktree lacks the untracked `kernel-sidecar/.venv`
    and the uninitialized `vendor/prime-agent` submodule — `ping_roundtrip`
    fails `has_runtime: false` there. Run them in the main tree or not at
    all; never treat that failure as a tip regression outside the main
    tree.
7. **If the tip is red from a peer's in-flight subsystem, don't fix it and
   don't block on it.** Prove your delta adds zero (error-count before/after
   on the untouched base vs patched), say so in the commit body, push.
   Fixing a peer's subsystem mid-flight from another slice is how
   oscillation happens.
8. **A slice is yours or nobody's.** Git branches sound shared but labour is
    not: never amend, rebase, or delete a branch/worktree you did not
    create; `fleet-hold-*` naming makes ownership legible in
    `git branch`/`git worktree list`.
9. **Stash mechanics are all-or-nothing and untracked-unaware.**
    `git stash push -- <paths>` FAILS ENTIRELY if any pathspec names an
    untracked file (no partial stash is created) — to preserve a peer's
    full WIP use `git stash push -u -m "peer-wip <why>"`, never bare
    pathspec lists from `git status --porcelain`. Untracked files never
    block a ff merge unless the incoming range adds a file at the same
    path. When in doubt whether a stash was created: `git stash list`
    BEFORE any further ref move — a failed stash plus a merge is how WIP
    silently rides through (observed 2026-10-05: the stash failed, the
    `;`-chained merge ran, and only the ff's path-disjointness saved the
    peer's 40-file WIP).

10. **Long-running legs push WIP checkpoints to `origin/<label>-wip`.**
    Any leg expected to run for hours (or driven by a subagent) pushes
    its uncommitted state to a backup branch on origin at every
    meaningful milestone — `git push origin HEAD:fleet-<label>-wip` —
    with commit messages labeled `WIP(...) [do not deploy]`. The WIP
    branch belongs to the agent running the leg: do not push or delete
    a peer's `*-wip` branch (rule 8 applies). WIP commits NEVER land
    on main; the successor resumes by cherry-picking the backup branch,
    squashing into one green iteration, and deleting the branch.
    (Observed 2026-10-05: the wt-wave1 worktree was deleted mid-leg by
    a concurrent storage sweep; the leg survived only because its
    checkpoint lived on `origin/fleet-wave1-wip`.)
11. **Storage cleanup must not eat in-flight work.** Before removing any
    `wt-*` directory, target cache, or running a storage sweep:
    `git worktree list`, then per worktree `git -C <wt> status
    --porcelain` and `git -C <wt> log --oneline -1` vs origin branches.
    A worktree whose tree is dirty or whose HEAD is ahead of every
    origin branch is a peer's in-flight leg — coordinate (rule 3) or
    leave it; never prune blind. The owner-ordered worktree retirement
    (iter-631) removed the *pattern*, not the duty to check.

### What Counts as an Iteration

- A bug fix → one iteration.
- A new feature → one iteration.
- A refactor (even mechanical) → one iteration.
- An AGENTS.md / README / CHANGELOG update → one iteration.
- A test sweep that produces no code change → typically **not** an iteration;
  just push the results as `chore(test-sweep): ...` if anything was added.

### When You Cannot Push

If `git push` fails:

1. **Network/auth error** — surface it to the user immediately. Do not silently
   keep working on top of an unpushed commit; the next iteration will compound
   the divergence.
2. **Non-fast-forward (someone else pushed)** — run `git pull --rebase origin main`,
   resolve conflicts, then push again. Do not force-push to `main`.
3. **Working tree dirty from build artifacts** — never commit `target/`. The
   `.gitignore` already excludes it; if you see it staged, unstage it.

### Pre-Existing Test Artifacts (do not commit)

The repo root contains SQLite test files (`test_db.sqlite`, `test_db_resp.sqlite`,
`test_db_native_xml.sqlite`, etc.) that are generated by the test suite. They
are git-ignored — **never** `git add` them. If they appear in `git status`, the
gitignore is doing its job; leave them alone.

---

## Operant Project Context

- Current release line: `0.2.1`
- Runtime config is TOML-first and shared through `crates/operant-core/src/config.rs`
- Rich CLI/TUI uses `ratatui` and lives under `crates/operant-cli/src/tui/`
- Autonomous coding mode lives in `crates/operant-cli/src/autonomous.rs` and is
  launched through `operant autonomous` or `operant run --autonomous`
- Repo-root `TODO.md` is the task ledger for autonomous mode; keep `Implemented`
  and `Pending` accurate when autonomous behavior changes
- Autonomous runtime writes repo-local `autonomous-status.toml` state and
  reloads repeated-failure pause state across restarts; keep that workflow
  documented when changing autonomous behavior
- The workspace view has `Conversation`, `Reasoning`, `Activity`, and management
  panels for `MCP`, `Skills`, and `Behavior`
- When config fields change, update `operant.example.toml` in the repo root in
  the same change
- When user-facing behavior changes, update `README.md`, `CHANGELOG.md`, and
  screenshots in `assets/` if the UI changed materially
- Tagged releases are created from `CHANGELOG.md` and built on the LOCAL
  machine — never on GitHub. The Actions quota is exhausted, so every
  workflow is `workflow_dispatch`-only: no push and no tag triggers anything.
  Release procedure: write the dated `## [X.Y.Z]` section, commit, push the
  tag `vX.Y.Z` (marks the release, builds nothing), then locally
  `cargo build --release -p operant-cli`, package the artifacts, and publish
  with `gh release create vX.Y.Z --notes-file <extracted section> <artifacts>`.
  Do NOT re-add push/tag triggers without quota to spend.
- GitHub Actions RUNS are banned — not reads. No workflow runs, no dispatch,
  no push/tag triggers: the quota is exhausted and every run burns minutes.
  Read-only API queries (`gh run view` / `list`, log fetching) cost zero
  quota and are PERMITTED for failure diagnosis. ALL verification (`check` /
  `test` / `clippy` / `fmt` / `doc`) and ALL release builds run on the LOCAL
  machine only. The permitted GitHub operations are git itself
  (pull/push/tag), `gh release create` for publishing locally-built
  artifacts, and read-only run/log inspection.
- Preferred verification commands (always run via `./scripts/check.sh`):
  - `cargo fmt --all`
  - `cargo check --workspace`
  - `cargo test --workspace`
  - `bash scripts/clippy-warning-gate.sh` (incremental clippy gate with
    `-D clippy::unwrap_used -D clippy::expect_used` — see "Local Compilation Protocol")
  - `cargo check --workspace --all-features` (all feature combos now compile —
    see "Local Compilation Protocol" for the fixed inventory)

---

## Design Preferences (DO NOT CHANGE)

These are the user's intentional design choices. Do not replace, remove, or
"improve" them. New AI agents working on this repo must respect these defaults:

### Voice / TTS
- **Default: Kokoro** (`tools/tts_tool.rs`)
- Kokoro engine is loaded lazily via `kokoro-tiny::TtsEngine`
- Default voice: `af_sky`
- Config: `config.tts.provider = "kokoro"` in `operant.example.toml`
- Do NOT switch to Edge TTS, OpenAI TTS, or any other provider as the default

### Memory
- **Default: memory-wire** (`memory_wire.rs`) — in-process memory engine
  (cargo git-dependency, branch `main`) replacing agentmemory; `retain` /
  `recall` over the `memory_wire` sync API with `catch_unwind` degrading
  to a memory miss instead of killing the turn
- `MemoryWireProvider` implements the `MemoryProvider` trait; degrades gracefully
  (empty prefetch / no-op sync) instead of failing the loop
- `BuiltinProvider` (file-backed MEMORY.md/USER.md) is the zero-dependency fallback
- All legacy providers (TDG, Hindsight, RetainDb, Mem0, LocalVector, agentmemory)
  were REMOVED; unknown provider names silently downgrade to builtin
  (see `build_memory_provider`)
- Config: `config.memory.provider = "memory-wire"` (default) | `"builtin"`

### Browser & Web Tools
- **Default: sourcehound** — `sourcehound` binary driven over its MCP server
  (stdio), serving `web_search` + `web_scrape` + `web_extract` + `web_crawl`
  and the `sourcehound` browser provider (`cloakctl.navigate` / `cloakctl.read` /
  `cloakctl.act`, raw CDP via `cloakctl.cdp`-published endpoint)
- All web tools degrade to a helpful error when the `sourcehound` binary is missing
  (`SOURCEHOUND_BINARY` override or PATH)
- Other backends still available: lightpanda, camofox, browserbase, browser-use,
  firecrawl (config `[browser] provider`)
- Do NOT switch the default away from sourcehound

### Platform Adapters (Gateway)
- **Supported: 7 platforms** in the gateway registry (`platform_registry()` in
  `crates/operant-cli/src/gateway_runner.rs`) — telegram, discord, slack,
  whatsapp, email_smtp, sms_twilio, webhooks
- **Working adapters**: Telegram, Discord, Slack (fully implemented in `gateway/mod.rs`)
- **Stub adapter**: Webhook (needs HTTP server implementation)
- **Config-only**: WhatsApp, Email, SMS (setup wizard + config flags exist,
  adapter code TBD)
- 20 phantom platforms were purged in iter-50 (matrix, mattermost, signal, etc.)
- Do NOT re-add purged platforms without a real adapter implementation
- **The gateway registry is NOT the whole channel surface.** `operant-channels`
  carries 31 `channel-*` features, 21 of them on by default (discord, slack,
  signal, mattermost, irc, imessage, dingtalk, qq, bluesky, twitter, reddit,
  notion, linq, wati, nextcloud, mochat, wecom, clawdtalk, webhook,
  whatsapp-cloud, email); `channel-telegram` is opt-in behind the
  `agentmemory`-independent `channel-telegram` feature. The preference above
  scopes to the *gateway registry*; it is not a claim that the workspace has 7
  messaging surfaces.

### Native Tool Integrations
- **AFT (Agent File Tools)**: 15 IDE-grade coding tools via subprocess
  (`aft_bridge.rs` + `aft_tools.rs`)
  - Auto-downloads from GitHub releases, auto-updates
  - When `aft_enabled=true`, basic file/terminal tools are auto-disabled (no duplication)
  - Feature flag: `aft_enabled` in config
- **sourcehound (Intelligence Gathering System)**: `web_search`, `web_scrape`,
  `web_extract`, `web_crawl` + sourcehound browser provider (`tools/sourcehound.rs`
  + `browser_provider.rs`), driven over the `sourcehound` binary's MCP server
  (stdio). No API keys required
- **LifeOS**: 22 Notion-backed holonic life-management tools (`lifeos_tools.rs`)
  - Feature flag: `lifeos` cargo feature + `lifeos_enabled` in config
  - Requires `NOTION_API_TOKEN` env var

### Context Management
- **Tiered eviction + decay curve** ported from `cortexkit/magic-context`
  (`context_management.rs`)
- T3 (tool results) evicted first, T2 (reasoning) second, T1 (user/assistant) last
- Recency reserve scales with context window: `budget/4096` clamped to [6, 50]
- Prompt-cache stability: system prompt split into frozen prefix (base + skills)
  + volatile suffix (memory + workspace)

---

## Architecture Overview

```
operant/
├── crates/
│   ├── operant-core/          # Core library (no CLI/TUI)
│   │   ├── src/
│   │   │   ├── agent/         # Agent loop, model clients, fallback
│   │   │   │   ├── mod.rs     # OperantAgent — run(), execute_tools(), process_stream()
│   │   │   │   ├── clients/   # OpenAI, Anthropic adapters
│   │   │   │   └── fallback.rs # FallbackModelClient
│   │   │   ├── tools/         # Tool registry + all tool implementations
│   │   │   │   ├── builtin.rs # register_builtin_tools()
│   │   │   │   ├── sourcehound.rs # web_search/scrape/extract/crawl + MCP seam (subprocess)
│   │   │   │   ├── aft_tools.rs   # 15 AFT IDE tools
│   │   │   │   └── memory_tools.rs # memory_* tools (memory-wire/builtin)
│   │   │   ├── memory_wire.rs # MemoryWireProvider (in-process, catch_unwind)
│   │   │   ├── memory_provider.rs # memory-wire + BuiltinProvider
│   │   │   ├── context_management.rs # Tiered eviction + decay curve
│   │   │   ├── aft_bridge.rs  # AFT subprocess + auto-update
│   │   │   ├── gateway/       # Platform adapters (Telegram/Discord/Slack/Webhook)
│   │   │   ├── mcp.rs         # MCP client (HTTP + Stdio + SSE)
│   │   │   └── config.rs      # AppConfig, BehaviorSettings, ToolSettings
│   │   └── Cargo.toml         # Features: anthropic (memory-wire git-dep baked in; sourcehound is a spawned binary, not a dep)
│   └── operant-cli/           # CLI + TUI
│       ├── src/
│       │   ├── main.rs        # CLI entry point, Clap enum, agent setup
│       │   ├── tui/           # 50+ TUI modules (app, render, prompt_input, dialogs)
│       │   ├── gateway_runner.rs # Gateway mode agent handler
│       │   ├── autonomous.rs  # Autonomous coding mode
│       │   ├── config.rs      # CLI config
│       │   ├── cmd_*.rs       # CLI subcommand handlers
│       │   └── dashboard_server.rs # axum dashboard backend
│       └── Cargo.toml         # Features: agent-runtime, gateway, channel-* (21 by default), hardware, plugins-wasm, anthropic, ci-all
├── scripts/
│   ├── dev-env.sh             # Source this before any cargo command
│   ├── check.sh               # Wrapper that applies dev-env + cargo
│   ├── self-test.sh           # Build + test + clippy + fmt sweep
│   └── provision-build-deps.sh # One-time dependency provisioning
├── Cargo.toml                 # Workspace members (17 crates, self-contained)
├── operant.example.toml       # Config template (7 gateway platforms; 21 channel-* features default-on)
├── TODO.md                    # Autonomous-mode task ledger
├── BUGS.md                    # Open issues tracker
└── AGENTS.md                  # THIS FILE
```

---

## Path Dependencies

Operant depends on two sibling-project git dependencies (see `Cargo.toml`).
External binaries are spawned, not path deps:

- **memory-wire** — in-process memory engine: `memory-wire = { git =
  "https://github.com/ishan-parihar/memory-wire", branch = "main" }`.
  Advance with `scripts/sync-vendors.sh`; pin `rev =` at release.
- **sourcehound** — MCP-subprocess web/browser engine: `sourcehound_mcp =
  { package = "sourcehound", git =
  "https://github.com/ishan-parihar/sourcehound", branch = "master" }`.
  In-process was measured unviable (their `[patch.crates-io]` evaporates for
  consumers; unconditional render stack). Same sync discipline.

If `cargo check` fails at workspace resolution, it's a workspace-internal
issue, not a missing clone.

---

## Build Environment Setup

The build needs libclang (for bindgen), ONNX Runtime (for kokoro-tiny), and
alsa runtime libs (for cpal). All of these are pre-provisioned under
`/home/z/my-project/local/`. The two scripts under `scripts/` apply the
required env vars.

```bash
# 1. Source the dev env (sets LIBCLANG_PATH, ORT_LIB_LOCATION, etc.)
source /home/z/my-project/operant/scripts/dev-env.sh

# 2. Use the check.sh wrapper (applies RUSTFLAGS + CARGO_INCREMENTAL=0 + dev-env)
/home/z/my-project/operant/scripts/check.sh check -p operant-core --lib
/home/z/my-project/operant/scripts/check.sh test  -p operant-core --lib -- <test_filter>

# 3. Or set env manually (equivalent to dev-env.sh):
source /home/z/.cargo/env
export LIBCLANG_PATH=/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu
export ORT_LIB_LOCATION=/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export ORT_PREFER_DYNAMIC_LINK=1
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"
export PKG_CONFIG_PATH=/home/z/my-project/local/pkgconfig
export LD_LIBRARY_PATH=/home/z/my-project/local/lib:/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export RUSTFLAGS="-L native=/home/z/my-project/local/lib"
export CARGO_INCREMENTAL=0

# 4. For fresh environments, run provision first:
/home/z/my-project/operant/scripts/provision-build-deps.sh
```

### Compile Strategy (read this — it saves 10+ minutes per iteration)

- **Never** run `cargo check --workspace` mid-iteration. A full workspace
  compile takes 10+ minutes on a 2-CPU box.
- **Always** scope cargo commands to the crate being edited:
  - Editing `operant-core` → `cargo check -p operant-core --lib`
  - Editing `operant-cli` → `cargo check -p operant-cli --bin operant`
- Reserve `--workspace` for the **final** verification step right before push.
- If the change is in a `tests/` file, scope to that test target:
  `cargo test -p operant-core --test <name> --no-run`.

### Disk Constraint

`target/debug/` can hit 5–8 GB. Between iterations, clean incremental state:

```bash
rm -rf target/debug/deps target/debug/build target/debug/incremental
```

Do **not** `rm -rf target/` wholesale — that throws away the release binary
and forces a full rebuild of every dependency.

---

## Local Compilation Protocol (READ BEFORE CODING)

**The canonical development loop is compile-locally-first.** Do not rely on CI
(workflows are intentionally left out of scope for now). Every iteration must
pass the full local pipeline before it is committed:

```bash
# 1. Source the dev env (sets LIBCLANG_PATH, ORT_LIB_LOCATION, PKG_CONFIG_PATH...)
source scripts/dev-env.sh

# 2. Format + fast scoped check of the crate you touched
cargo fmt --all
./scripts/check.sh check -p operant-core --lib          # or the crate you edited

# 3. Clippy — via the incremental gate. The gate runs `-D clippy::unwrap_used
#    -D clippy::expect_used` across the workspace (justified sites carry
#    #[expect] escapes — see scripts/expect-annotate.py) and fails only on
#    NEW warnings vs .ci/clippy-allowlist.txt.
bash scripts/clippy-warning-gate.sh

# 4. Tests for the changed area
cargo test -p operant-core                              # or -- <filter>

# 5. Full workspace verification before push
cargo check --workspace
cargo test --workspace
bash scripts/clippy-warning-gate.sh --update           # refresh allowlist if you
                                                        # FIXED warnings (prune entries)
```

### The clippy gate (scripts/clippy-warning-gate.sh)

- Runs `cargo clippy --workspace --all-targets` with `-D clippy::unwrap_used`
  and `-D clippy::expect_used`, and compares the remaining warnings against
  `.ci/clippy-allowlist.txt`.
- **Production code must not call `.unwrap()` / `.expect()`.** Justified sites
  (lock-poison recovery, once-init, invariants) carry an
  `#[expect(clippy::unwrap_used/expect_used, reason = "...")]` attribute — run
  `python3 scripts/expect-annotate.py crates/` (idempotent) after adding any.
- **Passes** when no NEW warnings appear; **reports** stale allowlist entries
  (fixed warnings) so they get pruned.
- `--update` regenerates the allowlist from current warnings — use it after
  fixing warnings, never to hide new ones.
- Test targets are exempted (`#![cfg_attr(test, allow(...))]` in lib/main files,
  `#![allow(...)]` headers in `tests/` dirs).
- Note: manifest-level `[workspace.lints]` inheritance is unusable in this
  environment (this cargo build rejects `lints` manifest keys as unused), so
  enforcement lives in the gate script's `-D` flags.

### `--all-features` now compiles (was broken) — use it freely

All previously-broken feature combinations are fixed (2026-08-03):

| Crate | Broken feature | Fix |
|-------|---------------|-----|
| operant-tools | `probe` | feature + dead code removed |
| operant-core | `anthropic` | temp released before send (non-Send guard across .await) |
| operant-gateway | `schema-export` | feature-gated import; `build.rs` registers check-cfg |
| operant-runtime | observability (`otel`, `prometheus`) | features wired to real deps (opentelemetry 0.27 / prometheus 0.14); otel.rs adapted; tests use `#[tokio::test(flavor = "multi_thread")]` |
| operant-hardware | `hardware` | firmware/ assets committed; vendor-SDK modules (nusb, probe-rs, tokio-serial, aardvark-sys, rppal) gated behind the undeclared `hardware-vendor` cfg (see build.rs) |

`cargo check --workspace --all-features` and per-crate `--all-features` are
validated to compile with **0 errors / 0 warnings** (see the G4 gate battery in
`docs/RUST_BEST_PRACTICES_PLAN.md`).

**operant-hardware `hardware` feature**: enables the self-contained modules
(pico_code, datasheet, firmware-embedded arduino/uno-q peripherals). The
vendor SDK modules (USB discovery, serial transport, probe-rs introspect,
Aardvark I2C/SPI, RPi GPIO) were never wired and stay gated behind the
*undeclared* `hardware-vendor` cfg so `--all-features` stays green; wire them
properly (declare deps + rename cfg) when the SDKs are available.

### The prompt_input/ decomposition (LANDED)

`crates/operant-cli/src/tui/prompt_input/` was decomposed from a 5,305-line
monolith into focused sub-modules (`kill_ring`, `typeahead`, `vim`; commits
dfcefae8 + ea70322c) and compiles clean with zero warnings. Treat it as normal
shipped code.

### Memory error handling (typed, anyhow-free)

`operant-memory` uses a **typed `Error`** (`crates/operant-memory/src/error.rs`,
`thiserror`-style) with `Result<T>` = `Result<T, Error>` and a `MemoryContextExt`
extension that mirrors `anyhow::Context`. `anyhow` was fully removed from the
crate (including dev-deps). The cross-crate contract lives in `operant-api`:

- `MemoryResult<T> = Result<T, MemoryError>` — the `Memory` trait seam
  (re-exported from `operant-memory` root).
- Backends (sqlite/qdrant/postgres/lucid/markdown/...) return the crate-local
  typed `Result`; trait impls convert into `MemoryResult` via `From`.
- Consumers keep `?` into `anyhow`-typed functions via `anyhow::Error::from`
  blanket conversions (the typed error is concrete, so `?` just works).

When touching memory code: return the typed `Error`, not `anyhow`; never add
`anyhow` back as a dependency of `operant-memory`.

---

## Known Gaps (from hermes-agent contrast audit)

**All hermes-agent gaps closed.** The operant project now has feature parity
with hermes-agent's core functionality (excluding Python-specific plugins).

### 2 minor CLI stubs (not bugs, just incomplete features)
- `operant cron run <id>` — prints "Manual execution via CLI is not yet implemented"
- `operant dashboard --stop` — prints "kill the process" (stop via signal instead)

These are tracked, low-priority items. Do not "fix" them unless explicitly
asked — the user has accepted them as documented gaps.

### Gaps CLOSED (15 total — all verified by grep + compile + tests)
- ✅ ~~MCP SSE transport~~ — McpSseClient with background reader task + oneshot
  response routing (iter-68)
- ✅ ~~Credential rotation~~ — CredentialPool restored + PooledCredential with
  OAuth fields (iter-66)
- ✅ ~~Platform registry~~ — `platform_registry()` with factory pattern
  replaces if/elif chain (iter-66)
- ✅ ~~MCP sampling/elicitation handlers~~ — server-initiated requests handled
  in stdio transport (iter-66)
- ✅ ~~`/steer` directive~~ — steer queue + drain between iterations (iter-65)
- ✅ ~~Context compression on overflow~~ — auto-compress via context_management
  on context_overflow errors (iter-63)
- ✅ ~~Hook system~~ — HookRegistry with 6 events + wildcard, wired into agent
  loop (iter-61/62)
- ✅ ~~Error recovery 3 vs 22~~ — 12 error classes with ClassifiedError (iter-61)
- ✅ ~~Sequential tool execution~~ — concurrent 8-worker pool (iter-56)
- ✅ ~~7 dead TUI dialogs~~ — elicitation/onboarding/file_injection/invalid_config/
  memory_update_notification/overage_upsell/desktop_upsell_startup deleted
  (iter-58). The remaining 7 dialogs (ask_user/bypass_permissions/custom_provider/
  device_auth/free_mode/import_config/key_input) are alive via indirect `.open()`
  calls from the connect_dialog handler.
- ✅ ~~Iteration budget grace call~~ — summarize instead of hard-stop (iter-57)
- ✅ ~~WebhookAdapter stub~~ — real HTTP server with HMAC (iter-54)
- ✅ ~~WhatsApp/Email/SMS adapters~~ — real implementations (iter-54)
- ✅ ~~Anthropic cache_control~~ — breakpoints on system prompt (iter-54)
- ✅ ~~Gateway clear_history every message~~ — session caching fix (iter-49)

---

## What IS Working (verified functional — iter-68 audit)

### CLI (52 command variants — real handlers, except the two accepted stubs below)
run, chat, autonomous, tools, test, config, sessions, mcp, skills, model,
completion, cron, kanban, gateway (16 sub-actions), checkpoints, memory,
profile, auth/login/logout, version, doctor, status, dump, logs, backup,
import, uninstall, update, insights, webhook, debug, plugins, curator,
setup, acp, dashboard, trajectory, architecture, channel, context, cookies,
hardware, hooks, migrate, pause, peripheral, resume, service, sop,
suggestions, tui

### Gateway (7 platform adapters — all have real code, platform registry)
- Telegram: fully implemented (long-poll + Bot API)
- Discord: fully implemented (Gateway WS + REST API)
- Slack: fully implemented (Socket Mode + Web API)
- WhatsApp: implemented (Cloud API outbound + webhook inbound)
- Email: implemented (SMTP outbound + webhook inbound)
- SMS: implemented (Twilio API outbound + Twilio webhook inbound)
- Webhooks: implemented (axum HTTP server + HMAC validation)
- Platform registry: factory pattern replaces if/elif chain (iter-66)

### TUI
- Entry point works (`TuiApp::enter` → ratatui + crossterm)
- Permission dialog system works (iter-20: real prompts instead of auto-approve)
- TUI bridge routes events (`bridge.rs`: 111 lines)
- Prompt-cache stability: frozen prefix + volatile suffix (iter-39)
- Session caching: `clear_history` only on session change (iter-49)
- Anthropic `cache_control` breakpoints (iter-54)
- 7 dead dialogs deleted (iter-58); 7 remaining are alive via `.open()`

### Web Dashboard
- Dashboard server: axum + `TcpListener::bind` + `axum::serve`
  (`dashboard_server.rs`)
- Routes: `/api/status`, `/api/boards`, `/api/health`, `/api/config`,
  `/assets/:filename`, `/`
- Static assets: fonts, JS, CSS served from `crates/operant-cli/src/dashboard/`

### Agent Loop (verified features)
- Concurrent tool execution: 8-worker pool + semaphore (iter-56)
- Iteration budget grace call: summarize instead of hard-stop (iter-57)
- Context-overflow auto-compression: classify + manage_context + retry (iter-63)
- `/steer` directive: real-time user steering between iterations (iter-65)
- Hook system: `AgentStart`/`AgentEnd` events wired into `run()` (iter-61/62)
- Error recovery: 12-class `ClassifiedError` with `should_compress`/`should_fallback` (iter-61)
- `sync_turn`: auto memory write-back after each turn — memory-wire
  `retain` (was agentmemory `/agentmemory/remember` pre-integration)
- Credential rotation: `CredentialPool` with `PooledCredential` + OAuth refresh (iter-66)
- MCP sampling/elicitation: server-initiated request handlers in stdio (iter-66)
- MCP SSE transport: `McpSseClient` with background reader + oneshot routing (iter-68)

### Memory (memory-wire — deeply integrated)
- `MemoryWireProvider` (in-process `memory_wire` crate, `catch_unwind` boundary)
- `recall_with_weights` retrieval in `prefetch`; `sync_turn` → `retain`
  (agent loop auto-calls after each turn)
- `BuiltinProvider` fallback (file-backed MEMORY.md/USER.md)
- Legacy providers (TDG/Hindsight/RetainDb/Mem0/LocalVector/agentmemory)
  removed — unknown provider names downgrade to builtin silently

### Context Management
- Tiered eviction (T3→T2→T1 oldest-first, iter-37/38)
- Decay curve (H = H50·2^((I-50)/D)/max(p,0.10), iter-38)
- Recency reserve scales with budget (budget/4096 clamped to [6,50], iter-43)
- FTS5 special character escaping (iter-46)
- Auto-compression on `context_overflow` errors (iter-63)

### Native Tool Integrations
- AFT: 15 IDE-grade tools (subprocess + auto-update, iter-40/41)
- sourcehound: web_search + web_scrape + web_extract + web_crawl + sourcehound browser provider (MCP subprocess, no API keys)
- LifeOS: 22 Notion tools (feature-gated, iter-47/48)
- AFT dedup: basic file/terminal tools auto-disabled when AFT enabled (iter-51)

### Unique Features (operant has, hermes doesn't)
- AFT bridge: IDE-grade coding tools via subprocess with auto-update
- sourcehound bridge: web_search/scrape/extract/crawl + browser provider over MCP subprocess
- LifeOS: 22 Notion-backed holonomic life-management tools
- Context management: tiered eviction + decay curve (ported from magic-context)
- Prompt-cache frozen prefix: Anthropic `cache_control` breakpoints
- Rust safety: memory-safe, no GC, single binary, fast startup

---

## Iteration History (recent)

- 2026-09-27 (production-readiness round, iters 347+): secret files
  created 0600 at open not chmod'd after the write (secrets.rs + wechat.rs,
  matrix.rs form); @agentmemory spawns pinned to 0.9.29 with
  `[memory] agentmemory_version` override (supply-chain); C5 mount cap
  uses cheap `provider_count()` not a full DumpTree per mount; R14-4
  Slack signing-secret finding withdrawn as misread (verification lives in
  the webhook adapter against `webhooks_secret`; dead `_signing_secret`
  field removed); tagged-release pipeline repaired (docs/CHANGELOG.md path,
  0.2.0 section, dead tdg-rust clone dropped).
- 2026-09-27 (deployment-audit closeout, iters 335–345, interleaved with a
  concurrent agent's 2c3c00b9/e716d7f3/a8a11bc5): doctor probes the configured
  endpoint not the provider default — omp omniroute gateway no longer reports a
  false `✗ OpenAI (invalid API key)` (iter-336); gate script self-diagnosis —
  deny violations now reach the allowlist comparison and the gate prints the
  offending site (iter-337, immediately caught iter-332's orphaned import);
  background review no longer poisons WRITE_ORIGIN for the process lifetime —
  task-local TASK_ORIGIN + scope_background_review, mutation-proven regression
  test (iter-338, R39-10 residual); lost 018 CLI-side WIP rebuilt as audit C3 —
  MetricsSnapshot on `operant status --json` + `architecture dump --live`, C6
  test contract repinned (iter-339, R39-12); doctor key scan can no longer send
  a base URL as Bearer token (iter-344, R39-11 companion); concurrent agent's
  EchoTool unit-struct lint fixed to unblock the gate (iter-345). Zeroclaw
  ports 1–2 landed: configurable CacheTtl (iter-340), WhatsApp markdown
  dialect via shared asterisk_dialect (iter-341); ports 3–8 blocked pending
  upstream source access.
- 2026-09-27: R39-9..R39-10 + R39-6 closed — memory stats reads the real
  session count from database.db (iter-330); clippy gate green workspace-wide
  under --all-features, 46 deny-sites annotated, allowlist refreshed, gate
  script -D defect documented (iter-331); background review no longer poisons
  WRITE_ORIGIN for process lifetime, one cross-module origin test lock,
  mutation-proven regression test (iter-332); one-family-per-DB-file invariant
  documented in migrations.rs (iter-333)
- 2026-09-25/26: deployment-audit fixes R39-1..R39-8 (BUGS.md) — lib-test
  compile, doctest link-search via build.rs, SeamUnavailable dark-safe seam,
  write_approval test lock, clippy annotations, cron split-brain →
  cron_db_path(), AFT dead-shim probe; live battery green against omp
  small-stack endpoint; zeroclaw port survey delivered (uncommitted 018
  CLI-side WIP lost to an unscoped checkout during iter-331 prep — recovery
  exhausted, reconstructible from docs/audit/2026-09-02-r16-*.md + c8fc536f)
- 2026-08-02/03: rust-best-practices plan + execution — clippy gate (4-entry
  baseline), 0 lib warnings (core+gateway), --all-targets clean, probe-rs
  vestige removed, anthropic Send fix, workspace fmt normalization (see
  docs/RUST_BEST_PRACTICES_PLAN.md)
- agentmemory integration: TDG memory removed → `AgentMemoryProvider`
  (REST + auto-spawn :3111) + BuiltinProvider fallback (feature-gated)
- igs-rust integration: web_scrape/web_extract + igs browser provider (keyless)
- iter-69: Verify operant with OpenCode API + mimo-v2.5-free model (test only)
- iter-68: MCP SSE transport — all hermes-agent gaps closed
- iter-67: Comprehensive AGENTS.md update — final audit status
- iter-66: Platform registry + MCP sampling/elicitation + credential rotation
- iter-65: `/steer` directive — real-time user steering
- iter-64: Corrected AGENTS.md — 7 dialogs alive via `.open()`
- iter-63: Context-overflow auto-compression
- iter-62: Wire HookRegistry into agent loop
- iter-61: Expanded error recovery + hook system
- iter-60: Update AGENTS.md — 7 gaps closed
- iter-59: Fix GatewayConfig test construction
- iter-58: Delete 7 dead TUI dialogs (~2900 LOC)
- iter-57: Iteration budget grace call
- iter-56: Concurrent tool execution (8-worker pool)
- iter-55: Update AGENTS.md with latest audit
- iter-54: WebhookAdapter + WhatsApp/Email/SMS + Anthropic `cache_control`
- iter-53: AGENTS.md rewrite + gateway status all 7 platforms
- iter-52: Clean `operant.example.toml` + remove phantom token fields
- iter-51: Wire igs/lifeos registration + AFT tool dedup
- iter-50: Purged 20 phantom platforms — keep only 7 supported
- iter-49: Fixed budget_config regression + yuanbao phantom + gateway session caching
- iter-48: LifeOS API alignment — 22 tools compile clean
- iter-47: Integrated igs-rust + lifeos-ops as native tool modules
- iter-46: FTS5 escaping + aft timeout + decay token-consistency
- iter-45: Deleted 9 more dead operant-core modules (~4.9k LOC)
- iter-44: Fixed test counts + memory flush after iter-24/31/42 changes
- iter-43: Bug fixes + remove rand dep
- iter-42: Deleted 17 dead operant-core modules (~12.6k LOC)
- iter-41: Wired 15 aft tools as OperantTool impls
- iter-40: AFT subprocess bridge with auto-update
- iter-39: Prompt-cache stability — frozen prefix + volatile suffix
- iter-37/38: Ported magic-context tiered eviction + decay curve
- iter-33: TDG hooks — auto-`sync_turn` in agent loop
- iter-32: Deepened TDG — HybridRetriever + EntityExtractor + `auto_wire_edges`
- iter-31: Unified TDG pool, gated tools on provider, fixed FTS5 + edge fields
- iter-30: Removed Hindsight/RetainDb/Mem0/LocalVector — TDG-only memory

---

## MVP Deployment (Current Focus)

**Goal**: Make operant deployable and functional for end-to-end testing.
**Config defaults**: memory-wire memory provider, sourcehound browser/web tools, Kokoro TTS (set in `operant.example.toml`).
**Web dashboard**: Copied from operant-agent, wired to axum backend.

### Quick Start for AI Agents

```bash
# 0. Sync to latest main.
cd /home/z/my-project/operant
git pull --ff-only

# 1. Apply dev env.
source scripts/dev-env.sh

# 2. Build the release binary (slow first time, ~5 min incremental).
cargo build --release

# 3. Run tests for the area you're touching.
cargo test -p operant-core --lib
cargo test -p operant-cli  --bin operant

# 4. Smoke-test the CLI.
./target/release/operant --version
./target/release/operant chat            # interactive
./target/release/operant run --query "Hello" --max-iterations 1
./target/release/operant dashboard       # web UI on :3000
```

### Common Development Tasks

**Fix a bug**
1. Read the error message carefully. Reproduce it locally.
2. Find the relevant file in `crates/operant-core/src/` or `crates/operant-cli/src/`.
3. Make the fix (surgical — see "Behavioral Guidelines").
4. `cargo check -p <crate>` to verify compilation.
5. `cargo test -p <crate> --lib -- <filter>` to verify the fix.
6. Commit + push (see "Iteration Loop" — non-negotiable).

**Add a feature**
1. Check whether a similar feature exists in `hermes-agent/` (Python reference).
2. Read the Python implementation for reference, but do not copy idioms.
3. Implement in Rust following existing patterns in this repo.
4. Add tests under `crates/<crate>/tests/` or inline `#[cfg(test)]` modules.
5. Update `CHANGELOG.md` and `operant.example.toml` if config changed.
6. Commit + push.

**Port from Python (hermes-agent)**
1. Find the Python file in `/home/z/my-project/hermes-agent/`.
2. Read and understand the implementation.
3. Create the Rust equivalent under `crates/operant-core/src/`.
4. Follow existing Rust patterns (async/await, `Result`, no `unwrap` in lib code).
5. Add tests.
6. Commit + push.

---

## Behavioral Guidelines

These guidelines reduce common LLM coding mistakes. Merge with the project-
specific instructions above as needed.

**Tradeoff**: These guidelines bias toward caution over speed. For trivial
tasks, use judgment — but the iteration loop (commit + push) is **never**
optional.

### 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them — don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

### 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes,
simplify.

### 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it — don't delete it.

When your changes create orphans:
- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: every changed line should trace directly to the user's request.

### 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:
- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:
```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it
work") require constant clarification.

---

**These guidelines are working if:** fewer unnecessary changes in diffs,
fewer rewrites due to overcomplication, and clarifying questions come before
implementation rather than after mistakes.

---

## Recursive AI Development Mechanism

**Purpose**: Enable AI agents to fix bugs and improve operant without a human
bottleneck. The user pulls from `origin/main` to test each iteration locally,
so the loop only works if every iteration ends with a push.

### How It Works

1. **AI agent reads this file** (AGENTS.md) to understand the project.
2. **AI agent finds a bug** (from `BUGS.md`, a user report, or a test failure).
3. **AI agent fixes the bug** following the guidelines above.
4. **AI agent runs tests** to verify the fix locally.
5. **AI agent commits the change** with a `feat(iter-N)` / `fix(iter-N)` message.
6. **AI agent pushes the commit** to `origin/main` — non-negotiable.
7. **User pulls** on their local system and tests.
8. **Repeat** until the bug is confirmed fixed.

### Issue Tracking

**File**: `BUGS.md` (repo root)

**Format:**
```markdown
# Open Issues

## Critical (Blocks Deployment)
- [ ] Issue 1: [Description]

## High (Affects Functionality)
- [ ] Issue 3: [Description]

## Medium (Enhancement)
- [ ] Issue 5: [Description]

## Low (Nice to Have)
- [ ] Issue 6: [Description]
```

When you fix an issue, **edit `BUGS.md` in the same iteration** that fixes the
code — do not leave it for a follow-up.

### Self-Test Script

**File**: `scripts/self-test.sh`

Runs build + tests + clippy + fmt + CLI smoke tests in sequence. Use this
before pushing if you've touched multiple areas and want one-shot verification.

```bash
./scripts/self-test.sh
```

For targeted work, prefer scoped commands:

```bash
cargo test -p operant-core --lib database
cargo test -p operant-core --lib agent
```

### Development Loop (the canonical form)

```bash
# AI agent development loop — one iteration = one cycle.
while true; do
  # 1. Sync.
cd /home/ishanp/Documents/GitHub/MY-PROJECTS/HERMES/operant
git pull --ff-only

  # 2. Apply dev env.
  source scripts/dev-env.sh

  # 3. Pick a task (from BUGS.md, user request, or test failure).
  #    ... edit code ...

  # 4. Verify locally (scoped — not --workspace unless final).
  cargo fmt --all
  ./scripts/check.sh check -p operant-core --lib
  cargo test -p operant-core --lib

  # 5. Commit with iteration number.
  git add <specific files>
  git commit -m "fix(iter-N): short subject" -m "Body: what + why."

  # 6. PUSH — non-negotiable. The user tests from origin/main.
  git push origin main

  # 7. Confirm.
  git log origin/main -1 --oneline   # must show your commit

  # 8. Wait for the next task / user feedback.
  sleep 60
done
```

### Verification Commands (run before every push)

```bash
cargo fmt --all
./scripts/check.sh check -p <crate>
cargo test -p <crate> --lib
cargo clippy -p <crate> --all-targets -- -D warnings
```

If any command fails, the iteration is **not complete**. Either fix it in the
same iteration or roll back the change — do not push broken code.

### Configuration

**Default config template**: `operant.example.toml` (repo root)
**User config**: `~/.operant/operant.toml`
**API keys**: `~/.operant/.env` (never in the config file, never committed)

To set up a fresh machine:
1. Copy `operant.example.toml` to `~/.operant/operant.toml`.
2. Edit `~/.operant/operant.toml` to set your provider/model.
3. Put API keys in `~/.operant/.env`:
   ```sh
   OPENAI_API_KEY=sk-...
   ANTHROPIC_API_KEY=sk-ant-...
   NOTION_API_TOKEN=secret_...
   ```
4. Run `operant setup` for the interactive wizard (optional).

Example config defaults (already set in `operant.example.toml`):
```toml
[memory]
provider = "memory-wire"  # Default memory provider (memory-wire | builtin)

[tts]
provider = "kokoro"   # Default TTS provider

[agent]
model = "gpt-4"       # Default model (override in user config)
```

---

## TL;DR for New Agents

1. **Read this file first.** Then `git pull --ff-only`.
2. **Every change is one iteration.** Find the next iteration number from
   `origin/main` AFTER a `git fetch origin` — never from local history, which is
   stale and races concurrent agents. See "Iteration Numbering".
3. **Source `scripts/dev-env.sh`** before any cargo command.
4. **Compile scoped** (`-p operant-core --lib`), never `--workspace` mid-iteration.
5. **Verify locally**: `cargo fmt`, `cargo check -p <crate>`, `cargo test -p <crate>`,
   `cargo clippy -p <crate> -- -D warnings`.
6. **Commit** with `feat(iter-N): ...` / `fix(iter-N): ...` / `docs(iter-N): ...`.
7. **PUSH.** `git push origin main`. Then `git log origin/main -1` to confirm.
8. **Do not commit** `target/`, `*.sqlite` test artifacts, or `~/.operant/.env`.
9. **Respect the design preferences** — Kokoro TTS, memory-wire memory,
    sourcehound browser/web tools, 7 platforms in the gateway registry (the
    `operant-channels` crate separately default-enables 21 `channel-*`
    features). Do not "improve" them.
10. **When in doubt, ask.** Pushing back is welcome; silently doing the wrong
    thing is not.

# Gap Register — Operant Agentic-Core (canonical, executor-facing)

**Workspace:** `operant-testing/` (repo `agentic-harness/operant`) · **Reconciled to:** `origin/main` @ `43a38696` (iter-1818)
**Numbering:** one namespace, `G-0xx` (3 digits). Legacy IDs are aliases only — see the alias table at the end.
**Rule for the executor:** a row is `closed-upstream` ONLY when a `CLOSED iter-N` row exists in
`docs/audit/2026-06-19-BUGS.md`. Everything else is `open` or `in-flight`.

## How to read a row

`Status` is one of `open` / `in-flight (this workspace)` / `closed-upstream` / `wontfix`.
`Anchor` is `file:line` **at the reconciled commit** — verify with
`git show 43a38696:<path> | sed -n '<line>p'` before editing; the tree moves.
`Symptom` is the literal string to grep for. `Fix` is the smallest change that closes it.

| ID | Status | Symptom (grep this) | Anchor (file:line @43a38696) | Repro | Fix | Verify |
|---|---|---|---|---|---|---|
| **G-001** | closed-upstream | `Turn ended early` | `crates/operant-core/src/agent/turn_finalizer.rs:150`; `crates/operant-cli/src/tui/app/agent_events.rs:381` | Any turn whose budget runs out | Landed by **iter-1769** (context caps, resume, early-turn recovery) + **iter-1783** (auto-poke, empty-summary branch). Nothing to do. | `cargo test -p operant-core --test auto_poke` |
| **G-002** | closed-upstream | `ran out of room mid-summary` | same arms | Same | iter-1783 split the notice into the two real causes (output cut vs S1-rejected); auto-poke (`operant-core/src/agent/auto_poke.rs`) continues the turn. | `cargo test -p operant-core --test auto_poke` |
| **G-003** | closed-upstream | `delegation that produced nothing` | `crates/operant-core/src/tools/sub_agent_tool.rs:1404` | `delegate_task` on a child turn that ends empty | iter-1800 named the failure ("The sub-agent finished its turn without producing any output"). | `cargo test -p operant-core delegate` |
| **G-004** | **open — deploy gap, not a source gap** | `aft inspect: unknown error` | the aft bridge binary at `~/.operant/aft/aft-v0.63.5/aft`; the source fix landed in `crates/operant-core/src/aft_bridge.rs:570` (the stderr ring, cf. RT-247 CLOSED iter-1799) | `aft_inspect` on any path | The repo source is fixed; the **deployed bridge binary (v0.63.5) predates it**. Re-deploy the bridge; then re-test. | `aft_inspect` on `operant/` must return output |
| **G-005** | **fixed — iter-1822** | `MCP response ID mismatch` | `crates/operant-core/src/mcp.rs:1396` (the stdio id check that wedged — HTTP's is at `crates/operant-core/src/mcp.rs:659`); iter-1822 replaces that single read with a drain loop in the working tree | One foreign/duplicate stdout line while a request is in flight | The stdio client read ONE line and hard-failed on id mismatch, wedging the client for the process's life. iter-1822 drains-until-match and parks foreign responses. | `cargo test -p operant-core --lib mcp::` (27/27) |
| **G-006** | **open — new defect found this sweep** | `id: u64` vs `id: Value` | `crates/operant-core/src/mcp.rs:320` (client `id: u64`) vs `crates/operant-cli/src/mcp_serve.rs:39` (server `id: Value`) | A server echoing a string id | Client must accept `serde_json::Value` for `id`; a number is the common case, a string is legal and `crates/operant-cli/src/mcp_serve.rs:822` asserts it. | a test with a string-echo server must pass |
| **G-007** | **open — misconfig** | `Tool timed out after 30s` | `crates/operant-cli/src/config.rs:317` `command_timeout: Some(30)`; `crates/operant-cli/src/cmd_setup.rs:979` `tool_timeout_secs … unwrap_or(30)`; MCP HTTP timeout is 60 s at `crates/operant-core/src/mcp.rs:399` | `papers_search`, `news_search`, `data_query`, `security_vuln`, `patents_search` | MCP-backed tools must not use the 30s tool timeout — give them the MCP timeout (or a larger value) so a slow upstream is not cut at 30s. | one deferred tool completes |
| **G-008** | **open — deploy gap** | 13 cron jobs `paused` | `crates/operant-cli/src/cmd_cron.rs:1`; the error is emitted by the model-routing layer (`Unable to determine provider for model 'claude-sonnet-4-20250514'`) | `cron list` | Jobs carry a bare model with no provider prefix. The working pattern exists: `crates/operant-cli/src/tui/operant_app/provider.rs:135` (prefix resolution) and `crates/operant-core/src/tools/transcription_tool.rs:68` (`resolve_model`). Normalise cron job models through `resolve_model`. | `cron list` shows `scheduled`, not `paused` |
| **G-009** | **open — feature gap (canonical RT-179)** | 72 TUI commands absent from the gateway | live register: `docs/audit/2026-06-19-BUGS.md` **RT-179** (M, INTEGRATION GAP) | Compare the `RegisteredCommand` list at `crates/operant-cli/src/tui/operant_app/app.rs:409` to `crates/operant-cli/src/gateway_commands.rs:1` | Register the missing TUI commands as gateway commands so Telegram/Discord/webhook reach the whole lifecycle. | each `/cmd` works from a channel |
| **G-010** | **open — feature gap** | no cron/scheduler TUI pane | `crates/operant-cli/src/tui/operant_app/app.rs:409` (`/cron` command exists; no overlay) | `grep -rl cron crates/operant-cli/src/tui` → commands only, no overlay | Add a cron overlay to `App::overlay_flags()` (29 entries at `operant-cli/src/tui/app/mod.rs`) so `/cron` has the pane other surfaces have. | `/cron` opens a pane |
| **G-011** | **open — search gap** | `aft_grep` / `aft_glob` time-limited, index never used | the aft bridge binary (`~/.operant/aft/aft-v0.63.5/aft`) | `aft_grep "fn main"` → 0 files searched | The bridge falls back to a filesystem walk capped at 50k files. Narrow the path/include, or build the trigram index; live ledger **RT-195** documents the same index gap. | a repo-root grep returns hits |
| **G-012** | **open — schema gap** | `file_state` has no `read` | the `file_state` MCP tool | `file_state operation=read` | Add `read` to the enum, or delete the arm from the docs so callers stop trying. | — |
| **G-013** | **open — deploy gap** | 3 untracked junk dirs | `.serena/`, `.tmp-test/`, `.zcode/` (+ `.tmpwork/`, `_kernel/`) | `git status --porcelain` | Decide: gitignore each, or delete. They are not part of this protocol. | `git status --porcelain` clean of them |
| **G-014** | closed-upstream | `session_search` SQL error | `no such column: core` (session DB) | — | Transient; same call returned results later. Not reproducing. | — |

## Layer classification

`TOOL-LAYER` = the tool exists but errors/truncates · `LOGIC-LAYER` = wrong output ·
`NETWORK-LAYER` = external dependency unreliable · `CONFIG-LAYER` = missing/broken configuration ·
`DEPLOY-LAYER` = source is right, the deployed binary/config is stale (G-004, G-008, G-013).

## ID alias table (legacy → canonical)

`G-01`=`G-001` · `G-02`=`G-002` · `G-03`=`G-003` · `G-04`=`G-004` · `G-05`=`G-005` ·
`G-06`=`G-006` · `G-07`=`G-007` · `G-08`=`G-008` · `G-09`=`G-009` · `G-10`=`G-010` ·
`G-11`=`G-011` · `G-12`=`G-012` · `G-13`=`G-013` · `G-14`=`G-014` ·
`G-015`=`G-015` (delegate, closed-upstream G-003) · `G-016`=`G-016` (dup of G-004) ·
Canonical ledger IDs, different meaning: `RT-###` in `docs/audit/2026-06-19-BUGS.md`.

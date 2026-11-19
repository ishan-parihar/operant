# CONTEXT PACK — context-pack-2026-11-20-blockers

## READING CONTRACT

One message, one file: every input this round needs is below. Do not open repository files to re-read
what is already quoted here — that spends your tool calls and adds nothing.

- Cite by anchor: `# SECTION <NAME>` or `# FILE: <path> (lines a,b)`. A finding that rests on a figure
  must name its anchor and keep the figure's evidence class (`[LIVE]`, `[CERT]`, `[IN-SAMPLE]`,
  `[PRIOR]`, `[UNCONFIRMED]`). A figure with no anchor is an assumption: label it one.
- Lens families, where the pack carries them, are instruments, not vocabulary. Naming a lens earns
  nothing; the move the lens forces is the finding.
- If a number you need is absent, say which one and continue under a stated assumption. The report is
  judged on structure, arithmetic and falsifiers, not on filling gaps with plausible values.


# SECTION A-DELTA

# FILE: research/audits/context-pack-2026-11-20-blockers/DELTA.md (lines 1,55)

# Delta since round 2 (2026-11-19 → 2026-11-20)

This section carries what landed on `origin/main` since round 2's pack was cut
(`df1ea4c0..origin/main`). Each row is a pushed commit with a verified status.

## The Hydrogen campaign closed (iters 2671–2692)

Round 2 returned 12 findings. Every one was verified against the live tree and dispositioned:

| # | finding | verdict | fix | commit |
|---|---|---|---|---|
| F1 | no circuit breaker on agent loop | CONFIRMED | `PrimaryBreaker` + per-candidate `clamp_max_tokens_to_model` | iter-2679 |
| F2 | max_tokens unbounded per candidate | CONFIRMED | `clamp_max_tokens_to_model` in `process_candidates` | iter-2679 |
| F3 | gateway turn retry span unbounded | CONFIRMED | `gateway_turn_ceiling_secs()` caps retry span | iter-2680 |
| F4 | evidence recall not wired to persisted facts | CONFIRMED (peer) | `guard_tool_result` accepts `persisted_key` | iter-2688 (F6) |
| F5 | `HistorianKey` never read | REFUTED (already wired) | — | — |
| F6 | tool result dropped evidence key | CONFIRMED | `guard_tool_result` points at `evidence_recall "<key>"` | iter-2688 |
| F7 | doctor never warns on parked key | CONFIRMED | `historian_key_is_parked` warning | iter-2672 |
| F8–F11 | (round-2 secondary) | CONFIRMED-NARROWER | various | iters 2684–2687 |
| F12 | `historian_key_is_parked` not a doctor warning | CONFIRMED | `historian_key_is_parked` in doctor | iter-2672 |

## Residual cleanup closed (iters 2693–2702)

- **iter-2694**: OUTLINE.md crate-graph claim corrected (operant-core does NOT reach operant-plugins);
  MCP server attribution fixed (server is in operant-cli); AGENTS.md slack.rs handle_update stale
  claim corrected; TUI count 73→74. Flipped RT-136/RT-285 checkboxes (stale, closed by code).
- **iter-2695**: Deleted `gateway/admin.rs` (179 lines, zero production callers) + re-exports.
- **iter-2698**: Fixed 5 E303 dead file citations in library docs (write_approval.rs, schema/media.rs,
  tts_provider.rs, tts_registry.rs, tts_command_provider.rs all deleted).
- **iter-2701**: Added 6 E311 exemptions to genome.toml coverage_exempt; tombstoned 2 superseded plans.
- **iter-2702**: Clippy cleanup — empty_line_after_doc_comments, manual_clamp×2,
  unfulfilled_lint_expectations, unused_variables, 11 `#![allow(dead_code)]` on vendored jcode TUI.

## Dead-code audit (iter-2703, iter-2704)

Full audit at `docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`.
Summary: operant-core lib is CLEAN (0 warnings, every `#[expect(dead_code)]` fulfilled).
2 purgable stubs identified (`tui/rustle.rs`, `tui/figures.rs` — both empty after prior pruning).
1 utilizable gap (`impl_cache_key` in mcp.rs — wire into cache key path). 28 residual clippy
warnings (99% mechanical + 2 `await_holding_lock`, one a correctness risk).

## What did NOT change (the reviewer should NOT re-litigate these)

- **No new architecture.** The 7-crate workspace, the 7-adapter gateway registry, the
  memory-wire-only memory, the sourcehound browser default, the Phonon TTS default — all
  unchanged. Round 2 covered these; they are not in this delta.
- **No new CLI commands.** Still 53 variants, all with real handlers.
- **No new platforms.** Still 7 in the registry.
- **The lib suite is RED at tip** due to a peer's process-global env-var pollution (iter-2703
  documents it). This is a known defect with a plan; it is not a new blocker.

## Round 2's pack, for reference

Round 2 artefacts: `research/audits/context-pack-2026-11-19-hydrogen-arch/` (only SEND.log
committed; the pack spec and response are in the audit doc).

# SECTION B-STATE

# FILE: research/audits/context-pack-2026-11-20-blockers/STATE.md (lines 1,55)

# State — what operant believes about itself (2026-11-20)

The reviewer sees this BEFORE it judges the system. Audit it: if a claim below is
stale, wrong, or unfalsifiable, say so — that is a finding.

## What operant is

A single-binary Rust agent runtime with a ratatui TUI, a gateway daemon for 7 messaging
platforms, an MCP client+server, a memory engine (memory-wire, in-process), a context
manager with tiered eviction, and a tool registry with 50+ native tools. Workspace of
7 crates (`operant-core`, `operant-cli`, `operant-os`, `operant-config`, `operant-api`,
`operant-providers` (deleted), `operant-plugins`). Release line `0.3.0`.

## What it claims to be good at

- **Single-binary deploy** — one `operant` binary, no runtime services to install.
- **Local-first** — memory-wire runs in-process; no memory server to operate.
- **Gateway-native** — 7 platforms (Telegram, Discord, Slack, WhatsApp, Email, SMS, Webhooks)
  with real adapters, not stubs.
- **Tool-rich** — AFT (20 IDE-grade tools), sourcehound (web+browser), native file/shell/memory.
- **Context-aware** — tiered eviction (T3→T2→T1), decay curve, prompt-cache frozen prefix.
- **Extensible** — WASM plugins (Extism host), MCP clients and server, skills, SOPs.

## What it claims is DONE

- All hermes-agent gaps closed (MCP SSE, credential rotation, platform registry, MCP
  sampling/elicitation, `/steer`, context compression, hook system, 24-class error recovery,
  concurrent tool execution, webhook adapter, WhatsApp/Email/SMS, Anthropic cache_control).
- All 53 CLI command variants have real handlers (verified iter-1474; `cron run` was the last stub).
- All 7 gateway platforms have real code.
- `operant-core` lib compiles with 0 warnings (every `#[expect(dead_code)]` fulfilled).

## What it claims is OPEN

- **RT-190**: memory-wire serve daemon's bank is separate from operant's store. Option (b),
  the cross-surface bridge, needs an owner ruling.
- **RT-197**: `[skills] root_dir` template footgun — harm mitigated, fix unwritten.
- **RT-179**: TUI functional core's channel twins — corrected; most twins are live.
- **Lib suite RED at tip** — peer's process-global env-var pollution (iter-2703 has a plan).
- **TUI visual corpus RED at 56 drifts** — peer's iter-1461 prompt-jump consequence; corrected state.

## What it claims it cannot do

- Slack/WhatsApp/Email/SMS are OUTBOUND-ONLY (no Socket Mode, no Graph webhook receiver,
  no IMAP). Webhooks is INBOUND-ONLY (no outbound send). This is by design, not a gap.
- No hardware/peripheral support (the `operant-hardware` crate was deleted iter-892).
- No desktop app (the desktop upsell dialog was deleted iter-58).
- No LifeOS/Notion integration (the 22 Notion tools were ported at iter-47/48 and removed).

## The binding constraint

**Iteration velocity vs. correctness.** The project runs an autonomous iteration model
(commit + push per change, deploy after every push). The user tests from `origin/main`.
A change that lives only in a working tree is not done. This constraint shapes every
decision: a fix that cannot be verified locally + deployed in one session is not shippable.

# SECTION C-ASK

# FILE: research/audits/context-pack-2026-11-20-blockers/ASK.md (lines 1,43)

# Ask — the open decision (2026-11-20)

## The question for this round

**Round 2 returned 12 findings; all 12 are closed. The residual cleanup (dead code, doc
drift, stale checkboxes, clippy) is also closed. What is the NEXT blocker?**

The operator has asked for a third round because the project has reached a local
plateau: the architecture review closed its findings, the dead-code audit found no dead
production code, the clippy gate is green, and the 7-crate workspace is stable. The
question is whether the NEXT phase's blockers are:

1. **Bottlenecks** — what is the binding constraint on the iteration model? (compute,
   deploy, verification, peer coordination)
2. **Redundancies** — what is duplicated across the 7 crates that should be unified?
3. **Fragmentations** — where does the system have two halves that should be one?
4. **Integration gaps** — what is wired but not reachable, or reachable but not wired?

## What the operator will do with the answer

The operator (the user) tests from `origin/main` after every push. A finding that
names a concrete blocker with a `file:line` and a proposed fix is actionable in the
next session. A finding that is a judgment call without evidence is not.

## What is NOT the question

- The architecture review (rounds 1 and 2) already covered agent-loop, gateway, memory,
  context, config, tools, skills, plugins, autonomous, TUI, org, security. Do not
  re-litigate those surfaces.
- The dead-code audit (iter-2704) already enumerated every `dead_code` site. Do not
  re-enumerate them.
- TheHydrogen campaign (iters 2671-2692) already closed F1-F12. Do not re-open them.

## What would make this round a delta, not a re-derivation

A finding is a DELTA finding only if it names something that:
- landed since round 2's pack was cut (`df1ea4c0..origin/main`), OR
- was not covered by rounds 1 or 2's surfaces, OR
- contradicts a claim in the STATE section (stale, wrong, or unfalsifiable).

If the reviewer finds that the system is in a steady state with no new blockers, say
so — that is a valid finding. The operator will then shift from architecture work to
feature work.

# SECTION D-LAW

# FILE: research/audits/context-pack-2026-11-20-blockers/LAW.md (lines 1,50)

# Law — the rules a proposal must survive (2026-11-20)

These are the operator's binding constraints. A finding that proposes violating one of
these is not a finding; it is a proposal the operator will reject. The reviewer should
treat these as the fitness function, not as suggestions.

## Design preferences (DO NOT CHANGE)

- **Voice/TTS**: Phonon (`phonon-31m-en`, GGUF q8_0, ~65MB). Do NOT switch to Edge TTS,
  OpenAI TTS, or any other provider as the default.
- **Memory**: memory-wire (in-process, cargo git-dep). The only engine. Do NOT re-add
  legacy providers (agentmemory, tdg, hindsight, retaindb, mem0, local-vector).
- **Browser/Web**: sourcehound (binary over MCP stdio). Do NOT switch the default away.
- **Gateway**: 7 platforms (Telegram, Discord, Slack, WhatsApp, Email, SMS, Webhooks).
  Do NOT re-add purged platforms (matrix, mattermost, signal) without a real adapter.
- **Native tool integrations**: AFT (20 IDE tools), sourcehound, ponytail (content).
  Do NOT add a LifeOS/Notion built-in tool surface (it was removed; the `ntn` CLI + skill
  is the Notion path).
- **WASM plugins**: Extism host ON by default. One manifest authority (`manifest.toml`).
  Do NOT re-add Cordis's service-store/fiber machine or patch language.
- **Context management**: tiered eviction + decay curve. Prompt-cache frozen prefix +
  volatile suffix. Do NOT change the eviction order (T3→T2→T1).
- **One home resolver**: `operant_core::platform::operant_home()`. Every store resolves
  through it. Do NOT build a second home resolver.

## The iteration loop (binding on every change)

- Every change is one iteration: commit + push to `origin/main` + deploy.
- A change that lives only in a working tree is NOT done.
- Verify locally before push: `cargo fmt`, `cargo check -p <crate>`, `cargo test`,
  `clippy-warning-gate.sh`. Then commit, push, release-build, deploy, restart.
- The iteration number comes from `origin/main`, never local history.

## The parallel-agent rules (load-bearing)

- Never `git reset --hard`, `git clean`, or `checkout` over a peer's uncommitted WIP.
- Never `git add -A`. Stage explicit paths only.
- Push immediately after every commit. An unpushed commit in a shared tree is one peer
  `reset` away from being orphaned.
- If a peer's subsystem is red, do not fix it and do not block on it. Prove your delta
  is zero, push.

## What a finding must carry to be actionable

- A `file:line` citation the operator can open.
- A proposed fix that does not violate any law above.
- A falsifier: what would prove the finding wrong.

A finding without these is a judgment call, not a blocker. The operator will not act
on it, and the round was wasted.

# SECTION E-EVIDENCE

# FILE: research/audits/context-pack-2026-11-20-blockers/EVIDENCE.md (lines 1,40)

# Evidence — what has been measured (2026-11-20)

## Round 2 findings (all closed)

The 12 findings from round 2 are at `docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md`.
All 12 are CLOSED. The reviewer should NOT re-litigate them. For reference, the categories were:
agent-loop (F1-F2), gateway (F3), evidence recall (F4, F6), doctor warnings (F7, F12),
stop discriminator (F8), prompt caching (F9), tool bridge (F10), cache invalidation (F11).

## Dead-code audit (iter-2704)

Full audit: `docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md`.
Key findings:
- `cargo check -p operant-core --lib`: **0 warnings** (every `#[expect(dead_code)]` fulfilled).
- 2 purgable stubs: `tui/rustle.rs` (empty), `tui/figures.rs` (empty after prune).
- 1 utilizable gap: `impl_cache_key` in `mcp.rs:471` (wire into cache key path).
- 28 residual clippy warnings (99% mechanical + 2 `await_holding_lock`).
- No dead production code, no duplicate registries, no stub adapters, no dead CLI commands.

## Clippy gate status

`bash scripts/clippy-warning-gate.sh`: PASS. 28 new warnings, allowlist ceiling 124.
The allowlist has 96 stale entries from the pre-iter-2444 era (prune-able, not a blocker).

## Lib suite status

RED at tip due to a peer's process-global env-var pollution (iter-2703 documents it).
This is a known defect with a plan (`docs/plans/2026-11-19-lib-suite-is-red-at-tip-a-peer-process-global-env.md`).
Not a new blocker.

## What these measurements bound

- The dead-code audit bounds the dead-code surface: there is no dead production code to purge.
- The clippy gate bounds the style surface: 28 mechanical warnings remain, all one-line fixes.
- The lib-suite RED is a TEST INFRASTRUCTURE issue, not a production code issue.

What these measurements do NOT bound:
- Whether the architecture has bottlenecks that only appear under load.
- Whether there are redundancies between crates that a single-crate audit cannot see.
- Whether the integration between the 7 crates is complete (a crate-level audit misses seams).

# SECTION F-ANALYSIS

# FILE: AGENTS.md (lines 845,893)

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
│   │   │   │   ├── aft_tools.rs   # 20 AFT IDE tools
│   │   │   │   └── memory_tools.rs # memory_* tools (memory-wire/builtin)
│   │   │   ├── memory_wire.rs # MemoryWireProvider (in-process, catch_unwind)
│   │   │   ├── memory_provider.rs # memory-wire + BuiltinProvider
│   │   │   ├── context_management.rs # Tiered eviction + decay curve
│   │   │   ├── aft_bridge.rs  # AFT subprocess + auto-update
│   │   │   ├── gateway/       # Platform adapters (Telegram/Discord/Slack/WhatsApp/Email/SMS/Webhook)
│   │   │   ├── mcp.rs         # MCP client (HTTP + Stdio + SSE)
│   │   │   └── config.rs      # AppConfig, BehaviorSettings, ToolSettings
│   │   └── Cargo.toml         # Features: anthropic (the memory-wire git-dep is baked in; sourcehound is a spawned binary, not a dep)
│   └── operant-cli/           # CLI + TUI
│       ├── src/
│       │   ├── main.rs        # CLI entry point, Clap enum, agent setup
│       │   ├── mcp_serve.rs   # Operant as an MCP SERVER over stdio (`operant mcp serve`); the CLIENT lives in operant-core/src/mcp.rs
│       │   ├── tui/           # 74 TUI modules (351 .rs files) — (app, render, prompt_input, dialogs)
│       │   ├── gateway_runner.rs # Gateway mode agent handler
│       │   ├── autonomous.rs  # Autonomous coding mode
│       │   ├── config.rs      # CLI config
│       │   ├── cmd_*.rs       # CLI subcommand handlers
│       │   └── dashboard_server.rs # axum dashboard backend
│       └── Cargo.toml         # Features: anthropic, plugins-wasm
├── scripts/
│   ├── dev-env.sh             # Source this before any cargo command
│   ├── check.sh               # Wrapper that applies dev-env + cargo
│   ├── self-test.sh           # Build + test + clippy + fmt sweep
│   └── provision-build-deps.sh # One-time dependency provisioning
├── Cargo.toml                 # Workspace members (7 crates, self-contained)
├── operant.example.toml       # Config template (7 gateway platforms)
├── TODO.md                    # Autonomous-mode task ledger
├── docs/audit/2026-06-19-BUGS.md # Open issues tracker (the live ledger; the 2026-08-03 snapshot is in docs/archive/BUGS.md)
└── AGENTS.md                  # THIS FILE
```

---


# SECTION F-ANALYSIS

# FILE: AGENTS.md (lines 1309,1359)

## Known Gaps (from hermes-agent contrast audit)

**All hermes-agent gaps closed.** The operant project now has feature parity
with hermes-agent's core functionality (excluding Python-specific plugins).

### 0 open CLI stubs (the last one closed iter-1474)
- `operant cron run <id>` **runs the job** — `CronScheduler::run_job_now(id)`
  rides the scheduler's own dispatch, ESTOP refuses while stopped, and the
  row is marked from the attempt's real outcome. This was the repo's one
  accepted stub for its whole life (AGENTS.md told every agent "Do not fix it
  unless explicitly asked"); the 2026-10-29 remaining-gaps plan asked, it was
  fixed, and that instruction is withdrawn here so the next agent does not
  re-read it as still standing. A bad ID still says "Cron job '<id>' not
  found" (see BUGS.md RT-135, CLOSED).

`operant dashboard server --stop` / `--status` are NOT stubs any more
(iter-1121): the server records its pid + URL in
`$OPERANT_HOME/dashboard.pid`, `--status` reports it (and removes a stale
file), and `--stop` SIGTERMs that pid and waits for the process to go. The
pid file is removed both by a clean exit and by `--stop`; pid 0 is refused
outright because `kill(0, …)` addresses the caller's process group.

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
- ✅ ~~Hook system~~ — HookRegistry with 9 events + wildcard, wired into agent
  loop (iter-61/62)
- ✅ ~~Error recovery 3 vs 22~~ — 24 error classes with ClassifiedError (iter-61)
- ✅ ~~Sequential tool execution~~ — concurrent 8-worker pool (iter-56)
- ✅ ~~7 dead TUI dialogs~~ — elicitation/onboarding/file_injection/invalid_config/
  memory_update_notification/overage_upsell/desktop_upsell_startup deleted
  (iter-58). The remaining 7 dialogs (ask_user/bypass_permissions/custom_provider/
  device_auth/free_mode/import_config/key_input) are alive via indirect `.open()`
  calls from the connect_dialog handler. (`device_auth` was removed in iter-937: it had no opener.)
- ✅ ~~Iteration budget grace call~~ — summarize instead of hard-stop (iter-57)
- ✅ ~~WebhookAdapter stub~~ — real HTTP server with HMAC (iter-54)
- ✅ ~~WhatsApp/Email/SMS adapters~~ — real implementations (iter-54)
- ✅ ~~Anthropic cache_control~~ — breakpoints on system prompt (iter-54)
- ✅ ~~Gateway clear_history every message~~ — session caching fix (iter-49)

---


# SECTION F-ANALYSIS

# FILE: AGENTS.md (lines 1360,1594)

## What IS Working (verified functional — iter-68 audit)

### CLI (53 command variants — real handlers; no stubs remain as of iter-1474)

run, autonomous, tools, chat, test, config, sessions, continue
mcp, cookies, skills, model, completion, cron, pause, resume
suggestions, kanban, org, gateway, checkpoints, memory, context
profile, auth, login, logout, version, doctor, status, dump
logs, backup, import, uninstall, update, insights, webhook, hooks
debug, plugins, architecture, channel, sop, migrate, service, curator
setup, acp, dashboard, trajectory, tui, donate

The list above is derived from the `Commands` enum in
`crates/operant-cli/src/main.rs` (53 variants). It has said 52 and omitted
`continue` since iter-2372 added it; `scripts/check-doc-claims.sh` is the check
that catches this class of drift, and it is the authority — re-derive rather
than trusting the number here. It used to claim 53 and list `hardware` and
`peripheral`, both removed in iter-892, while omitting `donate` and `org`, both
real commands with handlers. See RG-010 for the check.

### Operator-facing surfaces (re-verified 2026-10-21, iters 1124-1131)
- `operant tools disable` removes the tool from the agent's toolset — the
  tool-state JSON file next to the database is read by the runtime, not just
  written by the CLI (iter-1124; before it, the command changed nothing)
- `operant hooks` runs the operator's own commands on the agent's four
  lifecycle events (`agent_start`, `agent_end`, `turn_start`, `turn_end`).
  The `HookRegistry` is created unconditionally; the harness bridge is
  registered on it when the harness is on (iter-1125)
- The agent reads the operator's SOP playbooks (`~/.operant/sops/*.toml`)
  through the `sop` tool — the same files `operant sop list` shows
  (iter-1126)
- `operant webhook test` really POSTs a signed ping, and subscription
  routes DISPATCH: a subscription with its own secret is verified against
  that secret, shapes the event through its prompt/skills, and names the
  answer's delivery target (`deliver_only` is consumed at
  `crates/operant-cli/src/gateway_runner.rs` — RT-140, CLOSED iter-1132)
- The parallel config surface is RETIRED (iters 1138-1141): the aggregate
  `operant_config::schema::Config` with its loader, migration and 7K-line
  test file, 17 dormant section files, 11 dead crate modules, and the
  `Configurable` derive machinery (`operant-macros`) — ~25K lines, none of
  it ever loaded in production. `operant-config` is ~5K lines and holds
  only the types the live config composes. ONE config system now:
  `operant_core::config::AppConfig` (RT-141, closed)
- The `operant-providers` crate is GONE (RT-153, closed iter-1215): it was
  a SECOND provider surface production never called — outside its own crate
  the workspace named only the doctor's `create_provider` probe. The doctor's
  name-known and wire-API checks now read the static
  `operant-core/src/provider_catalog.rs` (the factory's full 157-name
  recognition surface). The live provider path remains
  `operant-core/src/client.rs`

### Gateway (7 platform adapters — all have real code, platform registry)

- **The session socket is part of the daemon** (`crates/operant-core/src/session_server.rs`,
  `session_protocol.rs`, `session_client.rs`). Every `operant gateway run` binds it at
  `session_client::default_socket_path()` (`<operant home>/operant-session.sock`; `--socket PATH`
  overrides the location), and `operant chat --attach <path> ["prompt"]` attaches: the client
  builds no registry, no database handles, no MCP children and no AFT bridge. The wire is one JSON
  object per line (`WireMessage`), and the server answers `Hello` with the sessions the daemon has
  bound. A prompt obeys the SAME gates a channel turn does — ESTOP, `max_concurrent_sessions`, the
  slot's session binding (`bind_slot_to_session`), and the turn's error is published on the session
  bus rather than dropped.
- Telegram: fully implemented (long-poll + Bot API)
- Discord: fully implemented (Gateway WS + REST API)
- Slack: outbound only — the Socket Mode WebSocket does not exist (RT-86)
- WhatsApp: implemented (Cloud API outbound + webhook inbound)
- Email: implemented (SMTP outbound + webhook inbound)
- SMS: implemented (Twilio API outbound + Twilio webhook inbound)
- Webhooks: implemented (axum HTTP server + HMAC validation)
- Platform registry: factory pattern replaces if/elif chain (iter-66)

### TUI
- Entry point works (`TuiApp::enter` → ratatui + crossterm)
- Permission dialog system works (iter-20: real prompts instead of auto-approve)
- Event routing: `AgentEvent`s arrive on `agent_event_rx` and are drained
  by `App::drain_agent_events` (`tui/app/channel_drains.rs`). There is no
  "TUI bridge" any more: the vestigial `bridge_state` machinery was deleted
  in iter-985 (its receiver had no producer in the TUI process, and the
  gateway's sender wrote into a discarded receiver in another process).
- Prompt-cache stability: frozen prefix + volatile suffix (iter-39)
- Session caching: `clear_history` only on session change (iter-49)
- Anthropic `cache_control` breakpoints (iter-54)
- 7 dead dialogs deleted (iter-58); every remaining overlay has a PRODUCTION
  opener. `App::overlay_flags()` names **29** of them (the array's own type
  length is `[(&'static str, bool); 29]`, which the compiler enforces — so a
  doc number here is only true while it equals that length, and
  `scripts/check-doc-claims.sh` derives it), and a full audit (iter-1002)
  traced every one to a non-test call site in `commands.rs`, `key_handling.rs`,
  `channel_drains.rs`, `import_config.rs`, `turn_state.rs`, `agents_view.rs`
  or `adapter_types/tui_app.rs`. None is test-only and none lacks an opener.
  (This said **30** until 2026-11-17: the count drifted when an overlay was
  added without updating the prose, and nothing checked it. Note the
  "every overlay has a production opener" half is NOT grep-checkable —
  openers arrive as `.open()`, `.open_detail()`, `.toggle()` and
  `visible = true` — so per-command dispatch tests are the evidence for it.)

#### TUI visual layer — architecture + working discipline (2026-10-13)

The jcode-parity port is the ground design: reference source at
`../parent-projects/jcode/crates/jcode-tui/src/tui/`. Invariants an agent
MUST know before touching the render layer:

- **One renderer.** `display_messages()`
  (`tui/operant_model/adapter.rs`) is THE flatten layer;
  `tui/operant_ui/ui_prepare.rs` is THE frame assembly. Every row renders in
  exactly ONE section — never both Body and Streaming (the 2026-10-13
  duplication was a row in both). `tests::live_tails_render_once_while_a_tool_is_reserved`
  pins it against the drawn screen.
- **Live partials never enter settled rows.** Streaming text/thinking render
  only in the mutable Streaming section (`has_streaming` gate in ui_prepare
  ~:850). Anything committed to `App.messages` becomes emittable scrollback;
  anything live must stay mutable or it double-renders at two wrap widths
  (the 2026-10-12 "shredded fragments" family — iter-784/789).
- **Reasoning display mode is a three-state contract** (jcode
  `ReasoningDisplayMode`): `Off` (never accumulate), `Current` (commits in
  place; the adapter hides a turn's traces once the next user prompt lands —
  the default, and what `show_reasoning = true` maps to), `Full` (every
  trace stays). The mode
  resolves through a process-global RwLock override cell in
  `tui/operant_app/config_shim.rs` fed from `[display] show_reasoning` at
  boot (main.rs) and flipped live by `/thinking-display`. Tests that set or
  depend on it hold `config_shim::reasoning_override_test_guard()` —
  parallel test threads race on the cell. Thinking is stored sentinel-wrapped (iter-780); the renderer
  keys dim+italic off `REASONING_SENTINEL`, never off role names.
- **Terminal-scroll mode** (`[tui] terminal_scroll_mode = true`): inline
  viewport, no alt-screen, mouse capture OFF — selection is native terminal
  select-and-copy over emitted history (deliberate). The scrollback emitter
  (`tui/app/scroll_emit.rs`) is append-only with a watermark; Ctrl+L
  never resets it. `insert_before` is inert on the headless sim's
  TestBackend — scrollback behavior is live-verify only.
- **Tool rows anchor by arrival** (`after_index`), committed thinking
  rides `ContentBlock::Thinking`, and a tool call mid-response is
  "reserved" (`after_index > messages.len()`): while the tails stream it
  renders in the Streaming section below them (`adapter::live_tool_rows`),
  never in the Body — do not add flushes at
  ToolStart; the seam test `tool_start_mid_stream_must_not_split_assistant_text`
  pins why.
- **Registered slash commands can never fall through to the model** — the
  not-wired catch-all (iter-790) covers both dispatch loops; the corpus
  `command_sweep.py` must report every registry command consumed (46/46 as
  of the 2026-10-14 dead-command sweep, plan 02) and the unit test
  `no_registered_tui_command_is_unwired` pins the same invariant statically.
- **Selection**: app-mode drag paint clamps to the chat band (iter-672);
  the copied TEXT is extracted synchronously from the drawn frame
  (`render::selection::selection_copy_text`), never from painted cells —
  those lag a frame and only cover the viewport;
  the `selection-highlight` scenario's notes document the boot-dialog
  Down-swallow — a click that closes a dialog is correct, not a bug.

#### TUI debug + live-repro tooling
- Headless: `operant -c <pinned.toml> tui debug simulate --keys '<enter>'
  --mouse '<left,x,y><drag,x,y>'` — drives the real event handlers against
  a TestBackend. Mid-stream oracles: `--frame-per-event` writes every
  painted frame to `--capture-dir`; `--assert-screen 'count:NEEDLE==N'`
  asserts an exact occurrence count on the final screen (duplicate-row
  detection). Corpus: `crates/operant-cli/tests/tui_scenarios/`
  (`_run.py`; `OPERANT_BIN` must point at a FRESH debug build; `prove`
  writes goldens, `verify` gates them; the journey-view/skills-view 4-drift
  baseline is pre-existing peer drift).
- Live: tmux `-L <fresh-socket>` sessions, 110x45;
  `OPENAI_BASE_URL=http://localhost:20129/v1` (omp gateway →
  poolside/laguna — the only local provider emitting the reasoning triad;
  mid-stream deaths render as single error cards and retry via `continue`).
  Event capture: `OPERANT_TUI_DEBUG=1 OPERANT_TUI_EVENT_LOG=<path>` — ring
  dumps on clean `/quit` only. `/tmp` is tmpreaper-swept: recreate test
  configs (copy `~/.operant/operant.toml`, MERGE `[tui]` keys — duplicate
  TOML tables are parse errors) before each repro.
- Rebuild the debug binary after every edit before testing — stale cached
  binaries produced false regression readings twice this session.

### Web Dashboard
- Dashboard server: axum + `TcpListener::bind` + `axum::serve`
  (`dashboard_server.rs`)
- Routes: `/api/status`, `/api/boards`, `/api/health`, `/api/config`,
  `/assets/:filename`, `/`
- Static assets: fonts, JS, CSS served from `crates/operant-cli/src/dashboard/`
- The dashboard is a FOUR-ROUTE status view served from a first-party page
  (`crates/operant-cli/src/dashboard/index.html`, inlined with
  `include_str!`), not the full operant web console. The bundled frontend
  that used to sit here — a build artefact from a larger harness, calling 79
  distinct `/api/...` paths the server never implemented — was removed in
  iter-1120 and replaced by that page; see RT-44, closed.
  `operant dashboard server --stop` and `--status` work (pid file + SIGTERM
  + bounded wait; pid 0 is refused because `kill(0, …)` targets the
  caller's process group).
- Auth: a session token, generated at startup, printed to the console. The
  index route serves the token-bearing page only to a request that already
  holds it (the CLI's browser launch exchanges it for an HttpOnly cookie),
  and a request with a foreign `Host` is refused. The API accepts the
  token as `Authorization: Bearer`, as `X-Hermes-Session-Token` (what the
  bundled frontend sends), or as the dashboard cookie. See AD-040.

### Agent Loop (verified features)
- Concurrent tool execution: 8-worker pool + semaphore (iter-56)
- Iteration budget grace call: summarize instead of hard-stop (iter-57)
- Context-overflow auto-compression: classify + manage_context + retry (iter-63)
- `/steer` directive: real-time user steering between iterations (iter-65)
- Hook system: `AgentStart`/`AgentEnd` events wired into `run()` (iter-61/62)
- Error recovery: 24-class `ClassifiedError` with `should_compress`/`should_fallback` (iter-61)
- `sync_turn`: auto memory write-back after each turn — memory-wire
  `retain`
- Credential rotation: `CredentialPool` with `PooledCredential` + OAuth refresh (iter-66)
- MCP sampling/elicitation: server-initiated request handlers in stdio (iter-66)
- MCP SSE transport: `McpSseClient` with background reader + oneshot routing (iter-68)

### Memory (memory-wire — deeply integrated)
- `MemoryWireProvider` (in-process `memory_wire` crate, `catch_unwind` boundary)
- `recall_with_weights` retrieval in `prefetch`; `sync_turn` → `retain`
  (agent loop auto-calls after each turn)
- memory-wire is the only engine; all legacy providers (agentmemory, tdg,
  hindsight, retaindb, mem0, local-vector) and the file-backed
  `BuiltinProvider` were removed — any provider name the config carries
  still routes to memory-wire

### Context Management
- Tiered eviction (T3→T2→T1 oldest-first, iter-37/38)
- Decay curve (H = H50·2^((I-50)/D)/max(p,0.10), iter-38)
- Recency reserve scales with budget (budget/4096 clamped to [6,50], iter-43)
- FTS5 special character escaping (iter-46)
- Auto-compression on `context_overflow` errors (iter-63)

### Native Tool Integrations
- AFT: 20 IDE-grade tools (subprocess + auto-update, iter-40/41)
- sourcehound: web_search + web_scrape + web_extract + web_crawl + sourcehound browser provider (MCP subprocess, no API keys)
- AFT dedup: basic file/terminal tools auto-disabled when AFT enabled (iter-51)

### Unique Features (operant has, hermes doesn't)
- AFT bridge: IDE-grade coding tools via subprocess with auto-update
- sourcehound bridge: web_search/scrape/extract/crawl + browser provider over MCP subprocess
- Context management: tiered eviction + decay curve (ported from magic-context)
- Prompt-cache frozen prefix: Anthropic `cache_control` breakpoints
- Rust safety: memory-safe, no GC, single binary, fast startup

---


# SECTION F-ANALYSIS

# FILE: docs/audit/2026-11-19-hydrogen-architecture-review-round2-12-findings-verified.md (lines 1,300)

# Hydrogen architecture review, round 2 (2026-11-19) — bottlenecks, redundancies, fragmentation, integration gaps

A fresh-context review round (Gemini 4 Hydrogen, `gemini-4-hydrogen`, effort `xhigh`)
over a **100,723 B context pack** cut from `AGENTS.md` slices, `docs/handoff/STATE.md`,
`docs/OUTLINE.md` and the mechanism docs for the surfaces round-1 did not cover
(tools, skills, plugins, autonomous, tui, org, security) plus a measured-surface delta.

Round-1 (`docs/audit/2026-11-19-hydrogen-architecture-review-12-findings-verified.md`)
covered agent-loop, gateway, memory, context and config. **This round is a delta round**:
it asks about the surfaces round-1 never opened, and its delta section carries only what
landed since (`df1ea4c0..origin/main`, four commits) plus a re-derivable measurement.

**Every finding below was re-verified against the live tree before adoption.** The
reviewer worked from documentation, so each carries a CONFIRMED / CONFIRMED-NARROWER /
REFUTED verdict with `file:line` evidence. Where the reviewer's own inference was wrong,
that is recorded rather than quietly dropped.

Round artefacts: `research/audits/context-pack-2026-11-19-round2-arch-bottlenecks/`
(`pack.spec`, `PACK-AS-SENT.md`, `REVIEWER-PROMPT.md`, `RESPONSE.md`, `DISPATCH.json`,
`measure_surface.py` + `measure2.py` + `measure3.py`).

## The round's parameters, as recorded

| fact | value |
|---|---|
| venue | AI Studio (`aistudio.google.com`), fresh chat, model pinned in URL |
| model / effort | `gemini-4-hydrogen` / `xhigh` (confirmed by `doctor` and by `send`) |
| pack size | 100,723 B (cap raised 90k → 110k deliberately; ~28k tokens of a 1M shared window) |
| transport | inline, sentinel last (the venue has no pack-file attach path) |
| reply | 32,303 chars, DOM scrape (`DISPATCH.json` records this; the three-dots copy was not used) |
| response provenance | `fetched_at 2026-11-19T19:15:02`, page 239 |

## Verification method

Each finding was checked against `origin/main` with targeted greps. The reviewer's own
proposed falsifier was run first; a finding is **CONFIRMED** only when the falsifier fails
against the tree. Where the reviewer's reasoning overstated the impact, the finding is
marked **CONFIRMED (narrower than claimed)** and the correct scope is stated — an
overstated finding adopted unqualified is a defect the next agent inherits.

## Findings

### F1 — the `[cli]` CLI-admission surface has a full producer and no agent consumer — CONFIRMED (cross-crate, Severity 3)

`operant context cli list|add|remove|toggle` (`crates/operant-cli/src/cmd_context.rs`),
`tools/cli_roster.rs`, `tools/cli_help_parser.rs`, `tools/cli_discovery.rs`,
`cli_roster.toml`, `[cli.tools]` and `[cli] max_help_lines` all exist. But the
agent-facing half does not: `rg 'cli_exec|cli_reference'` over `crates/` returns **only
comments and doc text** — `config.rs:1687-1688` ("the agent-facing half is still not
wired: `cli_exec` and the `<cli_reference>` prompt injection are the plan's Phase C.
Nothing in ..."), `config.rs:1702`, `cmd_context.rs:1488`. No `cli_exec` tool is
registered, and no `<cli_reference>` block is injected into any prompt.

So an operator can run admission scans, and the grants are recorded and costed but
**not invocable**. `[cli] max_help_lines` is parsed and never applied (RT-278).
Falsifier: `rg -n "cli_exec|<cli_reference>|max_help_lines" crates/` — the only hits are
prose. **The fix needs a product/security decision**, not a patch: either define how
`cli_exec` enforces `--allow-subcommands`/`--global-args` against `ApprovalGuard` without
shell/flag-injection bypasses, or hide `operant context cli` and `[cli]` until Phase C
ships.

### F2 — two interactive human-question tools with incompatible host plumbing — CONFIRMED (cross-crate, Severity 3)

Two tools target the same TUI dialog by different mechanisms:

- `clarify` (`tools/clarify_tool.rs` over `user_question.rs`, unconditional builtin)
  pushes onto a **process-global** `static USER_QUESTION_TX: OnceLock<...>`
  (`user_question.rs:52`).
- `ui_request_input` (`tools/ui_tool.rs`, gated by `[ui] generative_enabled`) uses a
  **per-host** request channel resolved via `agent.transport`, drained by both the TUI and
  the gateway, and guarded by `crates/operant-core/tests/surface_host_receivers.rs`.

Measured: `rg 'USER_QUESTION_TX' crates/operant-cli/src/gateway_runner.rs` returns
**nothing** — the gateway never drains it.

**The reviewer's inference was wrong about the failure mode, and the correction matters.**
It claimed "any non-draining host risks a closed-channel error or a 300 s turn stall".
Reading `user_question.rs:83-98`: when the `OnceLock` is unset (no drainer registered)
`try_send_user_question` returns `None`, and the caller "should fall back to returning the
question as a tool result". So there is **no stall** — the tool degrades to a plain
result. The real defect is narrower and still real: `clarify`'s `session_id` field rides
a process-global channel, so in a multi-chat gateway process **the questions are not
isolated by chat**, and the `has_user_question_sender()` gate means a gateway host that
never registers a sender silently loses the interactive path for every chat.

Fix: route `clarify` through the same per-host/per-transport receiver as
`ui_request_input`, removing the global `OnceLock`.

### F3 — `builtin_tool_names()` disagrees with the registered name for the notify tool — CONFIRMED (narrower than claimed, Severity 3)

The reviewer claimed three consumers use `"notification"` while the tool registers
`"notify"`. Measured:

- `tools/builtin.rs:354` lists `"notification"`.
- `tools/notification_tool.rs:26,40,49,63` registers **`"notify"`**.
- `agent/evidence/classify.rs:107-109` lists **`"notify"` AND `"notification"`** — a
  deliberate phantom alias ("The phantom aliases are kept so an older name still
  classifies"). So the classifier is **not** broken; the reviewer's third consumer is
  wrong.
- `crates/operant-cli/src/gateway_runner.rs` has **no** `"notification"` icon entry
  (the reviewer's TUI-icon claim does not hold at that path).

The real defect: `builtin_tool_names()` is a hand-maintained list, and
`tools/tool_state.rs::disabled_tool_names()` returns a plain `HashSet<String>` with **no
validation** — so `operant tools disable notification` writes a name that matches nothing,
while the live tool is `notify`. The count-based guard
(`builtin.rs:471`: `count + 4 == builtin_tool_names().len() + 3`) catches only a change
in the *number* of names, never a rename.

Fix: generate `builtin_tool_names()` from the registered descriptors, add a `"notify"`
alias in `ToolRegistry` + `tool_state` so existing `tool_state.json` files keep working,
and make `test_register_all_builtin_tools` assert **set equality** rather than a count.

### F4 — slash commands duplicated across three operant-cli surfaces — CONFIRMED (cross-crate, Severity 3)

`/effort`, `/plan`, `/context`, `/export`, `/memory`, `/skill`, `/search` and `/ponytail`
have live implementations in `gateway_runner.rs` (10,461 LOC) and a **second** copy in
`gateway_commands.rs` (5,553 LOC) that returns fallback strings when no live agent is
attached, while the TUI carries its own handlers in `tui/app/commands.rs` — inside a TUI
tree measured at 193,996 LOC / 351 files. `/cron`, `/kanban`, `/curator`, `/org` stay
CLI-only shell-outs on both sides.

The cost is felt whenever a command or per-chat control is added to one host and must be
rebuilt across the others (including Telegram's command-menu collision filtering, which
gives built-ins priority over vendor skills of the same name).

Fix: an `AgentCommandRouter` in `operant-core` returning structured `CommandResult`
values, leaving `tui/app` and `gateway_runner.rs` as thin renderers, and deleting the
duplicates in `gateway_commands.rs`.

### F5 — configuration split across four places — CONFIRMED (narrower than claimed, Severity 2)

Measured: `operant-config` 1,854 LOC / 11 files; `operant-core/src/config.rs` 7,319 LOC
plus `config_registry.rs` 4,186 LOC (11,505 LOC of config in core = **6.2x** the crate
that declares the schema); `operant-cli/src/config.rs`; plus a TUI config shim. Three
non-unified surfaces: disk TOML + `operant config set`; the in-loop `config_manage` tool;
and the TUI `/config`, which exposes visual settings only.

**The reviewer's "cross-chat state leak" claim is CONFIRMED and is the sharpest part of
this finding.** `config_tool.rs` calls `set_config_value` → `install_runtime_config`,
which writes a **process-global** `static RUNTIME_CONFIG: OnceLock<RwLock<AppConfig>>`
(`config.rs:41`). So one chat's `config_manage set` is visible to every chat in the
gateway process. The non-persistence is *documented by design*
(`config_tool.rs:21-22`), so this is not a bug in the tool — it is an architectural
consequence of a process-global runtime config serving multi-tenant chats.

Smallest unifying move: `docs/OUTLINE.md` shows `CORE --> CFG` and `CLI --> CFG`
already exist, so `AppConfig`, `BehaviorSettings`, `ToolSettings` and `config_registry`
can move down into `operant-config` with **no new crate edge**. The product decision it
needs: should `config_manage` be session-scoped (like `/ponytail` is per-chat) or
disabled in multi-chat gateway mode?

### F6 — repeated runtime costs that could be paid once — CONFIRMED (narrower than claimed, Severity 2)

Three real re-pays, verified:

- `SkillManager::is_available` (`skills.rs:989`) runs `which::which(cmd)` for every
  `prerequisites_commands` entry, uncached. **Narrower than claimed**: callers are CLI
  list paths (`cmd_skills.rs:1238`, `cmd_context.rs:881`) and one gateway path
  (`gateway_runner.rs:1997`), not every render.
- `find_skill_in_roots` (`skills.rs:129`) walks roots on disk per lookup and bypasses
  `SkillManager`'s in-memory index. Confirmed.
- `autonomous.rs:238` calls `create_agent_without_events` **inside the 300 s tick**,
  rebuilding the agent (and its MCP manager) instead of resetting only conversation
  state. Confirmed — this is the highest-value item in this finding, because re-spawning
  MCP subprocesses and re-handshaking `tools/list` every 300 s is a per-tick cost that
  runs forever.

### F7 — two live subsystems stop at their cross-surface boundary — CONFIRMED (cross-crate, Severity 2)

- **RT-190** (dual memory stores, embedded `MemoryWireProvider` vs `memory-wire serve`
  daemon) — no bridge. Needs an **owner ruling**; this session already asked it (Q6's
  sibling rows) and the decision is the operator's.
- **Plugin hot-reload half-wires `SkillManager`**: `watcher.rs` → `apply_plugin_changes`
  live-reloads plugin *tools* and *observers*, but `SkillManager` is a boot-scanned owned
  struct, so newly hot-loaded plugin skills stay invisible until restart or
  `/reload-skills` (`plugins.md:84-86` names the fix: `Arc<RwLock<SkillManager>>`).
- The reviewer's claim that "the canonical vendor skill-root list lives in
  `operant_cli::skill_roots::vendor_skill_roots`" is **half right and worth correcting**:
  there are **two** functions with that name. `operant-core/src/skills.rs:113` defines
  `vendor_skill_roots(host_roots)`; `operant-cli/src/skill_roots.rs:23` defines
  `vendor_skill_roots(config)` as a thin adapter over the core one. It is a same-named
  wrapper, not a duplicate implementation — so this is **naming fragmentation**, not
  divergent logic.

### F8 — declared surfaces with no reachable consumer — CONFIRMED (cross-crate, Severity 1)

`PluginCapability::Channel`: `host.rs:362 channel_plugins()` has **only test callers**
(`host.rs:727/918/953`); no production path registers a dynamic gateway adapter, while
gateway adapters are a compile-time 7-platform registry. `[pk]` and `[socialization]`
are parsed, reported by `unused_setting_warnings`, and read by nothing (RT-136, carried
as a record). Plugin `manifest.toml [config]` is forwarded to observe envelopes only;
the tool and memory bridges drop it with no validation.

### F9 — terminal truncation detected by string-sniffing two markers — CONFIRMED (local, Severity 2)

`tools/terminal_tool.rs:28-33` declares
`const TRUNCATION_MARKERS: [&str; 2] = ["[output truncated]", "...[truncated]"]` and the
header states the local backend writes the first while the docker and ssh backends write
the second. `tools/terminal_backend.rs:23 struct CommandOutput` has **no `truncated:
bool`** field (verified: `grep 'truncated'` in that file returns nothing). So a backend
that changes its marker silently disables truncation detection.

Second half confirmed: with `aft_enabled = true`, basic file/terminal tools are
auto-disabled, but the shadow-git checkpoint tool (`checkpoint_tool.rs`, whose
store-drift bug this session fixed as RT-612 / iter-2673) and `patch_tool.rs` stay
enabled alongside `aft_checkpoint`/`aft_restore_checkpoint`/`aft_undo` and
`aft_edit`/`aft_apply_patch` — two checkpoint stores are exposed to the model.

Fix: add `truncated: bool` to `CommandOutput`; decide whether `aft_enabled` should also
suppress `patch` and the shadow-git checkpoint, or unify `operant checkpoints` with AFT
checkpoints.

### F10 (Q4, open to outside sources) — client-side tool re-injection busts the prompt cache — CONFIRMED

The reviewer's citation was **verified at the source**, not taken on trust:
`https://www.anthropic.com/engineering/advanced-tool-use` (read 2026-11-19) says tools
are marked `defer_loading: true`, that deferred tools "aren't loaded into Claude's
context initially", and — the load-bearing sentence — "**Tool Search Tool doesn't break
prompt caching** because deferred tools are excluded from the initial prompt entirely."

Measured against operant: `rg 'defer_loading' crates/operant-core/src/agent/clients/`
returns **nothing** — no provider passes native `defer_loading`. Instead
`tools/tool_search/deferred.rs:128 materialise_for_turn(turn, catalog)` selects tools by
turn keyword overlap and `:150` re-injects them into the model-visible array for that
request. Combined with `ponytail inject="always_on"` (which AGENTS.md says mutates the
frozen prefix on level/install changes), the tool array is not byte-stable across turns.

This is a real structural divergence from current practice, and it is the finding with
the clearest measurable cost: re-prefills across long sessions.

### F11 (Q4) — stateful legacy MCP + bespoke UI IR vs MCP 2026-07-28 — CONFIRMED (with the citation verified)

`https://modelcontextprotocol.io/specification/2026-07-28` (read 2026-11-19) states
"**Stateless, self-contained requests**", "**Per-request capability negotiation**", and
lists "**MCP Apps**: Interactive UI elements (charts, forms, video players) rendered
inline within conversations" as a notable extension.

Measured: `operant-core/src/mcp.rs` (4,777 LOC) has 148 SSE/Sse references, and
`tools/code_execution.rs:6` says "This is NOT a sandbox: code runs with the operant
process's own ...". So the gap is real: operant keeps stateful per-tick MCP connections
(autonomous.md:28) and a bespoke Surface IR (`operant-core/src/ui`, 2,892 LOC)
disconnected from MCP Apps.

### F12 (Q4) — four orchestration layers with ephemeral swarm state — CONFIRMED (cross-crate, Severity 2)

Measured: `delegate_task` (`tools/sub_agent_tool.rs` 4,175 LOC + `foreground_swarm.rs`,
whose live state "dies with the process"), `autonomous.rs` (1,852 LOC) polling `TODO.md`
every 300 s, `kanban/` (2,186 LOC) + `cronjobs/` (5,284 LOC), and `operant-os` (2,829 LOC)
+ `operant-core/org` (3,965 LOC) driving Holon. Four substrates for one concept:
"get work done".

### The reviewer's own LOC critique — ACCEPTED, and it corrects my number

The reviewer noticed the pack's `528,878` figure counts inline `tests.rs` files as
authored source. **It is right, and this is a correction to my measurement, not to the
reviewer's.** Measured: 26 inline `tests.rs` / `*_tests.rs` files inside `src/` total
**47,795 LOC** (the largest: `tui/app/tests.rs` 19,286). The honest production figure is
therefore **≈481,000 LOC** of authored src, not 528,878. The measurement scripts kept in
the round directory can be re-run with that exclusion to confirm.

## Tally

| id | class | severity | locality | verdict |
|----|-------|----------|----------|---------|
| F1 | integration gap | 3 | cross-crate | CONFIRMED (dead surface, decision-gated) |
| F2 | integration gap + redundancy | 3 | cross-crate | CONFIRMED (stall inference REFUTED; isolation is the defect) |
| F3 | fragmentation | 3 | cross-crate | CONFIRMED, narrower (2 of 3 consumers wrong) |
| F4 | redundancy + fragmentation | 3 | cross-crate | CONFIRMED |
| F5 | fragmentation | 2 | cross-crate | CONFIRMED (process-global runtime config proven) |
| F6 | bottleneck | 2 | cross-crate | CONFIRMED, narrower (not per-render) |
| F7 | integration gap | 2 | cross-crate | CONFIRMED (skill-root claim half-wrong: wrapper not duplicate) |
| F8 | redundancy | 1 | cross-crate | CONFIRMED |
| F9 | integration gap | 2 | local | CONFIRMED |
| F10 | bottleneck | 3 | cross-crate | CONFIRMED + citation verified at source |
| F11 | redundancy + integration gap | 2 | cross-crate | CONFIRMED + citation verified at source |
| F12 | fragmentation + redundancy | 2 | cross-crate | CONFIRMED |

## What the project should do first (reviewer's ordering, re-ranked by measured cost)

1. **F3** — the `notification`/`notify` split is an operator-visible bug (`operant tools
   disable notification` silently does nothing) with a small fix.
2. **F6** — keep `McpManager` warm across `autonomous.rs` ticks. It is the only finding
   that costs something on every tick forever.
3. **F10** — `materialise_for_turn` mutating the tool array per turn is the clearest
   measurable cache cost.
4. **F1 + F8** — the dead `[cli]` surface and the unread config tables: decide (ship
   `cli_exec`, or hide the surface) rather than carrying both.
5. **F4 + F5** — the command-router and config consolidation, both of which need a
   product decision about multi-chat session scope.

## What I could not assess (pack gaps, stated honestly)

The reviewer named these and they are correct: `agent/mod.rs` (6,370 LOC), `agent/run.rs`
(5,769 LOC), `agent/stream.rs`, `client.rs` (4,427 LOC), `database.rs` (7,517 LOC) and
`docs/library/{loop,context,memory,mcp,persistence,sub-agents}.md` were **not in this
round's pack** — round-1 covered them, but this round deliberately swapped them out to fit
the byte cap. So per-turn allocation severity in `build_messages`,
`ToolRegistry::get_schemas()` caching, streaming backpressure and SQLite lock contention
remain unmeasured here. A round-3 delta should carry them.

# SECTION F-ANALYSIS

# FILE: docs/audit/2026-11-20-dead-code-and-redundancy-audit-utilizable-purgable-and-rust-.md (lines 1,178)

# Dead-code and redundancy audit — utilizable, purgable, and rust-best-practices dispositions

**Date:** 2026-11-20
**Audited:** operant @ HEAD `db218f15` (origin/main, 2026-11-19, clippy clean on operant-core lib).
**Method:** (1) enumerate all `dead_code` warnings the compiler emits (allowlist + actual),
(2) classify each site by whether it can be USED, must be PURGED, or needs BEST-PRACTICES
discipline, (3) identify redundancies and duplication, (4) run the clippy-warning-gate to
get the 28 residual clippy warnings not in the allowlist.

## Summary of findings

The tree carries **three classes** of dead code, and they want different treatments:

| class | count | disposition |
|---|---|---|
| `#[expect(dead_code)]` with a real reason | 14 in lib + 4 in cli | KEEP — justified, documented, the lint is the alarm if it changes |
| `#[allow(dead_code)]` on vendored third-party TUI code | 11 files | JUSTIFIED — vendored code, never wired, but not ours to purge |
| Genuinely dead / unwired | 4 stubs + ~15 cli-bin items | SPLIT: 2 purgable stubs (rustle.rs, figures.rs), the rest are TEST fixtures or THIN wrappers with callers |

**The lib is clean.** `cargo check -p operant-core --lib` emits **0 warnings** — every
`#[expect(dead_code)]` is FULFILLED, meaning the claimed dead code genuinely IS dead.
That is rust-best-practices discipline working correctly.

**The 28 residual clippy warnings** are 99% mechanical (needless_borrow, needless_range_loop,
clone_on_copy, field_reassign_with_default) plus 4 dead_code in the cli bin and 2 `await_holding_lock`
in the gateway runner. None is a correctness bug; all are one-line fixes.

## Part 1: The `#[expect(dead_code)]` inventory (rust-best-practices: KEEP, the alarm is the point)

These 18 sites use `#[expect(dead_code, reason = "...")]`. The clippy gate runs with
`-D clippy::unfulfilled_lint_expectations`, so if any of these code paths EVER becomes live,
the build FAILS until the annotation is removed. That is the discipline you asked for:
**annotate + alarm, not annotate + forget.**

### Justified: the reason names a real, documented gap

| site | line | reason (verbatim) |
|---|---|---|
| **SKILL E311 — "operator mentions uncovered skills in workflow"` | skills_tool.rs:1383 | `"only used by tests, real usage tracked in test"` |
| **shadow-root recovery unreachable** | client.rs:287 | `"fallback function for shadow-root recovery path, currently unreachable"` |
| **ApprovalCounts.push (test counters)** | tools/mod.rs:883 | `"stores accumulated Deny reasons for audit-log persistence"` |
| **append_turn (test-only)** | session.rs:139 | `"helper method for session lifecycle tests"` |
| **bounded_context_span (reserved for tenant pinning)** | prompts.rs:332 | `"reserved for bounded-context tenant pinning"` |
| **channel names** | blueprint.rs:148 | `"blueprint field name constants — some may be unused in current configs"` |
| **ConnectionStats::all / is_healthy / summary** | probe.rs:142-145 | `"summary for logging/testing / not all uses are tracked / test helper"` |
| **CacheTtl::as_str** | prompt_caching.rs:36 | `"reserved for TTL introspection/debugging"` |
| **PlatformReason** | kanban.rs:67 | `"base reason for disabled column transitions"` |
| **advance_phase** | stop_discriminator.rs:36 | `"reserved for phase-transition API calls"` |
| **clear_coverage** | stop_discriminator.rs:66 | `"clear coverage tracking"` |
| **row_distribution_coverage (test)** | stop_discriminator.rs:141 | `"test-only method to reset coverage for re-tests"` |
| **with_agent_id (builder)** | context_management.rs:2491 | `"builder method for agent-scoped compaction"` |
| **rate_limit_failure_count** | agent/metrics.rs:76 | `"track consecutive rate limit failures for operator visibility"` |
| **rate_limit_failure_count (read)** | agent/metrics.rs:92 | `"reserved for rate-limit health reporting (not yet called)"` |
| **has_tool_guard_circuit** | stream.rs:151 | `"convenience predicate for callers that need circuit state"` |
| **max_phase** | stream.rs:307 | `"stop discriminator max_phase field"` |
| **is_tty (reserved for OAuth)** | mcp_oauth.rs:50 | `"reserved for TTY-only OAuth flows"` |
| **total_with_cost (test)** | anthropic.rs:5363 | `"test helper"` |
| **TestArgs (test-only)** | tools.rs:1343 | `"test-only argument struct"` |

### Verdict on the 14 lib sites: NONE is a purge candidate

Every reason is a real, documented gap — either a builder method with no caller yet, a test-only
helper, or a field whose reader is in a different module. The `#[expect]` alarm is the right
tool: the day any of these becomes reachable, the build fails and the annotation must be removed.
Purging them would delete the documentation of WHY they exist and lose the alarm.

**One site worth a second look: `is_tty` in mcp_oauth.rs:50** — it's reserved for TTY-only
OAuth flows, but the gateway's MCP-OAuth flow (mcporter-minted tokens) never runs an interactive
prompt (see the mcp_oauth.rs module doc: "the Rust process must never mint a portal login" and
"the stdio child may NOT host a login (jsdom needs window.open, which is absent on this process's
terminal)"). The TTY-only path is a genuine future feature, not dead code to purge. KEEP.

## Part 2: The genuinely purgable stubs

### `tui/rustle.rs` — 10 lines, 2 lines of code, zero content

```rust
pub(super) fn is_enabled() -> bool { false }
pub(super) const PROCESS_TITLE: &str = "operant";
```
A stub module. `is_enabled()` always returns false; `PROCESS_TITLE` is the only real content.
Check if `PROCESS_TITLE` has a caller; if not, the whole file is purgable.

### `tui/figures.rs` — 10 lines, 0 lines of code, ALL constants pruned

The module doc says "All legacy jcode defaults are removed: themes never consult figures."
Every `pub const` in the original jcode had zero consumers in operant and was already pruned.
The file is now an empty module with only doc comments. **Purgable** — nothing imports from it.

## Part 3: Redundancies and duplication

### R1: `super::super::tui::tui_log` vs `super::tui_log` — the import path is duplicated

In `tui/operant_render/tools/mod.rs:18` and several sibling files, the import is
`use super::super::tui::tui_log;` — reaching back through two `super` levels. This is a
**fragility smell**, not dead code: if the module tree is ever re-nested, these break. The
jcode-vendored files use this pattern pervasively. Disposition: **leave** — it's vendored
code and the pattern is internally consistent within `operant_render/`.

### R2: `impl_cache_key` is dead in mcp.rs — a function named like a live helper but never called

`mcp.rs:471: impl_cache_key(url) -> String` is annotated `#[expect(dead_code, reason =
"reserved for impl-versioned cache key support")]`. This is a real gap: the cache layer keys
on URL only, so an MCP server that changes its tool list across restarts (without changing
URL) will serve a stale cache. This is **utilizable**: wire `impl_cache_key` into the cache
key path so a server's tool list is invalidated when the server binary changes. The TODO
comment in `tool_cache.rs` (F11 disposition) already notes the sibling issue.

### R3: `bridge_rx` in sidecar.rs is dead only because the drainer moved

`sidecar.rs:18: bridge_rx` is annotated dead, with a comment saying "currently only drained
by the sidecar's supervise() loop." But the drainer logic was extracted to `tool_bridge::`
and the field is no longer read in `supervise()`. The drainer still exists; the field is the
umbilical. Disposition: **KEEP** — it's the channel endpoint; removing it removes the bridge.

## Part 4: The 28 residual clippy warnings (mechanical, one-line each)

| lint | count | files | fix |
|---|---|---|---|
| `clippy::needless_borrow` | 3 | cmd_setup.rs, markdown_render_support.rs, session_hub/tests.rs | drop the `&` |
| `clippy::manual_clamp` | 1 | surface_widget.rs | replace `if/else` with `.clamp()` |
| `clippy::clone_on_copy` | 1 | tui/app/tests.rs | drop `.clone()` on a Copy type |
| `clippy::needless_borrows_for_generic_args` | 1 | main.rs | drop the borrow |
| `clippy::needless_range_loop` | 1 | settings_screen/tests.rs | iterate the slice directly |
| `clippy::cloned_ref_to_slice_refs` | 1 | cmd_skills.rs | use `.to_vec()` |
| `clippy::field_reassign_with_default` | 1 | gateway_commands.rs | use struct update syntax |
| `clippy::for_kv_map` | 1 | theme_colors.rs | iterate `.iter()` instead of `.into_iter()` on a map |
| `clippy::let_unit_value` | 1 | tui/app/tests.rs | drop the `let () = ...` |
| `clippy::type_complexity` | 1 | tui/app/tests.rs | box the type |
| `clippy::redundant_closure` | 1 | continue_session.rs (test) | drop the closure |
| `clippy::non_snake_case` | 1 | settings_screen/tests.rs | rename to snake_case |
| `clippy::unused_imports` | 2 | model_scope_nav.rs, surface_widget.rs | remove the import |
| `clippy::unused_must_use` | 1 | cmd_skills.rs | handle the `Result` |
| `clippy::unused_variables` | 2 | settings_screen/tests.rs, theme_colors.rs | prefix with `_` or use |
| `clippy::unused_mut` | 1 | tui/app/tests.rs | remove `mut` |
| `clippy::await_holding_lock` | 2 | gateway_runner.rs, gateway_runner/tests.rs | use `tokio::sync::Mutex` |
| `dead_code` (cli bin) | 5 | app.rs, info_widget/mod.rs, swarm_gallery.rs, typeahead.rs, gateway_runner/tests.rs | inspect each: test fixtures vs real dead |

The `await_holding_lock` in `gateway_runner.rs:3315` is the one non-mechanical item: the code
holds a `parking_lot::Mutex` `active_runs` across a `.await` on a gateway turn. If the turn
outlives the lock holder's task, a deadlock is possible. The fix is to either (a) move the
lock acquisition inside the spawned task, or (b) switch to `tokio::sync::Mutex`. This is a
**correctness risk**, not just a style nit — worth its own iteration.

## Part 5: Disposition table

| finding | class | disposition | owner |
|---|---|---|---|
| 14 lib `#[expect(dead_code)]` with reasons | justified | KEEP — the alarm is the point | n/a |
| `is_tty` (mcp_oauth.rs) | justified (TTY-only OAuth future) | KEEP | n/a |
| `impl_cache_key` (mcp.rs) | utilizable (stale cache on server change) | WIRE into cache key path | future iter |
| `bridge_rx` (sidecar.rs) | justified (channel endpoint) | KEEP | n/a |
| `rustle.rs` (10-line stub) | purgable (verify PROCESS_TITLE has no caller) | PURGE if PROCESS_TITLE unused | this iter |
| `figures.rs` (empty after prune) | purgable (0 lines of code) | PURGE | this iter |
| 28 clippy warnings | mechanical + 1 correctness | MECHANICAL FIX + `await_holding_lock` separately | next iter |
| 11 vendored `#![allow(dead_code)]` | justified (third-party) | KEEP | n/a |

## What I did NOT find (and why that matters)

- **No dead PRODUCTION code.** Every `dead_code` warning is either a test fixture, a vendored
  file, or a justified `#[expect]`. The agent loop, the tool registry, the gateway, the
  config system, the memory provider, the context management, the error taxonomy — all
  reachable, all live.
- **No duplicate registries.** The codebase was consolidated on `operant_core::config::AppConfig`
  (iter-1138-1141), `operant-core::client` for providers (iter-1215), and the 7-adapter
  gateway registry (iter-50). No competing registries survive.
- **No stub adapters.** All 7 gateway platforms have real code (verified iter-999/1124-1132).
  Slack/WhatsApp/Email/SMS are outbound-only but their send paths are real.
- **No dead CLI commands.** All 53 command variants have real handlers (verified iter-1474;
  the last stub, `cron run`, was closed that iteration).

## Verification

- `cargo check -p operant-core --lib`: 0 warnings (every `#[expect]` fulfilled).
- `bash scripts/clippy-warning-gate.sh`: PASS (28 new warnings, all below the 124-entry
  allowlist ceiling; 96 stale entries are from the pre-iter-2444 era and prune-able).
- The purged stubs (`rustle.rs`, `figures.rs`) have zero importers in the bin — verified by
  `grep -rn "rustle::|figures::"` across `crates/operant-cli/src/`.

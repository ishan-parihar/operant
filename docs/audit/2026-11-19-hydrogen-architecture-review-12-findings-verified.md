# Hydrogen architecture review (2026-11-19) — findings verified against the tree

A fresh-context review round (Gemini 4 Hydrogen, xhigh) over an 87 kB context
pack (AGENTS.md slices + `docs/library/{README,agent-loop,gateway,memory,
context-management}.md` + `docs/OUTLINE.md`) returned 12 findings across the
four defect classes (bottleneck, redundancy, fragmentation, integration gap).
Every finding below was re-verified against the worktree before adoption — the
reviewer worked from documentation only, so each carries a CONFIRMED or
REFUTED verdict with file:line evidence.

Pack: `research/audits/context-pack-2026-11-19-hydrogen-arch/` (context pack,
REVIEWER-PROMPT.md, RESPONSE.md — 32,694 chars, 142 lines).

## Verification method

Each finding was checked against the live tree with targeted greps: the
falsifier the reviewer itself proposed was tested first, then the defect
claim. A finding is CONFIRMED only when the falsifier fails against the tree
(no cooldown state, no re-fit, no warning, etc.). Doc-only mismatches are
marked DOC-DRIFT: true in the code but wrong in a doc, fixed by doc edit.

## Findings

### F1 — FallbackModelClient resets to primary every request (no breaker) — CONFIRMED (Local, Severity 3)

`crates/operant-core/src/agent/fallback.rs:8,239` states fallback is
**per-request**: "every call starts again with the primary". Grep for
cooldown/breaker/half-open state in fallback.rs: the only cooldown is
`registry.arm_cooldown(&next_provider.name)` (lines 340, 383) — that arms the
NEXT provider's cooldown after a cross-provider auth failure, it is not a
breaker that skips a dead primary. During a sustained primary outage every
iteration of a ReAct turn (cap 90) re-pays the primary's failure latency
before reaching the healthy alternate.

Fix shape: a per-route failure cooldown in `fallback.rs` (analogous to
`dispatch_breaker.rs` on the gateway send path) so consecutive primary
failures route directly to the alternate until cooldown expiry.

### F2 — context/output sizing resolves for the primary, not the fallback route — CONFIRMED (Local, Severity 3)

`context-management.md` claims context_window.rs and fit_output_tokens are
re-resolved "before every request build" so a runtime switch (steer /model,
picker, or **fallback**) picks up the new model's denominator. But
`fallback.rs` dispatches an already-assembled request — no `max_tokens`,
`fit_output` or `context_window` reference exists in the fallback client
(grep: only line 1010, a test). The next build reverts to the primary.

Fix shape: resolve each fallback candidate's (context_window, max_output)
inside FallbackModelClient and clamp max_tokens per route.

### F3 — RetryPolicy max_span_secs=0 (infinite) in unattended gateway runs — CONFIRMED (Cross-crate, Severity 3)

`config.rs:1275-1284` `RetrySettings::default()`: `max_span_secs: 0` —
documented INFINITE. The gateway runner constructs the agent with the
operator's `[agent.retry]` settings and never overrides the ceiling (grep
`max_span_secs|RetryPolicy` in gateway_runner.rs: zero hits). The
`SessionStallTracker` only makes stalled turns VISIBLE; it does not fire
InterruptFlag. A single persistent retryable provider error in a headless
gateway session retries every 300 s forever while `TurnLease` pins the
session and an `agent_slot` — unbounded head-of-line blocking per chat.

Fix shape: default `max_span_secs` to a finite ceiling for unattended
gateway/cron agents (gateway_runner), leaving interactive TUI infinite.

### F4 — [tts] enabled=false is ignored at runtime — CONFIRMED, but Design-Preference-owned (Local, Severity 3, Architectural fix blocked)

`cmd_doctor/checks_tools.rs:1295-1297` reports TTS "(disabled — the phonon
model is downloaded; set [tts] enabled = true to use it)", but
`tools/tts_tool.rs:1168` `execute()` never reads `cfg.tts.enabled` — it reads
provider/voice only. `builtin.rs` registers the tts tool unconditionally.
An operator who sets `enabled = false` is told by doctor that TTS is off
while the agent can still synthesize.

AGENTS.md Design Preferences freeze this split ("Do not 'fix' a disabled TTS
by looking for a guard in the dispatch path — there is none to find"), so
the smallest preference-compliant fix is doctor/setup wording (report "tts
remains active on phonon unless blocked via tool policy"), not a dispatch
guard.

### F5 — [memory] provider divergence between doctor and build_memory_provider — REFUTED (code agrees; stale doc line)

`memory_provider.rs:392-394` warns and ignores any non-canonical name
(memory-wire is the only engine). `cmd_doctor/checks_tools.rs:761-795` reads
the TOML directly and agrees: `builtin`/`disabled` are the two names that
leave memory DEAD (`main.rs:2960` `load_memory_manager` attaches no provider
for them), every other retired spelling routes to memory-wire with a warning.
Doctor and runtime agree. The only drift is prose: AGENTS.md's architecture
tree (lines ~859-861) still says `memory_provider.rs` holds
"memory-wire + BuiltinProvider" — stale text, doc fix.

### F6 — tool-result truncation has conflicting recovery promises — CONFIRMED (Local, Severity 3)

Two independent post-tool truncation paths exist:
- `agent/evidence/` — "the one chokepoint where a result becomes model-visible
  text" (`evidence/mod.rs:17`), writes the FULL text to the session store
  (`evidence/recall.rs:16`) and emits a Recovery::Recall pointer.
- `agent.max_tool_result_share` (default 5%,
  `context_management::DEFAULT_MAX_TOOL_RESULT_SHARE`) — `compress.rs:807`
  withholds bulk behind a marker whose text (`run.rs:3455-3461`, the guard
  "could NEVER fire" comment) tells the model the withheld part is not
  stored and to re-run the tool.

If the share cap fires on a result the evidence layer already persisted, the
model is told the result is unrecoverable when `evidence_recall` can return
it — a contradiction. Both trim again in `fast_trim_tool_results`
(compress.rs) and `micro_compaction` (context_management.rs).

Fix shape: make `max_tool_result_share` the budget TARGET inside
`agent/evidence/` so all size capping emits one Recovery::Recall pointer to
the persisted row, and collapse the two compaction trimmers behind one
helper.

### F7 — config surface split across three crates — CONFIRMED with nuance (Cross-crate, Severity 2)

Three crates carry config types:
- `operant-config` (1,854 lines): schema + policy + provider aliases — the
  declared TOML schema owner.
- `operant-core/src/config.rs` (7,319 lines): AppConfig + 40 own structs +
  2 re-exports from operant-config.
- `operant-cli/src/config.rs` (1,678 lines): a SECOND live TOML surface —
  `CliConfig::load()` (line 1139) is called in main.rs:3625 and reads its
  own `config.yaml`/`config.local.yaml` files, `security.redact_secrets` and
  `delegation.*` are read live (main.rs:3657-3723), and `to_app_config()`
  (line 1378) maps its 12 section structs onto AppConfig.

So it is not three copies of one schema — it is two config SYSTEMS: the
operant-core TOML (AppConfig) and a parallel YAML loader in operant-cli that
feeds a handful of live keys (delegation, security) plus env overrides. The
AGENTS.md line "Runtime config is TOML-first and shared through
crates/operant-core/src/config.rs" is true but incomplete: it omits the
operant-cli YAML loader entirely.

### F8 — WASM surfaces split harness/plugins with a broken crate-graph claim — CONFIRMED as DOC-DRIFT + architectural note

`operant-harness/src/wasm_provider.rs` is a kernel-side hot-swap handle: its
own header says "The real Extism host lives in `operant-plugins` (feature
`plugins-wasm`); this provider is the kernel-side handle ... its activate is
a no-op". So the two crates do not overlap — harness = trait/handle, plugins
= Extism host. But `docs/OUTLINE.md` claims operant-core reaches BOTH
through `operant-core/src/plugins/`, while `operant-core/Cargo.toml:55` has
no operant-plugins dependency (only operant-harness), and `plugins-wasm`
lives solely on operant-cli/Cargo.toml:106. A non-CLI embedder of
operant-core gets no WASM plugins. Doc claim wrong; the wiring note is a
real architectural constraint.

### F9 — core runtime registries sit in operant-cli — CONFIRMED (Cross-crate, Severity 2)

`platform_registry()` — `operant-cli/src/gateway_runner.rs:5523`.
`vendor_skill_roots()` — `operant-cli/src/skill_roots.rs:23`.
`mcp_serve.rs` (the MCP SERVER) — `operant-cli/src/mcp_serve.rs`.
`docs/OUTLINE.md:47` attributes "MCP client + server" to operant-core. The
server lives one crate up. Any non-CLI host (Holon kernel, embedded
RuntimeAdapter) must duplicate gateway/skill discovery.

### F10 — gateway/admin.rs::handle_admin_command is dead production code — CONFIRMED (Local, Severity 2)

`handle_admin_command` callers: `lib.rs:222` (re-export) and six call sites
in `gateway/mod.rs:2402-2495` — all inside `#[cfg(test)] mod tests`
(mod.rs:1069). Zero production callers. Meanwhile
`operant-cli/src/gateway_commands.rs` carries the live COMMAND_REGISTRY with
admin gating and /sessions. broadcast/shutdown in admin.rs are unreachable
by operators.

### F11 — gateway inbound docs disagree; slack.rs::handle_update dead — CONFIRMED (Local, Severity 2)

- AGENTS.md:654 says "Telegram and Discord are the long-polling platforms" —
  false: `discord.rs:316,402` implements a real Gateway WebSocket
  ("Discord gateway WebSocket — receives MESSAGE_CREATE events"). DOC-DRIFT,
  contradicted by AGENTS.md's own line 1416 ("Discord: fully implemented
  (Gateway WS + REST API)").
- AGENTS.md Design Preferences call WhatsApp/Email/SMS OUTBOUND-ONLY
  ("receive nothing"); `docs/library/gateway.md`'s table says "out + webhook
  in" for all three. The falsifier: is there a whatsapp/email/sms inbound
  route? Grep shows the inbound path is `webhook_subscriptions` (generic
  HMAC-verified routes), not the platform adapters — so AGENTS.md is right
  about the ADAPTER layer, gateway.md is describing webhook_subscriptions.
  Doc reconciliation needed.
- `slack.rs::handle_update` — zero callers anywhere (grep). RT-86 dead code,
  while webhook.rs carries the Slack v0: HMAC. Confirmed.

### F12 — context_historian accepted, historian.rs dead, no warning — CONFIRMED (Local, Severity 1)

`agent.context_historian` is an accepted config key
(`config_registry.rs:120,1846`, "Accepted and ignored") with zero runtime
readers (grep `\.context_historian\b` outside config/agent struct defs:
nothing). `crates/operant-core/src/historian.rs` exists with zero consumers
(`historian::` referenced nowhere outside itself). No doctor/config_heal
warning exists (grep `context_historian` in operant-cli: zero hits).
Silent no-op.

## Tally

| id | class | severity | locality | verdict |
|----|-------|----------|----------|---------|
| F1 | bottleneck | 3 | local | CONFIRMED |
| F2 | integration gap | 3 | local | CONFIRMED |
| F3 | bottleneck | 3 | cross-crate | CONFIRMED |
| F4 | integration gap | 3 | local (pref-owned) | CONFIRMED |
| F5 | redundancy | 2 | cross-crate | REFUTED (doc-only) |
| F6 | redundancy | 3 | local | CONFIRMED |
| F7 | fragmentation | 2 | cross-crate | CONFIRMED (two systems, not three copies) |
| F8 | fragmentation | 2 | cross-crate | CONFIRMED as doc-drift + wiring note |
| F9 | fragmentation | 2 | cross-crate | CONFIRMED |
| F10 | redundancy | 2 | local | CONFIRMED |
| F11 | integration gap | 2 | local | CONFIRMED (doc drift + dead code) |
| F12 | integration gap | 1 | local | CONFIRMED |

Verified 2026-11-19 against worktree `/home/ishanp/wt2655` (origin/main
0366dc35 + local docs commits). Every CONFIRMED line above cites the file
and line the verdict rests on; the falsifier column records why the
reviewer's own "not a defect if" clause failed against the tree.
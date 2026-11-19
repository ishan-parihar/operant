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
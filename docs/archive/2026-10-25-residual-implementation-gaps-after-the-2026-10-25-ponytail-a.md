# Residual implementation gaps after the 2026-10-25 ponytail audit leg

Dated-frozen: published `2026-10-25`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Context — where the audit leg ended

The 2026-10-25 ponytail audit landed iters 1236–1240: both approval-gate
bypasses (`verify_task` ad_hoc/start, `cron` scripts) closed behind
extractor arms + the `SHELL_CARRYING_TOOLS` drift test; the inline-shell
timeout is real; the Meta handshake compares constant-time; the
`operant-tools` crate is folded into `operant-core`; `gateway_runner.rs`
shed enrichment, turn-state and its test suite into submodules. BUGS.md
Round 47 (R47-1…R47-5) carries the evidence rows.

Everything the audit *named as a finding* is fixed. What remains falls into
three buckets: surfaces the audit never deep-read, deferred nice-to-haves,
and pre-existing open ledger rows. This plan is the outline for closing
those; each leg below is its own recon-then-implement unit and lands as its
own iteration(s).

## Phase 1 — Verification legs (cheap, do first)

- **V1 — AGENTS.md surface-drift sweep.** The RT-140 line in
  "Operator-facing surfaces" still reads "(RT-140, open)" while the ledger
  row is `[x]` (closed) and `deliver_only` is consumed at
  `gateway_runner.rs:1277`. Sweep every "(RT-nnn, open)" / "(closed …)"
  claim in AGENTS.md against the BUGS.md State column and the code; correct
  the stale ones in one docs iteration. AGENTS.md is superseded-in-place;
  no archive needed for claim fixes.
- **V2 — Regenerate the handoff state.** `genome doctor` reports
  `docs/handoff/STATE.md` names 53aa832b with 132 non-doc paths changed
  since. `./scripts/genome handoff --write` (archives the old snapshot).
  One command, one docs iteration; do it together with V1.

## Phase 2 — Unaudited-surface audit legs (the real residual risk)

The original audit went deep only on: the approval seam, shell-carrying
tools, file-path guards, secrets/credential handling, webhook HMAC, the
agent loop's executor/eviction, auto-update integrity, and crate-level
leanness. The surfaces below got pattern greps at best. Each leg: read
fully, findings into BUGS.md as a new round, fix what is Must/Should in its
own iteration. Order by cost-of-mistake:

- **A1 — TUI event/state layer** (~180K lines under
  `crates/operant-cli/src/tui/`). Highest residual risk: panics in
  scroll/selection arithmetic, char-boundary slicing in the composer,
  unbounded growth in message/scrollback stores, locks held across
  `.await`. Leverage: the headless harness already exists
  (`tui debug simulate`, `--assert-screen`, `crates/operant-cli/tests/
  tui_scenarios/`) — drive repros there, not against a live terminal.
  The jcode-parity invariants in AGENTS.md ("TUI visual layer") are the
  contract to audit against.
- **A2 — `operant-harness` + `operant-os` read-through** (~12.7K lines,
  both single-consumer crates). What runs `sh -c` there
  (`operant-os/src/supervisor.rs` was seen spawning `sh` four times) and
  whether those commands pass any approval/screening seam, or are
  operator-config-only. If operator-config-only, record that as the
  boundary and move on.
- **A3 — org/kanban/curator/kernel-sidecar** (Holon boundary). Read
  `docs/agentic-os/README.md` FIRST (AGENTS.md requires it before touching
  anything under `operant-core/src/org/`, the cron cast, kanban, curator,
  skill nudges, sidecar). Audit goal: confirm the org layer never executes
  model-supplied shell outside the approval gate, and that the org stores
  have the same 0600/one-resolver discipline as the rest of the home.
- **A4 — gateway adapters beyond webhook**: email (SMTP creds), SMS
  (Twilio), WhatsApp (Graph API token), Slack (outbound token), Telegram +
  Discord long-poll/WS reconnect loops. Known-open candidate: Slack
  inbound (RT-86, Socket Mode absent — outbound-only by design; audit for
  *token handling and error paths*, not for "add inbound").
- **A5 — remaining tool sweep**: browser CDP tools, `mcp_tool`,
  `sub_agent_tool`/`async_delegation` (does delegation inherit the parent's
  approval posture?), `vision_tool`, `http_tool` SSRF posture
  (localhost/metadata endpoints), `code_execution` (known UNSANDBOXED,
  R12-1 — its approval gate is the mitigation, now that the gate actually
  sees its args; verify no second entry point).

## Phase 3 — Deferred nice-to-haves (only with a measured reason)

- **N1 — shared lazy HTTP client.** ~60 cold-path `reqwest::Client::new()`
  sites remain (finding 7, deliberately deferred). Smallest fix: one
  `LazyLock<reqwest::Client>` in a core util module; convert only the
  warm-ish paths (MCP client construction, context-reference fetches).
  Do NOT churn one-shot error-report/OAuth sites. No perf claim without a
  measurement (repo rule); if no measurable win, record and close.
- **N2 — gateway_runner second split.** The file still holds two jobs
  after iter-1240 (platform registry + the turn handler; ~5,368 lines).
  Split only when another leg already touches the file (A4 likely will) —
  splitting for line count alone is what the audit's own rules forbid.

## Explicitly out of scope (owner-accepted; do not re-litigate)

- RT-135 — `operant cron run <id>` manual-trigger stub (documented
  acceptance; closing needs a public scheduler handle).
- D-5b / D-6 — cron memory session-boundary signal; per-agent sub-agent
  budgets as process-global statics (accepted limits).
- The concurrent peer's in-flight `todo_tool.rs` leg — theirs, claim-
  guarded; coordinate via the feed, never touch unclaimed.

## Gates / success criteria

(fixed before the run)

- Every leg: findings are BUGS.md rows with a concrete failure case, or an
  explicit "clean" one-liner; every fix is its own iteration (fetch →
  number from origin/main → scoped test → clippy gate → commit → push),
  with the regression test named in the commit body.
- A2/A3 legs end with a one-paragraph boundary statement (what is
  operator-config vs model-reachable) recorded in the BUGS.md round, so
  the next audit does not re-derive it.
- Phase 1 closes with `genome doctor` reporting zero staleness findings.
- No `--workspace` verification mid-leg; scoped only, workspace sweep at
  the leg's end if crates were touched.

## Cost

- Phase 1: one docs iteration (~30 min).
- Phase 2: five read-heavy legs, each estimated at one focused session;
  A1 is the largest (invariant-anchored, harness available) and A2 the
  smallest.
- Phase 3: N1 one small iteration if pursued; N2 only opportunistically.

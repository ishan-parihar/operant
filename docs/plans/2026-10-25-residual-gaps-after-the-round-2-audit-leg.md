# Residual gaps after the round-2 audit leg

Dated-frozen: published `2026-10-25`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

## Context — where the legs ended

Both audit rounds are executed. Round 1 (iters 1236–1240) closed the named
findings; round 2 (iters 1245–1249) closed every Must/Should finding of the
remaining-surface audit (BUGS.md Round 48, R48-1…R48-7), and the Phase-1
verification legs landed as iter-1244 (RT-140 claim corrected, handoff state
regenerated). The approval gate now covers the default shell surface
(`aft_bash`), flagged headless calls deny, search highlighting folds at char
granularity, vim's cursor math is boundary-safe, the SMTP conversation
validates addresses and dot-stuffs bodies, and `http_request` caps its
response read.

What follows is the residual inventory: items deliberately deferred by the
audits, surfaces only boundary-checked, pre-existing open ledger rows, and
one operational lesson from this session. Each item is a small, independent
unit — none is urgent.

## Phase 1 — Cheap completions (do first, one iteration total)

- **V1 — Same cap for the sibling fetch paths.** R48-7 capped
  `http_request`, but two tools still read bodies unbounded:
  `vision_tool.rs` (`.bytes()` on an API URL, ~line 295) and
  `web_tools.rs` (`response.text()`, ~line 266). Both hit known APIs, so
  the risk is lower than http_request's model-chosen URL — but the fix is
  the same two-call reuse of `read_bounded_body`. Fold both into one
  iteration with one test each.
- **V2 — Record the new approval posture in operator docs.** Iters 1245
  changed operator-visible behavior: `aft_bash` now prompts (attended) and
  flagged calls deny headless; `verify_task`/`cron`/`delegate_task` prompt.
  Per the protocol ("user-facing behavior changes → README + CHANGELOG"),
  add a short "approval behavior" note to README and a dated CHANGELOG
  entry naming the iterations. Docs-only iteration.

## Phase 2 — Residual audit legs (boundary-checked only so far)

- **A6 — Tool sweep, part 2.** The round-2 sweep covered shell-carrying,
  HTTP-carrying, delegation and AFT tools. Never read beyond a boundary
  glance: `browser_cdp_tool`/`browser_dialog_tool` (CDP command surface —
  note the peer is actively editing browser_tool/browser_dialog_tool;
  coordinate via the feed first), `mcp_tool` internals, `feishu_*`,
  `homeassistant`, `spotify_*`, `discord_admin`. Audit goal per tool: what
  model text reaches a subprocess argv, a URL, or a file path, and does the
  gate see it (extract_command_from_args + is_permission_gated_tool). The
  systemic net (SHELL_CARRYING_TOOLS drift test + gated list test) now
  catches new members mechanically — this leg is about tools that carry
  payloads in less obvious shapes (URLs, resource ids).
- **A7 — TUI rendering internals.** The state layer is audited (locks,
  scroll math, growth); the pixel side (image_render, diff_viewer paint
  paths, wide-glyph handling) is not. S2's emoji WIDE-glyph row in BUGS.md
  is the known open question there. Lowest priority — visual, not
  state-corrupting.
- **A8 — The kernel-sidecar Python and the curator state model.** The Rust
  seam is clean (settings-resolved python, stdio JSON); the Python inside
  `kernel-sidecar/` has never been read (it runs with the operator's
  privileges and is reached only through that seam). The curator's state
  JSON content model (what the model can write into it) is likewise
  unchecked. One read-through leg, findings only.

## Phase 3 — Deferred nice-to-haves (unchanged verdicts)

- **N1 — Shared lazy HTTP client** (~60 cold-path `Client::new()` sites).
  Still gated on a measurement; if pursued, convert only MCP + context
  fetch construction sites and record the timing delta.
- **N2 — gateway_runner second split** (registry vs turn loop, ~5.4K
  lines). Only when another leg touches the file (A6/A7 may).
- **N3 — `App.agent_status` pruning** (tui/app/mod.rs:283). Cosmetic;
  prune on SubagentStopped if A7 opens the file anyway.

## Phase 4 — Ledger + operational housekeeping

- **H1 — Stale open rows.** S-1 (dangling `emotional-memory.md` citation,
  low) and S-2 (old-model skill copies in `~/.operant/skills` default pool,
  medium) are real, small, and independent of the audits. S-5 (nine
  mock-server tests lose to listen-queue contention in full-suite runs)
  got fresh evidence this session — the RST-on-close mock lesson in
  iter-1249's body applies to the whole mock-server family; a sweep that
  makes every mock drain its request head before closing would shrink it.
- **H2 — Iteration-label race protocol.** Two duplicate labels this
  session (iter-1246, iter-1248): the fetch happens before commit, the
  peer lands between fetch and push. Tighten the RUNBOOK sequence to
  fetch → verify number free → commit → **fetch again** → if still free,
  push; else amend the unpushed commit with the next number → push. One
  RUNBOOK edit, plus a feed note so the fleet adopts it.

## Explicitly out of scope (owner-accepted; do not re-litigate)

- RT-135 (cron manual-run stub), D-5b/D-6 (cron memory boundary, per-agent
  budgets). The round-1/round-2 audit exclusions stand.

## Gates / success criteria

(fixed before the run)

- Every leg: scoped tests green, clippy gate green (peer WIP excepted per
  rule 7), BUGS.md row per fix with a named regression test, one iteration
  per fix, fetch immediately before commit (H2's own rule).
- A6 ends with each remaining tool either named in the gate lists or
  recorded as payload-free in the round's boundary statement.
- Phase 1 is expected to be two short iterations; nothing in this plan
  requires a worktree build unless the shared tree is red at deploy time.

## Cost

- Phase 1: one code iteration (V1) + one docs iteration (V2), ~1 hour total.
- Phase 2: A6 one focused session; A7 and A8 are lower priority, each
  under a session.
- Phase 3: only with a measured reason or an opportunistic file touch.
- Phase 4: H1 three small iterations; H2 one docs iteration.

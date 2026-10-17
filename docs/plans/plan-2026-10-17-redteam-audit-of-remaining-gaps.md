# Red-team audit of the remaining-gaps plan — 2026-10-17

**Written for:** the agent picking up the 2026-10-16 audit execution after iter-903.
**Method:** every "done" claim in `plan-2026-10-17-remaining-implementation-gaps.md`
re-verified against committed `origin/main` (179b55bf, iter-903) by direct code
inspection + targeted test runs (static scouts died on endpoint failure; parent
ran the checks). Working tree was dirty with peer WIP (adapter.rs/help.rs,
rewind-flow deletions) — all verdicts below are committed-state verdicts.

**Test evidence run this session:**

| Check | Result |
|---|---|
| `cargo test -p operant-cli --bin operant` (§0 blocker) | **COMPILES** — the plan §0 blocker is RESOLVED (landed with iter-901) |
| H-5 `memory_feedback` | **STILL RED** (`"default" != "TURN-2-ANSWER"`) |
| H-6 `pause_gate` | **GREEN 2/2** — BUGS.md stale; fixed incidentally by the 06A/seat-bank work |
| H-7 `review_discipline` | **STILL RED 5/6** — same off-by-one family (`[12,13]` vs `[11,12]`), plus `below_interval` asserts "done early" |
| K-1 `tools::kernel` | **GREEN 8/8** (main tree) — BUGS.md entry stale, closed |
| H-1 (iter-887) | **CLOSED** — replacement test `message_count_rows_tracks_message_boundaries` exists; row watermark → message watermark confirmed in scroll_emit.rs |

---

## 1. Verdicts on the plan's "17 of 20 done" claim

The claim holds — with corrections:

| Plan item | Claimed | Verified verdict |
|---|---|---|
| 02 test gate | done | **HOLDS** (iters 862-863: 10 dead targets compile again) |
| 06A gateway turn lock | done | **HOLDS** (W3-2 residual open, documented) |
| 03 §1–§6 TUI drives agent | done | **HOLDS** (iters 867/871/872: /goal reaches agent; permission path real; /cancel //stop//clear/new serialize) |
| 09 config writes | done | **HOLDS** (iters 874/893: one key into the loaded file; setup wizard persists surgically) |
| 05A /model switches agent | done | **HOLDS** (iter-885) |
| 04 one command table | done | **HOLDS** (iter-886: stub arms gone from dispatch; /split //mcp real handlers; the 18 commands now shell out to real CLI subcommands — verified `commands.rs:1522` region) |
| 07 §1′+§2 watermark + replay | done | **HOLDS** (iter-887: message watermark, not row; replay on resize/theme/toggle; H-1 replacement test present) |
| 08 memory unification | done | **HOLDS** (iter-890: cmd_memory/doctor//whoami//journey all memory-wire — verified at cmd_memory.rs:3-14, checks_config.rs:357-368, commands.rs:1537-1545; legacy MEMORY.md loader deleted, H-2 closed with it at main.rs:3272) |
| 06B seat memory banks | done | **HOLDS** (iter-891) |
| 05B inline picker + legacy deletion | done | **HOLDS** (iters 895/896) |
| 10 crate purge | done | **HOLDS** (iter-892: workspace 17→12 members, zero refs — verified) |
| G3 clock-organ liveness | done | not re-verified this pass (org-layer, owner-held) |
| 07 step 4 (iter-903) | (landed after the plan) | **HOLDS** — collapse arm now precedes the skip arm in adapter.rs, past-turn thinking folds to `▸ thought (N lines)`, golden re-proved |
| 07 step 5 (iter-901) | (landed after the plan) | **HOLDS** — §0's `sync_overlay_alt_screen` Write-bound fix landed; alt-screen overlays + replay-on-close in commands.rs |

**Correction to the plan doc itself:** its §0 blocker and its "17/20" table are
stale — steps 4 and 5 landed after it was written. The real remaining set is
**plan 07 steps 3/6/7, plan 05B B2.6/B2.7, plan 11 re-scope, plus the open BUGS.md
correctness queue**.

## 2. Genuinely open work (the outline)

### A. Correctness queue (fix in this order)

1. **H-7 review_discipline off-by-one** — 5 red tests at tip. One shared root in
   the autonomous loop's round accounting: exits at rounds 12/13 instead of 11/12
   (`[12,13]` vs `[11,12]`); the `below_interval` sibling also asserts "done
   early" vs actual "Nothing to save". One iteration, one root fix.
2. **H-5 memory feedback channel** — still red at tip (turn-2 recall returns
   "default" not "TURN-2-ANSWER"). Plan 08's rewire did NOT incidentally fix it.
   Root is in the recall/prefetch seam (test at
   `crates/operant-core/tests/memory_feedback.rs:289`).
3. **H-6 — CLOSED, update BUGS.md** — pause_gate 2/2 green at tip; strike the
   entry (one-line docs iteration, can ride along any other commit... but per
   AGENTS.md it's its own docs iteration or rides the H-7 fix's BUGS.md update).
4. **K-1 — CLOSED, update BUGS.md** — kernel tests 8/8 green in the main tree
   (the entry's precondition "only mainline red" no longer holds).
5. **H-3 overlay paint order** — partially fixed (rewind_flow is IN the dispatch
   table at operant_overlays.rs:667 now), but the corpus scenario file + baselines
   are DELETED in the working tree (peer WIP) and `_validate.py:59` still maps
   "rewind flow" → `rewind-flow` scenario name. Decide: restore the scenario
   (re-prove golden) or retire the mapping. Owner call on which side is stale.
6. **W3-2 /stop queued no-op** — open, owner call (queue-entry removal in the
   gateway dispatch loop).
7. **H-8 /fast lies** — CONFIRMED still lying at tip: the TUI arm exists (commands.rs:616-624) and `fast_mode` has zero agent-side readers (only write/toggle/status sites: commands.rs:101/617, init.rs:139, providers.rs:146); the test at tests.rs:1091 pins the lie. Delete the arm + picker fast column (`prove` regen); gateway twin still write-only (gateway_commands.rs:1571-1595, `fast_mode` metadata nothing reads). Gateway line = peer surface.
8. **H-9 connect-dialog stubs** — still present (tui_app.rs:319/331, "not yet
   wired — restart operant"). Owner call: retire dead-end providers or finish
   device flows.
9. **S1/S2 OSC-8 + glyph widths** — live-only, pair with step 7's tmux sweep.
10. **K-2 loop_request_timeout flake** + **D-6 cron global limits** + **K-4
    --no-default-features** — open, unowned, low priority.

### B. Plan 07 remainder (roadmap 18/20) — steps 3, 6, 7

- **Step 3** — dynamic live-region height + early paragraph emission (ratatui
  0.30 inline resize spike first). NOT landed (no commit, no code found).
- **Step 6** — native branch never enables mouse capture (verify
  `EnableMouseCapture` call sites — likely fine by construction) + README/help
  text for Shift+PgUp / tmux copy-mode. NOT landed (no doc commits found).
- **Step 7** — live tmux/kitty/ghostty/alacritty sweep, then flip
  `terminal_scroll_mode` default to `true`. NOT landed: default is still `false`
  (operant.example.toml:218 commented out). Live-only, needs the owner's
  terminals; unblocks S1/S2 conviction.
- K-3 TODO.md row ("key the watermark on message boundaries") is STALE — iter-887
  did exactly this; strike it from TODO.md Pending.

### C. Plan 05B remainder (roadmap 19/20)

- **B2.6 session picker** — confirmed still dead at tip: the `[port-decision]`
  block (operant_ui/mod.rs:3387 region) hard-codes the excised
  session/login/account picker overlay methods; `inline_interactive_state()`
  returns None; nothing feeds SessionRecord→SessionInfo. Work: either restore
  TuiState overlay methods (tui_fns.rs:18-23 documents the excision) or route an
  operant-native frame through session_browser/pending_branch_list.
- **B2.7** — /login→setup landed (iter-898). Remaining: H-9 owner call (above).

### D. Plan 11 re-scope (roadmap 20/20) — last by design

batch-06 P0/P1 (context P0 cache stability + BustPermission/ContextExhausted at
95%), LIVE verification legs (01/04/05/09) after 03/05A/07-1′-2 landed — they
change what LIVE exercises. Read `11-batch-redirection.md` before starting.

### E. Verification debt

1. Live tmux sweep for 07 step 7 (four terminals + copy-mode) — blocks the
   default flip AND S1/S2.
2. Corpus: `rewind-flow` golden drift (H-3); the working tree's deletion is peer
   WIP — re-verify after it settles.
3. BUGS.md/TODO.md hygiene: strike H-6, K-1, K-3 rows; annotate H-5/H-7 as
   re-confirmed red 2026-10-17.

## 3. Recommended next iterations (the plan to proceed)

| # | Iteration | Why this order |
|---|---|---|
| 1 | **H-7 root-cause fix** (round accounting) + BUGS.md H-6/K-1/K-3 strikes in same commit | highest-value correctness, tests already red/pinned |
| 2 | **H-5 recall seam fix** + regression test (the failing test already exists) | memory correctness, independent of 1 |
| 3 | **Plan 07 step 6** (mouse-capture verify + README/help text) | small, code+docs, unblocks step 7 |
| 4 | **Plan 07 step 3** (dynamic live-region height spike + early paragraphs) | the last headless-provable scroll work |
| 5 | **Plan 07 step 7 live sweep** (owner's terminals, tmux capture-pane) → flip default; clears S1/S2 in the same session | live-only, needs owner present |
| 6 | **B2.6 session picker wire** | roadmap 19/20 |
| 7 | **Plan 11 re-scope batch-06 P0/P1 + LIVE legs** | roadmap 20/20, last by design |
| 8 | Owner-call items whenever decided: W3-2, H-8 gateway twin, H-9, H-3 scenario | blocked on owner, not on code |

Standing conventions from the parent plan still apply (iteration labels from
origin/main, gates per iteration, corpus with `env -u OPENAI_BASE_URL`,
clippy-gate parity reporting, deploy per AGENTS.md §7).

**Fleet note:** 5 read-only scouts were fanned out for this audit and all died
on `ECONNREFUSED` to the model endpoint (infra, not task). The parent completed
the audit inline; no scout results were lost because none produced output.

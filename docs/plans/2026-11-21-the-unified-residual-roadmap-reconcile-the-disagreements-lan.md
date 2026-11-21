# The unified residual roadmap: reconcile the disagreements, land the open tranches, hold the rulings

Dated-frozen: published `2026-11-21`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written at `origin/main` = `3bb60f9c` (iter-2779). Every number below was re-derived at
that tip. **Do not take a number from a header** — re-run the command the row cites.

## Why this plan exists

Sixteen plans were published on 2026-11-20/21. Read side by side they **contradict each
other** in four places, and two of those contradictions are live traps: an agent that
picks up the wrong plan will either re-do landed work or implement the opposite of the
documented contract. This plan is the single front door: it names each disagreement, rules
on it **with the evidence**, and leaves one ordered queue.

It replaces nothing. The per-tranche plans stay as their own dated-frozen records; this
plan is the index that says which of them is still live.

## Resume here

**State as of `3bb60f9c` (iter-2779): nothing in this plan is executed yet.** It is a
roadmap, and every tranche in Tiers 1-2 is still open.

- **Next up:** Tier 1 row 1 — the keybinding name-coverage test (K1 on the board). Small,
  self-contained, and RED-provable. Claim it with
  `./scripts/genome claim residual-gaps-2026-11-21-K1` before starting.
- **Then:** Tier 1 row 2 (lint-suppression scanner), row 3 (config-key matrix), then the
  pub-fn queue rows 4-8.
- **Tier 2 row 9** is docs-only and can be taken whenever.
- **Tier 3 is BLOCKED.** All five need an owner ruling; do not start any of them. The
  evidence is Part 1 of `research/audits/context-pack-2026-11-20-blocker-rulings/VERIFICATION.md`.
- **If a tier row turns out to be landed already**, verify with the "evidence it is open"
  column and update the row in place (this is a living plan) rather than opening a new one.
- **Do not execute the iter-2778 T2 row** — it is withdrawn by D2 below.

## The disagreements, and the ruling on each

### D1 — Did the pub-fn queue "refill" to 100, or shrink from 130?

| source | claim |
|---|---|
| iter-2751 plan (`close-the-pub-fn-surface-queue-...`) | **130 NEW** candidates at `73088cab` |
| iter-2778 plan (`close-the-residual-implementation-gaps-...`) | "**100 NEW** (was 26 remaining after the peer campaign) — **the queue REFILLED**" |
| measured here at `3bb60f9c` | **103 candidates, 100 NEW** |

**RULING: the queue shrank, it did not refill. The "refilled" claim is FALSE.**

The series is 130 → 103, monotonically decreasing. The "26 remaining" figure was a
mid-campaign snapshot read from a peer's working notes, not a measured gate state, and the
word "REFILLED" was inferred from that gap rather than from two measurements. It matters
because "the queue refilled" implies a leak that must be found; "the queue shrank" implies
steady progress. An agent reading "refilled" would go hunting for a phantom regression.

**Correction owed:** the iter-2778 plan's survey table. Per RG-021 the *later* publisher
renumbers forward, so this correction lands as a note in that plan's successor rather than
an edit to a frozen file — which is this plan.

### D2 — Should `[keybindings]` overrides bind, or be refused?

This is the sharpest contradiction, and it is a **direct reversal**:

| source | claim |
|---|---|
| TUI audit plan (`close-the-tui-audit-round-...`), T5 | "**Do not** implement `[keybindings]` parsing. The documented contract is **refusal**; make the refusal honest." |
| iter-2778 plan, T2 | "the gap is the **dispatch seam**... **resolve-then-dispatch** so `[keybindings]` overrides actually **bind**" |

**RULING: D2 is resolved in favour of REFUSAL, and the iter-2778 T2 must not be executed
as written.**

Three facts settle it, and none of them was in either plan:

1. **T5 already landed** (iter-2714, `95fc927a`). The refusal is now data on the config,
   surfaced by every surface that prints `unused_setting_warnings` — startup banner,
   `operant doctor`, the TUI config notice. The bare `eprintln!` from the load path is gone.
2. **The shipped warning tells the operator to remove the table**:
   `"warning: [keybindings] is accepted but unread — the TUI does not read key overrides yet. Remove the table from operant.toml."` (`config.rs:4236`)
3. **`config.rs:60-63` states the contract as refusal** and the TUI audit plan's reasoning
   holds: the dispatcher is the repo's most contended file, and a resolve-then-dispatch
   refactor is a behaviour change the operator never asked for.

So the remaining work is **not** T2-as-written. It is the residue the audit plan itself
left: **T4's name-coverage test** — a test that every `BindingAction` in `DefaultBinding`
is reachable from a dispatch arm. That is the item that prevents recurrence (the audit
plan's own words: "the only item here that prevents recurrence rather than fixing an
instance"), and it is still unlanded: `grep -rn 'BindingAction' keybindings/ tests/` finds
no coverage test.

**Why this reversal is safe to make here rather than escalate:** it does not change the
operator's contract. The refusal is the documented behaviour, it shipped, and the
warning already says so. Implementing binding would *change* the contract; writing the
coverage test *enforces* the one that exists. The second is strictly the smaller step.

### D3 — Is RT-543 "two daemons that share nothing", or "one socket, shared"?

| source | claim |
|---|---|
| ledger row RT-543 | open, "there are TWO DAEMONS AND NEITHER SHARES ANYTHING" |
| iter-2761 plan | "RT-543's sharing half has no proof on the tree" |
| measured here | the proof **exists**: `crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs` |

**RULING: the sharing half is CLOSED and the row's premise is stale. Only the disposition
half remains, and that is ruling R1.**

iter-2746 (`456c98e0`) landed the test RT-543's verdict promised: one socket serves two
clients through one `McpManager` (Arc-shared, Clone-over-Arc). So the sentence "neither
shares anything" is no longer true of the runtime — it describes the pre-iter-2746 tree.
RT-544's closure (iter-2662) rests on that sharing claim and is therefore sound.

### D4 — Is the keybinding dispatcher contended (do not touch) or free (safe to refactor)?

| source | claim |
|---|---|
| TODO.md `## Pending` (2026-10-29) | "`key_handling.rs` is the repo's most contended file (the peer's live WIP has sat in it for days)" — **deliberately NOT scheduled** |
| iter-2778 plan T2 | "It is no longer contended. The refactor is bounded... the work is wiring, not writing" |
| measured here | **4 commits in the last 3 days**, none of them a peer's parked WIP |

**RULING: the 2026-10-29 refusal is OBSOLETE but its conclusion survives for a different
reason.**

The contention argument is gone (4 commits/3 days is ordinary traffic, not a parked branch),
so "someone else is editing it" is no longer a reason to skip. But D2 settles the question
independently: the change itself is out of scope because the contract is refusal. **The
conclusion holds; the reasoning does not.** Recorded so the next agent does not re-derive
the contention claim as the reason.

## The one ordered queue

Everything still open, in execution order. Each row names its owner class, so an agent
knows before starting whether it can finish the row alone.

### Tier 1 — agent-executable, no ruling needed (start here)

| # | tranche | evidence it is open | verify | plan of record |
|---|---|---|---|---|
| 1 | **The keybinding name-coverage test** (D2/D4 residue) | `grep -rn 'BindingAction' keybindings/ tests/` finds no coverage test | the test fails if a catalogued action loses its dispatch arm | `close-the-tui-audit-round-*` T4 |
| 2 | **Gate bug: `genome doctor`'s append-only check flags its own generated zone.** `scripts/genome:2974-2985` applies the `ledger_tocs` exclusion only when the role is a DIRECTORY, but `genome.toml:12` makes `worklog` a FILE while `genome.toml:46` lists it in `ledger_tocs`; the generated zone line changes in the same append that adds a row | `./scripts/genome doctor` reports "append-only violation: 1 line(s) removed from docs/kb/WORKLOG.md" | doctor reports no finding and a fresh append does not reproduce it |
| 3 | **Docs drift: three `process-architecture.md` rows sit `open` for CLOSED work.** `-033` RT-542 CLOSED iter-2654; `-034` RT-543 sharing half CLOSED+PINNED iter-2746 (only the ACP disposition remains, owner-gated R1); `-035` RT-545 CLOSED iter-2658 / RT-546 CLOSED iter-2197 | verified here against the ledger | no `open` row in that file names closed work |
| 4 | **Hygiene: a 24-day-old live claim blocks `genome doctor`.** `remaining-gaps-1364-001` `claimed-by agent-omarchy since 2026-10-27T06:27`; its own verdict records `/effort`, `/plan`, `/context` shipped (iter-1379/1386), leaving `/memory`, `/skill-invoke`, `/export`, `/search` | `genome doctor` reports the live claim | the claim is released or narrowed to the four real remainders |
| 5 | **Lint-suppression ledger**: fix the scanner's invalid `\[` regex (3 "unexplained" are false positives), then convert the 2 `agent/run.rs` `allow`-where-`expect` sites | `audit-lint-suppressions.py`: 12 correct expects, **13 conversion candidates** | 0 unexplained, conversion count drops by ≥2, `.ci/` allowlist untouched | iter-2778 plan T3 |
| 6 | **The 195 unreached config keys** — enumerate into one table with a per-surface gap matrix; mark `doctor`-gap keys (can silently misconfigure) vs `template`-gap (docs only) | `audit-config-keys.py --check`: 77 keys, −195 reach all four surfaces, 6 inherited gaps | the six keys appear with their matrix | iter-2778 plan T5 |
| 7 | **Pub-fn queue T1a** — acid-test the top ~15 by test-only caller count (`roster_path_in` 15, `add_session_tag` 12, `plan_span` 10, `canonical_name` 9, ...) | `--check`: 103 candidates, **100 NEW** | count strictly decreases; each deletion RED-proven in `dead_pub_fns_stay_dead.rs` (41 needles today) | iter-2778 plan T1 |
| 8-11 | **Pub-fn queue T1b-T1e** — the remaining ~85 candidates, ~25 per batch | same | same | same |

**Method for every pub-fn row (non-negotiable):** a candidate is NOT a verdict. Comment out
the `pub`, run `cargo check -p operant-core --lib`, restore. Clean compile with only
`dead_code` ⇒ dead, delete. Anything else ⇒ it stays with a recorded reason, or goes to
`ACCOUNTED_FOR`. Exclusions the auditor's own header names: `#[cfg(test)]`-only helpers
whose `pub` is load-bearing for a sibling test module, `pub use` re-export shims,
doc-example callers, and the 142 multiply-defined names the script skips.

### Tier 2 — docs-only, no ruling needed

| # | tranche | evidence | verify |
|---|---|---|---|
| 9 | **Close the two stale TODO.md residuals** (iter-2778 T4). (a) the session-hub `Database::init` per open **does not exist** — `grep -rn 'Database::init' session_hub/` hits only `tests.rs`; `open()` resets state and calls `rebuild_visible()` with data over the cached-then-refresh mpsc. The "plan 2-5" selector items (`sibling-branch`, `prompt-text-filter`, `tail-cap`) are **absent from the tree** and the 2026-11-14 hub plan records all seven operator items closed (iter-2317..2351). (b) the settings screen **does** persist config keys to `operant.toml` via `persist_config_key_at` (iter-2645) — that half is closed; the `settings.json` half is a **deliberate boundary** (the TUI's own visual state must not leak into a shared `operant.toml`, per `settings_screen/keys.rs:127` and `state.rs:355`) and belongs in `## Review` | a fresh read of TODO.md names no residual `grep` contradicts |

### Tier 3 — owner-gated, blocked (do not start before the ruling)

These are the five rows on `docs/kb/workflows/residual-gaps-2026-11-21.md` marked
`blocked`, plus the ledger's three open rows. They are collected here so the queue is
complete, and they stay in `TODO.md ## Review` because autonomous mode never touches that
section by design.

| # | ruling | reviewer recommendation (context-pack round 4) | campaign prior | evidence |
|---|---|---|---|---|
| R1 | RT-543 `operant acp` disposition | (a) deprecate `--socket`, keep stdio | (a) | `research/audits/context-pack-2026-11-20-blocker-rulings/VERIFICATION.md` Part 1 |
| R2 | RT-280 resume/stop flags | **(c)** delete both flags + 3 writers | (a) — **REVERSAL** | same, Part 1 R2 |
| R3 | RT-190 cross-surface memory | (c) accept two families, close the row | (c) | same, Part 1 R3 |
| T3a | `plugins-wasm`: is the surface worth its 15 unbumpable advisories? | not a reviewer question — a feature decision | — | extism 1.41.0 pins wasmtime 49.0.3, its own latest; advisories themselves triaged+allowlisted iter-2763 |
| T3b | `/reset` durability (ASKS Q6a) | not a reviewer question — a capability decision | — | `reset_slot_for_session` clears seat/model/effort/posture/charter and never touches the store |

**R2 is the one to read as a disagreement, not a confirmation.** The reviewer chose (c)
because (a) needs a `max_age_seconds` the repository cannot supply. Whoever rules should
know the reviewer and this campaign prefer different options.

**Note for R1:** D3 above changes its framing. RT-543's "two daemons" premise is stale —
the sharing half is proven. What is left to rule on is purely the `operant acp` surface
disposition.

### Tier 4 — checked and NOT gaps (do not re-open)

| candidate | verdict |
|---|---|
| The 15 wasmtime advisories | **triaged and allowlisted** iter-2763, reason per entry, `seen=20 allowlist=20 new=0 stale=0` |
| `libsqlite3-sys` major bump | **already triaged** (RT-616): 99 `rusqlite::` sites, 3 crates; scheduled on its own merit |
| `config_registry.rs` (4252 lines) duplicating `config.rs` | **not redundant** — generated by `gen-config-registry.py`, `--check` gated, 409 keys current |
| `operant-api` (60 pub items) / `operant-harness` (15 modules) | **not parallel surfaces** — live dependencies (api: plugins/config; harness: core/cli) |
| `TODO:`/`FIXME:` in `crates/` | **none** — the 2 matches are test fixtures (`autonomous_ticks.rs:77`, `skills_tool.rs:3255`) |
| 72 TUI modules / 349 .rs files; 7 workspace crates | **both accurate** — `find` and `Cargo.toml members` |
| RT-543's sharing half | **closed iter-2746** (D3) — the test exists |

## The plans this supersedes as front doors

Not deleted (genome law 3) — demoted to "detail of record":

| plan | status |
|---|---|
| `2026-11-21-close-the-residual-implementation-gaps-the-pub-fn-queue-the-.md` (iter-2778) | **executable detail for Tiers 1-2.** Its D1 "refilled" claim is corrected here; its T2 (resolve-then-dispatch) is **withdrawn** per D2 — use its T1/T3/T4/T5 |
| `2026-11-20-close-the-pub-fn-surface-queue-one-acid-proven-group-at-a-ti.md` | **landed.** T5 (`tests=0`) is empty; T1..T4 shipped (iter-2754/2757/2756/2767). Its T6 exclusion is now Tier 1 row 4 |
| `2026-11-20-close-the-three-residual-gaps-the-tests-lint-hole-the-rt-543-shari.md` | **landed.** T1 = iter-2744, T2 = iter-2746, T3 = the pub-fn batch |
| `2026-11-20-close-the-tui-audit-round-s-verified-findings-contrast-seam-.md` | **mostly landed** (iter-2714, T1-T5). **Open residue: T4's name-coverage test** → Tier 1 row 1 |
| `2026-11-20-close-the-four-residual-gate-reds-and-the-deferred-tool-surf.md` | **landed** (iters 2729-2735 per its successor's header) |
| `2026-11-20-close-the-e311-library-coverage-error-iter-2742-introduced-a.md` | **landed** — `genome lint` reports 0 E311 |
| `2026-11-20-the-plugin-trust-default-and-the-15-advisories-it-decides-the-se.md` | **T1/T2 landed** (iter-2769 signature default, iter-2763 advisories). **Open: the feature decision** → Tier 3 row T3a |
| `2026-11-20-close-the-key-hint-drift-*` | **executed** iter-2716 (T3/T4 declined with reasons in RT-617) |
| the other 2026-11-20 test/gate plans | **landed** — the battery is green except the pub-fn queue |
| `docs/kb/workflows/process-architecture.md` (`-033`/`-034`/`-035`) | **rows still `open` for CLOSED work** → Tier 1 row 3 (T7). Not a plan, a board; recorded here so the drift is visible from the front door |
| the `genome doctor` append-only check (`scripts/genome`) | **gate bug, open** → Tier 1 row 2 (T6). Not a plan; a live doctor finding |
| the `remaining-gaps-1364-001` live claim | **stale claim, open** → Tier 1 row 4 (T8). Not a plan; a hygiene item `genome doctor` reports |

## Gates / success criteria

(fixed before the run)

| tier | criterion |
|---|---|
| 1 | keybinding coverage test lands and is RED-proven (fails when a catalogued action loses its arm); `audit-lint-suppressions.py` reports 0 unexplained and the conversion count drops ≥2; the six config keys appear in TODO.md with their matrix; `--check` candidate count strictly decreases per pub-fn batch |
| 2 | TODO.md names no residual that a fresh `grep` contradicts |
| 3 | no action before the ruling; each ruling recorded in `docs/kb/ASKS.md` with its applied owner doc |
| 4 | no action — the table is the deliverable |
| all | `genome lint` 0 errors (W014 may remain, owner-gated); `check-doc-claims.sh` 0; `supply-chain-gate.sh` `new=0`; `cargo check --workspace --all-targets` 0; `dead_pub_fns_stay_dead.rs` gains a needle per deletion |

## Cost

| tier | cost |
|---|---|
| 1 | ~8-10 iterations (the pub-fn queue dominates at 4-6; rows 1-4 are 1 each; rows 5-6 are 1 each) |
| 2 | 1 iteration, docs-only |
| 3 | 0 until ruled; R1 and R2 each need a code iteration plus a release deploy (neither is docs-only); R3 is docs/ledger-only |
| 4 | 0 |

## What this plan deliberately does not do

- It does not rule on anything. Tiers 1-2 are agent-executable; Tier 3 is the owner's.
- It does not delete the superseded plans. They are dated-frozen detail of record.
- It does not re-run a context-pack round. The rulings evidence is published and verified.
- It does not weaken a gate. Every verify row above is a measurement, not an assertion.
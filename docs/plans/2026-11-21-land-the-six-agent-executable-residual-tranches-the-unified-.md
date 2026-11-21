# Land the six agent-executable residual tranches the unified roadmap leaves open

Dated-frozen: published `2026-11-21`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written at `origin/main` = `03f7ff3a` (iter-2780). Every row below was re-derived at that
tip by running the command the roadmap cites — **not** copied from the roadmap's prose.

**Relationship to the roadmap (one owner per question, law 1).** This plan does not own the
"what is left" question: `docs/plans/2026-11-21-the-unified-residual-roadmap-reconcile-the-disagreements-lan.md`
(iter-2780) does, and it is `core.living_plans`. This plan is the **execution queue** for
its Tier 1-2 rows. Nothing here re-opens a Tier 3 ruling.

## Why this plan exists

The roadmap says "nothing in this plan is executed yet" and names eleven Tier 1 rows. Re-deriving
each row at `03f7ff3a` found that **one is already closed** and **one is narrower than stated**.
Left as prose, the next agent re-derives them a third time. This plan records the derivation
once, per row, so the queue can be worked without re-auditing it.

## The derivations (re-run these; do not trust this table)

| roadmap row | re-derived state at `03f7ff3a` | evidence |
|---|---|---|
| 1. keybinding name-coverage test | **PARTIALLY OPEN** — not the gap as written | 61 `KeyAction` variants (`keybindings.rs:29-130`); `KeyBinding.action` is consumed **only** as `format!("{:?}", …)` for `/keys`, `/hotkeys`, `report.rs:176`, `cmd_tui_debug.rs:1081/2264` — it never dispatches. Dispatch is a 348-arm hand-written `KeyCode` if-chain in `app/key_handling.rs`. Per-surface guards already exist (`tests.rs:450` esc-close, `:564` scroll ladder, `:1276` composer). So a name-coverage test has no dispatch arm to check against; see T1. |
| 2. `genome doctor` append-only self-flag | **CLOSED** | `./scripts/genome doctor` reports `1 finding(s)`, and it is the stale `STATE.md`, not an append-only violation |
| 3. `process-architecture.md` open rows | **OPEN** | `docs/kb/workflows/process-architecture.md:39-41` rows `-033/-034/-035` read `open`, while the ledger records RT-542 **CLOSED iter-2654**, RT-543 sharing **PINNED iter-2746**, RT-545 **CLOSED iter-2658**, RT-546 **CLOSED iter-2197** |
| 4. 24-day live claim + stale STATE | **OPEN** | `genome doctor`: `live claim remaining-gaps-1364: … claimed-by agent-omarchy since 2026-10-27T06:27`; `stale state: docs/handoff/STATE.md names 413a7a88, HEAD is 73088cab (313 non-documentation path(s) changed)` |
| 5. lint-suppression scanner | **OPEN** | `audit-lint-suppressions.py`: `correct #[expect] sites: 12`, `conversion candidates: 13` |
| 6. unreached config keys | **OPEN** | `audit-config-keys.py --check`: `77 config keys; -195 of them reach all four surfaces`, `6 inherited gap(s)` |
| 7-11. pub-fn queue | **OPEN, and the count moved** | `audit-pub-fn-surface.py --check`: `candidates (0 production callers): 132`, `accounted for: 2`, `NEW: 130`. Of the 40 the default `--limit` prints, **9 have `tests=0`**. The campaign's `tests=0` close-out landed at iter-2767; this is the wider `0 production callers` queue, not a refilled one. |
| 9. two stale `TODO.md` residuals | **OPEN** | roadmap row 9; not re-derived here |

**D2's correction stands and is load-bearing for T1:** `[keybindings]` overrides are
*refused* by contract (`config.rs:60-63`, shipped warning at `config.rs:4236`). Do not
implement binding.

## Mechanism

Six tranches, in roadmap order. Each is one iteration, committed and pushed on its own.

### T1 — the keybinding guard, re-scoped to what is actually checkable

Row 1 as written asks for "every `KeyAction` reachable from a dispatch arm". That is not
a testable statement on this tree: `KeyBinding.action` does not drive dispatch (derivation
above), so there is no arm to reach. The recurrence guard that *is* available is the one
the module's own header promises — the catalogue must not advertise a chord nothing
handles. Write `every_catalogued_chord_is_handled_by_the_dispatcher`: for each
`DefaultBinding` in `DEFAULT_KEYBINDINGS`, drive `App::handle_key_event` with its chord in
its context and assert the binding was consumed rather than falling through. Exempt the
three declared-but-unbound actions explicitly (`keybindings.rs:18-22` says 60 of 63 carry
a default; the other 3 have no chord and must be named, not silently skipped).

**Falsifier:** the test fails if a catalogued chord is deleted from `defaults.rs` while the
if-chain still handles the key, or if a chord is added to the catalogue with no handler.

### T2 — `process-architecture.md`: three rows close, one row narrows

Update `docs/kb/workflows/process-architecture.md:39-41`:

- `-033` RT-542 → `done`, evidence `iter-2654`, citing `the_chat_attach_entry_point_builds_no_runtime`.
- `-034` RT-543 → **narrow, do not close.** The sharing half is `done` (iter-2746,
  `crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs`); the `operant acp`
  disposition half is owner-gated **R1** and must stay named as blocked in the row.
- `-035` RT-545/RT-546 → `done`, evidence `iter-2658` / `iter-2197`.

**Falsifier:** after the edit, no row in that file reads `open` for work the ledger marks
CLOSED. Re-run `python3 scripts/audit-ledger-index.py` — it must still report OK.

### T3 — release the 24-day claim; regenerate STATE.md

(a) `genome` claim `remaining-gaps-1364-001` is held by `agent-omarchy` since `2026-10-27`.
Its own verdict records `/effort`, `/plan`, `/context` shipped (iter-1379/1386), leaving
`/memory`, `/skill-invoke`, `/export`, `/search`. Narrow the claim to those four or release
it — do not leave a 24-day blanket claim blocking `doctor`.
(b) `./scripts/genome handoff --write` to regenerate `docs/handoff/STATE.md`, which still
names `413a7a88`.

**Falsifier:** `genome doctor` reports no `live claim` and no `stale state`.

### T4 — the lint-suppression ledger

Two parts, in this order:
1. Fix the scanner's invalid `\[` regex, which produces 3 false-positive "unexplained" rows.
2. Convert the 2 `agent/run.rs` `allow`-where-`expect` sites to `#[expect(…, reason = …)]`.

**Falsifier:** `audit-lint-suppressions.py` reports `0 unexplained` and the conversion count
drops by ≥2. `.ci/clippy-allowlist.txt` must be **untouched** — this is not a gate relaxation.

### T5 — the config-key gap matrix

Enumerate the unreached keys into one table with a per-surface matrix (note / heal / template /
doctor), splitting **doctor-gap** keys (can silently misconfigure) from **template-gap** keys
(docs only). Fix the 6 inherited gaps or record each with its reason.

**Falsifier:** the six keys appear in the table with their matrix, and
`audit-config-keys.py --check` names no gap without a recorded reason.

### T6 — the pub-fn queue, first acid-proven group

Take the **9 `tests=0` candidates** first — they are the only ones the acid test can decide
quickly: `all_records_mut`, `approver_of`, `child_sessions`, `determine_trust`, `enable_tool`,
`format_report`, `format_terminal`, `from_app_config`, `from_str_key`.

**Method is non-negotiable (roadmap's words, and iter-2754/2756/2757 proved it):** a candidate
is not a verdict. Comment out the `pub`, run `cargo check -p operant-core --lib`, restore.
Clean compile with only `dead_code` ⇒ dead, delete. Anything else ⇒ it stays with a recorded
reason, or goes into `ACCOUNTED_FOR`. Exclusions: `#[cfg(test)]`-only helpers whose `pub` is
load-bearing for a sibling test module, `pub use` shims, doc-example callers, the 142
multiply-defined names the auditor skips.

Every deletion adds a needle to `crates/operant-core/tests/dead_pub_fns_stay_dead.rs` (41 today),
and the test must be shown RED before the deletion and GREEN after.

**Falsifier:** `--check` count strictly decreases; the acid test fails if any deleted name
returns.

## What this plan does NOT do

- **No Tier 3 ruling.** R1 (RT-543 `operant acp` disposition), R2 (RT-280 resume/stop flags),
  R3 (RT-190 cross-surface memory), T3a (`plugins-wasm` advisories), T3b (`/reset` durability)
  are owner decisions. The roadmap blocks them; so does this plan.
- **No `[keybindings]` binding.** Refusal is the contract (D2).
- **No Tier 4 re-open.** The roadmap's "checked and NOT gaps" table stays closed.
- **No S2 app-side F12 buffer dump** — the roadmap does not schedule it and it needs a product
  call on whether an app-side dump is wanted at all.
- **No design-preference change.** Phonon, memory-wire, sourcehound, the 7 gateway platforms,
  tiered eviction — untouched.

## Gates / success criteria

(fixed before the run)

- `./scripts/genome lint` exits 0 before and after every tranche.
- `./scripts/genome doctor` reports no `live claim` and no `stale state` (T3).
- `python3 scripts/audit-ledger-index.py` reports OK (T2).
- `python3 scripts/audit-lint-suppressions.py`: 0 unexplained, conversion count −2 (T4).
- `python3 scripts/audit-config-keys.py --check`: 6 gaps each with a recorded reason (T5).
- `python3 scripts/audit-pub-fn-surface.py --check`: count strictly decreases, and
  `cargo test -p operant-core --test dead_pub_fns_stay_dead` is green (T6).
- Every tranche is one iteration: label re-derived from `origin/main` after `git fetch`,
  `git rev-list --left-right --count origin/main...HEAD` checked before the push, pushed,
  and confirmed with `git log origin/main --oneline | grep -c "iter-N" == 1`.

## Cost

Six iterations, all tooling/docs/one mechanical test. T6 is the expensive one: 9 acid tests,
each a `cargo check -p operant-core --lib` (~40 s each on this box) plus a needle per
deletion. No release build is needed unless T6 touches `operant-core` source (it deletes
`pub` fns, so it does — one scoped build, no deploy, because no behaviour changes).
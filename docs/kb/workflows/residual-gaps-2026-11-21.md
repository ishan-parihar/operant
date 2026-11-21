# Workflow: residual-gaps-2026-11-21

Executes the **unified residual roadmap**
`docs/plans/2026-11-21-the-unified-residual-roadmap-reconcile-the-disagreements-lan.md`,
which is the single front door and the living plan (it has a `Resume here` section). The
roadmap reconciles four disagreements between the 2026-11-20/21 plans; two change what an
agent should do:

- **D1:** the pub-fn queue did **not** refill. It shrank monotonically 130 -> 103. Do not
  hunt a phantom leak.
- **D2:** `[keybindings]` overrides are **refused by contract**, not bound. The old T2
  "resolve-then-dispatch" row is **WITHDRAWN**; the real residue is K1 below.

Detail of record for the tranches: `docs/plans/2026-11-21-close-the-residual-implementation-gaps-the-pub-fn-queue-the-.md` (its T2 withdrawn; its T1/T3/T4/T5 are still the detail).

Claim before you work (`./scripts/genome claim <id>`) and post the delta
(`./scripts/genome post "<msg>" --wf residual-gaps-2026-11-21`).

## Tier 1 — agent-executable, no ruling needed (start here, in order)

| id | status | item | claim | verdict |
|---|---|---|---|---|
| K1 | open | **The keybinding name-coverage test.** A test that every `BindingAction` named in `DefaultBinding` (150 entries, 10 contexts) is reachable from a dispatch arm. `grep -rn 'BindingAction' keybindings/ tests/` finds no coverage test today. The audit plan called this "the only item here that prevents recurrence rather than fixing an instance". RED-prove it first: the test must fail if a catalogued action loses its arm. | — | — |
| T6 | open | **Gate bug (do first): `genome doctor`'s append-only check flags its OWN generated zone.** `scripts/genome:2974-2985` applies the `ledger_toc` exclusion only when the role is a DIRECTORY, but `genome.toml:12` makes `worklog` a FILE (`docs/kb/WORKLOG.md`) while `genome.toml:46` lists `WORKLOG.md`+`feed.md` in `ledger_tocs`. The generated zone line changes in the same append that adds a row, so the check reads its own regeneration as a violation. Evidence: `./scripts/genome doctor` reports "append-only violation: 1 line(s) removed from docs/kb/WORKLOG.md in the last 10 commits". | — | — |
| T7 | open | **Docs drift: three `process-architecture.md` rows sit `open` for CLOSED work.** `-033` RT-542 CLOSED iter-2654; `-034` RT-543 sharing half CLOSED+PINNED iter-2746 (only the ACP disposition remains, owner-gated R1); `-035` RT-545 CLOSED iter-2658 and RT-546 CLOSED iter-2197. Verified here. | — | — |
| T8 | open | **Hygiene: a 24-day-old live claim blocks `genome doctor`.** `remaining-gaps-1364-001` has been `claimed-by agent-omarchy since 2026-10-27T06:27`; its own verdict records `/effort` (iter-1379), `/plan` (iter-1379) and `/context` (iter-1386) shipped, leaving `/memory`, `/skill-invoke`, `/export`, `/search`. Release or narrow the claim. | — | — |
| T1a | open | **Pub-fn batch 1:** acid-test and delete the top ~15 by test-only caller count (`roster_path_in` 15, `add_session_tag` 12, `plan_span` 10, `canonical_name` 9, `validate_span` 8, `parse_compartment` 7, `new_turn` 7, `get_session_tags` 7, `clear_active_memory_provider_for_tests` 7, `turn_interceptor_count` 6, ...). One iteration per deletion; add a RED-proven needle to `dead_pub_fns_stay_dead.rs` (41 needles today). | — | — |
| T1b | open | **Pub-fn batch 2:** next ~25 candidates by test count. | — | — |
| T1c | open | **Pub-fn batch 3:** next ~25 candidates. | — | — |
| T1d | open | **Pub-fn batch 4:** next ~25 candidates. | — | — |
| T1e | open | **Pub-fn batch 5 (tail):** remaining candidates until `audit-pub-fn-surface.py` reports 0 NEW. | — | — |
| T3 | open | **Lint-suppression ledger:** fix the scanner's invalid `\[` escape so it reads the inline `reason` (the 3 "unexplained" are false positives — each carries a reason the regex cannot read), then convert `agent/run.rs:1391` and `:2582` from allow to expect. `.ci/` allowlist untouched. | — | — |
| T5 | open | **The 195 unreached config keys:** enumerate into one table with the missing-surface matrix (`note`/`heal`/`template`/`doctor`), marking which have a `doctor` gap (can silently misconfigure) vs a `template` gap (docs only). Survey only, no fix. | — | — |

## Tier 2 — docs-only, no ruling needed

| id | status | item | claim | verdict |
|---|---|---|---|---|
| T4 | open | **Close the two stale TODO.md residuals.** (a) the session-hub `Database::init` per open does not exist (`grep -rn 'Database::init' session_hub/` hits only `tests.rs`); the "plan 2-5" selector items (`sibling-branch`, `prompt-text-filter`, `tail-cap`) are absent from the tree and the 2026-11-14 hub plan records all seven operator items closed (iter-2317..2351). (b) the settings screen DOES persist config keys to `operant.toml` via `persist_config_key_at` (iter-2645); the `settings.json` half is a DELIBERATE boundary (`settings_screen/keys.rs:127`, `state.rs:355`) and belongs in `## Review`. Rewrite both to say what is measured. | — | — |

## Tier 3 — owner-gated, BLOCKED (do not start before the ruling)

Evidence, per-ruling confidence, and the one measurement that would change each answer:
`research/audits/context-pack-2026-11-20-blocker-rulings/VERIFICATION.md` Part 1.

| id | status | item | claim | verdict |
|---|---|---|---|---|
| R1 | blocked | **RT-543 `operant acp` disposition.** Reviewer recommends (a) deprecate `--socket`, keep stdio; campaign prior (a). Note D3 in the roadmap: the row's "two daemons share nothing" premise is STALE (sharing proven iter-2746) — only the ACP surface disposition remains. Edits `cmd_acp.rs`, so it needs crate-scoped tests + a release deploy. | — | — |
| R2 | blocked | **RT-280 resume/stop flags.** Reviewer recommends **(c) delete both flags + 3 writers**; this **REVERSES the campaign prior (a)** because (a) needs a `max_age_seconds` the repo cannot supply. Needs a schema-compatibility decision so existing SQLite rows still deserialize. | — | — |
| R3 | blocked | **RT-190 cross-surface memory.** Reviewer recommends (c) accept two families, close the row; campaign prior (c). Docs/ledger-only when ruled. | — | — |
| T3a | blocked | **plugins-wasm: is the surface worth its 15 unbumpable advisories?** The advisories themselves are triaged+allowlisted (iter-2763, reason per entry); what is unruled is the FEATURE — extism 1.41.0 pins wasmtime 49.0.3 and is its own latest release, so they are permanently unbumpable while it is on. iter-2769 fixed the signature default. Keep on or default off. | — | — |
| T3b | blocked | **/reset durability (ASKS Q6a).** `reset_slot_for_session` clears seat/model/effort/posture/charter and never touches the store, so a "wiped" history returns on restart. Not a false claim (RT-389 corrected the doc) — a missing capability. Decide `discard` vs `keep`. | — | — |

## Checked and NOT gaps (do not re-open)

| candidate | verdict |
|---|---|
| The 15 wasmtime advisories | **triaged and allowlisted** iter-2763, reason per entry, `seen=20 allowlist=20 new=0 stale=0` |
| `libsqlite3-sys` major bump | **already triaged** (RT-616): 99 `rusqlite::` sites, 3 crates; scheduled on its own merit |
| `config_registry.rs` duplicating `config.rs` | **not redundant** — generated by `gen-config-registry.py`, `--check` gated, 409 keys current |
| `operant-api` / `operant-harness` | **not parallel surfaces** — live dependencies (api: 60 pub items, depended on by plugins/config; harness: 15 modules, depended on by core/cli) |
| `TODO:`/`FIXME:` in `crates/` | **none** — the 2 matches are test fixture strings (`autonomous_ticks.rs:77`, `skills_tool.rs:3255`) |
| 72 TUI modules / 349 files; 7 workspace crates | **both accurate** — `find` and `Cargo.toml members` |
| RT-543's sharing half | **closed iter-2746** — `rt543_one_socket_shares_one_mcp_child_set.rs` |

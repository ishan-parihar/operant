# Workflow: residual-gaps-2026-11-21

Executes `docs/plans/2026-11-21-close-the-residual-implementation-gaps-the-pub-fn-queue-the-.md`.
One row per iteration; one row per agent claim. Claim before you work
(`./scripts/genome claim <id>`) and post the delta to this workflow
(`./scripts/genome post "<msg>" --wf residual-gaps-2026-11-21`).

| id | status | item | claim | verdict |
|---|---|---|---|---|
| T4 | open | Close the two stale TODO.md residuals (docs-only) and move the settings.json design question to ## Review. Evidence: iter-2777 measurement -- hub has no per-open `Database::init`; settings screen persists config keys to `operant.toml` via `persist_config_key_at`. | — | — |
| T1a | open | Pub-fn queue batch 1: acid-test and delete the top ~15 by test-only caller count (roster_path_in 15, add_session_tag 12, plan_span 10, canonical_name 9, validate_span 8, parse_compartment 7, new_turn 7, get_session_tags 7, clear_active_memory_provider_for_tests 7, turn_interceptor_count 6, ...). One iteration per deletion; add a RED-proven needle to dead_pub_fns_stay_dead.rs each. | — | — |
| T1b | open | Pub-fn queue batch 2: next ~25 candidates by test count. | — | — |
| T1c | open | Pub-fn queue batch 3: next ~25 candidates. | — | — |
| T1d | open | Pub-fn queue batch 4: next ~25 candidates. | — | — |
| T1e | open | Pub-fn queue batch 5 (tail): remaining candidates until `audit-pub-fn-surface.py` reports 0 NEW. | — | — |
| T2 | open | Keybinding dispatcher: resolve-then-dispatch. `key_handling.rs` asks `keybindings::registry` for the action bound to (key, modifiers, context) and runs it; the if-chain becomes the registry's Default impl. Acceptance: a test that sets an override, runs the dispatch, and asserts the overridden action fired. | — | — |
| T3 | open | Lint-suppression ledger: fix the scanner's invalid `\[` escape so it reads the inline reason (3 unexplained are false positives), then convert `agent/run.rs:1391` and `:2582` from allow to expect. | — | — |
| T5 | open | Enumerate the 6 inherited config-key gaps (browser, checkpoints, database_path, moa, skills, terminal_backend) into TODO.md with the missing-surface matrix (note/heal/template/doctor). Survey only, no fix. | — | — |

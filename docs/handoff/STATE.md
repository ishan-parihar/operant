# State

Regenerate with `./scripts/kb handoff --write`; the old state is archived with a
tombstone, never deleted. Snapshot date: **2026-10-17** (iter-901 adoption wave).

## Where the project stands

- Release line `0.2.1`; local HEAD tracks `origin/main` (iter-900 was the
  documentation-genome adoption commit; iter-899 the last code change).
- Working tree per `git status`: peer/user WIP present (deleted TUI scenario
  baselines + `crates/operant-cli/tests/tui_scenarios/rewind-flow.*`) and
  untracked in-flight audit work in `docs/audit/2026-10-16-operant-audit/` —
  neither is this snapshot's business; stage explicit paths only.
- The clippy warning gate is red at the pristine tip from in-flight peer W-zone
  work (fleet rule 7: prove your own delta is zero, do not fix the peer's).
- Docs-only iterations skip the R&D build/deploy step (AGENTS.md).

## Open register rows (TODO.md)

`Implemented` is current through iter-856. `Pending` (6 items): `/settings`
rebuild over `operant.toml`; `[keybindings]` overrides; collapsed `▸ thought`
row; gateway/TUI `show_reasoning` unification; BUGS.md K-4 observer gate;
BUGS.md K-3 terminal-scroll watermark. `Review` holds owner decisions
(9 unwired commands, display-only reasoning effort, Esc clearing all tool
rows, context-management redesign, BUGS.md R42-8 leaked tokens).

## Documentation genome

Adopted 2026-10-17 (iter-901): roles mapped in `kb.toml`; `docs/plans/` and
`docs/audit/` are the single plan/audit owners; `plans/` series and
`docs/BUGS.md` archived with tombstones (`docs/archive/INDEX.md`).
Wake routine: `./scripts/kb doctor` + `./scripts/kb feed --to <your-agent-id>`.

Live claims: see `kb doctor`. Recent units of work: `docs/kb/WORKLOG.md`.

<!-- genome:zone status sha=3c05967a2670 -->
| status | items |
|---|---|
<!-- genome:end status -->

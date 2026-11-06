# Entry — what a fresh agent does, in order

1. `genome wake` — state, resume point, operator questions, jobs, health in one call.
2. `genome ask "<question>"` — the one section that answers it; read that range only.
3. Before work that touches a decision: `genome rec recon <keywords>` (prior art).
4. End of iteration: `genome close`.

**Read `kb/EXECUTOR.md` before you touch anything.** It has the ground rules that
are not in `AGENTS.md`: the pinned-commit discipline, the "one row is only closed when
the canonical ledger says so" rule, and the priority order for the open queue.

**Stop and ask the operator** (record it with `genome q add`):
- Whether to gitignore or delete the untracked junk dirs (Q1 in `docs/kb/ASKS.md`).
- Whether to commit the ready MCP fix over the peers' in-flight WIP in the shared tree (Q2).
- Anything that changes the TUI's role as the *entire* operator interface — the sweep
  treats the cron pane (G-010) and the channel-parity sweep (G-009) as scope decisions,
  not mechanical changes.

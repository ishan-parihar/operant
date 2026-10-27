# Verification — the harness that proves work

Owns: operant's verification harness — recipe detection, phased
execution, the evidence ledger, and the agent-facing `verify_task` tool.
Not for: the guardrail/approval stack ([security.md](security.md)), the
autonomous loop that consumes verification results
([autonomous.md](autonomous.md)).

Source: `crates/operant-core/src/verification.rs`,
`crates/operant-core/src/tools/verification_tool.rs`,
`crates/operant-core/src/estop.rs` (the pause that halts scheduled runs).

## The harness

`verification.rs` (hermes `agent/verify/` parity: recipes.py + runner.py +
environment.py) detects a project's verification recipe from its manifests
— Rust (Cargo.toml), Node (package.json), Python, Go, Java, Makefile,
docker-compose — then runs it phase by phase:

1. **bootstrap** — install/prepare dependencies,
2. **build** — compile,
3. **test** — run the suite,
4. **start** (optional) — boot the app and poll an HTTP readiness URL.

Every phase's result is recorded in the `verification_events` evidence
ledger in the state database — an auditable trail of what was run and
what it returned, not a boolean.

## The agent tool

`verify_task` (`tools/verification_tool.rs`) exposes the harness to the
model: it detects the recipe, runs the phases, and records every phase in
the evidence ledger. The model can therefore *prove* its work instead of
claiming it.

## ESTOP — the operator's brake

`estop.rs` — `operant pause` writes a sentinel file at
`<data_dir>/ESTOP`; `operant resume` removes it. While the sentinel
exists, the cron scheduler skips dispatching due jobs, the kanban CLI
dispatcher skips claiming tasks, and new gateway turns get a brief
"Operant is paused" reply. Existing work finishes; NEW work does not
start. A resumable pause for new work only.

## Intention

The harness exists because "the agent says it is done" is not evidence.
Recipe detection from manifests (rather than a configured command) keeps
it zero-config across arbitrary projects; the phases mirror what a human
run would do; the events ledger makes every verification citable later —
the autonomous loop and the org layer both read it. The ESTOP is the
operator's guarantee that a self-directed loop can always be stopped at
a boundary rather than mid-write.

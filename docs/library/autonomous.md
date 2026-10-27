# Autonomous mode — the self-directed loop

Owns: autonomous coding mode — the TODO.md ledger loop, the workspace
snapshot gate, the publish path, and failure-pause state.
Not for: the agent loop it drives
([agent-loop.md](agent-loop.md)), ESTOP ([verification.md](verification.md)
§estop), the iteration protocol agents follow (`AGENTS.md`).

Source: `crates/operant-cli/src/autonomous.rs`, launched by
`operant autonomous` or `operant run --autonomous`
(`crates/operant-cli/src/main.rs`).

## The loop

`run_autonomous` builds an `AutonomousRunner` over the current repo and
cycles: snapshot the workspace → pick the next task from the ledger →
run an agent turn on it → publish the change → repeat. Failure at any
step is recorded and the loop adapts rather than dying.

## The ledger: TODO.md

The repo-root `TODO.md` is the task ledger, parsed into `Implemented` /
`Pending` / `Review` sections (`WorkspaceSnapshot::implemented_items` /
`pending_items`). Autonomous behavior changes must keep those sections
accurate — the loop reads them as its queue.

## The workspace snapshot gate

Before doing anything the runner hashes the workspace state
(`workspace_key` = hash of todo contents + HEAD + worktree status). A
task's completion is judged against the snapshot it started from, and
the publish decision (below) compares what actually changed.

## The publish path

`PublishOutcome` is `Failed | Skipped(WorkspaceSnapshot) |
Pushed(WorkspaceSnapshot, StatusPublishSummary)`. The staging scope is
deliberately constrained (AD-027): the tick computes its OWN touched
paths and passes them to `git add -- <paths>` / commit — an owner's file
left dirty is never committed by a tick (regression test
`a_tick_does_not_commit_a_file_the_owner_left_dirty`).

## State across restarts

The runtime writes repo-local `autonomous-status.toml` state and
RELOADS repeated-failure pause state across restarts — a task that
failed N times before the restart is still paused after it. Keep that
workflow documented when changing autonomous behavior.

## Intention

Autonomous mode is the operator's delegation endpoint: the ledger is
the contract (the operator seeds it, the loop consumes it), the
snapshot gate is the honesty check (publish what actually changed, not
what the model claims), and the pause-state persistence exists because
a crash-looping task must not become a crash-looping *machine* after a
restart. The live `Pending` and `Review` rows (derive them from
`TODO.md` at read time — never carry a count here, RG-008) are tracked
in `docs/handoff/STATE.md`.

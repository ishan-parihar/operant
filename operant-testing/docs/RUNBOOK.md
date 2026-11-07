# RUNBOOK — how to work this knowledge base

## Orient

```bash
cd agentic-harness/operant/operant-testing
python3 ~/.agents/skill/project-genome/scripts/genome wake
```

## Re-reconcile before you work anything

```bash
cd agentic-harness/operant
git fetch origin
git log origin/main -1 --pretty=%s          # the numbering authority
# then read kb/red-team/03-reconciliation-vs-upstream.md and confirm the row you
# picked is still open
```

## Verify the findings still hold

```bash
cd agentic-harness/operant/operant-testing
bash kb/red-team/docs/verify.sh
```

`PASS` = a closed finding is still closed. `OPEN` = a row that is still open by design —
work it. `FAIL` = a closed finding regressed; fix that first.

## Take a row

1. Open `kb/gap-register.md` and read the row's **Anchor** — it is `file:line` at the
   pinned commit. Confirm it:
   `git show 758dd4c5:<path> | sed -n '<line>p'`
2. Write the test the row names FIRST and watch it fail.
3. Make it pass. Run `cargo fmt --all`, `cargo check --workspace`, `cargo test --workspace`,
   `bash scripts/clippy-warning-gate.sh`.
4. Update the row's status and its evidence; append to `docs/kb/WORKLOG.md`.
5. Commit with the next free `iter-N` (from `origin/main`, never local history), stage
   EXPLICIT paths — the shared tree carries peers' WIP — then push and confirm:
   `git log origin/main --oneline | grep -c "iter-N"` must print 1.

## Close the iteration

```bash
cd agentic-harness/operant/operant-testing
python3 ~/.agents/skill/project-genome/scripts/genome close
```

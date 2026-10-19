# Asks — what we asked the operator, and what they ruled

Status: LIVE · APPEND-ONLY (rows are amended in place by ruling, never
deleted) · Vocabulary: `open` | `ruled` | `withdrawn`.

**The rule this ledger exists for.** Operator feedback amends its OWNER
doc or record in the same session: a ruling is not a comment here, it is a
change to the file that owns the question, and the `applied-to` column
names that file. A ruling with an empty `applied-to` is a ruling nobody
has acted on yet — the genome's `doctor` reports it.

**How to use it.** `./scripts/genome ask` finds the owning sections for a
question. When the answer needs the operator, add a row here with status
`open`. When they rule, set the status to `ruled`, put the ruling in the
`ruling` column, and name the file you applied it to in `applied-to` — a
backticked path that must exist. `./scripts/genome doctor` then checks
that the ruling was applied.

| id | asked | status | question | ruling | applied-to |
|---|---|---|---|---|---|

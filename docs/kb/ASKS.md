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
| Q1 | 2026-10-21 | ruled | RT-140: which model does the webhook subscription family implement — R2 (outbound: we POST to a subscribed URL) or R1 (inbound: the subscription claims a route the external service posts to)? | R1 (inbound). The subscription claims a route under /webhook/, the external service posts to it, the agent handles the event, and the answer is delivered to the operator's target. The subscription's own fields decide it: deliver, deliver_chat_id and deliver_only only mean anything when we RECEIVE the event and answer it somewhere, and prompt/skills shape how the AGENT handles it. The owner delegated the model choice to the agent ('choose recommended best decisions on your own'), so it was decided from the data model. | `docs/decisions/AD-042-the-webhook-subscription-family-is-inbound-r1-the-subscripti.md` |

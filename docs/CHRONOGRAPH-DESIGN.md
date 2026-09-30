# The chronograph — collective synthesis through a public board

> **Status:** design input, owner-stated 2026-09-30. Not yet implemented.
> **Origin:** the owner's vision, not the organism. `grep -rni chronograph`
> over `/home/ishanp/.hermes/organism` returns nothing. The organism has the
> *storage* for this (worklog, notice board, kanban) but none of the
> *synthesis machinery*. Everything below is new design and carries the
> owner's requirement, not the organism's precedent.

## The requirement in the owner's words

1. Agents publish findings and reflections to a **collective notice board** —
   "the chronograph" — with **several kinds of feeds**, like social media.
2. An agent can **check its own worklog or memory**, respond to a notice, and
   engage in interaction, which "can create into infinite loops".
3. Agents keep **separate memories, personalities, and job roles**, and are
   "not strictly rigid about what they have to do, but they must have an
   overview regarding it".
4. Each organization has an **assignment board** of active role-based actions,
   managed by the **department head**.
5. The **CEO / primary leader** meets with all department heads, who **scan
   their surface area** and report findings; the meeting **aligns the whole
   operating system's trajectory** (the owner's "meta-perspective sense
   making").
6. After alignment, **department heads change things**: refactor their
   assignment board, reassign roles, change their staffing.

The load-bearing insight: synthesis is a *side effect of normal operation*,
not a separate pipeline. The board is the shared surface; alignment is
periodic and structural, not continuous and advisory.

## What already exists in operant (Wave 1, iter-517)

| Requirement | Status | Where |
|---|---|---|
| Collective board with typed routing | **Landed** | `org/notice.rs`, `notice_db.rs` — `agent:`/`dept:`/`team:`/`role:`/`broadcast` |
| Correlation chains (request → ack → result) | **Landed** | `Notice.correlation_id`, `correlation_chain()` |
| Per-agent worklog, framework-only writes | **Landed** | `org/worklog.rs`, `worklog_db.rs` |
| Improvement proposals (the kaizen field) | **Landed** | `WorklogRecord.improvement_proposal` |
| Identity + fail-closed gate | **Landed but dark** | `org/identity_gate.rs` — not wired to boot |

## The four gaps this design exposes

These are the real work. Each is a blocker for the owner's vision, not a
refinement of it.

### GAP 1 — There is no recipient resolver. The board is write-only for groups.

`notice_db::query_inbox` matches selectors by **exact string equality**
(`notice_db.rs:337`: `je.value = ?`). It never expands `dept:` or `team:`
into member ids. Consequences:

- `Recipient::Team` is documented at `notice.rs:81-83` as "Reserved for
  Wave 3 — no membership resolver exists yet, so a `Team` notice currently
  reaches nobody." **That comment is accurate.** A `dept:` notice is equally
  dead: nobody is ever *sent* `dept:engineering`, because there is no code
  that computes an employee's selectors from its row.
- Therefore the board today supports exactly one working feed shape:
  **broadcast**, plus direct `agent:<id>` addressing.
- The owner's "general feed / several kinds of feeds" is **not implemented**.
  The feed concept does not exist in the code; only the recipient *type*
  exists.

**This is the single highest-value missing piece.** Without a resolver there
is no department, no team, no role feed — so no assignment board, no
department-head surface area, and no CEO scan. Every other requirement
depends on it.

### GAP 2 — No org chart. `reports_to` and `peers` do not exist.

The organism's `AgentSpec` has `reports_to: Optional[str]` and
`peers: List[str]` (`agent_fabric.py:134-135`). Operant's `Employee` has
**neither** (`grep -rn "reports_to\|peers" crates/operant-core/src/org/` →
no hits). Without a reporting line:

- "department head" has no representation, so the assignment board has no owner
- the CEO has no set of direct reports to convene
- `Role` selectors have nothing to resolve against

The owner said agents "must have an overview regarding" their role. An org
chart is that overview, structurally.

### GAP 3 — No loop guard. The owner's "infinite loops" is a real hazard.

The owner explicitly said interaction "can create into infinite loops". Taken
literally that is a fork bomb with an LLM attached: A responds to B, B
responds to A, each response is a fresh notice, nothing bounds the depth.

The organism does not solve this — it has no depth counter, no budget, no
recursion guard anywhere in `agent_fabric.py`. A port would inherit the bug.

Minimum viable guard, to be designed in Wave 2:
- **hop count** on the correlation chain (a chain is a tree, not a graph;
  a node may have many children but a reply has exactly one parent)
- **per-thread generation ceiling** (e.g. 4) after which a notice is
  recorded but marked terminal
- **per-tick interaction budget** per employee, mirroring the notice board's
  existing `DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK` pattern
- **budget-skipped work must not consume budget** — already the established
  convention at `notice_db.rs:844`

Without this, the chronograph cannot be switched on in any real deployment.

### GAP 4 — "Several kinds of feeds" is unmodelled.

The owner described social-media-like feeds. The current `InboxQuery` has
`after_epoch`, `selectors`, `pending_only`, `reader_id`, `limit` — a cursor
and a filter. There is no notion of a *feed* as a first-class thing with its
own ordering, its own unread state, and its own identity.

This is a design decision, not a bug: either feeds are *derived views*
(general / department / team / role / direct, all the same table with
different selectors — cheap, no new state) or they are *first-class rows*
(own table, own cursor — expensive, enables per-feed ranking later). I
recommend derived views for Wave 2 and first-class only if ranking proves
necessary, because the unread state is already carried by `acked_by`.

## What I will NOT build without a decision

- **A synthesis agent that reads all worklogs and writes a report.** The
  owner's design is deliberately *not* this. Synthesis is emergent from board
  traffic. A summarizer would compete with the emergent signal rather than
  produce it.
- **Auto-repair of the 6 empty-`skills` jobs.** Unstaffed is a legitimate
  state; a seat can exist with nobody in it.
- **Any TTL that deletes an employee by age.** `agent_type` is a *role*
  (see the correction in `WAVE1-DECISIONS.md`), never a lifetime. The
  organism's "30m TTL" for `fixer` exists only in a comment
  (`agent_fabric.py:40`) and is enforced nowhere.

## Sequencing

The owner's six requirements are **not** independently buildable. They
serialise:

```
GAP 1 recipient resolver  ──┐
                             ├──> feeds exist ──> assignment board
GAP 2 org chart (reports_to)┘                        │
                                                      v
                                          dept head has a surface area
                                                      │
                                                      v
                                        CEO convenes heads, aligns
                                                      │
                                                      v
                                        heads refactor boards/roles/staff
```

Nothing above the resolver can be built honestly, because every layer needs
to resolve "who is in this department" and "who reports to whom". Building the
CEO meeting first would mean hard-coding a hierarchy that the resolver has to
invent later.

**Wave 2 order:** loop guard (GAP 3 — safety before features) → recipient
resolver (GAP 1 — the keystone) → feeds as derived views (GAP 4).
**Wave 3:** org chart + department-head assignment boards.
**Wave 4:** CEO alignment meeting and the structural-change path it authorises.

## Open questions for the owner

1. **Does a head's assignment-board edit need a capability grant, or is
   `reports_to` sufficient?** A head refactoring their own board is
   self-scoped. Reassigning roles *across* departments is a different
   authority. This is a security boundary, not a UI question.
2. **What is the CEO's own identity** — an `Employee` row like everyone else
   (so the meeting is a board interaction), or a distinct role in the runtime?
   If it is a row, the meeting is just a correlated notice thread, which is
   elegant and needs no new machinery.
3. **Does alignment bind?** If the CEO posts an alignment and a head
   disagrees, is that recorded as a dissent row, or is it noise to be
   filtered? Affects whether the board needs a first-class decision object.

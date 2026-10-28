# Red-team audit: the unified session-management re-architecture — redundant models, a surviving twin pane, and the copy/paste deltas

**Date:** 2026-10-28
**Auditor:** agent-jcode (tasked by the operator: remove the existing redundant model; the jcode pane should have been copy/paste-then-refactor; be /ponytail)
**Scope:** the 5-iteration re-architecture (iters 1444/1450/1458/1459/1460) against its own plan, against vendored jcode (`../external-repos/jcode`), and against the repo's surviving surfaces.

---

## Verdict in one paragraph

The re-architecture is **functionally complete and correct** — one loader, one record shape, three surfaces reading it, a real pane, a real gesture, a real API. But it is **not finished as a deletion story**: the plan removed ONE of the TUI's two session browsers and left the other one standing, so the codebase still carries two panes for browsing sessions with two listing sources. The pane itself is a *faithful refactor* of jcode's (not a reimplementation), with three real deltas: no mouse routes on the hub, no loading/spinner state, and no folder/group structure — all three are *correctly* scoped out for a single-DB daemon, but the mouse omission is the one an operator will feel. Below: ranked, with the exact cut for each.

---

## R1 — `session_branching.rs` is the surviving twin pane (CRITICAL, the main redundancy)

**What it is:** a complete second session browser — 319 lines of state + render, its own key arms (key_handling.rs:624-669, 11 arms), its own dialog priority (dialog_routing.rs:56), its own overlay row (operant_overlays.rs:363-365, `Sink::Buffer`), its own background drain (channel_drains.rs:156-183), its own loader (`fork_tree` → `db.session_parent` + `db.child_sessions`), its own tests.

**Why it is now redundant — the proof, not the assertion:**

1. **It lists sessions.** `fork_tree` returns the parent + forks of a session as a selectable list — that is a session list, filtered by parentage.
2. **The hub lists the same objects, richer.** `SessionEntryV2` already carries `parent_id`, and `RowKind::Branch` exists precisely to mark those rows. `session_hub/render.rs:59` already renders branches distinctly (indent under parent).
3. **The switch mechanism is shared.** Both call `pending_switch_session`. There is no mechanism the branch browser has that the hub lacks — the unifying is already done at the data layer; the pane layer was left split.

**The cost (why it matters, not just tidiness):** every session-surface change now pays twice — the overlays guard table, the dialog priority ladder, the key dispatch, the close-all path, the drain plumbing, the test surface. That is the exact "two surfaces, one migration" tax the plan's RT-A fragmentation finding was written to end.

**The cut (ponytail, one iteration):**
- `Ctrl+B` (and the keybinding at key_handling.rs:1359) opens the hub with a **`ByBranch` filter** — the hub's filter cycle gains a fourth mode; `rebuild_visible` filters `parent_id == Some(current)` OR `id == current`. No new code path, no new state, no new render.
- Delete `session_branching.rs`, `fork_tree` (adapter_types/history.rs — which then becomes a 12-line file with one caller), the `branch_list_rx` field + `drain_branch_list`, the key arms, the dialog row, the overlay row, the two tests.
- "New branch" (Ctrl+N-in-modal) is **rewind** — already a separate flow (`/rewind`, `pending_rewind`); the branch browser's create-name arm is a fork with a hand-typed title, which `agent/rewind.rs` does with a turn number. Name-feedback: `/rename`.
- **The one genuine loss:** the branch browser's delete-confirm arm. `sessions delete` exists in the CLI (`cmd_delete`), so the capability is not lost, only the shortcut.

**Payoff:** ~350 lines and one whole surface deleted; the "one session pane" claim in the CHANGELOG becomes true.

---

## R2 — the gateway's `PersistentSessionStore` still lists sessions into its own print path (HIGH)

`operant gateway sessions` (cmd_gateway.rs:849-887) prints a table from `store.list_active_sessions()` — a **second listing shape** with its own columns (platform/user/channel/created). The hub row now carries `source: "gateway"` and the API route can read the same DB.

**Honest verdict:** this is NOT fully redundant — the store is platform-bindings (which chat maps to which conversation), not the conversation record. Deleting it would lose real information the hub row does not carry (platform, channel, user-hash).

**The ponytail fix:** do **not** unify the stores. One line of honesty instead: the command's header says "Gateway Sessions" — it is a *binding* view, and the CHANGELOG/`/help` should say so, because an operator reading "gateway sessions" next to a unified-session feature reasonably assumes it is the same list. (10-minute docs fix; a store unification is a real feature with a real cost, not this task.)

---

## R3 — the pane vs jcode: it IS a refactor, and the deltas are where you'd expect

Measured against `../external-repos/jcode/crates/jcode-tui/src/tui/session_picker/` (6,328 lines across 6 files):

| jcode mechanism | operant hub | verdict |
|---|---|---|
| AND-token search over a precomputed index (filter.rs) | AND-token search over 8 fields (mod.rs:188 `rebuild_visible`) | **faithful refactor** — our per-field scan is the same semantics on a smaller record; no reimplementation |
| Filter modes (filter.rs) | 5 modes (All/CurrentDir/ByEmployee/Active/Archived) | **refactor + extension** (employee is operant's own) |
| Narrowing cache reuse (`cached_search_refs`) | none | **gap, correctly scoped out** — 500 rows × 8 fields is cheap; jcode caches because their picker walks server groups + orphan sessions + the file index |
| Cursor nav + page jump (navigation.rs) | `select_next/prev` only | **partial** — no PageUp/PageDown/Home/End in the hub (they exist for other overlays) |
| **Mouse: preview scroll, hover region** (navigation.rs:105-180) | **none — mouse.rs:831 gates to `visible` and nothing else** | **real gap** (see R4) |
| Loading/spinner state (loading.rs 3,102 lines) | `loading: bool`, no spinner | **correctly out of scope** — our load is one local SQLite query, not jcode's server + file-index fan-out |
| Crash-reason line, token estimates, server groups, orphan sessions | absent | **correctly out of scope** — no server groups in a single-DB model; ponytail says leave them out |

**Conclusion on the copy/paste question:** the pane was written as a refactor against jcode's *semantics* (the search contract, the filter cycle, the preview geometry, the close-tap handoff) rather than its *code* — which is the right call, because their picker is coupled to `SessionInfo`/server-group/`SessionRef` plumbing we deliberately do not have. The risk the operator named ("could have been copy/paste") did not materialize as a reimplementation; it materialized as the two *behavioral* gaps below.

## R4 — the hub has no mouse routes (MEDIUM, the felt gap)

jcode's pane scrolls the preview on wheel and hit-tests the preview region (`mouse_over_preview`, `handle_mouse_scroll`). Ours opens the hub (mouse.rs:831 exists only to *let the key events through*) and then ignores the wheel — so scrolling the transcript preview requires `←`/`→` (which scroll the *preview*), while every other operant overlay scrolls on wheel. An operator with a mouse will find the one pane that doesn't.

**The cut:** ~15 lines — in mouse.rs's visible branch, route `MouseEventKind::ScrollDown/Up` to `session_hub.scroll_preview(±2)` (same call the ←/→ arms already make). No new concept.

## R5 — PageUp/PageDown/Home/End absent in hub nav (LOW)

Trivial, same pattern as other overlays; only worth folding into R1's iteration since it touches the same key arm.

---

## What the audit does NOT ask for (explicit non-findings)

- **No pane-YAGNI.** The pane is right: three surfaces read one record, and the operator asked for the picker. It stays.
- **No `employee_id` criticism.** Honest NULL for predating rows is correct; no backfill lie.
- **No hub-scope creep** (no spinner, no server groups). The audit scopes them OUT deliberately and records why, so a future agent doesn't "restore parity" by porting 3,100 lines of loading.rs.
- **The API routes are correct** (read-only, token-gated, cap-enforced, flock-shared) — verified live in the deploy (F0149).

## The iteration, as I'd cut it

**One iteration: `fix(iter-1461): the branch overlay is the hub with a parent filter`** — R1 (delete `session_branching.rs` + `fork_tree` + `branch_list_rx`, add `HubFilterMode::ByBranch`, retarget Ctrl+B), plus R4's 15 mouse lines and R5's 3 nav arms folded in (same file family, same test battery). Gate: the TUI-scenario corpus (the excluded.json row for session-branching gets an honest replacement — the scenario's exclusion reason dies with the overlay), overlays guard test, hub tests, `command_sweep` 46→unchanged.

**One docs fix: `docs(iter-1462)`** — R2's honesty line for `gateway sessions` + the CHANGELOG's "one session pane" sentence corrected to be literally true after 1461.

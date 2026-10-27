# Unified session management: jcode-style session picker pane + omp-style unified session/subagent hub + employee tags

Dated-frozen: published `2026-10-28`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Companion audit (the evidence for every claim here):
`docs/audit/2026-10-28-session-management-red-team-audit-all-surfaces-vs-jcode-oh-m.md`.

## Mechanism

One full-frame page replaces the 70x20 session modal, unifying sessions,
branches, and live subagents into one list — jcode's picker geometry and
mechanics (full frame, 40/60 list+preview split, search, filter cycle,
grouping, background load), omp's unified record and gesture (one record kind
for main + sub, roster + focus views, double-tap ← on empty input), operant's
existing switch plumbing (`set_session_id` retarget — untouched) and its
employee/seat model (nullable `employee_id` tag on every session row).

The port source of record: `../external-repos/jcode/crates/jcode-tui/src/tui/session_picker*.rs`
(+ `jcode-tui-session-picker` types crate) for the pane; omp
`packages/tui/src/overlays/agent-hub.ts` + `session-observer-registry.ts` +
`coding-agent/src/modes/controllers/input-controller.ts:753-810` for the
unified record, the roster/focus split, and the ←← gesture.

### What gets built (4 things, nothing else)

**1. `SessionEntryV2` — the one record** (`crates/operant-core/src/session/mod.rs`
or a new `session/listing.rs`; fed by `database.rs`):

```rust
pub struct SessionEntryV2 {
    pub id: String,                  // sessions.id (or delegation handle for live subagents)
    pub parent_id: Option<String>,   // sessions.parent_session_id (branches, subagent parents)
    pub title: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub employee_id: Option<String>, // NEW column; None = the operator, personal mode
    pub source: String,              // sessions.source (local | gateway | cron | tui …)
    pub origin: Option<String>,      // platform:channel:user for gateway rows, None locally
    pub message_count: usize,
    pub cost_usd: f64,
    pub tokens: u64,                 // input+output+cache columns already exist
    pub cwd: Option<String>,
    pub status: RowStatus,           // Current | Live{streaming} | Idle | Archived
    pub kind: RowKind,               // Session | Branch | Subagent{goal, model, dispatch_time, status}
}
```

Loader: `Database::list_sessions_rich(limit) -> Vec<SessionEntryV2>` (one
SELECT — all columns exist except `employee_id`), joined for `RowStatus` with
the live-subagent registry snapshot (`async_delegation::records()` needs one
new accessor — the registry already exists at async_delegation.rs:100-152).
The gateway's `list_sessions` stays as-is (different table, different
lifecycle); only the TUI/CLI `sessions` surface moves to the v2 record.

**2. Employee tag** — one migration + one write point:

- `ALTER TABLE sessions ADD COLUMN employee_id TEXT` (additive-reconciliation
  pattern already in this repo — see the identical wave-2 ALTER at
  `gateway_session.rs:414-448`; copy that mechanism, do not invent one).
- Write point: session start (`database.rs` save path) reads the agent's seat
  (`agent.seat_id()` — the getter next to `set_seat_id`, events.rs:694) and
  stores it; TUI sessions store `None` unless a seat is set (personal mode).
  No second employee directory: `org/seat_policy_db` and operant-os `Employee`
  remain the source of truth for names/titles; the picker renders the id and
  resolves display names through the existing seat db when present.

**3. The page** — one new module `crates/operant-cli/src/tui/session_hub/`
(copy-then-refactor from jcode's session_picker, per the user's
copy/paste-then-refactor instruction):

- `mod.rs` — `SessionHub` state: items (grouped), filter mode, search query,
  selected index, list/preview areas, load receiver. Port jcode's
  `SessionPicker` state shape (session_picker.rs) minus external-CLI members.
- `render.rs` — port jcode's `render` (2382-2470): vertical constraints
  (search bar row, then `Min(8)`), horizontal 40/60 split, list rows with
  employee tag + status + cost, right pane = message preview (sessions) or
  live progress/result (subagents). NO 70x20 modal — this renders as a
  full-frame overlay mode in `overlay_visible_guards` (the journey_view
  pattern), replacing the `session_browser` row.
- `filter.rs` — port jcode's filter-mode cycle, reduced to what operant has:
  `All | CurrentDir | ByEmployee | Active | Archived`. (jcode's external-CLI
  modes dropped — no importers here.)
- `navigation.rs` + `loading.rs` — port jcode's cursor/scroll math and the
  cached-then-refresh load; the cache is the plan-03 mpsc pattern App already
  runs (`session_list_rx` at app/mod.rs:580) — reuse it, don't duplicate.
- **Roster/focus views (omp)**: the same page has two modes. List mode = the
  unified rows above. `Enter` on a `Subagent` row = focus view (omp's chat
  view): tail that delegation's goal/status/result inline. `Enter` on a
  `Session`/`Branch` row = the existing `pending_switch_session` flow
  (tui_app.rs:1125 — untouched). This is the omp unification: Main and
  subagents are peer rows in ONE page, not two surfaces.
- Rename (`r`) and the existing `/session`, `/resume`, `/sessions` commands
  route here (commands.rs:526 — command names unchanged, so no command-registry
  churn).

**4. The ←← gesture** — omp's exact shape, in operant's seam:

- `key_handling.rs` Left arm (:1617): the plain-Left `else` branch checks
  `prompt_input.is_empty() && cursor == 0` and calls
  `self.detect_left_double_tap()`; second tap within **500ms**
  (omp `LEFT_TAP_WINDOW_MS` / `LEFT_DOUBLE_TAP_MAX_GAP_MS`) opens the hub.
- Tap counter + `last_left_tap: Option<Instant>` on App (omp
  `#leftTapCount`, input-controller.ts:316-320).
- **Tap-state handoff** (omp's `armCloseTap`, agent-hub.ts:626-636): the
  opener seeds the hub's own close-detector with the current time so the SAME
  ←← that opened the page arms its close — without it the user needs a third
  ← to escape (omp issue #4780, already solved there; copy the solution).
- Focused-subagent behavior (omp input-controller.ts:763-772): if a subagent
  row is focused, ←← returns to the list (matches Esc), NOT to the chat.
- jcode's config-gated single-Left variant is NOT ported (double-tap is the
  ask; single-Left on empty is a foot-gun for terminal users who arrow-hold).

### Port mechanics (copy/paste then refactor — the user's instruction)

1. Copy jcode's `session_picker.rs` + `session_picker/` (filter, loading,
   navigation, render — ~6.3K lines) into `tui/session_hub/` verbatim.
2. Compile-fix loop: replace jcode types with operant's (`SessionEntryV2`,
   operant's theme/`rgb` helpers, `Line`/`Span` are the same ratatui), delete
   every external-CLI member (Codex/Claude/Pi/OpenCode/Cursor loaders,
   `ResumeTarget` external variants, onboarding band, crash banner — operant
   has no equivalent sources), delete server-grouping (no servers; grouping is
   by `employee_id` then `source`), delete live-pid presence (same-process;
   `RowStatus::Live` comes from the delegation registry instead).
3. What survives the cut is the load-bearing ~40%: layout math, search+token
   highlight, filter cycle, cursor/scroll, preview pane plumbing, the
   cached-then-refresh channel flow. That is the value jcode bought with the
   other 60%.

## Gates / success criteria

(fixed before the run)

- G1: `/resume` opens the full-frame page at 110x45; list+preview 40/60
  split; the old 70x20 modal is GONE (`session_browser.rs` deleted; its
  tests move to the hub; overlay guard table updated; no dead flag).
- G2: rows group by employee tag; `ByEmployee` filter cycle works; a session
  created under a seat shows that id, personal sessions show none.
- G3: subagent rows appear while a background delegation is live
  (`delegate_task(background=true)`), with status; Enter focuses; ←← or Esc
  returns; the row disappears when the registry prunes it.
- G4: Enter on a session row retargets via `pending_switch_session` and the
  transcript rehydrates (existing behavior — regression tests must stay
  green).
- G5: double-tap ← on empty input opens the page from the composer; a third
  ← inside the page closes it (tap-state handoff); ←← with text in the
  composer does nothing new (cursor still moves normally with text present).
- G6: `cargo test -p operant-cli --bin operant` scoped green; clippy gate
  passes (`-D unwrap/expect` — ported jcode code uses `unwrap_or_default`
  shapes already); `command_sweep.py` still reports every command consumed
  (command names unchanged, so 98/98 must hold).
- G7: 50-session cap is gone (loader takes a limit; default 500, search
  covers the rest — jcode's search-index approach).

## Cost

Four iterations, ordered to dodge the peer's dirty files (F0094: key_handling,
app/mod, commands, init, tui_app, terminal_setup):

1. **iter-A (core, zero TUI-file overlap)**: `SessionEntryV2` +
  `list_sessions_rich` + `employee_id` ALTER + seat write point +
  delegation-registry accessor + unit tests. Files: `database.rs`,
  `session/mod.rs`, `async_delegation.rs`, `agent/events.rs` (one getter).
2. **iter-B (the pane)**: `tui/session_hub/` copy+refactor; `/resume` routes
  to it; delete `session_browser.rs` + its overlay row. Files: new module,
  `render/operant_overlays.rs`, `dialog_routing.rs`, `app/mod.rs`,
  `app/commands.rs` (re-grep peer state first — by then F0094 should have
  landed).
3. **iter-C (subagent rows + focus view)**: registry join, `RowKind::Subagent`,
  focus view, tests with a fake delegation.
4. **iter-D (the gesture)**: key_handling Left arm + tap state + handoff +
  G5 tests. Lands last, smallest, most overlap-prone.

## Risks

- Peer WIP on key_handling/app files: iteration order above; each iteration
  re-greps the dirty set before touching it; the peer's F0094 explicitly
  listed the same files.
- jcode picker assumes multi-session servers; operant is single-process —
  grouping reduced to employee/source, presence reduced to the delegation
  registry. If a session server lands later (F0066 gap-4), `ServerGroup`
  slots back in at the grouping seam.
- The gateway keeps its own `gateway_sessions` table and lifecycle (7 platform
  adapters, employee-bound, rolling summaries). This plan does NOT unify the
  gateway store into `sessions` — that would break the gateway's restart
  rehydration. The gateway rows become visible in the hub later via the same
  `SessionEntryV2` join if the operator asks; not in scope now (YAGNI).
- operant-os/Holon drive seats, not sessions; the only OS touch is that the
  hub's employee column reads seat ids — no OS file changes.

## Resume here

All four iterations are open. Start at iter-A (core record + employee tag);
it has no file overlap with any in-flight peer work.

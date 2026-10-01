# TUI Overhaul — Fleet Roadmap (Waves 3–6)

> Companion to `docs/PLAN-TUI-OVERHAUL.md` (the design plan). This file is the
> **execution roadmap**: what is done, what is next, and the protocol rules the
> agent fleet must follow to keep parallel work safe.
>
> HEAD at time of writing: iter-530.

---

## 0. Correction first, because it changes the protocol

Three times now two agents have committed with the **same `iter-N` label**
(516, 518, 529). It is worth being precise about what did and did not happen,
because the fix is different depending on the cause.

**What did NOT happen:** nobody force-pushed, and no work was destroyed.

Evidence:

```
$ git reflog show origin/main | head -8
9b589cf7 refs/remotes/origin/main@{0}: update by push
ee27a5d0 refs/remotes/origin/main@{1}: update by push
c5c9271a refs/remotes/origin/main@{2}: update by push
...
78a1156a refs/remotes/origin/main@{6}: update by push
```

Every single update is `update by push` — a fast-forward. No `reset`, no
forced update. And both commits from each collision are still in history:

```
78a1156a fix(iter-518)   ← peer
75d2c8af feat(iter-518)  ← mine, both present
d32cfeee feat(iter-517)   ← mine, renumbered before push
04861610 fix(iter-516)   ← peer, both present
ee27a5d0 docs(iter-529)  ← peer
9b589cf7 test(iter-530)  ← mine, renumbered before push
```

**What DID happen — the actual mechanism.** `AGENTS.md` says to derive the next
iteration number with:

```bash
git log --oneline | grep iter-[0-9]+ | head -1   # ← reads LOCAL history
```

That is a **read-then-write race**. Two agents run that command at the same
moment, both read the same "last" number, both compute the same "next" number,
both write a commit with it. Nothing about git is unsafe here — the *number
allocation scheme* is unsafe. It has no lock and no allocation authority.

The damage is confined to a duplicated label on two commits. Twice I caught it
while my commit was still unpushed and amended locally (a purely local
operation). Once (518) I did not catch it, and the peer and I both shipped
`iter-518`.

---

## 1. Protocol amendments for `AGENTS.md`

These are the concrete rules that would have prevented all three collisions.
They belong in `AGENTS.md`, not in agent briefs, because the failure is
systemic rather than agent-specific.

### 1.1 Derive the iteration number from `origin/main`, never local history

```bash
git fetch origin
git log origin/main --oneline | grep -oE 'iter-[0-9]+' | head -1
```

Local history is stale the moment a peer pushes. `origin/main` is the only
numbering authority.

### 1.2 Label collisions are recoverable, and the recovery is cheap — but do it before pushing

Immediately after committing, before `git push`:

```bash
git fetch origin
git log origin/main --oneline | head -3   # did anyone take my number?
git rev-list --left-right --count origin/main...HEAD   # behind=? ahead=?
```

If a peer advanced `origin/main` past your base, or already used your number:
`git commit --amend` with the next free number, then push. Amending an **unpushed**
commit is local-only and safe. Amending after pushing would require a force-push,
which is forbidden — so the check belongs **before** the push, every time.

### 1.3 If a collision does reach `origin/main`, do not rewrite history

Two commits sharing a label is a cosmetic defect, not an incident. Renumbering
published history is strictly worse than the duplicate. Record it and move on:
leave both commits, note it in the next commit body. Git history is append-only
by design; an audit that can be rewritten is not an audit.

### 1.4 Never let two agents own the same file

The `iter-N` race was the *only* collision. Zero file-level conflicts occurred
across four parallel agents because each was given an exhaustive, exclusive file
list and told to stop and report rather than widen scope. Keep doing that.

### 1.5 One serialisation point per shared artefact

During Wave 3 the golden baselines were a shared artefact. Four agents each had
the ability to rewrite 120 files. The rule that made it safe: **no agent may
regenerate a golden; the integrator does it once, centrally, after all agents
land.** Any agent that does it concurrently with another agent's rebuild risks
capturing the wrong binary. Generalise this to any generated artefact.

---

## 2. Where we actually are

**Foundation — complete and gated.** iters 517–530.

| Landed | What |
|---|---|
| iter-517 | 4 jcode crates vendored as modules (9.6k LOC): 22 colour roles, markdown/wrap core, workspace-map widget, 3D anim samplers. 144 vendored tests, 0 warnings. |
| iter-518 | Visual regression gate: `--baseline` drift detection, `--capture-frames`, tmux real-terminal capture. Drift failure proven. |
| iter-519 | Both palettes bridged; `adapt_buffer_for_display` wired as the single per-frame choke point; base chrome migrated; 4 competing accent sources retired. |
| iter-521 | `modal_frame` primitive + `space` ladder landed with **zero call sites migrated**, so the goldens stayed valid. |
| iter-522 | Per-cell style goldens. Fixed a colour-depth divergence that had made the first goldens unreproducible. |
| iter-525 | 46-branch paint-in-z-order ladder → frozen 46-row dispatch table. `render_app` 413 → 178 lines. Benchmark: substitution is **2 ns/call** unconfigured, 0.41% of a 30fps frame configured. |
| iter-526 | DEC 2026 synchronized update + full-frame clear (defence-in-depth, not a bug fix). |
| iter-528 | Loop architecture: 9 named channel drains, `redraw_reason()`, input coalescing. |
| iter-530 | Fixed a pre-existing `ACTIVE_LOCK` test-isolation flake (5/5 parallel runs clean). |

**Gate status:** 50 scenarios, 120 goldens (75 text + 45 style), 60 gated
variant-runs, 5 assertions-only scenarios (7 variant-runs — two of them carry an
`.80x24` variant) with measured reasons, plus 3 surfaces in `excluded.json`.
`verify PASSED: 0 drift` as of iter-528. 803 TUI tests green.

**Wave 3 — LANDED (iter-533 goldens, iter-534 code).** Four agents, 38 files,
zero file conflicts. 16 sizing formulas and 5 title idioms collapsed into
`modal_frame`; ~40 colour literals became theme-repainted roles; 4 border-less
dialogs gained frames. 116 goldens regenerated centrally. `verify PASSED: 0 drift`,
803 tests, 0 warnings, fmt clean.

---

## 3. Wave 3 close-out (DONE — kept as the record)

Single integrator, sequential. No parallel agents. All five steps completed:

1. ~~`prove`~~ — double-render, byte-diff, regenerate, third-render re-verify.
   **58 proven, 0 unstable on the third run.**
2. ~~`verify`~~ — `PASSED: 0 drift` across 60 gated scenarios.
3. ~~Goldens committed separately~~ (iter-533, 91 files) from the code
   (iter-534, 41 files), so a reviewer sees two distinct facts.
4. ~~Reviewed for attributability.~~ Every drifted surface was predicted by the
   agent that owned it. `prove` caught the two exceptions itself — see below.
5. ~~Code committed~~ with per-group attribution.

### What `prove` caught that nothing else would have

Neither a build nor `cargo test` executes screen assertions — only golden
regeneration does. That step is **not skippable**, and it earned its place:

- `global-search` @120x40 asserted `contains:Esc: close`. Agent 3C had deliberately
  decomposed a hint crammed into the title (`Search [Esc: close, Enter: insert, …]`)
  into a title row plus a real bottom hint. The assertion was **pinning the exact
  defect the migration exists to remove**.
- `global-search` @80x24 asserted the same string and **correctly failed**: at that
  width the primitive truncates the hint and `Esc` appears nowhere. Right
  narrow-terminal behaviour, so the 80x24 variant now pins only that the surface
  renders, and hint elision is logged as Wave 4 item 4.5 rather than papered over.

### What the migration was expected to change (agents' own predictions)

- **Dialogs (3A):** square → rounded borders; title onto its own row; four
  `Esc` spellings converged on `HINT_ESC`; 4 border-less dialogs gained a frame.
- **Panels (3B):** floor guarantees where `.min(w-4)` could reach 0×0; two
  centred titles removed; the app's only centred title gone.
- **Overlays (3C):** `global_search`'s hint-in-title decomposed into title +
  bottom hint; `settings_screen`'s `"Esc close"` converged.
- **Chrome (3D):** colour-role convergence only, expected value-identical on the
  default theme.

### Bugs the migration found that the gate could not have caught

- `memory_file_selector`: height `(N+6).max(8)` with **no upper clamp** — could
  exceed the terminal.
- `session_branching`: absolute height `70` into `centered_rect`, which silently
  shrank to full-terminal with **no margin**.
- `dialog_select`: white-on-amber selected rows under the default theme — an
  existing contrast defect.
- `dialog_priority()` vs the inline key-handler order: **5 of 6 probe pairs
  diverge** (see §6).

These are why "make it consistent" produced more than cosmetic change.

---

## 4. Wave 4 — affordances operant lacks entirely

Nothing in Waves 0–3 adds these; they do not exist in operant today. All are
additive, all are independently shippable, and all are the difference between
"tidy" and "designed".

| # | Affordance | jcode reference | Notes |
|---|---|---|---|
| 4.1 | **Narrow-terminal collapse** — `MIN_*` floors per surface, graceful collapse instead of crushing | `ui.rs:2835-2851` | `modal_frame` already provides the modal floor; this extends it to side panels and pickers |
| 4.2 | **Empty states** for every panel and picker, distinguishing "nothing here yet" from "no matches" | `ui_input.rs:157-162` | operant has neither |
| 4.3 | **Focus indication** — focused pane's border takes the focus role | `chrome.rs:31-36` | operant has a `FocusTarget` enum but no visible focus state |
| 4.4 | **Overflow labelling** — `↑N` above, `+N more` below | `ui_input.rs:303-314` | never a silent truncation |
| 4.5 | **One hint line** — priority-ordered, mode-aware, suppressed when an overlay owns the rows | `ui_input.rs:2483-2513` | replaces scattered per-dialog hints |
| 4.6 | **Actionable errors** — name the recovery command, restore the user's input | `tui_lifecycle.rs:337-351` | operant shows messages without a next action |
| 4.7 | **Resize preserves reading position** | `app.rs:890-894` | currently teleports to the new bottom |

**Deployment:** 3 parallel agents, disjoint files — (a) focus + overflow, (b)
empty states, (c) hints + actionable errors. 4.1 and 4.7 are cross-cutting and
should be **one integrator**, not parallel agents: both touch the render path's
area computation.

Each affordance is a golden-affecting change. Golden regeneration stays
central (§1.5).

---

## 5. Wave 5 — structural follow-ups

Lower priority; each is a decision, not just work.

| # | Item | Why it matters |
|---|---|---|
| 5.1 | **Make `redraw_reason()` live.** It is currently `#[cfg_attr(not(test), allow(dead_code))]` because the only inspectable surfaces — the event bus and F12 overlay — live in `tui/debug/`, which was frozen. Unfreeze it and record the reason per frame. | Today a perf question is answered by bisecting the loop. |
| 5.2 | **Cadence gating.** operant draws unconditionally every iteration. jcode gates with `needs_redraw`. After 5.1 lands, gating becomes possible and is the real win from the reason table. | Idle CPU |
| 5.3 | **Inline interactive pickers.** jcode's largest subsystem operant lacks (~6.3k LOC). Too large to port; only worth it if the session-picker UX is a known pain. | Scope decision |
| 5.4 | **Oklab palette scoring.** ~2.4k LOC upstream. Not a parity gap — operant's palettes work and are now collision-tested. | Deferred deliberately |

---

## 6. Open decisions — these need the user, not an agent

### 6.1 `dialog_priority()` vs inline key routing (latent bug)

`key_handling.rs:45` claims the inline handlers already follow
`dialog_priority()`'s order. **They do not.** An empirical probe — built, run,
deleted — opened two dialogs at once and pressed the key each should get:

| Pair | `dialog_priority()` says | Inline actually gives key to | |
|---|---|---|---|
| McpApproval vs Stats | McpApproval | McpApproval | agree |
| BypassPermissions vs GlobalSearch | BypassPermissions | **GlobalSearch** | diverge |
| EffortPicker vs McpApproval | McpApproval | **EffortPicker** | diverge |
| Help vs HistorySearch | HistorySearch | **Help** | diverge |
| SessionBrowser vs SessionBranching | SessionBrowser | **SessionBranching** | diverge |
| Settings vs Export | Settings | **Export** | diverge |

Plus `memory_file_selector`, `theme_screen` and `rewind_flow` are gated inline
but **absent from the priority chain entirely**.

The worst case — `mcp_approval` at priority 3 versus inline 19 — is currently
**unreachable**, because `McpApprovalDialogState::show` has no call site (which
is why it is parked in `excluded.json`). So it is a loaded gun, not a firing
one: it arms itself the moment someone wires that dialog up, and nothing will
fail loudly at that moment.

**Options:** (a) leave as-is and document; (b) make `dialog_priority()` the real
router in its own commit, accepting the behaviour change and regenerating
goldens; (c) reconcile the two orders toward `dialog_priority()` and add the
three missing entries, keeping the inline shape.

I did **not** do this in iter-528: converting the router is a behaviour change
disguised as a refactor, and it belongs in its own iteration where the intent
can be judged.

### 6.2 Deferred `tui/debug/overlay.rs`

Two `.bg(Color::Black)` base fills remain there. Left alone because `tui/debug/`
was frozen for the duration of the migration. Two-line fix whenever unfrozen.

### 6.3 Peer's org feature

`cmd_org.rs` + `main.rs` + `tests/org_reason.rs` landed as iter-518. It had a
one-character type error that blocked every `operant-cli` build for several
hours; the peer fixed it properly in the end (a borrowing
`validated_reason<'a>() -> Result<&'a str>` rather than the `&` I applied).
**No action needed** — recorded so the history is legible.

---

## 7. Fleet rules distilled from what actually worked

These are not aspirational; each one prevented a real problem in Waves 0–3.

1. **Exhaustive, exclusive file lists.** Every agent brief ends with a fenced
   block of exactly the files it may edit, and "need a change elsewhere? STOP
   and report it instead." Zero file conflicts across four parallel agents.
2. **Name the frozen set explicitly**, including files a reader would assume are
   editable (`dispatch.rs`, `overlays/layout.rs`, `theme_colors.rs`).
3. **Forbid the shared artefact, centrally.** "DO NOT regenerate any golden" —
   not a preference, an instruction, with the reason stated.
4. **Quote the baseline.** Every agent was told the exact expected test count
   (803) and the exact expected gate output (`verify PASSED: 0 drift`), so
   "done" was checkable rather than asserted.
5. **Distinguish load-bearing mechanisms from guessed ones.** The colour-depth
   divergence, the `ACTIVE_LOCK` flake, and the dialog-order divergence were all
   found by *running* something and reading real output. Every one of them
   contradicted a plausible assumption. Briefs should say: measure, and if the
   probe contradicts the code comment, believe the probe.
6. **One serialisation point per artefact** (§1.5), and the integrator owns it.
7. **Serialise refactors that share a referee.** Phase 2 was deliberately run
   alone: two concurrent refactors verified against the same goldens make an
   ambiguous failure expensive to diagnose.
8. **Agents must report, not fix, out-of-scope findings.** Wave 3B found the
   `ACTIVE_LOCK` flake in a frozen file and reported it instead of touching it.
   That is the behaviour you want, and it should be asked for explicitly.
# Plan — 2026-11-19: Executing the Remaining Implementation Gaps (post-iter-2668)

**Status:** outline / outline-plan. Dated-frozen once landed. Supersedes the
scheduling half of `docs/plans/2026-11-19-remaining-implementation-gaps-the-open-ledger-rows-and-the-scale-socket-cluster.md`
(its T1–T7 closed: T1/T2 iters 2652/2654, T3 iter-2658, T4 iter-2661, T5 iters
2662/2663, T6 decision-gated, T7 iter-2664). This plan covers **what is still open
after iter-2668** and sequences it.

**Ground truth (re-derived at write time, not read from prose):**

- `git log origin/main --oneline -1` = `150a6f55 docs(iter-2668)`.
- `rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md` on origin = **9**.
- The 9 open checkbox rows on origin are: **RT-136, RT-209, RT-190, RT-179,
  RT-275, RT-280, RT-285, RT-293, RT-543**.
- **RT-554 is CLOSED on origin** (iter-2665, commit `a931aaeb`, ledger row flipped
  by `7a20660f` iter-2666). The peer's route (a) — `claim_session_here_marking` +
  `prune_dead_presence` returning reclaimed ids + guarded `end_reason='interrupted'`
  + the `sessions list` Status column — landed. **The earlier "T8 blocked" collision
  is resolved in the peer's favour; nothing of mine re-applies.** My complementary
  clear-on-resume patch stays parked in `stash@{0}` / `~/.jcode/scratch/my_rt554.patch`
  and is NOT scheduled here (see §7).

## Why each open row is not a plain patch

Every remaining row is one of three kinds: **(A) a stale checkbox on a row whose
work already shipped** (ledger hygiene), **(B) a product/architecture decision the
plan must not make silently** (needs an owner ruling before code), or **(C) a
genuine open tranche of bounded scope**. The sequencing below front-loads (A)
because it is safe, mechanical, and shrinks the open count before any ruling is
needed, then groups the (B) decisions so the owner rules once, then schedules the
(C) work that the rulings unlock.

| row | kind | one-line |
|---|---|---|
| RT-179 | A | all four functional-core twins done (`/memory` closed iter-1392); row prose already says "ALL FOUR REMAINING ITEMS ARE NOW DONE" — flip the checkbox |
| RT-136 | A / DARK-MERGE | confirmed-not-a-defect at iter-1176; `[pk]`/`[socialization]` honestly labelled, surfaced by `unused_setting_warnings` — close-as-documented |
| RT-209 | B (recommendation on file) | orphan `turn_attribution.rs`; render-at-turn-end (option 1) is recommended and does NOT touch the prompt cache |
| RT-543 | B (recommendation on file) | deprecate ACP daemon socket, keep stdio (option a) — the plan already records the recommendation |
| RT-275 | B | six unwired public surfaces (staged-write approval, DB tool-state, DB checkpoints, 3 config types) — wire or delete, each |
| RT-285 | B | three dormant surfaces (Autocompactor, `SessionStore::discard`, `TurnRetryState` restart signals) |
| RT-293 | B (honest half is C) | `notify` tool claims delivery it never makes; `MemoryManager` is write-only — the honest-labelling half needs no ruling |
| RT-280 | B | `PersistentSessionStore` resume/reset flags are read but never written by any production path |
| RT-190 | B | operant's memory-wire store is a 2nd disconnected bank family — ruled (c) document-the-boundary iter-1373; bridge (b) is the deferred tranche |

---

## Tranche A0 — ledger hygiene: flip the two rows whose work already shipped

**Scope:** `docs/audit/2026-06-19-BUGS.md` only. No code. One docs iteration.

- **RT-179** → `[x]`. Evidence: `/memory` closed iter-1392 (fifth twin after
  `/effort`+`/plan` 1379, `/context` 1386, `/export` 1391); the row's own tail
  already reads "ALL FOUR REMAINING ITEMS ARE NOW DONE". Re-derive the checkbox
  count after the flip (9→7... 8) in the "What is OPEN right now" header per
  RG-016 (the `rg -c` commands are the authority; prose is a claim about them).
- **RT-136** → close-as-documented (DARK-MERGE). It is a confirmed-not-a-defect:
  the tables carry "Nothing reads this table (RT-136)" doc comments, both are in
  `AppConfig::unused_setting_warnings`, and `operant.example.toml` advertises
  `[pk]` as NOT READ. A dark-merge row that is honestly labelled is closed by
  *recording the disposition*, not by writing a consumer. Rephrase the checkbox
  to a `[x]` whose body names iter-1176 as the sweep that proved it and states
  the disposition: **deliberately dark, surfaced, documented.**

**Verify:** `rg -c '^- \[ \]'` drops by exactly 2; `rg -c '^- \[x\]'` rises by 2;
`python3 scripts/audit-ledger-index.py` (the index-vs-checkbox gate) stays green;
the header's derived count matches. No cargo. **Risk: none.**

---

## Tranche A1 — the two recommendation-backed rulings (RT-209, RT-543)

Both rows already carry a RECOMMENDED option in their own ledger text. The owner
only needs to confirm the recommendation; then each is one bounded iteration.

### A1a — RT-209: render the turn breakdown at turn END (option 1)

The row measured that the plan's original seat (the volatile `<turn_context>`
tail) is architecturally impossible — that tail is built once per turn BEFORE the
loop, so every span would read `n/a` forever (the RT-199 failure re-labelled).
The row's **option 1 (RECOMMENDED)** emits on `TurnEnd` (`turn_end.rs`), which
already carries per-turn counters and already reaches every subscriber.

**Files:** `crates/operant-core/src/agent/turn_end.rs` (four `Duration` fields +
capture), the one emit site that fills them, `turn_attribution.rs` (wire
`TurnBreakdown::is_exclusive()` / `render()` to a real caller), a render consumer
(TUI status / trajectory — pick the cheapest that already subscribes to
`TurnEnd`).

**Guards the row demands, non-negotiable:** the double-count guard (inference +
tools both including the inter-tool gap must fail `sum == total`); the
`Incomplete`-state guard (the report must not collapse back to a `u64`);
`user_wait` stays `Unmeasured`, never `0s`.

**Test (mutation-proven):** a turn with a known inference span and a known tool
span renders a breakdown whose four spans sum to `total`; mutating the residual
(`saturating_sub`) or collapsing the `Option<Duration>` to `0` fails it.

**Decision needed:** confirm option 1 (operator sees it; the model does not). If
the owner wants the model to see it too, that is option 2 and re-opens the
prompt-cache risk — do NOT do option 2 without an explicit ruling.

### A1b — RT-543: deprecate the ACP daemon socket, keep stdio (option a)

The prior plan's T6 recorded the recommendation: editors speak ACP over stdio;
the socket half of `operant acp server` duplicates what `operant gateway run
--socket` now serves better (same 0600 gate, same slots, same child set, plus
channels). Deprecation = a loud startup warning naming the replacement + CHANGELOG
+ doc edits — NOT removal.

**Files:** `crates/operant-cli/src/cmd_acp.rs` (warn-on-start in the `Server`
socket arm), `docs/library/session-server.md`, `docs/library/cli-surface.md`
(`acp` row), `AGENTS.md` (command surface notes), `docs/CHANGELOG.md`.

**Test:** a warn-on-start test (the ACP socket server prints the deprecation line
naming `operant gateway run --socket`), plus the existing
`one_daemon_two_clients_one_shared_child_set` stays green.

**Decision needed:** confirm option (a) deprecate-not-retire. Do NOT do option
(b) full retire of `operant acp server` (that breaks the editor stdio
integration and is a separate, larger change).

---

## Tranche B — the four product-decision sweeps (RT-275, RT-285, RT-293, RT-280)

These four rows are explicitly "named, not fixed" because each surface needs a
per-item **wire / delete / keep-dark** decision. Schedule as ONE owner review
(the surfaces are small and the decisions are independent), then one iteration
per row applying the rulings. The decision table the owner fills in:

**RT-275 — six surfaces:**
| surface | default recommendation |
|---|---|
| `write_approval::stage_write`/`list_pending`/`get_pending`/`discard_pending` | delete (the feature was judged against seat-authority and never landed) |
| `Database::set_tool_state`/`get_tool_state`/`clear_tool_state`/`clear_all_tool_states` | delete (live store is `tool_state.json`) |
| `Database::store_checkpoint`/`get_checkpoint` | delete (live store is the shadow-git manager) |
| `operant-config` `ReliabilityConfig`/`MemoryPolicyConfig`/`MultimodalConfig` | delete OR wire — but if kept, they must be reachable from `AppConfig` or removed, because `[reliability]` today is a HARD load error |

**RT-285 — three surfaces:**
| surface | default recommendation |
|---|---|
| `session::autocompact::Autocompactor` | mount at the agent turn-end (post-run), marker default, behind a config key — OR delete |
| `SessionStore::discard` | make it destroy the persisted transcript (its doc says it does; today it only drops the warm copy) — this is a real behaviour fix |
| `TurnRetryState` restart signals | wire `restart_with_rebuilt_messages` into `agent/run.rs`'s content-filter-stall path, OR delete the three unused signals |

**RT-293 — two surfaces (the honest half needs NO ruling):**
| surface | default recommendation |
|---|---|
| `notify` tool | re-scope to "log a notification" and return `{"delivered": false, "reason": "no sink configured"}`, OR wire a real desktop/dbus sink — the false `delivered: true` must stop regardless |
| `MemoryManager` | delete (memory-wire owns MEMORY.md/USER.md) — the field is write-only |

**RT-280 — `PersistentSessionStore` flags:** either wire `suspend_session` /
`mark_resume_pending` to real `/stop` and restart paths, or delete the two flags
and their read branches. Recommendation: wire `resume_pending` (restart recovery is
a real gap) and delete `suspended` unless `/stop`-auto-reset is wanted.

**Per-row iteration shape (whichever way each ruling goes):** mutation-proven
test, `cargo check -p operant-core --tests` (and `-p operant-config` for RT-275),
clippy no new warnings vs base, `check-doc-claims.sh` 0 new failures, `genome
lint` baseline unchanged, ledger gate re-run, CHANGELOG entry, fetch-before-push.
For any DELETE, the `one_home_resolver` / doc-claim / clippy gates are the
backstop — run all three.

**The RT-293 honest half can ship BEFORE the ruling** as its own small iteration:
fix the `notify` result to stop claiming `delivered: true` (return the real "no
sink" outcome) and fix the `MemoryManager` stale-timestamp one-liner. This is
pure honesty, needs no decision, and shrinks RT-293 to just the wire/delete call.

---

## Tranche C — RT-190 memory bank bridge (deferred, largest)

Ruled (c) document-the-boundary at iter-1373; the boundary is written into
`operant.example.toml`. The remaining work is option (b): a one-time bank merge +
sync seam so operant's in-process `operant` bank and the `memory-wire serve`
daemon's `omp`/`memory-wire` banks cross. This is the only row that is a genuine
feature build (not a decision, not hygiene). It touches the memory subsystem
(`memory_wire.rs`, `memory_provider.rs`) and the `scripts/sync-vendors.sh` vendor
pin.

**Do NOT schedule until RT-275/285/293/280 are resolved** — the memory manager
delete-vs-wire ruling (RT-293) directly determines whether RT-190's bridge has a
second in-process consumer to reconcile. Sequence C last.

**Decision needed:** confirm cross-surface recall is actually wanted (the serve is
shared across ALL harnesses; moving operant onto it changes what every other agent
sees). If not wanted, RT-190 closes as "two families, boundary documented, bridge
declined" — a legitimate terminal state.

---

## Sequencing summary (each is one iteration = one commit + one push)

1. **A0** (docs only) — flip RT-179 + RT-136. Safe, mechanical, shrinks the count. **No ruling.**
2. **A1a** RT-209 — on owner confirm of option 1.
3. **A1b** RT-543 — on owner confirm of option (a).
4. **B-honest** RT-293 honest half (notify + memory-manager timestamp). **No ruling.**
5. **B-rulings** RT-275 / RT-285 / RT-280 / RT-293 (remaining halves) — one owner review, then one iteration per row.
6. **C** RT-190 — last, on owner confirm that cross-surface recall is wanted.

## Out of scope / parked

- **My clear-on-resume RT-554 patch** (stash@{0} / scratch). RT-554 is closed by
  the peer's route (a). If the operator later wants "resuming a session clears its
  interrupted flag" on TOP of the peer's mark-on-crash, rebase the scratch patch
  then — it is complementary and was never in conflict semantically, only in files.
- **Fleet rule 7 pre-existing failures** (the TUI-module drift 73-vs-74,
  `check-doc-claims` baseline) — record, don't fix.
- **AGENTS.md / genome lint baseline (15 errors / 3 warnings)** — pre-existing;
  the tranches must leave it unchanged, not clean it.

## Open owner rulings (blocking, in the order they unblock work)

1. RT-209 → option 1 (turn-end render, operator-visible)? [recommended yes]
2. RT-543 → option (a) (deprecate ACP socket, keep stdio)? [recommended yes]
3. RT-275/285/293/280 → the per-surface wire/delete table above. [defaults are recommendations]
4. RT-190 → is cross-surface recall wanted at all? [if no, close as declined]
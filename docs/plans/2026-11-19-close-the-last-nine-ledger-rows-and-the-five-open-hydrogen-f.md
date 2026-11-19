# What stands after iter-2675 — the last nine ledger rows and the five open Hydrogen findings

Two predecessor plans exist and this one is the **third and last**:
`2026-11-19-executing-the-remaining-implementation-gaps-post-2668.md` (iter-2669)
and `2026-11-19-close-the-last-nine-open-ledger-rows-the-dead-surfaces-go-th.md`
(iter-2671, mine). Both closed most of what they scheduled. What is left is
nine ledger rows and five confirmed Hydrogen defects, and the overlap between
them is the reason this plan exists rather than a fourth successor.

## The measured landscape at `208fefd1`

```text
git fetch origin
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   # 9
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   # 650+
```

### The nine open ledger rows

| # | row | true state at `208fefd1` |
|---|---|---|
| 1 | **RT-275** | `write_approval.rs` deleted iter-2665. **T1b shipped** — RT-612 closed iter-2673, `operant checkpoints` now reads the live store. Remaining: `Database::{set,get,clear}_tool_state` + `clear_all_tool_states`, `Database::{store,get}_checkpoint`, and `operant-config`'s three types |
| 2 | **RT-293** | both halves shipped iter-2665 (`notify` returns `{"logged": true}`, `MemoryManager` field deleted). **Checkbox never flipped** — the row is a stale `[ ]` |
| 3 | **RT-285** | restart signals deleted iter-2665. Remaining: `SessionStore::discard` still only drops the resident copy while its doc and caller claim a durable wipe |
| 4 | **RT-280** | untouched. `suspended` / `resume_pending` read on every access, written by nothing |
| 5 | **RT-209** | untouched. `TurnEnd::attribution` emitted; `log_attribution` is the only reader |
| 6 | **RT-543** | socket shipped (`run_session_socket` at `gateway_runner.rs:6446`, `chat --attach`, RT-542/545/547/552 closed). Remaining: ACP daemon disposition — deprecate (peer plan's A1b) vs retire |
| 7 | **RT-136** | confirmed-not-a-defect iter-1176; the index script already exempts its duplicate detail sections (`audit-ledger-index.py:76`) |
| 8 | **RT-179** | **stale checkbox** — the row's own tail reads "ALL FOUR REMAINING ITEMS ARE NOW DONE", five twins landed. iter-2674 opened Q5 asking for the ruling |
| 9 | **RT-190** | memory-bank topology; owner ruling on which store is canonical, explicitly not scheduled |

### The five open Hydrogen findings

A peer ran the context-pack review against **Gemini 4 Hydrogen at xhigh**
(iter-2671, `df1ea4c0`, 87 kB pack) and got 12 findings, each verified with
file:line evidence. **11 CONFIRMED, 1 REFUTED (F5).** Iters 2672/2673 closed
F5, F12 and RT-612. Five remain open:

| # | finding | verdict | fix shape |
|---|---|---|---|
| **F1** | `FallbackModelClient` is per-request (`fallback.rs:8,239`) — every ReAct iteration (cap 90) re-pays the dead primary's failure latency before reaching the healthy alternate. The only cooldown (`arm_cooldown` at :340,383) arms the NEXT provider, not a breaker on the primary | CONFIRMED, severity 3, bottleneck | per-route failure cooldown in `fallback.rs`, analogous to `dispatch_breaker.rs` |
| **F2** | `fallback.rs` dispatches an already-assembled request with no `max_tokens` / `context_window` / `fit_output` reference (grep: 1 hit, a test). `context-management.md` claims each route is re-fitted "before every request build" — false for the fallback route | CONFIRMED, severity 3, integration gap | resolve each fallback candidate's (context_window, max_output) inside `FallbackModelClient`, clamp `max_tokens` per route |
| **F3** | `RetrySettings::default().max_span_secs: 0` is INFINITE (`config.rs:1275`), and the gateway constructs the agent without overriding it (grep `max_span_secs` in `gateway_runner.rs`: 0 hits). A persistent retryable error retries every 300 s forever while `TurnLease` pins a session and an `agent_slot` | CONFIRMED, severity 3, bottleneck | default `max_span_secs` to a finite ceiling for unattended gateway/cron agents; leave interactive TUI infinite |
| **F4** | `tts_tool.rs::execute()` never reads `cfg.tts.enabled` (grep `enabled` in tts_tool.rs: 0 hits), yet doctor reports TTS "(disabled — set enabled = true)". The design preference forbids a dispatch guard, so the fix is wording only | CONFIRMED, severity 3, **preference-owned** | doctor/setup wording only — report "tts remains active on phonon unless blocked via tool policy" |
| **F7** | `operant-cli/src/config.rs` carries a SECOND live config system: `config.yaml` + `config.local.yaml` loaders (`:114`), with `delegation.*` and `security.redact_secrets` read live from main.rs. `AGENTS.md`'s "Runtime config is TOML-first and shared through operant-core" omits this entirely | CONFIRMED, severity 2, fragmentation | document the dual system, or converge the CLI YAML keys onto `AppConfig` |

### What each predecessor plan already closed — do not repeat

- iter-2669's plan: RT-542/544/545/547/552/527/554 all closed in 2654–2665.
- iter-2671's plan: T1b shipped as iter-2673/RT-612.
- Hydrogen review: F5 (memory divergence — doc-only), F12 (`context_historian`
  doctor warning, iter-2672), RT-612 (checkpoints store, iter-2673).
- The 42→15 clippy reduction (iter-2665), and the peer's F8/F9/F10/F11 doc
  drifts noted below.

### Pre-existing failures — do not chase them

- `audit-ledger-index.py`: 4 FAILs (RT-555 ×2, RT-553, RT-554) — peer-published
  rows, explicitly out of scope.
- `agent::tests::*` multi-filter race and the CJK truncation test — proven
  pre-existing at `d250e538`.
- Clippy gate at 15 warnings — all peer WIP in `surface_raster`/`mcp_oauth`.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 RT-275 | `database.rs`, `operant-config/src/schema/{platform_cfg,media}.rs`, `BUGS.md` | ~200 removed | none | low (public-API removal) |
| T2 RT-293 | `BUGS.md` | ~3 | none | none (stale checkbox) |
| T3 RT-285(2) | `session/mod.rs`, `agent/events.rs`, `BUGS.md` | ~60 | none | medium (durable delete) |
| T4 RT-209 | `recap.rs` or `BUGS.md` | ~40 | none | low |
| T5 RT-543 | `cmd_acp.rs`, docs, `CHANGELOG.md` | ~60 + docs | none | low (ruling-gated) |
| T6 RT-136/RT-179 | `BUGS.md` | ~10 | none | none |
| T7 F1 | `agent/fallback.rs` | ~40 | none | medium (breaker state) |
| T8 F2 | `agent/fallback.rs`, `agent/run.rs` | ~60 | none | medium (per-route clamp) |
| T9 F3 | `gateway_runner.rs` | ~10 | none | low (one override) |
| T10 F4 | `cmd_doctor/checks_tools.rs` | ~5 | none | none (wording) |
| T11 F7 | `operant-cli/src/config.rs`, `AGENTS.md`, `docs/library/config.md` | ~30 | none | low (doc) |
| T12 counts | `BUGS.md` | ~10 | none | none |

Twelve tranches. Zero new dependencies. Zero schema changes.

---

# T1 — RT-275: the four remaining callerless surfaces

`write_approval.rs` deleted iter-2665 (327 lines). RT-612 closed iter-2673,
which repointed `cmd_checkpoints.rs` at the tool's live store. What remains:

- `Database::set_tool_state` / `get_tool_state` / `clear_tool_state` /
  `clear_all_tool_states` (database.rs:2882–2930) — the live store is
  `tool_state.json` (`tools/tool_state.rs`), which `operant tools disable`
  writes. Zero non-test callers.
- `Database::store_checkpoint` (2675) / `get_checkpoint` (2724) — zero callers;
  the live store is the shadow-git manager.
- `operant-config`'s `ReliabilityConfig` / `MemoryPolicyConfig` /
  `MultimodalConfig` — named only in `docs/archive/` frozen plans, which are
  history, not live surface. Deleting them breaks no documented table.

**Keep** `list_checkpoints` and `delete_checkpoint` (database.rs:2696, 2744) —
they have callers and RT-612 already repointed the command. **Keep** the
`tools_state` (:716) and `checkpoints` (:619) tables — a live database must
keep opening; dropping a table is a migration.

**Test:** one guard per deleted name, built by concatenation so the test's own
source cannot satisfy it (the `turn_attribution_doc_claims.rs:858–870`
discipline). Mutation-prove by adding one back.

**Done:** four surfaces gone, `-p operant-core --lib` and `-p operant-config`
green, RT-275 closed.

---

# T2 — RT-293: flip the stale checkbox

Both halves shipped iter-2665 and are verified: `notify` says "Log a
notification intent" and returns `{"logged": true}` (the integration test
asserts the false key is gone); the `MemoryManager` field, builder,
`AgentCore` field, parameter threading and both call sites are removed, with
tombstone comments at `builders.rs:376` and `mod.rs:692`. The row's decision
was taken and shipped; the checkbox was never flipped.

**Files:** `docs/audit/2026-06-19-BUGS.md` only. One docs iteration.

---

# T3 — RT-285(2): `SessionStore::discard` loses data it claims to delete

`session/mod.rs:358` drops the resident copy (`self.warm().remove(key)`) and
nothing else, while its doc says *"Delete a session's transcript from memory
**and disk**. Destructive."* The sole production caller, `agent/events.rs:525`,
believes it. The ledger's account: switching away and back rehydrates the
"wiped" history.

The row offers two routes — durable delete, or a rename that makes the
in-memory scope explicit. **Take the durable one.** A rename leaves the caller
still believing it wiped a transcript, which is the defect.

Before writing it, check what the session's rows are:

```text
rg -n 'DELETE FROM messages|DELETE FROM sessions' crates/operant-core/src/database.rs
```

The delete must be transactional with the warm-map removal, and must answer
the destructive-op question with a `#[must_use]` or a typed result. The
existing negative-control test in `cron_session_isolation.rs` proves nothing
(it uses the memory-only `add_message` and re-selects the same id, which
`set_session_id` skips) — **replace it, do not keep it.**

**Test:** `discarding_a_session_destroys_the_transcript_on_disk` — insert via
the DB path, discard, construct a NEW store over the same database, acquire
the same key, assert the history is empty. Mutation-prove.

## DO NOT IMPLEMENT THIS TRANCHE WITHOUT AN OPERATOR RULING (recorded 2026-11-19)

**T1 is done by someone else** and this note re-scopes the rest of the plan.

`iter-2680` (peer, `d73063f8`) closed RT-275 itself, at the same time this
tranche was being built independently. It deleted the same four `Database`
tool_state/checkpoint methods, went further and also removed the three
`operant-config` types this plan said to KEEP (ReliabilityConfig,
MemoryPolicyConfig, MultimodalConfig), and landed a guard test
(`dead_db_surfaces_stay_dead.rs`) that does the same job as the one written
here. **T1 is CLOSED and must not be re-done.** The peer's wider deletion is
correct and this plan's instruction was wrong: `AppConfig` carries
`#[serde(default, deny_unknown_fields)]` (config.rs:44) and none of those
three tables are fields on it, so an operator naming `[reliability]` in
operant.toml already got a hard load error. The types protected nobody. The
"it is in a published schema" objection the earlier draft raised does not
survive that: an unreachable schema type whose table is rejected at load is
not a contract.

**T3 IS BLOCKED ON A RULING, and the plan's "take the durable one" is not
ours to take.** Q6 in `docs/kb/ASKS.md` (status `open`, asked 2026-11-19)
asks the operator exactly this question in (a): *should `/reset` destroy the
persisted transcript or only the in-memory view?* RT-285's own verdict says
"none is a wrong behaviour today" and "each needs a decision this campaign
must not make silently". Implementing durable discard is a destructive
datastore change decided without the person who owns the data, so it stops
here until Q6 is ruled. `SessionStore::discard` at session/mod.rs:358 and its
single production caller (`agent/events.rs:525`) are unchanged; RT-389 keeps
the doc honest in the meantime, so nothing is currently lying.

**T3's premise is half stale already.** The autocompactor third of RT-285 is
gone from the tree: `rg -n 'Autocompactor' crates/` returns nothing, and even
the word "autocompact" has no match in `crates/operant-cli/src`. Q6(b) asks
whether to mount or delete it, and the answer is now "it is already deleted".
That half of the row can close on measurement once (a) is ruled.

**Next unblocked tranche is T4 (RT-209).** Its own alternative route (re-scope
the row to "consumer delivered at the seam; event-carried value intentionally
unread") needs no operator input and no new behaviour, which makes it the
right next move rather than T3.

---

# T4 — RT-209: the attribution value has a seam and no subscriber

`TurnEnd::attribution` (`turn_end.rs:137`) is emitted; `log_attribution`
(`turn_end.rs:262`) is the only reader. `recap.rs:217` subscribes to the bus
and reads `end.session_id` — one field away from the consumer the row asks
for. The row's other candidate (`advisor`) does not exist in this tree; the
tranche says so rather than hunting for a file that is not there.

The governing guard is live: `turn_attribution_doc_claims.rs` asserts both
`BUGS.md` and `CHANGELOG.md` contain one of four phrasings naming the deferred
consumer (`"no subscriber reads"`, `"no SUBSCRIBER reads"`, `"no consumer
reads"`, `"every subscriber reads only"`). **If the subscriber lands, that
guard becomes a false claim and this tranche must update it in the same
commit** — its replacement must assert the opposite (a consumer reads it),
not merely be deleted.

**Alternative:** re-scope the row to "consumer delivered at the seam
(iter-1661); event-carried value intentionally unread", keep the doc-claim
guard with its phrasing updated.

---

# T5 — RT-543: the ACP daemon's disposition — DECISION GATE

The socket shipped (`gateway_runner.rs:6446`), `operant chat --attach` works,
and RT-542/545/547/552 closed against it. What remains is
`operant acp server --socket`'s disposition. The peer plan's A1b option
(deprecate with a loud startup warning naming `operant gateway run --socket`
as the replacement, keep stdio) is the adopted recommendation — it is
reversible, and editors speak ACP over stdio.

**Needs a ruling before any edit.** The row is `S` sized; the risk is an
operator running `operant acp server` today. The ruling says deprecate or
retire; either way `scripts/check-doc-claims.sh` re-derives the command count
(53 variants today).

---

# T6 — RT-136 and RT-179: the two stale checkboxes

- **RT-136** — confirmed-not-a-defect at iter-1176; both `[pk]` and
  `[socialization]` carry "Nothing reads this table (RT-136)" doc comments and
  are surfaced by `AppConfig::unused_setting_warnings` (config.rs:4014). One
  sentence recording the disposition: deliberately dark, surfaced, documented.
  The index script already exempts its duplicate detail sections.
- **RT-179** — the row's own tail reads "ALL FOUR REMAINING ITEMS ARE NOW
  DONE" and "This closes the RT-179 functional core"; five twins landed
  (`/effort`, `/plan`, `/context`, `/export`, `/memory`). Flip the checkbox
  and say so.

Doc-only. No ruling needed.

---

# T7 — F1: a breaker on the fallback route

`fallback.rs:8,239` — *"Fallback is per-request — every call starts again with
the primary."* During a sustained primary outage, every iteration of a ReAct
turn (cap 90) re-pays the primary's failure latency before reaching the
healthy alternate. The only cooldown (`arm_cooldown` at :340,383) arms the
NEXT provider's cooldown after a cross-provider auth failure — it is not a
breaker on the primary.

**Fix shape:** a per-route failure cooldown in `fallback.rs`, analogous to
`gateway_runner.rs`'s `dispatch_breaker.rs` on the send path — consecutive
primary failures route directly to the alternate until cooldown expiry.

**Test:** `a_dead_primary_is_skipped_until_cooldown_expires` — drive a client
whose primary fails every call and whose fallback succeeds; assert the second
call skips the primary (e.g. by counting primary attempts). Mutation-prove by
removing the cooldown check.

**Severity 3. Medium risk** — breaker state is cross-request mutable state and
must be keyed per route, not global.

---

# T8 — F2: fit the fallback route's request

`fallback.rs` dispatches an already-assembled request — no `max_tokens`,
`fit_output` or `context_window` reference exists (grep: 1 hit, a test).
`context-management.md` claims each route is re-fitted "before every request
build"; that is true for the primary and false for the fallback. The next
build reverts to the primary's denominator, so a fallback model with a
smaller context window overflows on the first fallback turn.

**Fix shape:** resolve each fallback candidate's (context_window, max_output)
inside `FallbackModelClient` and clamp `max_tokens` per route before dispatch.

**Test:** `a_fallback_route_is_fitted_with_its_own_window` — a fallback
candidate with a smaller window than the primary must produce a request whose
`max_tokens` fits the fallback's window, not the primary's. Mutation-prove.

**Severity 3. Medium risk** — touches the hot request-build path.

## T7 AND T8 ARE DONE -- by iter-2679 (peer, `47c8dc7a`), verified 2026-11-19

Neither tranche is to be re-done. The peer shipped both in one commit
(`feat(iter-2679): F1 circuit breaker + F2 per-candidate max_tokens clamp`,
468 insertions in `agent/fallback.rs`, 8 new tests) and the claims hold up
against the tree:

- **T7 (F1, the breaker) landed.** `struct PrimaryBreaker` at
  `fallback.rs:236`, with `failure_threshold` (default 3) and
  `cooldown_period` (default 60s) both configurable through
  `with_breaker_threshold` / `with_breaker_cooldown`, documented at :231-232 as
  "the primary fails N times in a row, it is skipped in favor of fallback
  models until cooldown elapses".
- **T8 (F2, per-candidate fit) landed, at all THREE call sites this plan named.**
  `clamp_max_tokens_to_model` is defined at `fallback.rs:363` and called at
  :484, :536 and :637 -- i.e. `try_models` (same-provider), and
  `try_next_provider_chat` + `try_next_provider_streaming` (cross-provider),
  which is exactly the coverage T8 specified. It is a no-op when the model has
  no known ceiling or no `max_tokens` is set, which is the degradation T8
  asked for.

So T7 and T8 are CLOSED and the plan records them as such. **T9 is the next
tranche in this family**, and it is not touched by iter-2679.

---

# T9 — F3: a finite retry ceiling for unattended runs

`RetrySettings::default().max_span_secs: 0` is INFINITE (`config.rs:1275`),
and the gateway constructs the agent without overriding it (grep
`max_span_secs` in `gateway_runner.rs`: 0 hits). A single persistent retryable
provider error retries every 300 s forever while `TurnLease` pins the session
and an `agent_slot` — unbounded head-of-line blocking per chat. The
`SessionStallTracker` only makes stalled turns VISIBLE; it does not fire
`InterruptFlag`.

**Fix shape:** default `max_span_secs` to a finite ceiling for unattended
gateway/cron agents (the `gateway_runner` construction site), leaving
interactive TUI infinite.

**Test:** `a_gateway_agent_has_a_finite_retry_span` — construct the agent the
way the gateway does and assert `max_span_secs > 0`; construct the TUI way and
assert it is infinite. Mutation-prove.

**Severity 3. Low risk** — one construction-site override.

## F3 IS DISPOSITIONED, AND ITS PREMISE WAS PARTLY FALSE. Read this before touching retry again.

Shipped as `iter-2680` (peer, `75284a07`): the gateway and cron hosts now derive
`gateway_behavior` from `app_config.agent` via a per-host `Cow` clone, setting
`retry.max_span_secs = 10 * large_span_secs` (default 3,000 s) ONLY when the
operator left it at `0`. An expressed ceiling is never lowered
(`gateway_runner/tests.rs:3420` pins it). That is a sound defensive change: it
bounds a span nobody can see, for the one host with nobody watching.

**But the finding it was written from was partly false, and that is recorded
here deliberately so the next reader does not repeat it. Measured
2026-11-19, at `75284a07` and re-proved empirically:**

1. **"retries every 300 s forever" — FALSE.** `TURN_MAX_RETRY_FAILURES` is
   **12** (`agent/turn_retry_state.rs:158`), consulted on BOTH retry paths
   (`agent/run.rs:1182` normal, `agent/run.rs:2184` streaming). A provider
   failing every time burns 12 attempts and the turn ends with a TYPED
   `Error::Provider` — not infinite, and not unpinned.
   **Empirical, not argued:** `cargo test -p operant-core --test
   retry_policy_wiring the_turn_cap_bounds_an_infinite_policy` at this commit
   passes in **12.37 s** against a client that fails every time
   (`FlakyClient::new(usize::MAX)`). The pre-existing test
   `retry_policy_wiring.rs` (`the_turn_cap_bounds_an_infinite_policy`)
   already pinned this; F3 was written without reading it.

2. **"0 means unbounded" — FALSE as a claim about the SYSTEM, true only for
   the one knob.** The `max_span_secs == 0` default is bounded by (1); only
   the config *value* is unset. The retry SPAN of a gateway turn was already
   finite at ~12 attempts.

**What was genuinely true:** the bound is an ATTEMPT COUNT, not a wall clock,
so at the default `large_span_secs = 300` a wedged turn holds its `TurnLease`
and `agent_slot` for up to ~1 hour with no preemptor — `SessionStallTracker`
only observes (`gateway/lifecycle.rs:8-12`). Reducing that ceiling to ~50
minutes is the real, defensible value of `iter-2680`, and it is worth noting
that the attempt-count bound still stands on top of it: the new ceiling caps
SPAN, not attempts.

**The lesson that matters for the "infinite default" debate:** the change
respects the documented preference (config.rs:1248-1250,
`operant.example.toml:327-329`) by applying only where the operator expressed
nothing, and never overwriting an expressed value. That is the correct way to
change an unattended seat's posture without contradicting the operator — and
it is the precedent to follow for any future "the default is wrong for
unattended runs" finding.

**The lesson that matters for reviewers:** this campaign refuted at the tree
four Hydrogen findings that each read a config default and inferred the
behaviour without following the consuming code path. The turn cap's own test
would have answered F3 in one grep. Before writing a fix, run the suite that
already covers the file.

---

# T10 — F4: doctor's TTS wording — PREFERENCE-OWNED

`tts_tool.rs::execute()` never reads `cfg.tts.enabled` (grep `enabled`:
0 hits), yet `cmd_doctor/checks_tools.rs:1295` reports TTS "(disabled — the
phonon model is downloaded; set `[tts] enabled = true` to use it)". The
design preference freezes this split ("Do not 'fix' a disabled TTS by looking
for a guard in the dispatch path — there is none to find"), so **the fix is
wording only**: doctor/setup report "tts remains active on phonon unless
blocked via tool policy".

**Test:** the doctor's TTS line does not claim `enabled = false` disables TTS.
Mutation-prove by restoring the false claim.

**Severity 3. None** — a string change.

---

# T11 — F7: the second config system

`operant-cli/src/config.rs` carries a live `config.yaml` + `config.local.yaml`
loader (`:114`) with `delegation.*` and `security.redact_secrets` read live
from main.rs. `AGENTS.md`'s "Runtime config is TOML-first and shared through
`crates/operant-core/src/config.rs`" is true but omits this entirely. It is
not three copies of one schema — it is two config SYSTEMS (AppConfig + a
parallel YAML loader feeding a handful of live keys).

**Fix shape (documentation):** correct `AGENTS.md` and `docs/library/config.md`
to describe the dual system and name which keys each owns. A convergence
(moving the YAML keys onto `AppConfig`) is a larger change with an
operator-visible migration and is explicitly NOT in this tranche.

**Test:** none — doc-only.

---

# T12 — the counts

Re-derive and write, once, after T1–T11:

```text
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md
python3 scripts/audit-ledger-index.py
bash scripts/check-doc-claims.sh
./scripts/genome lint
```

Expected open count after this plan: **2** (RT-280 and RT-190, both
owner-gated, both documented as not scheduled). Every other row carries its
receipt. Record the ledger-index failure set and show it is the SAME four
pre-existing FAILs (RT-555 ×2, RT-553, RT-554) and nothing new.

---

## Order and dependency

**T2 → T1 → T3 → T4 → T6 → T7 → T8 → T9 → T10 → T11 → T5 (ruling) → T12.**

T2 is a checkbox and a lookup. T1 is the deletion with a proven precedent.
T3 is the durable-delete fix. T4 is the subscriber. T6 is two checkboxes. T7
and T8 are the two `fallback.rs` findings and share a file — do them together
or T8 first (it is the narrower change). T9 is a one-line override. T10 is a
string. T11 is docs. T5 waits on a ruling. T12 is last.

## Explicitly NOT in scope

- **RT-280** and **RT-190** — owner rulings, not scheduled.
- **RT-555's ID collision**, **RT-553/RT-554's missing detail sections** — peer
  state, out of scope.
- **F5** (refuted — doc-only, already corrected), **F6** (tool-result
  truncation — a larger refactor of the evidence layer), **F8/F9** (doc-drift
  on OUTLINE.md's crate-graph claims, not a runtime defect), **F10**
  (`handle_admin_command` is test-only but harmless), **F11** (slack.rs's dead
  parser already removed; the AGENTS.md vs gateway.md doc reconciliation is a
  separate doc pass).
- The `agent::tests::*` multi-filter race and the CJK truncation test.
- Dropping the `tools_state` / `checkpoints` TABLES (T1 removes methods only).
- Re-auditing any closed row.

## Claim rule

Never take a number from a header. `git fetch origin`, read the top label, add
one. Three plans now exist for the same nine rows; this one owns execution
order because its ground truth includes iters 2672–2675.
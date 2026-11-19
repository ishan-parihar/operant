# What stands after iter-2666 — the last nine open ledger rows

## Relationship to the peer plan (read this first)

A parallel agent published
`docs/plans/2026-11-19-executing-the-remaining-implementation-gaps-post-2668.md`
(iter-2669, commit `c1901dfc`) while this one was being written. **It is stale at
write time on four of its items**, and this plan supersedes it on exactly those
four:

| peer's item | peer's plan | actually true at `c1901dfc` | authority |
|---|---|---|---|
| `write_approval::stage_write`/`list_pending`/... | "delete (never landed)" | **ALREADY DELETED** iter-2665 — `write_approval.rs` does not exist | `git show a931aaeb --stat` |
| RT-285 `TurnRetryState` restart signals | "wire or delete the three unused signals" | **ALREADY DELETED** iter-2665 | `turn_retry_state.rs:33-34` tombstone comment |
| RT-293 `MemoryManager` | "delete (write-only field)" | **ALREADY DELETED** iter-2665 | `builders.rs:376`, `mod.rs:692` tombstones |
| RT-293 `notify` `delivered: true` | "must stop regardless" | **ALREADY STOPPED** iter-2665 | `notification_tool.rs:57-59`, returns `{"logged": true}` |

Their Tranche A0 (flip RT-179 + RT-136) and their A1b (RT-543 deprecate-not-retire)
are both correct and **compatible with this plan** — T7 here covers the same two
rows, and T6's ruling set adopts their "deprecate, keep stdio" option over my
"retire" default. Their Tranche C (RT-190) is this plan's T4-adjacent row 9 and
both plans agree it is owner-gated and last.

The genome's one-owner-per-question rule (law 1) says two documents may not
answer the same thing. **This plan owns the execution order for the nine rows**
because it is the one whose ground truth includes iter-2665's shipped code; the
peer's plan remains the record of the *decision table* the owner fills in for
RT-275/285/280, and this plan does not re-derive that table. Where they conflict,
this file wins on **what is already done** and theirs wins on **what the owner
must rule**. Neither file is deleted — both are dated-frozen history the moment
they land.

---

Successor to `2026-11-19-remaining-implementation-gaps-the-open-ledger-rows-and-the-scale-socket-cluster.md`
(ten tranches, closed out at iter-2664/2665). That plan's own closing test was
"the ledger's open set is honest (every open row names its receipt or its
ruling)". It is **not met**: nine rows are still open, and of those nine only
four have had any code written against them.

This plan is the tail of that work. Nine open rows, nine tranches, each one
independently revertable.

## The measured landscape at `7a20660f`

```text
git fetch origin
grep -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   # 9
grep -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   # 650
python3 scripts/audit-ledger-index.py              # 4 FAILs, all pre-existing
```

The nine, in the order this plan takes them:

| # | row | what is actually true now | what is left |
|---|---|---|---|
| 1 | **RT-275** | `write_approval.rs` DELETED iter-2665. Three surfaces remain. | `Database::{set,get,clear}_tool_state` + `clear_all_tool_states` — **verified zero non-test callers**; `Database::{store,get}_checkpoint` — same; and `operant-config`'s `ReliabilityConfig` / `MemoryPolicyConfig` / `MultimodalConfig`. **Plus a NEW finding this plan's verification turned up:** `list_checkpoints`/`delete_checkpoint` DO have callers (`cmd_checkpoints.rs`) but read a `checkpoints` table `store_checkpoint` never fills, so `operant checkpoints list/prune` silently do nothing → T1b |
| 2 | **RT-293** | CLOSED in substance iter-2665 (`notify` reports `{"logged": true}`, `MemoryManager` field removed). **The checkbox was never flipped.** | flip the checkbox, and settle where the live MEMORY.md/USER.md import now runs |
| 3 | **RT-285** | Restart signals DELETED iter-2665. | `SessionStore::discard` still only drops the resident copy while its doc and its caller both claim a durable wipe |
| 4 | **RT-280** | untouched. `suspended` / `resume_pending` read on every access, written by nothing. | a ruling: remove, or wire |
| 5 | **RT-209** | untouched. `TurnEnd::attribution` is emitted; `log_attribution` is the only reader. | one subscriber, or a ruled re-scope |
| 6 | **RT-543** | the socket SHIPPED (`run_session_socket`, `chat --attach`, RT-542/545/547/552 all closed). | the ACP disposition the row asks for: retire it, or keep it as a mode |
| 7 | **RT-136** | confirmed-not-a-defect iter-1176, still `- [ ]` | one disposition sentence |
| 8 | **RT-179** | advancing feature; five twins landed | one sentence saying this plan does not advance it |
| 9 | **RT-190** | memory-bank topology | one sentence: owner ruling, not scheduled |

Three of the nine are paperwork (2, 7, 8). Four are code (1, 3, 5, 6). Two are
rulings (4, 9). That split is the reason this plan is short.

## What already landed (do not repeat)

- RT-542, RT-544, RT-545, RT-547, RT-552, RT-527, RT-554, RT-605, RT-610 —
  closed in iters 2654-2665, each with its receipt in its own row.
- The whole scale/socket cluster the predecessor plan named: the session
  socket, `chat --attach`, the wire round-trips for cancel/steer/queue, and
  `slot_count_for`. Do not re-pin these.
- The 42→15 clippy-warning reduction (iter-2665).

## Gates / success criteria

Every tranche ends green on:

```bash
source scripts/dev-env.sh
cargo fmt --all
CARGO_TARGET_DIR=/home/ishanp/wt2650-target MEMORY_WIRE_NO_SYNC=1 \
  ./scripts/check.sh check -p operant-core --lib
CARGO_TARGET_DIR=/home/ishanp/wt2650-target MEMORY_WIRE_NO_SYNC=1 \
  ./scripts/check.sh test -p operant-core --lib --offline -- <filter>
bash scripts/clippy-warning-gate.sh
bash scripts/check-doc-claims.sh
python3 scripts/audit-ledger-index.py
```

**Done means:** `grep -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md` prints a
number this plan accounts for row by row, and every row that closes names the
commit that closed it. Rows that stay open carry a ruling, not a deferral.

### The pre-existing failures — do not chase them

`python3 scripts/audit-ledger-index.py` fails on four counts **at the base** and
will keep failing after this plan unless the ID collision is ruled on:

1. `RT-555` index row not CLOSED but closed in the checkboxes.
2. `RT-555` used by TWO index rows and TWO detail sections (RG-021, two agents
   on the shared branch).
3. `RT-553`, `RT-554` — index rows with no detail section.

The predecessor plan forbade repairing these by any means other than the earned
row closes. This plan repeats that: **the later publisher renumbers forward and
records an ID NOTE**, and no tranche here may bulk-re-mark or touch
`.ci/clippy-allowlist.txt`. Record the failure set per tranche in the commit
body and show it did not GROW.

The `agent::tests::*` multi-filter race and
`cmd_sessions::truncation_tests::a_cjk_session_title_does_not_panic_when_truncated`
are also pre-existing (proven at `d250e538`, iter-2665 body). Do not re-audit.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1a RT-275 | `database.rs`, `operant-config/src/schema/{platform_cfg,media}.rs`, `BUGS.md` | ~200 removed | none | low (public-API removal) |
| T1b NEW | `cmd_checkpoints.rs`, `BUGS.md` (new row) | ~60 | none | low (routing) |
| T2 RT-293 | `BUGS.md`, `CHANGELOG.md` | ~10 | none | none (paperwork) |
| T3 RT-285(2) | `session/mod.rs`, `agent/events.rs`, `BUGS.md` | ~60 | none | medium (durable delete) |
| T4 RT-280 | `gateway_session.rs`, `BUGS.md`, `CHANGELOG.md` | per ruling | none | medium (ruling-gated) |
| T5 RT-209 | `advisor`/`recap` or `BUGS.md` | ~40 | none | low |
| T6 RT-543 | `cmd_acp.rs`, `session-server.md`, `cli-surface.md`, `AGENTS.md`, `CHANGELOG.md` | ~60 + docs | none | low (ruling-gated) |
| T7 records | `BUGS.md` | ~10 | none | none |
| T8 counts | `BUGS.md` | ~15 | none | none |

Eight tranches, zero new dependencies, no schema changes.

---

# T1 — RT-275: the four surfaces with no caller (iter-A)

## What the verification actually found — a defect the ROW does not name

I checked each checkpoint method for callers before writing this plan, and the
ledger's account of this surface is **wrong in a way that changes the tranche**.
The row says `Database::{store,get}_checkpoint` has no caller. That is true. It
also lists `Database::set_tool_state` / `get_tool_state` as "a shadow with no
writer" — true. But the row does not say that two of the four checkpoint methods
**do have production callers and read a table nothing ever writes**:

```text
rg -n 'INSERT INTO checkpoints' --type rust crates/ | grep -v database.rs   -> (empty)
```

`store_checkpoint` has no caller anywhere, so the `checkpoints` table is created
by the migration and **never written to**. Yet `crates/operant-cli/src/cmd_checkpoints.rs`
opens the `Database` and calls `db.list_checkpoints(&dir)` (lines 99, 121, 164)
and `db.delete_checkpoint(&cp.hash)` (179, 202). The live checkpoint store is the
filesystem shadow-git manager (`tools/checkpoint_tool.rs`, which has its own
`list_checkpoints` at :443 and a prune at :173).

So `operant checkpoints list` **always prints nothing** and
`operant checkpoints prune` **always prunes nothing** — while the filesystem
store the checkpoint TOOL fills grows beside it, invisible to the operator. That
is a false claim on a user-facing command, not dead code, and it is a NEW finding.

**This tranche therefore has two halves and they must not be merged:**

- **T1a (this row):** delete the two callerless methods — `store_checkpoint`
  (2675) and `get_checkpoint` (2724). Leave `list_checkpoints` (2696) and
  `delete_checkpoint` (2744) alone; they have callers, and deleting them here
  would break `cmd_checkpoints.rs` mid-tranche.
- **T1b (new row, RT-NEW):** point `cmd_checkpoints.rs` at
  `checkpoint_tool`'s store, or delete the command. That is its own row and its
  own iteration, because it changes what an operator sees.

**The tables stay**, for both halves: `checkpoints` (database.rs:619) and
`tools_state` (:716) are created by the migration, and a database that already
carries them must keep opening. Dropping a table is a migration and this tranche
is not one.

The row's stated reason for staying open was *"each of the six needs a decision
this campaign must not make silently — wire it, or make it private."* Half that
decision was already taken: iter-2665 deleted `write_approval.rs` outright
rather than parking it, and the ledger row was amended to PARTLY CLOSED in the
same commit. **That is the precedent, and it is the one this tranche follows.**
A surface with zero non-test callers and a live replacement is deleted, not
parked — a `#[doc(hidden)]` note is a promise that someone will wire it later,
and nothing in the tree is waiting to.

Verified at `7a20660f`, before writing a line:

```text
rg -n '\.(set_tool_state|get_tool_state|clear_tool_state|clear_all_tool_states)\(' --type rust \
  | grep -v 'database.rs'      ->  (empty)
rg -n '\.(store_checkpoint|get_checkpoint)\(' --type rust \
  | grep -v 'database.rs' | grep -v checkpoint_tool.rs   ->  (empty)
```

The live replacements are both named in the tree: tool state is
`tools/tool_state.rs` (`tool_state.json`, which `operant tools disable` writes),
and checkpoints are `tools/checkpoint_tool.rs` (the filesystem shadow-git
manager). The DB halves are a shadow with no writer.

**The tables stay.** `tools_state` (database.rs:716) and `checkpoints`
(:619) are created by the migration and dropped by nothing else; removing the
methods must NOT remove the tables, because a database that already carries
them must keep opening. Deleting a column from a live schema is a migration,
and this tranche is not one.

## Files (T1a only)

- `crates/operant-core/src/database.rs` — delete the four tool-state methods
  (`set_tool_state:2882`, `get_tool_state:2896`, `clear_tool_state:2909`,
  `clear_all_tool_states`) and the two callerless checkpoint methods
  (`store_checkpoint:2675`, `get_checkpoint:2724`). **Keep** `list_checkpoints`
  and `delete_checkpoint` — they have callers today and T1b will need them
  reachable to be repointed.
- `crates/operant-config/src/schema/platform_cfg.rs` — `ReliabilityConfig`,
  `MemoryPolicyConfig`. `crates/operant-config/src/schema/media.rs` —
  `MultimodalConfig`.
- `docs/audit/2026-06-19-BUGS.md` — amend RT-275 to CLOSED-with-a-new-row.

**The `operant-config` half is the one that needs care, and the row's own
objection to deleting it is correct**: *"Deleting a public type from
`operant-config` would fail any `operant.toml` that names its table."* Since
`AppConfig` uses `deny_unknown_fields`, an operator with `[reliability]` in
their config gets a HARD load error today — so the types are not protecting
anyone. I checked: the only mentions of `[reliability]` / `[memory_policy]` /
`[multimodal]` anywhere outside the crates are in `docs/archive/` **dated-frozen
plans**, which are history, not live surface. **Default action: delete, with a
CHANGELOG note** per the iter-2610 precedent.

## Test

One guard, in `crates/operant-core/tests/`, that fails if any deleted name comes
back with no caller — the repo's own shape (the RT-209 doc-claim test's
discipline, `turn_attribution_doc_claims.rs:858-870`):

```text
for name in [set_tool_state, get_tool_state, clear_tool_state,
             clear_all_tool_states, store_checkpoint, get_checkpoint]:
    assert no `pub fn <name>` outside its own test module
```

The needle must be built by concatenation so the test's own source cannot
satisfy it. Mutation-prove by adding one back.

**And one guard for the NEW finding**, because it is the half that matters:
`cmd_checkpoints.rs` must not call `Database::list_checkpoints`. It is a source
contract, it is what makes T1b's routing change impossible to silently undo,
and it fails today — which is the point. Write it in T1b's iteration, or in this
one if the row for it is opened here.

## Done means

Four surfaces gone, `cargo check -p operant-core --lib` and
`-p operant-config` green, the guard mutation-proven (add one back → it fails),
row PARTLY→CLOSED with the deletion named.

---

# T2 — RT-293: flip the checkbox, then find out where the import went (iter-B)

Paperwork, plus one question the deletion in iter-2665 left behind.

`notify` now says "Log a notification intent" and returns `{"logged": true}`,
and the integration test asserts the false key is GONE. The `MemoryManager`
field, builder, `AgentCore` field and both call sites were removed. Both halves
of the row's decision were taken and shipped.

**The question:** iter-2665 kept the *construction* in `load_memory_manager`
because the one-time MEMORY.md/USER.md import runs inside its factory. That
means the import's only home is a factory whose result is immediately dropped.
Verify that and record the answer in the row:

```text
rg -n 'load_memory_manager' crates/operant-cli/src/    # how many callers, and do they use the return?
```

If the import genuinely still fires, the row closes with a sentence naming where
it lives. If it does NOT fire any more (because the caller uses the returned
handle nowhere and the compiler elided nothing but the SIDE EFFECT is what
matters), that is a real regression iter-2665 introduced and **this tranche
fixes it first** — the import moves to where it runs unconditionally.

## Files

`docs/audit/2026-06-19-BUGS.md` (flip `- [ ]` → `- [x]`, name iter-2665 and
this tranche's verification), `docs/CHANGELOG.md` if the import moved.

## Done means

`grep -c '^- \[ \]'` drops by one and the row names what iter-2665 shipped.

---

# T3 — RT-285(2): `SessionStore::discard` loses data it claims to delete (iter-C)

The one remaining live defect in the three-surface row, and the only one on this
list where the code and its own documentation disagree about something
destructive.

`crates/operant-core/src/session/mod.rs:358` — `discard` drops the resident copy
(`self.warm().remove(key)`) and nothing else, while its doc says *"Delete a
session's transcript from memory **and disk**. Destructive, and named as such.
This is the only operation here that loses data."* The sole production caller,
`agent/events.rs:525`, believes it ("without this the very next acquire would
rehydrate the history") — and the ledger says the opposite happens: switching
away and back rehydrates the "wiped" history.

## The decision

The row offers two routes: durable delete, or a rename that makes the in-memory
scope explicit. **Take the durable one.** A rename would leave the caller in
`agent/events.rs` still believing it wiped a transcript, which is the defect;
and `/reset` not destroying what it says it destroys is not a naming problem.

Before writing it, check what the session's rows actually are:

```text
rg -n 'DELETE FROM messages|DELETE FROM sessions' crates/operant-core/src/database.rs
```

The delete must be transactional with the warm-map removal, and it must answer
the destructive-op question the row raises: a `#[must_use]` on the bool (or a
typed result) so a caller cannot silently drop a failed wipe. The existing
negative-control test in `cron_session_isolation.rs` proves nothing (it uses the
memory-only `add_message` and re-selects the same id, which `set_session_id`
skips) — **replace it, do not keep it as false comfort.**

## Test

`discarding_a_session_destroys_the_transcript_on_disk`: insert messages through
the DB path (not `add_message`), discard, then construct a NEW store over the
same database and acquire the same key. The history must be empty. Mutation-
prove: the in-memory-only body back → the new store rehydrates and it fails.

## Done means

`discard` matches its doc, the caller's comment is true, and the replaced
negative-control no longer asserts something vacuous.

---

# T4 — RT-280: `suspended` / `resume_pending` — DECISION GATE (iter-D)

Two persisted flags read on every session access by `get_or_create_session`,
written by three methods that have no production caller. The row's own words:
*"wiring it is a product decision, not a patch."*

**Recommendation: REMOVE.** Both fields, the three writers
(`suspend_session:1174`, `mark_resume_pending:1197`,
`suspend_recently_active:1914`), the read branches in `get_or_create_session`,
and the unreachable `suspended` branch in `prune_old_entries` — with a CHANGELOG
entry, per the same precedent as T1. Reasons: nothing can turn them on, the
`/stop` arm that would set `suspended` writes an in-memory breadcrumb and no
I/O, and the row records that the writers *drop the SQLite error*
(`let _ = self.save_entry(&entry)`) while sibling writers in the same struct
propagate it — so wiring them as-is would import a silent failure where the map
says "suspended" and the database says otherwise.

**The alternative, if the owner wants it wired:** the row names the missing
input — *"how recently active a session must be to be resumed"* — so the ruling
must include that number, and the error-dropping writers must be fixed in the
same tranche. Do not wire it half.

Either way the row's honest half is already done (the field docs say no
production path sets them). **Do not start without the ruling.**

---

# T5 — RT-209: the attribution value has a seam and no subscriber (iter-E)

`TurnEnd::attribution` (`turn_end.rs:137`) is populated and emitted;
`TurnEndBus::log_attribution` (`turn_end.rs:262`) is the only reader and it
renders one `debug!` line. `recap.rs:217` subscribes to the bus and reads
`end.session_id` — so a subscriber already exists that is one field away.

**Recommendation: subscribe in `recap.rs`.** It is the one host that already
has the receiver in hand; the cost is reading a field it currently ignores.
`advisor` was the row's other candidate, but `tui/app/advisor.rs` does not exist
in this tree (the advisor is `advisor_card_tests.rs` and the card's real home
elsewhere) — so the row's recommendation is stale on that half, and this
tranche says so rather than hunting for a file that is not there.

The guard that governs this row is live and must keep passing:
`crates/operant-core/tests/turn_attribution_doc_claims.rs` —
`the_docs_still_name_the_deferred_consumer` accepts exactly four phrasings
(`"no subscriber reads"`, `"no SUBSCRIBER reads"`, `"no consumer reads"`,
`"every subscriber reads only"`) and asserts BOTH `BUGS.md` and `CHANGELOG.md`
contain one. **If the subscriber lands, that guard becomes a false claim and
this tranche must update it in the same commit** — and its replacement must
assert the opposite (that a consumer reads it), not merely be deleted.

**Alternative, if the owner prefers no consumer:** re-scope the row to
"consumer delivered at the seam (iter-1661); event-carried value intentionally
unread", and keep the doc-claim guard as-is with its phrasing updated.

## Test

The guard's mutation proof, in its new direction: remove the subscriber → the
docs must not be allowed to claim a consumer exists.

---

# T6 — RT-543: the ACP daemon's disposition — DECISION GATE (iter-F)

The row says *"One server: the gateway serves the client socket, and the ACP
daemon becomes a mode of it (or is retired in favour of the socket)."* **The
first half shipped.** `run_session_socket` is called from
`gateway_runner.rs:6446`, `operant chat --attach` works, and the four scale rows
closed against it. What remains is the disposition of the second daemon:
`operant acp server --socket` still owns its own agent, registry, MCP children
and AFT bridge.

**Recommendation: deprecate, do not retire** — adopting the peer plan's A1b
option, which is better than my first draft's "retire". Keep `operant acp
attach` (the thin-client entry point, RT-547) and keep `operant acp server`
**working**, but add a loud startup warning naming `operant gateway run
--socket` as the replacement, plus a doc line saying it is deprecated. Reasons:
editors speak ACP over **stdio**, and removing the socket half breaks an editor
integration for no gain — the gateway's socket already serves the same clients
better (0600 gate, same slots, same child set, plus channels). A deprecation is
reversible; a deletion is not.

**This needs a ruling before any edit.** The row is `S` sized, so the code is
small; the risk is that an operator is running `operant acp server` today. The
ruling must say which of these it is:

- (a) retire — delete `cmd_acp.rs`'s `server` arm, update
  `docs/library/session-server.md` and `cli-surface.md`, and record the removal
  in `AGENTS.md`'s command list and the CHANGELOG;
- (b) keep as an explicit compat mode — then it needs a doc line saying it is
  deprecated and what replaces it.

Either way `AGENTS.md`'s "53 command variants" prose is re-derived by
`scripts/check-doc-claims.sh`, which is the authority.

---

# T7 — the three record rows (iter-G)

Doc-only, no code, no ruling needed.

- **RT-179** — the channel-twin gap is an advancing feature. **The peer plan's
  A0 is right and this plan adopts its evidence:** the row's own tail already reads
  *"ALL FOUR REMAINING ITEMS ARE NOW DONE"* (five twins landed — `/effort`,
  `/plan`, `/context`, `/export`, `/memory`, the last closing iter-1392), so this row
  is a stale checkbox, not an advancing feature. Flip it, and say so.
- **RT-136** — confirmed-not-a-defect at iter-1176; both `[pk]` and
  `[socialization]` carry an explicit "Nothing reads this table (RT-136)" doc
  comment on purpose and are surfaced by `AppConfig::unused_setting_warnings`.
  One sentence recording the disposition: deliberately dark, surfaced, documented.
  Note the index-script already exempts its duplicate detail sections
  (`audit-ledger-index.py:76`) so the gate will not newly fail.
- **RT-190** — memory-bank topology; an owner ruling on which store is
  canonical. One sentence: not scheduled.

---

# T8 — the counts (iter-H)

Re-derive and write, once, after T1-T7:

```text
grep -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md
grep -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md
python3 scripts/audit-ledger-index.py
bash scripts/check-doc-claims.sh
./scripts/genome lint
```

Expected: open drops from 9 to the number of rows this plan ruled-deferred
(RT-136, RT-179, RT-190, and RT-280/RT-543 only if their rulings said park).
Record the ledger-index failure set and show it is the SAME four pre-existing
FAILs (RT-555 ×2, RT-553, RT-554) and nothing new — a new FAIL is a defect this
tranche caused and must fix before pushing.

**Index flip:** every row this plan closes gets its State cell updated in its
own tranche, each with its receipt.

---

## Order and dependency

**T2 (paperwork, zero risk) → T1 → T3 → T5 → {T4, T6 rulings in parallel} → T7 → T8.**

T2 first because it is a checkbox and a lookup and it costs one iteration.
T1 and T3 are the two code tranches and are independent of each other.
T5 depends on nothing. T4 and T6 are rulings and can be requested at any time;
neither blocks the others. T7 last-but-one so T8's count is written once.

## Explicitly NOT in scope

- **RT-555's ID collision** — RG-021, the later publisher's call. Not repaired
  here, not renumbered.
- **RT-553 / RT-554's missing detail sections** — peer-published rows; adding
  sections is a different tranche and would collide with their owners.
- The `agent::tests::*` multi-filter race and the CJK truncation failure —
  proven pre-existing at `d250e538`.
- Any edit to `docs/kb/feed.md`, `research/`, `.serena/`, or a peer's files
  (`surface_raster.rs`, `mcp_oauth.rs` and the rest of the in-flight set).
- Dropping the `tools_state` / `checkpoints` TABLES (T1 removes methods only —
  a live database must keep opening).
- Re-auditing any closed row.

## Claim rule

Never take a number from a header. `git fetch origin`, read the top label, add
one. iter-2654 was duplicated by exactly this mistake; T2's paper-only shape
makes it tempting, and it is still wrong.
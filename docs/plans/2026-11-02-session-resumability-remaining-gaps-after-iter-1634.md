# Session resumability — remaining gaps after iter-1634

Dated-frozen: published `2026-11-02`. Registered in `genome.toml`
`core.living_plans`.

## What this is

An **outline** of what remains open from
`docs/plans/plan-2026-11-02-session-resumability-and-gateway-session-navigation.md`
after iteration 1 shipped as `feat(iter-1634)` (`21cf744d`, pushed and
release-deployed, verified on `origin/main`).

It is a gap inventory, not a replacement plan. The parent plan stays the
authority for *why* each gap matters; this document records *what is still
true in the tree* and orders it.

Every claim below was re-verified against the working tree and `origin/main`
today, not recalled. Where a claim says "not present", the verification
command is named so the next agent can re-run it rather than trust this file.

---

## Where iteration 1 actually landed, and where it did not

Iteration 1 shipped more than the plan's own "~15 CLI lines" estimate
predicted, and less than its five numbered items. Splitting it honestly:

| Parent-plan iter-1 item | State | Evidence |
|---|---|---|
| 1. `SessionInterrupted` marker, written at exit | **DONE** | `session_interrupted.rs` (249 lines), written from `run_exit_teardown` in `tui/app/mod.rs` |
| 2. PID-liveness companion in `active_sessions.rs` | **MECHANISM ONLY — DEAD CODE** | 263 lines present (`register_presence`, `unregister_presence`, `prune_dead_presence`, `session_is_live`, `presence_session_ids_for_pid`) but **zero production call sites**. Verified: `grep -rn "register_presence\|unregister_presence\|prune_dead_presence\|session_is_live" --include=*.rs crates/ \| grep -v active_sessions.rs` returns empty |
| 3. `RowStatus::Interrupted`, cleared on next turn | **DONE** | `listing.rs` variant + `api_label`, `database.rs` derives it at loader index 11, `agent/turn_context.rs` clears it after `save_session` |
| 4. Startup notice, not auto-attach | **DONE for TUI, ABSENT for gateway** | `tui_app.rs` consumes the marker and sets a status notice. The gateway has **no resume notice of any kind** — verified: `grep -rn "consume_marker" crates/operant-cli/src/gateway_runner.rs cmd_gateway.rs` returns empty |
| 5. `sessions resume --last` + `--interrupted` filter | **NOT STARTED** | `cmd_sessions.rs` has no `--last` flag; `resolve_session(None)` already defaults to most-recent but the subcommand still takes a required `&str`. No CLI surface reads `interrupted_session_ids` or `INTERRUPTED_END_REASON` outside the DB layer |

So the honest scorecard is: items 1 and 3 complete, item 4 half-complete
(TUI only), item 2 shipped unwired, item 5 untouched.

Two deviations from the parent plan were forced and are documented in-code at
iter-1634:

- **`libc` is not an `operant-core` dependency**, so `process_is_running` uses
  `fs::metadata` on `/proc/<pid>` rather than `kill(pid, 0)`.
- **The teardown DB write is a plain blocking call, not `block_in_place`**,
  which panics on the current-thread runtime that `tui debug simulate` and the
  tests use.

Gap 6 (`ORDER BY s.ended_at DESC` sorting NULLs last) was carried into
iter-1634 as collateral and **is DONE**: all four loaders now
`COALESCE(s.ended_at, s.started_at) DESC`. That fix exposed a second latent
bug in the same block — `DatabaseSession::updated_at` is a plain `String`
while `ended_at` is NULL for every running session, so `row.get(4)` took the
whole query down on the most common row in the table. Fixed at all four sites.

---

## The remaining gaps, ordered

### P0 — the presence registry is dead code (iter-1 item 2, finished)

**This is the highest-value remaining item and the smallest.** 263 lines of
registry exist and nothing calls them. Until something does, operant cannot
distinguish "closed at Ctrl+C" from "crashed mid-turn" — which is the exact
question the parent plan's Gap 3 exists to answer, and the question the
operator originally asked.

What it needs:

1. The TUI registers its session at startup (`register_presence`) and
   unregisters on the clean exit path that already clears the marker
   (the `/exit` `/quit` arm in `commands.rs`) and in `run_exit_teardown`.
2. The gateway registers whatever session it is currently streaming into.
3. `prune_dead_presence()` is called once at boot, so stale records do not
   accumulate forever across SIGKILLs.
4. A consumer: the resume notice and/or `RowStatus::Interrupted` should read
   `session_is_live` and say something *different* for a session whose
   process is still alive than for one that is dead. Right now both are
   labelled identically, which is the confusing half of the feature.

**Test that fails without the fix:** a session registered by a live pid reads
as live and is not pruned; one registered by a dead pid is pruned and reads as
not-live. Both already exist as unit tests — what is missing is the wiring
test, i.e. "after the TUI exits cleanly the session is not live".

**Cost:** ~30 lines of call sites + one test. No new files, no new dependency.

### P1 — the gateway has no resume notice (iter-1 item 4, finished)

The parent plan's Gap 1 is defined for the gateway as well as the TUI: "In the
gateway the equivalent is `/new`+`/resume` by opaque id." Iter-1634 gave the
TUI a notice and the gateway nothing, so a telegram/discord operator still
has no path from "I was interrupted" to "resume that".

Depends on P0 (it should not offer to resume a session that is still live in
another process — that is exactly the race the presence registry prevents).

**Test:** the gateway's boot path consumes the marker once and emits a reply
or log line naming the session; a second boot does not repeat it.

### P1 — `operant sessions resume --last` and `--interrupted` (iter-1 item 5)

`resolve_session(None)` already resolves most-recent correctly, so this is
mostly CLI plumbing:

- `cmd_resume` takes an `Option<&str>` so `operant sessions resume` with no id
  works.
- `--last` reads the marker directly and names the interrupted session, so the
  operator sees *why* that session is the one being resumed rather than
  assuming it is arbitrary.
- `sessions list --interrupted` reads `Database::interrupted_session_ids()`
  (already shipped and tested, currently read by nothing).

The parent plan's own gate applies: `command_sweep.py` must stay at its
current count — these are subcommand flags, not new TUI commands.

### P2 — Gap 4: gateway `/sessions` becomes a browser (iter-2)

Measured today and still true: `gateway_commands.rs:1237` lists only
*bindings* — session id, platform user, channel — with no title, message
count, or recency. A telegram user cannot choose from that.

The parent plan already corrected itself on platform reach and that
correction still holds: **Slack is outbound-only** (`gateway/slack.rs:13-27`,
`start()` logs and returns, no Socket Mode WebSocket), so the only platforms
where a user can type `/sessions` are **Telegram and Discord**. Any plan that
claims Slack must not.

The shape is one join: `list_sessions_rich` (already NULL-safe after Gap 6)
against the `gateway_sessions` binding table, rendered with a 1-based index, a
title, relative recency and a message count, capped at telegram-safe length
with an "older sessions omitted" tail. `/sessions <n>` pins through the
existing honesty gate rather than a new one.

**Depends on:** nothing, can run any time. **Is independent of P0/P1.**

### P2 — Gap 2's hub half: the interrupted badge and cursor placement (iter-3)

`RowStatus::Interrupted` renders as `⚠` (shipped in iter-1634) but the hub
does not otherwise know about it:

- `SessionHub::open()` still hard-resets `selected = 0`
  (`session_hub/mod.rs:229-232`), so a launched-with-marker hub does not land
  the cursor on the interrupted row.
- No sort rule ranks interrupted above idle at equal recency.

One parameter on `open()` plus a comparator. The TUI notice already names the
session, so this is the *keyboard* path to the same target rather than a new
behaviour.

### P3 — Gap 5's remainder and iter-5 docs

`RowStatus` now has four variants, so Gap 5 itself is closed. What remains is
documentation, and the genome's one-owner rule makes it mechanical:

- `docs/library/sessions.md` owns the marker's semantics and the presence
  registry's semantics. It currently mentions neither — verified:
  `grep -i "interrupted\|marker" docs/library/sessions.md` returns empty.
- `docs/library/gateway.md` owns `/sessions` browse vs `/resume` pin.
- `operant.example.toml`'s `[gateway]` section needs no key change (the plan
  is explicit: no new config surface), but its honesty line about what
  `/sessions` does needs updating once iter-2 lands.
- `docs/CHANGELOG.md` has no iter-1634 entry — the CHANGELOG is append-only
  per release, so this belongs with the next release section rather than as a
  standalone patch.
- `genome handoff --write` after the push.

---

## Operational gaps found while verifying (not in the parent plan)

These are not session-resumability features. They are vices in the process
that made iter-1634 expensive, and each one is worth its own iteration.

### O1 — `/usr/local/bin/operant` has been deleted

It was root-owned and stale at an October build earlier today; it is now
**absent entirely**
(`ls /usr/local/bin/operant` → `No such file or directory`). Something in a
peer's sweep removed it.

Consequence: the AGENTS.md deploy protocol says install to *both* paths, and
only one now exists. `~/.local/bin/operant` is on PATH and the gateway unit's
`ExecStart` resolves to it, so nothing is functionally broken — but any
operator or script reaching for the conventional path gets
`command not found`.

Needs a decision from the operator, not an agent: restore it as a root-owned
copy (requires `sudo`, which I cannot supply non-interactively), or amend
AGENTS.md to declare `~/.local/bin` the single install path.

### O2 — uncommitted work in the shared tree is not safe

Iter-1634's working copy was **destroyed twice** mid-session by concurrent
agents' `git stash push -u` on the shared tree. I recovered it both times from
the stash's own tree, but the mechanism was invisible to me until it happened.

Separately, **edits repeatedly vanished from the working tree from the very
start of the session**, before any peer interacted. I initially attributed
some of those to `cargo fmt` reflowing code. That explanation is wrong —
rustfmt does not delete struct field initializers or `match` arms, and the
disappearances included exactly those. **The cause is unidentified.**

The protocol's answer exists but is not being used: long-running legs push
WIP checkpoints to `origin/<label>-wip`. Nothing in this session did. That is
a behavioural fix, not a code fix.

### O3 — the CLI test suite is red for reasons unrelated to any single agent

Two failures appeared during iter-1634. I proved the mermaid one
(`render_side_panel_markdown_without_native_protocol_shows_mermaid_source`)
pre-existing by reproducing it identically on a pristine `origin/main`
worktree with zero of my changes. The rewind one belonged to a peer's
in-flight test and was swept up in their own commits.

Neither was fixed, per the "don't fix a peer's subsystem mid-flight" rule.
The consequence is that `cargo test -p operant-cli --bin operant` is currently
not green, so it cannot be used as a regression signal until someone
triages the failure list and attributes each one to an owner.

---

## Suggested order

```
1. P0  presence registry wiring             (~30 lines, unblocks P1)
2. P1  gateway resume notice                (~40 lines, depends on P0)
3. P1  sessions resume --last / --interrupted (~50 lines, independent)
4. P2  gateway /sessions browser            (~150 lines, independent)
5. P2  hub badge + cursor placement         (~80 lines, independent)
6. P3  docs + changelog                     (docs only)
   O1  install-path decision               (operator input needed)
   O2  WIP-checkpoint discipline            (process, not code)
   O3  CLI test triage                     (needs an owner)
```

Items 1-3 are each independently shippable and touch disjoint files from
items 4-6, so two agents could work lanes 1-3 and 4-6 in parallel with no
overlap.

## Resume here

**Start with P0 — the presence registry wiring.** It is the smallest remaining
item (~30 call sites, no new files, no new dependency), it unblocks P1, and it
is the only item that answers the question the operator originally asked
("how do I know a session crashed rather than closed"). Iter-1634 shipped the
mechanism without a single caller, so the 263 lines are dead code today.

Before touching anything, run the collision check — this tree has 2-4
concurrent agents on it and `iter-1635`/`iter-1636` landed while this outline
was being written:

```bash
git fetch origin
git status --porcelain -- crates/operant-core/src/active_sessions.rs \
                        crates/operant-cli/src/tui/app/mod.rs \
                        crates/operant-cli/src/tui/gateway_runner.rs \
                        crates/operant-cli/src/cmd_sessions.rs
```

`origin/main` is the numbering authority; compute the next `iter-N` from its
top label, never from local history.

Two lanes with zero file overlap, if more than one agent is available:

- **Lane A (P0 → P1):** `active_sessions.rs` call sites, the gateway notice in
  `gateway_runner.rs`, `cmd_sessions.rs` flags.
- **Lane B (P2):** `gateway_commands.rs` `/sessions` browser, and
  `session_hub/{mod,render}.rs` badge + cursor. Disjoint from Lane A.

O1 (`/usr/local/bin/operant` deleted) needs operator input, not an agent —
either restore it with `sudo`, or amend the deploy protocol in `AGENTS.md` to
name `~/.local/bin` as the single install path. Do not silently pick one.

O2 (WIP checkpointing) is the process fix that prevents repeating iter-1634's
two total losses of uncommitted work: a leg expected to run for hours pushes
`git push origin HEAD:fleet-<label>-wip` at every milestone. It costs nothing
and nothing in this session did it.

## What is NOT in this outline, and why

Carried from the parent plan's ponytail exclusions, unchanged: no external-CLI
transcript importers, no second daemon, no codex-style cwd prompt on resume,
no additional `SessionFilterMode` arms, no separate cross-process streaming
marker. Each was measured and rejected there with a reason, and this outline
adds nothing to the list.

Also not here: anything about turn attribution, rewind selection, or the
composer. Those are the other agents' lanes on this same tree and they are
actively landing commits against them.

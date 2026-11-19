# Close the consolidation gaps the other plan does not cover: the stale genome zones, the ledger id collision, and the missing surface audit

Dated-frozen: published `2026-11-20`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

Written by the operator agent after the iter-2676 tranche plan closed (iters 2680-2692).
Every row below was MEASURED at `origin/main` = `66533e7f` (iter-2692) before it was
written down. Where a number could not be measured cheaply, the row says so instead of
guessing — that is the lesson of the last plan, whose T12 predicted "2 open" and found 5.

## Why this plan exists next to iter-2693's

**iter-2693** (`docs/plans/2026-11-19-close-the-measured-residuals-stale-checkboxes-doc-drift-dead.md`)
landed while this one was being written, and it owns most of the same ground: the
Hydrogen F8-F11 findings, E303, E311, W006, the clippy triage, and the doc-claims drift.
Publishing this plan as originally drafted would have been a W006 duplicate of it -- the
exact failure T6 exists to fix. So this plan is RESCOPED to what iter-2693 does **not**
cover, and it defers to that plan on everything it does.

Measured by grepping iter-2693's plan for each finding code: it never mentions **E009**,
**E015**, **E104**, `genome emit`, `genome outline`, or `auth.json`. Those are the stale
GENERATED ZONES -- a different class from doc drift, because the fix is to run a generator,
not to reword prose. It also does not propose a general pub-fn surface auditor, which is
the one structural gap neither plan closes.

## Why not the five (now three) open ledger rows

iter-2694 closed RT-136 and RT-285 while this was being written, so the open set is now
**RT-190, RT-280, RT-543** -- all owner-gated or waiting on another subsystem (memory-bank
unification, gateway resume, two-daemon merge). Working them from here would be guessing at
an owner's decision.

## Measured baseline (all commands re-runnable)

```text
git log origin/main -1                          -> a5f0e16b docs(iter-2694)
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md   -> 3   (was 5; iter-2694 closed RT-136, RT-285)
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md   -> 656
bash scripts/clippy-warning-gate.sh             -> seen=16 allowlist=0 new=16 stale=0
   (iter-2693's plan says 42 -- RE-MEASURED at a5f0e16b: 16. That plan measured
    mid-peer-push; this plan's number is the one to trust, and its T8 should be
    re-derived before anyone triages 42 warnings that are not there.)
./scripts/genome lint --all                     -> 21 findings (16 E, 5 W)
python3 scripts/audit-ledger-index.py           -> 4 FAIL (RT-555 x3, RT-553, RT-554)
```

### The 16 clippy warnings, classified

`scripts/clippy-warning-gate.sh` is RED with `new=16` against an empty allowlist. That is
a gate failure, not a style note. The 16 break down as:

| count | lint | where | who owns it |
|---|---|---|---|
| 10 | `dead_code` | `tui/operant_ui/*` (7 files) + `tui/mod.rs` (3) | peer TUI WIP — **do not touch** |
| 1 | `unfulfilled_lint_expectations` | `tui/mod.rs` | peer TUI WIP — **do not touch** |
| 1 | `unused_variables` | `gateway_runner.rs` | peer WIP — **do not touch** |
| 1 | `manual_clamp` | `tui/surface_widget.rs` | peer TUI WIP — **do not touch** |
| 1 | `empty_line_after_doc_comments` | `operant-config/schema/platform_cfg.rs` | **nobody's — closeable** |
| 1 | `redundant_closure` | `operant-cli/tests/continue_session.rs` | **nobody's — closeable** |

So 14 of the 16 are peer-owned TUI work and **must not be touched** (fleet rule: never
edit a peer's in-flight file). Only 2 are genuinely unowned, and those two are T2. The
other 14 close themselves when the peer's W3 renderer work lands; this row was removed
when the plan was rescoped, because iter-2693's T8 owns that triage — but keep the claim
itself, since it is what stops a future agent editing a peer's TUI file
claim rather than assert it, so the next agent does not re-derive it.

### The 21 genome findings, classified by WHICH PLAN OWNS THEM

| code | count | what | owned by |
|---|---|---|---|
| E303 | 7 | docs cite files that do not exist | **iter-2693 T5** |
| E311 | 5 | sources named by no library page | **iter-2693 T6** |
| W010 | 3 | long docs with no table of contents | **iter-2693 T5/T6** (see A-note) |
| W006 | 2 | two pairs of plans claiming one question | **iter-2693 T7** |
| E009 | 1 | ledger:28 cites missing `.operant/auth.json` | **this plan, A4** |
| E015 | 1 | `docs/kb/feed.md` zone stale | **this plan, A1** |
| E019 | 1 | drift: doc says 73 TUI modules, code has 74 | **this plan, A3** |
| E104 | 1 | `AGENTS.md#stack` zone stale | **this plan, A2** |

A-note on W010: iter-2693's plan mentions W010 twice but assigns it no tranche of its own
(it appears in its landscape section, not its tranche list). If its T5/T6 do not clear
W010, it falls to this plan as **A8**. Verify before assuming either way.

### The 4 ledger-index FAILs, classified

```text
FAIL: index row(s) not CLOSED but closed in the checkboxes: RT-555
FAIL: id(s) used by TWO index rows: RT-555
FAIL: id(s) used by TWO detail sections: RT-555
FAIL: index row(s) with NO detail section: RT-553, RT-554
```

These are the SAME four the last plan predicted, and it predicted them correctly. They are
a **peer id collision**: two agents picked RT-555 independently. Per RG-021 the later
publisher renumbers forward and records an ID NOTE. That is A5 — mechanical, but it must
NOT be done blind, because one of the two RT-555 sections is a real closed finding and the
other may be live. The row spells out the disambiguation step.

## The gap with no row yet: no general surface audit exists

`scripts/audit-config-surface.py` already does this for CONFIG KEYS, and its docstring is
the argument for the general version: *"a check that is always red carries no
information"*, and it keeps an `ACCOUNTED_FOR` baseline so a NEW unread key fails while
known ones do not.

Nothing equivalent exists for FUNCTIONS. Measured with a single-pass identifier census over
`crates/**/*.rs`, splitting each file at its first `#[cfg(test)]`:

```text
pub fns in operant-core (non-test code)                  : 1749
name appears exactly once tree-wide (its own definition) : 179
```

That 179 is NOT a count of dead code, and this plan does not pretend it is. Spot-checks
show the shape:

- `with_token` (whatsapp) — 1 def + 1 real call in `gateway_runner.rs` => **not** a
  candidate, the census under-counts because the call is a method-chain fragment.
- `count_sessions`, `expand_tilde`, `is_warm`, `with_media`, `with_buttons`,
  `with_reply_to` — all have real production callers => **not** candidates.
- `subscriber_count` (`turn_end.rs`) — 1 def, 2 hits both `#[cfg(test)]` => **is** a
  candidate, and it is exactly the shape RT-209 shipped twice: a `pub` method whose only
  callers are its own tests.

So the honest expected yield is **a handful, not 179**, and the deliverable is the
DETECTOR plus an accounted-for baseline — not a mass deletion. A6 builds it, A7 deletes only what survives both the
census and the acid test (comment out the `pub`, `cargo check -p operant-core --lib`,
restore). That acid test is the one `dead_modules_stay_dead.rs` already documents as the
only trustworthy signal.

## The tranches

Deferred to iter-2693 and NOT repeated here: T1/T2 (clippy triage, including the 14
peer-owned warnings -- that plan's T8 owns the triage), T3 (E303, seven dead citations),
T4 (E311, five unnamed files), T6 (W006, the plan collisions). If you are executing this
plan, execute that one first; the numbering below is deliberately different so the two can
be cited side by side.

### A1 -- E015: `docs/kb/feed.md`'s ledger TOC zone is stale

`genome lint` reports `zone 'ledger-toc:docs/kb/feed.md' stale -- genome emit`. The fix is
the generator. Do not hand-edit inside the zone markers: the next `genome emit` overwrites
it and the drift returns, which is the whole reason the zone exists.

**Verify:** `genome lint --code E015` prints zero. Mutation: hand-edit inside the markers,
watch the drift come back.

### A2 -- E104: `AGENTS.md#stack` zone is stale

Same class: the generated stack block at the foot of AGENTS.md no longer matches the tree.
`genome outline` regenerates it. This matters more than it looks -- that block is what a
fresh agent reads to learn the repo's language, gate and wiring, and it currently describes
a tree that has moved (iter-2694 changed the TUI module count two screens above it).

**Verify:** `genome lint --code E104` prints zero.

### A3 -- E019: the TUI module count, and why no script caught it

`doc says 73, code has 74`. Measured: `rg -c '^pub mod ' crates/operant-cli/src/tui/mod.rs`
is **74**, and `find crates/operant-cli/src/tui -name '*.rs' | wc -l` is **351**.

Note carefully: iter-2694 ALREADY corrected the prose in AGENTS.md to "74 TUI modules
(351 .rs files)" -- and E019 STILL FIRES, because the genome's DRIFT check compares
against its own recorded baseline, not against the prose. So this is not "fix the number";
the number is fixed. This is the more interesting half:

**AGENTS.md claims `scripts/check-doc-claims.sh` is the authority for stated counts, and
that script did not catch this.** T9 of iter-2693 covers the prose. This row covers the
CHECK: determine whether the TUI module count is among `check-doc-claims.sh`'s derivations
at all (measured: it derives tool counts, overlay counts and command counts). If it is
not, add it -- deriving `pub mod` in `tui/mod.rs` and `.rs` file count -- so the next
hand-typed count fails at the gate instead of drifting for months.

**Verify:** E019 zero, AND a mutation (change the prose to 73) fails
`check-doc-claims.sh` by name. The second half is the point.

### A4 -- E009: the ledger cites evidence that does not exist

`ledger:28` cites `.operant/auth.json`. Either the path is wrong (find the real one --
`auth.json` lives under the operant home resolver, and AGENTS.md is explicit that a store
building its own path from the OS home lands in the wrong profile) or the claim is wrong.
Resolve it; do not delete the claim.

This one is worth doing carefully because it is the SAME class as the one-home-resolver
defect iter-1137 fixed: a doc citing an absolute-ish path that the resolver would place
elsewhere on any machine with `HERMES_HOME` or a sticky profile set.

**Verify:** `genome lint --code E009` prints zero.

### A5 -- the ledger id collision (RT-555 x2, RT-553/RT-554 orphaned)

Two index rows and two detail sections both named RT-555; RT-553/RT-554 are index rows
with no detail section. Per RG-021 the LATER publisher renumbers forward and records an ID
NOTE. Determine which RT-555 section is later by `git log` on the section, renumber THAT
one forward, and give RT-553/RT-554 either a detail section or an explicit index-only
disposition that the checker accepts.

**Do NOT** renumber both, and do not rewrite a peer's section without reading it. Note
iter-2693's plan also mentions this collision (T7 in its numbering) -- if that plan lands
first, this row is already closed and should be marked so rather than re-done.

**Verify:** `python3 scripts/audit-ledger-index.py` prints zero FAIL.

### A6 -- the general surface auditor (the structural gap)

`scripts/audit-config-surface.py` already does this for CONFIG KEYS, and its docstring is
the argument for the general version: *"a check that is always red carries no
information"*, with an `ACCOUNTED_FOR` baseline so a NEW unread key fails while known ones
do not.

Nothing equivalent exists for FUNCTIONS. Measured with a single-pass identifier census over
`crates/**/*.rs`, splitting each file at its first `#[cfg(test)]`:

```text
pub fns in operant-core (non-test code)                  : 1749
name appears exactly once tree-wide (its own definition) : 179
```

That 179 is NOT a count of dead code and this plan does not pretend it is:

- `with_token` (whatsapp) -- 1 def + 1 real call in `gateway_runner.rs` => the census
  under-counts, because the call is a method-chain fragment.
- `count_sessions`, `expand_tilde`, `is_warm`, `with_media`, `with_buttons`,
  `with_reply_to` -- all have real production callers => not candidates.
- `subscriber_count` (`turn_end.rs`) -- 1 def, 2 hits both `#[cfg(test)]` => IS a
  candidate, and exactly the shape RT-209 shipped twice: a `pub` method whose only callers
  are its own tests.

Honest expected yield: **a handful, not 179.** The deliverable is the DETECTOR plus an
accounted-for baseline, not a mass deletion.

Build `scripts/audit-pub-fn-surface.py` modelled on `audit-config-surface.py`:
single-pass census (the two-pass version is O(n^2) and takes >10 min -- measured),
split at `#[cfg(test)]`, skip names that appear more than once (a name collision is not a
caller), emit candidates with their TEST-only hit count beside them, and `--check` with an
`ACCOUNTED_FOR` baseline.

**Verify:** `--check` exits 0 on a clean baseline and non-zero when a fresh callerless
`pub fn` is added. Then disposition every candidate by hand into exactly one of: (a) caller
the census missed -> record the shape, (b) dead -> A7, (c) compat shim that must stay to
keep warning operators -> `ACCOUNTED_FOR` with the reason.

### A7 -- delete only what survives the acid test

For each (b): comment out the `pub`, run `cargo check -p operant-core --lib`, restore. A
clean compile with the item removed is the only trustworthy proof -- the acid test
`dead_modules_stay_dead.rs` documents, and why `rg` alone is not enough.

Ship ONE ITERATION PER COHERENT GROUP, each with a guard in the
`dead_modules_stay_dead.rs` / `dead_db_surfaces_stay_dead.rs` style. If a candidate turns
out to have a caller it goes back to (a) and NO guard is written.

**Verify:** `cargo check --workspace` clean; each new guard fails when its deleted name is
reintroduced.

## Gates / success criteria

(fixed before the run)

```bash
./scripts/genome lint --code E009 --all    # 0  (A4)
./scripts/genome lint --code E015 --all    # 0  (A1)
./scripts/genome lint --code E104 --all    # 0  (A2)
./scripts/genome lint --code E019 --all    # 0  (A3)
python3 scripts/audit-ledger-index.py      # 0 FAIL  (A5)
python3 scripts/audit-pub-fn-surface.py --check   # 0, baseline complete  (A6)
cargo check --workspace                    # clean  (A7)
cargo test -p operant-core --lib           # green
```

Plus the mutation half of A3, which is the row's real deliverable: changing the TUI module
count in AGENTS.md to a wrong number must make `bash scripts/check-doc-claims.sh` FAIL by
name. If it only prints OK, A3 is not done.

And: `rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md` must still print **3** at the end
and `rg -c '^- \[x\]'` must be >= 656. Nothing in this plan closes an open row; it closes
the layer underneath them.

## Cost

| tranche | files | est. size | risk |
|---|---|---|---|
| A1 | 1 (generated zone) | ~2 | none -- generator output |
| A2 | 1 (generated zone) | ~2 | none -- generator output |
| A3 | 1 script + 1 prose line | ~30 | low; the script change is the substance |
| A4 | 1 ledger line | ~3 | low |
| A5 | 1 ledger + 1-2 sections | ~15 | MEDIUM -- peer collision, read before renumbering |
| A6 | 1 new script | ~130 | low -- additive |
| A7 | N files + N guards | 20-200 | MEDIUM -- deletion, one group per iteration |

**Total: ~7 iterations.** Five are doc/gate work measured in minutes; A6 is the only
substantial build and A7 the only destructive step.

No owner ruling is needed for any row: none changes a Holon contract, a kernel field, or an
owner-decided AD.

## Order and dependency

```text
A1, A2, A4            independent; run the generators, minutes each
A3                    independent; the script change is the substance
A5                    last of the doc rows -- the only one that reads peer-authored sections
A6 -> A7              strict chain: detector, then deletions
```

A6-A7 are deliberately LAST. A mass deletion is the one thing here that can destroy work,
and it is only safe once the detector's false-positive shapes are known.

## What this plan explicitly does NOT do

- **Everything iter-2693 owns.** The Hydrogen F8-F11 findings, E303, E311, W006, the
  clippy triage, and the prose half of the TUI count. Read that plan first.
- **The three open rows** (RT-190, RT-280, RT-543). Owner-gated or waiting on another
  subsystem; not guessable from here.
- **TUI keybinding dispatcher refactor.** TODO.md `## Pending` records it was examined and
  deliberately not scheduled: a refactor of `key_handling.rs`, the repo's most contended
  file, delivering no capability an operator asked for.
- **`write_approval.rs` / `schema/media.rs` restoration.** Both deleted deliberately
  (iter-2665 precedent, iter-2680). iter-2693's T5 fixes the docs that still cite them.
- **`--update` on the clippy allowlist.** Forbidden by AGENTS.md;
  `.ci/clippy-allowlist.txt` stays empty.
- **Any convergence of CLI config keys onto `AppConfig`.** iter-2691 measured that there
  is no second system to converge.

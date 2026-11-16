# One identity, one report: red-teaming operant's skill infrastructure against hermes-agent's at scale

Date: 2026-11-16
Status: **red-team findings only — no code yet**
Inputs: `../external-repos/hermes-agent` @ `882cbc99e4`, `operant` @ `f76ae548`
Method: read both implementations on the host, then measure every claim against
operant's actual code. Each finding below names the hermes mechanism, the operant
line it should touch, and the observable defect.

---

## 0. Why this red-team exists

The iter-2395..2466 arc fixed three real defects: the advertised block spent its
budget by **deleting skills** (iter-2395, clip + window-scaled budget), the
search path had **four matchers ranking three different ways** (iter-2428/2429),
and the root list was **hand-built in seven places** (iter-2431/2464/2465).

Those are done. This document asks the next question: **what does a skill system
that has been running at scale for years do that operant does not?** Not
"which features are missing" — the feature list is complete. The question is
which *invariants* hermes enforces mechanically that operant leaves to the
operator's eye, because at 100+ skills the eye stops working.

The answer is five findings, in descending severity.

---

## 1. Hermes's five scale mechanisms (what to compare against)

Measured, with the file each lives in.

| # | mechanism | where | what it buys |
|---|---|---|---|
| A | `SKILL_PROMPT_DESC_LIMIT = 60` + **write-time reject** | `agent/skill_utils.py:846`, `tools/skill_manager_tool.py:174` | an over-long description cannot enter the pool, so the index never has to truncate |
| B | `resolve_skill_catalog` — **explicit shadowing resolution** | `agent/skill_utils.py:473` | a shadowed skill is `status="shadowed"` and **warned once**, never silently invisible |
| C | `iter_project_skill_files` — **quarantine chokepoint** | `agent/skill_utils.py:661` | a dangerous project skill is skipped at the one iteration point; fail-closed |
| D | `scan_skill_cached` — **content-digest scan cache** | `tools/skills_guard.py:934` | the guard runs per load without re-scanning unchanged content |
| E | `unified_search` — **trust-ranked merge** | `tools/skills_hub_search.py:261` | the `limit` cut never drops a builtin because a community source finished first |

The rest of the comparison is operant against these five.

---

## 2. Findings

### F-1 (HIGH, operational) — A shadowed skill disappears with no report

**Hermes** (`skill_utils.py:473-521`): `resolve_skill_catalog` annotates every
copy. A lower-precedence copy gets `status="shadowed"`, and a
`logger.warning` fires — once per (path, shadowing-path) key, so the noise is
bounded — naming the higher-precedence copy that owns the name.

**Operant** (`crates/operant-core/src/skills.rs:696-700`):

```rust
if self.skills.contains_key(&skill.name) {
    continue;      // <-- silent
}
```

The skill is dropped from the index, the `RootIssue` list does not learn about
it, and `skills list` prints nothing. The operator's question "I installed
`ponytail-review` — where is it?" answers **no**, and the reason (a same-named
operator skill shadows it) is invisible.

**This is the same defect class iter-2466 just fixed for dead symlinks**, one
layer over: a root the resolver *used* is fine, but a skill the resolver
*dropped* is not reported.

**What it is NOT:** a surface-disagreement bug. I first wrote that the listing
and the advertised block could disagree, and checked it before committing this
document - they cannot. `list()` (skills.rs:727) and `list_available()`
(skills.rs:762) both iterate `self.skills.values()`, so a skill dropped by the
`contains_key` guard is absent from **both**, consistently. The defect is
silence, not divergence, and the fix is a report rather than a reconciliation.

**Fix shape:** when `self.skills.contains_key(&skill.name)` is true, push a
`RootIssue { reason: "shadowed_by: <winning path>" }` instead of a bare
`continue`. Bounded by the same name-keyed dedupe hermes uses, so a pool of
110 skills with 2 clashes produces 2 lines, not 110.

**Proof obligation:** a test with a pool skill and an integration skill sharing
a name asserts `issues()` contains a `shadowed_by` entry naming the *pool* path.
Mutation: delete the `issues.push` and the test fails.

---

### F-2 (HIGH, safety) — Operant never scans a skill on the load path

**Hermes** (`skill_utils.py:635-660`): project-tier skills — the tier an
*untrusted clone* can write — pass through `is_quarantined_project_skill`
before they are yielded. The scan verdict is cached by content digest, and
**a scanner crash quarantines rather than admits** (`fail-closed`).

**Operant** (`skills.rs:243-300`, `SkillRoots::resolve`): the project root is
resolved and pushed like any other root. `skills_guard::scan_skill` exists and
is good — but `grep` shows its only production callers are
`binary_updater.rs:767` (install path) and `skill_marketplace.rs` (install
path). **Nothing on the load path calls it.**

So the guard is an **install-time gate with no load-time gate**. That is fine
for skills operant installed itself. It is not fine for the project root, whose
entire justification (`SkillRoots::resolve` pushes `<workspace>/skills`) is that
it is *the operator's own repo's* skills — i.e. a directory a `git clone` can
populate.

**Why this is not a hypothetical:** the project root is the one root an
attacker does not need filesystem access to plant. "Clone a repo, open it in
operant, the repo's `skills/` becomes agent context" is a prompt-injection
delivery path that exists today.

**Fix shape:** reuse the quarantine idea at the *one* chokepoint
(`walk_skill_dirs` or `load_root`), not at each call site — hermes's own
comment says why: "so the quarantine cannot be bypassed by a call site
forgetting the check". Fail closed on scanner error, and **cache by content
digest** (finding D is not separable from this one: without the cache the scan
cost lands on every `load_all`, and `skills.rs:506` already documents that
`load_all` runs per agent slot).

**Decision needed from the operator:** this is the one finding that changes
behavior in a way an operator can *feel* — a project skill that the guard
dislikes stops appearing. Hermes made that call (quarantine is on by default).
Operant should make it deliberately rather than by inheritance.

---

### F-3 (MEDIUM) — The description limit is a clip, not a contract

**Hermes** (`skill_manager_tool.py:174-185`): `_description_budget_error`
**rejects** a create or an edit that would write a description over 60 chars.
The docstring is explicit about the residual: "An unchanged legacy over-limit
description stays editable" — the reject fires only when the call *changes* it.

**Operant** (`context_management.rs:1936`): `ADVERTISED_SKILL_DESCRIPTION_CHARS
= 120`, applied by `clip_skill_description` at render. **No write path enforces
it.** Measured on this host, 40 skills with a `description:` line:

| stat | value |
|---|---|
| n | 40 |
| mean | 107 |
| median | 51 |
| over 60 | 10 (25%) |
| over 120 | 7 (18%) |

So at hermes's 60 chars, **a quarter of this install would clip**; at operant's
120, **under a fifth** would.

**Is operant's 120 the wrong number?** No - and this is worth stating plainly
because it looks like a miss. Hermes fits 60 because *its* shipped skills are
held to 60 by `tests/skills/test_authoring_standards.py:109`; operant's
descriptions run longer, and 60 would clip 25% of them (10 of 40 on this
host). **The clip is correct. The missing half is the enforcement.**

**Why it matters at scale:** with no write-time bound, the clip is the only
thing keeping the block bounded — and the clip silently degrades a description
the author thought was complete. The operator sees
`Use this skill for auditing rep...` and has no signal that 9 chars were lost
in a file they wrote.

**Fix shape:** a write-time check on the `skill_manage` create/update path,
mirroring hermes exactly — reject a *changed* over-limit description, never
block an edit to an unchanged legacy one (that rule is load-bearing: without it
a body typo fix on an old skill becomes impossible). The message should name the
measured length and the limit, and point at the skill body.

**Proof obligation:** a test that a create with a 130-char description is
refused, and — the discriminating half — that a *body* patch to a skill whose
description is already over the limit still succeeds.

---

### F-4 (MEDIUM) — No scan cache, so the guard cannot be on the hot path

**Hermes** (`tools/skills_guard.py:934-952`): `scan_skill_cached` keys the cache
on `sha256(content)` **plus** a `scanner_version` (currently
`skills-guard-v13`) plus a source identity. A hit requires all three to match,
so a scanner upgrade invalidates every stale verdict.

**Operant**: no `scan_cache` / `content_digest` / `cache_dir` anywhere in
`skills_guard.rs` or the skill modules. `scan_skill` is pure compute on every
call.

**This is not an independent finding — it is the prerequisite for F-2.** The
project-skill quarantine is only affordable because the scan is cached. Adding
the quarantine without the cache means `load_all` (which
`skills.rs:506-510` documents as running per agent slot, ~3,800 file reads per
walk) pays a full scan each time.

**Fix shape:** a content-digest-keyed cache under the operant home, keyed on
`sha256(content) + scanner_version + source`. The scanner version must be a
constant that is bumped when the rules change — otherwise a rule added today
serves yesterday's verdicts forever, which is a security bug wearing a
performance costume.

---

### F-5 (MEDIUM, not LOW as first assessed) - The limit cut breaks ties alphabetically, ignoring root precedence

**Hermes** (`skills_hub_search.py:261-271`): `unified_search` stable-sorts by
`TRUST_RANK` **before** `deduped[:limit]`, with a comment naming the exact
failure: "so the limit cut never drops a builtin/official entry because a
high-volume community source finished first".

**Operant** (`crates/operant-core/src/skill_mentions.rs:283-311`): the ranked
half is fine - post-iter-2428 this is the ONE canonical 4-tier matcher, and
every caller passes a limit (verified: `cmd_skills.rs:610` with
`SKILL_SEARCH_LIMIT`, the popup, and the `$` snapshot). So the truncation is
live, not hypothetical.

The defect is the **tiebreak**:

```rust
matched.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
```

Tier first, then **alphabetical by name**. Root precedence never enters the
key. So when a pool skill and a vendor skill match a query at the same tier,
the winner is decided by the alphabet - `zzz-vendor` beats `aaa-pool` - and
`take(limit)` then drops one of them on that basis.

**This is a real correctness bug, not a doc note.** Two skills of equal match
quality should resolve by root precedence (operator pool > extra roots >
vendor), because that is the order `SkillRoots` was built to express and the
order every *other* surface (listing, lookup, advertised block) already uses.
Search is the one surface that ignores it.

**Aggravating detail:** `loadable_skills_in` yields `(name, description)`
pairs with **no origin attached**, so the information needed to break the tie
correctly is discarded before the sort. The fix has to carry the root tier
through, which means touching the iterator's return type - small, but not a
one-line change.

**Fix shape:** carry the root index alongside each entry, sort by
`(tier, root_index, name)`, and truncate. Mutation proof: two skills at the
same tier where the pool one sorts alphabetically *later* than the vendor one;
assert the pool one survives the limit. Deleting the root-index key makes it
fail.

---

## 3. What operant does BETTER (do not "fix" these toward hermes)

Red-teaming that only finds faults produces a worse system. These are places
where copying hermes would be a regression.

| area | operant | hermes | verdict |
|---|---|---|---|
| description clip | 120, measured against this host's distribution (mean 111) | 60, enforced by a test on its own shipped skills | operant's number is *right for its corpus*; adopt hermes's *enforcement*, keep operant's *value* |
| root refusals | `RootIssue` with `describe()`/`explanation()`, surfaced by `skills list` (iter-2466) | `logger.warning` only | operant is better; hermes's warning is invisible to a TUI operator |
| name/dir mismatch | `name_dir_mismatch` issue at load time (iter-2337/2395) | aliases both names in the catalog | operant is better; hermes accepts two spellings forever |
| shadowing model | first-root-wins by name, documented at the call site | full `status` enum + `load_name` aliases | **hermes is better** (F-1) |
| Holon isolation | `resolve` returns early with the brief's library only | no equivalent | operant is better |

---

## 4. Ordered execution plan (when the operator says go)

Each slice: one iteration, one commit, one push, mutation-proven.

| slice | finding | acceptance |
|---|---|---|
| 1 | F-1 shadowing report | `issues()` names the winner; mutation (drop the push) fails |
| 2 | F-5 search tiebreak by root | pool skill survives the limit over an alphabetically-later vendor skill; mutation (drop root key) fails |
| 3 | F-4 scan cache | digest+version key; a bumped version invalidates a hit |
| 4 | F-2 quarantine | dangerous project skill absent from `load_all`; scanner error → quarantined |
| 5 | F-3 write-time reject | over-limit create refused; body patch on an over-limit skill still lands |

Order is dependency-driven: F-4 must precede F-2, and F-1 is independent and
cheapest so it goes first (it is also the one that closes a defect iter-2466
was already working on). F-5 moved up because measurement showed it is a live
correctness bug, not a note: every caller passes a limit.

**Not in scope:** no new root type, no new skill format, no change to the
advertised block's shape. The architecture is sound; these five are enforcement
gaps around it.

---

## 5. Verification protocol for the whole arc

1. Every fix ships a test that **fails without the fix**. A mutation proof that
   cannot fail is not a proof (iter-2429 taught this the hard way: the old
   boolean sort was stable and inherited fixture order, so the "proof"
   passed either way).
2. Each slice: `cargo test -p operant-core --lib` + the CLI skill tests, scoped,
   never `--workspace` mid-slice.
3. Clippy on touched files only; the 10 pre-existing deny errors in peer files
   are not mine to fix.
4. Board rows through `./scripts/genome` from a clean tree.

## 6. Claims I re-derived before committing (and two I got wrong)

Writing the findings was fast; checking them was the work. Every number in
this document was re-measured on this host rather than recalled.

| claim as first drafted | measured | correction |
|---|---|---|
| mean 111 / median 63 descriptions | mean 107, median 51, n=40 | numbers replaced with the derivation shown |
| "60 would clip 52% of them" | 10 of 40 = **25%** | corrected; the *conclusion* (keep 120) survives, the *argument* does not |
| "the listing and the advertised block disagree" (F-1) | both read `self.skills.values()` | **withdrawn** - the bug is silence, not divergence |
| "search may not be limited, so F-5 is a doc note" | every caller passes a limit | F-5 promoted LOW -> **MEDIUM**, a live bug |

Two of the four corrections made a finding *stronger* (F-5) or *narrower but
still real* (F-1). One (F-3) lost its headline statistic and kept its point.
That is the ratio I would expect from an honest red-team: most claims survive
contact with the code, and the ones that do not are worth finding before
someone builds on them.

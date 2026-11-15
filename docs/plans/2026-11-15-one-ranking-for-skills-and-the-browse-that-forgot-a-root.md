# One ranking for skills: the fourth matcher, and the browse that forgot a root

Dated-frozen: planned `2026-11-15`. Follows the iter-2395 re-architecture
(clip + window-scaled budget + reported `name_dir_mismatch` + one walker skip
rule) and the iter-2349 audit.

## The problem, measured

The re-architecture gave the **advertised block** one budget rule and one clip.
It did not touch the **search path**. Four matchers survived it, and three of
them rank differently. Same query, different order, per surface:

| surface | matcher | ranking |
|---|---|---|
| `skill_mentions::search_skills_in` (core) | canonical | 4 tiers: exact / prefix / substring / description |
| TUI `$` popup (`compute_skill_mention_suggestions`) | own copy | 2 tiers: name-substring, then description — a boolean `sort_by_key` |
| `operant skills search` (`cmd_skills::search_skills`) | own copy | **none** — `load_all()` directory-walk order |
| gateway picker (`offer_skill_picker`) | delegates | correct (calls `search_skills`) |

**Measured, on the operator's own pool (110 skills):**

```
$ operant skills search audit
  anti-ceremony     <- description-only hit, printed FIRST
  audit-website     <- prefix AND exact-name hit, printed SECOND
```

`search_skills_in` would print `audit-website` first. So the CLI and the core
disagree about the same query, and the CLI's answer is arbitrary: `load_all()`
returns insertion order over a `HashMap`-keyed cache, so nothing in the type
system or the code says "best match first".

**Measured, on a synthetic corpus (proving the popup tier too):**

```
entries: jsonpath (PREFIX), aio-json-dump (SUBSTRING)
q=json  core:  [1:jsonpath, 2:aio-json-dump]
        popup: [name:aio-json-dump, name:jsonpath]
```

The popup ranks a substring above a prefix; the core ranks a prefix above a
substring. Both are "correct" under their own rule; the operator gets whichever
surface they happened to type into.

**The roots gap.** `offer_skill_picker`'s `/skills` browse call site
(`gateway_runner.rs:6928`) passes bare `vendor_skill_roots_for(handler)`, while
the three sibling call sites (7366, 1031, 1657) build the list as
`vendor_skill_roots + skills_extra_roots`. So over a channel, **browse omits
`[skills] extra_roots` while invoke and search include them** — the exact
class of defect iter-2159 closed on the CLI side, still open on one gateway
path. No test pins it (`grep offer_skill_picker` finds zero test references).

## What is NOT a gap

- **The two walkers.** `skills_tool::collect_skills_recursive` and
  `skills::walk_skill_dirs` both exist, but the tool's `SkillMeta` carries a
  **child tree** (meta-skill nesting, `children: Vec<SkillMeta>`) that the
  shared walker deliberately flattens. Routing the tool through the shared walk
  would lose `children`. Justified divergence; the *skip rule* is already
  shared (iter-2395d).
- **The seeder.** `cmd_skills::collect_skill_dirs` is a four-line adapter over
  `walk_skill_dirs`. Already one walker.
- **The advertised block.** Fixed in iter-2395. Not re-litigated.
- **`search_skills` vs `cmd_skills::search_skills`.** A name collision, not a
  duplicate: the CLI one is a print-and-return-`Result` command handler; the
  core one returns ranked data. The collision is *how the fourth copy hid*.

## Red team: what breaks this plan

Written before the code, because two of the plan's own assumptions fail on
inspection.

1. **The popup's snapshot is a boot-time freeze, not a live read.**
   `SKILL_MENTION_SNAPSHOT` is a `OnceLock<RwLock<Vec<(String, String)>>>`,
   populated once at boot (`tui/app/init.rs:448`) and refreshed only by
   `/skills rescan`. Routing the popup's *ordering* through
   `search_skills_in` therefore ranks **the snapshot**, not the filesystem —
   which is correct and intended (the popup must stay a viewport), but it means
   **gate 2 cannot be tested through the popup by mutating a temp dir**; the
   test must populate `set_skill_mentions` directly (the pattern
   `prompt_input/tests.rs:2748` already uses).

2. **`/skills rescan` silently drops `extra_roots` — a live bug this plan must
   fix, not route around.** The rescan handler (`tui/app/commands.rs:1892`)
   calls the **two-argument** `register_invocable_skills(pool, vendor_roots)`.
   That form's own docstring says it "cannot see `[skills] extra_roots`". So an
   operator with configured extra roots who runs `/skills rescan` **loses those
   skills from `$` completion and `/skill <Tab>` until the process restarts** —
   the popup narrows below what boot offered. The fix is to pass the resolved
   `SkillRoots` (the `register_invocable_skills_in` form), which the boot path
   already uses. This is gate 4's real target; the gateway root mismatch is the
   same defect on a different surface.

3. **Name-shadowing is load-bearing.** `cmd_skills::search_skills` and
   `skill_mentions::search_skills` differ only by module, and the CLI one is
   what the subcommand dispatch calls. Renaming either during the split would
   touch the `Commands` enum's match arm; the plan keeps both names and changes
   only the CLI one's *body* (split into a data function + a printer).

4. **`load_all()` is `&mut self`.** Any ranking that wants to reuse the
   manager must own it. The existing code already does; a borrow-based rewrite
   would not compile. Noted so the executor does not attempt one.

## Mechanism

**One ranking, owned by core, every surface calls it.**

`skill_mentions::search_skills_in` is already the canonical 4-tier matcher and
already takes a `SkillRoots` (so `extra_roots` and the project root are in
scope by construction). Two changes:

1. **`cmd_skills::search_skills` becomes a printer over it.** Split the command
   into `ranked_skill_search(config, query) -> Vec<(String, String)>` (data,
   testable) and a thin print loop. Delete the inline `manager` + `filter`.
   The command's roots come from `crate::skill_roots::skill_roots(config)` —
   the same resolver the agent and the tools read — not a hand-built
   `vendor_skill_roots + extra_roots` chain.

2. **The popup routes through the same ranking.** `compute_skill_mention_suggestions`
   keeps its snapshot (the entries are already `(name, description)`), but the
   filter+sort becomes a call into core. The popup's 20-row cap stays: a
   suggestion list is a viewport, not an inventory.

The one thing that must NOT move: the popup renders a **description beside every
row**, so the returned pairs are already the render payload. The change is
ordering, not data shape.

3. **The `/skills` browse gets the same roots as its siblings.** Build the
   extended list once at the top of the gateway's skill-command block and pass
   it to every `offer_skill_picker` call in that block.

4. **`/skills rescan` re-registers through the resolved roots.** The rescan
   handler calls `register_invocable_skills(pool, vendor_roots)` — the form
   whose own docstring says it cannot see `extra_roots`. Switching it to
   `register_invocable_skills_in(&skill_roots(config), bundles)` makes a rescan
   a no-op for the popup's name set instead of a silent narrowing.

5. **The stale docstring** on `ADVERTISED_SKILLS_TOKEN_BUDGET` is corrected: it
   currently describes the pre-iter-2395 flat constant ("is a flat 4 000 … it
   advertises ~105 clipped skills and then starts deleting") as present
   behaviour, when the function below it now returns the window share.

## Gates / success criteria

(fixed before the run)

1. `operant skills search audit` on the operator's pool prints `audit-website`
   **before** `anti-ceremony` — an exact/prefix name hit outranks a
   description-only hit. Asserted on the *printed order*, not on a set.
2. A corpus test where a substring name sorts alphabetically before a prefix
   name: the prefix wins in BOTH `search_skills_in` and the popup. This is the
   mutation target — revert either ranking and the test fails.
3. `operant skills search` sees a skill that lives only in `[skills]
   extra_roots` (the same set the agent advertises). Before: absent or
   unranked.
4. **A gateway/TUI test asserting `/skills` browse and `$skill` invoke resolve
   against the SAME root list.** Currently there is none. The rescan half is
   testable in-process: populate the snapshot over roots A, rescan over roots
   A+B, and assert the popup still offers a name that lives only in B.
5. The existing core matcher tests stay green (`skill_mentions/tests.rs`:
   exact/prefix/description tiers).
6. `cargo clippy -p operant-core -p operant-cli --all-targets` introduces no new
   warning in the touched files.

## Cost

- 1 new pure function in `operant-cli/src/cmd_skills.rs` (returns data); the
  command handler shrinks.
- 1 changed filter+sort in the popup (deletes ~10 lines, adds a call).
- 1 hoisted `let` in the gateway's skill-command block (3 call sites, 1 line
  each).
- 1 stale docstring corrected (`ADVERTISED_SKILLS_TOKEN_BUDGET`'s, which still
  describes the pre-iter-2395 flat constant as current behaviour).
- No new config. No new dependency. No schema change.

## Risks

- **The popup's ranking becomes the core's, which is stricter.** A query that
  matched a skill by name-substring still matches (tier 2), so nothing
  disappears — only the order changes. The 20-row cap could in principle admit
  different rows under a new order; that is the intended fix, and gate 2 is the
  test that pins it.
- **`load_all()` is `&mut self`** (it inserts into the cache). The CLI printer
  must own the manager, not borrow it — the existing code already does this, so
  the change is the *ranking*, not the ownership.
- **A printed-order assertion is brittle if the pool changes.** Gate 1 builds
  its own temp corpus; it does not assert against the operator's live pool.

## Not doing

- Collapsing the two walkers (see "What is NOT a gap").
- A fuzzy matcher (subsequence / edit distance). The four tiers are exact,
  prefix, substring, description; adding fuzzy is a different feature with its
  own false-positive surface, and no measurement here asks for it.
- Ranking the **advertised** block (the frozen prefix) by relevance. It is
  alphabetical by name for prompt-cache stability — a byte change per turn
  would break the prefix. That is a deliberate trade, already recorded in the
  iter-2395 doc.
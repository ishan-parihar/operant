# Skill infrastructure at scale: the three architectures, and where operant's is wrong

Dated-frozen: audited `2026-11-14`; findings mark verified vs asserted.

## Scope

How jcode and hermes-agent handle skill management at scale, read from source at
`external-repos/jcode` and `external-repos/hermes-agent`, set against operant's
own three-walker implementation. The question the round asked: what does each
architecture do when the skill count is large, and which of the three is right.

Every number below was measured on this host or read from the named file. Where
a claim rests on reading rather than running, it is marked `[asserted]`.

## The three architectures

| | how a skill is found | advertised to the model | cost control |
|---|---|---|---|
| **operant** | 3 walkers (`find_skill_recursive`, `walk_skill_dirs`, `collect_skills_recursive`) + an alias-keyed `SkillManager` map | every description in full, in a frozen `<available_skills>` block | a 4 000-token ceiling, enforced by dropping whole skills |
| **jcode** | `SkillRegistry`, project skills as a per-session overlay composed at read time | all of them, each description clipped to 120 chars | the clip is the only control |
| **hermes-agent** | `resolve_skill_catalog` over tiers, with an on-disk snapshot cache | all of them, each description clipped to 60 chars | the clip, plus a write-time REJECT of anything longer |

## Findings

- **[verified]** operant's advertised block has no per-skill description cap.
  `agent/events.rs:1238` formats the full description into the line, and no clip
  exists anywhere on that path (`grep clip_skill_description` finds nothing in
  `agent/`). Measured on this host: 310 available skills, 21 674 tokens of
  descriptions against a 4 000-token budget, so **53 are advertised and 257 are
  dropped — 83%**. Every skill from `cognitive-kernel` alphabetically onward is
  invisible to the model.
- **[verified]** jcode and hermes both cap, at different widths, and neither has a
  token ceiling. jcode: `SKILL_DESC_MAX_CHARS = 120`
  (`jcode-base/src/prompt.rs:180`), collapsing whitespace to one line and cutting
  at a char boundary with an ellipsis. hermes:
  `SKILL_PROMPT_DESC_LIMIT = 60` (`agent/skill_utils.py:846`), cutting at a *word*
  boundary so the index never shows half a word.
- **[verified]** hermes enforces its cap at write time, not just at render time.
  `skill_manager_tool._description_budget_error` refuses a create or an edit that
  *changes* a description above 60 chars, while deliberately allowing an
  unchanged legacy over-limit body patch to proceed. operant has no write-time
  gate: `MAX_DESCRIPTION_LENGTH = 1024` in `tools/skills_tool.rs` only clips at
  *read* time and is 17x looser.
- **[verified]** the measured description distribution on this host explains why
  the cap matters: n=271, mean 111 chars, median 63, p90 256, max 392. A 60-char
  cap fits 48%, an 80-char cap 54%, a 120-char cap 64%. So a tight cap alone
  would still leave half of this pool over budget — the clip and the ceiling are
  solving different problems and operant has only the ceiling.
- **[verified]** hermes treats a frontmatter/directory name mismatch as a hard
  lint ERROR: `name-dir-mismatch` in `tools/skill_linter.py:90`. Operant *detects*
  the same condition (`validate_root_health`, `tools/skills_tool.rs:717`) but
  only inside `collect_tree_validation`, which runs on the `generate_map` and
  `skills audit` paths — never at load and never at write. That is precisely why
  iter-2337's resolver defect could exist: the rule was known, and it had no
  authority.
- **[verified]** operant's `SkillManager` still keys one skill under three aliases
  (frontmatter name, dir name, `category/skill`), which is the mechanism behind
  the four defects fixed in iter-2315/2316/2337. jcode has no equivalent: it keys
  on one name and resolves the project overlay separately
  (`effective_for_working_dir`), so a name can have exactly one identity.
- **[verified]** jcode's project skills are a per-session overlay composed at read
  time and never enter the shared registry (comment at `skill.rs:70`, issue
  #457). operant's `project_root` is resolved into the same flat root list as
  the pool, which is simpler but means a project skill occupies the same
  precedence space as a global one.
- **[asserted]** jcode's model does not scale past roughly 300 skills either: 310
  skills at 120 chars each is about 12 090 tokens, and 1 000 would be 39 000.
  Neither of the two reference designs solves the general case. What they do
  differently is fail *gracefully* — jcode spends 12k tokens and loses nothing,
  while operant spends 4k and loses 83% of its skills.

## What this means for the re-architecture

Three defects are confirmed, in priority order, and all three are design choices
rather than bugs to patch:

1. **No description cap on the prompt path.** Add the clip (jcode's 120 or
   hermes' 60) and the 4 000 ceiling stops being a skill-deletion mechanism. This
   is the single highest-value change and it is measured: at 80 chars, coverage
   goes from 53 to 144 skills.
2. **The ceiling's selection order is alphabetical.** `list_available` sorts by
   name and the render loop `continue`s past anything over budget, so the
   operator's 53 alphabetically-early skills consume the entire budget. jcode and
   hermes both render everything, so neither has this failure. A ceiling that
   drops by name is worse than no ceiling, because the drops carry no signal
   about which skills matter.
3. **Three walkers and a three-alias map, where the reference designs have one
   identity per name.** iter-2337 fixed the consequences; the cause is the
   multi-key map plus three independent discovery implementations. hermes' hard
   `name-dir-mismatch` rule exists precisely to keep this state unrepresentable.

Not a defect, and worth keeping: operant's `issues()` / `RootIssue` reporting and
the doctor row. Neither reference design surfaces a dead symlink or an unreadable
root the way operant now does.

## What was checked and found sound

- The listing/resolver/manager now agree on the visible set after iter-2337
  (verified: all three exclude a skill under a hidden dir, all three find a nested
  skill).
- The 4 000-token ceiling is enforced and the drop is announced in the prefix, so
  it is not silent — it is merely unintelligent about *which* skills it drops.
- The description-search tier added in iter-2321 ranks name hits above description
  hits in both the popup and `search_skills_in`.
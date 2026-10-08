# Writing Guide for Reference Nodes

> Vendored distillate of the **skill-creator** skill (its "Creating a skill",
> "Skill Writing Guide", "Running and evaluating test cases", and "Description
> Optimization" sections), kept *inside* meta-skill-creator so this skill is
> self-contained. If skill-creator is installed, its full eval harness
> (`run_eval.py`, benchmark aggregation, viewer) is optional heavy machinery for
> Phase 6 — useful, never required.

## Capture intent first

Before writing any file, answer from the conversation history (the user may have
already described the workflow):

1. What should this node enable the agent to do?
2. When should it be reached? (user phrases, task contexts)
3. What is the expected output format?
4. Does it need test cases? Objectively verifiable outputs (file transforms, data
   extraction, fixed workflow steps) benefit; subjective outputs (writing style)
   are better judged qualitatively. Suggest a default, let the user decide.

Interview for edge cases, input/output formats, example files, success criteria,
and dependencies before writing — and before writing test prompts.

## Descriptions (the root frontmatter)

- The description is **the triggering mechanism**: what it does AND when to use
  it. All "when to use" info goes there, not in the body.
- Be **pushy**. Agents under-trigger on useful skills; name the contexts
  explicitly ("Make sure to use this skill whenever the user mentions … even if
  they don't explicitly ask for a 'dashboard'"). List trigger phrases in quotes.
- Keep it a single-line quoted string. The validator warns below 60 characters.
- Below root there are no descriptions: the **first line after the H1** plays
  that role for navigation — write it as a routing sentence ("Descend when the
  output feels too smooth…" style guidance belongs in the parent's link text;
  the summary says what the file covers).

## Progressive disclosure

Three levels load: (1) name + description — always in context; (2) root SKILL.md
body — whenever the skill triggers; (3) reference files — as needed.

- Root SKILL.md: aim **under 200 lines** — it pays rent on every traversal.
  (Flat skills get 500; a meta-skill root is a routing table, not a manual.)
- Reference files: **under 500 lines**. Past ~300, add a table of contents.
- Point at every reference file from somewhere with **guidance on when to read
  it** — an unlinked file does not exist (the validator enforces this).
- Organize by variant when a node supports several domains/frameworks: one file
  each under `references/`, and the parent holds only the selection guide.

## Writing style

- Prefer the imperative voice; explain **why** a step matters, not what the line
  plainly says. Comments in scripts carry rationale (safety argument, measured
  reason), never narration.
- **Define output formats literally** — a fenced template the agent must fill:
  `## Output` + exact structure, so results are comparable across runs.
- **Use examples**: `Input:` → `Output:` pairs with realistic values beat
  abstract description. One worked example per node where practical.
- No malware, exploit code, or content that would surprise the user if described.
  The skill's contents must be exactly what the description promises.

## The eval loop (Phase 6 machinery)

1. **Spawn all runs in one turn** — for each test case, a with-skill run and a
   baseline run (no skill, or the pre-edit snapshot) as parallel subagents.
   Never run one cohort first and the other later.
2. Save outputs to `<skill>-workspace/iteration-<N>/<eval-name>/{with_skill,without_skill}/outputs/`;
   write `eval_metadata.json` (descriptive eval name + prompt + assertions).
3. **Draft assertions while runs are in flight.** Assertions must be objectively
   verifiable and descriptively named ("output CSV has a `region` column", not
   "output is good"). Subjective skills: judge qualitatively, don't force counts.
4. Capture `timing.json` (tokens, duration) from each run's completion
   notification — it is not recoverable later.
5. Grade, compare cohorts, fix the skill, iterate. On a navigation failure fix
   the **one-line summary or link text first** — cheaper than rewriting the file.
6. Finish by optimizing the **root description only** (a meta-skill has one
   triggering surface): test variants against realistic prompts, keep the variant
   that triggers on the widest set of legitimate requests without false fires.

# Shadow Diagnostic — Signatures and Interventions

> This file is the exhaustive reference for the five shadows. Read it when the shadow is ambiguous, when multiple shadows are co-active, or when you need to design a non-obvious intervention.

## Table of Contents
1. [The 2×2+1 Shadow Matrix](#1-the-221-shadow-matrix)
2. [Shadow 1: Dark-Addiction (Matrix Overload)](#2-shadow-1-dark-addiction-matrix-overload)
3. [Shadow 2: Dark-Allergy (Matrix Starvation)](#3-shadow-2-dark-allergy-matrix-starvation)
4. [Shadow 3: Golden-Addiction (Potentiator Flooding)](#4-shadow-3-golden-addiction-potentiator-flooding)
5. [Shadow 4: Golden-Allergy (Potentiator Stagnation)](#5-shadow-4-golden-allergy-potentiator-stagnation)
6. [Shadow 5: Sinkhole of Indifference (Great Way Starvation)](#6-shadow-5-sinkhole-of-indifference-great-way-starvation)
7. [Co-active Shadows](#7-co-active-shadows)
8. [Diagnostic decision tree](#8-diagnostic-decision-tree)

---

## 1. The 2×2+1 Shadow Matrix

The four lesser-shadows form a 2×2 matrix: {Dark, Golden} × {Addiction, Allergy}. The fifth shadow (Sinkhole of Indifference) is the greater-cycle analog — it is the macro-shadow that the lesser-cycle 2×2 cannot see.

```
                         ADDICTION (hyper-ingest)        ALLERGY (hypo-ingest)
                    ┌──────────────────────────────┬──────────────────────────────┐
   MATRIX           │  SHADOW 1                    │  SHADOW 2                    │
   (ingests         │  Dark-Addiction              │  Dark-Allergy                │
    Catalyst)       │  Loops, re-reads, never      │  Skips instructions, ignores │
                    │  synthesizes                 │  reference files             │
                    ├──────────────────────────────┼──────────────────────────────┤
   POTENTIATOR      │  SHADOW 3                    │  SHADOW 4                    │
   (ingests         │  Golden-Addiction            │  Golden-Allergy              │
    Experience)     │  Hallucinates, fabricates,   │  Refuses to adapt, outputs   │
                    │  produces ungrounded output  │  same result every time      │
                    └──────────────────────────────┴──────────────────────────────┘

   GREATER CYCLE:
   GREAT WAY        │  (Great Way addiction —      │  SHADOW 5                    │
   (ingests         │   fanatical crystallization) │  Sinkhole of Indifference    │
    Choice)         │                              │  Generic, depolarized output │
```

---

## 2. Shadow 1: Dark-Addiction (Matrix Overload)

### 2.1 Mechanism

The Matrix hyper-ingests Catalyst without metabolizing it into coherent Experience. The contact-boundary is too permeable — Catalyst floods in faster than the Matrix can synthesize. The agent loops: re-reading files, re-running searches, re-checking the same instructions, never producing output.

### 2.2 Surface signatures (in the skill itself)

- `SKILL.md` exceeds ~500 lines, with no clear progressive-disclosure structure
- Multiple reference files overlap in content (>30% redundancy)
- Instructions like "comprehensively ingest all referenced material before proceeding"
- No forced synthesis step between Catalyst-ingestion and action
- The skill asks the agent to read every file in a directory before acting
- Repeated instructions to "double-check" or "verify by re-reading"

### 2.3 Execution signatures (in the agent's behavior)

- The agent re-reads files it has already read in the session
- The agent runs the same search multiple times with slight variations
- The agent produces verbose intermediate summaries that don't advance the task
- Tool call count is high relative to output volume
- The agent "stalls" — produces lots of activity but no final deliverable
- Time-to-first-output is much longer than expected

### 2.4 What the user says

"The agent keeps re-reading the same file"; "It loops forever before producing anything"; "It's thorough but never finishes"; "It does way more work than necessary."

### 2.5 Intervention: contract the boundary

The intervention is to *reduce permeability* — force the Matrix to synthesize before ingesting more.

**Concrete patterns:**

- **Forced synthesis step:** Insert a required step between Catalyst-ingestion and action: "Before any further reading, write a 3-bullet summary of what you have extracted and what is missing."
- **Read-once directive:** "Never re-read a file you have already read in this session — cite the prior read instead."
- **Progressive disclosure enforcement:** Restructure the skill so that reference files are read *only when explicitly directed* by the SKILL.md, not preemptively.
- **Reference consolidation:** Merge overlapping reference files; remove redundant content.
- **Catalyst budget:** "You may invoke at most N tool calls before producing output. If you exceed N, stop and synthesize what you have."
- **Output-first ordering:** Move the output template before the ingestion instructions, so the agent knows what it is synthesizing toward.

**Do NOT:** Add more instructions. The Matrix is already overloaded. Adding instructions increases the Catalyst load and worsens the addiction.

---

## 3. Shadow 2: Dark-Allergy (Matrix Starvation)

### 3.1 Mechanism

The Matrix's contact-boundary has rigidified — it excludes necessary Catalyst. The agent skips instructions, ignores reference files, treats the skill as optional. The boundary is too impermeable.

### 3.2 Surface signatures (in the skill itself)

- `SKILL.md` is very terse (<100 lines) without progressive disclosure
- Rules stated without examples or reasoning
- No "why" behind instructions — just imperatives
- Brittle conditionals that exclude legitimate edge cases
- No permission to adapt or use judgment
- Reference files exist but the SKILL.md never explicitly directs the agent to read them at the right moment
- The skill assumes context the agent doesn't have

### 3.3 Execution signatures

- The agent skips steps the skill specifies
- The agent doesn't read reference files even when relevant
- The agent treats the skill as a suggestion, not a constraint
- Output is shallow — the agent produces the minimum, not the requested depth
- The agent "doesn't seem to get" what the skill is asking for
- Same failure mode repeats across invocations

### 3.4 What the user says

"The agent ignores half the instructions"; "It doesn't read the reference file"; "It treats the skill as optional"; "It keeps doing X even though the skill says Y."

### 3.5 Intervention: expand the boundary

The intervention is to *increase permeability* — admit more Catalyst, expose the "why", permit adaptation.

**Concrete patterns:**

- **Example injection:** For every rule, add a worked example. Examples are high-gradient Catalyst — they convey pattern that prose cannot.
- **Why-explanations:** For every instruction, explain *why* it matters. The agent's Potentiator needs the "why" to generate appropriate Catalyst for edge cases the skill doesn't anticipate.
- **Reference file directives:** Instead of "see references/foo.md", write "When you encounter X, read references/foo.md §3 before proceeding." The directive must be *triggered* by a specific situation, not optional.
- **Edge case permission:** Explicitly permit adaptation: "If the input does not match any of the above patterns, use your judgment and document your reasoning in the output."
- **Context-priming:** Add a brief preamble that establishes the skill's *purpose* (the Significator), so the agent can infer correct behavior in unstated cases.
- **Adaptive conditionals:** Replace rigid if-then rules with decision frameworks that permit judgment.

**Do NOT:** Add more rules without examples. Rules without examples are low-gradient Catalyst — they bounce off the rigidified boundary.

---

## 4. Shadow 3: Golden-Addiction (Potentiator Flooding)

### 4.1 Mechanism

The Potentiator hyper-generates possibility without structural substrate. The agent hallucinates, fabricates, produces output that *looks* right but isn't grounded in the Catalyst. The contact-boundary is too permeable on the Potentiator side — ungrounded Experience floods out.

### 4.2 Surface signatures (in the skill itself)

- Many open-ended "you may also..." branches without structural requirements
- Output formats that don't enforce verifiability (no citations, no source-tracking)
- Instructions that encourage creativity without requiring grounding
- No verification step between generation and output
- The skill primes the agent with examples of elaborate output without examples of *restrained* output
- "Be creative" or "be comprehensive" instructions without bounding criteria

### 4.3 Execution signatures

- The agent invents features not in the spec
- The agent fabricates citations, URLs, or sources
- Output includes plausible-sounding but unverifiable claims
- The agent over-produces — generates more than was asked, with extra sections
- The agent "hallucinates" tool outputs or file contents
- Output looks polished but doesn't match the input

### 4.4 What the user says

"The agent makes things up"; "It hallucinates features that aren't in the spec"; "It sounds confident but the citations are fake"; "I asked for X and got X plus Y plus Z."

### 4.5 Intervention: contract the boundary (Potentiator side)

The intervention is to *require structural substrate* before the Potentiator can branch.

**Concrete patterns:**

- **Citation requirement:** Every factual claim in the output must cite a specific source (file path + line, URL, tool output). Uncited claims are forbidden.
- **Verification step:** Insert a required step between generation and output: "For each claim in your output, identify the source. If you cannot identify a source, remove the claim."
- **Branching budget:** Limit the number of "you may also..." branches the agent may take per invocation.
- **Structural anchoring:** Before any creative branch, require a structural anchor: "Before generating X, quote the relevant section of the input that justifies generating X."
- **Restraint examples:** Add examples of *restrained* output (doing less, well) alongside examples of elaborate output.
- **Verifiable output format:** Restructure the output template so that every field has a "source" column or citation requirement.

**Do NOT:** Add "do not hallucinate" instructions. The agent is not hallucinating on purpose — the Potentiator is flooding. The fix is structural, not imperative.

---

## 5. Shadow 4: Golden-Allergy (Potentiator Stagnation)

### 5.1 Mechanism

The Potentiator refuses to evolve. The agent outputs the same sub-optimal result every time, treating the skill as a fixed script rather than a reasoning framework. The contact-boundary has rigidified on the Potentiator side — no new possibility is admitted.

### 5.2 Surface signatures (in the skill itself)

- Rigid step-by-step scripts with no branching
- No permission to use judgment
- No examples of *adaptive* behavior — only examples of *following the script*
- The skill's output template is so specific that all outputs look identical
- No decision frameworks — only if-then rules
- The skill has not been updated in many versions, even though the operating environment has changed

### 5.3 Execution signatures

- Output is identical (or near-identical) across very different inputs
- The agent does not adapt to edge cases — it forces them into the template
- The agent does not improve across invocations — same mistakes repeat
- The agent refuses to use tools not explicitly listed in the skill
- Output feels "mechanical" — correct but lifeless

### 5.4 What the user says

"The outputs are all the same"; "It doesn't adapt to different inputs"; "It feels like a script, not a reasoning agent"; "It makes the same mistake every time."

### 5.5 Intervention: expand the boundary (Potentiator side)

The intervention is to *inject novel architectural frameworks* that force the Potentiator to generate new pathways.

**Concrete patterns:**

- **Decision framework injection:** Replace rigid if-then rules with decision frameworks that require judgment: "Classify the input into one of {A, B, C} based on criteria X, Y, Z. Then apply the corresponding pattern, adapting it to the specific input."
- **Adaptive examples:** Add examples where the agent *adapted* the template to a non-standard input, with reasoning.
- **Novel mental model:** Introduce a new mental model or analogy that primes different reasoning pathways. (E.g. for a code-review skill, introduce the "marshalling yard" mental model for thinking about data flow.)
- **Reflection step:** Add a required reflection: "After producing output, identify one way your output could be improved if you had more time. Do not implement the improvement — just note it."
- **Judgment permission:** Explicitly permit judgment: "If the template does not fit the input, adapt the template. Document your adaptation in the output."
- **Comparative reasoning:** Require the agent to consider at least two approaches before committing to one.

**Do NOT:** Add more steps to the script. Stagnation is not solved by more scripting — it is solved by *permission to evolve*.

---

## 6. Shadow 5: Sinkhole of Indifference (Great Way Starvation)

### 6.1 Mechanism

The greater-cycle Great Way is starved of directional Choice. The agent's output is depolarized, generic, non-committal. The skill has lost its *polarization* — it produces output that "anyone could have written". This is the most under-diagnosed shadow because the output "passes" — there's no bug, no hallucination, no missing step.

### 6.2 Surface signatures (in the skill itself)

- The skill's intent is stated as a generic capability ("writes reports", "extracts data", "generates code") rather than a polarized commitment ("turns vague requests into structured PRDs by forcing the user to commit to scope")
- Output templates that permit any content, with no commitment to specific structure or voice
- No examples of *polarized* output — only examples of *correct* output
- The skill has accreted features across versions without committing to a direction
- The skill's "description" field in frontmatter is generic

### 6.3 Execution signatures

- Output is technically correct but generic
- Output could have been written by any competent agent (or human)
- No specific commitments — hedging language ("may", "could", "depending on")
- Template-feeling structure — every output follows the same generic shape
- The user says "it works but feels flat"
- The skill "passes evals" but users don't reach for it

### 6.4 What the user says

"It works but feels flat"; "The outputs are all the same"; "Anyone could have written this"; "It's correct but not useful"; "I don't reach for it anymore."

### 6.5 Intervention: architect Transformation pressure → commit Choice

The intervention is at the *greater* cycle, not the lesser. The skill must be forced to commit to a polarized Choice.

**Concrete patterns:**

- **Significator reconstruction:** Rewrite the skill's core intent as a *polarized* sentence, not a generic capability. (E.g. "writes reports" → "turns vague briefings into decision-ready memos by forcing a recommendation, not a summary".)
- **Choice commitment:** Add a "Choice vector" section to the SKILL.md that names the polarized commitment: "This skill commits to X. It does not do Y, even when asked."
- **Polarized output template:** Restructure the output template so it *requires* commitment. (E.g. "Recommendation: [one specific recommendation, no hedging]".)
- **Anti-hedging rules:** "Do not use the words 'may', 'could', 'depending on', 'various', or 'multiple' in the output. Commit to a specific direction."
- **Polarized examples:** Replace generic examples with examples that show *commitment* — output that takes a stance, makes a recommendation, names a trade-off.
- **Scope pruning:** Remove features that do not serve the polarized Choice. A skill that does three things generically is in the Sinkhole; a skill that does one thing with commitment has escaped it.

**Do NOT:** Add more features. The Sinkhole is not solved by adding capability — it is solved by *committing to a direction and pruning everything that doesn't serve it*.

### 6.6 Distinguishing "structural vulnerability" from "active shadow"

A common diagnostic error: the scanner reports a high score for a lesser-shadow (often S2 or S3) on a skill whose user complaint is purely about genericness (Shadow 5). The auditor then diagnoses an active lesser-shadow and proposes lesser-cycle edits, which dilute the polarized Choice and produce a worse outcome than doing nothing at Phase 1.

The distinction:

- **Structural vulnerability** = the skill has markers that *could* produce a shadow under some conditions, but the user's complaint does not describe that shadow's execution signature. Example: a 50-line skill with no examples is *structurally vulnerable* to S2, but if the user says "the output is generic", S2 is not *active* — the agent is following the (terse) instructions correctly, the output is just depolarized.
- **Active shadow** = the skill has markers AND the user's complaint describes that shadow's execution signature. Example: a 50-line skill with no examples, AND the user says "the agent skips the validation step half the time" — S2 is *active*.

**Decision rule:** When the scanner reports a high lesser-shadow score but the user's complaint is purely about genericness, polarization, or "feel" (Shadow 5 vocabulary), record the lesser-shadow as a *structural vulnerability*, not an *active shadow*. Do not propose lesser-cycle edits. Proceed to Phase 2 and let the Choice vector drive the rewrite.

This is the single most important diagnostic discipline in the protocol. Over-diagnosing lesser-shadows when only Shadow 5 is active produces an over-expansion that dilutes the polarized Choice — the worst possible outcome, because the skill ends up both longer AND still generic.

---

## 7. Co-active Shadows

In practice, multiple shadows are often co-active. The most common combinations:

### 7.1 Shadow 1 + Shadow 5 (Dark-Addiction + Sinkhole of Indifference)

The skill is bloated *and* generic. The agent loops re-reading context, then produces a generic output that doesn't commit to anything.

**Intervention order:** Resolve Shadow 5 first (commit to a Choice), then resolve Shadow 1 (contract the boundary to serve the Choice). If you resolve Shadow 1 first, you may produce a *faster* generic skill — same Sinkhole, less looping.

### 7.2 Shadow 2 + Shadow 4 (Dark-Allergy + Golden-Allergy)

The skill is too terse *and* too rigid. The agent skips instructions and produces mechanical output.

**Intervention order:** Resolve Shadow 2 first (expand with examples and "why"), then resolve Shadow 4 (inject frameworks that permit adaptation). The Shadow 2 expansion often naturally addresses Shadow 4 — examples are themselves Potentiator-expanders.

### 7.3 Shadow 3 + Shadow 5 (Golden-Addiction + Sinkhole of Indifference)

The agent hallucinates *and* produces generic output. This is the worst combination — confident-sounding fabrication that commits to nothing.

**Intervention order:** Resolve Shadow 3 first (require structural substrate), then resolve Shadow 5 (commit to a Choice). Resolving Shadow 5 first without Shadow 3 produces a *polarized hallucination* — worse than either alone.

### 7.4 Shadow 1 + Shadow 3 (Dark-Addiction + Golden-Addiction)

The agent loops re-reading, then hallucinates based on the overloaded context. Common in skills that ask the agent to "comprehensively ingest" large reference corpora.

**Intervention order:** Resolve Shadow 1 first (contract ingestion). Often the hallucination resolves on its own — the Potentiator was flooding because the Matrix was overloaded with un-synthesized Catalyst.

---

## 8. Diagnostic decision tree

When the diagnosis is ambiguous, walk this tree:

1. **Does the agent produce output at all?**
   - No (loops, stalls) → Shadow 1 (Dark-Addiction)
   - Yes, but wrong → continue
2. **Does the output match the input?**
   - No (hallucinated, fabricated) → Shadow 3 (Golden-Addiction)
   - Yes → continue
3. **Does the agent follow the skill's instructions?**
   - No (skips steps, ignores references) → Shadow 2 (Dark-Allergy)
   - Yes → continue
4. **Does the output vary across different inputs?**
   - No (mechanical, identical) → Shadow 4 (Golden-Allergy)
   - Yes → continue
5. **Does the output commit to a specific direction?**
   - No (generic, hedging, anyone-could-have-written) → Shadow 5 (Sinkhole of Indifference)
   - Yes → the skill is healthy; the user's complaint may be about something else

If you reach step 5 and the skill appears healthy, ask the user for a specific failing input. The skill may be healthy *on average* but fail on specific inputs — that's a localized holon defect, not a systemic shadow.

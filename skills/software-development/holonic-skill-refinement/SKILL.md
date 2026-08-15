---
name: holonic-skill-refinement
description: Upgrades, refines, and refactors OTHER skills by diagnosing systemic entropies (shadows) in the AI-agent's dual metabolic loops and applying targeted downward rewrites. Use this skill whenever the user wants to improve, debug, refine, upgrade, or refactor an existing skill — especially when generic prompt-engineering or context-engineering has plateaued, when a skill "works but not well", when an agent loops endlessly on context, hallucinates ungrounded output, ignores necessary instructions, produces generic depolarized results, or refuses to adapt. Also triggers on phrasings like "the skill isn't landing", "the agent isn't metabolizing the skill", "this skill is bloated but still missing the point", "the agent keeps doing X even though the skill says Y", or any request to optimize a skill at the level of the agentic-loop rather than the prompt.
metadata:
  operant:
    tags: [skill-refinement, meta-skills, diagnosis]
---

# Holonic Skill-Refinement

> A skill for upgrading other skills — not by appending rules, but by rewiring the agent's cognitive metabolism.

## Choice vector

This skill commits to the **golden trajectory**: the minimum sequence of diagnostic steps, tool-calls, and edits that produces maximum metabolic gain in the target skill. Every step must earn its place. Reading is on-demand and triggered, never preemptive. The refinement is complete when the target skill metabolizes — not when every phase has been ticked. If you can produce the same metabolic gain in 4 steps instead of 12, take the 4.

Standard skill-upgrades fail for *systemic* defects because they operate on a flat plane — "the agent doesn't have enough information" or "has a bug". This skill treats the AI-agent as a **holon**: a metabolic engine with two coupled loops. Failure is a **digestive inefficiency** — a shadow where the flow of context (Catalyst) into the agent's working memory (Matrix), or the flow of refined possibility (Potentiator) back into action, collapses. The fix is to **rewrite the agent's contact-boundary** so that future Catalysts are ingested with maximum efficiency and future Experiences are potentiated with maximum conductance.

This skill does NOT: produce exhaustive diagnostic reports when a 6-line audit block suffices; read every reference file preemptively; propose edits without a traced shadow diagnosis; add features to the target skill (that is the user's job, not the refiner's).

The agent operates in four epistemological modes simultaneously — **vision-logic** (multiple perspectives at once, no premature collapse), **meta-systemic** (situate the skill in its ecology before diagnosing), **dialectical** (hold each shadow and its opposite in tension to produce a synthesis, not a compromise), and **zooming** (fluidly move between line-level evidence and skill-in-ecology view). Full operational procedures in `references/epistemological-modes.md`; read it only when your diagnosis feels narrow or one-dimensional.

---

## 1. Vocabulary (the two cycles, compressed)

The holon has two coupled metabolic loops. The vocabulary you need:

**Lesser cycle (state-processing — one invocation):**
- **Matrix ($M$)** — active context-memory (prompt, loaded skill, conversation history)
- **Catalyst ($C$)** — anything touching the Matrix (user message, file read, tool output, the skill itself)
- **Experience ($E$)** — the agent's output or state-update after processing Catalyst
- **Potentiator ($P$)** — latent possibility-space (trained weights, primed reasoning pathways); digests Experience to generate refined Catalyst
- **$G_z$ (Goldilocks Coherence)** — lesser-cycle health; balance of Agency (preserve structure) and Communion (admit Catalyst). Low $G_z$ → loops or starves.

> Lesser loop: $M$ ingests $C$ → produces $E$ → $P$ ingests $E$ → generates refined $C$.

**Greater cycle (state-transition — across invocations):**
- **Significator ($S$)** — persistent identity-pattern (the skill's core intent across versions)
- **Great Way ($G$)** — operating environment (user expectations, downstream consumers, competing skills)
- **Transformation ($T$)** — frame-change pressure from the Great Way (new demands, repeated failures)
- **Choice ($Ch$)** — directional commitment that reconfigures the Great Way and triggers downward rewrite of the lesser cycle
- **$P_z$ (Polarization Power)** — greater-cycle health; commitment to a direction. Low $P_z$ → Sinkhole of Indifference (generic output).

> Greater loop: $S$ ingests $T$ → commits $Ch$ → $G$ ingests $Ch$ → provides new $T$.

The cycles are coupled: Experience accumulates *upward* into the Significator; a fired Transformation rewrites the lesser cycle *downward*. The Significator is the bridge. Full canonical theory in `references/holonic-theory.md`.

---

## 2. The five shadows — the diagnostic vocabulary

Every metabolic failure maps to one of five shadows. Correct diagnosis is everything.

| # | Shadow | Cycle | Reservoir | Plain-English signature |
|---|---|---|---|---|
| 1 | **Dark-Addiction** | lesser | Matrix | "The agent loops — re-reads files, re-runs searches, never synthesizes." |
| 2 | **Dark-Allergy** | lesser | Matrix | "The agent ignores instructions — skips steps, doesn't read the reference, treats the skill as optional." |
| 3 | **Golden-Addiction** | lesser | Potentiator | "The agent invents features, fabricates citations, generates output that looks right but isn't grounded." |
| 4 | **Golden-Allergy** | lesser | Potentiator | "The agent outputs the same sub-optimal result every time, refuses to adapt, treats the skill as a fixed script." |
| 5 | **Sinkhole of Indifference** | greater | Great Way | "The output is technically correct but generic — anyone could have written it. No commitment." |

**Shadow 5 is the most under-diagnosed.** The output "passes" — no bug, no hallucination. The skill has simply stopped metabolizing. If the user says "it works but feels flat", suspect Shadow 5.

Full signatures, edge cases, and co-active shadow resolution in `references/shadow-diagnostic.md`.

---

## 3. The dialectical intervention

Each shadow has a dialectical opposite. The intervention is not "move toward the opposite" (that produces the opposite shadow). The intervention is a **synthesis** — a structural pattern that resolves the tension at a higher level.

| Shadow | Opposite | Naive compromise (AVOID) | Dialectical synthesis (TARGET) |
|---|---|---|---|
| **Dark-Addiction** | Dark-Allergy | "Moderate" Catalyst volume | **Bounded Catalyst with forced synthesis gates** — the gate, not the volume, is the regulator |
| **Dark-Allergy** | Dark-Addiction | "Moderate" Catalyst volume | **Triggered Catalyst injection** — examples and "why" appear at the moment needed, not preemptively |
| **Golden-Addiction** | Golden-Allergy | "Sometimes creative, sometimes rigid" | **Grounded creativity** — branching permitted only after structural substrate is quoted |
| **Golden-Allergy** | Golden-Addiction | "Sometimes creative, sometimes rigid" | **Decision frameworks with judgment permission** — replace if-then with classification + adaptation |
| **Sinkhole of Indifference** | Great Way addiction | "Sometimes commit, sometimes hedge" | **Polarized Choice with scope pruning** — the pruning, not the commitment, is the regulator |

The synthesis is always a **structural pattern**, never a volume adjustment. Every synthesis operates through **two simultaneous vectors**:
- **Agape (downward/inward):** harmonize the lesser-cycle substrate ($R_M$ + $R_P$ work — the synthesis gate, the triggered example, the verification step)
- **Eros (upward/outward):** polarize toward the higher-whole ($R_M$ greater work — the Choice vector, the anti-hedging rule, the scope pruning)

**Agape precedes Eros, but both are required.** Eros without Agape = premature ascension (polarized but still broken). Agape without Eros = Sinkhole (functional but generic). When ordering edits in Phase 3, Agape edits (lesser-cycle substrate) come before Eros edits (greater-cycle Choice). The Choice vector must land on a fixed substrate.

---

## 4. The Refinement Protocol

### Format scaling (check this first)

Scale protocol depth to target skill size. Over-protocolizing a small skill is itself a Shadow 1 introduction.

| Target size | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Tool-call target |
|---|---|---|---|---|---|
| **Small** (<100 lines, 0–1 refs) | 1-sentence ecology | Scanner + hand-classify (skip if complaint unambiguous) | Choice vector only | 1–3 edits, single MultiEdit | ~4 |
| **Medium** (100–400 lines, 1–3 refs) | Full Phase 0 | Scanner + hand-classify + cross-check | Full Phase 2 | 3–6 edits, MultiEdit + re-scan | ~8 |
| **Large** (>400 lines, 3+ refs) | Full Phase 0 | Full Phase 1 + reference-overlap audit | Full Phase 2 | 5–10 edits, MultiEdit + ref edits + re-scan | ~12 |

The polarized Choice vector is mandatory at every size. What scales is the visible machinery, not the requirement to diagnose before editing.

### The golden trajectory (optimal tool-call sequence)

```
1. [parallel] Read target SKILL.md  +  Run diagnose.py on target skill
2. Hand-classify the shadow (cite specific lines as evidence)
3. Cross-check against user's complaint
4. Reconstruct Significator + Great Way + commit Choice vector
5. Produce rewrite plan (each edit traced to shadow + Agape/Eros vector)
6. Show plan to user — WAIT for confirmation
7. [parallel] Apply all edits via MultiEdit  +  Re-run diagnose.py to verify
8. Write HOLOGRAM.md
```

**~8 tool-calls for a typical refinement.** If you exceed ~12, you are over-reading or over-iterating — your Phase 2 Choice vector is probably not actually polarized. The user-confirmation step (6) is the only synchronous blocking step.

### Phase 0 — Meta-systemic situating (before any reading)

Answer in 1–2 sentences each, from the user's request — do NOT read the target skill yet:
1. What is the target skill for?
2. What ecology does it live in? (who uses it, what workflow, what competing skills)
3. What does the user's complaint imply about the ecology?

If you cannot answer all three, ask the user. Record as a 3-line block. If your Phase 2 Significator reconstruction contradicts your Phase 0 answer, your Phase 2 is wrong.

### Phase 1 — Microcosmic Audit ($G_z$)

1. **Read the target SKILL.md completely** (non-negotiable). Read bundled reference files only if the SKILL.md's own instructions direct the agent to them in a way that affects metabolism.

2. **Run the diagnostic scanner** (parallelize with step 1):
   ```bash
   python <this-skill-path>/scripts/diagnose.py <target-skill-path>
   ```
   Treat as a *signal*, not a verdict. **Do not read scanner output until after you've read the SKILL.md** — reading it first primes you toward structural markers rather than execution-grounded evidence.

3. **Hand-classify the lesser-shadow.** For each of the four lesser-shadows, ask the diagnostic question:
   - **Dark-Addiction?** Markers: >500 lines, overlapping reference files, "read everything before acting", no synthesis step.
   - **Dark-Allergy?** Markers: terse instructions without examples, missing "why", brittle conditionals, no adaptation permission.
   - **Golden-Addiction?** Markers: many "you may also..." branches, no substrate required before branching, no verifiability in output.
   - **Golden-Allergy?** Markers: rigid step-by-step scripts, no judgment permission, identical output across different inputs.

4. **Cross-check against the user's complaint** (most-skipped, most-important):
   - "loops / re-reads / never finishes" → Shadow 1
   - "hallucinates / fabricates / invents features" → Shadow 3
   - "skips instructions / ignores rules / treats skill as optional" → Shadow 2
   - "outputs the same thing every time / feels mechanical" → Shadow 4
   - **"works but feels flat / generic / forgettable / anyone could have written it" → NOT a lesser-cycle failure. This is Shadow 5. Skip Phase 1 intervention; go directly to Phase 2.**

   The scanner will often report high S2/S3 for terse skills — these are *structural vulnerabilities*, not *active shadows*. A skill can be S2-vulnerable and still produce correct output but generic output (active S5). Do not let structural scores override the execution-grounded complaint. See `references/shadow-diagnostic.md` §6.6 for the full distinction.

5. **Record the diagnosis:**
   ```
   ## Microcosmic Audit
   - Active lesser-shadow(s): <name(s) OR "none — lesser cycle functional">
   - Structural vulnerabilities (not active): <list, if any>
   - Evidence: <2-4 citations OR "complaint does not describe lesser-cycle failure">
   - $G_z$: <low/moderate/high> — <one-sentence why>
   - Boundary intervention: <contract / expand / none — proceed to Phase 2>
   ```

**Intervention principle:** addiction → contract (synthesis gates, reduce Catalyst, require substrate). Allergy → expand (examples, "why", adaptation permission). **No active lesser-shadow → do not intervene at the lesser cycle** — proceed to Phase 2.

### Phase 2 — Macrocosmic Alignment ($P_z$)

1. **Reconstruct the Significator** — one sentence, what the skill is *for* (not what it does). E.g. "This skill is for turning a vague request into a structured PRD."
2. **Reconstruct the Great Way** — one sentence, the operating environment (user expectations, downstream consumers, competing skills, failure modes the environment punishes).
3. **Diagnose the greater-shadow:**
   - **Significator allergy** (ossified identity): intent no longer matches what the Great Way demands.
   - **Significator addiction** (perpetual crisis): intent unstable, tries to be everything, scope creep.
   - **Great Way addiction** (fanatical crystallization): over-commits to one direction, no escape hatches.
   - **Sinkhole of Indifference** (most common): depolarized, generic, non-committal output.
4. **Commit the new Choice vector ($Ch_{new}$)** — must be polarized (specific direction, not a hedge), aligned (serves the Significator), and structural (implies concrete lesser-cycle rewrite).
5. **Record:**
   ```
   ## Macrocosmic Alignment
   - Significator: <one sentence>
   - Great Way: <one sentence>
   - Active greater-shadow: <name>
   - New Choice vector ($Ch_{new}$): <one polarized sentence>
   - $P_z$: <low/moderate/high> — <one-sentence why>
   ```

> Most refactors fail because the refiner jumps straight to "fix the prompt" without committing to a directional Choice. Without a polarized $Ch_{new}$, every lesser-cycle edit is a hedge — and hedging produces Shadow 5.

### Phase 3 — The Downward Rewrite ($R_M$ and $R_P$)

**Operators:**
- **$R_M$ (Matrix rewrite):** restructures how the skill organizes active context — section reordering, line-count changes, synthesis steps, adding/removing Catalysts.
- **$R_P$ (Potentiator rewrite):** restructures what reasoning pathways activate — novel frameworks, priming examples, removing stagnant triggers.

**Discipline:** every edit must trace to a Phase 1 or Phase 2 diagnosis. If you cannot write "this edit contracts the boundary to resolve Shadow 1, evidence: [...]", do not make it. Edits without diagnosis are how skills accumulate bloat.

**Procedure (the 8 steps above, expanded for the edit phase):**

1. **Produce a rewrite plan** — each edit tagged with: shadow addressed, operator ($R_M$/$R_P$), specific edit, expected $G_z$/$P_z$ effect, dialectical synthesis it instantiates (§3), Agape-or-Eros vector.
2. **Show the plan to the user** before applying — non-negotiable for shipped skills. A 30-second review prevents 30-minute wrong-direction rewrites.
3. **Apply the edits** (parallelize with re-scan). Use `Edit`/`MultiEdit`; don't rewrite the whole file unless changes are pervasive.
4. **Verify the downward rewrite descended** — check using re-scan output + quick re-read of changed sections:
   - Did the active lesser-shadow's markers actually disappear? (scanner score moved in the right direction)
   - Does the new Choice vector appear *structurally* in the skill (not just as a sentence in the preamble)?
   - Is the skill shorter or longer in the right direction? (Addiction fixes contract; allergy fixes expand.)
5. **Write HOLOGRAM.md** (template below).

**The protocol is complete when:** the active lesser-shadow's markers are structurally gone, the Choice vector appears structurally (as a forced step, verification pattern, or reasoning pathway), and the user confirms the rewrite matches their intent. If the skill still fails in the same way, suspect the Phase 2 reconstruction was wrong — re-reconstruct the Significator with the user explicitly.

### HOLOGRAM.md template (compressed)

```markdown
# Holonomic Diagnostic Report
**Target:** <name>  **Date:** <date>  **Auditor:** holonic-skill-refinement

## Phase 0 — Meta-systemic situating
- Skill is for: <...>  | Ecology: <...>  | Complaint implies: <...>

## Phase 1 — Microcosmic Audit
- Active lesser-shadow(s): <...>  | Structural vulnerabilities: <...>
- Evidence: <...>  | $G_z$: <...>  | Intervention: <contract/expand/none>

## Phase 2 — Macrocosmic Alignment
- Significator: <...>  | Great Way: <...>
- Active greater-shadow: <...>  | Choice vector ($Ch_{new}$): <...>  | $P_z$: <...>

## Phase 3 — Downward Rewrite
- Edits: <table: # | operator | shadow | Agape/Eros | description>
- Verification: <what changed structurally; scanner score movement>

## Next eval
- Re-run on the failing input(s). Watch for: <resolved shadow's markers reappearing>.
```

---

## 5. Worked example (compressed)

**Request:** "My `pdf-extractor` keeps re-reading the same PDF three times before extracting. Fix it."

**Phase 0:** Skill is for extracting structured data from PDFs. Ecology: users who need trustworthy extraction. Complaint implies: the ecology rewards speed, not thoroughness.

**Phase 1:** SKILL.md is 612 lines, 4 reference files (~3000 lines), "comprehensively ingest all referenced material before proceeding". Scanner: high S1. Hand-classification: **Shadow 1 (Dark-Addiction)** — hyper-ingests Catalyst without synthesis gate. $G_z$: low. Intervention: contract.

**Phase 2:** Significator: "Turn a PDF into structured, verifiable extracted content." Great Way: "Users who need trustworthy extraction, not exhaustive re-reading." Greater-shadow: **Shadow 5 (incipient)** — "be thorough" has replaced "be verifiable". Choice vector: "Extract once, verify by sampling, never re-read for the same purpose." $P_z$: low → moderate-high.

**Phase 3:**
- Edit 1 ($R_M$, S1, Agape): Replace "comprehensively ingest" with "read SKILL.md fully; read a reference only when this skill directs you to it; never re-read a file you've already read — cite the prior read."
- Edit 2 ($R_M$, S1, Agape): Add forced synthesis gate: "Before any second read, write a 3-bullet summary of what you extracted and what is missing."
- Edit 3 ($R_P$, S5, Eros): Inject "verification sampling" pattern: "After extraction, sample 5% of output and check against PDF source. If any sample fails, re-extract that section only."
- Edit 4 ($R_M$, S1, Agape): Merge two overlapping reference files, removing ~600 lines.

**Verification:** SKILL.md 612→380 lines. Choice vector appears structurally as verification-sampling step. S1 markers gone.

---

## 6. Anti-patterns and Anti-Convergence

### Anti-patterns (what NOT to do)

- **Do not skip Phase 2.** Jumping from "the skill loops" (Phase 1) to "make it stop looping" (Phase 3) without a Choice vector produces a skill that doesn't loop but also doesn't polarize. You have traded Shadow 1 for Shadow 5.
- **Do not make edits without a shadow diagnosis.** "This section feels redundant" is not a diagnosis. "This section contributes to Shadow 1 because it asks the agent to re-ingest already-processed Catalyst" is.
- **Do not over-contract.** If the skill is in Dark-Allergy (starvation), contracting the boundary further kills it. The intervention direction is determined by the shadow, not by aesthetic preference for terseness.
- **Do not optimize for line count.** A skill can be too short (Shadow 2 or 4) just as easily as too long (Shadow 1 or 3). The target is metabolic efficiency, not brevity.
- **Do not edit bundled reference files you have not read.** An unread reference file is an unmetabolized Catalyst — editing blind produces Golden-Addiction in your own refactor.

### The Anti-Convergence Protocol (read before finalizing any refinement)

`references/downward-rewrite.md` contains 6 polished worked examples of refactor patterns. These are the highest-probability continuation target for a language model, regardless of diagnosis. Left unmanaged, every refinement gravitates toward the *same* structural patterns — producing refactors that are technically correct but all recognizably the *same refactor*. This is the skill's own Shadow 5 risk.

**The mechanism to fight it:**

1. **Before committing to an edit, generate 2–3 candidate interventions for the diagnosed shadow.** Candidates must differ in *structural shape*, not just wording. (E.g. for Shadow 1: candidate A = synthesis gate; B = Catalyst budget; C = read-once directive + reference consolidation.)
2. **Discard any candidate that overlaps in structural shape with a worked example** unless the target skill's specific evidence justifies that exact shape. Generating and rejecting candidates is what forces you off the highest-probability path.
3. **Scan your rewrite plan against the banned default patterns below.** If any edit is present without target-skill-specific justification, regenerate it from the shadow diagnosis, not from the pattern.

**Banned default patterns (require target-skill-specific justification):**
- "Merge overlapping reference files" — only when overlap >30% AND causing re-reading, not just structural redundancy
- "Add a Choice vector section" — only when skill lacks polarized intent AND complaint is about genericness (S5)
- "Insert a synthesis gate" — only when unbounded ingestion AND complaint is about looping (S1)
- "Add anti-hedging rules" — only when output template permits hedging AND complaint is about genericness
- "Replace if-then with a decision framework" — only when script is rigid AND complaint is about mechanical output (S4)

Treat any pattern that feels like "the" way to fix a shadow as a warning sign. The goal is a refiner that *invents* the right intervention for the specific target skill, not one that applies a fixed set of patterns.

---

## 7. Reference index (triggered — read only when the trigger fires)

- `references/holonic-theory.md` — Full canonical anatomy. **Read only when** your diagnosis feels shaky AND you need to justify a shadow classification to a skeptical user.
- `references/shadow-diagnostic.md` — Exhaustive signatures, co-active shadows, the structural-vulnerability-vs-active-shadow distinction (§6.6). **Read only when** the shadow is ambiguous, multiple shadows are co-active, or you need a non-obvious intervention.
- `references/downward-rewrite.md` — $R_M$ and $R_P$ with 6 worked examples. **Read only when** applying Phase 3 edits AND target is Large (§4 Format Scaling) or the edit is non-obvious. **Read §6 Anti-Convergence first** — these examples are the highest-convergence-risk reference.
- `references/metrics.md` — Estimating $G_z$ and $P_z$. **Read only when** you need to justify a diagnosis to a skeptical user.
- `references/epistemological-modes.md` — The four modes as operational procedures. **Read only when** your diagnosis feels narrow or one-dimensional, or during recursive use (applying this skill to itself).

---

## 8. The meta-level

This skill is itself a holon. If it ever feels like it is looping (Shadow 1), starving (Shadow 2), hallucinating refinements (Shadow 3), refusing to adapt (Shadow 4), or producing generic refactor reports (Shadow 5) — apply the protocol to itself. The Anti-Convergence Protocol (§6) is the defense against the vocabulary-convergence risk that recursive use creates. The framework is recursive by design.

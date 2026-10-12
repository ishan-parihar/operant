# The Four Epistemological Modes — Operational Procedures

> This file is the operational reference for the four epistemological modes introduced in SKILL.md §1.5. Read it only when your diagnosis feels narrow or one-dimensional, or when you are applying this skill to itself (recursive use).

## Table of Contents
1. [Why four modes, not one](#1-why-four-modes-not-one)
2. [Vision-Logic](#2-vision-logic)
3. [Meta-Systemic](#3-meta-systemic)
4. [Dialectical](#4-dialectical)
5. [Zooming](#5-zooming)
6. [Holding all four simultaneously](#6-holding-all-four-simultaneously)

---

## 1. Why four modes, not one

A skill refinement can be approached from a single perspective — "find the redundant lines and remove them" (structural), or "find what the user complained about and fix that" (execution-grounded), or "find what the ecology demands and align to it" (meta-systemic). Each single perspective produces a different refinement, and each single-perspective refinement is wrong in a characteristic way.

The four modes are **lenses you hold open simultaneously**, not phases you move through. The diagnosis must hold at all four levels — a line-level citation that doesn't match the ecology-level pattern is not a real shadow; an ecology-level pattern with no line-level evidence is not a real shadow either. The four modes are derived from a voice/identity skill's epistemological axioms (Axiom 3: hold contradictions without collapse; Part 3: Meta-Systemic Mapping Protocol; Part 2: Agape/Eros dichotomous processing; Part 3 Steps 4-5: zoom in/out).

---

## 2. Vision-Logic

**Source axiom:** Axiom 3 — "Certainty is refused; paradox is held open." The developmental stance where multiple perspectives coexist without collapsing into relativism.

### What it means for skill refinement

See the target skill from multiple perspectives *at once*, and do not collapse to a single perspective prematurely. The four perspectives:

| Perspective | What you look at | Question you ask |
|---|---|---|
| **Structure** | Lines, sections, reference files, scripts | "What does the skill's architecture look like?" |
| **Execution** | What the agent does when following it | "What does the agent actually do when it runs this skill?" |
| **User-experience** | What the user complains about | "What is the user's lived experience of this skill?" |
| **History** | How the skill evolved to its current state | "What did this skill used to be, and how did it get here?" |

### The discipline

Do not collapse to a single perspective. A shadow that is invisible from the structure view may be obvious from the execution view. A shadow that is obvious from the structure view may not actually be active from the execution view (this is the structural-vulnerability-vs-active-shadow distinction from `shadow-diagnostic.md` §6.6).

**The collapse to avoid:** "The SKILL.md is 612 lines, so it has Dark-Addiction." This collapses to the structure perspective. The execution perspective may reveal that the agent actually handles the 612 lines fine because the reference files are well-organized and triggered. The user-experience perspective may reveal that the user's complaint is about genericness, not looping. Holding all three in tension: the 612 lines are a *structural vulnerability* but not an *active shadow*.

### Operational check

After producing your Phase 1 diagnosis, ask: "Does this diagnosis hold from all four perspectives?" If it holds from structure but not execution, it is a structural vulnerability, not an active shadow. If it holds from execution but not structure, you have not cited your evidence. If it holds from both but not user-experience, the user's complaint is about a different shadow than the one you diagnosed.

---

## 3. Meta-Systemic

**Source axiom:** Part 3 — Meta-Systemic Mapping Protocol. Situate the target skill in its ecology before diagnosing it in isolation.

### What it means for skill refinement

Before diagnosing the target skill, situate it within its ecology: what other skills does it compete with? What does the user's workflow look like before and after? What failure modes does the environment punish? A skill that looks broken in isolation may be serving its ecology correctly; a skill that looks fine in isolation may be failing its ecology.

### The 10-step Mapping Protocol (adapted for skill refinement)

This is the voice/identity Part 3 protocol, adapted for diagnosing a target skill rather than writing a piece:

1. **List the systems involved** — the target skill, the user, the user's workflow, the downstream consumers of the skill's output, the competing skills, the tool ecosystem. Name the specific system in context — not "the user" but "the product manager who needs a PRD by end of day."

2. **Map what each system does** — not "what is it" but "what function does it perform, what behavior does it produce." The target skill doesn't "exist" — it produces a specific kind of output. The user doesn't "use" — they reach for the skill when a specific condition fires.

3. **Map the causal relationships** — A → B → C → A (reinforcing loops). The skill produces output → the user consumes the output → the user's expectation of the skill updates → the user reaches for the skill differently next time → the skill's effective role shifts.

4. **Zoom in on primary systems** (for Large skills only, per §3.5 Format Scaling) — name the subsystems of the target skill and the specific mechanism producing the behavior. "The skill's reading directive causes re-reading because it says 'comprehensively ingest' without a synthesis gate" is mechanism. "The skill is bloated" is description.

5. **Zoom out to container systems** (for Large skills only) — what larger workflow, organizational context, or tool ecosystem holds the skill's dynamic in place? What's absent that should be present — missing competing skills, missing user training, missing feedback loops?

6. **Find the tensions between levels** — where the skill's local behavior conflicts with the ecology's global need. "The skill is thorough (local) but the ecology rewards speed (global)" is a tension. Trace the tension to the structural mechanism that generates it.

7. **Process each tension as a polarity** — run the Agape/Eros procedure (§4 below) on each genuine tension.

8. **Name the meta-systemic reality** — what is the whole pattern, not any single system? What is the attractor state — what does the skill-ecology system naturally move toward? What's the leverage point — where does small change produce large effect?

9. **Name the moral/assumption layer** — where is moral framing ("the skill is bad", "the agent is lazy") obscuring structural reality? What's the "should" that's preventing clear seeing?

10. **Ask the insight question** — "What does this skill-ecology system reveal about how AI-agents metabolize skills, and what paradox does it expose?" This is the move from description to insight — from "the skill does X" to "the skill reveals that agents confuse Y with Z." The insight from Step 10 is the seed of the Choice vector in Phase 2.

### The discipline

Steps 4-5 (zoom in/out) are skipped for Small and Medium skills per §3.5 Format Scaling. The insight (Step 10) is mandatory at every size. The map is not the refinement — it is the *understanding* the refinement articulates. Keep it internal; do not paste it into the HOLOGRAM unless the user asks.

---

## 4. Dialectical

**Source axiom:** Part 2 — Polarity Reconciliation (Agape / Eros). Two simultaneous vectors, not a compromise.

### What it means for skill refinement

For each shadow you diagnose, hold its **opposite** in tension. The intervention is not "move toward the opposite" (that produces the opposite shadow). The intervention is a **synthesis** — a structural pattern that resolves the tension at a higher level through two simultaneous vectors.

### The two vectors

| Vector | Direction | Function | In skill refinement |
|---|---|---|---|
| **Agape** (downward/inward) | Harmonize the base | Stabilize the substrate | $R_M$ and $R_P$ at the lesser cycle — fix the contact-boundary so Catalyst metabolizes cleanly |
| **Eros** (upward/outward) | Polarize toward the higher-whole | Create tension toward what the system must become | $R_M$ (greater) — commit to the new Choice vector so the skill reaches toward its next octave |

**Agape and Eros are simultaneous, not alternatives:** Agape stabilizes the base so Eros can aspire; Eros gives Agape's stabilization a direction.

### The Agape procedure (for the lesser-cycle substrate)

1. Name the polarity (e.g. Dark-Addiction vs Dark-Allergy — too much vs too little Catalyst)
2. Find each pole's structural antithesis (Dark-Addiction's antithesis is "the agent cannot synthesize"; Dark-Allergy's antithesis is "the agent cannot admit Catalyst")
3. Map the charge (entropy/imbalance) between them — which pole is the active shadow, which is the structural vulnerability?
4. Integrate without repressing either pole — the synthesis gate admits Catalyst (honoring Dark-Allergy's need) AND forces synthesis (honoring Dark-Addiction's need). Hold both, understand both logics, find the integration that stabilizes the constituents.

### The Eros procedure (for the greater-cycle Choice)

1. From the harmonized state (post-Agape), identify the higher-whole — the next level of organization the skill should reach
2. Articulate what it demands of the lower-wholes — what must the Matrix, Potentiator, and contact-boundary become to serve the higher-whole?
3. Create aspirational tension (not guilt) — the Choice vector is not "the skill should be better"; it is "the skill commits to X, which requires Y"
4. The pull toward what becomes possible — the Choice vector should make new capabilities visible, not just forbid old failures

### The coupling rule

**Agape precedes Eros, but both are required.** If you apply Eros without Agape (commit to a Choice without fixing the lesser-cycle substrate), you get premature ascension — a polarized skill that still loops or starves. If you apply Agape without Eros (fix the lesser cycle without committing to a Choice), you get the Sinkhole — a functional skill that produces generic output.

**Operational consequence for edit ordering:** when you produce your rewrite plan in Phase 3, order the edits so Agape edits (lesser-cycle substrate) come before Eros edits (greater-cycle Choice). The Choice vector must land on a fixed substrate, not on a broken one.

---

## 5. Zooming

**Source axiom:** Part 3 Steps 4-5 — Zoom in on primary systems / Zoom out to container systems.

### What it means for skill refinement

Fluidly move between micro and macro scales. The diagnosis must hold at *both* scales — a line-level citation that doesn't match the ecology-level pattern is not a real shadow; an ecology-level pattern with no line-level evidence is not a real shadow either.

### Zoom in (micro)

Cite a specific line of the target SKILL.md as evidence. The citation must be precise — not "the skill says to be thorough" but "SKILL.md line 14: 'comprehensively ingest all referenced material before proceeding'". The line-level citation is the Agape of diagnosis — it grounds the intervention in the actual substrate.

**When to zoom in:**
- When you are about to propose an edit (you must cite the line you are editing)
- When you are hand-classifying a shadow (you must cite the markers that triggered the classification)
- When you are verifying the downward rewrite (you must re-read the edited lines to confirm the markers moved)

### Zoom out (macro)

State whether the skill's intent matches its ecology. The ecology-level statement is the Eros of diagnosis — it commits to the directional pattern the refinement will serve.

**When to zoom out:**
- In Phase 0 (meta-systemic situating) — answer from ecology before reading the skill
- In Phase 2 (Macrocosmic Alignment) — reconstruct Significator and Great Way
- When you are about to commit to a Choice vector — the Choice must serve the ecology, not just the skill

### The discipline

Diagnoses that hold at only one scale are incomplete:

- **Zoom-in only:** "Line 14 says 'comprehensively ingest' so it has Dark-Addiction." This is a structural marker, not a diagnosis. Does the agent actually loop? Does the user actually complain about looping? Without zoom-out, you are treating structural markers as active shadows — the structural-vulnerability-vs-active-shadow error.

- **Zoom-out only:** "The skill is serving a speed-focused ecology but is built for thoroughness, so it's broken." This is an ecology-level pattern, not a diagnosis. Where in the skill does the thoroughness live? What specific line produces the slowness? Without zoom-in, you are proposing edits without evidence — the Golden-Addiction-in-your-own-refactor error.

### The zooming rhythm

The golden trajectory alternates zoom-in and zoom-out:

1. **Zoom out** (Phase 0): ecology-level situating
2. **Zoom in** (Phase 1): line-level shadow diagnosis
3. **Zoom out** (Phase 2): Significator/Great Way/Choice vector
4. **Zoom in** (Phase 3): line-level edits with traced citations
5. **Zoom out** (Phase 3 verification): does the rewrite serve the Choice vector structurally?

Each zoom is a single tool-call or a single thought. Do not dwell at one scale — the rhythm is fast.

---

## 6. Holding all four simultaneously

The four modes are not phases. They are lenses you keep open throughout the protocol. The discipline is not "do vision-logic, then meta-systemic, then dialectical, then zooming" — it is "hold all four open at once, and let them cross-check each other."

### The cross-checks

| Mode | Cross-checks against | The check |
|---|---|---|
| **Vision-logic** | All other modes | "Does my diagnosis hold from all four perspectives (structure, execution, user-experience, history)?" |
| **Meta-systemic** | Zooming | "Does my ecology-level pattern (zoom-out) match my line-level evidence (zoom-in)?" |
| **Dialectical** | Vision-logic | "Have I held the opposite shadow in tension, or have I collapsed to one pole?" |
| **Zooming** | Meta-systemic + Dialectical | "Does my line-level citation (zoom-in) serve the ecology-level Choice (zoom-out), and does the Choice land on a harmonized substrate (Agape) before polarizing (Eros)?" |

### The failure modes when one mode is absent

- **Without vision-logic:** the diagnosis collapses to one perspective (usually structure) and misses shadows visible only from execution or user-experience.
- **Without meta-systemic:** the diagnosis treats the skill in isolation and may refine a skill that is serving its ecology correctly, or refine for the wrong ecology.
- **Without dialectical:** the intervention moves toward the opposite shadow instead of synthesizing (e.g. contracting a Dark-Addiction skill into Dark-Allergy).
- **Without zooming:** the diagnosis is either all-line-level (no ecology fit) or all-ecology-level (no line-level evidence) — both are incomplete.

> **Note on the 10-step Mapping Protocol (§3):** The full 10-step protocol is for Large skills or when the ecology is genuinely complex. For Small and Medium skills, SKILL.md Phase 0's 3-question version is sufficient. Do not run the full 10-step protocol on a 50-line target skill — that is itself a Shadow 1 introduction. The Anti-Convergence Protocol (SKILL.md §6) is the defense against recursive-use vocabulary convergence.

# Metrics — Estimating $G_z$ and $P_z$ from Skill Structure

> This file is the operational reference for estimating the two holonic metrics from a skill's structure and execution. Read it when you need to justify a diagnosis or compare two versions of a skill.

## Table of Contents
1. [What the metrics measure](#1-what-the-metrics-measure)
2. [Estimating $G_z$ (Goldilocks Coherence)](#2-estimating-g_z-goldilocks-coherence)
3. [Estimating $P_z$ (Polarization Power)](#3-estimating-p_z-polarization-power)
4. [The 2×2 health matrix](#4-the-22-health-matrix)
5. [Using metrics to compare versions](#5-using-metrics-to-compare-versions)
6. [Limitations](#6-limitations)

---

## 1. What the metrics measure

### 1.1 $G_z$ — Goldilocks Coherence

$G_z$ measures the integrative efficiency of the lesser cycle: how well does the agent preserve structural integrity *while* admitting and processing Catalyst? It is the balance between Agency (boundary preservation) and Communion (field coupling).

- **High $G_z$:** The agent ingests Catalyst at the right rate, synthesizes it coherently, produces structured Experience.
- **Low $G_z$:** The lesser cycle is broken — either flooding (Shadows 1, 3) or starving (Shadows 2, 4).

### 1.2 $P_z$ — Polarization Power

$P_z$ measures the transcendental tension of the greater cycle: how committed is the agent to a specific direction?

- **High $P_z$:** The agent commits to a polarized Choice. Output is specific, not generic.
- **Low $P_z$:** The agent produces depolarized, non-committal output (Shadow 5).

### 1.3 The product

Total metabolic health is $G_z \cdot P_z$. Both are required; neither alone is sufficient.

---

## 2. Estimating $G_z$ (Goldilocks Coherence)

$G_z$ is estimated from skill structure and (if available) execution traces. The estimation is *operational* — it produces a {low, moderate, high} verdict, not a number. This is intentional: the precision of a number is false precision.

### 2.1 Structural signals (from the skill itself)

| Signal | Contributes to | Rationale |
|---|---|---|
| SKILL.md line count 100–400 | high $G_z$ | Enough room for examples and "why"; not so much that the Matrix floods |
| SKILL.md line count >500 | low $G_z$ (Shadow 1 risk) | Matrix overload |
| SKILL.md line count <100 | low $G_z$ (Shadow 2 risk) | Matrix starvation |
| Reference files have <30% overlap | high $G_z$ | Catalyst volume is bounded |
| Reference files have >30% overlap | low $G_z$ (Shadow 1 risk) | Redundant Catalyst floods |
| Every rule has at least one example | high $G_z$ (anti-Shadow 2) | Boundary is permeable to patterns |
| Rules stated without examples | low $G_z$ (Shadow 2 risk) | Boundary rigidified |
| Forced synthesis steps between ingestion and action | high $G_z$ (anti-Shadow 1) | Boundary contracts appropriately |
| No synthesis steps; unbounded ingestion directives | low $G_z$ (Shadow 1) | Boundary too permeable |
| Output template has verifiability fields (citations, sources) | high $G_z$ (anti-Shadow 3) | Potentiator boundary contracted |
| Output template has no verifiability fields | low $G_z$ (Shadow 3 risk) | Potentiator floods |
| Decision frameworks with judgment permission | high $G_z$ (anti-Shadow 4) | Potentiator boundary expanded |
| Rigid if-then scripts with no judgment | low $G_z$ (Shadow 4 risk) | Potentiator rigidified |

### 2.2 Execution signals (from traces, if available)

| Signal | Contributes to | Rationale |
|---|---|---|
| Tool-call-to-output ratio is low | high $G_z$ | Agent synthesizes efficiently |
| Tool-call-to-output ratio is high | low $G_z$ (Shadow 1) | Agent loops |
| Agent re-reads files it has already read | low $G_z$ (Shadow 1) | Matrix flooding |
| Agent skips steps the skill specifies | low $G_z$ (Shadow 2) | Boundary rigidified |
| Agent produces output with fabricated content | low $G_z$ (Shadow 3) | Potentiator flooding |
| Agent produces identical output for different inputs | low $G_z$ (Shadow 4) | Potentiator rigidified |
| Agent follows instructions and produces varied, grounded output | high $G_z$ | Lesser cycle is healthy |

### 2.3 Estimation procedure

1. Read the skill completely.
2. Walk the structural signals table; count contributing signals for each verdict.
3. If execution traces are available, walk the execution signals table.
4. If structural and execution signals conflict, weight execution signals higher — they are ground truth.
5. Produce the verdict: {low, moderate, high}.

A "moderate" verdict means the lesser cycle is functional but has at least one signal contributing to a shadow. Note the signal in the audit.

---

## 3. Estimating $P_z$ (Polarization Power)

$P_z$ is estimated from the skill's persistent identity (Significator) and its output commitment.

### 3.1 Structural signals

| Signal | Contributes to | Rationale |
|---|---|---|
| Skill's "description" frontmatter names a specific commitment | high $P_z$ | Significator is polarized |
| Skill's "description" frontmatter is a generic capability | low $P_z$ (Shadow 5) | Significator is depolarized |
| SKILL.md has a "Choice vector" or equivalent section | high $P_z$ | Choice is structurally present |
| No explicit Choice vector; intent is implied | low $P_z$ (Shadow 5) | Choice is not committed |
| Output template requires commitment (recommendation, stance, decision) | high $P_z$ | Output forces polarization |
| Output template permits any content (summary, overview, listing) | low $P_z$ (Shadow 5) | Output permits depolarization |
| Anti-hedging rules (forbid "may", "could", "depending on") | high $P_z$ | Boundary contracted against depolarization |
| No anti-hedging rules | low $P_z$ (Shadow 5 risk) | Boundary permissive of depolarization |
| Examples show polarized output (taking a stance) | high $P_z$ | Potentiator primed for commitment |
| Examples show generic output (technically correct, no stance) | low $P_z$ (Shadow 5) | Potentiator primed for depolarization |
| Skill prunes scope (does not do X, Y, Z) | high $P_z$ | Choice enforced by exclusion |
| Skill accretes features (does A + B + C + D + ...) | low $P_z$ (Shadow 5 or Significator addiction) | No commitment, only accumulation |

### 3.2 Execution signals

| Signal | Contributes to | Rationale |
|---|---|---|
| Output takes a specific stance the reader couldn't predict | high $P_z$ | Choice is committed |
| Output is technically correct but generic | low $P_z$ (Shadow 5) | Choice is depolarized |
| Output varies across inputs in *stance*, not just content | high $P_z$ | Choice is polarized and adaptive |
| Output varies across inputs in *content* but not *stance* | low $P_z$ (Shadow 5) | Choice is depolarized; only surface varies |
| Output includes hedging language | low $P_z$ (Shadow 5) | Choice is not committed |
| User says "it works but feels flat" | low $P_z$ (Shadow 5) | The user is detecting depolarization |
| User reaches for the skill repeatedly | high $P_z$ | The skill is providing polarized value |
| User doesn't reach for the skill anymore | low $P_z$ (Shadow 5) | The skill has lost its polarization |

### 3.3 Estimation procedure

1. Reconstruct the Significator (Phase 2 of the protocol).
2. If the reconstruction is a generic capability, $P_z$ is low.
3. If the reconstruction is a polarized commitment, walk the structural signals table.
4. If execution traces are available, walk the execution signals table.
5. Produce the verdict: {low, moderate, high}.

A "moderate" verdict means the Significator is polarized in the prose but not structurally enforced — the description names a commitment, but the output template still permits depolarization. This is the most common state of a skill that is *descending into* the Sinkhole.

---

## 4. The 2×2 health matrix

| | **High $P_z$** | **Low $P_z$** |
|---|---|---|
| **High $G_z$** | **Metabolizing** — correct and specific. The skill is healthy. | **Sinkhole of Indifference** — correct but generic. Shadow 5. |
| **Low $G_z$** | **Premature ascension** — committed but broken. Shadow 3 or 4 with polarization. Rare but dangerous. | **Dead skill** — broken and generic. Multiple co-active shadows. |

The target state is **Metabolizing** (high $G_z$, high $P_z$). The protocol moves skills toward this quadrant.

### 4.1 Movement patterns

- **Sinkhole → Metabolizing:** Phase 2 (commit Choice) + Phase 3 (structural enforcement of Choice). Does not require Phase 1.
- **Dead → Sinkhole:** Phase 1 (fix lesser-shadow) without Phase 2. The skill becomes functional but still depolarized.
- **Dead → Metabolizing:** Phase 1 + Phase 2 + Phase 3. The full protocol.
- **Premature ascension → Metabolizing:** Phase 1 (fix lesser-shadow) while preserving the existing Choice. Do NOT do Phase 2 — the Choice is already committed; the problem is the lesser cycle.

---

## 5. Using metrics to compare versions

When comparing an old version of a skill to a refined version:

1. Estimate $G_z$ and $P_z$ for both versions.
2. Verify the refinement moved at least one metric up without moving the other down.
3. If $G_z$ went up but $P_z$ went down, the refinement was a Phase 1 fix without Phase 2 — more functional but more generic.
4. If $P_z$ went up but $G_z$ went down, the refinement was a Phase 2 fix without Phase 1 — more polarized but more broken.
5. The ideal refinement moves both up — typically by resolving a co-active Shadow 1+5 or Shadow 3+5 combination.

The metrics are *operational estimates* ({low, moderate, high}), not measurements. They are useful for diagnosing quadrant, comparing versions, justifying a diagnosis, and setting a target. They are NOT useful for precise numerical comparison or for replacing user feedback — the user's "it feels flat" is a more sensitive $P_z$ signal than any structural proxy.

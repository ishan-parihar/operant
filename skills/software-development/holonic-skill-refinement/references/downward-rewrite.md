# The Downward Rewrite — $R_M$ and $R_P$ Worked Examples

> **Convergence warning:** This file contains 6 polished worked examples. They are the highest-probability continuation target for a language model. Before applying any pattern from this file, read SKILL.md §6 (Anti-Convergence Protocol) and generate 2–3 candidate interventions that differ in *structural shape*. Apply a pattern from this file only when the target skill's specific evidence justifies that exact shape.

## Table of Contents
1. [The Two Operators](#1-the-two-operators)
2. [The Discipline of Tracing](#2-the-discipline-of-tracing)
3. [Worked Example 1: Shadow 1 (research skill)](#3-worked-example-1-shadow-1-research-skill)
4. [Worked Example 2: Shadow 2 (code-gen skill)](#4-worked-example-2-shadow-2-code-gen-skill)
5. [Worked Example 3: Shadow 3 (citation skill)](#5-worked-example-3-shadow-3-citation-skill)
6. [Worked Example 4: Shadow 4 (doc-gen skill)](#6-worked-example-4-shadow-4-doc-gen-skill)
7. [Worked Example 5: Shadow 5 (writing skill)](#7-worked-example-5-shadow-5-writing-skill)
8. [Worked Example 6: Co-active Shadow 1+5 (data-extraction skill)](#8-worked-example-6-co-active-shadow-15-data-extraction-skill)

---

## 1. The Two Operators

- **$R_M$ (Matrix rewrite):** restructures *how the skill organizes active context* — section reordering, line-count changes, synthesis steps, adding/removing Catalysts (examples, "why" explanations, edge cases), restructuring output templates.
- **$R_P$ (Potentiator rewrite):** restructures *what reasoning pathways activate* — novel frameworks (mental model, decision tree, analogy), priming examples, removing stagnant triggers, judgment permission, reflection steps, polarized examples.

The operators are *functions of the new Significator* ($S_{new}$) — every edit must trace to the Phase 2 Choice vector. Edits that don't trace are lateral noise. (Formal expression: $M_{t+1} = \mathbf{R}_M(S_{new}) \cdot M_t$, $P_{t+1} = \mathbf{R}_P(S_{new}) \cdot P_t$ — see `holonic-theory.md` §7.1.)

---

## 2. The Discipline of Tracing

Before applying any edit, write the trace in compact form:

```
Edit: <description> | Operator: $R_M$/$R_P$ | Shadow: S# | Vector: Agape/Eros
Trace: <contracts/expands> the <Matrix/Potentiator> boundary to serve "<Choice vector>" by <mechanism>. Evidence: <citation>.
```

If you cannot fill every field, do not make the edit. Tracing is what distinguishes a downward rewrite from a generic refactor.

---

## 3. Worked Example 1: Shadow 1 (research skill)

**Target:** `deep-research` — multi-step web research.
**Complaint:** "The agent keeps re-running the same search with different phrasings. It never finishes."

**Phase 1:** S1 (Dark-Addiction). Evidence: 540 lines; "perform comprehensive searches across multiple phrasings before synthesizing"; no synthesis step. $G_z$: low. Intervention: contract.

**Phase 2:** Significator: "Turn a vague question into a decision-ready research brief." Great Way: "Knowledge workers who need answers, not exhaustive search logs." Greater-shadow: S5 (incipient). Choice vector: "Search until you can answer with a specific recommendation, then stop. Comprehensiveness is not the goal; decision-readiness is." $P_z$: low → moderate-high.

**Phase 3 — Edits:**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Replace "comprehensive searches across multiple phrasings before synthesizing" with "Search until you have enough evidence to make a specific recommendation. After each search, ask: 'Can I now answer with a specific recommendation?' If yes, stop and synthesize." | $R_M$ | S1 | Agape | Contracts Matrix boundary by inserting forced synthesis check between each search and the next. Evidence: "comprehensive searches across multiple phrasings" is the explicit Catalyst-flooding directive. |
| 2 | Add "Search budget" section: "Maximum 5 searches per research task. If you reach 5 without a recommendation, synthesize with what you have and note the gap explicitly." | $R_M$ | S1 | Agape | Contracts by enforcing hard Catalyst budget. Evidence: no budget existed. |
| 3 | Add "Recommendation first" output template requiring a specific recommendation in the first section, with evidence citations. Move "methodology" to an appendix. | $R_M$+$R_P$ | S1+S5 | Agape+Eros | $R_M$ restructures output so agent knows what it's synthesizing toward. $R_P$ primes "specific recommendation" pathways over "comprehensive summary" pathways. Evidence: original template put methodology first. |

**Verification:** SKILL.md 540→410 lines (contracted). "Comprehensive searches" gone. Choice vector appears structurally as recommendation-first template and synthesis-check.

---

## 4. Worked Example 2: Shadow 2 (code-gen skill)

**Target:** `api-endpoint-generator` — generates Next.js API routes.
**Complaint:** "The agent skips the validation step half the time. It doesn't read the schema reference."

**Phase 1:** S2 (Dark-Allergy). Evidence: 85 lines; "always validate input against the schema" with no example; schema reference mentioned as "see references/schema.md" but never triggered. $G_z$: low. Intervention: expand.

**Phase 2:** Significator: "Generate production-ready Next.js API routes with validated input." Great Way: "Developers who will deploy without re-checking validation." Greater-shadow: none dominant. Choice vector: "Every API route must include explicit input validation, with the validation code traceable to a specific schema field." $P_z$: moderate → high.

**Phase 3 — Edits:**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Replace "always validate input against the schema" with: "Before writing any handler logic, write the input validation block. For each field: identify the type, identify the constraints (required, optional, min, max, pattern), write a validation check, return 400 with a specific error message if validation fails. Example: `if (!req.body.name \|\| typeof req.body.name !== 'string') { return res.status(400).json({ error: 'name is required and must be a string' }); }`" | $R_M$+$R_P$ | S2 | Agape | Expands Matrix by injecting worked example alongside rule. $R_P$ primes specific validation pattern (per-field type + constraint + check + error). Evidence: original rule had no example. |
| 2 | Replace "see references/schema.md" with "When you are about to write validation code, read references/schema.md and identify the schema for this endpoint. Quote the schema fields in your output before writing the validation block." | $R_M$ | S2 | Agape | Expands by triggering reference read at the moment needed, requiring the agent to quote the schema. Evidence: original directive was optional and unsituated. |
| 3 | Add "Why validation matters" subsection: "Validation is not optional because downstream code assumes the input matches the schema. If validation is missing, a single malformed request can cause a 500 error. The validation block is the contract between the API and its callers." | $R_M$+$R_P$ | S2 | Agape | Expands with the "why". $R_P$ primes "contract" reasoning. Evidence: original stated rule without reasoning. |

**Verification:** SKILL.md 85→165 lines (expanded — correct for S2). Validation rule now has example, triggered reference, "why". S2 markers (terse, no example, optional reference) gone.

---

## 5. Worked Example 3: Shadow 3 (citation skill)

**Target:** `academic-citation-finder` — finds and formats citations.
**Complaint:** "The agent invents citations. The DOIs don't exist."

**Phase 1:** S3 (Golden-Addiction). Evidence: "provide 5-10 relevant citations per claim"; no verification step; no source-tracking; "comprehensive coverage". $G_z$: low. Intervention: contract (Potentiator side).

**Phase 2:** Significator: "Find verifiable academic citations for specific claims." Great Way: "Academic writers who will check every DOI." Greater-shadow: S5 (incipient). Choice vector: "Every citation must be verifiable — every DOI must resolve, every author traceable. Provide fewer if necessary; never fabricate." $P_z$: moderate → high.

**Phase 3 — Edits:**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Replace "provide 5-10 relevant citations per claim" with "provide only citations you have verified by retrieving the source. For each citation, include: title, authors, year, venue, DOI, and the URL of the page where you verified the citation." | $R_M$ | S3 | Agape | Contracts Potentiator boundary by requiring structural substrate (verification URL) before any citation output. Evidence: "5-10 relevant citations" is the flooding directive. |
| 2 | Add "Verification step" before output: "For each citation in your draft, perform a web search to verify the citation exists. If you cannot verify, remove the citation." | $R_M$ | S3 | Agape | Contracts by inserting verification between generation and output. Evidence: no verification step existed. |
| 3 | Add a "Restraint example": an example where the agent provided only 2 citations (because only 2 could be verified) and noted "No further verifiable citations found for this claim." | $R_P$ | S3 | Agape | Primes Potentiator with restraint pathway — "fewer citations, all verified" rather than only "many citations, some fabricated". Evidence: original examples were all comprehensive. |

**Verification:** "5-10 citations" gone. Choice vector ("verifiable") appears structurally as verification step and URL requirement. Restraint example added.

---

## 6. Worked Example 4: Shadow 4 (doc-gen skill)

**Target:** `meeting-notes-generator` — generates meeting notes from transcripts.
**Complaint:** "Every meeting's notes look identical. It doesn't adapt to different meeting types."

**Phase 1:** S4 (Golden-Allergy). Evidence: rigid step-by-step script (attendees, agenda, discussion, decisions, action items); no decision framework for meeting type; identical template for all meetings; no adaptation examples. $G_z$: low. Intervention: expand (Potentiator side).

**Phase 2:** Significator: "Turn meeting transcripts into the right kind of notes for the meeting type." Great Way: "Teams whose meetings vary — standups, design reviews, 1:1s, all-hands." Greater-shadow: none dominant. Choice vector: "Classify the meeting type first, then generate notes in the format appropriate to that type." $P_z$: moderate → high.

**Phase 3 — Edits:**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Replace rigid script with decision framework: "Step 1: Classify the meeting — Standup (≤15 min, status) → standup template; Design review (artifacts, critique) → design-review template; 1:1 (two participants, personal) → 1:1 template; All-hands (one-to-many, announcement) → all-hands template; Decision meeting → decision-meeting template; Other → default template + note the type. Step 2: Apply the corresponding template, adapting to the specific meeting. Step 3: If the template doesn't fit, adapt it. Document your adaptation in a 'Notes on format' section." | $R_P$ | S4 | Agape | Expands Potentiator by injecting decision framework requiring judgment. Evidence: original script had no classification; same template for all meetings. |
| 2 | Add "Adaptive example": an example where the agent received a design-review meeting, classified it correctly, used the design-review template, and noted in 'Notes on format' that it added a "Critique themes" section because the meeting had a long critique segment. | $R_P$ | S4 | Agape | Primes Potentiator with adaptive pathway — "adapt the template and document". Evidence: original examples were all template-following. |
| 3 | Add "Reflection step": "After producing the notes, identify one way the output could be improved if you had more context about the team. Note it in 'Notes on format'. Do not implement the improvement." | $R_P$ | S4 | Agape | Expands Potentiator by requiring reflection — primes generation of new pathways. Evidence: no reflection step existed. |

**Verification:** SKILL.md longer (correct for S4). Rigid script gone, replaced by decision framework. Choice vector ("classify first, then adapt") appears structurally as classification step and adaptive example.

---

## 7. Worked Example 5: Shadow 5 (writing skill)

**Target:** `blog-post-writer` — generates blog posts.
**Complaint:** "The posts are fine but they're all the same. I could have written them myself. They don't say anything."

**Phase 1:** No active lesser-shadow (lesser cycle healthy — agent produces output, follows instructions, doesn't hallucinate, varies somewhat). $G_z$: moderate-high. Intervention: none at lesser cycle.

**Phase 2:** Significator: "Writes blog posts on a topic." (generic — skill has lost polarization). Great Way: "Content marketers who need posts that get read, not posts that fill space." Greater-shadow: S5 (dominant). Choice vector: "Turn a topic into a blog post that takes a specific stance the reader cannot predict from the headline." $P_z$: low → high.

**Phase 3 — Edits:**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Rewrite description frontmatter from "Generates blog posts on a topic" to "Turns a topic into a blog post that takes a specific stance the reader cannot predict from the headline. Use when the user wants a blog post that says something, not one that fills space." | $R_M$ (greater) | S5 | Eros | Reconstructs Significator as polarized commitment. Evidence: original description permitted any output that was technically a blog post. |
| 2 | Add "Choice vector" section: "This skill commits to producing blog posts that take a specific, unpredictable stance. It does not: produce listicles unless explicitly asked; use 'in today's world' or 'in the digital age'; end with a generic call-to-action; hedge with 'may', 'could', 'depending on'. If the topic doesn't permit a specific stance, ask the user to refine rather than producing a generic post." | $R_M$ (greater) | S5 | Eros | Makes Choice structurally present. Anti-patterns (listicles, hedging, generic CTAs) are Sinkhole markers; forbidding them contracts against depolarization. Evidence: original permitted all of these. |
| 3 | Replace output template's "Conclusion: Summarize the key points and end with a call-to-action" with "Conclusion: State the one thing the reader should now believe that they did not believe before reading this post. Do not summarize. Do not include a call-to-action unless explicitly requested." | $R_M$ | S5 | Eros | Restructures output template to require commitment. Evidence: original "summarize and CTA" permitted any generic conclusion. |
| 4 | Replace generic examples with polarized examples: "Example: Topic 'productivity apps' → Post stance: 'Most productivity apps make you less productive because they optimize for engagement, not output. Here is the case for using fewer of them.'" | $R_P$ | S5 | Eros | Primes Potentiator with polarized examples — reachable pathways for "unpredictable stance" rather than "generic post about X". Evidence: original examples were all generic. |

**Verification:** Skill roughly same length (correct for S5 — replacement, not expansion/contraction). Generic description gone. Choice vector appears structurally as anti-patterns list, conclusion template, polarized examples.

---

## 8. Worked Example 6: Co-active Shadow 1+5 (data-extraction skill)

**Target:** `pdf-data-extractor` — extracts structured data from PDFs.
**Complaint:** "The agent re-reads the PDF three times before extracting. And the extraction is generic — it dumps everything, doesn't commit to what matters."

**Phase 1:** S1 (Dark-Addiction). Evidence: 612 lines; "comprehensively ingest all referenced material before proceeding"; 4 overlapping reference files. $G_z$: low. Intervention: contract.

**Phase 2:** Significator: "Turn a PDF into structured, verifiable extracted content." Great Way: "Users who need trustworthy extraction, not exhaustive re-reading." Greater-shadow: S5. Choice vector: "Extract once, verify by sampling, never re-read for the same purpose." $P_z$: low → moderate-high.

**Phase 3 — Edits (resolve S5 first, then S1 — per §7.1 of shadow-diagnostic.md):**

| # | Edit | Op | Shadow | Vector | Trace |
|---|---|---|---|---|---|
| 1 | Add "Choice vector" section: "This skill commits to extracting verifiable content. It does not re-read the PDF for the same purpose twice. If extraction is incomplete, it identifies the specific gap and re-reads only that section." | $R_M$ (greater) | S5 | Eros | Commits Significator to polarized direction. Evidence: no Choice vector existed. |
| 2 | Replace "comprehensively ingest all referenced material before proceeding" with "Read SKILL.md fully. Read a reference file only when this skill explicitly directs you to it. Never re-read a file you have already read in this session — cite the prior read instead." | $R_M$ | S1 | Agape | Contracts Matrix boundary. Evidence: original was the explicit Catalyst-flooding directive. |
| 3 | Add forced synthesis step: "Before any second read of the PDF, write a 3-bullet summary of what you extracted and what is missing. If the missing item is not in the PDF, do not re-read." | $R_M$ | S1 | Agape | Contracts by forcing synthesis between ingestion events. Evidence: no synthesis step existed. |
| 4 | Add "Verification sampling" pattern: "After extraction, sample 5% of the output (minimum 3 items) and check each against the PDF source. If any sample fails, re-extract that section only — do not re-extract the whole document." | $R_P$ | S5+S1 | Eros+Agape | Injects novel framework (verification sampling) priming targeted re-extraction rather than wholesale re-reading. Evidence: no verification pattern existed. |
| 5 | Merge `references/pdf-formats.md` and `references/structure.md` into a single `references/extraction.md`, removing ~600 lines of overlap. | $R_M$ | S1 | Agape | Contracts Catalyst volume by removing redundancy. Evidence: >40% overlap. |

**Verification:** SKILL.md 612→380 lines (contracted). "Comprehensively ingest" gone. Choice vector ("extract once, verify by sampling") appears structurally as verification-sampling step and read-once directive. Both S1 and S5 markers gone.

---

## Post-read reminder

These 6 examples are **calibration, not a phrase bank**. Do not copy the edit shapes (synthesis gate, Choice vector section, verification sampling, decision framework, anti-hedging rules, reference merge) into your refinement unless the target skill's specific evidence justifies that exact shape. Generate 2–3 structurally different candidates first. The Anti-Convergence Protocol (SKILL.md §6) is the defense against these examples becoming the only shapes you produce.

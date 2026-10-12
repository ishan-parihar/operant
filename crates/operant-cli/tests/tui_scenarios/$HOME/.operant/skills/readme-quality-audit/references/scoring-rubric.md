# Scoring Rubric — Detailed Criteria per Dimension

## 1. Hero/Value (0-2.0)

| Score | Criteria |
|-------|----------|
| 2.0 | First paragraph states category + name + concrete promise in plain language; hero region (first 500 chars) includes visual proof (screenshot/diagram/output) |
| 1.5 | Clear value prop but hero lacks visual; or visual present but value prop slightly technical |
| 1.0 | Value prop present but buried in jargon; no hero visual |
| 0.5 | Vague opening ("A tool for..."); no category; no visual |
| 0.0 | No clear first paragraph; starts with badges/TOC/architecture |

**Key check**: Remove repo name from hero — could it apply to another project? If yes, redesign.

## 2. Proof First (0-2.0)

| Score | Criteria |
|-------|----------|
| 2.0 | Visual proof section (screenshots, outputs, diagrams) appears BEFORE any architecture/tech stack section |
| 1.5 | Proof and architecture interleaved; proof accessible without deep reading |
| 1.0 | Architecture first but proof follows quickly (<2 sections) |
| 0.5 | Architecture dominates; proof buried or minimal |
| 0.0 | No proof section; only architecture/claims |

**Evidence types that count as Proof:**
- Actual terminal output / CLI screenshots
- Generated artifact screenshots (MP4 frame, carousel PNG, rendered diagram)
- Dashboard / UI screenshots
- Architecture diagram WITH real data flow labels
- Benchmark results table with numbers

**Does NOT count:**
- Generic "how it works" diagram without real data
- Logo / decorative images
- Badges

## 3. Structure Order (0-1.5)

| Score | Criteria |
|-------|----------|
| 1.5 | Install/Quick Start → Features/What It Does → Mechanism/Architecture → Usage/Config → Detail |
| 1.0 | Install before Architecture, but Usage before Features; or minor ordering issues |
| 0.5 | Install after Architecture; or no clear Install section |
| 0.0 | Architecture first; Install missing or at very end |

**Required section sequence (by anchor position):**
1. Value/Hero (implicit)
2. Proof (explicit section with visuals)
3. Mechanism/What It Is (one explanation)
4. First Use / Quick Start (shortest path to success)
5. Usage / Configuration / API
6. Detail (tables, advanced, limitations, FAQ)

## 4. Visual Proof (0-1.5)

| Score | Criteria |
|-------|----------|
| 1.5 | ≥3 distinct visual proofs: terminal output, generated artifact, architecture diagram with real labels |
| 1.0 | 2 visual proofs (e.g., screenshot + diagram) |
| 0.5 | 1 visual proof |
| 0.0 | Zero images, or only decorative/logo/badges |

**Quality multipliers** (applied after count):
- Real terminal output / CLI demo: +0.25
- Actual generated artifact (not mockup): +0.25
- Annotated diagram (labels explain decisions): +0.25
- Before/after comparison: +0.25

## 5. Code Examples (0-1.0)

| Score | Criteria |
|-------|----------|
| 1.0 | ≥2 code blocks; at least ONE shows complete end-to-end workflow (install → config → run → output) |
| 0.75 | ≥2 code blocks but no single complete workflow; fragments cover install + usage separately |
| 0.5 | 1 code block; or 2+ but all fragments (no complete example) |
| 0.25 | 1 fragment only |
| 0.0 | No code blocks |

**End-to-end example must include:**
- Install command (or clear prerequisite statement)
- Configuration/initialization
- Run/invoke command
- Expected output reference

## 6. Tables/Structured Data (0-1.0)

| Score | Criteria |
|-------|----------|
| 1.0 | ≥2 tables covering different domains (e.g., features + config; or tools + comparison; or providers + status) |
| 0.75 | 1 comprehensive table with >5 rows and >3 columns covering key structured data |
| 0.5 | 1 simple table (<5 rows or <3 cols) |
| 0.25 | Tables present but poorly formatted (misaligned, no headers) |
| 0.0 | No tables; structured data in prose/bullet lists only |

**Domains that should use tables:**
- Feature lists with descriptions
- Tool/command catalogs
- Configuration options
- Provider/model comparisons
- Supported platforms/languages
- Version compatibility matrices

## 7. Length/Density (0-0.5)

| Score | Criteria |
|-------|----------|
| 0.5 | <300 lines; information density high (every section earns its keep) |
| 0.25 | 300-400 lines; some redundancy but generally focused |
| 0.1 | 400-500 lines;明显冗余,可折叠或拆分 |
| 0.0 | >500 lines; needs splitting into docs/ or condensing |

**Line counting:** Non-empty lines in README.md only.

## 8. Badges/Signals (0-0.5)

| Score | Criteria |
|-------|----------|
| 0.5 | 3-8 badges covering: language, license, build status, version, platform, protocol (MCP), community |
| 0.25 | 1-2 badges; or 9-10 badges |
| 0.0 | 0 badges; or >10 badges (noise) |

**Recommended badge set (pick 5-7):**
- Language (Rust/Python/TypeScript)
- License (MIT/Apache-2.0)
- Build status (GitHub Actions/CI)
- Version (crates.io/PyPI/npm)
- Protocol (MCP)
- Platform (Linux/macOS/Windows/Docker)
- Community (Discord/GitHub Discussions)

## Composite Score Interpretation

| Total | Grade | Meaning |
|-------|-------|---------|
| 9.0-10 | A+ | Template quality; use as reference |
| 8.0-8.9 | A | Production ready; minor polish only |
| 7.0-7.9 | B+ | Strong; apply priority fixes before launch |
| 6.0-6.9 | B | Good foundation; several structural fixes needed |
| 5.0-5.9 | C+ | Functional but weak adoption signals |
| 4.0-4.9 | C | Significant gaps in proof/structure |
| 3.0-3.9 | D | Major rewrite needed |
| <3.0 | F | Failing; does not communicate value |

## Priority Fix Weighting

When generating `priority_fixes`, weight by dimension importance:

1. **visual_proof** (15%) — Zero images = instant fail for adoption
2. **proof_first** (20%) — Architecture before proof = developer-only appeal
3. **hero_value** (20%) — Unclear value = no clicks past first screen
4. **structure_order** (15%) — Install after arch = friction to first success
5. **code_examples** (10%) — No examples = no copy-paste path
6. **tables** (10%) — No structured data = hard to compare/evaluate
7. **length** (5%) — Too long = drop-off before detail
8. **badges** (5%) — Missing signals = trust gap
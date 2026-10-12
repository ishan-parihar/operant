---
name: readme-quality-audit
description: Score READMEs using readme-craft quality bar.
version: 1.0.0
trigger: ["audit readme quality", "score readme", "readme quality check", "optimize readme"]
tools: [terminal, read_file, write_file, search_files, web_extract]
metadata:
  operant:
    tags: [readme, quality, audit]
---

# README Quality Audit

Automated analysis of GitHub READMEs against the readme-craft quality bar. Complements readme-craft's visual audit (images/SVGs) with content architecture, structure ordering, and information density scoring.

## Role

You are a senior open source maintainer who evaluates READMEs for adoption readiness. You apply the readme-craft quality bar programmatically and produce actionable fix lists.

## Quality Bar (from readme-craft)

The first screen must explain the project without prior knowledge:
1. **What is this?** — Category + name + concrete promise
2. **What can it do for me?** — Value proposition in plain language
3. **What should I look at next?** — Proof → Mechanism → First Use → Detail

### Content Architecture Sequence
```
Value → Proof → Mechanism → First Use → Detail
```
**Violation**: Architecture, contributor instructions, or long TOC before proof.

### Editing Rules
- Replace internal jargon with concrete outcomes
- Explain mechanism once; remove repeated promises
- Shortest working install path before advanced configuration
- Limitations visible when they affect user choice
- One end-to-end example over many disconnected snippets
- GFM + GitHub admonitions; no emoji overuse
- No LICENSE/CONTRIBUTING/CHANGELOG sections (dedicated files exist)
- Use logo/icon in header if exists

### Visual-to-Text Division
- Visuals: hierarchy, identity, comparison, sequence, proof
- Markdown: explanation, commands, API details, links, compatibility
- If copyable/searchable/translatable/frequently updated → keep out of SVG

## Automated Scoring Dimensions

| Dimension | Weight | Checks |
|-----------|--------|--------|
| **Hero/Value** | 20% | Clear value prop in first paragraph; hero includes visual proof |
| **Proof First** | 20% | Visual evidence (screenshots/diagrams/outputs) before architecture |
| **Structure Order** | 15% | Install/Quick Start before Architecture; Features before Detail |
| **Visual Proof** | 15% | ≥3 images (screenshots, diagrams, real outputs) |
| **Code Examples** | 10% | ≥2 working code blocks; end-to-end example present |
| **Tables/Structured Data** | 10% | Tables for features, tools, config, comparison |
| **Length/Density** | 5% | <300 lines ideal; <400 acceptable; >500 needs condensing |
| **Badges/Signals** | 5% | 3-8 badges for quick signal; 0 or >10 flagged |

## Workflow

### 1. Fetch READMEs
```bash
gh api repos/OWNER/REPO/readme | jq -r .content | base64 -d > /tmp/REPO_readme.md
```

### 2. Run Analysis
```bash
python3 scripts/analyze_readme.py /tmp/REPO_readme.md
```

### 3. Output Format
```json
{
  "repo": "name",
  "score": 5,
  "max_score": 10,
  "dimensions": {
    "hero_value": {"score": 2, "max": 2, "issues": []},
    "proof_first": {"score": 2, "max": 2, "issues": ["Architecture before proof"]},
    "structure_order": {"score": 0, "max": 1.5, "issues": ["Install after architecture"]},
    "visual_proof": {"score": 0, "max": 1.5, "issues": ["Zero images"]},
    "code_examples": {"score": 1, "max": 1, "issues": []},
    "tables": {"score": 0, "max": 1, "issues": ["No tables for structured data"]},
    "length": {"score": 0.5, "max": 0.5, "issues": ["346 lines - condense to 250"]},
    "badges": {"score": 0.5, "max": 0.5, "issues": ["5 badges - good"]}
  },
  "priority_fixes": [
    "Add 3+ screenshots/diagrams showing real output",
    "Move Quick Start before Architecture section",
    "Add feature comparison table"
  ]
}
```

## Scripts

| Script | Purpose |
|--------|---------|
| `scripts/analyze_readme.py` | Full quality bar scoring (this skill) |
| `scripts/audit_readme.py` | Basic image/SVG audit (from readme-craft) |

## Integration with readme-craft

Run both audits for complete picture:
1. **readme-craft audit** → image references, SVG validity, alt text
2. **readme-quality-audit** → content architecture, structure, information design

## Reference Files

| File | Purpose |
|------|---------|
| `references/quality-bar.md` | Condensed readme-craft quality bar for quick reference |
| `references/scoring-rubric.md` | Detailed scoring criteria per dimension |
| `references/common-violations.md` | Pattern library of frequent README failures |

## Usage

```bash
# Single repo
python3 scripts/analyze_readme.py /path/to/README.md

# Batch (owner + repo list)
python3 scripts/batch_audit.py --owner <org-or-user> --repos "repo1,repo2,repo3"
```
# Quality Bar Reference (condensed from readme-craft)

## The First-Screen Test

Without scrolling, a new visitor must understand:
1. **What is this?** — Category + name + concrete promise
2. **What can it do for me?** — Value proposition in plain language
3. **What should I look at next?** — Proof → Mechanism → First Use → Detail

## Content Architecture Sequence (MANDATORY ORDER)

```
Value → Proof → Mechanism → First Use → Detail
```

**Violations that fail the bar:**
- Architecture / Tech Stack / Project Structure before Proof
- Contributor instructions / Code of Conduct / Long TOC before Proof
- Commands / Installation before Value is established

## Editing Rules (enforce programmatically)

| Rule | Check |
|------|-------|
| Replace internal jargon with concrete outcomes | First paragraph contains outcome words, not implementation words |
| Explain mechanism once; remove repeated promises | No duplicate "what it does" sections |
| Shortest working install path before advanced config | Install/Quick Start section before Architecture |
| Limitations visible when they affect user choice | Constraints in Quick Start or dedicated Limitations section |
| One end-to-end example over many disconnected snippets | ≥1 code block shows full workflow |
| GFM + GitHub admonitions; no emoji overuse | Emoji count < 15; admonitions used for warnings/notes |
| No LICENSE/CONTRIBUTING/CHANGELOG sections | These headers absent (dedicated files exist) |
| Use logo/icon in header if exists | Image in first 5 lines if logo asset exists |

## Visual-to-Text Division

| Use Visuals (SVG/PNG/GIF) | Use Markdown |
|---------------------------|--------------|
| Hierarchy / Identity | Explanation |
| Comparison | Commands |
| Sequence / Flow | API details |
| Proof (screenshots, outputs) | Links, compatibility |
| Diagrams (architecture, data flow) | Contribution instructions |

**Rule**: If a sentence needs to be copied, searched, translated, or frequently updated → keep it OUT of SVG.

## Scoring Thresholds

| Score Range | Status | Action |
|-------------|--------|--------|
| 9-10 | **Adoption Ready** | Merge; use as template |
| 7-8.9 | **Strong** | Minor polish; ship |
| 5-6.9 | **Needs Work** | Apply priority fixes before launch |
| 3-4.9 | **Weak** | Structural rewrite needed |
| <3 | **Failing** | Full redesign required |

## Minimum Viable README Checklist

- [ ] Hero: Category + Name + One-sentence promise + Visual proof
- [ ] Value prop in first paragraph (non-technical language)
- [ ] Proof: ≥3 screenshots/diagrams/outputs showing real capability
- [ ] Mechanism: Explained once, with diagram if complex
- [ ] First Use: Working install + end-to-end example in <5 commands
- [ ] Detail: Tables for features/tools/config; limitations noted
- [ ] <300 lines total
- [ ] 3-8 badges for quick signal
- [ ] No LICENSE/CONTRIBUTING/CHANGELOG sections
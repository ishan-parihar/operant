# Common Violations — Pattern Library

Patterns observed across 10 pinned repos (July 2026 audit). Each pattern includes detection logic and fix template.

---

## V1: Architecture Before Proof
**Frequency**: 4/10 repos (openscript, slideforge, reddit-httpx, metatrader-docker-deployment)

**Detection**: Architecture/Tech Stack section anchor position < Proof/Screenshots section anchor position

**Root cause**: Developer writes for developers, not adopters. Assumes reader wants implementation before seeing results.

**Fix template**:
```markdown
## Before (violation)
## Architecture
[diagram, tech stack, crate structure]

## Quick Start
[cargo build, run commands]

## After (correct)
## What It Does (Value + Proof)
[1-2 sentences + 2-3 screenshots/outputs]

## Quick Start
[cargo build, run commands]

## Architecture (for contributors)
[diagram, tech stack]
```

**Automated check**: `grep -n "## Architecture\|## Tech Stack\|## Project Structure" README.md` should appear AFTER `grep -n "## Quick Start\|## Install\|## Getting Started\|## Screenshots\|## Demo\|## Output"`

---

## V2: Zero Visual Proof
**Frequency**: 3/10 repos (slideforge, metatrader-docker-deployment, aportal partially)

**Detection**: `grep -c "!\[.*\](" README.md` == 0 (or only badges/logo)

**Root cause**: 
- slideforge: Generates visual output but README doesn't show it
- metatrader: Terminal/UI product but no screenshots
- aportal: CLI + dashboard but only text

**Fix template per project type**:
| Project Type | Required Visuals (minimum 3) |
|--------------|------------------------------|
| CLI tool | Terminal session (asciinema/GIF), --help output, real command run |
| MCP server | Tool list table, agent workflow diagram, sample JSON I/O |
| Deployment | Architecture diagram, terminal deploy log, web UI screenshot |
| Visual generator | 3+ actual outputs (carousel, video frame, diagram) |
| Dashboard/API | Dashboard screenshot, API response, health check |

**Automated check**: `python3 scripts/audit_readme.py README.md` → "Local images checked: 0"

---

## V3: Install After Architecture
**Frequency**: 4/10 (openscript, slideforge, reddit-httpx, metatrader-docker-deployment)

**Detection**: Install/Quick Start section line number > Architecture section line number

**Fix**: Move entire Quick Start section to immediately after Proof section.

**Exception**: If project is library-only (no CLI), "Installation" = `cargo add` / `pip install` can go in Usage. But Quick Start (end-to-end example) must still precede Architecture.

---

## V4: Excessive Length (>500 lines)
**Frequency**: 3/10 (social-forge 563, igs-rust 567, aportal 544)

**Detection**: `wc -l README.md` > 500

**Root causes**:
| Repo | Bloat Source | Fix |
|------|--------------|-----|
| social-forge | 76 sections, 26 code blocks, triple-interface docs | Collapse CLI/REST/MCP into comparison table; move configs to CONFIG.md |
| igs-rust | 63 sections, 22 tables, full 68-tool catalog | Move tool catalog to TOOLS.md; keep summary table (14 pools) |
| aportal | 54 sections, 22-bug audit, benchmarks, provider details | Move audit to AUDIT.md, benchmarks to BENCHMARKS.md, provider matrix to PROVIDERS.md |

**Pattern**: "Documentation site in README" — split when any section >50 lines.

---

## V5: No Tables for Structured Data
**Frequency**: 4/10 (operant, metatrader-docker-deployment, slideforge, aportal has tables but not for features)

**Detection**: Feature/tool/config data in bullet lists or prose instead of tables.

**Fix templates**:

**Feature list → Table**:
```markdown
## Before
- Feature A: does X
- Feature B: does Y

## After
| Feature | Description | Status |
|---------|-------------|--------|
| Feature A | Does X | ✅ |
| Feature B | Does Y | 🚧 |
```

**Tool catalog → Table** (igs-rust pattern):
```markdown
| Category | Tools | Description |
|----------|-------|-------------|
| News | 3 | Fetch, enrich, test sources |
| Research | 4 | Search arXiv, PubMed, download PDFs |
```

**Config options → Table**:
```markdown
| Env Var | Default | Description |
|---------|---------|-------------|
| PORT | 8000 | HTTP port |
| LOG_LEVEL | info | Log verbosity |
```

---

## V6: No Badges / Too Many Badges
**Frequency**: 3/10 zero badges (slideforge, aportal, metatrader), 1/10 too many (none in this set but common)

**Detection**: `grep -c "img.shields.io" README.md` == 0 or > 10

**Minimum badge set** (5 badges):
```markdown
[![Language](https://img.shields.io/badge/Rust-1.80+-orange?logo=rust)](https://rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-green)](LICENSE)
[![Build](https://img.shields.io/github/actions/workflow/status/OWNER/REPO/ci.yml?branch=main)](https://github.com/OWNER/REPO/actions)
[![MCP](https://img.shields.io/badge/MCP-Server-5B8DEF?logo=modelcontextprotocol)](https://modelcontextprotocol.io)
[![Version](https://img.shields.io/crates/v/CRATE_NAME)](https://crates.io/crates/CRATE_NAME)
```

---

## V7: Hero Lacks Visual Proof
**Frequency**: 4/10 (slideforge, metatrader, aportal, reddit-httpx partially)

**Detection**: First 500 characters contain no `![](` or `<img`

**Fix by composition mode** (from project-native-hero):

| Mode | When to Use | Template |
|------|-------------|----------|
| **Split** | One clear visual proof | Title left, screenshot/diagram right |
| **Integrated** | Content = identity (code, diagram) | Title woven into artifact |
| **Artifact Wall** | Multiple outputs explain product | 3-6 real outputs diagonal, title in negative space |
| **Background Proof** | One artifact dominates | Large screenshot behind/around title |
| **Title-Only** | Intentionally minimal, no honest proof | Typography + spacing only |

**Decision checklist**:
1. Can proof be understood at GitHub content width? → If no, use Title + Proof below
2. Does one artifact explain the product? → If yes, One-board hero
3. Does visitor need to compare outputs? → If yes, Artifact wall
4. Will title change often? → If yes, keep SVG title separate

---

## V8: Jargon in Value Proposition
**Frequency**: 2/10 (aportal "reverse-engineered AI web providers", metatrader "Docker-based deployment with noVNC")

**Detection**: First paragraph contains implementation words before outcome words.

**Fix**: Lead with user outcome, follow with mechanism.
```markdown
## Before
A high-performance Python AI Gateway for 9 reverse-engineered AI web providers — no API keys required.

## After
Access 9 AI models (Claude, GPT, Gemini, Kimi, Qwen, DeepSeek, GLM, Perplexity, Meta) from one API — no API keys needed. Python gateway + MCP server.
```

---

## V9: Repeated Promises
**Frequency**: 3/10 (social-forge "high-performance" ×3, "triple-interface" ×2, igs-rust "MCP server + CLI" ×4)

**Detection**: Same value claim in Hero + What It Does + Architecture + Features

**Fix**: State once in Hero, reference in later sections ("As shown above, X provides...")

---

## V10: LICENSE/CONTRIBUTING/CHANGELOG Sections in README
**Frequency**: 1/10 (metatrader has License section)

**Detection**: `grep -i "## License\|## Contributing\|## Changelog" README.md`

**Fix**: Delete section; ensure LICENSE.md, CONTRIBUTING.md, CHANGELOG.md exist in repo root.

---

## V11: Emoji Overuse
**Frequency**: 2/10 (social-forge, metatrader have high emoji density)

**Detection**: `grep -oP '[\x{1F300}-\x{1F9FF}]' README.md | wc -l` > 15

**Fix**: Keep emoji only in:
- Badge alt text (not rendered)
- Section headers (max 1 per H2)
- Checkmarks in tables (✅/❌/🚧)

---

## V12: No End-to-End Code Example
**Frequency**: 3/10 (operant, slideforge, metatrader)

**Detection**: No code block showing `install → config → run → output` in sequence

**Fix**: Add one "Golden Path" example:
```markdown
## Quick Start

```bash
# 1. Install
pip install openscript

# 2. Configure
export PEXELS_API_KEY=xxx

# 3. Create script
cat > my_video.json <<'EOF'
{"title": "Test", "scenes": [{"text": "Hello world"}]}
EOF

# 4. Run
openscript script-to-video --script my_video.json --output out.mp4

# 5. Result: out.mp4 (vertical, captioned, with TTS)
```
```

---

## Detection Script (run on any README)

```bash
#!/bin/bash
# detect_violations.sh

readme="$1"
echo "=== VIOLATION SCAN: $readme ==="

# V1: Architecture before Proof
arch_line=$(grep -n "^## Architecture\|^## Tech Stack\|^## Project Structure" "$readme" | head -1 | cut -d: -f1)
proof_line=$(grep -n "^## Screenshots\|^## Demo\|^## Output\|^## Proof\|^## Visual" "$readme" | head -1 | cut -d: -f1)
if [ -n "$arch_line" ] && [ -n "$proof_line" ] && [ "$arch_line" -lt "$proof_line" ]; then
  echo "❌ V1: Architecture (line $arch_line) before Proof (line $proof_line)"
fi

# V2: Zero visual proof
img_count=$(grep -c "!\[.*\](" "$readme" || echo 0)
badge_count=$(grep -c "img.shields.io" "$readme" || echo 0)
real_images=$((img_count - badge_count))
if [ "$real_images" -eq 0 ]; then
  echo "❌ V2: Zero visual proof (only badges)"
elif [ "$real_images" -lt 3 ]; then
  echo "⚠️  V2: Only $real_images visual proof(s) (need ≥3)"
fi

# V3: Install after Architecture
install_line=$(grep -n "^## Install\|^## Quick Start\|^## Getting Started" "$readme" | head -1 | cut -d: -f1)
if [ -n "$arch_line" ] && [ -n "$install_line" ] && [ "$install_line" -gt "$arch_line" ]; then
  echo "❌ V3: Install (line $install_line) after Architecture (line $arch_line)"
fi

# V4: Excessive length
lines=$(wc -l < "$readme")
if [ "$lines" -gt 500 ]; then
  echo "❌ V4: $lines lines (>500, needs condensing)"
elif [ "$lines" -gt 300 ]; then
  echo "⚠️  V4: $lines lines (>300, consider condensing)"
fi

# V5: Tables for structured data
table_count=$(grep -c "|.*|.*|" "$readme" || echo 0)
feature_bullets=$(grep -c "^-.*:" "$readme" || echo 0)
if [ "$table_count" -eq 0 ] && [ "$feature_bullets" -gt 5 ]; then
  echo "❌ V5: $feature_bullets feature bullets but 0 tables"
fi

# V6: Badges
if [ "$badge_count" -eq 0 ]; then
  echo "❌ V6: Zero badges"
elif [ "$badge_count" -gt 10 ]; then
  echo "⚠️  V6: $badge_count badges (>10, reduce)"
fi

# V7: Hero visual
first_500=$(head -c 500 "$readme")
if ! echo "$first_500" | grep -q "!\[.*\](" && ! echo "$first_500" | grep -q "<img"; then
  echo "❌ V7: Hero lacks visual proof (first 500 chars)"
fi

# V8: Jargon in first para
first_para=$(grep -v "^#" "$readme" | grep -v "^!" | grep -v "^$" | head -1)
jargon_words="gateway reverse-engineered deployment architecture orchestration substrate"
for w in $jargon_words; do
  if echo "$first_para" | grep -qi "$w"; then
    echo "⚠️  V8: Possible jargon '$w' in first paragraph"
  fi
done

# V12: End-to-end example
if ! grep -A 10 "Quick Start\|Install" "$readme" | grep -q "```"; then
  echo "❌ V12: No code example in Quick Start section"
fi

echo "=== SCAN COMPLETE ==="
```

---

## Severity Classification

| Severity | Violations | Blocks Adoption? |
|----------|------------|------------------|
| **P0 — Critical** | V1, V2, V7 | Yes — visitor leaves before understanding value |
| **P1 — High** | V3, V8, V12 | Yes — friction to first success; unclear promise |
| **P2 — Medium** | V4, V5, V6 | No — but reduces conversion/trust |
| **P3 — Low** | V9, V10, V11 | No — polish only |

---

## Repo-Specific Fix Priority (from July 2026 audit)

| Repo | P0 | P1 | P2 | P3 | Total Effort |
|------|----|----|----|----|--------------|
| slideforge | V1, V2, V7 | V3, V6, V12 | V5 | V11 | High (needs visuals) |
| metatrader-docker-deployment | V1, V2, V7 | V3, V6, V12 | V4, V5 | V10 | High (needs visuals) |
| aportal | V2, V7 | V8 | V4, V5, V6 | V9 | Medium |
| social-forge | - | - | V4 | V9, V11 | Low (condense) |
| igs-rust | - | - | V4 | V9 | Low (condense + split) |
| openscript | V1 | V3 | - | - | Low (reorder) |
| reddit-httpx | V1 | V3 | V4 | - | Low (reorder) |
| tdg-rust | - | - | V4 | - | Low (condense) |
| operant | - | V12 | V5 | - | Low (add table + example) |
| automaton | - | - | - | - | **Zero violations** |
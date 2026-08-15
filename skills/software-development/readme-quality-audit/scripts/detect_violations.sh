#!/usr/bin/env bash
# detect_violations.sh — Quick violation scan for any README
# Usage: ./detect_violations.sh README.md

set -euo pipefail

readme="${1:-README.md}"

if [[ ! -f "$readme" ]]; then
    echo "Usage: $0 <README.md>"
    exit 1
fi

echo "=== VIOLATION SCAN: $readme ==="

# V1: Architecture before Proof
arch_line=$(grep -n "^## Architecture\|^## Tech Stack\|^## Project Structure" "$readme" | head -1 | cut -d: -f1)
proof_line=$(grep -n "^## Screenshots\|^## Demo\|^## Output\|^## Proof\|^## Visual\|^## What It Does" "$readme" | head -1 | cut -d: -f1)
if [[ -n "$arch_line" && -n "$proof_line" && "$arch_line" -lt "$proof_line" ]]; then
    echo "❌ V1: Architecture (line $arch_line) before Proof (line $proof_line)"
fi

# V2: Zero visual proof
img_count=$(grep -c "!\[.*\](" "$readme" 2>/dev/null || echo 0)
badge_count=$(grep -c "img.shields.io" "$readme" 2>/dev/null || echo 0)
real_images=$((img_count - badge_count))
if [[ "$real_images" -eq 0 ]]; then
    echo "❌ V2: Zero visual proof (only badges)"
elif [[ "$real_images" -lt 3 ]]; then
    echo "⚠️  V2: Only $real_images visual proof(s) (need ≥3)"
fi

# V3: Install after Architecture
install_line=$(grep -n "^## Install\|^## Quick Start\|^## Getting Started" "$readme" | head -1 | cut -d: -f1)
if [[ -n "$arch_line" && -n "$install_line" && "$install_line" -gt "$arch_line" ]]; then
    echo "❌ V3: Install (line $install_line) after Architecture (line $arch_line)"
fi

# V4: Excessive length
lines=$(wc -l < "$readme")
if [[ "$lines" -gt 500 ]]; then
    echo "❌ V4: $lines lines (>500, needs condensing)"
elif [[ "$lines" -gt 300 ]]; then
    echo "⚠️  V4: $lines lines (>300, consider condensing)"
fi

# V5: Tables for structured data
table_count=$(grep -c "|.*|.*|" "$readme" 2>/dev/null || echo 0)
feature_bullets=$(grep -c "^-.*:" "$readme" 2>/dev/null || echo 0)
if [[ "$table_count" -eq 0 && "$feature_bullets" -gt 5 ]]; then
    echo "❌ V5: $feature_bullets feature bullets but 0 tables"
fi

# V6: Badges
if [[ "$badge_count" -eq 0 ]]; then
    echo "❌ V6: Zero badges"
elif [[ "$badge_count" -gt 10 ]]; then
    echo "⚠️  V6: $badge_count badges (>10, reduce)"
fi

# V7: Hero visual
first_500=$(head -c 500 "$readme")
if ! echo "$first_500" | grep -q "!\[.*\](" && ! echo "$first_500" | grep -q "<img"; then
    echo "❌ V7: Hero lacks visual proof (first 500 chars)"
fi

# V8: Jargon in first para
first_para=$(grep -v "^#" "$readme" | grep -v "^!" | grep -v "^$" | head -1)
jargon_words="gateway reverse-engineered deployment architecture orchestration substrate middleware"
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
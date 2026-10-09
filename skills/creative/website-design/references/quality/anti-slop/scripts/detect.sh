#!/usr/bin/env bash
# Anti-slop automated detector for website-design
# Checks for 17 templated tells, absolute bans, and slop patterns.
# Usage: ./detect.sh [options] [target_path]
# Options:
#   --check <name|number>  Run only one specific check (1-17 or check name)
#   --all                  Run all checks (default)
#   --quiet                Show summary only
#   --fail-fast            Stop on first failed check

set -euo pipefail

TARGET_PATH="."
FILTER_CHECK=""
QUIET=false
FAIL_FAST=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)
      FILTER_CHECK="$2"
      shift 2
      ;;
    --all)
      FILTER_CHECK=""
      shift
      ;;
    --quiet)
      QUIET=true
      shift
      ;;
    --fail-fast)
      FAIL_FAST=true
      shift
      ;;
    -h|--help)
      echo "Usage: $0 [--check <name|1-17>] [--quiet] [--fail-fast] [target_path]"
      exit 0
      ;;
    *)
      TARGET_PATH="$1"
      shift
      ;;
  esac
done

if [[ ! -e "$TARGET_PATH" ]]; then
  echo "Error: Target path '$TARGET_PATH' does not exist." >&2
  exit 2
fi

# Locate candidate files, excluding vendor and build artifacts
find_files() {
  if [[ -f "$TARGET_PATH" ]]; then
    echo "$TARGET_PATH"
  else
    find "$TARGET_PATH" -type f \
      \( -name "*.html" -o -name "*.jsx" -o -name "*.tsx" -o -name "*.vue" -o -name "*.svelte" -o -name "*.astro" -o -name "*.css" -o -name "*.md" -o -name "*.json" \) \
      ! -path "*/node_modules/*" \
      ! -path "*/.git/*" \
      ! -path "*/dist/*" \
      ! -path "*/build/*" \
      ! -path "*/.next/*" \
      ! -path "*/.astro/*" \
      ! -path "*/.svelte-kit/*" \
      ! -path "*/.output/*" \
      ! -path "*/coverage/*" \
      ! -path "*/quality/anti-slop/*" 2>/dev/null || true
  fi
}

PASSED_COUNT=0
FAILED_COUNT=0
TOTAL_CHECKS=0

run_check() {
  local num="$1"
  local name="$2"
  local desc="$3"
  local pattern="$4"
  local match_type="${5:-regex}"

  if [[ -n "$FILTER_CHECK" ]]; then
    if [[ "$FILTER_CHECK" != "$num" && "$FILTER_CHECK" != "$name" ]]; then
      return 0
    fi
  fi

  TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
  local matches=""

  local files
  files=$(find_files)

  if [[ -z "$files" ]]; then
    if [[ "$QUIET" = false ]]; then
      printf "[SKIP] %02d %s: No candidate files found\n" "$num" "$name"
    fi
    return 0
  fi

  case "$match_type" in
    regex)
      matches=$(echo "$files" | xargs -r grep -P -n "$pattern" 2>/dev/null || true)
      ;;
    count-eyebrows)
      # Check eyebrow density across files: fails if eyebrow count > max allowed (1 per 3 sections)
      local total_eyebrows=0
      local found_lines=""
      while IFS= read -r f; do
        local file_matches
        file_matches=$(grep -P -n "(uppercase\s+tracking-(?:wider|widest|\[0\.[1-3][0-9]*em\])|tracking-(?:wider|widest|\[0\.[1-3][0-9]*em\])\s+uppercase)" "$f" 2>/dev/null || true)
        if [[ -n "$file_matches" ]]; then
          local count
          count=$(echo "$file_matches" | wc -l)
          total_eyebrows=$((total_eyebrows + count))
          found_lines="${found_lines}${file_matches}"$'\n'
        fi
      done <<< "$files"
      if [[ $total_eyebrows -gt 3 ]]; then
        matches="$found_lines"
      fi
      ;;
    duplicate-cta)
      # Check for multiple synonymous CTAs in same file
      while IFS= read -r f; do
        local has_contact1 has_contact2 has_start1 has_start2
        has_contact1=$(grep -P -i -c "get in touch" "$f" 2>/dev/null || true)
        has_contact2=$(grep -P -i -c "contact us|let's talk|reach out" "$f" 2>/dev/null || true)
        has_start1=$(grep -P -i -c "get started" "$f" 2>/dev/null || true)
        has_start2=$(grep -P -i -c "try free|start free|sign up free" "$f" 2>/dev/null || true)

        if [[ $has_contact1 -gt 0 && $has_contact2 -gt 0 ]]; then
          matches="${matches}${f}: Duplicate CTA intent found (contact)\n"
        fi
        if [[ $has_start1 -gt 0 && $has_start2 -gt 0 ]]; then
          matches="${matches}${f}: Duplicate CTA intent found (signup)\n"
        fi
      done <<< "$files"
      ;;
  esac

  if [[ -z "$matches" ]]; then
    PASSED_COUNT=$((PASSED_COUNT + 1))
    if [[ "$QUIET" = false ]]; then
      printf "\033[0;32m[PASS]\033[0m %02d %s: %s\n" "$num" "$name" "$desc"
    fi
  else
    FAILED_COUNT=$((FAILED_COUNT + 1))
    printf "\033[0;31m[FAIL]\033[0m %02d %s: %s\n" "$num" "$name" "$desc"
    if [[ "$QUIET" = false ]]; then
      echo "$matches" | head -n 10 | sed 's/^/       /'
      local total_lines
      total_lines=$(echo "$matches" | wc -l)
      if [[ $total_lines -gt 10 ]]; then
        echo "       ... and $((total_lines - 10)) more match(es)"
      fi
    fi
    if [[ "$FAIL_FAST" = true ]]; then
      echo "Stopped on first failure (--fail-fast)."
      exit 1
    fi
  fi
}

# 17 Automated Anti-Slop Checks

# 01 Em-dash and en-dash ban (including unicode and html entities)
run_check 1 "em-dash" \
  "No em-dash or en-dash characters (use standard hyphen)" \
  "[\x{2014}\x{2013}]|&mdash;|&ndash;|&#8212;|&#8211;"

# 02 Banned warm-beige background hexes
run_check 2 "banned-warm-beige-bg" \
  "No banned warm-beige / cream background hexes (#f5f1ea, #f7f5f1, #fbf8f1, #efeae0, #ece6db, #faf7f1, #e8dfcb)" \
  "(?i)#(f5f1ea|f7f5f1|fbf8f1|efeae0|ece6db|faf7f1|e8dfcb)\b"

# 03 Banned warm craft accent hexes
run_check 3 "banned-warm-beige-accent" \
  "No banned brass / clay / oxblood / ochre accent hexes (#b08947, #b6553a, #9a2436, #9c6e2a, #bc7c3a, #7d5621)" \
  "(?i)#(b08947|b6553a|9a2436|9c6e2a|bc7c3a|7d5621)\b"

# 04 Banned warm near-black text hexes
run_check 4 "banned-warm-beige-text" \
  "No banned warm near-black / espresso text hexes (#1a1714, #1a1814, #1b1814)" \
  "(?i)#(1a1714|1a1814|1b1814)\b"

# 05 AI purple and violet glow tokens
run_check 5 "ai-purple-glow" \
  "No default AI purple/violet glow tokens or saturated purple gradients" \
  "(?i)(shadow-purple|shadow-violet|rgba\(\s*168\s*,\s*85\s*,\s*247|rgba\(\s*147\s*,\s*51\s*,\s*234|rgba\(\s*139\s*,\s*92\s*,\s*246|from-purple-(?:500|600)\s+to-indigo|#(8b5cf6|a855f7|7c3aed))\b"

# 06 Inter + Slate default pairing
run_check 6 "inter-slate-default" \
  "No default Inter paired with Slate-900 surface (choose intentional typography)" \
  "(?i)(font-inter.*(?:slate-900|slate-950)|(?:slate-900|slate-950).*font-inter|font-sans.*slate-900)"

# 07 Triple equal card grid default
run_check 7 "triple-equal-cards" \
  "No unvaried 3-column equal card grid patterns" \
  "(?i)grid-cols-1\s+md:grid-cols-3\s+gap-[0-9]+\s*\">(\s*<div[^>]*class=\"[^\"]*p-[0-9]+[^\"]*rounded[^\"]*\"){3}"

# 08 Eyebrow spam density
run_check 8 "eyebrow-spam" \
  "Eyebrow density must not exceed 1 per 3 sections" \
  "" \
  "count-eyebrows"

# 09 Section number labels
run_check 9 "section-number-labels" \
  "No decorative section numbers in headers (01 /, 001 ·, Step 01:)" \
  "(?i)(?:^|>|\"|\')\s*(?:0[0-9]\s*\/|00[0-9]\s*·|0[0-9]\s*·|Step\s*0?[1-9]\s*:|Phase\s*0?[1-9]\s*:|Stage\s*0?[1-9]\s*:)"

# 10 Middle dot overuse
run_check 10 "middle-dot-overuse" \
  "Middle dot rationed to max 1 per line in metadata strips" \
  "·.*·.*·"

# 11 Generic placeholder names and fake metrics
run_check 11 "generic-names-data" \
  "No generic placeholder names (John Doe, Acme) or fake-perfect numbers (99.99%)" \
  "(?i)\b(John\s+Doe|Jane\s+Doe|Sarah\s+Chan|Jack\s+Su|Acme(?:\s+Corp|\s+Co)?|SmartFlow|Cloudly|99\.99%|99\.9%)\b"

# 12 Marketing filler verbs
run_check 12 "filler-verbs" \
  "No lazy marketing filler buzzwords (elevate, seamlessly, unleash, revolutionize)" \
  "(?i)\b(elevate\s+your|seamlessly\s+integrate|unleash\s+the\s+power|revolutionize\s+your|next-gen\s+solution|all-in-one\s+platform)\b"

# 13 Div-based fake screenshots
run_check 13 "fake-screenshots" \
  "No div-based fake screenshots or fake browser/terminal window dots" \
  "(?i)(bg-gray-800.*rounded-t.*p-2.*space-x-2|bg-red-500.*bg-yellow-500.*bg-green-500|mock-terminal|fake-dashboard)"

# 14 Arrow link spam
run_check 14 "arrow-link-spam" \
  "No repetitive arrow symbols on every single link or button" \
  "(?i)(?:href=[^>]*>[^<]*[\x{2192}]|href=[^>]*>[^<]*->|href=[^>]*>[^<]*&rarr;)"

# 15 Glassmorphism overuse
run_check 15 "glass-overuse" \
  "No unmotivated backdrop-blur without inner borders and solid fallbacks" \
  "(?i)backdrop-blur-(?:md|lg|xl)\s+(?!.*(?:border-white|shadow-\[inset))"

# 16 Pure black / white surface extremes
run_check 16 "pure-black-white" \
  "No hardcoded pure #000000 / #ffffff on main container backgrounds" \
  "(?i)bg-\[#(?:000000|ffffff)\]"

# 17 Duplicate CTA intent
run_check 17 "duplicate-cta-intent" \
  "No conflicting duplicate CTA labels on single page (one label per intent)" \
  "" \
  "duplicate-cta"

echo "---------------------------------------------------------"
echo "Anti-slop check results: $PASSED_COUNT passed, $FAILED_COUNT failed ($TOTAL_CHECKS checks executed)"

if [[ $FAILED_COUNT -gt 0 ]]; then
  exit 1
fi
exit 0

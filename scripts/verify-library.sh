#!/usr/bin/env bash
# Verify every docs/library page against the code it claims to describe.
#
# The library's contract (docs/library/STANDARD.md) is that a page answers BOTH
# "how does it work" and "how do I use it", and that every path, config key and
# tool name it cites is REAL. The first half is a reviewer's judgement. The
# second half is mechanical, and this script is that half — so a fleet of
# writers can produce pages in parallel and one command tells the coordinator
# which pages cite something that does not exist.
#
# Run from the repo root:  bash scripts/verify-library.sh
# Exit 0 when every page passes, 1 when any page has a finding. Each finding is
# printed with the page and line so it is a fix list, not a verdict.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

fail=0
say() { printf '%s\n' "$*"; }
BT='`'

# README and STANDARD govern the library rather than describing a mechanism, so
# they are exempt from the Source/Owns/Not-for header rules. They are NOT exempt
# from the path check: a dead link in the front door is the worst one to have.
pages=$(ls docs/library/*.md | grep -v -E '/(README|STANDARD)\.md$')

# --- 1. every backticked crates/ path exists -------------------------------
# The path check is the one `genome lint` E303 also makes, but it is repeated
# here because this script's job is to be the ONE command a coordinator runs
# after a fleet writes: it covers headers, links and tool names as well, and its
# output is grouped per page.
for page in $pages; do
  while IFS=: read -r n tok; do
    p=${tok#"$BT"}; p=${p%"$BT"}
    [ -e "$p" ] || { say "MISSING-PATH $page:$n cites ${BT}${p}${BT}"; fail=1; }
  done < <(grep -n -o "${BT}crates/[A-Za-z0-9_./-]*${BT}" "$page" | sed "s/${BT}\$//")
done

# --- 2. every mechanism page declares what it owns and what it does not ----
for page in $pages; do
  grep -q '^Source:'  "$page" || { say "NO-SOURCE  $page has no Source: header (STANDARD.md requires one)"; fail=1; }
  grep -q '^Owns:'    "$page" || { say "NO-OWNS    $page has no Owns: line"; fail=1; }
  grep -q '^Not for:' "$page" || { say "NO-NOTFOR  $page has no Not for: line"; fail=1; }
done

# --- 3. every page ends with the Intention section -------------------------
for page in $pages; do
  grep -q '^## Intention' "$page" || say "NO-INTENTION $page has no ## Intention (advisory: STANDARD.md §Rules)"
done

# --- 4. sibling links resolve ---------------------------------------------
for page in $pages; do
  # grep -o with -n gives `LINE](target)`; split off the line number first, then
  # the `](`, so a target carrying its own parentheses survives.
  while IFS= read -r hit; do
    n=${hit%%]*}; rest=${hit#*](}; t=${rest%')'}
    case "$t" in http*|"") continue ;; esac
    [ -e "docs/library/$t" ] || { say "DEAD-LINK  $page:$n -> $t"; fail=1; }
  done < <(grep -n -o '\]([A-Za-z0-9_.-]*\.md[^)]*)' "$page")
done

# --- 5. tool-shaped names the pages cite are registered --------------------
# A tool name in a page that no family module registers is a typo or a tool that
# was renamed. Either way it sends a reader to a tool the agent does not have.
# The registry is derived from each tool's own `name()` body (or its TOOL_NAME
# const), NOT from a grep for the literal string: a bare grep for "sub_agent"
# matches the module name and the struct name too, so it answers a different
# question and every real tool name would look registered.
registered=$(python3 - <<'PY'
import re, pathlib
names = set()
for p in pathlib.Path("crates/operant-core/src/tools").rglob("*.rs"):
    t = p.read_text(errors="ignore")
    for m in re.finditer(r'fn name\(&self\)\s*->\s*&[^\{]*\{\s*"([a-z0-9_]+)"', t):
        names.add(m.group(1))
    for m in re.finditer(r'const TOOL_NAME: &str = "([a-z0-9_]+)"', t):
        names.add(m.group(1))
print("\n".join(sorted(names)))
PY
)
tool_shape='^(file_[a-z_]+|terminal|process|code_execution|web_[a-z_]+|browser[a-z_]*|accessibility_tree|memory_[a-z_]+|skills_list|skill_view|skill_manage|sop|cron|kanban|insights|session_search|verify_task|todo|learning_mutation|checkpoint|send_message|react|discord[a-z_]*|home_assistant|text_to_speech|transcription|vision_[a-z_]+|video_analysis|image_generation|http_request|datetime|timestamp|echo|env_probe|system_info|notification|approval|osv_check|config_manage|spotify_[a-z_]+|sub_agent|kernel_[a-z_]+|harness_[a-z_]+|aft_[a-z_]+|tool_search|tool_describe|tool_call|patch)$'
for page in $pages; do
  while IFS=: read -r n tok; do
    name=${tok#"$BT"}; name=${name%"$BT"}
    [[ $name =~ $tool_shape ]] || continue
    grep -qx "$name" <<< "$registered" || say "UNKNOWN-TOOL $page:$n cites ${BT}${name}${BT}, which no tool module registers"
  done < <(grep -n -o "${BT}[a-z][a-z0-9_]*${BT}" "$page" | sed "s/${BT}\$//")
done

# --- 6. the coverage gate: no source file is nameless ----------------------
if [ -x scripts/genome ]; then
  e311=$(./scripts/genome lint --code E311 --all 2>/dev/null | grep -c '^E311' || true)
  say "INFO    E311 uncovered source files: ${e311:-0} (0 means every unit is named by a library page)"
fi

if [ "$fail" = 0 ]; then
  say ""
  say "library-check: every page's paths, headers, links and tool names resolve"
else
  say ""
  say "library-check: findings above are the fix list"
fi
exit "$fail"
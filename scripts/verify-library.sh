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

# --- 4b. in-page TOC anchors resolve ---------------------------------------
# A Contents block that points at a heading that does not exist is a promise the
# page breaks, and it is invisible to a reader until they click. The anchor
# algorithm is GitHub's: lowercase, drop backticks and emphasis, delete anything
# that is not a word character, space, dot, underscore or dash, then spaces to
# dashes. Underscores and DOTS SURVIVE -- `agents_view.rs` anchors as
# `agents_view.rs`, not `agents_viewrs`, which is the mistake a naive slug
# generator makes and which silently breaks every module-named heading.
for page in $pages; do
  python3 - "$page" <<'PY'
import re, sys
path = sys.argv[1]
lines = open(path, errors="ignore").read().splitlines()

def anchor(title):
    s = title.lower().replace("`", "")
    s = re.sub(r"[^\w\s.-]", "", s)
    return re.sub(r"\s+", "-", s).strip("-")

real = {anchor(l.lstrip("# ").strip()) for l in lines if l.startswith("#")}
bad = [m.group(1) for m in re.finditer(r"\]\(#([^)]+)\)", "\n".join(lines))
       if m.group(1).lower() not in real]
for b in bad:
    print(f"DEAD-ANCHOR {path} -> #{b}")
PY
done 2>&1 | while IFS= read -r line; do say "$line"; fail=1; done

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
for p in pathlib.Path("crates/operant-core/src").rglob("*.rs"):
    t = p.read_text(errors="ignore")
    for m in re.finditer(r'fn name\(&self\)\s*->\s*&[^\{]*\{\s*"([a-z0-9_]+)"', t):
        names.add(m.group(1))
    # Two shapes besides the trait method: a TOOL_NAME const, and the
    # deferred-dispatch trio's TOOL_<X>_NAME consts (tool_search.rs declares
    # TOOL_SEARCH_NAME / TOOL_DESCRIBE_NAME / TOOL_CALL_NAME and registers each
    # through its own path). Missing the second shape would report three real
    # tools as unregistered, which is the false-positive the verifier must not
    # produce: a check that cries wolf on the good pages gets ignored.
    for m in re.finditer(r'const TOOL_NAME: &str = "([a-z0-9_]+)"', t):
        names.add(m.group(1))
    for m in re.finditer(r'const TOOL_[A-Z_]*NAME: &str = "([a-z0-9_]+)"', t):
        names.add(m.group(1))
    # Cross-cutting name SETS the runtime itself dispatches on. A page that
    # quotes NO_EFFECT_TOOL_NAMES is quoting a real list the guardrails read, so
    # those names are registered even though no single tool struct owns them.
    # Derived from the source, like everything else here.
    for m in re.finditer(r'const NO_EFFECT_TOOL_NAMES[^=]*=\s*&\[(.*?)\];', t, re.S):
        names |= set(re.findall(r'"([a-z0-9_]+)"', m.group(1)))
print("\n".join(sorted(names)))
PY
)
# The SHAPE check is deliberately permissive and the SET check is exact: a name
# must be registered somewhere (name() impl, TOOL_*_NAME const, the no-effect
# table, or the alias table) AND look like a tool name. The previous version of
# this line was a hand-written allowlist that itself contained names no module
# registers (sub_agent, learning_mutation, home_assistant, transcription), so it
# validated the wrong answer. RG-022: derive the set, never transcribe it.
tool_shape='^[a-z][a-z0-9_]{2,60}$'
# The alias layer is real and documented (`tools.rs::builtin_tool_aliases`): a
# retired name still resolves, so a page citing it is describing behaviour, not
# inventing a tool. Derived the same way as the registry, from the alias table,
# so a typo'd alias fails exactly like a typo'd tool.
aliases=$(python3 - <<'PY'
import re, pathlib
out = set()
for p in pathlib.Path("crates/operant-core/src").rglob("*.rs"):
    t = p.read_text(errors="ignore")
    for m in re.finditer(r'builtin_tool_aliases', t):
        # the table maps "retired" -> "live"; take the keys of the block that follows
        seg = t[m.end():m.end() + 4000]
        out |= set(re.findall(r'"([a-z0-9_]+)"\s*=>', seg))
        out |= set(re.findall(r'\(\s*"([a-z0-9_]+)"\s*,', seg))
print("\n".join(sorted(out)))
PY
)
# The strongest claim a library page makes about a tool is a SECTION HEADING
# named after it ("## `web_search` - the dispatch"), because that asserts the
# tool exists and this page owns its contract. Checking every backticked token
# would flag config keys, struct fields and locals, so the check is scoped to
# headings and to the tool-name column of a registry table.
# A heading can legitimately be named after a CONFIG VALUE rather than a tool
# ("when its inject mode is `always_on`"). Those are derived from config.rs the
# same way the tool names are, so the check never has to guess a value away.
config_vals=$(cut -d'"' -f2 -s crates/operant-core/src/config.rs operant.example.toml \
  | grep -E '^[a-z][a-z0-9_]{2,40}$' | sort -u)

# Heading lines only. A heading that names a tool asserts that tool exists and
# that this page owns its contract, so an unregistered name there is a real defect.
# (An earlier version scanned every backticked token and reported config keys and
# struct fields, which is a different and useless question.)
for page in $pages; do
  headings=$(grep -n '^#\+ .*' "$page" | grep "${BT}" || true)
  [ -n "$headings" ] || continue
  while IFS= read -r line; do
    n=${line%%:*}
    for tok in $(grep -o "${BT}[a-z][a-z0-9_]*${BT}" <<< "$line" | tr -d "${BT}"); do
      [[ $tok =~ $tool_shape ]] || continue
      grep -qx "$tok" <<< "$registered" && continue
      grep -qx "$tok" <<< "$aliases" && continue
      grep -qx "$tok" <<< "$config_vals" && continue
      say "UNKNOWN-TOOL $page:$n (heading) cites ${BT}${tok}${BT}, which no tool module registers"
      fail=1
    done
  done <<< "$headings"
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
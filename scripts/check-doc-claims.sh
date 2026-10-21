#!/usr/bin/env bash
# Derive the code-surface claims in AGENTS.md against the code, and report every
# difference. Run it from the repo root:  bash scripts/check-doc-claims.sh
#
# The point is that AGENTS.md's "What IS Working" and "Architecture Overview"
# are prose inventories of the code, and a prose inventory drifts in BOTH
# directions: it keeps entries that were deleted and misses entries that were
# added. Reading the list and judging it "looks right" is how that happens.
# This script diffs the claims against the code instead.
#
# Exit 0 when every claim matches, 1 when any differs. Each difference is
# printed, so the output is the fix list.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

fail=0
say() { printf '%s\n' "$*"; }
check() { # name actual expected
  if [ "$2" = "$3" ]; then
    say "OK    $1: $2"
  else
    say "DRIFT $1: doc says $3, code has $2"
    fail=1
  fi
}

# --- CLI command variants: the Commands enum -------------------------------
cli_actual=$(python3 - <<'PY'
import re
src = open('crates/operant-cli/src/main.rs').read()
i = src.index('enum Commands {'); depth = 0
for k in range(i + len('enum Commands {') - 1, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
print(len(re.findall(r'^    ([A-Z][A-Za-z0-9]*)', src[i:end], re.M)))
PY
)
cli_doc=$(grep -oE '^### CLI \([0-9]+ command variants' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "CLI command variants" "$cli_actual" "$cli_doc"

# --- the CLI LIST itself, both directions ---------------------------------
python3 - <<'PY'
import re, sys
src = open('crates/operant-cli/src/main.rs').read()
i = src.index('enum Commands {'); depth = 0
for k in range(i + len('enum Commands {') - 1, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
actual = {v.lower() for v in re.findall(r'^    ([A-Z][A-Za-z0-9]*)', src[i:end], re.M)}
text = open('AGENTS.md').read()
m = re.search(r'### CLI \(\d+ command variants[^\n]*\n+(?:\n*)([a-z][^\n]*\n(?:[a-z][^\n]*\n)*)', text)
listed = set()
for tok in re.split(r'[,\s]+', m.group(1)):
    tok = tok.strip(' .()')
    if tok and re.fullmatch(r'[a-z][a-z/-]*', tok):
        for p in tok.split('/'): listed.add(p)
ghosts = sorted(listed - actual)
missing = sorted(actual - listed)
if ghosts or missing:
    print(f"DRIFT CLI list: {len(ghosts)} listed but not real {ghosts}; {len(missing)} real but not listed {missing}")
    sys.exit(1)
print(f"OK    CLI list: {len(listed)} entries, exact match")
PY
[ $? -ne 0 ] && fail=1

# --- gateway platform adapters --------------------------------------------
# Count the ENTRIES in the registry function, not every `PlatformEntry {` in the
# file: the struct DEFINITION is one of them, and counting it says 8 platforms
# when the registry has 7.
plat_actual=$(python3 - <<'PY'
import re
src = open('crates/operant-cli/src/gateway_runner.rs').read()
i = src.index('fn platform_registry()')
depth = 0
for k in range(i, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
print(len(re.findall(r'PlatformEntry \{', src[i:end])))
PY
)
plat_doc=$(grep -oE 'Supported: [0-9]+ platforms' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "gateway platforms" "$plat_actual" "$plat_doc"

# --- error classes --------------------------------------------------------
err_actual=$(python3 - <<'PY'
import re
src = open('crates/operant-core/src/agent/error_classifier.rs').read()
i = src.index('pub enum FailoverReason {'); depth = 0
for k in range(i + len('pub enum FailoverReason {') - 1, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
print(len(re.findall(r'^    ([A-Z][A-Za-z0-9]*)', src[i:end], re.M)))
PY
)
err_doc=$(grep -oE '[0-9]+ error classes' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "error classes" "$err_actual" "$err_doc"

# Every place AGENTS.md states the taxonomy size must agree, INCLUDING the
# prose forms ("12 classes in the agent loop", "24-class ClassifiedError").
# A document that says 12 in one place and 24 in another is the failure this
# catches — the count is not the point, the agreement is.
#
# The forms WRAP ACROSS LINES ("(12 classes in\n the agent loop)"), so the
# document is flattened to one line before matching. A single-line regex
# silently misses the wrapped form — which is exactly the form that carried
# the wrong number.
tax_alls=$(tr '\n' ' ' < AGENTS.md \
           | grep -oE '[0-9]+[- ]+((error )?classes? (in the agent loop|with ClassifiedError)?|error classes|class .ClassifiedError)' \
           | grep -oE '^[0-9]+' | sort -u | tr '\n' ' ')
if [ "$(echo "$tax_alls" | wc -w)" -gt 1 ]; then
  say "DRIFT taxonomy size: AGENTS.md states it several different ways: $tax_alls"
  fail=1
else
  say "OK    taxonomy size: $err_actual stated consistently"
fi

# --- hook events ----------------------------------------------------------
hook_actual=$(python3 - <<'PY'
import re
src = open('crates/operant-core/src/gateway_pipeline.rs').read()
i = src.index('pub enum HookEvent {'); depth = 0
for k in range(i + len('pub enum HookEvent {') - 1, len(src)):
    if src[k] == '{': depth += 1
    elif src[k] == '}':
        depth -= 1
        if depth == 0: end = k; break
print(len(re.findall(r'^    ([A-Z][A-Za-z0-9]*)', src[i:end], re.M)))
PY
)
hook_doc=$(grep -oE '[0-9]+ events \+ wildcard' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "hook events" "$hook_actual" "$hook_doc"

# --- workspace members -----------------------------------------------------
ws_actual=$(python3 -c "import tomllib;print(len(tomllib.load(open('Cargo.toml','rb'))['workspace']['members']))")
ws_doc=$(grep -oE 'Workspace members \([0-9]+ crates' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "workspace members" "$ws_actual" "$ws_doc"

# --- every path named in the Architecture Overview tree --------------------
# The tree is a hierarchy, so a leaf name like `mcp.rs` is only meaningful
# together with its ancestors. The path is rebuilt from the indentation depth.
python3 - <<'PY'
import re, os, sys
text = open('AGENTS.md').read()
m = re.search(r'## Architecture Overview\s*```\n(.*?)```', text, re.S)
if not m:
    print("DRIFT could not find the Architecture Overview tree"); sys.exit(1)
block = m.group(1)

root = None
stack = []          # (indent, name)
bad = []
bad_features = []
checked = 0
for line in block.splitlines():
    if not line.strip():
        continue
    # the tree glyphs are the indentation; measure them
    stripped = line.lstrip('│ ')
    indent = len(line) - len(stripped)
    name = re.sub(r'^[^A-Za-z0-9_.\-]*', '', line).split('#')[0].strip()
    if not name or name == '.':
        continue
    name = name.rstrip('/')
    if not name:
        continue
    if name == 'operant':
        root = ''
        continue
    while stack and stack[-1][0] >= indent:
        stack.pop()
    stack.append((indent, name))
    path = os.path.join(*[n for _, n in stack]) if stack else name
    path = path.lstrip('/')
    if not path or path.startswith('crates/') is False and '/' not in path and not path.startswith('docs'):
        pass
    checked += 1
    # A tree line that ends in `Cargo.toml` and names FEATURES is a claim
    # about that crate's `[features]` table — check it the same way the paths
    # are checked.
    if path.endswith('Cargo.toml'):
        m_feat = re.search(r'#\s*Features:\s*(.+)$', line)
        if m_feat:
            import tomllib
            declared = set()
            try:
                declared = set(tomllib.load(open(path, 'rb')).get('features', {}))
            except Exception:
                pass
            # The list ends at the first parenthesis — the rest is a gloss.
            for feat in [f.strip().strip('`') for f in m_feat.group(1).split('(')[0].split(',')]:
                if feat and feat not in declared:
                    bad_features.append(f"{path}: {feat}")
    if '*' in path:                       # a glob like cmd_*.rs
        d = os.path.dirname(path) or '.'
        base = os.path.basename(path)     # listdir returns BARE names
        prefix = base.split('*')[0]
        suffix = base.split('*')[1] if '*' in base else ''
        if not os.path.isdir(d) or not [f for f in os.listdir(d)
                                        if f.startswith(prefix) and f.endswith(suffix)]:
            bad.append(path)
        continue
    if not os.path.exists(path):
        bad.append(path)

if bad:
    print("DRIFT tree paths that do not exist:", bad); sys.exit(1)
if bad_features:
    print("DRIFT features named in the tree that the crate does not declare:", bad_features); sys.exit(1)
print(f"OK    Architecture Overview tree: {checked} paths, every one exists")
PY
[ $? -ne 0 ] && fail=1

# --- AFT tools -------------------------------------------------------------
aft_actual=$(python3 - <<'PY'
import re
src = open('crates/operant-core/src/tools/aft_tools.rs').read()
# the REGISTERED tools, counted the way the code registers them
n_reg = len(re.findall(r'\.register\(Aft', src))
# the distinct tool names, counted from the schema definitions
n_names = len({m for m in re.findall(r'"(aft_[a-z_]+)"', src)})
if n_reg != n_names:
    print(f"DRIFT AFT internal: {n_reg} registered but {n_names} distinct names")
    raise SystemExit(2)
print(n_reg)
PY
) || { say "DRIFT AFT: registered tools and distinct names disagree"; fail=1; }
aft_doc=$(grep -oE '[0-9]+ (IDE-grade coding tools|AFT IDE tools|IDE-grade tools)' AGENTS.md | grep -oE '[0-9]+' | head -1)
check "AFT tools" "$aft_actual" "$aft_doc"
# every AFT count in the doc must agree with the code
aft_alls=$(grep -oE '[0-9]+ (IDE-grade coding tools|AFT IDE tools|IDE-grade tools)' AGENTS.md | grep -oE '[0-9]+' | sort -u | tr '\n' ' ')
if [ "$(echo "$aft_alls" | wc -w)" -gt 1 ]; then
  say "DRIFT AFT: AGENTS.md states the AFT count several different ways: $aft_alls"
  fail=1
fi

# --- TUI modules -----------------------------------------------------------
tui_actual=$(grep -c 'pub mod' crates/operant-cli/src/tui/mod.rs)
# The doc may write the count as "63 TUI modules" or as a lower bound
# ("50+ TUI modules"). A lower bound is NOT a checkable claim, so it is
# reported as such rather than silently accepted or silently ignored.
tui_doc=$(grep -oE '[0-9]+\+? TUI modules' AGENTS.md | head -1 | grep -oE '^[0-9]+')
if [ -z "$tui_doc" ]; then
  say "DRIFT TUI modules: AGENTS.md states no countable TUI-module count"
  fail=1
elif [ "$tui_doc" != "$tui_actual" ]; then
  say "DRIFT TUI modules: doc says $tui_doc, code has $tui_actual"
  fail=1
else
  say "OK    TUI modules: $tui_actual"
fi

# --- cargo features named in AGENTS.md must exist --------------------------
# A feature name in the document is a claim that the crate declares it. The
# LifeOS claim was the counter-example: AGENTS.md said "the `lifeos` cargo
# feature" for a feature no crate declared and the tool module no tree
# carries.
python3 - <<'PY'
import glob, re, sys, tomllib

text = open('AGENTS.md').read()
named = set()
for m in re.finditer(r'`([a-z][a-z0-9_-]*)`\s+cargo feature', text):
    before = text[max(0, m.start() - 40):m.start()].lower()
    # A NEGATED mention ("there is no `lifeos` cargo feature") is not a claim.
    if re.search(r'\b(no|not|never|without)\b[^.]*$', before):
        continue
    named.add(m.group(1))

declared = set()
for path in glob.glob('crates/*/Cargo.toml') + ['Cargo.toml']:
    try:
        data = tomllib.load(open(path, 'rb'))
    except Exception:
        continue
    declared |= set(data.get('features', {}))
    declared |= set(data.get('workspace', {}).get('features', {}))

missing = sorted(named - declared)
if missing:
    print("DRIFT cargo features named in AGENTS.md that no crate declares:", missing)
    sys.exit(1)
print(f"OK    cargo features: {len(named)} named in AGENTS.md, every one declared")
PY
[ $? -ne 0 ] && fail=1

# --- the live issue ledger's path -----------------------------------------
if [ -f docs/audit/2026-06-19-BUGS.md ]; then
  say "OK    issue ledger path: docs/audit/2026-06-19-BUGS.md exists"
else
  say "DRIFT issue ledger path: docs/audit/2026-06-19-BUGS.md missing"
  fail=1
fi
if grep -qE '^\s*├── BUGS\.md' AGENTS.md; then
  say "DRIFT AGENTS.md tree still names a root BUGS.md"
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  say ""
  say "docs-check: all AGENTS.md code-surface claims match the code"
else
  say ""
  say "docs-check: differences above are the fix list"
fi
exit "$fail"

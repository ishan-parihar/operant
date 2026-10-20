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
print(f"OK    Architecture Overview tree: {checked} paths, every one exists")
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

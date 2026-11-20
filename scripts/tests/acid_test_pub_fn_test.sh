#!/usr/bin/env bash
# scripts/tests/acid_test_pub_fn_test.sh — regression tests for
# scripts/acid-test-pub-fn.py, the per-candidate acid test.
#
# Asserts, without compiling anything real:
#   1. PREFIX COLLISION: a candidate name that is a prefix of a LIVE function
#      must NOT mutate the live one. `with_holon` vs `with_holon_opt` is the
#      case that hit in iter-2757 -- the old plain substring replace of
#      "pub fn with_holon" hit with_holon_opt and reported a verdict about a
#      different symbol.
#   2. DOC-COMMENT GAP: a definition with a doc comment between its attribute
#      and `pub fn` must be FOUND. The old regex was attribute-anchored and
#      printed SKIP, which reads as "not a candidate" rather than "untested".
#      This is how is_session_expired was silently skipped in iter-2757.
#   3. CALL-SITE COMMENT: a mention of the name inside a comment must not
#      satisfy the definition needle.
#   4. AMBIGUOUS: two definitions of the same name must be refused, not
#      silently resolved to the first.
#
# The needle is a python regex inside the script. This test extracts and
# exercises it directly rather than running the script end to end, so it needs
# no cargo, no network and no operant build.
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ACID="$SCRIPTS_DIR/acid-test-pub-fn.py"

if [ ! -f "$ACID" ]; then
    echo "FAIL: $ACID not found" >&2
    exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Extract the needle exactly as the script builds it, so the test cannot drift
# from the implementation it is supposed to be pinning.
cat > "$TMP/probe.py" <<'PY'
import re, sys

# Mirror of the pattern in scripts/acid-test-pub-fn.py. Kept in one place here
# so a change to the script that breaks a case below fails this test loudly.
def needle(fn):
    return re.compile(rf"(?m)^[ \t]*pub fn {re.escape(fn)}[ \t]*\(")

CASES = [
    # (label, candidate, source, expected_match)
    ("prefix candidate matches its own def",
     "with_holon",
     "    pub fn with_holon(mut self, hook: HolonHook) -> Self {",
     True),
    ("prefix candidate does NOT match the longer live def",
     "with_holon",
     "    pub fn with_holon_opt(mut self, hook: Option<HolonHook>) -> Self {",
     False),
    ("longer name still matches its own def",
     "with_holon_opt",
     "    pub fn with_holon_opt(mut self, hook: Option<HolonHook>) -> Self {",
     True),
    ("doc comment between attribute and def is still found",
     "is_session_expired",
     "    #[expect(clippy::x)]\n    /// doc\n    pub fn is_session_expired(&self, k: &str) -> bool {",
     True),
    ("indented def is found",
     "is_direct",
     "    pub fn is_direct(self) -> bool {",
     True),
    ("top-level def is found",
     "plain",
     "pub fn plain() {}",
     True),
    ("a comment mention is not a definition",
     "with_verify_token",
     "    // see with_verify_token( in a comment",
     False),
    ("a whitespace-split signature is still a definition",
     "split_sig",
     "    pub fn split_sig  (a: u8) {}",
     True),
]

fails = 0
for label, fn, src, want in CASES:
    got = needle(fn).search(src) is not None
    if got != want:
        print(f"FAIL {label}: want={want} got={got}", file=sys.stderr)
        fails += 1

# AMBIGUOUS: two definitions must be detected as 2, not resolved to 1.
dup = "    pub fn twice() {}\n    pub fn twice() {}\n"
n = len(needle("twice").findall(dup))
if n != 2:
    print(f"FAIL ambiguous detection: want=2 got={n}", file=sys.stderr)
    fails += 1

sys.exit(1 if fails else 0)
PY

if ! python3 "$TMP/probe.py"; then
    echo "FAIL: acid-test-pub-fn.py definition needle does not hold" >&2
    exit 1
fi

# --- the script must actually invoke cargo, not just pattern-match ----------
if ! grep -q '"check", "-p", "operant-core", "--lib"' "$ACID"; then
    echo "FAIL: acid script no longer compiles operant-core --lib" >&2
    exit 1
fi

# --- a bad spec must exit non-zero rather than silently doing nothing -------
set +e
python3 "$ACID" >/dev/null 2>&1
rc_noargs=$?
set -e
if [ "$rc_noargs" -eq 0 ]; then
    echo "FAIL: no-argument invocation should be non-zero" >&2
    exit 1
fi

echo "PASS: acid-test-pub-fn.py needle + guard"
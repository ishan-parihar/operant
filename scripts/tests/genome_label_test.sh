#!/usr/bin/env bash
# scripts/tests/genome_label_test.sh — the iteration label is READ, not computed.
#
# The defect this pins (2026-11-14 swarm audit, finding 16): the label was
# hand-computed before every push ("read the top from origin/main, add one"), and
# the hand computation raced. Four renumbers in one session came from reading a
# stale top or from a peer taking the number between the read and the push. The
# number is a fact about origin/main, so it is read from git.
#
# Asserted here, against a toy repo (the real repo is only READ):
#   1. the next label is the highest iter-N on origin/main, plus one
#   2. a DUPLICATE label does not shift it (max, not count)
#   3. subjects with no iter-N are ignored
#   4. --top prints the current top instead of the next
#   5. a repo with no iter-N label fails loudly rather than printing iter-1
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENOME="$REPO/scripts/genome"
TOY="/tmp/genome-label.$$"
trap 'rm -rf "$TOY"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

command -v git >/dev/null || fail "git is required"
command -v python3 >/dev/null || fail "python3 is required"

new_toy() {
  rm -rf "$TOY"
  mkdir -p "$TOY"
  ( cd "$TOY" \
    && git init -q . \
    && git config user.email test@example.invalid \
    && git config user.name test \
    && git config commit.gpgsign false )
}

# A commit on a branch, published as origin/main.
push_subject() {
  ( cd "$TOY" \
    && printf '%s\n' "$1" >> history.txt \
    && git add history.txt \
    && git commit -q -m "$1" \
    && git update-ref refs/remotes/origin/main HEAD )
}

# 1. the next label is max+1
new_toy
push_subject "feat(iter-40): first"
push_subject "fix(iter-42): second"
out="$(cd "$TOY" && "$GENOME" label --no-fetch 2>&1)" || fail "label must succeed: $out"
[ "$out" = "iter-43" ] || fail "expected iter-43, got $out"

# 2. a duplicate label does not shift the answer (max, not count)
push_subject "fix(iter-42): a duplicate label, the cosmetic defect AGENTS.md describes"
out="$(cd "$TOY" && "$GENOME" label --no-fetch 2>&1)" || fail "label must survive a duplicate: $out"
[ "$out" = "iter-43" ] || fail "a duplicate label must not change the next: $out"

# 3. subjects with no iter-N are ignored
push_subject "docs: a subject with no label at all"
push_subject "feat: subject (iter-99) in a non-leading position"
out="$(cd "$TOY" && "$GENOME" label --no-fetch 2>&1)" || fail "label must skip unlabelled subjects: $out"
[ "$out" = "iter-100" ] || fail "an embedded iter-99 is still a label: $out"

# 4. --top reports the current top, not the next
out="$(cd "$TOY" && "$GENOME" label --top --no-fetch 2>&1)" || fail "--top must succeed: $out"
[ "$out" = "iter-99" ] || fail "expected the top to be iter-99, got $out"

# 5. a repo with no iter-N label fails loudly
new_toy
push_subject "feat: no iteration label here"
out="$(cd "$TOY" && "$GENOME" label --no-fetch 2>&1)" && fail "an unlabelled history must not yield a label: $out"
echo "$out" | grep -q 'no iter-N label' || fail "the failure must say what is wrong: $out"

echo "ok: the iteration label is read from origin/main, not computed by hand"
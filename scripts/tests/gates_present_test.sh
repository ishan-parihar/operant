#!/usr/bin/env bash
# The gate-present gate's own acceptance test (iter-2585).
#
# `scripts/check-gates-present.sh` exists because a rebase or a stale-worktree
# reset silently DROPPED checks out of the gate scripts twice in one session
# (iters 2549 and 2551). It asserts that the markers those checks depend on are
# still present. That makes it load-bearing: if IT rots into a no-op that always
# exits 0, every check it claims to guard is unprotected and nothing says so.
#
# So this script is a gate on the gate, and it works the same way the config-key
# test does -- by MUTATION against a scratch copy of the tree. The real tree is
# never modified.
#
# The criterion is a mutation in each direction:
#   green tree                        -> exit 0
#   remove one required marker        -> exit 1, naming the file and the marker
#   remove one agentmemory_* field    -> exit 1, SHIM-DRIFT (field count)
#   soften the compat caveat prose    -> exit 1, MISSING-MARKER (caveat half)
#
# That last case is the one that was WRONG until iter-2584: the caveat check
# pinned a single sentence, so rewriting the adjacent sentence to argue the
# opposite ("fine to do casually") while all four fields sat there passed with
# exit 0 and the message "every required marker present". A check that only
# tests that a sentence EXISTS cannot notice the sentence being contradicted.
set -u

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# This script lives in scripts/tests/, so the repo root is two levels up.
ROOT="$(cd "$DIR/../.." && pwd)"
GATE="$ROOT/scripts/check-gates-present.sh"
CFG="$ROOT/crates/operant-core/src/config.rs"

fail=0
note() { printf '%s\n' "$*"; }
bad()  { printf 'FAIL: %s\n' "$*"; fail=1; }

[ -f "$GATE" ] || { note "FAIL: $GATE is missing"; exit 1; }
[ -f "$CFG"  ] || { note "FAIL: $CFG is missing"; exit 1; }

# ── 1. the gate is GREEN on the tree as it stands ──────────────────
# Everything below is a mutation of a green baseline. If the baseline is red the
# mutations prove nothing: a script that always exits 1 would "catch" every one.
if ! bash "$GATE" >/dev/null 2>&1; then
    bad "the gate is red on the current tree -- fix the findings before trusting this test"
fi

# ── scratch mirror ─────────────────────────────────────────────────
# The gate resolves its own paths with `cd "$(dirname "$0")/.."`, so a scratch
# copy of the scripts+config pair is enough to run it against a mutated config.
# It only needs the files it actually reads; anything absent reports its own
# MISSING-GATE, which would confuse the assertions, so the mirror is complete
# for every path the gate names.
scratch="$(mktemp -d)"
mkdir -p "$scratch/scripts" "$scratch/crates/operant-core/src"
cp "$GATE" "$scratch/scripts/"
cp "$CFG"  "$scratch/crates/operant-core/src/config.rs"

# Every other path the gate requires, copied so a missing-file report cannot be
# mistaken for a mutation being caught.
while IFS= read -r p; do
    case "$p" in
        scripts/check-gates-present.sh|crates/operant-core/src/config.rs) continue ;;
    esac
    [ -n "$p" ] || continue
    [ -e "$ROOT/$p" ] || continue
    mkdir -p "$scratch/$(dirname "$p")"
    cp -R "$ROOT/$p" "$scratch/$p"
done <<EOF
$(grep -oE '^(DC|VL|MC)=[^ ]+' "$GATE" | cut -d= -f2)
$(grep -oE 'require [^ ]+' "$GATE" | awk '{print $2}')
EOF

run_gate() { ( cd "$scratch" && bash scripts/check-gates-present.sh 2>&1 ); }

# ── 2. the scratch mirror is green (control) ───────────────────────
if ! run_gate >/dev/null; then
    bad "the scratch mirror is red before any mutation -- the mirror is incomplete, not the gate wrong"
fi

# Restore EVERY file a mutation touched, not just config.rs. The marker probe
# strips a line from a gate script, so restoring only config.rs leaves that
# mutation in place and the final green check fails for a reason that has
# nothing to do with the gate (caught once, iter-2585).
restore_all() {
    cp -R "$pristine"/. "$scratch"/
}
pristine="$(mktemp -d)"
trap 'rm -rf "$scratch" "$pristine"' EXIT
cp -R "$scratch"/. "$pristine"/

# ── 3. remove a required marker -> FAIL, and it names the file ─────
# Anchor on a marker the gate itself declares, so this test tracks the gate
# rather than a hardcoded guess at its contents. `require <file> <marker> ...`
# may span lines with `\` continuations, so the marker is read as the second
# shell word after joining them; a naive `head -1 | awk '{print $2}'` reads a
# function signature instead (that was wrong once -- iter-2585).
probe_lines="$(grep -E '^require ' "$GATE")"
marker="$(printf '%s\n' "$probe_lines" | head -1 \
  | sed 's/[[:space:]]*\\$//' \
  | sed -E "s/^require +([^ ]+) +'([^']*)'.*/\2/" \
  | sed -E 's/^require +[^ ]+ +"([^"]*)".*/\1/')"
if [ -z "$marker" ]; then
    bad "could not find a 'require' marker in the gate to mutate"
else
    # Pick a file the scratch mirror actually has. `require` is called with
    # SHELL WORDS (`"$DC"`, `"$VL"`), not literal paths, so each candidate is
    # resolved through the gate's own variable assignments before it is tested.
    target=""
    for f in $(printf '%s\n' "$probe_lines" | awk '{print $2}' | sort -u); do
        resolved="$f"
        case "$f" in
            *\$*)
                var="$(printf '%s' "$f" | tr -d '"' | tr -d '$')"
                resolved="$(grep -oE "^${var}=[^ ]+" "$GATE" | head -1 | cut -d= -f2)"
                ;;
        esac
        [ -n "$resolved" ] || continue
        if [ -f "$scratch/$resolved" ] && grep -qF -- "$marker" "$scratch/$resolved"; then target="$resolved"; break; fi
    done
    if [ -z "$target" ]; then
        bad "no mirrored file contains the probe marker [$marker]"
    else
        # Strip the marker from the COPY only.
        python3 - "$scratch/$target" "$marker" <<'PY'
import sys, pathlib
p, marker = sys.argv[1], sys.argv[2]
path = pathlib.Path(p)
lines = path.read_text().splitlines(keepends=True)
kept = [l for l in lines if marker not in l]
if len(kept) == len(lines):
    sys.exit("probe marker not found in %s" % p)
path.write_text("".join(kept))
PY
        out="$(run_gate)"; rc=$?
        if [ "$rc" -eq 0 ]; then
            bad "removing the marker [$marker] from $target PASSED the gate"
        fi
        if ! printf '%s' "$out" | grep -qF "$marker"; then
            bad "the gate failed but did not NAME the missing marker [$marker]"
        fi
    fi
fi
restore_all

# ── 4. remove one agentmemory_* field -> SHIM-DRIFT ────────────────
# The compat shim's whole point is that an existing operant.toml naming a
# retired key still parses. Dropping a field is the half that BREAKS that, and
# the field count is what notices.
python3 - "$scratch/crates/operant-core/src/config.rs" <<'PY'
import sys, pathlib
p = pathlib.Path(sys.argv[1])
s = p.read_text()
needle = "    pub agentmemory_url: Option<String>,\n"
if needle not in s:
    sys.exit("config shape changed: the agentmemory_url field is gone")
p.write_text(s.replace(needle, "", 1))
PY
if [ $? -ne 0 ]; then
    bad "could not apply the field-removal mutation"
else
    out="$(run_gate)"; rc=$?
    if [ "$rc" -eq 0 ]; then
        bad "removing a legacy agentmemory_* field PASSED the gate"
    fi
    if ! printf '%s' "$out" | grep -q 'SHIM-DRIFT'; then
        bad "the field removal failed without naming SHIM-DRIFT"
    fi
fi
restore_all

# ── 5. soften the caveat -> MISSING-MARKER ────────────────────────
# The defect this pins: a check that greps ONE sentence cannot notice the
# sentence next to it being rewritten to argue the opposite. Four dead fields
# plus a note calling their removal "fine to do casually" is exactly the silent
# decision the gate exists to prevent, and it used to pass with exit 0.
python3 - "$scratch/crates/operant-core/src/config.rs" <<'PY'
import sys, pathlib
p = pathlib.Path(sys.argv[1])
s = p.read_text()
needle = "but it must be a deliberate one"
if needle not in s:
    sys.exit("config shape changed: the deliberate-removal caveat is gone")
p.write_text(s.replace(needle, "and it is fine to do casually", 1))
PY
if [ $? -ne 0 ]; then
    bad "could not apply the caveat-softening mutation"
else
    out="$(run_gate)"; rc=$?
    if [ "$rc" -eq 0 ]; then
        bad "SOFTENING the compat caveat PASSED the gate (the iter-2584 defect)"
    fi
    if ! printf '%s' "$out" | grep -q 'MISSING-MARKER'; then
        bad "the softened caveat failed without naming MISSING-MARKER"
    fi
fi
restore_all

# ── 6. and the restored copy is green again ───────────────────────
if ! run_gate >/dev/null; then
    bad "the scratch mirror is red after restoring -- the mutations leaked"
fi

if [ "$fail" -eq 0 ]; then
    note "gates_present_test: ok (green tree; marker/field/caveat mutations all caught)"
fi
exit "$fail"
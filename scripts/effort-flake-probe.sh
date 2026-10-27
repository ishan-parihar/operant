#!/bin/bash
# Flake probe (2026-10-27): the turn-1 session saw one early `/effort high`
# sim assertion failure. REPRODUCED 5/5 — but the root cause was the probe's
# own assertion syntax, not a product flake: `--assert 'effort == high'`
# names a debug_snapshot key that has never existed (the snapshot exposes
# model/provider, not effort), so every run failed with "Unknown assertion
# path". The correct oracle for the effort picker is a SCREEN assertion
# (the effort-picker corpus scenario asserts `contains:reasoning · effort`).
# Kept as the record of the turn-1 "unexplained failure": explained, my
# syntax error, no product defect, no scenario change needed.
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
FIX="$JCODE_SCRATCH_DIR/atfix"
N="${1:-3}"
fails=0
for i in $(seq 1 "$N"); do
    out=$(env HOME="$FIX/home" \
        HERMES_HOME="$FIX/home/.operant" \
        OPERANT_COLOR_DEPTH=truecolor TERM=xterm-256color TZ=UTC \
        LANG=C.UTF-8 LC_ALL=C.UTF-8 \
        ANTHROPIC_API_KEY=offline-capture \
        OPERANT_DISABLED_ANIMATIONS=signal,prompt_entry \
        OPERANT_PERF_TIER=full \
        PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
        "$REPO/target/debug/operant" -c "$FIX/operant.toml" \
        tui debug simulate --keys '/effort high<enter>' --size 120x40 \
        --assert-screen "contains:reasoning" 2>&1)
    if echo "$out" | grep -q "Simulation succeeded"; then
        echo "run $i: PASS"
    else
        fails=$((fails+1))
        echo "run $i: FAIL"
        echo "$out" | tail -3
    fi
done
echo "failures: $fails/$N"

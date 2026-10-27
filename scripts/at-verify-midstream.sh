#!/bin/bash
# Live verification (2026-10-27): the @ file-ref popup must render
# MID-STREAM. Replicates tui_scenarios/_run.py's pinned env + fixture shape
# at fixed literal paths. Exit status: always 0 unless the sim itself fails;
# the human/agent reads the two counts at the end (popup_frames should be >0,
# streaming_popup_frames should equal popup_frames — every popup frame is
# painted during the live turn under --frame-per-event).
set -u
REPO="$(cd "$(dirname "$0")/.." && pwd)"
CAP="$JCODE_SCRATCH_DIR/atcap"
rm -rf "$CAP"
mkdir -p "$CAP"
FIX="$JCODE_SCRATCH_DIR/atfix"
mkdir -p "$FIX/home/.operant"

env HOME="$FIX/home" \
    HERMES_HOME="$FIX/home/.operant" \
    OPERANT_COLOR_DEPTH=truecolor TERM=xterm-256color TZ=UTC \
    LANG=C.UTF-8 LC_ALL=C.UTF-8 \
    ANTHROPIC_API_KEY=offline-capture \
    OPERANT_DISABLED_ANIMATIONS=signal,prompt_entry \
    OPERANT_PERF_TIER=full \
    PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    "$REPO/target/debug/operant" -c "$FIX/operant.toml" \
    tui debug simulate --keys '@cr' --size 120x40 \
    --agent-script /tmp/at-mid-stream.json \
    --frame-per-event --capture-dir "$CAP" 2>&1 | tail -4

echo "=== captured frames: $(ls "$CAP" | wc -l) ==="
# Find frames painted while the turn still streams AND the popup is visible.
popup_frames=0
streaming_popup_frames=0
for f in "$CAP"/frame-*.txt; do
    [ -f "$f" ] || continue
    if grep -q "crates/" "$f" && grep -q "@cr" "$f"; then
        popup_frames=$((popup_frames+1))
        # The spinner/streaming marker means the turn is still live.
        if grep -qE "Thinking|Working|Streaming| ESC " "$f"; then
            streaming_popup_frames=$((streaming_popup_frames+1))
        fi
    fi
done
echo "frames with @cr + directory rows: $popup_frames"
echo "of which show a live-stream marker: $streaming_popup_frames"

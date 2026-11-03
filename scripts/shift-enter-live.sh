#!/usr/bin/env bash
#
# shift-enter-live.sh — does the REAL TUI turn the kitty CSI-u Shift+Enter
# encoding into a newline rather than a submit?
#
# ## Why this exists (the gap the sim cannot cover)
#
# `operant tui debug simulate` proves the HANDLER maps Enter+SHIFT to
# `insert_newline` (key_handling.rs:1993) and that cmd_tui_debug can spell the
# chord. It does that by feeding the app a synthetic, already-decoded
# `KeyEvent`. Nothing in it exercises the decode step, which is exactly the
# step that failed for the operator: a terminal that was never asked for the
# kitty keyboard protocol sends a bare `\r` for EVERY Enter chord, and key
# handling cannot tell a newline request from a submit.
#
# keyboard_enhancement.rs requests the protocol (PushKeyboardEnhancementFlags
# with DISAMBIGUATE|REPORT_EVENT_TYPES|REPORT_ALTERNATE_KEYS at TUI startup),
# and a unit test pins the exact push/pop bytes. What no test can pin is the
# decode: that raw kitty bytes on a terminal input stream become
# Enter+SHIFT and not `\r`.
#
# ## Method
#
# Drive the REAL `operant chat` binary through a REAL pty (python's pty.fork,
# so the full raw-mode + crossterm decode path runs) and write the genuine
# kitty CSI-u encoding for Shift+Enter at it: `CSI 13 ; 2 u`.
#
#   - Decoded as Enter+SHIFT  -> the composer gains a newline, nothing submits.
#   - Decoded as plain Enter  -> the app SUBMITS "hello" and the turn runs.
#
# The two outcomes differ on screen, which is what makes this a real test.
# `--enter-too` sends a bare `\r` afterwards so the control case is visible in
# the same capture.
#
# ## What this does and does not prove
#
# A raw pty does not interpret kitty sequences, so this asserts the app-side
# decode of the bytes a compliant terminal emits when the push is honoured.
# It cannot prove any given terminal AGREED to the push: the protocol has no
# client-visible acknowledgement, and a multiplexer (tmux without
# extended-keys) or an old terminal may strip it. That residual is why
# terminal_setup.rs::advice and the Shift+Enter help text name Ctrl+J and
# Alt+Enter as configuration-free fallbacks.
#
# Usage: scripts/shift-enter-live.sh [--bin PATH] [--keep]
#   --bin PATH   operant binary to drive (default: ~/.local/bin/operant)
#   --keep       keep the workdir for debugging
#
# Exit: 0 = Shift+Enter produced a newline (no submit)
#       1 = Shift+Enter submitted, or the probe was inconclusive
#       2 = setup failure (no binary / TUI never painted)

set -uo pipefail

BIN="${HOME}/.local/bin/operant"
KEEP=false
WIDTH=120
HEIGHT=40
while [ $# -gt 0 ]; do
    case "$1" in
        --bin) BIN="$2"; shift 2 ;;
        --keep) KEEP=true; shift ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

[ -x "$BIN" ] || { echo "SETUP FAIL: no binary at $BIN" >&2; exit 2; }

HERE="$(mktemp -d /var/tmp/shift-enter-live-XXXX)"
trap '[ "$KEEP" = true ] || rm -rf "$HERE"' EXIT

echo "binary : $BIN ($("$BIN" --version 2>/dev/null))"

python3 - "$BIN" "$HERE" "$WIDTH" "$HEIGHT" <<'PYEOF'
import os, pty, select, sys, time

binary, home, width, height = sys.argv[1:5]
cfg = os.path.join(home, "operant.toml")
home_dir = os.path.join(home, "home")

# A pinned throwaway HOME + a pinned config with an UNREACHABLE base_url, the
# same Fixture shape the scenario corpus uses (tui_scenarios/_run.py). The TUI
# must paint its idle composer without any provider being reachable.
os.makedirs(os.path.join(home_dir, ".operant"), exist_ok=True)
with open(cfg, "w") as fh:
    fh.write('[client]\nbase_url = "http://127.0.0.1:9/v1"\n\n'
             '[agent]\nmodel = "offline-capture"\n')

env = dict(os.environ)
env["HOME"] = home_dir
env["HERMES_HOME"] = os.path.join(home_dir, ".operant")
# Name a kitty-protocol terminal: the app's own detection (terminal_setup.rs,
# perf/theme probes) and the byte encoding under test then agree. A raw pty
# passes bytes through verbatim, so the app sees exactly what we write.
env["TERM"] = "xterm-kitty"
env.pop("NO_COLOR", None)

pid, master = pty.fork()
if pid == 0:
    os.environ.clear()
    os.environ.update(env)
    os.chdir(home_dir)
    # The real binary and the real `chat` entry point: raw mode, the kitty
    # push at startup, and crossterm's decode of whatever arrives.
    os.execv(binary, [binary, "-c", cfg, "chat"])

# pty.fork() leaves the window at 0x0, and ratatui cannot lay out a composer
# row into a zero-height terminal -- the TUI paints cursor/colour escapes and
# nothing else, which reads as "the TUI never started". Set a real winsize
# BEFORE waiting for the first frame (TIOCSWINSZ is 0x5414 on Linux).
import fcntl, struct, termios
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", int(height), int(width), 0, 0))

def drain(seconds):
    end, chunks = time.time() + seconds, []
    while time.time() < end:
        r, _, _ = select.select([master], [], [], 0.15)
        if not r:
            continue
        try:
            data = os.read(master, 1 << 16)
        except OSError:
            break
        if not data:
            break
        chunks.append(data)
    return b"".join(chunks)

def send(blob):
    os.write(master, blob)

# --- Phase 1: the TUI must paint its idle composer -------------------------
buf = b""
for _ in range(120):
    buf += drain(0.25)
    if b"Ask operant" in buf:
        break
if b"Ask operant" not in buf:
    with open(os.path.join(home, "pty-capture.txt"), "wb") as fh:
        fh.write(buf)
    print("SETUP FAIL: the idle composer never painted")
    print("tail:", buf[-400:])
    os.close(master)
    sys.exit(2)
print("phase 1 : idle composer painted")

# --- Phase 2: type a word, then the kitty CSI-u Shift+Enter ---------------
send(b"hello")
time.sleep(0.3)
before = drain(0.5)

# CSI 13 ; 2 u : keycode 13 (Enter) with the shift modifier (mask bit 2).
send(b"\x1b[13;2u")
time.sleep(0.5)
after_chord = drain(0.6)

# A second word: on a newline it lands on the composer's second line.
send(b"world")
time.sleep(0.5)
after_word = drain(0.8)

# --- Phase 3: control case -- a BARE \r, which must submit ----------------
send(b"\r")
time.sleep(1.2)
after_plain = drain(2.5)

# Quit cleanly so the child does not linger.
send(b"/quit\r")
time.sleep(1.0)
drain(1.5)
try:
    os.close(master)
except OSError:
    pass

capture = before + after_chord + after_word + after_plain
with open(os.path.join(home, "pty-capture.txt"), "wb") as fh:
    fh.write(capture)
print(f"capture : {home}/pty-capture.txt ({len(capture)} bytes)")

# --- Verdict --------------------------------------------------------------
#
# The discriminator is CONCATENATION, not adjacency -- and the reason is a
# false positive I actually hit. My first two drafts asserted "'hello' and
# 'world' appear on adjacent rows". Mutation-proved WRONG: with the chord
# replaced by a bare `\r` (which must submit), the two words STILL landed on
# adjacent rows, because the submit moved 'hello' into a transcript row while
# 'world' went into the freshly emptied composer below it. The probe passed on
# a mutant it must have failed.
#
# What actually separates them is whether the two words end up CONCATENATED on
# one painted row. Insert a real newline and the composer holds 'hello' on one
# line and 'world' on the next. Let the chord submit and the cursor never
# advances a line, so the FOLLOW-ON text keeps landing on the same row and the
# capture shows 'helloworld' (or '1> helloworld') glued together.
import re

text = capture.decode("utf-8", "replace")
# Rebuild a row table from the CUP (ESC [ row ; col H) anchors, so text keeps
# the row it was painted at. Anchoring BEFORE stripping keeps the run intact.
m = re.sub(r"\x1b\][^\x07]*\x07", "", text)
m = re.sub(
    r"\x1b\[(\d*);?(\d*)H",
    lambda x: f"\n<@{x.group(1) or 1},{x.group(2) or 1}>",
    m,
)
m = re.sub(r"\x1b\[[0-9;?<>]*[a-zA-Z]", "", m)
m = re.sub(r"\x1b[()][A-B0]|\x1b[=><]", "", m)

rows = {}
for line in m.split("\n"):
    hit = re.match(r"<@(\d+),(\d+)>(.*)$", line.strip())
    if not hit:
        continue
    body = re.sub(r"<@\d+,\d+>", "", hit.group(3)).rstrip()
    if body:
        rows.setdefault(int(hit.group(1)), set()).add(body)

concatenated = [
    (r, t) for r in rows for t in rows[r] if "helloworld" in t.lower()
]
separate = [
    (r, t)
    for r in rows
    for t in rows[r]
    if t.strip().lower() in ("hello", "world")
]
print(f"rows with 'hello'/'world' glued together : {len(concatenated)}")
for r, t in concatenated[:4]:
    print(f"    row {r}: [{t.strip()[:90]}]")
print(f"rows holding a bare 'hello' or 'world'  : {len(separate)}")
for r, t in separate[:4]:
    print(f"    row {r}: [{t.strip()[:90]}]")

newline = bool(separate) and not concatenated
print("=== verdict ===")
print(f"Shift+Enter inserted a newline : {newline}")
sys.exit(0 if newline else 1)
PYEOF
rc=$?

echo
echo "probe rc=$rc"
[ "$KEEP" = true ] && echo "kept workdir: $HERE"
exit $rc

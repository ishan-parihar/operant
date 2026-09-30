#!/usr/bin/env bash
#
# tui-capture.sh — capture the REAL operant TUI through a real pty.
#
# `operant tui debug simulate` drives `App::run` against ratatui's TestBackend,
# which deliberately bypasses the interactive-only paths: raw mode,
# alt-screen, mouse capture, real terminal-mode reassertion on
# `Event::FocusGained`, OSC-8 hyperlinks, and pinned terminal images. This
# script covers those by running the actual `operant chat` binary inside a
# detached tmux pane and capturing the pane with ANSI escapes intact
# (`capture-pane -e`), so a style comparison is possible.
#
# Usage: see --help. Typical CI use:
#
#   scripts/tui-capture.sh --out baseline/chat.txt --wait-text "Ask anything"
#
# Exit codes: 0 captured + all assertions held, 1 capture/assert failure,
# 2 usage error.
set -uo pipefail

# ── Defaults ───────────────────────────────────────────────────────────────
BIN=""                       # resolved later: PATH, then target/{release,debug}
OUT="tui-capture.txt"        # output file (ANSI escapes preserved)
WIDTH=120
HEIGHT=40
TIMEOUT=30                  # seconds to wait for the wait condition
WAIT_TEXT=""                # poll until this substring appears in the pane
KEYS=""                     # literal text to type once the wait condition holds
ENTER=false                 # send Enter after --keys
ASSERT_CONTAINS=""          # gate: fail unless the capture contains this text
SESSION="operant-tui-capture-$$"
CONFIG_MODE=pin             # pin | inherit  (pin = ignore the user's ~/.operant)
KEEP_HOME=false             # keep the pinned temp HOME for debugging

usage() {
  cat <<'EOF'
Usage: scripts/tui-capture.sh [options]

Capture the real `operant chat` TUI inside a detached tmux session and write
the pane contents (with ANSI escapes preserved) to a file.

Options:
  --out PATH              Output file for the capture (default: tui-capture.txt).
                          Written with `tmux capture-pane -e`, so SGR colour
                          escapes are preserved — diffing two captures is a
                          style comparison, not just a text comparison.
  --size WxH              Pane size (default: 120x40). Pinned so wrapping and
                          layout are reproducible.
  --keys TEXT             Type TEXT literally into the TUI after the wait
                          condition holds (via `send-keys -l`). Use with
                          --enter to submit it. Special keys are NOT
                          interpreted; pass e.g. --keys '/help' --enter.
  --enter                 Send Enter after --keys.
  --wait-text TEXT        Poll the pane until TEXT appears before capturing
                          (default: wait for the pane to stop changing, with
                          a quiescence check — no blind sleep).
  --timeout SECONDS       Give up waiting after SECONDS (default: 30).
                          Capture is still attempted; --assert-contains will
                          then report what was actually on screen.
  --assert-contains TEXT  Gate: exit 1 unless TEXT appears in the capture.
                          Repeatable. ANSI escapes are stripped before
                          matching so colour never breaks the assertion.
  --bin PATH              operant binary to run (default: `operant` on PATH,
                          else target/release/operant, else target/debug/operant).
  --args "ARGS"           Extra args passed to the binary (default: "chat").
  --config-mode MODE      `pin` (default) ignores the developer's ~/.operant by
                          pointing HOME at a temp dir with a fixed config and a
                          placeholder API key, so a local theme or keybinding
                          set cannot leak into a committed capture. `inherit`
                          uses the real HOME. Note: a pinned capture points at
                          an unroutable endpoint, so it exercises rendering but
                          never a real agent turn.
  --keep-home             Do not delete the pinned temp HOME on exit.
  -h, --help              Show this help and exit.

Determinism guarantees:
  * tmux runs with `-f /dev/null` (your ~/.tmux.conf cannot alter the pane).
  * Pane size is pinned via new-session -x/-y.
  * HOME is pinned to a temp dir with a fixed minimal config.toml, and
    --config points at it explicitly.
  * TERM/COLORTERM are pinned; the waiting loop polls until the pane content
    stabilises instead of sleeping a fixed amount.
  * NOT pinned: the working directory. The TUI renders a git-branch chip and
    other CWD-derived text, so run from a fixed path if you intend to commit
    the capture as a baseline.

Cleanup: the tmux session is killed on exit, including on error and on
SIGINT/SIGTERM (trap). The pinned temp HOME is removed unless --keep-home.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out)            OUT="${2:?--out needs a value}"; shift 2 ;;
    --size)           IFS=x read -r WIDTH HEIGHT <<<"${2:?--size needs WxH}"; shift 2 ;;
    --keys)           KEYS="${2:?--keys needs a value}"; shift 2 ;;
    --enter)          ENTER=true; shift ;;
    --wait-text)      WAIT_TEXT="${2:?--wait-text needs a value}"; shift 2 ;;
    --timeout)        TIMEOUT="${2:?--timeout needs a value}"; shift 2 ;;
    --assert-contains) ASSERT_CONTAINS="${ASSERT_CONTAINS}"$'\n'"${2:?--assert-contains needs a value}"; shift 2 ;;
    --bin)            BIN="${2:?--bin needs a value}"; shift 2 ;;
    --args)           OPERANT_ARGS="${2:?--args needs a value}"; shift 2 ;;
    --config-mode)    CONFIG_MODE="${2:?--config-mode needs pin|inherit}"; shift 2 ;;
    --keep-home)      KEEP_HOME=true; shift ;;
    -h|--help)        usage; exit 0 ;;
    *)                echo "tui-capture: unknown option '$1' (try --help)" >&2; exit 2 ;;
  esac
done

command -v tmux >/dev/null 2>&1 || { echo "tui-capture: tmux not found on PATH" >&2; exit 1; }

# ── Resolve the binary ─────────────────────────────────────────────────────
if [ -z "$BIN" ]; then
  for candidate in \
      "$(command -v operant 2>/dev/null || true)" \
      "$(dirname "$0")/../target/release/operant" \
      "$(dirname "$0")/../target/debug/operant"; do
    if [ -n "$candidate" ] && [ -x "$candidate" ]; then BIN="$candidate"; break; fi
  done
fi
[ -n "$BIN" ] && [ -x "$BIN" ] || { echo "tui-capture: no operant binary found; pass --bin PATH" >&2; exit 1; }
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
OPERANT_ARGS="${OPERANT_ARGS:-chat}"

# ── Pin the environment so a developer's local config cannot leak ─────────
PIN_HOME=""
cleanup() {
  tmux -f /dev/null kill-session -t "$SESSION" >/dev/null 2>&1 || true
  if [ -n "$PIN_HOME" ] && [ "$KEEP_HOME" != true ]; then
    rm -rf "$PIN_HOME"
  elif [ -n "$PIN_HOME" ]; then
    echo "tui-capture: kept pinned HOME at $PIN_HOME" >&2
  fi
}
trap cleanup EXIT INT TERM

CONFIG_ARGS=()
CHILD_ENV=()
if [ "$CONFIG_MODE" = "pin" ]; then
  PIN_HOME="$(mktemp -d "${TMPDIR:-/tmp}/operant-tui-capture.XXXXXX")"
  # Fixed, minimal config: an unroutable endpoint and an explicit theme, so
  # nothing depends on what the developer has in ~/.operant.
  cat >"$PIN_HOME/operant.toml" <<'TOML'
# Pinned by scripts/tui-capture.sh for deterministic TUI captures.
[client]
base_url = "http://127.0.0.1:9/v1"

[agent]
model = "offline-capture"

[tui]
theme = "dark"
TOML
  mkdir -p "$PIN_HOME/.operant"
  cp "$PIN_HOME/operant.toml" "$PIN_HOME/.operant/operant.toml"
  CONFIG_ARGS=(--config "$PIN_HOME/operant.toml")
  # A placeholder key makes `has_credentials` true, which skips the first-run
  # "Connect a provider" onboarding dialog so the capture reaches the real
  # chat surface. The base_url above is unroutable, so a stray prompt fails
  # fast and deterministically instead of reaching a provider.
  CHILD_ENV=(ANTHROPIC_API_KEY=offline-capture-placeholder)
fi

export TERM="${TERM_OVERRIDE:-xterm-256color}"
export COLORTERM=truecolor
unset OPERANT_TUI_DEBUG OPERANT_TUI_EVENT_LOG NO_COLOR

# ── Start the session ─────────────────────────────────────────────────────
tui() { tmux -f /dev/null "$@"; }

tui new-session -d -s "$SESSION" -x "$WIDTH" -y "$HEIGHT" \
  "HOME='$PIN_HOME' ${CHILD_ENV[*]-} $BIN ${CONFIG_ARGS[*]-} $OPERANT_ARGS; printf '\n[tui-capture] process exited\n'; sleep 600"

if ! tui has-session -t "$SESSION" 2>/dev/null; then
  echo "tui-capture: failed to start tmux session '$SESSION'" >&2
  exit 1
fi

# ── Wait deterministically ────────────────────────────────────────────────
# Two strategies, never a blind sleep:
#   --wait-text : poll until the substring appears in the pane
#   default     : poll until two consecutive pane dumps are identical and
#                 non-empty (the TUI has finished painting / gone quiet)
wait_done=0
prev=""
deadline=$(( $(date +%s) + TIMEOUT ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  pane="$(tui capture-pane -p -t "$SESSION" 2>/dev/null || true)"
  if [ -n "$WAIT_TEXT" ]; then
    if printf '%s' "$pane" | grep -qF -- "$WAIT_TEXT"; then wait_done=1; break; fi
  else
    if [ -n "$pane" ] && [ "$pane" = "$prev" ]; then wait_done=1; break; fi
  fi
  prev="$pane"
  sleep 0.25
done

if [ "$wait_done" -eq 0 ]; then
  if [ -n "$WAIT_TEXT" ]; then
    echo "tui-capture: timed out after ${TIMEOUT}s waiting for '$WAIT_TEXT'" >&2
  else
    echo "tui-capture: pane never stabilised within ${TIMEOUT}s; capturing anyway" >&2
  fi
fi

# ── Optional interaction ─────────────────────────────────────────────────
if [ -n "$KEYS" ]; then
  tui send-keys -t "$SESSION" -l -- "$KEYS"
  [ "$ENTER" = true ] && tui send-keys -t "$SESSION" Enter
  # Give the TUI a moment to paint the result of the key(s) — same
  # stabilisation poll, so this is still not a blind sleep.
  prev=""
  deadline=$(( $(date +%s) + TIMEOUT ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    pane="$(tui capture-pane -p -t "$SESSION" 2>/dev/null || true)"
    [ -n "$pane" ] && [ "$pane" = "$prev" ] && break
    prev="$pane"
    sleep 0.25
  done
fi

# ── Capture (ANSI escapes preserved) ─────────────────────────────────────
mkdir -p "$(dirname "$OUT")" 2>/dev/null || true
if ! tui capture-pane -p -e -t "$SESSION" >"$OUT"; then
  echo "tui-capture: capture-pane failed for session '$SESSION'" >&2
  exit 1
fi
echo "tui-capture: wrote ${WIDTH}x${HEIGHT} capture (ANSI preserved) to $OUT"

# ── Gate ──────────────────────────────────────────────────────────────────
if [ -n "$ASSERT_CONTAINS" ]; then
  # Strip SGR/OSC sequences so a colour change never breaks a content gate.
  plain="$(sed -e 's/\x1b\[[0-9;?]*[A-Za-z]//g' -e 's/\x1b\][^\x07\x1b]*\(\x07\|\x1b\\\)//g' "$OUT")"
  rc=0
  while IFS= read -r needle; do
    [ -z "$needle" ] && continue
    if printf '%s' "$plain" | grep -qF -- "$needle"; then
      echo "tui-capture: ✓ contains '$needle'"
    else
      echo "tui-capture: ✗ expected '$needle' in $OUT" >&2
      rc=1
    fi
  done <<<"$ASSERT_CONTAINS"
  [ "$rc" -eq 0 ] || exit 1
fi

exit 0

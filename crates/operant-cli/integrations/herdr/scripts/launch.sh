#!/bin/sh
# Launch Operant in a Herdr pane.
#
# Herdr opens a real terminal PTY for a plugin pane, so Operant's TUI gets a
# TTY (an action or a CLI call has none and cannot run an interactive agent).
#
# Operant inherits HERDR_ENV, HERDR_PANE_ID and HERDR_SOCKET_PATH from the
# pane, which is what lets it report its own lifecycle state to Herdr. A
# process started any other way — a bare `operant` in a terminal, a cron job,
# the gateway daemon — reports nothing, and Herdr treats the pane as a plain
# terminal. That is the correct behaviour: only an agent running IN the pane
# can say what the pane is doing.

set -eu

if ! command -v operant >/dev/null 2>&1; then
  echo "operant is not on PATH — install it, then run this again." >&2
  exit 1
fi

exec operant "$@"

#!/bin/sh
# Set up the Operant -> Herdr integration.
#
# There is nothing to copy into the agent: Operant reports its own state to
# Herdr over the API socket, so the integration is live the moment Operant
# runs inside a Herdr pane. This action verifies that and says what is
# missing, instead of pretending there is an install step.
#
# `operant doctor --fix` performs the same verification, so the two never
# disagree about what "integrated" means.

set -eu

herdr_bin="${HERDR_BIN_PATH:-herdr}"

echo "Operant <-> Herdr integration"
echo

if ! command -v operant >/dev/null 2>&1; then
  echo "  ✗ operant is not on PATH — Herdr cannot run it." >&2
  exit 1
fi
echo "  ✓ operant: $(command -v operant)"

if ! command -v "$herdr_bin" >/dev/null 2>&1 && [ ! -x "$herdr_bin" ]; then
  echo "  ✗ herdr is not on PATH — install Herdr, then run this again." >&2
  exit 1
fi
echo "  ✓ herdr: $herdr_bin"

if [ "${HERDR_ENV:-}" != "1" ]; then
  # Not an error: the setup action is usually invoked from the Herdr UI,
  # where the shell that runs it is not itself a managed pane.
  echo "  → not running inside a Herdr pane (expected for a setup action)."
else
  echo "  ✓ inside a Herdr pane: ${HERDR_PANE_ID:-unknown}"
fi

echo
echo "Operant reports working / idle / blocked to Herdr by itself."
echo "Start one from this plugin's pane entrypoint and check it with:"
echo "  herdr agent list"

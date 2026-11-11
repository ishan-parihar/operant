#!/bin/sh
# Check the Operant -> Herdr integration.
#
# The question this answers is the one the operator actually asked: "can
# Herdr see my Operant sessions?" The answer is only yes while an Operant
# process is alive in a Herdr pane, so the check reports what it can see
# rather than a configuration summary.

set -eu

herdr_bin="${HERDR_BIN_PATH:-herdr}"

if ! command -v "$herdr_bin" >/dev/null 2>&1 && [ ! -x "$herdr_bin" ]; then
  echo "  ✗ herdr is not on PATH." >&2
  exit 1
fi

if ! command -v operant >/dev/null 2>&1; then
  echo "  ✗ operant is not on PATH — Herdr cannot run it." >&2
  exit 1
fi
echo "  ✓ operant: $(command -v operant)"

if [ "${HERDR_ENV:-}" = "1" ]; then
  echo "  ✓ running inside a Herdr pane: ${HERDR_PANE_ID:-unknown}"
else
  echo "  → this process is not inside a Herdr pane (only panes run Operant report)."
fi

# The live check: does Herdr list an operant agent right now?
agents=$("$herdr_bin" agent list 2>/dev/null || true)
if printf '%s' "$agents" | grep -q '"agent":"operant"'; then
  echo "  ✓ Herdr sees an operant agent:"
  printf '%s' "$agents" \
    | tr ',' '\n' \
    | grep -E '"agent":"operant"|"agent_status"|"pane_id"' \
    | sed 's/^/      /'
else
  echo "  → Herdr lists no operant agent right now."
  echo "    That is expected until an Operant session is running in a Herdr pane."
fi

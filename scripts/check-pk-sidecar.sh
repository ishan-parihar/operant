#!/usr/bin/env bash
# Phase 0 gate: pk-sidecar unit tests + import smoke against the vendored rlm.
set -euo pipefail
cd "$(dirname "$0")/../pk-sidecar"
uv run pytest -q "$@"

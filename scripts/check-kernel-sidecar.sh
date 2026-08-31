#!/usr/bin/env bash
# Phase 0 gate: kernel-sidecar unit tests + import smoke against the vendored rlm.
set -euo pipefail
cd "$(dirname "$0")/../kernel-sidecar"
uv run pytest -q "$@"

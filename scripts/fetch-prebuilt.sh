#!/usr/bin/env bash
# fetch-prebuilt.sh — Provision prebuilt binaries for the browser toolchain.
#
# Per user directive: use the latest prebuilt binaries for igs-rust and the
# obscura browser (stealth build) at each build, rather than building from
# source. This script is a thin wrapper around install-browser-deps.sh, which
# downloads the latest releases from GitHub and installs `igs` + `obscura` as
# global executables (idempotent, reuses the IGS-managed obscura copy so
# browser and IGS web tools share one binary).
#
# Called from:
#   - Manually:  ./scripts/fetch-prebuilt.sh
#   - bootstrap: as part of workspace setup
#   - CI:        before cargo build
#
# Configuration env vars (all optional — passed through):
#   IGS_REPO, OBSCURA_REPO, IGS_TAG, OBSCURA_TAG, GLOBAL_BIN_DIR, NO_SUDO
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
echo "[fetch-prebuilt] delegating to install-browser-deps.sh"
bash "$SCRIPT_DIR/install-browser-deps.sh"
echo "[fetch-prebuilt] done."

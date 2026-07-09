#!/usr/bin/env bash
# build-operant.sh — One-shot build script for the operant project.
# Pulls latest code, sets up environment, builds release binary, and installs.
#
# Usage:
#   ./scripts/build-operant.sh              # Full build
#   ./scripts/build-operant.sh --scope      # Scoped build (operant-cli only, faster)
#   ./scripts/build-operant.sh --check      # Check only (no binary produced)
#   ./scripts/build-operant.sh --install    # Build and install to ~/.cargo/bin

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
MODE="full"
DO_INSTALL=false

# Parse args
for arg in "$@"; do
    case "$arg" in
        --scope)   MODE="scope" ;;
        --check)   MODE="check" ;;
        --install) DO_INSTALL=true ;;
        --help|-h)
            echo "Usage: $0 [--scope|--check|--install]"
            echo "  (default)  Full workspace release build"
            echo "  --scope    Build operant-cli only (faster)"
            echo "  --check    cargo check only (no binary)"
            echo "  --install  Also install to ~/.cargo/bin"
            exit 0
            ;;
    esac
done

cd "$PROJECT_DIR"

# ── Step 1: Pull latest ──
echo "=== Step 1: Pull latest code ==="
git pull --ff-only 2>/dev/null || echo "  (pull skipped — not on a tracking branch or already up to date)"

# ── Step 2: Source build environment ──
echo ""
echo "=== Step 2: Source build environment ==="
source "$SCRIPT_DIR/setup-build-env.sh"

# ── Step 3: Build ──
echo ""
echo "=== Step 3: Build ($MODE) ==="
START=$(date +%s)

case "$MODE" in
    check)
        if [ -d "crates/operant-core" ]; then
            cargo check -p operant-core --lib 2>&1
            cargo check -p operant-cli --bin operant 2>&1
        else
            cargo check --workspace 2>&1
        fi
        ;;
    scope)
        echo "Building operant-cli (release)..."
        cargo build --release -p operant-cli --bin operant 2>&1
        ;;
    full)
        echo "Building full workspace (release)..."
        cargo build --release 2>&1
        ;;
esac

END=$(date +%s)
ELAPSED=$((END - START))
echo ""
echo "  Build completed in ${ELAPSED}s"

# ── Step 4: Install (optional) ──
if [ "$DO_INSTALL" = true ]; then
    echo ""
    echo "=== Step 4: Install to ~/.cargo/bin ==="
    cargo install --path crates/operant-cli --locked 2>&1
    echo "  Installed: $(~/.cargo/bin/operant --version 2>/dev/null || echo 'install may have failed')"
fi

# ── Step 5: Report ──
echo ""
echo "=== Build Complete ==="
if [ "$MODE" != "check" ]; then
    BIN="target/release/operant"
    if [ -f "$BIN" ]; then
        SIZE=$(du -h "$BIN" | cut -f1)
        echo "  Binary: $PROJECT_DIR/$BIN ($SIZE)"
    fi
fi
echo "  Version: $(~/.cargo/bin/operant --version 2>/dev/null || cargo run --release -p operant-cli --bin operant -- --version 2>/dev/null || echo 'unknown')"

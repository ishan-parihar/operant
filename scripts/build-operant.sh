#!/usr/bin/env bash
# build-rust-project.sh — Generic one-shot build script for any Rust workspace.
# Pulls latest code, sets up environment, builds, and optionally installs.
#
# Usage:
#   ./scripts/build-rust-project.sh              # Full workspace build
#   ./scripts/build-rust-project.sh --scope pkg   # Build specific package only
#   ./scripts/build-rust-project.sh --check       # Check only (no binary)
#   ./scripts/build-rust-project.sh --install pkg # Build and install a binary
#   ./scripts/build-rust-project.sh --release     # Release build (default)
#   ./scripts/build-rust-project.sh --debug       # Debug build

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
MODE="full"
PROFILE="release"
TARGET_PKG=""
DO_INSTALL=false

# Parse args
while [ $# -gt 0 ]; do
    case "$1" in
        --scope)    MODE="scope"; TARGET_PKG="${2:-}"; shift 2 ;;
        --check)    MODE="check"; shift ;;
        --install)  DO_INSTALL=true; TARGET_PKG="${2:-}"; shift 2 ;;
        --release)  PROFILE="release"; shift ;;
        --debug)    PROFILE="debug"; shift ;;
        --help|-h)
            echo "Usage: $0 [--scope PKG] [--check] [--install PKG] [--release|--debug]"
            echo "  (default)      Full workspace release build"
            echo "  --scope PKG    Build specific package only (faster)"
            echo "  --check        cargo check only (no binary produced)"
            echo "  --install PKG  Install a binary to ~/.cargo/bin"
            echo "  --release      Release build (default)"
            echo "  --debug        Debug build"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

cd "$PROJECT_DIR"

# Auto-detect project name from Cargo.toml
PROJECT_NAME=$(grep -m1 '^name' Cargo.toml 2>/dev/null | sed 's/.*= *"\([^"]*\)"/\1/' || echo "rust-project")
echo "Project: $PROJECT_NAME"

# ── Step 1: Pull latest ──
echo "=== Step 1: Pull latest code ==="
git pull --ff-only 2>/dev/null || echo "  (pull skipped — not on a tracking branch or already up to date)"

# ── Step 2: Source build environment ──
echo ""
echo "=== Step 2: Source build environment ==="
if [ -f "$SCRIPT_DIR/setup-build-env.sh" ]; then
    source "$SCRIPT_DIR/setup-build-env.sh"
else
    echo "  (setup-build-env.sh not found, using system defaults)"
fi

# ── Step 3: Build ──
echo ""
echo "=== Step 3: Build ($MODE, $PROFILE) ==="
START=$(date +%s)

build_check() {
    if [ -n "$TARGET_PKG" ]; then
        cargo check -p "$TARGET_PKG" 2>&1
    elif [ -d "crates" ]; then
        # Workspace with crates — check each crate
        for crate_dir in crates/*/; do
            crate_name=$(basename "$crate_dir")
            echo "  Checking $crate_name..."
            cargo check -p "$crate_name" 2>&1
        done
    else
        cargo check --workspace 2>&1
    fi
}

case "$MODE" in
    check)
        echo "Running cargo check..."
        build_check
        ;;
    scope)
        if [ -z "$TARGET_PKG" ]; then
            echo "ERROR: --scope requires a package name"
            echo "  Available packages:"
            cargo metadata --no-deps --format-version 1 2>/dev/null | grep '"name"' | sed 's/.*"name": "\([^"]*\)".*/  - \1/' || echo "  (could not list packages)"
            exit 1
        fi
        echo "Building $TARGET_PKG ($PROFILE)..."
        cargo build --profile "$PROFILE" -p "$TARGET_PKG" 2>&1
        ;;
    full)
        echo "Building full workspace ($PROFILE)..."
        cargo build --profile "$PROFILE" 2>&1
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
    if [ -n "$TARGET_PKG" ]; then
        cargo install --path "$TARGET_PKG" --locked 2>&1
        INSTALLED_BIN=$(cargo metadata --no-deps --format-version 1 2>/dev/null | grep -A5 "\"name\": \"$TARGET_PKG\"" | grep -o '"bin":\["[^\"]*"\]' | head -1 | sed 's/"bin":\["\([^"]*\)"\]/\1/' || echo "$TARGET_PKG")
    else
        # Try to find the main binary
        if [ -f "Cargo.toml" ]; then
            MAIN_BIN=$(grep -A5 '\[\[bin\]\]' Cargo.toml 2>/dev/null | grep 'name' | head -1 | sed 's/.*name.*= *"\([^"]*\)"/\1/' || echo "$PROJECT_NAME")
            cargo install --path . --locked 2>&1
            INSTALLED_BIN="$MAIN_BIN"
        else
            echo "ERROR: No Cargo.toml found — cannot install"
            exit 1
        fi
    fi
    if command -v "$INSTALLED_BIN" &>/dev/null; then
        echo "  Installed: $($INSTALLED_BIN --version 2>/dev/null || echo 'unknown version')"
    else
        echo "  Binary not found in PATH — check ~/.cargo/bin"
    fi
fi

# ── Step 5: Report ──
echo ""
echo "=== Build Complete ==="
if [ "$MODE" != "check" ]; then
    # Find the built binary
    if [ -n "$TARGET_PKG" ]; then
        BIN="target/$PROFILE/$(basename "$TARGET_PKG")"
    else
        BIN="target/$PROFILE/$PROJECT_NAME"
    fi
    if [ -f "$BIN" ]; then
        SIZE=$(du -h "$BIN" | cut -f1)
        echo "  Binary: $PROJECT_DIR/$BIN ($SIZE)"
    fi
fi
echo "  Rust: $(rustc --version 2>/dev/null || echo 'unknown')"
echo "  Cargo: $(cargo --version 2>/dev/null || echo 'unknown')"

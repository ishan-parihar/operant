#!/usr/bin/env bash
# setup-build-env.sh — Set up the operant build environment on the ishanp build machine.
# Source this before any cargo command: source scripts/setup-build-env.sh
#
# This replaces the hardcoded /home/z/ paths in dev-env.sh with correct paths
# for the ishanp build machine.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== Operant Build Environment ==="
echo "Project: $PROJECT_DIR"

# ── Rust / Cargo ──
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
    echo "  Cargo env: sourced from ~/.cargo/env"
elif command -v cargo &>/dev/null; then
    # Cargo is available system-wide (e.g. via distro package manager)
    echo "  Cargo env: cargo found at $(which cargo) (system install)"
else
    echo "  WARNING: cargo not found — install via https://rustup.rs"
    exit 1
fi

# ── Libclang (needed by bindgen for espeak-rs-sys) ──
# Auto-detect libclang location
LIBCLANG_FOUND=false

# Try common locations first
for dir in /usr/lib /usr/lib/llvm*/lib /usr/lib64 /usr/local/lib; do
    if [ -d "$dir" ] && ls "$dir"/libclang.so* >/dev/null 2>&1; then
        export LIBCLANG_PATH="$dir"
        echo "  LIBCLANG_PATH: $LIBCLANG_PATH"
        LIBCLANG_FOUND=true
        break
    fi
done

# Fallback: use pkg-config or find
if [ "$LIBCLANG_FOUND" = false ]; then
    if command -v pkg-config &>/dev/null; then
        LIBCLANG_PATH=$(pkg-config --variable=libdir libclang 2>/dev/null)
        if [ -n "$LIBCLANG_PATH" ] && [ -d "$LIBCLANG_PATH" ]; then
            export LIBCLANG_PATH
            echo "  LIBCLANG_PATH: $LIBCLANG_PATH (via pkg-config)"
            LIBCLANG_FOUND=true
        fi
    fi
fi

if [ "$LIBCLANG_FOUND" = false ]; then
    LIBCLANG_PATH=$(find /usr -name 'libclang.so*' -exec dirname {} \; 2>/dev/null | head -1)
    if [ -n "$LIBCLANG_PATH" ]; then
        export LIBCLANG_PATH
        echo "  LIBCLANG_PATH: $LIBCLANG_PATH (via find)"
    else
        echo "  WARNING: libclang not found — bindgen may fail"
    fi
fi

# ── Bindgen clang args ──
GCC_INCLUDE="/usr/lib/gcc/x86_64-linux-gnu/$(gcc -dumpversion)/include"
if [ -d "$GCC_INCLUDE" ]; then
    export BINDGEN_EXTRA_CLANG_ARGS="-I${GCC_INCLUDE} -I/usr/include"
else
    # Fallback: try to find any GCC include dir
    GCC_INCLUDE=$(find /usr/lib/gcc -name 'include' -type d 2>/dev/null | head -1)
    if [ -n "$GCC_INCLUDE" ]; then
        export BINDGEN_EXTRA_CLANG_ARGS="-I${GCC_INCLUDE} -I/usr/include"
    else
        export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/include"
    fi
fi
echo "  BINDGEN_EXTRA_CLANG_ARGS: $BINDGEN_EXTRA_CLANG_ARGS"

# ── ONNX Runtime (for kokoro-tts) ──
ORT_DIRS=(
    "/home/ishanp/Documents/GitHub/MY-PROJECTS/local/onnxruntime-linux-x64-1.20.1/lib"
    "/home/ishanp/Documents/GitHub/MY-PROJECTS/local/onnxruntime-linux-x64-1.21.0/lib"
    "/home/ishanp/local/onnxruntime-linux-x64-1.20.1/lib"
)
for d in "${ORT_DIRS[@]}"; do
    if [ -d "$d" ]; then
        export ORT_LIB_LOCATION="$d"
        export ORT_PREFER_DYNAMIC_LINK=1
        echo "  ORT_LIB_LOCATION: $ORT_LIB_LOCATION"
        break
    fi
done
if [ -z "${ORT_LIB_LOCATION:-}" ]; then
    echo "  WARNING: ONNX Runtime not found — kokoro-tts may fail to build"
fi

# ── PKG_CONFIG (ALSA for cpal) ──
PKG_DIRS=(
    "/home/ishanp/Documents/GitHub/MY-PROJECTS/local/pkgconfig"
    "/home/ishanp/local/pkgconfig"
    "/usr/lib/x86_64-linux-gnu/pkgconfig"
)
for d in "${PKG_DIRS[@]}"; do
    if [ -d "$d" ]; then
        export PKG_CONFIG_PATH="${d}:${PKG_CONFIG_PATH:-}"
        echo "  PKG_CONFIG_PATH: $PKG_CONFIG_PATH"
        break
    fi
done

# ── LD_LIBRARY_PATH (runtime libs) ──
LIB_DIRS=(
    "/home/ishanp/Documents/GitHub/MY-PROJECTS/local/lib"
    "/home/ishanp/local/lib"
)
for d in "${LIB_DIRS[@]}"; do
    if [ -d "$d" ]; then
        export LD_LIBRARY_PATH="${d}:${LD_LIBRARY_PATH:-}"
        echo "  LD_LIBRARY_PATH includes: $d"
        break
    fi
done
if [ -n "${ORT_LIB_LOCATION:-}" ]; then
    export LD_LIBRARY_PATH="${ORT_LIB_LOCATION}:${LD_LIBRARY_PATH:-}"
fi

# ── CARGO settings ──
export CARGO_INCREMENTAL=0
echo "  CARGO_INCREMENTAL: $CARGO_INCREMENTAL"

# ── Verify tools ──
echo ""
echo "=== Tool Versions ==="
echo "  rustc:  $(rustc --version 2>/dev/null || echo 'NOT FOUND')"
echo "  cargo:  $(cargo --version 2>/dev/null || echo 'NOT FOUND')"
echo "  cmake:  $(cmake --version 2>/dev/null | head -1 || echo 'NOT FOUND')"
echo "  git:    $(git --version 2>/dev/null || echo 'NOT FOUND')"
echo ""
echo "Build environment ready. Run 'cargo build --release' to compile."

#!/usr/bin/env bash
# Run cargo with the dev-env applied. Usage:
#   ./scripts/check.sh check --workspace
#   ./scripts/check.sh test --workspace --no-run
#   ./scripts/check.sh doc          # ci.yml's exact rustdoc gate
#
# Every path below is overridable by an environment variable and falls back to
# the value that was hardcoded before. Those defaults point at one developer's
# machine (/home/z/...), so on any other box every export resolved to a
# nonexistent directory; scripts/dev-env.sh had already taken this shape
# (LOCAL_DIR with a repo-relative default), and the two files drifting apart is
# why check.sh kept the machine-specific paths after dev-env.sh stopped using
# them. Sourcing a nonexistent ~/.cargo/env under `set -e` was the other way
# this could fail outright.
set -e

# Root of the locally-provisioned dependency tree (libclang, ONNX Runtime, the
# synthetic alsa.pc, the linker libs). Same variable name dev-env.sh uses, so
# `LOCAL_DIR=... ./scripts/check.sh ...` means the same thing to both.
LOCAL_DIR="${LOCAL_DIR:-/home/z/my-project/local}"
ORT_DIR="$LOCAL_DIR/onnxruntime-linux-x64-1.20.1"

# libclang path resolution
if [ -d "$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu" ]; then
    export LIBCLANG_PATH="$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu"
elif [ -f "/usr/lib/libclang.so" ]; then
    export LIBCLANG_PATH=/usr/lib
elif [ -f "/usr/lib/llvm21/lib/libclang.so" ]; then
    export LIBCLANG_PATH=/usr/lib/llvm21/lib
fi

# ORT_LIB_LOCATION path resolution
if [ -d "$ORT_DIR/lib" ]; then
    export ORT_LIB_LOCATION="$ORT_DIR/lib"
fi
export ORT_PREFER_DYNAMIC_LINK=1

# bindgen gcc args
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"

# pkg-config path resolution
if [ -d "$LOCAL_DIR/pkgconfig" ]; then
    export PKG_CONFIG_PATH="$LOCAL_DIR/pkgconfig"
fi

# LD_LIBRARY_PATH path resolution
LD_PATHS=""
if [ -d "$LOCAL_DIR/lib" ]; then
    LD_PATHS="$LOCAL_DIR/lib"
fi
if [ -d "$ORT_DIR/lib" ]; then
    if [ -n "$LD_PATHS" ]; then
        LD_PATHS="$LD_PATHS:$ORT_DIR/lib"
    else
        LD_PATHS="$ORT_DIR/lib"
    fi
fi
if [ -n "$LD_PATHS" ]; then
    export LD_LIBRARY_PATH="$LD_PATHS:$LD_LIBRARY_PATH"
fi

# PATH: a pip-installed cmake lands in a venv, cargo lives in ~/.cargo/bin.
# Both entries are appended only when they exist — a PATH entry pointing at a
# nonexistent directory is noise, and the previous hardcoded pair named a
# different user's home. $HOME is expanded here, not baked in.
for bin_dir in "$HOME/.venv/bin" "$HOME/.cargo/bin"; do
    if [ -d "$bin_dir" ]; then
        PATH="$bin_dir:$PATH"
    fi
done
export PATH
export CARGO_INCREMENTAL=0

# Linker flags
if [ -d "$LOCAL_DIR/lib" ]; then
    export RUSTFLAGS="-L native=$LOCAL_DIR/lib"
fi

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR/.."

# `doc` is a shortcut for the rustdoc gate, not a passthrough: ci.yml:140 runs
# `cargo doc --workspace --no-deps --all-features` under RUSTDOCFLAGS=-Dwarnings,
# and that is exactly the gate the local loop is supposed to stand in for. The
# flags are baked in so the local command and the CI job cannot drift; pass them
# explicitly to override. Any other subcommand is forwarded to cargo unchanged,
# so this stays backward compatible with every existing caller.
if [ "${1:-}" = "doc" ]; then
    shift
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:--Dwarnings}"
    exec cargo doc --workspace --no-deps --all-features "$@"
fi

exec cargo "$@"

#!/usr/bin/env bash
# Run cargo check / test with the dev-env applied. Usage:
#   ./scripts/check.sh check --workspace
#   ./scripts/check.sh test --workspace --no-run
set -e

# libclang path resolution
if [ -d "/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu" ]; then
    export LIBCLANG_PATH=/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu
elif [ -f "/usr/lib/libclang.so" ]; then
    export LIBCLANG_PATH=/usr/lib
elif [ -f "/usr/lib/llvm21/lib/libclang.so" ]; then
    export LIBCLANG_PATH=/usr/lib/llvm21/lib
fi

# ORT_LIB_LOCATION path resolution
if [ -d "/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib" ]; then
    export ORT_LIB_LOCATION=/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
fi
export ORT_PREFER_DYNAMIC_LINK=1

# bindgen gcc args
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"

# pkg-config path resolution
if [ -d "/home/z/my-project/local/pkgconfig" ]; then
    export PKG_CONFIG_PATH=/home/z/my-project/local/pkgconfig
fi

# LD_LIBRARY_PATH path resolution
LD_PATHS=""
if [ -d "/home/z/my-project/local/lib" ]; then
    LD_PATHS="/home/z/my-project/local/lib"
fi
if [ -d "/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib" ]; then
    if [ -n "$LD_PATHS" ]; then
        LD_PATHS="$LD_PATHS:/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib"
    else
        LD_PATHS="/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib"
    fi
fi
if [ -n "$LD_PATHS" ]; then
    export LD_LIBRARY_PATH="$LD_PATHS:$LD_LIBRARY_PATH"
fi

export PATH="/home/z/.venv/bin:/home/z/.cargo/bin:$PATH"
export CARGO_INCREMENTAL=0

# Linker flags
if [ -d "/home/z/my-project/local/lib" ]; then
    export RUSTFLAGS="-L native=/home/z/my-project/local/lib"
fi

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$DIR/.."
exec cargo "$@"

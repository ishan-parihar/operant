#!/usr/bin/env bash
# Run cargo check / test with the dev-env applied. Usage:
#   ./scripts/check.sh check --workspace
#   ./scripts/check.sh test --workspace --no-run
set -e
export LIBCLANG_PATH=/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu
export ORT_LIB_LOCATION=/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export ORT_PREFER_DYNAMIC_LINK=1
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include"
export PKG_CONFIG_PATH=/home/z/my-project/local/pkgconfig
export LD_LIBRARY_PATH=/home/z/my-project/local/lib:/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib
export PATH="/home/z/.venv/bin:/home/z/.cargo/bin:$PATH"
export CARGO_INCREMENTAL=0
# Linker needs to find libsonic.so + libasound.so symlinks (created by provision-build-deps.sh)
export RUSTFLAGS="-L native=/home/z/my-project/local/lib"
cd /home/z/my-project/operant
exec cargo "$@"

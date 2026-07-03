#!/usr/bin/env bash
# Source this file before running cargo commands in this repo.
# Sets up build-time environment for operant on systems without root apt access.
#
# Required because:
#  - libclang (for bindgen via espeak-rs-sys / ort-sys)
#  - ONNX Runtime (for ort-sys via kokoro-tiny)
#  - cmake (for espeak-rs-sys's espeak-ng build)
#  - alsa runtime lib (for cpal via kokoro-tiny's playback feature)
#
# To provision the dependencies, see scripts/provision-build-deps.sh.

set -e

# libclang (extracted from libclang1-19 deb, no root needed)
export LIBCLANG_PATH="${LIBCLANG_PATH:-/home/z/my-project/local/libclang_extract/usr/lib/x86_64-linux-gnu}"

# ONNX Runtime (prebuilt tarball from microsoft/onnxruntime releases)
export ORT_LIB_LOCATION="${ORT_LIB_LOCATION:-/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib}"
export ORT_PREFER_DYNAMIC_LINK="${ORT_PREFER_DYNAMIC_LINK:-1}"

# Bindgen needs GCC's resource headers (stddef.h etc.) on systems without clang resource dir
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include}"

# pkg-config for alsa (we ship a synthetic alsa.pc pointing at the runtime libasound.so.2)
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-/home/z/my-project/local/pkgconfig}"

# Runtime linker path so the built binary can find libonnxruntime + libasound
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/home/z/my-project/local/lib:/home/z/my-project/local/onnxruntime-linux-x64-1.20.1/lib}"

# cmake (pip-installed)
export PATH="/home/z/.venv/bin:$PATH"

# Rust
. "$HOME/.cargo/env"

echo "[dev-env] LIBCLANG_PATH=$LIBCLANG_PATH"
echo "[dev-env] ORT_LIB_LOCATION=$ORT_LIB_LOCATION"
echo "[dev-env] PATH includes $(which cmake cargo rustc | tr '\n' ' ')"

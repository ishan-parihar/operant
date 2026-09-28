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

# Every path below is derived from LOCAL_DIR rather than hardcoded. The
# previous version pointed at /home/z/my-project/local, which is one developer's
# machine: on any other box every export below resolved to a nonexistent
# directory, and the `set -e` plus a missing `~/.cargo/env` meant sourcing this
# file could fail outright. LOCAL_DIR defaults to the repo's own `local/` — the
# same directory crates/operant-core/build.rs searches for libsonic — so
# provisioning and linking agree with no configuration.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_DIR="${LOCAL_DIR:-$(dirname "$SCRIPT_DIR")/local}"
ORT_DIR="$LOCAL_DIR/onnxruntime-linux-x64-1.20.1"

# libclang (extracted from libclang1-19 deb, no root needed)
# Prefer the symlink the provisioner makes for a system libclang: bindgen globs
# for 'libclang.so' and 'libclang-*.so.*', and a distro that ships only
# libclang.so.N.N satisfies neither, so pointing LIBCLANG_PATH straight at
# /usr/lib fails even when libclang is installed.
if [ -z "$LIBCLANG_PATH" ] && [ -e "$LOCAL_DIR/libclang/libclang.so" ]; then
  LIBCLANG_PATH="$LOCAL_DIR/libclang"
fi
export LIBCLANG_PATH="${LIBCLANG_PATH:-$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu}"
if ! ls "$LIBCLANG_PATH"/libclang.so >/dev/null 2>&1 && ! ls "$LIBCLANG_PATH"/libclang-*.so.* >/dev/null 2>&1; then
  echo "[dev-env] WARN: no libclang under LIBCLANG_PATH=$LIBCLANG_PATH" >&2
  echo "[dev-env]       run scripts/provision-build-deps.sh first." >&2
fi

# ONNX Runtime (prebuilt tarball from microsoft/onnxruntime releases)
export ORT_LIB_LOCATION="${ORT_LIB_LOCATION:-$ORT_DIR/lib}"
export ORT_PREFER_DYNAMIC_LINK="${ORT_PREFER_DYNAMIC_LINK:-1}"

# Bindgen needs GCC's resource headers (stddef.h etc.) on systems without clang resource dir
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include}"

# pkg-config for alsa (we ship a synthetic alsa.pc pointing at the runtime libasound.so.2)
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$LOCAL_DIR/pkgconfig}"

# Runtime linker path so the built binary can find libonnxruntime + libasound
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-$LOCAL_DIR/lib:$ORT_DIR/lib}"

# pip-installed cmake lands in a venv. Prepend it only if it is actually there —
# a PATH entry pointing at a nonexistent directory is noise, and the old value
# hardcoded a different user's home.
if [ -d "$HOME/.venv/bin" ]; then
  export PATH="$HOME/.venv/bin:$PATH"
fi

# Rust
if [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi

echo "[dev-env] LOCAL_DIR=$LOCAL_DIR"
echo "[dev-env] LIBCLANG_PATH=$LIBCLANG_PATH"
echo "[dev-env] ORT_LIB_LOCATION=$ORT_LIB_LOCATION"
echo "[dev-env] PATH includes $(which cmake cargo rustc 2>/dev/null | tr '\n' ' ')"

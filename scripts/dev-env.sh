#!/usr/bin/env bash
# Source this file before running cargo commands in this repo.
# Sets up build-time environment for operant on systems without root apt access.
#
# Required because:
#  - libclang (for bindgen via espeak-rs-sys / ort-sys)
#  - cmake (for espeak-rs-sys's espeak-ng build)
#  - alsa runtime lib (for cpal)
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

# libclang. One resolution for every build script, in
# scripts/libclang-resolve.sh. The old inline version fell through to
# "$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu" — a directory that
# exists only after the Debian .deb fallback ran — and only printed a
# warning, so the build died two minutes later inside bindgen with a message
# that named neither the cause nor the fix. It also trusted a dangling
# `local/libclang/libclang.so` symlink, which passes a plain existence test
# and then fails to load.
if ! . "$(dirname "${BASH_SOURCE[0]}")/libclang-resolve.sh"; then
  echo "[dev-env] ERROR: no loadable libclang found." >&2
  echo "[dev-env]        run scripts/provision-build-deps.sh, or set" >&2
  echo "[dev-env]        LIBCLANG_PATH to a directory holding libclang.so." >&2
  exit 1
fi
_resolved_libclang="$(resolve_libclang "$LOCAL_DIR")"
export LIBCLANG_PATH="$_resolved_libclang"

# Bindgen needs GCC's resource headers (stddef.h etc.) on systems without clang resource dir
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include}"

# pkg-config for alsa (we ship a synthetic alsa.pc pointing at the runtime libasound.so.2)
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$LOCAL_DIR/pkgconfig}"

# The linker search path for the repo's own static libraries (libsonic.a, the
# one crates/operant-core/build.rs looks for) and CARGO_INCREMENTAL=0 (this
# host has two CPUs; an incremental rebuild costs more here than a fresh one).
# The environment is ONE source: check.sh sources this file (RT-167).
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-L native=$LOCAL_DIR/lib"
export CARGO_INCREMENTAL=0

# Runtime linker path for libasound (alsa).
#
# ONNX Runtime used to be exported here too (iter-2513 removed it). Kokoro,
# the previous TTS default, linked libonnxruntime.so.1 and made the installed
# binary exit 127 without this file sourced. Phonon's `ptts` runtime is pure
# Rust, so `readelf -d` on the built binary now shows no libonnxruntime NEEDED
# entry and no rpath is required. Keep the alsa path: cpal still dlopens it.
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-$LOCAL_DIR/lib}"

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
echo "[dev-env] PATH includes $(which cmake cargo rustc 2>/dev/null | tr '\n' ' ')"

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
# Dynamic linking of the ONNX runtime is a LOCAL-ONLY choice, and it is the
# reason a locally built operant differs from a released one. Measured on the
# same source, same commit:
#
#   CI release v0.2.0   NEEDED: libstdc++ libgcc_s libm libc ld-linux  (no onnx)
#                       52 MB, statically linked ONNX, self-contained
#   this dev-env        NEEDED: ... plus libonnxruntime.so.1
#                       31 MB, needs a shared library at runtime
#
# Every problem chased in iters 425-430 -- the missing RUNPATH, the ONNX copy in
# install.sh, the provisioning step in build.yml -- comes from this one line. CI
# does not set it, so the published binary has never needed the shared object.
# Set to 0 to build the same self-contained binary that ships.
export ORT_PREFER_DYNAMIC_LINK="${ORT_PREFER_DYNAMIC_LINK:-1}"

# Bindgen needs GCC's resource headers (stddef.h etc.) on systems without clang resource dir
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/14/include -I/usr/include}"

# pkg-config for alsa (we ship a synthetic alsa.pc pointing at the runtime libasound.so.2)
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$LOCAL_DIR/pkgconfig}"

# Runtime linker path so the built binary can find libonnxruntime + libasound
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-$LOCAL_DIR/lib:$ORT_DIR/lib}"

# Embed the ONNX Runtime path in the binary as an rpath. Without it the shipped
# executable carries no record of where its shared library lives: `ldd` reports
# "libonnxruntime.so.1 => not found" and the binary exits 127 unless the shell
# that runs it happens to have sourced this file. An installed binary must not
# depend on the caller's environment — that is the whole difference between a
# build and a deployment. (Static linking is not an option: ort-sys cannot link
# the prebuilt ONNX Runtime that way.)
#
# RUNTIME_LIB_DIR, not ORT_DIR: the build tree may be a disposable worktree or a
# checkout on a tmpfs, and an rpath into one makes every install die the moment
# that directory goes away. The runtime copy is what install.sh provisions into
# the user's library directory, which outlives any checkout.
RUNTIME_LIB_DIR="${RUNTIME_LIB_DIR:-$HOME/.local/lib/operant}"
if [ -d "$RUNTIME_LIB_DIR" ] && [ -d "$ORT_DIR/lib" ]; then
  export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-L native=$ORT_DIR/lib -C link-arg=-Wl,-rpath,$RUNTIME_LIB_DIR"
fi

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

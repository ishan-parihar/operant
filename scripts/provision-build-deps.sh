#!/usr/bin/env bash
# Provision build-time dependencies for operant on a system without root apt access.
# Idempotent: skips downloads that already exist.
#
# After running this, source scripts/dev-env.sh before cargo commands.
set -e

# Default to the repo's own `local/`, which is exactly where
# crates/operant-core/build.rs looks when OPERANT_NATIVE_LIB_DIR is unset. The
# previous default was an absolute path from one developer's machine
# (/home/z/my-project/local), so on any other machine this script provisioned a
# directory that nothing then built from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL_DIR="${LOCAL_DIR:-$(dirname "$SCRIPT_DIR")/local}"
mkdir -p "$LOCAL_DIR"

# ── 1. libclang (for bindgen) ──
# bindgen dlopens libclang and globs for 'libclang.so' / 'libclang-*.so.*'. Many
# distros ship ONLY libclang.so.N.N, which matches NEITHER pattern, so a path
# pointing at such a directory fails even though libclang is installed. Prefer a
# system libclang, linked under a name bindgen will find, before falling back to
# the Debian .deb (which also needs dpkg-deb, absent on non-Debian systems).
LINK_DIR="$LOCAL_DIR/libclang"
mkdir -p "$LINK_DIR"
# Prefer the plain name bindgen globs, then any versioned variant.
SYS_LIBCLANG=""
for cand in /usr/lib/libclang.so /usr/lib/libclang.so.*.* /usr/lib/*/libclang.so.*.*; do
  if [ -e "$cand" ]; then SYS_LIBCLANG="$cand"; break; fi
done
if [ -n "$SYS_LIBCLANG" ]; then
  ln -sf "$SYS_LIBCLANG" "$LINK_DIR/libclang.so"
  echo "[provision] using system libclang: $SYS_LIBCLANG"
elif [ ! -e "$LINK_DIR/libclang.so" ] && [ ! -f "$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu/libclang-19.so.19" ]; then
  echo "[provision] no system libclang; falling back to the Debian .deb..."
  if command -v apt-get >/dev/null 2>&1; then
    echo "[provision] installing libclang via apt-get (needs root)..."
    sudo apt-get update -qq && sudo apt-get install -y -qq libclang-dev
    SYS_LIBCLANG=$(ls /usr/lib/libclang.so.*.* /usr/lib/*/libclang.so.*.* 2>/dev/null | head -1)
    [ -n "$SYS_LIBCLANG" ] && ln -sf "$SYS_LIBCLANG" "$LINK_DIR/libclang.so"
  else
    curl -fsSL "http://ftp.debian.org/debian/pool/main/l/llvm-toolchain-19/libclang1-19_19.1.7-3+b1_amd64.deb" \
      -o "$LOCAL_DIR/libclang.deb"
    mkdir -p "$LOCAL_DIR/libclang_extract"
    if command -v dpkg-deb >/dev/null 2>&1; then
      dpkg-deb -x "$LOCAL_DIR/libclang.deb" "$LOCAL_DIR/libclang_extract/"
    else
      # `ar` ships with binutils and is present everywhere; dpkg-deb is not.
      # Without this the provisioner cannot run on Arch, Fedora, nix or Alpine.
      ( cd "$LOCAL_DIR/libclang_extract" && ar x ../libclang.deb && tar xf data.tar.xz )
    fi
    rm -f "$LOCAL_DIR/libclang.deb"
  fi
fi
if [ -e "$LINK_DIR/libclang.so" ]; then
  echo "[provision] LIBCLANG_PATH=$LINK_DIR"
fi

# ── 2. ONNX Runtime (for ort-sys via kokoro-tiny) ──
ORT_VERSION="1.20.1"
ORT_DIR="$LOCAL_DIR/onnxruntime-linux-x64-$ORT_VERSION"
if [ ! -f "$ORT_DIR/lib/libonnxruntime.so" ]; then
  echo "[provision] downloading ONNX Runtime $ORT_VERSION..."
  curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VERSION/onnxruntime-linux-x64-$ORT_VERSION.tgz" \
    -o "$LOCAL_DIR/ort.tgz"
  tar xzf "$LOCAL_DIR/ort.tgz" -C "$LOCAL_DIR"
  rm "$LOCAL_DIR/ort.tgz"
fi

# ── 3. cmake (for espeak-rs-sys's espeak-ng build) ──
if ! command -v cmake >/dev/null 2>&1; then
  echo "[provision] installing cmake via pip..."
  pip3 install cmake
fi

# ── 4. alsa runtime (for cpal via kokoro-tiny's playback feature) ──
# Create a libasound.so symlink so the linker can find the runtime libasound.so.2
mkdir -p "$LOCAL_DIR/lib"
ALSA_SYSTEM="/usr/lib/x86_64-linux-gnu/libasound.so.2"
if [ -e "$ALSA_SYSTEM" ]; then
  ln -sf "$ALSA_SYSTEM" "$LOCAL_DIR/lib/libasound.so"
else
  echo "[provision] WARNING: $ALSA_SYSTEM not found — cpal audio playback will not link."
  echo "[provision]   Install it (Debian/Ubuntu: libasound2) or disable TTS playback."
fi

# ── 5. sonic runtime (for espeak-ng, linked by operant-core/build.rs) ──
# Deliberately NOT an unconditional `ln -sf`. That command succeeds even when the
# target does not exist, which is how a dangling `local/lib/libsonic.so` came to
# be committed to this repository: it was machine-local provisioning output that
# was never supposed to be tracked, and it points at a path that does not exist
# on a machine without libsonic0 installed.
SONIC_SYSTEM="/usr/lib/x86_64-linux-gnu/libsonic.so.0"
if [ -e "$SONIC_SYSTEM" ]; then
  ln -sf "$SONIC_SYSTEM" "$LOCAL_DIR/lib/libsonic.so"
  echo "[provision] libsonic.so -> $SONIC_SYSTEM"
elif [ -f "$LOCAL_DIR/lib/libsonic.a" ]; then
  echo "[provision] libsonic: using the committed $LOCAL_DIR/lib/libsonic.a (no system libsonic.so.0)"
else
  echo "[provision] WARNING: neither $SONIC_SYSTEM nor local/lib/libsonic.a found."
  echo "[provision]   espeak-ng (TTS) will fail to link. Install libsonic0, or place a libsonic.a in $LOCAL_DIR/lib."
fi

# Synthetic alsa.pc so pkg-config can satisfy alsa-sys without libasound2-dev
mkdir -p "$LOCAL_DIR/pkgconfig"
cat > "$LOCAL_DIR/pkgconfig/alsa.pc" << 'EOF'
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib/x86_64-linux-gnu
includedir=${exec_prefix}/include

Name: alsa
Description: Advanced Linux Sound Architecture Library
Version: 1.2.14
Libs: -L${libdir} -lasound
Libs.private: -lm -lpthread -ldl -lrt
Cflags: -I${includedir}
EOF

echo "[provision] done. Source scripts/dev-env.sh before running cargo."

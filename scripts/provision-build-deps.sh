#!/usr/bin/env bash
# Provision build-time dependencies for operant on a system without root apt access.
# Idempotent: skips downloads that already exist.
#
# After running this, source scripts/dev-env.sh before cargo commands.
set -e

LOCAL_DIR="${LOCAL_DIR:-/home/z/my-project/local}"
mkdir -p "$LOCAL_DIR"

# ── 1. libclang (for bindgen) ──
if [ ! -f "$LOCAL_DIR/libclang_extract/usr/lib/x86_64-linux-gnu/libclang-19.so.19" ]; then
  echo "[provision] downloading libclang1-19 deb..."
  curl -fsSL "http://ftp.debian.org/debian/pool/main/l/llvm-toolchain-19/libclang1-19_19.1.7-3+b1_amd64.deb" \
    -o "$LOCAL_DIR/libclang.deb"
  mkdir -p "$LOCAL_DIR/libclang_extract"
  dpkg-deb -x "$LOCAL_DIR/libclang.deb" "$LOCAL_DIR/libclang_extract/"
  rm "$LOCAL_DIR/libclang.deb"
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
if [ ! -L "$LOCAL_DIR/lib/libasound.so" ]; then
  ln -sf /usr/lib/x86_64-linux-gnu/libasound.so.2 "$LOCAL_DIR/lib/libasound.so"
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

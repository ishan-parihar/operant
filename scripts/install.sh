#!/bin/bash
# Install script for Operant
# Builds the release binary and installs it globally

set -e

cd "$(dirname "$0")"

# ── Native build dependency preflight ──
# No sonic check: operant-core/build.rs emits no `-lsonic` (nothing references
# a sonic symbol; the audio backend is stubbed), so libsonic is neither
# required nor consulted. A stale local/lib/libsonic.a on disk is inert.
#
# ONNX Runtime is also NOT optional, contrary to what this comment used to claim.
# Measured on the shipped binary:
#     readelf -d ~/.local/bin/operant | grep NEEDED
#       libonnxruntime.so.1  <- a hard NEEDED entry, not a soft reference
# so a machine without it cannot start operant at all. Only the *build* can
# sometimes succeed without it (kokoro-tty is optional); the *installed binary*
# always needs it. The rpath added in iter-425 points at ~/.local/lib/operant for
# this reason, and the install step below populates that directory.
echo "=== Checking Native Build Dependencies ==="
for tool in cmake pkg-config; do
    if command -v "$tool" >/dev/null 2>&1; then
        echo "  ok   $tool"
    else
        echo "  warn $tool missing — espeak-rs-sys may fail to build (TTS only)"
    fi
done

echo ""
echo "=== Building Operant Release Binary ==="
cargo build --release -p operant-cli

echo ""
echo "=== Installing Runtime Libraries ==="
# The binary embeds an rpath to this directory (see scripts/dev-env.sh). If the
# library is not copied here, `operant` exits 127 with
# "libonnxruntime.so.1: cannot open shared object file" on every invocation
# except inside a shell that happens to have sourced dev-env.sh. The rpath must
# point somewhere that outlives the build tree, not into a checkout or tmpfs.
RUNTIME_LIB_DIR="${RUNTIME_LIB_DIR:-$HOME/.local/lib/operant}"
ORT_DIR="local/onnxruntime-linux-x64-1.20.1/lib"
if [ -d "$ORT_DIR" ]; then
    mkdir -p "$RUNTIME_LIB_DIR"
    cp -a "$ORT_DIR"/libonnxruntime.so* "$RUNTIME_LIB_DIR"/ 2>/dev/null \
        && echo "  ok   libonnxruntime -> $RUNTIME_LIB_DIR" \
        || echo "  WARN could not copy libonnxruntime; operant may not start"
else
    echo "  WARN no local ONNX Runtime at $ORT_DIR — run scripts/provision-build-deps.sh"
    echo "       before installing, or operant will not start on this machine."
fi

echo ""
echo "=== Installing to /usr/local/bin/ ==="
sudo cp target/release/operant /usr/local/bin/operant
sudo chmod +x /usr/local/bin/operant

# Also drop a copy in ~/.cargo/bin when it exists — PATH usually resolves
# there first, and a stale copy shadows the fresh /usr/local/bin one.
if [ -d "$HOME/.cargo/bin" ]; then
    cp target/release/operant "$HOME/.cargo/bin/operant"
fi

echo ""
echo "=== Seeding Bundled Skills ==="
# Pack the 29-skill pool shipped with the repo into the user skills directory
# (~/.operant/skills) so a fresh install is agent-ready from scratch. Idempotent
# (keeps existing skills); FORCE=1 re-seeds. Best-effort — never abort install.
bash "$(dirname "$0")/install-skills.sh" || echo "WARN: skill seeding failed (non-fatal)"

echo ""
echo "=== Installing Gateway systemd Service ==="
# The unit file's single source of truth lives in the binary itself
# (`operant gateway install`) — no template to drift out of sync. Enabled so
# the gateway auto-starts on login once configured (`operant setup`, then
# `operant gateway start`). Best-effort: skipped where systemd is absent.
if command -v systemctl >/dev/null 2>&1 && systemctl --user show boot.target >/dev/null 2>&1; then
    target/release/operant gateway install --force \
        || echo "WARN: gateway service install failed (non-fatal)"
    systemctl --user daemon-reload 2>/dev/null || true
    systemctl --user enable operant-gateway 2>/dev/null || true
else
    echo "systemd user session not available — skipping service install"
    echo "Start the gateway manually with: operant gateway run"
fi

echo ""
echo "=== Installation Complete ==="
echo ""
echo "You can now run: operant --version"
echo "Or start chatting: operant"
echo ""
echo "To initialize configuration: operant setup"
echo "To start the dashboard: operant dashboard"
echo "To start the gateway: operant gateway start"
echo ""
echo "Browser tooling: sourcehound -> $(command -v sourcehound || echo MISSING)"
echo "Skills: $(find "${HERMES_SKILLS_DIR:-${HERMES_HOME:-$HOME/.operant}/skills}" -maxdepth 2 -name SKILL.md 2>/dev/null | wc -l | tr -d ' ') installed"

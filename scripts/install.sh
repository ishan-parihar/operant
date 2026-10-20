#!/usr/bin/env bash
# Install script for Operant
# Builds the release binary and installs it per-user (~/.local/bin)
# Usage: curl -fsSL https://raw.githubusercontent.com/ishan-parihar/operant/master/scripts/install.sh | bash
set -euo pipefail
IFS=$'\n\t'

cd "$(dirname "$0")"

# ── helpers ───────────────────────────────────────────────────────────────────
die() { printf '[operant] error: %s\n' "$*" >&2; exit 1; }
log() { printf '[operant] %s\n' "$*"; }

# ── args ──────────────────────────────────────────────────────────────────────
FORCE_ROOT=0
SKIP_CTA=0
for _arg in "$@"; do
  case "$_arg" in
    --force-root) FORCE_ROOT=1 ;;
    --no-cta)     SKIP_CTA=1 ;;
  esac
done

# ── root guard ────────────────────────────────────────────────────────────────
# The installer writes to $HOME and never needs root.
if [ "${EUID:-$(id -u)}" -eq 0 ] && [ "$FORCE_ROOT" != "1" ]; then
  die "Don't run as root — this installer targets \\$HOME. Re-run as your user (override: --force-root)."
fi

# ── CTA epilogue: star + donate (gated by --no-cta / SKIP_CTA) ────────────────
print_cta() {
  [ "${SKIP_CTA:-0}" = "1" ] && return 0
  printf '\n%s\n' "──────────────────────────────────────────────────────"
  printf ' ⭐  Found Operant useful? Star it:  https://github.com/ishan-parihar/operant\n'
  printf ' 💜  Support development:\n'
  printf '     GitHub Sponsors : https://github.com/sponsors/ishan-parihar\n'
  printf '     Razorpay (UPI)  : https://rzp.io/rzp/ishan-parihar\n'
  printf '     Or run `operant donate` anytime.\n'
  printf '%s\n' "──────────────────────────────────────────────────────"
}

# Native build dependency preflight
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
OS="$(uname -s)"
MISSING=""
for tool in cmake pkg-config; do
    if command -v "$tool" >/dev/null 2>&1; then
        echo "  ok   $tool"
    else
        echo "  missing  $tool"
        MISSING="$MISSING $tool"
    fi
done

if [ -n "$MISSING" ]; then
    if command -v apt-get >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  sudo apt-get install -y cmake pkg-config"
    elif command -v dnf >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  sudo dnf install -y cmake pkgconfig-pkg-config"
    elif command -v pacman >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  sudo pacman -S --noconfirm cmake pkgconf"
    elif command -v zypper >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  sudo zypper install -y cmake pkg-config"
    elif command -v apk >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  apk add --no-cache cmake pkgconf"
    elif command -v brew >/dev/null 2>&1; then
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}). Install them:\n  brew install cmake pkg-config"
    else
        die "cmake and/or pkg-config are required to build Operant (missing:${MISSING}) on $OS. Install 'cmake' and 'pkg-config' via your system package manager."
    fi
fi

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
ORT_DIR="local/onnxruntime-linux-x64-1.24.4/lib"
# ort 2.x needs ONNX Runtime API 24 (BUGS CR-2). A library that does not serve it
# is refused here, not at the first TTS call, where the panic used to abort.
ort_serves_api_24() {
    python3 - "$1" <<'PROBE'
import ctypes, sys
lib = ctypes.CDLL(sys.argv[1])
lib.OrtGetApiBase.restype = ctypes.c_void_p
base = lib.OrtGetApiBase()
get_api = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_uint32)(
    ctypes.cast(base, ctypes.POINTER(ctypes.c_void_p))[0])
sys.exit(0 if get_api(24) else 1)
PROBE
}
if [ -d "$ORT_DIR" ] && ort_serves_api_24 "$ORT_DIR/libonnxruntime.so"; then
    mkdir -p "$RUNTIME_LIB_DIR"
    # Remove the destination's existing ONNX Runtime files and symlinks
    # FIRST. `cp -a` preserves symlinks, and the destination may already
    # hold a `libonnxruntime.so.1` symlink from an EARLIER install that
    # points at a DIFFERENT version — the copy then leaves that stale
    # symlink in place (or writes through it), and the installed binary
    # fails to start with
    #     libonnxruntime.so.1: version `VERS_1.24.4' not found
    #     (required by .../operant)
    # because the loader follows the OLD link. That is not hypothetical: it
    # is what left this machine's `~/.local/lib/operant/libonnxruntime.so.1`
    # pointing at 1.20.1 while the binary needs 1.24.4, and the binary
    # could not start at all. The versioned library is copied by name, and
    # the symlink is recreated to point at the one that was just copied.
    rm -f "$RUNTIME_LIB_DIR"/libonnxruntime.so* 2>/dev/null || true
    cp -a "$ORT_DIR"/libonnxruntime.so* "$RUNTIME_LIB_DIR"/ 2>/dev/null \
        && echo "  ok   libonnxruntime -> $RUNTIME_LIB_DIR" \
        || echo "  WARN could not copy libonnxruntime; operant may not start"
    # The loader resolves `libonnxruntime.so.1`, so make sure that name
    # points at the version that is actually present.
    if [ -f "$RUNTIME_LIB_DIR/libonnxruntime.so.1.24.4" ]; then
        ln -sf libonnxruntime.so.1.24.4 "$RUNTIME_LIB_DIR/libonnxruntime.so.1"
    fi
    ln -sf libonnxruntime.so.1 "$RUNTIME_LIB_DIR/libonnxruntime.so" 2>/dev/null || true
else
    echo "  WARN no local ONNX Runtime at $ORT_DIR — run scripts/provision-build-deps.sh"
    echo "       before installing, or operant will not start on this machine."
fi

echo ""
echo "=== Installing to ~/.local/bin/ ==="
BIN_DIR="${HOME}/.local/bin"
mkdir -p "$BIN_DIR"
install -m 0755 target/release/operant "$BIN_DIR/operant"
log ">>> generated: $BIN_DIR/operant <<<"

# Also drop a copy in ~/.cargo/bin when it exists — PATH usually resolves
# there first, and a stale copy shadows the fresh ~/.local/bin one.
if [ -d "${HOME}/.cargo/bin" ]; then
    install -m 0755 target/release/operant "${HOME}/.cargo/bin/operant"
    log ">>> generated: ${HOME}/.cargo/bin/operant <<<"
fi

# Add ~/.local/bin to PATH in the user's shell rc (~/.bashrc, ~/.zshrc,
# ~/.profile) if not already present. Idempotent: grep-before-append.
PATH_LINE='export PATH="$HOME/.local/bin:$PATH"'
for rcfile in "${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.profile"; do
    if [ -f "$rcfile" ]; then
        if ! grep -qs 'export PATH=.*\.local/bin' "$rcfile" 2>/dev/null; then
            printf '\n%s\n' "$PATH_LINE" >> "$rcfile"
            log ">>> generated: $rcfile (PATH) <<<"
        fi
    fi
done

# Export for the current session so `operant` is immediately runnable.
export PATH="${HOME}/.local/bin:${PATH}"

echo ""
echo "=== Seeding Bundled Skills ==="
log ">>> generated: ${HOME}/.operant/skills <<<"
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
    # Confirm the service is actually active before claiming success.
    if systemctl --user is-active --quiet operant-gateway 2>/dev/null; then
        echo "  gateway service is active"
    else
        echo "  gateway service installed (run 'operant gateway start' to activate)"
    fi
else
    echo "systemd user session not available — skipping service install"
    echo "Start the gateway manually with: operant gateway run"
fi

# Verify the binary is runnable before claiming success.
if command -v operant >/dev/null 2>&1; then
    operant --version
else
    echo "WARN: operant not found on PATH — add ~/.local/bin to your PATH and re-run"
fi

echo ""
echo "=== Installation Complete ==="

printf '\n%s\n' "──────────────────────────────────────────────────────"
printf ' Next steps:\n'
printf '   1. operant setup                  Interactive wizard: provider, memory, TTS, gateway\n'
printf '   2. operant chat                    Start chatting (TUI)\n'
printf '   3. operant run --query "..."        One-shot agent run (no TUI)\n'
printf '%s\n' "──────────────────────────────────────────────────────"

echo ""
echo "Browser tooling: sourcehound -> $(command -v sourcehound || echo MISSING)"
echo "Skills: $(find "${HERMES_SKILLS_DIR:-${HERMES_HOME:-$HOME/.operant}/skills}" -maxdepth 2 -name SKILL.md 2>/dev/null | wc -l | tr -d ' ') installed"

print_cta

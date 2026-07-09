#!/usr/bin/env bash
# fetch-prebuilt.sh — Download latest prebuilt binaries for igs-rust-mcp + camofox browser.
#
# Per user directive: "rather than using their crates, use the latest prebuilt
# binaries for igs-rust-mcp and obscura browser at each new build/compilation,
# while removing lifeos-ops totally."
#
# This script fetches the latest prebuilt binaries from GitHub releases and
# installs them to ~/.operant/bin/. It is designed to be called:
#   - Manually: ./scripts/fetch-prebuilt.sh
#   - From bootstrap.sh: as part of workspace setup
#   - From CI: before cargo build
#
# Configuration via env vars (all optional):
#   IGS_RUST_MCP_REPO   default: ishan-parihar/igs-rust
#   CAMOFOX_REPO        default: ishan-parihar/obscura
#   OPERANT_BIN_DIR     default: ~/.operant/bin
#   PREBUILT_ARCH       default: x86_64-unknown-linux-gnu (auto-detected)
#
# Idempotent: re-running checks existing binaries and only downloads if newer.
set -euo pipefail

log()  { echo "[fetch-prebuilt] $*"; }
fail() { echo "[fetch-prebuilt] FAIL: $*" >&2; exit 1; }

# ─── Config ──────────────────────────────────────────────────────────────────
IGS_RUST_MCP_REPO="${IGS_RUST_MCP_REPO:-ishan-parihar/igs-rust}"
CAMOFOX_REPO="${CAMOFOX_REPO:-ishan-parihar/obscura}"
OPERANT_BIN_DIR="${OPERANT_BIN_DIR:-$HOME/.operant/bin}"

# Auto-detect architecture
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  PREBUILT_ARCH="${PREBUILT_ARCH:-x86_64-unknown-linux-gnu}" ;;
  Linux-aarch64) PREBUILT_ARCH="${PREBUILT_ARCH:-aarch64-unknown-linux-gnu}" ;;
  Darwin-x86_64) PREBUILT_ARCH="${PREBUILT_ARCH:-x86_64-apple-darwin}" ;;
  Darwin-arm64)  PREBUILT_ARCH="${PREBUILT_ARCH:-aarch64-apple-darwin}" ;;
  *) PREBUILT_ARCH="${PREBUILT_ARCH:-x86_64-unknown-linux-gnu}" ;;
esac

mkdir -p "$OPERANT_BIN_DIR"

# ─── Helper: get latest release tag from GitHub API ──────────────────────────
get_latest_tag() {
  local repo="$1"
  curl -sSfL --max-time 15 \
    "https://api.github.com/repos/$repo/releases/latest" \
    | python3 -c "import json,sys; print(json.load(sys.stdin).get('tag_name',''))" 2>/dev/null \
    || echo ""
}

# ─── Helper: download a release asset ─────────────────────────────────────────
download_asset() {
  local repo="$1"
  local tag="$2"
  local asset_pattern="$3"   # e.g. "igs-rust-mcp-x86_64-unknown-linux-gnu"
  local dest="$4"
  local url="https://github.com/$repo/releases/download/$tag/$asset_pattern"

  log "  Downloading $url"
  if curl -sSfL --max-time 120 -o "$dest" "$url"; then
    chmod +x "$dest"
    log "  ✓ Saved to $dest"
    return 0
  else
    log "  ✗ Download failed (asset may not exist for this arch/tag)"
    return 1
  fi
}

# ─── 1. igs-rust-mcp ─────────────────────────────────────────────────────────
install_igs_rust_mcp() {
  log "── igs-rust-mcp ──"
  local bin_path="$OPERANT_BIN_DIR/igs-rust-mcp"
  local version_file="$OPERANT_BIN_DIR/igs-rust-mcp.version"

  # Check if already installed + up-to-date
  local current_tag=""
  if [[ -f "$bin_path" && -f "$version_file" ]]; then
    current_tag=$(cat "$version_file" 2>/dev/null || echo "")
  fi

  local latest_tag
  latest_tag=$(get_latest_tag "$IGS_RUST_MCP_REPO")
  if [[ -z "$latest_tag" ]]; then
    log "  Could not fetch latest tag for $IGS_RUST_MCP_REPO (offline?)"
    if [[ -f "$bin_path" ]]; then
      log "  Using existing binary at $bin_path"
      return 0
    fi
    log "  WARN: No igs-rust-mcp binary available. IGS MCP tools will be unavailable."
    log "  (This is non-fatal — operant builds and runs without it.)"
    return 0
  fi

  if [[ "$current_tag" == "$latest_tag" && -f "$bin_path" ]]; then
    log "  Already at latest ($latest_tag) — skipping"
    return 0
  fi

  log "  Latest release: $latest_tag (current: ${current_tag:-none})"

  # Try common asset name patterns
  local asset_name="igs-rust-mcp-$PREBUILT_ARCH"
  if download_asset "$IGS_RUST_MCP_REPO" "$latest_tag" "$asset_name" "$bin_path"; then
    echo "$latest_tag" > "$version_file"
    return 0
  fi

  # Fallback: try without arch suffix
  if download_asset "$IGS_RUST_MCP_REPO" "$latest_tag" "igs-rust-mcp" "$bin_path"; then
    echo "$latest_tag" > "$version_file"
    return 0
  fi

  log "  WARN: Could not download igs-rust-mcp binary for $PREBUILT_ARCH"
  log "  The repo may not publish prebuilt binaries for this platform."
  log "  To build from source: clone $IGS_RUST_MCP_REPO && cargo build --release"
  log "  (This is non-fatal — operant builds and runs without it.)"
  return 0
}

# ─── 2. camofox / obscura browser ────────────────────────────────────────────
install_camofox() {
  log "── camofox / obscura browser ──"
  local bin_path="$OPERANT_BIN_DIR/camofox"
  local version_file="$OPERANT_BIN_DIR/camofox.version"

  local current_tag=""
  if [[ -f "$bin_path" && -f "$version_file" ]]; then
    current_tag=$(cat "$version_file" 2>/dev/null || echo "")
  fi

  local latest_tag
  latest_tag=$(get_latest_tag "$CAMOFOX_REPO")
  if [[ -z "$latest_tag" ]]; then
    log "  Could not fetch latest tag for $CAMOFOX_REPO (offline?)"
    if [[ -f "$bin_path" ]]; then
      log "  Using existing binary at $bin_path"
      return 0
    fi
    log "  WARN: No camofox binary available. Browser tool will fall back to"
    log "  lightpanda or other configured provider."
    return 0
  fi

  if [[ "$current_tag" == "$latest_tag" && -f "$bin_path" ]]; then
    log "  Already at latest ($latest_tag) — skipping"
    return 0
  fi

  log "  Latest release: $latest_tag (current: ${current_tag:-none})"

  # Try common asset name patterns
  for asset_name in "camofox-$PREBUILT_ARCH" "camofox" "obscura-$PREBUILT_ARCH" "obscura"; do
    if download_asset "$CAMOFOX_REPO" "$latest_tag" "$asset_name" "$bin_path"; then
      echo "$latest_tag" > "$version_file"
      return 0
    fi
  done

  log "  WARN: Could not download camofox/obscura binary for $PREBUILT_ARCH"
  log "  The repo may not publish prebuilt binaries for this platform."
  log "  To use camofox: install it manually and set CAMOFOX_URL env var."
  log "  (This is non-fatal — operant builds and runs without it.)"
  return 0
}

# ─── Run ─────────────────────────────────────────────────────────────────────
log "Architecture: $PREBUILT_ARCH"
log "Install dir:  $OPERANT_BIN_DIR"
log ""

install_igs_rust_mcp
log ""
install_camofox

log ""
log "── Summary ──"
log "  igs-rust-mcp: $([[ -x "$OPERANT_BIN_DIR/igs-rust-mcp" ]] && echo "installed ✓" || echo "not installed")"
log "  camofox:      $([[ -x "$OPERANT_BIN_DIR/camofox" ]] && echo "installed ✓" || echo "not installed")"
log ""
log "── Wiring ──"
log "  igs-rust-mcp: Add to operant.toml [mcp] section as a stdio MCP server:"
log "    [mcp.igs]"
log "    command = \"$OPERANT_BIN_DIR/igs-rust-mcp\""
log "    args = []"
log ""
log "  camofox: Launch the browser REST server, then set:"
log "    export CAMOFOX_URL=http://localhost:9222"
log "    (or set it in operant.toml [browser] provider = \"camofox\")"
log ""
log "Done. Both are optional — operant builds and runs without them."

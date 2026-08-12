#!/usr/bin/env bash
# install-browser-deps.sh — Provision `igs` and `obscura` as GLOBAL executables.
#
# Called automatically by `./scripts/install.sh` (and usable standalone).
# Guarantees the operant agent can run `igs ...` / `obscura ...` CLI commands
# directly, and that browser + IGS web tools share the SAME obscura binary.
#
# Sources (mirrors igs-rust's own managers):
#   igs     → github.com/ishan-parihar/igs-rust/releases  (musl tarball)
#   obscura → github.com/h4ckf0r0day/obscura/releases    (stealth tarball)
#
# Idempotent: re-running keeps existing up-to-date binaries and skips
# downloads. Honors: IGS_BIN_DIR, GLOBAL_BIN_DIR, IGS_REPO, OBSCURA_REPO,
# IGS_TAG, OBSCURA_TAG (pin versions), NO_SUDO=1 (never sudo).
set -euo pipefail

log()  { echo "[install-browser-deps] $*"; }
fail() { echo "[install-browser-deps] FAIL: $*" >&2; exit 1; }

# ─── Config ──────────────────────────────────────────────────────────────────
IGS_REPO="${IGS_REPO:-ishan-parihar/igs-rust}"
OBSCURA_REPO="${OBSCURA_REPO:-h4ckf0r0day/obscura}"
IGS_TAG="${IGS_TAG:-}"          # pin e.g. v1.0.3 (default: latest)
OBSCURA_TAG="${OBSCURA_TAG:-}"  # pin e.g. v0.2.0 (default: latest)

# IGS-managed binary dir (the shared copy IGS web tools use).
IGS_MANAGED_DIR="${IGS_MANAGED_DIR:-$HOME/.config/igs-mcp/bin}"
# Operant-managed fallback for obscura when IGS is not installed.
OPERANT_BIN_DIR="${OPERANT_BIN_DIR:-$HOME/.operant/bin}"
# Where the GLOBAL executables land. `~/.local/bin` is user-writable and on
# PATH; fall back to /usr/local/bin (sudo) when it is not on PATH.
GLOBAL_BIN_DIR="${GLOBAL_BIN_DIR:-$HOME/.local/bin}"

# ─── Platform detection ──────────────────────────────────────────────────────
detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os-$arch" in
    Linux-x86_64)  echo "linux-x86_64" ;;
    Linux-aarch64) echo "linux-aarch64" ;;
    Darwin-x86_64) echo "darwin-x86_64" ;;
    Darwin-arm64)  echo "darwin-arm64" ;;
    *) echo "unsupported" ;;
  esac
}

# Map platform → igs tarball asset name.
igs_asset() {
  case "$1" in
    linux-x86_64)  echo "igs-\$tag-x86_64-unknown-linux-musl.tar.gz" ;;
    linux-aarch64) echo "igs-\$tag-aarch64-unknown-linux-musl.tar.gz" ;;
    darwin-x86_64) echo "igs-\$tag-x86_64-apple-darwin.tar.gz" ;;
    darwin-arm64)  echo "igs-\$tag-aarch64-apple-darwin.tar.gz" ;;
    *) fail "no igs release for platform $1" ;;
  esac
}

# Map platform → obscura tarball asset name (always the stealth build so
# browser + IGS share the anti-detection binary).
obscura_asset() {
  case "$1" in
    linux-x86_64)  echo "obscura-x86_64-linux-stealth.tar.gz" ;;
    linux-aarch64) echo "obscura-aarch64-linux-stealth.tar.gz" ;;
    darwin-x86_64) echo "obscura-x86_64-macos-stealth.tar.gz" ;;
    darwin-arm64)  echo "obscura-aarch64-macos-stealth.tar.gz" ;;
    *) fail "no obscura release for platform $1" ;;
  esac
}

# ─── Helpers ─────────────────────────────────────────────────────────────────
latest_tag() {
  local repo="$1"
  curl -sSfL --max-time 20 "https://api.github.com/repos/$repo/releases/latest" \
    | python3 -c "import json,sys; print(json.load(sys.stdin).get('tag_name',''))" 2>/dev/null \
    || echo ""
}

path_on_path() {
  case ":$PATH:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

install_global() {
  # $1 = source binary, $2 = name (igs|obscura), $3 = global bin dir
  local src="$1" name="$2" bin_dir="$3"
  mkdir -p "$bin_dir"
  cp -f "$src" "$bin_dir/$name"
  chmod +x "$bin_dir/$name"
  # Also mirror into /usr/local/bin when possible so the executable is truly
  # global (operant's install.sh already uses sudo there for the operant bin).
  if command -v sudo &>/dev/null && [[ "${NO_SUDO:-0}" != "1" ]]; then
    if sudo -n true 2>/dev/null; then
      sudo cp -f "$src" "/usr/local/bin/$name"
      sudo chmod +x "/usr/local/bin/$name"
    fi
  fi
  log "  ✓ $name -> $bin_dir/$name"
}

download_tar_gz() {
  # $1 = url, $2 = dest dir, $3 = member to extract (binary name)
  local url="$1" dest="$2" member="$3" tmp
  tmp="$(mktemp -d)"
  log "  Downloading $url"
  curl -sSfL --max-time 300 -o "$tmp/pkg.tar.gz" "$url" || { rm -rf "$tmp"; return 1; }
  tar -xzf "$tmp/pkg.tar.gz" -C "$tmp" "$member" 2>/dev/null \
    || tar -xzf "$tmp/pkg.tar.gz" -C "$tmp" 2>/dev/null
  local found
  found="$(find "$tmp" -maxdepth 2 -type f -name "$member" | head -1)"
  if [[ -z "$found" ]]; then
    log "  ✗ binary '$member' not found in archive"
    rm -rf "$tmp"
    return 1
  fi
  mkdir -p "$dest"
  cp -f "$found" "$dest/$member"
  chmod +x "$dest/$member"
  rm -rf "$tmp"
  return 0
}

# ─── 1. igs ──────────────────────────────────────────────────────────────────
install_igs() {
  local platform tag asset url
  platform="$(detect_platform)"
  [[ "$platform" == "unsupported" ]] && fail "unsupported platform"
  tag="${IGS_TAG:-$(latest_tag "$IGS_REPO")}"
  [[ -z "$tag" ]] && fail "could not resolve latest igs tag (offline?)"

  if command -v igs &>/dev/null; then
    local have
    have="$(igs --version 2>&1 | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
    local want
    want="$(echo "$tag" | sed 's/^v//')"
    if [[ -n "$have" && "$have" == "$want" ]]; then
      log "  igs already at $have (latest) — skipping"
      return 0
    fi
    log "  igs $have found but latest is $want — upgrading"
  else
    log "  igs not found — installing $tag"
  fi

  # Asset filenames drop the 'v' prefix (igs-1.0.3-...tar.gz), while the
  # download URL keeps it (/releases/download/v1.0.3/...).
  asset="$(igs_asset "$platform" | sed "s/\$tag/${tag#v}/")"
  url="https://github.com/$IGS_REPO/releases/download/$tag/$asset"
  local tmp
  tmp="$(mktemp -d)"
  if download_tar_gz "$url" "$tmp" "igs"; then
    install_global "$tmp/igs" "igs" "$GLOBAL_BIN_DIR"
  else
    log "  ✗ igs download failed for $platform (asset may not be published)"
  fi
  rm -rf "$tmp"
}

# ─── 2. obscura (shared with IGS) ───────────────────────────────────────────
install_obscura() {
  local platform tag asset url src=""
  platform="$(detect_platform)"
  [[ "$platform" == "unsupported" ]] && fail "unsupported platform"

  # Prefer the IGS-managed copy when present — this is the SAME binary IGS
  # web tools use (single-binary guarantee for browser + IGS).
  if [[ -x "$IGS_MANAGED_DIR/obscura" ]]; then
    log "  reusing IGS-managed obscura ($IGS_MANAGED_DIR/obscura)"
    install_global "$IGS_MANAGED_DIR/obscura" "obscura" "$GLOBAL_BIN_DIR"
    # mirror the worker too when present (obscura worker subprocess)
    if [[ -x "$IGS_MANAGED_DIR/obscura-worker" && ! -x "$GLOBAL_BIN_DIR/obscura-worker" ]]; then
      cp -f "$IGS_MANAGED_DIR/obscura-worker" "$GLOBAL_BIN_DIR/obscura-worker"
      chmod +x "$GLOBAL_BIN_DIR/obscura-worker"
      log "  ✓ obscura-worker -> $GLOBAL_BIN_DIR/obscura-worker"
    fi
    return 0
  fi

  # Operant-managed fallback: already downloaded before?
  if [[ -x "$OPERANT_BIN_DIR/obscura" ]]; then
    log "  reusing operant-managed obscura ($OPERANT_BIN_DIR/obscura)"
    install_global "$OPERANT_BIN_DIR/obscura" "obscura" "$GLOBAL_BIN_DIR"
    return 0
  fi

  tag="${OBSCURA_TAG:-$(latest_tag "$OBSCURA_REPO")}"
  [[ -z "$tag" ]] && fail "could not resolve latest obscura tag (offline?)"
  asset="$(obscura_asset "$platform")"
  url="https://github.com/$OBSCURA_REPO/releases/download/$tag/$asset"
  log "  downloading stealth obscura $tag ($asset)"
  if download_tar_gz "$url" "$OPERANT_BIN_DIR" "obscura"; then
    install_global "$OPERANT_BIN_DIR/obscura" "obscura" "$GLOBAL_BIN_DIR"
  else
    log "  ✗ obscura download failed for $platform"
  fi
}

# ─── Run ─────────────────────────────────────────────────────────────────────
log "platform: $(detect_platform)"
log "global bin dir: $GLOBAL_BIN_DIR"

install_igs
install_obscura

log ""
log "── Summary ──"
log "  igs:      $(command -v igs || echo MISSING)"
log "  obscura:  $(command -v obscura || echo MISSING)"
log ""
if ! path_on_path "$GLOBAL_BIN_DIR"; then
  log "NOTE: $GLOBAL_BIN_DIR is not on PATH. Add it, e.g.:"
  log "  echo 'export PATH=\"$GLOBAL_BIN_DIR:\$PATH\"' >> ~/.bashrc"
fi
log "Done."

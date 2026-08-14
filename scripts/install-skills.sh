#!/usr/bin/env bash
# install-skills.sh — Seed the bundled skill pool into the user skills dir.
#
# Called automatically by `./scripts/install.sh` (and usable standalone), so a
# packaged install ships the same 29-skill pool the repo ships — no network, no
# marketplace fetch, works offline.
#
# Layout: the repo pool is CATEGORIZED (`skills/<category>/<skill>/SKILL.md`);
# the user skills dir is FLAT (`~/.operant/skills/<skill>/SKILL.md`), matching
# exactly what the runtime's first-run bootstrap and `operant skills seed`
# (cmd_skills::seed_bundled_skills) produce. `operant skills list` therefore
# sees the same leaf skills whether seeded by this script, by the first-run
# bootstrap, or by `operant skills seed --force`.
#
# Idempotent: re-running keeps existing skills and skips the copy when the
# target already has any skill installed. `FORCE=1` (or `--force`) re-seeds
# everything, overwriting local edits. Honors: HERMES_HOME, HERMES_SKILLS_DIR,
# OPERANT_BUNDLED_SKILLS_DIR, NO_SUDO (unused, accepted for symmetry).
set -euo pipefail

log()  { echo "[install-skills] $*"; }
fail() { echo "[install-skills] FAIL: $*" >&2; exit 1; }

# ─── Config ──────────────────────────────────────────────────────────────────
# Honor the FORCE env var (do not clobber it), and allow --force/-f args.
FORCE="${FORCE:-0}"
[[ "${1:-}" == "--force" || "${1:-}" == "-f" ]] && FORCE=1

# Pool shipped with operant: `<scripts>/../skills` (repo checkout).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POOL_DIR="${OPERANT_BUNDLED_SKILLS_DIR:-$SCRIPT_DIR/../skills}"

# User skills dir: honors the same overrides the binary honors.
#   HERMES_SKILLS_DIR  → config `[skills] root_dir` override (binary reads it)
#   HERMES_HOME        → operant home (default `~/.operant`)
SKILLS_DIR="${HERMES_SKILLS_DIR:-${HERMES_HOME:-$HOME/.operant}/skills}"

# ─── Helpers ─────────────────────────────────────────────────────────────────
# True when a dir contains a skill (SKILL.md directly inside).
has_skill() { [[ -f "$1/SKILL.md" ]]; }

# Copy a single skill dir into the flat user skills dir (skip existing unless
# forced). Mirrors cmd_skills::seed_bundled_skills semantics.
copy_skill() {
  local src="$1" name
  name="$(basename "$src")"
  local dst="$SKILLS_DIR/$name"
  if [[ -e "$dst" && "$FORCE" != "1" ]]; then
    log "  = $name (already installed)"
    return 0
  fi
  # `cp -R src dst` NESTS when dst already exists (dst/<name>/…), so remove the
  # stale target first on a forced re-seed — the pool copy must land flat.
  if [[ -e "$dst" && "$FORCE" == "1" ]]; then
    rm -rf "$dst"
  fi
  mkdir -p "$(dirname "$dst")"
  cp -R "$src" "$dst"
  chmod -R u+rwX "$dst"
  log "  ✓ $name"
}

seed_pool() {
  [[ -d "$POOL_DIR" ]] || fail "bundled pool '$POOL_DIR' not found (run from the repo checkout)"
  log "pool:   $POOL_DIR"
  log "target: $SKILLS_DIR"

  mkdir -p "$SKILLS_DIR"
  if [[ "$FORCE" != "1" && -n "$(ls -A "$SKILLS_DIR")" ]]; then
    log "target already has skills — skipping (FORCE=1 to re-seed)"
    return 0
  fi

  local entry sub
  local copied=0
  for entry in "$POOL_DIR"/*; do
    [[ -d "$entry" ]] || continue
    if has_skill "$entry"; then
      # Flat skill directly in the pool root.
      copy_skill "$entry"
      copied=$((copied + 1))
    else
      # Category dir: every subdir carrying SKILL.md is a leaf skill.
      for sub in "$entry"/*; do
        [[ -d "$sub" && -f "$sub/SKILL.md" ]] || continue
        copy_skill "$sub"
        copied=$((copied + 1))
      done
    fi
  done
  log "seeded $copied skill(s)"
}

# ─── Run ─────────────────────────────────────────────────────────────────────
seed_pool
log "Done. Verify with: operant skills list"

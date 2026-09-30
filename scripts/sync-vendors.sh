#!/usr/bin/env bash
# scripts/sync-vendors.sh — advance the memory-wire branch dep to latest HEAD.
#
# Why this exists: operant bakes memory-wire in-process as a cargo
# git-dependency tracking `main` (user-ordered persistent-latest). The
# dev loop (scripts/check.sh) already advances it on a stamp TTL; this script
# is the explicit form — run it to force-advance now, e.g. right after an
# upstream fix lands. sourcehound_mcp is NOT covered: it is commented out of
# the manifest (held: vendored-render divergence), so `cargo update -p
# sourcehound` would error under `set -euo pipefail` — a second package arg
# here is how a future re-add breaks this script silently. Do NOT wire this
# into build.rs: implicit network at build time breaks offline builds.
#
# At release time, convert tracking to pins: replace `branch =` with the
# printed `rev =` in crates/operant-core/Cargo.toml and re-run this script.
set -euo pipefail
cd "$(dirname "$0")/.."

rev_of() { # $1 = package name in Cargo.lock
  awk -v pkg="$1" '
    /^\[\[package\]\]/{found=0}
    $0 == "name = \"" pkg "\""{found=1}
    found && /^source = "git\+/{print; exit}' Cargo.lock \
    | grep -oE '[0-9a-f]{40}$' | head -1
}

echo "== revs before =="
echo "  memory-wire:  $(rev_of memory-wire)"

cargo update -p memory-wire

echo "== revs after =="
echo "  memory-wire:  $(rev_of memory-wire)"

echo "== verify resolve + compile =="
./scripts/check.sh check -p operant-core --lib
echo "sync-vendors: OK"

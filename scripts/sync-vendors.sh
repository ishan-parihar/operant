#!/usr/bin/env bash
# scripts/sync-vendors.sh — advance git-vendored engine deps to latest branch HEADs.
#
# Why this exists: operant bakes memory-wire and sourcehound_mcp in-process as
# cargo git-dependencies tracking their active branches (memory-wire: main,
# sourcehound: master). Cargo.lock pins exact revs, so a plain build does NOT
# pick up upstream work — including the separate agent upgrading memory-wire.
# Run this script to pull the latest commits from both repos into the lock,
# verify the graph still resolves and compiles, and report the new revs.
#
# When to run: before release builds, at iteration start when upstream moved,
# or whenever `git ls-remote` shows the branches ahead of the lock. Do NOT
# wire this into build.rs: implicit network at build time breaks offline
# builds and destroys reproducibility (the lock pin is the build fact).
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
echo "  sourcehound:  $(rev_of sourcehound)"

cargo update -p memory-wire -p sourcehound

echo "== revs after =="
echo "  memory-wire:  $(rev_of memory-wire)"
echo "  sourcehound:  $(rev_of sourcehound)"

echo "== verify resolve + compile =="
./scripts/check.sh check -p operant-core --lib
echo "sync-vendors: OK"

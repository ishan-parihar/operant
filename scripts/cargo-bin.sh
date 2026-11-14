# shellcheck shell=bash
# Resolve a cargo that can actually start. SOURCE this, then call
# `resolve_cargo_bin`; it sets CARGO_BIN and returns non-zero when nothing works.
#
# Why this exists (2026-11-14, MEASURED). `~/.local/bin/cargo` is first in PATH
# on this host and is NOT cargo: it `exec`s `systemd-run`, which exits 1 with
# "Failed to connect to user scope bus via local transport: No data available".
# Piped, that failure disappears — a pipeline's status is the LAST command's —
# so a gate can report PASS having compiled nothing. Three "green" checks in
# the 2026-11-14 swarm audit were vacuous this way, and
# `scripts/clippy-warning-gate.sh` printed
#   [clippy-gate] seen=0 allowlist=0 new=0 stale=0
#   [clippy-gate] pass
# having run no clippy at all.
#
# build.sh has probed its candidates since iter-1707; check.sh ended in a bare
# `exec cargo "$@"` and the clippy gate still does, so those two were the gates
# that could not run. One resolver now, shared, so the three scripts cannot
# disagree about which binary they measured with.
#
# A caller that needs a specific cargo can pre-set CARGO_BIN and this honours it.
resolve_cargo_bin() {
    local candidate
    CARGO_BIN="${CARGO_BIN:-$(command -v cargo || true)}"
    if [ -n "$CARGO_BIN" ] && "$CARGO_BIN" --version >/dev/null 2>&1; then
        return 0
    fi
    for candidate in /usr/bin/cargo /usr/local/bin/cargo; do
        if [ -x "$candidate" ] && "$candidate" --version >/dev/null 2>&1; then
            CARGO_BIN="$candidate"
            return 0
        fi
    done
    CARGO_BIN=""
    return 1
}
#!/bin/bash
# Install script for Operant
# Builds the release binary and installs it globally

set -e

echo "=== Building Operant Release Binary ==="
cd "$(dirname "$0")"
cargo build --release -p operant-cli

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
echo "=== Installing Browser Dependencies (igs + obscura) ==="
# The agent's IGS web tools and the shared obscura browser are driven via the
# `igs` and `obscura` CLIs on PATH. Provision them globally (idempotent; reuses
# the IGS-managed obscura so browser + IGS share one binary).
# Best-effort: browser deps are optional — never abort operant's install if
# the download is unavailable (offline / unsupported platform).
bash "$(dirname "$0")/install-browser-deps.sh" || echo "WARN: browser deps provisioning failed (non-fatal)"

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
echo "Browser tooling: igs -> $(command -v igs || echo MISSING), obscura -> $(command -v obscura || echo MISSING)"
echo "Skills: $(find "${HERMES_SKILLS_DIR:-${HERMES_HOME:-$HOME/.operant}/skills}" -maxdepth 2 -name SKILL.md 2>/dev/null | wc -l | tr -d ' ') installed"

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

echo ""
echo "=== Installation Complete ==="
echo ""
echo "You can now run: operant --version"
echo "Or start chatting: operant"
echo ""
echo "To initialize configuration: operant setup"
echo "To start the dashboard: operant dashboard"
echo "To start the gateway: operant gateway start"

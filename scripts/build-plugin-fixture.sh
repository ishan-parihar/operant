#!/usr/bin/env bash
# Rebuild the G8 fixture WASM module. The built artifact is COMMITTED at
# crates/operant-plugins/fixtures/fixture.wasm so the integration tests
# (crates/operant-plugins/tests/fixture_plugin.rs) run on machines without
# the wasm32-wasip1 target; run this only when the fixture source changes.
# Requires: rustup target add wasm32-wasip1
set -euo pipefail
cd "$(dirname "$0")/../crates/operant-plugins/fixtures/fixture-plugin"
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/fixture_echo.wasm ../fixture.wasm
ls -l ../fixture.wasm

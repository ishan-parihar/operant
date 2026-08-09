# 013 — Integration smoke tests: gateway WS, ACP, channel→agent

Stamped: `d394c136`. Priority: **P2**. Do after 003 (gateway surface finalized) and
005 (channels finalized).

## Why

All ~4,800 tests are in-module unit tests; only `operant-core` has integration tests
(4 files). The gateway (WS/SSE/ACP) and channel→agent paths have **zero** end-to-end
tests — the exact surfaces where wiring bugs (R13 routes, R15 channels, R17 /yolo,
R22 whatsapp) were found by hand. A small smoke layer catches regression at the
boundary.

## Files in scope

- New `crates/operant-gateway/tests/gateway_smoke.rs`
- New `crates/operant-gateway/tests/acp_smoke.rs` (if the ACP server can be started
  in-process; else fold into gateway_smoke via the ACP endpoint)
- New `crates/operant-cli/tests/channel_smoke.rs` (channel adapter send path with a
  mocked HTTP server — no network)

## Files out of scope

- Unit-test expansion in the crates themselves.
- Real third-party network calls (Telegram/Discord/etc.) — mock HTTP only.

## Steps

1. **Gateway WS roundtrip**: start the gateway in-process (use the existing
   `start_gateway`-style builder in `gateway_runner.rs`/`gateway/lib.rs` — the
   router-boot harness already lives inside `operant-gateway`'s own lib tests (219
   tests; the 34-test `operant-api` crate is the provider/schema layer and does NOT
   boot the router — mirror the gateway crate's harness, not operant-api), connect a
   real WebSocket client
   (tokio-tungstenite is likely already a dev-dependency — verify), send a message,
   assert a scripted-provider agent response arrives and a session row is persisted.
   Include: bad-route 404, `/health` 200, WS auth rejection when token auth configured.
2. **ACP roundtrip**: start the ACP server (R18), issue the JSON-RPC handshake, one
   prompt message, assert a response notification arrives and the server stays honest
   (Running→Idle states, R18 contract).
3. **Channel→agent smoke**: pick one adapter whose transport is HTTP-mockable
   (WhatsApp Cloud is ideal — R22 wired `phone_number_id` end-to-end; or Telegram
   send via a local mock server). Assert: incoming webhook message → agent turn → the
   outbound send hits the mock with the reply. This is the R15/R22 regression net.
4. Run the new tests; add a `tests/README.md` note on how to run them offline.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-gateway --all-targets -- -D warnings && cargo clippy -p operant-cli --all-targets -- -D warnings
cargo test -p operant-gateway --test gateway_smoke && cargo test -p operant-gateway --test acp_smoke && cargo test -p operant-cli --test channel_smoke
cargo test --workspace --all-features --lib          # final gate (unit suites unaffected)
```

## Test plan

- The three smoke tests ARE the deliverable. Keep them fast (<10s each) and fully
  offline (scripted provider, mocked HTTP).
- Assert at least one positive + one negative case per surface (auth rejection,
  bad-route, honest state transitions).

## Maintenance note

- These tests are the first line of defense for gateway wiring changes — any new
  route/channel wiring should extend them rather than add bespoke boot code.
- If the gateway boot path changes, update the tests in the same commit (they pin the
  boot contract).

## Escape hatches

- If the gateway can't boot in-process (e.g. needs a real port binding that the test
  runner forbids), bind port 0 (ephemeral) and read the assigned port from the server
  handle — do not hardcode ports.
- If ACP cannot run in-process in one round, ship the WS smoke first and file the ACP
  smoke as a follow-up round.

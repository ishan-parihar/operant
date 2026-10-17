# 003 — Gateway stub surface: implement or remove mounted 501 endpoints; document auth

Stamped: `d394c136`. Priority: **P0**.

## Why

`crates/operant-gateway/src/api.rs` **mounts** several handlers that return
`501 NOT_IMPLEMENTED`, so a configured-but-unfinished integration silently fails:
`/pair`, `/webhooks/github`, `/webhooks/gmail`, `/webhooks/gmail/push`,
`/webhooks/linq`, `/webhooks/nextcloud-talk`, `/webhooks/wati`
(routes at `api.rs:1508–1518`, stub handlers at `api.rs:1655–1717`).
Meanwhile `crates/operant-gateway/src/api.rs:39` (router-doc/error text) tells clients
"Unauthorized — pair first via POST /pair" — an advertised auth flow that returns 501.
The gateway is effectively unauthenticated-by-default.

## Files in scope

- `crates/operant-gateway/src/api.rs` (routes + stub handlers)
- `crates/operant-gateway/src/ws.rs` (auth error text at ~line 39; token extraction
  already exists: `Authorization: Bearer`, `Sec-WebSocket-Protocol: bearer.<token>`,
  `?token=` — ws.rs:124–134)
- `crates/operant-config/src/schema.rs` (gateway settings: host/port default, any
  `auth_token` field)

## Files out of scope

- Implementing the actual GitHub/Gmail/Linq/Nextcloud/Wati integrations (separate
  add-on surface; if a platform integration exists elsewhere, wire it — otherwise the
  route must be removed, not stubbed).
- The channels crate platform adapters.

## Current state (evidence)

- `api.rs:1661–1717`: 9 stub handlers (`handle_api_github_webhook`,
  `handle_api_gmail_webhook`, `handle_api_gmail_push`, `handle_api_linq_webhook`,
  `handle_api_nextcloud_talk_webhook`, `handle_api_wati_webhook`,
  `handle_api_pair`, `handle_api_pair_status`, `handle_api_pair_generate_code`).
- `api.rs:1508–1518`: routes mounted to those stubs.
- `ws.rs:39`: error string references `POST /pair`.
- WS auth primitives already exist (`extract_ws_token`, ws.rs:124–134) and there is a
  `token` config option (ws.rs:56).

## Steps

1. **Inventory each mounted stub**: grep the codebase for the corresponding platform
   integration (e.g. `github webhook` handler, `linq` channel adapter, `wati` adapter,
   gmail push consumer). For each stub:
   - If a real implementation exists but isn't wired → wire it (route → real handler).
   - If no implementation exists → **remove** the route AND the stub handler
     (dead 501 surface is worse than a 404 — a 404 tells the truth).
2. **Pair flow**: either (a) implement minimal real pairing — `POST /pair` creates a
   random token, persists it to the gateway state (0600, via the core secret-write
   helper from plan 002, `crates/operant-core/src/fs_secrets.rs`), returns it once,
   `extract_ws_token` validates against it — or (b) remove the `/pair` routes and
   rewrite the auth error text at `api.rs:39` to describe the actual methods (config
   `token`, bearer header/subprotocol/query). Prefer (a) if the
   dashboard/`pair`-referencing clients exist; prefer (b) if nothing calls it. Verify
   which by grepping for `/pair` callers.
3. **Document the auth posture**: state in the gateway README section (or
   `docs/security.md` created in plan 014) what the default bind host is, that the
   gateway is unauthenticated by default, and how to enable `token` auth. Note the
   default `host:port` in `gateway_runner.rs` / `gateway/lib.rs:696`.
4. Add tests (below). Run the gateway suite.

## Done criteria

```bash
cargo fmt --all --check
cargo clippy -p operant-gateway --all-targets -- -D warnings
cargo test -p operant-gateway --lib webhook && cargo test -p operant-gateway --lib pair && cargo test -p operant-gateway --lib auth
cargo test --workspace --all-features --lib          # final gate
```
Manual: `operant gateway` (local) → `curl -X POST localhost:<port>/webhooks/github`
returns **404** (removed) or a real response — never 501. `ws.rs` error text matches
reality.

## Test plan

- `test_unmounted_stub_returns_404`: for each removed stub, router returns 404.
- `test_pair_flow_roundtrip` (if pairing implemented): POST /pair → token; WS connect
  with `?token=` succeeds; without token → 401.
- `test_ws_token_extraction_from_all_sources`: header / subprotocol / query (extend the
  existing `extract_ws_token` unit tests if present).

## Maintenance note

- Never mount a stub returning 501 for a real route. Add a router-level test that walks
  the mounted routes and asserts no handler is a stub (or keep a small allow-list).
- The `#[allow(dead_code)]` gmail Pub/Sub handler (`gateway/src/lib.rs:1903`) should be
  removed or wired in the same pass.

## Escape hatches

- If a "real implementation" exists but is half-finished (compiles, wrong behavior),
  treat as not-implemented: remove the route, and leave a `BUGS.md` note listing the
  integration as a future add-on. Do not ship half-wired webhooks.

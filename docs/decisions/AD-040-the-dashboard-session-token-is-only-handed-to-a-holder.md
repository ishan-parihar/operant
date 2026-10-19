---
ID: "AD-040"
Title: "The dashboard's session token is only ever handed to a holder of that token"
Status: "Active"
Date: "2026-10-19"
Description: "The dashboard served its session token on an unauthenticated route, so the bearer auth was theatre; the token is now exchanged for an HttpOnly cookie and the Host header is validated"
---

# AD-040: The dashboard's session token is only ever handed to a holder of that token

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The dashboard's security model, as written in its own module doc:

> Security: API endpoints (except /api/health) require a bearer token that
> is generated at startup and printed to the console. The token is injected
> into index.html as a global variable so the frontend can call the API
> without manual configuration.

The auth existed and it worked. `GET /api/status` without a token returned
401; with the token it returned 200. The problem was where the token came
from.

`handle_index` injected the token into the HTML, and the index route
required no auth. So `GET /` handed the token to anyone who could reach the
port.

Reproduced against the iter-995 binary:

```
GET / -> 200 OK
  token present in the UNAUTHENTICATED body: __HERMES_SESSION_TOKEN__="operant_24cb14a9..."
GET /api/config with the scraped token -> 200 OK
  {"model":"small-stack","base_url":"https://aroute.ishanparihar.com/v1",...}
```

The bearer auth was the lock on the back door, and the front door was
unlocked. Anyone who could reach the port could read the key off the
mantelpiece.

Two properties made it worse than a theoretical concern:

- The dashboard binds to `127.0.0.1` by default. Loopback is reachable from
  every web page the operator visits.
- There was no `Host` header validation. A page can point a name it controls
  at 127.0.0.1 (DNS rebinding), and the browser then sends a `Host` header
  the page chose — the one header that tells a loopback service "this
  request came from a stranger".

## Decision

**The token is only ever handed to a request that already holds it.** Three
changes, all in `crates/operant-cli/src/dashboard_server.rs`.

### 1. The index route gates on the token

The exchange:

1. The CLI opens the browser at `/?token=...`. The token is generated in
   `cmd_dashboard.rs`, not inside the server, because the server binds and
   blocks — the caller has to have the token to open the browser, and the
   browser must not launch before the listener is up.
2. That request presents the token in the query. The server sets an
   **HttpOnly, SameSite=Strict** cookie carrying it and redirects to `/`,
   so the token does not linger in the address bar or in the browser's
   history.
3. `GET /` presents the cookie, and the server serves the page with the
   token injected. The frontend reads it from
   `window.__HERMES_SESSION_TOKEN__`, so it has to be in the page.
4. A request with neither gets a 401 and the body
   `{"error": "unauthenticated"}`.

The 401 body deliberately carries **no `login_url`**. The frontend's 401
path does `window.location.assign(e.login_url)`, and the only URL that
carries the token is the one the CLI prints — putting it in a 401 body
would hand the token to every caller that can reach the port.

### 2. The `Host` header is validated

A `host_guard` middleware refuses a request whose `Host` is not one this
server answers to. The rule has three cases, in order:

- **An absent `Host` is refused.** HTTP/1.1 requires the header and every
  browser sends it, so a request without one is not one a browser made.
- **A host that is an IP literal is allowed.** A server bound to `0.0.0.0`
  is reachable on every interface, and the operator reaches it by the
  address their machine has on the LAN — which is neither the bind address
  nor on the allowlist. Refusing it would 403 the operator's own browser.
  A DNS-rebinding attack uses a NAME it controls, not an IP literal, so
  this does not open the hole.
- **Otherwise the host must be on the allowlist**: the bound address plus
  the loopback spellings of it.

### 3. The server and the browser launch are separated

`run_dashboard` was split into `build_app` / `bind_listener` / `serve`, so
the CLI can bind FIRST and then open the browser. A browser that launches
before the server is listening gets a connection-refused on its first
request, and the operator sees a dead dashboard.

## Why the cookie and not a header-only design

The frontend is a prebuilt bundle that reads the token from the page and
sends it as a header. Rebuilding the bundle was out of scope; the cookie is
what lets the existing bundle work while the token stays out of reach of a
cross-origin caller. `HttpOnly` means JavaScript cannot read it, and
`SameSite=Strict` means a cross-site request never carries it. The token
still ends up in the page — that is the bundle's contract — but only for a
request that already proved it holds the token.

## The header the bundle actually sends

The bundle's fetch wrapper sets **`X-Hermes-Session-Token`** on every
request. The string `Authorization` does not appear in it at all.

The server read only `Authorization: Bearer`. So every API call the real
dashboard made returned 401: the UI could never have worked, and the
server's own tests passed because they sent the header the SERVER
expected rather than the one the CLIENT sends. The auth was not just
weak — it was wired to a contract that does not exist on the other side.

`check_auth` now accepts the token from any of three places:

- `Authorization: Bearer` — a human or a script.
- `X-Hermes-Session-Token` — what the bundled frontend sends.
- The `operant_dashboard_token` cookie — what the browser sends on a
  same-origin fetch, which is what lets the UI work without the token
  being readable by JavaScript.

The test reads the header name OUT OF THE BUNDLE, so a change to the
bundle's contract fails the test rather than silently breaking the UI
again.

## The dashboard is four routes, not the full console

The bundled frontend is a build artefact from a much larger operant
harness: it calls **79 distinct `/api/...` paths**. This server implements
**four**. Every other call 404s, and the `/api/ws` WebSocket the bundle
opens for live updates does not exist at all.

That is a product gap, not a bug, and it is recorded as BUGS.md RT-44 with
the three options: build the 75 missing routes, ship a bundle that matches
the four routes, or declare the four routes to be the dashboard. The second
and third are much cheaper than the first, and the choice is the owner's.

## Evidence

Verified end to end against the running server:

```
GET /                    -> 401 Unauthorized   (no token in the body)
GET /?token=...          -> 303 See Other
  set-cookie: operant_dashboard_token=...; HttpOnly; SameSite=Strict; Path=/
  location: /
GET /  (with the cookie) -> 200 OK              (the page carries the token)
GET /api/status (Bearer) -> 200 OK
GET /  (Host: attacker.example) -> 403 Forbidden
```

Eight tests against the REAL server on an ephemeral port, in
`dashboard_server::tests`:

- `the_index_route_does_not_hand_the_token_to_an_unauthenticated_caller`
- `the_token_in_the_query_becomes_an_httponly_cookie`
- `the_cookie_holder_gets_the_page_with_the_token`
- `a_request_with_a_foreign_host_is_refused`
- `the_host_check_accepts_the_operators_own_addresses`
- `an_ip_literal_host_is_allowed_even_when_it_is_not_the_bind_address`
- `a_request_without_a_host_header_is_refused`
- `the_api_routes_refuse_an_unauthenticated_caller`

Plus three that exercise `check_auth` directly, because the 200-path
handlers read the SQLite database SYNCHRONOUSLY and a test doing blocking
socket I/O on the same runtime exhausts the worker pool and deadlocks:

- `the_api_accepts_the_header_the_frontend_bundle_sends`
- `the_api_accepts_the_dashboard_cookie`
- `the_api_refuses_a_caller_with_no_token`

Mutations, all caught: serving the token to anyone fails the first;
disabling the Host guard fails the fourth; not reading the bundle's header
fails `the_api_accepts_the_header_the_frontend_bundle_sends`.

The tests need `flavor = "multi_thread"`. They do BLOCKING socket I/O on
the test's own thread, and the default current-thread runtime then has no
thread left to run the server on — a deadlock, not a slow test. The first
attempt hung and was killed.

## Consequences

- The dashboard's auth is now the thing it claimed to be. A caller that
  cannot present the token learns nothing, and a caller that can present
  it is the operator's own browser.
- A DNS-rebinding page cannot reach the dashboard at all: the `Host` it
  supplies is not on the allowlist.
- The CLI opens the browser at a URL that carries the token, and the
  operator may see it in the address bar for the moment before the
  redirect. That is the one place the token is visible outside the
  operator's own machine.

## Open

- The token is still injected into the page HTML, because the prebuilt
  frontend bundle reads it from there. A bundle that took the token from
  the cookie instead would remove even that. Rebuilding the bundle is a
  separate piece of work.
- The `/api/health` route is unauthenticated by design (a health check
  needs no credential). It returns liveness and platform counts, not
  secrets.

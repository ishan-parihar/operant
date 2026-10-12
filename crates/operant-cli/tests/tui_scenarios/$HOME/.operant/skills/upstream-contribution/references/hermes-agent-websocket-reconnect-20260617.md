# Case Study: hermes-agent WebSocket Auto-Reconnect

## Problem

Dashboard WebSocket connections (JSON-RPC sidecar, events feed, PTY chat)
had no auto-reconnect. Gateway restart (after `hermes update`) severed all
connections. Only recovery: manual "reconnect" button or hard page refresh.

## Files Changed

- `web/src/lib/gatewayClient.ts` — Core reconnect logic (+117/-5)
- `web/src/components/ChatSidebar.tsx` — Events feed reconnect + session re-creation (+237/-105)

## Key Design Decisions

1. **Exponential backoff**: 1s → 2s → 4s → 8s → 16s → 30s cap, max 15 attempts
2. **Auth re-minting**: Single-use tickets (TTL=30s) must be re-minted on each attempt
3. **Generation counter**: Events feed uses `activeWsGeneration` to invalidate stale reconnects
4. **Explicit close = no reconnect**: `close()` sets `_explicitlyClosed` flag
5. **Auth rejection (4401/4403/4408) doesn't retry**: User must reload
6. **Sidecar session re-creation**: After reconnect, `session.create` fires automatically

## Verification

- `tsc -b && vite build` — zero errors
- Dashboard restarted, serving new bundle
- WebSocket endpoint responding (400 = non-WS HTTP request, correct)

## PR

https://github.com/NousResearch/hermes-agent/pull/47921
Branch: `<contributor>:fix/websocket-auto-reconnect` → `NousResearch:main`
State: OPEN

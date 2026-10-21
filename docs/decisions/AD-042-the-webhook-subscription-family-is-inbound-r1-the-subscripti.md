---
ID: "AD-042"
Title: "The webhook subscription family is INBOUND (R1): the subscription claims a route, the agent answers, and the answer is delivered to the target"
Status: "Active"
Date: "2026-10-21"
Description: "The webhook subscription family is INBOUND (R1): the subscription claims a route, the agent answers, and the answer is delivered to the target"
---

# AD-042: The webhook subscription family is INBOUND (R1): the subscription claims a route, the agent answers, and the answer is delivered to the target

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`operant webhook subscribe` recorded a subscription and NOTHING dispatched to
it (RT-140). The ledger row spelled out two candidate models and said the
choice was a product decision, not an implementation detail:

- **R2 (outbound).** The subscription is a destination WE post to when
  something happens. Evidence: `test` posts TO the url; the `secret` is a
  signing secret for OUR request. Open questions: which events (the CLI's
  default is "message"), and what triggers them.
- **R1 (inbound).** The subscription describes a route the EXTERNAL service
  posts to, with `prompt`/`skills` shaping how the agent handles the event
  and `deliver`/`deliver_chat_id` saying where the answer goes. Evidence:
  `deliver_only` ("don't process via agent") and `deliver_chat_id` only make
  sense here. Open question: the subscription was keyed by an absolute URL
  while the adapter's routes are path segments.

The owner delegated the choice ("choose recommended best decisions on your
own"), so the decision was made from the data model rather than by asking.

## Decision

**R1 (inbound).** The subscription claims a ROUTE; the external service posts
to it; the agent handles the event; the answer is delivered to `deliver`.

The fields decide it:

1. `deliver`, `deliver_chat_id` and `deliver_only` only mean anything when WE
   receive the event and answer it somewhere. In R2 there is no answer to
   deliver, and "deliver only, don't process via agent" has no referent.
2. `prompt` and `skills` shape how the AGENT handles the event. In R2 the
   agent is not involved — a payload field is not a prompt.
3. The trigger question dissolves in R1: the trigger is an HTTP POST, which
   the inbound adapter already receives. In R2 the trigger would have to be
   an event bus for events operant has no general emitter for.
4. R2 would make operant a relay — every inbound platform message POSTed to
   every subscribed URL. Operant is an agent that answers on messaging
   platforms; R1 is the shape that fits the product.

**The route key.** The subscription was keyed by an absolute URL while the
adapter's routes are path segments. The route is now the LAST PATH SEGMENT of
the URL (`https://example.com/webhook/orders` claims `orders`), with
`--route` to name one explicitly. A segment that is not a plain path token
claims nothing, and the operator then names the route.

**Per-route secrets.** A subscription that carries a secret is verified
against THAT secret, so two routes can use different ones; without one, the
adapter's global `[gateway] webhooks_secret` applies.

**Where the answer goes.** The inbound channel cannot carry a reply — the
webhook adapter is inbound-only — so the agent's answer was dropped on the
floor: the event was handled and the operator never learned the outcome.
`deliver_to` on the message moves the answer to the platform the operator
named, and the delivery ledger records it there.

**The admin gate.** The gate is a USER gate. A signature-verified webhook
event has no user, so it can never be an admin — but refusing it made the
webhook route dead in every deployment that configures `admins`, which is
every real deployment. An AUTHENTICATED event bypasses the gate; an
unauthenticated one is still refused when admins are configured.

## Consequences

- `operant webhook subscribe <url>` dispatches: the URL's last segment is
  the route, and the CLI prints the route, the answer's destination, and
  says plainly when no destination was named.
- A subscription with no route is still recorded and still dispatches
  nothing — the CLI says so instead of implying it is live.
- The event name is the payload's `event` (or `type`) field; a payload that
  names none is a "message" event, which is what the CLI subscribes to by
  default. A subscription that does not list the event does not apply to it.
- `deliver_only` delivers the EVENT body without running the agent, and
  without the agent's prompt wrapped around it.
- The subscription's `skills` are named to the agent in the event (the
  agent reads them with the skills tool); they are not a hard filter on
  which skills are loaded, because the agent's skill set is attached when
  the agent is built and the gateway builds it once.

## Evidence

- `crates/operant-core/src/gateway/webhook_subscriptions.rs`: the store, the
  route derivation, the delivery target, the event filter, the prompt
  composition.
- `crates/operant-core/src/gateway/webhook.rs`: the dispatch — the
  subscription lookup, the per-route secret, the event shaping.
- `crates/operant-core/src/gateway/mod.rs`: the admin-gate exemption and the
  delivery-target override.
- `crates/operant-cli/src/gateway_runner.rs`: the deliver-only short
  circuit and the send to the named platform.
- Tests: `webhook::tests::a_subscription_route_dispatches_the_event` (the
  production path: a real HTTP POST through the adapter, the real store),
  `gateway::tests::an_authenticated_event_is_not_refused_by_the_admin_gate`,
  `gateway::tests::a_delivery_target_moves_the_answer_to_the_named_platform`,
  and the four unit tests in `webhook_subscriptions::tests`.

## Orthogonality

RT-140 records the defect and its fix; the durable decision is WHICH model the family implements (R1 inbound vs R2 outbound) - a product choice whose reasoning a future reader needs, and whose rejection of R2 is not recoverable from the code (recon R102108524400e: webhook subscription dispatch inbound route delivery)

## Revisions

- 2026-10-21: created

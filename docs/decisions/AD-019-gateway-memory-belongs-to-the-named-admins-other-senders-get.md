---
ID: "AD-019"
Title: "Gateway memory belongs to the named admins; other senders get their own bank"
Status: "Active"
Date: "2026-10-19"
Description: "A gateway turn is retained, and recalls the personal bank, only for a sender named in gateway.admins; any other unbound sender runs in a bank of its own"
---

# AD-019: Gateway memory belongs to the named admins; other senders get their own bank

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

Two gaps in the gateway's memory (BUGS KB-6, KB-8). A gateway turn from any sender was retained into the owner's personal bank, and an unbound turn recalled that bank. `gateway.admins` is the only sender gate, and an empty list admits every sender (`gateway_runner.rs`, the admin check). The shipped default is `admins = []` (`config.rs`, `operant.example.toml`), so the default configuration exposed the personal bank to anyone who could reach the bot.

## Decision

1. A gateway turn is retained only when its sender is in `gateway.admins`. An empty list retains nothing from the gateway.
2. An unbound sender who is not in `gateway.admins` runs in `MemoryScope::Sender`, keyed by platform and user id. Its bank is `sender-` plus the first 16 bytes of a SHA-256 digest, so no two senders share a bank. A named admin keeps `MemoryScope::Personal`.
3. The scope is set once per turn, before the session lookup, so a sender with no session row never inherits the previous turn's scope. A bound seat overrides it.

## Consequences

- Behaviour change: with `gateway.admins` empty, gateway chats no longer read or write the owner's personal memory, and nothing said in a gateway chat is retained. Add your user id to `gateway.admins` to restore both.
- A stranger's memories accumulate in their own bank. No cleanup command exists for those banks; that is a follow-up, not part of this decision.
- Tests: `only_a_named_admin_reaches_the_personal_bank` (`gateway_runner.rs`; red when the helper returns `Personal` for everyone) and `only_a_named_admin_has_its_turns_retained`. The wiring of the scope in `run_turn` is checked by reading, not by a gateway test.

## Orthogonality

AD-010 keeps a chat's conversation on its session row and starts a fresh one on a seat change. This decision completes the memory half of GW-3 for unbound chats. AD-006 and AD-007 (Holon fences) are not touched.

## Revisions

- 2026-10-19: created


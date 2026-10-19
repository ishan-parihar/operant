---
ID: "AD-010"
Title: "A chat's conversation is kept on its row; /new and a seat change start a fresh one"
Status: "Active"
Date: "2026-10-19"
Description: "The conversation id persists on the session row, /new and a seat change start a fresh transcript, the continuation summary is keyed by it, and seat runs set their memory scope"
---

# AD-010: A chat's conversation is kept on its row; /new and a seat change start a fresh one

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

A chat's transcript is keyed by an agent session id. That id was either a `/resume` pin held in memory or a hash of the chat key. Four defects followed from that:

- **GW-5:** `/new` closed the store row by the conversation id, which matches no row. The turn kept the same hash, so the same transcript and the same summary came back.
- **GW-6:** a seat change kept the transcript. A new employee's turn ran with the old employee's full history. The `/resume` pin was also memory-only, so a restart fell back to the hash transcript.
- **GW-7:** the continuation summary was keyed by chat, so a `/resume` onto another transcript injected the previous roll.
- **GW-2 and GW-9:** cron and socialization runs never set a memory scope, and `/session` named `premiere` as the seat of an unbound chat, although an unbound session is ungoverned (G+1).

## Decision

1. A chat's conversation id is stored in `gateway_sessions.conversation_id`. NULL means the hash. The turn reads the in-memory pin first, then the row, then the hash.
2. `/new` closes the row by its session key, creates a fresh row, and starts a fresh conversation (a `gw_` id). The reply names that id, which is what `/resume` takes.
3. A seat change to a different employee, including the first bind, starts a fresh conversation. The old transcript stays stored under its id and is reachable with `/resume <id>`. A transcript is the context a seat has run with, so one seat does not inherit another's history by default.
4. `/resume` writes the id to the row as well as the pin, so a restart keeps it. A chat with no row yet keeps the pin only, and that pin is lost on restart.
5. The continuation summary is keyed by the conversation id. Existing summary rows were keyed by chat and no longer match, so the rolling summary is dropped once on upgrade.
6. `bind_seat_run`, the shared preamble for cron and socialization runs, sets `MemoryScope::Seat` to the seat.
7. `/session` with no argument reports an unbound chat as unbound and ungoverned. It never names the premiere.

## Consequences

- A user who wants the old context after `/new` or a rebind runs `/resume <id>` with the id the reply printed.
- A chat's history no longer reaches a new seat without an explicit `/resume`.
- The runner-side conversation read and the summary keys have no unit seam. They are covered by the store tests `a_conversation_id_survives_a_save_of_its_row` and by reading.
- Tests: `a_conversation_id_survives_a_save_of_its_row` (store), `a_seat_run_sets_its_own_seat_as_the_memory_scope` (cron), `an_unbound_chat_is_never_reported_as_the_premiere` (commands).

## Orthogonality

AD-009 covers the spend ledger that follows the seat. AD-004 covers the binding check. The conversation rule here does not change which seat a turn is governed by.

## Revisions

- 2026-10-19: created

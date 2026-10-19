---
ID: "AD-028"
Title: "A new org-mode chat starts unbound; no default employee is set"
Status: "Active"
Date: "2026-10-19"
Description: "New org-mode chats start unbound, with no default employee, which is the current behaviour and the recommended option for the seat-default question"
---

# AD-028: A new org-mode chat starts unbound; no default employee is set

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The TODO Review list asked for the default employee of a new org-mode chat. The options were a `[holon] default_employee` roster id, or leaving new chats unbound. `premiere` is not on the roster, so AD-004 rules it out.

## Decision

New chats stay unbound (option c). `default_employee_binding()` in `operant-core/src/gateway_session.rs` returns `None`, so a new session has no employee. The GW-9 change touches only the `/session` report, which fits this option.

Recorded so the default is a decision and not an accident. No roster id was added.

## Consequences

- Nothing binds a new chat to a seat until the user binds it.
- The comment on `default_employee_binding` says an unbound session is ungoverned. This record does not change what an unbound session may do. That needs its own review, and the owner should read it; it is flagged in the final report.

## Orthogonality

AD-004 (org employee validation) and AD-010 (the conversation is kept on its session row) apply to bound chats.

## Revisions

- 2026-10-19: created


---
ID: "AD-008"
Title: "A chair notice is retried until delivered; driver tokens are plain"
Status: "Active"
Date: "2026-10-19"
Description: "The Holon kernel retries a failed chair notice on the next heartbeat, and refuses driver placeholders that are not plain tokens"
---

# AD-008: A chair notice is retried until delivered; driver tokens are plain

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

This decision is implemented in the Holon repository (commit `992b08f`, local, not pushed) and recorded here because operant's `chair notify` exit code depends on it.

Two defects in the Holon kernel:

- **OS-5:** `Kernel.notify()` advanced the `notify_seq` cursor before running the notify command. A failed send therefore marked its items seen, and the chair never heard about them.
- **OS-11:** `drivers.render()` interpolated `employee_slug` and `scope` into the `sh -c` command unquoted, because the driver templates embed them inside larger tokens (alias names). Both come from operator-written data (the roster and `holon.toml`).

## Decision

1. The cursor moves past an item only when nothing is owed, or when the message was delivered. A failed send leaves its items owed, and the next heartbeat retries them.
2. `employee_slug` and `scope` must match `[A-Za-z0-9_][A-Za-z0-9_.-]*`, or `render()` raises `DriverError`. The other placeholders are still shell-quoted, as before.

## Consequences

- A notify command that keeps failing is retried on every heartbeat, and each attempt writes a `notify` event with status `failed`. That is noisier than before, and it is the price of not losing a notice.
- A roster slug or scope name with any other character now fails to render with a visible error, instead of reaching the shell.
- Tests: `test_a_failed_delivery_is_owed_again_on_the_next_heartbeat` (`tests/test_loops.py`) and `test_a_driver_refuses_a_scope_name_that_is_not_a_plain_token` (`tests/test_scope.py`). The full Holon suite passes (374).

## Orthogonality

The operant side of OS-5 (exit nonzero when no chair was reached) was decided in iter-963. AD-006 covers the body-side fence.

## Revisions

- 2026-10-19: created

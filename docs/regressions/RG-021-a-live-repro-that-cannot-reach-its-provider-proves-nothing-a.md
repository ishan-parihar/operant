---
ID: "RG-021"
Title: "a live repro that cannot reach its provider proves nothing about the feature"
Status: "Active"
Date: "2026-11-01"
Description: "a live repro that cannot reach its provider proves nothing about the feature"
Severity: "High"
---

# RG-021: a live repro that cannot reach its provider proves nothing about the feature

Immutable once numbered: a correction is a new record that supersedes this one.

## Symptom

The rewind selector's arrow keys were reported as not navigating three times in a row, from a tree whose implementation was in fact correct. Each attempt produced a convincing screenshot of a frozen selector from a feature that works, and roughly ninety minutes went into hunting a defect that did not exist.

## Root cause

The harness layer, not the feature. Each attempt took a different route to the same wrong answer: the first never reached the provider at all, the second reached it and the request died in transit, and the third was answered by a process that was not the one being tested. The three traps, in the order they were hit:

1. **The ambient proxy owns every 127.x request.** `HTTP_PROXY=http://127.0.0.1:43117` is set in the shell and the Rust reqwest stack in this build does not honour `NO_PROXY`, so a mock bound to 127.0.0.1 is never contacted — the proxy answers instead (here with `{"error":"upgrade_required","Use WebSocket."}`, indistinguishable from a provider error at the TUI surface). `NO_PROXY` is not consulted by the client, so setting it changes nothing. The mock must bind a loopback alias the proxy does not watch (`127.0.0.2`) AND the proxy vars must be `unset` for the child process.
2. **Port 20132 is not free.** A foreign service, reported by `ss -tlnp` as `omniroute (v16...)`, owns `127.0.0.1:20132`. A mock that "is listening" is answering from someone else's socket. `ss -tln | grep <port>` showing LISTEN is not evidence the mock holds it — read the `users:` column.
3. **The shell is not the TUI.** After one Ctrl+C too many the process exits and the pane falls back to bash, where every subsequent "prompt" and "arrow key" is typed into the composer of a shell that never reads them. `pane_current_command` must read `operant` before any keystroke is meaningful. The F12 debug overlay compounds this: it is the highest-priority key consumer and swallows everything until toggled back off.

A fourth, subtler trap: a stale process from an earlier attempt keeps serving the old `base_url`. Restarting the binary in a new tmux session does not prove the new session is being observed.

## Law

> **A live repro names its own provider, port and owning process, or it is not a live repro.** Before any keystroke is meaningful: `ss -tlnp` shows the mock's pid on the intended address, the pane's current command is the binary under test, and the provider has logged at least one request. A repro whose provider received nothing is evidence about the harness, never about the feature — and the honest move is to fix the harness or fall back to the deterministic suite, not to re-open a fix that is already landed.

The corollary is the one that saves the most time: when a repro contradicts a passing test suite, suspect the repro first. Three of the four traps above produce a *convincing* screenshot of a feature doing nothing, from a feature that works.

## Detection

Before any keystroke is credited: `ss -tlnp` shows the mock's pid on the intended address (not merely LISTEN), `tmux list-panes -F '#{pane_current_command}'` reads `operant`, and the mock's own log records at least one request. A live repro whose provider log is empty is evidence about the harness — fix the harness or fall back to the suite, and never re-open a fix that is already landed and pinned by a passing test.

## Orthogonality

Existing records cover sync_marks cursor resets and RT-205's duplicate-final-character rendering. None covers the harness layer: three separate live-repro attempts reported this same rewind defect as still present when the feature was in fact correct, because the provider never received a request. That failure mode has its own law and is not derivable from any of them. (recon R11011836527be: live rewind selector mock provider)

## Revisions

- 2026-11-01: created

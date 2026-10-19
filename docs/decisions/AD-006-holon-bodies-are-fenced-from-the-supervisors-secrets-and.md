---
ID: "AD-006"
Title: "Holon bodies are fenced from the supervisor's secrets and their slot"
Status: "Active"
Date: "2026-10-19"
Description: "A Holon body cannot read the supervisor's environment, cannot inherit chair authority, and dies with its slot"
---

# AD-006: Holon bodies are fenced from the supervisor's secrets and their slot

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

A body is a same-uid child of the supervisor. The Holon red-team found five paths out of its slot:

- **OS-1:** the body reads `/proc/$PPID/environ`, which is the supervisor's environment (channel tokens, provider keys).
- **OS-2:** `HOME` is in the body's baseline, so `~/.config/holon/keys/<instance>.chair` is reachable. Holon's `principal()` falls back to that file when no run token is set, so a body that unsets `HOLON_RUN_TOKEN` can run `holon run finish` as chair.
- **OS-6:** Ctrl-C or SIGTERM on `supervise` killed the supervisor and not its body's process group. The body ran until its own timeout while its lease expired and the kernel re-claimed the stage, so the same stage could run twice.
- **OS-7:** every non-`running` run status was treated as a clean self-submit.
- **OS-10:** the timeout kill sent SIGTERM to the group and SIGKILL only to the leader, so a grandchild that traps TERM survived.

## Decision

1. The supervisor clears `PR_SET_DUMPABLE` before every body spawn (Linux). `exec` resets the flag, so each body stays dumpable; only the supervisor's `/proc` entries close.
2. Every body environment pins `HOLON_CHAIR_KEY` to a constant that no chair digest matches. Holon reads that override before the key file, so the CLI's implicit chair lookup resolves to `anonymous`.
3. A body's process group is SIGKILLed when the body's future is dropped. `supervise` races its whole loop against SIGINT, SIGTERM and SIGHUP, so a signalled supervisor takes its body with it at once instead of waiting for lease expiry.
4. The run's kernel status decides the slot (`closed_by`). `succeeded` and `failed` are a self-submit and a clean stop. `running` is open. Any other status (`timeout`, `abandoned`, or one this code does not know) is a reclaim: the body is killed and `finish` is skipped, because the kernel already closed the run.
5. A kill of a body's group is SIGTERM, then a 5 s grace (`KILL_GRACE`), then SIGKILL to the group.

## Consequences

- **Residual, not closed here:** a same-uid body can still `cat` the operator-owned chair key file explicitly. Closing that needs act bodies under a separate uid, which is the O-11 isolation work owned by the Rust runner. Until it lands, an act body is trusted with the operator's files.
- Off Linux there is no dumpable flag. The supervisor logs a warning for each body it spawns, and the environment stays readable there.
- A signalled supervisor kills its body with SIGKILL, so the body loses in-flight work. The run then closes by lease expiry as `timeout`.
- A SIGKILLed supervisor cannot run any of this. Its body keeps running until the body's own timeout, and the kernel may re-claim the stage in the meantime. This decision does not cover that case.
- Heartbeat failure still only warns. The status poll (every 15 s) detects a reclaim, so one failed heartbeat does not kill a live body.

## Orthogonality

AD-007 covers the fail-closed policy check, and AD-008 covers the Holon-side notify and driver fixes. The observe-posture denial of `holon_skill_invoke` (OS-4) was decided in iter-963 and is not changed here.

## Revisions

- 2026-10-19: created

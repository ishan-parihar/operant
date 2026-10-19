---
ID: "AD-017"
Title: "ONNX Runtime is pinned to 1.24.4, and install refuses a library without API 24"
Status: "Active"
Date: "2026-10-19"
Description: "The build and install scripts use ONNX Runtime 1.24.4, and install.sh refuses a runtime that does not serve API 24"
---

# AD-017: ONNX Runtime is pinned to 1.24.4, and install refuses a library without API 24

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

The `ort` crate needs ONNX Runtime API 24. The scripts pinned 1.20.1, which serves API 17 and 20 and returns NULL for 21, 23 and 24. The release built and installed with exit 0, so the mismatch appeared only when TTS started (BUGS CR-2).

## Decision

1. ONNX Runtime 1.24.4 is the pinned version. It is provisioned under `local/onnxruntime-linux-x64-1.24.4/` (git-ignored) by `scripts/provision-build-deps.sh`. The pins in `scripts/dev-env.sh`, `scripts/check.sh`, `scripts/setup-build-env.sh`, `scripts/install.sh` and AGENTS.md move with it. The 1.21.0 entry is removed, because it does not serve API 24 either.
2. `install.sh` probes the library before it copies it: `GetApi(24)` must return a non-NULL pointer, or the copy is skipped with a warning.

## Consequences

- Probed on this machine: 1.24.4 serves API 17, 20, 21, 23 and 24 and returns NULL for 25; the install probe accepts it and refuses 1.20.1.
- The installed copy under `~/.local/lib/operant` is still 1.20.1 until the deploy copies the 1.24.4 libraries over it. The deploy step does that.
- The probe needs `python3`. Without it, `install.sh` refuses the copy and says so, which is the safe direction.
- `skills/remote-build-ssh/SKILL.md` carries the same pin and now matches.

## Orthogonality

AD-016 makes a runtime mismatch a tool error. This record removes the mismatch on the supported install path.

## Revisions

- 2026-10-19: created

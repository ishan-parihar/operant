---
ID: "AD-018"
Title: "The doctor reports tool availability from configuration, flags duplicate binaries, and loads the config once"
Status: "Active"
Date: "2026-10-19"
Description: "operant doctor marks a tool available only through its configuration or a present env var, reports TTS as unverified, flags a second operant on PATH, and loads the config once"
---

# AD-018: The doctor reports tool availability from configuration, flags duplicate binaries, and loads the config once

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

Three doctor and startup defects came from one pattern. A row with no env list counted as available whatever its config (CR-4). Two operant binaries on PATH were not detected, so a user could launch the copy the deploy did not update (CR-6). The config loaded twice, and each load-time warning printed twice (CR-5).

## Decision

1. A tool row is available when it is configured or one of its env vars is set. An empty env list no longer makes a row available by itself.
2. TTS is reported as `configured — the ONNX Runtime is checked on first use` when enabled, and as not configured otherwise. The doctor does not load the engine, because that loads the model; so a configured TTS is never reported as proven available.
3. `check_operant_binaries` lists the distinct `operant` executables on PATH and reports more than one as a manual issue. Two names that resolve to one file count once.
4. `main` loads the config once and passes the result to `run`. `apply_startup_proxy` takes the loaded config.

## Consequences

- `vision` with no provider now shows as not configured, not available. That is the true state, and it adds the setup advisory to the summary when vision is unset.
- The duplicate-binary warning is a manual issue, not an automatic fix: removing the root-owned `/usr/local/bin/operant` needs the owner's sudo. This record does not remove it.
- Verified: the dev `doctor` prints each `[memory]` warning once, where the installed binary printed each twice. Tests: `two_copies_are_found_and_a_repeated_directory_is_counted_once` (`crates/operant-cli/src/cmd_doctor/checks_tools.rs`), mutation fails it.

## Orthogonality

AD-016 covers the TTS engine's panic. AD-017 covers the runtime it loads.

## Revisions

- 2026-10-19: created

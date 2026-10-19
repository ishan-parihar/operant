---
ID: "AD-016"
Title: "A TTS engine start that panics is a tool error, and the release profile unwinds"
Status: "Active"
Date: "2026-10-19"
Description: "The Kokoro engine starts on its own task, so an ONNX Runtime panic becomes a tool error, and the release profile unwinds instead of aborting"
---

# AD-016: A TTS engine start that panics is a tool error, and the release profile unwinds

Immutable once numbered: a correction is a new record that supersedes this one.

## Context

`ort` panics when the installed ONNX Runtime does not serve the API version it was built against. The panic came from `TtsEngine::new` (`kokoro-tiny` 0.1.0 pulls `ort` with its default `api-24` feature), and `[profile.release] panic = "abort"` turned it into SIGABRT, so a `text_to_speech` call without an explicit provider ended the gateway or the TUI (BUGS CR-1, CR-3).

## Decision

1. `start_kokoro()` runs `TtsEngine::new()` on a spawned task. A panic there is a `JoinError`, which `join_engine_start` turns into a tool error that names the ONNX Runtime API. The message is `Failed to initialize Kokoro engine: …`, the same shape as an ordinary init failure.
2. `[profile.release]` sets `panic = "unwind"`. Under `abort` no panic can be caught, so the task boundary would not help.

## Consequences

- The release binary is larger and the unwind tables are kept. Build time is unchanged.
- A panic in another optional subsystem now unwinds to its own task boundary instead of ending the process. Code that relied on abort-on-panic would change. That reliance was not searched for.
- The unwind path was verified by `a_panicking_engine_start_is_a_tool_error` (`crates/operant-core/src/tools/tts_tool.rs`). Its mutation (the panic arm stops matching) fails the test. Whether a real kokoro init on the installed runtime now returns the error was not run; it needs the model files.

## Orthogonality

AD-017 removes the mismatch itself by pinning a runtime that serves API 24. This record makes a mismatch, if it happens again, a tool error rather than a crash.

## Revisions

- 2026-10-19: created

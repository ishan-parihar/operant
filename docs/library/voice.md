# Voice — TTS and STT as agent tools

Owns: operant's speech surfaces — the `text_to_speech` and `transcription`
tools and their provider matrices. 
Not for: the tools' schemas ([tools.md](tools.md)), provider auth
([providers.md](providers.md)).

Source: `crates/operant-core/src/tools/tts_tool.rs`, `tts_provider.rs`,
`tts_registry.rs`, `tts_command_provider.rs`,
`crates/operant-core/src/tools/transcription_tool.rs`,
`crates/operant-cli/src/tui/operant_model/transcription.rs` (TUI wiring).

## What this page no longer covers, and why

The interactive voice-mode state machine (`voice.rs`: Idle → Listening →
Processing → Speaking, FFmpeg/Termux recorders, energy-based VAD, the
Whisper hallucination filter) was **deleted in iter-1420**
(`docs/audit/2026-10-27-ponytail-audit-memory-compat-and-dead-code.md`):
1,516 lines, 25 pub fns, and a module-qualified call-path scan found
**zero callers** — the state machine was never wired into the CLI, the
gateway, or the TUI. The same sweep deleted `retrieval.rs` (1,068 lines,
zero qualified callers). If push-to-talk voice mode is ever wanted, it is
a NEW build against a fresh design, not a restore — the audit record is
the honest account of what existed and why it died.

## STT: the transcription tool (groq + openai)

`transcription_tool.rs` speaks two providers — `groq`
(`whisper-large-v3-turbo` default) and `openai` (`whisper-1` default),
keys from `[tools.stt]`. The per-provider model knobs exist because a
hard-coded model name on a provider that retires it 404s with no way
out short of a new build (`groq_model` / `openai_model`).

## TTS providers (11 built-in + custom commands + plugins)

The `text_to_speech` tool's dispatch (tts_tool.rs): `edge` (free,
keyless), `elevenlabs`, `openai`, `minimax`, `mistral`, `gemini`, `xai`,
`neutts`, `kittentts`, `piper` (local), `kokoro` (local, ONNX) — plus
custom shell commands (`tts_command_provider.rs`) and plugin providers
via the registry (`tts_registry.rs`).

**Kokoro is the configured default** (`[tts] provider = "kokoro"` in
`operant.example.toml`): the engine loads lazily via `kokoro-tiny`
(TtsEngine), default voice `af_sky`, backed by the repo-provisioned
ONNX Runtime. This is an operator design choice — do not switch the
default away from Kokoro.

## The registry pattern

`tts_registry.rs` holds `Arc<dyn TtsProvider>` behind a RwLock:
plugin-declared providers register alongside the built-ins, and the
tool resolves by name at call time. The trait surface
(`tts_provider.rs`: `SynthesisResult`, `AudioFormat`, `TtsError`) is the
contract every provider implements.

## Intention

The voice surface is deliberately tool-shaped: `text_to_speech` and
`transcription` are model-callable TOOLS (the agent speaks/listens
inside a turn), resolved through the provider registry at call time.
The dead state machine was the opposite bet — interactive primitives
waiting for a caller that never came; iter-1420 closed it honestly
instead of letting the library keep pretending it was reachable.

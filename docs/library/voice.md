# Voice — TTS, STT, and the voice-mode state machine

Owns: operant's speech surfaces — the voice-mode state machine, recording,
the STT and TTS provider matrices, VAD, and the hallucination filter.
Not for: the TTS tool's schema ([tools.md](tools.md)), provider auth
([providers.md](providers.md)).

Source: `crates/operant-core/src/voice.rs` (the library core),
`crates/operant-core/src/tools/tts_tool.rs`, `tts_provider.rs`,
`tts_registry.rs`, `tts_command_provider.rs`,
`crates/operant-core/src/tools/transcription_tool.rs`,
`crates/operant-cli/src/tui/operant_model/transcription.rs` (TUI wiring).

## The voice-mode state machine (`voice.rs`)

Voice mode is a library, not a tool: composable primitives for the CLI
and gateway. The state machine runs Idle → Listening → Processing →
Speaking, with:

- **Recording** via subprocess (ffmpeg/arecord) or Termux:API
  (`FFmpegRecorder`, `TermuxRecorder`), push-to-talk.
- **Voice Activity Detection** — energy-based RMS threshold with a
  silence callback, so recording can stop itself when the user stops
  talking.
- **STT dispatch** — transcription through the configured provider.
- **Whisper hallucination filter** — Whisper-family models hallucinate
  text on silence; the filter drops the known phantom outputs.
- **TTS synthesis** — the spoken reply.

## STT providers (6 + local)

`SttProvider::parse_provider` (voice.rs): `whisper` (also `openai`),
`google`, `azure`, `assemblyai`, `deepgram`, `local` — unknown names
default to Whisper. Config: `[tools.stt]`.

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

The voice surface is deliberately split: `voice.rs` is the interactive
state machine (the thing a human talks to), while `text_to_speech` /
`transcription` are model-callable TOOLS (the thing the agent uses to
speak/listen inside a turn). Both share the provider matrices. The VAD +
hallucination filter exist because push-to-talk without them produces
phantom transcripts — a known Whisper-family behavior that once produced
confident nonsense from silence.

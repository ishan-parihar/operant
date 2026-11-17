# Voice — TTS and STT as agent tools

Owns: operant's speech surfaces — the `text_to_speech` tool and its
provider dispatch, the `transcribe_audio` tool, the provider/extension
registries, and how synthesized audio reaches a channel.
Not for: the tools' schemas and the registry that carries them
([tools.md](tools.md)), provider auth and the credential files
([providers.md](providers.md)), the gateway adapters and their session
store ([gateway.md](gateway.md)), the model message path
([agent-loop.md](agent-loop.md)).

Source: `crates/operant-core/src/tools/tts_tool.rs` (dispatch + the
built-ins), `crates/operant-core/src/tools/tts_provider.rs` (the trait and
error type), `crates/operant-core/src/tools/tts_registry.rs`
(`TtsPluginRegistry`), `crates/operant-core/src/tools/tts_command_provider.rs`
(`CommandProvider`), `crates/operant-core/src/tools/transcription_tool.rs`
(STT), `crates/operant-cli/src/tui/operant_model/transcription.rs`
(the `<transcription>` wrapper),
`crates/operant-cli/src/gateway_runner/enrichment.rs` (Telegram voice
download + transcribe), `crates/operant-cli/src/gateway_runner.rs`
(audio delivery), `crates/operant-api/src/vad.rs` (the VAD seam),
`crates/operant-core/src/config.rs` (`[tts]`, `[tools.stt]`).

## The shape of the surface

Voice in operant is **tool-shaped**, not an interactive mode. The agent
calls `text_to_speech` and `transcribe_audio` inside a turn like any
other tool; there is no push-to-talk recorder, no VAD-driven state
machine and no interactive voice mode. The legacy interactive surface
(the FFmpeg/Termux recorders, energy-based VAD, the Whisper
hallucination filter, the Idle → Listening → Processing → Speaking state
machine, the `/voice` recapture chord) was purged wholesale in iter-668
and iter-1420 — roughly 1,500 lines with, on the module-qualified
call-path scan, **zero live callers**. The keybinding catalogue entry went
with it. If a hold-to-talk mode is ever wanted it is a NEW build against
a fresh design, not a restore.

## `text_to_speech` — the dispatch

`TtsTool::generate_speech` resolves the provider in three tiers, then
dispatches:

1. **Command providers** (a `HashMap<String, CommandProvider>` on the
   tool) win over everything, because config is more local than a plugin
   install.
2. **Plugin providers** from `TtsPluginRegistry`, reached when the name
   is not a built-in (`TtsPluginRegistry::is_builtin`).
3. **The eleven built-in match arms.**

The tool's registered name is `text_to_speech` (`TtsTool::name()`), and
its two arguments carry the config defaults: `provider` defaults to the
value of `default_provider()` (`"phonon"`) and `voice` to
`default_voice()` (`"default"`). `execute` then applies the override
rule: a caller that left the provider at its default adopts
`[tts] provider`, and a caller that left the voice at the sentinel adopts
`[tts] voice` when that key is set and non-empty.

The dispatch (tts_tool.rs): `edge` (keyless, via the `edge-tts` CLI — but
not local; `pip install edge-tts`), `elevenlabs`,
`openai`, `minimax`, `mistral`, `gemini`, `xai`, `neutts`, `kittentts`,
`piper` (local binaries/Python) and `phonon` (local, GGUF, default) — plus
custom shell commands (`tts_command_provider.rs`) and plugin providers
via the registry (`tts_registry.rs`).

`edge` was a stub until iter-2546: it returned `success: true` with a message
about the Python dependency and **no audio**. It now shells out to the
`edge-tts` CLI exactly the way `piper_local` does — write a temp file, read
the bytes, base64-encode them, delete. A zero-length or absent output file is
an ERROR, because `edge-tts` exits 0 on empty input and leaves an empty file.
`no_builtin_provider_reports_success_without_audio` is the invariant guard.

**`voice` is optional and its sentinel is provider-scoped.** The schema default
is `"default"`, a real voice name ONLY for Phonon; all ten other providers
validate the voice id and reject it (`edge-tts`: `ValueError: Invalid voice
'default'.`). `resolve_voice` maps the sentinel (or an empty string) to the
provider's own default, so an omitted `voice` works everywhere.

**Phonon is the configured default** (`[tts] provider = "phonon"` in
`operant.example.toml`): the engine loads lazily via `ptts`
(`Synth::builder`), default voice `default`, backed by a revision-pinned
GGUF checkpoint (`gradium/phonon-31m-en`, ~65 MB q8_0) cached under
`~/.operant/cache/phonon/<repo>-<rev>/` (the one home owns it; a
pre-iter-2524 `~/.cache/phonon/gradium/phonon-31m-en` tree is migrated on
first use). It replaced Kokoro (ONNX Runtime, `af_sky`) in iter-2513. This is
an operator design choice — do not switch the default away from Phonon.

| provider | key / dependency | default voice | default model | output |
|---|---|---|---|---|
| `edge` | free, keyless | `en-US-AriaNeural` | — | mp3 |
| `elevenlabs` | `ELEVENLABS_API_KEY` | `pNInz6obpgDQGcFmaJgB` | `eleven_multilingual_v2` | mp3 |
| `openai` | `OPENAI_API_KEY` | `alloy` | `gpt-4o-mini-tts` | audio bytes |
| `minimax` | `MINIMAX_API_KEY` | — | — | audio bytes |
| `mistral` | `MISTRAL_API_KEY` | — | — | audio bytes |
| `gemini` | `GEMINI_API_KEY` | — | `gemini-2.5-flash-preview-tts` | audio bytes |
| `xai` | `XAI_API_KEY` | — | — | audio bytes |
| `neutts` | `neutts` binary on PATH | — | `neuphonic/neutts-air-q4-gguf` | wav |
| `kittentts` | `python3` + the `kittentts` package | `Jasper` | `KittenML/kitten-tts-nano-0.8-int8` | wav |
| `piper` | `piper` binary on PATH | — | — | wav |
| `phonon` | local, GGUF via `ptts` (default) | `default` | `gradium/phonon-31m-en` q8_0 GGUF | wav (24 kHz) |

`BUILTIN_NAMES` in `tts_registry.rs` is the same eleven and is the guard
that stops a plugin from shadowing a built-in: registering one of those
names is ignored with a warning.

`edge` is **not local and not optional**: it shells out to Microsoft's
Edge services over the `edge-tts` CLI, which is why it earns no file
dependency and is not the default. A missing CLI, a non-zero exit and a
zero-length output file are each an explicit error.

## The two extension registries

- **`tts_registry.rs::TtsPluginRegistry`** is a
  `RwLock<HashMap<String, Arc<dyn TtsProvider>>>`. It is populated by
  plugins; the built-in registry on `TtsTool` is created empty in
  `TtsTool::new()` and can be swapped with `with_plugin_registry`.
  `is_available()` defaults to `true`, `voice_compatible()` to `false`,
  `list_voices()`/`list_models()` to empty. A plugin write goes to a temp
  `tts_plugin_output.mp3`, is base64'd, then deleted.
- **`tts_command_provider.rs::CommandProvider`** wraps a shell command
  template. Placeholders `{text}` and `{output_path}` (or `{{escaped}}`)
  are substituted with single-quote escaping, and the text is piped
  through a temp `input.txt`. Defaults are mp3 and a 120 s timeout, both
  overridable (`with_format`, `with_timeout`, `with_voice`, `with_model`,
  `with_voice_compatible`).

**Both extension registries are currently empty at runtime.** Nothing in
the tree calls `register_command_provider` or registers a
`TtsPluginRegistry` against a live `TtsTool`; the plugin registry is
populated by a WASM plugin's `TtsProvider` export, which no shipped plugin
in this tree provides. So today the eleven built-ins are the whole
surface.

## `transcribe_audio` — the STT path

`TranscriptionTool` speaks two providers:

| provider | key (config, then env) | default model |
|---|---|---|
| `groq` | `tools.stt.groq_api_key`, then `GROQ_API_KEY` | `whisper-large-v3-turbo` (`whisper-large-v3-turbo`) |
| `openai` | `tools.stt.openai_api_key`, then `OPENAI_API_KEY` / `VOICE_TOOLS_OPENAI_KEY` | `whisper-1` |

The per-call `model` argument beats the config value beats the built-in
default (`resolve_model`); an empty configured value falls through to the
default. The keys exist because a provider retires a model name and a
hardcoded one 404s with no way out short of a new build — that is what
`[tools.stt] groq_model` / `openai_model` are for, and
`a_configured_stt_model_overrides_the_builtin_default` pins the
precedence.

Before any network call the tool checks the file exists, is a file, has
one of ten accepted extensions (`.mp3 .mp4 .mpeg .mpga .m4a .wav .webm
.ogg .aac .flac`) and is at most 25 MB (`MAX_FILE_SIZE`). The payload is
a multipart upload. A result JSON carries `{success, transcript,
provider, model, error?}`.

On the gateway, `gateway_runner/enrichment.rs::enrich_voice` downloads a
Telegram voice message to a temp file, calls `transcribe_audio` with
`provider: "groq"`, and rewrites the message text as `[Transcription:
…]`. When either the download or the transcription fails it degrades to
`[Voice message: M:SS]` — the model still gets the message, minus its
content, and no turn dies.

The other half of the STT story is the model-facing wrapper in
`tui/operant_model/transcription.rs`: `wrap_transcription` wraps a
transcript in `<transcription>…</transcription>` so the model knows the
words came from speech recognition and may carry recognition errors, and
`strip_transcription` removes the tags for display (unbalanced tags are
kept verbatim so a literal `<transcription>` is never hidden). It is a
jcode-parity port, used by the UI layer, not by the tools.

## How audio leaves the process

A successful `text_to_speech` result carries `{audio: <base64>, format,
provider, voice}`. On the gateway, `AgentEvent::ToolComplete` for
`result.name == "text_to_speech"` decodes the base64 and calls
`gateway_runner`'s `send_voice`, which routes to the platform adapter.
`PlatformAdapter::send_voice` is a **default no-op** — it returns `Ok(())`
for every platform that does not implement it; Telegram is the real
implementation (multipart to the configured API base, mime from the
format: `ogg|opus` → audio/ogg, `mp3` → audio/mpeg, else audio/wav). So a
Discord or Slack deployment gets a tool success with no audible delivery.

The channel-level toggle is `/voice [on|off|status]`, which writes a
session `voice_enabled` metadata row. **Nothing reads that row back**:
the TTS interception in `gateway_runner.rs` fires for every successful
`text_to_speech` result regardless of the flag, so `/voice off` does not
stop voice messages today. Treat `/voice` as a status/placeholder
command, not a control.

## Configuration

| key | default | what it changes |
|---|---|---|
| `tts.provider` | `"phonon"` | the provider used when a tool call omits it |
| `tts.enabled` | `false` | a feature flag only: read by `doctor` / `setup` / `post_setup` to print a row; the tool ignores it |
| `tts.voice` | `None` | the voice used when a call leaves `voice` at the sentinel; omitted, the provider's own default is used |
| `tools.stt.groq_api_key` | `None` | key for the groq route (falls back to env) |
| `tools.stt.openai_api_key` | `None` | key for the openai route (falls back to env) |
| `tools.stt.groq_model` | `"whisper-large-v3-turbo"` | model id when the call omits one |
| `tools.stt.openai_model` | `"whisper-1"` | model id when the call omits one |

`operant.example.toml` comments the provider list on `[tts] provider` as
`edge|elevenlabs|openai|minimax|mistral|gemini|xai|neutts|kittentts|piper|phonon`,
and deriving the setup-wizard menu from `builtin_providers()` holds it to the
same list.

## Surfaces

- Tools: `text_to_speech` and `transcribe_audio`, both in
  `builtin_tool_names()` (registered as `TtsTool::new()` and
  `TranscriptionTool::new()` in `crates/operant-core/src/tools/builtin.rs`).
  Note `builtin_tool_names()` carries the short name `"tts"` in its list
  while the registered tool is `text_to_speech`; the count test tolerates
  the mismatch, but a model must call `text_to_speech`.
- Channels: `/voice [on|off|status]` (status only, see above), and the
  automatic voice-message delivery on `ToolComplete`.
- CLI/doctor: `operant doctor` prints a `tts` row from `[tts] enabled`;
  the engine loads lazily, so the doctor deliberately does NOT synthesise
  anything to prove it.
- Environment keys: `ELEVENLABS_API_KEY`, `OPENAI_API_KEY`,
  `MINIMAX_API_KEY`, `MISTRAL_API_KEY`, `GEMINI_API_KEY`, `XAI_API_KEY`,
  `GROQ_API_KEY`, `VOICE_TOOLS_OPENAI_KEY`. **These are read once, in the
  constructors** — a key exported after the process started is invisible;
  restart after changing one.

## VAD

`crates/operant-api/src/vad.rs` is the only VAD surface that still exists:
the `Vad` trait (mono f32 chunks in, `VadEvent` transitions out:
`Silence` / `SpeechStart` / `SpeechEnd`) and a `NoopVad` that always
reports `Silence`. Its own doc names it "used when
`gateway-voice-duplex` is enabled but no real VAD implementation is
configured. A real implementation (energy-based or webrtcvad) will follow
in a separate PR." **Nothing in this tree implements or consults `Vad`** —
there is no `gateway-voice-duplex` feature, no caller of `vad::`, and no
gateway receive path to gate. It is a seam, not a capability: treat it as
the place a duplex implementation would land, and do not describe operant
as having voice-activity detection.

## Failure modes

- **A missing key** → the provider's own error, verbatim:
  `"ELEVENLABS_API_KEY not set"`, `"MISTRAL_API_KEY not set"`, etc.
- **A missing local binary or package** → `"NeuTTS not installed. Install
  with: pip install neutts"` / `"Piper not installed. Install from:
  https://github.com/rhasspy/piper"`; the kitten path surfaces the
  Python traceback line.
- **A missing `edge-tts` CLI** → `"edge-tts is not installed. Install
  with: pip install edge-tts"`; a non-zero exit and a zero-length output
  file are each an error carrying the CLI's own stderr tail.
- **No size/format abuse** → the transcription tool refuses up front with
  the accepted-extension list and the 25 MB cap.
- **A Telegram voice download fails** → `[Voice message: M:SS]`, not a
  failed turn.
- **A platform without `send_voice`** → the tool reports success and the
  audio goes nowhere.

## Intention

The voice surface is deliberately tool-shaped: speech is something the
agent produces and consumes inside a turn, resolved at call time through
a dispatch table, rather than an input layer waiting for a recording. The
dead interactive state machine was the opposite bet — interactive
primitives with no live caller — and the purge records that honestly
rather than leaving the library claiming a voice mode that nothing
reaches. The same honesty governs the seams left behind: `edge`
synthesises through a subprocess instead of pretending to succeed,
the plugin and command registries are empty,
`NoopVad` says so in its own doc, and the doctor refuses to claim a TTS
route works until the engine is actually loaded. Switching the
default away from Phonon, like the other frozen design preferences, is an
operator decision the code documents but does not make.

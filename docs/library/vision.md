# Vision — the auxiliary-LLM contract

Owns: image and video understanding in operant — the honest
auxiliary-LLM contract, video analysis, image generation, and the
slots that configure them.
Not for: multimodal message content on the model path
([providers.md](providers.md)), the model-slot vocabulary
([config.md](config.md)), voice ([voice.md](voice.md)), the tool
registry ([tools.md](tools.md)).

Source: `crates/operant-core/src/tools/vision_tool.rs` (`vision_analyze`),
`crates/operant-core/src/tools/video_analysis_tool.rs` (`video_analyze`),
`crates/operant-core/src/tools/image_generation_tool.rs` (`image_generate`),
`crates/operant-core/src/model_roles.rs` (the `Vision` slot),
`crates/operant-core/src/config.rs` (`[vision]`,
`[auxiliary_models.vision]`).

## The contract

`vision_analyze` returns **text**. It resolves the image, base64-encodes
it, POSTs it to an auxiliary vision LLM, and hands the caller that model's
description. The pixels never reach the caller's context, and the tool's own
description says so in the same breath ("your own model does not see the
image, so never claim to have seen it yourself"). The result JSON carries
`"native_vision": false` as a machine-readable copy of the same claim.

That is a deliberate contract, not a missing feature. Until iter-1249 the
tool emitted a `_multimodal` envelope asserting the image had been attached
natively; the only consumer of that envelope lived in the
`operant-providers` crate, deleted at iter-1215, so the assertion was false
in every build that shipped it. Worse, the fast path that produced it was
gated on `base_url.contains("openai")`, which matches any OpenAI-compatible
endpoint including visionless ones. The gate, the envelope, the module doc,
the tool description and the schema now all state the same truth, and
`vision_tool.rs`'s tests assert the envelope is absent.

## Data flow

1. **Resolve the source.** Local paths first (`resolve_file_path` handles
   `file://` and `~`), then `data:` URLs, then `http(s)` URLs.
2. **Read it, with bounds.** Local reads are capped at 20 MB
   (`MAX_IMAGE_SIZE_BYTES`); a URL download is capped at 50 MB
   (`MAX_DOWNLOAD_SIZE_BYTES`), 30 s timeout
   (`DOWNLOAD_TIMEOUT_SECS`), max 10 redirects, and passes
   `check_url_safety` first — a private/internal address is refused as SSRF.
3. **Type it from the bytes, not the extension.** `detect_mime_type` reads
   PNG/JPEG/GIF/BMP/WEBP magic numbers; the extension is the fallback
   (`mime_from_extension`, which defaults to `image/jpeg` for an unknown
   extension).
4. **Resize if it needs it.** `needs_resize` fires when the estimated
   base64 size (bytes × 4/3 + 100) exceeds 4 MB (`EMBED_TARGET_BYTES`) or a
   dimension exceeds 7900 px (`EMBED_MAX_DIMENSION`). Resize is
   progressive: up to 5 halvings with Lanczos3, JPEG quality steps
   85/70/50 (PNG has no quality knob, so only dimensions shrink).
5. **Encode as a data URL.** `data:<mime>;base64,...`.
6. **Call the auxiliary model** with a 120 s timeout
   (`AUX_VISION_TIMEOUT_SECS`) against `<base_url>/chat/completions`,
   `max_tokens: 2000`, `temperature: 0.1`, and a prompt that first asks for
   a full description and then the caller's question.
7. **Return `{success, analysis, image_url, size_bytes, mime_type,
   native_vision: false}`.**

## The route, and the one that wins when nothing is configured

`call_auxiliary_vision_llm` reads `[auxiliary_models.vision]` from
`runtime_config()`:

| key | effect when set | when absent |
|---|---|---|
| `model` | sent as `model` | `google/gemini-2.0-flash-001` (`DEFAULT_VISION_MODEL`) |
| `api_key` | used as the bearer token | falls back to `OPENROUTER_API_KEY` |
| `base_url` | used as the endpoint base | `https://openrouter.ai/api/v1` |

With no `[auxiliary_models.vision]` block at all, the tool takes the
environment-only branch: `AUXILIARY_VISION_MODEL` (else the same Gemini
default) plus `OPENROUTER_API_KEY`. If neither a slot key nor
`OPENROUTER_API_KEY` yields a key, the tool fails with an error that names
both fixes — "Set auxiliary_models.vision in config or OPENROUTER_API_KEY
env var" — rather than degrading to a pretend answer. The same rule applies
to the two video/media tools: they need `FAL_KEY`, and they say so.

Note `[vision]` (`VisionSettings` in `config.rs`: `provider`, `model`,
`base_url`, `api_key`) is a **display-only** surface. Its only readers are
`operant setup` / `post_setup` / `doctor`, which print the row; nothing on
the runtime path consults it. Routing lives in
`[auxiliary_models.vision]`.

## The other two tools

`video_analyze` (`tools/video_analysis_tool.rs`) posts
`{video_url, prompt}` to `https://queue.fal.run/fal-ai/video-analysis`
with `Authorization: Key $FAL_KEY`, and returns the `analysis` string. The
prompt defaults to "Describe what's happening in this video". An earlier
`mode` knob was removed (RT-344) because it was deserialized and never
sent.

`image_generate` (`tools/image_generation_tool.rs`) goes through fal too,
with eight model names: `flux-pro` (default), `flux-dev`, `flux-klein`,
`gpt-image-1`, `gpt-image-2`, `recraft`, `ideogram`, `flux-schnell`.
Defaults are `aspect_ratio = "1:1"` (accepted: `1:1 16:9 9:16 4:3 3:4`,
unknown values fall back to `1:1`). An unknown model is an ERROR listing
the supported set (RT-336) — it used to serve flux-pro under the requested
name, so a typo produced "a flux-pro image labelled banana".

`video_analysis_tool.rs` and `image_generation_tool.rs` read `FAL_KEY`
once, in their constructors. A key exported after the process started is
invisible to them; change the env and restart.

## Configuration

| key | default | what it changes |
|---|---|---|
| `auxiliary_models.vision.model` | none → `google/gemini-2.0-flash-001` | the vision model id |
| `auxiliary_models.vision.api_key` | none → `OPENROUTER_API_KEY` | the bearer token |
| `auxiliary_models.vision.base_url` | none → `https://openrouter.ai/api/v1` | the endpoint base |
| `auxiliary_models.vision.provider` | none | accepted, not read by this path |
| `vision.provider` / `.model` / `.base_url` / `.api_key` | `Option::None` | `setup`/`doctor` status rows only |

Setting `[auxiliary_models.vision].model` in the TUI is the
`/model vision <id>` path (`ModelSlot::Vision` → `ALL_SLOTS`), and it
persists to the config file. (The `[multimodal]` config types were
deleted iter-2680 — the multimodal schema was removed as dead code, so
none of those keys change what `vision_analyze` does.)

## Surfaces

- Tools, as registered in
  `crates/operant-core/src/tools/builtin.rs`: `vision_analyze`
  (`VisionTool`), `image_generate` (`ImageGenerationTool::new()`),
  `video_analyze` (`VideoAnalysisTool::new()`). All three are in
  `builtin_tool_names()`, so `operant tools list` shows them and
  `operant tools disable` can remove them.
- The TUI slot picker (`ModelSlot::Vision`, label `vision`) and the
  channel command `/model vision <id>` resolve to the same key.
- `operant setup` prints a Vision row, driven by
  `config.vision.provider.is_some()` — a hint, not the routing check.

## Failure modes

- **No vision model configured** → "No vision model configured. Set
  auxiliary_models.vision in config or OPENROUTER_API_KEY env var."
- **Auxiliary call fails** → `Vision model failed: <cause>`, where the
  cause is the provider's own status and body.
- **URL blocked** → SSRF refusal before any fetch, naming the private
  address rule.
- **Too large** → named at each stage: download (50 MB), local read (20 MB),
  or after base64 (20 MB). The resize path exists to keep images inside the
  embed budget, not to accept unbounded input.
- **No `FAL_KEY`** → `FAL_KEY not set` from either fal-backed tool.
- **Unknown image model** → an error naming the model and the supported
  eight.

## Intention

The design keeps the main loop provider-agnostic: any chat model can "see"
because seeing is delegated to one auxiliary call whose result is text.
That is why the pixels-never-reach-the-caller rule is load-bearing rather
than a limitation to fix — the moment the tool claims an attachment that
does not exist, the model narrates an image it never had. A future native
multimodal path must state what it changes, per the iter-1249 lesson: a
capability claim the code does not support is worse than no claim.

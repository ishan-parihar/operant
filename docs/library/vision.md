# Vision — the auxiliary-LLM contract

Owns: image and video understanding in operant — the honest
auxiliary-LLM contract, video analysis, and image generation.
Not for: multimodal message content (the model path,
[providers.md](providers.md)), voice ([voice.md](voice.md)).

Source: `crates/operant-core/src/tools/vision_tool.rs`,
`crates/operant-core/src/tools/video_analysis_tool.rs`,
`crates/operant-core/src/tools/image_generation_tool.rs`,
`crates/operant-config/src/schema/media.rs` (media types).

## `vision_analyze` — text description, never pixels

The tool downloads an image from a URL, local path, or data URL,
converts to base64, and sends it to the **auxiliary vision LLM**
(`[auxiliary_models] vision`, or `OPENROUTER_API_KEY`). The tool result
is a TEXT description — **the main model never receives the pixels**.

This is the honest contract, and it is recent: until iter-1254/1261 a
substring gate (`base_url.contains("openai")`) and a `_multimodal`
envelope claimed "you can see it natively now" while nothing consumed
the envelope — a model believing it would attach an image and
hallucinate. The gate, the envelope, the module doc, the tool
description, and the schema summary now all state the same truth: the
image never reaches the caller's context; the one real path is the
auxiliary vision LLM, and the result is its text.

## `video_analysis`

Frame-extraction + analysis over the same auxiliary contract: a video is
sampled and described, with the result again being text the main model
reads.

## `image_generation`

Generation through a configured image model (the provider matrix rides
`[tools]` config). Output is an image artifact plus a reference the
model can cite.

## Intention

The single-auxiliary-LLM design keeps the main loop provider-agnostic:
any chat model can "see" because seeing is delegated. The
pixels-never-reach-the-main-model rule is a consequence of that contract,
not a limitation to "fix" — a future native-multimodal path must state
what it changes honestly, per the iter-1254 lesson (a capability claim
the code does not support is worse than no claim).

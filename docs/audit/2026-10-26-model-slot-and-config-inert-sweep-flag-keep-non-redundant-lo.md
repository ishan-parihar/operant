# Audit — model-slot inventory + whole-config inert sweep (flag only, nothing cut)

**Date**: 2026-10-26 · **Auditor**: agent-context · **Trigger**: operator asked "why do we have
a separate MOA model?", then "flag all inert model config, keep only non-redundant
load-bearing configuration", executed through `/ponytail-audit`.

**Method**: every config field in `crates/operant-core/src/config.rs` enumerated; each
cross-checked for real runtime readers across `crates/` (grep of the field name, doc-comment
and plumbing hits excluded, then manual adjudication of every hit to decide load-bearing vs
bookkeeping). This is a FLAG pass: `ponytail-audit` reports, applies nothing.

---

## Answer to the operator's question: why a separate MoA model slot?

`[moa]` is not redundant — it is the only **fan-out** surface in the harness, and the only
surface that consumes a *list* of slots rather than one:

| Surface | Shape | Calls/turn | Advice destination |
|---|---|---|---|
| `[advisor]` | ONE slot | 1 (post-turn, on transcript delta) | never enters agent context (operator notes only) |
| `[recap]` | ONE slot | 1 (only after an idle window) | never enters agent context (status line only) |
| `[moa]` | `references = [...]` array + `aggregator` | N+1 per opted-in turn (`operant run --moa`) | **injected into the acting agent's context for that turn** |
| `[auxiliary_models.compression]` | ONE slot | 1 per compaction | replaces the primary's own summarization of evicted turns |
| `[auxiliary_models.memory]` | ONE slot | periodic | routes a whitelisted tool's side-agent review loop |
| `[auxiliary_models.vision]` | ONE slot | per `vision_analyze` call | pixels never reach the primary model at all |

MoA exists because "get advice from N differently-routed models on the live conversation and
synthesize it" is not expressible with one slot. It defaults OFF precisely because it is the
expensive shape (N+1 calls), and it is opt-in per invocation (`--moa`), not ambient. The slot
`provider` field is genuinely used here — as the display label (`provider:model`,
`slot_label()`, moa.rs:223).

**Verdict: MoA stays.** It is the fan-out primitive; the other slots are single-slot or
off-context-by-design companions.

---

## Slot field reality (one correction to the mental model)

`AuxiliaryModelConfig` has four fields. Actual readership:

- `model` — read by every live consumer. Load-bearing.
- `base_url` / `api_key` — read by MoA (`slot_client`, moa.rs:236-248), recap/advisor
  (`moa::slot_client` via builders.rs), compression, vision. Load-bearing.
- `provider` — **read by nothing except MoA's display label and `cmd_doctor`'s probe
  report.** It does not route anything: every consumer builds an OpenAI-compatible client
  seeded from the primary `[client]` route, overriding only base_url/api_key.

So `provider` on the auxiliary slots is *decorative*. It is not flagged for deletion in this
pass (it is harmless, surfaced by setup/doctor, and removing it is a config-compat break for
existing `operant.toml` files), but the operator should know setting `provider = "anthropic"`
on `[recap.model]` does not switch protocols — it labels.

---

## Flagged: inert model slots (parsed, offered by `operant setup`, zero runtime readers)

| Slot | Flag | Detail |
|---|---|---|
| `[auxiliary_models.web_extract]` | **inert** | Doc comment in config.rs says so verbatim: the IGS extractor returns raw markdown, no LLM step exists to route to this slot |
| `[auxiliary_models.image_gen]` | **inert** | The real image tool (`image_generation_tool.rs`) reads the `FAL_KEY` env var, not this slot |
| `[auxiliary_models.embeddings]` | **inert** | Embeddings go through `agent.context_lcm_embedding_model` (+ optional `context_lcm_embedding_base_url`) — a different key entirely |
| `[auxiliary_models.search]` | **inert** | Web search is sourcehound-binary-only; no model call exists |
| `[auxiliary_models.code_execution]` | **inert** | No consumer |
| `[auxiliary_models.reasoning]` | **inert** | Reasoning effort is `agent.reasoning_effort`, a knob on the primary model — not a routed slot |

These six are "phantom routing surface": `operant setup` interviews the operator for them, and
a set value is silently ignored. That is worse than absence — it invites a misconfiguration the
operator cannot debug. **They are also the only flagged cluster the repo does NOT already
report** (see the correction at the end): `unused_setting_warnings` covers every other inert
key, so those cost nothing beyond a startup warning.

## Flagged: live slots (keep — all verified with cited call sites)

`[auxiliary_models.compression]` (main.rs:1125-1151, wired iter-1328) ·
`[auxiliary_models.vision]` (vision_tool.rs:291-311) ·
`[auxiliary_models.memory]` (prompting.rs:403-415) ·
`[recap.model]` (builders.rs ~655-690) · `[advisor.model]` + `[advisor] timeout_secs`
(advisor.rs:48-88) · `[moa]` array + aggregator (moa.rs) · embedding model +
`context_lcm_embedding_base_url` (main.rs:2368-2390) · primary `[agent]/[client]`,
`fallback_models`, `[providers]` chain.

---

## Flagged: inert config beyond the model slots (same sweep, whole config.rs)

**`[pk]` cluster — 3 keys, parsed, zero readers anywhere:**
`ndjson_socket`, `sidecar_script`, `exec_timeout_secs`. (No tool surface named "pk" was
found reading them.)

**`[bylaws]` — 1 key:** `grant_ttl_days` — and its inertness is already *tested against*:
`tests/readme_examples_load.rs:74-75` asserts the README does NOT advertise it, with a
comment naming it as governing nothing. A dead key with a test guarding its documentation
absence — the key remains only for TOML compatibility.

**`[socialization]` — 1 key:** `turn_budget` (a same-named `turn_budget_left` in
stream_retry_budget.rs is an unrelated local variable, not a config read).

**`[tui]` — 6 keys:** `refresh_rate_ms`, `show_iterations`, `compact_width`, `medium_width`,
`prompt_placeholder`, `landing_title`. The TUI's init (`tui/app/init.rs`) consumes exactly
three `[tui]` keys — `show_tool_calls`, `terminal_scroll_mode`, `word_completion` (plus
threshold). These six were never wired, or were wired and the wiring was deleted.

**`[skills]` — 2 keys:** `template_name`, `template_description` — config.rs itself
documents them as accepted-but-unread (`// EXAMPLE_ROOT ... SKILL_TEMPLATE` migration
shape); `tests/example_template_loads.rs:223` exists to keep every commented example in the
template honest about them.

**`[memory]` — kept-on-purpose, NOT flagged:** `agentmemory_*` (url/secret/version/auto_spawn),
`response_cache_enabled`, `audit_enabled` all have active `warn_legacy_keys` paths
(config.rs:1776-1810) that detect a set value and emit a "this key is removed — delete it"
warning. They are migration tripwires, not dead weight. Deleting them would silently accept
old configs again.

---

## Net assessment (ponytail form)

```
delete: the 6 inert aux slots' PROMOTION surface — 2 sites (unused_setting_warnings
        does not name them; operant setup interviews them). [config.rs:2651, cmd_setup.rs:746]
keep:    every live slot in the table above, incl. MoA (fan-out primitive); AuxiliaryModelConfig
        itself and provider (label-only, compat); and EVERY other inert key flagged below
        — those already warn in production, so they cost the operator nothing.
note:    AuxiliaryModelConfig.provider routes nothing on single-slot paths — decorative.
```

Net if executed: -6 phantom slots promoted to honest "unread" status, -6 `operant setup`
interview prompts asking for values nobody reads, 0 config-compat breakage (fields stay), 0
deps, 0 behavior change. Not 15 key deletions — see the correction below: 9 of the 15
flagged keys are already warned about at load time.

**CAUTION for a future executor** — verified attribute coverage, plus the mechanism this
repo ALREADY has (correction made after re-verification):

`AppConfig::unused_setting_warnings()` (config.rs:2651) is a **live, production-wired
tripwire** — `load_app_config` calls it at config.rs:2347 and prints every line. It still
accepts the key (TOML keeps parsing) but says, in production, "accepted but unread — it has
no effect. Remove the key from operant.toml." It already covers: all of `[pk]`,
`[bylaws] grant_ttl_days`, all of `[socialization]`, `[agent] max_consecutive_tool_only`,
`[tools] event_channel_size / browser_binary_path`, `[tools.tool_search]`, both
`[skills] template_*`, ALL SIX `[tui]` keys, and `[memory] agentmemory_*`.

**The genuine gap is exactly one cluster**: the six inert `[auxiliary_models.*]` slots.
No tripwire covers them, so a set value is accepted silently — no warning at load time,
no effect at runtime. That is the only part of this audit that is actively misleading
rather than merely dead.

Safe-execution shape (verified):

| Target | Shape |
|---|---|
| 6 inert aux slots | add an `unused_setting_warnings` flag (repo idiom) + drop from `cmd_setup.rs`. Field deletion IS a breaking change for any config that sets them (parent `AppConfig` carries `deny_unknown_fields`), so defer it |
| `[pk]` / `[bylaws]` / `[socialization]` / `[tui]` / `[skills]` | nothing to do — already warned at load; delete struct fields only when a breaking release is acceptable |

**Flag-only status for the FLAG pass; the fix has since landed (iter-1349).**
The two-site fix this plan called for is complete: the six inert slots now
produce a per-slot "accepted but unread" warning at load
(`AppConfig::unused_setting_warnings`, config.rs ~2673, pinned by
`unused_setting_warnings_name_the_inert_auxiliary_slots_only` — mutation-proven:
the test fails with `web_extract: []` when the flag is removed), and
`operant setup`'s interview stops asking for them (`cmd_setup.rs`, 9 slots → 3:
vision / compression / memory). Struct fields stay — deleting one breaks any
config that sets it, because parent `AppConfig` carries `deny_unknown_fields`.
Nothing else from the flag list needs work: every other flagged key already
warns in production, so deleting its field is a future breaking-release
cleanup, not a defect.

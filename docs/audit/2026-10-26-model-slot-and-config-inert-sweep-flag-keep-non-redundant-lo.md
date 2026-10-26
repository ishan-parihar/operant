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
a set value is silently ignored. That is worse than absent — it invites a misconfiguration the
operator cannot debug.

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
delete: 15 inert config keys (6 model slots + pk trio + grant_ttl_days + turn_budget +
         6 [tui] + 2 [skills]) — migration shim or outright removal. [config.rs]
keep:    every live slot in the table above, incl. MoA (fan-out primitive), and the
         [memory] legacy tripwires (config.rs:1776-1810).
note:    AuxiliaryModelConfig.provider routes nothing on single-slot paths — decorative.
```

Net if executed: -15 config keys, -6 `operant setup` interview prompts asking for values
nobody reads, 0 deps. No code paths change behavior — every flagged key already governs
nothing.

**CAUTION for a future executor** — the attribute coverage is per-struct, verified here:

| Struct holding flagged key(s) | `deny_unknown_fields` | Safe-execution shape |
|---|---|---|
| `AuxiliaryModels` (6 slots) | **no** | deleting the field is safe for any config that never set it; a config that SET it then hits `deny_unknown_fields` on the PARENT — confirm `AppConfig` tolerates an unknown `auxiliary_models.*` key before removing, else old configs break |
| `PkSettings` (3 keys) | yes | removal makes a config naming them fail to load. Prefer the tripwire: keep the field, add a `warn_legacy_keys` path |
| `BylawsSettings` (`grant_ttl_days`) | yes | same tripwire path (a README test already asserts its absence from docs) |
| `SocializationSettings` (`turn_budget`) | yes | same tripwire path |
| `TuiSettings` (6 keys) | yes | same tripwire path; ALSO strip them from `operant setup`'s interview, which is the half that actively misleads the operator |
| `SkillsSettings` (2 keys) | yes | already documented accepted-but-unread; `example_template_loads.rs:223` keeps the template honest — leave the fields, drop the setup prompts |

Do NOT silently drop a key from a `deny_unknown_fields` struct without one of the two shapes —
that is how a config key silently becomes a load error. The `[tui]` six are the
highest-value cut: `operant setup` interviews the operator for them and every set value is
ignored — a misconfiguration the operator cannot debug.

**Flag-only status**: this pass cut nothing (ponytail-audit reports, applies nothing).
Next iteration, if the operator approves: strip the 6 inert slots from `cmd_setup.rs`'s
interview (highest value, zero load-compat risk), and leave the struct fields for a
follow-up that adds tripwires per the table above.

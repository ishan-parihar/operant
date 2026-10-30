# Model Picker Upgrade Plan — roles over the existing inline picker, one
# selection surface shared by the TUI and the gateway channels

**Date:** 2026-10-30
**Kind:** pre-upgrade survey (no code change in this iteration)
**Boundary:** surveys the oh-my-pi model picker's role nuance, operant's
current model-selection surface (TUI inline picker + gateway `/model`), and
produces a ranked, /ponytail plan. Nothing below is implemented yet.

---

## 0. The one-paragraph answer

oh-my-pi's nuance is **roles**: a model selection is not just "which id", it is
"(which id, for which workload)". Its chat roles are `default`, `smol`, `slow`,
`vision`, `plan`, `commit`, `tiny`, `memory`, `task`, `advisor` — each a named
model slot with its own priority chain, its own thinking level, and a
`@role` alias you can type. `default` is the conversation model and the ONLY
role whose assignment also switches the active session; every other role is
switched without touching the conversation. Operant's analogue is **already
structurally there and already partially live**: `[auxiliary_models.*]` holds
per-task slots, `[recap]` is literally documented in `config.rs:70-75` as "the
omp `smol` role", and the TUI's `/model` is a single flat inline picker over
the agent's own endpoint. The gap is not "operant lacks roles" — it is that
operant's roles are **config-file-only, unassigned, unlisted, and invisible to
every selection surface**.

So the plan is deliberately NOT "port oh-my-pi's model hub". It is:

1. **Make the existing inline picker role-aware** — 3 columns it already
   renders, one per role, instead of 1 flat list (small).
2. **Make gateway `/model` a first-class channel command with the same
   vocabulary** — `default`, `smol`, `slow` + the live aux slots (small).
3. **Give both one selection vocabulary in `operant-core`** so a Telegram
   `/model smol` and a TUI `/model` resolve through the same function
   (medium, but the only step that makes the surfaces agree).

---

## 1. oh-my-pi: the parts that carry the nuance

Source read in `../external-repos/oh-my-pi`.

### 1.1 Roles, not ids (`packages/tui/src/overlays/model-browser.ts:58-99`)

```ts
export type ModelRole = "default" | "smol" | "slow" | "vision" | "plan"
  | "commit" | "tiny" | "memory" | "task" | "advisor" | "widget" | "image"
  | "web" | "speech" | "dictation" | "judge";
```

Split three ways, which matters more than the list itself:

| group | roles | property |
|---|---|---|
| `CHAT_MODEL_ROLE_IDS` | default, smol, slow, vision, plan, commit, tiny, memory, task, advisor, widget | accept an ordinary chat model |
| `KIND_ROLE_IDS` | image, web, speech, dictation, judge | accept only that catalog kind |
| — | — | `judge` exceptionally accepts tiny + chat too |

Three behaviours that create the whole feel:

- **`default` is the only self-switching role.** `docs/models.md:699`:
  "Assigning `default` normally also switches the active model, unless a
  higher-priority settings layer overrides the edited assignment."
- **Every other role is a silent assign.** Same paragraph: "the session-only
  model picker changes the active model without rewriting role assignments" —
  and the roles view assigns `plan` without entering plan mode.
- **Roles carry a thinking level.** `plan: "@slow:high"` — a `:level` suffix on
  the role's value. The model switch and the effort switch are ONE selection.

### 1.2 Resolution is a priority chain, not a lookup
(`packages/catalog/src/compat/rules.json`, `roleDefaults`)

`smol` is ~38 ordered candidates (`claude-haiku-5-5`, `haiku`, `gemini-*-flash`,
`gpt-*-mini`, `-mini`, …) and the chain is walked until a provider with concrete
credentials matches. `slow` is its mirror. Roles can alias each other (`judge`
→ `@tiny` → `@smol` → `@default`), unset `smol`/`slow` inherit a configured
`default`, and `advisor` uses `slow` or its own chain rather than the live
conversation model. Keyline in `docs/models.md:697`: *"The `tiny` role selects
lightweight models for background work such as session titles; when unset, it
resolves through `@smol`."*

The consequence for a picker: **an unset role is not an error, it is a chain**.
That is what makes `/model smol` safe to type on a fresh install.

### 1.3 The picker itself is two surfaces on one browser

| surface | file | shape |
|---|---|---|
| alt+P / `/switch` session-only | `overlays/model-picker.ts` (514 lines) | bottom-anchored 40% overlay, **no provider sidebar** |
| `/models` hub | `overlays/model-hub.ts` + `model-browser.ts` (2283) | fullscreen, roles view + provider catalogs |

`model-picker.ts` is the interesting one. Its whole body is:

- hydrate synchronously from the catalog snapshot so the first Enter acts
  (a `.then()` re-sync only when the catalog moves);
- a mode flag: `session` (model rows) / `role` (search begins with `@`) /
  `task` (Task-subagent target) / `selection` (no mutation at all);
- `@` at the start of the query swaps the list from models to `@role` rows,
  colored by their ctrl+p segment position;
- over-context models are marked, and picking one compacts first
  (`selector-controller.ts:789-796`, `switchAfterCompaction`).

The `@`-prefix mode switch is the cheap part: one `startsWith("@")`, one item
swap. That is the entire idea worth taking.

### 1.4 Presets are a snapshot, not a feature
`/modelpreset save|switch|list|delete` writes every role assignment plus
`defaultThinkingLevel` under `modelPresets`. Switching replaces `--model`/
`--smol` session overrides and reports which config layer still wins.
**Not portable in one iteration** — operant has no `modelPresets`, no
`--model`/`--smol` session overrides, and no layered settings storage. Leave it
out of scope (recorded so it is not re-litigated).

---

## 2. operant today: the simpler model-picker, found

### 2.1 TUI — `open_model_picker_inline` (plan 05B), not a modal
`crates/operant-cli/src/tui/app/providers.rs:102-184`. The legacy
`tui/model_picker/` overlay described in `docs/audit/2026-10-16-operant-audit/05`
is **gone from the tree** (`git ls-files` has no `model_picker/`); only
`scripts/extract_model_picker.py` and `session_hub/` remain. What replaced it:

- rows = `GET {config.client.base_url}/models` (the agent's OWN endpoint, with
  the client's key — `fetch_endpoint_models`, `providers.rs:12-51`), **merged
  with** `[agent] model` + `agent.fallback_models` always shown and marked
  "configured" (`providers.rs:117-133`);
- opens instantly on the configured rows, background fetch refreshes in place
  through `model_fetch_rx` → `drain_model_fetch`
  (`app/channel_drains.rs:256-317`);
- rendered by the shared `InlineInteractiveState` three-column picker
  (`operant_app/tui_fns.rs:439-461`), which **already carries `is_default`,
  `is_favorite`, `recommended`, `usage_score`, `old`, `created_date`,
  `effort`** — the jcode-parity fields, most `#[allow(dead_code)]` because the
  live renderer ignores them.

That field set is the important discovery: **operant's picker data model is
already role-ready and renders in flat insertion order.** The upgrade is a
render + source change, not a rewrite.

### 2.2 What the TUI picker applies (`app/key_handling.rs:2794-2811`)

```rust
PickerAction::Model => {
    self.pending_model = Some(entry.name.clone());
    if let Some(effort) = entry.effort.clone() { self.pending_effort = Some(effort); }
    self.set_status_notice(format!("Model: {} (applying…)", entry.name));
}
```

One action, one drain. The `effort` on the row rides the SAME `pending_effort`
drain the `/effort` picker uses — so **oh-my-pi's "role carries a thinking
level" already has a working channel in operant**, just unused for it.

### 2.3 Gateway `/model` — the live path, and its three real gaps

`gateway_commands.rs:1786-1812` (sync arm) and `gateway_runner.rs:2678-2695`
(turn-path read).

**Live today:** `/model <name>` writes `model_override` into session metadata;
the turn path reads it back and calls `agent.set_model()` (which takes `&self`
through `Arc<OperantAgent>` and is checked at each `run()` —
`operant-core/src/agent/events.rs:491-507`). This is the ONE metadata key the
turn path reads (`docs/audit/2026-10-25-...parity-gaps...md` F3 confirms).

**Gap 1 — no list, so the vocabulary is unusable.** `/model` with no args prints
`Current model: X / Session override / Usage: /model <name>`. There is no
catalogue, no `list`, no role names. An operator on Telegram must already know
an exact endpoint id. This is the direct cause of the user's ask: "the
gateway-channels' model updating/changing works natively through the slash
commands".

**Gap 2 — the sync arm never bootstraps the session row.** The `/goal` arm
(`gateway_commands.rs:1572-1580`) has an explicit create-or-find because RT-175
hit it: a command can be the FIRST message a chat ever sends, so
`update_session_metadata` is a silent no-op and the reply still promises
success. The `/model` arm (`:1801-1806`) has **no `create_session`** — verified.
On a fresh chat, `/model gpt-4o` replies "Model switch requested" and the
override evaporates.

**Gap 3 — nothing resolves or validates.** No catalog is consulted, no provider
is inferred for `--provider` (declared in `args_hint: "[model] [--provider name]"`
at `:360` but never parsed — the whole `a.trim()` is the model id). A typo
silently becomes a `set_model("gpt-4oo")` that fails at request time with a
provider error, far from the command that caused it.

### 2.4 Roles operant already has (the surprising inventory)

`crates/operant-core/src/config.rs`:

| slot | status |
|---|---|
| `agent.model` | live — the conversation model |
| `agent.fallback_models` | live — per-request same-provider chain |
| `agent.show_reasoning` (via `/reasoning`) | live |
| `auxiliary_models.vision` | live — `tools/vision_tool.rs:291,318` |
| `auxiliary_models.compression` | live — `main.rs:1118,1139` |
| `auxiliary_models.memory` | live — `agent/prompting.rs:403` |
| `auxiliary_models.web_extract/image_gen/embeddings/search/code_execution/reasoning` | **inert**, and `config.rs:2877-2905` already emits a per-slot warning telling the operator to delete the key |
| `[recap]` | live, documented at `config.rs:70-75` as "the omp `smol` role" |
| `[moa].references` / `.aggregator` | live when `[moa] enabled` |
| `[advisor]` | live |

So operant's `smol` exists as `[recap]`, and it has three more genuinely live
aux slots. **Nothing lets an operator SEE or SET any of these from any
surface** — `/model` touches only `agent.model`. That, not the id vocabulary, is
the real gap.

---

## 3. The mismatch, stated precisely

| | oh-my-pi | operant |
|---|---|---|
| role count (chat) | 11, each with a chain | 0 named; 9 flat config slots |
| role alias syntax | `@smol`, `@smol:high` | none |
| assign vs switch | `default` switches, others assign | everything switches, one target |
| role resolution | 38-candidate priority chain | exact id only |
| role visible in picker | yes — `@` prefix mode, ctrl+p cycle | no — one flat list of endpoint ids |
| role visible over chat channel | `/model @smol` | no |
| presets | `/modelpreset` | none |
| over-context handling | compact-then-switch | none |
| session-scoped override | `--model` / `--smol` flags | `model_override` metadata (one key) |

Operant's TUI picker is genuinely simpler and, per the audit, genuinely better
rooted (real endpoint, not a name-inferred catalog). **Keep it.** Add roles to
it. Port the picker chrome only, not the hub.

---

## 4. Plan — ranked, /ponytail

Each step is one iteration. Every step's acceptance is a command, not a
narrative. Every step ships a regression test that fails without the fix and
cites it in the commit body.

### Step 1 — `/model list` + `/model <role>` over the gateway (the actual ask)
**Why first:** closes the user's ask end-to-end, needs no new types.
**New concept:** none. A role is a name the resolver already understands.

1. In `operant-core`, add the one seam:
   `pub fn resolve_model_selection(id: &str, cfg: &AppConfig) -> Selection`
   returning `{ resolved_id, slot: ModelSlot, thinking: Option<String> }` where
   `ModelSlot` is the existing slots (`Conversation`, `Fallback`, `Recap`,
   `Vision`, `Compression`, `Memory`, `MoaReference`, `MoaAggregator`).
   Resolution order: exact `agent.model` match → configured aux slot by name →
   the short aliases below → error naming the close matches.
2. Gateway `/model` with no args prints the catalogue: `agent.model`,
   `fallback_models`, and each SET aux slot with its slot name. One line each.
   This is the missing list.
3. Gateway `/model <role>` (role ∈ `default|smol|slow|vision|compression|memory`)
   resolves through (1) and writes `model_override`. **`smol` maps to
   `[recap]`, `slow` to `[auxiliary_models.memory]`** — documented as omp
   aliases in `operant.example.toml`, not as new config.
4. Fix Gap 2 in the same iteration: the create-or-find `/goal`
   (`:1572`) got, applied to `:1801`.
5. Reject an unknown id with the close matches, never a silent override.

**Acceptance:** `/model` on a chat with no session row prints the catalogue AND
creates the row; `/model gpt-4oo` replies with candidates and writes nothing;
`/model smol` resolves to the `[recap]` model; a gateway test drives
`handle_command` end-to-end and asserts the store's metadata and the reply.

**Size:** S.

### Step 2 — the TUI picker gets a role column and role rows
Depends on (1) so both surfaces resolve identically.

1. `open_model_picker_inline` (`providers.rs:102`) keeps its endpoint rows and
   appends one **role row per SET slot**, marked by slot name
   (`default`, `smol`, …). Today's `[agent] model / fallback_models` detail
   becomes the role's detail line — the data is already there in
   `PickerEntry.detail`.
2. Enter on a role row applies it to that slot, not to `agent.model`;
   `PickerAction::Model` (`key_handling.rs:2798`) grows one arm, or better,
   `PickerAction` gains `ModelSlot(ModelSlot)` and the apply routes through the
   same resolver as Step 1.
3. Role rows carry their slot's thinking level in `entry.effort`, so the
   existing `pending_effort` line (`key_handling.rs:2801-2803`) applies it —
   oh-my-pi's "role carries a thinking level", free.
4. Set the already-dead fields instead of leaving them dead: `is_default` on the
   `default`-role row, `is_current` on the active conversation row. The
   `#[allow(dead_code, reason = "jcode-parity port…")]` on `PickerEntry`
   (`tui_fns.rs:465`) shrinks.

**Acceptance:** a headless `tui debug simulate` scenario (the existing
`model-picker.scenario.json`) asserts the role rows render and that Enter on
`smol` writes the `[recap]` slot; `no_registered_tui_command_is_unwired`-style
static test pins the new action arm.

**Size:** S.

### Step 3 — `/model` validation and `--provider` actually parse
Gap 3, now that (1) exists.

1. Parse `--provider <name>` (declared at `gateway_commands.rs:363`, never
   parsed) into a provider-scoped id, or refuse with the known providers.
2. Validate the resolved id against the endpoint catalog before writing
   `model_override` — reuse `fetch_endpoint_models`'s
   `model_catalog::resolve` (`providers.rs:23-33`) rather than a new fetch.
3. Report the switch AND the slot it landed in, so an operator on Telegram can
   tell a conversation switch from a `smol` assignment.

**Acceptance:** `/model --provider bogus gpt-4o` refuses and names the known
providers; a test asserts an unknown model never reaches `set_model`.

**Size:** S.

### Step 4 — purge the inert aux slots from the *picker vocabulary* only
`config.rs:2877-2905` already warns on six inert slots. The picker must NOT
list slots that have no consumer, or it becomes the phantom surface the
config comment describes. `ModelSlot` in Step 1 should enumerate only the live
ones (conversation, fallback, recap, vision, compression, memory, moa×2) and
**not** the six inert ones. Deleting the config keys is a separate, larger
iteration — `deny_unknown_fields` means removing a field breaks a config that
sets it, and this plan is about selection, not config migration.

**Acceptance:** the picker's role list length equals the live slot count; a test
that adds a set inert slot asserts it does not appear as a role row.

**Size:** XS.

---

## 5. Explicitly NOT in this plan

- **The oh-my-pi `/models` hub** (2283-line fullscreen, roles view, provider
  catalogs, per-model cost/pricing-status columns). Operant's inline picker is
  smaller and correctly rooted; the hub is three steps of polish with no
  correctness payoff.
- **`@role` search-mode in the picker.** Nice, and one `startsWith("@")`, but
  role ROWS give the same reachability without a mode flag. Add it later if
  role count grows past ~8.
- **`/modelpreset`** — needs `modelPresets` config, `--model`/`--smol` session
  overrides, and layered settings precedence. Three absent foundations.
- **The 11-role set verbatim** — `tiny`, `task`, `advisor`, `widget`, `commit`,
  `plan` are roles oh-my-pi's features own. Operant's set is its live slots.
- **38-candidate priority chains.** Operant's `model_catalog::resolve` already
  ranks (`default_model`, `agentic` set, `providers.rs:62-79`); the chain would
  be a second resolver.
- **Compaction-on-overflow before a switch** (`selector-controller.ts:789`).
  Operant has context-overflow auto-compression in the agent loop; wiring it
  into a switch is its own iteration and not needed for the ask.

## 6. Risks

| risk | mitigation |
|---|---|
| `smol`→`[recap]` is a surprising alias | it is `config.rs:70-75`'s own words; document in `operant.example.toml` and the `/model` reply |
| a second resolver appears | Step 1 is the ONLY selection resolver; the TUI, gateway, and any future CLI call it |
| role rows make the picker noisy with many slots | Step 4 filters to live slots; today that is ~8 |
| Slack/WhatsApp/Email/SMS cannot receive at all (`slack.rs:403-408`, F4) | the feature lands on Telegram + Discord + webhook; Slack needs Socket Mode first (RT-86). The plan's surfaces are the three that can receive. |

## 7. Follow-ups found while surveying (not this plan)

- `PickerEntry.recommendation_rank` / `usage_score` / `old` /
  `estimated_reference_cost_micros` are still `#[allow(dead_code)]` with no
  reader — Step 2 consumes two of them; the rest are a later decision.
- `scripts/extract_model_picker.py` is now fully orphaned (no
  `model_picker.rs` to decompose). Delete in a chore iteration.
- `docs/audit/2026-10-16-operant-audit/05-model-picker-jcode-inline.md` §A1 row 1
  claims the TUI `/model` never reaches `agent.set_model`. The
  `pending_model` drain (`key_handling.rs:2800`) now does. That audit doc is
  stale on that point.

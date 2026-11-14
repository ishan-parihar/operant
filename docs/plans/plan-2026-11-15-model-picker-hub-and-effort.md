# Plan — a models page that owns model, role, and effort

**Date:** 2026-11-15 (revision 2 — supersedes revision 1, which was written before
the oh-my-pi model-picker teardown landed and got four things wrong: it said 17
roles where there are 16; it missed that omp's `/model` opens the *hub* while
`/switch` and `Alt+P` open the *compact* picker; it priced session scope as a plain
read when omp deliberately does **not** restore a temporary switch; and it omitted
that `Shift+Tab` is omp's *effort* key, which is the key this plan takes away.)
**Base:** `origin/main` @ `59bab8e0`
**Status:** outline. Writing this file executes nothing.
**Sibling:** [plan-2026-11-15-session-hub-surfaces-and-agent-identity.md](plan-2026-11-15-session-hub-surfaces-and-agent-identity.md)
— its T4 takes `Shift+Tab` for the agent identity and **depends on M1 here**
landing first.

## What the operator asked for

"`SHIFT+TAB` must change the identity of the active-agent, from the
employee-list/personas, by replacing the effort-ladder. The effort-modification
must be ported to the models page. Check how oh-my-pi implements the
model-picker, and create a plan to implement that level of model-picking system,
integrated with the various roles, and effort."

Three deliverables: the effort ladder moves off `Shift+Tab`; effort becomes part
of the models page; and the models page reaches oh-my-pi's level, with roles and
effort as first-class parts of it.

## The structural finding this plan turns on

**Operant's effort picker is not a separate surface — it is the model picker's
kind with different rows, so choosing an effort runs the model apply path.**

`open_effort_picker_inline` (`app/providers.rs:702-747`) builds a
`PickerKind::Model` picker whose rows carry `action: PickerAction::Model`, a
display `name` of `"◐ normal"`, and `effort: Some("normal")`. The apply step is
shared (`key_handling.rs:3317-3325`):

```rust
PickerAction::Model => {
    self.pending_model = Some(entry.name.clone());
    if let Some(effort) = entry.effort.clone() {
        self.pending_effort = Some(effort);
    }
```

So one keypress on an effort row sets **both** a model and an effort, and the
model it sets is the row's display label.

That single fact explains why effort cannot simply be "moved": the surface that
holds it is not a surface, it is a mode of another one, and the mode has no
identity of its own. Every tranche below follows from giving effort, model, and
role each their own identity.

## Ground truth — oh-my-pi's model system

oh-my-pi has **four** modules, not one, and the split is the design:

| Module | Lines | Owns |
|---|---|---|
| `packages/tui/src/overlays/model-hub.ts` | 4,214 | The full-screen `/models` hub: a sidebar of **scopes** (recently used, role management, all models, one entry per provider) beside a `ModelBrowser` body. The Roles view assigns directly: pick a role, pick a model, adjust thinking in an inline strip, or clear the role back to auto. |
| `packages/tui/src/overlays/model-browser.ts` | 2,377 | The shared list body: rows, grouping, fuzzy filter, thinking strip. |
| `packages/tui/src/overlays/model-picker.ts` | 505 | The **compact session-only** picker (`Alt+P` / `/switch`): a bottom-anchored floating overlay with no provider sidebar. "A search beginning with `@` exposes the configured ctrl+p quick roles." |
| `packages/tui/src/overlays/model-selector.ts` | 107 | The **model-string grammar**: parsing only, no UI. |

And the effort capability lives in the catalogue, not the UI:

| Fact | Evidence |
|---|---|
| `Effort` = `minimal, low, medium, high, xhigh, max` — six, ordered least to most intensive | `packages/catalog/src/effort.ts:1-17` |
| `ThinkingLevel` = those six plus `inherit` and `off` | `packages/agent/src/thinking.ts:8-19` |
| `getSupportedEfforts(model)` returns `[]` when `!model.reasoning`, else `model.thinking.efforts` | `packages/catalog/src/model-thinking.ts:23-28` |
| An unserved level follows `thinking.effortMap` when it reaches the wire unchanged, otherwise **climbs down to the nearest lower tier** | `model-thinking.ts:43-88` |
| `:max` and `:auto` are gated behind flags **because real model ids end in `:max`** (e.g. `glm-4.7:max`) — an ungated split silently reinterprets them | `model-selector.ts:14-31` |
| Role aliases are `@role` or `pi/role`, and `splitThinkingSuffix` takes the alias prefix length so `@smol:xhigh` splits correctly | `model-selector.ts:34-58`; `model-resolver.ts:1109-1128` |
| **16** roles in two sections — `chat` (11): default, smol, slow, vision, plan, commit, tiny, memory, task, advisor, widget; `kind` (5): image, web, speech, dictation, judge | `model-browser.ts:78-95` (`MODEL_ROLE_IDS`, counted: 16), `:96-108` (`CHAT_MODEL_ROLE_IDS`, 11), `:109` (`KIND_ROLE_IDS`, 5); metadata `config/model-roles.ts:56-91` |
| Every role carries `tag`, `name`, `color`, `section`, and an **`accepts(model)` predicate** that decides which models the role may take | `model-roles.ts:36-51`, `:54-97` |
| `/switch` accepts "fuzzy ids, provider/id, `@role`, `:level`" and is **session-only** | `slash-commands/builtin-modes.ts:457-463` |

### The surface map, which is the part operant gets wrong by having one surface

| Input | omp opens | Evidence |
|---|---|---|
| `/model` (no args) | **Hub** | `builtin-modes.ts:451-454` |
| `/model <sel>` | nothing — resolves and `session.setModel()` **persistent** | `:422-450` |
| `/switch` (no args) | **Compact picker**, session-only | `:491-497` |
| `/switch <sel>` | nothing — `switchSessionModel(model, level)` | `:498-504` |
| `Alt+P` (`app.model.selectTemporary`) | **Compact picker** | `custom-editor.ts:146`, `input-controller.ts:719-723` |
| `Alt+M` (`app.model.select`) | **Hub** | `custom-editor.ts:145`, `input-controller.ts:727-728` |
| `Ctrl+P` / `Shift+Ctrl+P` | no picker — cycle **role** models forward / back | `custom-editor.ts:143-144`, `input-controller.ts:715-716` |
| `Shift+Tab` (`app.thinking.cycle`) | no picker — cycle **effort** | `custom-editor.ts:142`, `input-controller.ts:702-706` |
| status-line model segment click | **Compact picker** | `interactive-mode.ts:2302-2305` |
| `Alt+P` **inside** the open picker | flips session-model ↔ task-subagent target | `model-picker.ts:283-289` |

Dispatch (`selector-controller.ts:727-737`): `selectOnly` → compact;
`temporaryOnly` **or `ctx.focusedAgentId`** → compact; otherwise → hub.

**Operant today has exactly one surface for all of this** — `open_model_picker_inline`,
reached from `/model` and `/models` — with no session/global distinction and no
compact-vs-hub split. That is the gap this plan closes, and it is why the tranches
are ordered M1-M5 (correctness) before M7 (the layout that would hold the split).

**Favourites are operant's own, not a port.** The study checked: omp has **no**
favourites or pinning feature at all — repo-wide `ctrl+n` returns zero matches, and
`favourite|favorite` finds only unrelated words. Operant's `Ctrl+N` / `Ctrl+O` /
in-picker `Shift+Tab`-cycles-favourites (`providers.rs:595-625`) are its own
invention and stay. That matters for M7: the marker column has to carry a fact omp
never had, so the layout cannot be copied wholesale.

Four ideas are load-bearing for this plan:

- **The role is not a label, it is a predicate.** `smol` accepts only
  `kind === "chat"`; `tiny` accepts `tiny | chat`; `web` requires
  `model.webSearch !== undefined`. A role that can be assigned a model it cannot
  run is the phantom surface operant's `config.rs::unused_setting_warnings`
  already exists to prevent.
- **Effort is a property of the model, not of the session.** The ladder is
  clamped per model before it reaches the wire.
- **The grammar is one string**, not three controls: `[@role][:level]` and
  `[provider/]modelId[:level]`. The literal-id guard exists because the grammar
  is otherwise ambiguous with real ids.
- **Two surfaces for two scopes**: a compact session-only picker, and a
  full-screen hub where roles are *managed* rather than merely selected.

## What operant has today — measured

| Thing | State | Evidence |
|---|---|---|
| Model picker | Exists: inline overlay over configured `[agent] model` + `fallback_models` + assigned role slots + the fetched endpoint catalogue | `providers.rs:107-200` |
| Picker row type | Rich: `is_current`, `is_default`, `is_favorite`, `recommended`, `recommendation_rank`, `usage_score`, `old`, `created_date`, `effort`, plus per-option `provider`/`api_method`/`available`/`detail`/cost | `tui_fns.rs:478-492` |
| Favourites / default | `Ctrl+N` toggles, `Ctrl+O` sets default, in-picker `Shift+Tab` cycles favourites | `providers.rs:595-625` |
| Role vocabulary | 8 `ModelSlot`s: Conversation, Fallback, Recap, Vision, Compression, Memory, MoaAggregator, MoaReference — each a config key with a name and a description | `model_roles.rs:58-98` |
| Role rows in the picker | Assigned slots only, pinned to the top, resolving through the same `model_roles` seam the gateway's `/model` uses | `providers.rs:139-160`, `:519-557` |
| Effort enum | `Low / Normal / High / Max` — **4** | `model_effort.rs:7-15` |
| Effort wire vocabulary | `"" \| minimal \| low \| medium \| high \| xhigh` — **6, and `max` is absent** | `commands.rs:1356` |
| Effort ladder | `Shift+Tab` cycles Low→Normal→High→Max→Low, sending `medium`, `high`, `xhigh`, `""` | `key_handling.rs:17-33` |
| Effort picker | Exists: `/effort` opens a `PickerKind::Model` picker with 4 rows | `providers.rs:702-747` |
| Per-model effort capability | **Absent.** `model_effort.rs`'s own header claims `model_supports_effort` / `model_supports_max_effort`; `grep` finds them in that comment and nowhere else | `model_effort.rs:1-5` |
| Session-scoped model | **Absent.** `pending_model` writes `config.agent.model` and `agent.set_model` globally | `tui_app.rs:2041-2049` |

## Must fix first: the effort picker sets a model that does not exist

**The failing case.** Press `/effort`, arrow to `◕ high`, press Enter.

1. `apply_inline_picker_entry` takes the `PickerAction::Model` arm
   (`key_handling.rs:3320-3325`).
2. `pending_model = Some("◕ high")` — the row's **display label**, symbol and
   all, is set as the conversation model.
3. `pending_effort = Some("high")`.
4. The drain calls `agent.set_model("◕ high")`, then writes
   `app.model_name = "◕ high"` and `app.config.agent.model = "◕ high"`
   (`tui_app.rs:2041-2049`).
5. Every model display now reads `◕ high`, and the next request asks the provider
   for a model named `◕ high`.

Two of the four rows are worse still: they send wire values the operator-facing
validator rejects. `open_effort_picker_inline` builds rows with wire strings
`low | normal | high | max`, while `/effort-set` accepts
`"" | minimal | low | medium | high | xhigh` (`commands.rs:1356`). So `normal`
and `max` are rejected on the `/effort-set` path and are not in the wire
vocabulary on the drain path either. Only `low` and `high` are valid, and both
still clobber the model.

**This is M1 and it lands alone.** It is small, it is a live defect, and every
other tranche edits the same function.

## Gap ledger — dependency-ordered tranches

Each tranche lands in its own commit with its own `done when`.

### M1 — An effort row sets an effort and nothing else (the defect above)

**Change.** Give the effort surface its own `PickerKind` — `PickerKind::Effort` —
and its own apply arm that touches `pending_effort` only. Fix the row wire values
to the six the wire actually accepts, and make the four-row list carry what the
wire can express (see M2 for the enum itself; M1 only stops the clobber and the
rejection).

**Rejected alternative.** Keep one kind and branch on
`options[0].api_method == "effort"`. Rejected: that is the shape that produced the
defect — a surface identified by a field inside a row rather than by its own type.

**done when.** Selecting each of the four effort rows leaves `pending_model`
untouched and `app.model_name` unchanged, and every row's wire value is accepted
by the validator the `/effort-set` path uses. Guard: one test per row asserting
`pending_model.is_none()` after apply.

**Touches.** `tui_fns.rs` (the kind), `key_handling.rs` (the apply arm),
`providers.rs` (the row wire values), tests.

### M2 — The effort enum carries what the wire carries

**Problem.** `EffortLevel` has 4 values; the wire accepts 6 and lacks `max`.
`minimal` and `low` both collapse to `EffortLevel::Low`
(`commands.rs:1360-1364`), so once set they are indistinguishable, and the ladder
can never produce `minimal`. Meanwhile the enum's `Max` sends `xhigh`, so
`Effort::Max` is unreachable — operant's top is oh-my-pi's second-from-top.

**Change.** Make `EffortLevel` the six wire levels, named as the wire names them:
`minimal, low, medium, high, xhigh, max`. Keep `symbol()` and `label()`, add
`wire()` as the single source of the string, and delete every hand-written
mapping — `commands.rs:1360-1364`, `key_handling.rs:19-24`, and
`providers.rs:126-133` each spell the mapping out and are three chances to drift.

**Default that preserves behaviour.** The current default is `Normal`, whose wire
is `medium`; the renamed default is `Medium` with wire `medium`. **Byte-identical
on the wire**, so no behaviour change ships inside a rename.

**Three more disagreements between the enum and the wire, all measured, all in
`M2`'s scope:**

1. **"off" has no representation.** `cycle_effort`'s top arm is
   `Max => (Low, String::new())` (`key_handling.rs:23`): cycling past the top sends
   the *off* wire value while setting `effort_level = Low`. So the operator turns
   reasoning off and the display says `Low`, and the next press sends `medium`.
   The enum cannot express the state it is in.
2. **`/effort-set ""` leaves the enum stale.** Its `_ => self.effort_level` arm
   (`commands.rs:1364`) deliberately does not change the enum for `""`, so setting
   off through the command leaves `effort_level` reporting whatever it was before.
3. **`minimal` is unreachable**, as above: two wire levels collapse to one enum
   value.

Add an `Off` variant (wire `""`) so all three resolve to one state with one
representation. omp separates this differently — `ThinkingLevel` has both `off`
and `inherit`, and `ConfiguredThinkingLevel = ThinkingLevel | "auto"` keeps `auto`
*outside* the wire type so provider mapping only ever sees concrete efforts
(`tui/src/thinking.ts:143-152`). Operant needs `Off`; it does **not** need `auto`
(see non-goals).

**done when.** All six levels plus `Off` are reachable from the ladder and from
`/effort`, each sends its own name as the wire value, `Off` is the only state whose
wire is `""`, and one function is the only place the string is produced. Guard: a
table test over all seven asserting `wire()` and the drain's
`set_reasoning_effort` argument agree, plus one asserting the cycle's top arm lands
on `Off` rather than `Low`.

**Touches.** `model_effort.rs`, `key_handling.rs`, `commands.rs`, `providers.rs`,
`app/init.rs`, tests.

### M3 — Effort is clamped to what the model supports

**Problem.** Nothing checks whether the model can think at all. `Shift+Tab` to
`xhigh` on a model with `reasoning: false` sends a field the provider may reject,
and the operator learns from a failed request. This is the capability the
`model_effort.rs` header already claims and does not have.

**Change.** Port omp's rule (`model-thinking.ts:23-88`):

- A model carries `reasoning: bool` and an optional ladder of supported efforts.
  Operant's fetched endpoint catalogue (`fetch_endpoint_models`,
  `providers.rs:180-200`) is where this lands; a model with no ladder is treated
  as reasoning-incapable rather than as accepting everything.
- `get_supported_efforts(model)` returns empty when `!reasoning`, else the ladder.
- `clamp_effort(model, requested)` follows an explicit effort map when it sends
  the level to a served tier that reaches the wire unchanged, otherwise **climbs
  down to the nearest lower tier** — never up.
- The picker renders only the supported rungs on a model row, and the ladder
  refuses to climb past them.

**Rejected alternative.** Send the requested level and let the provider reject it.
Rejected: a per-request failure is a worse teacher than a picker that cannot offer
the rung, and omp already worked out the clamp order.

**done when.** On a model whose ladder is `[low, medium, high]`, requesting
`xhigh` resolves to `high`, requesting `minimal` resolves to `low`, and a model
with `reasoning: false` offers no rungs at all. Guard: the three cases as three
assertions on `clamp_effort`, plus a picker test that the rung list matches.

**Two additions from omp that make the capability real rather than nominal:**

- **A ceiling, separate from the ladder.** `getAvailableEffortSelectors`
  (`model-controls.ts:625-636`) cycles `off → auto → model-supported efforts → off`,
  omits `auto` when `model.thinking.unverified`, and filters the whole cycle by a
  configured ceiling. A subagent additionally gets `thinkingLevelCeiling` from
  `task.maxEffort` (default `max`) whenever the caller passed an effort hint
  (`task/executor.ts:4930-4932`, `task/settings.ts:628-642`), so **a subagent's
  effort cycle is capped** — and the ceiling rides into retry-fallback recovery so
  effort can never be clamped back up past it (`turn-recovery.ts:2119`).
  Operant's delegations are its subagents, so the same ceiling belongs on a
  `RowKind::Subagent`-spawned session.
- **Gate the surface on the capability, not just the rungs.** omp's `/effort` with
  no argument opens the picker **only if `model?.reasoning`**, else it prints
  "no thinking" (`builtin-modes.ts:889-892`). Operant should answer the same way
  rather than opening a picker whose every rung will be clamped away.

**Touches.** the endpoint catalogue fetch, a new `model_effort` capability module,
the picker render, the ladder.

### M4 — One grammar for model, role, and effort

**Problem.** `/model <x>` takes a model id. There is no way to say "the smol role
at xhigh" in one string, so role and effort each need their own control and the
composer cannot express a combined intent.

**Change.** Port omp's grammar (`model-selector.ts:34-58`), which is the piece
that makes the other tranches compose:

- `[@role][:level]` and `[provider/]modelId[:level]`.
- `split_thinking_suffix(pattern, min_colon_index)` splits a trailing `:level`,
  and takes a minimum colon index so a role-alias prefix (`@smol:xhigh`) splits at
  the right colon.
- **The literal-id guard is mandatory, not optional.** omp gates `:max` and
  `:auto` behind flags because real ids end in `:max` (`glm-4.7:max`); an ungated
  split silently reinterprets a real model. Operant's `/model` must consult the
  live catalogue before splitting.
- `resolve_model_selector(selector) -> ResolvedSelector { model, role, effort }`
  is the one entry point; `/model`, `/agent`'s model rows, and the picker all call
  it.

**done when.** `/model @smol:xhigh` resolves to the smol role's model at `xhigh`;
`/model glm-4.7:max` resolves to the literal id `glm-4.7:max` with no effort
suffix; `/model claude-opus-4-8` is unchanged. Guard: the literal-id case is the
one that proves the guard — assert it explicitly.

**Do not carry omp's naming trap.** omp has **two** functions called
`resolveSessionModelSelector` with different signatures and **different accepted
forms**, and its own `/switch` help text describes the wrong one:

| | Restore path | `/model` / `/switch` wrapper |
|---|---|---|
| Where | `config/model-resolver.ts:1782-1791` | `slash-commands/builtin-modes.ts:60-73` |
| Accepts | `provider/id` and `provider/id:level` only | fuzzy ids, `provider/id`, `@role`, `:level`, bare ids, `*` |
| Rejects | **bare ids and `@role`** | — |
| Why it differs | uses side-effect-free `hasConfiguredAuth` so a restore never refreshes OAuth or runs `!command` keys (`:1773-1781`) | resolves against `session.scopedModels`, i.e. the live available set |

Operant has the same split whether or not it names it: startup reads
`config.agent.model` as a literal id, while `/model` should accept the grammar. So
name them differently — `restore_model_selector` and `resolve_model_selector` — and
give the restore path no role expansion.

**Forms to accept, and their order.** omp's resolution order is the part worth
copying verbatim (`model-resolver.ts:2222-2234`, `:2153-2469`):

1. `provider/id` exact, authoritative against the **full** catalogue.
2. `provider/id:level` — exact base, then suffix split.
3. bare id (`sonnet`, `opus`) — flat id / full-selector match over the **available
   set only**, provider-locked selectors excluded, ranked MRU → provider order →
   provider MRU → deprioritized last → catalogue order (`:656-690`).
4. `@role`, legacy `pi/role`, bare `*` — alias via prefix length, then chain
   expansion.
5. `@role:level` — the alias prefix length is the colon floor, so `@a:b` ids
   survive (`:1268-1272`).
6. unprefixed role name — **only** if `settings.getModelRole(name)` is defined.
7. bare `model:level` — suffix split; a literal id ending in the suffix wins.
8. `model@upstream` — routing slug, honoured only when the base resolves to a
   routing-capable aggregator.
9. unknown → error with a same-provider "did you mean"; disabled provider →
   refused with an actionable error.

Two precedence rules are load-bearing: explicit `provider/id` and *authenticated*
bare ids **beat** configured role names, which beat unauthenticated catalogue-only
ids; and `findExactCliModel` passes `catalogFallback: false` so a bundled default
cannot shadow a configured `modelRoles.default` (`:2169-2175`, `:2309-2311`).

**Touches.** a new `model_selector.rs` beside `model_effort.rs`, `commands.rs`
(`/model`), the picker's apply path, tests.

### M5 — Session-scoped model and effort

**Problem.** Both are global. Choosing a model rewrites `config.agent.model`, so
trying a model for one conversation silently changes every other session, and the
operator cannot get back without remembering what was there.

**Change.** omp's split, which its own docs state as the point of the compact
picker: "Model entries switch the current session only"
(`model-picker.ts:1-6`). Operant:

- `App` carries `session_model: Option<String>` and
  `session_effort: Option<EffortLevel>`, seeded from config, **never written back
  to it**.
- `set_model` on the agent is session-scoped; the config write becomes an explicit
  `/model set-default` (or the existing `Ctrl+O`), so persisting is a separate
  act from switching.
- The status line distinguishes the two, because a session override the operator
  cannot see is indistinguishable from a global change they did not want.

**The persistence decision, which omp answers deliberately and operant must not
skip.** omp does **not** write a session-only switch to config; it appends an entry
to the **session log**:

- `ModelChangeEntry { type: "model_change", model: "provider/id", role?, resolvedModelIsFallback? }`
  (`session-entries.ts:148-156`, written `session-manager.ts:3704-3714`).
- `ThinkingLevelChangeEntry { type: "thinking_level_change", thinkingLevel?, configured? }`
  (`session-entries.ts:138-146`).
- Restore then reads the log, not the config: `getRestorableSessionModels`
  (`session-context.ts:139-156`) returns `[models[role]]` when the last
  `model_change` carried a **role**, else `[models.default]`.

The consequence is subtle and worth copying as a decision rather than an accident:
**a plain temporary switch is deliberately NOT restored on resume** — it has no
role key to restore under — while a *role* switch is. `ephemeral` ("fallback") is
treated like default (`:147`), and `lastModelChangeRole` skips it
(`session-manager.ts:533-548`).

Operant's choice: the session's `model` column (`database.rs:43`) already exists
and a plain read would restore *everything*, including a throwaway experiment.
**Default: follow omp — restore a session-scoped switch only when it was made
through a role, and let a one-off model experiment die with the session.** That
needs one extra bit on the session row (or a `model_role` column), so this tranche
is a migration after all; it is small, and it is the honest cost of the behaviour.

**One more rule from omp's dispatch** (`selector-controller.ts:727-737`): while a
subagent is focused (`ctx.focusedAgentId`), model switching is **forced
session-only** and never reaches the hub. Operant has focused subagents
(`focused_subagent`), so the same guard belongs here: a focused child gets a
session-scoped switch and no global write.

**Rejected alternative.** Keep one model and add a "temporary" flag. Rejected: a
flag on a global is still a global, and the resume path has nothing to read.

**done when.** `/model <x>` changes the current session only; a second session in
the same process keeps its own model; `config.agent.model` is byte-identical after
the switch; a role switch survives resume and a plain one does not; switching while
a subagent is focused writes no config. Guard: two sessions, two switches, assert
each independently — a global write passes a one-session test.

**Touches.** `app/mod.rs`, `tui_app.rs` (the drain), `commands.rs`, `database.rs`
(one column), `adapter_types/model_registry.rs`, tests.

### M6 — Roles as assignments with acceptance predicates

**Problem.** A role in operant is a config key with a description. Nothing stops
the operator assigning a model the role's consumer cannot use, and the picker's
role rows carry no statement of what the role accepts. omp's roles each carry an
`accepts(model)` predicate (`model-roles.ts:36-51`), which is what makes its Roles
view able to filter the catalogue before the operator chooses.

**Change.**

- Give `ModelSlot` an `accepts` predicate over the model metadata M3 introduces:
  at minimum "chat-shaped" for `Conversation`, `Fallback`, `Recap`, `Memory`, and
  `Compression`; an image-capable model for `Vision`; nothing narrower until a
  consumer demands it.
- The picker filters a role's candidate list through it, and says so when the
  assigned model does not satisfy it rather than silently showing a broken row.
- Add a **Roles view** to the models page: one row per slot with its current
  model, and Enter to assign. This is omp's Roles view and the "integrated with
  the various roles" the request names.

**Rejected alternative.** Keep roles as picker rows and add no view. Rejected: the
rows already exist and the operator's complaint is that the page is not a *system*
— a list of eight rows interleaved with models is not a role surface.

**What a role is allowed to change — settle this before writing the view.** omp's
answer is narrow and worth copying: **a role is only a named selector string.**
`applyRoleModel` (`model-controls.ts:426-431`) calls `setModel(entry.model,
entry.role)` and then `setThinkingLevel` **only if the selector carried an explicit
`:level`**. It does not touch the system prompt or the tool set; those roles'
consumers map the same id elsewhere (task → subagent dispatch, commit → the commit
message generator, advisor → session advisors).

That matches operant's existing rule and is the reason the non-goal below is
correct: `ModelSlot::Conversation` is already documented as "the ONLY slot whose
selection also switches the live agent", and omp states the same thing at runtime —
assigning `default` from the hub calls `setModel(persist)` and changes the live
session, while **any other role writes settings only and never touches the current
model** (`selector-controller.ts:1019-1029`). Two independent implementations, one
rule.

**The definition layers, so a role is data rather than code.** omp resolves a role
in this order (`model-resolver.ts:1260-1307`, cycle-safe via a visited set):

1. the built-in per-role default **chains** from the catalogue's KDL rules
   (`catalog/src/compat/rules/runtime/behavior.kdl:427+`, read through
   `roleDefaultModels()`), which are ordered pattern lists — exact ids,
   provider-qualified ids, dot/dash aliases, bare family names, terminal globs.
2. `ROLE_CONFIGURED_FALLBACK` when a role has no configured value
   (`advisor → slow` configured-only, `memory → tiny`, `tiny → smol`,
   `widget → smol`) (`:1196-1201`).
3. user config `modelRoles.<role>`, `modelTags`, `cycleOrder`.

Operant's slots are config keys with no chain and no fallback, so an unassigned
slot is simply empty — which the picker already handles honestly
(`providers.rs:150-153`). **Default: do not port the chain.** It is a large
mechanism whose only job is to pick a model the operator never chose; operant's
slots are explicitly assigned or explicitly empty, and that is a clearer contract.
Recorded as a decision rather than a gap.

**`Ctrl+P` is the role-cycle key, and it is cheap to add once M6 exists.** omp binds
`Ctrl+P` / `Shift+Ctrl+P` to cycle the configured role models forward and backward
(`custom-editor.ts:143-144`, `input-controller.ts:715-716`), over `cycleOrder`
(default `["smol","default","slow"]`), skipping unavailable roles
(`getRoleModelCycle`, `model-controls.ts:410-424`). No picker. Operant's 8 slots are
a natural cycle list; this is the one piece of M6 that is a key binding rather than
a view, and it should land with it.

**done when.** Assigning a non-image model to `Vision` is refused with the reason
named; the Roles view lists all 8 slots with their current assignment; an
unassigned slot says so rather than offering an empty row (the rule
`providers.rs:150-153` already states); assigning a non-`Conversation` slot leaves
the live conversation model untouched. Guard: the last one is the rule — assert the
live model is unchanged after a non-Conversation assignment.

**Touches.** `model_roles.rs`, `providers.rs`, `key_handling.rs` (the cycle key),
the picker render, tests.

### M7 — The page: scopes in a sidebar, models in the body

**Problem.** Everything is one flat inline list: configured models, fetched
catalogue, and role rows, in insertion order, with a single marker column that
shows **one** of favourite / recommended / new / date / default because the ladder
is `else if` (`ui_inline_interactive.rs:60-88`). A row that is both a favourite
and recommended shows only one fact. And three carried fields are never rendered
at all: `usage_score`, `recommendation_rank` and `estimated_reference_cost_micros`
appear in `operant_ui/` **only** as struct-literal initialisers in tests —
`usage_score: 0` at `:1101`, `:1203`, `:1231`, `:1277` and so on — with no reader
anywhere in the render path.

**Change.** Split along omp's seam (`model-hub.ts:1-12`): a **scope sidebar**
(recently used · roles · all models · one entry per provider) beside a single
**browser body**, and keep the compact picker as the session-only fast path. In
the body, replace the `else if` marker ladder with a set of independent chips so a
row can be a favourite *and* recommended, and render the fields already on
`PickerEntry`.

**Sequencing note.** M7 is the largest and the least urgent. M1-M5 fix defects and
add the grammar; M6 adds roles; M7 is the layout. **An operator who stops after M5
has a correct system in the existing layout.**

**done when.** The scope sidebar lists every provider plus roles; a row that is
both favourite and recommended shows both; `usage_score` and the reference cost
appear on the rows that carry them. Guard: a two-fact row asserting both chips,
which the current `else if` cannot express.

**Six mechanics from omp's body that are cheaper to copy now than to invent later.**

1. **Hydrate synchronously, refresh in the background, rebuild only if the catalogue
   actually moved** (`model-picker.ts:195-216`). The comment at `:195-197` names the
   bug it prevents: *the first Enter is dropped while the refresh is pending*.
   Operant has exactly this shape — `model_picker_fetch_pending`,
   `model_fetch_rx`, and a `fetching` flag (`providers.rs:169-200`,
   `channel_drains.rs:346-362`) — so this is a live hazard, not a hypothetical.
2. **One comparator for both sort and search rank.** omp's `modelItemComparator`
   takes a `skipRoleRank` flag (`:355-410`, `:650-663`) so MRU and version order
   define the browse list *and* the search tiebreak. Two orderings for one list is
   how a search result appears to ignore the operator's own recency.
3. **Affinity must exclude auto-selected roles.** `:571-583` — "Auto-selected roles
   are catalog policy, not evidence of user preference." Operant's role slots are
   all explicitly assigned today, so this is a rule to state before it is needed
   rather than a fix.
4. **A separator is a disabled sentinel item, excluded by one predicate every path
   already honours** (`:1314-1338`, `#isDisabled` `:1238-1240`) — so no navigation
   path needs to know it exists. Operant's group headings are separate items, which
   works, but a sentinel is cheaper than a second item kind.
5. **A cancel ladder.** Clear a non-empty query first, then close (`:1438-1445`);
   the hub extends it to four levels (`model-hub.ts:2098-2110`). Operant's hub
   already does this for search (`key_handling.rs:665-672`) and the model picker
   should match.
6. **Two navigation models that do not fight**: the wheel **pans** and never moves
   the selection; the keyboard **snaps** (`:1691-1697`, `:1451-1457`). Hover gets a
   reserved background band; keyboard selection is a cursor glyph plus an accent
   name (`:1615-1619`).

**Column drop order is a documented priority, not a guess.** omp declares
`MODEL_PICKER_COLUMNS` with a hide priority (`Int` 1 · `t/s` 2 · `$/M` 3 · `Ctx` 4),
drops the performance column below 76 columns, `tps` from 76, full from 96, and below
`MIN_NAME_WIDTH = 16` drops COST then CONTEXT (`:812-817`, `:977-979`, `:1729-1733`).
The context icon trails its number so right-alignment pins to a fixed column
(`:881-891`). Operant's ladder should carry the same declaration rather than
dropping fields ad hoc.

**Two guards a port must not forget.** `contextWindow` can be `null`
(`types.ts:1753`) and every omp display path guards `if (ctx > 0)` — a naive port
prints `null ctx`. And `RECENT_LIMIT = 15` exists only in the hub
(`model-hub.ts:323`); the compact picker's recent block is unbounded
(`:654` vs the picker), so the two surfaces do not share a recency window.

**Sizing, if the compact picker is kept.** omp's is a bottom-anchored floating
overlay — `{ anchor: "bottom-center", width: "100%", maxHeight: "100%" }` with **no**
`fullscreen` flag (`selector-controller.ts:914-923`) — while the hub is the one that
passes `fullscreen: true` (`:211-222`). Height is `floor(rows * 0.4) - 4 chrome - 5
browser frame`, minimum 5 rows (`model-picker.ts:83-89`, `:302-305`).

**Touches.** a new `tui/model_hub.rs`, `ui_inline_interactive.rs`, `providers.rs`,
tests.

## Design decisions, with the rejected alternative

| Decision | Rejected | Reopens when |
|---|---|---|
| Effort gets its own `PickerKind` | Branching inside the model kind on a row field | The two surfaces ever need to share an apply path |
| `EffortLevel` becomes the six wire levels | Keeping 4 and mapping | A provider needs a level outside the six |
| `wire()` is the single source of the string | Per-site mappings | Never — three copies already drifted |
| Clamp effort **down** only, never up | Sending and letting the provider reject | A provider documents an up-map |
| Session scope is a field on `App`, not a flag on config | A `temporary` boolean beside the global | Resume needs to read it, which M5 already provides |
| Roles get an `accepts` predicate | Free assignment with a warning | A role's consumer widens its input |
| The grammar consults the live catalogue before splitting | Splitting on the last colon unconditionally | Never — `glm-4.7:max` is the counterexample |

## Non-goals

- No per-role system prompt. omp's roles are model assignments and change nothing
  else (`model-controls.ts:426-431`); a role that also rewrote the prompt would
  collide with the employee charter in the sibling plan.
- **No `auto` effort.** omp's `auto` classifies the prompt per turn — a 4-second
  budget, an "ultrathink" magic keyword that bypasses to Max, a Low floor, and a
  rule that it never picks Max provisionally (`model-controls.ts:667-730`,
  `thinking.ts:289-309`). That is a feature, not a level, and it is deliberately
  kept **outside** the wire type so provider mapping only sees concrete efforts
  (`thinking.ts:143-152`). Operant's `Off` is enough for this plan; revisit `auto`
  separately if the operator asks for it.
- **No role default chains.** omp resolves an unassigned role through catalogue KDL
  pattern chains and a configured-fallback table (`model-resolver.ts:1196-1201`,
  `:1260-1307`). Operant's slots are assigned or explicitly empty, which is a
  clearer contract; see M6.
- **No favourites work.** They are operant's own feature, not a port — omp has
  none. They keep their current behaviour and only gain an independent chip in M7.
- No service tiers, credit badges, or measured-performance columns. omp's
  `serviceTiers`, Factory-Droid credit cards, and `ModelBrowserPerf` (tps/ttft) are
  catalogue-funded features operant's endpoint catalogue does not carry.
- No provider login or key management — `/login`, the connect dialog, and
  `custom_provider_dialog.rs` stay as they are.
- No model benchmark, price comparison, or context-window accounting beyond
  displaying what the catalogue carries.
- No change to the gateway's `/model` routing seam (`model_roles.rs` is the shared
  contract and both sides must keep reading it).

## Open question, and the default if it goes unanswered

**Q:** Should the compact session-only picker be a *separate* overlay from the
models page, as in omp (`model-picker.ts` beside `model-hub.ts`), or one page with
a "session only" scope?

**Default:** one page with a scope. Operant's picker is already a single inline
overlay with a scope-ish filter, and a second overlay is a second surface to keep
in sync — the defect M1 exists because two surfaces already shared one apply path.

## What changes when this is executed

Nothing yet. This file implements nothing. Executing it lands seven commits, M1 → M7.

**M1 is independent and should not wait.** It fixes a live defect in a function
every other tranche edits, and it is small enough to land on its own.

**M1 is also the sibling plan's blocker.** T4 there takes `Shift+Tab`; M1 here is
what makes effort survive that. Land M1 (and ideally M2) before or with T4.

M1-M3 together are the smallest correct unit: the defect, the vocabulary, and the
capability check. An operator who stops after M3 has honest effort control on the
existing page, and the layout work in M7 is then cosmetic rather than corrective.
# Model roles — which slot a selection means

Owns: the named model slots (`ModelSlot`) that exist in operant's config, the
ONE selection resolver every surface calls so `/model` cannot drift, the runtime
model catalogue (live endpoint → models.dev → configured), and the
`@role[:level]` selector grammar with the effort axis.
Not for: the provider clients and credential rotation
([providers.md](providers.md)), the fallback/failover decision itself, or the
provider's own model list.

Source: `crates/operant-core/src/model_roles.rs` (the slots, the resolver, the
presentation helpers), `crates/operant-core/src/model_catalog.rs` (runtime
discovery: live, models.dev, configured),
`crates/operant-core/src/models_dev.rs` (the capability/pricing catalog and its
disk cache), `crates/operant-cli/src/tui/model_selector.rs` (the `@role[:level]`
grammar), `crates/operant-cli/src/tui/model_effort.rs` (the effort ladder and a
provider's support for it).

## The rule

> A "model role" is a NAMED MODEL SLOT plus the workload it serves. Operant
> already has slots; this module names them so a selection can say WHICH slot it
> means, and so the TUI's `/model`, the gateway's `/model`, and any future
> surface resolve an operator's words through ONE function instead of three that
> drift.

Two behaviours carry the semantics:

1. **`Conversation` is the only self-switching slot.** Selecting it moves the
   live agent AND persists `[agent] model`. Every other slot is a plain
   assignment: the conversation keeps running on whatever it was on, and the
   slot's own consumer picks the new model up on its next call.
2. **An unknown role is not a silent success.** Operant has no priority chain
   and must not invent one here, so an unknown name is an error that NAMES the
   close matches — the operator corrects it, rather than discovering a typo as
   a provider failure three turns later.

## The eight slots

`ModelSlot` has exactly eight variants, and `ALL_SLOTS` lists them in the order
the picker and the channel command present them:

| Slot (name typed) | Config it maps to | Notes |
|---|---|---|
| `default` (`conversation`, `agent`, `chosen`) | `[agent] model` | the ONLY self-switching slot |
| `fallback` (`fallbacks`) | `[agent] fallback_models` | list-shaped; a selection replaces the chain's head only |
| `smol` (`recap`) | `[recap] model` | the idle-recap model |
| `vision` | `[auxiliary_models.vision] model` | image analysis |
| `compression` (`compress`) | `[auxiliary_models.compression] model` | context auto-compression summaries |
| `slow` (`memory`) | `[auxiliary_models.memory] model` | the memory-routing post-processor |
| `aggregator` (`aggregate`) | `[moa] aggregator model` | the Mixture-of-Agents synthesizer |
| `reference` (`references`, `advisor`, `advisors`) | `[moa] references` | APPENDS; the one append-shaped slot, so it is not interchangeable |

`ModelSlot::parse` is case-insensitive and accepts the canonical name, the
config's own path spelling (`auxiliary_models.vision` → `vision`), the omp-parity
aliases, and returns `None` for an unknown word — never a best guess.
`assigned_models(cfg)` reads the live config for a slot (empty is a normal
first-run state, NOT a reason to refuse a picker row); `persist_key()` is the
ONE place a slot maps to a config key, so a surface that persists a role cannot
write a key nothing reads — that is the defect the six inert
`[auxiliary_models.*]` slots are the record of.

## The one resolver

`resolve_selection(word, cfg, effort)` resolves in a fixed order:

1. `word` parses as a `ModelSlot` role name → that slot with its first assigned
   model. `smol` means "use what `[recap]` already holds". A known role with
   NOTHING set refuses with `SlotUnassigned` rather than falling through to the
   direct-id path, because that would silently switch the CONVERSATION when the
   operator asked for the recap slot.
2. `word` exactly matches a model already assigned to some slot → that slot with
   that model (a literal id wins, so a model literally called `slow` is never
   misread as the memory role).
3. Otherwise `word` is a DIRECT id for the conversation slot — the main use of
   the command, and requiring the id to already be configured is how a typo
   becomes a dead turn.

`ResolutionError` names what to try: `SlotUnassigned` (set the slot),
`NotSingleValued { slot, count }` (a list-shaped slot; say which index, or set
it to one model), `EmptyModel`. `slot_summaries(cfg)` returns the slots that
hold at least one model in `ALL_SLOTS` order for a picker's rows — an unassigned
slot is OMITTED rather than shown empty — and `describe_slots(cfg)` renders the
one-line-per-slot reply a channel command gives, marking each with
`(assignment only)` unless it switches the conversation.

## The selector grammar (`model_selector.rs`)

`resolve(selector, available)` is the TUI's half of the same idea, and it is one
string rather than three controls:

```text
@role[:level]              the role's model, optionally at a level
role[:level]               same, when `role` is a known role name
[provider/]modelId[:level] a specific model, optionally at a level
```

Two omp rules are load-bearing and are why this is not a `split_once(':')`:

1. **The literal-id guard.** Real model ids END IN a level (`glm-4.7:max`), so
   an ungated split silently reinterprets a real model as a model plus a level.
   The whole selector is checked against the live catalogue BEFORE anything is
   split off it — so `split_thinking_suffix` takes a minimum colon index, and
   the resolver consults `available` first.
2. **A colon whose tail is not a level** is either an extended id (`x:free`,
   `x:beta`) or a mistyped level. The two are told apart by what sits BEFORE the
   colon: when that is a model the catalogue lists, the operator appended a
   suffix to a real model, so the tail was meant as a level and the error names
   it. When it is not listed, the whole string is left alone as an extended id.

`SelectorError` carries the offending text, because "unknown model" without the
text is not actionable: `UnknownRole(String)`, `UnknownEffort(String)`,
`UnknownModel { typed, hint }` (the `hint` is the closest catalogue entry).
An empty `available` means the fetch has not resolved yet, so the resolver
accepts any `provider/id` rather than refusing on no evidence.

## The effort axis (`model_effort.rs`)

`EffortLevel` has seven rungs and `ALL` is the one list the ladder, the picker
and the parser iterate, so a new rung cannot be added to one and forgotten by
the others: `Off` (send no reasoning field at all, the default), `Minimal`,
`Low`, `Medium`, `High`, `XHigh`, `Max`. `wire()` gives the string sent on the
wire (`off` is the empty string) and `from_wire()` is the ONLY parser.

The axis is deliberately SEPARATE from the role: `model_roles::EffortRef` is
either `Unspecified` or `Override(String)`, and the resolver never invents a
level. A `:high` suffix on a role value would make one string mean two things
and break the moment an id contains a colon.

`effort_support(provider, model)` reports whether a provider accepts effort at
all (`EffortSupport::allows_effort`, with a `refusal()` sentence for the ones
that do not), and `clamp_effort(provider, model, requested)` maps a requested
level onto the levels a provider actually accepts — so a model that tops out at
`high` is not sent `xhigh`.

## Runtime discovery (`model_catalog.rs`)

There is no compiled-in model list. `resolve(req)` builds the catalogue in
order:

1. **Live** — one GET against the provider's own `/models` endpoint, chosen by
   `WireFamily` (`OpenAiCompat` with a bearer token, `Anthropic` with
   `x-api-key` plus `anthropic-version`, `Google` with the key in the query).
   `listing_request()` returns the URL, headers and query as DATA so the wire
   shape is testable without a network call, and the error string names the
   endpoint without echoing the key. The rows are parsed into `LiveModel { id,
   context_length, max_output }` — the listing already carried the limits, and
   the old parser threw both away, which is why the runtime used to budget every
   model against one hardcoded 128k window.
2. **models.dev**, only for a mapped provider (`provider_to_models_dev`, 13
   providers) and only when live returned nothing.
3. **Configured** models are always appended, deduplicated.

`CatalogSource` (`Live`, `ModelsDev`, `Configured`) rides on every `CatalogRow`,
so a caller can say where a model came from, and `Catalog::notice` explains why
the live list is missing so the UI can say so instead of showing a short list
with no reason. `default_model(catalog, configured, supports_tools)` picks the
configured model when it is in the catalog, otherwise the first row whose
models.dev entry supports tools, otherwise the first row.

`models_dev::cached_capabilities(provider, model)` reads the disk cache for
`context_window`, `max_output`, `supports_tools`, `supports_vision`,
`supports_reasoning`, `supports_structured_output`, the modalities and the
per-million input/output costs; `cached_model_prices()` maps every cached row's
id to its price pair. Both read the cache only and return empty when there is
none — a missing cache is a miss, never an error.

## Surfaces

`/model` is handled in the TUI (`tui/app/commands.rs`) and over the gateway
(`gateway_commands.rs`), and a bare `/model` opens the picker (the resolver
returns an empty `ResolvedSelector`, which means "open the picker"). The TUI's
apply and the gateway's `/model` both funnel through `model_roles::resolve_selection`
so "what does `/model smol` mean" has exactly one answer.

## Failure modes

| What breaks | What you see |
|---|---|
| `/model smol` with `[recap]` unset | `ResolutionError::SlotUnassigned` naming the slot and what it serves, not a silent success |
| `/model fallback` when the chain holds several models | `NotSingleValued { slot, count }` — say which index, or set the slot to one |
| The live `/models` endpoint is unreachable | the catalog falls back to models.dev (mapped providers) then configured, with `notice` naming why |
| The provider is unmapped | no models.dev fallback is attempted; configured models only |
| A model id requested at a level the provider does not serve | `clamp_effort` maps it, or `EffortSupport::refusal()` explains |
| A typo'd bare word | `UnknownModel` with the closest catalogue entry as `hint` |
| A `:notalevel` suffix on a real model | `UnknownEffort` naming the tail, so `gpt-5:free` is diagnosed rather than guessed |

## Intention

The slots already existed in `AppConfig`; what did not exist was a way to name
one. The vocabulary is a deliberate contraction of oh-my-pi's `ModelRole` set
onto operant's live slots: omp has 11 chat roles each backed by a
~38-candidate priority chain, operant has nine flat config slots of which
`AppConfig::unused_setting_warnings` reports several as inert. Both facts bound
this module — the role list is operant's, and it must not name a slot nothing
routes to.

What is deliberately absent: `@role` alias syntax on the resolver (that is the
selector's, and it is why the two modules are separate), priority chains, and
presets — each needs a precedence model operant's config does not have. A
missing slot (`None` in config) means "unassigned", not an error, because a role
a config never mentions is a normal first-run state.
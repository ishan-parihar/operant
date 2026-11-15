# Plan — a models panel and a settings panel that own what the slash commands do

**Date:** 2026-11-15
**Revision:** 2 — supersedes revision 1 in place. What changed: the registry
count was wrong by 4.6x (83 → **385**), which made the inert-key count wrong
too (5 → **41**); the `live` filter's source moved from the warning body to the
field's own doc comment, because the body builds its messages at runtime and
cannot be searched for keys, and because a peer parked two keys whose doc blocks
say so while the warning body does not; and the `Recent` scope was re-priced
after measurement showed operant tracks no model recency at all. Tranches S1–S5
are otherwise unchanged in shape and order.
**Base:** `origin/main` @ `733ea7de`
**Status:** outline. Writing this file executes nothing. Revision 2 is the
execution base: S1 starts from the counts and the `live` source stated here.
**Sibling:** [plan-2026-11-15-model-picker-hub-and-effort.md](plan-2026-11-15-model-picker-hub-and-effort.md)
(M1–M7, shipped) and
[plan-2026-11-15-session-hub-surfaces-and-agent-identity.md](plan-2026-11-15-session-hub-surfaces-and-agent-identity.md)
(T1–T7, shipped). This plan builds on both and supersedes neither.

## What the operator asked for

"Check how oh-my-pi implements the model-picker and setting page, and create a
plan to implement that level of model-picking and settings panel systems. This
would replace the existing overlay based slash-commands, while retaining it for
the gateway channels."

Plus two riders: the model-picker panel must carry "the nuance of the various
agent-role models"; and the settings panel must be "just as nuanced as the omp,
but based on what operant actually operate on."

Three deliverables: a models **panel** at omp's level, a settings **panel** at
omp's level, and a rule for which slash commands those two panels replace — with
the gateway keeping the commands.

## The two reference measurements this plan is built on

Both references were read at `oh-my-pi` @ `19a76233fb`. They are the yardstick,
not a porting target: omp is TypeScript with a settings registry of 613 entries;
operant is Rust with a generated key table of **385** (revision 2 correction:
this file first said 83, which counted one tuple form and missed the rest).

### oh-my-pi's settings panel is a projection, not a UI

| Measured | omp |
|---|---|
| `register()` calls in production | **613** |
| Declared with `ui` metadata | 448 |
| Rendered as panel rows | **415** |
| Tabs / sections | **10 / 46** |
| Widget kinds | **7** + 1 bespoke |
| Rows that are pure table rows | **~398 of 415** |

The panel is `createSettingsHost()` walking the registry and projecting each
entry through `entryToSettingDef`
(`packages/tui/src/overlays/settings-defs.ts:230-297`). Adding a setting is one
`register({...})` block plus one line in `DOMAINS`; only ~17 rows need a
hand-written arm (3 runtime-option, 7 preview, 2 compaction sentinels, 1
provider-limits, 4 path special cases).

The seven widget kinds: **toggle** (214), **enum inline cycle** (16), **option
list** (84), **number stepper** (54), **text** (44), **secret-text** (3),
**multiselect** (2, both ordered). There is **no** colour picker, keybinding
editor, path picker, action button, read-only row, or sub-page link.

Two behaviours worth copying exactly:

- **Every edit commits immediately.** No staging area, no Save button. What is
  transient is *preview* (7 settings), which reverts on Esc.
- **The panel edits the layered value and never an env-supplied one.**
  `SettingsHost.get` uses `Setting.layered()`, which skips env and runtime
  overrides, and appends "`$PI_X` overrides this setting while it is set" to the
  description instead of pre-filling the env value
  (`settings-ui.ts:49-60`). An env credential is never written into config.

### oh-my-pi's model picker is two surfaces, not one

| Surface | Key | Scope it can set | Persists? |
|---|---|---|---|
| **Compact picker** | `Alt+P`, `/switch` | session model · quick-role application · task-subagent model · select-only | **No — never writes config** |
| **Hub** | `Alt+M`, `/model` | role → model assignment, per-scope | Yes |

The compact picker has four *modes* in one pane (`session | role | task |
selection`), each with its own status hint and its own primary-action label
(`model-picker.ts:91-94`). The hub has a two-pane scope sidebar
(`recent | roles | all | <provider>…`), a Roles view, fallback-chain rows, and
`x` to clear an assignment.

**The role vocabulary is 16 roles, not 8** — `default, smol, slow, vision,
plan, commit, tiny, memory, task, advisor, widget, image, web, speech,
dictation, judge` — split into 11 *chat* roles and 5 *kind* roles, each with an
`accepts(model)` predicate.

**Role resolution is a priority chain, not a lookup.**
`resolveConfiguredRolePattern` (`model-resolver.ts:1260-1307`) walks: the role's
own configured value → `ROLE_CONFIGURED_FALLBACK` (`advisor→slow`,
`memory→tiny`, `tiny→smol`, `widget→smol`) → for `smol`/`slow` only, the
`default` role's value → `roleDefaults` from the catalog's `rules.json`. A role
with no value is `auto → <resolved>` and renders with a **hollow** dot against
the configured **filled** dot.

**Thinking rides inside the model string** — `provider/id:level` — so it travels
with the role value across scopes and fallback chains
(`model-selector.ts:81-83`).

## What operant already has (measured at `9c2daeb4`)

This is the part that makes the plan tractable: **most of the registry already
exists.**

### The settings registry exists and is generated

`crates/operant-core/src/config_registry.rs` — 1996 lines, **generated** by
`scripts/gen-config-registry.py` from the config structs' own doc comments, with
a `--check` staleness gate.

```rust
/// `(dotted key, default, note)`, sorted by dotted key.
pub const CONFIG_KEYS: &[(&str, &str, &str)] = &[ … ];  // 385 keys
pub const KNOWN_KEYS: &[&str] = &[ … ];
```

It already feeds **three** consumers: `operant config describe`, `operant
config notes`, and the agent-facing `config_tool`. So the "one source of truth"
property omp gets from `register()` is a property operant **already has** for
config keys — it just carries no `ui` metadata and nothing renders it.

### The model roles are already nuanced — more than the request assumes

`crates/operant-core/src/model_roles.rs` — **8 slots**, each carrying a name, an
`accepts()` capability predicate, a description naming its live consumer, a
`persist_key()`, and a `switches_conversation()` flag:

| Slot | Name | Consumer | Shape |
|---|---|---|---|
| `Conversation` | `default` | the live agent — **the only self-switching slot** | chat |
| `Fallback` | `fallback` | same-provider chain on error | chat |
| `Recap` | `smol` | idle status-line recap | chat |
| `Vision` | `vision` | image analysis | **image** |
| `Compression` | `compression` | context auto-compression | chat |
| `Memory` | `slow` | memory-routing post-processor | chat |
| `MoaAggregator` | `aggregator` | MoA synthesizer | chat |
| `MoaReference` | `reference` | MoA advisory — **the one append-shaped slot** | chat |

`RoleAcceptance` is three-state (`Accepts | Unverified | Refuses(why)`), so a
slot with no catalogue row is not indistinguishable from one that accepts.

The scope axis exists as data: `ModelScope::{Recent, Roles, All, Provider(_)}`
in `tui/model_scope.rs`, driven by `Ctrl+]` (`app/model_scope_nav.rs`), with
tests pinning that a role row belongs to `Roles` and to no provider.

### The write seam exists

`persist_config_key(dotted, raw_value)` → `persist_config_key_at(path, …)`:
`toml_edit`-based, **comment-preserving**, atomic via temp-and-rename, creating
the file owner-only when absent. **35 call sites** already use it.

### The panel pattern exists

`session_hub/` is a full-frame hub: 78k `mod.rs` + 88k `render.rs`, with
`HubFilterMode`, `HubGroupMode`, `HubSortMode` as three orthogonal axes,
`PaneFocus`, per-session async preview, inline rename, and an armed-close
gesture. It is registered at `DialogPriority::SessionHub = 190`.

## The four structural findings the plan turns on

### 1. The registry contains keys the config itself reports as inert

`AppConfig::unused_setting_warnings` names every key that parses and is read
by nothing. Measured at revision 2 against `CONFIG_KEYS` (385 keys):
**41 registry keys are inert** — 26 named by a `flag!(self.x != …, "key")`
invocation, plus three clusters compared through locals rather than `self`:
`auxiliary_models.*` (6), `observability.*` (6), `context_injection.*` (3).

```
agent.max_consecutive_tool_only   agent.session_reset
bylaws.grant_ttl_days             cli.max_help_lines
memory.agentmemory_url/secret     pk.enabled / sidecar_script /
                                  ndjson_socket / exec_timeout_secs
skills.template_name/_description socialization.enabled / schedule /
                                  turn_budget / pairs
tools.browser_binary_path         tools.event_channel_size
tools.tool_search.search_default_limit / max_search_limit
tui.compact_width / landing_title / medium_width /
    prompt_placeholder / refresh_rate_ms / show_iterations
auxiliary_models.web_extract / image_gen / embeddings /
                  search / code_execution / reasoning
observability.otel_endpoint / otel_service_name / otel_headers /
              runtime_trace_mode / runtime_trace_path /
              runtime_trace_max_entries
context_injection.global_quota / dept_quota / self_quota
```

A panel projected straight off `CONFIG_KEYS` would ship 41 controls that do
nothing — one in nine rows. The registry needs a liveness filter before it can
drive a panel, which is precisely the "phantom selection surface"
`model_roles.rs` was written to prevent.

**The filter's source cannot be the warning body alone.** Two reasons, both
measured:

- The body does not name keys as dotted literals — it builds each message from
  `format!` at runtime, so an exact-string search of the function finds **zero**
  registry keys. The keys are only recoverable from the `flag!` arguments
  (`self.tui.compact_width != …` paired with its `"[tui]"` label) and from the
  loop literals in the three cluster arms.
- **Peers are parking keys faster than the warning body is updated.** At
  revision 2, `agent.context_historian` and `agent.context_historian_max_span`
  were parked — their doc blocks say "PARKED … accepted and ignored", and
  `unused_setting_warnings` does **not** name them. A filter reading only the
  warning body would offer two dead controls.

So `live` is computed from the **field's own doc comment**, which is the source
the generator already reads for `note`. The parked markers are a small, stable
vocabulary — `PARKED`, `accepted and ignored`, `accepted but unread`,
`READ BY NOTHING` — and a field carrying one is `live: false`. The warning body
is the cross-check, not the source: S1's test asserts every key the body names
is also `live: false`, which catches a parking that forgot the marker.

### 2. There is no per-key live setter

`RUNTIME_CONFIG` is a `OnceLock<RwLock<AppConfig>>` with exactly one writer,
`install_runtime_config(config)` — a **whole-struct replace**
(`config.rs:3014-3018`). There is no `set_config_key`.

So a panel write is **persist, then re-install** — never a field mutation. That
is why the current settings screen's `context_window` arm is the only row that
touches `AppConfig`, and why its `persist_warning` distinction (applied but not
persisted vs. never applied) exists at all.

### 3. The settings screen is 8 rows and three widget kinds, hand-written twice

`settings_screen/state.rs:all_entries` builds 8 rows; `apply_and_save` matches
on the same 8 keys with a hand-written arm each; `keys.rs:toggle_or_cycle_current`
matches on them a **third** time. Adding a row is three edits in two files.

Current catalogue: `context_window` (Number), `output_style` (Enum),
`bar_style` (Enum), `reduce_motion` (Bool), `show_cwd` (Bool),
`show_git_branch` (Bool), `fileAutocompleteLimit` (Number),
`fileAutocompleteShowHiddenFiles` (Bool).

It also writes to **two different stores**: `settings.json` for the TUI's own
visual settings, `operant.toml` for `context_window` and `bar_style`. omp has
one.

### 4. The role vocabulary is deliberately contracted, and that must hold

omp has 16 roles on ~38-candidate priority chains. Operant has 8 flat slots and
**no chain** — `model_roles.rs` says so explicitly: *"Operant has no chain and
must not invent one here (`model_catalog::resolve` already ranks a catalogue; a
second ranker would be a second truth)."*

So "the nuance of the various agent-role models" is **not** "port 16 roles and a
priority chain". It is: give the 8 slots the *presentation* nuance omp gives its
16 — the filled/hollow assigned-vs-auto distinction, the per-slot acceptance
refusal, the per-slot consumer description, and an explicit unassign — without
growing a second resolver.

## The architecture

Two full-frame panels plus one compact picker, all reading the same seams the
commands read.

```
                       ┌──────────────────────────┐
   Alt+M / /model  ───▶│  Models panel (hub)      │── role assignment ──▶ model_roles
   Alt+P / /switch ───▶│  Compact model picker    │── session switch ───▶ pending_model drain
   Alt+, / /settings ─▶│  Settings panel          │── key writes ───────▶ persist_config_key
                       └──────────────────────────┘
                                  │
              every write goes through the SAME operant_core seam
              the slash command uses — so the channel keeps working
```

### The rule that keeps the gateway working

**A panel is a TUI front end over an existing `operant_core` seam. It never
reimplements the write.**

This is what makes "replace the slash commands, retain them for the gateway"
coherent rather than a fork. The command stays the channel's surface; the panel
is the TUI's surface; both call the same function. If a panel ever needs a write
that no seam provides, the seam is added to `operant_core` first and the command
is rewired to it in the same tranche — the panel never gets a private write path.

`RegisteredCommand` already carries `remote_only`, and the remote set is small.
The gateway's reachability does not change.

### Panel placement

Both panels are full-frame, following `session_hub/` rather than the inline
overlay: a `mod.rs` (state) + `render.rs` + `keys.rs`, registered in
`DialogPriority`, opened by a key and by a command, closed by Esc.

The inline picker **stays** for the compact path. omp keeps both surfaces for a
reason: the compact picker is a bottom-anchored overlay that never persists,
and the hub is where assignment lives. Collapsing them would make "switch this
session's model" and "assign the vision slot" the same gesture.

## Tranches

One commit per tranche, each pushed with the next free `iter-N` label, each
tranche's `done when` verified at that commit.

---

### S1 — The registry learns to describe a panel row

**Problem.** `CONFIG_KEYS` carries `(key, default, note)`. A panel needs to know
which tab, which section, which widget, what the options are, and whether the
key is live.

**Change.** Extend the generator (`scripts/gen-config-registry.py`) to emit a
second table beside `CONFIG_KEYS`, derived from the struct AST rather than
hand-written:

```rust
pub struct SettingRow {
    pub key: &'static str,          // "agent.context_window"
    pub default: &'static str,
    pub note: &'static str,
    pub widget: Widget,             // inferred from the field type
    pub section: &'static str,      // "agent" — the TOML table
    pub live: bool,                 // false when unused_setting_warnings reports it
}

pub enum Widget {
    Bool,
    /// An enum with ≤ 6 variants: cycle in place.
    Cycle(&'static [&'static str]),
    /// A number with a known range: stepper with a step.
    Number { min: i64, max: i64, step: i64 },
    /// Free text, parsed as the field's type.
    Text,
    /// A set of paths / strings.
    List,
}
```

Widget inference rules, from the field type:

- `bool` → `Bool`
- an enum with ≤ 6 variants → `Cycle`, options in declaration order
- an integer with a documented range → `Number`
- `String` → `Text`
- `Vec<String>` / `PathBuf` → `List`
- **anything else → omit the row.** A row the generator cannot type is a row the
  panel cannot edit, and omitting it is honest where a text box would be a trap.

Measured on the current `config.rs`: **16 `f64`/`f32` fields** (ratios and
thresholds — a text box would invite `0.9` where the code wants `0.85..0.98`)
and **100 enum-typed fields** are the two classes the inference must handle
explicitly. The f64s are omitted, which is the bulk of the honest-omission set;
the enums are mostly `Cycle` but a few have more than six variants and fall back
to `Text`, which the panel marks read-only-by-parsing rather than offering a
cycle through twelve options.

`live` is computed by parsing the *body* of `unused_setting_warnings` for the
keys it names, so the two cannot drift — the same trick the file header already
describes for defaults and notes.

**Why the generator and not a hand-written table.** omp's registry is
hand-written and its cost is visible: 33 declared settings are unrenderable
because a human picked the wrong schema type, and 117 have no `ui` at all. A
generated table cannot have that class of defect — a field added to
`AppConfig` appears in the panel or is *deliberately* omitted, with no third
state.

**Tabs and sections.** The `section` is the TOML table the key lives under
(`agent`, `tui`, `gateway`, …). The panel groups by section and orders sections
by a fixed list — no per-key tab assignment, because the config's own table
structure is already the grouping the operator recognises from `operant.toml`.

**done when:**
- `python3 scripts/gen-config-registry.py --check` passes on a clean tree.
- A new field added to `BehaviorSettings` appears in `CONFIG_KEYS` and in the
  row table with an inferred widget, with no hand edit.
- A test asserts every key `unused_setting_warnings` can name has `live: false`
  in the row table, and that no `live: false` row is absent from that list.

---

### S2 — The settings panel

**Problem.** 8 rows, three widget kinds, hand-written three times, two stores.

**Change.** A full-frame panel that walks the S1 row table.

```
┌─ Settings ───────────────────────────────────────────────┐
│  agent            tui            gateway      …          │  ← sections
├──────────────────────────────────────────────────────────┤
│  > Context window              262144                   │  ← Number
│    Total prompt and response capacity in tokens.          │
│    Model                       gpt-4                     │  ← Text
│    Reasoning effort            medium  ◀                 │  ← Cycle
│    Context compression         on                        │  ← Bool
│    …                                                      │
├──────────────────────────────────────────────────────────┤
│  ↑↓ move · ←→ cycle · Enter edit · / search · Esc back    │
└──────────────────────────────────────────────────────────┘
```

Four widget kinds only — `Bool`, `Cycle`, `Number`, `Text` — because those are
what the generator can infer and what the config actually needs. omp's
multiselect and secret-text have no counterpart here: no config key is an
ordered set, and a credential typed into a panel would be written to
`operant.toml` in the clear, which the panel should not invite.

Behaviours copied from omp deliberately:

- **Commit on pick.** No staging, no Save. A `Bool` or `Cycle` writes when the
  operator moves off it; a `Number` or `Text` writes on Enter.
- **Env override is shown, not edited.** When `env_override_keys()` names the
  key, the row is read-only and its description carries "`$HERMES_MODEL`
  overrides this setting while it is set." Writing an env-shadowed key is the
  defect omp's `layered()` exists to prevent.
- **Inert rows are hidden by default and listed on demand.** A key with
  `live: false` is not rendered; `/` search can still find it, marked
  "accepted but unread", so the operator can see why it is not in the list
  rather than wondering where it went.
- **Two report states, kept distinct.** The existing `persist_warning`
  (applied, not persisted) versus `apply_error` (never applied) distinction
  survives, because `install_runtime_config` means a write can take effect for
  the session and still fail to reach disk.

The write path is one function, `apply_setting(key, value)`:

1. Validate against the row's widget (a `Cycle` refuses an unknown option; a
   `Number` refuses out-of-range).
2. `persist_config_key(key, raw)` — the existing seam.
3. On success, reload and `install_runtime_config` the changed config, so the
   panel reflects what is now live.
4. Report per the two states above.

**The two-store question.** The TUI's own visual settings (`theme`,
`reduce_motion`, `show_cwd`, `show_git_branch`, `file_autocomplete_*`) stay in
`settings.json`, because they are per-profile UI state that must not leak into a
shared `operant.toml`. They get their own section in the panel, and the panel
knows which store a key belongs to from the row table rather than from a match
arm. `context_window` and `bar_style` are the two that cross over, and they keep
writing to `operant.toml`.

**done when:**
- Every rendered row traces to a `CONFIG_KEYS` entry; a test asserts no row
  exists whose key is not in the registry.
- Every `live: false` key is absent from the default view and findable by
  search, marked unread.
- A `Bool`, a `Cycle`, a `Number` and a `Text` row each have a test proving the
  value reaches `operant.toml` and the live config.
- A row shadowed by an env var is read-only and names the variable.
- `settings_screen/` is deleted and `cargo test -p operant-cli --bin operant`
  is green.

---

### S3 — The models panel

**Problem.** The picker is a 3-column inline list with a scope axis reachable by
`Ctrl+]` but no sidebar, no per-slot assigned/auto distinction, and no unassign.

**Change.** A full-frame panel with a scope sidebar and a Roles view, following
`session_hub/`'s structure.

```
┌─ Models ─────────────────────────────────────────────────┐
│ │ Scope      │  Role        Model              Note     │
│ ├────────────┼──────────────────────────────────────────┤
│ │ Recent  3  │  default     gpt-4              ● live    │
│ │ Roles   8  │  fallback    gpt-4o-mini        ●         │
│ │ All   142  │  smol        gpt-4o-mini        ○ auto    │
│ │ ─────────  │  vision      —                  x clear   │
│ │ OpenAI  61 │  compression gpt-4o-mini        ●         │
│ │ Anthropic  │  slow        —                  x clear   │
│ │ …          │  aggregator  —                  x clear   │
│ │            │  reference   —                  x clear   │
├──────────────────────────────────────────────────────────┤
│ │ ↑↓ row · ←→ scope · Enter assign · x clear · Esc back  │
└──────────────────────────────────────────────────────────┘
```

**Roles view is the whole point.** One row per slot, showing:

- `●` filled — the slot holds a configured model;
- `○ auto` hollow — the slot is unassigned and its consumer falls back;
- `—` — unassigned with no fallback;
- the slot's consumer description on the detail line, so a live slot is
  distinguishable from a name that merely parses;
- `x` to clear.

`Conversation` is marked `● live` because selecting it switches the running
agent — the one asymmetry in the slot set, and the one the operator most needs
to see.

**Scope sidebar** is `ModelScope` as it already exists: `Recent · Roles · All ·
<provider>…`. Two additions:

- **`Recent` needs a recency source, because operant has none.** `contains()`
  returns `false` for `Recent` because "the picker carries no recency window",
  and that is still true: `PickerEntry::usage_score` is declared, rendered as a
  chip when non-zero (`ui_inline_interactive.rs:99`), and **never set anywhere**
  — every construction site passes `0`, so the chip never appears. There is no
  model-use log, no MRU order, no `record_model_use`. So this tranche adds the
  smallest one that is real: record `(model, when)` on every applied model and
  every slot assignment into the existing per-profile store, then rank `Recent`
  by it. Writing a recency list is a new fact with a new write path, which is
  exactly why it is called out here rather than folded silently into the
  sidebar work.
- **`Roles` becomes the default scope when the panel opens from `/agents`**, and
  `All` when it opens from `/model` — the operator's entry point declares their
  question.

**Assignment flow.** Enter on a role row → the catalogue filtered by that slot's
`accepts()` predicate → Enter on a model → `persist_config_key(slot.persist_key(),
model)`, then the slot's `switches_conversation()` flag decides whether the live
agent is switched too. An acceptance refusal is shown at the moment of choosing,
naming what the consumer needs — the M6 behaviour, already built.

**No priority chains.** A slot with no assignment shows `○ auto` and its
consumer's documented fallback, which for every slot is a plain default rather
than a chain. The hollow dot communicates "not configured" without inventing a
resolver.

**What stays inline.** `Alt+P` / the compact picker keeps its job: switch this
session's model, nothing persisted. It gains nothing from the panel and loses
nothing to it.

**done when:**
- Every slot in `ALL_SLOTS` has a row, including `Conversation`, marked live.
- Assigning a slot writes its `persist_key()` and does not switch the
  conversation unless `switches_conversation()` is true.
- A slot whose `accepts()` refuses a model cannot be assigned it, and the
  refusal names the requirement.
- `x` on an assigned slot clears it and the row falls back to `○ auto`.
- `Recent` selects the models the catalog ranks highest, not nothing.

---

### S4 — The command crossover

**Problem.** "Replace the overlay-based slash commands, while retaining it for
the gateway channels" needs a rule, not a sweep.

**Change.** Classify all 114 registered commands and move only the ones a panel
actually subsumes.

| Class | Count | Disposition |
|---|---|---|
| **Settings-and-selection** — `/model`, `/models`, `/agents`, `/effort`, `/config`, `/settings`, `/theme`, `/permissions` | ~8 | Panel is the TUI surface; command still resolves for channels and typeahead |
| **Actions and reports** — `/stats`, `/doctor`, `/export`, `/context`, `/cost`, `/info`, `/version`, `/changelog`, `/hotkeys`, `/auth` | ~30 | Stay commands. A panel would be a report viewer, not a settings surface |
| **Session and turn control** — `/clear`, `/compact`, `/cancel`, `/resume`, `/rename`, `/rewind`, `/quit` | ~25 | Stay commands. Not settings; the session hub already covers the picker half |
| **Aliases and hidden** — `/models`, `/cls`, `/usage`, `/sessions`, `/todo` | ~30 | Untouched |

**The rule, stated once:** a command moves behind a panel only when its entire
effect is *setting a value that persists*. `/model` qualifies (assigns a slot);
`/stats` does not (produces a report); `/clear` does not (mutates a session).
Anything that is an action keeps its command, because a channel needs to trigger
it and a panel would add a keystroke without removing one.

The **typeahead and `/help` entries stay** for every command in the first
class. The panel is an additional front end, not a replacement — which is what
makes the gateway retention automatic rather than a special case.

**done when:**
- A test asserts every command whose effect is a persisted value is reachable
  both as a command and from its panel.
- A test asserts no command lost its `RegisteredCommand` row.
- `operant --help` and the gateway's command list are unchanged.

---

### S5 — Red-team and audit pass

**Problem.** Three new surfaces over a shared config, with a generated table
between them.

**Change.** A `/ponytail-audit` over the result, with the panel architecture
red-teamed on these specific seams:

- **Every panel write reaches disk and the live config, or says which it did
  not.** The two report states are the defect class this panel is most likely
  to reintroduce.
- **No panel row can write a key the config will not read.** The `live` filter
  is the guard; a row that bypasses it is the phantom surface.
- **The generated table and `unused_setting_warnings` cannot drift.** Both are
  parsed from the same source; the test is what proves it.
- **Env-shadowed keys are never written by a panel.** This is how a credential
  reaches a config file.
- **The gateway path is byte-identical before and after** for every command in
  the first class.
- **`Recent` ranks what was actually recorded, and says so when empty** — an
  invented order is the failure the current `contains()` comment warns about,
  and an empty `Recent` on a fresh install is the correct state, not a defect.

**done when:** the audit reports no must-fix finding, and every finding it does
report is either fixed in this tranche or named as deferred with a reason.

## What this plan deliberately does not do

- **Port 16 roles or a priority chain.** Operant has 8 live slots and one
  ranker. A second resolver is a second truth.
- **Move thinking into the model string.** omp's `provider/id:level` grammar
  makes one string mean two things; operant keeps the axis separate
  (`EffortRef::Override`), which is already settled.
- **Build a multiselect or secret-text widget.** No config key needs either,
  and a credential in a panel invites writing one to a shared TOML file.
- **Give the panel a private write path.** Every write goes through
  `operant_core`; a missing seam is added to `operant_core` first.
- **Retire the inline picker.** It is the compact surface and it never
  persists, which is a different job from the panel's.
- **Replace action commands.** Only value-setting commands gain a panel front
  end; the command stays for every surface.

## Open questions for the operator

1. **Panel opening keys.** `Alt+M` / `Alt+P` are omp's; this plan assumes them
   for the models panel and compact picker, and `Alt+,` for settings (the
   conventional "preferences" chord). Confirm, or name others.
2. **`Recent` depth.** omp uses `RECENT_LIMIT = 15`. The plan assumes the same
   number until the operator says otherwise.
3. **The `settings.json` boundary.** The plan keeps TUI visual state in
   `settings.json` and everything else in `operant.toml`. The alternative — one
   store — is cleaner and is a migration; say if you want it.
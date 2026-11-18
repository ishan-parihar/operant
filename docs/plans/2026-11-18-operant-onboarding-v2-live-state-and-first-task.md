# operant onboarding v2 — live-state driven, first-task-first

Dated-frozen: published `2026-11-18`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**Supersedes** `docs/archive/2026-11-18-operant-onboarding-as-a-first-class-subsystem.md`
(archived 2026-11-18). The predecessor's four-tranche mechanism is retired whole; this
plan is its replacement, written after an external review of that plan's own context
pack (hydrogen-xhigh, `/context-pack-review` round 1, archived at
`research/audits/context-pack-2026-11-18-onboarding-round1/` — RESPONSE.md, DELTA.md,
REFERENCE.md, LAW.md, lenses.md, DISPATCH.json).

**Base:** `origin/main` @ `ec8ec33b` (iter-2641). Every file, symbol and count below was
read from the working tree at that commit. Line numbers drift; the symbol names are the
contract.

**Pre-registration:** this plan is the execution outline for
`docs/audit/2026-11-18-the-operant-setup-and-onboarding-surface-audit.md` (the measured
state of the tree). Read that audit first; it is where every "already exists" claim is
established. This plan does not re-derive those numbers.

**What this plan is NOT:** it is not a port of n8n Nod. It is a port of the FOUR
behaviours the review's REFERENCE.md names — live model readiness (skip what already
works), one silent resume record, a first-task landing that always has cards, and a
profile that refuses secrets before it is written — applied to operant's actual
mechanisms. The review broke 18 of the predecessor's claims; each break is answered
below by a named change anchored to a verified symbol.

---

## What the review broke (and what answers it)

The review verdicts are 18 "breaks" + 1 "not in evidence" + 1 "holds" across Lenses 1-9
(RESPONSE.md). This table is the response contract: every break maps to a tranche in
this plan or to an explicit non-goal.

| review finding | verdict | answer in this plan |
|---|---|---|
| Lens 1 Port 1: cards must require live model readiness + connected sources, not tool registration | breaks | T1 (`endpoint_ok` from `context_window::discover`) + T3 card gates |
| Lens 1 Port 2: card must BE the prompt (label == text), <= 10 words, yolo default unsafe | breaks | T3 (`prompt_text == label`, no ellipsis, `taskProblem`-style validation) |
| Lens 1 Port 3: cmd_setup's sourcehound probe must share the doctor's detector | breaks | T2 (one probe helper; wizard + doctor call it) |
| Lens 1 Port 4: profile/free-text must pass redaction BEFORE storage or prompt assembly | breaks | T4 (`redact_sensitive_text` gate before write/inject) |
| Lens 2 T2 net-negative (cron-seat jargon on quick path, yolo Enter-default, TUI never reached) | breaks | T1 (live-state skip + safe default; governance off the default path) |
| Lens 2 T3 net-negative (200-LOC JSON + lock, inverted --replay) | breaks | T5 (one `onboarding` record inside `Settings`; silent resume) |
| Lens 3: T3 builds a third store; lock belongs in `persist_config_key_at` | breaks | T5 (no new store; flock in the surgical writer) |
| Lens 3: T3's state model ≠ REFERENCE §1-2 (owner/version-keyed, welcome never resumed, replay clears) | breaks | T5 (version-keyed, welcome-never-resumed, replay-clears) |
| Lens 4: T2's "before step_integrations" unreachable on quick path; TUI gate ignored | not in evidence | T1 (branch anchored at `cmd_setup.rs:403-405`, OUTSIDE `if !quick`) |
| Lens 4: T2 leaves yolo as Enter-default; TUI-first operators never see it | breaks | T1 (safe default posture; TUI gate covered) |
| Lens 5: T3 locks onboarding.lock but leaves `persist_config_key_at` unlocked vs gateway | breaks | T5 (flock in `persist_config_key_at`) |
| Lens 5: version:1 without a version-equality check; step names drift; no modelReady on resume | breaks | T5 (`ONBOARDING_VERSION` const; step vocabulary = wizard's own; live re-check) |
| Lens 6: T2 alone worse, T3 alone worse; T1 alone mildly better, T4 alone holds | breaks/holds | whole plan (T2 cut; T3 -> T5; T1/T4 keep with fixes) |
| Lens 7: cards fail <=10-word / concrete / connected-source / modelReady / fallback | breaks | T3 (all five: 10-word cap, sendable text, source gates, `endpoint_ok`, fallback pool) |
| Lens 7: label != prompt_text breaks "title is the prompt" | breaks | T3 (label == prompt_text enforced by test) |
| Lens 8: T1's local-inference/memory checks are not onboarding UX; move to a successor | breaks | T2 keeps sourcehound only; local-inference + memory deferred (named) |
| Lens 8: omitted REFERENCE behaviours (modelReady, version invalidation, conditional steps, silent resume, about-you, commonTasks fallback, N-dot chrome) | breaks | T1-T5 (all adopted in the shape this tree supports) |
| Lens 9: redaction.rs guards memory_wire egress, not build_frozen_prefix / config; no Luhn+issuer | breaks | T4 (redact before frozen-prefix write; card family named as an OPEN gap, not silently assumed) |
| Lens 9: "no new state machine" contradicted Finding 4/7 | breaks | T5 (fixes both stores' seams instead of adding one) |

**What still HOLDS from the predecessor:** T1-alone is mildly better (doctor reporting
sourcehound absence), T4-alone is net-better than `Vec::new()` — both survive here as T2
and T3 with their review-mandated fixes applied.

---

## Mechanism

Five tranches. Each is one iteration, each independently shippable, each names the file it
touches and the test that proves it. No new crate, no new dependency (libc is already a
dep), no new state machine beyond T5's version-keyed record.

The target UX (the review's "Optimal Onboarding" paragraph, translated to this tree):

> detect from live runtime state whether a working LLM provider/model already responds
> and skip model configuration automatically (re-routing to it only when broken), omit
> irrelevant admin/cron-budget steps up front behind a safe default posture with
> action-time confirmation, run one optional skippable "About you" elicitation that
> refuses secrets/cards before writing a single redacted profile block in the frozen
> prefix, persist a single version-keyed step marker inside existing settings/config so
> interrupted runs resume silently (never back to welcome) while an explicit UI/CLI
> replay resets it cleanly, and land immediately in chat with three complete <= 10-word
> clickable prompts (guaranteed by safe fallback cards) whose title is the exact message
> sent and whose underlying model and connected sources are verified ready to run.

---

### T1 — the wizard runs on live state: skip the model step when one already works

**Problem (review Lens 1 Port 1/3, Lens 2, Lens 4).** The quick wizard
(`handle_setup_command` -> `effective_quick = quick || !reconfigure`,
`cmd_setup.rs:63`) always runs `step_provider_and_model` (`cmd_setup.rs:442`) and then
jumps straight to `step_integrations` (`cmd_setup.rs:403-405`, outside `if !quick` at
`:365` — the review's "not in evidence" break was a pack artifact; the call IS outside
the branch, and this plan anchors to the measured lines). A returning operator with a
working endpoint and a valid key is asked the whole provider menu again. And the TUI's
first-run gate (`tui/adapter_types/tui_app.rs:119-129`) only checks
`has_credentials` (`auth_store.has_any_key()` OR `resolve_api_key()`,
`tui/app/init.rs:36-37`) — a key on disk counts as ready whether or not the endpoint
answers.

**Change (one decision point, two surfaces):**

1. **`cmd_setup.rs` — a live-readiness skip at the top of the `None` arm.** Before
   `step_provider_and_model`, call a new helper `provider_ready(config) -> bool` that
   resolves the endpoint the config ALREADY names (`client.base_url` + `client.api_key`
   + `agent.model`) and asks `operant_core::context_window::discover` whether the
   endpoint answered (`Discovered.endpoint_reachable`, `context_window.rs:184`, TTL 600s
   `:279` — one fetch, cached, no extra round-trip on the happy path). When ready:
   print `print_success("Provider <name> and model <model> already respond — skipping
   provider setup.")`, write the readiness result into the T5 step record, and go
   straight to `step_integrations`. When not ready: the current flow, unchanged.
2. **`tui_app.rs` — the same helper behind the first-run gate.** The gate becomes:
   `!app.has_credentials || !provider_ready(&app.config)` — the connect dialog opens
   when there is no key OR the named endpoint does not answer. The status message
   distinguishes the two ("Connect a provider" vs "Provider <name> is not answering —
   check the endpoint or key").

**What is NOT here (review Lens 2's cut, honored):** no `step_governance` on the
default path, no cron-seat jargon, no yolo->scoped default flip. Governance stays
inside `if !quick` (`cmd_setup.rs:387`); the safe-default work is T1's own `approval_mode`
note below.

**Safe default posture (review Lens 4 "yolo Enter-default"):** the quick path does NOT
prompt for seat policy. Instead T1 adds ONE line to the wizard's closing summary
(`show_post_setup` / the `print_success("Setup complete!...")` block at
`cmd_setup.rs:419-430`): `print_info("Seat policy is 'yolo' (fully autonomous). Change
it any time with: operant setup --section governance")`. The choice is visible, not
forced; the cron-seat default stays the owner's deliberate call (iter-671 comment at
`cmd_setup.rs:1468-1472`, owner: "I do not want yolo mode to be gone").

**Files:** `crates/operant-cli/src/cmd_setup.rs` (one helper + one call site + one print
line); `crates/operant-cli/src/tui/adapter_types/tui_app.rs` (the gate condition).

**Tests (mutation-proven):**
- `provider_ready` is false for an unreachable endpoint and true for a reachable one —
  drive it against a closed local port and a `std::net::TcpListener`-backed stub, never
  a live network. Mutation: delete the `endpoint_reachable` check and the closed-port
  case must fail.
- The quick wizard skips `step_provider_and_model` when ready: build an `AppConfig`
  whose `client.base_url` points at the stub, drive `run_setup_wizard` with the stdin
  script that answers ONLY `step_integrations`' prompt, assert the provider menu never
  appeared (the wizard's own `print_page_header` output is captured).
- `the_first_run_gate_opens_when_the_endpoint_is_dead`: a config with a key but an
  unreachable base URL must still open `connect_dialog` (this is the TUI half; the
  existing gate test at `ui_prepare/tests.rs` must stay green).

**Risk:** `discover` is a network call on the first run of the day (10-min TTL). Bounded
by the same 15s-timeout fetch the agent loop already pays; a timeout degrades to
"not ready" (the wizard runs, as today), never to a hang.

### T2 — one sourcehound detector, not two (doctor + wizard agree)

**Problem (review Lens 1 Port 3, Lens 3).** `cmd_setup.rs:1191` probes
`tools::sourcehound::is_available()` and `post_setup.rs:135/142` probes it twice more;
`doctor.rs` never does (zero `sourcehound` mentions in the file, measured). `operant
doctor` reports all-green while every web tool will error.

**Change:** add ONE check to `diagnose()` (`doctor.rs:82`) and ONE shared helper.

- `check_sourcehound` in `doctor.rs`, beside `check_cli_tools` (`:825`), calling the
  EXISTING `crate::tools::sourcehound::is_available()` (`tools/sourcehound.rs:133`,
  which resolves via `find_sourcehound_binary` -> `SOURCEHOUND_BINARY` then PATH,
  `:117`). `ok` with the resolved path when present; `warn` naming
  `SOURCEHOUND_INSTALL_HINT` (`sourcehound.rs:74`) when absent.
- The wizard's `step_browser_and_skills` (`cmd_setup.rs:1191-1198`) and
  `post_setup.rs`'s two rows (`:135`, `:142`) keep their current UX but route through
  the same `is_available()` they already call — no second detector is introduced, and
  the doctor row is added to the drift pin below.

**Explicitly NOT here (review Lens 8's cut, honored):** `check_local_inference` (Ollama
11434 / llama.cpp) and `check_memory_wire` are OUT of this plan. They are named gaps in
the audit (finding 3), they do not serve onboarding UX, and they belong to a diagnostics
successor (a `doctor.rs` runtime-health round). Recorded here so nobody re-adds them to
this plan.

**Files:** `crates/operant-core/src/doctor.rs` only (`diagnose()` gains one line, one
function beside the existing seven).

**Tests (mutation-proven):**
- `diagnose()` emits a `warn` naming sourcehound when `SOURCEHOUND_BINARY` points at a
  nonexistent path; `ok` with the path when it points at a real executable. Mutation:
  delete the `check_sourcehound` call from `diagnose()` and the warn test fails.
- `engine_check_list_is_the_shared_drift_pin` (`doctor.rs:1019`) gains the new category
  in its expected list — the pin is the guard that this row stays in both consumers
  (CLI text render + gateway JSON).

**Risk:** the doctor text render grows one row. No exit-code change (warnings do not
gate `doctor_exit_code`; only `issues` do — `cmd_doctor/mod.rs:146-152`).

### T3 — the first-task landing: `suggestion_prompts()` authored against reality

**Problem (review Lens 1 Port 2, Lens 7 both, Lens 6).** The seam
`tui/operant_app/tui_state.rs:668` (decl) / `tui/app/tui_state_impl.rs:1244` (impl)
returns `Vec::new()`. The renderer (`ui_prepare.rs:978`) already handles the non-empty
case and its contract is tested (`ui_prepare/tests.rs:304` — the onboarding screen must
not promise numbered selection; the `[N]`-prefix and number-press claims were removed in
iter-1929 BECAUSE this seam was empty, and they stay removed).

**Change (deterministic, no LLM call, no boot I/O):**

- **Shape:** keep `Vec<(String, String)>` but enforce the review's contract:
  `label == prompt_text` (the title IS the prompt — "tapping sends exactly that title"),
  `<= 10 words`, starts with a letter, no trailing ellipsis, no template verbs with
  unfilled placeholders. A helper `valid_card(label, prompt) -> bool` encodes the
  `taskProblem` rules (`onboarding-suggestions.ts:149-151`: word count + letter-start;
  the connected-source check becomes the registry check below).
- **Gates (each card's prompt must name only tools that would run):**
  1. **Live model readiness** — `suggestion_prompts()` reads a cached
     `endpoint_ok: Option<bool>` written at TUI boot (T1's `provider_ready` result
     parked on `App`); when `Some(false)` or `None`, the list is EMPTY (never greet
     with a broken tool — the card would send a turn that cannot run).
  2. **Tool registration ∩ availability** — the prompt's named tools are checked
     against the live `ToolRegistry::get_schemas()` output (`tools.rs:902`:
     registered ∩ available ∩ not-disabled; `builtin_tool_names()`,
     `builtin.rs:328`, is the registration side). A card naming a tool the registry
     would hide is dropped, not shown-and-hoped.
  3. **Connected sources** — the cron/digest card requires the `cron` tool
     (`tools/cron_tool.rs`, registered when cronjobs are enabled) AND a delivery
     platform (gateway `webhooks` or telegram configured) — registration alone is not
     "connected", per the review's taskProblem rule.
- **Fallback pool (commonTasks analogue):** a fixed, always-valid set of 3 cards that
  require only the always-registered tools (`file_read`/`file_search` equivalents —
  operant's `aft_read_file`/`aft_search` when AFT is enabled, else the builtin
  read/search; verify against `get_schemas()` at render time, not at authoring time).
  When the gated set is empty, the fallback set fills the screen; when BOTH are empty
  (no tools at all), the list stays empty and the renderer's existing empty-state
  stands (it is honest: no tools, no cards).
- **Example gated cards (each passes the validator; final wording is the executor's,
  the contract is the test):**
  - `"Audit this repo's open findings and list the top three"` — file tools present.
  - `"Search the web for the latest operant releases"` — sourcehound present
    (web tools registered via the 23 composites).
  - `"Set up a daily digest of my open tasks"` — cron tool + delivery platform.

**Files:** `crates/operant-cli/src/tui/app/tui_state_impl.rs` (the method body);
`crates/operant-cli/src/tui/operant_ui/ui_prepare.rs` gains nothing (it already renders
the non-empty case; the `label == prompt` contract is enforced in the producer, not the
renderer). A helper module `crates/operant-cli/src/tui/onboarding_cards.rs` (~120 lines)
holding the validator + the gated set + the fallback set.

**Tests (mutation-proven):**
- An empty registry + no endpoint -> `Vec::new()` (never a card naming an unregistered
  tool).
- sourcehound absent -> no web card; present -> web card.
- Every produced card passes `valid_card` (property test: word count <= 10, starts with
  a letter, label == prompt).
- `endpoint_ok == Some(false)` -> empty list (the "never greet with a broken tool"
  invariant; mutation: drop the readiness gate and this fails).
- The fallback set is non-empty whenever ANY file tool is registered.
- `ui_prepare/tests.rs:304` stays green (no `[N]` promise returns).

**Risk:** per-frame work. Mitigation: the method reads two already-resident structures
(the registry + the cached `endpoint_ok`); NO I/O in the render path. If it needs I/O,
it belongs at boot (T1) — a card producer that blocks the frame is a defect.

### T4 — the "About you" profile: one redacted block in the frozen prefix

**Problem (review Lens 1 Port 4, Lens 9).** The predecessor assumed `redaction.rs`
guards profile data on its way to prompts. Measured: `redact_sensitive_text_if_enabled`
(`redaction.rs:441`) is called at memory egress (`memory_wire.rs:442/445/517/645`) and
the API message builder (`client.rs` via `redact_api_message_content`,
`redaction.rs:452`) — NOT at `build_frozen_prefix` (`agent/events.rs:1245`) and NOT on
config-resident data. A `<user_profile>` block (the audit's finding 8: the frozen prefix
is the right home, adjacent to `<employee_charter>` at `events.rs:1257-1263`) would be
the FIRST profile path, and it would bypass every existing scrubber.

**Change (one optional, skippable step; one redacted block):**

- **Elicitation** — a new optional wizard step `step_about_you` (own section name
  `about`, callable via `operant setup --section about`), OFF the default quick path
  (opt-in via the full wizard or the explicit section). Three questions max, every one
  skippable (`prompt_yes_no("Share a short profile for the agent?", false)` gates the
  rest), free-text capped at 200 chars (the REFERENCE's `profileTextChars`), a
  "Not now" exit that writes nothing.
- **The refusal gate (the review's core demand):** every free-text answer runs
  `redact_sensitive_text` BEFORE it is stored or injected; a refusal (the input changed
  under redaction -> it contained a secret pattern) DROPS that answer and says so. A
  second pass refuses digit runs that pass a Luhn+issuer card check — NOTE: the
  Luhn+issuer family does NOT exist in `redaction.rs` today (measured: zero `luhn`/
  `card`/`issuer` matches; the families are provider tokens, ENV lines, auth headers,
  private keys, DB connstrs, JWTs, Telegram tokens, phone numbers, URL userinfo,
  `redaction.rs:8-13`). T4 adds `redact_card_numbers(&str) -> (String, bool)` beside
  `redact_sensitive_text` (Luhn + issuer, the `memory-tree.ts:14-33` algorithm ported),
  called in the same gate. This is the ONE new scrubber in the plan, and it is a
  REFUSAL (drop the answer), not a MASK (the profile block is short; a masked card
  number is worse than a missing answer).
- **Storage** — the answers persist as a new `[agent] user_profile = "..."` key (one
  string, assembled from the kept answers, `<user_profile>`-ready) through the EXISTING
  surgical writer (`config::persist_config_key("agent.user_profile", ...)`, the
  `persist_config_key` pair at `config.rs:3345/3416`; the wizard's `persist_config`
  diff at `cmd_setup.rs:1308` picks it up with no new writer). The config template gains
  the key + a comment (`operant.example.toml`, per AGENTS.md's config-change rule).
- **Injection** — `build_frozen_prefix` (`agent/events.rs:1245`) appends a
  `<user_profile>` block after `<employee_charter>` when `config.agent.user_profile` is
  non-empty, byte-stable across turns (the frozen-prefix cache discipline holds: it
  changes only when the operator re-runs the step). The block is capped (500 chars, the
  same ceiling `<available_skills>` uses, `events.rs:1283-1285`) so a profile can never
  push the protected floor over a small window.

**Files:** `crates/operant-cli/src/cmd_setup.rs` (the step + its section arm);
`crates/operant-core/src/redaction.rs` (the card family); `crates/operant-core/src/
agent/events.rs` (the prefix block); `crates/operant-core/src/config.rs` (the
`user_profile` field on `BehaviorSettings`, `config.rs:751`, threaded into
`AgentConfig` via the `From<&BehaviorSettings>` impl at `agent/mod.rs:264` — the
field is set in `BehaviorSettings` because that is what `build_frozen_prefix` reads
through, and the `From` impl is the one place the two structs meet);
`operant.example.toml` (the key + comment).

**Tests (mutation-proven):**
- A free-text answer containing an `sk-` token is stored REDACTED or dropped, never
  verbatim (mutation: bypass the gate and the test fails).
- A 16-digit Luhn-valid card number is dropped with a "refused" notice; a 13-19 digit
  NON-Luhn run (an order id) is KEPT (the review's "long IDs no longer look like card
  numbers" property, pinned both ways).
- `build_frozen_prefix` includes the block iff `user_profile` is non-empty, and the
  block is byte-stable across two calls (the cache discipline).
- The quick wizard never prompts the step (it stays opt-in); `--section about` does.

**Risk:** a second scrubber (the predecessor's non-goal). Justification, stated so it is
re-readable: the existing families are PATTERN masks over unbounded text; the card
family is a CHECKSUM refusal over a bounded answer, and it exists because the review
proved the existing families do not cover it. If a future round folds card numbers into
`redact_sensitive_text`, this helper's CALL SITE stays and its body moves — the gate is
the contract, not the regex.

### T5 — one version-keyed resume record (no new store, no new lock file)

**Problem (review Lens 3 both, Lens 5 both, Lens 9).**
`Settings::has_completed_onboarding: bool` (`tui/adapter_types/config.rs:95`, written by
`persist_onboarding_complete`, `tui/app/turn_state.rs:272`) is the only live onboarding
state; `OnboardingConfig::seen` (`operant-cli/src/config.rs:1089`) is write-only
(`CliConfig::load` reads no YAML — measured: the only file read in `config.rs` is
`load_dotenv_file`, `:1334`; `seen` has zero readers). The predecessor's `onboarding.json`
+ `onboarding.lock` would be a THIRD store, would leave `persist_config_key_at`
(`config.rs:3416`) unlocked against a live gateway daemon (`gateway_runner.rs:2600`
holds `gateway.lock`), and would resume a CLI step list the TUI cannot drive.

**Change (one record, one lock):**

- **The record:** `Settings` (settings.json, the store the TUI and the wizard BOTH
  already read — `save_sync`/`load_sync`, `adapter_types/config.rs:115-118`) gains ONE
  field: `onboarding: OnboardingProgress` with `{ version: u32, completed: Vec<String> }`.
  `version` is compared against a `const ONBOARDING_VERSION: u32 = 1` in
  `adapter_types/config.rs`; a mismatch invalidates the record (empty `completed`),
  mirroring REFERENCE §1 ("a version bump invalidates the saved step"). Step names are
  the wizard's OWN section vocabulary (`provider`, `about`, `governance`, `terminal`,
  `tts`, `browser`, `gateway`, `agent`, `tools`) — the same strings the `Commands::Setup`
  `section` arg accepts (`main.rs:549-555`), so the record cannot drift from the wizard.
  `welcome`/fresh-install is never a stored step: an empty `completed` means "start at
  the first step", never "resume at welcome".
- **Writers:** `persist_onboarding_complete` (`turn_state.rs:272`) becomes
  `mark_step_done(step: &str)` — it sets the bool AND appends to `completed` (dedup,
  cap at the step count). The wizard calls the same function after each step's
  `persist_config` succeeds; the TUI connect flow (`providers.rs:377` — the only
  `persist_onboarding_complete` caller) passes `"provider"`.
- **Readers:** the TUI first-run gate (`tui_app.rs:119-129`, extended by T1) resumes:
  `completed` minus invalid steps = the steps to skip; the gate's condition becomes
  "no key OR endpoint dead OR steps remain". The CLI wizard's `--replay` semantics are
  REPLACED (review Lens 2's cut): `operant setup` resumes SILENTLY (skips completed
  steps, says which), and a new `--fresh` flag clears the record first (the REFERENCE's
  "replay clears the store", surfaced as an explicit CLI flag + a `/onboarding replay`
  TUI command the executor may defer — the flag is the contract).
- **The lock:** `persist_config_key_at` (`config.rs:3416`) gains an exclusive `flock` on
  `<operant_home>/operant.toml.lock` (sibling of `gateway.lock`'s path pattern,
  `cronjobs/lock.rs:28`), held for the read-validate-rename window. The gateway's
  `GatewayDaemonLock` (`gateway_runner.rs:2600`) is NOT reused (it guards the DAEMON,
  not the file); the new lock guards the FILE, which is the review's actual finding.
  Lock acquisition is non-blocking: a contended write fails with a clear "another
  process is writing operant.toml, retry" instead of blocking the wizard.

**Files:** `crates/operant-cli/src/tui/adapter_types/config.rs` (the field + the
version const); `crates/operant-cli/src/tui/app/turn_state.rs` (the writer);
`crates/operant-cli/src/cmd_setup.rs` (call sites + the `--fresh` flag + silent resume);
`crates/operant-cli/src/main.rs` (the flag); `crates/operant-core/src/config.rs` (the
flock in `persist_config_key_at`).

**Tests (mutation-proven):**
- Round-trip: write `completed: ["provider"]`, read it back; a version mismatch yields an
  empty record (mutation: drop the version check and the mismatch case must fail).
- Silent resume: a wizard run with `completed: ["provider"]` does not show the provider
  menu; `--fresh` clears and shows it.
- `welcome` is never stored: `mark_step_done("welcome")` is refused (assert the record
  is unchanged).
- The flock: two `persist_config_key_at` calls on the same path from two processes — the
  second fails with the retry message, and the file is left valid (mutation: remove the
  flock and the contended-write test must fail).
- `the_memory_wire_default_is_reported_ok` and the doctor pins stay green (no doctor
  change here).

**Risk:** a second store beside `operant.toml` — NO: the record lives in settings.json,
which the TUI already treats as its own (visual prefs + the onboarding bool), and it
holds PROGRESS, never settings. Every answer still lands in `operant.toml` through the
surgical writer. The line is "did the operator answer this step", and if a future reader
wants a setting from the record, that is a defect.

---

## Gates / success criteria

(fixed before the run — each tranche is one iteration and must clear all of these
before it is committed)

- `source scripts/dev-env.sh` then `cargo fmt --all` — clean.
- `cargo check -p operant-core --lib` and
  `cargo check -p operant-cli --bin operant` — **0 errors**.
- `cargo clippy -p operant-core --lib -- -D warnings` and
  `cargo clippy -p operant-cli --bin operant -- -D warnings` — clean; and
  `bash scripts/clippy-warning-gate.sh` — no NEW warnings.
- The tranche's own tests, run scoped: e.g. `cargo test -p operant-core --lib --
  doctor` for T2, `cargo test -p operant-cli --lib -- cmd_setup` for T1/T4/T5.
- `bash scripts/check-doc-claims.sh` — exit 0 (T4 adds a config key; the gate covers
  `operant.example.toml`).
- `./scripts/genome lint` — no NEW error vs this plan's base (measured at `ec8ec33b`:
  10 errors / 3 warnings, all pre-existing — E009 ledger, E104 AGENTS.md#stack stale
  zone, E303 voice.md's 3 stale TTS citations, E311's 5 uncited config modules; NONE
  from this plan's files. A new one is a regression.)

**Definition of done for the whole plan:** a fresh operator runs `operant setup`, is
asked for a provider+key only when the named endpoint does not already answer, never
meets cron-seat jargon on the default path, can opt into a redacted profile that the
frozen prefix carries, lands in a TUI whose empty state offers up to three cards whose
text IS the prompt and whose tools are all live, and after an interruption resumes at
the first unanswered step with no `--replay` incantation. `operant doctor` and the
wizard agree about sourcehound.

**Explicitly NOT in scope** (so nobody builds them): a Vault subsystem; a second
scrubber beyond T4's card family; a container runtime; an async provisioning phase; a
`memory.restate` analogue (memory-wire has `retain`); a per-step migration framework;
changing the default seat mode (the owner's call, iter-671); `check_local_inference` /
`check_memory_wire` (a diagnostics-successor round); the gateway or TUI render layer
beyond T3's card producer and T5's gate condition.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 | 2 (`cmd_setup.rs`, `tui_app.rs`) | ~80 | none (reuses `context_window::discover`) | low |
| T2 | 1 (`doctor.rs`) | ~40 | none | low |
| T3 | 2 (`tui_state_impl.rs`, new `onboarding_cards.rs`) | ~130 | none | low |
| T4 | 5 (`cmd_setup.rs`, `redaction.rs`, `events.rs`, `config.rs`, `operant.example.toml`) | ~180 | none (Luhn is std) | medium (new config key + frozen-prefix block) |
| T5 | 5 (`config.rs` TUI, `turn_state.rs`, `cmd_setup.rs`, `main.rs`, `config.rs` core) | ~160 | none (libc already a dep) | medium (flock semantics) |

Five iterations. Each independently revertable. T1 must land before T3 (cards need the
readiness cache) and before T5's gate (the gate reads T1's helper); T2 is independent;
T4 is independent of T3; T5 depends on T1's helper for the gate but can land its
record/lock without it.

## Order and dependency

**T2 → T1 → T3 → T4 → T5.** T2 first because it is the smallest and unblocks the
doctor/wizard agreement the rest cite. T1 second because T3 and T5 both read its
`provider_ready`. T3 third (the first-task landing is the review's highest-UX-leverage
item). T4 fourth (the profile block is independent of the cards). T5 last (the record
and the lock are the riskiest, and they can land against a tree where T1-T4 already
shipped — the record simply has more steps to skip).

## Resume here

**Landed (verified on `origin/main`, not inferred):**

- Nothing yet. This plan is published at iter-2642; the first tranche (T2) is the first
  iteration that will append here.

**Open:** T1, T2, T3, T4, T5.

Start at T2. `research/audits/context-pack-2026-11-18-onboarding-round1/RESPONSE.md`
holds the review's 18 breaks with the evidence for each; this plan's table above is the
response contract.
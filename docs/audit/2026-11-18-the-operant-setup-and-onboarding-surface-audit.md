# the operant setup and onboarding surface audit

Dated-frozen: audited `2026-11-18`; findings mark verified vs asserted.

**Audited revision:** `origin/main` @ `3d8defbb` (`git log -1 --format=%cI`
= 2026-11-17T13:48:24+05:30). Every file, line number and count below was read
from the working tree at that commit. The tree compiled clean before the audit
(`cargo check -p operant-cli --bin operant`: **exit 0**, 8 warnings — the same
warning count `docs/CODEBASE-REALITY.md` measures), so a claim below cannot be
blamed on a broken checkout.

**Why this audit exists.** An external proposal asked operant to adopt n8n
**Nod**'s onboarding architecture, and supplied a plan that named crates,
modules and surfaces this tree does not have (`operant-tools`, `operant-harness`
as "the unified persistence seam", `kernel-sidecar` health checks inside
`operant setup`, "70+ tools", "8 crates"). Acting on that text would have built
a second setup wizard, a second detector and a second state store on top of
mechanisms that already exist and already work. This audit re-measures every
claim against the code, so the plan
(`docs/archive/2026-11-18-operant-onboarding-as-a-first-class-subsystem.md`; the
live successor is
`docs/plans/2026-11-18-operant-onboarding-v2-live-state-and-first-task.md`) is
built on what is there rather than on what was described.

**Method.** `agentgrep` for structure, `rg`/`sed` for the cited lines, and
`cargo check` for the build state. No claim below is relayed from another
agent; each carries the command or the file that produced it.

---

## Scope

The question this audit answers: **what does operant's setup and first-run
surface actually consist of today, and which parts of Nod's onboarding
architecture it could adopt without duplicating something that already works.**

In scope:
- `operant setup` — the interactive wizard (`crates/operant-cli/src/cmd_setup.rs`)
- `operant doctor` — the diagnostic engine (`crates/operant-core/src/doctor.rs`)
- the TUI first-run gate (`crates/operant-cli/src/tui/adapter_types/tui_app.rs`)
- the frozen-prefix injection seam (`crates/operant-core/src/agent/events.rs`)
- the seat-policy / bylaws surface (`crates/operant-core/src/org/`)
- the tool-registry availability contract (`crates/operant-core/src/tools/`)
- secret handling (`crates/operant-core/src/redaction.rs`,
  `fs_secrets.rs`, `crates/operant-config/`)

Out of scope (named so nobody re-asks): the gateway adapters, the TUI render
layer, `kernel-sidecar`, `docs/agentic-os/`, and the WASM plugin host. Each is
touched by onboarding at most once, and each is separately audited.

---

## Surface inventory (derived, not read)

| question | value | how derived |
|---|---|---|
| workspace crates | **7** | `Cargo.toml` members; `ls crates/` |
| version | **0.3.0** | `Cargo.toml:14` |
| builtin tool names | **65** | `builtin_tool_names()` vec literal, `builtin.rs:319-389` |
| sourcehound composite tools | **23** | `rg -c '^\s+spec\(' sourcehound_composites.rs` |
| total model-visible builtin tools | **88** before the registry's availability filter | 65 + 23 |
| gateway command definitions | **62** | `rg -o 'name: "[a-z-]+"' gateway_commands.rs \| sort -u` |
| setup wizard steps | **8** | `run_setup_wizard` match arms + step fns |
| seat modes | **4** — `yolo`, `standard`, `scoped`, `lockdown` | `org/seat_policy.rs:36` |
| `operant doctor` checks | **8** functions | `rg 'fn check_' doctor.rs` |
| library mechanism pages | **39** | `ls docs/library/*.md \| wc -l` |
| config template | `operant.example.toml`, 1000+ lines | repo root |

The proposal's "70+ tools / 8 crates / `operant-tools`" matches **none** of
these. That is finding 1.

---

## Findings

Ordered Correct → Safe → Load → Tested → Fast → Lean.

### 1 — [verified] HIGH — the proposal describes a tree that does not exist

The proposal names a crate `operant-tools` ("`operant-tools`: already bundles
70+ JSON-schema tools"). **`crates/operant-tools` does not exist.**
`Cargo.toml` lists seven members and `operant-tools` is not one of them; the
tools live inside `operant-core` (`crates/operant-core/src/tools/`, 2.3 MB).
`README.md:80` still prints the `operant-tools` line in its crate tree and
`README.md:84` still says "(8 crates total)".

The proposal also places `kernel-sidecar` under "`operant-api` + `kernel-sidecar`"
as if they were siblings; `operant-api` is a 3,382-LOC type-alias crate, and the
kernel is an optional sidecar with its own `[tools.kernel]` gate.

**Consequence if followed:** every line number, every file path and every
"already exists" claim in the proposal is wrong by construction. An executor
following it would write new code into crates that do not exist.

**Direction:** the plan cites this audit's inventory table, and the README
crate-tree claim is closed in the same iteration the onboarding work lands
(it is a doc-drift finding, one line, and `check-doc-claims.sh` is the gate).

### 2 — [verified] HIGH — `operant setup` already IS the onboarding wizard

The proposal's headline recommendation is to "upgrade `operant setup` from a
linear CLI questionnaire into a resumable onboarding flow". Measured:
`cmd_setup.rs` is **1,683 lines** with a real wizard that already does most of
what Nod's does:

| wizard step | line | what it asks |
|---|---|---|
| provider + model | `442` | provider menu, model, API key, aux models |
| fallback + strategy | `669` | fallback chain and strategy |
| gateway | `840` | messaging platforms |
| terminal | `899` | stream / theme / reasoning / backend |
| tools | `965` | tool + behaviour settings |
| TTS | `1041` | TTS provider (menu pins the builtin list, iter-2538) |
| browser + skills | `1185` | sourcehound probe, skills dir |
| agent settings | `1238` | agent behaviour |
| **governance** | `1474` | **seat policy + budget** (iter-671) |

It also already: backs up the config through the shared `config_backup`
snapshot scheme (`run_setup_wizard`, `:292-306`); supports `--non-interactive`
(report only, `:48`), `--reset`, `--reconfigure`, and section scoping
(`Some("governance")` etc.); and persists **surgically** — `persist_config`
diffs old vs new and writes only changed dotted keys through the one
`persist_config_key` writer, so comments survive (`:1308-1391`).

**Consequence:** the plan's "resumable OnboardingState + dual-mode wizard"
framing would duplicate 1,683 lines of working code. The real gap is narrower
and is finding 4.

### 3 — [verified] MEDIUM — `operant doctor` is already the single diagnostic engine, and the proposal missed it

The proposal's Batch 2 says to "extract `operant doctor` checks into a reusable
library function". Measured: **they already are.** `operant-core/src/doctor.rs`
is 1,408 lines whose module doc states the history plainly — two engines once
existed (`operant-runtime::doctor` and `operant-cli::cmd_doctor`) and this
module is the surviving one, called by both `operant doctor` and the gateway's
`GET /api/doctor`.

`diagnose(&AppConfig) -> Vec<CheckResult>` (`:82-92`) is exactly the reusable
library function the proposal asks for. It composes eight checks: config
semantics, config resilience, memory provider, skill roots, data root,
environment, CLI tools.

**Gaps that are real** (each verified by reading the function bodies):
- **no sourcehound check.** `check_cli_tools` (`:825`) discovers CLI tools via
  `cli_discovery::discover_cli_tools`, and the setup wizard probes sourcehound
  itself at `cmd_setup.rs:1191`. So `operant doctor` can report "all green"
  while `sourcehound` is absent and every web tool will error — the two
  detectors disagree about the same binary, which is the drift class AGENTS.md
  rule 6 warns about.
- **no memory-wire init / migration barrier** (the proposal wants one; none
  exists and `memory_wire.rs` degrades to a miss on `catch_unwind`).
- **no local-inference probe** (Ollama `11434`, llama.cpp). `doctor` checks
  PATH binaries only.
- **no kernel-sidecar health check.**

**Consequence:** "extract into a reusable function" is a no-op; "add sourcehound
and local-inference checks to `diagnose()`" is the real work, and it is one
function body.

### 4 — [verified] HIGH — there is no resumable state, and the two flags that exist are both write-only or unused

Three separate "onboarding state" mechanisms exist, and none of them is a
resumable state machine:

1. `Settings::has_completed_onboarding: bool` (`tui/adapter_types/config.rs:95`)
   — a single bool. Written by `persist_onboarding_complete()`
   (`tui/app/turn_state.rs:272-280`). Read once, in the TUI's first-run gate
   (`adapter_types/tui_app.rs:119-129`), to auto-open the connect dialog.
2. `OnboardingConfig { seen: HashMap<String, bool> }`
   (`operant-cli/src/config.rs:1089-1091`) — declared, defaulted, wired into
   `CliConfig` at `:95`/`:151`, and **read by nothing.** `rg` finds no reader.
3. `suggestion_prompts()` (`tui/operant_app/tui_state.rs:1244-1249`) — the
   seam where Nod's starter cards would live. It returns `Vec::new()` with the
   comment: *"operant's onboarding has no starter prompt cards; returns empty
   — wire when they are authored."*

**Consequence:** the proposal's Stage 5 (starter intent cards) has a real,
named, empty seam. That is the single cleanest insertion point in the whole
tree, and it is already tested to not promise what it does not do
(`ui_prepare/tests.rs:304` — the onboarding screen must not promise numbered
selection).

### 5 — [verified] MEDIUM — seat policy exists in setup but is gated behind `--reconfigure`

`step_governance` (`cmd_setup.rs:1474-1553`) asks for seat mode and budget, and
its docstring is explicit that it governs cron-driven seats. But the wizard's
default path is **quick mode**: `handle_setup_command` sets
`effective_quick = quick || !reconfigure` (`:63`), and the quick branch runs
only `step_provider_and_model` — `step_governance` is inside `if !quick`
(`:404-412`). The TUI's default first-run gate does not ask about seat policy at
all; it opens the connect dialog and stops.

So an operator who runs `operant setup` (the documented happy path) never sees
governance, and the default seat mode stays `yolo`
(`config.rs:552`, `operant.example.toml:1032`). The proposal wants `scoped` as
the recommended default; that is a one-line default change plus a wizard
reordering, not a new subsystem.

**Note:** the seat machinery itself is fully built — `seat_policy.rs`,
`seat_authority.rs`, `seat_policy_db.rs`, `seat_budgets.rs` under
`crates/operant-core/src/org/`, with the `SeatModeParseError` fail-closed
behaviour and the `lockdown` tests in place. Nothing here needs building.

### 6 — [verified] MEDIUM — secrets are already redacted before storage; the proposal would add a second, worse scrubber

The proposal wants a custom regex/entropy secret-scrubber before writing a
profile note into memory. Measured: `operant-core/src/redaction.rs` is 771
lines with 42 functions, 24 token prefixes, a pre-screen, and a test suite that
pins each family (OpenAI `sk-`, GitHub PAT, AWS key, Slack app-level, private
key block, DB connstr, JWT, Telegram bot token, URL userinfo, JSON fields).
It is applied at every egress that outlives a turn: `memory_wire.rs:428-433`
and `:504`, `:632` ("a fact outlives the session, so a secret is redacted
before storage"). The config write path is owner-only at the `open(2)`
(`fs_secrets.rs::create_owner_only`, 0600), and `persist_config_key_at` writes
a sibling temp file, validates it through the real loader, and only then
renames over — with a documented RT-418 note about the mode round-trip.

**Consequence:** a second scrubber would reproduce exactly the bug the
proposal itself flags (Nod's v0.7.3 "long IDs no longer look like card
numbers") — a second boundary, disagreeing with the first. No new scrubber.

### 7 — [verified] LOW — config writes are atomic but not lock-guarded, and no writer serialises against a running daemon

`persist_config_key_at` is atomic (temp + validate + rename) but takes **no
`flock`**. The tree has `flock` machinery — the gateway daemon holds an
exclusive lock on `<home>/gateway.lock` (`gateway_runner.rs:2435-2465`,
`cmd_cron.rs:157-182` probes it) — but nothing wraps `operant.toml` writes.

**Consequence:** `operant setup --replay` (if built) can race a live gateway
that is mid-read of the config. Low likelihood, real failure mode: a dropped
key. Fix is one lock in `persist_config_key_at`, not a new state machine.

### 8 — [verified] LOW — the frozen-prefix seam is the right home for a static profile block

`build_frozen_prefix` (`agent/events.rs:1222-1359`) assembles, in order: base
system prompt, `<employee_charter>`, `<available_skills>` + skills guidance,
`<available_capabilities>`, `<ponytail>` block, and the harness prompt slot.
Every block is byte-stable across turns and the doc comments say so explicitly
("so the frozen-prefix cache discipline holds").

**Consequence:** a `<user_profile>` block belongs here, adjacent to
`<employee_charter>`, not in memory-wire. The proposal's own Part 1 finding 3
reaches this conclusion; this audit confirms the seam exists and names it.

### 9 — [asserted] LOW — the "sourcehound is private / 404s" claim does not hold on this machine

The proposal's finding 2 says `sourcehound` is private and would be absent on a
fresh install. On this host both resolve:
`/home/ishanp/.local/bin/sourcehound`, `/home/ishanp/.local/bin/cloakctl`.
`sourcehound.rs` resolves it via `SOURCEHOUND_BINARY` then PATH
(`find_sourcehound_binary`), and every web tool degrades to a helpful error
when absent (`is_available()` at `:133`).

**Not verified:** whether a *fresh external* install (a different machine,
no `~/.local/bin`) has it. That is an install-script question
(`scripts/`, not `install.sh` — the proposal names a script that does not
exist here either; the entry point is `scripts/build.sh` per AGENTS.md §7).

**Consequence:** the fix is finding 3's doctor check (report the absence
before turn 1), not a fallback browser engine.

---

## What the audit does NOT find

Recorded so nobody re-opens it (per `docs/kb/CLOSED.md` discipline):

- **No second onboarding wizard to write.** `cmd_setup.rs` is the wizard.
- **No second doctor engine to write.** `doctor.rs` is the engine.
- **No new scrubber to write.** `redaction.rs` covers it.
- **No seat machinery to write.** `org/seat_policy*.rs` covers it.
- **No atomic-write machinery to write.** `persist_config_key_at` covers it.
- **No "vault" subsystem to write.** `operant-os` owns in-flight approval;
  per `README.md:104` the grant ledger, escalation queue and worklog store live
  in **Holon**, and this tree deliberately does not have them.
- **No starter-card mechanism to write.** `suggestion_prompts()` is the seam
  and it is empty.

The honest summary: operant does not need Nod's onboarding *architecture* — it
has every primitive. It needs four small, specific things: (1) sourcehound +
local-inference checks in `diagnose()`, (2) seat policy visible on the default
setup path, (3) a resumable `OnboardingState` with `--replay` that the existing
flags can drive, and (4) authoring `suggestion_prompts()`. Each is an iteration,
not a subsystem.
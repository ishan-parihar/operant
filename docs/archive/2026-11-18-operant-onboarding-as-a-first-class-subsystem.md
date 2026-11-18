# operant onboarding as a first-class subsystem

Dated-frozen: published `2026-11-18`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**Base:** `origin/main` @ `3d8defbb`. Every line number below is cited from
that commit; the tree compiled clean at that commit
(`cargo check -p operant-cli --bin operant` → exit 0, 8 warnings), so an
executor hitting a red gate is hitting something this plan did not see.

**Pre-registration:** this plan is the execution outline for
`docs/audit/2026-11-18-the-operant-setup-and-onboarding-surface-audit.md`.
Read that audit first — it is where every "already exists" claim is measured.
This plan does not re-derive those numbers; it acts on them.

**What this plan is NOT:** it is not a port of n8n Nod's architecture. Nod's
onboarding is an asynchronous two-phase bootstrap around Docker containers and a
pg-boss queue. Operant is a single Rust binary with a synchronous wizard, an
in-process memory engine and no container runtime. The transferable parts of
Nod's design are four behaviours, not one architecture:

1. **Never greet the user with a broken tool.** Nod gates starter suggestions
   on what the runtime can actually do. Operant's analogue is the registry's
   `registered ∩ available ∩ not-disabled` contract (README.md:51) — the model
   must only be offered tools that would run.
2. **A starter card is a prompt, not a permission.** Selecting one submits a
   user turn; every mutation still clears the seat policy. Operant already has
   this invariant in its seat layer — the plan only needs to not violate it.
3. **One diagnostic authority.** Nod does not have this problem; operant
   already solved it (`doctor.rs`) but does not yet check everything setup
   checks.
4. **Secrets never reach the model or the prompt.** Already true
   (`redaction.rs`); the plan must not add a second scrubber.

---

## Mechanism

Four tranches. Each is one iteration, each is independently shippable, each
names the file it touches and the test that proves it. There is **no new
subsystem, no new crate and no new state machine** — the audit found every
primitive already in place.

### T1 — `diagnose()` gains the checks setup already performs

**Problem (audit finding 3).** `operant doctor` reports all-green while
`sourcehound` is absent and every web tool will error. `cmd_setup.rs:1191`
probes sourcehound; `doctor.rs` does not. Two detectors, one binary, disagreeing.

**Change:** add three checks to `crates/operant-core/src/doctor.rs`, alongside
the existing `check_cli_tools`:
- `check_sourcehound` — calls `crate::tools::sourcehound::is_available()`,
  reports `ok` with the resolved path or `warn` with
  `SOURCEHOUND_INSTALL_HINT` (the constant already exists at
  `tools/sourcehound.rs`).
- `check_local_inference` — TCP-probes `127.0.0.1:11434` (Ollama) and
  `127.0.0.1:8080` (llama.cpp server) with a short timeout; `ok` when one
  answers, `warn` ("no local inference endpoint; cloud providers unaffected")
  when none do. This is a *hint*, not a gate: most installs are cloud-only.
- `check_memory_wire` — reports whether the memory engine resolved, degrading
  to `warn` not `error` (the engine is `catch_unwind`-guarded by design).

**Files:** `crates/operant-core/src/doctor.rs` only. `diagnose()` gains three
lines; each check is one function beside the existing eight.

**Test (mutation-proven):** `crates/operant-core/tests/` — a test that sets
`SOURCEHOUND_BINARY` to a nonexistent path and asserts `diagnose()` emits a
`warn` naming sourcehound; and one that asserts a present binary yields `ok`.
Mutation: delete the `check_sourcehound` call from `diagnose()` and the first
test must fail. (No live Ollama probe in the test — point the probe at a closed
port and assert the `warn` branch.)

**Risk:** the local-inference probe opens a socket. Bounded by a 200 ms connect
timeout on `127.0.0.1` only; never a DNS name, never a remote host.

### T2 — seat policy is reachable from the default setup path

**Problem (audit finding 5).** `step_governance` (`cmd_setup.rs:1474`) is
inside the `if !quick` branch, and `handle_setup_command` defaults to quick
mode (`:63`). The documented happy path never asks, so the default seat mode
stays `yolo` (`config.rs:552`, `operant.example.toml:1032`).

**Change (two lines, one decision):** in `run_setup_wizard`'s `None` arm, call
`step_governance(&mut updated)` **after** the quick branch's
`step_provider_and_model` and **before** `step_integrations` — so both quick
and full see it. Accept the current value as the default so an operator who
does not care presses Enter and gets today's behaviour.

**Do NOT change the default in this tranche.** Flipping
`unrestricted_default` from `yolo` to `scoped` changes what a cron seat can do
for every existing install on upgrade — that is a behaviour change with a
migration cost, and it is a separate iteration with its own changelog entry and
its own `operant.example.toml` note. This tranche only makes the choice
*visible*.

**Files:** `crates/operant-cli/src/cmd_setup.rs` (the `None` arm, ~4 lines).
No config change, no template change.

**Test:** `cmd_setup.rs`'s existing `wizard_persist_is_surgical` and
`the_setup_wizard_menus_every_builtin_and_no_phantom` still pass (the new call
adds no menu). Add one test: the quick path persists a `bylaws` key the
operator changed, proving governance runs on the default path. Mutation:
revert the call site into the `!quick` arm and the test fails.

**Risk:** quick mode gets one more prompt. Mitigation: the governance prompt
prints the current value and accepts Enter; an operator who wants zero prompts
already has `--non-interactive`.

### T3 — a resumable `OnboardingState` with `--replay`

**Problem (audit finding 4).** Three onboarding-state mechanisms exist and none
resumes: a bool (`has_completed_onboarding`), a write-only map
(`OnboardingConfig::seen`, zero readers), and an empty seam
(`suggestion_prompts()`).

**Change:** one state file, one writer, one reader.

- **State:** `~/.operant/onboarding.json` (under the one-home resolver
  `platform::operant_home()` — never a path built from the OS home; the
  `one_home_resolver` test guards this and its `ALLOWED` list is where any
  exception must be named with a reason).
  Shape: `{ "version": 1, "completed_steps": ["provider", "governance"],
  "updated_at": "<rfc3339>" }`. Steps are the wizard's own step names, so the
  state cannot drift from the wizard's vocabulary.
- **Writer:** `persist_config_key_at`-shaped — sibling temp file, validate as
  JSON, atomic rename, 0600 (it is onboarding metadata, and it may name an
  endpoint). Reuse `fs_secrets::write_secret_file` for the mode, not for
  secrecy.
- **Lock:** an exclusive `flock` on `<home>/onboarding.lock`, mirroring the
  gateway daemon's `GatewayDaemonLock` (`gateway_runner.rs:2435-2465`) so a
  live daemon and a wizard cannot interleave writes. This also closes audit
  finding 7 for this file.
- **`--replay`:** `operant setup --replay` (new flag on the existing `Setup`
  variant) loads the state, **skips** completed steps unless `--reconfigure`
  names them, and never clobbers `operant.toml` keys it did not ask about —
  `persist_config` already diffs, so this falls out for free.
- **Reader:** `run_setup_wizard` and the TUI's first-run gate
  (`adapter_types/tui_app.rs:119-129`), which currently reads only the bool.

**Files:** new `crates/operant-cli/src/onboarding_state.rs` (~180 lines);
`crates/operant-cli/src/main.rs` (one `Setup` flag);
`crates/operant-cli/src/cmd_setup.rs` (load/save calls);
`tui/adapter_types/tui_app.rs` (read the state, not just the bool).

**Tests:** round-trip (write state, read it back); a corrupt file degrades to
"no state" rather than panicking; `--replay` does not touch a key it did not
change (drive `persist_config` with a sentinel key in the TOML and assert it
survives). Mutation: make the writer non-atomic (write in place) and the
crash-safety test must fail.

**Risk:** a second store beside `operant.toml`. Mitigation: the state file
holds *progress*, never *settings* — the line is "did the operator answer
this step", and every answer still lands in `operant.toml` through the existing
surgical writer. If a future reader wants a setting, that is a defect.

### T4 — author `suggestion_prompts()` (the starter-card seam)

**Problem (audit finding 4, third item).** `suggestion_prompts()`
(`tui/operant_app/tui_state.rs:1244-1249`) returns `Vec::new()` and says so.
It is the exact seam Nod's idea cards would occupy, and it is currently dead.

**Change:** author the prompts. **Deterministically** — no LLM call, no
untrusted-file read, no `TODO.md` ingestion. Three sources, intersected:

1. **The profile the operator just gave** (from T3's state, or a new
   `[onboarding]` config section if a profile is warranted — decide in T3,
   not here).
2. **`get_schemas()`** — the live registry. A prompt naming `web_search` is
   not offered when sourcehound is absent.
3. **`diagnose()`'s verdicts** — T1's sourcehound check.

Shape: `Vec<(String, String)>` of `(label, prompt_text)` pairs, exactly what
the trait signature asks for. Example set, each gated:
- `"Audit this repository's open findings and summarise the three most urgent"`
  Shown when the `file_read`/`file_search` tools are registered (always).
- `"Search the web for …"` — shown only when sourcehound is present.
- `"Set up a daily digest"` — shown when the `cron` tool is registered.

**Non-goals (named so nobody adds them):** no LLM call at boot (the proposal's
own Part 1 finding 6 is right: cold-start cost + prompt injection from an
untrusted `TODO.md`). No starter card is a permission. A card injects a user
turn; the seat policy gates every tool the turn calls, unchanged.

**Files:** `crates/operant-cli/src/tui/operant_app/tui_state.rs` (the method
body); a helper in the same module that intersects the registry with the
profile. `ui_prepare.rs` already renders the empty state
(`:971`); the non-selection promise is already tested
(`ui_prepare/tests.rs:304`).

**Tests:** an empty registry yields an empty prompt list (never a prompt naming
a tool that is not registered); a registry with sourcehound absent yields no
web prompt; the list is deterministic (same inputs, same order).

**Risk:** the prompt list is per-frame work. Mitigation: it reads two
already-resident structures (the registry and the state file), no I/O in the
render path — if it needs I/O, it belongs at boot.

---

## Gates / success criteria

(fixed before the run — each tranche is one iteration and must clear all of
these before it is committed)

- `source scripts/dev-env.sh` then `cargo fmt --all` — clean.
- `cargo check -p operant-core --lib` and
  `cargo check -p operant-cli --bin operant` — **0 errors**.
- `cargo clippy -p operant-core --lib -- -D warnings` and
  `cargo clippy -p operant-cli --bin operant -- -D warnings` — clean; and
  `bash scripts/clippy-warning-gate.sh` — no NEW warnings.
- The tranche's own tests, run scoped: e.g.
  `cargo test -p operant-core --lib -- doctor` for T1,
  `cargo test -p operant-cli --lib -- cmd_setup` for T2.
- `bash scripts/check-doc-claims.sh` — exit 0 (T1 adds no doc claim; T3 may
  add one line to `operant.example.toml` if it adds a config key, and the gate
  covers the template).
- `./scripts/genome lint` — no NEW error. Measured at this plan's base the
  lint already reports **2 errors / 3 warnings** (`E009` ledger:28 cites a
  missing `.operant/auth.json`; `E015` a stale WORKLOG zone) — both pre-date
  this plan and neither is in `docs/plans/` or `docs/audit/`. Confirm the same
  count before and after; a new one is a regression.
  **Note:** `docs/plans/` is NOT an `owner_globs` path in `genome.toml` (only
  `docs/library/*.md` is), so plans do not require an `docs/INDEX.md` row to
  pass `E301` — five plans landed since 2026-11-10 without one. Add rows
  anyway if the plan becomes a standing reference.

**Definition of done for the whole plan:** an operator can run
`operant setup` on a fresh machine, be asked about seat policy on the default
path, have `operant doctor` agree with the wizard about sourcehound, resume an
interrupted setup with `--replay` without losing a config key, and land in a
TUI whose empty state offers two or three prompts that name only tools that
would actually run.

**Explicitly NOT in scope** (so nobody builds them): a Vault subsystem; a
second scrubber; a container runtime; an async provisioning phase; a
`memory.restate` analogue (memory-wire has `retain`); a per-step migration
framework; changing the default seat mode; the gateway or TUI render layer.

## Cost

| tranche | files | ~LOC | new deps | risk |
|---|---|---|---|---|
| T1 | 1 (`doctor.rs`) | ~90 | none (std TCP) | low |
| T2 | 1 (`cmd_setup.rs`) | ~6 | none | low |
| T3 | 2 new + 3 edited | ~200 | none (`libc` already a dep for `flock`) | medium (state-file semantics) |
| T4 | 1–2 (`tui_state.rs` + helper) | ~120 | none | low |

Four iterations. No new crate, no new dependency, no new state machine beyond
T3's progress file. Each tranche is independently revertable.

## Order and dependency

T1 and T2 are independent and can go in either order. T3 depends on nothing
but should land before T4 (T4 reads T3's state for the profile). Suggested
order: **T1 → T2 → T3 → T4**, one iteration each, pushed separately per
AGENTS.md's iteration loop.
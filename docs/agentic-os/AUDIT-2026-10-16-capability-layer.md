# Audit: the capability layer across the Holon seam (2026-10-16)

Status: **open work list.** A red-team audit of how the capabilities the organization declares
(Holon's units, pooling, `policy.json`, the brief) reach a body running in operant, and of
everything operant adds to a Holon run's prompt on its own. Item ids (R-nn) match Holon's
`docs/design/2026-10-16-capability-layer-audit.md` (the kernel's half) and the owner's overlay
record `2026-10-16-agentic-os-red-team-audit.md` (the full evidence). Measured against operant
source at iter-873 and the deployed gateway (built 2026-10-16 17:09; `operant doctor` green).

**How to use it.** §1 items are operant defects or gaps: each is one iteration under
`AGENTS.md`'s protocol, with the acceptance written here. §2 is the order relative to the
kernel; respect it, or org mode turns itself off (the read-API version trap). §3 is the method:
read it before you claim an item closed.

## 1. Operant-side items

### R-07 — a Holon run gets workspace context the kernel never chose · CONFIRMED

- **Where:** `crates/operant-core/src/agent/run.rs:2412` appends `<workspace_context>` to every
  turn's volatile suffix from `prompting.rs::load_context_file_prompt`. That loads
  (a) `~/.config/operant/context/*` through `dirs::config_dir()`, which is **not** under the run's
  `HERMES_HOME`, so it is the owner's personal context; and (b) the first workspace context file
  found walking up from the cwd (`context_files.rs:135`; without a git root the walk reaches `/`).
  Nothing checks `platform::is_holon_run()` (`platform.rs:147`).
- **Why it matters:** a body whose workdir sits inside a legacy tree (every legacy venture pool
  carries its own `AGENTS.md`) gets that tree's old protocol beside the kernel's brief: two
  instruction sets, one of them stale. This is the double injection the C-track's single-injector
  law forbids, and it is coming from the runtime.
- **Fix:** in a Holon run, skip the global context dir entirely, and load workspace files only
  when the kernel names them in `brief.json` (until the kernel grows that field: none).
- **Accept:** a test with `HOLON_RUN_DIR` set, a planted `AGENTS.md` in the cwd and a planted
  global context file: the built messages contain neither. Personal mode stays byte-identical
  (invariant 5).

### R-08 — memory in a Holon run is not isolated · CONFIRMED (gate missing) / PLAUSIBLE (leak)

- **Where:** `memory_provider.rs:353-366`: memory-wire is the only engine, and the adapter's
  `[memory] provider = "builtin"` pin is ignored (the live gateway logs the warning). The
  per-turn retain `run.rs:1528` (`submit_sync_turn`) and the per-turn prefetch
  (`run.rs`, `<memory_context>`) have no Holon-run gate. Production code never opens
  `MemoryScope::Seat` (only tests do), so every Holon employee would share one bank.
- **Not observed:** after 6 live Holon runs on operant, the owner's `~/.operant/memory_wire.sqlite`
  holds no Holon content, and the adapter home holds no store. Trace where those runs' provider
  pointed before you assume either outcome.
- **Fix:** in a Holon run, open no memory provider: the kernel's context bundle is the run's
  memory, and learnings leave through `result.json` (W6). The alternative is a
  `Seat { HOLON_EMPLOYEE_ID }` bank inside `HERMES_HOME`. Either way, personal mode keeps
  memory-wire as the default (a design preference in `AGENTS.md`: do not change it).
- **Accept:** extend `crates/operant-core/tests/holon_capture.rs` (today it covers tool writes,
  not this path): a Holon-run turn with a provider configured leaves no retained memory and
  injects no `<memory_context>`.

### R-09 — the skill list shows the standard skill while the brief shows the department's fork · CONFIRMED

- **Where:** the frozen prefix (`agent/events.rs:822`, `build_frozen_prefix`) lists the skills
  under `[skills] root_dir`. The Holon adapter fills that once, organisation-wide, so a
  department's tailored role skill reaches the body twice in two versions.
- **Fix:** when `brief.json` names a run-local skills directory (the kernel half of R-09), use it
  as the skill root for this run. Until it does, list no skills in a Holon run (the brief is the
  render).
- **Accept:** a Holon run whose brief names a skills dir lists only those skills.

### R-10 — the continual-harness lane reads a store outside the run's home · PLAUSIBLE (default off)

- **Where:** `tools/kernel/mod.rs` `injection_block` injects `[continual harness]` lessons
  whenever `[kernel] enabled`, from `state_dir`, which defaults to
  `~/.local/share/operant/kernel/harness`, outside `HERMES_HOME`. `BOUNDARY.md` §4 says operant
  keeps no continual-harness lesson store for org work.
- **Fix:** gate `injection_block` on `!is_holon_run()` (the Holon adapter will also pin
  `[kernel] enabled = false`).
- **Accept:** with the kernel enabled and `HOLON_RUN_DIR` set, the volatile suffix carries no
  `[continual harness]` block.

### R-04 — the runtime never reads the registry the organization declares · CONFIRMED

- **Where:** nothing in `crates/` reads `api_units` or `api_services` (grep: 0 hits).
  `tool_search.rs:67` (`is_deferrable`) defers only `mcp_*` tools, so progressive disclosure,
  the mechanism built to make many tools cheap, never sees a declared service.
- **Why it matters:** the brief's capability block is static prose, invisible to
  `tool_search`. And the services cannot be *called* in observe stages at all (R-03).
- **Fix (after the kernel publishes read API v4, §2):** read `api_pool(unit, dept)` and
  `api_services` (v4 adds `effect`, `note`, `args`, `output`, `health`); register each service
  the run's department pools as a deferrable tool `svc_<unit>_<service>`, with its schema from
  `args`. Execute it as argv (never through a shell) via the existing `holon_command_fence`.
  Keep it out of the observe allow list unless `effect = "read"`. Treat a stale or missing view
  as "no services" plus a doctor line; never fail the turn.
- **Accept:** a scratch instance pooling one read and one write service: in observe,
  `tool_search` finds the read service, and calling it runs and records usage; the write service
  is absent. In act, both are present and both pass the commands fence.

### R-03 — observers cannot run any declared service · CONFIRMED

- **Where:** `crates/operant-os/src/policy.rs:37` (`OBSERVE_ALLOW`) has no `terminal`, correctly.
  But research, plan, shape and verify all run in observe, so every `[[services]] command` the
  brief lists is refused in exactly the stages that research and plan.
- **Fix:** R-04's read-effect tools, not a wider observe shell. Close this with R-04.

### R-20 — the Notion arm hosts webhooks but nothing pushes · CONFIRMED

- **Where:** `crates/operant-cli/src/gateway_runner.rs:1716` (`holon_notion_hook`) handles inbound
  events with `holon_notion.py apply`. The outbound mirror (`push`, `reconcile`, `verify`) exists
  only in the Python `serve` loop. W5's acceptance line "`holon-notion.service` unnecessary"
  cannot be met while that is true.
- **Fix:** one of two, chosen with the owner. (a) The clock organ also runs
  `holon_notion.py push` every 20–60 s and `reconcile` every 15 min, as `no_agent` jobs beside
  `holon tick`. (b) Drop the gateway arm and keep `holon-notion.service` as the one receiver and
  pusher. One webhook receiver per box, never two.
- **Accept (a):** with `holon-notion.service` stopped, a kernel change reaches Notion within the
  push interval, and doctor reports the last push's age.

### R-02 — two names for one CDP endpoint · CONFIRMED (low)

- `tools/browser_tool.rs:170` reads `BROWSER_CDP_URL`; the legacy apply engines read
  `OBS_CDP_URL`. When their departments migrate, one variable must win. Either the browser tool
  also accepts `OBS_CDP_URL` (with a deprecation line), or the migration renames it in the
  department's `[secrets.dept]` / env. Holon's browser surface manifest names whichever wins.

### R-11 — doc truth · CONFIRMED

- This repo's `REMAINING-GAPS.md` and the owner's overlay register disagree (org mode on vs
  gated; W5 notify closed vs open). The overlay register is the one place open work is tracked;
  `REMAINING-GAPS.md` should link it rather than keep a parallel closure log.
- `AGENTS.md` → "Design Preferences" describes `BuiltinProvider` as the zero-dependency memory
  fallback, and lightpanda/camofox/browserbase/browser-use/firecrawl as available browser
  backends. The code deleted them (`memory_provider.rs`: "BuiltinProvider is deleted";
  `browser_provider.rs:1-6`). The **preferences** stand; the descriptive lines are stale. That
  section is marked DO NOT CHANGE, so raise it with the owner instead of editing it.

## 2. Order relative to the kernel

1. **Now, no kernel dependency:** R-07, R-08, R-10, R-09's interim (no skill list in Holon runs),
   R-02, R-11.
2. **Before the kernel bumps the read API to v4:** add `"4"` to `KNOWN_API_VERSIONS`
   (`crates/operant-os/src/lib.rs`), reading v1–v3 shapes unchanged. A kernel at v4 that operant
   does not know turns org mode off on the live box.
3. **After the kernel publishes v4** (`api_pool`, service columns): R-04, which also closes R-03.
4. **The kernel's R-06** (pooled roots move to `read_roots` in `policy.json`) needs **no**
   operant change: the write fence keeps reading `roots`, which then excludes pooled roots. Do
   not start fencing `read_roots` as writable.
5. **R-20** when the owner picks (a) or (b).

## 3. How to verify an item here (the method that found these)

1. **Every injector needs an `is_holon_run()` answer.** For each thing that adds text to the
   prompt (system prefix, volatile suffix, memory prefetch, skills, lessons, MoA guidance), ask
   what it does under `HOLON_RUN_DIR`. R-07, R-08 and R-10 are three injectors nobody asked.
2. **Prove it against the real prepared env**, not a stub. Use the env
   `holon dispatch --prepare --json` returns (the `operant-os/tests/e2e.rs` pattern). An
   interface package is done only when the real artifact crosses the seam.
3. **Read the deployed binary's startup log.** "key ignored" warnings mean an adapter's
   guarantee is void (R-08 was found that way).
4. **Run `operant doctor` before declaring something dead.** In this audit a missing
   `last-tick` stamp looked like a dead clock; doctor showed a fresh tick.
5. **Open Holon's databases only `mode=ro`, through `api_*` views** (invariant 1), even when you
   are only looking.
6. **Grep both repos.** A Holon view nobody reads here, or a policy field operant parses but
   never enforces, is the seam's commonest defect.
7. **Label findings CONFIRMED or PLAUSIBLE**, and say what you did not trace. Close an item
   only with its acceptance test in the default battery (`HOLON_REPO` set).

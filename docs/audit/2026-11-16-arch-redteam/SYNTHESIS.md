# Synthesis — architecture red-team + ponytail audit, 2026-11-16

Audited revision: **origin/main @ 2d431ac0** (pinned before dispatch).
Status: **partial** — 2 of 9 workers delivered; 7 still running.

Every row below was verified by ME against `origin/main` with `git show` /
`git grep`. A worker's rank is recorded as **confirmed**, **raised**, or
**lowered**, with the reason. Nothing here is relayed unverified.

The shared working tree was DIRTY throughout (a peer session's 19 modified
files). No worker read the working tree; every read went through git.

---

## The findings that matter

### 1. The approval gate is not on the dispatch path — `mcp serve` runs shell tools ungated

`crates/operant-core/src/approval.rs:662` is the ONLY caller of
`blast_radius::screen` in the workspace (verified:
`git grep -n 'blast_radius::screen' origin/main` returns exactly one hit).
The gate therefore sits in the agent loop's pre-flight
(`agent/stream.rs:1263-1268`) and the kernel bridge's own copy — not inside
`ToolRegistry::execute`.

`crates/operant-cli/src/mcp_serve.rs` contains **zero** references to
`approval`, `check_tool_approval`, or `blast_radius` (verified:
`git grep -c` returns nothing). It builds the FULL registry
(`mcp_serve.rs:88` → `build_tool_registry`, `:166` enumerates every tool for
`tools/list`, including `terminal` at `builtin.rs:354` and `code_execution` at
`builtin.rs:324`), and `handle_tools_call` (`:372-410`) gates only on
`registry.contains(tool_name)` before calling `registry.execute` at `:404`.

**An MCP client can therefore invoke `terminal` with an arbitrary command and
nothing screens it** — not the hardline blocklist, not the 47 dangerous
patterns, not blast radius.

A5 ranked this CRITICAL; I rank it **HIGH**. `mcp_serve` is stdio
(`mcp_serve.rs:1-7`), so the caller is a local process the operator configured
and could already spawn a shell itself. What keeps it HIGH rather than LOW is
the intended deployment: an operator adds `operant mcp serve` to Claude Desktop
or VS Code precisely so a MODEL can drive these tools, and in that
configuration the model chooses arguments with none of the guardrails the
operator would expect `operant` to apply. It is the agent loop's gate, absent
on the surface built to let a model call the tools.

Note `approval.rs:643-647` already claims the gate "lives HERE, inside the one
function every tool call already passes through." That is true of
`check_tool_approval` and false of the dispatch point, and that sentence is
why nobody looked for a third caller.

`operant test <tool> <args>` (`main.rs:3440-3482` → `test_tool`, dispatched
from `main.rs:3848`) is the same hole with a
human holding the keyboard: it builds the full registry and calls
`registry.execute` at `:3469` with no `approval` reference anywhere in the
handler. I verified this one myself, line by line, because it was relayed by a
worker and it is the second instance of the class rather than a corollary of
the first.

**Fix:** move the `check_tool_approval` call into `ToolRegistry::execute`
beside the disabled-set refusal (`tools.rs:1180`), or give `execute` an
explicit `gated: bool` every non-agent caller must state out loud.

### 2. The same defect shape one level down: `/search` bypasses `tools disable`

`crates/operant-cli/src/gateway_runner.rs:1962` constructs `SessionSearchTool`
and calls `OperantTool::execute` on it with `ToolContext::default()`. There is
no `direct_call_is_disabled` check and no registry hop, so
`operant tools disable session_search` leaves the channel command searching.

A2 built the complete tree-wide inventory of this class, and the result is
narrow and credible: exactly FOUR production `OperantTool::execute` call sites
exist outside `ToolRegistry::execute`:

| site | tool | guarded? |
|---|---|---|
| `gateway_runner/enrichment.rs:143` | `TranscriptionTool` | YES — `direct_call_is_disabled` at `:136` (the RT-3 fix) |
| `gateway_runner.rs:1962` | `SessionSearchTool` | **NO** |
| `mcp.rs:2458` | inner `McpTool` | n/a — trait-impl body, entered only via the registry |
| `plugin_tools.rs:69` | plugin inner | n/a — trait-impl body |

Every other `.execute(` hit (~100) is inside a `#[cfg(test)]` module. I spot-
checked three of A2's other claims (the `tts` name mismatch, the live
`lcm_*`/`ctx_*` registration, and `MemoryRecallTool`'s deadness) and all held.

### 3. The operator-facing CLI prints credentials; the in-loop tool masks

`crates/operant-cli/src/cmd_config.rs:94-96` is
`serde_yaml::to_string(config)` + `println!`, with no masking. `cmd_dump.rs`
does the same in both its YAML and `--json` forms. None of the credential
fields carry `skip_serializing`:

- `ClientSettings::api_key` — `config.rs:565`
- `GatewaySettings::telegram_token` / `discord_token` / `slack_token`
- `GatewaySettings::email_smtp_pass` — `config.rs:1945`
- `SttSettings::groq_api_key` / `openai_api_key`

The contrast is the point: `tools/config_tool.rs:137` routes its dump through
`redact_secrets` (`:258-282`), which masks `api_key`, `token`, `secret`,
`password`, `pass`, `auth`. **So the model sees `***` and the operator's
terminal sees the key.** The policy exists; one of the two surfaces honours it.

**Fix trap, verified:** do NOT add `skip_serializing`. `config_tool.rs:235`
round-trips through `serde_json::to_value(&AppConfig)`, and a skipped field
would be **silently dropped** on the next `operant config set`. Reuse
`redact_secrets` in `handle_show`/`handle_dump_command` instead.

### 4. A credential writer that chmod's after create

`crates/operant-core/src/mcp_oauth.rs:365-427`:

```
399:  let mut file = std::fs::File::create(&tmp)   // umask default, 0644
410:  file.set_permissions(0o600)?;                // chmod AFTER create
413:  file.write_all(json.as_bytes())?;            // the secret lands after
417:  std::fs::rename(&tmp, path)
```

AGENTS.md names this exact rule: secret files must be created 0600 **at open**,
not chmod'd after the write, because a window exists in between. Every other
credential writer in the tree uses `fs_secrets` (A5 verified this
independently); this is the one that does not.

### 5. `operant config env-path` reports a file the runtime does not read

`crates/operant-cli/src/cmd_config.rs:382-401`. The candidate order is
`HERMES_CONFIG` → `HERMES_CONFIG_DIR` → `dirs::config_dir()/operant/.env` →
`~/.config/operant/.env`, and the FIRST candidate that exists is printed.

The runtime's actual file is `env_store::operant_env_path()`
(`env_store.rs:51`) = `operant_core::platform::operant_home().join(".env")`.
So if `~/.config/operant/.env` happens to exist, it is printed **in preference
to the real file**, whose existence is never consulted.

A5 ranked this MEDIUM; I raise it to **HIGH**. The operator is told the wrong
path even when the right one is right there, and this is the value they paste
into a bug report or edit expecting the runtime to notice. `HERMES_CONFIG_DIR`
is read on this one line and nowhere else in the workspace.

### 6. The one-home guard has three independent weaknesses, and it is the only gate

`crates/operant-core/tests/one_home_resolver.rs` is the sole mechanical
enforcement of the one-home-resolver rule. It passes today — I ran it — and it
has three separate defects:

- **Its matcher misses the spelling that is actually used.** It requires the
  literal `".operant"` within ±10 lines of the substring `home_dir`.
  `theme_files.rs` uses `std::env::var("HOME")`, so it is never examined.
- **Its parser is a brace counter.** A brace inside an attribute
  (`#[expect(clippy::unwrap_used, reason = "x")]`) opens and closes on the
  ATTRIBUTE's braces, leaving the real test function in the "production" text
  (a false positive); a raw string containing `#[cfg(test)]` truncates the scan
  (a false negative). A5 measured both cases and reports that no file in the
  tree currently trips either — so this is robustness, not a live defect.
- **4 of its 13 ALLOWED entries name files that no longer exist.** Verified with
  `git cat-file -e`: `operant-config/src/platform/native.rs`,
  `operant-config/src/schema/config_impl.rs`, `operant-providers/src/lib.rs`,
  `operant-providers/src/openai_codex.rs` are all gone. Dead allowlist rows that
  can never match.

The composite matters more than any one: the only gate for this class has a
name-shape matcher, a confused parser, and stale config.

### 7. `theme_files::dir()` is a second (really third) operant-home resolver

`crates/operant-cli/src/tui/theme_files.rs:20-39`. It reads
`OPERANT_CONFIG_DIR` and then `$HOME`, and never calls
`operant_core::platform::operant_home()` — so it ignores `HERMES_HOME` and the
sticky active profile. Under `HERMES_HOME` or after `operant profile use`,
themes resolve to the DEFAULT profile's `~/.operant/themes` while every other
store follows the resolver. It is live: `theme_colors.rs:345`,
`theme_screen.rs:173`/`:207` all call it.

A5 found something sharper than my version: the module doc claims it restates
"the same rule operant-core's loader follows," but the loader
(`config.rs:3638-3662`) puts `OPERANT_CONFIG_DIR` first and then pushes
`platform::operant_home().join("operant.toml")` — it does NOT follow the rule
the doc describes. So the code implements a third variant and its comment
describes a rule that does not exist.

Both of us rank it MEDIUM (read-only surface). Agreed.

---

## Lean / ponytail

### `operant-api` is ~95% dead

Measured: of 3,403 LOC in `crates/operant-api/src/`, only `tool.rs` (57 LOC)
has any external consumer — `operant_api::tool` is used by
`operant-plugins` (`lib.rs:14`, `runtime.rs:10`, `wasm_tool.rs:7`) and
`plugin_tools.rs`. Every other module has **zero** references outside its own
crate:

| module | LOC | external refs |
|---|---|---|
| schema | 844 | 0 |
| provider | 792 | 0 |
| memory_traits | 403 | 0 |
| observability_traits | 369 | 0 |
| channel | 296 | 0 |
| glob | 146 | 0 |
| runtime_traits | 142 | 0 |
| peripherals_traits | 84 | 0 |
| session_keys | 63 | 0 |
| media | 56 | 0 |
| agent | 55 | 0 |
| vad | 46 | 0 |
| **tool** | **57** | **6** |

~3,050 LOC deletable. Caveat that matters: `pub` items in a library crate are
reachable by external users, so this needs an owner decision, not a silent
sweep — and `operant-api/src/lib.rs:19` exports `pub mod channel`, so the dead
surface is part of the published API.

### NOT a lean target: `config_registry.rs`

Worth recording so a later pass does not re-flag it. 3,964 lines that are
almost entirely DATA: `CONFIG_KEYS` ends at :1617, `SETTING_ROWS` at :3578,
`KNOWN_KEYS` runs to the end. It is live — it drives the settings screen
(`settings_screen/state.rs:89,442`) and `cmd_config.rs`. Line count is not
complexity.

### Small cuts, verified

- `TRANSIENT_RETRY_BACKOFFS_SECS` (`cronjobs/scheduler.rs:17`) is never used.
  Its only remaining mention is a doc comment at `:62` referring to the constant
  that no longer exists.
- `MemoryRecallTool` (`tools/memory_tools.rs:347-402`) is never registered in
  production; the only instantiation is `tests/integration_test.rs:422`, and
  its advertised name `memory_recall` is an alias to `memory_search`.
- `ApprovalsConfig` + `CliConfig.config_file`/`local_config_file`/
  `config_version` (`operant-cli/src/config.rs:742-756, 53-58`) — A5's L3/L4.

---

## Rust discipline, measured

The repo's strictest rule is "production code must not call `.unwrap()` /
`.expect()`". I ran the gate per crate rather than grepping:

```
cargo clippy -p <crate> --lib -- -D clippy::unwrap_used -D clippy::expect_used
operant-core: 1    operant-api: 0    operant-config: 0    operant-plugins: 0
```

**One violation in the whole library surface.** `agent/events.rs:816` is a
production `.expect("user_turns RwLock poisoned — programmer error")` with no
`#[expect]` annotation. Everything else carries one. The 4,433 raw `.unwrap()`
grep hits are overwhelmingly `#[cfg(test)]` modules, which are exempt — which
is why the raw count is meaningless and the gate is the only honest number.

`.ci/clippy-allowlist.txt` is EMPTY (0 entries), so the gate denies zero
baseline sites today.

`cargo clippy -p operant-core --lib` also reports 7 real warnings, of which
the substantive ones are:
- `tools/sub_agent_tool.rs:713` — `pub fn call_with_session` returns the
  private type `ChildAnswer`: a private type in a public interface.
- `agent/events.rs:786` — an unfulfilled `#[expect]` (the lint no longer fires,
  so the attribute is stale).
- `advisor.rs:723`, `turn_context.rs:331`, `config_backup.rs:67`,
  `session_server.rs:125` — small idioms.

### The `anyhow`-in-library-crates claim does not match the tree

AGENTS.md: library crates return typed errors; `anyhow` is for binaries only.
Measured: `anyhow` is a declared dependency of `operant-core`, `operant-api`,
`operant-config` and `operant-plugins`, and appears in 77 PUBLIC signatures
(`operant-api` 34, `operant-core` 41, `operant-config` 1, `operant-plugins` 1).

The largest concentration is `operant-api/src/channel.rs`, where
`anyhow::Result` appears in 34 public TRAIT METHOD signatures — a library crate
publishing an error type it does not own. Since that whole module is dead (see
the lean table), this is a discipline violation expressed entirely in dead
surface: it matters as a signal, not as a live correctness risk.

---

## Verified clean — recorded so it is not re-reported

- **Webhook HMAC is sound.** `gateway/webhook.rs` uses
  `constant_time_eq` at every comparison (126, 175, 214, 409), rejects a bad
  signature with 401 rather than logging, and has 5-minute replay windows for
  the Slack (`v0:<ts>:<body>`) and Stripe timestamped schemes.
- **The unauthenticated-webhook POST is warned, not silent** (`webhook.rs:360`
  emits a `warn!` naming the exposure). Minor asymmetry: the WhatsApp/Meta GET
  verify branch at `:413` is `None => true` with no equivalent warning; that
  path only echoes a challenge.
- **The binary updater does not install unverified bytes silently.**
  `binary_updater.rs:314-332` SHA256-verifies when a manifest is present and,
  when none is, emits a `warn!` that names the residual risk. Deliberate and
  documented.
- **The platform registry is exactly 7 adapters** — verified by extracting the
  adapter types from `platform_registry()` (`gateway_runner.rs:5101-5190`):
  Telegram, Discord, Slack, WhatsApp, Email, SMS, Webhook. AGENTS.md's claim
  holds.
- **TUI reasoning-override test discipline holds.** Every
  `set_reasoning_display_override` call site in `tui/app/tests.rs` sits inside a
  test holding `config_shim::reasoning_override_test_guard()`. `tests.rs:7274`
  looks unguarded but is the cleanup tail of
  `reasoning_alias_sets_the_live_thinking_mode`, whose guard is at `:7240`.
- **The cron wedge fix holds.** `abandoned_claim_jobs` is defined
  (`db.rs:1215`) AND called (`scheduler.rs:469`), and the reclaim loop re-arms
  via `compute_next_run` + `set_next_run`, warning rather than silently
  disabling a job whose schedule yields no next occurrence.
- **The one-home guard's ALLOWED entries all carry written justifications** —
  the *stale* ones are a different problem (see §6).

---

## Coverage caveat

- 2 of 9 workers delivered (A2 tools, A5 config/secrets). A1 agent loop, A3
  gateway, A4 memory/context, A6 skills/plugins/MCP, A7 TUI (350 files), A8 CLI
  surface, A9 lean+rust are still running. **This synthesis is partial by
  construction** and will need a second pass over their docs.
- My own verification is concentrated where I judged a mistake would cost most:
  the security gates (§1-5), the guard (§6), and the discipline numbers. I did
  not independently re-read the context-eviction tiering or the TUI render
  invariants; those rest on the workers alone.
- The audited revision is pinned to `2d431ac0`. Any fix that lands before the
  remaining docs are written will invalidate their line numbers, which is why I
  have not started editing.

---

## Correction log

Recorded because a synthesis that hides its own revisions is not an audit.

- **`operant test` — citation corrected, claim unchanged.** The A5 report cited
  `main.rs:3440-3470`; the handler actually runs to `:3482` (I read `test_tool`
  in full and confirmed `registry.execute` at `:3469` with no approval check).
  Substance held; range fixed in §1.
- **`mcp serve` severity — lowered CRITICAL → HIGH by me**, not by the worker.
  `mcp_serve.rs:1-7` says stdio; there is no TCP listener. The worker's facts
  were right, its blast-radius framing did not account for the transport.
- **`env-path` severity — raised MEDIUM → HIGH by me.** The worker described a
  wrong answer; the code prints a WRONG file in preference to the right one
  when both exist (`cmd_config.rs:396` only falls through when the first
  candidate is absent), which is a different and worse claim.
- **The `anyhow` rule is stated in AGENTS.md and contradicted by the tree.**
  Not a worker finding: a documentation-vs-code mismatch I measured directly
  (4 library crates declare the dep; 77 public signatures use it). The honest
  reading is that the rule is aspirational, and the 34 hits in
  `operant-api/src/channel.rs` sit inside a module that is dead anyway.
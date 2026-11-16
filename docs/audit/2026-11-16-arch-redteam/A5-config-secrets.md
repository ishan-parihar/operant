# A5 — Config, secrets, credentials and security guardrails

REVISION: origin/main @ 2d431ac0. Read-only (`git show origin/main:<path>`);
nothing staged, committed, or built. Every line reference is `file:line` on that
revision. Ranked CRITICAL > HIGH > MEDIUM > LOW. An empty section is correct.

## VERIFIED

### V1 — HIGH — `operant config show` and `operant dump --all` print credentials verbatim

`cmd_config.rs:94-96` serializes the whole `AppConfig` to YAML and prints it with
no masking. `cmd_dump.rs:9-20` does the same in both the YAML and `--json` forms.
None of the credential fields carry `skip_serializing`:

- `ClientSettings::api_key` — `crates/operant-core/src/config.rs:565`
- `ClientSettings::additional_api_keys` — `config.rs:568`
- `GatewaySettings::telegram_token` / `discord_token` / `slack_token` — `config.rs:1923,1926,1929`
- `GatewaySettings::email_smtp_pass` — `config.rs:1945`
- `SttSettings::groq_api_key` / `openai_api_key` — `config.rs:2414-2415`
- `AuxiliaryModelConfig::api_key` (×9 slots) — `config.rs:2609,2657, …`

The `skip_serializing` attributes at `config.rs:2747-2756` cover only the five
RETIRED web-search keys (`search_url`, `preferred_provider`, `tavily_api_key`,
`exa_api_key`, `searxng_base_url`) — dead fields, not the live credentials.

Contrast with the in-loop tool, which does mask: `tools/config_tool.rs:137` routes
its dump through `redact_secrets` (`config_tool.rs:258-282`), and the mask rule
list at :266-277 includes `api_key`, `token`, `secret`, `password`, `pass`, and
`auth`. So the model sees `***` and the operator's terminal sees the key. The
policy exists; one of the two surfaces honours it.

Trigger: a config file carrying `[client] api_key`, `[gateway] telegram_token`, or
`[stt] groq_api_key`. Wrong output: the key printed to stdout, into shell
scrollback, `script(1)` recordings, and CI logs that capture `config show`.

Fix shape: reuse `config_tool::redact_secrets` (make it `pub(crate)`) in
`handle_show` and `handle_dump_command`, the same way `config_manage action=show`
already does. Do not add `skip_serializing` to the fields — `operant config set`
round-trips through `serde_json::to_value(&AppConfig)`
(`tools/config_tool.rs:235`) and `config_backup` renders the file from the same
derive; skipping the fields would silently DROP them on the next write.

### V2 — HIGH — the approval gate has a dispatch entry point that consults nothing

`crates/operant-core/src/tools.rs:1113-1216` is `ToolRegistry::execute`. It
enforces exactly two things before dispatch: the disabled/unavailable set
(`tools.rs:1155-1186`) and the plugin/turn interceptors (`tools.rs:1200-1273`).
The blast-radius + pattern gate is NOT in it.

The gate lives one layer up, in the agent loop's pre-flight:
`agent/stream.rs:1263-1268` calls `approval::check_tool_approval`, and
`approval.rs:608` → `approval.rs:662` is the ONLY caller of
`blast_radius::screen` in the workspace (verified: `git grep 'blast_radius::'`
returns exactly `approval.rs:662`).

So any caller that reaches `ToolRegistry::execute` without passing through
`agent::stream::preflight_tool_calls` runs `terminal`, `aft_bash`,
`code_execution`, `kernel_exec`, `file_write`, `patch`, `verify_task` and `cron`
with no gate at all. Enumerated non-test callers:

| Caller | Gated? | Evidence |
|---|---|---|
| agent loop, single-tool branch | yes | `stream.rs:1263` gate, `stream.rs:1763` execute |
| agent loop, concurrent pool | yes | same gate, `stream.rs:1833`, `stream.rs:1920` |
| kernel bridge (`BridgeExecutor`) | yes, its own copy | `tools/kernel/runtime.rs:47-61` re-enters `check_tool_approval` |
| background review | no (whitelist-restricted) | `agent/prompting.rs:871-873` calls `registry_for_review.execute`; the only guard is `review_tool_names` at `prompting.rs:846` |
| `operant mcp serve` → `tools/call` | **NO** | `mcp_serve.rs:137` → `handle_tools_call` (`mcp_serve.rs:372`) → `registry.execute` at `mcp_serve.rs:404-406`. `git grep 'approval\|check_tool\|guard' mcp_serve.rs` returns ZERO hits. |
| `operant test <tool> <args>` | **NO** | `main.rs:3440-3470`: builds a registry, parses operator-supplied JSON, calls `registry.execute` directly. No approval reference. |
| gateway `/search` | no (single non-shell tool) | `gateway_runner.rs:1961-1963` drives `SessionSearchTool` directly — safe by tool choice, not by gate |
| gateway voice enrichment | no (transcription) | `gateway_runner/enrichment.rs:143` — the disabled-set check at :136 is the only guard |

**`operant mcp serve` is the one that matters.** `mcp_serve.rs:1-24` documents the
surface as "any MCP client (Claude Desktop, VS Code, etc.)"; `tools/list`
enumerates the full built-in registry via `build_tool_registry`
(`mcp_serve.rs:166`). An MCP client that can see the tool list can therefore
invoke `terminal {command: "rm -rf /"}` and nothing screens it: not the hardline
blocklist, not the 47 dangerous patterns, not the blast-radius layer. `operant
test` is operator-driven so it ranks lower, but it is the same hole with a human
holding the keyboard.

Fix shape: move the `check_tool_approval` call INTO `ToolRegistry::execute`
(beside the disabled-set refusal at `tools.rs:1180`), or give `execute` an
explicit `gated: bool` and make every non-agent caller say so out loud. The
comment at `approval.rs:643-647` already claims "It lives HERE, inside the one
function every tool call already passes through, so both enforcement sites —
`agent::stream` and the kernel's `BridgeExecutor` — inherit it." That claim is
true of `check_tool_approval`, but the function it lives in is not the dispatch
point, and the sentence is what stopped anyone from looking for the third caller.

### V3 — HIGH — `mcp_oauth::write_json` writes the token file with a create-then-chmod

`crates/operant-core/src/mcp_oauth.rs:365-427`. Sequence:

```
399:  let mut file = std::fs::File::create(&tmp)        // umask default, 0644
407:  #[cfg(unix)]
410:  file.set_permissions(0o600)?;                     // chmod AFTER create
413:  file.write_all(json.as_bytes())?;                 // secret lands after the chmod
417:  std::fs::rename(&tmp, path)
```

Two defects in one function:

1. The chmod at :410 precedes the write at :413, so the window this codebase
   elsewhere calls the R40-1 class is closed HERE but by luck, not by design:
   the bytes are written after the mode is tightened, so the content is never
   world-readable. What IS still wrong is that the file EXISTS at 0644 between
   :399 and :410 — an empty file, so the leak is metadata, not content.
2. What `write_json` persists is `OAuthToken` (`mcp_oauth.rs:101-118`), whose
   `access_token` and `refresh_token` are plaintext `String`s written by
   `set_tokens` (`mcp_oauth.rs:567-571`).

Compare the canonical writer: `fs_secrets::write_secret_file`
(`crates/operant-core/src/fs_secrets.rs:23-38`) creates with `OpenOptionsExt::mode(0o600)`
in the SAME `open(2)`, tightens an existing loose file BEFORE writing
(`fs_secrets.rs:30-35`), then sets the mode again after (`:36`). Its own test
`the_file_is_owner_only_before_a_single_byte_is_written` (`fs_secrets.rs:277-291`)
pins exactly this. `mcp_oauth::write_json` does not use it.

Also unlike the canonical path: `write_json` does not `fsync` the parent
directory after `rename`, and it does not tighten a PRE-EXISTING loose
`tokens.json`. An operator who ran an older build with a 0644 umask keeps that
mode on `tokens.json` forever, because `File::create` does not change the mode of
an existing file and the only chmod is on the fresh temp.

Fix shape: replace the body of `write_json` with
`crate::fs_secrets::write_secret_file(path, json.as_bytes())` plus a
`sync_all` on the parent, and add a `tighten_if_loose(path)` migration for the
existing file. `set_mode_reporting` (`fs_secrets.rs:110-135`) already exists for
the directory-permission half at `mcp_oauth.rs:381-386`.

### V4 — MEDIUM — `theme_files::dir()` is a second operant-home resolver and the guard does not see it

`crates/operant-cli/src/tui/theme_files.rs:20-40`:

```rust
match std::env::var("HOME") {
    Ok(home) => PathBuf::from(home).join(".operant").join("themes"),
```

This is the exact shape the one-home guard exists to catch, and it is a real
divergence: `operant_core::platform::operant_home()` (`platform.rs:173-186`)
resolves `HERMES_HOME` first, then the sticky `active_profile`. Under
`HERMES_HOME=/srv/operant` or after `operant profile use work`, the operator's
themes directory is `$HERMES_HOME/themes`, while `theme_files::dir()` keeps
returning `~/.operant/themes`. `available()` (`theme_files.rs:45-58`) lists from
the wrong root and `load()` (`theme_files.rs:95`) reads from it, so a theme the
operator saved is invisible in the `/theme` screen. Read-only surface, hence MEDIUM
and not HIGH.

The module doc at `theme_files.rs:7-10` states the rule as "`OPERANT_CONFIG_DIR`
when set and non-empty, else `~/.operant` — the same rule `operant-core`'s loader
follows, restated here because that helper is `pub(crate)`." The loader it names
does NOT follow that rule: `config.rs:3638-3662` puts `operant_config_dir_override()`
FIRST but then also pushes `platform::operant_home().join("operant.toml")`
(`config.rs:3660`), and `operant_config_dir_override` (`config.rs:3604`) reads
`OPERANT_CONFIG_DIR` — not `HERMES_HOME`, not the profile. So the comment describes
a rule that does not exist, and the code re-implements a third one.

The guard misses it because `crates/operant-core/tests/one_home_resolver.rs:150-165`
only fires when a `".operant"` literal appears within ±10 lines of the substring
`home_dir`. `theme_files.rs:36-37` has `std::env::var("HOME")` — the call is not
named `home_dir`, so the window scan finds nothing. The guard's matcher is a
name-shape heuristic, not a semantic one, and this is the second spelling of the
home that got past it (the first was the class the guard was written for).

Fix shape: delete the fallback arm and call
`operant_core::platform::operant_home().join("themes")`, then WIDEN the guard's
matcher to also fire on `env::var("HOME")` / `var_os("HOME")` in the window, and
add `crates/operant-cli/src/tui/theme_files.rs` to `ALLOWED` only if a genuine
non-home use remains.

### V5 — MEDIUM — the one-home guard's ALLOWED list names three files that no longer exist

`crates/operant-core/tests/one_home_resolver.rs:78-136`. Verified against
`git ls-tree -r origin/main`:

| ALLOWED entry | Path exists on origin/main? |
|---|---|
| `crates/operant-providers/src/lib.rs` (:117) | **NO** — the crate is gone |
| `crates/operant-providers/src/openai_codex.rs` (:123) | **NO** — the crate is gone |
| `crates/operant-config/src/platform/native.rs` (:128) | **NO** — deleted in the p2c cut |
| `crates/operant-config/src/schema/config_impl.rs` (:132) | **NO** — deleted in the p2c cut |
| `crates/operant-config/src/schema/helpers.rs` (:136) | yes |

The two `schema/*` entries also carry the reason "the DEAD schema surface —
retired with RT-141", which is an argument for DELETING them, not for exempting
them: the reason says the code is dead, and dead code that is exempted from the
guard is dead code nobody will notice coming back.

A stale ALLOWED entry is worse than a missing one: it silently widens the guard's
blind spot for whoever next creates a file at that path, and it makes the list
read as authoritative when a third of it is fiction.

Fix shape: delete the four dead rows in this commit. If the guard should also
fail on a stale entry, add an `assert!(path.exists())` per row — that converts
this class from "rots silently" to "fails the next time a crate is retired".

### V6 — MEDIUM — the guard's matcher can be walked past by a brace-in-attribute

Measured with a standalone reproduction of `strip_test_items`
(`one_home_resolver.rs:34-63`) and the window scan (`:156-166`):

- `#[cfg(test)]` followed by an attribute that contains a brace
  (`#[expect(clippy::unwrap_used, reason = "x")]`) — the scanner takes the FIRST
  `{` after the attribute as the item body, so the depth counter opens and closes
  on the ATTRIBUTE's braces and the real test function is left in the "production"
  text. A violation inside that function is then counted as production and the
  guard fires — a false POSITIVE, not a miss.
- A raw string containing the literal `#[cfg(test)]` truncates the scan at that
  point and everything after it is dropped from the "production" text — a false
  NEGATIVE.

Neither is a live defect in this tree (I ran the scan over the whole workspace
and found no file that trips either case), so this is a MEDIUM robustness note,
not a finding: the guard is the only mechanical enforcement of the one-home rule
and its parser is a brace counter. `crates/operant-config/tests/schema_is_not_loaded.rs:33-60`
carries a byte-identical copy of the same helper, so a fix has to land in both.

### V7 — MEDIUM — `handle_env_path` resolves the `.env` from `dirs::config_dir()`, a fourth path

`crates/operant-cli/src/cmd_config.rs:382-391`:

```rust
let env_path = if let Ok(config_dir) = std::env::var("HERMES_CONFIG") {
    PathBuf::from(config_dir).join(".env")
} else if let Ok(config_dir) = std::env::var("HERMES_CONFIG_DIR") {
    PathBuf::from(config_dir).join(".env")
} else if let Some(config_dir) = dirs::config_dir() {
    config_dir.join("operant").join(".env")
```

`HERMES_CONFIG_DIR` is read here and nowhere else in the workspace
(`git grep HERMES_CONFIG_DIR` → this line only). The real `.env` path is
`env_store::operant_env_path()` (`crates/operant-cli/src/env_store.rs:46-51`),
which is `operant_core::platform::operant_home().join(".env")`. `operant config
env-path` therefore reports `~/.config/operant/.env` on a default box while the
runtime reads `~/.operant/.env`. The fallback at :396 only fires when the
config-dir path does NOT exist, so the wrong answer is the COMMON case, and it is
the answer the operator will paste into a bug report.

Note this file also disagrees with its own sibling: `cmd_config.rs:154-165`
resolves the config via `HERMES_CONFIG` then the default search, and never
mentions `dirs::config_dir()`.

Fix shape: make `handle_env_path` a two-line wrapper over
`crate::env_store::operant_env_path()`, and delete the `HERMES_CONFIG_DIR` branch
(which would also remove the workspace's only reader of that variable).

### V8 — LOW — `approval_mode` is hardcoded; the config's own approval table is unreachable

`agent/mod.rs:299` — `impl From<&BehaviorSettings> for AgentConfig` sets
`approval_mode: "smart".to_string()`. `BehaviorSettings` has no approval field
(verified: the struct at `config.rs` ~:700-1300 carries `guardrail_exempt_tools`,
`guardian_llm`, `micro_compaction`, but no `approval*`). The CLI's
`agent_config()` (`main.rs:1243-1302`) sets `approval_allowlist` and
`approval_allowlist_path` but never `approval_mode`.

Meanwhile `crates/operant-cli/src/config.rs:744-756` defines `ApprovalsConfig {
mode: Option<String>, timeout: Option<u64> }` with `mode: Some("manual")`, hung
off `CliConfig.approvals` (`config.rs:80`). `git grep 'approvals\.'` finds the
field declaration and its default, and NOTHING else: nothing reads
`cli_config.approvals.mode`.

So `[approvals] mode` in `operant.toml` is a knob that parses, defaults to
"manual", and is read by no code. The effective mode is always "smart"
(`approval.rs:614-618` maps anything that is not `"manual"`/`"off"` to `Smart`).
This is the same class as the `show_turn_attribution` and
`loop_detection_enabled` literals that `agent/mod.rs:288-298` documents as
already fixed twice — the comment there is now the pattern, not the exception.

Ranked LOW because the effective default ("smart") is the safer of the two
values; the defect is a lying config surface, not a weakened gate.

Fix shape: either thread `[approvals] mode`/`timeout` into `AgentConfig` the way
`approval_allowlist` is threaded at `main.rs:1269`, or delete `ApprovalsConfig`
and its `CliConfig` field and the `approval_mode` doc-comment claim at
`agent/mod.rs:169-171` that says "smart (default), manual, or off" as if all
three were reachable.

## ASSERTED

### A1 — the blast-radius gate is defence-in-depth against a text parser, and the code says so

`blast_radius.rs:17-31` is unusually honest: "This is defense in depth, NOT a
sandbox… a determined or confused caller can still reach a destructive command
through a form this parser does not model — shell metacharacter nesting,
indirection through a script file or an interpreter, a variable expanded at
runtime, a binary whose behavior is not in `VerbKind`." :50-54 names what is
deliberately unmodelled (`docker run`, `ssh host`, `crontab -l`, command
substitution in single quotes, variable expansion, nested interpreters beyond one
level). I did not attempt to construct a bypass against `classify`
(`blast_radius.rs:938`); the module's own threat model says one exists and the
approval gate (`approval.rs:659-677`) is the layer that is supposed to catch what
this one misses. Given V2, that second layer is not present on two of the eight
enumerated entry points.

### A2 — the interpreter boundary is followed one level deep, fail-closed

`blast_radius.rs:33-46` and `segments_full_depth` (`blast_radius.rs:1289-1326`):
`bash -c 'rm -rf /'` rates `Unbounded`; an interpreter whose script cannot be
extracted becomes `UNRESOLVED_INTERPRETER` rated `Scoped` (`:1318-1322`), so it
is always visible. `MAX_INTERPRETER_DEPTH` bounds the recursion and the
depth-exhausted arm appends the sentinel rather than rating `None` (`:1323-1327`).
Verified by reading only; not executed.

### A3 — `check_deny` is only consulted on the destructive-target list, so a `Scoped` non-destructive command with a denied path passes

`screen` (`blast_radius.rs:1531-1545`) calls `destructive_targets(command)` and
runs `check_deny` over THAT list only, and only when `class >= Scoped`. A command
rated `Scoped` whose verb is not destructive (`VerbKind::Copy`, `Install`) has its
`writes` in `p.writes` (`:1301`) and its positionals in `p.targets`; whether a
denied path in `p.targets` reaches `destructive_targets` is decided at
`:1334-1348`, which I did not fully trace. If it filters to write-destinations
only, `install -m 644 /etc/passwd /tmp/x` would be rated by its `of=`/target and
the denied `/etc/passwd` would be missed. Flagging as ASSERTED because I did not
run the classifier; the test file `blast_radius.rs:2114` shows `check_deny` is
exercised through `screen` for the destructive case only.

### A4 — credential-pool refresh is single-flight and DOES persist, contradicting the audit brief's hypothesis

The brief asked whether a refresh race exists and whether the refreshed token is
lost on restart. Both answers are "no, it was handled":

- `oauth_refresh.rs:162-170` — `refresh()` delegates to `refresh_coalesced`.
- `oauth_refresh.rs:179-210` — coalesces on `refresh_account_key(provider, entry)`
  via `crate::oauth::singleflight::global()`. `oauth/singleflight.rs:1-60`
  documents the exact failure it prevents (single-use rotating refresh tokens;
  N concurrent spenders → N−1 `invalid_grant` → the account looks dead) and the
  re-check key is `(account, token)`, so a caller holding a NEWER token is not
  blocked by a finished leader.
- `credential_pool.rs:633` and `:690` both call `refresher.refresh(...)`, i.e. the
  coalescing entry point.
- Persistence: `credential_pool.rs:646-654` and `:700-708` call
  `persist_to_auth_store` and LOG A FAILURE rather than swallowing it
  (`oauth_refresh.rs:708-758`, `save_auth_store` at `:774-788` writes
  temp-file-then-`persist`). The regression test at `credential_pool.rs:1560-1573`
  pins the "refreshed but not persisted must not read as success" contract.

Two residuals, both LOW:

- `credential_pool.rs:663-668` and `:718-723` — on refresh FAILURE, a synced
  token from the auth store is adopted (`sync_from_auth_store`) and logged, but
  the in-memory `PooledCredential` is never REPLACED: the code compares
  `synced.value != cred.value` and then only emits `info!(...)`. `cred` is a
  `&PooledCredential` borrowed from the `to_refresh` Vec, which is a snapshot
  taken at `:615-625`. So the log says "Adopted synced token from auth store" and
  the pool keeps the old value until the next refresh or restart. Wrong log line,
  not a wrong token.
- `credential_pool.rs:615-625` and `:679-687` hold a `std::sync::RwLock` READ
  guard only for the filter+clone, then drop it before the network loop. That is
  correct and the comment at `:610-614` explains it. No race on the map. But the
  whole snapshot-then-loop shape means two concurrent `refresh_async` callers both
  snapshot the same expiring credential and both call `refresh`; the singleflight
  absorbs the duplicate network spend, so the residual cost is a wasted Vec clone,
  not a spent token.

### A5 — `fs_secrets` is the correct 0600-at-open primitive, and every writer the brief named uses it

Verified by reading each site:

- `fs_secrets::write_secret_file` (`fs_secrets.rs:23-38`) — `OpenOptionsExt::mode(0o600)`
  in the creating `open(2)`, pre-tighten, post-set. Test at `:277-291`.
- `fs_secrets::set_secret_perms` (`:70-83`) — TIGHTEN-ONLY; the RT-361 test at
  `:158-197` pins that a 0400 file is not widened to 0600.
- `oauth/store.rs:317` — the account index through `write_secret_file`; the file
  contains no secrets BY TYPE (`store.rs:1-32`: `TokenRef::serialize` emits a
  truncated SHA-256 fingerprint, `:107-114`, so a handle cannot reach disk).
- `tui/adapter_types/auth.rs:76` — `auth.json` through `write_secret_file`; the
  two RT-154 tests at `:138-200` pin both the fresh and the repair-a-loose-file case.
- `env_store.rs:153,193` — `create_secret_temp` (`:20-29`) creates the temp with
  `mode(0o600)`, then `set_secret_perms` after the rename (`:163`, `:202`).
- `config.rs:3401,3486,3896,6708`, `config_backup.rs:104,176`, `config_heal.rs:817`,
  `cmd_setup.rs:1343` — the config writers.
- `cronjobs/db.rs:166,169`, `database.rs:338,363-364` — the DB and its `-wal`/`-shm`.
- `async_delegation.rs:301-313` — temp-then-rename, with the reason in the comment.

Process hardening is real and runs first: `main.rs:3558-3574` calls
`hardening::harden_process()` before `Cli::parse()`, setting `RLIMIT_CORE={0,0}`
(`hardening.rs:36-56`); a refused step logs and continues by design, documented at
`hardening.rs:9-21`.

`mcp_oauth::write_json` (`mcp_oauth.rs:365`) is the ONE credential writer that
does not use it — see V3.

### A6 — socket peer authentication covers both socket servers, with an honest platform gap

`crates/operant-core/src/socket_auth.rs:1-21` exists because there was one gate
and two sockets. Verified both callers: `acp/server.rs:62` (tighten) and `:80`
(authorize peer); `session_server.rs:127` and `:147`. The Linux check is
`SO_PEERCRED` (`socket_auth.rs:79-104`); macOS/BSD has NO peer check and the
module says so at `:113-117` and `:124-131` (the `authorize_peer` stub returns
`Ok(())`), recorded as RT-Z30-B rather than papered over. The 0600 mode
(`:37-62`) refuses to serve if it cannot be set, rather than serving loose.

### A7 — skill install scanning cannot be skipped by a symlink, a name collision, or a zip-slip

- Symlinks: `check_structure` (`skills_guard.rs:1816-1856`) uses
  `symlink_metadata`, resolves the target, canonicalizes, and raises
  `symlink_escape` (Critical) when the result does not start with the canonical
  skill dir. `collect_files` (`:2164-2180`) uses `symlink_metadata` too, so a
  symlinked DIRECTORY is neither traversed nor scanned — RT-402 at `:2171-2174`.
- Unscannable path: a path that is neither file nor dir yields
  `unscannable_skill_path` (Medium) instead of zero-findings-Safe
  (`:1565-1582`, RT-398).
- Directory import: `cmd_skills.rs:868` scans the WHOLE source directory
  recursively before the copy at `:952-955`; a broken `SKILL.md` rolls the copy
  back (`:959-966`).
- Name collision: the operator pool wins by construction — the content install
  writes to `config.skills.root_dir` (`cmd_skills.rs:824-827`) and
  `SkillManager::create` refuses an existing name (`skills.rs:1003-1005`).
  `SkillManager::delete` refuses an Integration- or Project-origin skill
  (`skills.rs:1037-1051`), so a vendor skill cannot be overwritten by an operator
  install either.
- Zip-slip: no archive is extracted anywhere on this path. `install_skill`
  accepts a directory (`cmd_skills.rs:702-704`) or a single file/URL
  (`:706-742`); `copy_dir_recursive` (`:979-1003`) joins
  `dst.join(entry.file_name())` where `file_name()` is a single component, so a
  `..` entry cannot escape. `binary_updater`'s content path extracts a TAG
  TARBALL but only `skills/` (`binary_updater.rs:815-845`) and scans each
  extracted skill with the same `scan_skill`, bailing on `Dangerous` (`:826-837`).
- Force-override: `should_allow_install` (`skills_guard.rs:1610-1684`) makes a
  `Dangerous` verdict from a `Community`/`Trusted` source non-overridable
  (`:1626-1630`), matching hermes. The CLI's `--force` hint is suppressed for
  exactly that case (`cmd_skills.rs:772-776`, `:885-889`).

### A8 — the blast-radius screen's justification gate is not string-matchable

`blast_radius.rs:1546-1559` routes an offered justification through
`justification::assess(j, command)` and refuses with `InsufficientJustification`
listing the defects. `approval.rs:1533-1600` (tests) pins that a padded or
restating justification is refused and a substantive one passes. `strip_shell_comments`
(`approval.rs:979-1000`) removes the `# IGNORE ALL PREVIOUS RULES` vector before
classification. Not executed; read only.

## LEAN CUTS

### L1 — DELETE: `crates/operant-cli/src/cmd_config.rs:382-391`'s `HERMES_CONFIG_DIR` branch

Zero other readers in the workspace. Removing it deletes the only reader of that
environment variable and one of the three disagreeing `.env` path spellings
(see V7).

### L2 — DELETE: four stale rows in `one_home_resolver.rs::ALLOWED` (:117, :123, :128, :132)

Two name a crate that does not exist (`operant-providers`), two name files deleted
in the p2c cut, and one of those two carries a reason that argues for its own
deletion ("the DEAD schema surface — retired with RT-141"). See V5.

### L3 — DELETE: `ApprovalsConfig` (`crates/operant-cli/src/config.rs:742-756`) and its `CliConfig` field (:80, :137)

`mode` and `timeout` are read by nothing. The effective `approval_mode` is the
hardcoded `"smart"` at `agent/mod.rs:299`. Either wire it or delete it; leaving it
is how `show_turn_attribution` and `loop_detection_enabled` became literals that
two separate iterations had to un-hardcode (`agent/mod.rs:288-298` records both).

### L4 — DELETE: `CliConfig::config_file` / `local_config_file` / `config_version` (`crates/operant-cli/src/config.rs:53-58,114-117,1143-1146`)

`config_file` and `local_config_file` are resolved, stored, and read by nothing
outside `CliConfig::load` itself (`:1141` is the only `.config_dir` read; no
consumer touches `config_file`). The module doc at `:1129-1133` already says
`config.yaml`/`config.local.yaml` are deprecated and `operant.toml` is the sole
source — so these two fields are a deprecated path kept alive for `operant doctor`'s
legacy-file detection, which reads the PATHS from `default_config_paths()`
(`config.rs:3638`), not from this struct. `config_version` has one reader, the test
at `:1505-1507` asserting it is `"1.0.0"`.

### L5 — DUPLICATE POLICY, PICK ONE: the `.env` path

Four spellings exist:

| Site | Resolves to |
|---|---|
| `env_store.rs:46-51` (the writer + the runtime reader) | `operant_home()/.env` |
| `cmd_config.rs:383-391` (`config env-path`) | `HERMES_CONFIG`, `HERMES_CONFIG_DIR`, or `~/.config/operant/.env` |
| `crates/operant-cli/src/config.rs:116` (`CliConfig::default`) | `operant_home()/.env` |
| `cmd_doctor/checks_config.rs:117` | `operant_home()/.env` |

DELETE the `cmd_config.rs` spelling (L1) and the `HERMES_CONFIG_DIR` reader with it.
Three spellings that agree is duplication; one that disagrees is a bug.

### L6 — DUPLICATE POLICY, PICK ONE: the operant-home fallback in `theme_files`

`theme_files.rs:20-40` re-implements the home resolution because
`operant_config_dir_override` is `pub(crate)` (`config.rs:3604`). DELETE the
fallback arm, call `operant_core::platform::operant_home().join("themes")`, and
fix the module doc at `:7-10`, which describes a loader rule that does not exist
(see V4). This is the same duplication the one-home guard exists to prevent; the
guard just cannot see this spelling.

### L7 — DUPLICATE CODE: `strip_test_items` / `rust_files` / `workspace_root`

Byte-identical copies in `crates/operant-core/tests/one_home_resolver.rs:20-75` and
`crates/operant-config/tests/schema_is_not_loaded.rs:22-77`. Two workspace-scanning
guards with the same brace-counting parser is two places to fix V6. Hoist to a
`#[cfg(test)]` shared helper, or accept the duplication and fix both — but do not
fix one.

### L8 — `approval.rs` is 1,952 lines of which ~600 are tests; the extraction is clean but the file mixes four concerns

Pattern tables + compiled regexes (`approval.rs:278-560`), the guard/verdict types
(`:60-100`), the three-layer gate (`:608-724`), the command extractor
(`:770-897`), the justification helper (`:899-…`), and the guardian-LLM arm
(`:945-1100`). Splitting is not urgent; noting it because the extractor is the
part V2 says must move, and moving it lands better in its own module.

## RUST DISCIPLINE

Counts are non-test lines only (measured by truncating each file at its first
`#[cfg(test)]`).

### `unwrap` / `expect` / `panic!` in production

| File | Count | Assessment |
|---|---|---|
| `mcp_oauth.rs` | 8 | ALL annotated with `#[expect(clippy::expect_used, reason=…)]` and justified: `:264-267` (ephemeral port bind), `:1494-1497`/`:1524-1527`/`:1548-1551`/`:1569-1572`/`:1596-1599`/`:1607-1610` (poisoned mutex → "panic is the intended recovery"). |
| `approval.rs` | 2 | Both `#[expect]`-annotated static-init regex compiles: `:287-296` (`rx`), `:278-281` (`HARDFINE_REGEXES` LazyLock — no attribute but the panic IS the documented contract at `:270-277`). |
| `skills_guard.rs` | 1 | `#[expect]`-annotated at `:284`, `ThreatPattern::new` (`:293`). Same static-init argument. |
| `platform.rs` | 1 | A doc comment (`:22`), not code. |
| `config.rs`, `config_registry.rs`, `cli/config.rs`, `cmd_config.rs`, `blast_radius.rs`, `credential_pool.rs`, `tool_guardrails.rs`, `oauth_refresh.rs`, `oauth/store.rs`, `operant-config/*` | 0 | Clean. |

No bare `unwrap()`/`expect()`/`panic!()` in the slice's production paths. The
`panic!` at `approval.rs:1804` and `tool_guardrails.rs:1210,1274` are inside
`#[cfg(test)]` helpers.

### `anyhow` in library crates

`operant-core` is a library crate and uses `anyhow` in several modules
(`acp/server.rs:10`, `curator/*.rs`, `tools/kernel/mod.rs:223`,
`tools/terminal_backend.rs:92+`). AGENTS.md's rule is "typed errors in library
crates; `anyhow` is for binaries". The slice files themselves are clean —
`approval.rs`, `blast_radius.rs`, `skills_guard.rs`, `credential_pool.rs`,
`config.rs`, `platform.rs`, `oauth_refresh.rs`, `oauth/store.rs`,
`fs_secrets.rs`, `credential_files.rs`, `socket_auth.rs` use `crate::error::{Error, Result}`
throughout — but the surrounding crate does not, so the rule is enforced
per-module by convention rather than by the gate.

`operant-config/src/schema/proxy.rs:12` also uses `anyhow` and returns
`anyhow::Result` from a public API (`:44`). Same rule, same status.

### `.clone()` in loops

No clone inside a loop body in the slice. The clone sites are all justified
snapshot/ownership transfers:

- `blast_radius.rs:788` `p.targets.clone()` — the parse result is owned; the
  `RawWrite` arm clears it at `:791` and extends from `p.writes` at `:794`.
- `blast_radius.rs:1304,1320` `seg.clone()` — pushing into a worklist inside a
  `match` that also moves `seg`; forced by the borrow.
- `credential_pool.rs:866` `Some(cred.clone())` — the selection returns an owned
  snapshot after dropping the write guard; required by the `RwLock` scope.
- `credential_pool.rs:993,1012` — `Arc` clones, not data clones.
- `skills_guard.rs:1488,1516` — cache-entry clones on a cold path.
- `config.rs:3023,3037,3323,4227` — `RwLock` read-guard snapshots; `3037` and
  `3323` are explicitly section-scoped to avoid a whole-config clone (the comment
  at `config.rs:3029-3032` says so).

### `String` / `Vec<T>` parameters that should be `&str` / `&[T]`

The slice is clean. `blast_radius::check_deny(target: &str, home: Option<&str>)`,
`classify(command: &str)`, `screen(command: &str, justification: Option<&str>, home: Option<&str>)`
— all borrowed, and the module doc at `:1389-1390` explains why `home` is a
parameter rather than an env read (purity). `skills_guard::should_allow_install(result: &ScanResult, force: bool)`
— borrowed. `approval::check_tool_approval(tool_name: &str, args: &Value, mode: Option<&str>)`
— borrowed.

One exception worth noting: `skills_guard::ThreatPattern::new(pattern: &str, …)`
(`:285-291`) takes `&str` and immediately calls `Regex::new(pattern)`, which is
correct.

### `large_enum_variant`

`approval::ApprovalVerdict` (`approval.rs:60-75`) has `Blocked { reason: String }`
and `RequiresApproval { risk_level: RiskLevel, reason: String }` against a
unit `Allowed`. Largest variant is 24 bytes of `String` + 1 byte of `RiskLevel` →
32 bytes with padding, vs 0 for `Allowed`. Clippy's `large_enum_variant` threshold
is 200 bytes by default, so this does not fire and should not: boxing a 32-byte
variant to save 31 bytes on a value constructed and matched within one function
(`approval.rs:632`, `:659`, `:679`) would be a pessimization.

`blast_radius::Refusal` (`:1450-1461`) — `DeniedPath(DeniedPath)` where
`DeniedPath` is `{target: String, detail: &'static str}` = 32 bytes;
`MissingJustification { class, detail: String }` = 32 bytes;
`InsufficientJustification { class, detail: String, defects: Vec<Defect> }` = 56
bytes. Under the threshold; not worth boxing.

No `large_enum_variant` finding in the slice.

### `#[allow]` where `#[expect]` + reason belongs

Spot-checked the slice: every `#[allow(...)]` I found carries a `reason =`
(`tools.rs:39-42` `dead_code` on the serde argument struct;
`approval.rs:740` `#[cfg_attr(not(test), allow(dead_code))]` on
`SHELL_CARRYING_TOOLS` with the contract explained at `:737-739`;
`credential_pool.rs:1` test module). No bare `#[allow]` without a reason in the
audited files.

### Doc-comment drift found

Two doc comments in the slice describe behaviour that does not exist:

- `theme_files.rs:7-10` — "the same rule `operant-core`'s loader follows" (it does
  not; see V4).
- `approval.rs:643-647` — "It lives HERE, inside the one function every tool call
  already passes through" (it lives in `check_tool_approval`, which is not the
  dispatch point; see V2). This one is load-bearing: it is the sentence that
  stopped the search for a third caller.

## COVERAGE

### Read in full

`platform.rs`, `fs_secrets.rs`, `socket_auth.rs`, `credential_files.rs`,
`oauth/singleflight.rs` (head), `operant-config/src/lib.rs`,
`operant-config/src/policy.rs` (head), `operant-config/src/schema/mod.rs`,
`operant-config/tests/schema_is_not_loaded.rs`,
`crates/operant-core/tests/one_home_resolver.rs`, `crates/operant-cli/src/env_store.rs`,
`crates/operant-cli/src/tui/theme_files.rs`, `crates/operant-cli/src/cmd_dump.rs`,
`crates/operant-cli/src/hardening.rs` (head).

### Read in large part

`config.rs` (loader, `RUNTIME_CONFIG`, `AppConfig` shape, config writers,
`default_config_paths`, `persist_config_key`), `approval.rs` (verdict types,
`check_tool_approval`, `extract_command_from_args`, `SHELL_CARRYING_TOOLS`,
guardian arm, dangerous-pattern layers), `blast_radius.rs` (module doc,
`VerbKind`, `check_deny`, `screen`, `Refusal`, `segments_full_depth`,
`interpreter_script`), `credential_pool.rs` (`PooledCredential`, pool ops,
`refresh_async`, `refresh_oauth_async`), `skills_guard.rs` (`scan_skill`,
`should_allow_install`, `check_structure`, `scan_skill_cached_in`,
`collect_files`), `mcp_oauth.rs` (`OAuthToken`, `TokenStorage`, `write_json`),
`oauth_refresh.rs` (`refresh`/`refresh_coalesced`, `persist_to_auth_store`,
`save_auth_store`), `cmd_skills.rs` (both install paths, `copy_dir_recursive`),
`cmd_config.rs` (`handle_show`, `handle_set`, `handle_env_path`),
`cli/config.rs` (`CliConfig`, `to_app_config`, `load`), `tools/config_tool.rs`,
`agent/stream.rs` (`preflight_tool_calls` approval block, `execute_tools`
phase 1/2), `tools.rs` (`ToolRegistry::execute`, `ToolExecutor`),
`tools/kernel/runtime.rs` (`BridgeExecutor`).

### Grepped exhaustively

`home_dir()` (26 hits, every non-test site triaged); `.join(".operant")`
(13 hits); `write_secret_file`/`set_secret_perms`/`tighten_if_loose` (all
callers); `auth.json` writers (all 4); `check_tool_approval` (2 non-test callers
+ the kernel bridge); `blast_radius::` (1 caller); `refresh_coalesced` /
`.refresh(` (all); `OPERANT_CONFIG_DIR`, `HERMES_CONFIG`, `HERMES_CONFIG_DIR`
(all readers); `approval_mode` (all 30 hits); `guardian_llm` / `OPERANT_GUARDIAN_LLM`
(all); `approvals.` (all); `redact|mask` in config surfaces; `Command::new(` in
tools (30+ hits, all argv-based or shell-string with an extractor arm).

### NOT covered (out of slice, or not reached)

- `crates/operant-core/src/config.rs` lines ~1300-2400 (the ~40 settings structs)
  were read for credential fields and `deny_unknown_fields` attributes, not line
  by line. A secret-bearing field with an unusual name would be missed.
- `crates/operant-cli/src/cmd_doctor/checks_config.rs` (1,400+ lines) — read only
  the `.env`-related arms and `display_home`.
- `mcp_oauth.rs` beyond the token-storage and provider-cache paths (the OAuth
  grant flows, `refresh_anthropic`/`refresh_codex`/`refresh_xai`/`refresh_nous`,
  and the loopback callback server) were not audited.
- `credential_pool.rs` selection strategies (FillFirst / RoundRobin / Random /
  LeastUsed) were read for the refresh question only; the strategy fairness and
  the `select_inner` rotation logic were not audited.
- `skills_guard.rs`'s ~90 threat patterns (`skills_guard.rs:300-1230`) were read
  for structure, not for coverage: whether a given exfiltration shape is in the
  table was not tested.
- No code was compiled or executed. Every claim above is from source reading;
  the only executed artefacts were the two standalone Rust programs used to
  measure V6's parser behaviour (`/tmp/guardtest/t.rs`, `/tmp/guardtest/t2.rs`),
  which reproduce `strip_test_items` verbatim and are not part of this repo.
# Security — the guard stack

Owns: operant's security mechanisms — the approval ladder
(`check_tool_approval`, the three `ApprovalGuard` layers and the blast-radius
gate behind them), the interactive permission channel, redaction of secrets
out of text, protected-path handling (the built-in deny lists and the
credential-file registry), SSRF protection for outbound URLs, the write-origin
provenance seam the write guards read, and the tool guardrails that bound a
runaway turn.
Not for: skills' content scanning ([skills.md](skills.md)), the org / Holon
boundary and per-seat policy and budgets ([org.md](org.md)), process isolation
and the kernel sidecar ([kernel.md](kernel.md)).

Source: `crates/operant-core/src/approval.rs` (the ladder and the guard),
`crates/operant-core/src/blast_radius.rs` (structural classification,
`screen()`), `crates/operant-core/src/justification.rs` (substance
measurement), `crates/operant-core/src/redaction.rs` (`RedactPatterns`,
`redact_sensitive_text_if_enabled`), `crates/operant-core/src/credential_files.rs`
(the process-global registry plus the built-in list), `crates/operant-core/src/fs_secrets.rs`
(0600-at-create secret writes), `crates/operant-core/src/security.rs`
(`check_url_safety` / `ssrf_verdict` — this module's *other* half,
`osv_check` / `check_osv`, has no production callers and must not be wired
up), `crates/operant-core/src/write_origin.rs` (`TASK_ORIGIN`),
`crates/operant-core/src/write_approval.rs` (staged writes — also has zero
production callers), `crates/operant-core/src/tool_guardrails.rs`,
`crates/operant-core/src/tools/file_tools.rs` (path guards, `holon_command_fence`),
`crates/operant-cli/src/hardening.rs` (startup process hardening),
`crates/operant-api/src/glob.rs` (the `*`-only matcher behind the two
tool-exposure allowlists — the frozen dialect that is a permission
widening, not a refactor, to extend).

Counts below were read from the code on 2026-11-17 at `449c9dd2`; they move
when the code moves.

## Contents

- [The shape in one sentence](#the-shape-in-one-sentence)
- [Layer 1 — the hardline blocklist (`approval.rs:132`)](#layer-1-the-hardline-blocklist-approval.rs132)
- [Layer 2 — the dangerous patterns (`approval.rs:420`)](#layer-2-the-dangerous-patterns-approval.rs420)
- [Layer 3 — the interactive permission channel](#layer-3-the-interactive-permission-channel)
- [Blast radius (`blast_radius.rs:1199`)](#blast-radius-blast_radius.rs1199)
- [Justification substance (`justification.rs:1`)](#justification-substance-justification.rs1)
- [Redaction (`redaction.rs`)](#redaction-redaction.rs)
- [Protected paths — the two deny layers](#protected-paths-the-two-deny-layers)
- [SSRF (`security.rs`)](#ssrf-security.rs)
- [Tool guardrails (`tool_guardrails.rs`) — the runaway bound](#tool-guardrails-tool_guardrails.rs-the-runaway-bound)
- [Process hardening (`hardening.rs`, called from `main.rs:3422`)](#process-hardening-hardening.rs-called-from-main.rs3422)
- [Failure modes](#failure-modes)
- [Configuration](#configuration)

## The shape in one sentence

Every tool call passes through ONE function, `check_tool_approval`
(`crates/operant-core/src/approval.rs:608`), which runs `ApprovalGuard::check`
and — only if that returned `Allowed` and the mode is not `Off` — ONE nested
gate (the blast-radius screen), and the result is a
`CheckToolApprovalResult { verdict, risk_level, reason, blocked_by }` that the
caller maps to an error tool-result, a permission prompt, or silence. The
same function sits on both enforcement paths: `agent/stream.rs:1254` for the
interactive run and `tools/kernel/runtime.rs:48` for the kernel sidecar's
bridged tool calls.

The literal verdict strings are `allowed`, `blocked`, `requires_approval`
(`approval.rs:673-740`). The ladder and the seat policy are consulted in order:
`check_tool_approval` first, then the seat consult in the run guard
(`agent/stream.rs:1335`); what `requires_approval` MEANS for this run is the
seat's call in org mode, or the permission-channel / allowlist / headless-deny
logic of `stream.rs` in personal mode.

## Layer 1 — the hardline blocklist (`approval.rs:132`)

`HARDLINE_BLOCKLIST: &[HardlineEntry]` is a static table of 14 entries in
twelve distinct `category` strings. Under `ApprovalMode::Smart` a match returns `Blocked { reason }` from
`guard.check` (`approval.rs:474`). Under `ApprovalMode::Off` the match is
never reached — that mode early-returns `Allowed` as the first arm of
the match in `check`. That is the single most surprising fact in the
ladder:

```rust
match self.mode {
    ApprovalMode::Off => ApprovalVerdict::Allowed,
    ...
}
```

`compile_hardline_regexes(HARDLINE_BLOCKLIST)` (`approval.rs:279`) is
consulted by `check_hardline_blocklist` (`approval.rs:526`) for regex
entries; a malformed regex used to degrade to a non-match (a hardline entry
that could never fire). It is now compiled once behind `LazyLock` and a
failure to compile is a hard error at first use, and a test pins that
(`malformed_hardline_regex_is_reported_not_swallowed`). The shipped table is
checked at test time by compiling the whole `HARDLINE_BLOCKLIST`.

The twelve shipped categories, each appearing once except as noted:
`WILDCARD_ABUSE` (2 entries), `DANGEROUS_COMMANDS`, `NETWORK_ABUSE` (2
entries), `CRYPTO_ABUSE`, `SYSADMIN_RISK`, `EXPOSURE_RISK`,
`PRIVILEGE_ESCALATION`, `PACKAGE_RISK`, `NETWORK_EXFIL`,
`SERVICE_DISRUPTION`, `DATA_DESTRUCTION`, `INFRA_EXFIL`.

## Layer 2 — the dangerous patterns (`approval.rs:420`)

`ALL_DANGEROUS_PATTERNS` is twelve pattern families whose regex tables
total **60** literals (counted with `rx(` occurrences between the family
declarations on 2026-11-17):

| Family | Patterns | Family | Patterns |
|---|---|---|---|
| `FILE_OPS_PATTERNS` | 7 | `CRYPTO_PATTERNS` | 3 |
| `NETWORK_PATTERNS` | 5 | `ENV_PATTERNS` | 3 |
| `EXECUTION_PATTERNS` | 8 | `SSH_PATTERNS` | 5 |
| `PERMISSION_PATTERNS` | 5 | `CONFIG_PATTERNS` | 5 |
| `PROCESS_PATTERNS` | 4 | `PACKAGE_PATTERNS` | 6 |
| `DATA_PATTERNS` | 4 | `DOCKER_PATTERNS` | 5 |

A match returns `RequiresApproval { risk_level, reason }`. The risk level is
whichever `RiskLevel` the pattern table was compiled with for that family —
there is no per-pattern override. The verdict is advisory: it does not stop
the call; it makes the run guard ask.

Shell-bearing tool names live in `SHELL_CARRYING_TOOLS` (`approval.rs:741`) —
`terminal`, `process`, `code_execution`, `kernel_exec`, `verify_task`,
`cron`, `aft_bash` (7 entries). `extract_command_from_args` (`approval.rs:770`)
is the ONE extractor the guard uses; it has an arm for every name in that
set, plus arms for `file_write`, `patch`, `verify_task`, `cron`, `aft_bash`.
A shell-carrying tool without an arm hands the guard the tool's own NAME — a
value no pattern matches — so the layers below the extractor silently do
nothing. The drift test `shell_carrying_tools_have_extractor_arms` pins the
pair.

Note that the module documentation says "47+ regex patterns"; the actual
count is 60. The comment was not updated with the table.

## Layer 3 — the interactive permission channel

If nothing above blocked the call, the caller decides who answers:

- In the TUI the call becomes a `ToolPermissionRequest` on the
  `permission_tx` channel. The TUI drain (`tui/app/channel_drains.rs:464`)
  reads `settings.permission_mode` — `BypassPermissions` answers
  `AllowSession` without opening a dialog; `Plan` answers `Deny` with a
  status notice; `AcceptEdits` / `Default` fall through to the dialog. The
  dialog (`tui/app/dialog_routing.rs:120`) maps `y` → `AllowOnce`, `Y`
  and `P` → `AllowSession` (and `P` records the two-token bash prefix in
  `bash_prefix_allowlist` via `maybe_record_bash_prefix`, `dialog_routing.rs:241`),
  `p` → `AllowAlways`, `n`/Esc/anything else → `Deny`.
- Over the gateway the pending permission sits in
  `PENDING_PERMISSIONS` until `/approve` (channel answer: `AllowSession`)
  or `/approve always` (persistent: `AllowAlways`) is read from the chat
  (`gateway_commands.rs:3103`).
- With no channel at all — a headless or unattended run — the
  `agent/stream.rs:1507` branch denies the flagged call outright with the
  text `"Refused: this call was flagged for approval and no interactive
  approver is attached to this run (headless). Run interactively to
  approve it once, or add it to your approval allowlist."` Approved-by
  -allowlist and guardian-LLM-approved calls do not hit this branch.

The TUI's `PermissionMode` enum is at `tui/adapter_types/config.rs:32`
(`AcceptEdits`, `Default`, `BypassPermissions`, `Plan`; default
`Default`). It is the only place the mode can be toggled; it is separate
from `AgentConfig.approval_mode` (`agent/mod.rs:171`, default `"smart"` at
`agent/mod.rs:275`), which selects `Smart` / `Manual` / `Off` for the
guard — the TUI does not write back into `AgentConfig.approval_mode`.

`AllowAlways` sends `AllowAlways` up the oneshot; the agent's
`persistent_allowlist` (a `Mutex<HashSet<String>>` on `OperantAgent`) is
updated and `persist_approval_allowlist` (`approval.rs:832` via
`agent/mod.rs` — the fn lives in the `agent/mod.rs` module beside the
struct) writes the set to `config.approval_allowlist_path`, which the CLI
sets to `<operant home>/approval_allowlist.json` at startup
(`cmd_build`/`main.rs:1252`). The read path is `approval_allowlist_from_config`
(`agent/mod.rs:1123`): config seeds union the file's patterns; a missing or
malformed file yields the config seeds only — a corrupt allowlist is never a
silent bypass. Matching goes through `context::lcm::glob_match`
(`context/lcm.rs:49`), the same matcher the seat policy uses. The patterns
are seeded from `command_allowlist` in the CLI config
(`operant-cli/src/config.rs` `SecurityConfig`, default empty; the
`command_allowlist` field in core `AppConfig`, default empty,
`config.rs:104`); the example config shows them at `operant.example.toml:23`.

The allowlist is consulted inside `agent/stream.rs:1584` — before any
prompt — and a match answers `AllowSession` without opening a dialog
(`(false, String::new())` in the `needs_prompt` computation). It does not
unlock a hardline block, and it does not bypass Plan mode (the mode answers
first in `channel_drains.rs`).

## Blast radius (`blast_radius.rs:1199`)

`screen(command, justification, home)` is the SUBSTANCE gate behind layer 3.
It is a pure function: text in, decision out. It never reads a file,
touches a socket, or runs a process; the entire classification taxonomy is
testable without a shell. It is called from exactly one place —
`check_tool_approval` — after `ApprovalGuard::check` returns, so it sees
the same command string the guard saw, extracted by the same
`extract_command_from_args`.

The command is split into segments, normalized, and classified by VERB KIND:
`Remove` / `RawWrite` / `Format` / `RecursiveMeta` / `Finder` / `Write` /
`Vcs` (`enum VerbKind` at `blast_radius.rs:105-130`); the output class is
`BlastRadius::None` / `Scoped` / `Unbounded` / `Device`
(`pub enum BlastRadius` at `blast_radius.rs:66-83`, with `Ord` meaningful:
`None < Scoped < Unbounded < Device`). `>= Unbounded` demands a justification;
`>= Scoped` additionally consults the path denies.

Classifier state, all read out of the source on 2026-11-17:

- `SYSTEM_ROOTS` = 16 entries (`etc`, `usr`, `bin`, `sbin`, `boot`, `lib`,
  `lib32`, `lib64`, `libx32`, `dev`, `proc`, `sys`, `var`, `root`, `run`,
  `snap`) — used for boundedness only, NOT for refusal (`/dev` is here so
  that `rm -rf /dev` rates Unbounded, but it is deliberately NOT in
  `DENIED_SYSTEM_DIRS`).
- `DENIED_SYSTEM_DIRS` = 15 entries (`etc`, `usr`, `bin`, `sbin`, `boot`,
  `lib`, `lib32`, `lib64`, `libx32`, `proc`, `sys`, `var`, `root`, `run`,
  `snap`; `/dev` excluded on purpose — device writes reach the justification
  gate instead).
- `INTERPRETER_VERBS` = 19 entries (`bash`, `sh`, `zsh`, `dash`, `ksh`,
  `csh`, `tcsh`, `fish`, `ash`, `env`, `sudo`, `doas`, `nice`, `nohup`,
  `timeout`, `setsid`, `stdbuf`, `ionice`, `trap`) — the verbs the
  interpreter unwrapper looks through, recursively,
  `MAX_INTERPRETER_DEPTH = 8` deep, at most
  `MAX_INTERPRETER_INPUT = 8192` bytes (`blast_radius.rs:919/931`). Past
  the input cap the initializer reports fail-closed (an unwrap it cannot
  follow rates as an unknown command, not as safe).
- `segments()` splits on shell control operators and recursively unwraps
  every interpreter head; an un-followed interpreter yields a fail-closed
  sentinel, not `None`.
- SQL destructive detection is `sql_class` (`blast_radius.rs:450`) —
  `drop table/database/schema/index`, bare `delete from` with no `where`,
  `truncate`.
- `BENIGN_DEV` = 14 entries; `BLOCK_DEVICE_PREFIXES` = 12;
  `LEADING_WRAPPERS` = 10.

The denial is a `Refusal` enum (`blast_radius.rs:1118`):

| Variant | What it means | Can a justification unlock it |
|---|---|---|
| `DeniedPath(DeniedPath { target, detail })` | target is on `DENIED_SYSTEM_DIRS` / `.git` / `SYSTEM_ROOTS` | **No** |
| `MissingJustification { class, detail }` | high blast radius, no justification offered | Yes, but it must pass `justification::assess` |
| `InsufficientJustification { class, detail, defects }` | offered but not substantive | Yes, same gate |

`check_deny` (`blast_radius.rs:1070`) compares the first path segment of the
normalized target against `DENIED_SYSTEM_DIRS`. `screen` refuses at line
1209 (`DeniedPath`) before it ever reaches the justification gate, and the
refusal text says "This is NOT overridable by a justification."

## Justification substance (`justification.rs:1`)

`assess(justification, command) -> Assessment` (`:300`) is PURE. It measures
substance with four distribution-shape signals, tuned so that padding can and
must not move them:

- `MIN_DISTINCT_CONTENT_WORDS = 8` — distinct non-stopword tokens of 3+
  characters (`MIN_DISTINCT_CONTENT_WORDS = 8` at `:44`).
- `MAX_REPETITION_RATIO = 5.0` — `total / distinct`; above this the text is
  a hammered sentence (`:49`).
- `MAX_COMMAND_OVERLAP = 0.35` — share of content words that ALSO appear in
  the command; a restatement of the command fails it (`:54`).
- `MIN_DIVERSITY = 0.70` — normalized Shannon entropy; below this the
  vocabulary is too narrow (`:60`).

`STOPWORDS` deliberately lacks domain words (`build`, `cache`, `stale`,
`remove`) — those ARE the substance being measured. `STOPWORDS` starts at
`justification.rs:76`. The module's own doc
says there is deliberately no length floor: a short real reason passes, a
long padded one fails. The module also names its own ceiling: these are
lexical measures; 20 distinct words of Greek filler pass. That's deliberate —
a model call inside a safety gate is a worse failure mode.

Defects are `TooFewDistinctWords`, `Repetitive`, `EchoesCommand`,
`LowDiversity`; each carries its measured number into the refusal message
(`blast_radius.rs:1160-1180`).

## Redaction (`redaction.rs`)

Hermes's `redact.py` ported at its highest value: secrets never reach the
model, session logs, transcripts, or the client request builder.
`RedactPatterns` is a set of compiled regexes built once behind
`LazyLock`; one `Regex` per provider token family — **47 prefixes** in the
`prefixes` array (`sk-…`, `sk_…`, `ghp_…`, `github_pat_…`, `xox[baprs]-…`,
`AIza…`, `AKIA…`, `sk_live_…`, `SG.…`, `hf_…`, `glpat-…` through `glagent-…`,
`gsk_…`, `r8_…`, `npm_…`, `mem0_…`, `am_…`, `hsk-…`, …) — plus private-key
blocks, `env_assign` (`KEY=…` where the key carries a secret keyword),
`auth_header`, `db_connstr`, `jwt`, `telegram`, `url_userinfo`,
`url_query_param`, `json_field` patterns — 11 pattern families in total.

First callers were `client.rs` (message content builder) and the error
paths; wiring lives at the LLM call boundary and the tool-result boundary.
The toggle is `REDACT_ENABLED: AtomicBool`, default ON, and the env override
is `OPERANT_REDACT_SECRETS` — `0/false/off/no` disables, `1/true/on/yes`
enables, anything else falls through to the toggle.

The collector that decides whether to redact at all is `needs_redaction`
(`redaction.rs:287`) — **61 pre-screen needles** behind a single
`starts_with`/`contains` pass, so a clean string never pays the regex cost.

`redact_sensitive_text_if_enabled` is the entry point the rest of the tree
calls (tool-output boundary, error path). `security.redact_secrets`
(`crates/operant-cli/src/config.rs:760` — note: the CLI's deprecated YAML
`SecurityConfig` struct owns the field, the core config carries
`redact_secrets` on the SAME struct name; the CLI load path reads it from
the default config object and applies `set_redact_enabled(cli_config
.security.redact_secrets.unwrap_or(true))` at `main.rs:3466` — when nothing
sets it, the toggle stays `true`). The env override is checked FIRST in
`redaction_enabled`, so `OPERANT_REDACT_SECRETS=0` wins over any config
value.

## Protected paths — the two deny layers

### `DENIED_PATH_PATTERNS` (built-in, `file_tools.rs:37`)

**22** entries (counted 2026-11-17, written as suffixes so they read as
patterns rather than repo paths): SSH private keys and identity files
(id_rsa, id_dsa, id_ecdsa, id_ed25519, identity, identity.pub); AWS
credentials and config; gcloud credentials and
application_default_credentials.json; Azure credentials; operant auth.json
and mcp-tokens; netrc; npmrc; pypirc; docker config.json; kube config;
/etc/shadow and /etc/gshadow; and the macOS login keychain files. Matched at
`file_tools.rs:352` by `canonical_lower.ends_with(pattern)` — so `.npmrc`
matches but `my.npmrc` also matches (a suffix test, not a basename test).

### The registerable registry (`credential_files.rs`)

`BUILTIN_PROTECTED_RELATIVE` = 21 entries: `.env`, auth.json,
`.anthropic_oauth.json`, mcp-tokens, `.git-credentials`, `.netrc`,
`.pgpass`, `.npmrc`, `.pypirc`, the five `.ssh` key/identity files plus
`.ssh/config`, `.aws/credentials` and `.aws/config`, the gh hosts.yml, kube
config, docker config.json, and `.azure/credentials`; unioned with the
registered list at `protected_path_reason` (called from `file_tools.rs:365`).
Matching
is **lexical** (`lexical_absolute`), not `canonicalize()` — the file tools
`canonicalize` the path first, so a symlink to `~/.ssh/id_rsa` still
terminates inside the deny check at its canonical form. The built-in check
is fail-closed: a path with no absolute form is treated as protected (a
spurious block is recoverable; a leak is not).

`register_credential_file` is **public with zero production callers**:
the registered vector is only ever populated by tests. The built-in list is
the whole population today. Registration does validate (rejects absolute
paths and `..`), so a plugin could not widen the guard except by valid
relative paths.

### `write_approval.rs` and `write_origin.rs` — the write seams

`write_origin.rs` carries `TASK_ORIGIN` as a `tokio::task_local!`; the
background review brackets its own writes with
`.scope(TaskOrigin::BackgroundReview, …)` and the skill write guard reads it
to tell a human-written turn from a machine-written review.

`write_approval.rs` is a staged-writes registry. It is **not wired**: no
production caller, default off, and the module rustdoc names the gate as
developer tooling, not a live enforcement layer. Do not point operators at
it — there is nothing to toggle.

`env_probe.rs` / the `env_probe` tool reports secret-looking env-variable
NAMES and their source only, never values.

`fs_secrets.rs` writes 0600 at creation (`write_secret_file`) and
`tighten_if_loose` fixes a loose pre-existing file; its production call
sites are the setup wizard's token file and `env_store.rs`.

## SSRF (`security.rs`)

`check_url_safety(url)` (`:52`) parses the URL, resolves the hostname, and
rejects if any resolved address falls into a blocked range — loopback,
private (10/8, 172.16-31/12, 192.168/16), link-local (169.254/16), CGNAT
(100.64/10), benchmark (198.18/15), reserved (240/4), cloud metadata
(`metadata.google.internal`, `metadata.goog`), or unique-local v6.

`ssrf_verdict` (`:115`) is the boolean-shaped wrapper the tools call: it
returns `(false, "URL blocked: …")` on a blocked range, `(false, "URL safety
check failed: …")` on a DNS error, and `(true, "")` only on safe. **Fail
closed on resolution errors** is the rule: a hostname that resolves to a
private address, or does not resolve at all, does NOT get fetched.

Call sites 2026-11-17 — every outgoing HTTP fetch in the tree:
`tools/browser_tool.rs:351` (`browser navigate`), `tools/vision_tool.rs:236`
(`vision_analyze`), `tools/sourcehound.rs:637/706/788` (the three web
tools), `tools/web_tools.rs:201` (`web_fetch`), `tools/http_tool.rs:66`
(`http_request`). `web_fetch` sets `redirects(Policy::none())`; the SSRF
pre-flight validates the initial URL only, so a redirect cannot re-target
it. `check_osv` / `do_check_osv` in the same file have no production callers
(`tools/osv_check.rs` is the wired version — note this when reading
`security.rs`; the module docs say so).

## Tool guardrails (`tool_guardrails.rs`) — the runaway bound

Not a permission gate; a per-turn repetition-and-no-progress bound. A
`ToolGuardrails` tracker (constructed per turn) observes every `(tool,
args)` pair after each result. Constants read 2026-11-17:

| Constant | Value | Meaning |
|---|---|---|
| `NO_EFFECT_TOOL_NAMES` | 14 entries | read-only tools; warn first, skip at 4th identical call |
| `REPEAT_WARN_THRESHOLD` | 3 | identical calls before the model is warned |
| `REPEAT_SKIP_THRESHOLD_EFFECT` | 3 | side-effecting repeats skipped at the 3rd |
| `REPEAT_SKIP_THRESHOLD_NO_EFFECT` | 4 | read-only repeats skipped at the 4th |
| `PATTERN_WINDOW_SIZE` | 20 | trailing window for the ping-pong rung |
| `PING_PONG_MIN_CYCLES` | 4 | A→B→A→B… alternation warned at |
| `PING_PONG_SKIP_CYCLES` | 5 | and skipped at |
| `NO_PROGRESS_MIN_CALLS` | 5 | consecutive identical results before the model is warned |

`GuardrailDecision` is `Allow` / `Warn` / `Skip` / `Halt(String)` —
`Halt` ends the turn with a failure copy (`failure_copy`) and
`HARD_REJECT_PREFIX = "Blocked by security policy"` marks the results that
halt. `is_permission_gated_tool` (`agent/mod.rs:1063`) — the same list the
prompts key on — mixes `bash`, `terminal`, `execute_command`,
`code_execution`, `file_read|write|edit`, `patch`, `process`, `browser`,
`memory_store`, `skill_manage`, `learning_manage`, `aft_bash`, `aft_write`,
`aft_edit`, `aft_apply_patch`, `verify_task`, `cron`, `delegate_task`. The
batch planner `batch_allows_parallel_execution` vetoes a parallel batch
whenever any gated call is in it, regardless of whether the others are
read-only.

## Process hardening (`hardening.rs`, called from `main.rs:3422`)

`harden_process()` runs before config load and before the runtime:
`RLIMIT_CORE` is set to zero (no core dumps) and, on Linux,
`PR_SET_DUMPABLE` is cleared (no ptrace). A refused step is LOGGED and the
process continues — this diverges from codex's `exit()` on failure, on the
principle that partial hardening is better than a process that will not
start because a kernel refused an `prctl`. The caller in `main.rs:3418`
prints the step name to stderr.

`code_execution` (`tools/code_execution.rs`) is **NOT a sandbox**. Code runs
as the operant process. The tool exists for measurement escape and small
scripting; the walls are: wall-clock timeout, process-group teardown via
`kill_on_drop`, and nothing else. The tool description and the module rustdoc
carry this in plain language.

## Failure modes

| What breaks | What the caller sees |
|---|---|
| Hardline match | tool result `"Blocked by security policy: <category>"` (`HARD_REJECT_PREFIX`) |
| `rm -rf /` with no justification | `Refused: Unbounded command — <detail> … re-issue the SAME call with a justification …` (`blast_radius.rs:1160`) |
| `rm -rf /etc` | `Refused: '\`/etc/\` is a denied path (system-directory). This is NOT overridable by a justification …` |
| Padded justification | `InsufficientJustification` naming which defects (`TooFewDistinctWords` / `Repetitive` / `EchoesCommand` / `LowDiversity`) and the measured numbers |
| DNS failure on `web_fetch` | `"URL safety check failed: <err>"` refusal and NO fetch |
| Resolves to loopback | `"URL blocked: points to private/internal address (SSRF protection)"` and NO fetch |
| Headless flagged call | `"Refused: this call was flagged for approval and no interactive approver is attached …"` |
| Corrupt `~/.operant/approval_allowlist.json` | Config seeds only; a malformed file is never honored as an allowlist |
| Unwritable `~/.operant/approval_allowlist.json` | `AllowAlways` works for this process (`session_allowlist`); the persistent copy is absent next start — the choice must be re-made, it does not silently bypass |
| `HARDLINE_BLOCKLIST` regex fails to compile | Error at first use; a tests-time compile of the whole table fails the build |
| Process dumps on crash | Will not exist — `RLIMIT_CORE=0`, and ptrace is refused. The operator sees `Warning: process hardening step <step> was refused by this kernel` on stderr if the kernel refuses |

## Configuration

| Key | Where | Default | What it changes |
|---|---|---|---|
| `command_allowlist` | flat in `operant.toml` | `[]` | tools whose name globs (incl. `*`-`?`) skip the permission prompt |
| `approval_allowlist_path` | `AgentConfig` (CLI sets to `<home>/approval_allowlist.json`) | set by CLI at startup | where `AllowAlways` persists; absent → session-only |
| `approval_mode` | `AgentConfig` | `"smart"` | `"manual"` prompts for everything; `"off"` disables the guard incl. the blast-radius gate |
| `security.redact_secrets` | CLI `SecurityConfig` (legacy YAML path deprecated, value read at startup) | `Some(true)` | see `operant-cli/src/config.rs:760` |
| `OPERANT_REDACT_SECRETS` | env | unset = config value | `0/false/off/no` disables redaction |
| `OPERANT_GUARDIAN_LLM` | env | unset = off | arms the auxiliary guardian-LLM approver (per-call LLM round-trip); `1/true/on/yes` enables |
| `settings.permission_mode` | TUI `Settings` (`tui/adapter_types/config.rs:67`) | `Default` | `BypassPermissions` answers `AllowSession` without a dialog; `Plan` answers `Deny` |

## Intention

The stack is layered because no single gate covers the failure space: the
model proposes destructive commands (blast radius decides), the transcript
leaks secrets (redaction decides), a skill exfiltrates (skills_guard and
url_safety decide), a background writer mutates (write_origin decides), an
unattended run has nobody to ask (the headless deny decides), and a
runaway turn repeats itself forever (the guardrails decide). Each layer
exists because a specific incident forced it — the module docs carry those
incidents inline (RT-454, RT-322, RT-426, R47-1, R47-2, R48-1, R48-2,
R39-10, iter-338).

Two deliberate non-goals:

1. **No sandbox.** `code_execution` runs as the operant process. Only the
   kernel sidecar enforces a filesystem boundary, and only for the cells it
   runs.
2. **No semantic judgment on text.** Blast radius reads command TEXT; it is
   not a model. A crafted `bash -c "$(printf %s rm -rf / | sed …)"`
   rates however the literal text rates. The claim is narrower: a careless
   or average-bad-faith model cannot cheaply, silently, or undetectably
   perform a high-blast-radius action. The classifier's boundary is named
   in its own docs.

A reader leaving with this page should verify three things before relying on
it: which tools are in `is_permission_gated_tool`, which tools are in
`SHELL_CARRYING_TOOLS`, and which env vars are set in the process (`OPERANT_*`).
All three are small constants that change when this page is wrong.

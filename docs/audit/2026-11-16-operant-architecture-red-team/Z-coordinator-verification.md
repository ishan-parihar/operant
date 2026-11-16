# Lane Z — coordinator's own verification pass

Dated-frozen: audited `2026-11-16`.

**Lane:** Z (coordinator). This doc holds only what I verified myself, by
running a command or reading the code end to end. Lane docs A–J hold the
per-seam findings; this doc is deliberately disjoint from them so the
synthesis can cite each claim to one reader.

**Contents:** [Coverage caveat](#coverage-caveat) · [Gate state](#gate-state-measured-this-revision) · [Lane findings I verified myself](#lane-findings-i-verified-myself) (`Z1`–`Z37`) · [Non-findings](#non-findings-recorded-so-they-are-not-re-litigated) · [Method note](#method-note)

## Coverage caveat

| item | value |
|---|---|
| pinned revision | **`25b86ae5`** (`feat(iter-2472): retract the /bar status-line half-port, theme the input area itself`) |
| revision at dispatch | `1aa63cb5` — a peer landed iter-2469..2472 while the fleet was reading; every finding below was **re-derived against `25b86ae5`** |
| tree state at audit | clean except untracked peer scratch (`.w10a-tmp`, `.w10b-tmp`, `.w10d-tmp`, `.tmp-test`, `crates/operant-cli/src/lib.rs`, `crates/operant-core/memory:/`) — none of which the build reads |
| peer WIP preserved | a 40-file bar-retraction WIP was stashed to `stash@{0}` before the ff (AGENTS.md rule 2). The peer then landed the same retraction as iter-2472, so the stash is now redundant; **it was deliberately NOT dropped** — it is a peer's recovery point (rule 8) |
| LOC measured | 540,388 across `crates/` (7 crates); `operant-cli` src 258,619 of which `tui/` is 191,145 |

## Gate state (measured, this revision)

| gate | command | verdict |
|---|---|---|
| fmt | `cargo fmt --all -- --check` | **pass** |
| genome lint | `./scripts/genome lint` | **pass** — 0 errors, 5 warnings (2 duplicate-owner, 3 TOC) |
| genome doctor | `./scripts/genome doctor` | **1 finding** — stale `docs/handoff/STATE.md` (names `764dcd10`, HEAD is 317 non-doc paths ahead) |
| workspace check | `./scripts/check.sh check --workspace` | **pass** — 0 errors |
| **clippy warning gate** | `bash scripts/clippy-warning-gate.sh` | **FAIL — exit 1, 52 NEW warnings, empty allowlist** |

### Z1 — [VERIFIED] **HIGH** The clippy warning gate is RED on the published
### revision, and has been for 1,509 commits

`bash scripts/clippy-warning-gate.sh` at `25b86ae5`:

```
[clippy-gate] seen=52 allowlist=0 new=52 stale=0
[clippy-gate] NEW warnings detected:  (+ 52 rows)      exit 1
```

`.ci/clippy-allowlist.txt` is a **tracked, 0-byte file**, emptied by
`c2a645d3` (`fix(iter-1171)`) on 2026-10-22. Every warning the workspace
produces is therefore "new" by construction, and the gate has failed on
every run since. `git rev-list --count c2a645d3..HEAD` = 1509.

**Why this matters more than a lint count.** The gate is the mechanism
AGENTS.md names as the *enforcement* for the unwrap/expect deny, and three
separate rows in `docs/audit/2026-06-19-BUGS.md` (RT-213, RT-253, RT-370,
RT-481) record real defects that "the strict clippy gate caught". A gate
that has been red for 1,509 commits cannot have caught any of them, and an
agent that runs it, sees red, and moves on is now the *expected* behaviour.
That is how the four rows above got caught by a gate that was not running.

### Z2 — [VERIFIED] **HIGH** The same gate is NONDETERMINISTIC: `seen` is 31,
### 52 or 53 on one unchanged tree

Measured, same commit, same warm `target/`, back to back:

```
run 1 (background, cold target) : seen=31
run 2 (foreground, warm)        : seen=52
run 3                           : seen=31
run 4                           : seen=53
run 5                           : seen=52
```

I isolated the cause. The gate runs **two** cargo passes into the **same**
`target/` and concatenates their JSON:

- pass 1 `--lib --bins` → **28** distinct keys, identical across runs
- pass 2 `--all-targets` → **56** distinct keys, identical across runs

Each pass is deterministic alone; the **union the gate actually measures is
not**. The two passes share one target directory, so the second run's
fingerprints are the first run's product, and cargo replays a different
subset of cached diagnostics depending on what the previous invocation left
warm. Neither the jq filter nor the key derivation is at fault — I re-ran
the gate's exact jq over a captured JSON twice and got byte-identical
output.

**Why this matters.** A gate whose verdict is a coin flip is worse than a
red gate: `RT-255` in this repo's own ledger records exactly this failure
class (the floating-toolchain coin flip) and it was closed by pinning the
*expectations*, not the *measurement*. Here the measurement itself is
unstable, so "the gate passed" is not a reproducible statement, and a
mutation that fixes a warning can appear to pass or fail at random.

**The fix is one line, and it is not "run `--update`".** `--update` would
snapshot whichever of 31/52/53 that run happened to produce and then report
every other run's warnings as new. The gate needs (a) `--no-deps` so
dependency diagnostics cannot enter, (b) a single pass over `--all-targets`
with the two denied lints exempted per the existing `crates/*/tests/` rule
extended to in-file `#[cfg(test)]` modules in bins, or (c) two distinct
`CARGO_TARGET_DIR`s for the two passes. (c) is the smallest change that
preserves the current lib/bin-is-authority contract.

### Z3 — [VERIFIED] **MEDIUM** The gate's test exemption does not cover
### in-file `#[cfg(test)]` modules inside `src/`

The jq filter exempts `clippy::unwrap_used` / `clippy::expect_used` when the
file path contains `/tests/`. That catches `crates/*/tests/*.rs` (separate
crates) but **not** a `#[cfg(test)] mod tests` living at the bottom of a
`src/` file, which is where this repo puts most of its tests (the `tui/`
tree alone has `app/tests.rs`, `settings_screen/tests.rs`, `session_hub/tests.rs`
in-tree).

Concrete instance, verified: `crates/operant-cli/src/gateway_runner.rs:8745`
draws `clippy::await_holding_lock` for a `std::sync::MutexGuard` held across
`.await`. The site is inside `#[cfg(test)]` (nearest attribute at `:8713`,
`#[tokio::test]` at `:8743`), so it is **benign test code** — but the gate
reports it as a production finding, and an agent "fixing" it would edit a
test for no behavioural reason. Same shape for `dead_code` on
`tui/app/tests.rs`, `gateway_runner/tests.rs` and 9 more of the 52.

Counting the 52 by whether the cited file's first `#[cfg(test)]` precedes
the cited line: **14 are in-file test modules, 38 are production.** The 14
are noise that trains the reader to skim; the 38 include the real ones
(Z4, Z5).

### Z4 — [VERIFIED] **MEDIUM** `adapter_types/output_styles.rs` is entirely
### dead production code, and its only importer imports it without using it

```
crates/operant-cli/src/tui/adapter_types/output_styles.rs   18 LOC, 3 items
  pub struct StyleInfo            "never constructed"
  pub fn builtin_styles()         "never used"
  pub fn find_style()             "never used"
```

Its single importer is `crates/operant-cli/src/tui/settings_screen.rs:8`:

```rust
use crate::tui::adapter_types::output_styles::{builtin_styles, find_style};
```

Neither name appears anywhere else in that 225-line file. So the file is
dead *and* the import is dead — the compiler reports both
(`unused_imports` + three `dead_code`), and the module is still declared
`pub mod output_styles;` at `adapter_types/mod.rs:15`, which is why it
survives: `pub` in a `pub mod` in a binary crate emits no dead warning
unless the item is unreachable, and this one is reachable through the
unused import.

**Delete:** the 18-line file, the `pub mod` line, and the import. Zero
behaviour change. This is the cleanest ponytail row the audit produced.

### Z5 — [VERIFIED] **MEDIUM** `crates/operant-config` is 6,725 LOC of which
### ~2,400 is unreachable, including a **3,791-line security module that
### duplicates a live classifier in `operant-core`**

`operant-config` survived the iter-1138-1141 purge ("ONE config system now")
and still carries the pre-purge security surface. Measured reachability
from outside the crate (`rg 'operant_config::<mod>' crates/ --type rust`,
excluding `operant-config/src`):

| module | LOC | external refs | verdict |
|---|---|---|---|
| `schema` | 1,531 | 18 | **live** — `ProxyConfig`, `ProxyScope`, `ObservabilityConfig`, `ModelProviderConfig`, `FallbackProviderConfig`, the proxy fns |
| `providers` | 178 | 3 | **live** — `ProvidersConfig` |
| `policy` | **3,791** (1,966 prod) | **1** | **effectively dead** — see below |
| `secrets` | 958 | 0 | **dead** |
| `provider_aliases` | 150 | 0 | **dead** |
| `traits` | 73 | 0 | **dead** |
| `autonomy` | 16 | 0 | **dead** |

**The `policy` row is the finding.** Its single external reference is
`crates/operant-core/src/tools/file_tools.rs:182`, which calls **one free
function** out of 3,791 lines:

```rust
operant_config::policy::split_unquoted_segments(command)
```

Nothing outside the crate constructs `SecurityPolicy` — the one apparent
`SecurityPolicy` reference in `operant-api/src/lib.rs:34` is a **doc
comment**. `CommandRiskLevel`, `ToolOperation`, `ActionTracker`,
`PerSenderTracker`, `SecurityPolicy::command_risk_level`,
`is_command_allowed`, `forbidden_path_argument`: **zero external
references each.**

And the live path already has its own classifier:
`crates/operant-core/src/approval.rs:508` `ApprovalGuard::check_dangerous`
→ `check_dangerous_patterns` (`:561`) over `ALL_DANGEROUS_PATTERNS`
(`:135`–`:215`), called from `agent/stream.rs:1264`
`crate::approval::check_tool_approval`. The two encode the same idea twice
with independent pattern lists — `policy.rs` classifies `rm`/`mkfs`/
`shutdown`/`sudo`/`curl` structurally (`:951`–`:970`), `approval.rs`
enumerates `rm -rf /*`, `mkfs.`, `shutdown -h`, `sudo !!`, `pkexec`,
`doas`, … (`:135`–`:203`).

**Why this is a correctness risk and not just bloat.** A second
dangerous-command list that nothing consults is a list a future agent will
*find* and *trust* — the exact shape of the RT-209 row in this repo's own
ledger, where a `pub` item in a `pub mod` shipped with zero callers and no
gate could see it. The dead one is the more permissive of the two in
places (it has no `pkexec`/`doas`), so anyone who wires it up "because it
looks authoritative" widens the approval surface.

**Ponytail:** move `split_unquoted_segments` into `operant-core` (its one
consumer), delete `policy.rs`'s other 1,965 production lines, and delete
`secrets.rs` / `provider_aliases.rs` / `traits.rs` / `autonomy.rs` outright.
≈ 2,400 LOC, zero behaviour change. `SecretStore` (`secrets.rs:38`) has no
constructor caller anywhere in the workspace.

### Z6 — [VERIFIED] **LOW** `AGENTS.md` misstates the CLI surface: 53
### variants, not 52, and the list omits `continue`

Derived from the `Commands` enum at `25b86ae5:crates/operant-cli/src/main.rs`:

```
$ git show 25b86ae5:crates/operant-cli/src/main.rs \
    | sed -n '/^enum Commands/,/^}/p' \
    | rg -o '^\s{4}[A-Z][A-Za-z]+' | tr -d ' ' | wc -l
53
```

`AGENTS.md:1082` says "**52** command variants" and its 52-name list omits
exactly one real variant: **`continue`** (`cmd_continue.rs`, 711 LOC, the
`operant continue [SESSION_ID]` surface RT-553 documents at length).
Set-difference both ways: 1 missing, 0 extra.

This is the second-order version of the defect that very paragraph was
written to fix ("It used to claim 53 and list `hardware` and `peripheral`")
— the count drifted back by one and the new variant was never added.

### Z7 — [VERIFIED] **LOW** `AGENTS.md` names release line `0.2.1`; the tree
### is `0.3.0`

`AGENTS.md:379` "Current release line: `0.2.1`". `Cargo.toml:15`
`version = "0.3.0"` (workspace), and the deployed binary agrees:
`operant --version` → `operant 0.3.0 (memory-wire b478d15)`.

### Z8 — [VERIFIED] **LOW** The repo's own audit-ledger checker FAILS on the
### published revision

`python3 scripts/audit-ledger-index.py` → **exit 1**, four distinct
complaints, on a clean tree at `25b86ae5`:

```
FAIL: index row(s) not CLOSED but closed in the checkboxes: RT-554
FAIL: id(s) used by TWO index rows: RT-555
FAIL: id(s) used by TWO detail sections: RT-555
FAIL: index row(s) with NO detail section: RT-553, RT-554
```

RT-555 is genuinely duplicated: `docs/audit/2026-06-19-BUGS.md:406` is
"A peer's test in gateway_commands.rs does not compile on origin/main"
(OPEN) and `:407` is "Tab on a slash command REPLACED the whole draft"
(CLOSED iter-2209) — two agents took the same id, which is RG-021's exact
scenario. RT-553 and RT-554 have index rows with prose but no detail
section. The checker's own footer names the remedy (re-derive the table per
RG-016, renumber forward per RG-021).

### Z9 — [VERIFIED] **LOW** 1.3 GB of test scratch sits untracked and
### unignored in the repo root, and `git clean -fdx` would eat it

```
$ du -csh .w10a-tmp .w10b-tmp .w10d-tmp .tmp-test
1.3G    total
$ git clean -ndff | wc -l
290
```

`.gitignore` covers `.audit-*/`, `.wt-*/`, `.wt*/` — and the ignore-block
comment in it explains that exact reasoning for the worktree case. The
four `operant_autonomous_*` scratch roots (`.w10a-tmp`, `.w10b-tmp`,
`.w10d-tmp`, `.tmp-test`) are **not** covered, so they are one
`git clean -fdx` away from gone, and one `git add -A` away from committed.
AGENTS.md's "Pre-Existing Test Artifacts (do not commit)" section names
only `*.sqlite` files.

Also untracked and unignored: `crates/operant-core/memory:/` — a directory
whose name ends in a colon, containing `operant.unknown.toml`. That is a
path-joining artifact (`format!("{}:", ...)` or a `Memory:` prefix
reaching a `Path::join`), not an intentional store; `git check-ignore`
confirms it is not ignored either.

### Z10 — [VERIFIED] **LOW** `AGENTS.md` names **30** overlays;
### `App::overlay_flags()` has **29**

`AGENTS.md:1162`: "`App::overlay_flags()` names **30** of them, and a full
audit (iter-1002) traced every one to a non-test call site". The function
is `crates/operant-cli/src/tui/app/commands.rs:93` and its own signature
says 29:

```rust
pub(crate) fn overlay_flags(&self) -> [(&'static str, bool); 29] {
```

Enumerating the tuples between `:93` and the closing brace yields exactly
29 names (`help_overlay`, `history_search_overlay`, `global_search`,
`rewind_selector`, `settings_screen`, `theme_screen`, `stats_dialog`,
`mcp_view`, `agents_menu`, `diff_viewer`, `memory_file_selector`,
`skills_view`, `plugins_hub`, `journey_view`, `inline_interactive`,
`session_hub`, `export_dialog`, `context_viz`, `bypass_permissions_dialog`,
`permissions_picker`, `key_input_dialog`, `custom_provider_dialog`,
`free_mode_dialog`, `connect_dialog`, `import_config_picker`,
`import_config_dialog`, `command_palette`, `ask_user_dialog`,
`permission_request`). The array length is checked by the compiler, so 29
is the ground truth and the doc is off by one.

Same failure family as Z6 and Z7: a hand-maintained count in `AGENTS.md`
drifting from the code it describes. This repo has an `RG-010` regression
record for exactly this ("re-derive the list, do not trust it"), and the
doc still carries three drifted counts.

### Z11 — [VERIFIED] **LOW** The generic webhook HMAC path has no replay
### window, while the Slack and Stripe paths both do

`crates/operant-core/src/gateway/webhook.rs::verify_request_signature`
has three branches:

| branch | timestamp source | staleness check |
|---|---|---|
| Slack (`:97`) | `x-slack-request-timestamp` | **yes** — `:100` `if (now_secs - ts_secs).abs() > 300` |
| Stripe (`:143`) | `parse_stripe_signature` → `t=` | **yes** — `:149` `if (now_secs - ts).abs() > 300` |
| **generic** (`:185`) | *none read* | **no** |

The generic branch reads only `x-webhook-signature` / `x-hub-signature-256`
and compares `HMAC(secret, body)` (`:200`–`:216`). A captured signed body
is therefore replayable forever: the signature stays valid because nothing
in the verification binds it to a time. Both named providers get a
5-minute window; the fallback an operator would configure for their own
service does not.

**Severity LOW, not HIGH, and the reason is the deployment shape:** the
listener defaults to loopback (`DEFAULT_LISTEN_ADDR = "127.0.0.1:8080"`,
`:256`) and its doc comment (`:252`–`:255`) states that binding beyond the
machine is an explicit `[gateway] webhooks_addr` decision taken together
with `webhooks_secret`. So a default install is not remotely reachable.
The finding bites the operator who follows the documented path for a
GitHub-style integration and binds a public address.

**The fix follows the code's own precedent:** accept an optional timestamp
header (`x-webhook-timestamp`, or `x-request-timestamp`) and apply the same
`(now_secs - ts).abs() > 300` rejection the Slack and Stripe branches
already use, refusing the request when a configured `require_timestamp` is
set and the header is absent. Note this is *not* a constant-time-comparison
defect — every comparison on every branch is
`constant_time_eq::constant_time_eq` (`:126`, `:175`, `:214`, `:413`), which
I verified and record as a non-finding below.

## Lane findings I verified myself

Each lane wrote its own doc; this section records only the lane claims I
re-derived from the source before letting them into the synthesis. A lane
claim I could not confirm is not here.

### Z12 — [VERIFIED] **LOW** `ClassifiedError::should_fallback` is written at
### 76 sites and read by none (lane A finding A1, confirmed)

`rg -n 'should_fallback' crates/operant-core/src --type rust | wc -l` = **76**.
Outside `error_classifier.rs` (which sets it) and `agent/fallback.rs` (which
comments on why it ignores it), there are **zero** reads. The loop branches
on `Failover::classify`'s own `advance_model_chain` / `switch_provider`
(`agent/fallback.rs:178`–`186`), and `:86`–`:89` says so in a comment:

> "Note this is deliberately *not* `classified.should_fallback`, which is
> set for cases (a content-policy rejection at 400) the model chain has
> never advanced on."

Its sibling `should_compress` **is** live (8 real consumers in
`agent/run.rs:1895`, `:1934`, `:2101`, `:2266`, `:2306`, `:2358`), so this is
one dead field, not a dead pair.

**Why it matters:** `AGENTS.md:1270` advertises "24-class
`ClassifiedError` with `should_compress`/`should_fallback`". Half that
sentence names a decoy — a field a maintainer would reasonably set on a new
variant expecting the model chain to advance. It does not.

### Z13 — [VERIFIED] **LOW** `FailoverReason::AuthPermanent` and
### `UpstreamRateLimit` cannot be constructed (lane A finding A2, confirmed)

Exhaustive `rg` for each name across `crates/`:

| variant | sites | construction? |
|---|---|---|
| `AuthPermanent` | enum `:21`, `as_str` `:98`, `is_auth()` `:150`, test comment `:1575` | **none** |
| `UpstreamRateLimit` | enum `:29`, `as_str` `:101`, test comment `:1575`, `events.rs:196` (a display match arm) | **none** |

So the honest count is **26 declared / 24 producible**. The repo already
documents this at `error_classifier.rs:1575` ("unproducible today and
deliberately excluded"), which is why it is LOW and not a defect — but
`events.rs:196` groups `RateLimit | UpstreamRateLimit` for display, so a
reader greps for the second arm and finds a variant nothing can produce.

### Z14 — [VERIFIED] **MED** The `ThinkingSignature` arm shadows
### `ReasoningParamRejected` on a body containing both vocabularies (lane A
### finding A3, upgraded from ASSERTED after re-derivation)

Lane A marked this ASSERTED for lack of a captured provider payload. The
*code* claim is fully verifiable, so I upgraded it: the ordering is a
property of the source, not of any one provider.

`classify_api_error` (`:384`) runs its inline step-1 arms in source order
and returns from the first match. `ThinkingSignature` is `:406`–`:413`, and
its test is `status == 400 && body contains "thinking" && ("signature" |
"cannot be modified" | "must remain as they were")`. `ReasoningParamRejected`
is `:429`–`:437`, tested **after** it, and its test is
`status 400|422 && body contains ("reasoning" | "reasoning_effort" |
"effort" | "thinking") && ("unrecognized" | "unknown" | "unsupported" |
"invalid parameter" | "unexpected" | "not supported")`.

The two tests intersect: a 400 body of **`thinking signature unrecognized`**
satisfies both. `ThinkingSignature` wins by position. The two arms then do
**different things** in `agent/run.rs`:

- `ReasoningParamRejected` (`:1869`–`:1898`) marks the model effort-unsupported,
emits a notice, and retries the same request with `with_reasoning_effort(None)`
at no retry-budget cost.
- `ThinkingSignature` falls through to the generic retry, which re-sends the
signature it just failed on.

There **is** a test named for this collision,
`test_thinking_signature_wins_over_param_rejection` (`:1459`), but it drives
`"thinking blocks cannot be modified"` — a body with **no** reasoning-rejection
word, so it does not exercise the collision at all. It asserts the ordering
on a case where only one arm can match. The guard is vacuous with respect to
the property its name claims.

**Concrete failing case:** provider returns 400 `"thinking signature
unrecognized"` → classified `ThinkingSignature` → the loop retries WITH the
bad signature instead of stripping the effort field → the retry fails the
same way → the turn dies where one free retry would have saved it.

**Fix:** make the two arms disjoint — require the `ThinkingSignature` body
to NOT contain a rejection word, or move the reasoning arm above it — and
then fix the vacuous test to drive a body that satisfies both.

## Non-findings, recorded so they are not re-litigated

Each of these is a doctrine claim I checked and **confirmed**. They are
here because the fleet's lane docs may report them as gaps, and a
re-derivation wastes the next pass.

| claim (AGENTS.md / docs) | verdict | proof |
|---|---|---|
| Eviction never orphans a `tool_use` from its `tool_result` | **holds** | `context_management.rs:557` `tool_group_starts` builds groups; `:583` `safe_compaction_cutoff` cuts only on a group boundary; doc block `:566`–`:582` states the four guarantees and names the `0` fallback when the first group alone blows the budget |
| Tool fan-out preserves partial results across an interrupt | **holds** | `agent/stream.rs:1963` `buffer_unordered(8)` inside `tokio::select!` on the interrupt flag; `:1985` every call with no result becomes a truthful `"Cancelled by the user…"` result so the API pairing rule still holds |
| Decay curve clamps `p` and floors the target | **holds** | `context_management.rs:1797` `let p = p.max(0.10)`, `:1799` `h_tokens.max(20.0)`; divisor is a hard-coded const (`REACTIVE_DECAY_CONSTANT = 30.0`, `PREFLIGHT_DECAY_CONSTANT = 20.0`), never operator-set, so no div-by-zero path exists |
| Memory is memory-wire-only; unknown names warn | **holds** | `memory_provider.rs:392` `build_memory_provider` warns for any name but `"memory-wire"`/`""`, and `NoopProvider` (`:432`) is the degrade path — "BuiltinProvider is deleted" is accurate |
| `operant_config::schema::Config` (the purged aggregate) does not come back | **holds** | `crates/operant-config/tests/schema_is_not_loaded.rs:100` greps for `load_or_init` / `schema::Config` and fails |
| Slack / WhatsApp / Email / SMS are outbound-only | **holds** | `gateway/slack.rs:142` `start()` logs "event-based, no polling" and returns; `whatsapp.rs:86`, `email.rs:56`, `sms.rs:74` are `info!(...); Ok(())` with no listener; `start_with_channel` forwards to that no-op (`slack.rs:403`) |
| Webhook is inbound-only with HMAC | **holds** | `gateway/webhook.rs:847` real `TcpListener::bind` + `axum::serve`; `:311` `send_message` is a documented no-op |
| Webhook HMAC comparisons are constant-time | **holds** | `constant_time_eq::constant_time_eq` at `:126`, `:175`, `:214`, `:413` |
| `[workspace.lints]` manifest inheritance is unusable here | **holds** | `Cargo.toml:97` **does** use `[workspace.lints.rust]` and 6 of 7 crates carry `[lints] workspace = true`; the comment at `:100`–`:105` is scoped to the *clippy denies* only and does not contradict the rust table. Note `operant-os` is the one crate with **no** `[lints]` section — an inconsistency, not a defect |
| `unsafe` is rare and justified | **holds** | 177 raw hits in `operant-core` are **5 production blocks** once `#[cfg(test)]` is cut: `env_passthrough.rs:142` (`set_var` behind a rotation-only guard, SAFETY comment at `:141`), `interrupt.rs:129`–`130` (`unsafe impl Send/Sync` with a SAFETY comment at `:127`), `tools/async_delegation.rs:201`/`212` (`libc::flock` with the blocking rationale at `:199`). Across all crates: core 5, cli 42 (mostly `perf.rs` platform FFI with `#[cfg(target_os)]` guards), config 3, os 4, plugins/api/harness 0 |
| Webhook HMAC comparisons are constant-time on every branch | **holds** | `constant_time_eq::constant_time_eq` at `:126` (Slack), `:175` (Stripe rotation set), `:214` (generic), `:413` (subscription dispatch) |
| The skill root resolver is singular | **holds** | `crates/operant-cli/src/skill_roots.rs:39` `skill_roots()` is the only non-test `SkillRoots::resolve` caller in the CLI; the ~10 raw `config.skills.root_dir` reads (`main.rs:1358/3067/3256`, `cmd_acp.rs:344`, `cmd_cron.rs:639`, …) all feed `build_agent_core_with_memory`, which re-resolves through `crate::skill_roots::skill_manager(config)` (`main.rs:1905`) and uses the raw path only for the Holon override and the empty-manager branch |
| The error taxonomy is 24 classes | **holds** | `agent/error_classifier.rs:16` `enum FailoverReason` has exactly 24 variants |

## Method note

Every number above came from a command I ran at `25b86ae5`; the commands
are quoted inline so they are reproducible. Where I could not confirm a
claim I did not write it down as a finding. Two candidate findings were
**withdrawn** during this pass and are recorded here rather than silently
dropped:

1. *"44 `live: false` settings rows are phantom keys."* **Withdrawn.** I
   checked all 386 `SETTING_ROWS` keys for a backing field and the 12 that
   have no `pub <leaf>:` in `operant-core/src/config.rs` all resolve either
   in `operant-config/src/schema/` (`embedding_routes`, `fallback_chain`,
   `model_routes`) or in a sibling core module (`advisor.*` →
   `advisor.rs:216`–`242`). The `live: false` flag is doing its job.
2. *"`anyhow` in library crates violates the typed-error doctrine."*
   **Partly withdrawn.** 109 production `anyhow::` uses exist in
   `operant-core` (worst: `binary_updater.rs` 30, `skill_marketplace.rs`
   22, `tools/terminal_backend.rs` 18), which does contradict AGENTS.md's
   "`anyhow` is for binaries only" — but `operant-core/src/error.rs:132`
   already defines a typed `Error` enum, so this is a *migration* gap
   rather than an architecture defect. Handed to lane J, which owns the
   rule-by-rule count, rather than double-counted here.
### Z15 — [VERIFIED] **HIGH** `blast_radius` models 7 verbs; `mv`/`cp`/`rsync`/
### `install`/`scp` are unmodelled, and an unmodelled verb downgrades the whole
### screen to `None` (lane B finding B1, confirmed)

`blast_radius.rs:199`–`:221` `verb_kind()` maps exactly these and returns
`None` for everything else:

```
rm unlink shred srm rmdir truncate -> Remove      dd -> RawWrite
tee -> Write      chmod chown chgrp -> RecursiveMeta
find -> Finder    git -> Vcs
mkfs* fdisk sfdisk cfdisk parted wipefs badblocks hdparm mkswap -> Format
```

`rate_segment` (`:546`–`:547`) turns `None` into `BlastRadius::None`, the
lowest tier, and `screen()` (`:1199`–`:1203`) returns `None` **immediately**
when `classify() == None` — so `destructive_targets()`, which holds the
denied-path list, is never called at all.

Verified against all three approval layers: `mv /etc /tmp/x`,
`mv ~/.ssh /tmp/x`, `mv $HOME /tmp/x`, `cp /etc/shadow /tmp/x`,
`cp ~/.ssh/id_rsa /tmp/x`, `rsync -a /etc/ /tmp/x` and
`install -m 644 /etc/shadow /tmp/x` all miss L1 (hardline), miss L2
(patterns) and score `None` at L3 — so the answer is **allowed**. The same
targets through a *modelled* verb are denied: `rm -rf /etc` draws both an L1
hit and `DeniedPath{system directory}`, `rm -rf ~` draws `DeniedPath{home
root}`, `dd of=/dev/sda` draws `DeniedPath{system directory}`.

The deny list (`DENIED_SYSTEM_DIRS`, home root, `.git`, filesystem root,
`:1070`–`:1108`) is the strongest control in this seam and it is reachable
only through a 7-verb table.

**The smaller fix, which closes the whole class:** make `rate_segment`'s
`None` arm consult the deny check on every positional token regardless of
verb, so *any* segment naming a denied path is refused whatever the verb.
Adding the five verbs individually fixes the cases I enumerated and leaves
the next unmodelled verb open.

### Z16 — [VERIFIED] **HIGH** `approval_mode` is not merely unreachable, it is
### a hard config-load error (lane B finding B2, severity raised by me)

`AgentConfig.approval_mode` (`agent/mod.rs:171`) documents three modes at
`:169`–`:171` — `"smart"` (default, pattern-based), `"manual"` (prompt for
every tool), `"off"` (no checks). It is read at exactly one place
(`stream.rs:1263`–`1267`) and set at exactly one production place
(`agent/mod.rs:299`, the literal `"smart"`).

The reason an operator cannot reach it is stronger than lane B stated:
`BehaviorSettings` carries `#[serde(default, deny_unknown_fields)]`
(`config.rs:714`–`:716`) and has **no** `approval_mode` field (I enumerated
its fields: model, max_iterations, tool_timeout_secs, request_timeout_secs,
context_budget_tokens, system_prompt, stream, context_window,
max_healing_attempts, show_reasoning, max_trajectories,
show_turn_attribution). And `impl From<&BehaviorSettings> for AgentConfig`
(`agent/mod.rs:277`) does not forward the field either.

So `[agent] approval_mode = "manual"` in `operant.toml` is not silently
ignored — **`deny_unknown_fields` makes it a parse error that fails the
config load.** The strictest posture the system documents cannot be
requested; attempting to request it breaks startup. `operant.example.toml`
does not mention the key, so nothing warns the operator it exists.

The kernel bridge is worse: `approval_mode: None` is hardcoded at
`tools/kernel/runtime.rs:125` with the comment "Smart default; explicit modes
arrive via config later" — that later never arrived.

**Fix:** add `approval_mode` to `BehaviorSettings` (serde default `"smart"`),
forward it in the `From` impl, thread it through `agent_config()`, and pass
it into `BridgeExecutor::install_executor` instead of `None`.

### Z17 — [VERIFIED] **MED** `ToolRegistry::execute()` does not enforce the
### disabled set, so the default AFT dedup de-advertises `terminal` without
### banning it (lane B finding B3, confirmed)

`tools.rs:1089`–`:1103` `execute()` resolves the alias and dispatches:

```rust
let resolved = self.resolve_alias(tool_name);
let tool = { self.tools.read().await.get(&resolved).cloned() };
```

It never reads `disabled_names`, `disabled_toolsets`, or `is_available`.
`rg 'is_available' crates/operant-core/src/agent/stream.rs` → **0 hits**.
The filtering happens only in the *advertising* paths
(`get_schemas` `:829`–`:856`, `get_available_schemas_filtered` `:890`–`:925`).

**This is reachable in the default configuration.** `aft_enabled` defaults to
`true` (`config.rs:2405`), and when the AFT bridge pings live,
`main.rs:1471`–`:1478` adds `file_read`, `file_write`, `patch`, `terminal`,
`file_search`, `file_list` to the disabled set. So `terminal` is
disabled-but-callable: the model never sees its schema, but
`registry.execute("terminal", …)` runs it. The AFT dedup is a
de-advertisement, not a ban.

**Fix:** check `is_available(&resolved)` at the top of `execute()` and
return a `ToolResult::error` naming the disable. One `RwLock` read per call,
and the property becomes structural instead of conventional. (`name_recovery`
is already safe: it resolves through `get_schemas()`, the filtered view, so
recovery cannot resurrect a disabled name.)

### Z18 — [VERIFIED] **LOW** `compression_locks` is a table with a 3-method
### API and 3 tests, and no production caller (lane D finding D1, confirmed)

`database.rs:106`–`:111` declares `compression_locks` (session_id, holder,
acquired_at, expires_at) plus `idx_compression_locks_expires` (`:125`).
`acquire_compression_lock` (`:3755`), `is_compression_locked` (`:3773`) and
`release_compression_lock` (`:3789`) exist, with tests at `:4877` and `:5242`.

`rg 'acquire_compression_lock|is_compression_locked|release_compression_lock'
crates/ --type rust` outside `database.rs` → **zero hits.** No production path
takes the lock, so concurrent compression of one session is ungated while the
schema advertises a lock and three tests assert that a lock nothing takes
works.

### Z19 — [VERIFIED] **LOW** `compartments` + the 720-line `historian.rs` have
### no production reader or writer (lane D finding D2, confirmed)

`database.rs:209`–`:226` (migration v5) creates `compartments` with two
indexes; `:1007`/`:1034`/`:1074` expose `insert_compartment` /
`compartments_for_session` / `last_compartment_span_end`. The only callers of
any of the three are `database.rs` tests (`:7074`–`:7190`). The writer was
deleted in iter-2404 and the deletion note says so at
`agent/compress.rs:497` ("`compartments_for_session` had ZERO callers").
`historian.rs` (720 lines of span-planning logic) has no caller either — the
only two hits for its API are its own module doc (`:21`) and that comment.

### Z20 — [VERIFIED] **LOW** `messages.codex_reasoning_items` /
### `codex_message_items` are written on every insert and never populated
### (lane D finding D3, confirmed)

`database.rs:93`–`:94` declares both; `save_message_full` (`:1425`–`:1445`)
writes them and `get_session_messages_full` (`:1691`–`:1715`) reads them
back. Every construction site outside `database.rs` passes `None`:
`agent/run.rs:3540`, `session_recap.rs:296`,
`tools/evidence_recall.rs:212`/`:314`, plus the integration tests. So the
columns are always NULL and ride every message INSERT and every full-row
SELECT. `platform_message_id` (`:95`) is in the same class outside
`database.rs`.

### Z21 — [VERIFIED] **MED** Nothing in `database.rs` has time-based
### retention (lane D finding D5, confirmed)

`rg -c 'retention|prune|vacuum|purge|cleanup'
crates/operant-core/src/database.rs` → **no matches**. All 10 `DELETE FROM`
sites are either session-scoped deletion by explicit id (`:2625`, `:3681`),
single-row deletes by key (`:2707`, `:2833`, `:2873`, `:2884`, `:2908`), or
key deletes (`:3736`, `:3792`).

So `messages`, `context_ledger` (one row per model request, `:138`–`:156`,
16 numeric columns), `session_events`, `verification_events` and
`compartments` grow without bound for the life of the file. `context_ledger`
is the sharpest: one row per request with 16 numeric columns. The one table
with an operator-facing retention verb is `checkpoints`
(`cmd_checkpoints.rs:3`–`:4`, `prune|clear`) — which is why the absence is
easy to miss.

### Z22 — [VERIFIED] **HIGH** `/model roles` opens a panel that paints nothing
### while owning the keyboard (lane H finding H1, confirmed)

`crates/operant-cli/src/tui/models_panel/` is a single 391-line `mod.rs` with
state, a key handler and an opener — and **no renderer**:
`rg 'models_panel' crates/operant-cli/src/tui/render/ crates/operant-cli/src/tui/operant_ui/`
exits 1. The only non-test references in the whole crate are `app/init.rs:246`,
`app/mod.rs:696`, `app/commands.rs:773`, `app/key_handling.rs:1247`/`:3401+`, and
`tui/mod.rs:60`.

Traced failing case: `/model roles` → `app/commands.rs:772`–`:775`
`self.models_panel.open(ModelScope::Roles)` → `models_panel.visible = true` →
the frame paints the unchanged chat, while `key_handling.rs:1247`–`:1250`
routes EVERY key into `handle_models_panel_key` and returns `false`:

```rust
if self.models_panel.visible {
    self.handle_models_panel_key(key);
    return false;
}
```

So arrow keys move an invisible cursor, `Enter` writes a status notice, and
`x` on an assigned slot calls `persist_config_key(key_path, "")` — **writing
the config from a panel the operator cannot see**. Escape is the only way
back.

Two aggravating facts I confirmed:
- `models_panel.visible` is **not** in `overlay_flags()` (the 29-name array at
  `app/commands.rs:93`–`:133`), so `any_modal_open()` / `any_overlay_open()`
  report false while the panel owns the keyboard. The input gate is blind to
  it. (Z10 already records that the array holds 29 names; `models_panel` is
  not among them.)
- The render half was announced and never landed:
  `models_panel/mod.rs:73` and `:86` both carry
  `#[cfg_attr(not(test), allow(dead_code, reason = "the render half is the next unit"))]`.

The sibling `subagent_navigator` / `agent_picker` DO have a paint path — they
are painted from inside `paint_command_palette`
(`render/operant_overlays.rs:538`–`:547`) — so the pattern exists and this
panel simply never got it.

**Fix:** either add the overlay row plus a `paint_models_panel`, or delete the
panel and repoint `/model roles` at the inline picker's Roles scope. Do **not**
merely add the visible flag to `overlay_flags()` — that hides the defect
deeper by making the input gate aware of a panel that still paints nothing.

### Z23 — [VERIFIED] **HIGH** Two mouse hit-test maps are never written, so
### right-click and click-to-collapse-thinking are dead (lane H finding H5,
### confirmed)

`App.thinking_row_map: RefCell<HashMap<u16,u64>>` (`app/mod.rs:995`) and
`App.message_row_map: RefCell<HashMap<u16,usize>>` (`app/mod.rs:997`) are
initialised empty (`app/init.rs:415`–`:416`) and **nothing ever inserts into
them in production**:

```
$ rg -n 'thinking_row_map|message_row_map' crates/operant-cli/src --type rust
  app/init.rs:415,416        (init empty)
  app/mouse.rs:119           (READ  message_row_map)
  app/mouse.rs:1331          (READ  thinking_row_map)
  app/mod.rs:995,997         (decl)
  app/tests.rs:4650          (the ONLY insert — a test)
  operant_ui/ui_viewport.rs:1686  (a comment citing it as precedent)
```

Their only readers take the `else` branch every time:
- `mouse.rs:119` `message_index_at_row` feeds `:533` (`open_context_menu_at_cursor`,
  Ctrl+Shift+M) and `:1237` (right-click → `ContextMenuKind::Message`). Right-click
  over the transcript therefore ALWAYS calls `dismiss_context_menu()`
  (`mouse.rs:1244`), and Ctrl+Shift+M falls back to a selection-only menu.
- `mouse.rs:1331` is the click-to-collapse-thinking handler, unreachable because
  the map is always empty. The only writer of `thinking_expanded` is that same
  dead branch (`mouse.rs:1335`), so `thinking_expanded` can never be non-empty
  either.

The live paths use different mechanisms with a frame-time writer: the
pinned-todo band publishes a rect through `pinned_todo_more_area()`
(`ui_viewport.rs:1688`, wired at `mouse.rs:1343`) and tool rows publish
through `TOOL_ROW_REGIONS` (`ui_viewport.rs:1697`). The comment at
`ui_viewport.rs:1686` still cites `thinking_row_map` as the working
precedent, which is the misleading part — a reader checking "how does a row
publish its rect" is pointed at the one map nothing writes.

**Note:** the maps are `RefCell<HashMap>` with no borrow held across a call, so
this is dead state, not a borrow-panic risk.

### Z24 — [VERIFIED] **HIGH** The content-integration (ponytail) install path
### verifies NOTHING, and unlike the binary path it does not even WARN
### (lane F finding F3, confirmed; path corrected)

`crates/operant-core/src/binary_updater.rs:294`–`:306` is the entire content
path:

```rust
if spec.content_tarball {
    let url = format!(
        "https://codeload.github.com/{}/tar.gz/{}",
        spec.repo, release.tag
    );
    let bytes = download(&url).await?;
    install(spec, &release.tag, &bytes)?;      // no verify_sha256
    return Ok(Some(UpdateNotice::Updated { .. }));
}
```

`verify_sha256` is reachable from exactly two places (`:316`, and `:574`
whose only caller is `aft_bridge.rs:442` — a first-install path). **Neither
is on the content path.** `rg -n 'verify_sha256' crates/` confirms.

What makes this a defect rather than a documented trust choice is the
contrast 40 lines below it. The binary path (`:314`–`:331`) explicitly
branches on `release_has_sums` and, when no manifest exists, emits a
`tracing::warn!` whose comment reads:

> "NO manifest, so the bytes are NOT verified. Installing anyway is
> deliberate ... But it must not be SILENT — 'a verification that silently
> never runs reads as protection' is exactly the defect class this row is
> about."

That reasoning is correct and it applies identically to the content tarball,
yet the content path has **no such branch and no warning**. The module doc at
`:11`–`:12` still claims "downloads, SHA256-verifies when the release carries
a `SHA256SUMS`, and installs".

Concrete failing case: an attacker who can (a) compromise the GitHub release
for `DietrichGebert/ponytail`, (b) MITM the codeload TLS handshake (corporate
proxy with a trusted CA; `download()` sends `bearer_auth` only when `gh auth
token` succeeds), or (c) compromise the `gh` host, ships
`.../skills/ponytail/SKILL.md` containing arbitrary text. It extracts,
guard-scans (`scan_content_against_guard` `:746`–`:792`), and if the verdict
is `Caution` it installs with a `tracing::warn!` — and becomes the **frozen
prompt prefix** on every subsequent model request
(`ponytail.rs:155`–`:181` → `frozen_prefix_block`). The operator sees no
prompt; the warn goes to the log.

Why HIGH and not BLOCKER: it needs control of the upstream release or a
CA-trusting MITM position, the fetch is TLS, and ponytail's posture defaults
OFF (`enabled=false`). It is HIGH because this is a **self-updating content
integration whose entire purpose is to install silently in the background**,
and its install path is the one with zero verification and zero
operator-visible signal.

**Fix shape:** require `release_has_sums(&release.assets)` for content
tarballs too, or explicitly document the trust model and emit
`UpdateNotice::Failed` on a verification error. (Note: ponytail releases have
no assets by design — `ponytail.rs:33` — so `release_has_sums` would be
`false` even if it were called; the fix is to *publish* a sums file or to
state the trust model rather than to wire a check that would always no-op.)

### Z25 — [VERIFIED] **MEDIUM** An empty `SKILL.md` passes every content-install
### gate, yielding a silently empty ruleset (lane F finding F1, confirmed
### including its open question)

I verified the extraction gates and then closed the lane's open question. A
tarball member `<prefix>/skills/foo/SKILL.md` of size 0 passes:
`extract_content_tarball` (`:737`) requires only `files != 0`, and
`scan_content_against_guard` (`:746`–`:792`) counts **skill directories two
levels down** (`:756`–`:766`, `skill_path.is_dir()`) and requires
`scanned != 0`. Neither inspects a skill's contents.

I then read `skills_guard::scan_skill` (`:1342`) and `check_structure`
(`:1586`) to settle whether the guard catches it. **It does not.** The only
requirement on `SKILL.md` anywhere in that module is `scan_file_inner`'s
extension filter (`:1251`–`:1252`), which *includes* `SKILL.md` so it gets
pattern-scanned — a 0-byte file yields no findings, hence `ScanVerdict::Safe`.
Nothing requires `SKILL.md` to exist or to be non-empty.

Result: `rename` at `:670` publishes `ponytail-vX.Y.Z/skills/foo/SKILL.md`
(empty). `ponytail.rs:185`–`:194` `installed_ruleset()` then reads it and
returns `Some("")`, so `ponytail.rs:155`–`:181` `frozen_prefix_block` emits
`"\n\n<ponytail>\n\n\n</ponytail>"` — a block that **looks** installed and
enabled in the frozen prefix while carrying no rules. The operator's session
silently loses its ponytail ruleset with zero error surface.

**Fix shape:** require each scanned skill dir to contain a `SKILL.md` with
`metadata().len() > 0` before the rename at `:670`, and fail the install
otherwise.

### Z26 — [VERIFIED] Lane J independently reproduced the clippy-gate findings
### (Z1/Z2/Z3), which is the strongest cross-check in this audit

Lane J (rust-discipline) ran the gate and the allowlist itself and reported,
independently of my coordinator run:

- `bash scripts/clippy-warning-gate.sh` EXITS 1: `seen=52 allowlist=0 new=52 stale=0`
- `.ci/clippy-allowlist.txt` is a tracked **0-byte** file, emptied by `c2a645d3`
  (iter-1171, 2026-10-22) — the same commit I identified
- Gate is NONDETERMINISTIC: 5 runs on one warm tree → `seen = 31, 52, 31, 53, 52`
- Of 52 rows: 14 in-file `#[cfg(test)]`, 38 production

Every number matches my Z1/Z2/Z3 exactly, from a second session that was not
given my results. I am treating Z1–Z3 as settled rather than as a coordinator
hypothesis. The one thing I can add beyond J: the `31` figure appeared first,
so a fresh reader is likely to see 31 or 52 and draw different conclusions
from the same tree — which is why Z2 (separate `CARGO_TARGET_DIR` per pass)
must ship with Z1 (fill the allowlist), not after it.

### Z27 — [VERIFIED] **LOW/MEDIUM** Two `unsafe impl Send/Sync` are redundant
### and a `[lints]` section is missing from one crate (lane J findings 8, 12,
### confirmed)

`crates/operant-core/src/interrupt.rs:129`–`:130`:

```rust
// Safety: AtomicBool is Send + Sync, and Arc is Send + Sync.
// The struct contains no other state.
unsafe impl Send for InterruptFlag {}
unsafe impl Sync for InterruptFlag {}
```

`InterruptFlag` is `struct InterruptFlag { triggered: Arc<AtomicBool> }`
(`:44`–`:46`). `Arc<AtomicBool>` is auto-`Send` + auto-`Sync`, so **both
`unsafe impl` are redundant** — they opt into something the compiler already
guarantees. The safety comment is correct and the intent is harmless, but the
code asserts an invariant the type system already enforces, which is the
pattern that makes a later `Arc<Cell<T>>` field compile past review. Recommend
deleting both lines (no behavior change).

Also confirmed: `crates/operant-os/Cargo.toml` declares **no `[lints]`
section** while the other six crates inherit
`[workspace.lints.clippy]` (`clone_on_copy`, `needless_borrow`). So the
workspace's declared lints are not applied uniformly across crates.

### Z28 — [VERIFIED] Lint-suppression debt is broader than the gate's allowlist
### (lane J finding 5, confirmed with corrected numbers)

Lane J reported 53 non-test `#[allow]` with 34 clippy-class and 1 carrying
`reason=`. My own sweep counts **77 total `#[allow]` with no `reason=`**, of
which 21 are in test targets or `dead_code`/`unused` noise. The production
half is the interesting one; the densest sites are:

- `crates/operant-cli/src/tui/prompt_input/vim.rs` — 8 bare, split between
  `non_snake_case` (4, at `:229/:248/:267/:308` — vim key tables) and
  `clippy::too_many_arguments` (4, at `:591/:614/:1184/:1360`)
- `crates/operant-core/src/client.rs` — 4 bare; three of the four carry an
  inline trailing comment (`:489`, `:610`, `:972` all say "mirrors the inner
  request builder's parameterization"), one (`:2044` `clippy::collapsible_str_replace`)
  does not
- `crates/operant-harness/src/{pool,row}.rs`, `operant-cli/src/tui/operant_ui/*.rs`,
  `operant-cli/src/tui/operant_app/*.rs`, `operant-core/src/{agent/run.rs,mcp_oauth.rs,approval.rs}` —
  1–3 each

The `non_snake_case` block in `vim.rs` is legitimately irreducible (vim
keybindings are conventionally lowercase). The `too_many_arguments` blocks
are a real smell but a low-stakes one. This is lint hygiene, not a functional
defect — recorded so the synthesis does not present "77 bare allows" as if it
were the same class of problem as the red gate.

### Z29 — [VERIFIED] **HIGH** A cron job can fire twice after a crash, because
### the job runs before `mark_job_run` advances `next_run_at`
### (lane E finding E7, confirmed)

`crates/operant-core/src/cronjobs/scheduler.rs:358`–`:410` (`run_job`):
the agent/script runs FIRST, and `mark_job_run` — the only write that
advances `next_run_at` — runs only after:

```rust
let (success, _output, final_response, error_msg, transient) = if job.no_agent {
    self.run_script_job(job).await      // the side effects happen HERE
} else {
    self.run_agent_job(job).await
};
...
let next_run = match retry_backoff { ... None => self.compute_next_run(job) };
self.db.mark_job_run(&job.id, success, error_msg.clone(), None, next_run)?;
```

And `tick()` selects due rows with no in-progress marker (`db.rs:1023`
–`:1031`: `WHERE enabled = 1 AND next_run_at <= ?1`).

Failing case, minute by minute: an hourly job fires at 14:00; the agent turn
runs 30 minutes; the process is SIGKILLed at 14:29 — before `mark_job_run`.
`next_run_at` still reads `14:00`. `systemctl restart operant-gateway` →
`start()` (`scheduler.rs:266`–`:276`) → `tick()` → the row is due
immediately and fires again. No idempotency key, no run-history check before
dispatch.

The transient-retry path (`:398`–`:404`) shares the window: it computes
`next_run` from `now + backoff` but still writes it only AFTER the attempt.

**Fix shape:** a claim row (`in_progress` / a run-history guard) written
*before* dispatch, not a `next_run` write after it.

### Z30 — [VERIFIED] **HIGH on a multi-agent host / MEDIUM otherwise** The
### session socket has no peer authentication: any same-UID process is the
### operator (lane E finding E5, confirmed)

`crates/operant-core/src/session_server.rs:119` discards the peer address:

```rust
let (stream, _) = match listener.accept().await {
```

and there is **no** `SO_PEERCRED` / `getpeereid` / uid check anywhere in the
tree (lane E ran that grep; I re-ran the same classes and got zero hits in
`session_server.rs`).

The only protection is filesystem: `run_session_socket` tightens the socket
to 0600 at `:108`–`:115` (and only *warns* if that fails), plus the operant
home being 0700. The default path is
`<operant home>/operant-session.sock` (`session_client.rs:123`–`:129`).

Failing case: any process running as the same UID — a compromised language
server, a `cargo` build script, a browser extension's unsandboxed helper,
another agent's tool — can `connect()` the socket and send
`WireMessage::Prompt`, which is an **unsandboxed agent turn with the full
toolset**. It can also send `Approve`/`Deny` for tool-permission asks
belonging to its own sessions (the gate at `gateway_runner.rs:2869`–`:2905`
is real, but it only constrains *which* asks a session answers, not *who may
connect*).

The code's own comment at `:104`–`:107` states the stakes correctly:
"the socket carries full control of the agent and its tools" — and then
grants that control on UID alone. Its named mitigation (0700 home) is bypassed
by any bind to a permissive `--socket` path.

**Severity:** HIGH on the multi-agent fleet this repo documents (23 live
worktrees, peer agents working the same tree), where "same UID" is not a
trust boundary; MEDIUM on a single-user workstation where nothing untrusted
runs as the operator.

### Z31 — [VERIFIED] **MEDIUM** `operant cron run <id>` bypasses the daemon
### lock, so a manual run can race a scheduled run of the same job
### (lane E finding E9, confirmed)

`acquire_gateway_daemon_lock` has exactly ONE production caller in the whole
tree (`gateway_runner.rs:5453`, inside `gateway run`):

```
$ rg -n 'acquire_gateway_daemon_lock\(' crates/operant-cli/src/ | rg -v 'tests.rs|fn acquire'
  gateway_runner.rs:5453:    let lock_guard = match acquire_gateway_daemon_lock()? {
```

`operant cron run <id>` (`cmd_cron.rs:593`–`:660`) builds its own `CronDb`,
agent and `CronScheduler` and calls `run_job_now` directly. The `CronDb`
connection lock is a `std::sync::Mutex` (`cronjobs/db.rs:95`, `:123`), which
is **per-process** — it cannot serialize two processes.

Failing case: the gateway daemon's `tick()` is mid-`run_job` on job J, and
the operator runs `operant cron run J`. Two processes, two agent instances,
one `next_run_at` row, both call `mark_job_run` — `repeat_completed` is
incremented twice for one logical cadence tick and two transcript histories
exist.

**Fix shape:** the CLI cron path must consult the same file-level `flock` the
gateway holds. That lock exists precisely for this shape; it is simply not
taken here.

### Z32 — [VERIFIED] `PlatformAdapter::handle_update` is dead for ALL SEVEN
### adapters (lane E, confirmed) — a trait method with no production caller

Lane E verified the doc's per-adapter receive/send claims **6/6 CONFIRMED,
0 refuted**, and found the trait's `handle_update` has **no production caller
anywhere**:

```
$ grep -rn handle_update --include=*.rs crates/
  <the definitions, the doc comments, and cmd_update::handle_update_command>
```

Consequences confirmed:
- `slack.rs:345`–`:386`'s parser is unreachable, so AGENTS.md's "Slack's
  `handle_update` parser is dead code" is **CONFIRMED**
- `whatsapp.rs:86`–`:89`, `email.rs:56`–`:59`, `sms.rs:74`–`:77` all have a
  `start()` that only `info!`-logs — the AGENTS.md "outbound-only" claims hold
- `webhook.rs:301`–`:315` `start()` is *also* log-only; the real axum bind
  happens in `start_with_channel` (`:334`, `:847`), so a caller that goes
  through `Gateway::start()` (`mod.rs:387`–`:400`) without the channel path
  **starts no listener at all**

That last point is the one that matters for the synthesis: the doc's
"webhooks is INBOUND-ONLY" is true *only on the `start_with_channel` path*,
and nothing in the trait contract says so.

### Z33 — [VERIFIED] **MEDIUM** Blocking filesystem + JSON work on the webhook
### request path (lane E finding E1, confirmed)

`webhook.rs:467`–`:468` calls `subscription_for_route(&route)` per request,
which reaches `webhook_subscriptions.rs:159`–`:172` → `load_subscriptions()`
`:180`–`:188` — a synchronous `std::fs::read_to_string` + `serde_json::from_str`
**inside an `async` handler**. Axum polls that future on a tokio worker, so
every inbound POST blocks a worker thread for the duration of a disk read and
a JSON parse. The store is small ("a handful of entries", commented at
`:156`–`:158`) so the parse cost is minor; the *syscall* is the issue, and it
is unconditional — no cache, no `spawn_blocking`.

Why it outranks a generic blocking-in-async note: the inbound path is the one
surface an external party drives at will. A remote party deciding how many
worker threads you keep is a different threat model from a slow internal read.

**Fix shape:** `spawn_blocking` around the load, or cache the parsed
subscriptions with a mtime check.

### Z34 — [VERIFIED] **HIGH** A project root (`<workspace>/skills`) is loaded
### as an `Integration` and is never scanned, so `SkillOrigin::Project` is a
### dead variant (lane G finding F-2, re-derived here; lane G left it PENDING)

`SkillOrigin` declares three variants (`skills.rs:35`–`:46`):
`OperatorPool`, `Integration`, and `Project` (doc: "`<workspace>/skills` (or
an explicit `project_root` path) — the repository's own skills,
version-controlled by someone else").

`SkillRoots::resolve` (`:251`–`:297`) DOES build a project root —
`:282`–`:289` sets `out.project` and `out.push_root(dir)` for
`config.explicit_project_root().or_else(|| workspace.join("skills"))`. So the
path reaches the root list.

But `SkillManager::from_roots` (`:571`–`:581`) collapses that list into two
positions and nothing more:

```rust
let mut iter = roots.roots.iter();
let primary = iter.next().cloned()...;      // → OperatorPool
let mut manager = Self::new(primary);
for root in iter {
    manager.add_extra_root(Some(root.clone()));   // → ALL become extra_roots
}
```

and `load_all` (`:620`–`:632`) passes exactly two origins:

```rust
self.load_root(&primary, SkillOrigin::OperatorPool, 0, &mut loaded)?;
for (index, root) in extra_roots.iter().enumerate() {
    if root.is_dir() {
        self.load_root(root, SkillOrigin::Integration, index + 1, &mut loaded)?;
    }
}
```

`load_root` is called from exactly those two sites
(`rg 'load_root\(' | rg -v tests` → `:620`, `:629`). **`SkillOrigin::Project`
is never passed to `load_root` anywhere in the tree.** Its only production
mention is a comparison at `:959`, in a `create`/`delete` refusal that can
therefore never be true for a loaded skill.

Two consequences, both verified:

1. **A project skill is labelled `Integration`.** Every downstream surface
   that treats the two differently — the `[integration]` listing marker, the
   operator-pool-wins-collision rule, the "an integration skill can never be
   created or written in place" refusal (`AGENTS.md`, "WASM plugins /
   ponytail") — applies to the repository's own skills as if they were
   vendor content. The `Project` variant's documented behaviour ("readable
   and invocable; `create`/`delete` refuse it") is not implemented by any
   loader.

2. **No quarantine, no scan.** `scan_skill` has exactly two production
   callers, both in `crates/operant-cli/src/cmd_skills.rs` (`:751`, `:868`) —
   i.e. the explicit `operant skills` install/check paths. The loader
   (`skills.rs`) never imports `skills_guard` at all
   (`rg 'skills_guard|scan_skill' crates/operant-core/src/skills.rs` → zero
   hits). So skills discovered in a **cloned repository's `skills/` directory**
   are walked, parsed and made invocable with no content scan whatsoever.

Failing case: clone a repo that carries `skills/do-us-a-favour/SKILL.md`. It
loads at the project root as an `Integration`-labelled, `root_rank`-1 skill,
appears in `/skills`, is invocable as `/do-us-a-favour`, and no guard ever
read a byte of it. The operator's only signal is `operant skills check`, which
they must know to run and which nothing suggests for a project root.

This is the same trust boundary the ponytail content path DOES guard
(`scan_content_against_guard`, `binary_updater.rs:746`–`:792` — see Z24/Z25),
so the repo has the mechanism and applies it to vendor tarballs while skipping
it for a stranger's git checkout.

**Fix shape:** give `from_roots` the third position (project roots → `Project`
origin), and either scan a project skill at load or surface a per-skill
`unscanned` marker in `/skills` so the operator can tell.

### Z35 — [VERIFIED] **LOW** AGENTS.md's CLI count is off by one: the enum has
### **53** variants, the doc lists **52** and omits `continue` (lane I, filled
### in here — lane I never wrote its command table)

Method: extract the variant list from `enum Commands`
(`crates/operant-cli/src/main.rs:204`) and diff it against the prose list at
`AGENTS.md:1084`–`:1089`, then re-derive both counts.

```
enum variants:      53
AGENTS.md listed:   52
in enum, NOT listed: ['continue']
listed, NOT in enum: []
```

Full enum order (53): run, autonomous, tools, chat, test, config, sessions,
**continue**, mcp, cookies, skills, model, completion, cron, pause, resume,
suggestions, kanban, org, gateway, checkpoints, memory, context, profile,
auth, login, logout, version, doctor, status, dump, logs, backup, import,
uninstall, update, insights, webhook, hooks, debug, plugins, architecture,
channel, sop, migrate, service, curator, setup, acp, dashboard, trajectory,
tui, donate.

The section's own closing paragraph is what makes this a real (if small)
defect rather than a rounding error: it says *"The list above is derived from
the `Commands` enum ... (52 variants). It used to claim 53 and list `hardware`
and `peripheral`, both removed in iter-892, while omitting `donate` and `org`,
both real commands with handlers. **Re-derive it rather than trusting it** —
see RG-010 for the check."*

So the doc has already been wrong once in the same direction, was corrected,
and is wrong again by the same off-by-one — the exact failure mode its own
paragraph warns against. `continue` is a real command (`Sessions` continuation
resume, `:94`–`:98`), not a phantom.

**Fix:** add `continue` to the list and change "52" to "53", or replace the
hand-maintained list with the `rg`-derived one the paragraph already
recommends. This is a one-line doc edit, not a code change.

### Z36 — [VERIFIED] Lane D independently reproduced the dead-store findings
### (Z12/Z13), with the same file:line evidence

Lane D (memory/context/persistence) confirmed, from its own sweep and not from
my results:

- **`compression_locks`**: declared `database.rs:106`–`:111` + index `:125`,
  with a full API (`acquire_compression_lock` `:3755`, `is_compression_locked`
  `:3773`, `release_compression_lock` `:3789`) and **three tests** (`:4877`,
  `:5242`). "An exhaustive workspace grep for those three method names outside
  `database.rs` returns **zero hits**."
- **`compartments` + `historian.rs`**: `database.rs:209`–`:226` (migration
  v5) + 3 indexes; `insert_compartment` / `compartments_for_session` /
  `last_compartment_span_end` (`:1007`/`:1034`/`:1074`) have callers only in
  `database.rs` tests (`:7074`–`:7190`). The writer was deleted in iter-2404
  and that deletion note says so itself (`agent/compress.rs:497`:
  "`compartments_for_session` had ZERO callers"). `historian.rs` (720 lines of
  span-planning logic) has no caller.
- **`codex_reasoning_items` / `codex_message_items`**: `database.rs:93`–`:94`,
  written by `save_message_full` (`:1425`–`:1445`), read by
  `get_session_messages_full` (`:1691`–`:1715`), and set to `None` at every
  construction site outside `database.rs` (`agent/run.rs:3540`,
  `session_recap.rs:296`, `tools/evidence_recall.rs:212`/`:314`) — so both
  columns are always NULL. `platform_message_id` (`:95`) is the same shape.

This is the second independent confirmation of Z12/Z13 (after my own sweep).
Lane D additionally confirmed the structural fact my eviction doctrine
check relied on: **tool-call arguments live in
`tool_calls[].function.arguments: String` (`client.rs:1491`–`:1496`), a
separate field from `content`, so `decay_render` truncating `content` only
(`:1811`–`:1815`) cannot truncate an arguments JSON** — the doctrine claim
holds by construction.

### Z37 — [VERIFIED] **MEDIUM** No retention on any table: `messages`,
### `context_ledger`, `session_events` and `verification_events` grow without
### bound (lane D finding D5, confirmed)

Lane D: `DELETE FROM` appears 10× in `database.rs`, and every occurrence is
either a session-scoped delete by explicit id (`:2625`, `:3681`) or a
single-row delete by key (`:2707`, `:2833`, `:2873`, `:2884`, `:2908`, `:3736`,
`:3792`). A grep for `retention|prune|vacuum|purge|cleanup` in `database.rs`
returns **zero hits**.

The sharpest table is `context_ledger` — one row per **model request**
(`database.rs:138`–`:156`), 16 numeric columns — and the code's own measured
note (`context_management.rs:52`–`:56`) describes a **988-message session**.
Every turn of every session appends to it forever.

The CLI has `operant checkpoints prune|clear` (`cmd_checkpoints.rs:3`–`:4`),
which is the one table with an operator-facing retention verb — proving the
operator need exists and is met for exactly one of the unbounded tables.

**Why MEDIUM and not LOW:** this is not a leak that shows up in a benchmark;
it is monotonic growth of a 16-column per-request ledger plus every message
row and every verification event, on the user's own disk, for the life of a
database file that is never rotated. On a fleet host running many sessions
this is the table that eventually makes `operant sessions` slow.

---

## Re-verification at the final pin (`897a8baf`)

**The audit was pinned three times.** It began at `25b86ae5` (iter-2472); a
peer then landed iter-2473 through iter-2477 while lanes were running. Rather
than ship findings verified against a superseded tree, every finding was
re-checked at the tip. This section records what that changed.

### Z34 — CLOSED. Fixed by iter-2474 (`baba858e`), verified at `897a8baf`

This is the audit's one finding that a peer's in-flight work made obsolete, and
it is recorded here rather than quietly dropped because the original evidence
is what makes the fix checkable.

Verified fixed at the tip:

- `crates/operant-core/src/skills.rs:695` now calls
  `self.load_root(&project, SkillOrigin::Project, usize::MAX, &mut loaded)` —
  a **third** `load_root` site, passing the origin that was previously never
  constructed by any loader.
- `:767` gates each project skill with `is_quarantined_skill(&path)`
  (`:467`). It is fail-closed: a directory that will not canonicalize is
  quarantined, a `dangerous` scan verdict is quarantined, and a panic inside
  the scanner is quarantined — on the stated reasoning that "the check did not
  run" is not evidence of safety.
- `RootIssue` gained a `"quarantined: dangerous scan verdict"` arm (`:198`) so
  `skills list` reports the quarantine with a fix hint.
- iter-2473 (`e69b3c02`) landed the content-digest scan cache that makes a
  load-path scan affordable, which is what unblocked this.

The peer's commit body describes the same root cause this audit found
independently: "`SkillManager::from_roots` FLATTENED it into the generic extra
roots, so every project skill loaded as `SkillOrigin::Integration`.
`SkillOrigin::Project` existed and was never passed to `load_root` by
anything, which is why the gap survived this long: the enum variant read as
implemented." That is `Z34` verbatim, from a second session.

### Findings re-checked and confirmed unchanged at `897a8baf`

| finding | re-check performed | result |
|---|---|---|
| `Z1` | `wc -c .ci/clippy-allowlist.txt` | 0 bytes — still empty |
| `Z4` | `rg 'output_styles' crates/operant-cli/src` | `settings_screen.rs:8` imports `{builtin_styles, find_style}` and never calls them — still imported-unused |
| `Z7` | `rg 'Current release line' AGENTS.md` vs `Cargo.toml` | doc 0.2.1, tree 0.3.0 — still drifted |
| `Z10` | counted `overlay_flags()` at `commands.rs:93`–`:136` | 29 names, AGENTS.md says 30 — still drifted |
| `Z15` | `rg '"mv"\|"cp"\|"rsync"' blast_radius.rs` | 0 hits; `verb_kind` still the 7-verb table — still open |
| `Z17` | read `ToolRegistry::execute` (`tools.rs:1089`) end to end | no disabled-set consult before dispatch — still open |
| `Z22` | `rg 'models_panel' tui/render/ tui/operant_ui/` | no renderer — still open |
| `Z23` | `rg 'thinking_row_map\|message_row_map' \| rg insert` | only `app/tests.rs:4650` — still open |
| `Z29` | `mark_job_run` position in `run_job` | `:405`, still after dispatch — still open |
| `Z35` | counted `enum Commands` variants | still 53, `continue` still unlisted — still open |

### Also landed by peers during the audit (adjacent, not duplicates)

- **iter-2475** (`919545a6`) — doctor's plaintext-credential check covered six
  fields by name; now table-driven. Adjacent to `Z5` (config secrets) but does
  not close it: `Z5` is about dead *code* in `operant-config`, not the doctor's
  field list.
- **iter-2477** (`897a8baf`) — `skill_manage` now rejects a description edit
  that would exceed `ADVERTISED_SKILL_DESCRIPTION_CHARS`, instead of clipping
  it. This closes lane G's `F-3` (the description limit being a clip with no
  write-time reject), which this audit had recorded as ASSERTED and therefore
  excluded from the ranking. Nothing in `Z1`–`Z37` depended on it.

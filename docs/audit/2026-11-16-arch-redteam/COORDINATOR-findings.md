# Coordinator's own verified findings

Audited revision: **origin/main @ 2d431ac0**.
Read-only. Every line below was read via `git show origin/main:<path>` or
`git grep ... origin/main`. The shared working tree is DIRTY with a peer
session's uncommitted files and was NOT read.

This file records only what I verified MYSELF, by reading the code, rather
than relaying a worker's claim. It exists so the synthesis can distinguish
"coordinator-confirmed" from "worker-reported".

## Contents

1. [C-1 — HIGH — `theme_files::dir()` bypasses the one-home resolver](#c-1--high--theme_files::dir()-bypasses-the-one-home-resolver)
2. [C-2 — MEDIUM — dashboard session token fails open to a constant](#c-2--medium--dashboard-session-token-fails-open-to-a-constant)
3. [C-3 — LOW — cron reclaim scan swallows its own DB error](#c-3--low--cron-reclaim-scan-swallows-its-own-db-error)
4. [C-4 — verified clean](#c-4--verified-clean-recorded-so-it-is-not-re-reported)
5. [C-5 — the `anyhow`-in-library-crates claim, measured](#c-5--the-anyhow-in-library-crates-claim-measured)
6. [C-6 — the A5 V2 "ungated mcp serve" finding, CALIBRATED DOWN](#c-6--the-a5-v2-ungated-mcp-serve-finding-calibrated-down-to-high)
7. [C-7 — the A5 findings I graded ABOVE A5's own grade](#c-7--the-a5-findings-i-graded-above-a5-s-own-grade)

---

## C-1 — HIGH — `theme_files::dir()` bypasses the one-home resolver

`crates/operant-cli/src/tui/theme_files.rs:20-39`.

The module doc states the intent plainly: `<config dir>/themes/<name>.toml`,
where `<config dir>` is `OPERANT_CONFIG_DIR` when set, else `~/.operant` —
"restated here because that helper is `pub(crate)`."

The restatement is incomplete. `dir()` reads `OPERANT_CONFIG_DIR` and then
`$HOME`, and **never consults `operant_core::platform::operant_home()`**. So it
ignores both `HERMES_HOME` and the sticky active-profile file.

Consequence: an operator who runs with `HERMES_HOME=/some/other/home`, or who
switches profiles with `operant profile use`, gets themes from the DEFAULT
profile's `~/.operant/themes` while every other store (database, config,
credentials, logs) follows the resolver. Custom themes silently disappear and
the built-ins still load, so the failure looks like "my theme file was lost"
rather than "the path resolved wrong".

This is live code: `theme_colors.rs:345`, `theme_screen.rs:173` and `:207` all
call it, so the theme picker and the palette both read the wrong root.

### Why the repo's own guard missed it

`crates/operant-core/tests/one_home_resolver.rs` is the guard for exactly this
class. It passes today — I ran it: `the_operant_home_has_one_resolver ... ok`.
It missed this site because its detection requires BOTH:

1. the literal `".operant"` on a line, and
2. the substring `home_dir` within a `WINDOW` of that line.

`theme_files.rs` uses `std::env::var("HOME")`, not `dirs::home_dir()`, so
condition 2 never holds and the file is never examined. The guard is a
substring detector, and this is the input shape it does not cover.

I enumerated the evasion class: `git grep -l 'env::var("HOME")'` over
non-test sources returns 12 files, 8 of which also mention `.operant`
(`cmd_gateway.rs`, `cmd_migrate.rs`, `cmd_tui_debug.rs`, `theme_files.rs`,
`config.rs`, `doctor.rs`, `platform.rs`, `tools/aft_tools.rs`). I read each:
`config.rs` and `platform.rs` ARE the resolver (correct);
`cmd_gateway.rs:670` builds `~/.config/systemd` (genuinely OS-home, not an
operant store); `cmd_migrate.rs:30` builds `~/hermes-agent` (a source path);
`cmd_tui_debug.rs:2238` is inside a test. **`theme_files.rs` is the only
production violation**, and it is the one the guard cannot see.

### The fix

Route `dir()` through `operant_core::platform::operant_home()` and drop the
hand-rolled `OPERANT_CONFIG_DIR`/`$HOME` ladder. Then widen the guard's
detector from `home_dir` to also match `env::var("HOME")`, so the next
restatement is caught.

### Test coverage

`theme_files.rs:195-228` tests the `OPERANT_CONFIG_DIR` branch only, by
`set_var`-ing it and restoring. There is no test that sets `HERMES_HOME` or a
profile, which is why the defect is invisible to the suite.

---

## C-2 — MEDIUM — dashboard session token fails open to a constant

`crates/operant-cli/src/dashboard_server.rs:161-177`.

```rust
let mut bytes = [0u8; 32];
if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
    let _ = f.read_exact(&mut bytes);
}
```

Two independent silent-failure paths, both leaving `bytes` predictable:

1. If `/dev/urandom` cannot be opened, the `if let` is skipped entirely and
   `bytes` stays all-zero. The function returns `"operant_0000...0"` (64
   zeros) — a **constant, guessable token**.
2. `read_exact` is `let _ =`-swallowed, so a short read (a signal, a
   container with a restricted `/dev`) leaves a partially-zero, low-entropy
   token and reports success.

On this host `/dev/urandom` is `crw-rw-rw-` and present, so path 1 does not
fire today. But the failure mode is "silently mint a well-known credential",
which is the wrong direction for an auth token, and nothing detects it: there
is no test that asserts two calls differ, or that the token is not constant
(`git grep` for a token test returns nothing).

Severity is MEDIUM rather than CRITICAL because the trigger is an environment
fault, not attacker-controlled — but it is fail-open in a credential path,
and the dashboard token is the only thing between a network client and the
API.

### Secondary: the comparison is not constant-time

`dashboard_server.rs:319` compares with `t == *token`, an ordinary `==` that
short-circuits on the first differing byte. The rest of this repo does this
correctly: `gateway/webhook.rs` uses `constant_time_eq::constant_time_eq` for
every HMAC and verify-token comparison (lines 126, 175, 214, 409).

`operant-cli` does not depend on `constant_time_eq` at all, which is why this
is the odd one out. Bounded severity: the token is 256-bit random and the
server is local, so a remote timing oracle is impractical — but it is an
inconsistency in the one place a secret is compared, and the dependency is
already in the workspace.

### The fix

Make the token non-failure-optional: on either failure, return an error or
abort startup rather than minting a token. Then compare with
`constant_time_eq`. Add a test asserting 1000 generated tokens are pairwise
distinct and none is the all-zero string.

---

## C-3 — LOW — cron reclaim scan swallows its own DB error

`crates/operant-core/src/cronjobs/scheduler.rs:469`:

```rust
for job_id in self.db.abandoned_claim_jobs().unwrap_or_default() {
```

`abandoned_claim_jobs()` (`cronjobs/db.rs:1215`) returns `Result<Vec<String>,
Error>` and can fail on both `prepare` and `query_map`. `unwrap_or_default()`
turns any such failure into an empty vec, so a transient DB error during the
reclaim scan is indistinguishable from "no claims were abandoned".

The consequence is mild because the scan repeats, so the next tick re-tries —
but it means a wedge caused by a failed reclaim is silent, which is the exact
failure this code exists to prevent. The reclaim path was added to fix a real
HIGH (a claim settled as abandoned without re-arming `next_run_at` wedged the
job forever); leaving its error path silent reintroduces a quieter version of
the same "nothing tells the operator" problem.

Note the inner loop is careful: an unparseable schedule logs a `warn!` and
leaves the row alone rather than silently clearing it. The outer error path is
the one that lost that discipline.

### The fix

Match on the result and `warn!` on `Err`, or propagate. One line.

---

## C-4 — verified clean (recorded so it is not re-reported)

These are claims I checked and found CORRECT. They belong here so a later pass
does not re-open them as suspected gaps.

- **The cron wedge fix holds.** `abandoned_claim_jobs` is defined
  (`db.rs:1215`) AND called (`scheduler.rs:469`), and the reclaim loop
  re-arms via `compute_next_run` + `set_next_run`, with an explicit `warn!`
  when a schedule yields no next occurrence instead of silently disabling the
  job.
- **Webhook HMAC is sound.** `gateway/webhook.rs` uses
  `constant_time_eq` at every comparison (126, 175, 214, 409), rejects a bad
  signature with 401 (not merely logging), and has 5-minute replay windows
  for the Slack (`v0:<ts>:<body>`) and Stripe timestamped schemes.
- **The unauthenticated-webhook POST is warned, not silent.** `webhook.rs:360`
  emits a `warn!` when no signing secret is configured, naming the exposure.
  The asymmetry worth noting: the WhatsApp/Meta GET verify branch at
  `webhook.rs:413` has `None => true` (no secret ⇒ accept the challenge) with
  no equivalent warning. Minor, and the GET path only echoes a challenge.
- **The binary updater refuses to install unverified bytes when a checksum
  manifest IS present, and loudly warns when it is absent**
  (`binary_updater.rs:314-332`). That is a documented, deliberate tradeoff
  with a `tracing::warn!` that names the residual risk — not a silent skip.
- **TUI reasoning-override test discipline holds.** I audited every
  `set_reasoning_display_override` call site in `tui/app/tests.rs`; each is
  inside a test that holds
  `config_shim::reasoning_override_test_guard()`. `tests.rs:7274` looks
  unguarded but is the cleanup tail of
  `reasoning_alias_sets_the_live_thinking_mode`, whose guard is at `:7240`.
- **The one-home guard's ALLOWED entries all carry written justifications**,
  as AGENTS.md requires (13 entries, each with a reason string).

---

## C-5 — the `anyhow`-in-library-crates claim, measured

AGENTS.md: "Library crates return typed errors; `anyhow` is for binaries
(`operant-cli`) only."

Measured, not assumed:

| crate | `anyhow` a declared dep? | `anyhow` in public signatures |
|---|---|---|
| operant-api | **yes** (`anyhow = "1.0"`) | 34 |
| operant-core | **yes** (workspace) | 41 |
| operant-config | **yes** (`anyhow = "1.0"`) | 1 |
| operant-plugins | **yes** (`anyhow = "1.0"`) | 1 |
| operant-os | no | 0 |
| operant-harness | not a dep | 0 |

So the stated rule is not the rule the tree follows. The largest concentration
is `operant-api/src/channel.rs`, where `anyhow::Result` appears in 34 PUBLIC
TRAIT METHOD signatures (`send`, `listen`, `start_typing`, `send_draft`, …) —
a library crate publishing an error type it does not own.

Caveat, and it matters: `operant_api::channel` has **zero** references outside
its own crate (`git grep 'operant_api::channel'` over `crates/` outside
`operant-api/`), and no implementor. So this is a discipline violation
expressed entirely in dead surface — which makes it a LEAN cut (see A9) more
than a live correctness risk.

The `.unwrap()`/`.expect()` rule DOES hold. I ran the strict gate per crate:

```
cargo clippy -p <crate> --lib -- -D clippy::unwrap_used -D clippy::expect_used
operant-core: 1   operant-api: 0   operant-config: 0   operant-plugins: 0
```

The single violation is `crates/operant-core/src/agent/events.rs:816`, a
production `.expect("user_turns RwLock poisoned — programmer error")` with no
`#[expect]` annotation. Everything else carries one.
---

## C-6 — the A5 V2 "ungated mcp serve" finding, CALIBRATED DOWN to HIGH

A5's V2 is correct on the facts and I verified every link of it:

- `blast_radius::screen` has exactly ONE caller in the workspace:
  `approval.rs:662`. Verified with `git grep -n 'blast_radius::screen' origin/main`.
- `mcp_serve.rs` contains **zero** references to `approval`, `check_tool_approval`
  or `blast_radius`. Verified: `git grep -c` returns nothing.
- `mcp_serve.rs:88` builds the FULL registry (`build_tool_registry`), and
  `:166` enumerates it for `tools/list` — including `terminal`
  (`builtin.rs:354`) and `code_execution` (`builtin.rs:324`).
- `handle_tools_call` (`mcp_serve.rs:372-410`) gates only on
  `registry.contains(tool_name)`, then calls `registry.execute` at `:404`.

So an MCP client can invoke `terminal` with an arbitrary command and nothing
screens it: not the hardline blocklist, not the pattern list, not blast radius.

**Where I disagree with the grade, and why.** A5 called this CRITICAL. It is
HIGH, because `mcp_serve` is **stdio** — `mcp_serve.rs:1-7`: "Reads JSON-RPC
request lines from stdin, writes response lines to stdout." There is no TCP
listener and no HTTP surface. The caller is therefore a local process the
operator configured, which could already spawn a shell on its own.

The exposure that keeps this HIGH rather than LOW is the intended deployment:
an operator adds `operant mcp serve` to Claude Desktop or VS Code precisely so
that a MODEL can drive these tools. In that configuration the model chooses the
tool arguments, and it does so with none of the guardrails the operator would
expect `operant` to apply — the same reasoning that makes the in-process agent
loop's gate worth having. It is the agent-loop gate, absent on the surface whose
whole purpose is to let a model call these tools.

`operant test <tool> <args>` (main.rs:3440-3470) is the same hole with a human
holding the keyboard, so it ranks lower still.

The fix A5 proposes is right: move `check_tool_approval` into
`ToolRegistry::execute` beside the disabled-set refusal, or add an explicit
`gated: bool` every non-agent caller must state. Note that `approval.rs:643-647`
already contains a comment claiming the gate "lives HERE, inside the one function
every tool call already passes through" — which is true of `check_tool_approval`
and false of the dispatch point, and that sentence is why nobody looked for a
third caller.

---

## C-7 — the A5 findings I graded ABOVE A5's own grade

I re-read the code for these two rather than relaying the worker's number.

### V1 (credentials printed verbatim) — HIGH, agreed

`cmd_config.rs:94-96` is `serde_yaml::to_string(config)` + `println!`, no
masking. `config_tool.rs:137` DOES mask (`redact_secrets`), so the policy exists
and one of the two surfaces honours it. `ClientSettings::api_key`
(`config.rs:565`) carries no `skip_serializing`.

The worker's fix caveat is correct and I checked it: do NOT add
`skip_serializing`, because `config_tool.rs:235` round-trips through
`serde_json::to_value(&AppConfig)` and a skipped field would be **silently
dropped** on the next `operant config set`. Reuse `redact_secrets` instead.

### V7 (`operant config env-path` reports the wrong file) — MEDIUM, raised to HIGH

A5 ranked this MEDIUM. The code makes it worse than "a wrong answer":

```rust
let env_path = if HERMES_CONFIG       { ... }
              else if HERMES_CONFIG_DIR { ... }
              else if dirs::config_dir() { config_dir/operant/.env }
              else { ~/.config/operant/.env };
if env_path.exists() { print(env_path) }
else if crate::env_store::operant_env_path().exists() { print(that) }
```

The FIRST candidate that exists wins. The runtime's actual file is
`env_store::operant_env_path()` = `operant_home()/.env` (`env_store.rs:51`).
`~/.config/operant/.env` is not the runtime path — but if one happens to exist,
it is printed **in preference to the real file**, and the real file's existence
is never consulted. So the operator is told the wrong path even when the right
one is right there. `HERMES_CONFIG_DIR` is read on this one line and nowhere
else in the workspace.

This is the failure mode where an operator pastes a path into a bug report, or
edits the wrong file expecting the runtime to notice. Two lines fixes it: call
`crate::env_store::operant_env_path()` and delete the ladder.

### V6 (guard brace-counter) — agreed MEDIUM, with a caveat in its favour

The worker is right that `strip_test_items` is a brace counter that a
brace-in-attribute can confuse, and that the fix must land in TWO places
(`one_home_resolver.rs` and the byte-identical copy at
`operant-config/tests/schema_is_not_loaded.rs:33-60`). The worker also
correctly reports that it ran the scan over the whole workspace and found **no
file that trips either case**, so this is robustness, not a live defect. That
honesty is why I am keeping it at MEDIUM and recording it as such.

It does, however, compound C-1: the guard is the ONLY mechanical enforcement of
the one-home rule, and it has (a) a name-shape matcher that misses
`env::var("HOME")` (C-1's root cause), (b) a brace-counting parser that can be
confused, and (c) 4 stale ALLOWED entries pointing at deleted files (V5). Three
independent weaknesses in the one gate that is supposed to make this class of bug
impossible. That is the real finding — not any single one of them.

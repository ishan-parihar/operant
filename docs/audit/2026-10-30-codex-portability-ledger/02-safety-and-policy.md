# 02 — Safety and policy: approval, permissions, execution policy

**Subsystem family:** what stands between a model's request and a command running on the
operator's machine, and whether the operator can express that boundary as data.

**Verdict key:** `IMPORT-DIRECT` · `IMPORT-PATTERN` · `SKIP`.

---

## Row 1 — execpolicy's `Decision` enum and prefix matching

- **Codex:** `execpolicy/src/decision.rs:1` (27 LOC) — `pub enum Decision { Allow, Prompt, Forbidden }` with `parse()`. `execpolicy/src/rule.rs:16-64` — `PatternToken { Single(String), Alts(Vec<String>) }` and `PrefixPattern { first: Arc<str>, rest: Arc<[PatternToken]> }` with `matches_prefix()`.
- **Size:** 27 + 58 = **85 LOC** of genuinely portable types.
- **Operant today:** `crates/operant-core/src/approval.rs:60-101` — `pub enum ApprovalVerdict { Allowed, Blocked { reason }, RequiresApproval { risk_level, reason } }` and `pub enum ApprovalMode { Manual, Smart, Off }`. These are **type-level** and already good.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** S for the types, M for the file format.
- **Value:** the *decision* model is already equivalent. The gap is **where the rules live**.

**Why it matters.** Operant's safety rules are **47 hardcoded regex patterns across 12
categories** plus a 12-category hardline blocklist, compiled into the binary
(`crates/operant-core/src/approval.rs:1-30`). Codex's rules live in a **file the
operator writes**, and the engine is a separate, independently testable crate
(`execpolicy/`, 1,251 LOC total).

The operator consequence: adding a rule to operant today means editing Rust and
shipping a new binary. In codex it means writing a line in a policy file.

**The portable part.** codex's `Decision` maps cleanly onto operant's
`ApprovalVerdict`: `Allow` → `Allowed`, `Prompt` → `RequiresApproval`, `Forbidden` →
`Blocked`. The `PrefixPattern` matcher (58 LOC) is a **better command matcher** than
operant's regexes because it matches on **token position**: `["git", ["push", "commit"]]`
matches `git push` and `git commit` but not `git push --force` beyond the prefix — and
`Alts` expresses alternatives without regex alternation syntax.

**The non-portable part.** codex's policy files are **Starlark**, not YAML/TOML:
`execpolicy/src/parser.rs:57-84` calls `AstModule::parse` and `Evaluator::eval_module`,
and `execpolicy/Cargo.toml` declares `starlark = { workspace = true }`. Importing the
parser adds a scripting language to the approval path. **Do not import it.**

**The ponytail version.** operant already has TOML config everywhere
(`crates/operant-core/src/config.rs`). A TOML policy file parsed by `toml` (already a
dependency) with a `[[rule]]` array carrying `program` / `args` / `decision` /
`justification` is ~80 lines of parsing code and reuses operant's existing config
idiom. Take codex's *rule semantics* and *decision model*; write the 80 lines of TOML
parsing rather than adopting Starlark.

**Also worth lifting verbatim:** the `justification` field
(`execpolicy/src/rule.rs:73-83`) — every rule carries a human-readable reason, and
codex surfaces it in approval prompts and rejection messages. operant's
`ApprovalVerdict::Blocked { reason }` has a reason but the *rule* does not, so the
reason is generated at match time rather than authored.

**And the CLI:** `execpolicy/src/execpolicycheck.rs` is a `codex execpolicy check
--rules path git status` subcommand that prints the JSON evaluation. operant has no
equivalent — the operator cannot ask "would this command be allowed?". That is a
**diagnostic** worth having, and it is a thin wrapper over the same evaluator.

---

## Row 2 — per-rule load-time validation (`match` / `not_match`)

- **Codex:** `execpolicy/src/rule.rs` — `validate_match_examples` / `validate_not_match_examples`; `execpolicy/src/parser.rs:372-377` parses `match` / `not_match` example invocations and `execpolicy/src/parser.rs:84` calls `validate_pending_examples_from`.
- **Size:** the validation itself is small; the whole `execpolicy` crate is 1,251 LOC.
- **Operant today:** `ABSENT` for policy-file examples. operant's approval patterns are tested by Rust unit tests in `approval.rs`, not by examples embedded in the rules.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** S
- **Value:** high, and cheap.

**What it does.** A policy rule can carry example invocations that are checked **when
the policy loads**:

```starlark
prefix_rule(
    pattern = ["cmd", ["alt1", "alt2"]],
    decision = "prompt",
    match = [["cmd", "alt1"], "cmd alt2"],      # must match this rule
    not_match = [["cmd", "oops"], "cmd alt3"],  # must NOT match this rule
)
```

Strings are tokenized with `shlex` (`execpolicy/src/parser.rs:297`).

**Why it matters.** When an operator writes a safety rule, the failure mode is a rule
that silently does not match — a typo in the program name, a wrong arity. Codex
catches that at **load time**, before the rule is trusted. operant's equivalent failure
is a regex that compiles but never fires.

This is the ponytail-optimal row in this document: the semantics are ~40 lines, the
value is "the operator's safety rules are self-checking", and it removes a whole class
of silent failure.

---

## Row 3 — the dangling `operant-runtime` sandbox reference (operant's own defect)

- **Codex:** N/A — this row is an operant defect found during the contrast, not a codex import.
- **Operant today:** **two** live dangling references, both claiming isolation exists:
  - `crates/operant-core/src/blast_radius.rs:25-26`:
    ```
    //! Real isolation lives in `operant-runtime`'s sandbox backends (landlock,
    //! bubblewrap, firejail, docker, sandbox-exec — `SandboxBackend`).
    ```
  - `crates/operant-core/src/approval.rs:654-657` (inside the enforced entry point):
    ```
    // Defense in depth, NOT a sandbox: it reads command text. See
    // `crate::blast_radius` for what it does not model, and
    // `operant_runtime`'s landlock/bubblewrap/firejail/docker backends for
    // real isolation.
    ```
- **Verdict:** `FIX` (not an import)
- **Effort:** S
- **Value:** correctness of the documentation, and it removes a false safety claim.

**The facts.** `operant-runtime` was deleted in iter-892 (AGENTS.md records the 2026-10-17
purge of `operant-runtime`, `operant-hardware`, `operant-gateway`, `operant-channels`).
It is absent from `Cargo.lock`. `crates/operant-os/src/policy.rs` contains **no**
`SandboxBackend` — the grep returns nothing. There is **no** sandbox layer in operant:
`rg -ci seccomp crates/` → 0 files; `rg -ci seatbelt` → 0; `rg -ci apparmor` → 0;
`rg -ci "restricted token"` → 0.

So both comments tell a reader that real isolation exists elsewhere. It does not. The
`approval.rs` one is the worse of the two, because it sits **inside the enforced
function** — the one place an engineer debugging an approval decision will read.

**Note on scope.** `rg -n "operant_runtime|operant-runtime" crates/ --type rust` returns
13 hits, but most are **historical notes** that are correct as history — e.g.
`crates/operant-core/src/observer.rs:207` ("used to live in the deleted `operant-runtime`
crate") and `crates/operant-core/src/tool_guardrails.rs:772` ("moved from
operant-runtime/src/agent/loop_detector.rs @ 4ddbe62e"). Those should stay. The two
that make a **present-tense claim about isolation that does not exist** are
`blast_radius.rs:25` and `approval.rs:656`. Fix those two; leave the history alone.

**The fix** is prose: say that operant has **no** OS-level sandbox and that the
approval guard plus blast-radius classification are the whole safety surface.

**This is the one row in this ledger that is a plain bug fix, and it should ship
first** — it is ~10 lines of comment across two files and it removes a false promise
from a security surface.

---

## Row 4 — composable permission profiles

- **Codex:** `protocol/src/permission_profile_intersection.rs:1` — `intersect_effective_permission_profiles(authority, requested, cwd)` with `PermissionIntersectionError { ExternalSandbox, PlatformDefaults, UnsupportedPath }`. It **fails closed** on unsupported shapes and canonicalizes grant paths so "symlinks cannot acquire authority beyond either input" (the file's own doc comment, `permission_profile_intersection.rs:35-38`).
- **Size:** the intersection file is ~230 LOC, but it depends on `protocol/src/permissions/` (a `PermissionProfile` model, `FileSystemSandboxPolicy`, `NetworkSandboxPolicy`, `FileSystemSpecialPath`).
- **Operant today:** `ABSENT`. `rg -n "PermissionProfile|permission_profile" crates/` → no hits. operant's closest thing is `crates/operant-cli/src/tui/adapter_types/config.rs:32` — `pub enum PermissionMode { AcceptEdits, Default, BypassPermissions, Plan }`, a **4-state mode selector** with no notion of a permission *set*.
- **Verdict:** `IMPORT-PATTERN` (the *concept*, far in the future) / `SKIP` (the code, now)
- **Effort:** L
- **Value:** conceptually important, practically not yet.

**Why it matters conceptually.** codex's model is: a permission profile is a **set**
(what paths are writable, what network is allowed, what is denied-read), and when two
authorities apply — an outer sandbox and an inner request — they are **intersected**,
and the intersection **fails closed** on any shape it cannot reason about. operant's
model is a **mode**: one of four presets, with the approval guard deciding
case-by-case whether to prompt.

**Why SKIP the code.** A permission profile only matters when there is a sandbox to
constrain, or multiple authorities to reconcile. operant has neither: one process, one
operator, one approval guard. `BypassPermissions` already covers the "no guard" case.

**The one idea worth taking now** is the **fail-closed** discipline: when the approval
guard encounters something it cannot classify, it should **block**, not allow. Let me
check whether operant does that.

---

## Row 5 — fail-closed on unclassifiable requests

- **Codex:** `protocol/src/permission_profile_intersection.rs:42-52` — external sandboxes, platform-default profiles, and unsupported path shapes each return a typed error rather than a permissive result.
- **Operant today:** two layers with **different** postures, which is the actual finding.
- **Verdict:** `SKIP` (operant already fails closed where it matters)
- **Effort:** — 
- **Value:** operant is already safe here. Recording it so nobody "fixes" a non-problem.

**The verified behaviour.**

Layer 1-2 (`ApprovalGuard::check`, `crates/operant-core/src/approval.rs:474-500`): in
`Smart` mode, a command that matches **no** pattern returns `ApprovalVerdict::Allowed`
at line 497. That is fail-**open** — an unrecognised command is permitted.

Layer 3 (`blast_radius::screen`, `crates/operant-core/src/blast_radius.rs:806-837`):
runs on everything layers 1-2 allowed (`approval.rs:658`). It is fail-**closed** for
the dangerous class — `classify()` returns `BlastRadius::None` for a benign command
(line 808-810, allowed), but an `Unbounded` command with **no** justification returns
`Refusal::MissingJustification` (line 823), and with an **insufficient** one returns
`Refusal::InsufficientJustification` (line 829). A denied path (filesystem root, home
root, system dirs, `.git`) is refused outright (line 816) regardless of justification.

**So the composition is:** unrecognised *commands* are allowed (fail-open), but
unrecognised *blast radii* are not — a command the parser cannot model rates as
`None` and is allowed. That is the same limitation `blast_radius.rs:17-24` documents
honestly: "a determined or confused caller can still reach a destructive command
through a form this parser does not model".

**Why SKIP.** codex's fail-closed rule applies to **profile intersection**, where the
inputs are permission *sets* and a set that cannot be reasoned about must not be
silently widened. operant's input is a command *string* with no set semantics, so
there is no "unsupported shape" to fail closed on — there is only "does this string
match a pattern", and the honest answer to a non-match in a `Smart` mode is "allow",
because `Smart` is by definition "only ask when something looks dangerous". Making
non-matches block would turn every ordinary command into a prompt.

**Keep the honesty, lose the false promise.** The gap is not the policy, it is the
documentation (Row 3): `approval.rs:654-657` says "Defense in depth, NOT a sandbox"
and then points at `operant_runtime`'s sandbox backends for "real isolation". There is
no real isolation. Fix the comment.

---

## Row 6 — sandbox backends

- **Codex:** `linux-sandbox/` (12,852 LOC: `seccomp.rs`, `bwrap.rs`, `launcher.rs`, `proxy_routing.rs`, `daemon_mounts.rs`) + `windows-sandbox-rs/` (30,369 LOC) + `bwrap/` crate. `protocol/src/sandbox.rs:1` defines `SandboxType { None, MacosSeatbelt, LinuxSeccomp, WindowsRestrictedToken, WindowsMxc }`.
- **Operant today:** no sandbox. See Row 3.
- **Verdict:** `SKIP`
- **Effort:** — 
- **Value:** not now.

**Why:** 43,000+ LOC of platform-specific isolation code, a seccomp filter layer, a
network proxy, and a mount daemon. The operator has not asked for a sandbox. operant's
current posture — blast-radius classification plus an approval guard — is a deliberate
choice with a documented limitation, and the ponytail-correct move is to **fix the
comment** (Row 3) rather than to buy 43,000 LOC.

**When this changes:** if operant ever gains a remote/gateway execution tier where the
agent's commands run on a machine the operator does not personally watch, sandboxing
becomes the answer. Note it in the ledger so the next reader does not re-derive it.

---

## Row 7 — tool-level hooks (`PreToolUse` / `PostToolUse` / `PermissionRequest`)

- **Codex:** `hooks/src/events/pre_tool_use.rs:1` (844 LOC), `post_tool_use.rs:1` (661 LOC), `permission_request.rs:1` (378 LOC). The event names are in `hooks/src/lib.rs:29-42` — `HOOK_EVENT_NAMES` is a 14-element array.
- **Operant today:** `crates/operant-core/src/gateway_pipeline.rs:65-89` — `pub enum HookEvent` with **9 variants**: `GatewayStartup`, `SessionStart`, `SessionEnd`, `AgentStart`, `AgentEnd`, `TurnStart`, `TurnEnd`, `Command(String)`, and `Hooks(String)` (a catch-all registration slot). `HookEvent::matches` at `gateway_pipeline.rs:91-101` supports a wildcard for `Command("*")` and a catch-all for `Hooks(_)`.
- **Verdict:** `IMPORT-PATTERN`
- **Effort:** M
- **Value:** operant can observe **turns** but not **individual tool calls**.

**This corrects iter-1484, twice.** That plan said operant had "4 events" and codex
had "9 events + wildcard". The verified facts:
- operant has **9 variants** (8 emitted + 1 catch-all slot), **not 4**. The
  `agent_start`/`agent_end`/`turn_start`/`turn_end` set that iter-1484 named is the
  subset the *CLI* documents (`crates/operant-cli/src/cmd_hooks.rs:4`), not the whole enum.
- codex has **14** events, **not 9**. The 9 figure counted only
  `HOOK_EVENT_NAMES_WITH_MATCHERS` (`hooks/src/lib.rs:48-58`) — the events whose
  matcher fields are meaningful. The full list: `PreModelRequest`, `PostModelRequest`,
  `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`,
  `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`,
  `Stop`, `Interrupt`.

**The actual gap** is narrower and sharper than "5 more events": operant's hook events
are all **turn-level or session-level**. There is **no** event that fires for an
individual tool call. An operator cannot write "log every time the agent runs `rm`"
or "block `git push` from the hook layer" — the approval guard is the only place that
sees individual tools.

codex's `PreToolUseRequest` (`hooks/src/events/pre_tool_use.rs:24-42`) carries
`session_id`, `turn_id`, `cwd`, `tool_cwd`, `transcript_path`, `model`,
`permission_mode`, `tool_name`, `matcher_aliases`, `tool_use_id`, `tool_input` — and
its outcome (`pre_tool_use.rs:44-50`) can **block** the call, supply a **reason**, add
**additional context** to the turn, or **rewrite the tool input**.

**Ponytail version.** operant's `HookEvent` is already a clean enum with a working
wildcard. Adding `ToolStart(String)` and `ToolEnd(String, String)` variants plus two
emit sites in the tool dispatch path is maybe 60 lines — but it needs the tool
**input** to be useful, which needs the richer `ToolContext` from Row 5 of
`01-tool-surface.md`. **These two rows are one change, not two.**

---

## Family summary

| Row | Mechanism | Codex LOC | Verdict | Effort |
|---|---|---|---|---|
| 1 | `Decision` + prefix matching | 85 portable | IMPORT-PATTERN | S (types) / M (file format) |
| 2 | `match` / `not_match` load-time validation | ~40 | IMPORT-PATTERN | S |
| 3 | dangling `operant-runtime` sandbox reference | — | **FIX (ship first)** | S |
| 4 | composable permission profiles | ~230 + model | IMPORT-PATTERN (concept) / SKIP (code) | L |
| 5 | fail-closed on unclassifiable requests | — | see below | — |
| 6 | sandbox backends | 43,000+ | SKIP | — |
| 7 | tool-level hooks | 1,883 (3 events) | IMPORT-PATTERN | M |

**Best in family:** Row 3 — it is a five-line comment fix that removes a false safety
claim from a security surface. **Best import:** Row 1, because it moves the operator's
safety rules from compiled Rust into an authored file without adopting Starlark.

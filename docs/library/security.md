# Security — the guard stack

Owns: operant's security mechanisms — the approval ladder, blast-radius
classification, justification measurement, redaction, the credential
registry, path safety, SSRF protection, and the write-origin/staging
gates.
Not for: skills' content scanning ([skills.md](skills.md)), the
platform-boundary question ([org.md](org.md)).

Source: `crates/operant-core/src/approval.rs`,
`blast_radius.rs`, `justification.rs`, `redaction.rs`,
`credential_files.rs`, `security.rs` (url_safety + more),
`fs_secrets.rs`, `write_origin.rs`, `write_approval.rs`,
`crates/operant-core/src/tools/file_tools.rs` (path guard),
`tool_guardrails.rs`, `crates/operant-core/src/tools/code_execution.rs`
(the NOT-a-sandbox disclaimer).

## The approval ladder (`approval.rs`)

A 3-layer guard for tool execution:

1. **Hardline blocklist** — commands/tools matching dangerous patterns
   ALWAYS require approval (12 categories: wildcard abuse, dangerous
   commands, network abuse, crypto abuse, sysadmin risk, exposure risk,
   privilege escalation, package risk, network exfil, service
   disruption, data destruction, infra exfil).
2. **Blast radius** (see below) — everything else is classified and the
   risky classes prompt.
3. **Allowlist** — `command_allowlist` config patterns + interactive
   "Always allow" choices persisted to
   `~/.operant/approval_allowlist.json`, honored across restarts.

Interactive prompts are real (TUI `p` key, gateway `/approve`);
non-interactive origins go through the staged gate (below).

## Blast radius (`blast_radius.rs`)

Every function is PURE and side-effect free: reads a command string,
returns a rating, never touches a terminal/filesystem/env/clock — so the
whole taxonomy is exhaustively testable without a shell. The
classification is STRUCTURAL, not a table of exact strings.

The module doc names one boundary it deliberately does **not** model:
indirection through a script or an interpreter (`sh -c`, `python -c`)
and runtime variable expansion. The consequence is measurable —
`bash -c "rm -rf /etc"` is rated `None`, because the verb `segments()`
(`crates/operant-core/src/blast_radius.rs:596-624`) sees is `bash`. codex
closes exactly this hole by unwrapping the interpreter before
classifying, then skipping `sudo` / `env` / `trap` wrappers to the verb
beneath them; the fix recorded for operant is ~34 lines of string work
with zero new dependencies, not a tree-sitter parser.

Note the asymmetry with `approval.rs`: its hardline blocklist matches
literal strings (so `rm -fr /etc` misses that layer), while
`blast_radius` has the same flag-cluster logic codex has
(`has_short_flag`, `crates/operant-core/src/blast_radius.rs:200-205`) and
`screen()` is called from `crates/operant-core/src/approval.rs:662` — so the
flag-permutation hole is already closed and the interpreter boundary is the
one that is not.

Row: `docs/audit/2026-10-31-codex-architecture-deepen/02-command-safety.md`,
`docs/plans/2026-10-31-codex-portability-the-twelve-rows-the-deep-dive-adds.md` §1.

## Justification substance (`justification.rs`)

The destructive-command justification gate must not be satisfiable by
retrying: it measures **substance, not length** — every signal is a
property of the token distribution (distinct content words, etc.), so
padding cannot move any of them. "A longer meaningless justification
must fail" is the whole point.

## Redaction (`redaction.rs`)

Hermes's 1197-line `redact.py` ported at its highest value: secrets
never reach the model, session logs, or transcripts — provider token
prefixes (`sk-…`, `ghp_…`, `xox…`, `AIza…`, `AKIA…`), env-shaped
secrets, and more, applied to tool outputs, LLM error messages, message
content, and the system prompt.

## The credential registry (`credential_files.rs` + `env_probe.rs`)

- `credential_files.rs` — the registerable layer of protected files the
  agent must never read or send (the file tools' built-in hard-deny is
  the base layer).
- `env_probe.rs` / the `env_probe` tool — a proactive, deterministic
  report of which environment variables look secret and are exposed to
  the agent process: NAMES, never values. Secrets are never
  blind-spotted into a prompt or trajectory.

## Path safety (`tools/file_tools.rs`)

All file tools reject paths that escape the user's home directory or
resolve to known-sensitive system files (the iter-125 fix for
"path-traversal validation" — `/etc/shadow`, `~/.ssh/id_rsa`).

## SSRF (`security.rs::url_safety`)

DNS resolution + IP class checks against private, loopback, link-local,
CGNAT, benchmarking, and cloud-metadata ranges. Fail-closed on DNS
error.

## Write provenance + staging

- `write_origin.rs` — task-local origin tracking (background review,
  cron, gateway set their origin) so write guards can scope by origin.
- `write_approval.rs` — staged write approval: non-interactive writes
  are STAGED for later approve/discard; interactive origin bypasses.
  See [background-review.md](background-review.md).

## Not-a-sandbox honesty

`code_execution` runs with the operant process's own permissions and
filesystem/network access — hard wall-clock timeout + whole
process-group teardown, but NOT a sandbox. The module doc, tool
description, and this page all carry the same disclaimer.

## Intention

The stack is layered because no single gate covers the failure space:
the model can propose destructive commands (blast radius decides), a
transcript can leak secrets (redaction decides), a skill or plugin can
exfiltrate (the guard + url_safety decide), a background writer can
mutate (staging decides), and a retrying model can grind past any
interactive prompt (the guardrail ladder decides). Each layer exists
because a specific incident forced it — the module docs carry those
incidents inline.

# Plan — native capability extension: MCP and CLI admitted at runtime, phase-activated (2026-10-28)

Date: 2026-10-28 · Status: **proposed** · Owner: operator agent

This plan is written to be **red-team audited after implementation**. Every claim that a
reviewer must be able to falsify is stated with the file, line, or measured number that
supports it, and every capability that does NOT exist yet says so plainly. A reviewer who
finds a claim here unsupported should treat that as a defect in this document, not merely in
the code.

---

## Problem

Operant extends the agent's capability two ways today, both compile-time:

| Family | Admission today | Evidence |
|---|---|---|
| MCP servers | config-declared, then discovered at connect via `tools/list` | `mcp.rs:475,497,976,1022` call `list_tools()` → `send_request("tools/list")` |
| CLI tools | **compiled into the binary** — `const KNOWN_CLIS: &[KnownCli]`, 16 entries | `crates/operant-core/src/tools/cli_discovery.rs:58`; live binary reports exactly 16 (`operant context cli list`) |
| CLI invocation | **does not exist** — gestures only in help text | `cli_exec` appears solely in doc comments at `cmd_context.rs:135,152,160,1039` and `config.rs:56` |

The operator's requirement, restated so an auditor can check it:

> Nothing of CLI/MCP may be compile time. As MCP servers are added via `tools/list`, CLI
> capabilities are added via their `--help` flags, so the relevant context is injected into
> agent-runtime.

### The cost that makes this urgent (measured, not asserted)

Captured from the deployed binary's own wire — `operant mcp serve`, JSON-RPC `tools/list`:

| Surface | Serialized bytes | Est. tokens (operant's `estimate_tokens`, `div_ceil(4)`) | Per capability |
|---|---|---|---|
| 94 tools, full JSON schemas | 75,016 B | **18,754 tok/request** | 798 B |
| Same 94, names joined only | 1,224 B | **306 tok/request** | 13 B |
| 106 skills, name + one line (frozen prefix) | 13,610 B | 3,403 tok | 128 B |

**Tools cost 6.2× more per capability than the skills surface that already works.** Phase
activation is worth **~18,448 tokens per request** on the current registry.

### The seam that makes this cheap (the single most important architectural fact)

`tools/call` dispatches on the **registry**, not on what was advertised:

- `mcp_serve.rs:390` gates `tools/call` with `registry.contains(tool_name)`
- `mcp_serve.rs:352` builds `tools/list` from `registry.get_schemas()`
- `tools.rs:707` `execute()` resolves by name via `resolve_alias` (`tools.rs:462`)

**"Advertised" and "resolvable" are already decoupled.** Therefore phase activation needs
NO new dispatch machinery — names can be advertised cheaply while full schemas resolve on
demand. Any proposal that introduces a new dispatch layer is over-solving this.

---

## What is NOT in scope

Stated so the red-team does not audit phantom work:

- **No new crate, no new "platform-ops" subsystem.** The skills surface
  (`skills_tool.rs`, `skill_view`, `<available_skills>` in `agent/events.rs`) is the native
  model; this plan generalizes it, it does not replace it.
- **No embedding / vector search** over tool or CLI descriptions.
- **No auto-pruning daemon** that revokes capabilities by usage statistics.
- **No change to `is_deferrable`'s existing behavior** for `mcp_*` and the 23 sourcehound
  composites (`tool_search.rs:80`) — Move 1 generalizes that seam rather than replacing it.
- **Nothing in Holon.** No Holon-manifest parsing, no unit registry, no department
  ownership. Operant-native only, per the operator's instruction.

---

## Phase A — CLI capabilities admitted at runtime via `--help`

### A0. The attack surface, named before the code

`--help` introspection requires **executing a binary the operator named**. That is
precisely the risk the current `const` catalogue avoids by construction. It is not
avoidable here — it is the mechanism the operator asked for — so it must be bounded, and
this plan states the bounds so a reviewer can check each one:

| Risk | Bound the implementation MUST show |
|---|---|
| Arbitrary code execution at admission | The binary is executed **only** with `--help`, never with user-supplied args. No argv from config reaches the spawn — a config field can name a *binary*, nothing else. |
| A hostile binary hanging the agent | Reuse `run_with_timeout` (`cli_discovery.rs:204`): stdin closed (`Stdio::null`), stdout/stderr piped, hard deadline. **No new spawn primitive may be written for this.** |
| A hostile binary writing to disk / reading env | spawn inherits env today; the introspecting spawn MUST NOT pass secrets — assert in review that no `HOME`/`OPERANT_HOME`/credential env is forwarded, or that the child's output is parsed only for help text. |
| Output used as instructions | `--help` text is **data, never instructions**. The injected `<cli_reference>` must present it as a description of an external command, and the existing approval gate must still cover invocation. |
| PATH shadowing | A config-named binary resolving through a writable PATH dir is equivalent to the existing `terminal` tool's exposure; the plan does not widen it, and the approval gate remains the control. |

A red-team pass that finds an unbounded spawn here has found a genuine defect; the review
checklist in "Verification" includes it explicitly.

### A1. Replace the const with a runtime roster

`cli_discovery.rs:58` `KNOWN_CLIS` becomes a TOML roster under the operant home
(seed default: the 16 entries below, so nothing regresses). Discovery keeps probing via
`command -v`, so a roster entry without an installed binary reports *not installed* rather
than being dropped — the catalogue stops overstating what exists.

Seed entries (migrated verbatim from the existing const): `git`, `python`, `python3`,
`node`, `npm`, `pip`, `pip3`, `docker`, `cargo`, `make`, `kubectl`, `rustc`, `claude`,
`gemini`, `kilo`, `gws`.

### A2. Admit by `--help`, the way MCP admits by `tools/list`

For each installed binary:

1. run `<bin> --help` under A0's bounds;
2. parse the discovered subcommands into a roster of named capabilities — **one CLI binary
   yields N capabilities, exactly as one MCP server yields N tools**;
3. persist the parsed roster so admission survives restart without re-executing `--help`;
4. keep the operator's hand-edited stanza authoritative where it overlaps the parsed one.

`git --help` should yield `add/commit/push/status/...`; this is the parity claim and it is
falsifiable — a reviewer can run the admission and count the names.

---

## Phase B — Phase-activated disclosure of every extended capability

Mirror `<available_skills>` (`agent/events.rs`) rather than inventing a shape:

- **Phase 1 (frozen prefix):** `<available_capabilities>` carrying **name + one-line
  description** only, for every extended capability (MCP tools, CLI subcommands, and the
  already-deferred composites). Byte-stable so prompt-cache hits survive.
- **Phase 2 (on demand):** full JSON schema by name. The expanding tool already exists as
  `tool_describe` (`tools/tool_search.rs`) — it is **promoted from bridge-only to a
  first-class disclosure path**, not rebuilt.
- **Hot set stays eager:** a small always-described set, measured options:

| hot size | Total (hot schemas + rest names-only) |
|---|---|
| 10 | ~5,561 tok |
| 20 | ~8,213 tok |
| 30 | ~10,046 tok |
| 0 (all names-only) | **306 tok** |

The hot set is a config knob, not a hardcoded list — the plan does not pre-judge which
tools are hot, it only asserts that some set must be or every call round-trips.

---

## Phase C — the invoke path (`cli_exec`)

Today `cli_exec` **does not exist** — only doc comments promise it
(`cmd_context.rs:135`, `config.rs:56`). This plan must build it, or Phase A admits
capabilities the agent still cannot call.

1. a `cli_exec` tool that invokes an admitted CLI subcommand, with `global_args` /
   `allow_subcommands` **enforced rather than stored** — both are read by nothing today,
   which the shipped help text already admits ("stored for phase 2 and read by nothing
   yet");
2. invocation routes through the existing approval gate (`agent/stream.rs`,
   `agent/mod.rs`) so admitting a capability never implies silently running it;
3. `<cli_reference>` injection carries the parsed `--help` text as the description block the
   operator asked for.

---

## Phase D — agent self-management, integrated like skill management

One management surface named `cap_manage`, mirroring `skill_manage`'s verb shape rather
than inventing a new one:

- `list` — admitted capabilities, with install/availability state
- `add` / `remove` — admit a CLI binary or MCP server by name
- `enable` / `disable`
- **`cost`** — the per-capability token bill, ranked. Neither family exposes this today; it
  is the verb that makes bloat visible to the agent instead of only to an auditor, and it
  reads the numbers measured in "Problem".

CLI admission via `context cli ...` remains available; `cap_manage` is the in-agent face
of the same roster, not a second configuration store.

---

## Verification (each item falsifiable by a reviewer)

1. **Admission is runtime, proven negative:** with zero code change, `cap_manage add
   <unlisted-binary>` succeeds for a binary the shipped binary had never heard of. Today
   `context cli add ssh` fails with *"not in the CLI catalogue"* — that exact string must
   stop being reachable for a binary that exists on PATH.
2. **`--help` parity, counted:** `git --help` admission yields N subcommands (N asserted,
   not "several"); a binary with a useless `--help` degrades to a single capability and
   does not crash admission.
3. **Bounded spawn, mutation-proven:** removing the timeout/stdin-closed/inherit
   constraints in a scratch copy makes the A0 tests fail by name. A `run_with_timeout`
   bypass compiles nowhere in the new code.
4. **Schema decoupling, mutation-proven:** advertising a tool without emitting its schema
   must still allow `tools/call` to resolve it — `registry.contains` is the gate, not the
   advertised list. Reverting `get_schemas` to full emission must break the *cost* test,
   not the *dispatch* test.
5. **Measured cost, before and after:** the wire capture (the same `operant mcp serve` +
   JSON-RPC `tools/list` probe used to produce this doc's table) must report the
   post-implementation per-request cost at or near the predicted figure, with the delta
   recorded on this doc.
6. **Prompt-cache stability:** the phase-1 block is byte-stable between turns with an
   unchanged roster; a roster change changes it. Admit capability → add capability →
   confirm only the delta moves.
7. **`global_args`/`allow_subcommands` become real:** stored values must change `cli_exec`
   behavior (RED test: a default-deny `allow_subcommands` blocks an unlisted subcommand).
8. **Approval still gates:** an admitted CLI capability cannot execute without the same
   approval path `terminal` uses.
9. **Red-team checklist (explicit):** every row of the A0 table addressed in code and in
   the diff; a hostile `--help` printing fake instructions must not alter agent behavior
   (assert with a poisoning fixture, the way skills use one at
   `agent/events.rs:880`).

---

## Sequencing

| Phase | Depends on | Why this order |
|---|---|---|
| A1 (roster) | nothing | pure data migration, reversible, unlocks everything |
| A2 (`--help` admission) | A1 | the operator's core requirement |
| B (phase activation) | A2 | the 61× saving only matters once there is a real capability set |
| C (`cli_exec`) | A2, B | admit-then-invoke; today invoked by nothing |
| D (`cap_manage`) | A, B, C | the management face comes last, over a finished substrate |

A1 and A2 are one iteration each. B/C/D are separable, and C is the only phase that is
*blocking* in the sense that A without C admits capabilities the agent cannot call — if the
implementation stops after B, the shipped state must say so in the help text exactly as it
does today, honestly.

---

## Open questions for the owner (answered before Phase A starts)

1. **Hot set size** — default 0 (measure first) or ~10? The table gives the tradeoff.
2. **Who may admit** — the agent itself, or only the operator's config? The requirement
   says agents manage their own configuration; if that is wanted, admission writes to the
   operant-home roster and is auditable there. If operator-only, `cap_manage add` becomes a
   request rather than an action.
3. **`--help` output retention** — parsed subcommand roster only (recommended), or also a
   bounded raw excerpt per capability? Raw text is more useful to the model and more
   injection-prone; the default should be parsed-only unless the owner wants the excerpt.

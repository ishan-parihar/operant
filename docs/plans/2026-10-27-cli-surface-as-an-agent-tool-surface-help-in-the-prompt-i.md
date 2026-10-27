# CLI surface as an agent tool surface: `--help` in the prompt instead of N tool schemas

- Date: 2026-10-27
- Status: proposed (architecture, not yet executed — see "Why not just build it" at the end)
- Origin: operator request 2026-10-27, following the ponytail-audit on compat naming
- Companion plan: `2026-10-27-executing-the-remaining-implementation-gaps-identified-in-th.md`

## The idea, stated precisely

Today the agent's tool surface is **schemas**: each tool advertises a name, a JSON
schema, and a description, and all three ride every request. The proposal is to
add a second, complementary surface for CLIs: inject a **`--help` derived
reference** into the system prompt instead of N per-command schemas, so the agent
sees the CLI's real surface (flags, subcommands, syntax) at a fraction of the wire
cost, and executes commands through a small, uniform executor.

Three properties make this worth doing rather than another tool:

1. **The source of truth is the CLI itself.** No schema drifts from the binary —
   `--help` cannot be stale, because it is generated from the same clap tree that
   dispatches the command.
2. **Cost scales with surface area, not with registered tools.** One shared
   reference block, versus 23 sourcehound composites or 52 CLI subcommands each
   paying schema tax per request (the RT-192 wire-cost work measured this exact
   shape: ~590 tokens for five redundant memory tools).
3. **Agents stop hunting.** `agent/mod.rs:77` already instructs the model to run
   `operant <cmd> --help` for syntax — meaning the pattern is proven in practice
   and currently *unscaffolded*: each call re-discovers flags one `--help` at a
   time, and nothing tells it which CLIs exist in the first place.

## What already exists (this is an extension, not a build)

Verified at tip, and this is why the plan is small:

- `operant-core/src/tools/cli_discovery.rs` — `discover_cli_tools(additional,
  excluded) -> Vec<DiscoveredCli>` probes PATH for a `KNOWN_CLIS` roster and
  returns `{name, path, version, category}`. Already production-wired into the
  doctor (`doctor.rs:512 check_cli_tools`), so discovery, timeout handling
  (`run_with_timeout`, stdin closed) and the exclusion list all exist and are
  tested.
- `run_with_timeout(name, args, timeout)` — the safe-spawn primitive (hard
  deadline, piped stdio, no stdin) that the executor must reuse verbatim.
- The registry alias layer and `ToolRegistry` — the insertion seam.
- The `operant` CLI has 52 subcommands, each clap-generated, each with real
  `--help`.

What does **not** exist: nothing converts `--help` output into prompt text, and
no tool lets the agent invoke an arbitrary CLI.

## Architecture

```
                ┌─────────────────────────────┐
                │ discover_cli_tools()        │  EXISTS (PATH probe + roster)
                └──────────────┬──────────────┘
                               │ Vec<DiscoveredCli>
                               ▼
                ┌─────────────────────────────┐
                │ cli_surface::collect_help() │  NEW: one --help per CLI,
                │  - cached, bounded          │  truncated to a token budget
                └──────────────┬──────────────┘
                               │ Vec<CliHelp{name, text}>
                               ▼
                ┌─────────────────────────────┐
                │ prompt composer (frozen     │  NEW: a <cli_reference> block
                │  prefix, prompt-cache safe) │  on the tail, cache-stable
                └─────────────────────────────┘
                               │
                               ▼
                ┌─────────────────────────────┐
                │ cli_exec tool               │  NEW: ONE tool, uniform args
                │  {cli, argv[], timeout}     │  reusing run_with_timeout
                └─────────────────────────────┘
```

### Component 1 — `cli_surface::collect_help` (new, small)

For each discovered CLI, capture `--help` through the existing
`run_with_timeout`. What is INJECTED is decided by the measurement (see the cost
section): **name + Usage line only** — but `collect_help` still captures and keeps
the full text, because `cli_exec` can serve the tail on demand from the same
cache instead of re-spawning.

Two caching rules, both learned from this repo's own history:

- **Cache across turns, keyed by `(name, version)`.** A `--help` call per CLI per
  request would re-spawn 8-10 subprocesses every turn; the RT-191/1310 work
  already established that spawn costs dominate. Version in the key means a
  binary upgrade invalidates automatically.
- **Extract, do not truncate.** The block carries the `Usage:` line (and the first
  description line as a fallback where a CLI prints no `Usage:`), never raw
  capped `--help` text. The full text stays in the cache, retrievable through
  `cli_exec` — so the agent can always get depth without it costing every request.

Russian-doll rule: **nesting is explicit, not discovered.** The injected block
advertises `cli_exec` and names each CLI; the agent learns subcommands by
running `cli_exec(cli="git", argv=["--help"])` or `git <sub> --help`. We inject
one level, not the tree — the tree is what the tool call is for.

### Component 2 — prompt injection point

Goes in the **volatile tail as a `<cli_reference>` prefix on the latest user
message** — the iter-911 prompt-cache contract, NOT the frozen system prefix.
Reason: the discovered set depends on what is installed *now*, so it is
per-turn volatile context, exactly like memory recall and workspace state.
Everything before the latest user message stays byte-stable, which is the
property that makes Anthropic cache breakpoints worth having (iter-54/39).

The block is compact by construction: tool name, one-line purpose, version, the
capped help text. Suggested shape:

```xml
<cli_reference>
Available command-line tools (invoke via cli_exec; subcommands: <cli> <sub> --help):
- git (2.39.3): usage: git [--version] [--help] ...
  <capped --help text>
- rg (14.1.0): ...
</cli_reference>
```

### Component 3 — `cli_exec` tool (one tool, not N)

```jsonc
{
  "name": "cli_exec",
  "args": {
    "cli": "git",              // must be a discovered CLI (allow-list, not arbitrary)
    "argv": ["status", "--short"],  // arguments, NOT shell — no sh -c
    "timeout_ms": 30000        // optional, capped by the host config
  }
}
```

The single most important safety property: **argv only, never a shell string.**
The existing `run_with_timeout` spawns `Command::new(name).args(args)` with no
shell, so injection through a filename or flag is not possible — that is the
difference between this and `terminal`. It inherits the registry's permission
flow (approval prompts) like any other tool.

### Component 4 — config surface

```toml
[cli_surface]
enabled = true              # default: true when any CLI is discovered
inject_help = "usage"       # "usage" (261 tok, the measured default) |
                            # "description" (285) | "off" ; full help text is
                            # REMOVED from the config surface — it cost 3,643 tok
max_per_cli_lines = 40      # cap for ON-DEMAND help retrieval only
additional = []             # extra names beyond KNOWN_CLIS (reuses the
                            # discovery param that already exists)
excluded = []               # exclusion list (already exists)
```

`additional`/`excluded` are already parameters of `discover_cli_tools`, so this
wires existing machinery rather than re-inventing it.

## Why the cost argument actually holds — MEASURED, and it refuted the first design

First implementation task done (measured 2026-10-27, this box, 15 CLIs discovered:
git cargo rg docker npm node python3 gh gws claude gemini kilo make grep curl).
Estimator: chars/4, the repo's own `estimate_tokens` convention. Measured, not
estimated:

| injected shape | bytes | tok/req |
|---|---|---|
| name + full `--help`, 40-line caps (the original design) | 14,569 | **3,643** |
| name + first-40-lines, 400-char caps | 5,438 | 1,360 |
| **name + one-line description** | 1,138 | **285** |
| **name + Usage: line only** | 1,043 | **261** |
| `cli_exec` schema alone | 299 | 75 |

The 40-line design — the one this plan originally proposed — costs 3,643
tok/req, barely better than the 23-composite schema surface (peer RT-194:
4,781 before the deferral fix) it was meant to replace. **Rejected on its own
number.** Inventing per-request cost to save schema cost is not a win.

What the number supports is the cheap shape, and it is a *better* product for the
same reason it is cheaper: inject **name + Usage line (261 tok/req)**, and let
subcommand depth arrive on demand. Verified on this box: `git stash --help`
yields 408 lines in one argv call, and `git --help` contains the full subcommand
catalogue. So the 261 tok/req buys the agent two things that no schema count
buys today — *which CLIs exist*, and *each one's real usage grammar* — while
`git push --help`, `cargo build --help`, and `docker run --help` cost zero
per-request tokens and land as a normal tool result in the turn that needs them.

The frozen-vs-volatile note still holds for all four shapes (volatile tail).
The plan's cost gate is therefore: **the shape must stay under ~300 tok/req for
this roster, or the block should be name-only.**

## Scope this does NOT include

- **Not a `terminal` replacement.** Interactive sessions, pipes, cds and shell
  state remain the terminal tool's job. `cli_exec` is one-shot argv.
- **Not arbitrary execution.** The CLI must be in the discovered set. A model
  cannot name a binary that is not installed; `additional` is operator
  configured.
- **Not operant's own 52 subcommands** as separate tools. Those already have a
  documented surface (`skill_view(name='operant')` + `references/cli-reference.md`)
  and the model already knows to use them; the injected block supplements, and
  operant itself can simply be one entry in the discovered set.

## Phasing

1. **Measure first — DONE, and it changed the design.** See the cost section:
   the 40-line design was rejected on its own number (3,643 tok/req). Surviving
   shape: name + Usage line at 261 tok/req.
2. **`cli_surface` + injection**, behind `[cli_surface] inject_help = "usage"`.
   Prompt-cache contract pinned by a test (the block rides the tail; the frozen
   prefix stays byte-identical with and without it).
3. **`cli_exec`** behind its own flag, RED-first: argv-not-shell (passing
   `argv = ["status; rm -rf x"]` must arrive as one argument to the CLI, not be
   interpreted), timeout cap honored, permission prompt fires, and a
   not-discovered CLI is refused.
4. **Docs + `operant.example.toml`** in the same change (the repo's rule).

## Why not just build it (why this is a plan, not a commit)

Two reasons, both from this repo's own history. The prompt-injection half touches
the prompt-cache stability contract, which took three iterations to get right
(iter-39/54/911) and has a live regression suite guarding byte-stability — that
deserves a test-first implementation, not a confident edit. And the executor half
is a new execution surface, which is exactly the kind of thing that should land
with its own RED tests for the argv-not-shell property before it is reachable.

Say go and I will execute phase 1 (the measurement) first, because if the number
does not beat the schema surface the rest of the plan is not worth building.

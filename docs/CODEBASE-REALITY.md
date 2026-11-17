# Operant — codebase reality (measured 2026-11-18)

Every number below was derived from the tree at `ac29a18a` with the command
shown, not copied from a document. Where a document disagrees with a command,
the command wins (that is `docs/handoff/STATE.md`'s own rule, and RG-016's law).

Derive the current values yourself rather than trusting this file:

```bash
git log origin/main -1 --oneline                      # tip sha + iteration
git ls-files | wc -l                                  # tracked files
find crates/*/src -name '*.rs' | xargs wc -l | tail -1 # rust src LOC
grep -cE '^- \[ \]' docs/audit/2026-06-19-BUGS.md     # OPEN findings
grep -cE '^- \[x\]' docs/audit/2026-06-19-BUGS.md     # CLOSED findings
bash scripts/check-doc-claims.sh                       # doc-vs-code drift gate
```

## What this project is

`operant` is a Rust coding **agent + runtime**: an agent loop with a tool
registry, a TUI, a CLI, a multi-platform messaging gateway, a WASM plugin host,
and a memory engine. Version `0.3.0` (Cargo.toml; AGENTS.md said 0.2.1 until
iter-2602). It is developed by a **fleet of concurrent AI agents**, not a solo
author — 3,688 commits on `origin/main`, ~900 in the last two weeks, iteration
labels in the 2600s. That social fact explains most of the repo's shape,
including its governance docs and its 12 numbered parallel-agent rules.

## Size and shape

| measure | value | how derived |
|---|---|---|
| tracked files | 2,635 | `git ls-files \| wc -l` |
| markdown files | 988 | tracked by extension |
| Rust files | 856 | tracked by extension |
| Rust src LOC | ~518,200 | `find crates/*/src -name '*.rs' \| xargs wc -l` (moves as peers edit) |
| crates | 7 | `Cargo.toml` members, all present on disk |
| commits on origin/main | 3,688 | `git rev-list --count origin/main` |

The ratio is the headline: **more markdown than Rust**. This is a
documentation-governed codebase (`genome`, `docs/decisions/`, `docs/regressions/`,
`docs/plans/`, per-workflow claim boards), where the prose is treated as a
claim about the code that must be mechanically checked.

## The seven crates (src LOC, measured)

| crate | LOC | files | role |
|---|---|---|---|
| `operant-cli` | 261,798 | 428 | CLI + TUI + gateway runner + dashboard + autonomous mode |
| `operant-core` | 239,908 | 276 | agent loop, tools, gateway adapters, memory, context, cron |
| `operant-harness` | 5,531 | 16 | harness boundary |
| `operant-api` | 3,382 | 14 | cross-crate type aliases / seams |
| `operant-os` | 2,829 | 13 | OS integration |
| `operant-plugins` | 2,815 | 7 | Extism WASM plugin host |
| `operant-config` | 1,854 | 11 | config types the live config composes |

Two crates hold **97%** of the code. Everything else is boundary or plugin
plumbing.

## Where the complexity actually lives (operant-core, by directory)

```
tools/      2.3M   the tool registry + every tool impl
agent/      2.0M   agent loop, model clients, fallback, error classifier
gateway/    528K   7 platform adapters (telegram/discord/slack/whatsapp/email/sms/webhooks)
cronjobs/   232K
context/    188K   tiered eviction + decay curve
org/        164K
oauth/      132K
kanban/     100K
```

## Test surface (measured)

- `operant-core`: 60 integration test files, ~3,925 `#[test]`/`#[tokio::test]` in src+tests
- `operant-cli`: 6 integration test files, ~2,875 in src+tests
- Core lib compiles: **0 errors**, 6 warnings
- CLI bin compiles: **0 errors**, 8 warnings
- CLI **test** targets compile: **0 errors**, 25 warnings
- `bash scripts/check-doc-claims.sh`: **exit 0** (all 12 code-surface claims match)

So the tip is buildable and its gates are green. The failing tests are a known,
bounded set (documented or proven isolation-flakes), not a broken tree.

## The open work: 16 findings, 643 closed

`docs/audit/2026-06-19-BUGS.md` is the live ledger. Measured: **16 open, 643
closed**. The open set clusters into recognisable themes rather than random
bugs:

- **Surfaces that exist but are not wired to anything** — RT-136 (config sections
  parsed then dropped), RT-275/RT-285 (dormant surfaces that look live), RT-179
  (72 TUI commands with no gateway twin), RT-547 (the thin client has no caller)
- **Two of something that should be one** — RT-190 (a second disconnected memory
  bank), RT-543 (two daemons sharing nothing), RT-542 (every session rebuilds the
  whole stack)
- **Claims that outrun the code** — RT-209 (a shipped module that is dead code),
  RT-293, RT-544 (a promised scale story that does not exist), RT-555 (a test
  that did not compile)

That taxonomy matters more than the count: the dominant defect class in this
repo is **not** "the code is wrong", it is **"the code and the claim about the
code disagree"**. Both this round's real findings (iter-2591 overlay count,
iter-2602 version) and several open ledger rows are that class.

## What the last session actually landed (iters 2591-2602)

| iter | commit | what |
|---|---|---|
| 2591 | `33294f49` | AGENTS.md said 30 TUI overlays, array held 29; fixed + added the missing gate |
| 2594 | `d745664d` | config: stale sidecar overruled the operator's own file; unreachable bare-key arm restored keys as unreadable tables |
| 2596 | `abd5cc2c` | corrected my own "5 of 5" overstatement |
| 2597 | `0fcdc0ec` | corrected the provenance gate (compare to last *code* commit) |
| 2599 | `8188ccf9` | corrected my ACP "race" claim (0/12 in isolation) |
| 2600 | `b4c9de72` | **AGENTS.md rule 12** — shared detached HEAD, peer reset can delete your commit |
| 2601 | `061e9d41` | recorded the iter-2600 duplicate label |
| 2602 | `ac29a18a` | release line was a minor version stale; added version derivation to the gate |

Two of those (2591, 2594) were real defects in shipped code; the rest were
corrections to claims, including my own.

## The three structural problems an owner should know about

1. **Stale prose is the default failure mode, and it is only partly gated.**
   `check-doc-claims.sh` covers 12 claims in AGENTS.md. It does **not** cover
   `docs/handoff/STATE.md` — which currently says "Four ledger items are OPEN"
   against a measured 16, and repeats the stale 0.2.1 release line. Any doc
   outside the gate drifts silently.

2. **The shared worktree is a single detached HEAD.** Every agent commits onto
   one head; a peer's `git reset` to sync can delete an unpushed commit
   (measured this session, recovered from the object store). Rule 12 now
   documents it, but the structural fix would be per-agent branches or worktrees.

3. **Scale is carried by two crates.** `operant-cli` + `operant-core` are 97% of
   ~518k LOC, and `operant-cli` alone is 428 files. That is where every change
   lands and where the compile cost lives.

## Orientation for a fresh agent

1. Read `AGENTS.md` top-to-bottom first — it is the operating manual, and its
   Parallel-Agent Coordination Protocol (now 12 rules) is load-bearing.
2. Run `./scripts/genome doctor` and `./scripts/genome feed --to <you>` before
   touching anything (fleet wake check).
3. `source scripts/dev-env.sh` before any cargo command; scope builds with `-p`.
4. Derive every number. Never copy a count from a doc — including this one.
5. Read `docs/INDEX.md` for which doc owns which question; `docs/library/` for
   how each mechanism works.
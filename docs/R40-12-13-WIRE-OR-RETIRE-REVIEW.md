# R40-12 / R40-13 — wire-or-retire review, and a YAGNI audit of the harness

Audited 2026-09-28 at `operant` HEAD. Every number below came from a command run
against the committed tree; the few I could not measure are marked. The
independent scout reports were spot-checked before any of this was believed —
three of their claims did not survive checking and are corrected here.

## Part 1 — the four crates that compile into every build but are never called

### The headline

None of the four is referenced by any code in the workspace. The only binary
target is `operant` (`crates/operant-cli/src/main.rs:11-13`), and it never names
them. Verified by search for both `operant_gateway::` and `use operant_gateway`
outside the crates themselves: **0 hits**.

They are in the build because `operant-cli/Cargo.toml:21-24` declares all four
`optional = true` and the features that pull them in (`channel-telegram`,
`channel-discord`, `channel-slack` at lines 133-135, `gateway` at 130) are
**default-on**. A default feature is a claim that a user can turn something on.
Here it compiles a subsystem that no command can reach.

### Per crate

| crate | LOC | state | verdict |
|---|---|---|---|
| `operant-gateway` | 16,503 | **unreached** — 0 code refs | RETIRE or wire the binary |
| `operant-channels` | 74,523 | **root is `start_channels`**, 0 callers | wire root only |
| `operant-runtime` | 88,714 | referenced *by* the other three, not the reverse | follow the others |
| `operant-tools` | 46,686 | same | follow the others |

**`operant-gateway` is the strongest retire case.** The two apparent references
the scout reported as "outside itself" are not references: `cmd_gateway.rs:262`
and `:269` are `systemctl --user is-active operant-gateway` — a *unit name* in a
string, and `operant-runtime/src/onboard/mod.rs:1114` is a comment. There is no
`use` of the crate anywhere.

It also contains the single most expensive piece of dead code in the workspace.
`crates/operant-gateway/src/api_config.rs` is a complete HTTP config facade —
`Config::get_prop` / `set_prop` over HTTP, including a JSON-Patch handler with
`test`/`replace` ops, `PropKind` type fidelity handling, and a documented
`map_prop_error` path. **1,880 lines.** It is *fully implemented and fully
unreachable*, because it is a handler module inside a crate that no binary
starts. Note the comment at `api_config.rs:3-5`: it advertises itself as a
second frontend over the same primitive as `operant config get/set/list`. That
is a design that was started and never connected.

**The important structural finding: the live channel stack is a different
implementation.** `operant_core::gateway` (`crates/operant-core/src/gateway/`,
11 files, 7 adapters — telegram, discord, slack, whatsapp, email, sms, webhook)
is what `operant gateway` actually runs, via `platform_registry()` at
`crates/operant-cli/src/gateway_runner.rs:613` and `build_adapters()` at `:704`.

So the workspace contains **two complete gateway/channel implementations**:
one that runs and one that does not. The 35 adapters in `operant-channels` are
reachable from `operant-channels::orchestrator::startup::start_channels`
(`orchestrator/startup.rs:26`), which has zero callers. Verified directly: a search for `start_channels(` across the workspace,
excluding its own definition and doc comments, returns **no external caller** —
the only other hits are prose. The 5 textual matches are doc comments
(`consts.rs:9`, `orchestrator/mod.rs:9,17`, `operant-runtime/agent/agent.rs:79`)
and the definition itself. Remove that one function and the entire internal
graph loses its root — `build_channel_by_id` (`factory.rs:12`) has one
non-test caller, `send_channel_message` (`factory.rs:508`), which itself has
zero callers. `CRON_CHANNEL_REGISTRY` is written once at `startup.rs:454` and
read once at `mod.rs:220`.

This is the same root cause as **R15-1**, which the ledger already subsumes
under R40-13.

### Recommendation

**Retire `operant-gateway`.** On `operant-channels`, **decide the two-world
config question first** (Part 2) — wiring its root is not a one-call fix once
you know what it would revive.

Rationale, and the cost of being wrong:

- **Retiring operant-gateway is cheap and low-risk** because the working gateway
  already lives in `operant-core`. You lose a *second* frontend for the config
  HTTP API. If that facade is wanted, it is recoverable — it is code that
  compiles, and `git` keeps it.
- **Wiring the channels root is NOT the cheap move the first draft of this
  document claimed.** iter-411 found that `operant-channels` is the only
  production consumer of `operant_config::` — the 1,043-field schema whose
  loader has zero callers. Wiring `start_channels` would bring that world
  back to life, alongside a hand-maintained allowlist bridging it to the live
  `AppConfig` (`gateway_runner.rs:716`). Settle the two-world divergence
  before wiring; see Part 2.
- **Do not wire all four** to "be safe." That is the YAGNI failure in reverse —
  committing to maintain four subsystems you have not decided you want. The
  decision you asked for is *which capability*, not *how to make everything
  reachable*.
- **The 35 adapters are real, not speculative.** Unlike most YAGNI candidates
  they have implementations, which is the test that matters. But they duplicate
  what `operant-core/src/gateway` already ships — that crate has **7**
  `impl ...Adapter for` sites, against 35 here. Check the overlap before wiring
  all 35: telegram, discord and slack exist in **both** stacks.

## Part 2 — YAGNI audit of the harness

17 crates, ~430k lines of Rust. The largest: `operant-core` 133k (232 files),
`operant-cli` 89k (217), `operant-providers` 38k, `operant-config` 31k,
`operant-memory` 14k.

### The pattern that matters most: three config schemas

1. `operant-config` — the pure-DTO schema, 31k, deriving a `Configurable` CRUD surface.
2. `operant-core/src/config.rs` — **a second, 2,501-line `AppConfig`**.
3. `operant-gateway/src/api_config.rs` — the unreachable HTTP facade (above).

Two independent `AppConfig` definitions in the same binary is not a design
choice, it is a divergence waiting to happen. Whichever is canonical, the other
should go. This is the highest-value structural finding in the audit and it is
**not** recorded in `BUGS.md`.

### Ranked findings

| # | finding | size | category | call |
|---|---|---|---|---|
| 1 | `operant-gateway` + its 1,880-line config facade | 16.5k | unreached | RETIRE |
| 2 | Second 2,501-line `AppConfig` in operant-core | 2.5k | duplicate | pick a winner |
| 3 | 35 channel adapters duplicating a live stack | 74.5k | duplicate | wire root, then dedupe |
| 4 | 4 crates in the default build with no entry point | 226k | dead weight in build | fix #1, then #3 |
| 5 | `DEAD_CODE_AUDIT.md` is stale | 63 items | process debt | delete or regenerate |

On #5: that file is a live trap. Its header still contains a literal `$(date)`
— a shell substitution that was never expanded — and it asserts four wire-up
items were "resolved in Phase 10" (commit `8797c093`). Anyone treating it as
current will act on conclusions this audit contradicts. Either regenerate it or
delete it. A stale audit is worse than none, because it is confidently wrong.

### What is NOT YAGNI — do not touch

- **`operant-core`'s size is not the problem.** 133k lines of engine + tools is
  proportionate for a working agent harness. Size alone is not redundancy; I
  found no evidence of duplicated *logic* within it, only the schema duplication above.
- **The 7 gateway adapters in `operant-core` are live** and are the ones your
  `gateway` command actually runs. They are not candidates.
- **The TUI and web dashboard are not redundant** despite rendering similar data.
  They are different consumption surfaces with genuinely different interaction
  models; the data-shaping they share lives in the API crate.
- **`operant-macros` and `operant-tool-call-parser`** (1.3k and 3.0k) earn their
  place — one is the `#[secret]` encryption attribute that R40-15 depends on, the
  other is a parser, not an abstraction.

### The one category I could not close — CLOSED, iter-411

A **write-only config knob** — a field that parses, persists, and displays but
changes no behavior — is a correctness bug, not waste, and this project has been
bitten by that class repeatedly (`forceLine`, `forceStage`, `targetSessionLength`;
R40-14's inert approval blocklist). That pass is now done.

**The method, and why it is trustworthy here.** The concern with a static
write-only-knob sweep is that absence of evidence is not evidence of absence: a
field read through a generated accessor or a stringly-typed path lookup has zero
text hits outside its definition. That risk is real in this repo and had to be
ruled out, not assumed. It was:

- `Configurable` (`crates/operant-macros/src/lib.rs:107`) generates `get_prop`,
  `set_prop`, `prop_fields`, `secret_fields`, `init_defaults` — a
  **stringly-typed CRUD surface** with no typed getters, which is exactly the
  shape that defeats text search.
- Every `get_prop(` call in the workspace outside tests is in
  `operant-runtime/src/onboard/field_visibility.rs:151,156` — and that is the
  **write** path, onboarding defaults being applied — plus
  `operant-config/src/helpers.rs`, which is the macro's own path parser.
- Every other `get_prop` call in the tree is inside `core_tests.rs`.

So a production behavioral read *cannot* go through `get_prop`. It must be a
direct Rust field access, which text search does find. The absence claims below
are therefore sound, not merely unrefuted.

### The finding: an entire config world is unreachable

| | world (1) `operant-config` | world (2) `operant-core/src/config.rs` |
|---|---|---|
| size | 1,043 fields / 154 structs | 266 fields / 36 structs |
| loader | `Config::load_or_init()` `config_impl.rs:299` | `load_app_config()` |
| non-test callers of loader | **0** | many |
| files in `operant-cli` using it | **0** | **61** |
| read at boot by the binary | no | **yes** |

**`Config::load_or_init()` has zero non-test callers** — verified by excluding
the definition and doc references. `operant-cli`, the only binary, contains
**zero** references to `operant_config::`. The only production consumers of that
crate are in `operant-channels`, which is itself unreachable.

The consequence: **the 1,043-field schema is dead, and so is every field in
it.** Not write-only — wholly unreachable. Any knob set in the shape that
schema describes has never been read by a release binary. That subsumes and
reframes the "which AppConfig is canonical" question: world (2) is the live one
and is already the de facto answer, by usage rather than by decision.

This also explains the third schema from Part 2. The HTTP CRUD facade
(`operant-gateway/src/api_config.rs`) reads and writes world (1) — the schema no
binary loads. So the config-editing surface writes to a world the runtime never
reads. That is the most serious item in this document: it is not dead code, it
is a **write-only subsystem**, and it is the exact defect class this project has
already paid for twice (R40-14, R40-15).

**This inverts my own recommendation from the first draft**, which proposed
wiring `operant-channels` at one point. If that were done, world (1) would come
alive through the channels path — reviving 1,043 fields whose relationship to
the live `AppConfig` is maintained by hand via a small allowlist
(`gateway_runner.rs:716`). Wiring the channels root is therefore **not**
obviously the cheap move it looked like; the two worlds' divergence has to be
settled first.

### No live write-only knob was found

Across world (2) — the 266 fields the binary actually reads — no field was
confirmed write-only. Every field examined had at least one behavioral read
site, or was read via the live `runtime_config()` global installed by
`install_runtime_config` (`cmd_config.rs:10,85`, `cmd_auth.rs:474`).

That is weaker than it sounds and should be read as such: it means no instance
was *confirmed*, not that none exists. The untested surface is the `#[nested]`
`HashMap` and `Vec<T>` sections, whose leaves are reached through `get_prop`
paths in places this pass did not exhaustively enumerate.

## Part 3 — agentic-utility, and how to integrate it

### First, the correction that changes the plan

`agentic-utility` **is not a Rust project collection and has no `.git`**. It is
a plain directory of ~20 *sibling repos* grouped into thematic folders. It has
no manifest, so "which projects in it" is really "which of these 20 repos".

Measured, and the language mix is the whole story:

| project | Rust | TS | Py | git remote | role |
|---|---|---|---|---|---|
| `memory/memory-wire` | 18.2k | — | — | `memory-wire.git` | **memory replacement** |
| `memory/tdg-rust` | 47.1k | — | 1 | `tdg-rust.git` | memory (superseded) |
| `internet/sourcehound` | 57.5k* | 392 | 166 | *(no remote)* | **IGS replacement** |
| `internet/browsefleet` | — | 69 | 2 | `browsefleet.git` | web, TS |
| `internet/cloakctl` | — | — | 47 | `cloakctl.git` | browser, Python |
| `memory/_audit/*` | — | — | — | third-party checkouts | **baselines, not candidates** |

\* sourcehound's real source is 57.5k; the ~196k remainder is a vendored
`obscura` browser engine under `vendor/obscura-crates/`. Do not read the raw
254k as its size.

Note `memory/_audit/` contains checkouts of **agentmemory, hindsight and amb**.
Those are evaluation baselines you built to compare against. They are not
candidates for integration, and integrating one would be integrating a
third-party project under your namespace.

### The mapping

**`sourcehound` replaces IGS — confirmed, with one caveat.**
It is Rust, it exposes an MCP server (`rmcp` 1.6) and a `sourcehound` binary
(`src/cli.rs`), and it already vendors the obscura browser engine, which is the
same engine IGS drives. It also currently carries **392 TS and 166 Py files**,
so the "one Python sidecar" goal is closer than it looks but not yet met.

Caveat: **it is not a drop-in either.** IGS is invoked as a subprocess —
`igs <args> --format json`, with SSRF gating and a 60s default timeout clamped to
5..600s from `tools.igs_timeout_secs` (`crates/operant-core/src/tools/igs.rs`).
`sourcehound` must implement that argv contract and its JSON output shape, or
`run_json` / `run_extract` in `igs.rs` must be rewritten against its real API.

**`memory-wire` replaces agentmemory — capability yes, API no.**
18.2k lines of Rust, lib + bin, axum REST *and* an rmcp MCP server. It
implements the capability you want: hook-based auto-capture, banks, retain /
recall / reflect, consolidation. Its own docs say it mirrors
"Hindsight banks/observations/mental-models and agentmemory."

**But it is not a drop-in, and this is the one hard blocker.** I checked:

```
$ grep -rn 'agentmemory' memory-wire/src
  src/lib.rs:7   // doc comment comparing the two
  src/memory.rs:3, 20   // comments
  src/capture.rs:1      // comment
$ grep -rn '\.route(' memory-wire/src/main.rs
  /health
  /banks/:id/retain | recall | reflect
  /banks/:id/memories | memories/:mid | stats
```

Zero route matches for `/agentmemory/*`. Operant's contract is
`DEFAULT_AGENTMEMORY_URL = "http://localhost:3111"` with
`/agentmemory/smart-search` and `/agentmemory/remember` — hybrid BM25 +
embeddings, npx-spawned **pinned to 0.9.29**, Bearer secret, 5s/10s timeouts.
`memory-wire` speaks Hindsight's shape (`/banks/:id/recall`), not agentmemory's.

So this is a **rewrite of the provider**, not a config change. `AgentMemoryProvider`
in `agent_memory.rs` gets replaced by a `MemoryWireProvider` implementing the
same `MemoryProvider` trait, and a migration must move existing banks. The
53-tool MCP registration is the good news: it is lazily connected, so swapping
the server behind it is a small change.

`tdg-rust` (47k) is the previous generation of this and is already recorded as
removed from operant. Do not re-integrate it.

### Your requirement: no static code, pulled at build time

You said you don't want static vendored code and want vendors pulled on each
build. Worth being precise, because the three mechanisms behave very
differently and two of them will not do what you want:

| mechanism | resolves at build? | stays current? | honest verdict |
|---|---|---|---|
| **cargo git dependency** | once, then pinned in `Cargo.lock` | **only after `cargo update`** | closest to your requirement |
| git submodule | no — pinned commit | no, drifts silently | fails your requirement |
| git subtree / `vendor/` dir | no — copied static source | no | exactly what you rejected |
| runtime auto-download (today's IGS) | no — at first use | yes, but *unreviewed* | supply-chain risk |

**The honest version of the freshness claim.** Cargo resolves a git dependency
once and writes the result into **operant's own `Cargo.lock`**; every later
build reuses that lock and does *not* re-resolve. A new upstream commit is
invisible until someone runs `cargo update`. So a git dependency gives you
**pinned freshness on a schedule you control**, not continuous freshness. Your
requirement — no static copied-in code, refreshed from the real upstream — is
met, and the git dependency is still the only mechanism of the four that meets
it. It just does not deliver "always current" on its own; a scheduled
`cargo update` is the missing half, and that is a CI step, not a cargo feature.

Operant's `Cargo.lock` currently has **0** git-sourced entries, so this would be
the workspace's first and the interaction is unproven here.

**Which surface a dependency would take** — decide this before committing:
- `sourcehound` ships **two targets**: a lib `sourcehound_mcp`
  (`Cargo.toml:72-75`) and a bin `sourcehound` (`[[bin]]`, `src/cli.rs`). A
  cargo dependency takes the **lib**; today's IGS subprocess contract needs the
  **bin**. That choice decides whether this is a code dependency or a
  process-swap.
- The lib is small but **real, not a stub**: `src/lib.rs` is a 650-byte
  re-export barrel exposing `server` (`SourcehoundMcpServer`), `config`
  (`load_settings`), `tools`, `http`, and `Settings`; the browser, clustering,
  fusion, cache and parsers are deliberately `pub(crate)`. It is embeddable —
  the public surface is the MCP server, not the scraping primitives.
- Its manifest declares `[package]` at line 1 and `[workspace]` at line 18 in
  the same file, with members being only `vendor/*` and `toon-helper`. The root
  package is its own workspace root while shipping ~196k lines of vendored
  obscura as members. A git dependency pulls that vendor tree along — static
  source inside the thing meant to stay current.

**Recommendation: use a cargo git dependency.** `crates/operant-cli/Cargo.toml:21-24`
already shows the pattern — `{ workspace = true, optional = true }` with the real
dependency declared in the root `Cargo.toml`. Put `memory-wire` and `sourcehound`
there the same way, add a scheduled `cargo update` in CI so freshness is real
rather than assumed, and pin a tag for release builds so they stay reproducible.

Four failure modes you are accepting, stated plainly:

1. **Network at build time.** Offline or air-gapped builds break. This is the
   real cost of the requirement, and it is the price of "always up to date".
2. **Unreviewed upstream code lands in your binary.** A build-time pull is
   *less* auditable than a pin, not more. Nothing reviews a new upstream commit
   before it compiles into your release. You are trading auditability for
   freshness, and freshness is what you asked for.
3. **Version skew.** A harness that expects response field `X` against a server
   that renamed it fails at runtime, not at build time. Pin the major version and
   add a contract test.
4. **Flakiness from upstream churn.** Every build becomes non-reproducible. Two
   builds a day apart produce different binaries. This is the strongest argument
   for a dated tag rather than a branch, and my recommendation is: **git
   dependency on a branch for development, git dependency on a tag for release.**
   Cargo supports both, and the tag is the only way to get a reproducible build.

`soul` — sorry, one more: **`sourcehound` has no git remote configured.** I
verified. You cannot cargo-depend on a directory with no origin, and this is
also the "no static code" hazard in its purest form — it is currently untracked
local source. Push it to a remote first.

## Part 4 — decisions I need from you

1. **operant-gateway: retire, or wire it as a standalone binary?** My call is
   retire — `operant-core`'s gateway is the live one and the 1,880-line config
   facade is recoverable from git. But it is your call, not an audit's.
2. **operant-channels: wire `start_channels`, or retire 35 adapters that
   duplicate 3 platforms already live in operant-core?** Lean wire-at-one-point,
   then dedupe telegram/discord/slack.
3. **The two config worlds are settled by usage, not by preference** — world (2)
   (`operant-core`'s `AppConfig`, 266 fields, 61 files in `operant-cli`) is
   live; world (1) (`operant-config`, 1,043 fields) has no loader caller and no
   binary reference. The open question is no longer "which is canonical" but
   "delete world (1), or merge it into world (2)". Merging is a large job;
   deleting leaves the HTTP config facade in the unreachable gateway crate with
   nothing to talk to.
4. **`memory-wire` rewrite: is the agentmemory data portable?** Before committing
   to the swap, confirm your existing banks can migrate to the
   Hindsight-shaped schema. This is the one item that could make the plan fail.
5. **Vendoring policy: branch (fresh, non-reproducible) or tag (reproducible,
   needs manual bumps)?** And confirm you accept builds requiring network.
6. **Which `sourcehound` surface do you want** — the `sourcehound_mcp` lib
   (a real code dependency) or the `sourcehound` bin (keeping today's subprocess
   contract)? And who pushes `sourcehound` to a remote, since it currently has
   none?

## What I did not do

- I did not delete anything. Every "RETIRE" above is a recommendation with a
  measured blast radius, not an action taken.
- I did not start the `memory-wire` provider rewrite. It is a substantial
  migration whose feasibility turns on decision 4.
- I did not re-run the compile or the doc build. `origin/main` is still broken
  by the missing `pub mod cache_monitor;` (R40-21), so any measurement taken now
  would be against a tree that does not build.
- I could not exhaustively enumerate the `#[nested] HashMap` / `Vec<T>` config
  leaves, whose paths are resolved via `get_prop`. No live write-only knob was
  confirmed in the 266-field live schema, but that is "none confirmed", not
  "none present".

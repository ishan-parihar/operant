# Implementation plan — what is genuinely left

Written 2026-09-29 at `96dfbd56`, against the tree as measured on that commit.
Every number here was measured this session, not carried forward. Where a
previous claim was wrong, the correction is recorded rather than the number
quietly replaced.

Three questions were asked. They have three different answers, and only one is
the answer the question anticipated.

---

## 0. The one-paragraph version

The project is **feature-complete and shippable**, and was until 31 commits
ago shipping with a red CI. That regression is fixed and CI is now green on
all three jobs. The genuine remaining work is small: one release, one missing
local gate, two small investigations, and one structural refactor that is a
real engineering decision rather than busywork. `sourcehound` and
`memory-wire` are not integrated at all and are a greenfield addition, not a
gap. `agentgateway` is not an inference provider, but there is a real
integration hiding inside the wrong question.

---

## 1. Where the project actually stands

### 1.1 Release state

| | |
|---|---|
| Latest release | **v0.2.0**, published 2026-09-28, `618f8f3b` |
| Artifacts | `operant-linux-x86_64.tar.gz` (20.9 MB), `operant-macos-arm64.tar.gz` (18.0 MB), `operant-windows-x86_64.zip` (19.9 MB) |
| Crate version | `0.2.0` — matches the tag |
| CI on `main` | **green** (Rustfmt, Clippy, Documentation) — verified on `ca811f70` |
| Commits since v0.2.0 | 31 — 10 code-touching, 21 docs-only, **4 user-visible** |

The three release-gating native legs all build. That was not true before
iter-442/444/445/446, when the Linux leg died on `alsa-sys`, the macOS leg on
a committed Linux ELF that ld64 refuses, and the Windows leg on a missing
`windows` dependency plus `tokio::signal::unix`.

### 1.2 The CI regression, and the gate gap that let it through

CI was **red on all 31 commits since v0.2.0**. One cause: a broken intra-doc
link (`[`palette`]`) in a doc comment added at iter-462, which
`-D rustdoc::broken-intra-doc-links` turns into a build failure. Fixed at
iter-476; `DOC_EXIT=0` against CI's exact command.

This is the **third distinct local gate gap** hit this session:

| gate | structurally cannot see |
|---|---|
| `check --workspace` | examples — cost a broken `main` at iter-409 |
| `test --workspace` | doctests |
| `check` **and** `test` | rustdoc — cost 31 red commits |

There is still no local alias for the third. That is item 2 below.

### 1.3 Ledger state

`BUGS.md` holds 35 `R40-*` entries. Re-verified individually rather than
trusted from status lines, **27 are resolved and 4 are genuinely open**. Six
headlines claimed open work that was already done; all were corrected at
iter-479. The four real items are R40-12, R40-22, R40-24, R40-25.

---

## 2. The backlog, ranked

### Item 1 — Ship v0.2.1 (mechanical, ~1 hour)

Not optional, and it is the cheapest thing on this list.

- **`docs/CHANGELOG.md`'s `[Unreleased]` last absorbed work at iter-414.** It is
  a *narrative* section (Fixed / Added / Changed / Known limitations), not
  per-iteration, so nothing is missing in a countable sense — the honest
  statement is that iter-415→479 was never added to it.
- That range contains **4 user-visible changes**, all keybinding:
  - `iter-455` — four wrong-action catalogue entries corrected, including
    `Shift+Tab` catalogued as "Previous completion" while actually cycling to
    `BypassPermissions`
  - `iter-453/454/457` — behavioural pins for `Ctrl+U`, `Ctrl+W`, `Enter+Shift`,
    `Ctrl+R`, `Tab`, `Ctrl+Y`, `Down`
  - `iter-459` — `operant.example.toml` listed the wrong browser default
- **Pre-flight, all verified this session:** CI green on `ca811f70`; all three
  native legs green as of the v0.2.0 build; crate version and changelog agree.
- Tag, confirm the tag-triggered Build, then confirm the release actually
  attached assets. A release entry with no artifacts is not a release.

### Item 2 — Add a local doc gate (small, prevents a recurring class)

`scripts/check.sh` has no `doc` subcommand. Add one that runs CI's exact
invocation, so the rustdoc gap stops being invisible:

```
RUSTDOCFLAGS="-Dwarnings" cargo doc --workspace --all-features --no-deps
```

The reason this is item 2 and not a footnote: three separate gate gaps this
session all had the same shape — a gate I believed covered more than it does.
Writing the third one down is what stops the fourth.

### Item 3 — R40-24: the `$HOME/` directory (small investigation)

An 8.8 MB untracked directory literally named `$HOME` sits in the repo root,
written by a workspace test run. Not in a source tarball, so it cannot ship
that way — but it is a test writing into the repo, and a single-quoted
variable somewhere is the likely cause. **A previous search failed to find the
creator and that is recorded, not papered over.** Worth one more pass with a
different method.

### Item 4 — R40-12 + R40-25: channel feature forwarding (the real refactor)

This is the largest genuine engineering item left, and the only one where the
answer is a decision rather than a chore.

**The defect.** `operant-gateway/Cargo.toml` declares `default = []` and 24
correctly-written forwarding features:

```toml
channel-email = ["operant-channels/channel-email"]
```

But its `[dependencies]` entry hardcodes 21 channel features inline:

```toml
operant-channels = { workspace = true, features = ["channel-signal", "channel-acp-server", ...] }
```

Cargo unions features across the whole graph, so those 21 are enabled
unconditionally and **all 24 forwarding features are decorative**. You cannot
build `operant-gateway` without those channels no matter what you select.

**This was measured, not assumed.** Removing 7 channels from operant-cli's
defaults produced a **byte-identical 51,026,384-byte binary**, 1 crate
recompiled against 12 in the baseline — because the edit was a proven no-op.
The change was reverted rather than ship a Cargo.toml comment asserting a
reduction that measurement had just refuted.

**Also relevant to AGENTS.md.** `agent-runtime` enables 23 `channel-*` features
by default, so "7 platforms only" is off by 17 — and because AGENTS.md states
that count as a *design preference* ("do not replace, remove, or improve"),
the code is what drifted, not the doc.

**The fix, and the trap.** Delete the hardcoded list, then have operant-cli's
`channel-*` features forward to **both** `operant-channels` **and**
`operant-gateway`. The second half is not optional: `gateway = ["dep:operant-gateway"]`
passes no channel flags, so removing the list without it silently drops every
channel. That is not a failure mode to discover during a dead-code cleanup.

### Item 5 — The three write-only config knobs (small, correctness)

Confirmed unread, and the serde false-negative was ruled out rather than
assumed: `operant-core/src/config.rs` has no `Configurable` derive and no
`get_prop` (those live on a *different* config type), the CLI's one generic
dotted-path surface is a **write-only** setter, and `operant config` has no
`get` subcommand.

| field | default | note |
|---|---|---|
| `max_consecutive_tool_only` | 90 | not in `operant.example.toml` |
| `event_channel_size` | 100 | documented at `operant.example.toml:260` |
| `lifeos_enabled` | false | **documented at `:285` and does not gate its feature** |

`lifeos_enabled` is the one to look at: it is the feature flag for the 22
Notion-backed LifeOS tools, a user can set it, the file parses, and the tools
are unaffected. A write-only knob is a correctness bug, not waste.

### Item 6 — R40-22: duplicate remotes (trivial)

`origin` and `github` both point at `github.com/ishan-parihar/operant.git`
under different local ref names; neither ref is refreshed by the other's
fetch, so tracking refs go stale silently. Note that `gitlab` **no longer**
aliases GitHub — it points at `gitlab.com/ishan-parihar/operant.git`, so the
original entry's claim that all three share a URL is false in the opposite
direction. Decide whether the GitLab remote is wanted.

---

## 3. `sourcehound` and `memory-wire` — not integrated, and that is the finding

### 3.1 The direct answer

**Neither is integrated. Not partially — not at all.**

- Both names appear in exactly **one file**: `docs/R40-12-13-WIRE-OR-RETIRE-REVIEW.md`, a review document.
- `Cargo.lock` contains **0** git-sourced entries, so neither is a dependency.
- `sourcehound` has **no git remote configured**. You cannot cargo-depend on a
  directory with no origin.

So there is no partial integration to assess and no implementation to audit.
This is a greenfield addition with a now-measured contract to hit.

### 3.2 What a replacement would actually have to do

This is the part that was missing, and it is large enough to change the
decision. Both current integrations are **larger than they look**.

**agentmemory is integrated twice, independently:**

| | `operant-core/src/agent_memory.rs` | `operant-memory/src/agentmemory.rs` |
|---|---|---|
| trait | `MemoryProvider` (19 methods, **3 required**) | `Memory` (different trait) |
| config source | `memory.agentmemory_url` from config file | **`AGENTMEMORY_URL` env var** |
| endpoints | 7 under `/agentmemory/*` | 4, same shapes |
| `forget` / `count` | — | **return "unsupported"** |

Replacing the backend means replacing **both**, and the second is the more
hidden of the two. The good news is the trait shape: 3 required methods, 16
with defaults, so a new provider is small.

**IGS is integrated in three files, not one:**
- `tools/igs.rs` — `web_scrape` / `web_extract` / `web_crawl` subprocess tools
- `browser_provider.rs` — the CDP browser provider
- `tools/web_providers/igs.rs` — the search-provider arm in the fallback chain

**Three premises I held that measurement corrected:**

1. **There is no `igs` auto-download.** Operant never fetches the `igs`
   binary; `IGS_INSTALL_HINT` is a `curl` string in an error message. The only
   auto-downloads are for **Obscura** and **Lightpanda**.
2. **`browser.provider` defaults to `"obscura"`**, not `"igs"`
   (`config.rs:1181`), and the `_ =>` catch-all is **Lightpanda**
   (`browser_provider.rs:767`) — a typo'd provider name silently becomes
   Lightpanda.
3. **The MCP server is lazily registered.** The "53 tools" is not eagerly
   spawned; it is injected as a deferred config entry, so `npx` only spawns if
   the user actually connects it.

**The blocking mismatch.** `memory-wire` speaks Hindsight's shape
(`/banks/:id/recall`), not agentmemory's (`/agentmemory/smart-search`). This
is a **rewrite of the provider, not a config change** — on both clients.

### 3.3 The decision this actually needs

Not "how do I integrate these" but **"should I".** The honest sizing:

- Both projects are the user's own, and `sourcehound` has no remote, so step
  zero is pushing it somewhere before any cargo dependency is possible.
- `memory-wire` is 18.2k lines and does implement the capability (hook-based
  capture, banks, retain/recall/reflect, consolidation).
- But the measured cost is **two provider rewrites plus 7 endpoints plus 3
  timeouts plus a pinned npx spawn plus a second hidden client**, against a
  backend that currently works and has 1965 passing tests behind it.
- `sourcehound` would replace IGS via the **subprocess** contract (its `bin`
  target), which means argv and JSON-shape compatibility, not a library
  dependency — its `lib` target is a 650-byte re-export barrel, so a cargo
  dependency buys the MCP server, not the scraping primitives.

**Recommendation: sequence these after v0.2.1, and treat memory-wire as a
rewrite to be scheduled rather than a swap to be performed.** The current
memory stack is the most deeply integrated subsystem in the project, and
`R@3`-style measurements are the only way to know a replacement is not worse —
which is exactly the discipline that stopped `semantic_compaction_cutoff` from
being wired on a feature that measured *negative*.

---

## 4. `agentgateway` — the question contains a category error

### 4.1 What it actually is

From its own README and topics, verified this session:

> **"Next Generation Agentic Proxy for AI Agents and MCP servers"**
> topics: `ai-gateway`, `api-gateway`, `reverse-proxy`, `service-mesh`,
> `kubernetes`, `rust`

It **never calls a model.** It routes traffic *to* OpenAI, Anthropic, Gemini
and Bedrock "through a unified OpenAI-compatible API", adding load balancing,
failover, budget and spend controls, prompt enrichment, guardrails, and
OpenTelemetry. It also federates MCP and does Kubernetes-native inference
routing.

5.1k stars, 894 forks, Apache-2.0, a **Linux Foundation** project, written in
Rust, self-described as "in active development".

### 4.2 Three different things are being called a "gateway"

The word does the confusing here, and operant uses it for a *third* meaning.
Worth separating before drawing any conclusion:

| | what it is | in this repo |
|---|---|---|
| **inference client** | calls a model — Anthropic/OpenAI HTTP, streaming tool calls | `operant-core/src/agent/clients/` |
| **`operant-gateway`** | **messaging platforms** — Telegram, Discord, Slack, WhatsApp, Email, SMS, Webhooks | `operant-core/src/gateway/`, 7 adapters |
| **agentgateway** | an **LLM proxy** — routes *to* providers, adds failover/budget/guardrails/OTel | external, not a dependency |

`operant-gateway` and `agentgateway` share a word and nothing else. The
naming collision is why item 4 of the backlog is genuinely hard: renaming
`operant-gateway` would be a large mechanical change across a crate that is
already hard to reason about (see its hardcoded feature list, §2 item 4).

### 4.3 So: is it a better inference provider than ours?

**No, and the question does not have that answer available.** Our inference
path is the Anthropic/OpenAI client inside `operant-core`. agentgateway sits
*in front of* those providers. They are not competitors; they are different
layers. Adopting it as an "inference provider" is not a swap — it is a
misunderstanding of what it does.

One prior of mine was also wrong and worth recording: I assumed it was
Kubernetes-only. Its README lists a **Standalone Quickstart** alongside the
Kubernetes one, so a non-k8s deployment path exists. I would rather check than
assert.

### 4.4 The real integration hiding inside the wrong question

Because agentgateway exposes an **OpenAI-compatible API** and operant's
OpenAI path is driven by a configurable `client.base_url`
(`operant.example.toml`, default `https://api.openai.com/v1`), operant can
point at an agentgateway instance **as a config change rather than a rewrite.**

**The compatibility question is now answered, and the answer is yes.** Verified
in agentgateway's own source at `c9573ed1`, not inferred:

- **Streaming SSE** is first-class, not a shim — `ChatCompletionChunk` and
  `StreamResponseDelta` with `content` / `tool_calls` / `reasoning_content`,
  and `data: [DONE]` is appended on clean body close rather than left to the
  upstream.
- **Tool calling works in both directions, cross-provider.** The golden fixtures
  are byte-exact round-trip tests, and `multi-turn-tools.json` contains exactly
  the shape operant emits: `user → assistant(tool_calls) → tool(tool_call_id) →
  assistant(tool_calls) → tool(tool_call_id)`, with snapshots asserting
  conversion to Gemini/Vertex and Bedrock.
- **Streaming tool calls with incremental argument fragments** — the same
  `index → id` plus incremental-merge pattern operant already implements in
  `crates/operant-core/src/agent/clients/openai.rs` (`StreamToolCallIndex`,
  `merge_stream_tool_call`). agentgateway emits the OpenAI-standard form, so
  that client code needs no change.
- There is a documented client-integration catalog for exactly this shape
  (Codex, Cursor, Copilot, Claude Code) pointed at the OpenAI-compatible
  endpoint — a tested configuration, not a stretch.

So the spike will work. The question is whether it is *worth* running, and the
answer is a qualified no:

| | |
|---|---|
| **Buys** | 21 providers from config instead of a Rust adapter each; virtual-model aliasing with weighted/failover/CEL routing; per-key budgets with token-bucket limiting; guardrails (regex, PII, OpenAI moderation, Bedrock Guardrails, Model Armor); OTel traces |
| **Does not buy** | anything about operant's own inference code — that stays, and it already exists: a provider trait, an OpenAI client with streaming tool-call merge, an Anthropic client with `cache_control`, and `FallbackModelClient` |
| **Costs — deployment shape** | the mature product is aimed at **fleets**. Virtual keys issued to "users or applications", RBAC with a CEL policy engine, multi-tenant cost analytics, ACME, a Postgres option, and a 92 MB binary carrying an embedded React UI and two allocators. operant is a single-developer local/VPS tool; this is org-grade machinery to obtain the ~20% that fits one user (failover, guardrails, one dashboard), with the other 80% as permanent surface area. **The mismatch is in the problem being solved, not the protocol.** |
| **Costs — supply** | **bus factor of one** on the core proxy at Solo.io. Not visible in the star count. |
| **Costs — churn** | the current v1.6 alpha shipped **two wire-format-breaking changes two days apart**, one of which (`(breaking) llm: default messages -> responses`) changes the default upstream format for Anthropic Messages — *precisely* the path operant's Anthropic client uses. Both are un-tagged; pinning to v1.5.0 avoids them, tracking head absorbs them. |
| **Costs — translation** | behind the gateway, agentgateway re-encodes operant's request into the upstream's format. Its fixtures show that round-trips correctly, but that layer is still moving, and it becomes a dependency on someone else's release cadence. |

**Recommendation: decline the gateway-as-control-plane migration; do not spend
the spike either, unless a concrete trigger appears.** The trigger would be
"we need more providers" or "we need spend visibility across a team" — either
makes it a good answer. "Our inference path should be better" is a different
layer, and the honest answer is that the premise was wrong.

**The direct competitor is LiteLLM Proxy**, not agentgateway — older, larger
provider catalogue, the de-facto-standard config surface, the same
OpenAI-compatible endpoint. agentgateway's documented edge is Rust (single
static binary, no Python runtime) and a real streaming-guardrail path. Its
published head-to-head benchmark against LiteLLM is vendor marketing; treat the
numbers accordingly. SaaS options (Portkey, Cloudflare AI Gateway, Helicone,
Braintrust) are the wrong category for a local-first binary.

**Standing recommendation, unchanged: keep operant's own abstraction.** It is
already written, tested, released, and is 80% of the value for a single-user
deployment. The remaining 20% is ~2 config files and zero dependencies — until
the day it is not, and then this section is the spike to run.

---

## 5. What I did not do, and why

- **Did not touch `sourcehound` or `memory-wire`.** They are not integrated,
  and §3.3 is a decision, not an implementation.
- **Did not start the channel refactor (item 4).** It is a real behavioural
  change with a silent-failure mode, and it should be scheduled deliberately.
- **Did not wire `semantic_compaction_cutoff`.** iter-406 measured hybrid
  retrieval as *worse* than plain compaction (R@3 2/5 → 1/5). Wiring an unproven
  primitive is how a knob comes to look like protection while providing none.
- **Did not resolve the appearance questions** (the ~188 `DarkGray`/`Black`
  sites, the four `bridge_state` badges). All are now measured to be *blocked
  structurally* rather than open: 12 of 13 palette roles vary across the 8
  themes, and the one invariant role is a background role already rejected for
  role mismatch — so no value-preserving migration exists. Those need a product
  decision, not engineering.
- **Did not tag v0.2.1.** It is item 1, and it should ship on a green CI and a
  green Build, which is a decision about *when* to cut a release.

## 6. Corrections to claims made earlier in this work

Recorded because each was wrong in a way that would have propagated:

| claim | correction |
|---|---|
| "the 1,043-field `operant-config` schema is dead" | **False.** The *loader* is dead; the schema *type* is alive — `operant-core` depends on it non-optionally and `operant-tools/src/web_search_tool.rs:106` deserialises it in production. |
| "the changelog is 61 iterations stale" | **Imprecise.** `[Unreleased]` is narrative, not per-iteration. It last absorbed work at iter-414; iter-415→479 was never added. |
| "`install.sh` installs to `~/.cargo/bin`" | **False.** Line 72 is a `sudo cp` to `/usr/local/bin`; line 78 is an *additional* copy, with a comment explaining PATH shadowing. |
| "`igs` is auto-downloaded when missing" | **False.** No download code exists for `igs`; the auto-downloads are Obscura and Lightpanda. |
| "13 open BUGS.md entries" | **Wrong.** 4 are genuinely open. Six headlines claimed open work that was already done. |
| "agentgateway is Kubernetes-only" | **Wrong.** A Standalone Quickstart is documented alongside the Kubernetes one. |

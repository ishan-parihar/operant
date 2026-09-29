# Integration plan — sourcehound + memory-wire as unmodified vendored dependencies

Written 2026-09-29. Supersedes nothing in `IMPLEMENTATION-PLAN.md`; that document
remains the state-of-the-project report. This one covers the two integrations
and the agentgateway question.

**Constraint, taken as given and treated as load-bearing: we may not modify
either project.** Every hook therefore lives on our side of the seam. This plan
is organised around that, not around it.

---

## 0. A correction before anything else

I previously reported that **sourcehound has no git remote** and therefore
cannot be cargo-depended on. **That was wrong.** I repeated it from
`docs/R40-12-13-WIRE-OR-RETIRE-REVIEW.md` without checking. Measured:

| | sourcehound | memory-wire |
|---|---|---|
| remote | `github.com/ishan-parihar/sourcehound` | `github.com/ishan-parihar/memory-wire` |
| also | `gitlab.com/ishan-parihar/igs-rust` (duplicate-remote pattern, same as operant) | — |
| branch / head | `master` @ `0712777` | `main` @ `eec026f` |
| version | `1.6.0` | `0.4.0` |
| licence | MIT AND Apache-2.0 | MIT OR Apache-2.0 |
| size | 76 files, **52,093** lines | 18 files, **16,985** lines |
| targets | `bin: sourcehound`, `lib: sourcehound_mcp` | `bin: memory-wire`, `lib: memory_wire` |
| working tree | **1 dirty file** | clean |

Both are permissively licensed and both are dual bin+lib, so the vendoring
requirement is satisfiable. Note sourcehound's `repository` field is correct in
its manifest; memory-wire has **no `repository` field at all**, which is worth
adding on our side of a `[patch]`-free dependency graph, not theirs.

**Both are axum HTTP servers with MCP front-ends.** sourcehound has
`src/server.rs` and `src/browser/mod.rs`; memory-wire depends on `axum`,
`rusqlite`, `tokio`, `reqwest`.

---

## 1. The vendoring mechanism

The requirement is "directly linked as GitHub vendors, each rebuild updates
their source, and we must not modify them." That is exactly what a **cargo git
dependency** is, and it is the right tool — no vendoring copy, no submodule, no
fork, and `cargo update` moves to their HEAD.

```toml
# operant/Cargo.toml — workspace dependencies
memory-wire = { git = "https://github.com/ishan-parihar/memory-wire" }
sourcehound = { git = "https://github.com/ishan-parihar/sourcehound" }
```

Three properties worth being explicit about, because they are what make this
satisfy "must not modify":

1. **We never write to their tree.** There is no fork, no `[patch]` pointing at
   a local copy, no submodule. A `git` dependency is read-only by construction.
2. **A rebuild updates to their HEAD** via `cargo update -p memory-wire`. The
   pin lives in `Cargo.lock` and is reviewable in a normal diff.
3. **Our hooks cannot regress their behaviour**, because there is no behaviour
   of theirs in our process to regress — see §2 for why that is the plan's
   central design choice.

### 1.1 The one thing that will bite

`git` dependencies build **their library into our binary.** For memory-wire
that is 16,985 lines. For sourcehound it is **52,093 lines plus a browser
subsystem** — and its `lib` target is a 23-line barrel over
`config`/`server`/`tools`/`error`/`license`/`http`. So the sourcehound *library*
target is not a usable API; it is a server's guts with a thin lid.

**Therefore: git-dependency memory-wire's `lib` only if we want in-process
calls, and run sourcehound as a subprocess.** See §3.1 — this is a real fork in
the road and it is the first decision I need from you.

---

## 2. Why both integrations are HTTP-from-the-outside, not library calls

Under "must not modify", the temptation is to link their libs and call them.
For memory-wire that is defensible (17k lines, a genuinely useful API). For
sourcehound it is not.

But there is a stronger argument for HTTP on **both**, and it is the one that
matches your framing:

> **operant already has both seams, and they were built for exactly this.**

- **Memory:** `MemoryProvider` (`crates/operant-core/src/memory_provider.rs`) —
  a trait with **19 methods of which only 3 are required** and 16 carry working
  defaults. It was designed as a plug-in point and agentmemory already
  implements it. A `MemoryWireProvider` is a new `impl`, not a change to
  anything existing.
- **Web tools:** a provider chain (`crates/operant-core/src/tools/web_tools.rs`)
  with a failover list, plus a `BrowserProvider` trait with 7 implementations
  and a `build_browser_provider` factory.
- **MCP:** a **deferred** server-registration path. The agentmemory 53-tool MCP
  server is injected as a config entry and only spawns if the user connects it.

So the integration surface is: **one new trait impl, one new provider arm, one
config entry.** No changes to their code, and no changes to ours beyond
additions at seams that already exist.

---

## 3. memory-wire — the honest mapping

### 3.1 Integration mode: in-process library (decided at iter-485)

I originally recommended a subprocess. **That recommendation is superseded.**
memory-wire is under active development and being upgraded specifically for
operant integration, and you have decided to bake it into the binary. That is
coherent and I am adopting it — a cargo git dependency tracks your *branch*
rather than a released version, which is exactly the right coupling while the
crate is moving.

| | **A. Subprocess + HTTP** | **B. cargo git-dep — ADOPTED** |
|---|---|---|
| our code | `MemoryWireProvider` posting to `127.0.0.1:<port>` | `MemoryWireProvider` calling `memory_wire::api::*` directly |
| binary cost | +1 process | **+16,985 lines** in the operant release artifact |
| reach | HTTP routes only | **everything**, incl. `recall_with_weights`, `retain_tagged`, `retain_doc` |
| failure mode | process death → memory miss | **their panic kills the agent mid-turn** |
| isolation | clean | shared process and allocator |
| version coupling | wire contract only | must agree on versions with a 0.x crate |

What you gain, concretely: `recall_with_weights` and `retain_tagged`/
`retain_doc` are reachable in-process, which is a real capability the HTTP
surface does not expose. That is worth more than a localhost hop.

**Three consequences to own, since this is a 0.x crate in our release path:**

1. **Every memory-wire change is an operant rebuild.** `cargo update` moves us
   to your HEAD, so a breaking change in a 0.x crate is *our* breaking change.
   Pin with `rev = "..."` if a given moment of your branch is not release-ready.
2. **A panic in their code is fatal to an in-flight turn.** Subprocess isolation
   degrades to a memory miss; in-process does not. The cheapest mitigation is a
   `catch_unwind` at the `MemoryProvider` boundary, converting a panic into a
   degraded-memory state rather than a dead agent. Worth doing regardless of
   mode.
3. **Version drift is our problem at build time.** A 0.x crate may bump its
   minor version on any commit, so `Cargo.lock` will churn. That is expected, not
   a defect, but it means the lock diff needs review discipline.

Note that sourcehound stays a **subprocess** — 52,093 lines with a 23-line
barrel lib over a server and a browser subsystem is not a library worth
compiling into a release artifact (§4.2).

### 3.2 The mapping, trait method by trait method

This is the part worth reading carefully, because the routes do not line up with
agentmemory's and I would rather show that than round it off.

| `MemoryProvider` method | agentmemory does | memory-wire equivalent | verdict |
|---|---|---|---|
| `prefetch(query)` | `POST /agentmemory/smart-search` | `POST /banks/{bank}/recall` | **clean** |
| `on_memory_write(action, target, content)` | `POST /agentmemory/remember` | `POST /banks/{bank}/retain` | **clean** |
| `check_health()` | `GET /agentmemory/health` | `GET /banks/{bank}/stats` | **clean** (different name, same role) |
| `system_prompt_block()` | `POST /agentmemory/context` | `POST /banks/{bank}/recall` with a standing query | **substitute** — see below |
| `on_pre_compress()` | `POST /agentmemory/context` | same as above | **substitute** |
| `sync_turn(user, assistant)` | `POST /agentmemory/observe` | `POST /banks/{bank}/retain` — **called by us, not by them** | **workable, changes volume** |
| `initialize(session_id)` | `POST /agentmemory/session/start` | *none* | **not needed** — operant already owns session identity |
| `on_session_end()` | `POST /agentmemory/session/end` | *none* | **not needed** — same reason |
| `ensure_server()` | spawn `npx …@0.9.29` | spawn `memory-wire` binary, or defer to the user's service | **clean** |

Three of these deserve explanation rather than a tick.

**`sync_turn` is workable, and this is the key realisation.** memory-wire has
**no turn-ingestion endpoint and no public turn-ingestion function** — I
checked: `capture` exports only `redact_pii` and `hash_content`, and no route
matches `session|turn|observe|hook|capture`. Its design is explicitly opt-in.

That sounds disqualifying and is not, because **`sync_turn` is called by our own
agent loop**, not by their server. We call `retain` with the turn ourselves. We
already truncate to 500/2000 chars on our side (`agent_memory.rs:587-588`).
So automatic per-turn capture is **preserved without touching memory-wire** —
we simply choose to retain every turn.

The cost is a **policy** question, not a capability gap: every turn becomes a
memory, so the bank grows fast and recall degrades unless `/reflect`
consolidation runs. agentmemory hides this server-side. This needs a number from
you, not a line of code from them.

**`system_prompt_block` / `on_pre_compress` are substitutes, not equivalents.**
agentmemory's `/context` is *session-scoped* — it takes `sessionId` and returns
assembled context with no query. memory-wire has no context-assembly endpoint;
`recall` takes a query. The honest options are (a) `recall` with a standing
project-goal query, or (b) leave these two returning `""` and rely on `prefetch`,
which is the trait's own default. **(b) is defensible and I would start there** —
it is the default behaviour, so it degrades to what operant does today.

**`initialize` / `on_session_end` having no equivalent is correct, not a gap.**
Those exist to tell agentmemory about session boundaries. operant already tracks
session identity itself (it is what writes the session files), so re-telling
another service is redundant work. Both default to no-ops on the trait.

### 3.3 The real risk, stated plainly

**This is not a drop-in replacement, and I would be misleading you to call it
one.** It is a genuinely different memory design:

| | agentmemory | memory-wire |
|---|---|---|
| capture | automatic, server-side, every turn | explicit, caller-driven |
| unit | session | bank |
| consolidation | server-side | `/reflect`, on demand |
| maturity in operant | 1965 tests behind it | none yet |

Adopting it means the memory behaviour changes for users, and the only way to
know whether recall got *better* is to measure it. **Recommendation: build the
`MemoryWireProvider` behind the existing `provider = "..."` config switch,
default it to `agentmemory`, and measure before defaulting it to memory-wire.**
That is the same discipline that stopped `semantic_compaction_cutoff` being
wired on a feature that measured negative (R@3 2/5 → 1/5), and it costs one
config flag.

---

## 4. sourcehound — replacing IGS

### 4.1 What actually changes

Measured today, IGS is integrated in **three** places, and one of my earlier
claims about it was wrong:

- **There is no `igs` auto-download.** Operant never fetches the `igs` binary;
  `IGS_INSTALL_HINT` is a `curl` string inside an error message. The only
  auto-downloads are for **Obscura** and **Lightpanda**.
- **`browser.provider` defaults to `"obscura"`**, not `"igs"`
  (`config.rs:1181`), and the `_ =>` catch-all in `build_browser_provider` is
  **Lightpanda** — a typo'd provider name silently becomes Lightpanda.
- The 53-tool agentmemory MCP server is **lazily** registered, not eagerly spawned.

| current IGS surface | file | replacement approach |
|---|---|---|
| `web_scrape` / `web_extract` / `web_crawl` subprocess tools | `operant-core/src/tools/igs.rs` | new `OperantTool` impls calling sourcehound's HTTP surface |
| `IgsBrowserProvider` (CDP) | `browser_provider.rs` + `tools/igs.rs` | new `BrowserProvider` impl |
| `IgsSearchProvider` (fallback chain arm) | `tools/web_providers/igs.rs` | new `WebSearchProvider` arm |
| `igs` binary resolution + `IGS_INSTALL_HINT` | `igs.rs:36,33` | sourcehound binary resolution |
| deferred MCP entry | `config.rs:1611` | sourcehound MCP entry |

That is **four additions and zero modifications** — the shape your constraint
demands.

### 4.2 The one design decision

sourcehound's `lib` is a 23-line barrel over a **52,093-line** server with a
browser subsystem. **Do not git-depend on it.** Run it as a subprocess and
speak HTTP/MCP, exactly as IGS is driven today. This keeps the release binary
the same size and keeps their blast radius outside our process.

### 4.3 A trap to record

`build_browser_provider`'s `_ =>` arm returns **LightpandaProvider**. If a new
provider name is added without an explicit arm, it silently becomes Lightpanda
and *tests still pass*. Any sourcehound provider must be added as an **explicit
match arm**, and the existing test that pins the catch-all behaviour
(`browser_provider.rs:974-977`) must keep passing — which it will, because we are
adding an arm rather than changing the fallback.

---

## 5. agentgateway — is it worth it, and would it beat our fallback?

### 5.1 First, what our fallback actually is

I measured this rather than taking the question's framing, and it is stronger
than "the current fallback implementation" suggests.
`crates/operant-core/src/agent/fallback.rs`:

- **A typed 3-way verdict**, not a message-string sniff: `RetrySame { after:
  Option<Duration> }` / `FallBack { reason: FailoverReason }` / `GiveUp`.
- Classified **once** by `Failover::classify` and shared by `chat` and
  `chat_streaming` — the source comment says this is so "the two paths cannot
  drift apart and neither re-derives the reason from a message string."
- **Rate-limit-aware in a way that matters**: honours `Retry-After`, knows
  "rate-limit buckets are per model" so a *different model on the same provider*
  is a valid substitute, and **prefers a cheaper sibling when it has a known
  price**. That is cost-aware failover.
- **Two independent chains** — `advance_model_chain()` and `switch_provider()` —
  plus a `ProviderRegistry` for credential handling.
- **18 tests**, including `rate_limit_exhaustion_triggers_provider_switch`,
  `non_retryable_error_mid_chain_stops_fallback`,
  `exhausted_provider_chain_returns_original_error`.

**agentgateway would not be better at this.** It does not know operant's
per-model rate-limit bucket semantics, and it would not prefer a cheaper sibling
on a `Retry-After` — that is operant-specific knowledge living in the right
place.

### 5.2 So where would it actually be better?

| capability | ours | agentgateway |
|---|---|---|
| sequential failover on error | ✅ typed, cost-aware, 18 tests | ✅ config-driven |
| **spend / dollar budgets** | ✅ **already fully wired** — see below | ✅ per-key, token-bucket |
| **rate limiting** | ✅ `operant-core/src/rate_limiter.rs`, per-model token accounting | ✅ |
| **weighted routing** (70/30 split) | ❌ strictly ordered | ✅ |
| **cross-provider load balancing** (proactive, not just on failure) | ❌ | ✅ |
| **guardrails** (PII, moderation, Bedrock Guardrails) | ❌ | ✅ |
| **cost attribution per key/team** | ❌ we have per-session/daily/monthly, not per-key/team | ✅ |
| provider count | a Rust adapter each | 21 from config |
| failure domain | one process | **two** |

### 5.2.1 A correction I had to make to this table

I first wrote that operant "has iteration budgets, not money budgets." **That was
wrong, and I only caught it by grepping instead of trusting the framing of the
question.** Measured:

- `operant-config/src/cost/tracker.rs:51` — `check_budget(estimated_cost_usd)`
  returning `BudgetCheck`, enforcing **projected daily and monthly** ceilings
- `operant-runtime/src/agent/cost.rs:176` — `check_tool_loop_budget()`, a
  pre-flight gate reading a task-local context
- `operant-runtime/src/agent/loop_/run.rs:422-425` — the context is built from
  `config.combined_pricing()` and **scoped at `:538` and `:882`**
- `operant-runtime/src/agent/loop_/tool_loop.rs:282` — **the gate is called
  inside the tool loop**, which is where it belongs
- also scoped in `operant-gateway/src/lib.rs:1036` and
  `operant-channels/src/orchestrator/dispatch.rs:678`
- `operant-core/src/rate_limiter.rs` already provides rate limiting

So budget enforcement is **implemented, wired, and tested** — including a test
for the un-scoped (`None`) case at `loop_/tests.rs:4569`. I checked for the
zero-caller pattern that bit this project three times this session
(`safe_compaction_cutoff`, `semantic_compaction_cutoff`) and it does **not**
apply here.

**The one real caveat, and it is a data gap not a code gap.** `check_budget(0.0)`
is deliberate — the cost of an upcoming call is unknowable — so the gate refuses
when spend is *already* over the ceiling rather than pre-empting the call that
would cross it. And `warn_once_missing_pricing` (`cost.rs:155-163`) says
outright that with no pricing entry "budget enforcement is inert for this
model". So **enforcement is exactly as good as the pricing table.** If the
models you actually use lack `[cost.prices."{provider}/{model}"]` entries, the
budget you have configured is not being enforced. That is worth checking before
concluding you need any external spend control.

### 5.3 The verdict

**On failover, retry and routing: no, it would not be better — ours is stronger
where it counts.** The case for it is entirely in the *right-hand column's
missing rows*: weighted routing, proactive load balancing, spend budgets, rate
limiting, guardrails, cost attribution.

**But for a single-developer CLI, those are largely not your problem.** A solo
user does not need per-team cost attribution, RBAC with a CEL policy engine, or
ACME. And the price is specific and measured:

- a **92 MB** always-on second process (carrying an embedded React UI and two
  allocators), versus operant as one binary you run;
- a **second secret store** — provider keys move out of your config into the
  gateway's;
- **bus factor of one** on the core proxy at Solo.io;
- **protocol-translation risk** — the current v1.6 alpha shipped two
  wire-format-breaking changes two days apart, one changing the default upstream
  format for Anthropic Messages, which is precisely the path your Anthropic
  client uses. You would depend on a translation layer you do not control.
- It models a **fleet**: virtual keys issued to "users or applications",
  multi-tenant analytics, a Postgres option. You would run org-grade machinery
  to obtain the ~20% that fits one user.

### 5.4 The smaller ask, which is the real recommendation

If the actual desire behind the question is *weighted routing* or *guardrails* —
the two rows most likely to matter to you — **weighted routing is far cheaper
than a gateway**, because you already have the seam:

- **Weighted routing** is a change to `model_chain()`, which is a plain
  `Vec<String>` built in four lines (`fallback.rs:347-354`). Carrying a weight
  per model and selecting on it is tens of lines in a file that already has a
  `ProviderRegistry` and 18 passing failover tests.
- **Guardrails** are the one capability here that is genuinely absent and
  genuinely non-trivial — regex/PII filtering, moderation-model checks, and
  streaming interception are not a config change. But they are also the one you
  are least likely to need for a single-developer CLI.

**Not spend control — you already have it.** A daily and monthly dollar ceiling
is implemented, wired into the tool loop, and tested. The only thing to check is
whether `[cost.prices."{provider}/{model}"]` is populated for the models you use,
because without a price the enforcement is inert by design and the code warns
you when that happens.

**Recommendation: decline agentgateway.** Its remaining advantages over what you
own are weighted routing, proactive load balancing, guardrails, per-key cost
attribution, and provider count — and the first two are small changes to code
you already have. Say which one you actually want and I will scope it honestly.

---

## 6. What I need from you

Four decisions, all of which change the work rather than decorate it:

1. **memory-wire: in-process library, per your decision at iter-485.** §3.1 now
   records what that costs and what to do about it. The one thing I would add
   regardless of mode: a `catch_unwind` at the `MemoryProvider` boundary, so a
   panic in a 0.x crate degrades to a memory miss instead of killing a turn.
2. **The `sync_turn` write policy.** If we retain every turn, the bank grows
   fast. Options: retain every turn, retain every Nth, retain only turns that
   produced a tool call, or run `/reflect` on a size trigger. This is a number
   you pick, not a default I should invent.
3. **`system_prompt_block` / `on_pre_compress`:** substitute a standing-query
   `recall`, or leave both returning `""` and rely on `prefetch`? I recommend
   leaving them — it is the trait default, so it degrades to today's behaviour.
4. **Is weighted routing the real need behind the agentgateway question?** If
   yes, it is a tens-of-lines change to `model_chain()`. Guardrails are the one
   genuinely absent capability and genuinely non-trivial; tell me if that is the
   one you want. **Spend control is not on this list because you already have
   it** — see §5.2.1.

Also worth confirming: sourcehound's working tree has **1 dirty file** and its
branch is `master` while memory-wire is on `main`. A git dependency tracks the
default branch, so that is fine — but if `master` is not the branch you intend
to ship from, pin `branch = "..."` or `rev = "..."` rather than taking HEAD.

---

## 7. Sequence, if you want it done

1. `MemoryWireProvider` behind `memory.provider = "memory-wire"`, defaulting to
   `agentmemory`. Subprocess. Wire a `check_health` and `prefetch` test first.
2. MCP entries for both, as **deferred** servers, so the tool surfaces appear
   without a new spawn path.
3. `sourcehound` provider arm as an **explicit** match in
   `build_browser_provider` + the web-provider chain. Reject any implementation
   that does not add an explicit arm (§4.3).
4. Measure recall on both backends before defaulting either. Do not skip this —
   it is the step whose absence let `semantic_compaction_cutoff` look like
   protection for ~50 iterations while protecting nothing.

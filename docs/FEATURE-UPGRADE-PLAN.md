# Feature Upgrade Plan — oh-my-pi, hermes-agent, zeroclaw → operant

Status: audit complete, no code written. This doc is the architecture answer to
"the four names are just outlines — how should we actually implement them."

Sources (all read at source, working trees untouched):
- `parent-projects/oh-my-pi` @ `d1932a6ff8` (clean, current) — two audits:
  pre-execution + advisory, and self-reflection + offline consolidation.
- `parent-projects/hermes-agent` @ remote HEAD (21,698 commits ahead of local;
  read via git plumbing) — see `docs/FEATURE-GAP.md` §2 for the full findings.
- `parent-projects/zeroclaw` @ remote HEAD (2,205 commits ahead of local;
  read via git plumbing) — see `docs/FEATURE-GAP.md` §3.
- `parent-projects/aft` (`cortexkit/aft`) — AFT integration recommendation, §7.

Companion docs: `docs/FEATURE-GAP.md` (presence/absence + zeroclaw Rust
patterns), `docs/INTEGRATION-PLAN.md` (memory-wire in-process / sourcehound
subprocess — decided, plan updated).

## 0. The one-line version

No reference project has automatic pre-execution reconnaissance — the gap is
real and confirmed three times over, and oh-my-pi's `prewalk` is a
**name collision** (a model-cost handoff, not recon). The two mechanisms worth
porting whole are oh-my-pi's **advisor** (peer-shadow critic, interrupt power,
no veto) and its **retry-with-feedback** injection shape (deterministic, free).
Reflection and dreaming exist in oh-my-pi but each ships with a real defect to
fix while porting, and **nobody — including oh-my-pi — evaluates whether a
written memory was ever useful**. That usefulness signal is the piece we have
to design ourselves; without it every consolidation loop is a write-only
ratchet.

## 1. Recon ("prewalk") — absent everywhere; design it, don't port it

**Finding.** Three independent searches (hermes, zeroclaw, oh-my-pi) found no
automatic task-derived recon phase. oh-my-pi's `prewalk` is the trap:
`PrewalkCoordinator` (`packages/coding-agent/src/session/prewalk.ts:100`,
WIRED at `agent-session.ts:1724`) arms a one-way model downgrade — inject a
plan nudge, wait for a todo list, then on the first `edit`/`write`
(`isPrewalkImplementationAction`, `prewalk.ts:51-62`) swap to a cheap model
(`setModelTemporary`, `prewalk.ts:209`). Its plan prompt says the opposite of
recon: *"STOP: In NEXT reply, before further exploration, write complete
plan"* (`prompts/system/prewalk-plan.md:1`). Deterministic, zero extra LLM
calls, default **off** (`session/settings.ts:74-77`). The recon-adjacent
pieces are all non-automatic: the `scout` read-only subagent is
model-invoked only (`prompts/agents/scout.md`, spawn gate at
`task/spawn-policy.ts:61-71`); AGENTS.md/CLAUDE.md loading is static
session-start preamble (`discovery/agents-md.ts:21-23`); the only task-aware
pre-execution hints are one-line spawn advisories (`task/index.ts:426,446`).

**Architecture for operant.** Do not call it `prewalk` — the name now means a
cost handoff in oh-my-pi and will mislead every future reader. Call it
`recon` or `prepass`. Shape: an automatic harness-driven phase before the
first tool call of a task, not a model decision. It uses operant's **own
existing** AFT surface (outline/zoom/search/callgraph — 20 tools registered,
`aft_enabled` default true) with a read-only grant, bounded by a file cap and
a token cap, result injected as a developer/context message the acting model
can see but did not ask for. Config-gated with a skip flag. The piece
oh-my-pi never built is the trigger — ours fires on task start, unconditionally,
instead of waiting for the model to feel curious.

## 2. Advisor — port oh-my-pi's peer-shadow critic, minus nothing, plus a budget

**Finding.** `packages/coding-agent/src/advisor/` — 13 files, 3,826 lines, a
genuine second agent, not MoA ensembling and not a permission prompt.
`SessionAdvisors` (`session/session-advisors.ts:452`), `AdvisorRuntime`
(`advisor/runtime.ts:253`), single admission authority `AdviseTool`
(`advisor/advise-tool.ts:178`). Trigger WIRED at `agent-session.ts:1726`
(`onPrimaryTurnEnd`): every completed primary turn, incremental delta,
coalesced ×3 (`runtime.ts:212`). It is a **different model**
(`session-advisors.ts:920-950`) with its own telemetry identity
(`advise-tool.ts:137-143`), read-only tools (`read/grep/glob`,
`advise-tool.ts:152`, extensible via `WATCHDOG.yml`), and the project
conventions rendered into its own system prompt "so it can hold the driving
agent to them" (`advisor/watchdog.ts:24-32`). Delivery is three channels —
`aside` (queued), `steer` (interrupting), `preserve` (visible card)
(`advise-tool.ts:108-124`) — and steer **aborts in-flight interruptible
tools** (`agent-loop.ts:174` `TOOL_INTERRUPT_ABORT_REASON`, 250 ms poll at
`:180`), firing even after a terminal answer (`advise-tool.ts:90-94`). There
is deliberately **no veto**: `"weigh, don't blindly obey"` (`:38`), and the
primary's prompt never mentions advisories (`:35-37`). Spam control is what
makes it usable: dedupe + rank-aware escalation + per-update budget
(`emission-guard.ts`, `advise-tool.ts:161-176`), deferred flush at the turn
boundary (`:229-234,266-291`), post-interrupt immunity (default 3 turns,
`advisor/settings.ts:40-61`, blockers exempt), auto-resume suppression
(`:95-102`). Config: default **false** for interactive, **on** for `rpc`/`acp`
protocol hosts (`advisor/settings.ts:10-22`). Cost: a full second model loop
per turn, plus an optional up-to-30 s backlog stall (`:24-38`).

**Architecture for operant.** Port the shape onto our turn loop, reusing our
own registry: post-turn hook → message delta → advisor agent (cheap model,
read-only file tools from our existing tool set) → severity-graded delivery
(queue vs. interrupt-the-turn) → emission guard with per-update budget →
default off for `chat`, on for gateway/protocol hosts (mirroring their
`protocolDefault` split, which is the correct default — agent-facing mode is
where an unobserved critic pays for itself). Keep interrupt power, do not add
veto — oh-my-pi's soft-framing choice is load-bearing: a veto turns every
advisor false positive into a stuck turn, while an interrupt turns it into a
reconsidered one. The one thing to carry over verbatim is the "primary never
mentioned" rule: no prompt changes on the acting side, so the advisor can be
disabled with zero behavioral residue.

## 3. Reflection (post-turn self-capture) — port oh-my-pi's Auto-Learn, fixing its two defects

**Finding.** Exactly one real mechanism: Auto-Learn
(`autolearn/controller.ts:75-138`). Trigger is a tool-call threshold on a
completed turn (default ≥5, `autolearn/settings.ts:38`) with four suppressors
— aborted turn (`:104-112`), plan-mode review (`:118`), goal mode (`:123`),
and the master switch `autoContinue` (`:130-131`, default **false**,
`settings.ts:13`). The fork is well built: separate `Agent` (`sdk.ts:1507-1577`),
full structural message copy not a summary (`:1518-1526`), verbatim system
prompt (`:1529`), fresh session id + prompt-cache key + empty provider state
(`:1536-1538`, `:1517`), deliberately **no fallback resolver** so a
hallucinated tool stays not-found (`:5059-5066`). Writes go through the
`learn` tool (`tools/learn.ts:79-179`) to the memory backend (typed
`rememberScoped`: source/importance/veracity/type, `:97-115`; failed writes
throw rather than lie, `:116-119`) and to `SKILL.md` files under
`~/.omp/agent/managed-skills/` whose store is the best-defended code in the
subsystem — name regex, symlink/hardlink/`O_NOFOLLOW`/re-stat-on-handle,
injection-neutralized descriptions on write *and* read, per-name mutation
serialization, priority 5 below every authored skill (`managed-skills.ts`).
Rate limiting is host-side threshold plus model-side "capture sparingly"
gating plus at-most-one capture with coalescing (`:133-152`). Two real
defects: a running capture is **not aborted by a new user prompt** (the
`prompt()` path at `agent-session.ts:6818-6848` never touches the abort
controller; abort sites are only cancel/branch/btw/dispose), so it competes
for rate limit mid-turn; and the capture is **entirely invisible** —
`display: false`, no session events, and project-scope memory writes carry
`approval: "read"` (`learn.ts:52-57`). Weaker neighbours: mnemopi
auto-retain (transcript capture every 4 turns, no judgement —
`mnemopi/state.ts:610-627`); the `reflect` tool is a read tool, not
self-appraisal (`tools/memory-reflect.ts:19-96`); the `judgment/` directory
is a hardened classifier utility (thinking level, staging, eval), not
critique. And the gap that matters: **no usefulness evaluation exists
anywhere** — zero hits for any utility/feedback signal; the nearest
instrument (`recall-diagnostics.ts`) measures tier hit rates, not whether
recalled content helped.

**Architecture for operant.** Our `sync_turn` seam (auto memory write-back
after each turn) is the natural host — it already exists, so this is a
threshold + fork upgrade, not a new seam. Keep: the fork isolation rules
(separate session/cache key, no fallback resolver, verbatim prompt), the
typed write envelope, the managed-skill hardening (port that file's
discipline wholesale — it is the reference implementation for
machine-written system-prompt content). Fix while porting: abort-on-new-prompt
(one line at the submission path), and a visible "auto-learn wrote N
memories / M skills" event per capture (the invisibility is what makes the
current design unaccountable). Design ourselves: the usefulness signal —
score recalled memories by whether the turn that consumed them succeeded,
feed it back into the admission decision. Without that, capture is a
write-only ratchet; every reference project ships exactly that ratchet, so
this is where we get to be better rather than merely at parity.

## 4. Dreaming (offline consolidation) — port the sharpshooter shape, skip the distributed substrate

**Finding.** Three subsystems at three levels of "offline", only one truly
periodic. **Sharpshooter** is the real thing: 60 s in-process interval timer
(`sharpshooter/scheduler.ts:20-56`, ref-counted per bank dir, immediate first
tick, failures debug-swallowed), per-prompt async extraction on
`message_start` (`backend.ts:93-99`) with a `smol`/`Low` model and bounded
context (`extract.ts:91-98,154-171,229-230`), file-per-delta queue that is
lock-free and crash-safe by construction (`queue.ts:1-10`), and a
file-locked consolidator (`consolidate.ts:83-204`) rewriting three memory
files under host-enforced validation (exactly-one-tool-call, no duplicate
filenames, secret redaction, 120-line ceiling, anti-wipe guard, atomic
temp+rename — `:233-295`). Its two best ideas are both portable prompt+code
pairs: the **friction admission law** ("memory is earned by friction, not by
decision-existence… When in doubt, leave it out",
`sharpshooter-consolidate-system.md:5-13`, plus "ceilings, not targets",
`:40`) and **host-verified evidence** (the `evidence` field must be an exact
substring of the user prompt, checked in host code — `extract.ts:266` —
assistant text can never become evidence). Surface: `/memory
queue/stats/diagnose/sync` (`backend.ts:172-233`). Its defect: the tick has
**no foreground-busy check** (nothing in `sharpshooter/` references
streaming state), so a consolidation call can land mid-turn on the same
credentials. **Mnemopi `sleep`** is consolidation without a timer —
algorithmic AAAK summarizer (`llm_used: 0`), age-tier degrade with vector
invalidation, Bayesian veracity conflict resolution (`veracity-consolidation.ts`)
whose confidence only ever increases (no outcome feedback — a write-once
ratchet, do not copy the weighting without the missing signal); triggers are
session start and explicit enqueue, never idle. The **`local` pipeline** is
the deepest transcript processor (startup-triggered, ≥12 h-idle rollouts,
2-phase LLM, genuinely aggressive pruning including skill-dir deletion —
`memories/index.ts:988-1041`) wrapped in ~1,400 lines of SQLite job-queue
leases and heartbeats that buy cross-process safety for a pipeline with one
process. `/gc` is manual storage maintenance, not a curator; there is no
cron/daemon anywhere (46 `setInterval` sites checked, only sharpshooter's and
one lease heartbeat do semantic work).

**Architecture for operant.** Sharpshooter's shape, minus its defects, minus
the distributed substrate: per-prompt lightweight extract with
host-pinned evidence → append-only delta store → periodic consolidate while
the process runs, with a **defer-while-streaming guard** and a shared
low-priority lane (the two fixes oh-my-pi needs). Admission by the friction
law; output under host-side ceilings (line cap, anti-wipe, redaction, atomic
write); inspectable queue + force-sync command from day one. Take the
two-phase prune-to-model's-output idea from the `local` pipeline but not its
leases. Do not port veracity-weighting without the usefulness signal from §3
— it is a ratchet without feedback.

## 5. Retry-with-feedback — smallest change, port first

**Finding.** `session/turn-recovery.ts`: five deterministic detectors (empty
stop, unexpected stop, malformed tool call, stream stall, thinking loop),
each capped at 3 attempts, each injecting a static template with an attempt
counter as a `developer` message and re-driving the turn
(`appendMessage` + `scheduleAgentContinue`, e.g. `:1013-1021`). Zero LLM
cost for the retry itself; the empty-stop path durably deletes the useless
turn first (`:1006`); the thinking-loop template is aimed at the loop itself
("pick the most boring viable one; act; do not deliberate further"). The
triggers are protocol anomalies, not correctness validators — nothing runs
the tests and re-injects the failure — and the adjacent
`output-schema-validator.ts` rejects bad subagent payloads without feeding a
re-attempt loop.

**Architecture for operant.** Same injection shape on our loop (developer
message + re-drive + attempt counter + cap), then add the detectors
oh-my-pi lacks: test-command failure and tool-output schema failure as
retry triggers with the failure text as the injected context. This is the
cheapest item on this list and the only one that is pure mechanism with no
model-cost question attached.

## 6. Rust-level fixes from zeroclaw (concrete, cheap, no design needed)

1. **Unguarded `tokio::spawn`+`unfold` pairs leak task + socket on early
   drop.** Operant already owns zeroclaw's 20-line `AbortOnDrop` guard and
   applies it to exactly one provider; 6 sites in `compatible.rs` and 1 in
   `anthropic.rs` need it. (Full detail: `docs/FEATURE-GAP.md` §3.)
2. **No parallel-execution predicate.** Our 8-worker pool runs every batch
   concurrently; zeroclaw serializes when >1 file mutation or any approval is
   present. Port the predicate, not the pool.
3. **Wire the dead `evaluate_response`.** Verified zero external callers
   (matches only at `eval.rs:131` + its own test module `:287-360`) while
   sister `estimate_complexity` is live at `agent.rs:1456`. Tens of lines for
   a self-critique-and-revise loop none of the three projects has. Trap:
   a second, *live* `EvalResult` in `skillforge/evaluate.rs:47` scores
   skills — do not mistake it for this one.

## 7. AFT — recommended integration

Verdict first: **keep the subprocess bridge; pin the release tag at build
time; do not link `agent-file-tools`.**

Measured facts (all verified, not inferred): operant's `aft_bridge.rs`
already spawns the `aft` binary over JSON-over-stdio — which is AFT's
documented adapter architecture — and self-updates to `releases/latest`
with 6 h backoff; the chain is live (`resolve_aft_binary` at `:111` →
`check_and_download_update` at `:150`, called at `:445`/`:870`). AFT's
`lib.rs` (348 lines, 40+ `pub mod` incl. `db`, `watch`, `lsp`, `pty`,
`callgraph`, `checkpoint`, `backup`) is the whole engine, not a wrapper, and
it requires MSRV 1.92 against operant's declared 1.89 (installed 1.98.0, so
local builds are unaffected — the cost is the support floor). AFT keeps
indexes, backups, checkpoints and a sqlite db under a shared root
(`~/.local/share/cortexkit/aft/`); linking the lib while keeping the bridge
puts two engines — two watchers, two writers — on the same files. And the
conceptual point: a cargo git-dependency yields a *library you link*, while
this integration needs an *executable you spawn* — the memory-wire pattern
transfers to the former, not the latter.

Recommendation, in order:
1. **Keep the bridge.** It is the supported shape, it works, it already
   self-updates. No migration risk.
2. **Replace `releases/latest` with a build-time-pinned tag.** A committed
   pin file (`aft.pin`: tag + asset sha) plus an explicit sync script
   (`./scripts/sync-aft.sh`: query latest, write pin, commit) gives
   "each rebuild tracks AFT source" with deterministic builds — "operant vX
   ships AFT vA.B" becomes a build fact instead of a runtime surprise. No
   build-time network (offline builds keep working off the pin); no silent
   behavior change under a fixed operant version. `latest` stays as the
   offline/unknown fallback.
3. **Do not add `agent-file-tools` as a dependency.** Cost is measured:
   MSRV 1.89→1.92, double-engine disk conflict, and operant inheriting
   AFT's watcher/db/LSP/PTY lifecycle. Revisit only if the bridge is ever
   dropped entirely — picking one path, not layering, is the constraint.
4. **Only exception, unverified:** a pure stateless piece such as
   `aft-tokenizer` may later be linked behind the same cargo-git pattern as
   memory-wire — precondition (not yet checked): no oxc/MSRV drag, no
   shared-disk writes, no daemon state. Do not do it until verified.

## 8. Build order and open decisions

Order by payoff-per-line and irreversibility, cheapest and most
deterministic first: retry-with-feedback (§5) → `evaluate_response` wiring
(§6.3) → `AbortOnDrop` + parallel predicate (§6.1–6.2) → recon (§1) →
reflection with the two fixes + usefulness signal (§3) → advisor (§2) →
dreaming (§4). The advisor and dreaming are the expensive loops and both
need the usefulness signal to be worth running — that signal is the one
piece of original design on this list, and it gates the two biggest builds.

Decisions for the user: advisor model identity (which cheap model shadows);
`sync_turn` write policy (already open from `docs/INTEGRATION-PLAN.md` —
the reflection fork writes through this seam, so the policy covers both);
recon budget caps (files/tokens per task); dreaming schedule (in-process
interval vs. explicit sync only); whether gateway/protocol hosts get the
advisor on by default (recommended yes, mirroring oh-my-pi's
`protocolDefault` split).

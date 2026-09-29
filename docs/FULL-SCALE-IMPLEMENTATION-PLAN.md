# Full-Scale Implementation Plan — bake-in + feature upgrades

Covers two workstreams the owner approved together: (A) sourcehound and
memory-wire **in-process** (not subprocess), and (B) the feature upgrades
from the three reference-project reports (`docs/FEATURE-UPGRADE-PLAN.md`
§§1–6). AFT stays on the bridge per the agreed recommendation and is out of
scope here except for its tag-pin (§8).

Measured against live source, 2026-09-29 (shallow clones, read-only):
- `github.com/ishan-parihar/sourcehound` @ HEAD: v1.6.0, MIT AND Apache-2.0,
  lib target `sourcehound_mcp` (`[lib] path = "src/lib.rs"`), 52,093 own
  lines. Public: `config` (`load_settings`), `server`
  (`SourcehoundMcpServer`), `tools` (implementations + types), `error`,
  `license`, `http`. Locked `pub(crate)`: `browser`, `cloakctl`, `cache`,
  `clustering`, `fusion`, `parsers`, `persistence`. Workspace vendors the
  obscura engine tree as members (upstream declares `workspace = true`, so
  they must resolve). No MSRV declared.
- `github.com/ishan-parihar/memory-wire` @ HEAD: v0.4.0, MIT OR Apache-2.0,
  dual `bin` + `lib` (`memory_wire`: `api, capture, embed, memory, recall,
  store, vector`). API is **synchronous** (`api.rs:408` `retain`,
  `:587` `recall`, `:612` `recall_with_weights`, plus `retain_tagged`,
  `retain_doc`, `reflect`). 30,177 lines. No MSRV declared. (Prior finding
  stands: it speaks `/banks/:id/recall`-shaped in-process calls, not
  agentmemory's HTTP shape — every consumer is a rewrite, not a config
  change.)

## Build order (dependency-ordered, cheapest first)

P0 — memory-wire provider (replaces agentmemory; unlocks reflection writes).
P1 — deterministic wins: retry-with-feedback, `evaluate_response`,
  `AbortOnDrop`, parallel predicate.
P2 — sourcehound tools surface (replaces IGS search/extract).
P3 — recon prepass.
P4 — reflection fork + usefulness signal.
P5 — advisor critic.
P6 — dreaming consolidation.
P7 — release hardening + docs + tag.

P4's usefulness signal gates P5/P6 — the advisor and dreaming loops are not
worth running without it (nobody has it; see UPGRADE-PLAN §0).

## P0. memory-wire `MemoryProvider` (in-process)

Goal: `memory_wire` as a cargo git-dependency tracking the
actively-developed branch; agentmemory REST client retired.

1. Add `memory_wire = { git = "https://github.com/ishan-parihar/memory-wire",
   branch = "<active>" }` to `operant-core` (confirm branch name with owner;
   do not guess `main` vs `master`).
2. Implement the `MemoryProvider` trait over `api::retain` / `recall` /
   `recall_with_weights`. Sync API → call directly; if profiling shows the
   store blocks the loop, move behind `spawn_blocking`, not a new thread.
3. `sync_turn` → `retain` (keep our 500/2000-char truncation; write-volume
   policy per owner decision — every turn vs. threshold-gated).
   `prefetch` → `recall_with_weights` (weights are the reason to prefer it
   over plain `recall`).
4. **`catch_unwind` at the provider boundary.** A 0.x in-process crate's
   panic otherwise kills the agent mid-turn; degrade to a memory miss.
5. Retire `AgentMemoryProvider` (REST + `:3111` auto-spawn) and the deferred
   MCP registration string behind the same flag flip; keep `BuiltinProvider`
   as fallback. Delete, don't deprecate — two live providers is how the
   double-engine conflict starts.
6. Gate: existing memory test suite green + new tests pinning retain→recall
   round-trip and the panic-degrades-to-miss behavior (mutation-prove the
   latter: remove the guard, test must fail).

Accepted costs (owner-approved): every memory-wire change becomes an operant
rebuild + release-gate risk; `Cargo.lock` churn on each update. Mitigation:
pin `rev =` at release time, track branch during development.

## P1. Deterministic wins (four independent units, parallelizable)

1. **Retry-with-feedback.** oh-my-pi `turn-recovery.ts:1013-1021` shape:
   deterministic detector → append `developer` message with attempt counter
   → re-drive the turn, cap 3. Port the shape; add the detectors oh-my-pi
   lacks (test-command failure, tool-output schema failure) with the failure
   text as injected context. Zero model-cost question. Gate: behavioral
   tests pressing each detector + mutation proof (disable the arm, test
   must fail).
2. **`evaluate_response` wiring.** Verified dead (zero external callers;
   sister `estimate_complexity` live at `agent.rs:1456`). Wire as
   score-and-maybe-revise after the draft response, threshold-gated. Trap:
   live `skillforge/evaluate.rs:47` scores skills — do not touch it.
3. **`AbortOnDrop` on the 7 unguarded spawn+unfold sites** (6
   `compatible.rs`, 1 `anthropic.rs`). We own the guard; apply it.
4. **Parallel-execution predicate.** Serialize batches with >1 file mutation
   or any approval; keep the 8-worker pool otherwise. Port zeroclaw's
   predicate, not its pool.

## P2. sourcehound tools surface (replaces IGS)

Goal: `sourcehound_mcp` as a cargo git-dependency; IGS search/extract
surface re-homed onto `tools::`.

1. Add `sourcehound_mcp = { git =
   "https://github.com/ishan-parihar/sourcehound", branch = "<active>" }`.
   **Trial-build first**: the vendored obscura members must resolve under
   our graph. If the resolver chokes on the nested workspace, stop and
   report — do not restructure their repo from our side.
2. Re-home in order: `web_search` → sourcehound `tools::` search functions;
   `web_scrape`/`web_extract` → `tools::`/`http::`; the fallback-chain
   search provider (`tools/web_providers/igs.rs`) → same. Read each
   `tools::` signature before wiring — call functions, do not run their
   `server`; we need tool calls, not an MCP server in-process.
3. **The browser lock.** `browser` and `cloakctl` are `pub(crate)` — our
   `browser.provider = "obscura"` CDP path cannot link them. Three options,
   owner's call: (a) open `pub mod browser` in sourcehound (one-line change
   on their side, they own the repo); (b) drive browsing through `tools::`
   if the functions cover it; (c) keep our own obscura CDP path and let
   sourcehound own search/extract only. Default to (c) unless the owner
   picks (a) — do not stall P2 on it.
4. Compile-cost is the price: 52k lines plus the vendored engine tree enter
   our build graph. Measure the delta on the release binary and record it;
   if it is unacceptable, the fallback is subprocess over their own MCP
   server (they serve rmcp; we already speak MCP client) — design the
   call sites behind a seam so the fallback doesn't rewrite them.
5. Gate: existing web-tool tests green against the new surface +
   `catch_unwind` at the boundary (same 0.x reasoning as P0) +
   mutation-proved wiring test (nothing passes with the call removed).

Fallback B (only if the trial build fails): spawn the `sourcehound` binary,
connect over our existing MCP client. Do not layer both.

## P3. Recon prepass (greenfield; no port exists)

Automatic harness-driven phase before a task's first tool call, over our own
AFT surface with a read-only grant, bounded by file + token caps, result
injected as developer context, config-skippable. Name it `recon`, never
`prewalk` (oh-my-pi collision). Gate: behavioral tests asserting the phase
fires before the first tool call and stays within caps; a skipped-config
test asserting it doesn't fire.

## P4. Reflection fork + usefulness signal (the original design on this list)

Port oh-my-pi Auto-Learn's fork isolation (separate session/cache key, no
fallback resolver, verbatim prompt, typed write envelope, managed-skill
hardening) onto our `sync_turn` seam, with the two fixes applied from day
one: abort-on-new-prompt, and a visible "wrote N memories / M skills" event.
Then design the usefulness signal — score recalled memories by whether the
consuming turn succeeded, feed back into admission. P5/P6 wait on this.

## P5. Advisor critic

oh-my-pi `advisor/` shape: post-turn delta → cheap-model shadow agent with
read-only tools → aside/steer delivery (interrupt yes, veto no) → emission
guard with budget. Default off for `chat`, on for gateway/protocol hosts.
No prompt changes on the acting side (zero residue when disabled). Gate:
wrong-action-style tests — a deliberately bad advisory must be interruptible
and ignorable, never blocking.

## P6. Dreaming consolidation

Sharpshooter shape minus defects minus distributed substrate: per-prompt
extract with host-pinned evidence → append-only delta store → periodic
consolidate with defer-while-streaming guard → friction-law admission →
host-side output ceilings → inspectable queue + force-sync. Skip leases,
heartbeats, SQLite jobs (single process). Admission consumes the P4
usefulness signal.

## P7. Release hardening

Full battery (`check` + `test` workspace, clippy gate, fmt, rustdoc gate —
the third gate that cost 31 red commits once), CHANGELOG `[Unreleased]`
refresh, `aft.pin` refresh per the agreed AFT recommendation, tag per the
established release process. Probes removed from `/tmp` before sign-off.

## Open decisions (owner's, with recommendations)

1. Branch names to track for both git-deps (recommend: the active dev
   branch during development, `rev =` pin at release).
2. `sync_turn` write policy — every turn vs. threshold (recommend threshold;
   every-turn is the write ratchet we criticize elsewhere).
3. Sourcehound browser lock, option (a)/(b)/(c) (recommend (c) — don't
   stall P2).
4. Recon budgets (files/tokens per task).
5. Advisor model identity + gateway-hosts default-on (recommend yes).
6. Dreaming trigger: interval while running vs. explicit sync only.

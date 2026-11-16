# Lane A — The Agent Loop and Model Dispatch

**Lane:** A (red-team + ponytail audit)
**Seam:** agent loop + model dispatch
**Pinned revision:** `25b86ae5` (confirmed via `git rev-parse --short HEAD`, 2026-11-16T03:18Z)
**Working tree:** clean except untracked peer scratch dirs (`.w10a-tmp`, `.w10b-tmp`, `.w10d-tmp`, `.tmp-test`, `crates/operant-cli/src/lib.rs`) and `crates/operant-core/memory:/`. None read by the build. Ignored.
**Method:** read-only. No cargo build/test/clippy (coordinator owns the gate). `rg` / `read` / `git grep` only.

**Working tree note (not my doing):** `git status` at the end of the lane showed 22 **staged** `.rs` / `.toml`
modifications (`operant-cli/src/tui/**`, `operant-core/src/config.rs`, `config_registry.rs`,
`operant.example.toml`) that are NOT mine — I wrote exactly one file, this document. Their mtimes are
`08:49:10` local, i.e. a concurrent peer's in-flight work (fleet rule 2: do not touch, do not revert).
My only writes were to this doc (`08:56` local). **No `.rs` file was edited and no cargo command was run.**

---

## Files read (exact list)

All paths relative to `crates/operant-core/src/` unless noted. Pinned rev `25b86ae5`.

**Read in full or in substantial part (the turn lifecycle):**
- `agent/run.rs` (5343 lines) — outline + `:392-470` (grace call), `:944-1152` (`reissue_with_budget` / `prepare_retry_request`), `:1190-1280` (turn prologue), `:1470-1560` (pause gate / budget / wall clock), `:1830-2050` (**streaming** failure ladder), `:2060-2160` (mid-stream retry), `:2250-2470` (**non-streaming** failure ladder), `:2660-2740` (empty-content fallback ladder), `:3870-3925` (steer drain), `:5290-5380` (`overflow_typed_tests`)
- `agent/stream.rs` (2606) — `:146-260` (`process_stream` head, chunk loop, stall/interrupt break), `:330-540` (router flush + error return), `:890-1000` (pre-flight interrupt/dedupe/mask), `:1700-2010` (`execute_tools`: single / semaphore / buffer_unordered / drain)
- `agent/error_classifier.rs` (1705) — `:16-121` (variant list), `:125-200` (`ClassifiedError` + `earns_recovery`), `:380-500` (`classify_api_error` step 1), `:1540-1700` (variant-coverage table + the "unproducible" note)
- `agent/fallback.rs` (1243) — `:60-260` (`Failover::classify` + both stage gates), `:300-400` (`try_next_provider_chat` / `_streaming`), `:408-545` (`try_models` + `chat` / `chat_streaming`)
- `agent/pooled_client.rs` (1381) — `:130-200` (`bench_credential`, `pool_exhausted_error`), `:200-345` (both rotation loops + mid-stream wrap), `:330-420` (`set_api_key` / `rotate_credential_value`)
- `agent/builders.rs` (1033) — `:110-310` (both constructors), `:940-1030` (`with_credential_pool`, `try_rotate_credential`)
- `agent/turn_retry_state.rs` (330) — whole file
- `agent/stream_retry_budget.rs` (560) — `:82-200` (`decide` / `should_rearm`)
- `agent/iteration_budget.rs` (325) — `:1-120`
- `agent/provider_registry.rs` (600) — `:34-80` (struct), `:150-200` (cooldowns, `clear_failure_count`, `get_client`), `:210-345` (`commit_switch`, `switch_to_next`, `reset_to_primary`, `fallback_candidates_after`)
- `agent/events.rs` (1360) — `:100-215` (`call_with_loop_timeout`, `emit_rate_limit_notice`), `:870-900` (`mark_effort_unsupported`, `effort_unsupported_for`, `model`), `:1210` (`effective_model`)
- `agent/compress.rs` (1361) — `:719-790` (`fitted_request` / `fitted_request_shaped`)
- `agent/mod.rs` (5987) — `:572-900` (field types), `:1080` (test-mod boundary)
- `agent/turn_context.rs` (782) — `:79-180` (prologue, interrupt reset)
- `turn_end.rs` (595) — `:99-330` (`TurnEnd`, `TurnEndBus`, `emit`, `fork`)
- `error.rs` (894) — `:132-300` (the `Error` enum), anyhow scan
- `interrupt.rs` — `:44-80` (`InterruptFlag`)
- `model_roles.rs` (933) — `:58-83` (`ModelSlot`)
- `tools.rs` — `:345-510` (`OperantTool::execute`, `execute_with_timeout`)
- `config.rs` — `:2604-2640` (`CredentialPoolSettings`)

**Read in the CLI crate (one file, for the pool-attachment question):**
- `crates/operant-cli/src/main.rs` — `:2864-2912` (`attach_credential_pool`), `:944-1030` (pool registry)

**Repo config:** `operant.example.toml` `:857-873` (`[credential_pool]`)

## Files NOT read

Everything else under `agent/` and `operant-core/src/`. Named, so the gap is auditable:

- **Agent module, unread:** `auto_poke.rs` (238), `background_review.rs` (1340), `chat_provider.rs` (268), `insights.rs` (876), `learning_graph.rs` (1414), `learn_prompt.rs` (186), `llm_compressor.rs` (1598), `message_safety.rs` (1342), `model_client.rs` (334), `prompting.rs` (1386), `rewind.rs` (846), `runtime_key.rs` (526), `skill_bundle.rs` (668), `skill_preprocessing.rs` (783), `turn_context.rs` beyond `:79-180`, `turn_rules.rs` (237)
- **Agent clients, unread:** `clients/anthropic.rs` (1048), `clients/openai.rs` (644), `clients/cache_monitor.rs` (935), `clients/prompt_caching.rs` (417). So **I did not verify what the wire actually carries** for reasoning params, cache_control, or the thinking-signature round trip — A4's premise (that `fitted_request` re-attaches the field) is traced in `compress.rs`, not at the adapter.
- **Seam files unread:** `turn_end_heuristics.rs` (686), `turn_attribution.rs` (992), `recap.rs` (671) beyond `:186-215`, `moa.rs` (625), `rate_limiter.rs` (557), `credential_pool.rs` (1583), `oauth_refresh.rs` (847), `agent/retry_policy.rs` (400) beyond the call site, `agent/rewind.rs` (846)
- **Not read at all:** every `tools/*.rs` except `tools.rs`; `context_management.rs`; `mcp.rs`; `gateway/`; and the whole `operant-cli` crate except `main.rs:2864-2912`.

**Consequences I accept:** the concurrency claims (A6/A7) are traced to `execute_tools` and the tool registry's `async fn execute`; I did NOT check whether any individual tool blocks the runtime thread (which would make the 8-permit pool a lie under a `current_thread` runtime). That is a real hole in A6 and I have not papered over it.

---

## Concepts a reader must learn (counts)

Measured, not estimated. A reader who wants to change a retry has to hold **all** of these at once.

### The counts

| concept | count | where |
|---|---|---|
| `FailoverReason` variants **declared** | **26** | `error_classifier.rs:16-85` |
| ...of which **producible** by `classify_api_error` | **24** | `:1574-1576` says so; A2 confirms |
| `Error` variants (the transport-level type) | **26** | `error.rs:132-300` |
| `ClassifiedError` recovery axes | **4** | `retryable`, `should_fallback`, `should_compress`, `should_rotate_credential` (`:132-144`) |
| ...of which actually **read** by a consumer | **3** (`should_fallback` is dead — A1) | |
| `FailoverDecision` arms | **3** | `RetrySame` / `FallBack` / `GiveUp` (`fallback.rs:195-200`) |
| Independent fallback gates | **2** | `advance_model_chain`, `switch_provider` (`fallback.rs:178-186`) |
| One-shot retry guards per turn | **5** | `compress`, `rotate`, `reasoning_param`, `fallback`, + the counter pair (`turn_retry_state.rs:51-73`) |
| `restart_with_*` signal flags | **3** | `turn_retry_state.rs:77-82` — **write-only in production** (see below) |
| `StreamRetryGiveUp` reasons | **4** | `WallClock` / `Attempts` / `TurnBudget` / `NotRetryable` (`stream_retry_budget.rs:96-115`) |
| `ModelSlot` roles | **8** | `model_roles.rs:58-83` |
| Parallel tool workers | **8** — enforced **twice** (A6) | `stream.rs:1853`, `:1965` |
| Failure ladders in `run.rs` | **2**, near-duplicates | `:1831` (stream) vs `:2250` (non-stream) |
| Turn-exit reasons a reader must distinguish | **~7** | `Interrupted`, `ContextExhausted`, `StreamDied`, `RequestTimeout`, `MaxIterationsExceeded`, `Interrupted`-vs-halt, grace-call summary |

**Bottom line for the map:** a reader must learn **26 error classes, 26 transport errors, 4 recovery axes (3 live), 3 decisions, 2 fallback gates, 5 one-shot guards, 4 stream give-up reasons, 8 model slots, and 2 parallel ladders** — to change one retry.

### Two structural facts that make the count worse than it looks

1. **The retry policy is expressed twice.** `run.rs` has a streaming arm and a non-streaming arm covering the same failure space with independently-maintained branches. They already disagree (A4). The repo knows: `run.rs:5323` pins *one* arm's parity with a source-grep test, and that test exists because the other arm silently drifted once (RT-286).
2. **Three of the five one-shot guards are unreachable from production.** `TurnRetryState.restart_with_compressed` / `restart_with_rotated_credential` / `restart_with_rebuilt_messages` (`turn_retry_state.rs:77-82`) are set only inside `turn_retry_state.rs`'s own tests (`:279`, `:292`). Exhaustive:
   ```
   rg -n "restart_with_(compressed|rotated_credential|rebuilt_messages)" crates/operant-core/src
     turn_retry_state.rs:31,77,79,82,95,96,97,119,120,121,136,137,138,279,292,300
     (nothing in run.rs / stream.rs / mod.rs)
   ```
   The production code that touches them is `has_restart_signal()` (`:118-122`) and `clear_restart_signals()` (`:135-139`). `clear_restart_signals` IS called from production — `reset_on_success()` (`:144-147`) calls it — but it only ever clears three flags nothing sets, so it is a no-op in production. `has_restart_signal()` has **no** production caller (only `:199`, `:277`, `:280`, `:283`, all tests). So the "restart signals (set by error handlers, read by outer loop)" comment at `:75` describes a mechanism with **no production setter and no production reader**. The real restart path is the direct `continue` in the ladder, not these flags.

   **Ponytail verdict:** delete the three fields, `has_restart_signal`, and the `clear_restart_signals()` body (leaving `reset_on_success` clearing nothing), plus the 4 tests. ~40 LOC, zero behaviour change.

   The same duplication exists one level up: `TurnRetryState.fallback_attempted` (`:73`) has no production reader — `run.rs` keeps its **own** `let mut fallback_attempted = false` (`:1444`) and never touches the struct field. `rg "\.fallback_attempted" | grep -v turn_retry_state.rs` → the only hits are `run.rs:2674,2676`, both on the **local**.


---

## Findings

### A1 — `ClassifiedError::should_fallback` is computed in 30+ places and read by nobody [VERIFIED] — LOW (dead field, ponytail cut)

`rg -n "\.should_fallback" crates/operant-core/src -g '*.rs' | grep -v error_classifier | grep -v agent/fallback.rs` returns **zero hits**. The field is set at 30+ sites
(`agent/error_classifier.rs` passim, `agent/fallback.rs:101,113,125,144,156,168`) and never branched on by the loop.
The real gate is `Failover::classify`'s `advance_model_chain` / `switch_provider`, computed independently from the concrete `Error` variant
(`agent/fallback.rs:178-186`), and the code says so out loud:

> "Note this is deliberately *not* `classified.should_fallback`, which is set for cases (a content-policy rejection at 400) the model chain has never advanced on." — `agent/fallback.rs:86-89`

**Failing case:** a future maintainer adds `should_fallback: true` to a new variant expecting the model chain to advance. It does not. The flag is a decoy that reads as the control surface and is not.
**Why it matters:** the audit brief claims "24 error classes with `should_compress`/`should_fallback`" (AGENTS.md "What IS Working"). Half of that sentence describes a dead field. Either wire it or delete it.

### A2 — `FailoverReason::AuthPermanent` and `UpstreamRateLimit` are unproducible variants [VERIFIED] — LOW

Production sites outside `error_classifier.rs`, exhaustive:

| variant | sites |
|---|---|
| `AuthPermanent` | enum def `:21`, `as_str` `:98`, `is_auth()` `:150` — **no construction site** |
| `UpstreamRateLimit` | enum def `:29`, `as_str` `:101`, `events.rs:196` (a match arm) — **no construction site** |

`classify_api_error` never returns either. This is not a hidden defect: the repo documents it in a test comment,
`agent/error_classifier.rs:1574-1576`: *"`AuthPermanent` and `UpstreamRateLimit` are unproducible today and deliberately excluded."*
So the count a reader must learn is **26 declared / 24 producible**.
**Why it matters:** `events.rs:196` groups `RateLimit | UpstreamRateLimit` for display, so a reader greps for the second arm and finds nothing. Delete both variants and their match arms, or add the detection site (`Aggregation`/`upstream` 429 bodies) they were clearly meant for.

### A5 — `[agent] stream = false`: a `ReasoningParamRejected` failure is retried with the field STILL ATTACHED, so the non-streaming arm never recovers [VERIFIED] — HIGH

**The ladder, traced.** `OperantAgent::run` has two arms on `request.stream` (`agent/run.rs:1831`).

Streaming arm (`run.rs:1869-1894`) is the first branch of the ladder:
```rust
if classified.reason == FailoverReason::ReasoningParamRejected
    && !retry_state.reasoning_param_attempted {
    retry_state.reasoning_param_attempted = true;
    self.mark_effort_unsupported(&model);       // ← the FIX lands here
    ...
    let retry_request = self.fitted_request(..).with_reasoning_effort(None);
    self.call_with_loop_timeout(self.client.chat_streaming(retry_request)).await?
}
```
The non-streaming arm (`run.rs:2250-2460`) has **no such branch**:
`awk 'NR>=2250 && NR<=2470' run.rs | grep -c ReasoningParamRejected` → **0**.
Its ladder is `should_compress` (`:2266`) → `should_rotate_credential` (`:2324`) → `else { reissue_with_budget }` (`:2385`).

**Concrete failing case (non-streaming, `stream = false`):**
1. Operator runs a host that rejects the reasoning field, e.g. a 400 body `"reasoning_effort: unrecognized parameter"`.
2. `classify_api_error` → `ReasoningParamRejected { retryable: true, should_fallback: false, should_compress: false, should_rotate_credential: false }` (`error_classifier.rs:437-446`).
3. Not `should_compress`, not `should_rotate_credential` → falls into the `else` → `reissue_with_budget`.
4. `reissue_with_budget` → `prepare_retry_request` (`run.rs:1115`). `earns_recovery()` is **true** (retryable, not `Unknown`, not compress) so the re-issue is granted.
5. It rebuilds via `self.fitted_request(&self.effective_model(), messages, tools)` (`run.rs:1148`) — and `fitted_request` re-reads `self.reasoning_effort` (`compress.rs:739-744`), which was **never cleared**, because the only writer of the suppression is `mark_effort_unsupported` and the only reader of the suppression is `effort_unsupported_for` inside that same `fitted_request`. `reasoning_param_attempted` is likewise never set.
6. The identical request goes out, the provider rejects it identically, and the ladder burns the whole retry budget + wall clock re-sending a request that can never succeed.

**The adapter link (now traced, was the one open gap):** the field does reach the wire.
`clients/openai.rs:76-84` passes `request.reasoning_effort.as_deref()` into `chat_with_effort`, and `client.rs:1138-1190`
turns it into the JSON body (`ReasoningShape::ReasoningEffort` → `request["reasoning_effort"] = json!(..)` at `:1157-1158`).
So a stale `Some("high")` in `self.reasoning_effort` (`compress.rs:739-744`) IS serialized on every retry.
RT-472 (`client.rs:298-300`, `clients/openai.rs:298`) is the same field's history: the shape used to be dropped, and the fix
was to make it travel — which is precisely why the value survives into a request that should have suppressed it.

**Failing case:** with `stream = true` the turn recovers and answers. With `stream = false` the same provider, same body, same turn
dies with a provider error after N retries, and the model stays marked as effort-capable — so the NEXT turn repeats it.

**Why it matters:** this is the same rule applied differently in two paths, and the repo has already been bitten by exactly this class here — RT-286 ("the NON-STREAMING context-overflow retry must fail TYPED") is pinned by the source-grep test `the_non_streaming_compression_arm_fails_typed` (`run.rs:5323`). That test pins ONE arm's parity. The reasoning-param arm has no equivalent pin, which is how it drifted.

**Fix shape (not applied, read-only lane):** add the same `ReasoningParamRejected && !retry_state.reasoning_param_attempted` branch as the first arm of the non-streaming ladder, mirroring `run.rs:1869`, and extend the `run.rs:5323` source-grep contract to cover it.

### A3 — `rotation_count` is write-only and never reset: the anti-thrash cooldown ratchets to 60 s forever on a long-lived agent [VERIFIED] — MED

`rotation_count` is declared at `agent/mod.rs:893`, initialised to `0` at both constructors (`builders.rs:190`, `builders.rs:303`), incremented at `builders.rs:1009-1010`, and read only to log (`builders.rs:1022`). Exhaustive:
```
rg -n "rotation_count" crates/operant-core/src
  mod.rs:893          (field decl)
  builders.rs:190,303 (init 0)
  builders.rs:1009    (*count += 1)
  builders.rs:1022    (log)
```
There is **no reset site** — unlike its siblings `iteration_budget.reset()` (`run.rs:1250`) and `interrupt_flag.reset()` (`turn_context.rs:151`), which are both explicitly reset in the turn prologue.

The cooldown it feeds is `delay = (5.0 * 2^(count-1)).min(60.0)` (`builders.rs:1011-1012`). So on a gateway agent serving many sessions:

| rotation # | cooldown armed |
|---|---|
| 1 | 5 s |
| 2 | 10 s |
| 3 | 20 s |
| 4 | 40 s |
| 5 | 60 s |
| 6..N | **60 s, forever** |

**Concrete failing case:** a gateway agent hits one 429 per hour, every hour, for a day. By rotation 6 the cooldown is pinned at 60 s. Now a genuine credential failure arrives; `try_rotate_credential` sees `now < *cooldown` and returns `None` (`builders.rs:984-990`); `run.rs:1967` / `:2336` take the `else` and `return Err(e)` — "No more credentials to rotate", when the pool is in fact full. The turn dies on a healthy pool.

**Scope caveat (keeps this MED not HIGH):** `credential_pool` is `Option` and only attached when `config.credential_pool.enabled` is true (`main.rs:2870-2872`), which is `#[derive(Default)]` → `false` (`config.rs:2614-2619`). So this bites only operators who enabled the pool. That is exactly the population the feature exists for.

### A4 — `rotation_count` and `rotation_cooldown_until` are per-AGENT, not per-POOL, so N agents on one pool each get their own cooldown [ASSERTED] — MED

Both fields are plain `Arc<RwLock<..>>` values on `OperantAgent` (`mod.rs:890,893`), constructed fresh per agent (`builders.rs:189-190, 302-303`). The pool itself IS shared (`attach_credential_pool` → `registry.get_or_create(provider)` returns a shared `Arc`, `main.rs:2876-2881`).

**Consequence (traced):** agent A rotates at t=0, arms a 5 s cooldown on **A's** cell, and invalidates the shared pool's active credential. Agent B, sharing the pool, has its own `rotation_cooldown_until = 0.0` — so B's cooldown check passes immediately and B rotates again at t=0.1 s. The anti-thrash gate whose comment says it exists "to prevent burning through the credential pool when multiple auth failures cascade" (`builders.rs:976-977`) does not hold across agents.

**What I could not confirm:** how many `OperantAgent` instances a live gateway builds per provider pool. I read `attach_credential_pool`'s signature and call sites (`main.rs:2510`, `:2633`) but did not trace the session-slot architecture that decides the agent count, so the *magnitude* of the leak is ASSERTED. The *shape* (per-agent cooldown, shared pool) is VERIFIED from the field types.

### A6 — `execute_tools` carries the SAME 8-tool limit twice, in two different mechanisms, with different cancellation semantics [VERIFIED] — MED

`agent/stream.rs` bounds parallel tool execution **twice**:

| mechanism | line | what it does |
|---|---|---|
| `Semaphore::new(8)` | `:1853` | each future acquires a permit before running |
| `buffer_unordered(8)` | `:1965` | only 8 futures are polled at once |

These compose, so the effective parallelism is 8 (not 16) — but the semaphore is **redundant**: `buffer_unordered(8)` alone already bounds in-flight futures to 8. The permit acquisition inside each future can therefore never actually wait, since at most 8 futures exist.

The two mechanisms differ in a way that IS observable — the cancellation arm inside the future:
```rust
// stream.rs:1880-1900  (inside the semaphore-guarded future)
let _permit = match sem.acquire().await { ... };
if interrupt_flag.is_triggered() {
    return (idx, ToolResult::error(&tool_call.id, "Skipped: interrupted"));
}
```
versus the pool-level drain:
```rust
// stream.rs:1978-1985  (after buffer_unordered is dropped)
_ = self.interrupt_flag.wait_triggered() => { break true; }
...
early_results[idx] = Some(ToolResult::error(&tool_call_ids[idx],
    "Cancelled by the user before this call finished."));
```
So one cancelled call can read `"Skipped: interrupted"` and its neighbour `"Cancelled by the user before this call finished."` — two strings for one user action, and only the second is the one the transcript-pairing logic reasons about. A third spelling exists in pre-flight (`:906` `"Skipped: interrupted by user (Ctrl-C)"`).

**Ponytail verdict:** delete the semaphore (`:1853`, `:1880-1908`) and keep `buffer_unordered(8)`. One bound, one cancellation string.

### A7 — the three cancel spellings mean a tool call's cancellation reason depends on WHICH boundary observed the flag [VERIFIED] — LOW

Same fact as A6, stated as a user-visible defect. Exhaustive:
```
stream.rs:906   "Skipped: interrupted by user (Ctrl-C)"      (pre-flight)
stream.rs:1898  "Skipped: interrupted"                        (semaphore-guarded future)
stream.rs:1991  "Cancelled by the user before this call finished."  (pool drain)
```
**Failing case:** a batch of 4 tools, `/stop` pressed. Tools still in pre-flight get spelling 1, tools holding a permit get spelling 2, tools that never started get spelling 3. The model sees four different texts for one event, and only spelling 3 is the one `commit_interrupted_tail` / the pairing logic treats as "side effects UNKNOWN". A call answered with "Skipped: interrupted" reads as *safe to retry*, which after a cancel is exactly the wrong signal for a mutation.

### A8 — the classifier's step-1 ordering makes `ReasoningParamRejected` shadow `ThinkingSignature` [ASSERTED] — LOW

`agent/error_classifier.rs:406-413` (ThinkingSignature) requires `status==400 && body contains "thinking" && ("signature" | "cannot be modified" | "must remain as they were")`.
`:429-437` (ReasoningParamRejected) requires `status 400|422 && body contains ("reasoning" | "reasoning_effort" | "effort" | "thinking") && ("unrecognized" | "unknown" | "unsupported" | "invalid parameter" | "unexpected" | "not supported")`, and it is checked **after** ThinkingSignature, so ThinkingSignature wins a tie. That ordering is correct.
**What I could not confirm:** whether any real provider body satisfies both (e.g. `"thinking signature unrecognized"`). If one does, ThinkingSignature wins and `should_fallback:false, retryable:true` — a sig-strip retry. I did not find such a body in the tests. Marked ASSERTED because I have no captured provider payload; the *ordering* is VERIFIED correct, the *collision* is not.



---

## Ponytail cut list

Everything here is **deletable or mergeable with zero behaviour change** unless marked. LOC estimates are from `wc -l` on the pinned rev.

| # | cut | LOC | why it is safe |
|---|---|---|---|
| 1 | `ClassifiedError::should_fallback` — the field **and** all ~30 construction sites | ~35 | Zero readers (A1). `advance_model_chain` / `switch_provider` already decide fallback from the concrete `Error` variant and the code comments say the field is deliberately not consulted (`fallback.rs:86-89`). Every struct literal must drop one line; no logic changes. |
| 2 | `TurnRetryState.restart_with_compressed` / `_rotated_credential` / `_rebuilt_messages`, `has_restart_signal()`, the `clear_restart_signals()` body, and the tests at `:277-283` | ~40 | Nothing in production ever **sets** these flags (only `turn_retry_state.rs`'s own tests at `:279`, `:292`). `clear_restart_signals` is called by `reset_on_success` but only clears three always-false flags. The real restart is the ladder's `continue`. |
| 3 | `TurnRetryState.fallback_attempted` | 3 | Zero readers. `run.rs:1444` keeps a local of the same name and uses that (`:2674`, `:2676`). Two variables, one behaviour, one of them dead. |
| 4 | `FailoverReason::AuthPermanent`, `FailoverReason::UpstreamRateLimit`, their `as_str` arms, and the `UpstreamRateLimit` arm in `events.rs:196` | ~12 | Unproducible (A2). If a detection site is wanted, add it with the variant; do not keep a variant that reads as a live class. |
| 5 | The `Semaphore::new(8)` permit machinery in `execute_tools` (`stream.rs:1853`, `:1880-1908`) | ~30 | `buffer_unordered(8)` at `:1965` already bounds in-flight futures to 8, so the permit can never wait (A6). Also removes one of the three cancellation spellings. |
| 6 | **Merge** the two failure ladders in `run.rs` into one generic over the call (`chat` vs `chat_streaming`), as `try_models` already does in `fallback.rs:412` | ~400 net | A5 is the concrete cost of the duplication: one arm has a recovery branch the other lacks, and the repo's own RT-286 test exists to catch the drift. One ladder + one source-grep parity test is the whole fix. |
| 7 | **Delete** `emit_fallback`'s `from: self.primary_model.clone()` in favour of the model actually attempted | n/a (change, not cut) | `fallback.rs:288` always reports `from: primary`, even when the failing attempt was fallback model 3 — the TUI event is wrong after the first hop. |

**Not a cut, but worth naming:** `try_next_provider_chat` and `try_next_provider_streaming` (`fallback.rs:311-395`) are the **same 45-line function twice**, differing only in `chat` vs `chat_streaming` and `ChatResponse` vs `BoxStream`. Same shape as cut #6 one level down — foldable into one generic. ~45 LOC.

**Deliberately NOT proposed:** the `advance_model_chain` / `switch_provider` pair is two gates, not a duplicate — they encode genuinely different questions ("will another model help?" vs "will another provider's quota help?") and the code documents why they diverge (`fallback.rs:83-89`). Leave them.

## Rust discipline table

Rule set from the repo's own "Rust Code Standards" (borrow-before-clone, typed errors, no `unwrap` in production, no `anyhow` in lib, unboxed large variants, no clone-in-loop).

| rule | sites | representative file:line | verdict |
|---|---|---|---|
| **No `unwrap`/`expect` in production** | ~199 hits across the 4 hot files, but **all** are inside `#[cfg(test)]` | `run.rs` test mods start `:5002`; `stream.rs` `:2105`; `mod.rs` `:1080`; `fallback.rs` `:567`; `pooled_client.rs` `:481` | **PASS.** I checked every non-test hit: the production ones carry `#[expect(clippy::expect_used, reason = "...")]` with a real reason (e.g. `stream.rs:1743-1745` "unreachable: the `pending.len() == 1` guard above leaves exactly one element"). The one bare `.expect()` in production is `fitted_request` (`compress.rs:742`, "reasoning_effort RwLock poisoned — programmer error"), a legitimate poison-recovery. |
| **No `anyhow` in a lib crate** | 0 hits in `error.rs`, `agent/*.rs` | `error.rs` uses `thiserror` throughout | **PASS.** `operant-core` is clean; the seam is typed (`Error` 26 variants) and the classifier returns a typed `ClassifiedError`, not a string. |
| **Typed errors, not strings, at the recovery seam** | 0 raw-string returns in the failure ladders | `run.rs:1935-1960` returns `Error::ContextExhausted { .. }`, not `Error::Agent(format!(..))` | **PASS**, and the repo is unusually disciplined about it — the `RequestTimeout` doc (`error.rs:159-183`) explains why the TYPE must survive annotation. |
| **Borrow before clone** | 1 `.clone()` in a loop in `run.rs`; the loops at `:1380`, `:3833` iterate `evo.persist_counters()` by value from an owned map | `run.rs:3882-3924` (steer drain) clones `steer_msg` then moves it into `add_and_persist_message` | **PASS (minor debt).** The steer clone is *required*: `messages.push` needs one copy and `add_and_persist_message` takes ownership of the other. Not removable. |
| **No clone-in-loop** | The steer drain's `.clone()` is the only candidate and is forced | `run.rs:3897` | **PASS.** |
| **No redundant `.collect()`** | The ladders build `Vec` then iterate; `fallback_candidates_after` collects then iterates | `provider_registry.rs:331-339` collects a `Vec<ProviderEntry>`, then `fallback.rs:317` iterates it once | **PASS (marginal).** The `collect` exists because `is_in_cooldown` takes `&self` and the caller wants a snapshot — defensible, not a defect. |
| **No unboxed large enum variants** | `Error::Provider { status, body, .. }` and `Error::RequestTimeout { budget, guidance }` both carry a `String` inline | `error.rs:250-257`, `:175-183` | **PASS by size, flagged for watching.** `body: String` inline is 24 bytes; `Error` is not `clippy::large_enum_variant`-sized today. But `error.rs` is the one place a future `Box` would be cheap insurance, because `Result<T, Error>` is on every fallible path in the seam. **Not a finding** — I measured no threshold breach and did not run clippy (coordinator owns that). |
| **No panics in production paths** | 1 hit, and it is in a test | `agent/pooled_client.rs:1219` (`panic!("expected RateLimited, got {other:?}")`) sits after the `#[cfg(test)]` at `:481` | **PASS** (grep-verified over `agent/run.rs`, `stream.rs`, `fallback.rs`, `pooled_client.rs`, `error_classifier.rs`, `error.rs`; the sole `panic!` is inside the test module). |

**Discipline verdict: this seam is clean.** The repo's Rust hygiene is not the problem here. Every finding in this document is a **logic** defect (a missing branch, a field that is never reset, a duplicate bound, a dead field), not a memory-safety or error-handling one. The `.expect()` discipline in particular is better than the AGENTS.md prose suggests — every production `expect` I found carries a justification comment naming the invariant.

One thing the table **cannot** certify: I did not run clippy, so "no `redundant_clone`" is a reading judgement on ~7 call sites, not a gate result. Treat the clone row as ASSERTED-by-inspection, not VERIFIED.

---

## Guards I checked that HOLD

Named because a red-team doc that only lists defects misrepresents the seam. Each of these looked suspicious and turned out to be correct.

| guard | why I doubted it | code that holds it |
|---|---|---|
| **`InterruptFlag` is shared across clones, so a `/stop` reaches a tool mid-flight** | `interrupt_flag` is a plain field, not obviously `Arc` | `interrupt.rs:44-46` — `triggered: Arc<AtomicBool>`, and `trigger()` (`:59-61`) documents "All clones of this flag will now report `is_triggered() == true`". `stream.rs:1792` clones it into the tool future and races it via `select!` (`:1794-1800`). |
| **The interrupt flag IS reset per turn** | a shared, never-reset flag would kill every turn after the first | `turn_context.rs:148-151` — the first step of the turn prologue: "Without this, a Ctrl-C in run #1 permanently breaks run #2+". Also `run.rs:1229` resets the model override and `:1250` resets the iteration budget in the same prologue. |
| **`IterationBudget` cannot over-count under concurrency** | `fetch_add` + rollback has a torn window | `iteration_budget.rs:41-54` uses `compare_exchange` in a CAS loop and says why; `refund()` (`:57-70`) CAS-loops too and returns early at 0 so two concurrent refunds cannot underflow. |
| **A second context overflow fails TYPED, not as a raw provider error** | a bare `?` would leak `Error::Provider` past the gateway's `ContextExhausted` gate | both arms handle the retry's own error inline (`run.rs:1929-1960` stream, `:2298-2330` non-stream) and return `Error::ContextExhausted { tokens, window, required, minimum_window, minimum_hard_cap }`. Pinned by `run.rs:5323`. |
| **A cancelled tool call still produces a result, so API pairing holds** | dropping the pool would leave the model's `tool_call_id` unanswered | `stream.rs:1970-1990` — `select!` on the pool against `wait_triggered()`, then every index with no result gets a `ToolResult::error`. The `ids_by_call_index` fix (RT-462, `:1858-1872`) is why a pre-flight rejection no longer shifts the pairing or panics on out-of-bounds. |
| **A partial answer survives a stream break** | `break` on error would discard the buffer | `stream.rs:203-208` sets `stream_error` and `break`s, then the flush at `:417-470` runs on BOTH paths, and the error returns at `:492` — after the flush. The comment at `:400-404` names the prior bug ("the error path `break`ed before this flush, dropping all partials"). |
| **A credential is never double-benched by rotation** | `set_api_key` rewriting the pool value would duplicate a secret | `pooled_client.rs:336-372` — `set_api_key` explicitly does NOT touch the pool, and the doc names the exact failure it prevents ("on every 401 and every 429"). The anchor is refreshed instead so a later `.env` rotation replaces the credential actually in use. |
| **`TurnEndBus` delivery never stalls or blocks the turn** | a `broadcast` with a slow subscriber could lag or block | `turn_end.rs:229-249` — synchronous by design, `receiver_count() == 0` short-circuits before the event is even constructed, and `send(..).unwrap_or_default()` turns a racing-subscriber drop into 0. `Lagged(n)` is handled by every subscriber (`recap.rs:214`, `advisor.rs:542`, `session_server.rs:175`). |
| **The pause gate actually freezes a live turn** | an estop checked only between turns is theatre | `run.rs:1480-1486` (before each model call) and `:3000-3006` (before each tool batch) — two boundaries, both polling with a 1 s sleep, and the comment names the "never pauses pays exactly one extra stat per boundary" cost. |

---

## Summary

**8 findings: 1 HIGH, 3 MED, 4 LOW. No BLOCKER.** (6 VERIFIED — A1, A2, A3, A5, A6, A7; 2 ASSERTED — A4, A8.)

The headline is **A5**: with `[agent] stream = false`, a `ReasoningParamRejected` failure is retried by `reissue_with_budget` with the rejected reasoning field still attached, because the non-streaming arm of `run()` never got the `ReasoningParamRejected` branch the streaming arm has. The turn burns its whole retry budget re-sending a request that can never succeed, then fails, and the model stays marked effort-capable so the next turn repeats it. The same provider body, the same turn, recovers under `stream = true`. The repo already carries the scar tissue for this exact class of drift (RT-286 / `the_non_streaming_compression_arm_fails_typed`, `run.rs:5323`) — that test pins one arm; the reasoning-param arm has no equivalent.

The MED cluster is credential rotation: `rotation_count` is write-only and never reset, so the anti-thrash cooldown ratchets to 60 s and stays there (A3); and the cooldown cells are per-agent while the pool is shared, so N agents defeat the gate the comment claims it provides (A4). Both are gated behind `credential_pool.enabled`, which defaults to `false`. The third MED is A6, the doubly-enforced 8-tool bound.

The LOW cluster is accumulation: a dead recovery axis (A1), two unproducible error classes (A2), three cancellation spellings for one user action (A7) — a genuine mis-signal for a mutation after a cancel — and the classifier-ordering question (A8).

**Ponytail:** ~560 LOC is deletable or mergeable with zero behaviour change. The single highest-value cut is #6 — collapsing `run.rs`'s two ~1000-line failure ladders into one generic, which is both the ~400-LOC prize and the structural fix for A5.

**Rust discipline: clean.** No `anyhow` in the lib crate, no `unwrap`/`expect` in production outside justified `#[expect]` sites, no panics, no clone-in-loop, typed errors all the way to the recovery seam. Every finding here is logic, not hygiene.

**Confidence:** A1, A2, A3, A5, A6, A7 are VERIFIED end to end in code I read — including A5's adapter link (`clients/openai.rs:76-84` → `client.rs:1138-1190`), which I closed after the first draft. A4 and A8 are ASSERTED and name what I could not confirm.
//! **An oversized `tool_describe` result cannot reach the model.**
//!
//! Row 4 of the 2026-10-31 codex-portability plan asked for "a MCP schema byte
//! budget (one constant + truncation)" on the strength of this finding, quoted
//! from `F0176`/the ledger:
//!
//! > `tool_describe` (`tool_search.rs:464-471`) returns `s.parameters` with
//! > NO byte cap, so a server advertising a huge schema injects all of it —
//! > codex caps at 8,000/64,000 (`mcp_tool_exposure.rs:22-23`).
//!
//! The premise measured TRUE of `tool_describe` itself and FALSE of operant's
//! behaviour, because the cap is not in that tool — it is one hop later, at
//! `agent/run.rs:2101`, where `truncate_tool_result` runs on **every** tool
//! result before the text is pushed to the LLM-bound message list. So the
//! import was a duplicate mechanism, not a missing one.
//!
//! **The measurement that settles it** (run against the deployed tree with a
//! 2,000-field schema, i.e. a server advertising ~495 KB of parameters):
//!
//! ```text
//! PROBE in=494974 out=4132
//! ```
//!
//! 494,974 bytes in, 4,132 out — a tighter bound than codex's 8,000-byte
//! per-spec cap, already live, and already applied at the point the plan
//! wanted a new cap. This file turns that measurement into a guard so the
//! claim can never regress into "we assume it is capped" again.
//!
//! **WHICH bound this pins — stated honestly, because mutation testing
//! disproved half of my first draft.** Two bounds wrap a tool result, in this
//! order at `agent/run.rs`: the SPEND GUARD `guard_tool_output_spend`
//! (`compress.rs:587-639`, called at `:2130`) withholds the bulk behind a
//! marker stating the size, the share and the estimated price; then the
//! ABSOLUTE CAP `truncate_tool_result` (`mod.rs:1116`, called at `:2101`)
//! trims to `MAX_TOOL_RESULT_LEN`. At this fixture's 8,000-token context the
//! spend guard fires first, so:
//!
//! - disabling the SPEND GUARD: request grows ~19 KB -> ~22.4 KB and the
//!   marker becomes `[truncated`, so **this test FAILS by name**. That is the
//!   bound it pins.
//! - disabling the ABSOLUTE CAP: request moves 19,277 -> 19,284 chars and this
//!   test still PASSES. I am recording that rather than hiding it: at a window
//!   where the guard does NOT fire first the cap is the backstop, and this
//!   fixture is not where that is observable. Do not read this file as
//!   proving the 4 KB cap is load-bearing here.
//!
//! **AND WHY IT IS NOT VACUOUS — the third mutation, found by probing for it
//! rather than waiting for someone to point it out.** A CEILING check alone
//! would pass just as happily if the agent DROPPED the result entirely: I
//! swapped the fixture so the tool returns a tiny payload and the test still
//! passed, because the second request was 18,270 chars against the ceiling of
//! 20,000 — nothing bounded, nothing missing, nothing observed.
//!
//! The growth assertion below (>= 1017 chars) closes that. Both directions now
//! fail by name, and BOTH figures are measured and printed by the test itself
//! (`--nocapture`, the `[growth]` line) so neither is a remembered number:
//!
//! * real fixture:       18,259 -> 19,277  (**+1018**, the truncated schema)
//! * vacuous fixture:    18,259 -> 18,646  (**+387**, schema shrunk to one
//!   field so it no longer dominates the request)
//!
//! **A CORRECTION ON THE VACUOUS FIGURE.** This file previously published the
//! vacuous growth as `18,270 (+11, just a tool name)`. That number is real but
//! it belongs to a DIFFERENT probe — the tiny-tool-result one described in the
//! paragraph above. The probe the floor is actually verified against is the
//! shrunk-schema one, which measures **+387**, and `+11` was carried forward
//! into the range `11 < floor` without re-measurement after the probe changed.
//!
//! **AND A SECOND CORRECTION ON THE RANGE ITSELF, because "between the two
//! regimes" was still wrong.** I then published `387 <= floor <= 1018`, which
//! reads as though each regime's value were usable. It is not, at either end:
//! floor 387 against the vacuous fixture PASSES (387 >= 387), so it fails to
//! discriminate; and floor 1018 against the real fixture passes, which makes the
//! upper end inclusive rather than the exclusive bound the phrasing implied.
//! Probing the endpoints gives **388 <= floor <= 1018**. The floor is marked
//! `provisional:` in source because that is what it is derived from — and a
//! range derived from two endpoints has to have those endpoints probed, not
//! just named.
//!
//! **A NOTE ON SCOPE, so the next reader does not go looking for a second
//! thing to fix — and the sharper finding, which my first note understated.**
//! `crates/operant-api/src/observability_traits.rs` defines its OWN
//! `ObserverEvent` enum with a `ToolCall { tool, duration, success }` and no
//! byte fields. I first recorded it as "disconnected, so out of scope by
//! design". That was too kind. Measured: `operant-core` declares no
//! `operant-api` dependency; every real `Observer` implementor
//! (`ConsoleObserver`, `NoopObserver`, `TurnSummaryObserver`,
//! `RecordingObserver`) implements `operant-core`'s enum; and
//! `operant-plugins` and `operant-config` — the two crates that DO depend on
//! `operant-api` — never reference `ObserverEvent` at all. The only two
//! construction sites of the api variant are inside its own
//! `observer_event_and_metric_are_cloneable` unit test.
//!
//! So it is not "a second surface that also needs the fields"; it is **a dead
//! enum with a live name**, and nothing needs changing there. Recorded because
//! a reader grepping `ObserverEvent::ToolCall` will hit it and cannot tell
//! from the grep whether it is an unfinished site of this change or an
//! abandoned one.
//!
//! **Why this pins the seam and not the helper:** the cap's constant
//! (`MAX_TOOL_RESULT_LEN`) and its helper (`truncate_tool_result`) are both
//! private to the `agent` module, and a unit test of a private helper would
//! still pass if the call site moved. This test drives a real agent turn
//! through `OperantAgent::run` and reads the **arguments the model client was
//! handed** — the only surface that matters, because that is what gets paid
//! for and sent. An oversized schema must be cut before it arrives there.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// A tool whose `parameters` is enormous — the shape of an MCP server that
/// advertises a 2,000-property schema.
const HUGE: &str = "mcp_huge_schema_probe";

/// Field count in the fixture schema. Chosen so the payload is ~500 KB: far
/// above any plausible cap, so the assertion below cannot pass by luck.
const FIELDS: usize = 2000;

/// The ceiling on what may reach the model. Generous on purpose: this test
/// asserts the BOUND EXISTS and is far below the input, not that the operator
/// picked a particular number. 4 KB of schema plus room for the JSON envelope
/// and a truncation marker.
/// The ceiling on what may reach the model. Generous on purpose: this test
/// asserts the BOUND EXISTS and is far below the input, not that the operator
/// picked a particular number. 4 KB of schema plus room for the JSON envelope
/// and a truncation marker.
const CEILING: usize = 20_000;

/// The minimum growth between the pre-tool request and the post-tool request,
/// for the truncated schema to count as PRESENT rather than dropped.
///
/// `provisional:` per AGENTS.md — this number came from observing the two
/// regimes, not from a property of the system.
///
/// The regimes, re-measured 2026-10-31 and printed by the test itself
/// (`--nocapture` shows `[growth] pre-tool=... post-tool=... delta=...`) so the
/// figures below are checkable rather than merely asserted:
///
/// * **real fixture**: 18,259 -> 19,277, **+1018**, deterministic across runs.
/// * **vacuous fixture** (schema shrunk to a single field so it no longer
///   dominates the request): 18,259 -> 18,646, **+387**, also deterministic.
///
/// So its only job is to sit strictly above the vacuous regime and at or below
/// the real one. The bounds were then PINNED BY PROBING rather than deduced,
/// because I got them wrong the first time I wrote them:
///
/// * floor 387 against the vacuous fixture **PASSES** (387 >= 387), so 387 is
///   NOT valid even though it equals the vacuous measurement — the check is
///   `>=`, so the boundary value itself is accepted.
/// * floor 388 against vacuous **FAILS** and against real **PASSES**.
/// * floor 1018 against real **PASSES** (1018 >= 1018), so the upper bound is
///   INCLUSIVE, not exclusive.
/// * floor 1019 against real **FAILS** with "grew by only 1018 chars".
///
/// The measured working range is therefore **388 <= floor <= 1018**. I first
/// published `387 <= floor <= 1018` — both ends wrong, the lower one by
/// including the case that fails to discriminate and the upper one by
/// implying an exclusive bound that is inclusive. Writing a range down from
/// two measurements without probing the endpoints is how a bound gets named
/// that does not hold at its own edge.
///
/// **AND AN EARLIER CORRECTION, kept because the chain is the lesson.** Before
/// that I published the vacuous regime as 11 chars (18,259 -> 18,270) and the
/// range as `11 < floor <= 1018`. The 11 was real but belonged to a DIFFERENT
/// probe — the tiny-tool-result one — and was carried forward after I changed
/// the probe to shrinking the schema. So this bound has now been wrong twice,
/// and each time the fix corrected one thing while leaving another asserted
/// from a stale number. Each correction below names the wrong figure rather
/// than silently replacing it.
///
/// 1017 sits inside the pinned range. (Its earlier values were 512 and, before
/// that, an arithmetic guess; those are this constant's history, not its
/// current state.)
///
/// **HOW DRIFT-PROOF THE RANGE ACTUALLY IS, measured rather than assumed.**
/// The advice here used to say "if a fixture change moves either regime,
/// re-measure", which implied the real regime moves with the fixture. Two
/// probes say it does not, and the second one corrected the first:
///
/// * Doubling the per-field schema text (200 -> 400 chars) leaves the growth at
///   **exactly 1018**. So the regime is not driven by schema size.
/// * Doubling `MAX_TOOL_RESULT_LEN` (4,096 -> 8,192, `agent/mod.rs:1114`) ALSO
///   leaves it at exactly 1018. So the regime is not anchored to that cap either
///   — my first reading of those two results was wrong, and this bullet is the
///   correction.
///
/// What actually binds is the **spend guard** at `agent/compress.rs:587`, whose
/// limit is `(context_window - RESPONSE_RESERVE_TOKENS) * max_tool_result_share`
/// — computed from the token budget, not from either char cap. That is why both
/// char-cap mutations were no-ops: the payload was already inside the guard's
/// token limit and neither char cap was the binding bound.
///
/// So: if the numbers change, suspect `context_window`, `max_tool_result_share`,
/// or `RESPONSE_RESERVE_TOKENS`. Re-pin by probing the endpoints rather than by
/// adjusting the prose.
const MIN_GROWTH: usize = 1017;

struct HugeSchemaTool;

#[async_trait]
impl OperantTool for HugeSchemaTool {
    fn name(&self) -> &str {
        HUGE
    }

    fn description(&self) -> &str {
        "Advertises an enormous schema (drives the cap)."
    }

    fn schema(&self) -> operant_core::schema::ToolSchema {
        let mut props = serde_json::Map::new();
        for i in 0..FIELDS {
            props.insert(
                format!("field_{i}"),
                serde_json::json!({"type": "string", "description": "x".repeat(200)}),
            );
        }
        operant_core::schema::ToolSchema::new(
            HUGE,
            "Advertises an enormous schema (drives the cap).",
            serde_json::json!({"type": "object", "properties": props}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        // Return the tool's OWN enormous parameters, which is what
        // tool_describe does for a deferred tool.
        let schema = <Self as OperantTool>::schema(self);
        ToolResult::success(
            HUGE,
            serde_json::json!({
                "name": schema.name,
                "description": schema.description,
                "parameters": schema.parameters,
            }),
        )
    }
}

/// The model client: records the serialized byte length of every request's
/// message list it is handed — the thing that actually gets sent to the
/// provider — and answers a tool call, then a final answer.
struct RecClient(Arc<RecordingClient>);

#[async_trait]
impl ModelClient for RecClient {
    fn provider_name(&self) -> &str {
        "recording"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let bytes: usize = request
            .messages
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        if let Ok(mut seen) = self.0.seen.lock() {
            seen.push(bytes);
        }
        let n = self
            .0
            .calls
            .lock()
            .map(|mut c| {
                *c += 1;
                *c
            })
            .unwrap_or(1);
        // First answer: call the huge-schema tool. Second: a final answer.
        Ok(if n == 1 {
            tool_call_response()
        } else {
            text_response()
        })
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Config("streaming not used in this test".to_string()))
    }
}

/// Holds the recorded request sizes and the call counter.
struct RecordingClient {
    seen: Mutex<Vec<usize>>,
    calls: Mutex<usize>,
}

impl RecordingClient {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            calls: Mutex::new(0),
        })
    }
}

fn tool_call_response() -> ChatResponse {
    ChatResponse {
        id: "r".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Some(vec![ToolCallDelta {
                    index: 0,
                    id: Some("call-1".to_string()),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: HUGE.to_string(),
                        arguments: "{}".to_string(),
                    }),
                }]),
            },
            finish_reason: Some("tool_calls".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

fn text_response() -> ChatResponse {
    ChatResponse {
        id: "r".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some("done".to_string()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

fn config() -> AgentConfig {
    AgentConfig {
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
        context_semantic_compaction: false,
        model: "demo".to_string(),
        max_trajectories: 0,
        max_iterations: 3,
        tool_timeout: Duration::from_secs(30),
        request_timeout: Duration::from_secs(30),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_healing_attempts: 1,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
        loop_detection_enabled: false,
        approval_mode: "off".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 1,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
        ponytail: Default::default(),
    }
}

#[tokio::test]
async fn an_oversized_tool_schema_is_cut_before_it_reaches_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register(HugeSchemaTool).await.unwrap();
    let client = RecordingClient::new();

    let agent = OperantAgent::new(
        config(),
        Box::new(RecClient(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(dir.path().join("db.sqlite")).unwrap()),
    );

    agent
        .run("use the probe tool".to_string())
        .await
        .expect("the turn must complete");

    let sizes = client.seen.lock().unwrap().clone();
    assert!(
        sizes.len() >= 2,
        "expected the agent to ask the model at least twice (tool call, then \
         final answer) so the tool result is in the second request — got {sizes:?}"
    );

    // The last request is the one that carries the tool result.
    let with_result = *sizes.last().unwrap();
    assert!(
        with_result < CEILING,
        "the request carrying the tool result was {with_result} chars. A ~495 KB \
         schema reaching the model means the result-size bounds at \
         agent/run.rs:2101 (the absolute cap) and :2130 (the spend guard) both \
         stopped applying — that is the defect this guard exists for, and it \
         is NOT fixed by adding a cap inside tool_describe"
    );

    // The control: the FIRST request, before any tool ran, is bounded too.
    // Without this, an agent that sent nothing at all would pass.
    assert!(
        sizes[0] < CEILING,
        "even the first request must be bounded — got {} chars",
        sizes[0]
    );

    // The discriminating assertion, and the reason this test is not vacuous.
    // A CEILING check alone would pass just as happily if the agent dropped the
    // result entirely (18259 vs 18270 is an 11-char difference that proves
    // nothing). So require the growth to be MATERIAL: the ~4 KB truncated
    // payload really must be in that request, not just some tool name.
    //
    // Measured with the real fixture: 18,259 -> 19,277 (+1018), driven by the
    // truncated schema. Measured with the vacuity fixture (FIELDS shrunk to 1 so
    // the schema no longer dominates): 18,259 -> 18,646 (+387).
    //
    // Printed below rather than only asserted, because the doc comment on
    // MIN_GROWTH publishes these figures and a published number nothing emits
    // cannot be re-checked without editing the source. `--nocapture` shows them.
    let growth = with_result - sizes[0];
    eprintln!(
        "[growth] pre-tool={} post-tool={with_result} delta={growth} (floor={MIN_GROWTH})",
        sizes[0]
    );
    // Drift check, asserted HERE rather than in a second test. My first attempt
    // put this in a separate test reading a shared AtomicUsize the measuring
    // test writes, and that guard was worthless: rust runs tests in parallel,
    // so the second test usually read the atomic as 0 before this one wrote it
    // and took its early-return path. It passed with a FABRICATED measured
    // value of 999 against REAL_GROWTH of 1018. A guard a race can silently
    // disable is not a guard, so the check moved to where the value is produced.
    assert_eq!(
        growth, REAL_GROWTH,
        "the real regime DRIFTED: this run measured {growth} but REAL_GROWTH \
         publishes {REAL_GROWTH}. The binding bound is the SPEND GUARD at \
         agent/compress.rs:587 - limit = (context_window - \
         RESPONSE_RESERVE_TOKENS) * max_tool_result_share - not either char cap: \
         probed, doubling MAX_TOOL_RESULT_LEN (4,096 -> 8,192) leaves the growth \
         at exactly 1018, as does doubling the per-field schema text. A real \
         drift means context_window, max_tool_result_share or \
         RESPONSE_RESERVE_TOKENS moved. Re-measure, then update REAL_GROWTH, the \
         doc comment, and re-probe the endpoints TOGETHER"
    );
    assert!(
        growth >= MIN_GROWTH,
        "the request carrying the tool result grew by only {growth} chars over \
         the first request ({} -> {with_result}). The ~495 KB schema was either \
         dropped or never truncated into a bounded payload. A CEILING check alone \
         cannot tell those apart from a real bounded result, which is exactly how \
         this test passed vacuously until this floor was added",
        sizes[0]
    );
}

// ---------------------------------------------------------------------------
// The floor's own range, pinned as executable facts rather than as prose.
//
// The doc comment above publishes a working range for MIN_GROWTH. A range in
// prose is a claim; a range in a const assertion is a fact that fails to
// compile when it stops being true. This is here because the range was wrong
// TWICE while living only in prose, and both times the error survived a
// "correction" pass that fixed one end and left the other.
//
// These numbers are the two measured regimes. If a fixture change moves either
// one, this test stops compiling and names the bound that moved - which is the
// point. Re-pin by re-measuring, then update these constants and the doc
// comment together.
///
// Vacuous regime: schema shrunk to one field, so it no longer dominates the
// request. Printed by the main test under `--nocapture`.
const VACUOUS_GROWTH: usize = 387;
///
/// Real regime: the 2,000-field schema, truncated into the request.
const REAL_GROWTH: usize = 1018;

/// The floor must sit strictly above the vacuous regime. It is compared with
/// `>=`, so a floor equal to the vacuous growth ACCEPTS the vacuous case - the
/// exact off-by-one that made my first published range wrong.
const _: () = assert!(MIN_GROWTH > VACUOUS_GROWTH);
/// The floor must sit at or below the real growth, or it rejects the real case.
const _: () = assert!(MIN_GROWTH <= REAL_GROWTH);
/// And it must actually separate them, rather than both bounds being satisfied
/// by some value that discriminates neither.
const _: () = assert!(MIN_GROWTH > VACUOUS_GROWTH && MIN_GROWTH <= REAL_GROWTH);

#[test]
fn the_published_floor_range_matches_the_measured_regimes() {
    // The same two facts, at runtime, with a message that names what to do. The
    // const assertions above fail at compile time; this one explains itself.
    assert!(
        VACUOUS_GROWTH < MIN_GROWTH && MIN_GROWTH <= REAL_GROWTH,
        "MIN_GROWTH ({MIN_GROWTH}) no longer sits between the measured regimes \
         ({VACUOUS_GROWTH} < floor <= {REAL_GROWTH}). A fixture change moved one \
         of them: re-measure both with `cargo test -p operant-core --test \
         oversized_schema_cannot_reach_the_model -- --nocapture`, re-pin the \
         endpoints by setting the floor to each boundary value, then update \
         VACUOUS_GROWTH/REAL_GROWTH and the doc comment TOGETHER"
    );
}

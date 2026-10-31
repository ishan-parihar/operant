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
//! 20,000 — nothing bounded, nothing missing, nothing observed. The growth
//! assertion below (>= 512 chars) closes that: with the real fixture the
//! request grows 18,259 -> 19,277 (+1018, the truncated schema), and with the
//! vacuity mutant 18,259 -> 18,270 (+11, just a tool name). Both directions
//! now fail by name. The floor sits between those regimes (~100x apart) and is
//! marked `provisional:` in source because that is what it is derived from.
//!
//! **A NOTE ON SCOPE, so the next reader does not go looking for a second
//! thing to fix.** `crates/operant-api/src/observability_traits.rs` defines
//! its OWN `ObserverEvent` enum with a `ToolCall { tool, duration, success }`
//! and no byte fields. It is a genuinely disconnected type: `operant-core`
//! has no dependency on `operant-api`, and only `operant-plugins` and
//! `operant-config` consume it. The live observer surface is `operant-core`'s,
//! so that parallel enum is out of this guard's scope by design — recorded
//! here because a reader grepping `ObserverEvent::ToolCall` will hit it and
//! may otherwise mistake it for an unfinished site of the same change.
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
/// regimes, not from a property of the system: a vacuous run (tool returns a
/// tiny payload) grows by 11 chars, and the real run grows by 1018. Its only
/// job is to sit between those, and anything from ~64 to ~511 does that. 512
/// was chosen near the middle with ~50% headroom under the real value. If a
/// fixture change moves the real number, re-measure rather than trusting this.
const MIN_GROWTH: usize = 512;

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
    // truncated schema. Measured with the vacuity mutant (tool returns a tiny
    // result): 18,259 -> 18,270 (+11) — far below this floor.
    let growth = with_result - sizes[0];
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

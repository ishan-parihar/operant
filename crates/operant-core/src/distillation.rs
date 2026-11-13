//! State distillation for durable long-term memory.

use std::collections::HashSet;

use std::sync::Arc;

use tracing::debug;

use crate::agent::{ChatRequest, ModelClient};
use crate::client::{Message, Role};
use crate::error::{Error, Result};

const DISTILLATION_SYSTEM_DIRECTIVE: &str = "Analyze the conversation history. Extract ONLY permanent, durable knowledge, rules, and user preferences. Ignore ephemeral bugs, narrative, or code snippets. Output ONLY a JSON array of strings containing these concise facts.";

/// Distill durable facts from a completed session and persist them to
/// long-term memory — through the ONE backend (plan 11C3): the memory-wire
/// provider. The old MemoryManager write path is gone with the
/// <long_term_memory> prompt block; distilling into a file nobody reads
/// was a dead write.
pub async fn distill_session_to_memory(
    client: Arc<dyn ModelClient>,
    model: String,
    history: Vec<Message>,
) -> Result<usize> {
    if history.is_empty() {
        return Ok(0);
    }

    // Resolve the store BEFORE the LLM call: with no memory-wire provider
    // attached the distilled facts have nowhere to go, and the call would be
    // pure spend (and, in tests, would eat a scripted response meant for the
    // next turn — BUGS H-5).
    let Some(provider) = crate::tools::memory_tools::active_memory_provider().await else {
        debug!("no memory-wire provider attached; skipping session distillation");
        return Ok(0);
    };

    let transcript = format_history_for_distillation(&history);
    if transcript.trim().is_empty() {
        return Ok(0);
    }

    let messages = vec![
        Message::system(DISTILLATION_SYSTEM_DIRECTIVE),
        Message::user(format!("Conversation history:\n{}", transcript)),
    ];
    let request = ChatRequest::new(&model, messages);
    let response = client.chat(request).await?;
    let content = response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .ok_or_else(|| Error::ParseResponse("Distillation response had no content".to_string()))?;

    let facts = parse_distilled_facts(&content)?;
    let mut stored = 0;
    let mut seen = HashSet::new();

    for fact in facts {
        let trimmed = fact.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }

        // (plan 11C3) retain into the memory-wire provider's active
        // instance — the same store the tools and recall use.
        provider.retain_fact(trimmed, &["distilled".to_string(), "long_term".to_string()]);
        stored += 1;
    }

    debug!(stored, "Distilled session facts into long-term memory");
    Ok(stored)
}

fn parse_distilled_facts(raw: &str) -> Result<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    // Strip markdown code fences if present.
    let json_str = if trimmed.starts_with("```") {
        let inner = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        inner.trim_end_matches("```").trim()
    } else {
        trimmed
    };

    if json_str.is_empty() {
        return Ok(vec![]);
    }

    // The payload is the ARRAY; the model often wraps it in prose ("Here are
    // the facts: [...]") or in a fence the stripper above did not catch
    // (a fence followed by a sentence). MEASURED on the operator's live
    // gateway log: 27 distillations failed with "expected value at line 1
    // column 1" — the response did not START with JSON, so the whole
    // distillation (the facts that make a session self-improving) was
    // dropped every time.
    //
    // Take the first '[' that opens a JSON array and the last ']'. A leading
    // '[' is tried first; a prose bracket before the array is tried after,
    // so "the facts [see below]: [...]" still parses.
    if let Ok(facts) = serde_json::from_str::<Vec<String>>(json_str) {
        return Ok(facts);
    }
    let Some(end) = json_str.rfind(']') else {
        return Err(Error::ParseResponse(
            "Invalid distillation JSON: no JSON array of facts in the response".to_string(),
        ));
    };
    for start in json_str.match_indices('[').map(|(i, _)| i).take(4) {
        if end <= start {
            break;
        }
        if let Ok(facts) = serde_json::from_str::<Vec<String>>(&json_str[start..=end]) {
            return Ok(facts);
        }
    }

    Err(Error::ParseResponse(format!(
        "Invalid distillation JSON: no JSON array of facts in a {} character response",
        json_str.len()
    )))
}

fn format_history_for_distillation(history: &[Message]) -> String {
    history
        .iter()
        .filter(|message| message.role != Role::System && !message.content.trim().is_empty())
        .map(|message| format!("{}: {}", message.role.as_str(), message.content.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mockito::Server;

    use super::*;
    use crate::client::ClientConfig;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_distillation_{}_{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir should be created");
        dir
    }

    #[test]
    fn parses_strict_json_array() {
        let facts = parse_distilled_facts("[\"User prefers concise answers\"]").unwrap();
        assert_eq!(facts, vec!["User prefers concise answers"]);
    }

    #[test]
    fn parses_an_array_wrapped_in_prose() {
        // MEASURED on the operator's live gateway log: 27 distillations failed
        // with "expected value at line 1 column 1" because the model wrapped
        // the array in a sentence.
        let facts = parse_distilled_facts(
            "Here are the durable facts from that session:\n[\"User prefers concise answers\"]",
        )
        .unwrap();
        assert_eq!(facts, vec!["User prefers concise answers"]);
    }

    #[test]
    fn parses_an_array_inside_a_fence_followed_by_prose() {
        let facts = parse_distilled_facts("```json\n[\"a fact\"]\n```\nThat is all.").unwrap();
        assert_eq!(facts, vec!["a fact"]);
    }

    #[test]
    fn rejects_non_array_distillation_output() {
        let error = parse_distilled_facts("{\"fact\":\"nope\"}").unwrap_err();
        assert!(error.to_string().contains("Invalid distillation JSON"));
    }

    #[tokio::test]
    async fn distills_facts_and_persists_memory() {
        // A DEDICATED server, not one from mockito's shared pool: the pool
        // hands a server to one test and returns it to the pool when that
        // test ends, so a later test can be given the same server — and, under
        // high parallelism, a server whose listener has already closed. The
        // symptom was several unrelated tests failing together on one shared
        // port.
        let mut server = Server::new_with_opts_async(mockito::ServerOpts {
            port: 0,
            ..Default::default()
        })
        .await;
        let _mock = server
            .mock("POST", "/v1/chat/completions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "distill_1",
                    "object": "chat.completion",
                    "created": 0,
                    "model": "demo",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "[\"User prefers concise answers\", \"Project uses Rust\"]"
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2
                    }
                })
                .to_string(),
            )
            .create_async()
            .await;

        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let raw_client = OpenAIClient::new(ClientConfig {
            base_url: format!("{}/v1", server.url()),
            api_key: None,
            timeout: Duration::from_secs(5),
            model_request_timeouts: Default::default(),
            rate_limit: Default::default(),
            prompt_cache_ttl: Default::default(),
        });
        let client: Arc<dyn ModelClient> = Arc::new(OpenAIModelClient::new(raw_client));
        // Cross-module lock: this test attaches the process-global provider.
        let _guard = crate::tools::memory_tools::PROVIDER_TEST_GUARD.lock().await;
        let dir = test_dir("persist");
        // (plan 11C3) distillation retains through the active provider.
        let provider =
            std::sync::Arc::new(crate::memory_wire::MemoryWireProvider::new(dir.clone()).unwrap());
        crate::tools::memory_tools::set_active_memory_provider(provider.clone()).await;

        let stored = distill_session_to_memory(
            client,
            "demo".to_string(),
            vec![
                Message::user("Please be concise in future answers."),
                Message::assistant("Understood."),
            ],
        )
        .await
        .unwrap();

        assert_eq!(stored, 2);
        // Both distilled facts are recallable from the shared store.
        let hits = provider.recall_text("concise", 5);
        assert!(!hits.trim().is_empty(), "distilled fact landed: {hits:?}");

        crate::tools::memory_tools::clear_active_memory_provider_for_tests().await;
        let _ = std::fs::remove_dir_all(dir);
    }
}

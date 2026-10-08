// Vendored from jcode (crates/operant-compaction-core/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.

/// Approximate chars per token for estimation
pub const CHARS_PER_TOKEN: usize = 4;

/// Best-effort context size (tokens) from a provider usage report.
///
/// Providers disagree on what `input_tokens` means:
/// - **Split accounting** (Anthropic-style): `input_tokens` is only the
///   *uncached* remainder; cache reads/writes are separate counters, so the
///   real context size is `input + cache_read + cache_creation`.
/// - **Subset accounting** (OpenAI-style): `input_tokens` (`prompt_tokens`)
///   already includes cached tokens; `cached_tokens` is a subset and must NOT
///   be added again.
///
/// This is the single source of truth for that heuristic. Both the sidebar
/// context figure and the compaction manager's observed-token feed must use it
/// so the two never disagree (issue #441). When in doubt, avoid over-counting
/// unless there is strong evidence of split accounting.
pub fn effective_context_tokens_from_usage(
    provider_name: &str,
    input_tokens: u64,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
) -> u64 {
    let cache_read = cache_read_input_tokens.unwrap_or(0);
    let cache_creation = cache_creation_input_tokens.unwrap_or(0);
    let provider_name = provider_name.to_lowercase();

    // OpenAI cache writes, like reads, are subsets of the inclusive input count.
    if provider_name.contains("openai") || provider_name.contains("codex") {
        return input_tokens;
    }

    let split_cache_accounting = provider_name.contains("anthropic")
        || provider_name.contains("claude")
        || cache_creation > 0
        || cache_read > input_tokens;

    if split_cache_accounting {
        input_tokens
            .saturating_add(cache_read)
            .saturating_add(cache_creation)
    } else {
        input_tokens
    }
}

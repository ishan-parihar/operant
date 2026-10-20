//! Context management — tiered eviction + decay curve rendering.
//!
//! Ports the key techniques from `cortexkit/magic-context` (a TypeScript
//! OpenCode plugin) into native Rust. operant previously had zero live
//! context management — `build_messages` concatenated system + memory +
//! skills + context_files + full conversation history every iteration,
//! which meant any long-running session would eventually exceed the
//! context window and 400-error.
//!
//! ## Techniques ported from magic-context:
//!
//! 1. **Tiered eviction** (`evict_to_budget`): when the message array
//!    exceeds a token budget, evict oldest-first within tiers:
//!    - T3 (lowest priority): tool results — large, ephemeral, replaceable
//!    - T2 (medium priority): assistant reasoning/thinking — verbose
//!    - T1 (highest priority): user messages + assistant final answers
//!      System messages are never evicted.
//!
//! 2. **Decay curve** (`decay_render`): older messages are rendered into
//!    progressively shorter summaries based on age + importance. The
//!    formula is `H = H50 * 2^((I-50)/D) / max(p, 0.10)` where H50 is
//!    the half-life at 50% importance, D is the decay constant, and p
//!    is the message importance (0-1). No LLM calls — pure deterministic
//!    truncation.
//!
//! 3. **Token estimation** (`estimate_tokens`): char-count / 4 heuristic
//!    (CJK-aware via the iter-18 fix). This is a placeholder until a
//!    real tokenizer (tiktoken-rs) is wired in.

use crate::client::{Message, Role};

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Estimate the token count of a string. Uses char-count / 4 (CJK-aware
/// since iter-18 switched from byte count). This is a rough heuristic —
/// a real tokenizer (tiktoken-rs) would be more accurate but adds a
/// dependency. The heuristic is good enough for budgeting decisions
/// (eviction thresholds), not for exact billing.
pub fn estimate_tokens(text: &str) -> usize {
    // char count / 4 is the standard heuristic for English text.
    // For CJK text each char is ~1 token, so the heuristic overestimates
    // for English and underestimates for CJK — acceptable for budgeting.
    text.chars().count().div_ceil(4)
}

/// Estimate the token count of a single message (role + content).
///
/// The count covers the WHOLE message the provider sees — not just its
/// text. A tool-call turn's `content` is EMPTY by construction (the text
/// is suppressed so the provider sees a pure tool-call turn), and the
/// payload is in `tool_calls[].function.arguments`; the reasoning is in
/// `msg.reasoning` and is serialised back to the provider on the next
/// request. Counting only `content` charged 4 tokens for a turn that
/// carried 100 KB of tool arguments — so the eviction was blind to
/// exactly the messages that carry the most payload, and the request the
/// provider rejected was over-budget by an amount the estimator never
/// saw.
pub fn estimate_message_tokens(msg: &Message) -> usize {
    // 4 tokens overhead per message (role tags, separators) — matches
    // OpenAI's pricing model.
    let mut tokens = 4 + estimate_tokens(&msg.content);

    // The tool-call arguments. A `write_file` with a 100 KB body is
    // ~25 000 real tokens.
    if let Some(calls) = msg.tool_calls.as_ref() {
        for call in calls {
            tokens += estimate_tokens(&call.function.arguments);
            // The tool name and the id travel with the call.
            tokens += estimate_tokens(&call.function.name);
            tokens += estimate_tokens(&call.id);
        }
    }

    // The reasoning, which IS sent back to the provider.
    if let Some(reasoning) = msg.reasoning.as_ref() {
        tokens += estimate_tokens(reasoning);
    }

    tokens
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

// ---------------------------------------------------------------------------
// output-token fitting (omp `output-budget.ts` parity, iter-792)
// ---------------------------------------------------------------------------

/// Smallest output cap the fitter considers worth requesting (omp
/// `MIN_FITTED_OUTPUT_TOKENS`). Not a hard floor here: when the room itself
/// shrinks below this, the fitter still requests the room — a 400 rejection
/// (what requesting MIN inside a smaller window produces) loses the whole
/// call, while a tiny admissible cap lets compaction rescue the next turn.
/// The constant documents the point at which a session should compact
/// rather than keep fitting.
pub const MIN_FITTED_OUTPUT_TOKENS: u32 = 1024;

/// Absolute headway subtracted from the remaining room before fitting (omp
/// `OUTPUT_FIT_HEADWAY_TOKENS`, measured there against a 262,144-token host:
/// fitted caps landed +11 to +42 tokens over the true count and were rejected
/// 400 — 64 covers the observed drift with margin). Covers chat-template
/// framing and reasoning wrappers no local count can see.
pub const OUTPUT_FIT_HEADWAY_TOKENS: u32 = 64;

/// Local prompt estimates are padded by 1/this before sizing the cap (omp
/// `PROMPT_ESTIMATE_MARGIN_DIVISOR`): the local chars/4 heuristic can
/// disagree with the provider's tokenizer by a few percent, and
/// undercounting reproduces the request-rejection this fitter exists to
/// prevent. A provider-REPORTED anchor is exact and skips this margin.
const PROMPT_ESTIMATE_MARGIN_DIVISOR: u32 = 10;

/// Fit an output-token cap so `prompt + output` stays inside the model's
/// context window.
///
/// Chat-Completions providers (OpenAI, DeepSeek, vLLM, aroute) reject a
/// request whose prompt tokens + `max_tokens` exceed the window — every
/// such request fails with a 400 BEFORE generation starts, long before
/// compaction would rescue it, and side turns (advisor, recap, background
/// review) have no overflow-recovery path at all. omp derived this fitter
/// from exactly that incident class (`output-budget.ts`); the constants are
/// theirs, measured, and cited in provenance above.
///
/// * `window` — the model's context window (0 or absent = unknown → no fit).
/// * `prompt_tokens` — provider-reported prompt size when one is fresh
///   else the local `estimate_total_tokens` heuristic. NOTE: a reported
///   count is exact for the PREVIOUS request, so the caller must not treat
///   it as the size of the request it is about to send — the agent's
///   `estimate_current_tokens` takes the larger of the two for that
///   reason.
/// * `prompt_is_reported` — true when `prompt_tokens` came from the
///   provider's own usage report (skip the 10% margin); false when it is a
///   local estimate (apply it).
///
/// Returns the fitted cap, or `None` when no fitting is possible/needed
/// (no window known, prompt unknown, or the requested cap already fits).
/// Callers apply the result as `max_tokens = fitted` on the request.
pub fn fit_output_tokens(
    requested: Option<u32>,
    window: usize,
    prompt_tokens: usize,
    prompt_is_reported: bool,
) -> Option<u32> {
    if window == 0 || prompt_tokens == 0 {
        return None;
    }
    let padded = if prompt_is_reported {
        prompt_tokens
    } else {
        prompt_tokens + prompt_tokens / PROMPT_ESTIMATE_MARGIN_DIVISOR as usize
    };
    let room = window
        .saturating_sub(padded)
        .saturating_sub(OUTPUT_FIT_HEADWAY_TOKENS as usize);
    let room_u32 = u32::try_from(room).ok()?;
    match requested {
        // No cap requested: providers default to their own (often huge)
        // max_tokens, which is exactly the rejection shape. Fit to the room.
        None => Some(room_u32),
        // Requested cap already fits: leave it untouched — the caller's
        // preference is respected, zero behavior change for small prompts.
        Some(req) if req <= room_u32 => None,
        Some(_) => Some(room_u32),
    }
}

// ---------------------------------------------------------------------------
// tool_use / tool_result correlation
// ---------------------------------------------------------------------------

/// A message that *requests* tool calls: an assistant turn carrying at least
/// one [`ToolCall`](crate::client::ToolCall). Each `ToolCall::id` is the id the provider expects to see
/// answered by a [`Message::tool_call_id`].
pub fn is_tool_use(msg: &Message) -> bool {
    msg.role == crate::client::Role::Assistant
        && msg
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
}

/// For every index, the first index of the tool-use group it belongs to.
///
/// A group is an assistant `tool_use` turn plus the `Role::Tool` turns that
/// answer it — the shape every OpenAI-compatible provider emits and every
/// strict provider validates. Messages inside one group must be kept or
/// dropped **together**: a `tool_use` with no `tool_result` is malformed, and
/// so is a `tool_result` with no `tool_use`.
///
/// A lone `Role::Tool` message (no preceding `tool_use` in the slice) is its own
/// group — the input is already malformed there, and this pass must not make it
/// worse by growing the blast radius.
pub fn tool_group_starts(messages: &[Message]) -> Vec<usize> {
    let mut starts = vec![0usize; messages.len()];
    let (mut start, mut i) = (0usize, 0usize);
    while i < messages.len() {
        if is_tool_use(&messages[i]) {
            while i + 1 < messages.len() && messages[i + 1].role == crate::client::Role::Tool {
                i += 1;
            }
        }
        for slot in starts.iter_mut().take(i + 1).skip(start) {
            *slot = start;
        }
        start = i + 1;
        i += 1;
    }
    starts
}

/// Pick a compaction cut that can never orphan a `tool_use` from its
/// `tool_result`.
///
/// Returns the **exclusive end index of the surviving prefix**:
/// `messages[..k]` is kept and `messages[k..]` is dropped. `budget_tokens` is
/// the ceiling for the *kept* portion, so a larger budget can only move the cut
/// **later**.
///
/// Guarantees, each covered by a test in this module:
///
/// - **No orphan, in either direction.** The cut only ever lands on a group
///   boundary, so a `tool_use` and the `tool_result`s answering it are always
///   kept or dropped together.
/// - **Monotonic in budget.** The walk visits groups left to right and its only
///   budget-dependent branch is `kept + cost > budget_tokens`, which is
///   monotone in `budget_tokens`; so for `b2 >= b1`
///   `safe_compaction_cutoff(m, b2) >= safe_compaction_cutoff(m, b1)`.
/// - **All-tool-pairs history.** Every message belongs to exactly one group, so
///   the cut still lands on a boundary. `k` strictly advances each iteration, so
///   the loop always terminates — no panic, no spin.
/// - **Documented fallback.** If the *first* group alone blows the budget there
///   is no cut that both fits and stays whole, so this returns `0`: no prefix
///   survives. `0` is the natural end of the same left-to-right scan (it is also
///   the answer for `budget_tokens == 0`), which is what keeps the monotonicity
///   guarantee true — a "keep everything" answer here would report a budget that
///   was never met and would move the cut *earlier* as the budget grew. Callers
///   that need a head+tail shape should use the group-aware eviction in
///   [`evict_to_budget`], which keeps the newest turns instead of the oldest.
pub fn safe_compaction_cutoff(messages: &[Message], budget_tokens: usize) -> usize {
    let n = messages.len();
    if n == 0 {
        return 0;
    }
    let starts = tool_group_starts(messages);

    let mut kept = 0usize;
    let mut k = 0usize;
    while k < n {
        // Advance to the end of the group starting at `k` (groups are contiguous).
        let mut end = k + 1;
        while end < n && starts[end] == starts[k] {
            end += 1;
        }
        let cost: usize = messages[k..end].iter().map(estimate_message_tokens).sum();
        if kept.saturating_add(cost) > budget_tokens {
            return k; // `k == 0` here: nothing fits, so no prefix survives
        }
        kept += cost;
        k = end;
    }
    n
}

// ---------------------------------------------------------------------------
// Semantic compaction cut
// ---------------------------------------------------------------------------

/// Characters of each group's text handed to the embedder. A group can be a
/// whole tool exchange, so this is capped; 512 chars is plenty for the
/// goal-bearing sentence and it bounds the embedding request.
const SEMANTIC_GROUP_CHARS: usize = 512;

/// Groups embedded per pass. Bounds one pass to a single batch-sized embedding
/// call. Groups past the cap fall back to the recency default — they are the
/// most recent ones, which is what recency already protects.
const SEMANTIC_MAX_GROUPS: usize = 64;

/// The conversation goal a compaction cut is chosen against.
///
/// **What this is: the last `Role::User` turn** — the live task statement.
/// Every earlier user turn is a superseded request. The two alternatives were
/// rejected for concrete reasons, not taste:
///
/// - *A running summary* is a better target in principle, but producing one
///   needs an LLM call on the very path that runs because the context window
///   is nearly exhausted. A compaction pass that can fail to compact is a
///   regression, so no model call is made here.
/// - *The recent tool-result set* is the material being cut, so scoring
///   against it is circular: it would rank tool output by its similarity to
///   other tool output.
///
/// Returns `None` when there is no non-empty user turn, which routes the
/// caller to the recency behaviour.
pub fn compaction_goal(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == crate::client::Role::User && !m.content.trim().is_empty())
        .map(|m| m.content.trim().to_string())
}

/// The outcome of one semantic compaction pass.
///
/// `kept` is a *selection of groups*, not a prefix of `messages`: that is the
/// whole point of the semantic stage. The safety net is applied afterwards and
/// recorded in `head_cut` — see [`semantic_compaction_cutoff`].
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticCut {
    /// Indices into the ORIGINAL `messages` that survive, ascending. Every
    /// tool group is either wholly present or wholly absent.
    pub kept: Vec<usize>,
    /// [`safe_compaction_cutoff`] run over the retained subsequence with the
    /// same budget — the certified exclusive end index into `kept`. The
    /// messages `kept[head_cut..]` are dropped even though the semantic stage
    /// selected them, which is what makes the net load-bearing rather than
    /// decorative.
    pub head_cut: usize,
    /// False when the pass degraded to recency: no embedder, no goal, or an
    /// embedding call that failed. `kept` is then exactly
    /// `0..safe_compaction_cutoff(messages, budget_tokens)`.
    pub semantic: bool,
    /// Mean goal cosine over the kept groups, or `0.0` when `semantic` is
    /// false. A degraded pass reports `0.0` rather than a fabricated number.
    pub goal_similarity: f32,
}

impl SemanticCut {
    /// Materialise the surviving messages, moving out of `messages` so no
    /// message body is ever cloned by the pass itself.
    pub fn apply(self, messages: Vec<Message>) -> Vec<Message> {
        self.kept
            .into_iter()
            .take(self.head_cut)
            .filter_map(|i| messages.get(i).cloned())
            .collect()
    }
}

/// Choose a compaction cut by similarity to the conversation's goal, with
/// [`safe_compaction_cutoff`] as the final safety net.
///
/// ## Why a group *selection*, not a single index
///
/// `safe_compaction_cutoff` is a **prefix** cut: it keeps `messages[..k]` and
/// drops the tail. For a prefix cut, "retained goal relevance" is monotone
/// non-decreasing in `k` — a longer prefix is a superset — so its argmax under
/// any similarity score is always the largest feasible `k`, which is exactly
/// what the budget already computes. A similarity-driven *prefix* cut is
/// therefore provably identical to the recency cut. Scoring only the groups
/// inside `messages[..cap]` is that degenerate case, and it is worse than
/// useless: the head is the oldest content, which is the part a drifted
/// conversation can least afford to keep.
///
/// So the semantic stage selects a **subset of whole groups** across the whole
/// conversation, and returns indices. A group-subset message list is not a
/// novel shape here — [`evict_to_budget`] already returns a non-prefix list
/// (head plus a recency reserve) — and it is where the goal signal has
/// something to do: a conversation that drifted into weather filler and then
/// restated its goal should keep the earlier release-related exchange and drop
/// the drift, not keep the oldest bytes.
///
/// ## Why the safety net is applied *after* the semantic choice
///
/// A semantically better cut that orphans a `tool_use` from its
/// `tool_result` is strictly worse than a recency cut that does not — the
/// provider rejects the malformed request, so the turn is lost entirely rather
/// than degraded. So the net is not advisory, and the two compose in this
/// exact order:
///
/// 1. `cap = safe_compaction_cutoff(messages, budget_tokens)` — computed, and
///    returned verbatim on the degraded path. It is the only answer a pass
///    with no semantic signal may give.
/// 2. **Semantic stage.** Every group in `messages` is scored by cosine against
///    the goal embedding and filled greedily, highest score first, under the
///    same budget. Group 0 is reserved first and can never be dropped,
///    matching the guarantee [`evict_to_budget`] makes about the system
///    prompt. Skipped groups do not abort the fill, so a cheap high-scoring
///    group can still be taken after an expensive one was passed over.
/// 3. **Safety net, last.** `head_cut = safe_compaction_cutoff(&retained,
///    budget_tokens)`. The retained subsequence is group-aligned by
///    construction (whole groups only, in original order), so this lands on a
///    group boundary and can never split a pair; it also re-asserts the budget
///    over the retained subsequence itself, catching anything the greedy fill's
///    integer arithmetic let through. The result is truncated to
///    `kept[..head_cut]`, so the returned selection is *by construction* a
///    prefix the net itself certifies.
///
/// ## Degradation
///
/// `embedder: None`, an empty goal, more than `SEMANTIC_MAX_GROUPS` groups,
/// an embedding error, or a vector that is empty / non-finite / the wrong
/// width all yield `semantic: false` and the pure recency cut `0..cap`.
/// Embeddings are the optional part; a compaction path that failed without
/// them would be a regression, so the degraded path is the *same function*
/// with the semantic stage skipped, not a second implementation.
pub async fn semantic_compaction_cutoff(
    messages: &[Message],
    budget_tokens: usize,
    embedder: Option<&dyn crate::context::Embedder>,
) -> SemanticCut {
    // Computed once, and the sole answer the degraded path may give.
    let cap = safe_compaction_cutoff(messages, budget_tokens);
    let recency = move || SemanticCut {
        kept: (0..cap).collect(),
        head_cut: cap,
        semantic: false,
        goal_similarity: 0.0,
    };

    let Some(embedder) = embedder else {
        return recency();
    };
    let Some(goal) = compaction_goal(messages) else {
        return recency();
    };

    let starts = tool_group_starts(messages);
    // Every group boundary in the conversation, as (start, end) pairs. The
    // whole conversation is scored, not just `messages[..cap]` — see the
    // function doc for why a prefix-restricted search is degenerate.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < messages.len() {
        let mut end = i + 1;
        while end < messages.len() && starts[end] == starts[i] {
            end += 1;
        }
        groups.push((i, end));
        i = end;
    }
    if groups.is_empty() {
        return recency();
    }
    // More groups than we are willing to embed: recency already protects the
    // tail, so degrading is the honest answer rather than a partial embed.
    if groups.len() > SEMANTIC_MAX_GROUPS {
        return recency();
    }

    let group_text = |(s, e): (usize, usize)| -> String {
        let mut out = String::new();
        for m in &messages[s..e] {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(m.content.trim());
            if out.chars().count() >= SEMANTIC_GROUP_CHARS {
                break;
            }
        }
        out.chars().take(SEMANTIC_GROUP_CHARS).collect()
    };

    // One embedding round-trip for the goal plus every group. Any failure here
    // is a degrade, not an error: compaction must not fail because an optional
    // embedding endpoint is down.
    let mut texts = Vec::with_capacity(groups.len() + 1);
    texts.push(goal.clone());
    texts.extend(groups.iter().map(|g| group_text(*g)));
    let vectors = match embedder.embed(&texts).await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, "semantic compaction cut: embed failed, using recency");
            return recency();
        }
    };
    let Some(goal_vec) = vectors
        .first()
        .filter(|v| !v.is_empty() && v.iter().all(|x| x.is_finite()))
    else {
        return recency();
    };

    // Score each group. A non-finite or wrong-width vector scores 0.0 rather
    // than poisoning the sort (cosine of a NaN is NaN and compares Equal to
    // everything, which would make the ordering arbitrary).
    let mut scored: Vec<((usize, usize), f32)> = Vec::with_capacity(groups.len());
    for (gi, g) in groups.iter().enumerate() {
        let score = match vectors.get(gi + 1) {
            Some(v) if v.len() == goal_vec.len() && v.iter().all(|x| x.is_finite()) => {
                crate::context::embedder::cosine_similarity(goal_vec, v).max(0.0)
            }
            _ => 0.0,
        };
        scored.push((*g, score));
    }

    // Greedy fill, best score first. `sort_by` is stable and the group start is
    // the secondary key, so equal scores keep conversation order — a tie never
    // reshuffles the history.
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.0.cmp(&b.0.0))
    });

    let group_cost = |g: (usize, usize)| -> usize {
        messages[g.0..g.1].iter().map(estimate_message_tokens).sum()
    };
    // Reserve group 0 up front: the system prompt / opening turn is never
    // silently dropped, exactly as `evict_to_budget` guarantees. Located by its
    // range, NOT by position — `scored` is already sorted by score, so
    // `scored[0]` is the best match, not the first group.
    let Some((first, first_score)) = scored.iter().find(|(g, _)| g.0 == 0) else {
        return recency();
    };
    let mut kept_tokens = group_cost(*first);
    // Selected GROUPS, as (start, end) ranges. Ranges, not starts: a tool group
    // spans several messages and every one of them must come along.
    let mut selected: Vec<(usize, usize)> = vec![*first];
    let mut sim_sum = *first_score;
    for (g, score) in scored.iter().filter(|(g, _)| g.0 != 0) {
        let cost = group_cost(*g);
        if kept_tokens.saturating_add(cost) > budget_tokens {
            continue; // skip it, keep trying cheaper lower-scoring groups
        }
        kept_tokens += cost;
        selected.push(*g);
        sim_sum += *score;
    }

    // Restore conversation order — a group permutation is not a valid message
    // sequence, and tool_use must stay immediately before its results.
    selected.sort_unstable();
    // Flatten the surviving groups into ascending message indices. `kept` is
    // index-aligned with `retained` by construction (`retained[i]` is
    // `messages[kept[i]]`), which is what makes the truncation below sound.
    let mut kept: Vec<usize> = Vec::with_capacity(kept_tokens);
    for (s, e) in &selected {
        kept.extend(*s..*e);
    }
    let retained: Vec<Message> = kept
        .iter()
        .filter_map(|&i| messages.get(i).cloned())
        .collect();

    // The safety net, applied last. `retained` is group-aligned by
    // construction, so this is a group boundary; it also re-asserts the budget
    // over the retained subsequence itself.
    let head_cut = safe_compaction_cutoff(&retained, budget_tokens);
    kept.truncate(head_cut);

    let count = selected.len().max(1) as f32;
    SemanticCut {
        kept,
        head_cut,
        semantic: true,
        goal_similarity: sim_sum / count,
    }
}

// ---------------------------------------------------------------------------
// Bust permission (plan 06 P1): the gate every history rewrite passes through
// ---------------------------------------------------------------------------

/// Share of the window at which a rewrite pass is forced: the request is too
/// large to keep appending, so folding and the ladder may run.
pub const FORCE_BUST_PERCENT: usize = 85;
/// Share of the window at which a request that no rewrite could bring under
/// the line fails with `Error::ContextExhausted` instead of a provider 400.
pub const CONTEXT_EXHAUSTED_PERCENT: usize = 95;

/// Whether this request may rewrite history. Under `None` the request is the
/// previous request plus appended messages, so the provider's prompt cache
/// keeps hitting. Computed once per request in `build_messages`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BustPermission {
    None,
    /// The cache is cold for a reason outside the rewrite's control (model
    /// or system prompt changed, or idle past the TTL), so a rewrite costs no
    /// extra cache miss.
    Natural(&'static str),
    /// The estimate is at or above `FORCE_BUST_PERCENT` of the window.
    Force,
    /// The user asked for compaction (`/compact`).
    Explicit,
}

/// The previous request, for the natural-bust check.
#[derive(Debug, Clone)]
pub struct BustFingerprint {
    pub model: String,
    pub system: crate::agent::clients::cache_monitor::PrefixDigest,
    pub at: std::time::Instant,
}

/// Why the cache is cold anyway, if it is: the model or system prompt changed
/// since the previous request, or the previous request is older than the TTL.
pub fn natural_bust_reason(
    prev: Option<&BustFingerprint>,
    now: &BustFingerprint,
    ttl: std::time::Duration,
) -> Option<&'static str> {
    let prev = prev?;
    if prev.model != now.model {
        Some("model changed")
    } else if prev.system != now.system {
        Some("system prompt changed")
    } else if now.at.duration_since(prev.at) > ttl {
        Some("prompt cache idle past its TTL")
    } else {
        None
    }
}

/// Pure decision behind [`BustPermission`]. `Explicit` wins, then `Force`,
/// then `natural`.
pub fn bust_permission(
    estimated: usize,
    window: usize,
    natural: Option<&'static str>,
    explicit: bool,
) -> BustPermission {
    if explicit {
        BustPermission::Explicit
    } else if estimated * 100 >= window * FORCE_BUST_PERCENT {
        BustPermission::Force
    } else if let Some(reason) = natural {
        BustPermission::Natural(reason)
    } else {
        BustPermission::None
    }
}

// ---------------------------------------------------------------------------
// Micro-compaction (hermes micro-compaction parity; opt-in via
// `OPERANT_MICRO_COMPACTION`, off by default)
// ---------------------------------------------------------------------------

/// Soft threshold: once the estimated conversation crosses this share of
/// the context window, the oldest foldable exchange folds. Below the 80%
/// preflight threshold by design — micro-compaction works continuously
/// AHEAD of the batch ladder, not as its replacement.
pub const MICRO_COMPACTION_THRESHOLD_PERCENT: usize = 40;
/// Protected head: the frozen prefix + first exchange are never folded.
pub const MICRO_COMPACTION_HEAD_KEEP: usize = 4;
/// Protected recency tail: the last N messages are never folded.
pub const MICRO_COMPACTION_TAIL_KEEP: usize = 8;
/// Character ceiling for the folded gist a marker message carries.
pub const MICRO_COMPACTION_FOLD_MAX_CHARS: usize = 240;
/// Opening of every marker message a fold writes. Lets a later fold recognise
/// an already-folded exchange.
const MICRO_MARKER_PREFIX: &str = "[compacted exchange — earlier conversation folded]";

/// Pure env parse behind the opt-in — testable without touching the
/// process environment (the agent reads it ONCE at construction).
pub fn parse_micro_compaction_env(raw: Option<&str>) -> bool {
    matches!(
        raw.unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// Is micro-compaction armed? `OPERANT_MICRO_COMPACTION` truthy — read
/// once at agent construction (`OperantAgent::micro_compaction`), never
/// probed per call.
pub fn micro_compaction_enabled() -> bool {
    parse_micro_compaction_env(std::env::var("OPERANT_MICRO_COMPACTION").ok().as_deref())
}

/// Fold the oldest complete exchange (a user message through just before
/// the next user message) into a single compact marker message. PURE: the
/// caller owns the threshold decision and the store. Returns the marker
/// text when a fold happened, `None` when nothing is foldable.
///
/// Invariants:
/// - the protected head (`head_keep`) and recency tail (`tail_keep`) are
///   never touched — folding stays strictly below the frozen prefix, or
///   the PromptCacheGuard fires by design;
/// - a fold never splits a tool pair: the exchange's trailing tool
///   results travel with it (the slice boundary extends past consecutive
///   `Tool` messages), so no orphaned half survives;
/// - the folded gist keeps the exchange's last assistant text,
///   head/tail-preserved at `max_chars`.
pub fn micro_compact(
    messages: &mut Vec<Message>,
    head_keep: usize,
    tail_keep: usize,
    max_chars: usize,
) -> Option<String> {
    let len = messages.len();
    let tail_start = len.saturating_sub(tail_keep);
    if len <= head_keep || tail_start <= head_keep {
        return None;
    }
    // The first foldable user message inside the unprotected middle. A marker
    // is a user message too, but it is already a folded exchange: skipping it
    // lets the next fold move past earlier ones instead of stalling on it.
    let start = (head_keep..tail_start).find(|&i| {
        messages[i].role == Role::User && !messages[i].content.starts_with(MICRO_MARKER_PREFIX)
    })?;
    // The exchange ends at the next user message inside the middle window.
    let mut end = (start + 1..tail_start)
        .find(|&i| messages[i].role == Role::User)
        .unwrap_or(tail_start);
    // Tool-pair integrity: if the message just past the boundary is a tool
    // result whose request is inside the slice, extend the fold to include
    // those results (both halves travel together or neither does).
    while end < len && messages[end].role == Role::Tool {
        end += 1;
    }
    if end <= start + 1 && messages[start].role == Role::User {
        // A lone user message with no reply is not a completed exchange.
        return None;
    }

    // The gist: the exchange's LAST assistant text.
    let gist = messages[start..end]
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.content.as_str())
        .unwrap_or("");
    let kept = if gist.chars().count() <= max_chars {
        gist.to_string()
    } else {
        // Head/tail preservation, mirroring the engine's tool-result trim.
        let head_len = max_chars * 2 / 3;
        let tail_len = max_chars - head_len;
        let head_end = gist
            .char_indices()
            .nth(head_len)
            .map(|(i, _)| i)
            .unwrap_or(gist.len());
        let tail_start_b = gist
            .char_indices()
            .rev()
            .nth(tail_len.saturating_sub(1))
            .map(|(i, _)| i)
            .unwrap_or(gist.len());
        if head_end >= tail_start_b {
            gist.chars().take(max_chars).collect()
        } else {
            format!("{} […] {}", &gist[..head_end], &gist[tail_start_b..])
        }
    };
    let marker_text = format!("{MICRO_MARKER_PREFIX} {kept}");
    messages.splice(
        start..end,
        std::iter::once(Message::user(marker_text.clone())),
    );
    Some(marker_text)
}

/// Wave-4 PromptCacheGuard: the frozen prefix (leading run of system
/// messages) must survive every preflight rung byte-identical so
/// provider prompt caches keep hitting across a session's calls.
/// `decay_render` and `evict_to_budget` already preserve system
/// messages by construction; this guard converts that implicit
/// invariant into a checked one — drift from a future rung edit
/// surfaces as a loud warning instead of a silent cache-bust tax.
///
/// Returns `true` when the prefix survived. Drift logs a warning in
/// release and trips a `debug_assert` in test builds.
pub fn prompt_cache_guard(prefix: &[Message], after: &[Message]) -> bool {
    let prefix_len = prefix.len();
    let ok = after.len() >= prefix_len
        && prefix
            .iter()
            .zip(after.iter())
            .all(|(b, a)| b.role == a.role && b.content == a.content);
    if !ok {
        tracing::warn!(
            prefix_len,
            "PromptCacheGuard: frozen prefix drifted across the preflight ladder — provider prompt cache will miss"
        );
        debug_assert!(false, "frozen prefix drifted across the ladder");
    }
    ok
}

/// Evict messages from `messages` until the total token count fits within
/// `budget_tokens`. Returns the new (potentially shorter) message vec.
///
/// ## Eviction tiers (lowest priority evicted first):
///
/// - **System messages**: never evicted (system prompt, memory, skills).
/// - **T3 — tool results**: evicted first. Large, ephemeral, replaceable
///   (the agent can re-run the tool if it needs the result again).
/// - **T2 — assistant reasoning**: evicted second. Verbose thinking
///   blocks that aren't essential to the conversation flow.
/// - **T1 — user + assistant-final**: evicted last. These are the
///   actual conversation turns.
///
/// When evicting from a tier, the oldest messages are removed first
/// (FIFO within tier). A recency reserve of `keep_recent` messages
/// (default 6) is always preserved regardless of tier — the agent
/// needs recent context to understand the current turn.
///
/// This is a port of magic-context's tiered target-headroom eviction,
/// simplified to a single pass (magic-context uses idempotence latches
/// + multi-pass; we don't need that for a first implementation).
pub fn evict_to_budget(messages: Vec<Message>, budget_tokens: usize) -> Vec<Message> {
    evict_to_budget_reporting(messages, budget_tokens).0
}

/// The outcome of an eviction pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvictionOutcome {
    /// The messages that survived.
    pub kept: usize,
    /// The estimated tokens that survived.
    pub tokens: usize,
    /// Whether the result met the budget.
    ///
    /// False when the PROTECTED messages — the system prompt and the
    /// newest turns — are larger than the budget on their own. The system
    /// prompt is never evicted (it is the agent's instructions), so no
    /// eviction can meet the budget in that case, and the caller has to
    /// know rather than send an over-budget request.
    pub within_budget: bool,
}

/// Evict to the budget, and REPORT whether the budget was met.
///
/// [`evict_to_budget`] returns the surviving messages and nothing else, so
/// a caller had no way to tell "the eviction did its job" from "the
/// eviction ran out of things to evict and the result is still over
/// budget". The tiered pass can only evict from the evictable set — the
/// system prompt and the recency reserve are not candidates — so when the
/// protected messages alone exceeded the budget, the result stayed over
/// budget with no signal.
///
/// The reserve IS trimmed (oldest first, whole tool groups) before this
/// gives up, so the only case that cannot meet the budget is a protected
/// set that is larger than the budget by itself — and that is worth
/// reporting rather than hiding.
pub fn evict_to_budget_reporting(
    messages: Vec<Message>,
    budget_tokens: usize,
) -> (Vec<Message>, EvictionOutcome) {
    let total = estimate_total_tokens(&messages);
    if total <= budget_tokens {
        let kept = messages.len();
        return (
            messages,
            EvictionOutcome {
                kept,
                tokens: total,
                within_budget: true,
            },
        );
    }

    // Recency reserve: scale with budget so large contexts preserve more
    // recent messages. For a 128k context, ~20 messages; for a 4k context,
    // ~6. Clamped to [6, 50] to avoid degenerate cases. Was fixed at 6,
    // which was too few for large contexts (the agent lost too much recent
    // context) and too many for tiny contexts (it couldn't evict enough).
    //
    // (iter-139 — fixed ponytail-audit bug A25: the previous .min(messages.len())
    // made keep_recent = messages.len() when there were < 6 messages, which
    // made the recency reserve cover ALL messages → eviction impossible.
    // Dropping the .min() is correct: if there are fewer messages than
    // keep_recent, the eviction loop simply finds nothing to evict, which
    // is the right behavior — you don't need to evict when you have few
    // messages.)
    let keep_recent = (budget_tokens / 4096).clamp(6, 50);
    let n = messages.len();

    // Build a list of (group start, tier) for evictable tool groups. System
    // messages (index 0, role=System) and the last `keep_recent` messages are
    // never evicted, and a group that *reaches into* the recency reserve is
    // skipped entirely — dropping its head would strand its tool results.
    let starts = tool_group_starts(&messages);
    let mut evictable: Vec<(usize, u8)> = Vec::new();
    for (i, msg) in messages.iter().enumerate() {
        if i == 0 && msg.role == crate::client::Role::System {
            continue; // never evict system prompt
        }
        if i >= n.saturating_sub(keep_recent) {
            continue; // recency reserve
        }
        if starts[i] != i {
            continue; // not the group's first message
        }
        let mut end = i + 1;
        while end < n && starts[end] == i {
            end += 1;
        }
        if end > n.saturating_sub(keep_recent) {
            continue; // group overlaps the recency reserve
        }
        evictable.push((i, message_tier(msg)));
    }

    // Sort by tier descending (T3=3 first), then by index ascending
    // (oldest first within tier).
    evictable.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Evict until under budget. Eviction always takes a WHOLE tool group: a
    // lone `tool_use` or a lone `tool_result` is a malformed request, so the
    // unit of eviction is the group, never the individual message.
    let mut keep = vec![true; n];
    let mut current_total = total;
    for (i, _tier) in &evictable {
        if current_total <= budget_tokens {
            break;
        }
        for j in *i..n {
            if starts[j] != *i {
                break;
            }
            current_total = current_total.saturating_sub(estimate_message_tokens(&messages[j]));
            keep[j] = false;
        }
    }

    // ── The final trim ─────────────────────────────────────────────────
    //
    // The tiered pass can only evict from `evictable`, so it CANNOT meet a
    // budget that the protected messages alone exceed: the system prompt
    // at index 0 and the recency reserve are never candidates. When their
    // combined size was over the budget, the loop exhausted `evictable`
    // and the function returned a list that was STILL OVER BUDGET — with
    // no signal to the caller, which had no way to know the eviction had
    // not done its job. (Reproduced: a 40 KB system prompt against a
    // 100-token budget returned 10,052 tokens.)
    //
    // So the reserve is trimmed here, oldest first, keeping the NEWEST
    // turns — which is what the reserve is for. The system prompt stays:
    // it is the agent's instructions, and dropping it makes the request
    // meaningless rather than merely smaller.
    //
    // Tool groups stay whole: a group is dropped as a unit, so a
    // `tool_use` is never separated from its `tool_result`.
    if current_total > budget_tokens {
        // The reserve runs from `reserve_start` to the end. Walk it from
        // the OLDEST end, dropping whole groups until the budget is met.
        //
        // Two bounds keep the trim from destroying the conversation:
        //
        // - `trim_start` is at least 1 when index 0 is the system prompt.
        //   `reserve_start` SATURATES AT 0 for a history shorter than
        //   `keep_recent` (which is at least 6), so without this the trim
        //   would start on the system prompt and drop it — the one thing
        //   eviction must never do.
        // - `trim_end` stops before the newest message: that is the live
        //   turn, the one the model is answering.
        let mut trim_start = n.saturating_sub(keep_recent);
        if !messages.is_empty() && messages[0].role == crate::client::Role::System {
            trim_start = trim_start.max(1);
        }
        let trim_end = n.saturating_sub(1);
        if trim_start >= trim_end {
            // Nothing but the protected floor is left to give.
        } else {
            // The PROTECTED floor: the system prompt and the newest turn.
            // Nothing below may drop these, so when they alone exceed the
            // budget, trimming the rest cannot help.
            let protected_floor: usize = messages
                .iter()
                .enumerate()
                .filter(|(j, m)| (*j == 0 && m.role == crate::client::Role::System) || *j + 1 == n)
                .map(|(_, m)| estimate_message_tokens(m))
                .sum();

            // Snap DOWN to the GROUP START: `starts[i]` maps every member
            // of a group to that group's FIRST index, so a reserve that
            // begins in the middle of a group would otherwise have its
            // tool_result dropped while its tool_use stayed — an orphaned
            // tool_use, which is a malformed history the providers reject.
            // `starts[x] <= x` always, so this is a plain snap down (a
            // `.max()` would cancel it and the orphan would come back).
            let mut i = starts[trim_start];
            while i < trim_end && current_total > budget_tokens {
                // Stop when trimming further cannot help: the PROTECTED
                // floor (the system prompt and the newest turn) is
                // already larger than the budget, so emptying the reserve
                // would destroy the conversation and still miss the
                // budget. The outcome reports `within_budget: false`, and
                // the caller decides what to do about it.
                if protected_floor > budget_tokens {
                    break;
                }
                // Advance to the end of the group, so the group drops as a
                // unit. The group may extend past `trim_end`; taking it
                // whole is what keeps the pair together.
                let mut end = i + 1;
                while end < n && starts[end] == i {
                    end += 1;
                }
                // Never drop a SYSTEM message: it is the agent's
                // instructions, and dropping it makes the request
                // meaningless rather than merely smaller. A group that
                // CONTAINS one is left whole.
                if (i..end).any(|j| messages[j].role == crate::client::Role::System) {
                    break;
                }
                // A group that CONTAINS the newest message holds the live
                // turn — the one the model is answering. Dropping it would
                // answer a question the model has not been asked yet, so
                // the group stays whole. The cost is that the trim cannot
                // reclaim the last group's tokens, which is correct: the
                // live turn is not reclaimable.
                if end >= n {
                    break;
                }
                for j in i..end {
                    if keep[j] {
                        current_total =
                            current_total.saturating_sub(estimate_message_tokens(&messages[j]));
                        keep[j] = false;
                    }
                }
                i = end;
            }
        }
    }

    let kept_messages: Vec<Message> = messages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, msg)| msg)
        .collect();
    let tokens = estimate_total_tokens(&kept_messages);
    let outcome = EvictionOutcome {
        kept: kept_messages.len(),
        tokens,
        within_budget: tokens <= budget_tokens,
    };
    if !outcome.within_budget {
        // The caller has to know: the protected messages alone exceed the
        // budget, so no eviction could meet it.
        tracing::warn!(
            tokens,
            budget_tokens,
            "context eviction could not meet the budget — the protected \
             messages (system prompt and the newest turns) are larger than it"
        );
    }
    (kept_messages, outcome)
}

/// Classify a message into an eviction tier. Higher = evicted first.
fn message_tier(msg: &Message) -> u8 {
    use crate::client::Role;
    match msg.role {
        Role::System => 0, // never evicted (handled separately, but defensive)
        Role::Tool => 3,   // T3: tool results — large, ephemeral
        Role::Assistant => {
            // T2 if it has reasoning/thinking, T1 if it's a final answer.
            // Heuristic: if the content is long (>500 chars) or contains
            // <think> tags, it's likely reasoning.
            if msg.content.len() > 500 || msg.content.contains("<think>") {
                2
            } else {
                1
            }
        }
        Role::User => 1, // T1: user messages — highest priority
    }
}

// ---------------------------------------------------------------------------
// Decay curve rendering
// ---------------------------------------------------------------------------

/// Render messages with a decay curve: older messages are truncated
/// proportionally to their age. The most recent messages are kept in full;
/// older messages are progressively shortened.
///
/// Ported from magic-context's `decay-curve.ts`. The formula is:
///   `H = H50 * 2^((I-50)/D) / max(p, 0.10)`
/// where:
///   - `H` = output length (chars to keep)
///   - `H50` = baseline length at 50% importance (default 200 chars)
///   - `I` = importance percentile (0-100; older = lower)
///   - `D` = decay constant (default 30; higher = slower decay)
///   - `p` = importance weight (default 1.0; clamped to >= 0.10)
///
/// No LLM calls — pure deterministic truncation. The idea is that old
/// messages still contribute context (so the agent remembers what was
/// discussed) but don't consume the full token budget.
///
/// Only applies to non-system messages before the current turn. System
/// messages are always kept in full (they're the system prompt, memory,
/// skills, etc.), and so is the current turn.
pub fn decay_render(messages: Vec<Message>, h50: usize, decay: f64) -> Vec<Message> {
    use crate::client::Role;

    let n = messages.len();
    if n <= 1 {
        return messages;
    }
    // The current turn (the latest user message and everything after it) is
    // never decayed: it is the request being answered, and truncating it
    // drops the user's own words rather than old context.
    let current_turn = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .unwrap_or(n);

    messages
        .into_iter()
        .enumerate()
        .map(|(i, msg)| {
            // System messages (index 0) are never decayed.
            if i == 0 && msg.role == Role::System {
                return msg;
            }
            if i >= current_turn {
                return msg;
            }

            // Importance percentile: most recent = 100, oldest = ~0.
            // i=0 is system (skipped above), so conversation starts at i=1.
            let conv_index = i.saturating_sub(1);
            let conv_len = n.saturating_sub(1);
            let importance = if conv_len == 0 {
                100.0
            } else {
                100.0 * (conv_index as f64 + 1.0) / (conv_len as f64)
            };

            // Decay curve formula. H50 is in TOKENS (not chars) for
            // consistency with estimate_tokens. We convert to chars
            // for truncation: tokens × 4 (the same heuristic used in
            // estimate_tokens). This ensures the decay targets match
            // the budget calculations — previously H50=200 was treated
            // as 200 chars (~50 tokens), which was too aggressive.
            let p = 1.0_f64; // default importance weight
            let p = p.max(0.10);
            let h_tokens = (h50 as f64) * 2.0_f64.powf((importance - 50.0) / decay) / p;
            let target_tokens = h_tokens.max(20.0) as usize; // never < 20 tokens
            let target_chars = target_tokens * 4; // tokens → chars heuristic

            if msg.content.chars().count() <= target_chars {
                msg // already short enough
            } else {
                // Hard-truncate to target_chars. We do NOT add a "[…truncated]"
                // marker because that would change the message content and
                // could confuse the model ("what is this marker?"). The
                // truncation is a context-management internal — the model
                // doesn't need to know it happened. If the model needs the
                // full content, it can re-request it via a tool call.
                let truncated: String = msg.content.chars().take(target_chars).collect();
                Message {
                    content: truncated,
                    ..msg
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tool-result spend guard
// ---------------------------------------------------------------------------

/// Default ceiling on the share of the context window a **single** tool result
/// may occupy before operant withholds the bulk of it.
///
/// 5% is deliberately conservative. `agent::truncate_tool_result` already caps
/// a result at 4096 bytes, which is ~0.8% of a 128k window (so the guard stays
/// out of the way on big-context models) but a *quarter* of a 16k window and
/// more than an entire 4k window. A share-based bound tracks whichever model is
/// configured instead of assuming one window size, and it still fires on the
/// 4096-byte cap wherever that cap is too loose.
///
/// Set to 0 to disable the guard.
pub const DEFAULT_MAX_TOOL_RESULT_SHARE: f64 = 0.05;

/// Token budget reserved for the guard's own marker text, so the body we inline
/// plus the marker stays under `tool_result_token_limit`.
const SPEND_MARKER_TOKEN_RESERVE: usize = 48;

/// Footer appended after a withheld tool result. Never silently shortened: the
/// model is told exactly how much was removed.
const SPEND_TRUNCATION_FOOTER_PREFIX: &str = "\n[operant: end of tool result — withheld ";

/// Token ceiling for a single tool result at `max_share` of `budget_tokens`.
///
/// Returns `usize::MAX` when `max_share <= 0.0` (guard disabled), so callers
/// and the guard itself agree on the threshold from one place.
pub fn tool_result_token_limit(budget_tokens: usize, max_share: f64) -> usize {
    if max_share <= 0.0 {
        return usize::MAX;
    }
    ((budget_tokens as f64) * max_share).round().max(1.0) as usize
}

/// Guard one tool result against consuming the context window.
///
/// Returns `None` when the result fits within `max_share` of `budget_tokens` —
/// the caller keeps the string it already owns, so no clone happens on the hot
/// path. Otherwise returns a replacement whose **body is an exact, unmodified,
/// char-boundary prefix of `content`**, wrapped in a marker that states the
/// size, the token count, the share of the window, the ceiling, and the
/// estimated input price (when a `models.dev` rate is supplied).
///
/// **Safety.** The only edits to tool output are (a) dropping a suffix and
/// (b) adding operant-authored marker text. Nothing is reordered, substituted
/// or synthesised, and a suffix is never dropped without saying so — a silently
/// shortened tool result is worse than a failed one, because the model would
/// reason from output the tool never produced.
pub fn guard_tool_result(
    tool_call_id: &str,
    content: &str,
    budget_tokens: usize,
    max_share: f64,
    cost_per_million_input: Option<f64>,
) -> Option<String> {
    let limit = tool_result_token_limit(budget_tokens, max_share);
    let tokens = estimate_tokens(content);
    if tokens <= limit {
        return None;
    }

    let share_pct = if budget_tokens == 0 {
        0.0
    } else {
        100.0 * tokens as f64 / budget_tokens as f64
    };
    let cost_note = match cost_per_million_input {
        Some(rate) => format!(
            " Estimated input price at {rate:.4}/1M tokens: ${:.4}.",
            tokens as f64 / 1_000_000.0 * rate
        ),
        None => String::new(),
    };
    let header = format!(
        "[operant: oversized tool result for call {tool_call_id} — {} chars, ~{tokens} tokens, \
         {share_pct:.1}% of the {budget_tokens}-token context window. The ceiling for one result \
         is {limit} tokens ({:.0}% of the window).{cost_note} Only the first {{KEPT}} tokens are \
         inlined below; the remainder was withheld so the conversation stays inside its window. \
         Re-run the tool with a narrower slice (head/tail, a line range, or a narrower query) \
         if you need the rest.]\n",
        content.chars().count(),
        max_share * 100.0,
    );

    // Size the body against the ceiling minus the marker this guard spends.
    // `estimate_tokens` is chars/4, so the inverse is tokens*4 bytes.
    let keep_bytes = limit
        .saturating_sub(estimate_tokens(&header))
        .saturating_sub(SPEND_MARKER_TOKEN_RESERVE)
        .saturating_mul(4);
    let body = crate::agent::safe_truncate_str(content, keep_bytes);
    let kept_tokens = estimate_tokens(body);

    let footer_text = format!(
        "{SPEND_TRUNCATION_FOOTER_PREFIX}{} of {tokens} tokens ({} tokens removed).]",
        kept_tokens,
        tokens.saturating_sub(kept_tokens)
    );
    let header = header.replace("{KEPT}", &kept_tokens.to_string());

    Some(format!("{header}{body}{footer_text}"))
}

// ---------------------------------------------------------------------------
// Combined budget management
// ---------------------------------------------------------------------------

/// Apply context management to a message array: first decay-render older
/// messages, then evict if still over budget. This is the main entry
/// point called by `build_messages`.
///
/// `budget_tokens` is the target context window (e.g. 120000 for GPT-4).
/// `reserve_for_response` is the tokens to leave free for the model's
/// response (e.g. 4096).
///
/// The returned outcome says whether the result actually met the budget.
/// The eviction can only give up what is EVICTABLE — the system prompt and
/// the newest turn are protected — so when those alone exceed the budget,
/// the result is still over it, and the caller has to be told rather than
/// send an over-budget request.
pub fn manage_context(
    messages: Vec<Message>,
    budget_tokens: usize,
    reserve_for_response: usize,
) -> Vec<Message> {
    let (kept, _outcome) = manage_context_reporting(messages, budget_tokens, reserve_for_response);
    kept
}

/// [`manage_context`], reporting whether the budget was met.
pub fn manage_context_reporting(
    messages: Vec<Message>,
    budget_tokens: usize,
    reserve_for_response: usize,
) -> (Vec<Message>, EvictionOutcome) {
    let effective_budget = budget_tokens.saturating_sub(reserve_for_response);

    // Step 1: decay-render older messages to compress them.
    let decayed = decay_render(messages, 200, 30.0);

    // Step 2: evict if still over budget.
    evict_to_budget_reporting(decayed, effective_budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Role};

    // ── output-token fitting (iter-792) ────────────────────────────────

    /// The incident shape this fitter exists to prevent: a large requested
    /// cap (or provider default) plus a big prompt exceeds the window and
    /// the provider 400s the request before generation. The fitter must
    /// shrink the cap to window − prompt − headway.
    #[test]
    fn fit_output_tokens_shrinks_an_oversized_cap() {
        // window 128k, prompt 120k reported: 8k room minus 64 headway.
        let fitted = fit_output_tokens(Some(32_768), 128_000, 120_000, true);
        assert_eq!(fitted, Some(128_000 - 120_000 - 64));
    }

    /// A requested cap that already fits is left untouched — the fitter is
    /// a guard, not a reducer; small prompts keep their caller preference.
    #[test]
    fn fit_output_tokens_leaves_a_fitting_cap_alone() {
        assert_eq!(fit_output_tokens(Some(4096), 128_000, 1_000, true), None);
        // None-requested on a small prompt: the whole room is the cap.
        assert_eq!(fit_output_tokens(None, 128_000, 1_000, true), Some(126_936));
    }

    /// No cap requested: fit the largest useful cap so the provider's own
    /// (often window-sized) default can never produce a rejected request.
    #[test]
    fn fit_output_tokens_fits_a_cap_when_none_requested() {
        // 100k reported, window 128k → 27,936 room → floor 1024 applies? No:
        // 27,936 > 1024, so the fitted cap is the full room.
        let fitted = fit_output_tokens(None, 128_000, 100_000, true);
        assert_eq!(fitted, Some(128_000 - 100_000 - 64));
    }

    /// The floor: when the room shrinks below 1024 the fitter still requests
    /// the room itself — requesting more reproduces the 400-rejection class;
    /// the tiny-but-admissible cap lets the turn land and compaction rescue
    /// the next one.
    #[test]
    fn fit_output_tokens_requests_the_room_when_it_shrinks() {
        let fitted = fit_output_tokens(Some(8192), 128_000, 127_500, true);
        assert_eq!(fitted, Some(436)); // 128000-127500-64 = 436 < 1024
        // exactly at the floor boundary:
        let fitted = fit_output_tokens(Some(8192), 128_000, 126_912, true);
        assert_eq!(fitted, Some(1024));
        assert_eq!(MIN_FITTED_OUTPUT_TOKENS, 1024);
    }

    /// Local (heuristic) prompt counts get the 10% margin; provider-reported
    /// counts are exact and skip it. Same arithmetic, two trust levels.
    #[test]
    fn fit_output_tokens_pads_local_estimates_only() {
        // 100k local → padded 110k → room 17,936.
        let local = fit_output_tokens(Some(32_768), 128_000, 100_000, false);
        assert_eq!(local, Some(128_000 - 110_000 - 64));
        // 100k reported → room 27,936.
        let reported = fit_output_tokens(Some(32_768), 128_000, 100_000, true);
        assert_eq!(reported, Some(128_000 - 100_000 - 64));
        assert!(local.unwrap() < reported.unwrap());
    }

    /// Unknown window / unknown prompt: no fit, no behavior change. The
    /// fitter must never invent a cap when it cannot see the room.
    #[test]
    fn fit_output_tokens_no_op_without_window_or_prompt() {
        assert_eq!(fit_output_tokens(Some(4096), 0, 100, true), None);
        assert_eq!(fit_output_tokens(Some(4096), 128_000, 0, true), None);
    }

    fn make_msg(role: Role, content: impl Into<String>) -> Message {
        Message::new(role, content.into())
    }

    /// An assistant turn that requests one tool call — i.e. a `tool_use`.
    fn make_tool_use(id: &str) -> Message {
        Message::assistant("").with_tool_calls(vec![crate::client::ToolCall {
            id: id.to_string(),
            function: crate::client::ToolCallFunction {
                name: "read_file".to_string(),
                arguments: r#"{"path":"a.rs"}"#.to_string(),
            },
        }])
    }

    /// Assert that no surviving `tool_result` lacks its `tool_use` and no
    /// surviving `tool_use` lacks every one of its `tool_result`s.
    #[track_caller]
    fn assert_no_orphans(kept: &[Message]) {
        let answered: std::collections::HashSet<&str> = kept
            .iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_call_id.as_deref())
            .collect();
        let requested: std::collections::HashSet<&str> = kept
            .iter()
            .filter(|m| is_tool_use(m))
            .flat_map(|m| m.tool_calls.as_ref().into_iter().flatten())
            .map(|tc| tc.id.as_str())
            .collect();
        for id in &requested {
            assert!(
                answered.contains(id),
                "tool_use {id} survived with no tool_result"
            );
        }
        for id in &answered {
            assert!(
                requested.contains(id),
                "tool_result {id} survived with no tool_use"
            );
        }
    }

    #[test]
    fn estimate_tokens_basic() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 -> 3
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1); // 1 char / 4 = 0.25 -> 1 (rounded up)
    }

    #[test]
    fn prompt_cache_guard_passes_identical_prefix() {
        let prefix = vec![
            make_msg(Role::System, "frozen head"),
            make_msg(Role::System, "frozen skills"),
        ];
        let after = vec![
            make_msg(Role::System, "frozen head"),
            make_msg(Role::System, "frozen skills"),
            make_msg(Role::User, "question"),
        ];
        assert!(prompt_cache_guard(&prefix, &after));
    }

    #[test]
    #[should_panic(expected = "frozen prefix drifted")]
    fn prompt_cache_guard_catches_content_drift() {
        let prefix = vec![make_msg(Role::System, "frozen head")];
        let after = vec![
            make_msg(Role::System, "mutated head"),
            make_msg(Role::User, "question"),
        ];
        assert!(!prompt_cache_guard(&prefix, &after));
    }

    #[test]
    #[should_panic(expected = "frozen prefix drifted")]
    fn prompt_cache_guard_catches_truncated_prefix() {
        let prefix = vec![
            make_msg(Role::System, "frozen head"),
            make_msg(Role::System, "frozen skills"),
        ];
        assert!(!prompt_cache_guard(
            &prefix,
            &[make_msg(Role::User, "question")],
        ));
    }

    #[test]
    fn evict_to_budget_noop_when_under_budget() {
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi there"),
        ];
        let result = evict_to_budget(msgs.clone(), 10000);
        assert_eq!(result.len(), msgs.len());
    }

    /// The eviction must MEET the budget, or REPORT that it could not.
    ///
    /// The tiered eviction loop breaks as soon as the total fits, but it
    /// only evicts from the `evictable` set — the system prompt at index 0
    /// and the whole recency reserve are never candidates. When their
    /// combined size alone exceeded the budget, the loop exhausted
    /// `evictable` and the function returned a list that was STILL OVER
    /// BUDGET, with no signal to the caller. The caller had no way to know
    /// the eviction had not done its job.
    ///
    /// The contract now: the result meets the budget, OR the outcome says
    /// it did not.
    #[test]
    fn the_eviction_meets_the_budget_or_reports_that_it_could_not() {
        // The system prompt alone is far larger than the budget. It is
        // never evicted (it is the agent's instructions), so no eviction
        // can meet this budget — and the caller must be told.
        let mut msgs = vec![make_msg(Role::System, "s".repeat(40_000))];
        for i in 0..10 {
            msgs.push(make_msg(Role::User, format!("recent message {i}")));
        }
        let budget = 100;

        let (result, outcome) = evict_to_budget_reporting(msgs, budget);
        assert!(
            !outcome.within_budget,
            "an impossible budget must be reported as unmet, not hidden"
        );
        assert!(
            outcome.tokens > budget,
            "the reported tokens must be the ones that survived"
        );
        assert_eq!(
            outcome.tokens,
            estimate_total_tokens(&result),
            "the reported tokens must match the messages that survived"
        );
        // The system prompt is never evicted, and the newest turn (the
        // live one) survives too.
        assert_eq!(
            result.first().map(|m| m.role.clone()),
            Some(Role::System),
            "the system prompt must survive"
        );
        assert_eq!(
            result.last().map(|m| m.role.clone()),
            Some(Role::User),
            "the newest turn must survive"
        );
    }

    /// The reserve IS trimmed when that is what is blowing the budget, so
    /// the "could not meet it" case is the narrow one.
    ///
    /// Without this the reserve was untouchable, and a conversation whose
    /// recent turns were larger than the budget came out of eviction
    /// unchanged — still over budget.
    #[test]
    fn the_eviction_trims_the_recency_reserve_when_it_is_what_exceeds_the_budget() {
        // No system prompt: the reserve alone is over the budget.
        let mut msgs = Vec::new();
        for i in 0..20 {
            msgs.push(make_msg(
                Role::User,
                format!("recent message {i} {}", "x".repeat(200)),
            ));
        }
        let budget = 100;

        let (result, outcome) = evict_to_budget_reporting(msgs, budget);
        assert!(
            outcome.within_budget,
            "the reserve must be trimmed to meet the budget, got {} tokens",
            outcome.tokens
        );
        assert!(
            outcome.tokens <= budget,
            "the result must be within the budget, got {}",
            outcome.tokens
        );
        assert!(!result.is_empty(), "the newest turns must survive the trim");
    }

    /// The final trim must not cut a tool group in half.
    ///
    /// The trim starts at the recency reserve, which can begin in the
    /// MIDDLE of a tool group — a `tool_result` whose `tool_use` sits at a
    /// lower index. The tiered pass deliberately leaves such a group alone
    /// (it skips a group that overlaps the reserve), so the trim is the
    /// only code that touches it: without snapping to the group's start,
    /// the lone `tool_result` was dropped and its `tool_use` survived — an
    /// orphaned `tool_use`, which is a malformed history the providers
    /// reject with a 400.
    ///
    /// The construction is exact, not approximate: with 11 messages the
    /// reserve starts at index 5, which is EXACTLY the `tool_result`, and
    /// the group's real start is index 4. Without the snap the trim drops
    /// one message and the `tool_use` at index 4 survives alone.
    #[test]
    fn the_final_trim_never_orphans_a_tool_use_from_its_result() {
        // system(0), u0..u2 (1..3), tool_use(4), tool(5), r0..r4 (6..10).
        let mut msgs = vec![make_msg(Role::System, "sys")];
        for i in 0..3 {
            msgs.push(make_msg(Role::User, format!("u{i}")));
        }
        msgs.push(make_tool_use("call_1"));
        msgs.push(Message::tool("call_1", "result ".repeat(200)));
        for i in 0..5 {
            msgs.push(make_msg(Role::User, format!("r{i}")));
        }
        assert_eq!(msgs.len(), 11, "the construction's length is load-bearing");

        let (result, _outcome) = evict_to_budget_reporting(msgs, 60);
        assert_no_orphans(&result);
    }

    /// The trim must not drop the LIVE turn.
    ///
    /// The newest message is the one the model is answering. When the
    /// history ends on a `tool_result` whose `tool_use` sits at `n-2`, the
    /// group is `[n-2, n)` — and a trim that drops the whole group takes
    /// the newest message with it. The group must be kept WHOLE instead,
    /// which is also what keeps the pair together: clamping the drop
    /// RANGE to the reserve would split the group and bring the orphan
    /// back.
    ///
    /// The fixture is tuned so the trim actually REACHES the live group:
    /// the protected floor (the system prompt and the newest message) must
    /// FIT the budget, or the trim stops before it starts and the live
    /// turn survives for the wrong reason. The tool_use carries content so
    /// there is a window: floor <= budget < floor + tool_use.
    #[test]
    fn the_final_trim_keeps_the_live_turn() {
        // system(0), u0..u4 (1..5), tool_use(6), tool_result(7).
        let mut msgs = vec![make_msg(Role::System, "sys")];
        for i in 0..5 {
            msgs.push(make_msg(Role::User, format!("u{i}")));
        }
        msgs.push(make_tool_use("call_2"));
        msgs.push(Message::tool("call_2", "result ".repeat(200)));
        assert_eq!(msgs.len(), 8, "the construction's length is load-bearing");

        // The budget is chosen so the trim REACHES the live group:
        // `protected_floor` (the system prompt + the newest message) is
        // 358 tokens, so a budget of 360 lets the floor check pass, while
        // the running total is still over the budget when the walk
        // arrives at the tool_use. A smaller budget would stop the trim
        // before it started, and the live turn would survive for the
        // wrong reason.
        let (result, _outcome) = evict_to_budget_reporting(msgs, 360);
        assert!(
            result
                .iter()
                .any(|m| m.tool_call_id.as_deref() == Some("call_2")),
            "the live turn's tool_result must survive the trim"
        );
        assert_no_orphans(&result);
    }

    /// The token estimate must see the WHOLE message the provider sees.
    ///
    /// A tool-call turn's `content` is EMPTY by construction — the text is
    /// suppressed so the provider sees a pure tool-call turn — and the
    /// payload is in `tool_calls[].function.arguments`. Counting only the
    /// content charged 4 tokens for a turn that carried 100 KB of tool
    /// arguments, so the eviction was blind to exactly the messages that
    /// carry the most payload, and the request the provider rejected was
    /// over-budget by an amount the estimator never saw.
    ///
    /// The reasoning counts too: it is serialised back to the provider on
    /// the next request.
    #[test]
    fn the_token_estimate_sees_the_tool_arguments_and_the_reasoning() {
        use crate::client::{Role, ToolCall, ToolCallFunction};

        // A tool-call turn: empty content, a big argument payload.
        let big_args = "x".repeat(40_000);
        let tool_turn = Message {
            role: Role::Assistant,
            content: String::new(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                function: ToolCallFunction {
                    name: "write_file".to_string(),
                    arguments: big_args.clone(),
                },
            }]),
            extra_content: None,
        };
        let counted = estimate_message_tokens(&tool_turn);
        assert!(
            counted > 9_000,
            "a tool turn carrying 40 KB of arguments must not be estimated at \
             4 tokens — got {counted}"
        );

        // The reasoning counts too.
        let reasoning_turn = Message {
            role: Role::Assistant,
            content: "the answer".to_string(),
            reasoning: Some("y".repeat(40_000)),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        };
        let counted = estimate_message_tokens(&reasoning_turn);
        assert!(
            counted > 9_000,
            "the reasoning is sent back to the provider, so it must be counted — \
             got {counted}"
        );

        // A plain text message is unchanged.
        let plain = Message::new(Role::User, "hello world");
        assert_eq!(estimate_message_tokens(&plain), 4 + 3);
    }

    #[test]
    fn evict_to_budget_removes_tool_results_first() {
        // System + user + tool_result + 6 recent messages (so the tool
        // result is NOT in the recency reserve and CAN be evicted).
        let msgs = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "old user message"),
            make_msg(Role::Tool, "very long tool result ".repeat(50)),
            make_msg(Role::User, "msg 1"),
            make_msg(Role::Assistant, "msg 2"),
            make_msg(Role::User, "msg 3"),
            make_msg(Role::Assistant, "msg 4"),
            make_msg(Role::User, "msg 5"),
            make_msg(Role::Assistant, "recent answer"),
        ];
        // Budget of 100 forces eviction. The tool result is T3 and
        // outside the recency reserve (last 6), so it gets evicted first.
        let result = evict_to_budget(msgs, 100);
        // Tool result should be evicted.
        assert!(
            !result.iter().any(|m| m.role == Role::Tool),
            "tool result should be evicted"
        );
        // System should be preserved.
        assert!(result.iter().any(|m| m.role == Role::System));
    }

    #[test]
    fn evict_never_removes_system_prompt() {
        let msgs = vec![
            make_msg(Role::System, "system prompt that is important"),
            make_msg(Role::Tool, "tool result ".repeat(100)),
            make_msg(Role::User, "recent"),
        ];
        let result = evict_to_budget(msgs, 10);
        // System prompt must survive even under extreme budget pressure.
        assert!(result.first().is_some_and(|m| m.role == Role::System));
    }

    #[test]
    fn evict_preserves_recency_reserve() {
        // 10 messages, all tool results (T3). Budget forces eviction.
        let msgs: Vec<Message> = (0..10)
            .map(|i| make_msg(Role::Tool, format!("tool result {}", i)))
            .collect();
        let result = evict_to_budget(msgs, 50);
        // The last 6 (keep_recent) should always be preserved.
        assert!(
            result.len() >= 6,
            "recency reserve of 6 should be preserved, got {}",
            result.len()
        );
    }

    #[test]
    fn decay_render_preserves_system_message() {
        let msgs = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "old message that is quite long ".repeat(20)),
            make_msg(Role::User, "recent message"),
        ];
        let result = decay_render(msgs, 50, 30.0);
        // System message should be unchanged.
        assert_eq!(result[0].content, "system prompt");
    }

    #[test]
    fn decay_render_truncates_old_messages() {
        let long_content = "this is a very long message ".repeat(50);
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, &long_content), // oldest, should be truncated
            make_msg(Role::User, &long_content), // middle
            make_msg(Role::User, &long_content), // most recent, should be ~full
        ];
        let result = decay_render(msgs, 100, 30.0);
        // The oldest (index 1) should be shorter than the newest (index 3).
        let oldest_len = result[1].content.chars().count();
        let newest_len = result[3].content.chars().count();
        assert!(
            oldest_len < newest_len,
            "oldest should be shorter than newest: {} vs {}",
            oldest_len,
            newest_len
        );
    }

    #[test]
    fn manage_context_combines_decay_and_evict() {
        // 10 messages: system + 8 tool results + recent. The tool results
        // are T3 (evict first) and outside the recency reserve (last 6).
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::Tool, "result ".repeat(200)), // T3, oldest
            make_msg(Role::Tool, "result ".repeat(200)), // T3
            make_msg(Role::Tool, "result ".repeat(200)), // T3
            make_msg(Role::User, "msg 1"),
            make_msg(Role::Assistant, "msg 2"),
            make_msg(Role::User, "msg 3"),
            make_msg(Role::Assistant, "msg 4"),
            make_msg(Role::User, "msg 5"),
            make_msg(Role::Assistant, "recent answer"),
        ];
        // Budget of 100 tokens + 50 reserve = 50 effective.
        // The 3 tool results (~350 tokens each before decay) are T3 and
        // outside the recency reserve (keep_recent = 6, so last 6 are
        // preserved). They should be evicted.
        let result = manage_context(msgs, 100, 50);
        let total = estimate_total_tokens(&result);
        assert!(
            total <= 50,
            "total {} should be <= effective budget 50",
            total
        );
        // System + recent should survive.
        assert!(result.first().is_some_and(|m| m.role == Role::System));
        assert!(result.iter().any(|m| m.content == "recent answer"));
    }

    #[test]
    fn evict_to_budget_never_orphans_a_tool_use_from_its_result() {
        // A tool_use group followed by enough recent turns to push the group
        // out of the recency reserve. Eviction drops tool results first (T3),
        // which used to strand the surviving tool_use with no answer.
        let mut msgs = vec![
            make_msg(Role::System, "system prompt"),
            make_msg(Role::User, "old user message"),
            make_tool_use("call_1"),
            Message::tool("call_1", "very long tool result ".repeat(50)),
        ];
        for i in 0..6 {
            msgs.push(make_msg(Role::User, format!("msg {i}")));
            msgs.push(make_msg(Role::Assistant, format!("reply {i}")));
        }
        let result = evict_to_budget(msgs, 60);
        assert_no_orphans(&result);
    }

    // -----------------------------------------------------------------------
    // Safe compaction cut
    // -----------------------------------------------------------------------

    #[test]
    fn safe_cut_should_not_orphan_a_tool_result_from_its_tool_use() {
        // [0] user, [1] tool_use(call_1), [2] tool_result(call_1), [3] user
        let msgs = vec![
            make_msg(Role::User, "read the file"),
            make_tool_use("call_1"),
            Message::tool("call_1", "file contents ".repeat(40)),
            make_msg(Role::User, "thanks"),
        ];
        // A prefix cut cannot strand a tool_result — an earlier index always
        // survives with a later one — so direction A is reachable only from the
        // SUFFIX-keeping cut, which is what `llm_compressor` uses: it replaces
        // everything before the tail boundary with a summary and keeps
        // `messages[tail_start..]`. A raw boundary of 2 summarizes away
        // call_1's request while keeping its answer; snapping the boundary to
        // the group start keeps both halves.
        let raw_tail_boundary = 2;
        let safe_boundary = tool_group_starts(&msgs)[raw_tail_boundary];
        assert_eq!(safe_boundary, 1, "tail must move back to the tool_use");

        // The naive boundary really is malformed — otherwise this proves nothing.
        let naive_surviving_tail = &msgs[raw_tail_boundary..];
        assert!(
            naive_surviving_tail
                .iter()
                .any(|m| m.tool_call_id.as_deref() == Some("call_1")),
            "sanity: the naive tail still carries the tool_result"
        );
        assert!(
            !naive_surviving_tail.iter().any(is_tool_use),
            "sanity: the naive tail lost the tool_use that answered it"
        );

        let safe_tail = &msgs[safe_boundary..];
        assert!(
            safe_tail.iter().any(is_tool_use),
            "the snapped tail carries the tool_use with its result"
        );
        assert_no_orphans(safe_tail);

        // And the same property holds for the group-aware eviction, which keeps
        // a suffix and is the other suffix-shaped cut in the pipeline.
        for budget in (0..=400).step_by(11) {
            let kept = evict_to_budget(msgs.clone(), budget);
            assert_no_orphans(&kept);
        }
    }

    #[test]
    fn safe_cut_should_not_orphan_a_tool_use_from_its_tool_result() {
        // [0] user, [1] tool_use(call_1), [2] tool_result(call_1), [3] user
        let msgs = vec![
            make_msg(Role::User, "read the file"),
            make_tool_use("call_1"),
            Message::tool("call_1", "file contents ".repeat(40)),
            make_msg(Role::User, "thanks"),
        ];
        // A budget that fits the leading user turn but not the whole tool group
        // tempts a naive cut to land at 2, keeping call_1's request while
        // dropping its answer. Group closure must pull the cut back to 1.
        let group_cost: usize = msgs[1..3]
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>();
        let budget = estimate_message_tokens(&msgs[0]) + group_cost - 1;
        let k = safe_compaction_cutoff(&msgs, budget);
        assert!(
            k <= 1,
            "cut must not keep a tool_use whose result is dropped (k={k})"
        );
        assert_no_orphans(&msgs[..k]);
    }

    #[test]
    fn safe_cut_should_be_monotonic_in_budget() {
        let mut msgs = vec![make_msg(Role::System, "system prompt")];
        for i in 0..12 {
            msgs.push(make_msg(
                Role::User,
                format!("question {i} {}", "x".repeat(60)),
            ));
            msgs.push(make_tool_use(&format!("call_{i}")));
            msgs.push(Message::tool(
                format!("call_{i}"),
                format!("answer {i} {}", "y".repeat(60)),
            ));
        }
        let mut previous = 0usize;
        for budget in (0..=4000).step_by(37) {
            let k = safe_compaction_cutoff(&msgs, budget);
            assert!(
                k >= previous,
                "budget {budget}: cut moved earlier ({previous} -> {k})"
            );
            assert!(k <= msgs.len());
            assert_no_orphans(&msgs[..k]);
            previous = k;
        }
    }

    #[test]
    fn safe_cut_should_handle_history_that_is_all_tool_pairs() {
        // Nothing here is safe to cut: every message belongs to a tool group.
        let msgs: Vec<Message> = (0..8)
            .flat_map(|i| {
                [
                    make_tool_use(&format!("call_{i}")),
                    Message::tool(format!("call_{i}"), format!("result {i}")),
                ]
            })
            .collect();
        for budget in [0usize, 1, 8, 64, 4096, 1_000_000] {
            let k = safe_compaction_cutoff(&msgs, budget);
            assert!(k <= msgs.len(), "k={k} out of range");
            assert_no_orphans(&msgs[..k]);
        }
    }

    // -----------------------------------------------------------------------
    // Tool-result spend guard
    // -----------------------------------------------------------------------

    #[test]
    fn oversized_tool_result_should_report_size_and_token_price() {
        let huge = "x".repeat(200_000);
        let budget = 8_000usize;
        let guarded = guard_tool_result("call_42", &huge, budget, 0.05, Some(3.0))
            .expect("200k chars is 50% of an 8k window — guard must fire");

        assert!(guarded.contains("call_42"), "names the call it replaced");
        assert!(
            guarded.contains("200000 chars"),
            "states the original size: {guarded}"
        );
        assert!(
            guarded.contains("50000 tokens"),
            "states the token count: {guarded}"
        );
        assert!(
            guarded.contains("625.0% of the 8000-token context window"),
            "states the share of the window: {guarded}"
        );
        assert!(
            guarded.contains("400 tokens") && guarded.contains("5% of the window"),
            "states the ceiling: {guarded}"
        );
        assert!(
            guarded.contains("Estimated input price at 3.0000/1M tokens: $0.1500"),
            "states the token price: {guarded}"
        );
        // The body really is bounded, and the withheld part is accounted for.
        assert!(
            estimate_tokens(&guarded) <= budget,
            "guard overshot the budget"
        );
        assert!(
            guarded.contains("tokens removed"),
            "reports how much was withheld: {guarded}"
        );
    }

    #[test]
    fn oversized_tool_result_should_never_silently_truncate() {
        let huge = "x".repeat(200_000);
        let guarded =
            guard_tool_result("call_7", &huge, 8_000, 0.05, None).expect("guard must fire");
        // Truncation is always announced, in both the header and the footer.
        assert!(
            guarded.contains("oversized tool result"),
            "truncation is announced up front: {guarded}"
        );
        assert!(
            guarded.contains("was withheld"),
            "says the remainder was withheld, not dropped silently"
        );
        assert!(
            guarded.contains("withheld") && guarded.contains("tokens removed"),
            "footer accounts for the withheld tokens"
        );
        // What survived is an exact prefix of the original — no substitution.
        let body_start = guarded.find(']').expect("header closes") + 2;
        let body = &guarded[body_start..guarded.find("\n[operant: end").unwrap_or(guarded.len())];
        assert!(
            huge.starts_with(body),
            "body must be an unmodified prefix of the original output"
        );
        // A result within the ceiling is returned untouched (no clone, no marker).
        let small = "y".repeat(80);
        assert_eq!(guard_tool_result("call_8", &small, 8_000, 0.05, None), None);
        // 0.0 disables the guard entirely.
        assert_eq!(guard_tool_result("call_9", &huge, 8_000, 0.0, None), None);
    }

    // -----------------------------------------------------------------------
    // Semantic compaction cut
    // -----------------------------------------------------------------------

    /// A conversation that drifted: two weather turns, then a tool exchange
    /// carrying a release fact, then the user restating the release goal.
    /// Index map (5 groups): G0=[0,1) system · G1=[1,3) weather ·
    /// G2=[3,5) weather · G3=[5,8) tool exchange (the release fact at [6]) ·
    /// G4=[8,10) the restated goal.
    fn drifted_conversation() -> Vec<Message> {
        vec![
            make_msg(Role::System, "you are a helpful assistant"),
            make_msg(Role::User, "what is the weather today"),
            make_msg(Role::Assistant, "sunny and warm"),
            make_msg(Role::User, "and tomorrow"),
            make_msg(Role::Assistant, "rain likely"),
            make_tool_use("call_a"),
            Message::tool("call_a", "release pipeline: run release.sh --dry-run first"),
            make_msg(Role::Assistant, "dry-run noted before deploy"),
            make_msg(Role::User, "GOAL: fix the release pipeline"),
            make_msg(Role::Assistant, "on it"),
        ]
    }

    /// Budget at which the prefix cut ends at G3's start (index 5), so the
    /// release fact at [6] is on the wrong side of the recency cut.
    ///
    /// The token counts are the CORRECTED ones — the estimate now counts a
    /// tool-call turn's ARGUMENTS and its reasoning, not just its (empty)
    /// content. The cumulative cost of the fixture is
    /// [11, 22, 30, 37, 44, 55, 71, 82, 94, 100], so a recency cut that
    /// keeps indices 0..5 (dropping the fact at 6) needs a budget of 55 to
    /// 70 — and the SEMANTIC cut needs 63 to hold the system group (11) +
    /// the goal group (12) + the opening question (11) + the tool group
    /// (29).
    ///
    /// So the window is [55, 70] and 63 sits in it, satisfying both
    /// premises: the recency cut loses the goal fact, the semantic cut
    /// keeps it. Under the OLD (blind) estimate the tool group cost 20 and
    /// 60 was enough. The budget is a property of the fixture AND the
    /// estimator, and the estimator changed.
    const DRIFT_BUDGET: usize = 63;

    #[tokio::test]
    async fn semantic_cut_should_still_respect_safe_compaction_cutoff() {
        let msgs = drifted_conversation();
        let cap = safe_compaction_cutoff(&msgs, DRIFT_BUDGET);
        assert_eq!(cap, 5, "the recency cut must end before the tool group");

        let mock = crate::context::MockEmbedder::default();
        let cut = semantic_compaction_cutoff(&msgs, DRIFT_BUDGET, Some(&mock)).await;
        assert!(cut.semantic, "an embedder was available, so the pass ran");

        // (1) The result the net certifies, rebuilt from the selection.
        let kept_msgs: Vec<Message> = cut
            .kept
            .iter()
            .take(cut.head_cut)
            .filter_map(|&i| msgs.get(i).cloned())
            .collect();
        assert!(
            !kept_msgs.is_empty(),
            "the net must not have truncated the whole selection"
        );

        // (2) No orphan in either direction — the property the net exists for.
        assert_no_orphans(&kept_msgs);

        // (3) Group alignment: a kept message is either its own group's head or
        //     belongs to a group whose head was kept too, so no group is
        //     entered in the middle.
        let starts = tool_group_starts(&msgs);
        let kept_set: std::collections::HashSet<usize> = cut.kept.iter().copied().collect();
        for &i in &cut.kept {
            assert!(
                starts[i] == i || kept_set.contains(&starts[i]),
                "index {i} kept without its group head {}",
                starts[i]
            );
        }

        // (4) The headline guarantee: the returned selection is EXACTLY the
        //     prefix `safe_compaction_cutoff` certifies for the same budget.
        assert_eq!(
            cut.head_cut,
            safe_compaction_cutoff(&kept_msgs, DRIFT_BUDGET),
            "the semantic choice must be the safety net's own answer, applied last"
        );
        assert!(
            estimate_total_tokens(&kept_msgs) <= DRIFT_BUDGET,
            "the retained set must fit the budget it was selected for"
        );

        // (5) And the goal-relevant group actually survives, which is the
        //     reason the pass exists — the recency cut at [6] drops it.
        assert!(
            kept_msgs
                .iter()
                .any(|m| m.content.contains("release.sh --dry-run")),
            "the goal-relevant tool result must survive: {:?}",
            kept_msgs
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
        );
        // (6) A low-relevance group is what got sacrificed.
        assert!(
            !kept_msgs.iter().any(|m| m.content == "sunny and warm"),
            "a goal-irrelevant turn should be the one dropped"
        );
    }

    #[tokio::test]
    async fn semantic_cut_should_fall_back_to_recency_without_embeddings() {
        let msgs = drifted_conversation();

        // (a) No embedder at all.
        let none = semantic_compaction_cutoff(&msgs, DRIFT_BUDGET, None).await;
        assert!(!none.semantic, "must report the degraded path");
        assert_eq!(none.goal_similarity, 0.0, "no fabricated score");
        let cap = safe_compaction_cutoff(&msgs, DRIFT_BUDGET);
        assert_eq!(none.kept, (0..cap).collect::<Vec<_>>());
        assert_eq!(none.head_cut, cap);
        // The degraded selection is exactly today's behaviour.
        let applied = none.apply(msgs.clone());
        assert_eq!(applied.len(), cap, "the degraded cut is the safe cut");
        assert!(
            applied
                .iter()
                .zip(msgs.iter())
                .all(|(a, b)| a.role == b.role && a.content == b.content),
            "the degraded cut must be the messages the recency cut keeps"
        );

        // (b) An embedder that fails degrades identically — compaction must not
        //     fail because an optional embedding endpoint is down.
        struct FailingEmbedder;
        #[async_trait::async_trait]
        impl crate::context::Embedder for FailingEmbedder {
            fn model_id(&self) -> &str {
                "failing"
            }
            async fn embed(&self, _: &[String]) -> crate::error::Result<Vec<Vec<f32>>> {
                Err(crate::error::Error::Agent("embedder is down".to_string()))
            }
        }
        let failed = semantic_compaction_cutoff(&msgs, DRIFT_BUDGET, Some(&FailingEmbedder)).await;
        assert!(!failed.semantic, "an embedder error must degrade, not fail");
        assert_eq!(failed.kept, (0..cap).collect::<Vec<_>>());

        // (c) No user turn ⇒ no goal ⇒ recency.
        let no_user: Vec<Message> = msgs
            .iter()
            .filter(|m| m.role != Role::User)
            .cloned()
            .collect();
        let goal_less = semantic_compaction_cutoff(&no_user, DRIFT_BUDGET, Some(&mock())).await;
        assert!(!goal_less.semantic);
        assert_eq!(compaction_goal(&no_user), None);
    }

    fn mock() -> crate::context::MockEmbedder {
        crate::context::MockEmbedder::default()
    }

    #[test]
    fn compaction_goal_is_the_last_user_turn() {
        let msgs = drifted_conversation();
        assert_eq!(
            compaction_goal(&msgs).as_deref(),
            Some("GOAL: fix the release pipeline"),
            "the live task statement, not the superseded opening turn"
        );
        // An empty user turn does not become the goal.
        let blank = vec![
            make_msg(Role::User, "real goal"),
            make_msg(Role::User, "   "),
        ];
        assert_eq!(compaction_goal(&blank).as_deref(), Some("real goal"));
    }

    /// Measures goal-fact retention on a clearly-labelled SYNTHETIC fixture.
    ///
    /// HONESTY NOTE: this fixture is synthetic and I wrote it. It is not a
    /// session log. It encodes exactly one claim — that a recency prefix cut
    /// loses a late goal-relevant fact that a group-selection cut keeps — and
    /// it is built so the claim is testable at all: the goal-relevant group
    /// sits *after* the recency cut, which is the only configuration where the
    /// two strategies can disagree. It is a demonstration, not evidence of
    /// benefit in production. The embedder is `MockEmbedder`, a hash trick
    /// (`context/embedder.rs:126`) that scores token overlap, so a genuine
    /// paraphrase would NOT be matched by it.
    #[tokio::test]
    async fn measure_goal_fact_retention_recency_vs_semantic() {
        let msgs = drifted_conversation();
        let cap = safe_compaction_cutoff(&msgs, DRIFT_BUDGET);
        let baseline = msgs[..cap]
            .iter()
            .any(|m| m.content.contains("release.sh --dry-run"));
        let cut = semantic_compaction_cutoff(&msgs, DRIFT_BUDGET, Some(&mock())).await;
        let head_cut = cut.head_cut;
        let similarity = cut.goal_similarity;
        let semantic = cut
            .apply(msgs)
            .iter()
            .any(|m| m.content.contains("release.sh --dry-run"));
        println!(
            "MEASURE goal_fact_retained: recency_prefix={baseline} semantic_group_cut={semantic} \
             (budget={DRIFT_BUDGET}, cap={cap}, head_cut={head_cut}, similarity={similarity:.4})"
        );
        assert!(
            !baseline,
            "fixture invariant: the recency cut must lose the goal fact"
        );
        assert!(semantic, "the semantic cut must keep it");
    }

    /// The pass must never orphan a pair, across every budget and every
    /// drift shape — not just the one the fixture above uses.
    #[tokio::test]
    async fn semantic_cut_never_orphans_a_pair_at_any_budget() {
        let mock = mock();
        for extra_filler in 0..6 {
            let mut msgs = drifted_conversation();
            for i in 0..extra_filler {
                msgs.insert(1, make_msg(Role::User, format!("filler {i}")));
            }
            for budget in [0usize, 8, 32, DRIFT_BUDGET, 120, 400, 100_000] {
                let cut = semantic_compaction_cutoff(&msgs, budget, Some(&mock)).await;
                let kept: Vec<Message> = cut
                    .kept
                    .iter()
                    .take(cut.head_cut)
                    .filter_map(|&i| msgs.get(i).cloned())
                    .collect();
                assert_no_orphans(&kept);
                assert!(
                    estimate_total_tokens(&kept) <= budget || kept.is_empty(),
                    "budget {budget}: kept {} tokens",
                    estimate_total_tokens(&kept)
                );
                assert_eq!(
                    cut.head_cut,
                    safe_compaction_cutoff(&kept, budget),
                    "budget {budget}: the net must be the last word"
                );
            }
        }
    }
    // ── Micro-compaction ───────────────────────────────────────────────────

    #[test]
    fn micro_compact_folds_the_oldest_exchange_into_one_marker() {
        let mut messages = vec![
            make_msg(Role::System, "frozen prefix"),
            make_msg(Role::User, "first question"),
            make_tool_use("t1"),
            make_msg(Role::Tool, "tool result for the first question"),
            make_msg(Role::Assistant, "first answer"),
            make_msg(Role::User, "second question"),
            make_msg(Role::Assistant, "second answer"),
            make_msg(Role::User, "latest question"),
            make_msg(Role::Assistant, "latest answer"),
        ];
        let before = messages.len();
        let marker = micro_compact(&mut messages, 1, 2, 240).expect("foldable");
        assert!(
            marker.contains("first answer"),
            "the gist keeps the exchange's assistant text"
        );
        assert!(marker.contains("[compacted exchange"));
        // head + fold marker + tail preserved; the middle collapsed to ONE.
        assert_eq!(messages[0].content, "frozen prefix", "head untouched");
        assert_eq!(
            messages.len(),
            before - 3,
            "4 exchange messages folded to 1"
        );
        assert_eq!(
            messages[1].role,
            Role::User,
            "the marker rides as a user message"
        );
        assert!(messages[1].content.contains("first answer"));
        assert_eq!(
            messages.last().unwrap().content,
            "latest answer",
            "recency tail untouched"
        );
        // The fold never split the tool pair: both halves left together.
        assert!(
            !messages
                .iter()
                .any(|m| m.role == Role::Tool && m.content.contains("first question")),
            "the folded tool result is gone with its request"
        );
    }

    #[test]
    fn micro_compact_respects_head_and_tail_windows() {
        let mut messages = vec![
            make_msg(Role::System, "frozen"),
            make_msg(Role::User, "q1"),
            make_msg(Role::Assistant, "a1"),
            make_msg(Role::User, "q2"),
            make_msg(Role::Assistant, "a2"),
        ];
        // head_keep 1 + tail_keep 4 leaves nothing foldable in the middle.
        assert_eq!(micro_compact(&mut messages, 1, 4, 240), None);
        assert_eq!(messages.len(), 5, "nothing touched");
        // A conversation shorter than the head window never folds.
        assert_eq!(micro_compact(&mut messages, 8, 2, 240), None);
    }

    #[test]
    fn micro_compact_truncates_long_gists_head_tail() {
        let long = "x".repeat(1_000);
        let mut messages = vec![
            make_msg(Role::System, "s"),
            make_msg(Role::User, "q"),
            make_msg(Role::Assistant, long.clone()),
            make_msg(Role::User, "q2"),
            make_msg(Role::Assistant, "a2"),
            make_msg(Role::User, "q3"),
            make_msg(Role::Assistant, "a3"),
        ];
        let marker = micro_compact(&mut messages, 1, 2, 60).expect("foldable");
        assert!(
            marker.contains("[…]"),
            "long gists fold head/tail with an explicit marker"
        );
        assert!(
            marker.chars().count() < 120,
            "the marker stays compact (got {})",
            marker.chars().count()
        );
    }

    #[test]
    fn micro_compaction_env_parse_is_pure_and_opt_in() {
        assert!(!parse_micro_compaction_env(None));
        assert!(!parse_micro_compaction_env(Some("off")));
        assert!(parse_micro_compaction_env(Some("1")));
        assert!(parse_micro_compaction_env(Some(" TRUE ")));
    }

    #[test]
    fn micro_compact_folds_past_an_earlier_marker() {
        let mut msgs = vec![
            make_msg(Role::User, "q0"),
            make_msg(Role::Assistant, "a0"),
            make_msg(Role::User, "q1"),
            make_msg(Role::Assistant, "a1"),
            make_msg(Role::User, "q2"),
            make_msg(Role::Assistant, "a2"),
            make_msg(Role::User, "q3"),
            make_msg(Role::Assistant, "a3"),
        ];
        assert!(micro_compact(&mut msgs, 0, 1, 240).is_some());
        assert!(
            micro_compact(&mut msgs, 0, 1, 240).is_some(),
            "the second fold must not stall on the first marker"
        );
        assert_eq!(
            msgs.iter()
                .filter(|m| m.content.starts_with(MICRO_MARKER_PREFIX))
                .count(),
            2
        );
    }

    #[test]
    fn decay_render_keeps_the_current_turn_whole() {
        let long = "x".repeat(40_000);
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, &long), // earlier turn: may decay
            make_msg(Role::Assistant, "answer"),
            make_msg(Role::User, &long), // current turn: whole
        ];
        let result = decay_render(msgs, 100, 30.0);
        assert!(
            result[1].content.len() < long.len(),
            "the earlier turn decays"
        );
        assert_eq!(
            result[3].content.len(),
            long.len(),
            "the current turn is never truncated"
        );
    }

    #[test]
    fn bust_permission_is_none_below_the_force_line_without_a_natural_reason() {
        // 84% of a 10k window: one token under the force line.
        assert_eq!(
            bust_permission(8_400, 10_000, None, false),
            BustPermission::None
        );
    }

    #[test]
    fn bust_permission_forces_at_the_force_line() {
        assert_eq!(
            bust_permission(8_500, 10_000, None, false),
            BustPermission::Force
        );
        assert_eq!(
            bust_permission(9_900, 10_000, None, false),
            BustPermission::Force
        );
    }

    #[test]
    fn bust_permission_is_natural_when_the_cache_is_cold_anyway() {
        assert_eq!(
            bust_permission(1_000, 10_000, Some("model changed"), false),
            BustPermission::Natural("model changed")
        );
    }

    #[test]
    fn bust_permission_explicit_wins_over_every_other_reason() {
        assert_eq!(
            bust_permission(9_900, 10_000, Some("model changed"), true),
            BustPermission::Explicit
        );
        assert_eq!(
            bust_permission(0, 10_000, None, true),
            BustPermission::Explicit
        );
    }

    #[test]
    fn natural_bust_needs_a_previous_request_and_detects_each_cause() {
        use crate::agent::clients::cache_monitor::PrefixDigest;
        use std::time::{Duration, Instant};
        let at = Instant::now();
        let fp = |model: &str, system: &str, at: Instant| BustFingerprint {
            model: model.to_string(),
            system: PrefixDigest::of(system, &[]),
            at,
        };
        let ttl = Duration::from_secs(300);
        let prev = fp("m1", "sys", at);

        assert_eq!(natural_bust_reason(None, &fp("m1", "sys", at), ttl), None);
        assert_eq!(
            natural_bust_reason(Some(&prev), &fp("m1", "sys", at), ttl),
            None
        );
        assert_eq!(
            natural_bust_reason(Some(&prev), &fp("m2", "sys", at), ttl),
            Some("model changed")
        );
        assert_eq!(
            natural_bust_reason(Some(&prev), &fp("m1", "sys2", at), ttl),
            Some("system prompt changed")
        );
        assert_eq!(
            natural_bust_reason(
                Some(&prev),
                &fp("m1", "sys", at + ttl + Duration::from_secs(1)),
                ttl
            ),
            Some("prompt cache idle past its TTL")
        );
    }
}

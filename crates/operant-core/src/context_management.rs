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

use crate::client::Message;

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
pub fn estimate_message_tokens(msg: &Message) -> usize {
    // 4 tokens overhead per message (role tags, separators) — matches
    // OpenAI's pricing model.
    4 + estimate_tokens(&msg.content)
}

/// Estimate total tokens for a slice of messages.
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

// ---------------------------------------------------------------------------
// tool_use / tool_result correlation
// ---------------------------------------------------------------------------

/// A message that *requests* tool calls: an assistant turn carrying at least
/// one [`ToolCall`]. Each `ToolCall::id` is the id the provider expects to see
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
/// recorded in [`head_cut`] — see [`semantic_compaction_cutoff`].
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
/// `embedder: None`, an empty goal, more than [`SEMANTIC_MAX_GROUPS`] groups,
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
// Tiered eviction
// ---------------------------------------------------------------------------

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
    let total = estimate_total_tokens(&messages);
    if total <= budget_tokens {
        return messages;
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

    messages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, msg)| msg)
        .collect()
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
/// Only applies to non-system messages. System messages are always kept
/// in full (they're the system prompt, memory, skills, etc.).
pub fn decay_render(messages: Vec<Message>, h50: usize, decay: f64) -> Vec<Message> {
    use crate::client::Role;

    let n = messages.len();
    if n <= 1 {
        return messages;
    }

    messages
        .into_iter()
        .enumerate()
        .map(|(i, msg)| {
            // System messages (index 0) are never decayed.
            if i == 0 && msg.role == Role::System {
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
pub fn manage_context(
    messages: Vec<Message>,
    budget_tokens: usize,
    reserve_for_response: usize,
) -> Vec<Message> {
    let effective_budget = budget_tokens.saturating_sub(reserve_for_response);

    // Step 1: decay-render older messages to compress them.
    let decayed = decay_render(messages, 200, 30.0);

    // Step 2: evict if still over budget.
    evict_to_budget(decayed, effective_budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Role};

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
    fn evict_to_budget_noop_when_under_budget() {
        let msgs = vec![
            make_msg(Role::System, "system"),
            make_msg(Role::User, "hello"),
            make_msg(Role::Assistant, "hi there"),
        ];
        let result = evict_to_budget(msgs.clone(), 10000);
        assert_eq!(result.len(), msgs.len());
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
    const DRIFT_BUDGET: usize = 60;

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
}

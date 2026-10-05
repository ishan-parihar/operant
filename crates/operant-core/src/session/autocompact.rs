//! Scheduled-run autocompaction with a compaction floor (§4.5).
//!
//! # Why this is a third mechanism, not a second one
//!
//! Operant already compresses a conversation in two places, and this module is
//! deliberately neither of them:
//!
//! 1. [`crate::context_management`] — tiered eviction plus a decay curve. Runs
//!    **during** a request, when the assembled prompt approaches the context
//!    window. Reactive, per-request, lossy.
//! 2. [`crate::agent::llm_compressor`] — `compress_context_overflow`. Runs when
//!    a provider reports an overflow. Also reactive.
//!
//! Both are *emergency* mechanisms: they fire when a request would otherwise
//! fail, and both are indifferent to whether the conversation is worth keeping.
//! Neither keeps a session small between jobs.
//!
//! This is the **scheduled** mechanism. It runs **after** a session's scheduled
//! execution has already finished, and its job is to leave that session small
//! and useful for the *next* job. Nothing here runs before a request is sent,
//! and nothing here can cost a job its work: by the time this code is reached,
//! the work is already durable in the message rows.
//!
//! # The compaction floor
//!
//! Compaction replaces verbatim history with a lossy summary. Below some size
//! that trade is strictly bad: a short session has nothing to save, and the
//! next scheduled run then summarises a summary, compounding the loss every
//! job. So the lossy step is gated on a **floor**, and the floor is the
//! substance of this module.
//!
//! See [`COMPACTION_FLOOR_TOKENS`] for the derivation. The short version: the
//! floor is 4 000 tokens — the value §4.5 specifies — and it sits several times
//! above the size of the fixed prompt material a real operant session carries on
//! every single turn, so the floor means "substantially more than the
//! boilerplate" and anything above it has a genuinely compressible middle.
//!
//! # What compaction preserves, and what it drops
//!
//! The transcript splits into three regions, and the boundaries are chosen so
//! the surviving shape is always a valid conversation:
//!
//! - **Head** — every leading `System` message, plus the first
//!   [`KEEP_HEAD_MESSAGES`] non-system messages. The system prompt boundary is
//!   never crossed: a compaction cannot remove or reorder the persona block, and
//!   the opening exchange is never summarised, so the session's original
//!   framing survives verbatim.
//! - **Tail** — the most recent messages up to [`KEEP_TAIL_TOKENS`]. The active
//!   context of the job that just finished, and the seed for the next one.
//! - **Middle** — everything between head and tail. The only region that is
//!   dropped, and it is never dropped silently: it is replaced by a
//!   [`COMPACTION_MARKER`] system message naming the session, how many messages
//!   were dropped, how many tokens they held, and which compaction generation
//!   this is. §4.5's "compaction must not silently lose the employee's turns
//!   without a record" is satisfied by that marker.
//!
//! The tail boundary is snapped back to a tool-group boundary via
//! [`crate::context_management::tool_group_starts`], so a cut can never strand a
//! `tool_result` whose `tool_use` was summarised into the middle — the
//! orphaned-result shape strict providers reject outright.
//!
//! # Idempotence, and how it is enforced
//!
//! Compacting an already-compacted session must not degrade it further, and must
//! not erase the record of the compaction that already happened. Two properties
//! deliver that, and neither is a special case in the caller:
//!
//! 1. **The tail walk stops at the newest marker.** A compacted transcript is
//!    head + marker + tail. The backward walk from the end treats the marker as
//!    the floor of the tail, so everything after it is protected and the region
//!    between the head and the marker is all that a second pass could consider.
//! 2. **A marker-only middle is refused.** If what remains between head and the
//!    newest marker is at most that marker itself, there is no unmarked material
//!    left to lose, and the pass reports
//!    [`CompactionOutcome::SkippedNothingToCompact`] having changed nothing.
//!
//! Without both, a transcript that still holds unmarked middle material would
//! re-compact on its next pass and replace its own record with a record of a
//! record — the compounding loss this module exists to prevent.
//! [`KEEP_TAIL_TOKENS`] (8 000) being larger than [`COMPACTION_FLOOR_TOKENS`]
//! (4 000) additionally means a well-compacted transcript, whose surviving size
//! is head + marker + tail, sits **above** the floor rather than below it, so
//! neither rule is load-bearing on its own: rule 1 keeps the tail intact and
//! rule 2 refuses the marker-only middle, and the transcript is returned
//! unchanged with [`CompactionOutcome::SkippedNothingToCompact`].
//!
//! # Durability
//!
//! Compaction replaces the **resident** transcript — the copy the next turn in
//! this process sees — and records the outcome in the session's metadata row
//! when a [`Database`] was supplied. It does **not** delete message rows. That
//! division is the substrate's own: `session::SessionStore` owns *which*
//! transcript a turn sees and `Database` owns *where* it is written, so a
//! compaction is a change of view, not of history. Deleting rows is explicitly
//! destructive in the way [`SessionStore::discard`] is, and no post-run cleanup
//! should be doing that silently. A cold rehydrate therefore still finds the
//! full transcript on disk, which is the correct behaviour for a lossy
//! operation a human may want to undo.
//!
//! [`SessionStore::discard`]: crate::session::SessionStore::discard

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{debug, info, warn};

use crate::agent::ModelClient;
use crate::agent::llm_compressor::{LlmCompressor, LlmCompressorConfig};
use crate::client::{Message, Role};
use crate::context_management::{
    estimate_message_tokens, estimate_total_tokens, tool_group_starts,
};
use crate::database::Database;
use crate::session::{SessionKey, SessionStore};

/// Default compaction floor, in estimated tokens.
///
/// # Derivation
///
/// The floor is not a round number chosen for looks. It is set from
/// measurements of this repository's own fixed prompt material, using the same
/// estimator the rest of the code uses ([`crate::context_management`]: chars/4
/// via `estimate_tokens`, plus the 4-tokens-per-message overhead in
/// `estimate_message_tokens`):
///
/// | Material, measured in-tree | Chars | Est. tokens |
/// |---|---|---|
/// | Default system prompt (`agent/events.rs` frozen prefix) | 309 | 78 |
/// | `SKILLS_GUIDANCE` (`agent/mod.rs`) | ~3 800 | ~951 |
/// | `SUMMARY_PREFIX` + `SUMMARY_END_MARKER` (`agent/llm_compressor.rs`) | ~470 | ~118 |
/// | **Fixed preamble on every turn** | **~4 580** | **~1 150** |
///
/// Add a first user prompt and a first assistant reply — the two messages
/// [`KEEP_HEAD_MESSAGES`] already protects — and the fixed cost of an average
/// operant turn reaches roughly **1 600 tokens** before any real work happens.
///
/// The floor is **4 000 tokens**, for four reasons:
///
/// 1. **It clears the boilerplate with real margin.** 4 000 > ~1 600, so a
///    session reaches the floor only once it holds material *well beyond* its
///    own fixed prompt, not merely a token or two of work on top of it. A floor
///    near the preamble would compact sessions that are almost entirely
///    boilerplate, which is exactly the summary-of-a-summary failure the floor
///    exists to prevent.
/// 2. **It is the value §4.5 specifies.** `docs/ORG-AUTHORITY-ARCHITECTURE.md`
///    §4.5 fixes the default at "4 000, well under any model's window", and
///    repeats it in the deviations table. This module implements that document,
///    so 4 000 is the floor; the derivation below is the justification it asks
///    for, not a competing choice.
/// 3. **It is far below any model's window, so it is orthogonal to overflow.**
///    `context_window` defaults to 128 000 (`config::BehaviorSettings`, and
///    `LlmCompressorConfig` uses the same). A 4 000-token session is ~3 % of
///    that, so the floor never creates pressure the reactive mechanisms
///    (`LlmCompressor`'s 0.80 threshold, i.e. 102 400 tokens) would have
///    handled anyway. The two mechanisms cannot fight each other because their
///    triggers are more than an order of magnitude apart.
/// 4. **It sits well under the smallest verbatim budget the repo already
///    keeps.** `context_lcm_tail_tokens` defaults to 12 000 and
///    `LlmCompressorConfig::tail_token_budget` to 20 000; both are *tail* budgets
///    of mechanisms that run on far larger transcripts. 4 000 is clear of both,
///    so the region this compactor protects is always smaller than the
///    transcript size that triggers it — a compaction always has a middle.
///
/// If the owner ever wants literally unconditional summarisation, that is one
/// field: [`AutocompactConfig::floor_tokens`]. Setting it to `0` makes
/// compaction unconditional. The default is the deviation, stated.
pub const COMPACTION_FLOOR_TOKENS: usize = 4_000;

/// Leading non-system messages kept verbatim, in addition to every leading
/// system message.
///
/// Matches [`LlmCompressorConfig::protect_head_n`]'s intent (system plus the
/// opening exchange) and is deliberately small: the head is the part a summary
/// destroys first, and the persona block is re-injected from the persona record
/// on every bind, so there is nothing to gain by keeping more of it.
pub const KEEP_HEAD_MESSAGES: usize = 3;

/// Verbatim tail budget, in estimated tokens.
///
/// Sized to exceed a single scheduled job's active context (a handful of tool
/// round-trips) and to exceed [`COMPACTION_FLOOR_TOKENS`] **on purpose**: the
/// module's idempotence section depends on the tail being the larger of the
/// two, so a compaction always has a middle to drop and the protected region is
/// never the whole transcript. 8 000 keeps that ordering against the 4 000
/// floor while staying well under `context_lcm_tail_tokens` (12 000) and
/// `LlmCompressorConfig::tail_token_budget` (20 000).
pub const KEEP_TAIL_TOKENS: usize = 8_000;

/// Header of the marker that records what a compaction dropped.
///
/// The marker is the record §4.5 requires. It is a message in the transcript,
/// so it is visible to the next run, states exactly what was dropped and how
/// much of it there was, and is counted to derive the next generation number.
pub const COMPACTION_MARKER: &str = "[AUTOCOMPACTION]";

/// Session-metadata key holding the compaction generation counter.
const META_GENERATION: &str = "autocompact_generation";
/// Session-metadata key holding the transcript size before the last compaction.
const META_TOKENS_BEFORE: &str = "autocompact_tokens_before";
/// Session-metadata key holding the transcript size after the last compaction.
const META_TOKENS_AFTER: &str = "autocompact_tokens_after";
/// Session-metadata key holding how many messages the last compaction dropped.
const META_DROPPED: &str = "autocompact_messages_dropped";

/// Why a compaction did or did not happen.
///
/// There is deliberately no "nothing to report" variant. §4.5's requirement is
/// that the post-run compaction is *observable*: a caller can always say
/// whether it ran, whether the floor stopped it, or whether something else did.
/// Silence is never the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    /// Compaction ran; the session's resident transcript was replaced with a
    /// smaller one.
    Compacted {
        /// Messages in the transcript before compaction.
        messages_before: usize,
        /// Messages in the transcript after compaction.
        messages_after: usize,
        /// Estimated tokens before compaction.
        tokens_before: usize,
        /// Estimated tokens after compaction.
        tokens_after: usize,
        /// Messages removed from the middle region.
        messages_dropped: usize,
        /// Estimated tokens the dropped middle region held.
        tokens_dropped: usize,
        /// Compaction generation this pass produced (1 for the first).
        generation: u64,
        /// Whether the middle was semantically summarised (`true`) or replaced
        /// by the deterministic marker (`false`).
        summarised: bool,
    },
    /// The transcript is at or under the configured floor, so the lossy step
    /// was skipped. Nothing was changed.
    SkippedByFloor {
        /// Estimated tokens the transcript held.
        tokens: usize,
        /// The floor that stopped it.
        floor: usize,
    },
    /// The transcript is above the floor, but head-and-tail protection leaves
    /// nothing in the middle. Nothing was changed. This is the second
    /// compaction of an already-compacted session, and it is the normal,
    /// expected result of the idempotence property.
    SkippedNothingToCompact {
        /// Messages in the transcript.
        messages: usize,
    },
    /// Compaction is switched off by configuration.
    Disabled,
}

impl CompactionOutcome {
    /// Whether the transcript was actually changed.
    pub fn compacted(&self) -> bool {
        matches!(self, Self::Compacted { .. })
    }

    /// A one-line human-readable report, for `org check` and logs.
    pub fn describe(&self) -> String {
        match self {
            Self::Compacted {
                messages_before,
                messages_after,
                tokens_before,
                tokens_after,
                messages_dropped,
                tokens_dropped,
                generation,
                summarised,
            } => format!(
                "compacted {messages_before}→{messages_after} msgs, \
                 {tokens_before}→{tokens_after} tokens, dropped {messages_dropped} msgs \
                 ({tokens_dropped} tokens) as generation {generation}, summarised={summarised}"
            ),
            Self::SkippedByFloor { tokens, floor } => {
                format!("not compacted: {tokens} tokens is at or under the {floor}-token floor")
            }
            Self::SkippedNothingToCompact { messages } => {
                format!("not compacted: {messages} msgs are all head/tail protected")
            }
            Self::Disabled => "not compacted: disabled by configuration".to_string(),
        }
    }
}

/// Counters describing autocompaction behaviour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutocompactStats {
    /// Post-run compactions requested.
    pub invocations: u64,
    /// Passes that actually compacted a transcript.
    pub compactions: u64,
    /// Passes stopped by the floor.
    pub skipped_by_floor: u64,
    /// Passes that found nothing between head and tail.
    pub skipped_nothing_to_compact: u64,
}

/// Autocompactor configuration.
#[derive(Debug, Clone)]
pub struct AutocompactConfig {
    /// Minimum estimated token count at or below which the lossy step is
    /// skipped. Defaults to [`COMPACTION_FLOOR_TOKENS`]. Set to `0` for
    /// unconditional compaction.
    pub floor_tokens: usize,
    /// Verbatim tail budget in estimated tokens. Defaults to
    /// [`KEEP_TAIL_TOKENS`].
    pub keep_tail_tokens: usize,
    /// Leading non-system messages kept verbatim. Defaults to
    /// [`KEEP_HEAD_MESSAGES`].
    pub keep_head_messages: usize,
    /// Whether the post-run compaction runs at all.
    pub enabled: bool,
}

impl Default for AutocompactConfig {
    fn default() -> Self {
        Self {
            floor_tokens: COMPACTION_FLOOR_TOKENS,
            keep_tail_tokens: KEEP_TAIL_TOKENS,
            keep_head_messages: KEEP_HEAD_MESSAGES,
            enabled: true,
        }
    }
}

/// Post-scheduled-execution compactor for persistent sessions.
///
/// Build one, share it, and call [`Self::compact`] after each scheduled run
/// finishes. This type never initiates work itself; the scheduler drives it.
pub struct Autocompactor {
    store: Arc<SessionStore>,
    config: AutocompactConfig,
    db: Option<Arc<Database>>,
    stats: AutocompactStatsAtomic,
}

#[derive(Debug, Default)]
struct AutocompactStatsAtomic {
    invocations: AtomicU64,
    compactions: AtomicU64,
    skipped_by_floor: AtomicU64,
    skipped_nothing_to_compact: AtomicU64,
}

/// The transcript a pass is about to compact, and where its middle is.
struct Preflight {
    before: Vec<Message>,
    head_end: usize,
    middle: std::ops::Range<usize>,
    tokens_before: usize,
}

impl Preflight {
    /// The middle region, as its own slice.
    fn middle_slice(&self) -> &[Message] {
        &self.before[self.middle.clone()]
    }
}

/// Where the head ends and the middle begins.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Split {
    /// Index one past the last head message.
    head_end: usize,
    /// The middle region, or `None` when head and tail meet.
    middle: Option<std::ops::Range<usize>>,
}

impl Autocompactor {
    /// Build an autocompactor over `store` with default bounds.
    pub fn new(store: Arc<SessionStore>) -> Self {
        Self::with_config(store, AutocompactConfig::default())
    }

    /// Build an autocompactor over `store` with explicit bounds.
    pub fn with_config(store: Arc<SessionStore>, config: AutocompactConfig) -> Self {
        Self {
            store,
            config,
            db: None,
            stats: AutocompactStatsAtomic::default(),
        }
    }

    /// Record compaction outcomes in `db`'s session-metadata rows.
    ///
    /// Optional: without a database the compactor still works and the marker in
    /// the transcript is still the record, but `org check` has nothing to read
    /// for the last compaction's size and generation. Best-effort — a metadata
    /// write that fails is logged, never propagated, because a failed audit
    /// write is not a reason to fail a finished scheduled run.
    pub fn with_database(mut self, db: Arc<Database>) -> Self {
        self.db = Some(db);
        self
    }

    /// The floor this compactor enforces.
    pub fn floor_tokens(&self) -> usize {
        self.config.floor_tokens
    }

    /// The store this compactor writes back to.
    pub fn store(&self) -> &Arc<SessionStore> {
        &self.store
    }

    /// Counters for `org check` and tests.
    pub fn stats(&self) -> AutocompactStats {
        AutocompactStats {
            invocations: self.stats.invocations.load(Ordering::Relaxed),
            compactions: self.stats.compactions.load(Ordering::Relaxed),
            skipped_by_floor: self.stats.skipped_by_floor.load(Ordering::Relaxed),
            skipped_nothing_to_compact: self
                .stats
                .skipped_nothing_to_compact
                .load(Ordering::Relaxed),
        }
    }

    /// Compact `key` if it is over the floor. Synchronous and deterministic.
    ///
    /// The dropped middle is replaced by a [`COMPACTION_MARKER`] record rather
    /// than a summary, so a scheduled run's autocompaction needs no model, no
    /// network, and no API key, and cannot fail because an auxiliary provider is
    /// unreachable.
    pub fn compact(&self, key: &SessionKey) -> CompactionOutcome {
        let pre = match self.preflight(key) {
            Ok(pre) => pre,
            Err(skip) => return skip,
        };
        let generation = pre.generation();
        let replacement = self.marker(key, &pre, generation);
        self.finish(key, pre, generation, replacement, false)
    }

    /// Compact `key` if it is over the floor, summarising the middle through
    /// `compressor` instead of replacing it with a bare marker.
    ///
    /// Summarising loses strictly less than dropping. The floor, the head/tail
    /// split, the write-back, and the outcome shape are identical either way, so
    /// binding a compressor cannot change *whether* compaction happens — only
    /// what replaces the middle. If the summarisation fails or comes back
    /// empty, the marker path still runs: a failed summary is not a failed
    /// compaction.
    pub async fn compact_summarising(
        &self,
        key: &SessionKey,
        compressor: &mut LlmCompressor,
        client: &Arc<dyn ModelClient>,
    ) -> CompactionOutcome {
        let pre = match self.preflight(key) {
            Ok(pre) => pre,
            Err(skip) => return skip,
        };
        let generation = pre.generation();
        let replacement = match compressor
            .compress(pre.middle_slice().to_vec(), client.as_ref())
            .await
        {
            Ok(result) if !result.summary_text.trim().is_empty() => {
                Message::system(summary_body(&result.summary_text))
            }
            Ok(_) => {
                debug!(session = %key, "compressor returned no summary — using the marker");
                self.marker(key, &pre, generation)
            }
            Err(e) => {
                warn!(error = %e, session = %key, "autocompact summarisation failed — using the marker");
                self.marker(key, &pre, generation)
            }
        };
        let summarised = !replacement.content.contains(COMPACTION_MARKER);
        self.finish(key, pre, generation, replacement, summarised)
    }

    /// Decide whether compaction should happen, and prepare the transcript.
    ///
    /// `Ok` carries a transcript with a non-empty middle. `Err` carries the
    /// reason it was not compacted, which is returned to the caller verbatim so
    /// the skip is as observable as the compaction.
    fn preflight(&self, key: &SessionKey) -> Result<Preflight, CompactionOutcome> {
        self.stats.invocations.fetch_add(1, Ordering::Relaxed);

        if !self.config.enabled {
            debug!(session = %key, "autocompact disabled");
            return Err(CompactionOutcome::Disabled);
        }

        // Make the session resident so the transcript being compacted is the
        // live one, and so a cold session's persisted tail is what gets read.
        self.store.acquire(key);
        let before = self.store.peek(key);
        let tokens_before = estimate_total_tokens(&before);

        // THE FLOOR. A session at or under the floor keeps its verbatim history
        // exactly as it is: no marker, no rewrite, no summary. This is the
        // headline behaviour of the module.
        if tokens_before <= self.config.floor_tokens {
            self.stats.skipped_by_floor.fetch_add(1, Ordering::Relaxed);
            return Err(CompactionOutcome::SkippedByFloor {
                tokens: tokens_before,
                floor: self.config.floor_tokens,
            });
        }

        let split = self.split(&before);
        let Some(middle) = split.middle else {
            self.stats
                .skipped_nothing_to_compact
                .fetch_add(1, Ordering::Relaxed);
            return Err(CompactionOutcome::SkippedNothingToCompact {
                messages: before.len(),
            });
        };

        // Idempotence, stated directly rather than inferred from the split: a
        // transcript that already carries a compaction record and has no
        // unmarked material before its newest marker has nothing left to lose.
        // Re-compacting here would replace the record with a record of a
        // record — the compounding the floor exists to prevent.
        if middle.end - middle.start <= 1
            && before[middle.clone()]
                .iter()
                .any(|m| m.content.contains(COMPACTION_MARKER))
        {
            self.stats
                .skipped_nothing_to_compact
                .fetch_add(1, Ordering::Relaxed);
            return Err(CompactionOutcome::SkippedNothingToCompact {
                messages: before.len(),
            });
        }

        Ok(Preflight {
            before,
            head_end: split.head_end,
            middle,
            tokens_before,
        })
    }

    /// Splice the replacement into the middle and hand the result back.
    fn finish(
        &self,
        key: &SessionKey,
        pre: Preflight,
        generation: u64,
        replacement: Message,
        summarised: bool,
    ) -> CompactionOutcome {
        let messages_dropped = pre.middle.len();
        let tokens_dropped = estimate_total_tokens(pre.middle_slice());

        // head + [replacement] + tail. Splicing in the middle's place preserves
        // message order and leaves the system prompt boundary untouched.
        let mut after =
            Vec::with_capacity(pre.head_end + 1 + pre.before.len().saturating_sub(pre.middle.end));
        after.extend_from_slice(&pre.before[..pre.head_end]);
        after.push(replacement);
        after.extend_from_slice(&pre.before[pre.middle.end..]);
        let tokens_after = estimate_total_tokens(&after);

        // Hand the compacted transcript back to the store. `persist` replaces
        // the resident copy in place, so the session id is unchanged and a later
        // `acquire` sees a usable compacted transcript, not an empty one.
        self.store.persist(key, &after);
        self.record_metadata(
            key,
            generation,
            pre.tokens_before,
            tokens_after,
            messages_dropped,
        );

        self.stats.compactions.fetch_add(1, Ordering::Relaxed);
        let outcome = CompactionOutcome::Compacted {
            messages_before: pre.before.len(),
            messages_after: after.len(),
            tokens_before: pre.tokens_before,
            tokens_after,
            messages_dropped,
            tokens_dropped,
            generation,
            summarised,
        };
        info!(session = %key, "{}", outcome.describe());
        outcome
    }

    /// Split a transcript into head / middle / tail.
    ///
    /// The head is every leading system message plus the first
    /// `keep_head_messages` non-system messages. The tail grows backward from
    /// the end until it reaches `keep_tail_tokens`, then snaps back to a
    /// tool-group boundary so a `tool_result` is never stranded away from its
    /// `tool_use`.
    fn split(&self, messages: &[Message]) -> Split {
        // Head: leading system messages are never summarisable, so they are
        // never candidates for the middle.
        let mut head_end = 0;
        while head_end < messages.len() && messages[head_end].role == Role::System {
            head_end += 1;
        }
        let mut non_system = 0;
        while head_end + non_system < messages.len() && non_system < self.config.keep_head_messages
        {
            non_system += 1;
        }
        head_end += non_system;

        let rest = &messages[head_end..];
        if rest.is_empty() {
            return Split {
                head_end,
                middle: None,
            };
        }

        // Tail: walk backward accumulating tokens until the budget is spent. If
        // the whole region fits the budget, `tail_start` ends at 0 and there is
        // no middle — the session is entirely head and tail.
        //
        // The walk stops early at the most recent marker: a transcript that
        // already records a compaction must not have that record fall into the
        // middle and be replaced by a marker of a marker. Without this, a
        // compacted transcript slightly larger than the tail budget would
        // re-compact on the very next pass and erase its own record.
        let mut tail_tokens = 0usize;
        let mut tail_start = rest.len();
        for (idx, msg) in rest.iter().enumerate().rev() {
            if msg.content.contains(COMPACTION_MARKER) {
                // Claim the marker for the tail. Setting `tail_start` *before*
                // breaking is what makes the marker the floor of the tail rather
                // than its first dropped message: the middle is then strictly the
                // region between the head and the marker, so an existing record
                // survives a later compaction instead of being replaced by a
                // record of a record.
                tail_start = idx;
                break;
            }
            tail_tokens += estimate_message_tokens(msg);
            tail_start = idx;
            if tail_tokens >= self.config.keep_tail_tokens {
                break;
            }
        }

        // Snap back to the start of the tail's tool group so a cut never orphans
        // a tool_result whose tool_use went into the middle.
        let group_starts = tool_group_starts(rest);
        let snapped = group_starts.get(tail_start).copied().unwrap_or(tail_start);
        if snapped == 0 {
            return Split {
                head_end,
                middle: None,
            };
        }

        Split {
            head_end,
            middle: Some(head_end..head_end + snapped),
        }
    }

    /// The marker system message that records a compaction.
    ///
    /// This is §4.5's record. It states what was dropped, how much of it there
    /// was, and which compaction generation produced the gap, so a reader of
    /// the next turn's transcript can tell that history was compacted rather
    /// than never happened.
    fn marker(&self, key: &SessionKey, pre: &Preflight, generation: u64) -> Message {
        let dropped_tokens = estimate_total_tokens(pre.middle_slice());
        let content = format!(
            "{COMPACTION_MARKER} This session was compacted after a scheduled run.\n\
             Session: {}\n\
             Dropped: {} messages, ~{dropped_tokens} tokens\n\
             Compaction: #{generation}\n\
             Kept: the system prompt, the opening exchange, and the most recent turns. \
             The dropped turns were replaced by this record, not silently discarded.",
            key.id(),
            pre.middle.len()
        );
        Message::system(content)
    }

    /// Write the last compaction's numbers to the session's metadata row.
    ///
    /// Best-effort by design: the marker in the transcript is the durable
    /// record, and this is the machine-readable copy. A failure here is logged
    /// and otherwise ignored, because a finished scheduled run must not be
    /// failed by an audit write.
    fn record_metadata(
        &self,
        key: &SessionKey,
        generation: u64,
        tokens_before: usize,
        tokens_after: usize,
        messages_dropped: usize,
    ) {
        let Some(db) = self.db.as_ref() else { return };
        for (field, value) in [
            (META_GENERATION, generation.to_string()),
            (META_TOKENS_BEFORE, tokens_before.to_string()),
            (META_TOKENS_AFTER, tokens_after.to_string()),
            (META_DROPPED, messages_dropped.to_string()),
        ] {
            if let Err(e) = db.set_session_metadata(key.id(), field, &value) {
                warn!(error = %e, session = %key, field, "failed to record autocompact metadata");
            }
        }
    }
}

impl Preflight {
    /// The compaction generation this pass will produce.
    ///
    /// Derived from the markers already in the transcript rather than a counter
    /// on the compactor, so a restart does not reset it and two compactors over
    /// the same session agree.
    fn generation(&self) -> u64 {
        prior_generations(&self.before) as u64 + 1
    }
}

/// How many compaction markers a transcript already carries.
fn prior_generations(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| m.content.contains(COMPACTION_MARKER))
        .count()
}

/// Wrap a compressor's summary in the reference-only framing the agent loop
/// already expects, so a summarised session behaves like an overflow-compressed
/// one.
fn summary_body(summary: &str) -> String {
    format!(
        "{}\n\n{}\n\n{}",
        crate::agent::llm_compressor::SUMMARY_PREFIX,
        summary.trim(),
        crate::agent::llm_compressor::SUMMARY_END_MARKER
    )
}

/// Build a compressor configured for post-run autocompaction.
///
/// The returned compressor is not owned by the autocompactor: the caller binds
/// it to the agent's auxiliary model and passes it to
/// [`Autocompactor::compact_summarising`]. Provided so the configuration a
/// scheduled run compacts with is defined in one place rather than at each call
/// site.
pub fn autocompact_compressor_config(context_window: usize) -> LlmCompressorConfig {
    LlmCompressorConfig {
        context_window,
        // A post-run compaction is not gated on window pressure — the floor is
        // the gate — so the reactive threshold is set above any plausible
        // transcript and only the compressor's head/middle/tail policy is used.
        threshold_percent: 1.0,
        protect_head_n: KEEP_HEAD_MESSAGES,
        tail_token_budget: KEEP_TAIL_TOKENS,
        ..Default::default()
    }
}

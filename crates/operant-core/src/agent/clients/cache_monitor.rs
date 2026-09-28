//! Prompt-cache hit/miss detection — two complementary mechanisms.
//!
//! Operant already *sets* `cache_control` breakpoints (see
//! [`super::prompt_caching::apply_cache_control`]) and reads
//! `input_tokens`/`output_tokens` from the response. Before this module
//! nothing read the response's `cache_read_input_tokens` /
//! `cache_creation_input_tokens`, so a silently missing cache re-paid full
//! price for a large prefix on every turn and nothing said so.
//!
//! ## 1. Client-side prefix tracker ([`PrefixTracker`])
//!
//! Hash the *cacheable prefix* and compare it against the recently-seen
//! set. When the digest repeats, the request re-uses bytes the provider
//! has already been handed — a **necessary** condition for a hit, and the
//! strongest thing a client can know without the provider reporting
//! anything.
//!
//! What is hashed ([`CacheablePrefix::from_messages`]):
//!
//! * the **first `Role::System` message** — the frozen prefix
//!   (`build_frozen_prefix()`: base prompt + `<available_skills>` +
//!   `SKILLS_GUIDANCE` + harness prompt sections) plus whatever the
//!   memory provider appended. This is exactly the content sitting
//!   behind breakpoint #1 on the wire.
//! * **every enabled tool schema**, in order (name + description +
//!   parameter schema). Anthropic/OpenAI content-address the tool block
//!   ahead of `system`, so a tool-set change invalidates the cache just
//!   as a prompt change does.
//!
//! What is deliberately **not** hashed: the volatile suffix (memory
//! recall, workspace context), the MoA guidance, and the conversation
//! history. Those change every turn, and hashing them would report a
//! miss on every request — a detector that cries wolf is worse than no
//! detector. Because the history is excluded *structurally* (the tracker
//! only ever receives the prefix), "history grew" can never be mistaken
//! for "cache missed".
//!
//! Stability caveat, stated plainly: under the default `agentmemory`
//! provider the first system message embeds a live `/agentmemory/context`
//! response, so the on-wire prefix does change when the memory store
//! changes. The tracker will report `Miss` then — which is a **true**
//! statement about the bytes, and simultaneously a real cache-efficiency
//! finding. It is not a false positive.
//!
//! ## 2. Server-side monitoring seam ([`ServerCacheMonitor`])
//!
//! A process-wide registry a proxy/gateway in front of the provider can
//! push observations into — including for requests operant did not
//! originate. See [`ObservedCacheBehaviour`].
//!
//! ## Honesty contract
//!
//! Every verdict carries an [`Evidence`] tag and the verdict never
//! claims more than it knows:
//!
//! * [`Evidence::Proven`] — the provider reported `cache_read_*` /
//!   `cache_creation_*` token counts, or a proxy reported them.
//! * [`Evidence::Inferred`] — derived from prefix-hash continuity. A
//!   repeated digest proves *eligibility*, not a hit.
//! * [`Evidence::Unobserved`] — no baseline yet; nothing is claimed.
//!
//! `CacheVerdict::is_proven()` is the gate. Anything that is not proven
//! must render as such — see `impl Display for CacheVerdict`.

use crate::client::{Message, Role};
use crate::schema::ToolSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;
use std::sync::OnceLock;

/// Errors from the server-side monitoring seam. Typed, not stringly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheMonitorError {
    /// A report carried neither a cache-read nor a cache-creation count,
    /// so it cannot classify anything. Accepted shape, rejected content.
    #[error("cache observation reported neither cache_read_tokens nor cache_creation_tokens")]
    EmptyObservation,
    /// The `source` label is empty — an unattributable observation cannot
    /// be traced back to a proxy, so it is refused rather than counted.
    #[error("cache observation has an empty source label")]
    MissingSource,
}

// ── Evidence / outcome taxonomy ─────────────────────────────────────────

/// How much a verdict actually knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// The provider (or a proxy that saw the provider) reported
    /// `cache_read_input_tokens` / `cache_creation_input_tokens`.
    /// Observation, not inference.
    Proven,
    /// Derived from a prefix-hash comparison made on this machine. An
    /// unchanged digest proves the request was *eligible* for a cache
    /// hit; it does not prove one occurred.
    Inferred,
    /// Nothing was observed and no baseline exists. No claim is made.
    Unobserved,
}

/// What the verdict concludes about this request's cache behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheOutcome {
    /// The provider served part of the prefix from cache.
    Hit,
    /// The provider encoded the prefix fresh.
    Miss,
    /// Not enough information to say.
    Unknown,
}

/// Why an inferred verdict came out the way it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceReason {
    /// First request seen by this tracker — no baseline to compare to.
    NoBaseline,
    /// This exact prefix digest was already sent, so the request is
    /// eligible for a cache read. NOT proof of one.
    StablePrefix,
    /// The prefix bytes changed. A content-addressed cache cannot serve
    /// the new bytes, so the request cannot hit. (Still inferred: we did
    /// not observe the provider.)
    PrefixChanged,
    /// The provider reported usage but no cache counters, and there is
    /// no baseline either.
    ProviderSilent,
}

/// A single cache-health conclusion, with its evidence class attached.
///
/// The two fields are deliberately independent: `outcome` alone is not a
/// claim, `evidence` is what makes it honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheVerdict {
    pub outcome: CacheOutcome,
    pub evidence: Evidence,
    pub reason: InferenceReason,
    /// Provider-reported cache-read tokens. `Some` only when `evidence`
    /// is [`Evidence::Proven`].
    pub cache_read_tokens: Option<u64>,
    /// Provider-reported cache-creation tokens. `Some` only when
    /// `evidence` is [`Evidence::Proven`].
    pub cache_creation_tokens: Option<u64>,
}

impl CacheVerdict {
    /// True only when a provider (or a proxy that saw one) reported the
    /// counts. This is the single honest gate: anything else must not be
    /// presented to an operator as a confirmed cache hit.
    pub fn is_proven(&self) -> bool {
        self.evidence == Evidence::Proven
    }

    /// PROVEN hit: `cache_read_input_tokens > 0`.
    pub fn proven_hit(cache_read_tokens: u64, cache_creation_tokens: u64) -> Self {
        Self {
            outcome: CacheOutcome::Hit,
            evidence: Evidence::Proven,
            reason: InferenceReason::StablePrefix,
            cache_read_tokens: Some(cache_read_tokens),
            cache_creation_tokens: Some(cache_creation_tokens),
        }
    }

    /// PROVEN miss: the provider encoded the prefix fresh
    /// (`cache_read == 0`, `cache_creation > 0`).
    pub fn proven_miss(cache_read_tokens: u64, cache_creation_tokens: u64) -> Self {
        Self {
            outcome: CacheOutcome::Miss,
            evidence: Evidence::Proven,
            reason: InferenceReason::PrefixChanged,
            cache_read_tokens: Some(cache_read_tokens),
            cache_creation_tokens: Some(cache_creation_tokens),
        }
    }

    /// INFERRED hit: the prefix digest was already seen, so the request
    /// was eligible. Explicitly *not* a confirmed cache read.
    pub fn inferred_hit() -> Self {
        Self {
            outcome: CacheOutcome::Hit,
            evidence: Evidence::Inferred,
            reason: InferenceReason::StablePrefix,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        }
    }

    /// INFERRED miss: the prefix bytes changed.
    pub fn inferred_miss() -> Self {
        Self {
            outcome: CacheOutcome::Miss,
            evidence: Evidence::Inferred,
            reason: InferenceReason::PrefixChanged,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        }
    }

    /// Nothing observed, nothing claimed.
    pub fn unobserved(reason: InferenceReason) -> Self {
        Self {
            outcome: CacheOutcome::Unknown,
            evidence: Evidence::Unobserved,
            reason,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        }
    }

    /// Classify provider-reported counters. `None` on both means the
    /// provider said nothing about caching.
    pub fn from_reported(
        cache_read: Option<u64>,
        cache_creation: Option<u64>,
        baseline: Option<InferenceReason>,
    ) -> Self {
        match (cache_read, cache_creation) {
            (Some(0), Some(created)) if created > 0 => Self::proven_miss(0, created),
            (Some(read), created) if read > 0 => Self::proven_hit(read, created.unwrap_or(0)),
            _ => Self::unobserved(baseline.unwrap_or(InferenceReason::ProviderSilent)),
        }
    }
}

impl fmt::Display for CacheVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.outcome, self.evidence) {
            (CacheOutcome::Hit, Evidence::Proven) => write!(
                f,
                "cache hit (proven: {} tokens read from cache)",
                self.cache_read_tokens.unwrap_or(0)
            ),
            (CacheOutcome::Miss, Evidence::Proven) => write!(
                f,
                "cache MISS (proven: {} tokens written to cache, 0 read)",
                self.cache_creation_tokens.unwrap_or(0)
            ),
            (CacheOutcome::Hit, Evidence::Inferred) => write!(
                f,
                "cache hit (INFERRED: cacheable prefix unchanged and previously sent — \
                 provider did not report cache usage, not a confirmed read)"
            ),
            (CacheOutcome::Miss, Evidence::Inferred) => write!(
                f,
                "cache MISS (INFERRED: cacheable prefix changed, so a content-addressed \
                 cache cannot serve it — provider did not report cache usage)"
            ),
            _ => write!(
                f,
                "cache behaviour unknown ({:?}, no provider-reported cache counters)",
                self.reason
            ),
        }
    }
}

// ── Prefix digest ──────────────────────────────────────────────────────

/// Truncated SHA-256 of the cacheable prefix.
///
/// 16 hex chars (64 bits) — enough to make an accidental collision
/// irrelevant at session scale while keeping the log line short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrefixDigest([u8; 8]);

impl PrefixDigest {
    /// Hash `system` + `tools` into a stable digest.
    ///
    /// The cache TTL is deliberately *not* part of the digest: TTL governs
    /// how long an entry lives, not what it contains, so a 5m→1h switch
    /// does not change which bytes the provider would serve from cache.
    pub fn of(system: &str, tools: &[ToolSchema]) -> Self {
        let mut hasher = Sha256::new();
        // Length-prefix each field so ("ab","c") and ("a","bc") cannot
        // collide through concatenation.
        hasher.update((system.len() as u64).to_le_bytes());
        hasher.update(system.as_bytes());
        hasher.update((tools.len() as u64).to_le_bytes());
        for tool in tools {
            let name = tool.name.as_str();
            let description = tool.description.as_str();
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hasher.update((description.len() as u64).to_le_bytes());
            hasher.update(description.as_bytes());
            // `parameters` is a serde_json::Value; its `to_string` is the
            // canonical serialisation and is deterministic for a given
            // map, so this is stable across turns.
            let params = tool.parameters.to_string();
            hasher.update((params.len() as u64).to_le_bytes());
            hasher.update(params.as_bytes());
        }
        let full = hasher.finalize();
        let mut out = [0u8; 8];
        out.copy_from_slice(&full[..8]);
        Self(out)
    }

    /// Lowercase hex, the form a proxy would put on the wire.
    pub fn as_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Parse the hex form produced by [`PrefixDigest::as_hex`].
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 16 {
            return None;
        }
        let mut out = [0u8; 8];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let pair = std::str::from_utf8(chunk).ok()?;
            out[i] = u8::from_str_radix(pair, 16).ok()?;
        }
        Some(Self(out))
    }
}

impl fmt::Display for PrefixDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_hex())
    }
}

/// The byte-stable head of a request: everything the provider can serve
/// from its prompt cache.
#[derive(Debug, Clone)]
pub struct CacheablePrefix {
    /// First system message on the wire (the frozen prefix + whatever the
    /// memory provider appended).
    pub system: String,
    /// Enabled tool schemas, in request order.
    pub tools: Vec<ToolSchema>,
}

impl CacheablePrefix {
    /// Build directly from a system prompt + tool list (tests, proxies).
    pub fn new(system: impl Into<String>, tools: &[ToolSchema]) -> Self {
        Self {
            system: system.into(),
            tools: tools.to_vec(),
        }
    }

    /// Extract the cacheable head of a request message list.
    ///
    /// Takes the **first** `Role::System` message only. Every later system
    /// message is a volatile block (memory recall, workspace context, MoA
    /// guidance) and the whole conversation after it grows every turn —
    /// both are excluded by construction, so a growing conversation can
    /// never be misreported as a cache miss.
    ///
    /// Returns `None` when the request carries no system message, i.e.
    /// there is no cacheable head to track.
    pub fn from_messages(messages: &[Message], tools: &[ToolSchema]) -> Option<Self> {
        let first = messages.iter().find(|m| m.role == Role::System)?;
        Some(Self::new(first.content.clone(), tools))
    }

    pub fn digest(&self) -> PrefixDigest {
        PrefixDigest::of(&self.system, &self.tools)
    }
}

// ── Client-side tracker ────────────────────────────────────────────────

/// How many distinct prefix digests stay warm. Small on purpose: the
/// realistic set is one per (model, skill-set) combination plus a couple
/// of routed-model alternates.
const WARM_PREFIX_CAPACITY: usize = 8;

/// Client-side prefix tracker.
///
/// Remembers the last N distinct [`PrefixDigest`]s. A digest already in
/// the warm set means the exact same cacheable bytes were sent before, so
/// the request is **eligible** for a cache read. A new digest means the
/// bytes changed and a content-addressed cache cannot serve them.
///
/// Cheap to clone (`Arc`-backed inner), so the agent loop and any number
/// of readers can hold one.
#[derive(Debug, Clone)]
pub struct PrefixTracker {
    warm: std::sync::Arc<Mutex<VecDeque<PrefixDigest>>>,
}

impl Default for PrefixTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl PrefixTracker {
    pub fn new() -> Self {
        Self {
            warm: std::sync::Arc::new(Mutex::new(VecDeque::with_capacity(WARM_PREFIX_CAPACITY))),
        }
    }

    /// Record `prefix` and report what its digest implies.
    pub fn observe(&self, prefix: &CacheablePrefix) -> CacheVerdict {
        self.observe_digest(prefix.digest())
    }

    /// Same as [`PrefixTracker::observe`], for callers that already hold
    /// the digest (avoids re-hashing).
    ///
    /// A poisoned lock must not wedge the agent loop, so the guard is
    /// recovered rather than unwrapped.
    pub fn observe_digest(&self, digest: PrefixDigest) -> CacheVerdict {
        let (seen_before, had_baseline) = {
            let mut warm = self
                .warm
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let seen_before = warm.contains(&digest);
            let had_baseline = !warm.is_empty();
            if !seen_before {
                if warm.len() >= WARM_PREFIX_CAPACITY {
                    warm.pop_front();
                }
                warm.push_back(digest);
            }
            (seen_before, had_baseline)
        };

        if seen_before {
            CacheVerdict::inferred_hit()
        } else if !had_baseline {
            CacheVerdict::unobserved(InferenceReason::NoBaseline)
        } else {
            CacheVerdict::inferred_miss()
        }
    }

    /// Number of distinct prefixes currently warm. Exposed for
    /// diagnostics and the tests below.
    pub fn warm_len(&self) -> usize {
        self.warm
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

// ── Server-side monitoring seam ────────────────────────────────────────

/// One observed cache behaviour, as reported by a provider response or
/// by a proxy/gateway sitting in front of the provider.
///
/// Construct one of these and hand it to
/// [`ServerCacheMonitor::report`]. Works for requests operant did not
/// originate — a shared gateway can report for every tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedCacheBehaviour {
    /// Who observed it: `"operant:anthropic"`, `"proxy:litellm"`, …
    /// Required — an unattributable observation is refused.
    pub source: String,
    /// Provider the request went to (`"anthropic"`, `"openai"`, …).
    pub provider: String,
    /// Model name, when known.
    #[serde(default)]
    pub model: Option<String>,
    /// [`PrefixDigest::as_hex`] of the request's cacheable prefix, when the
    /// reporter can compute or forward it. This is what lets a proxy
    /// observation be reconciled with a client-side prefix digest.
    #[serde(default)]
    pub prefix_hash: Option<String>,
    /// Anthropic `cache_read_input_tokens` / OpenAI
    /// `prompt_tokens_details.cached_tokens`.
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    /// Anthropic `cache_creation_input_tokens`.
    #[serde(default)]
    pub cache_creation_tokens: Option<u64>,
}

impl ObservedCacheBehaviour {
    /// A provider-reported observation for a request operant made itself.
    pub fn from_provider(
        source: impl Into<String>,
        provider: impl Into<String>,
        model: Option<String>,
        cache_read_tokens: Option<u64>,
        cache_creation_tokens: Option<u64>,
    ) -> Self {
        Self {
            source: source.into(),
            provider: provider.into(),
            model,
            prefix_hash: None,
            cache_read_tokens,
            cache_creation_tokens,
        }
    }

    /// Attach a prefix digest so the observation reconciles with the
    /// client-side tracker.
    pub fn with_prefix_hash(mut self, digest: PrefixDigest) -> Self {
        self.prefix_hash = Some(digest.as_hex());
        self
    }

    /// The verdict this observation supports, or an error-free
    /// `unobserved` when the reporter said nothing about caching.
    pub fn verdict(&self) -> CacheVerdict {
        CacheVerdict::from_reported(self.cache_read_tokens, self.cache_creation_tokens, None)
    }
}

/// A report the seam accepted, with the classification it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheObservation {
    pub report: ObservedCacheBehaviour,
    pub verdict: CacheVerdict,
}

/// Aggregate view of everything the seam has accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CacheMonitorSnapshot {
    /// Total reports accepted.
    pub reports: u64,
    /// Reports that proved a cache read.
    pub proven_hits: u64,
    /// Reports that proved a fresh encode.
    pub proven_misses: u64,
    /// Reports that carried no cache counters at all.
    pub unclassified: u64,
    /// Distinct prefixes the seam has a proven verdict for.
    pub tracked_prefixes: usize,
}

impl fmt::Display for CacheMonitorSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} reports: {} proven hit, {} proven miss, {} unclassified \
             across {} tracked prefixes",
            self.reports,
            self.proven_hits,
            self.proven_misses,
            self.unclassified,
            self.tracked_prefixes
        )
    }
}

#[derive(Debug, Default)]
struct ServerState {
    /// Bounded ring of recent reports, for `recent()`.
    recent: VecDeque<CacheObservation>,
    /// Latest proven verdict per prefix hex.
    by_prefix: HashMap<String, CacheVerdict>,
    snapshot: CacheMonitorSnapshot,
}

/// Bounded number of retained reports.
const OBSERVATION_HISTORY: usize = 64;

/// The server-side monitoring seam.
///
/// Deliberately *not* a proxy: it accepts observations and answers
/// aggregate questions. It opens no sockets, spawns no tasks, and holds
/// no connection to any provider.
#[derive(Debug, Clone, Default)]
pub struct ServerCacheMonitor {
    state: std::sync::Arc<Mutex<ServerState>>,
}

impl ServerCacheMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept one observed cache behaviour.
    ///
    /// Errors only on an unusable report (no source label, or no cache
    /// counters at all) — a rejected report is never counted.
    pub fn report(
        &self,
        report: ObservedCacheBehaviour,
    ) -> Result<CacheObservation, CacheMonitorError> {
        if report.source.trim().is_empty() {
            return Err(CacheMonitorError::MissingSource);
        }
        if report.cache_read_tokens.is_none() && report.cache_creation_tokens.is_none() {
            return Err(CacheMonitorError::EmptyObservation);
        }

        let verdict = report.verdict();
        let observation = CacheObservation {
            report: report.clone(),
            verdict,
        };

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.snapshot.reports += 1;
        match verdict.outcome {
            CacheOutcome::Hit => state.snapshot.proven_hits += 1,
            CacheOutcome::Miss => state.snapshot.proven_misses += 1,
            CacheOutcome::Unknown => state.snapshot.unclassified += 1,
        }
        if let Some(hex) = &report.prefix_hash {
            state.by_prefix.insert(hex.clone(), verdict);
            state.snapshot.tracked_prefixes = state.by_prefix.len();
        }
        if state.recent.len() >= OBSERVATION_HISTORY {
            state.recent.pop_front();
        }
        state.recent.push_back(observation.clone());
        Ok(observation)
    }

    /// The latest proven verdict recorded for `digest`, if any. This is
    /// what upgrades a client-side INFERRED verdict to PROVEN on the next
    /// request that carries the same prefix.
    pub fn verdict_for(&self, digest: &PrefixDigest) -> Option<CacheVerdict> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.by_prefix.get(&digest.as_hex()).copied()
    }

    /// Most recent reports, newest last.
    pub fn recent(&self) -> Vec<CacheObservation> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.recent.iter().cloned().collect()
    }

    pub fn snapshot(&self) -> CacheMonitorSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot
    }
}

/// The process-wide seam. Same `OnceLock` idiom as `runtime_config()`.
pub fn server_cache_monitor() -> &'static ServerCacheMonitor {
    static MONITOR: OnceLock<ServerCacheMonitor> = OnceLock::new();
    MONITOR.get_or_init(ServerCacheMonitor::new)
}

// ── Reconciliation ─────────────────────────────────────────────────────

/// The digest plus the verdict the agent loop should act on.
///
/// Prefers a PROVEN verdict from the server seam for the same prefix;
/// otherwise falls back to the client-side inferred verdict.
///
/// The seam's verdict is the outcome of the **last** request that carried
/// these exact bytes, so a seam-backed verdict lags the current request by
/// one. That lag is deliberate and stated: repeating a proven result for
/// identical bytes is a claim about those bytes, whereas inventing a fresh
/// verdict for a request whose response has not arrived yet would be a
/// fabrication.
pub fn reconcile(
    prefix: &CacheablePrefix,
    tracker: &PrefixTracker,
) -> (PrefixDigest, CacheVerdict) {
    let digest = prefix.digest();
    let inferred = tracker.observe_digest(digest);
    let verdict = match server_cache_monitor().verdict_for(&digest) {
        Some(proven) => proven,
        None => inferred,
    };
    (digest, verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> ToolSchema {
        ToolSchema::new(
            name,
            format!("{name} description"),
            json!({"type": "object"}),
        )
    }

    fn tools() -> Vec<ToolSchema> {
        vec![tool("Bash"), tool("FileRead")]
    }

    /// The stable cacheable head is unchanged → the request is eligible
    /// for a cache read. Note the verdict is INFERRED, not proven: the
    /// provider reported nothing.
    #[test]
    fn prefix_tracker_should_report_hit_when_stable_prefix_is_unchanged() {
        let tracker = PrefixTracker::new();
        let prefix = CacheablePrefix::new("frozen system prompt", &tools());

        let first = tracker.observe(&prefix);
        assert_eq!(first.outcome, CacheOutcome::Unknown);
        assert_eq!(first.evidence, Evidence::Unobserved);

        let second = tracker.observe(&prefix);
        assert_eq!(second.outcome, CacheOutcome::Hit);
        assert_eq!(second.reason, InferenceReason::StablePrefix);
        assert!(
            !second.is_proven(),
            "an unchanged prefix hash proves eligibility, not a cache read"
        );
    }

    /// A different system prefix is a different digest → a content
    /// addressed cache cannot serve the new bytes.
    #[test]
    fn prefix_tracker_should_report_miss_when_system_prefix_changes() {
        let tracker = PrefixTracker::new();
        tracker.observe(&CacheablePrefix::new("prompt A", &tools()));

        let verdict = tracker.observe(&CacheablePrefix::new("prompt B", &tools()));
        assert_eq!(verdict.outcome, CacheOutcome::Miss);
        assert_eq!(verdict.reason, InferenceReason::PrefixChanged);
        assert_eq!(verdict.evidence, Evidence::Inferred);
    }

    /// The crucial one: the conversation grows every turn, and that must
    /// NOT read as a cache miss. The history is structurally excluded
    /// from the digest, so a 6-turn conversation with 20 messages still
    /// reports a hit.
    #[test]
    fn prefix_tracker_should_not_report_miss_merely_because_history_grew() {
        let tracker = PrefixTracker::new();
        let mut messages = vec![Message::system("frozen system prompt")];
        tracker.observe(
            &CacheablePrefix::from_messages(&messages, &tools()).expect("system message present"),
        );

        // Grow the conversation across five more turns, the way a real
        // ReAct loop does.
        for turn in 0..5 {
            messages.push(Message::user(format!("question {turn}")));
            messages.push(Message::assistant(format!("answer {turn}")));
            messages.push(Message::tool(format!("call_{turn}"), "result"));
            messages.push(Message::user(format!("follow-up {turn}")));

            let prefix =
                CacheablePrefix::from_messages(&messages, &tools()).expect("system present");
            let verdict = tracker.observe(&prefix);
            assert_eq!(
                verdict.outcome,
                CacheOutcome::Hit,
                "history growth ({} messages) must not read as a cache miss",
                messages.len()
            );
        }
    }

    /// A tool-set change invalidates the cache just like a prompt change:
    /// the tool block sits ahead of `system` in the provider's prefix.
    #[test]
    fn prefix_tracker_reports_miss_when_tool_set_changes() {
        let tracker = PrefixTracker::new();
        tracker.observe(&CacheablePrefix::new("same prompt", &tools()));

        let verdict = tracker.observe(&CacheablePrefix::new("same prompt", &[tool("Bash")]));
        assert_eq!(verdict.outcome, CacheOutcome::Miss);
    }

    /// PROVEN and INFERRED must never be confusable — that separation is
    /// the whole point of the module.
    #[test]
    fn cache_verdict_should_distinguish_proven_from_inferred() {
        let proven = CacheVerdict::proven_hit(1200, 0);
        let inferred = CacheVerdict::inferred_hit();

        assert_eq!(proven.outcome, inferred.outcome, "both are hit-shaped");
        assert_ne!(
            proven.evidence, inferred.evidence,
            "evidence class must differ"
        );
        assert!(proven.is_proven());
        assert!(!inferred.is_proven());
        assert_eq!(proven.cache_read_tokens, Some(1200));
        assert_eq!(
            inferred.cache_read_tokens, None,
            "an inferred verdict must not carry provider token counts"
        );
        assert!(
            proven.to_string().contains("proven"),
            "proven display must say so: {proven}"
        );
        assert!(
            inferred.to_string().contains("INFERRED"),
            "inferred display must not read as a confirmation: {inferred}"
        );
    }

    /// `from_reported` is the only bridge from wire data to a verdict, and
    /// it must refuse to invent a hit out of silence.
    #[test]
    fn cache_verdict_from_reported_never_invents_a_hit() {
        assert!(!CacheVerdict::from_reported(None, None, None).is_proven());
        assert_eq!(
            CacheVerdict::from_reported(None, None, None).outcome,
            CacheOutcome::Unknown
        );
        assert_eq!(
            CacheVerdict::from_reported(Some(0), Some(0), None).outcome,
            CacheOutcome::Unknown,
            "zero/zero is not a hit"
        );
        assert_eq!(
            CacheVerdict::from_reported(Some(0), Some(900), None).outcome,
            CacheOutcome::Miss
        );
        assert_eq!(
            CacheVerdict::from_reported(Some(900), Some(0), None).outcome,
            CacheOutcome::Hit
        );
        assert!(CacheVerdict::from_reported(Some(900), Some(0), None).is_proven());
    }

    /// The seam accepts an observation from a proxy, classifies it, and
    /// makes it retrievable by prefix digest.
    #[test]
    fn server_monitor_should_accept_observed_cache_behaviour() {
        let monitor = ServerCacheMonitor::new();
        let digest = PrefixDigest::of("frozen system prompt", &tools());

        let observation = monitor
            .report(ObservedCacheBehaviour {
                source: "proxy:litellm".to_string(),
                provider: "anthropic".to_string(),
                model: Some("claude-opus-4-5".to_string()),
                prefix_hash: Some(digest.as_hex()),
                cache_read_tokens: Some(2048),
                cache_creation_tokens: Some(0),
            })
            .expect("a proxy observation with cache counters is accepted");

        assert_eq!(observation.verdict.outcome, CacheOutcome::Hit);
        assert!(observation.verdict.is_proven());
        assert_eq!(observation.report.source, "proxy:litellm");

        let snapshot = monitor.snapshot();
        assert_eq!(snapshot.reports, 1);
        assert_eq!(snapshot.proven_hits, 1);
        assert_eq!(snapshot.proven_misses, 0);
        assert_eq!(snapshot.tracked_prefixes, 1);

        // The observation is retrievable by the client-side digest, which
        // is what lets it upgrade a later inferred verdict.
        let upgraded = monitor.verdict_for(&digest).expect("prefix was tracked");
        assert!(upgraded.is_proven());
        assert_eq!(upgraded.outcome, CacheOutcome::Hit);

        // And an unrelated prefix gets nothing — no cross-talk.
        let other = PrefixDigest::of("a different prompt", &tools());
        assert!(monitor.verdict_for(&other).is_none());
    }

    /// An unusable report is refused, not silently counted.
    #[test]
    fn server_monitor_rejects_unusable_reports() {
        let monitor = ServerCacheMonitor::new();

        let no_source = ObservedCacheBehaviour {
            source: "  ".to_string(),
            provider: "anthropic".to_string(),
            model: None,
            prefix_hash: None,
            cache_read_tokens: Some(1),
            cache_creation_tokens: Some(0),
        };
        assert_eq!(
            monitor.report(no_source).err(),
            Some(CacheMonitorError::MissingSource)
        );

        let no_counters =
            ObservedCacheBehaviour::from_provider("proxy:x", "anthropic", None, None, None);
        assert_eq!(
            monitor.report(no_counters).err(),
            Some(CacheMonitorError::EmptyObservation)
        );

        let snapshot = monitor.snapshot();
        assert_eq!(snapshot.reports, 0, "refused reports must not be counted");
    }

    /// Digest encoding round-trips so a proxy can hand back the hex form.
    #[test]
    fn prefix_digest_hex_round_trips() {
        let digest = PrefixDigest::of("system", &tools());
        let hex = digest.as_hex();
        assert_eq!(hex.len(), 16);
        assert_eq!(PrefixDigest::from_hex(&hex), Some(digest));
        assert_eq!(PrefixDigest::from_hex("short"), None);
        assert_eq!(PrefixDigest::from_hex("zzzzzzzzzzzzzzzz"), None);
    }

    /// Field boundaries are length-prefixed, so shifting a byte from the
    /// system prompt into a tool name cannot collide.
    #[test]
    fn prefix_digest_is_not_confusable_across_field_boundaries() {
        let a = PrefixDigest::of("ab", &[tool("c")]);
        let b = PrefixDigest::of("a", &[tool("bc")]);
        assert_ne!(a, b);
    }

    /// A warm-set, not a last-value slot: a request that reverts to an
    /// earlier prefix (a fork, a routed model alternating) still reads as
    /// eligible.
    #[test]
    fn prefix_tracker_remembers_more_than_the_last_prefix() {
        let tracker = PrefixTracker::new();
        let a = CacheablePrefix::new("A", &tools());
        let b = CacheablePrefix::new("B", &tools());

        tracker.observe(&a);
        assert_eq!(tracker.observe(&b).outcome, CacheOutcome::Miss);
        assert_eq!(
            tracker.observe(&a).outcome,
            CacheOutcome::Hit,
            "A is still warm, so returning to it is eligible, not a miss"
        );
    }
}

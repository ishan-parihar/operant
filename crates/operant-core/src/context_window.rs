//! One resolver for the model's context window (and its output ceiling).
//!
//! Before this module, `agent.context_window` was a single constant —
//! 128,000 — that every model was budgeted against, regardless of what the
//! provider actually served. MEASURED on the operator's deployment: the
//! route advertised `large-stack` at 1,048,576 tokens and the runtime sized
//! every request against 128,000, so the agent compacted at 12.5 % of the
//! real window and could never use the rest. Three sources of truth already
//! existed in the tree and none of them was read:
//!
//! * the provider's own `/models` listing (`context_length` — discarded by
//!   `model_catalog::parse_ids`, which kept only the id),
//! * the models.dev catalog (`limit.context` — `lookup_models_dev_context`
//!   had zero callers),
//! * the limit a rejected request teaches us (`context_limit:*` in
//!   `state_meta` — read, but as a permanent one-way ratchet).
//!
//! This module is the ONE place that decides the number. Precedence, highest
//! first:
//!
//! 1. **Configured** — `agent.context_window > 0`. An operator's explicit
//!    number always wins; it is the only lever that can cover a local alias
//!    no catalog knows.
//! 2. **Learned** — a limit a provider's own overflow error stated for this
//!    (provider, model). Revisable: see `Agent::note_request_succeeded`.
//! 3. **Endpoint** — the provider's `/models` row for this model.
//! 4. **Default** — [`DEFAULT_CONTEXT_WINDOW`], the historical 128k, now a
//!    last resort rather than the answer for every model.
//!
//! models.dev is deliberately NOT a leg: the service is deprecated and its
//! numbers are stale. The endpoint that serves the model is the authority.
//!
//! `0` means AUTOMATIC: resolve from the sources above. It is a legal config
//! value and the example TOML ships it, so a fresh install tracks the model
//! it is actually pointed at.
//!
//! The same resolution yields [`ResolvedWindow::max_output`], the model's
//! output ceiling. It is not cosmetic: the output fitter used to ask for
//! `window − prompt − headway` as `max_tokens`, which is ~1,008,000 tokens
//! on a 1M window — no provider accepts that, so every request would be a
//! 400 before generation started (see `context_management::fit_output_tokens`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::model_catalog::{self, WireFamily};

/// The window used when nothing else knows one — the historical 128k.
///
/// It is a LAST RESORT now, not the answer for every model. A provider with
/// an unknown window is budgeted conservatively rather than optimistically;
/// guessing larger would turn every turn into an overflow 400.
pub const DEFAULT_CONTEXT_WINDOW: usize = 128_000;

/// The smallest window the runtime can operate with: the 4,096-token
/// response reserve plus one token of input. Mirrors
/// `config::validate_context_window`'s floor.
pub const MIN_CONTEXT_WINDOW: usize = 4_097;

/// `agent.context_window = 0`: resolve from the live sources.
pub const AUTOMATIC: usize = 0;

/// The `max_tokens` the fitter requests when the model's own output ceiling
/// is unknown.
///
/// A provider rejects `max_tokens` above the model's output ceiling, and the
/// ceilings in the wild range from 4,096 (small local models) to 65,536
/// (Gemini). 8,192 is the largest value every widely-used route accepts: a
/// reply that needs more than that is continued by the finish-reason
/// machinery, whereas a request above the ceiling is a 400 before
/// generation. When a source KNOWS the ceiling (the normal path — the
/// window resolver carries it), that value is used instead.
pub const UNKNOWN_MODEL_OUTPUT_CEILING: usize = 8_192;

/// Where a resolved window came from. Surfaced by `operant context window`
/// so the operator can see WHY the agent is budgeting against a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSource {
    /// `agent.context_window` (an explicit operator decision).
    Configured,
    /// A limit a provider overflow error taught us for this route.
    Learned,
    /// The provider's own `/models` listing.
    Endpoint,

    /// [`DEFAULT_CONTEXT_WINDOW`] — nothing knew better.
    Default,
}

impl WindowSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Learned => "learned",
            Self::Endpoint => "endpoint",
            Self::Default => "default",
        }
    }
}

/// What the runtime budgets against, and where each figure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedWindow {
    /// Tokens the agent builds against (never zero, never below
    /// [`MIN_CONTEXT_WINDOW`]).
    pub window: usize,
    pub source: WindowSource,
    /// The model's output ceiling, when a source knew it.
    pub max_output: Option<usize>,
}

impl ResolvedWindow {
    /// The pre-resolution default: the historical 128k, marked as such.
    pub fn automatic() -> Self {
        Self {
            window: DEFAULT_CONTEXT_WINDOW,
            source: WindowSource::Default,
            max_output: None,
        }
    }

    /// The starting cell for an agent built from `agent.context_window`.
    ///
    /// No I/O: an explicit value is already resolved (source `Configured`);
    /// `0` starts at the fallback and `Agent::refresh_context_window`
    /// replaces it with the live answer on the first run.
    pub fn from_config(configured: usize) -> Self {
        if configured > AUTOMATIC {
            Self {
                window: configured.max(MIN_CONTEXT_WINDOW),
                source: WindowSource::Configured,
                max_output: None,
            }
        } else {
            Self::automatic()
        }
    }

    /// Clamp the caller's requested output cap to what this route accepts.
    ///
    /// `None` when the model's own ceiling is unknown — the fitter then
    /// applies [`UNKNOWN_MODEL_OUTPUT_CEILING`] itself.
    pub fn output_ceiling(&self) -> Option<usize> {
        self.max_output
    }
}

impl Default for ResolvedWindow {
    fn default() -> Self {
        Self::automatic()
    }
}

/// What the live sources said about one model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Discovered {
    pub context_window: Option<usize>,
    pub window_source: Option<WindowSource>,
    pub max_output: Option<usize>,
    /// Whether the provider's OWN listing answered.
    ///
    /// The distinction matters because "the endpoint said nothing" and "the
    /// endpoint could not be reached" are different facts with different
    /// remedies, and they used to look identical: both left the window at
    /// the 128k default, silently. On a 1M model that is 10% of the window
    /// the operator pays for — the same class of surprise as the original
    /// 4,242-token window, except it heals by luck when the endpoint comes
    /// back.
    pub endpoint_reachable: bool,
}

impl Discovered {
    /// Record whether the provider's own listing answered, without
    /// disturbing the answers this value carries.
    fn with_reachability(mut self, reachable: bool) -> Self {
        self.endpoint_reachable = reachable;
        self
    }
}

/// The `state_meta` key a learned limit lives under.
///
/// Scoped to provider AND model: a limit learned on one route must never
/// shrink another model's window, which is what made the old key's ratchet
/// so damaging when a fallback model answered the 400.
pub fn learned_limit_key(provider: &str, model: &str) -> String {
    format!("context_limit:{provider}:{model}")
}

/// The pure precedence step: no I/O, no locks, fully testable.
///
/// `configured == 0` means automatic. A learned limit is a CEILING — it
/// cannot raise the configured or discovered window above what it says,
/// because a rejected request is hard evidence that the larger number does
/// not fit.
pub fn resolve(
    configured: usize,
    learned: Option<usize>,
    discovered: Discovered,
) -> ResolvedWindow {
    // A KNOWN ceiling is used as stated: it is the provider's own number,
    // and clamping it to the unknown-model guard would truncate a model that
    // legitimately emits more (Gemini's 65,536, Claude's 64,000).
    let max_output = discovered
        .max_output
        .filter(|tokens| (256..=1_048_576).contains(tokens));
    let (window, source) = if configured > AUTOMATIC {
        (configured, WindowSource::Configured)
    } else if let Some(learned) = learned {
        (learned, WindowSource::Learned)
    } else if let Some(window) = discovered.context_window {
        (
            window,
            discovered.window_source.unwrap_or(WindowSource::Endpoint),
        )
    } else {
        (DEFAULT_CONTEXT_WINDOW, WindowSource::Default)
    };
    ResolvedWindow {
        window: window.max(MIN_CONTEXT_WINDOW),
        source,
        max_output,
    }
}

/// One `state_meta` read, parsed.
pub fn learned_from_meta(value: Option<String>) -> Option<usize> {
    value?.trim().parse::<usize>().ok().filter(|n| *n > 0)
}

// ---------------------------------------------------------------------------
// Discovery (network, cached)
// ---------------------------------------------------------------------------

/// How long a discovery answer (including a negative one) is reused.
///
/// The number is a compromise: long enough that a turn loop never re-fetches
/// per request, short enough that a provider adding a model or raising a
/// window is picked up without a restart. A negative answer caches for the
/// same period so an unreachable endpoint costs ONE timeout, not one per
/// turn.
const DISCOVERY_TTL: Duration = Duration::from_secs(600);

struct CacheEntry {
    at: Instant,
    discovered: Discovered,
}

static DISCOVERY_CACHE: Mutex<Option<HashMap<String, CacheEntry>>> = Mutex::new(None);

fn cache_key(provider: &str, base_url: &str, model: &str) -> String {
    format!("{provider}\u{1f}{base_url}\u{1f}{model}")
}

fn cache_get(key: &str) -> Option<Discovered> {
    let guard = match DISCOVERY_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let entry = guard.as_ref()?.get(key)?;
    (entry.at.elapsed() < DISCOVERY_TTL).then_some(entry.discovered)
}

fn cache_put(key: String, discovered: Discovered) {
    let mut guard = match DISCOVERY_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(
        key,
        CacheEntry {
            at: Instant::now(),
            discovered,
        },
    );
}

/// Forget every cached discovery (tests, and `operant context window --refresh`).
pub fn clear_discovery_cache() {
    if let Ok(mut guard) = DISCOVERY_CACHE.lock() {
        *guard = None;
    }
}

/// Ask the live sources what this route serves, cached per
/// (provider, base_url, model).
///
/// Endpoint first, models.dev underneath it: the provider's own listing is
/// authoritative for what it serves RIGHT NOW, the catalog fills in the
/// fields the listing omits (Anthropic lists ids with no context figure at
/// all; models.dev knows the window but not the operator's alias).
pub async fn discover(
    provider: &str,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> Discovered {
    let model = model.trim();
    if model.is_empty() {
        return Discovered::default();
    }
    let key = cache_key(provider, base_url, model);
    if let Some(hit) = cache_get(&key) {
        return hit;
    }
    // ONE fetch: `discover_endpoint` reports whether the endpoint ANSWERED
    // (not whether it had something to say about this model — "reachable but
    // silent" and "unreachable" are different facts, and the second one means
    // the window fell back to a default).
    //
    // The chain is CONFIGURED -> LEARNED -> ENDPOINT -> DEFAULT. models.dev is
    // NOT in it: the service is deprecated, its numbers are stale (they
    // disagreed with the operator's own route by an order of magnitude on
    // this host), and consulting it cost the turn a 15s-timeout network fetch
    // whenever the endpoint was unreachable. The endpoint that actually
    // SERVES the model is the only authority on what it accepts.
    let (endpoint_reachable, endpoint) =
        discover_endpoint(provider, base_url, api_key, model).await;
    let discovered = endpoint
        .unwrap_or_default()
        .with_reachability(endpoint_reachable);
    cache_put(key, discovered);
    discovered
}

async fn discover_endpoint(
    provider: &str,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
) -> (bool, Option<Discovered>) {
    let Some(family) = wire_family(provider) else {
        return (false, None);
    };
    let rows = match model_catalog::fetch_live_details(family, base_url, api_key).await {
        Ok(rows) => rows,
        // Unreachable, or no `/models` listing at all: the window then comes
        // from the next leg (the catalog, or the default), and the caller
        // must be able to say SO rather than letting it look like "not
        // reported".
        Err(_) => return (false, None),
    };
    // The endpoint ANSWERED, even when it has nothing to say about this
    // model: that is a different fact from being unreachable.
    let discovered = rows
        .iter()
        .find(|row| {
            row.id == model
                || row
                    .id
                    .rsplit('/')
                    .next()
                    .is_some_and(|leaf| leaf == model.rsplit('/').next().unwrap_or(model))
        })
        .map(|row| Discovered {
            context_window: row.context_length,
            window_source: row.context_length.map(|_| WindowSource::Endpoint),
            max_output: row.max_output,
            endpoint_reachable: true,
        });
    (true, discovered)
}

/// The wire family a provider's `/models` endpoint speaks, when it has one.
///
/// Mirrors the CLI's picker mapping: anything not Anthropic or Google is
/// OpenAI-compatible in practice (aroute, OpenRouter, vLLM, Ollama, LM Studio,
/// DeepSeek, …). `None` only for the Anthropic feature being compiled out.
pub fn wire_family(provider: &str) -> Option<WireFamily> {
    let provider = provider.to_ascii_lowercase();
    match provider.as_str() {
        "google" | "gemini" => Some(WireFamily::Google),
        #[cfg(feature = "anthropic")]
        "anthropic" | "claude" => Some(WireFamily::Anthropic),
        "" => None,
        _ => Some(WireFamily::OpenAiCompat),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "The endpoint answered but has nothing for this model" and "the
    /// endpoint could not be reached" are DIFFERENT facts, and they used to
    /// look identical: both left the window at the 128k default and said
    /// nothing. A provider outage silently budgeted a 1M model against 10%
    /// of the window it pays for.
    #[test]
    fn reachability_is_recorded_even_when_the_endpoint_has_no_answer() {
        // Reachable but silent for this model.
        let reachable = Discovered {
            endpoint_reachable: true,
            ..Discovered::default()
        };
        assert!(reachable.endpoint_reachable);
        // Unreachable: the default's reachability is false.
        assert!(!Discovered::default().endpoint_reachable);
    }

    /// The resolved window says WHICH source answered, so the caller can
    /// name it — and an automatic window with no answer at all falls back to
    /// the documented default, never to the 0 sentinel.
    #[test]
    fn an_automatic_window_with_no_answer_uses_the_default() {
        let resolved = resolve(AUTOMATIC, None, Discovered::default());
        assert_eq!(resolved.window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(resolved.source, WindowSource::Default);
    }

    use super::*;

    #[test]
    fn configured_wins_over_every_live_source() {
        let resolved = resolve(
            200_000,
            Some(64_000),
            Discovered {
                endpoint_reachable: false,
                context_window: Some(1_048_576),
                window_source: Some(WindowSource::Endpoint),
                max_output: Some(32_768),
            },
        );
        assert_eq!(resolved.window, 200_000);
        assert_eq!(resolved.source, WindowSource::Configured);
        assert_eq!(resolved.max_output, Some(32_768));
    }

    #[test]
    fn automatic_uses_the_endpoint_before_the_catalog() {
        let resolved = resolve(
            AUTOMATIC,
            None,
            Discovered {
                endpoint_reachable: false,
                context_window: Some(1_048_576),
                window_source: Some(WindowSource::Endpoint),
                max_output: Some(131_072),
            },
        );
        assert_eq!(resolved.window, 1_048_576);
        assert_eq!(resolved.source, WindowSource::Endpoint);
        // The ceiling is real, not cosmetic: a model may advertise an output
        // window larger than the request the fitter may ever ask for.
        assert_eq!(resolved.max_output, Some(131_072));
    }

    #[test]
    fn a_learned_limit_beats_the_endpoint_but_not_the_operator() {
        let discovered = Discovered {
            endpoint_reachable: false,
            context_window: Some(1_048_576),
            window_source: Some(WindowSource::Endpoint),
            max_output: None,
        };
        let auto = resolve(AUTOMATIC, Some(200_000), discovered);
        assert_eq!(auto.window, 200_000);
        assert_eq!(auto.source, WindowSource::Learned);
        let explicit = resolve(400_000, Some(200_000), discovered);
        assert_eq!(explicit.window, 400_000);
        assert_eq!(explicit.source, WindowSource::Configured);
    }

    #[test]
    fn nothing_known_falls_back_to_the_historical_default() {
        let resolved = resolve(AUTOMATIC, None, Discovered::default());
        assert_eq!(resolved.window, DEFAULT_CONTEXT_WINDOW);
        assert_eq!(resolved.source, WindowSource::Default);
        assert_eq!(resolved.max_output, None);
    }

    #[test]
    fn a_tiny_learned_probe_is_floored_at_the_minimum_window() {
        // A bad probe can teach 4,096; validate_context_window would reject
        // it as a config value, so the resolver must not adopt it verbatim.
        let resolved = resolve(AUTOMATIC, Some(4_096), Discovered::default());
        assert_eq!(resolved.window, MIN_CONTEXT_WINDOW);
        assert_eq!(resolved.source, WindowSource::Learned);
    }

    #[test]
    fn an_endpoint_window_is_labelled_endpoint() {
        let resolved = resolve(
            AUTOMATIC,
            None,
            Discovered {
                endpoint_reachable: true,
                context_window: Some(200_000),
                window_source: Some(WindowSource::Endpoint),
                max_output: None,
            },
        );
        assert_eq!(resolved.source, WindowSource::Endpoint);
        assert_eq!(resolved.window, 200_000);
    }

    #[test]
    fn learned_meta_parses_only_a_positive_number() {
        assert_eq!(learned_from_meta(Some("200000".into())), Some(200_000));
        assert_eq!(learned_from_meta(Some(" 0 ".into())), None);
        assert_eq!(learned_from_meta(Some("".into())), None);
        assert_eq!(learned_from_meta(Some("nope".into())), None);
        assert_eq!(learned_from_meta(None), None);
    }

    #[test]
    fn a_learned_limit_is_scoped_to_provider_and_model() {
        assert_ne!(
            learned_limit_key("openai", "gpt-5"),
            learned_limit_key("openai", "gpt-5-mini")
        );
        assert_ne!(
            learned_limit_key("openai", "gpt-5"),
            learned_limit_key("aroute", "gpt-5")
        );
    }

    #[test]
    fn wire_family_recognises_the_special_endpoints() {
        assert_eq!(wire_family("google"), Some(WireFamily::Google));
        assert_eq!(wire_family("gemini"), Some(WireFamily::Google));
        assert_eq!(
            wire_family("aroute"),
            Some(WireFamily::OpenAiCompat),
            "an OpenAI-compatible route must be probed, not skipped"
        );
        assert_eq!(wire_family(""), None);
    }
}

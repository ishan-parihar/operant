//! The upgrade contract: a load that heals, names what it healed, and
//! persists the result exactly once.
//!
//! # Why this exists
//!
//! `load_app_config_impl` used to normalize a config in memory and drop the
//! result on the floor. The operator who wrote a value the current binary
//! considers out of range kept the bad value in the file forever; the operator
//! who wrote a key a NEWER binary understands could not start at all, because
//! every settings struct carries `deny_unknown_fields` and serde turned the
//! unknown key into a hard error. The survey that named both:
//! `docs/audit/2026-11-14-the-config-surface-survey-...md` §3.
//!
//! # The contract
//!
//! One type ([`Healed`]) names every change the loader made, with the dotted
//! key, the old value, the new value and WHY. [`AppConfig::heal`] sums the
//! per-struct tables into one `Vec<Healed>`, so the answer to "what did the
//! load change?" is a value, not a log grep.
//!
//! The rules the tables obey:
//!
//! - **Total**: every settings struct contributes a table, so a new key with
//!   no rule still reaches the operator through the registry gate
//!   (`scripts/audit-config-keys.py`) rather than by accident.
//! - **Idempotent**: `heal()` on an already-healed config returns an empty
//!   `Vec`. A second load heals nothing and rewrites nothing.
//! - **Persisted once**: [`heal_and_persist`] writes the healed document back
//!   only when a row exists, atomically, through the same surgical writer
//!   every other config write uses.
//! - **Named**: one `warn!` line per row at load, so a headless run and a
//!   gateway log both carry what changed and why.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// One change the loader made to a config value.
///
/// `key` is the dotted path an operator can pass to `operant config notes`,
/// `old`/`new` are display strings (a value that cannot be rendered is
/// `<unset>`, not empty), and `why` is the static reason the row exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Healed {
    /// Dotted path, e.g. `client.rate_limit.max_retries`.
    pub key: &'static str,
    /// What the file said, rendered for display.
    pub old: String,
    /// What the loader now holds.
    pub new: String,
    /// Why the change was made. Static on purpose: a reason that changes per
    /// run is a message, and messages cannot be asserted in a test.
    pub why: &'static str,
}

impl Healed {
    /// A row for a key that was ABSENT and now holds its default.
    pub fn missing(key: &'static str, new: impl Into<String>, why: &'static str) -> Self {
        Self {
            key,
            old: "<unset>".to_string(),
            new: new.into(),
            why,
        }
    }

    /// A row for a key whose value was out of range and is now clamped.
    pub fn clamped(
        key: &'static str,
        old: impl Into<String>,
        new: impl Into<String>,
        why: &'static str,
    ) -> Self {
        Self {
            key,
            old: old.into(),
            new: new.into(),
            why,
        }
    }

    /// A row for a key whose value conflicted with another and lost.
    pub fn conflict(
        key: &'static str,
        old: impl Into<String>,
        new: impl Into<String>,
        why: &'static str,
    ) -> Self {
        Self {
            key,
            old: old.into(),
            new: new.into(),
            why,
        }
    }

    /// The `key: old -> new (why)` line the load logs and the doctor renders.
    pub fn render(&self) -> String {
        format!("{}: {} -> {} ({})", self.key, self.old, self.new, self.why)
    }
}

/// Clamp `value` into `lo..=hi`, returning the clamped value and the row when
/// it moved.
///
/// `lo > hi` is a programming error in the caller's constant, not a config
/// problem: it is returned as an empty result so a miswritten table cannot
/// produce an infinite or panicking heal.
pub(crate) fn clamp_u64(
    key: &'static str,
    value: u64,
    lo: u64,
    hi: u64,
    why: &'static str,
    out: &mut Vec<Healed>,
) -> u64 {
    if lo > hi {
        return value;
    }
    let clamped = value.clamp(lo, hi);
    if clamped != value {
        out.push(Healed::clamped(
            key,
            value.to_string(),
            clamped.to_string(),
            why,
        ));
    }
    clamped
}

/// Clamp `value` into `lo..=hi` for a `usize` field.
pub(crate) fn clamp_usize(
    key: &'static str,
    value: usize,
    lo: usize,
    hi: usize,
    why: &'static str,
    out: &mut Vec<Healed>,
) -> usize {
    if lo > hi {
        return value;
    }
    let clamped = value.clamp(lo, hi);
    if clamped != value {
        out.push(Healed::clamped(
            key,
            value.to_string(),
            clamped.to_string(),
            why,
        ));
    }
    clamped
}

/// Clamp a float, treating a non-finite value as the LOW bound.
///
/// `NaN` and the infinities parse from TOML, and every `PartialOrd` comparison
/// against them is false — a naive `if v < lo || v > hi` lets `NaN` through
/// untouched and it then poisons whatever arithmetic reads it.
pub(crate) fn clamp_f64(
    key: &'static str,
    value: f64,
    lo: f64,
    hi: f64,
    why: &'static str,
    out: &mut Vec<Healed>,
) -> f64 {
    let clamped = if value.is_nan() {
        lo
    } else {
        value.clamp(lo, hi)
    };
    if clamped != value || value.is_nan() {
        out.push(Healed::clamped(
            key,
            format!("{value}"),
            format!("{clamped}"),
            why,
        ));
    }
    clamped
}

// ── Unknown keys: the upgrade case ─────────────────────────────────

/// The sidecar that holds keys the current binary does not know.
///
/// A config written by a NEWER operant names keys this build has never heard
/// of. `deny_unknown_fields` made that a hard load failure, which is the
/// single worst upgrade behaviour a config loader can have: the operator
/// upgrades the binary and the daemon will not start.
///
/// The parked keys live in a SIDECAR, not in the operator's file, so:
///
/// - the operator's `operant.toml` stays exactly what they wrote;
/// - the parked set is inspectable and deletable in one known place;
/// - a downgrade back to the newer binary loses nothing, because the sidecar
///   is read back and merged before the strict parse.
///
/// The file is `[section] key = value` TOML — the same shape the operator's
/// config uses, so a human can read it without a tool.
pub const UNKNOWN_SIDECAR: &str = "operant.unknown.toml";

/// Where the unknown-key sidecar lives for a given config file.
pub fn unknown_sidecar_for(config_path: &Path) -> PathBuf {
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(UNKNOWN_SIDECAR)
}

/// Split a config document into (known-document, parked-unknown-document).
///
/// The split is by SECTION against a set of known dotted keys: a table whose
/// every leaf is known stays; a leaf that is unknown moves to the sidecar
/// with its value verbatim. An unknown TOP-LEVEL key is parked as its own
/// one-key table under `__unknown_top_level__`, because a bare key at the top
/// of a sidecar would collide with a section of the same name.
///
/// Returns `None` when nothing is unknown, so a clean config never rewrites
/// its own file and never creates an empty sidecar.
pub fn split_unknown(raw: &str, known: &dyn Fn(&str) -> bool) -> Option<(String, toml::Value)> {
    let doc: toml::Value = toml::from_str(raw).ok()?;
    let mut parked = toml::value::Table::new();
    let mut changed = false;

    let root = doc.as_table()?;
    let mut kept = toml::value::Table::new();

    for (name, item) in root {
        // A section that is not a table (a bare scalar at the top level) is
        // only "known" if the whole dotted path is.
        let Some(table) = item.as_table() else {
            if known(name) {
                kept.insert(name.clone(), item.clone());
            } else {
                parked.insert(format!("__unknown_top_level__.{name}"), item.clone());
                changed = true;
            }
            continue;
        };

        let mut kept_table = toml::value::Table::new();
        for (leaf, value) in table {
            let dotted = format!("{name}.{leaf}");
            if known(&dotted) || value.is_table() {
                // A nested table is recursed into only one level: the
                // settings tree is two levels deep everywhere it matters
                // (`[client.rate_limit] max_retries`), and a deeper nesting
                // is a provider profile the providers schema owns.
                if value.is_table() && !known(&dotted) {
                    parked.insert(dotted, value.clone());
                    changed = true;
                    continue;
                }
                kept_table.insert(leaf.clone(), value.clone());
            } else {
                parked.insert(dotted, value.clone());
                changed = true;
            }
        }
        if !kept_table.is_empty() {
            kept.insert(name.clone(), toml::Value::Table(kept_table));
        }
    }

    if !changed {
        return None;
    }

    let mut kept_doc = toml::value::Table::new();
    // Preserve the version stamp through a split: dropping it would make the
    // rewritten file look like a legacy v0 config to `config migrate`.
    if let Some(version) = kept.get("version") {
        kept_doc.insert("version".to_string(), version.clone());
    }
    for (name, item) in kept {
        if name == "version" {
            continue;
        }
        kept_doc.insert(name, item);
    }

    let kept_str = toml::to_string_pretty(&toml::Value::Table(kept_doc)).ok()?;
    Some((kept_str, toml::Value::Table(parked)))
}

/// Merge a parked sidecar back INTO a config document before the strict parse.
///
/// This is the downgrade path: a newer binary parked its keys, the operator
/// reinstalls the older binary, and the older binary must see its own keys
/// again instead of silently dropping them.
pub fn merge_parked(raw: &str, parked: &toml::Value) -> Option<String> {
    let mut doc: toml::Value = toml::from_str(raw).ok()?;
    let root = doc.as_table_mut()?;
    let parked = parked.as_table()?;

    for (dotted, value) in parked {
        let segments: Vec<&str> = dotted.split('.').collect();
        match segments.as_slice() {
            [leaf] => {
                // `__unknown_top_level__.name` and a bare top-level key both
                // land here; a bare key is a top-level config key.
                let leaf = leaf.strip_prefix("__unknown_top_level__.").unwrap_or(leaf);
                root.insert(leaf.to_string(), value.clone());
            }
            [section, leaf] => {
                let entry = root
                    .entry(section.to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let Some(table) = entry.as_table_mut() {
                    table.insert(leaf.to_string(), value.clone());
                }
            }
            _ => continue,
        }
    }

    toml::to_string_pretty(&doc).ok()
}

/// Render a parked value the way the loader would show it.
pub fn render_parked(parked: &Value) -> String {
    match parked {
        Value::Object(_) | Value::Array(_) => {
            toml::to_string_pretty(parked).unwrap_or_else(|_| "<complex>".to_string())
        }
        other => other.to_string(),
    }
}

/// The reason strings the heal tables pass. One place, so a test can assert
/// the operator-visible wording without repeating the literal.
pub mod why {
    /// A value was outside the range its reader can use.
    pub const RANGE: &str = "out of range; clamped to the nearest value the runtime can use";
    /// A negative or zero where only a positive value is meaningful.
    pub const POSITIVE: &str = "must be positive; healed to the documented default";
    /// A knob the current binary accepts but nothing reads.
    pub const INERT: &str = "the current binary does not read this key";
}
// ── The tables ─────────────────────────────────────────────────────

/// Every heal this build knows how to make, applied to `config` in place.
///
/// The returned rows name what changed. An EMPTY vec means the config is
/// already in the shape this binary wants, which is what makes the persist
/// step a no-op and the operation idempotent.
///
/// The tables below are the ones with a MEASURED reader: each entry names the
/// code that cannot use the value outside the range. A knob with no reader at
/// all is not clamped here — clamping a value nothing reads is theatre, and
/// the surface gate reports those instead.
pub fn heal_config(config: &mut crate::config::AppConfig) -> Vec<Healed> {
    let mut out = Vec::new();

    heal_client(&mut config.client, &mut out);
    heal_agent(&mut config.agent, &mut out);
    heal_gateway(&mut config.gateway, &mut out);
    heal_tool_search(&mut config.tools.tool_search, &mut out);
    heal_rate_limit(&mut config.client.rate_limit, &mut out);

    out
}

/// `[client]` — the transport's own knobs.
fn heal_client(client: &mut crate::config::ClientSettings, out: &mut Vec<Healed>) {
    // A zero header timeout turns every request into an instant failure on a
    // provider that takes a moment to answer: `reqwest` treats 0 as "no
    // timeout" in some versions and as an immediate deadline in others, so
    // the only safe reading is the documented floor.
    client.timeout_secs = clamp_u64(
        "client.timeout_secs",
        client.timeout_secs,
        1,
        86_400,
        why::RANGE,
        out,
    );
    // A blank base URL makes every request a relative-path error that names
    // no host. The default is the only value the client can route with.
    if client.base_url.trim().is_empty() {
        out.push(Healed::clamped(
            "client.base_url",
            "\"<empty>\"".to_string(),
            format!("\"{}\"", crate::config::ClientSettings::default().base_url),
            "a request needs an absolute base URL; restored to the default",
        ));
        client.base_url = crate::config::ClientSettings::default().base_url;
    }
    // Per-model timeouts of 0 mean "no per-model override" to the reasoning
    // floor lookup (it treats 0 as absent), so a 0 here is silently the same
    // as deleting the entry. Drop it rather than keep a key that lies.
    client.model_request_timeouts.retain(|model, secs| {
        if *secs == 0 {
            out.push(Healed::clamped(
                "client.model_request_timeouts",
                format!("{model} = 0"),
                "<entry removed>".to_string(),
                "0 is indistinguishable from an absent per-model timeout; the entry is dropped",
            ));
            return false;
        }
        true
    });
}

/// `[agent]` — the behaviour knobs with a range a reader can violate.
fn heal_agent(agent: &mut crate::config::BehaviorSettings, out: &mut Vec<Healed>) {
    // `max_iterations` of 0 ends the turn before the model is called once:
    // `iteration_budget` starts at 0 and the loop's own guard is
    // `iteration >= max`, so a 0 is a turn that never runs.
    agent.max_iterations = clamp_usize(
        "agent.max_iterations",
        agent.max_iterations,
        1,
        10_000,
        why::POSITIVE,
        out,
    );
    agent.tool_timeout_secs = clamp_u64(
        "agent.tool_timeout_secs",
        agent.tool_timeout_secs,
        1,
        86_400,
        why::POSITIVE,
        out,
    );
    // The request ceiling: 0 means the client's own timeout governs (which is
    // the documented "unset"), so 0 is legal and is left alone. The UPPER
    // bound exists because a year-long ceiling is the same as none.
    if agent.request_timeout_secs > 86_400 {
        out.push(Healed::clamped(
            "agent.request_timeout_secs",
            agent.request_timeout_secs.to_string(),
            "86400".to_string(),
            why::RANGE,
        ));
        agent.request_timeout_secs = 86_400;
    }
    agent.max_healing_attempts = clamp_usize(
        "agent.max_healing_attempts",
        agent.max_healing_attempts,
        0,
        100,
        why::RANGE,
        out,
    );
    heal_retry(&mut agent.retry, out);
}

/// `[agent.retry]` — the two-span agentic retry policy.
///
/// `large_span_secs` of 0 is NOT healed to a positive number the way other
/// seconds-valued keys are: a zero-length large span is a busy loop against a
/// dead provider (worse than the hang the policy exists to remove), so the
/// policy clamps it to 1 s at the point of use and the heal names it. A
/// `small_span_retries` of 0 is legal and means "no small span", so it is left
/// alone. `max_span_secs` of 0 is the documented INFINITE, not a defect.
fn heal_retry(retry: &mut crate::config::RetrySettings, out: &mut Vec<Healed>) {
    if retry.large_span_secs == 0 {
        out.push(Healed::clamped(
            "agent.retry.large_span_secs",
            "0".to_string(),
            "1".to_string(),
            "a zero-length large span is a busy loop against a dead provider; \
             the smallest usable span is 1 second",
        ));
        retry.large_span_secs = 1;
    }
    // The ceiling is opt-in and `0` means infinite. The only value worth
    // naming is one the operator cannot have meant: a ceiling SHORTER than
    // the span it bounds, which stops the ladder before its first large-span
    // retry ever fires — a config that silently does nothing.
    if retry.max_span_secs > 0 && retry.max_span_secs < retry.large_span_secs {
        out.push(Healed::clamped(
            "agent.retry.max_span_secs",
            retry.max_span_secs.to_string(),
            retry.large_span_secs.to_string(),
            "a ceiling shorter than the large span stops the ladder before its \
             first large-span retry; the ceiling is raised to one span",
        ));
        retry.max_span_secs = retry.large_span_secs;
    }
}

/// `[gateway]` — session bounds and retention.
fn heal_gateway(gateway: &mut crate::config::GatewaySettings, out: &mut Vec<Healed>) {
    // 0 concurrent sessions refuses every message before it reaches an agent,
    // which is indistinguishable from a broken gateway. The cap is a bound,
    // not an on/off switch: `None` is unlimited.
    if let Some(cap) = gateway.max_concurrent_sessions {
        let healed = clamp_usize(
            "gateway.max_concurrent_sessions",
            cap,
            1,
            10_000,
            why::POSITIVE,
            out,
        );
        gateway.max_concurrent_sessions = Some(healed);
    }
    // 0 retention days means "keep forever" and is documented as such — a
    // valid, deliberate choice. Nothing to clamp.
    let _ = &gateway.session_retention_days;
}

/// `[tools.tool_search]` — a percentage and two token budgets.
fn heal_tool_search(search: &mut crate::config::ToolSearchSettings, out: &mut Vec<Healed>) {
    // A threshold above 100 would let the catalog listing claim more than the
    // whole window, which is the "evict the conversation to describe the
    // tools" failure.
    search.threshold_pct = clamp_f64(
        "tools.tool_search.threshold_pct",
        search.threshold_pct,
        0.0,
        100.0,
        why::RANGE,
        out,
    );
    search.listing_max_tokens = clamp_usize(
        "tools.tool_search.listing_max_tokens",
        search.listing_max_tokens,
        0,
        1_000_000,
        why::RANGE,
        out,
    );
    // The default limit must not exceed the cap: `tool_search` clamps at
    // request time, so a default above the cap is a key that lies about what
    // it will return.
    if search.search_default_limit > search.max_search_limit {
        out.push(Healed::conflict(
            "tools.tool_search.search_default_limit",
            search.search_default_limit.to_string(),
            search.max_search_limit.to_string(),
            "a default limit above max_search_limit is clamped down by the tool anyway",
        ));
        search.search_default_limit = search.max_search_limit;
    }
}

/// `[client.rate_limit]` — the L1 transport ladder.
fn heal_rate_limit(rl: &mut crate::config::RateLimitSettings, out: &mut Vec<Healed>) {
    // `max_retries` counts the RETRIES, and 0 is meaningful ("do not retry"),
    // so only a negative-class value (impossible for u32) or an absurd count
    // needs healing. The ceiling is one hour of retries at the max delay.
    rl.max_retries = clamp_u64(
        "client.rate_limit.max_retries",
        u64::from(rl.max_retries),
        0,
        1_000,
        why::RANGE,
        out,
    ) as u32;
    rl.base_delay_secs = clamp_u64(
        "client.rate_limit.base_delay_secs",
        rl.base_delay_secs,
        0,
        86_400,
        why::RANGE,
        out,
    );
    rl.max_delay_secs = clamp_u64(
        "client.rate_limit.max_delay_secs",
        rl.max_delay_secs,
        0,
        86_400,
        why::RANGE,
        out,
    );
    // A max delay below the base makes the exponential backoff a flat line at
    // the max — legal, but it silently defeats the ladder's shape, so name it.
    if rl.max_delay_secs < rl.base_delay_secs {
        out.push(Healed::conflict(
            "client.rate_limit.max_delay_secs",
            rl.max_delay_secs.to_string(),
            rl.base_delay_secs.to_string(),
            "max_delay_secs below base_delay_secs flattens the backoff to a constant",
        ));
        rl.max_delay_secs = rl.base_delay_secs;
    }
    rl.bucket_refill_rate = clamp_f64(
        "client.rate_limit.bucket_refill_rate",
        rl.bucket_refill_rate,
        0.0,
        10_000.0,
        why::RANGE,
        out,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract's own rule 1: a healed config heals to nothing.
    #[test]
    fn heal_is_idempotent_on_defaults() {
        let mut config = crate::config::AppConfig::default();
        let first = heal_config(&mut config);
        let second = heal_config(&mut config);
        assert!(second.is_empty(), "second heal moved: {second:?}");
        // The default IS already in range, so the first pass is empty too.
        assert!(first.is_empty(), "default config healed: {first:?}");
    }

    #[test]
    fn out_of_range_values_are_clamped_and_named() {
        let mut config = crate::config::AppConfig::default();
        config.agent.max_iterations = 0;
        config.client.timeout_secs = 0;
        let rows = heal_config(&mut config);
        assert_eq!(config.agent.max_iterations, 1);
        assert_eq!(config.client.timeout_secs, 1);
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"agent.max_iterations"), "{keys:?}");
        assert!(keys.contains(&"client.timeout_secs"), "{keys:?}");
        for row in &rows {
            assert!(!row.why.is_empty(), "row without a reason: {row:?}");
            assert!(!row.new.is_empty(), "row without a new value: {row:?}");
        }
    }

    /// A NaN threshold survives every `PartialOrd` comparison, so a naive
    /// range check lets it through and it then poisons the listing budget.
    #[test]
    fn nan_threshold_is_healed_not_passed_through() {
        let mut config = crate::config::AppConfig::default();
        config.tools.tool_search.threshold_pct = f64::NAN;
        let rows = heal_config(&mut config);
        assert_eq!(config.tools.tool_search.threshold_pct, 0.0);
        assert!(
            rows.iter()
                .any(|r| r.key == "tools.tool_search.threshold_pct"),
            "a NaN threshold must produce a row"
        );
    }

    #[test]
    fn default_limit_above_cap_is_reported_as_a_conflict() {
        let mut config = crate::config::AppConfig::default();
        config.tools.tool_search.max_search_limit = 5;
        config.tools.tool_search.search_default_limit = 10;
        let rows = heal_config(&mut config);
        assert_eq!(config.tools.tool_search.search_default_limit, 5);
        assert!(
            rows.iter()
                .any(|r| r.key == "tools.tool_search.search_default_limit"
                    && r.old == "10"
                    && r.new == "5"),
            "the conflict row must name both values: {rows:?}"
        );
    }

    #[test]
    fn zero_session_cap_is_healed_to_one() {
        let mut config = crate::config::AppConfig::default();
        config.gateway.max_concurrent_sessions = Some(0);
        let rows = heal_config(&mut config);
        assert_eq!(config.gateway.max_concurrent_sessions, Some(1));
        assert!(
            rows.iter()
                .any(|r| r.key == "gateway.max_concurrent_sessions")
        );
    }

    /// A zero-length large span is a busy loop against a dead provider, which
    /// is strictly worse than the hang the policy exists to remove.
    #[test]
    fn a_zero_large_span_is_healed_to_one_second() {
        let mut config = crate::config::AppConfig::default();
        config.agent.retry.large_span_secs = 0;
        let rows = heal_config(&mut config);
        assert_eq!(config.agent.retry.large_span_secs, 1);
        assert!(
            rows.iter().any(|r| r.key == "agent.retry.large_span_secs"),
            "the busy-loop clamp must be named: {rows:?}"
        );
    }

    /// `max_span_secs = 0` is the documented INFINITE, not a defect: healing
    /// it to a number would silently give the operator a ceiling they never
    /// set and stop a retry they expected to run forever.
    #[test]
    fn an_infinite_ceiling_is_left_alone() {
        let mut config = crate::config::AppConfig::default();
        config.agent.retry.max_span_secs = 0;
        let rows = heal_config(&mut config);
        assert_eq!(config.agent.retry.max_span_secs, 0);
        assert!(
            !rows
                .iter()
                .any(|r| r.key.starts_with("agent.retry.max_span_secs")),
            "an infinite ceiling must not be healed: {rows:?}"
        );
    }

    /// A ceiling shorter than the span it bounds stops the ladder before its
    /// first large-span retry ever fires — a config that silently does
    /// nothing, which is the one shape an operator cannot diagnose.
    #[test]
    fn a_ceiling_below_the_span_is_raised_to_one_span() {
        let mut config = crate::config::AppConfig::default();
        config.agent.retry.large_span_secs = 300;
        config.agent.retry.max_span_secs = 60;
        let rows = heal_config(&mut config);
        assert_eq!(config.agent.retry.max_span_secs, 300);
        assert!(
            rows.iter()
                .any(|r| r.key == "agent.retry.max_span_secs" && r.old == "60" && r.new == "300"),
            "the conflict row must name both values: {rows:?}"
        );
    }

    /// `small_span_retries = 0` is legal (no small span: every retry waits the
    /// large span). Healing it would delete a deliberate choice.
    #[test]
    fn a_zero_small_span_is_left_alone() {
        let mut config = crate::config::AppConfig::default();
        config.agent.retry.small_span_retries = 0;
        let rows = heal_config(&mut config);
        assert_eq!(config.agent.retry.small_span_retries, 0);
        assert!(
            !rows
                .iter()
                .any(|r| r.key.starts_with("agent.retry.small_span_retries")),
            "a zero small span must not be healed: {rows:?}"
        );
    }

    /// The heal is idempotent over the retry block too: the second pass moves
    /// nothing (the contract's own rule 1, applied to the new keys).
    #[test]
    fn the_retry_block_heals_idempotently() {
        let mut config = crate::config::AppConfig::default();
        config.agent.retry.large_span_secs = 0;
        let _ = heal_config(&mut config);
        let second = heal_config(&mut config);
        assert!(
            !second.iter().any(|r| r.key.starts_with("agent.retry")),
            "second heal moved a retry row: {second:?}"
        );
    }
}

// ── Persist once ───────────────────────────────────────────────────

/// Write a healed config back to the file it came from, ONCE, atomically.
///
/// The plan's rule: a heal that is not persisted is a heal the next run has
/// to make again, forever, with the operator's file never actually fixed.
///
/// What makes this safe:
///
/// - **Serialized as the struct, not patched from a diff.** The healed
///   `AppConfig` is the authority, so a heal that spans two sections lands in
///   one write and the file cannot end up half-healed.
/// - **Through the same writer every other config write uses.** The document
///   is validated by the real loader BEFORE the rename, and the mode the
///   operator's file had (0600 where secrets live) is carried across.
/// - **Only when a row exists.** `heal_config` returning empty is the
///   idempotence proof; this function is never reached in that case, and a
///   second load therefore writes nothing.
///
/// A failure here is a WARNING, not a load error: the process already holds a
/// healed config in memory and refusing to start because the file could not
/// be rewritten would turn a successful recovery into an outage.
pub fn persist_healed(
    source: &Option<std::path::PathBuf>,
    config: &crate::config::AppConfig,
    healed: &[Healed],
) {
    let Some(path) = source.as_deref() else {
        // No file was loaded (built-in defaults). There is nothing to
        // persist INTO, and creating one would make a read-only load write.
        return;
    };
    if !path.exists() {
        return;
    }

    // The healed value is the whole config, so the write is the struct's own
    // serialization. Comments the operator wrote are NOT preserved by this
    // path — that is the known cost of a struct-level rewrite, and it is why
    // the surgical writer (`persist_config_key_at`) remains the path for
    // every deliberate `config set`. A heal touches only the keys it names.
    let Ok(rendered) = toml::to_string_pretty(config) else {
        tracing::warn!(
            path = %path.display(),
            "the healed config could not be rendered to TOML — the file is unchanged"
        );
        return;
    };

    let tmp = path.with_extension("toml.healed.tmp");
    if let Err(e) = crate::fs_secrets::write_secret_file(&tmp, rendered.as_bytes()) {
        eprintln!(
            "warning: the healed config could not be staged at {}: {e} — the file is unchanged",
            tmp.display()
        );
        return;
    }
    // Carry the original mode across, exactly as the surgical writer does.
    if let Ok(mode) = std::fs::metadata(path).map(|m| m.permissions())
        && let Err(e) = std::fs::set_permissions(&tmp, mode)
    {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "the config file's mode could not be carried onto the healed copy"
        );
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        eprintln!(
            "warning: the healed config could not replace {}: {e} — the file is unchanged",
            path.display()
        );
        return;
    }
    tracing::info!(
        path = %path.display(),
        rows = healed.len(),
        "config healed and persisted"
    );
}

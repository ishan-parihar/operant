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

/// The parked keys recorded for a config file, as one readable string.
///
/// `None` when no sidecar exists (nothing was ever parked). A read or parse
/// error degrades to `None` too: this exists so a test or an operator can
/// CONFIRM what was dropped, and a missing sidecar is the honest answer when
/// the file cannot be read, never a reason to fail the caller.
pub fn read_parked(sidecar: &Path) -> Option<String> {
    std::fs::read_to_string(sidecar).ok()
}

/// The parked keys as a TOML VALUE, for the downgrade merge.
///
/// Separate from [`read_parked`] because [`merge_parked`] needs the value, not
/// its text: rendering the text and re-parsing it would be a second parser that
/// can disagree with the first. `None` for the same three reasons the text
/// reader returns `None` — absent, unreadable, or not valid TOML.
pub fn read_parked_value(sidecar: &Path) -> Option<toml::Value> {
    let text = std::fs::read_to_string(sidecar).ok()?;
    toml::from_str(&text).ok()
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

    // The operator's own file WINS every collision. The sidecar exists to
    // restore keys the current file does not carry (the downgrade case), not
    // to overrule it. Nothing in production ever DELETES a sidecar -- only
    // tests do -- so it is merged on every load forever; with `insert` it
    // silently replaced a value the operator had since edited, and the older
    // parked value won. MEASURED before this guard: raw says
    // `model = "operator-current"`, sidecar says `sidecar-stale`, merged
    // result was `sidecar-stale`.
    //
    // Reachable concretely: build A does not know `[agent] approval_mode` and
    // parks it; the operator upgrades to a build that does know it and sets
    // `approval_mode = "off"` in their own file; without this guard the value
    // parked by the OLDER build is what takes effect, so the operator's edit
    // does nothing and nothing reports it.
    for (dotted, value) in parked {
        // A bare TOP-LEVEL key is wrapped by `split_unknown` as
        // `__unknown_top_level__.<name>`, because an unwrapped bare key in a
        // sidecar would collide with a SECTION of the same name. Split on the
        // FIRST dot only: the remainder is the key, which may itself contain
        // dots.
        //
        // This arm used to be unreachable. The wrapped key has TWO segments, so
        // it fell into the `[section, leaf]` arm below and was restored as a
        // nested `[__unknown_top_level__]` table instead of as the key it was
        // -- the writer wrapped, the reader did not unwrap, and the downgrade
        // restore silently produced a table no config reader looks at.
        if let Some(rest) = dotted.strip_prefix("__unknown_top_level__.") {
            root.entry(rest.to_string())
                .or_insert_with(|| value.clone());
            continue;
        }
        let segments: Vec<&str> = dotted.split('.').collect();
        match segments.as_slice() {
            [leaf] => {
                // A bare top-level key that was NOT wrapped. Kept for a sidecar
                // written by a build without the wrapper.
                root.entry(leaf.to_string())
                    .or_insert_with(|| value.clone());
            }
            [section, leaf] => {
                let entry = root
                    .entry(section.to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let Some(table) = entry.as_table_mut() {
                    table
                        .entry(leaf.to_string())
                        .or_insert_with(|| value.clone());
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
    heal_proxy(&mut config.proxy, &mut out);
    heal_observability(&mut config.observability, &mut out);
    heal_logging(&mut config.logging, &mut out);
    heal_kernel(&mut config.tools.kernel, &mut out);
    heal_bylaws(&mut config.bylaws, &mut out);

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

/// `[proxy]` — the outbound proxy's shape.
///
/// The values themselves are validated at startup by
/// `apply_startup_proxy` (`ProxyConfig::validate`), which REJECTS a malformed
/// URL rather than healing it, because a silently rewritten proxy URL would
/// send traffic somewhere the operator did not ask for. What heals here is the
/// one combination that is not a bad value but a bad PAIR: `enabled` with no
/// URL at all, and `scope = "services"` with no selector. Both make the proxy
/// do nothing while reading as configured, so the operator sees "proxy on" and
/// gets direct traffic.
fn heal_proxy(proxy: &mut operant_config::schema::ProxyConfig, out: &mut Vec<Healed>) {
    // Every scheme field is a URL the operator typed. A malformed one is
    // DROPPED, not rewritten: `validate()` rejects it at startup, but the heal
    // pass runs first, so a bad URL would otherwise survive into the validated
    // config and refuse to load. Dropping it degrades to "that scheme uses the
    // next fallback", which is the safe direction — the alternative (guessing
    // the intended host) would send traffic somewhere unasked.
    for field in [
        ("proxy.http_proxy", &mut proxy.http_proxy),
        ("proxy.https_proxy", &mut proxy.https_proxy),
        ("proxy.all_proxy", &mut proxy.all_proxy),
    ] {
        let keep = field.1.as_deref().and_then(|raw| {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        });
        if keep.is_none() && field.1.is_some() {
            out.push(Healed::missing(
                field.0,
                "a proxy URL",
                "a blank proxy URL was dropped, so that scheme falls through to the next one",
            ));
        }
        *field.1 = keep;
    }
    // A `no_proxy` entry that names nothing is the same shape as the other two:
    // the operator asked to bypass a host and the bypass silently matches no
    // one, so traffic they expected to go direct goes through the proxy.
    // An empty LIST is meaningful ("bypass nothing") and stays; a list whose
    // entries are all blank strings is a typo.
    if proxy.no_proxy.iter().any(|h| h.trim().is_empty()) {
        out.push(Healed::missing(
            "proxy.no_proxy",
            "a host name",
            "no_proxy holds an empty entry, so that bypass matches no host",
        ));
        proxy.no_proxy.retain(|h| !h.trim().is_empty());
    }
    // A `services` entry that names nothing is the same shape as a blank
    // `no_proxy` host: the selector list reads as configured and matches no
    // one. The empty LIST stays (it means "no selectors", which pairs with a
    // non-`services` scope); blank ENTRIES inside it are typos.
    if proxy
        .services
        .iter()
        .any(|selector| selector.trim().is_empty())
    {
        out.push(Healed::missing(
            "proxy.services",
            "a service key",
            "services holds a blank entry, so that selector matches no service",
        ));
        proxy.services.retain(|s| !s.trim().is_empty());
    }
    if proxy.enabled && !proxy.has_any_proxy_url() {
        out.push(Healed::missing(
            "proxy.enabled",
            "false",
            "proxy is enabled but no proxy URL is set, so every request would go direct",
        ));
        proxy.enabled = false;
    }
    if proxy.enabled
        && proxy.scope == operant_config::schema::ProxyScope::Services
        && proxy.normalized_services().is_empty()
    {
        out.push(Healed::missing(
            "proxy.scope",
            "\"zeroclaw\"",
            "proxy scope is 'services' with no selector, so the proxy would match nothing",
        ));
        proxy.scope = operant_config::schema::ProxyScope::default();
    }
}

/// `[bylaws]` — the org-wide default budget's three STRING knobs.
///
/// The reader (`cronjobs/scheduler.rs`) tests `basis == "usd"` and
/// `mode != "hard"`, so an unknown `basis` silently counts TOKENS and an
/// unknown `mode` silently behaves as `soft`. A typo therefore does not fail;
/// it quietly changes what the cap means, which is the worst of the three
/// outcomes (a hard cap becomes advisory, or a USD cap becomes a token cap).
fn heal_bylaws(bylaws: &mut crate::config::BylawsSettings, out: &mut Vec<Healed>) {
    let budget = &mut bylaws.budget;
    let basis = budget.basis.trim().to_ascii_lowercase();
    if !matches!(basis.as_str(), "tokens" | "usd") {
        out.push(Healed::missing(
            "bylaws.budget.basis",
            "tokens | usd",
            "basis is not 'tokens' or 'usd', so the cap would silently count tokens",
        ));
        budget.basis = "tokens".to_string();
    } else if basis != budget.basis {
        budget.basis = basis;
    }
    let mode = budget.mode.trim().to_ascii_lowercase();
    if !matches!(mode.as_str(), "soft" | "hard") {
        out.push(Healed::missing(
            "bylaws.budget.mode",
            "soft | hard",
            "mode is not 'soft' or 'hard', so the cap would silently behave as soft (warn, do not refuse)",
        ));
        budget.mode = "soft".to_string();
    } else if mode != budget.mode {
        budget.mode = mode;
    }
    let window = budget.window.trim().to_ascii_lowercase();
    if !matches!(window.as_str(), "daily" | "weekly" | "monthly") {
        out.push(Healed::missing(
            "bylaws.budget.window",
            "daily | weekly | monthly",
            "window is not daily/weekly/monthly, so the cap would roll on an unknown schedule",
        ));
        budget.window = "daily".to_string();
    } else if window != budget.window {
        budget.window = window;
    }
    // A negative cap is not "ungoverned" (that is 0) and not a real cap; the
    // reader subtracts from it, so it would refuse every run from the first.
    if budget.cap < 0.0 {
        out.push(Healed::missing(
            "bylaws.budget.cap",
            "0",
            "cap is negative, so the seat would be over budget before it starts (0 = ungoverned)",
        ));
        budget.cap = 0.0;
    }
}

/// `[tools.kernel]` — the sidecar's bridge and pyskill knobs.
///
/// The two enums in this section (`tools.kernel.pyskill`,
/// `tools.kernel.tool_bridge`) are serde-parsed, so a bad value fails the load
/// and needs no heal row. What heals here are the NUMERIC caps and the
/// "enabled with an empty allowlist" pair: `tool_bridge` is deny-by-default, so
/// `enabled = true` with no entries reads as on and permits nothing, which is the
/// same silent no-op shape the proxy section had.
fn heal_kernel(kernel: &mut crate::config::KernelSettings, out: &mut Vec<Healed>) {
    let bridge = &mut kernel.tool_bridge;
    if bridge.enabled && bridge.allowlist.is_empty() {
        out.push(Healed::missing(
            "tools.kernel.tool_bridge.allowlist",
            "a tool name",
            "tool_bridge is enabled with an empty allowlist, so no bridged call is permitted",
        ));
        bridge.enabled = false;
    }
    // A cap of zero is a valid "no bridged calls" but only when the bridge is
    // off; zero AND enabled is the contradiction, and clamping to 1 keeps the
    // switch meaningful.
    if bridge.enabled && bridge.max_calls_per_exec == 0 {
        out.push(Healed::missing(
            "tools.kernel.tool_bridge.max_calls_per_exec",
            "1",
            "max_calls_per_exec is 0 while the bridge is enabled, so every cell would fail",
        ));
        bridge.max_calls_per_exec = 1;
    }
    if bridge.per_call_timeout_secs == 0 {
        out.push(Healed::missing(
            "tools.kernel.tool_bridge.per_call_timeout_secs",
            "1",
            "per_call_timeout_secs is 0, so every bridged call would time out instantly",
        ));
        bridge.per_call_timeout_secs = 1;
    }
    // Blank import prefixes name no module; the list is an allowlist, so a
    // blank entry is a typo that silently matches nothing.
    if kernel
        .pyskill
        .allowed_imports
        .iter()
        .any(|prefix| prefix.trim().is_empty())
    {
        out.push(Healed::missing(
            "tools.kernel.pyskill.allowed_imports",
            "an import prefix",
            "pyskill.allowed_imports holds a blank entry, so that prefix matches no module",
        ));
        kernel
            .pyskill
            .allowed_imports
            .retain(|p| !p.trim().is_empty());
    }
}

/// `[logging]` — the level name.
///
/// The level is parsed case-insensitively by the logger, but an unknown name
/// matches no directive, so the process logs NOTHING while looking configured
/// — the same shape as a typo'd observer backend.
fn heal_logging(log: &mut crate::config::LoggingSettings, out: &mut Vec<Healed>) {
    const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];
    let level = log.level.trim().to_ascii_lowercase();
    if !LEVELS.contains(&level.as_str()) {
        out.push(Healed::missing(
            "logging.level",
            LEVELS.join(" | "),
            "logging level is not a level name, so the logger would emit nothing",
        ));
        log.level = "info".to_string();
    } else if level != log.level {
        log.level = level;
    }
}

/// `[observability]` — the backend enum and the trace mode enum.
///
/// RT-274 closed by documenting that six of these seven keys are ACCEPTED BUT
/// NOT APPLIED: only `backend` is read (by the observer factory), and a set
/// value on any other is reported by `unused_setting_warnings`. So the heal
/// has one job here — keep the two ENUM-shaped strings inside the value set
/// their doc comment promises, because a typo'd backend name silently selects
/// no observer at all, which is the "observability is on and nothing is
/// recorded" failure.
fn heal_observability(
    obs: &mut operant_config::schema::ObservabilityConfig,
    out: &mut Vec<Healed>,
) {
    const BACKENDS: &[&str] = &["none", "log", "verbose", "prometheus", "otel", "console"];
    let backend = obs.backend.trim().to_ascii_lowercase();
    if !BACKENDS.contains(&backend.as_str()) {
        out.push(Healed::missing(
            "observability.backend",
            BACKENDS.join(" | "),
            "backend is not one of the observer backends, so no observer would be mounted",
        ));
        obs.backend = "none".to_string();
    } else if backend != obs.backend {
        obs.backend = backend;
    }

    const TRACE_MODES: &[&str] = &["none", "off", "rolling", "full"];
    let mode = obs.runtime_trace_mode.trim().to_ascii_lowercase();
    if !TRACE_MODES.contains(&mode.as_str()) {
        out.push(Healed::missing(
            "observability.runtime_trace_mode",
            TRACE_MODES.join(" | "),
            "runtime_trace_mode is not a trace mode, so the runtime trace would not record",
        ));
        obs.runtime_trace_mode = "off".to_string();
    } else if mode != obs.runtime_trace_mode {
        obs.runtime_trace_mode = mode;
    }
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

    /// A typo'd observer backend silently mounts NO observer, which is the
    /// "observability is on and nothing is recorded" failure.
    #[test]
    fn an_unknown_observer_backend_falls_back_to_none() {
        let mut config = crate::config::AppConfig::default();
        config.observability.backend = "opente-lemetry".to_string();
        let rows = heal_config(&mut config);
        assert_eq!(config.observability.backend, "none");
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"observability.backend"), "{keys:?}");
    }

    /// A real backend name is kept, and a case variant is normalized rather
    /// than rejected — the operator's intent is legible.
    #[test]
    fn a_known_observer_backend_is_kept_and_normalized() {
        let mut config = crate::config::AppConfig::default();
        config.observability.backend = " OTEL ".to_string();
        let rows = heal_config(&mut config);
        assert_eq!(config.observability.backend, "otel");
        assert!(
            !rows.iter().any(|r| r.key == "observability.backend"),
            "a normalized value must not be reported as missing"
        );
    }

    /// `runtime_trace_mode` has the same shape: a typo would record nothing.
    #[test]
    fn an_unknown_trace_mode_falls_back_to_off() {
        let mut config = crate::config::AppConfig::default();
        config.observability.runtime_trace_mode = "rollng".to_string();
        let rows = heal_config(&mut config);
        assert_eq!(config.observability.runtime_trace_mode, "off");
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(
            keys.contains(&"observability.runtime_trace_mode"),
            "{keys:?}"
        );
    }

    /// `[bylaws.budget]`'s three STRING knobs are read with `==` and `!=`, so
    /// an unknown value silently flips what the cap means rather than failing.
    #[test]
    fn an_unknown_budget_basis_becomes_tokens() {
        let mut config = crate::config::AppConfig::default();
        config.bylaws.budget.basis = "dollars".to_string();
        let rows = heal_config(&mut config);
        assert_eq!(config.bylaws.budget.basis, "tokens");
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"bylaws.budget.basis"), "{keys:?}");
    }

    /// A typo'd `mode` would silently behave as `soft` — a hard cap becoming
    /// advisory is the dangerous direction, so the heal must name it.
    #[test]
    fn an_unknown_budget_mode_becomes_soft_and_is_named() {
        let mut config = crate::config::AppConfig::default();
        config.bylaws.budget.mode = "stricter".to_string();
        let rows = heal_config(&mut config);
        assert_eq!(config.bylaws.budget.mode, "soft");
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"bylaws.budget.mode"), "{keys:?}");
    }

    /// A negative cap is not "ungoverned" (that is 0) and not a real cap.
    #[test]
    fn a_negative_budget_cap_becomes_ungoverned() {
        let mut config = crate::config::AppConfig::default();
        config.bylaws.budget.cap = -5.0;
        let rows = heal_config(&mut config);
        assert_eq!(config.bylaws.budget.cap, 0.0);
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"bylaws.budget.cap"), "{keys:?}");
    }

    /// `[tools.kernel.tool_bridge]` is DENY-by-default: enabled with no
    /// allowlist reads as on and permits nothing.
    #[test]
    fn an_enabled_bridge_with_no_allowlist_is_disabled() {
        let mut config = crate::config::AppConfig::default();
        config.tools.kernel.tool_bridge.enabled = true;
        config.tools.kernel.tool_bridge.allowlist.clear();
        let rows = heal_config(&mut config);
        assert!(!config.tools.kernel.tool_bridge.enabled);
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(
            keys.contains(&"tools.kernel.tool_bridge.allowlist"),
            "{keys:?}"
        );
    }

    /// A zero per-call timeout fails every bridged call instantly.
    #[test]
    fn a_zero_bridge_timeout_becomes_one_second() {
        let mut config = crate::config::AppConfig::default();
        config.tools.kernel.tool_bridge.per_call_timeout_secs = 0;
        let rows = heal_config(&mut config);
        assert_eq!(config.tools.kernel.tool_bridge.per_call_timeout_secs, 1);
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(
            keys.contains(&"tools.kernel.tool_bridge.per_call_timeout_secs"),
            "{keys:?}"
        );
    }

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

    /// `[proxy]` enabled with no URL is the shape that reads as "proxied"
    /// and routes direct. The heal turns it OFF rather than inventing a URL,
    /// because guessing a host would send traffic somewhere unasked.
    #[test]
    fn proxy_enabled_without_a_url_is_disabled_and_named() {
        let mut config = crate::config::AppConfig::default();
        config.proxy.enabled = true;
        let rows = heal_config(&mut config);
        assert!(!config.proxy.enabled, "enabled must turn off");
        let row = rows
            .iter()
            .find(|r| r.key == "proxy.enabled")
            .expect("the heal must name the key it changed");
        assert!(!row.why.is_empty() && !row.new.is_empty(), "{row:?}");
    }

    /// A blank URL is a typo, not a setting: the scheme is dropped so it falls
    /// through to the next one instead of refusing to load.
    #[test]
    fn blank_proxy_urls_are_dropped_and_named() {
        let mut config = crate::config::AppConfig::default();
        config.proxy.http_proxy = Some("   ".to_string());
        config.proxy.https_proxy = Some("http://127.0.0.1:7890".to_string());
        let rows = heal_config(&mut config);
        assert_eq!(config.proxy.http_proxy, None, "the blank URL must go");
        assert_eq!(
            config.proxy.https_proxy.as_deref(),
            Some("http://127.0.0.1:7890"),
            "a real URL must survive"
        );
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"proxy.http_proxy"), "{keys:?}");
    }

    /// `scope = "services"` with no selector matches nothing, so the scope
    /// falls back to the default rather than silently proxying nothing.
    #[test]
    fn proxy_services_scope_without_selectors_falls_back() {
        let mut config = crate::config::AppConfig::default();
        config.proxy.enabled = true;
        config.proxy.all_proxy = Some("http://127.0.0.1:7890".to_string());
        config.proxy.scope = operant_config::schema::ProxyScope::Services;
        let rows = heal_config(&mut config);
        assert_eq!(
            config.proxy.scope,
            operant_config::schema::ProxyScope::default(),
            "an empty services scope must fall back"
        );
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"proxy.scope"), "{keys:?}");
    }

    /// Blank entries in `no_proxy` / `services` match no host and no service.
    #[test]
    fn blank_proxy_list_entries_are_dropped() {
        let mut config = crate::config::AppConfig::default();
        config.proxy.no_proxy = vec!["localhost".to_string(), "  ".to_string()];
        config.proxy.services = vec!["openai".to_string(), String::new()];
        let rows = heal_config(&mut config);
        assert_eq!(config.proxy.no_proxy, vec!["localhost".to_string()]);
        assert_eq!(config.proxy.services, vec!["openai".to_string()]);
        let keys: Vec<&str> = rows.iter().map(|r| r.key).collect();
        assert!(keys.contains(&"proxy.no_proxy"), "{keys:?}");
        assert!(keys.contains(&"proxy.services"), "{keys:?}");
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

#[cfg(test)]
mod merge_parked_tests {
    use super::*;

    /// The operator's own file wins a collision with a stale sidecar.
    ///
    /// Nothing in production ever DELETES a sidecar (only tests do), so it is
    /// merged on every load forever. With `insert` the older parked value
    /// silently replaced whatever the operator had since written, and nothing
    /// reported it. MEASURED before the fix: raw said `model = "operator-current"`,
    /// the sidecar said `sidecar-stale`, and the merge produced `sidecar-stale`.
    #[test]
    fn the_operators_own_value_wins_over_a_stale_sidecar() {
        let raw = "[agent]\nmodel = \"operator-current\"\n";
        let mut parked = toml::value::Table::new();
        parked.insert(
            "agent.model".to_string(),
            toml::Value::String("sidecar-stale".into()),
        );
        let merged = merge_parked(raw, &toml::Value::Table(parked)).expect("merge renders");
        let v: toml::Value = toml::from_str(&merged).expect("merged is valid toml");
        assert_eq!(
            v.get("agent")
                .and_then(|a| a.get("model"))
                .and_then(|m| m.as_str()),
            Some("operator-current"),
            "the operator's edited value must survive; a stale sidecar must not overrule it"
        );
    }

    /// The downgrade case must still work: a key the operator's file does NOT
    /// carry is restored from the sidecar. Without this the fix would be
    /// "ignore the sidecar", which re-breaks iter-2364.
    #[test]
    fn a_key_absent_from_the_operator_file_is_still_restored() {
        let raw = "[agent]\nmodel = \"operator-current\"\n";
        let mut parked = toml::value::Table::new();
        parked.insert("agent.max_iterations".to_string(), toml::Value::Integer(42));
        let merged = merge_parked(raw, &toml::Value::Table(parked)).expect("merge renders");
        let v: toml::Value = toml::from_str(&merged).expect("merged is valid toml");
        let agent = v.get("agent").expect("agent section");
        assert_eq!(
            agent.get("model").and_then(|m| m.as_str()),
            Some("operator-current"),
            "the operator's own key is untouched"
        );
        assert_eq!(
            agent.get("max_iterations").and_then(|m| m.as_integer()),
            Some(42),
            "a key the operator's file omits must be restored from the sidecar --              that is the whole point of the downgrade path (iter-2364)"
        );
    }

    /// A parked BARE TOP-LEVEL key is restored as that key, not as a nested
    /// table under a `__unknown_top_level__` section.
    ///
    /// Two defects meet here. `split_unknown` WRAPS a bare key as
    /// `__unknown_top_level__.<name>` so it cannot collide with a section of
    /// the same name; `merge_parked` is the reader that must unwrap it. It did
    /// not -- the wrapped key has two segments, so it fell into the
    /// `[section, leaf]` arm and came back as `[__unknown_top_level__]`, a
    /// table no config reader consults. The writer wrapped and the reader did
    /// not unwrap, so the downgrade restore lost the value in a place nothing
    /// reads.
    ///
    /// This asserts the RESTORE, which is the behaviour that was broken; the
    /// first draft of this test asserted only that the operator's value
    /// survived and passed under BOTH implementations, so it proved nothing.
    #[test]
    fn a_parked_bare_top_level_key_is_restored_as_that_key() {
        // The operator's file does NOT carry the key (that is the downgrade
        // case), so the sidecar's wrapped form must put it back at the top.
        let raw = "[agent]\nmodel = \"operator-current\"\n";
        let mut parked = toml::value::Table::new();
        parked.insert(
            "__unknown_top_level__.theme".to_string(),
            toml::Value::String("sidecar-dark".into()),
        );
        let merged = merge_parked(raw, &toml::Value::Table(parked)).expect("merge renders");
        let v: toml::Value = toml::from_str(&merged).expect("merged is valid toml");
        assert_eq!(
            v.get("theme").and_then(|t| t.as_str()),
            Some("sidecar-dark"),
            "a bare parked key must come back as `theme`, not as a nested \
             [__unknown_top_level__] table nothing reads. Got:\n{merged}"
        );
        assert!(
            v.get("__unknown_top_level__").is_none(),
            "the wrapper must be unwrapped, not emitted as a section: {merged}"
        );
    }

    /// The operator's own BARE top-level value still wins over the wrapped
    /// parked copy, so the precedence rule holds on this arm too.
    #[test]
    fn the_operators_bare_top_level_value_wins_over_a_stale_sidecar() {
        let raw = "theme = \"operator-dark\"\n";
        let mut parked = toml::value::Table::new();
        parked.insert(
            "__unknown_top_level__.theme".to_string(),
            toml::Value::String("sidecar-light".into()),
        );
        let merged = merge_parked(raw, &toml::Value::Table(parked)).expect("merge renders");
        let v: toml::Value = toml::from_str(&merged).expect("merged is valid toml");
        assert_eq!(
            v.get("theme").and_then(|t| t.as_str()),
            Some("operator-dark"),
            "the operator's edited value must survive on this arm too"
        );
        assert!(
            v.get("__unknown_top_level__").is_none(),
            "no wrapper section may appear: {merged}"
        );
    }
}

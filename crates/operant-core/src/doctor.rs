//! ONE doctor engine — `docs/ORGANISM-ARCHITECTURE.md` §6 F2.
//!
//! History: two divergent engines existed. `operant-runtime::doctor` ran on
//! the retired config aggregate and served only the gateway's
//! `GET /api/doctor`; `operant-cli::cmd_doctor` ran on [`AppConfig`] and
//! served `operant doctor`. Same intent, duplicated logic, drifting check
//! lists. This module is the surviving engine: the checks run on
//! [`AppConfig`] (the config type the bylaws and gateway read),
//! `operant doctor` renders them via [`render_text`] alongside its
//! install-level sections, and the gateway serializes the same
//! [`diagnose`] results at its boundary.
//!
//! Runtime-doctor checks that read fields only the retired aggregate carried
//! are named gaps, not fakes: channels, delegate agents, `gateway.port`,
//! `memory.embedding_model`, `config_path` (config-file presence), the
//! runtime `workspace_dir` (SOUL.md/AGENTS.md presence there), and daemon
//! heartbeat state (the `operant-runtime` daemon supervisor has no callers,
//! so `daemon_state.json` is never written anywhere). See
//! `docs/CHANGELOG.md` `[Unreleased]`.

use serde::Serialize;
use std::io::Write;
use std::path::Path;

use crate::config::AppConfig;
use crate::platform::operant_home;

const COMMAND_VERSION_PREVIEW_CHARS: usize = 60;
/// Below this many MB free at the data root, doctor warns.
const DISK_WARN_MB: u64 = 100;

// ── Result model ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Warn,
    Error,
}

/// One engine finding. The gateway serializes this verbatim into
/// `GET /api/doctor`; [`render_text`] prints it for `operant doctor`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

impl CheckResult {
    fn ok(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Ok,
            category,
            message: msg.into(),
        }
    }
    fn warn(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warn,
            category,
            message: msg.into(),
        }
    }
    fn error(category: &'static str, msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            category,
            message: msg.into(),
        }
    }
}

// ── Entry points ───────────────────────────────────────────────────

/// Run every engine check and return the structured results.
///
/// This is THE check list: both `operant doctor` (text render) and the
/// gateway API (JSON) consume exactly this output — see
/// `engine_check_list_is_the_shared_drift_pin` for the pin.
pub fn diagnose(config: &AppConfig) -> Vec<CheckResult> {
    let mut items = Vec::new();
    check_config_semantics(config, &mut items);
    check_config_resilience(config, &mut items);
    check_memory_provider(config, &mut items);
    check_skill_roots(config, &mut items);
    check_data_root(&mut items);
    check_environment(&mut items);
    check_cli_tools(config, &mut items);
    items
}

// ── skill roots ────────────────────────────────────────────────────

/// Where the skills actually come from, and what was refused.
///
/// The `$` popup, `/skill <Tab>`, the `/skills` panel, `skills_list`,
/// `skill_view`, the gateway picker and the agent's `SkillManager` all read
/// the same resolved list now (iter-2260), so a skill that is missing is
/// missing from ONE place — and the reason it is missing is a configuration
/// fact the operator can act on. A root that is missing, a file rather than
/// a directory, denied by `@folder:`'s fail-closed list, or holding a dead
/// symlink was previously dropped in silence.
///
/// The advertised count is reported beside the loaded count because the two
/// are different measurements and only the difference is actionable: a skill
/// whose platform or prerequisite env var is absent is listed and then
/// dropped by `is_available`, so `skills_list` and the in-loop `skills` tool
/// disagree about how many exist.
fn check_skill_roots(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "skills";
    let roots = crate::skills::SkillRoots::resolve(&config.skills, &[], None, None);

    if roots.is_empty() {
        let reason = roots
            .issues
            .first()
            .map(|i| format!(" (first refusal: {}", i.reason))
            .unwrap_or_default();
        items.push(CheckResult::error(
            cat,
            format!(
                "no skill root resolves — the `$` popup, `/skill <Tab>`, `/skills`, \
                 skills_list/skill_view and the gateway picker are all empty{reason})"
            ),
        ));
        return;
    }

    let mut manager = crate::skills::SkillManager::from_roots(&roots);
    let (loaded, available) = match manager.load_all() {
        Ok(_) => {
            let listed = manager.list().len();
            let available = {
                let mut n = 0usize;
                for (name, _) in manager.list() {
                    if manager.get(&name).is_some_and(|s| manager.is_available(s)) {
                        n += 1;
                    }
                }
                n
            };
            (listed, available)
        }
        Err(e) => {
            items.push(CheckResult::error(
                cat,
                format!("skill root scan failed: {e}"),
            ));
            return;
        }
    };

    // TWO sources, not one: `roots.issues` holds the roots the RESOLVER
    // refused (missing, not a directory, denied), while the WALK's findings --
    // dead symlinks inside a root that did resolve -- land in the MANAGER's
    // list. Reading only the first silently drops the second, which is the
    // exact defect this row exists to report: a dead symlink in the operator's
    // own pool was counted nowhere and printed never.
    let mut refusals: Vec<&crate::skills::RootIssue> = roots.issues.iter().collect();
    refusals.extend(manager.issues().iter());
    let truncated = manager.is_truncated();

    if loaded == 0 {
        items.push(CheckResult::warn(
            cat,
            format!(
                "{} skill root{} resolve, but none holds a loadable skill ({} refusal{})",
                roots.roots.len(),
                if roots.roots.len() == 1 { "" } else { "s" },
                refusals.len(),
                if refusals.len() == 1 { "" } else { "s" },
            ),
        ));
    } else {
        items.push(CheckResult::ok(
            cat,
            format!(
                "{} skill{} loadable from {} root{}{}{}",
                loaded,
                if loaded == 1 { "" } else { "s" },
                roots.roots.len(),
                if roots.roots.len() == 1 { "" } else { "s" },
                if available == loaded {
                    String::new()
                } else {
                    format!(", {available} usable in this environment")
                },
                if truncated {
                    format!(
                        " (walk truncated at {} entries)",
                        crate::skills::SKILL_ROOT_WALK_LIMIT
                    )
                } else {
                    String::new()
                },
            ),
        ));
    }

    // One row per refused root: the path and the reason, because the two
    // together are the fix ("point extra_roots at the directory you meant").
    for issue in &refusals {
        let detail = match &issue.link_target {
            Some(target) => format!("{} -> {}", issue.entry.display(), target.display()),
            None => issue.entry.display().to_string(),
        };
        items.push(CheckResult::warn(
            cat,
            format!("skill root refused ({}): {detail}", issue.reason),
        ));
    }
}

// ── memory provider pins that leave the memory_* tools dead ─────────

/// `builtin` and `disabled` are the two provider names the loader treats
/// as the legacy no-op (`load_memory_manager`: no MemoryProvider
/// attaches), so memory_store/recall/search answer "No memory backend
/// attached" all turn. Doctor names the pin: the 2026-10-25 tooling
/// audit found an operator running with `provider = "builtin"` for weeks
/// with every memory tool dead and nothing anywhere saying why.
fn check_memory_provider(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "config";
    if !config.memory.enabled {
        items.push(CheckResult::warn(
            cat,
            "[memory] enabled = false — long-term memory is off; memory_store/recall/search \
             are dead. Set enabled = true to use memory-wire",
        ));
        return;
    }
    match config.memory.provider.as_str() {
        "memory-wire" | "" => items.push(CheckResult::ok(
            cat,
            "memory provider memory-wire (in-process engine) is active",
        )),
        "builtin" => items.push(CheckResult::warn(
            cat,
            "[memory] provider = \"builtin\" pins the legacy file-backed no-op: no memory \
             backend attaches, so memory_store/recall/search answer \"No memory backend \
             attached\". Delete the `[memory] provider` line — memory-wire is the default",
        )),
        "disabled" => items.push(CheckResult::warn(
            cat,
            "[memory] provider = \"disabled\" — no memory backend attaches, so \
             memory_store/recall/search are dead. Delete the `[memory] provider` line \
             to enable memory-wire",
        )),
        other => items.push(CheckResult::warn(
            cat,
            format!(
                "[memory] provider \"{other}\" is not a known engine (memory-wire is the \
                 only one): the runtime downgrades it to the builtin no-op and the \
                 memory_* tools stay dead"
            ),
        )),
    }
}

// ── Bylaws / organism health (Wave 5, ORGANISM-ARCHITECTURE §4) ─────

/// Plain-text render used by `operant doctor`'s Diagnostics section.
///
/// Categories group under `[<category>]` headers, matching the shape the
/// retired runtime engine printed, so consumers of the text form see the
/// same structure.
pub fn render_text(results: &[CheckResult]) -> String {
    let mut out = String::new();
    let mut current_cat = "";
    for result in results {
        if result.category != current_cat {
            current_cat = result.category;
            out.push_str(&format!("[{current_cat}]\n"));
        }
        let icon = match result.severity {
            Severity::Ok => "✓",
            Severity::Warn => "⚠",
            Severity::Error => "✗",
        };
        out.push_str(&format!("  {icon} {}\n", result.message));
    }
    out
}

// ── Config semantic validation ────────────────────────────────────

fn check_config_semantics(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "config";
    let providers = &config.providers;

    // The `[providers]` section is optional in the AppConfig world:
    // `client.base_url` drives routing when it is absent. Only validate it
    // when it has content that can silently stop matching.
    let section_has_content = !(providers.models.is_empty()
        && providers.model_routes.is_empty()
        && providers.embedding_routes.is_empty()
        && providers.fallback_chain.is_empty());

    match providers.fallback.as_deref() {
        Some(provider) => {
            if let Some(reason) = provider_validation_error(provider) {
                items.push(CheckResult::error(
                    cat,
                    format!("default provider \"{provider}\" is invalid: {reason}"),
                ));
            } else if let Some(reason) = provider_unreachable_error(provider) {
                // The name is KNOWN, so the name check passes — but the
                // runtime cannot speak its protocol, so every request
                // would fail. Reporting it as "valid" was the misreport.
                items.push(CheckResult::error(
                    cat,
                    format!("default provider \"{provider}\" is not reachable: {reason}"),
                ));
            } else {
                items.push(CheckResult::ok(
                    cat,
                    format!("provider \"{provider}\" is valid"),
                ));
            }

            if let Some(entry) = providers.fallback_provider() {
                // API key presence (ollama is the keyless exception).
                if provider != "ollama" {
                    if entry.api_key.is_some() {
                        items.push(CheckResult::ok(cat, "API key configured"));
                    } else {
                        items.push(CheckResult::warn(
                            cat,
                            "no api_key set (may rely on env vars or provider defaults)",
                        ));
                    }
                }

                match entry.model.as_deref() {
                    Some(model) => {
                        items.push(CheckResult::ok(cat, format!("default model: {model}")));
                    }
                    None => items.push(CheckResult::warn(cat, "no default_model configured")),
                }

                // The provider profile's temperature is ACCEPTED BUT NOT
                // APPLIED (RT-272): nothing reads it, so an out-of-range
                // value cannot change any request. Reporting it as an ERROR
                // made the doctor fail on a knob that does nothing — a
                // phantom failure the operator could not act on. The
                // load-time warning (AppConfig::unused_setting_warnings)
                // names the knob instead.
                let default_temperature = entry.temperature.unwrap_or(0.7);
                if (0.0..=2.0).contains(&default_temperature) {
                    items.push(CheckResult::warn(
                        cat,
                        format!(
                            "temperature {default_temperature:.1} is set but NOT applied — the runtime does not read it"
                        ),
                    ));
                } else {
                    items.push(CheckResult::warn(
                        cat,
                        format!(
                            "temperature {default_temperature:.1} is out of range (expected 0.0–2.0) AND is NOT applied — the runtime does not read it"
                        ),
                    ));
                }
            }
        }
        None => {
            if section_has_content {
                items.push(CheckResult::warn(
                    cat,
                    "no providers.fallback configured — requests without a matching route fail at runtime",
                ));
            }
        }
    }

    // Cross-provider fallback chain (schema `reliability.fallback_providers`
    // parity: the AppConfig world's chain is `[providers] fallback_chain`).
    for fb in &providers.fallback_chain {
        if let Some(reason) = provider_validation_error(&fb.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "fallback provider \"{}\" is invalid: {}",
                    fb.provider, reason
                ),
            ));
        } else if let Some(reason) = provider_unreachable_error(&fb.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "fallback provider \"{}\" is not reachable: {}",
                    fb.provider, reason
                ),
            ));
        }
    }

    // Model routes.
    for route in &providers.model_routes {
        if route.hint.is_empty() {
            items.push(CheckResult::warn(cat, "model route with empty hint"));
        }
        if let Some(reason) = provider_validation_error(&route.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "model route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.is_empty() {
            items.push(CheckResult::warn(
                cat,
                format!("model route \"{}\" has empty model", route.hint),
            ));
        }
    }

    // Embedding routes.
    for route in &providers.embedding_routes {
        if route.hint.trim().is_empty() {
            items.push(CheckResult::warn(cat, "embedding route with empty hint"));
        }
        if let Some(reason) = embedding_provider_validation_error(&route.provider) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "embedding route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.trim().is_empty() {
            items.push(CheckResult::warn(
                cat,
                format!("embedding route \"{}\" has empty model", route.hint),
            ));
        }
        if route.dimensions.is_some_and(|value| value == 0) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "embedding route \"{}\" has invalid dimensions=0",
                    route.hint
                ),
            ));
        }
    }
}

// ── L4: the upgrade-resilience sweep ──────────────────────────────
//
// (plan 2026-11-14, L4) The doctor's answer to "what does this config do on
// an UPGRADE?" Seven named rows, each firing on a real defect and each silent
// on a default config. Every row names the key it is about, because a doctor
// line that says "your config is fine" without saying WHICH config is not a
// diagnostic.
//
// The categories are stable strings, so `operant doctor --json` and any
// dashboard reading the same results can key off them.
fn check_config_resilience(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "config.resilience";

    // 1. `config.unknown-key` — a key this binary does not know, parked in
    //    the sidecar at load. The load succeeded, so this is a WARN: the
    //    operator's file carries keys from a NEWER operant, and either they
    //    upgrade or those settings are not in effect.
    match crate::config::loaded_config_path() {
        Some(path) => {
            let sidecar = crate::config_heal::unknown_sidecar_for(&path);
            if sidecar.exists() {
                let parked = std::fs::read_to_string(&sidecar).unwrap_or_default();
                let count = parked
                    .lines()
                    .filter(|l| {
                        let t = l.trim();
                        !t.is_empty() && !t.starts_with('#') && !t.starts_with('[')
                    })
                    .count();
                items.push(CheckResult::warn(
                    cat,
                    format!(
                        "{} key(s) in {} are unknown to this operant build and were parked in {} — they are NOT in effect. Upgrade operant, or remove them from the config.",
                        count,
                        path.display(),
                        sidecar.display()
                    ),
                ));
            }
        }
        None => items.push(CheckResult::ok(
            cat,
            "no config file is loaded — the built-in defaults are in effect",
        )),
    }

    // 2. `config.deprecated-key` — a key the loader accepts and nothing reads.
    //    (The existing `unused_setting_warnings` runs on the load path; the
    //    doctor names the same class so a `doctor` run reports it too.)
    for line in config.unused_setting_warnings() {
        items.push(CheckResult::warn(cat, line));
    }

    // 3. `config.missing-section` — a section that is new in this version is
    //    absent. Reportable only for the sections whose absence changes
    //    behaviour, so the doctor names the two that do.
    if config.gateway.max_concurrent_sessions.is_none() {
        items.push(CheckResult::warn(
            cat,
            "gateway.max_concurrent_sessions is unset — the gateway accepts unlimited concurrent sessions; set a cap (the default is 32)".to_string(),
        ));
    }
    if config.client.rate_limit.max_retries == 0 {
        items.push(CheckResult::warn(
            cat,
            "client.rate_limit.max_retries = 0 — the transport will not retry a transient network failure at all; the default is 3".to_string(),
        ));
    }

    // The two-span retry policy's own values (`[agent.retry]`). A large span of
    // 1 s is a busy loop against a dead provider (the heal clamps it, but a
    // config that reaches the doctor with it is worth naming); a ceiling below
    // the span stops the ladder before its first large-span retry, which is a
    // config that silently does nothing — the one shape an operator cannot
    // diagnose from the outside.
    if config.agent.retry.large_span_secs <= 1 {
        items.push(CheckResult::warn(
            cat,
            format!(
                "{} is {} — that is a busy loop against a dead provider, not a retry span; the default is 300",
                "agent.retry.large_span_secs",
                config.agent.retry.large_span_secs
            ),
        ));
    }
    // The policy as configured. Reported as its own row because the three
    // knobs above are DEFECTS and this is the shape: an operator reading
    // `[config.resilience]` on a healthy host should see what the retry policy
    // is doing, not only what is wrong with it. `small_span_retries` of 0 is
    // legal and deliberate (every retry waits the large span), so without this
    // row a deliberate config is indistinguishable from an unset one.
    items.push(CheckResult::ok(
        cat,
        format!(
            "{}: {} immediate re-issue(s), then every {}s{}; ceiling {}",
            "agent.retry",
            config.agent.retry.small_span_retries,
            config.agent.retry.large_span_secs,
            if config.agent.retry.announce_large_span {
                " (announced)"
            } else {
                " (unannounced)"
            },
            if config.agent.retry.max_span_secs == 0 {
                "infinite".to_string()
            } else {
                format!("{}s", config.agent.retry.max_span_secs)
            },
        ),
    ));
    if config.agent.retry.max_span_secs > 0
        && config.agent.retry.max_span_secs < config.agent.retry.large_span_secs
    {
        items.push(CheckResult::warn(
            cat,
            format!(
                "{} ({}) is below {} ({}) — the ladder stops before its first large-span retry, so the retry policy is inert",
                "agent.retry.max_span_secs",
                config.agent.retry.max_span_secs,
                "agent.retry.large_span_secs",
                config.agent.retry.large_span_secs
            ),
        ));
    }

    // 4. `config.range` — a value the reader cannot use, clamped by the heal
    //    tables at load. The doctor reports the CLAMPED value so the operator
    //    sees what is actually in effect rather than what they wrote.
    if config.agent.max_iterations == 1 {
        items.push(CheckResult::warn(
            cat,
            "agent.max_iterations is 1 — the turn ends after a single model call, so no tool-using turn can complete; the default is 90".to_string(),
        ));
    }
    if config.agent.tool_timeout_secs == 1 {
        items.push(CheckResult::warn(
            cat,
            "agent.tool_timeout_secs is 1 second — every tool call that takes longer than a moment will be killed; the default is 120".to_string(),
        ));
    }
    if config.client.timeout_secs == 1 {
        items.push(CheckResult::warn(
            cat,
            "client.timeout_secs is 1 second — the provider's response headers will not arrive in time on any real endpoint; the default is 60".to_string(),
        ));
    }

    // 5. `config.two-files` — both `config.toml` and `operant.toml` exist.
    //    The loader already warns on the sibling; the doctor names the pair so
    //    a `doctor` run catches what a headless load printed once.
    if let Some(path) = crate::config::loaded_config_path() {
        for ignored in crate::config::ignored_config_files(&path) {
            items.push(CheckResult::warn(
                cat,
                format!(
                    "{} is not read — the live config is {}. Move its keys into the live file, or delete it.",
                    ignored.display(),
                    path.display()
                ),
            ));
        }
    }

    // 6. `config.unparseable` — the file the loader COULD NOT parse. The load
    //    falls back to defaults, so the process starts with a config the
    //    operator did not write. This is the row that stops a broken file
    //    looking like a working install.
    if let Some(path) = crate::config::loaded_config_path()
        && path.exists()
        && let Err(e) = crate::config::parse_config_file_without_recording(&path)
    {
        items.push(CheckResult::error(
            cat,
            format!(
                "{} does not parse ({e}) — the running config is the built-in defaults, NOT the file. Fix the file, or restore a backup with `operant config backups`.",
                path.display()
            ),
        ));
    }
}

fn provider_validation_error(name: &str) -> Option<String> {
    crate::provider_catalog::known_provider_wire_api(name)
        .is_none()
        .then(|| crate::provider_catalog::unknown_provider_error(name))
}

/// The wire APIs the RUNTIME can actually speak.
///
/// The runtime has exactly two model clients: `AnthropicModelClient`
/// (Anthropic's Messages API) and `OpenAIModelClient` (the chat-
/// completions shape). Every other provider the factory knows is
/// implemented in the provider catalog but is NOT reachable from the run
/// path — the runtime's provider routing sends every non-anthropic
/// provider to the OpenAI-compatible client.
///
/// So a provider whose wire API is neither of these two is configured but
/// unreachable: the doctor used to report it as "valid", and every request
/// would fail against an endpoint that speaks a different protocol.
const RUNTIME_WIRE_APIS: &[&str] = &["chat_completions", "messages"];

/// The reason a provider cannot be served by the runtime, if it cannot.
///
/// `None` means the runtime can serve it.
fn provider_unreachable_error(name: &str) -> Option<String> {
    // (iter-1215) The wire API comes from the static provider catalog —
    // the `operant-providers` factory this used to construct answered
    // exactly this metadata question with 37K lines of provider
    // implementations the runtime never called (RT-153).
    let wire_api = crate::provider_catalog::known_provider_wire_api(name)?;
    if RUNTIME_WIRE_APIS.contains(&wire_api) {
        return None;
    }
    Some(format!(
        "it speaks the \"{wire_api}\" wire API, and the runtime only speaks \
         chat-completions (OpenAI-compatible) and messages (Anthropic) — so \
         every request to it would be sent in the wrong protocol"
    ))
}

fn embedding_provider_validation_error(name: &str) -> Option<String> {
    let normalized = name.trim();
    if normalized.eq_ignore_ascii_case("none") || normalized.eq_ignore_ascii_case("openai") {
        return None;
    }

    let Some(url) = normalized.strip_prefix("custom:") else {
        return Some("supported values: none, openai, custom:<url>".into());
    };

    let url = url.trim();
    if url.is_empty() {
        return Some("custom:<url> requires a non-empty url".into());
    }
    match url::Url::parse(url) {
        Ok(_) => None,
        Err(err) => Some(format!("invalid custom provider URL: {err}")),
    }
}

// ── Data root integrity ────────────────────────────────────────────

/// The runtime engine checked the retired aggregate's `workspace_dir`
/// (`~/.operant/workspace`, bootstrapped only by schema-config loads).
/// The AppConfig world has no workspace concept; its data root —
/// `operant_home()`, which holds state.db, skills, cron — is where the
/// writable/disk checks carry over to.
fn check_data_root(items: &mut Vec<CheckResult>) {
    let cat = "workspace";
    let root = operant_home();

    if !root.exists() {
        items.push(CheckResult::warn(
            cat,
            format!(
                "data root {} not found (will be created on first use)",
                root.display()
            ),
        ));
        return;
    }
    items.push(CheckResult::ok(
        cat,
        format!("data root exists: {}", root.display()),
    ));

    // Writable check.
    let probe = probe_file_path(&root);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(mut probe_file) => {
            let write_result = probe_file.write_all(b"probe");
            drop(probe_file);
            let _ = std::fs::remove_file(&probe);
            match write_result {
                Ok(()) => items.push(CheckResult::ok(cat, "data root is writable")),
                Err(e) => items.push(CheckResult::error(
                    cat,
                    format!("data root write probe failed: {e}"),
                )),
            }
        }
        Err(e) => items.push(CheckResult::error(
            cat,
            format!("data root is not writable: {e}"),
        )),
    }

    // Disk space (best-effort via `df`).
    if let Some(avail_mb) = disk_available_mb(&root) {
        if avail_mb >= DISK_WARN_MB {
            items.push(CheckResult::ok(
                cat,
                format!("disk space: {avail_mb} MB available"),
            ));
        } else {
            items.push(CheckResult::warn(
                cat,
                format!("low disk space: only {avail_mb} MB available"),
            ));
        }
    }
}

fn probe_file_path(base: &Path) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    base.join(format!(
        ".operant_doctor_probe_{}_{}",
        std::process::id(),
        nanos
    ))
}

fn disk_available_mb(path: &Path) -> Option<u64> {
    let output = std::process::Command::new("df")
        .arg("-m")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_df_available_mb(&stdout)
}

fn parse_df_available_mb(stdout: &str) -> Option<u64> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let avail = line.split_whitespace().nth(3)?;
    avail.parse::<u64>().ok()
}

// ── Environment checks ─────────────────────────────────────────────

fn check_environment(items: &mut Vec<CheckResult>) {
    let cat = "environment";

    check_command_available("git", &["--version"], cat, items);

    // Shell — Unix uses $SHELL, Windows uses %ComSpec% (path to cmd.exe).
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("ComSpec").ok().filter(|s| !s.is_empty()));
    match shell {
        Some(s) => items.push(CheckResult::ok(cat, format!("shell: {s}"))),
        None => items.push(CheckResult::warn(
            cat,
            "neither $SHELL nor %ComSpec% is set",
        )),
    }

    // HOME
    if std::env::var("HOME").is_ok() || std::env::var("USERPROFILE").is_ok() {
        items.push(CheckResult::ok(cat, "home directory env set"));
    } else {
        items.push(CheckResult::error(
            cat,
            "neither $HOME nor $USERPROFILE is set",
        ));
    }

    check_command_available("curl", &["--version"], cat, items);
}

// ── CLI tool discovery ─────────────────────────────────────────────

fn check_cli_tools(config: &AppConfig, items: &mut Vec<CheckResult>) {
    let cat = "cli-tools";

    let roster_path = crate::tools::cli_roster::roster_path_from_config(config);
    let discovered = crate::tools::cli_discovery::discover_cli_tools(&roster_path, &[], &[]);

    if discovered.is_empty() {
        items.push(CheckResult::warn(cat, "No CLI tools found in PATH"));
    } else {
        for cli in &discovered {
            let version_info = cli
                .version
                .as_deref()
                .map(|v| truncate_for_display(v, COMMAND_VERSION_PREVIEW_CHARS))
                .unwrap_or_else(|| "unknown version".to_string());
            items.push(CheckResult::ok(
                cat,
                format!("{} ({}) — {}", cli.name, cli.category, version_info),
            ));
        }
        items.push(CheckResult::ok(
            cat,
            format!("{} CLI tools discovered", discovered.len()),
        ));
    }
}

fn check_command_available(
    cmd: &str,
    args: &[&str],
    cat: &'static str,
    items: &mut Vec<CheckResult>,
) {
    match std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout);
            let first_line = ver.lines().next().unwrap_or("").trim();
            let display = truncate_for_display(first_line, COMMAND_VERSION_PREVIEW_CHARS);
            items.push(CheckResult::ok(cat, format!("{cmd}: {display}")));
        }
        Ok(_) => items.push(CheckResult::warn(
            cat,
            format!("{cmd} found but returned non-zero"),
        )),
        Err(_) => items.push(CheckResult::warn(cat, format!("{cmd} not found in PATH"))),
    }
}

fn truncate_for_display(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    /// A provider the runtime CANNOT speak must not be reported as valid.
    ///
    /// Bedrock's name is known to the factory, so the name check passed
    /// and the doctor printed `provider "bedrock" is valid` — but the
    /// runtime's provider routing sends every non-anthropic provider to
    /// the OpenAI-compatible client, and Bedrock speaks AWS's Converse
    /// API. So an operator could select Bedrock in the TUI (it is in the
    /// provider catalogue) and every request would fail against an
    /// endpoint that cannot parse a chat-completions request.
    ///
    /// The check reads the provider's declared wire API, which is why the
    /// three non-chat-completions providers now declare theirs — the
    /// trait's default says "chat_completions", and a provider that does
    /// NOT speak it must say so.
    #[test]
    fn a_provider_the_runtime_cannot_speak_is_not_reported_as_valid() {
        // The name is known — so the NAME check passes.
        assert!(
            provider_validation_error("bedrock").is_none(),
            "bedrock is a known provider name"
        );
        // But the runtime cannot serve it.
        let reason = provider_unreachable_error("bedrock")
            .expect("bedrock speaks a wire API the runtime does not");
        assert!(
            reason.contains("converse"),
            "the reason must name the wire API, got: {reason}"
        );

        // And the providers the runtime CAN serve are still fine.
        assert!(
            provider_unreachable_error("openai").is_none(),
            "openai is chat-completions — the runtime speaks it"
        );
        assert!(
            provider_unreachable_error("anthropic").is_none(),
            "anthropic is messages — the runtime speaks it"
        );
    }

    use super::*;
    use operant_config::schema::{EmbeddingRouteConfig, FallbackProviderConfig, ModelRouteConfig};
    use serial_test::serial;

    #[test]
    fn the_builtin_memory_pin_is_named_by_the_doctor() {
        // The 2026-10-25 tooling audit: an operator ran with
        // provider = "builtin" for weeks, every memory_* tool dead, and
        // no surface said why.
        let mut config = AppConfig::default();
        config.memory.provider = "builtin".to_string();
        config.memory.enabled = true;
        let results = diagnose(&config);
        let hit = results
            .iter()
            .find(|r| r.message.contains("provider = \"builtin\""))
            .expect("the builtin pin must be flagged");
        assert_eq!(hit.severity, Severity::Warn);
        assert!(
            hit.message.contains("Delete the `[memory] provider` line"),
            "the row must carry the fix: {}",
            hit.message
        );
    }

    #[test]
    fn the_disabled_memory_pin_and_unknown_names_are_flagged() {
        let mut config = AppConfig::default();
        config.memory.provider = "disabled".to_string();
        assert!(
            diagnose(&config)
                .iter()
                .any(|r| r.severity == Severity::Warn && r.message.contains("\"disabled\""))
        );

        let mut config = AppConfig::default();
        config.memory.provider = "tdg".to_string();
        assert!(diagnose(&config).iter().any(|r| r.message.contains("tdg")));

        let mut config = AppConfig::default();
        config.memory.enabled = false;
        assert!(
            diagnose(&config)
                .iter()
                .any(|r| r.message.contains("enabled = false"))
        );
    }

    #[test]
    fn the_memory_wire_default_is_reported_ok() {
        let config = AppConfig::default();
        let results = diagnose(&config);
        assert!(
            results.iter().any(|r| r.severity == Severity::Ok
                && r.message
                    .contains("memory-wire (in-process engine) is active")),
            "the default must be an OK row: {results:?}"
        );
    }

    fn fixture_config() -> AppConfig {
        // A config with real `[providers]` content so the config category
        // produces results on any machine.
        let mut config = AppConfig::default();
        if let Ok(home) = std::env::var("HERMES_HOME") {
            config.database_path = std::path::Path::new(&home).join("database.db");
        }
        config.providers.fallback = Some("openrouter".into());
        config.providers.models.insert(
            "openrouter".into(),
            operant_config::schema::ModelProviderConfig {
                api_key: Some("sk-test".into()),
                model: Some("test-model".into()),
                temperature: Some(0.7),
                ..Default::default()
            },
        );
        config
    }

    /// The drift pin (task F2): `operant doctor` renders and the gateway
    /// serializes THE SAME check list — one `diagnose` call. The category
    /// order below is the contract; adding or removing a category is a
    /// deliberate change that must update every consumer.
    #[test]
    fn engine_check_list_is_the_shared_drift_pin() {
        let results = diagnose(&fixture_config());

        let mut categories: Vec<&str> = Vec::new();
        for result in &results {
            if categories.last() != Some(&result.category) {
                categories.push(result.category);
            }
        }
        assert_eq!(
            categories,
            [
                "config",
                "config.resilience",
                "config",
                "skills",
                "workspace",
                "environment",
                "cli-tools",
            ]
        );

        for category in categories {
            assert!(
                results.iter().any(|r| r.category == category),
                "category {category} produced no results"
            );
        }
    }

    /// The skill-roots row names the resolved roots AND what was refused.
    /// A row that said only "N roots" would be the silent drop again: the
    /// operator's actionable fact is WHICH path the config named and why it
    /// did not resolve.
    #[test]
    fn the_skill_roots_row_names_every_refused_root_and_its_reason() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let good = tmp.path().join("pool");
        std::fs::create_dir_all(&good).expect("pool dir");
        let skill = good.join("demo");
        std::fs::create_dir_all(&skill).expect("skill dir");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\ndescription: a demo skill\n---\n\nbody\n",
        )
        .expect("SKILL.md");
        // A dead symlink INSIDE a root that DOES resolve. The resolver never
        // sees it (the root is fine); only the WALK finds it, and it lands on
        // the MANAGER's issue list, not the resolver's. Reading only the
        // resolver's list -- which this row did -- dropped it in silence,
        // which is the exact defect the row exists to report.
        std::os::unix::fs::symlink(tmp.path().join("never-existed"), good.join("DEADLINK"))
            .expect("symlink");

        // One good root, one missing, one that is a file. All three are
        // configuration the operator wrote; only the first resolves.
        let missing = tmp.path().join("not-there");
        let a_file = tmp.path().join("a-file");
        std::fs::write(&a_file, "not a directory").expect("file");
        let mut dead_dir = tmp.path().join("dead");
        std::fs::create_dir_all(&dead_dir).expect("dead dir");
        dead_dir.push("gone");
        std::os::unix::fs::symlink(tmp.path().join("never-existed"), &dead_dir).expect("symlink");

        let mut config = AppConfig::default();
        config.skills.root_dir = good.clone();
        config.skills.extra_roots = vec![missing.clone(), a_file.clone(), dead_dir.clone()];

        let rows: Vec<String> = diagnose(&config)
            .into_iter()
            .filter(|r| r.category == "skills")
            .map(|r| format!("{:?}: {}", r.severity, r.message))
            .collect();

        let joined = rows.join("\n");
        assert!(
            joined.contains(&good.display().to_string()) || joined.contains("loadable"),
            "the resolved root is not named: {joined}"
        );
        assert!(
            joined.contains(&missing.display().to_string()),
            "a configured-but-missing root is dropped in silence: {joined}"
        );
        assert!(
            joined.contains(&a_file.display().to_string()),
            "a root that is a file is dropped in silence: {joined}"
        );
        // The walk's finding, with its target: a dead symlink in a root the
        // operator's own pool named is a defect to REPORT, not to skip.
        assert!(
            joined.contains(&good.join("DEADLINK").display().to_string()),
            "a dead symlink inside a live root is dropped in silence: {joined}"
        );
        assert!(
            joined.contains("dead_symlink"),
            "the reason is not named: {joined}"
        );
        assert!(
            joined.contains("never-existed"),
            "the link target is not named, so the operator cannot fix it: {joined}"
        );
    }

    /// Both consumer surfaces derive from one list: the text render and the
    /// gateway JSON shape carry every result of the same `diagnose` call.
    #[test]
    fn cli_render_and_gateway_serialize_consume_the_same_results() {
        let results = diagnose(&fixture_config());

        let text = render_text(&results);
        for result in &results {
            assert!(
                text.contains(result.message.as_str()),
                "missing: {}",
                result.message
            );
        }
        for category in ["config", "workspace", "environment", "cli-tools"] {
            assert!(text.contains(&format!("[{category}]")));
        }

        let gateway_payload = serde_json::to_value(&results).unwrap_or_default();
        let Some(array) = gateway_payload.as_array() else {
            panic!("gateway payload must be a JSON array");
        };
        assert_eq!(array.len(), results.len());
        for (entry, result) in array.iter().zip(&results) {
            assert!(entry.get("severity").is_some());
            assert!(entry.get("category").is_some());
            assert!(entry.get("message").is_some());
            assert_eq!(
                entry.get("message").and_then(serde_json::Value::as_str),
                Some(result.message.as_str())
            );
        }
    }

    #[test]
    fn provider_validation_checks_custom_url_shape() {
        assert!(provider_validation_error("openrouter").is_none());
        assert!(provider_validation_error("custom:https://example.com").is_none());
        assert!(provider_validation_error("anthropic-custom:https://example.com").is_none());
    }

    /// RT-272: the provider profile's temperature is ACCEPTED BUT NOT
    /// APPLIED, so an out-of-range value cannot break a request — the
    /// doctor must WARN (naming the knob as not applied), not ERROR on a
    /// setting the operator cannot act on.
    #[test]
    fn config_validation_warns_about_an_unapplied_temperature() {
        let mut config = fixture_config();
        if let Some(entry) = config.providers.models.get_mut("openrouter") {
            entry.temperature = Some(5.0);
        }
        let items = diagnose(&config);
        let temp_item = items.iter().find(|i| i.message.contains("temperature"));
        assert!(temp_item.is_some());
        assert_eq!(temp_item.map(|i| i.severity), Some(Severity::Warn));
        assert!(
            temp_item
                .map(|i| i.message.contains("NOT applied"))
                .unwrap_or(false),
            "the warning must say the knob is not applied: {temp_item:?}"
        );
    }

    /// RT-272: even a VALID temperature is not applied, so the doctor must
    /// not report the knob as working.
    #[test]
    fn config_validation_reports_a_valid_temperature_as_unapplied() {
        let items = diagnose(&fixture_config());
        let temp_item = items.iter().find(|i| i.message.contains("temperature"));
        assert!(temp_item.is_some());
        assert_eq!(temp_item.map(|i| i.severity), Some(Severity::Warn));
        assert!(
            temp_item
                .map(|i| i.message.contains("NOT applied"))
                .unwrap_or(false),
            "the warning must say the knob is not applied: {temp_item:?}"
        );
    }

    #[test]
    fn config_validation_catches_unknown_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("totally-fake".into());
        let items = diagnose(&config);
        let prov_item = items
            .iter()
            .find(|i| i.message.contains("default provider"));
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Error));
    }

    #[test]
    fn config_validation_catches_malformed_custom_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("custom:".into());
        let items = diagnose(&config);
        let prov_item = items.iter().find(|item| {
            item.message
                .contains("default provider \"custom:\" is invalid")
        });
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Error));
    }

    #[test]
    fn config_validation_accepts_custom_provider() {
        let mut config = AppConfig::default();
        config.providers.fallback = Some("custom:https://my-api.com".into());
        let items = diagnose(&config);
        let prov_item = items.iter().find(|i| i.message.contains("is valid"));
        assert!(prov_item.is_some());
        assert_eq!(prov_item.map(|i| i.severity), Some(Severity::Ok));
    }

    /// `[providers]` is optional in the AppConfig world: an absent section
    /// (plain `client.base_url` installs) must not produce a check result
    /// at all, let alone an error that would flip `operant doctor`'s exit
    /// code on a working machine.
    #[test]
    fn config_validation_silent_when_providers_section_unused() {
        let items = diagnose(&AppConfig::default());
        assert!(
            !items
                .iter()
                .any(|i| i.message.contains("providers.fallback")),
            "empty [providers] section must not warn"
        );
    }

    #[test]
    fn config_validation_warns_missing_fallback_when_section_has_content() {
        let mut config = AppConfig::default();
        config.providers.model_routes = vec![ModelRouteConfig {
            hint: "fast".into(),
            provider: "groq".into(),
            model: "llama".into(),
            api_key: None,
        }];
        let items = diagnose(&config);
        let fb_item = items
            .iter()
            .find(|i| i.message.contains("no providers.fallback configured"));
        assert!(fb_item.is_some());
        assert_eq!(fb_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_bad_fallback_chain_entry() {
        let mut config = AppConfig::default();
        config.providers.fallback_chain = vec![FallbackProviderConfig {
            provider: "fake-provider".into(),
            model: "m".into(),
        }];
        let items = diagnose(&config);
        let fb_item = items.iter().find(|i| {
            i.message
                .contains("fallback provider \"fake-provider\" is invalid")
        });
        assert!(fb_item.is_some());
        assert_eq!(fb_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_empty_model_route() {
        let mut config = AppConfig::default();
        config.providers.model_routes = vec![ModelRouteConfig {
            hint: "fast".into(),
            provider: "groq".into(),
            model: String::new(),
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items.iter().find(|i| i.message.contains("empty model"));
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_empty_embedding_route_model() {
        let mut config = AppConfig::default();
        config.providers.embedding_routes = vec![EmbeddingRouteConfig {
            hint: "semantic".into(),
            provider: "openai".into(),
            model: String::new(),
            dimensions: Some(1536),
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items.iter().find(|item| {
            item.message
                .contains("embedding route \"semantic\" has empty model")
        });
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn config_validation_warns_invalid_embedding_route_provider() {
        let mut config = AppConfig::default();
        config.providers.embedding_routes = vec![EmbeddingRouteConfig {
            hint: "semantic".into(),
            provider: "groq".into(),
            model: "text-embedding-3-small".into(),
            dimensions: None,
            api_key: None,
        }];
        let items = diagnose(&config);
        let route_item = items
            .iter()
            .find(|item| item.message.contains("uses invalid provider \"groq\""));
        assert!(route_item.is_some());
        assert_eq!(route_item.map(|i| i.severity), Some(Severity::Warn));
    }

    #[test]
    fn environment_check_finds_git() {
        let mut items = Vec::new();
        check_environment(&mut items);
        let git_item = items.iter().find(|i| i.message.starts_with("git:"));
        // git should be available in any CI/dev environment
        assert!(git_item.is_some());
        assert_eq!(git_item.map(|i| i.severity), Some(Severity::Ok));
    }

    #[test]
    fn parse_df_available_mb_uses_last_data_line() {
        let stdout =
            "Filesystem 1M-blocks Used Available Use% Mounted on\n/dev/sda1 1000 500 500 50% /\n";
        assert_eq!(parse_df_available_mb(stdout), Some(500));
    }

    #[test]
    fn truncate_for_display_preserves_utf8_boundaries() {
        let preview = truncate_for_display("🙂example-alpha-build", 3);
        assert_eq!(preview, "🙂ex…");
    }

    #[test]
    #[serial]
    fn bylaws_category_is_gone_with_the_organism() {
        // RT-420: HERMES_HOME is process-wide; without the shared lock this
        // test races every other env-mutating test in the crate.
        let _guard = crate::platform::test_env_lock_blocking();
        // G+1: the bylaws checks retired (Holon owns the organization's
        // health); the doctor's categories are exactly the kept set.
        let tmp = tempfile::TempDir::new().unwrap();
        // SAFETY: serial test, single-threaded runner.
        unsafe { std::env::set_var("HERMES_HOME", tmp.path()) };
        let results = diagnose(&fixture_config());
        assert!(
            !results.iter().any(|r| r.category == "bylaws"),
            "bylaws category must be gone: {results:?}"
        );
        // SAFETY: same serial-test rationale.
        unsafe { std::env::remove_var("HERMES_HOME") };
    }

    #[test]
    fn probe_path_is_hidden_and_unique() {
        let tmp = tempfile::TempDir::new().unwrap();
        let first = probe_file_path(tmp.path());
        let second = probe_file_path(tmp.path());

        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".operant_doctor_probe_"))
        );
    }

    #[test]
    fn render_text_groups_by_category_with_icons() {
        let results = vec![
            CheckResult::ok("config", "one".to_string()),
            CheckResult::ok("config", "two".to_string()),
            CheckResult::warn("environment", "three".to_string()),
            CheckResult::error("workspace", "four".to_string()),
        ];
        let text = render_text(&results);
        assert_eq!(
            text,
            "[config]\n  ✓ one\n  ✓ two\n[environment]\n  ⚠ three\n[workspace]\n  ✗ four\n"
        );
    }
}

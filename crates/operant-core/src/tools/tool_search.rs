//! Progressive tool disclosure ("tool search") for Operant.
//!
//! When active, MCP server tools (names prefixed `mcp_`, registered by
//! `McpNamespacedTool`) are replaced in the model-visible tools array by
//! three bridge tools — `tool_search`, `tool_describe`, `tool_call` — and
//! surfaced on demand. Native builtin tools never defer.
//!
//! Design constraints (hermes `tools/tool_search.py` parity):
//!
//! * **Native tools never defer.** Only MCP tools (the `mcp_*` namespace)
//!   are deferrable. Always-load means always-load.
//! * **Tiered disclosure:**
//!   - Tier 0 — no MCP tools present (or `enabled: "off"`): pure
//!     passthrough, everything eager, bridge tools hidden.
//!   - Tier 1 — MCP tools present and the catalog listing fits the
//!     listing budget (`min(threshold_pct% of context, listing_max_tokens)`):
//!     bridge + skills-style listing (name + short description per tool),
//!     degrading to a names-only listing when the full form is over budget.
//!   - Tier 2 — even names-only is over budget: bare bridge + a
//!     one-line-per-server summary (server name + tool count) so the model
//!     still knows WHICH domains are reachable; individual tools are
//!     discoverable only via `tool_search`.
//! * **The catalog is stateless across turns and tools-array assemblies.**
//!   It is rebuilt from the live registry every time (`tool_search` /
//!   `tool_describe` / `tool_call` all read the registry at call time).
//! * **Bridge tools route through the registry executor exactly like a
//!   direct call**, so timeouts, guardrails, and tool-result truncation
//!   fire identically. `tool_call` executes the underlying tool via
//!   `ToolRegistry::execute` and returns its result verbatim.
//!
//! Two additions live here:
//!
//! * [`deferred`] — turn-triggered materialisation. The bridge above is
//!   model-driven; this is the turn-driven path that re-injects a deferred
//!   tool into the model-visible array when the user's turn overlaps it.
//! * [`name_recovery`] — bounded edit-distance recovery of hallucinated
//!   or near-miss tool names, refusing on equidistant ties.

pub mod deferred;
pub mod name_recovery;

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::config::ToolSearchSettings;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolExposure, ToolRegistry, ToolResult};

/// Bridge tool names. Reserved — any registered tool with these names is
/// treated as part of the bridge, never deferred and never hidden while
/// the bridge is active.
pub const TOOL_SEARCH_NAME: &str = "tool_search";
pub const TOOL_DESCRIBE_NAME: &str = "tool_describe";
pub const TOOL_CALL_NAME: &str = "tool_call";
pub const BRIDGE_TOOL_NAMES: &[&str] = &[TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME];

/// Cheap token estimate from char count without a real tokenizer (~4 chars
/// per token for English+JSON). Underestimating errs toward NOT activating
/// the listing (safe default); overestimating would truncate listings that
/// actually fit.
const CHARS_PER_TOKEN: f64 = 4.0;

/// Whether a tool name is deferrable.
///
/// Two families defer, both for the same reason — describing them eagerly costs
/// far more tokens than the turn that needs them:
///
/// 1. **MCP server tools** (`mcp_*`): an unknown, unbounded set of tools from
///    arbitrary servers. The bridge is the only way a model can discover them.
/// 2. **sourcehound v2 composites** (`papers_search`, `news_fetch`, ... — 23
///    tools): eagerly described they cost **~4,738 tokens on EVERY request**
///    (measured on the live binary, heaviest `news_fetch` ~810); as catalog
///    lines they cost **~760 on the turn that actually researches**. BUGS.md
///    RT-194; the audit that measured both numbers is
///    `docs/audit/2026-10-25-memory-sourcehound-integration-gaps.md` §5.
///
/// Native tools stay eager: they are the surface every turn may need.
pub fn is_deferrable(name: &str) -> bool {
    name.starts_with("mcp_") || is_sourcehound_composite(name)
}

/// Whether `name` is one of sourcehound's 23 v2 composites under its operant
/// spelling (`.` replaced by `_`: `papers.search` -> `papers_search`).
///
/// The check is a scan over the SAME table [`super::sourcehound_composites`]
/// registers, so the classification and the registration cannot drift apart:
/// a composite the table stops naming stops being deferred (and stops being
/// registered), which `composite_table_has_no_duplicates` plus the
/// registration test catch.
fn is_sourcehound_composite(name: &str) -> bool {
    super::sourcehound_composites::composite_specs()
        .iter()
        .any(|spec| spec.tool_name == name)
}

/// Result of assembling the model-visible tools array.
pub struct AssembledTools {
    /// Schemas to send to the LLM (core tools + bridge tools when active).
    pub visible: Vec<ToolSchema>,
    /// Deferred (MCP) schemas hidden behind the bridge this assembly.
    pub deferred: Vec<ToolSchema>,
}

/// Partition schemas and apply tiered disclosure.
pub fn assemble_tools(
    all: Vec<ToolSchema>,
    settings: &ToolSearchSettings,
    context_window: usize,
) -> AssembledTools {
    let mut core = Vec::new();
    let mut deferred = Vec::new();
    let mut hidden = Vec::new();
    for s in all {
        // Partition on the exposure the REGISTRY stamped at
        // `get_schemas_all` (tools.rs). The TYPE is authoritative and the name
        // rule is only a fallback for a schema that never went through the
        // registry (`ToolSchema::new` leaves `Direct`), which in production is
        // no schema at all — `tools.rs:676` is the sole caller and always
        // passes stamped schemas.
        //
        // So:
        // - Direct stays eager even when its name starts with `mcp_`: an
        //   operant-native tool the prefix used to catch by accident.
        // - Deferred is deferred even with no `mcp_` prefix: a type that opted
        //   in deliberately.
        // - `is_deferrable` survives only for unstamped schemas, i.e. the
        //   tools the registry never minted (the bridge tools' own schemas and
        //   hand-built test fixtures).
        match s.exposure {
            ToolExposure::Hidden => hidden.push(s),
            ToolExposure::Deferred => deferred.push(s),
            ToolExposure::Direct => core.push(s),
        }
    }
    // A Hidden tool is registered and callable but offered to nobody. It is
    // dropped here rather than filtered upstream so `get_schemas` (the
    // operator-facing listing) still shows it: hiding is a MODEL-visibility
    // decision, not a registration one.
    drop(hidden);
    // The bridge tools are registered like any other tool; they are always
    // excluded from the "core" pass so they never duplicate.
    core.retain(|s| !BRIDGE_TOOL_NAMES.contains(&s.name.as_str()));

    let active = match settings.enabled.as_str() {
        "on" => true,
        "off" => false,
        // "auto": activate only when deferrable tools exist.
        _ => !deferred.is_empty(),
    };

    if !active || deferred.is_empty() {
        // Tier 0 — pure passthrough. EVERYTHING stays eager (including
        // any MCP tools); bridge tools stay hidden.
        let mut visible = core;
        visible.extend(deferred);
        return AssembledTools {
            visible,
            deferred: Vec::new(),
        };
    }

    let listing = build_listing(&deferred, settings, context_window);
    let mut visible = core;
    visible.push(build_search_schema(&listing));
    visible.push(build_describe_schema());
    visible.push(build_call_schema());

    AssembledTools { visible, deferred }
}

/// Resolve the listing mode against the budget:
/// "full" → skills-style listing; "names" → names-only; "bare" → Tier 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListingMode {
    Full,
    Names,
    Bare,
}

fn listing_enabled(settings: &ToolSearchSettings) -> bool {
    settings.listing != "off"
}

/// Effective listing token budget = min(listing_max_tokens, threshold% of
/// context window). A zero context window means "unknown" → rely on the
/// absolute cap only.
fn listing_budget(settings: &ToolSearchSettings, context_window: usize) -> usize {
    let pct_budget = if context_window > 0 {
        (settings.threshold_pct / 100.0 * context_window as f64) as usize
    } else {
        usize::MAX
    };
    settings.listing_max_tokens.min(pct_budget)
}

fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as usize
}

/// Decide which listing form fits the budget.
fn resolve_listing_mode(
    deferred: &[ToolSchema],
    settings: &ToolSearchSettings,
    context_window: usize,
) -> ListingMode {
    if !listing_enabled(settings) {
        return ListingMode::Bare;
    }
    let budget = listing_budget(settings, context_window);
    let full = full_listing_text(deferred);
    if estimate_tokens(&full) <= budget {
        return ListingMode::Full;
    }
    let names = names_listing_text(deferred);
    if estimate_tokens(&names) <= budget {
        return ListingMode::Names;
    }
    ListingMode::Bare
}

fn full_listing_text(deferred: &[ToolSchema]) -> String {
    deferred
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n")
}

fn names_listing_text(deferred: &[ToolSchema]) -> String {
    deferred
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Group deferred tools by their `mcp_<server>_` prefix for the Tier 2
/// per-server summary.
fn server_summary(deferred: &[ToolSchema]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for s in deferred {
        // `mcp_<server>_<tool>` — strip the leading `mcp_` and take the
        // first underscore-delimited segment as the server name (tool
        // names may themselves contain underscores).
        let rest = s.name.strip_prefix("mcp_").unwrap_or(&s.name);
        let server = match rest.split_once('_') {
            Some((server, _)) => server,
            None => rest,
        };
        match counts.iter_mut().find(|(name, _)| name == server) {
            Some((_, n)) => *n += 1,
            None => counts.push((server.to_string(), 1)),
        }
    }
    counts
        .iter()
        .map(|(server, n)| format!("- {}: {} tool(s)", server, n))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_listing(
    deferred: &[ToolSchema],
    settings: &ToolSearchSettings,
    context_window: usize,
) -> String {
    match resolve_listing_mode(deferred, settings, context_window) {
        ListingMode::Full => full_listing_text(deferred),
        ListingMode::Names => format!(
            "Deferred tools ({}): {}",
            deferred.len(),
            names_listing_text(deferred)
        ),
        ListingMode::Bare => format!(
            "Deferred MCP servers ({} tool(s) total):\n{}",
            deferred.len(),
            server_summary(deferred)
        ),
    }
}

fn build_search_schema(listing: &str) -> ToolSchema {
    let listing_block = if listing.is_empty() {
        String::new()
    } else {
        format!("\n\nCurrently deferred tools (call tool_describe for full schemas):\n{listing}")
    };
    ToolSchema::new(
        TOOL_SEARCH_NAME,
        format!(
            "Search the catalog of deferred (MCP) tools that are NOT directly callable this \
             turn. Returns matching tool names and short descriptions. Load a full JSON schema \
             with tool_describe, then invoke with tool_call.{listing_block}"
        ),
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to match against deferred tool names and descriptions. Empty matches all."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return."
                }
            },
            "required": ["query"]
        }),
    )
}

fn build_describe_schema() -> ToolSchema {
    ToolSchema::new(
        TOOL_DESCRIBE_NAME,
        "Return the full JSON schema for one deferred (MCP) tool returned by tool_search, so \
         you can construct valid arguments before calling tool_call.",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact deferred tool name (as returned by tool_search)."
                }
            },
            "required": ["name"]
        }),
    )
}

fn build_call_schema() -> ToolSchema {
    ToolSchema::new(
        TOOL_CALL_NAME,
        "Invoke a deferred (MCP) tool by its exact name with JSON arguments. Use tool_search to \
         discover the name and tool_describe to load its schema first.",
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Exact deferred tool name to execute."
                },
                "arguments": {
                    "type": "object",
                    "description": "Tool arguments as a JSON object."
                }
            },
            "required": ["name", "arguments"]
        }),
    )
}

fn synthetic_call_id(tool: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{tool}_{nanos}")
}

/// `tool_search` — search the deferred catalog (rebuilt statelessly from
/// the live registry at call time).
pub struct ToolSearchTool {
    registry: ToolRegistry,
}

impl ToolSearchTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }

    /// The deferred schemas currently registered (MCP tools only).
    async fn deferred_schemas(&self) -> Vec<ToolSchema> {
        self.registry
            .get_schemas()
            .await
            .into_iter()
            .filter(|s| is_deferrable(&s.name))
            .collect()
    }
}

#[async_trait]
impl OperantTool for ToolSearchTool {
    fn name(&self) -> &str {
        TOOL_SEARCH_NAME
    }

    fn description(&self) -> &str {
        "Search the catalog of deferred (MCP) tools not directly callable this turn."
    }

    fn schema(&self) -> ToolSchema {
        build_search_schema("")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(10)
            .min(25);

        let deferred = self.deferred_schemas().await;
        // Rank instead of sorting alphabetically (iter-1510). An empty query
        // matches EVERYTHING, so alphabetical order truncated at `limit` handed
        // the model the alphabetically-first tools rather than the closest
        // ones; and for a real query the substring filter admitted every
        // tool that merely mentioned a term, so the most relevant result
        // could be cut by the limit while a marginally-related one survived.
        // This is the token-overlap score from the portability ledger's cheap
        // version — a name hit weighs three times a description hit. The ledger
        // suggested a shorter-name tiebreak; this sorts equal scores by name ASC
        // instead, because a deterministic name order is the one property a test
        // can pin ("shorter name" would make `web` outrank `web_search` for no
        // reason a reader could check). Both are heuristics; determinism is not
        // optional.
        let mut scored: Vec<(usize, &crate::schema::ToolSchema)> = deferred
            .iter()
            .filter(|s| search_matches(&query, s))
            .map(|s| (relevance_score(&query, s), s))
            .collect();
        // Total order, so the result is deterministic: score descending, then
        // name ascending as the final tiebreak. Without the explicit name
        // tiebreak two equal-scoring tools could swap places between runs and
        // a test (or the model) would see a different order for the same query.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.truncate(limit);
        let matches: Vec<(String, String)> = scored
            .into_iter()
            .map(|(_, s)| (s.name.clone(), s.description.clone()))
            .collect();

        ToolResult::success(
            synthetic_call_id(TOOL_SEARCH_NAME),
            json!({
                "query": query,
                "total": deferred.len(),
                "matches": matches
                    .into_iter()
                    .map(|(name, description)| json!({ "name": name, "description": description }))
                    .collect::<Vec<_>>()
            }),
        )
    }
}

/// True when a tool is a candidate for this query.
///
/// An empty query matches everything (it is the "browse the catalog" case,
/// where relevance ordering matters most precisely because there is no signal
/// to rank on). A non-empty query splits into whitespace-separated terms and
/// requires at least one term to appear in the name or description.
///
/// Token-splitting on the QUERY is the change from the old substring test
/// (`name.contains(query)`): the old form required the whole query to appear
/// verbatim, so "github issue" missed a tool named `gh_issue_list` whose
/// description says "list github issues". Each term is matched as a substring
/// rather than a whole word, so partial names still match.
fn search_matches(query: &str, schema: &crate::schema::ToolSchema) -> bool {
    if query.is_empty() {
        return true;
    }
    let name = schema.name.to_lowercase();
    let description = schema.description.to_lowercase();
    query
        .split_whitespace()
        .any(|term| name.contains(term) || description.contains(term))
}

/// Relevance score: query terms hitting the NAME weigh three times a term that
/// hits only the DESCRIPTION.
///
/// The weights are a heuristic, not a measurement — the ledger that prescribed
/// this (the portability contrast's "cheap version") chose them because the
/// name is what the model has already decided it wants. They are deliberately
/// round and reviewable; if a measurement later shows 3:1 is wrong, the constant
/// is one line and every test that pins an ordering will name the new behaviour.
fn relevance_score(query: &str, schema: &crate::schema::ToolSchema) -> usize {
    if query.is_empty() {
        return 0;
    }
    let name = schema.name.to_lowercase();
    let description = schema.description.to_lowercase();
    let mut score = 0usize;
    for term in query.split_whitespace() {
        if name.contains(term) {
            score += 3;
        }
        if description.contains(term) {
            score += 1;
        }
    }
    score
}

/// `tool_describe` — return the full JSON schema for one deferred tool.
pub struct ToolDescribeTool {
    registry: ToolRegistry,
}

impl ToolDescribeTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl OperantTool for ToolDescribeTool {
    fn name(&self) -> &str {
        TOOL_DESCRIBE_NAME
    }

    fn description(&self) -> &str {
        "Return the full JSON schema for one deferred (MCP) tool."
    }

    fn schema(&self) -> ToolSchema {
        build_describe_schema()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let name = match args.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                return ToolResult::error(
                    synthetic_call_id(TOOL_DESCRIBE_NAME),
                    "tool_describe requires a non-empty 'name' argument".to_string(),
                );
            }
        };

        let schema = self
            .registry
            .get_schemas()
            .await
            .into_iter()
            .find(|s| s.name == name);

        match schema {
            Some(s) if is_deferrable(&s.name) => ToolResult::success(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                json!({
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.parameters,
                }),
            ),
            Some(_) => ToolResult::error(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                format!("'{name}' is not a deferred (MCP) tool — call it directly."),
            ),
            None => ToolResult::error(
                synthetic_call_id(TOOL_DESCRIBE_NAME),
                format!("No tool named '{name}' is registered."),
            ),
        }
    }
}

/// `tool_call` — execute a deferred tool through the normal registry
/// executor, so timeouts/guardrails/truncation behave identically to a
/// direct call.
pub struct ToolCallTool {
    registry: ToolRegistry,
}

impl ToolCallTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl OperantTool for ToolCallTool {
    fn name(&self) -> &str {
        TOOL_CALL_NAME
    }

    fn description(&self) -> &str {
        "Invoke a deferred (MCP) tool by name with JSON arguments."
    }

    fn schema(&self) -> ToolSchema {
        build_call_schema()
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let id = synthetic_call_id(TOOL_CALL_NAME);
        let requested = match args.get("name").and_then(Value::as_str) {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => {
                return ToolResult::error(
                    id,
                    "tool_call requires a non-empty 'name' argument".to_string(),
                );
            }
        };
        let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let name = requested.clone();

        if BRIDGE_TOOL_NAMES.contains(&name.as_str()) {
            return ToolResult::error(id, format!("'{name}' is a bridge tool — call it directly."));
        }

        // Bounded edit-distance recovery: a near-miss deferred name (the
        // model writing `mcp_gh_create_pr` for `mcp_github_create_pr`)
        // is repaired once here, so the rest of the bridge sees a real
        // name. Only attempted when the name is not ALREADY registered: a
        // name that IS registered but is unavailable is a deliberate ban,
        // and recovery must never launder a ban into a near-miss success.
        let name = if self.registry.contains(&name).await {
            name
        } else {
            match &name_recovery::resolve(&self.registry, &name).await {
                name_recovery::Recovery::Recovered { canonical, .. } => canonical.clone(),
                name_recovery::Recovery::Exact => name,
                refused => {
                    return ToolResult::error(
                        id,
                        name_recovery::refusal_message(&requested, refused),
                    );
                }
            }
        };

        if !is_deferrable(&name) {
            return ToolResult::error(
                id,
                format!("'{name}' is not a deferred tool — call it directly."),
            );
        }
        // Availability check mirrors a direct call: a deferred tool that is
        // disabled (by name or toolset) must not be invocable through the
        // bridge around the ban. `is_available` covers registration too.
        if !self.registry.is_available(&name).await {
            return ToolResult::error(
                id,
                format!("Tool '{name}' is not registered or is currently disabled."),
            );
        }

        // Execute through the registry: the executor stamps the inner
        // result's id/name and enforces the per-tool timeout. The inner
        // result is returned verbatim so the loop's normal tool-result
        // pipeline (truncation, persistence) applies to the real tool.
        match self.registry.execute(&name, &id, arguments, context).await {
            Ok(result) => result,
            Err(e) => ToolResult::error(id, e.to_string()),
        }
    }
}

#[cfg(test)]
struct DeferredDummy {
    name: &'static str,
    description: &'static str,
    exposure: ToolExposure,
}

#[cfg(test)]
impl DeferredDummy {
    fn new(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            exposure: ToolExposure::Direct,
        }
    }

    /// A non-MCP tool that declares itself deferred by TYPE. The name is
    /// deliberately free of the `mcp_` prefix so the string rule and the
    /// type rule disagree — which is what makes the test able to tell them
    /// apart.
    fn deferred(name: &'static str, description: &'static str) -> Self {
        Self {
            name,
            description,
            exposure: ToolExposure::Deferred,
        }
    }
}

#[cfg(test)]
#[async_trait]
impl OperantTool for DeferredDummy {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name, self.description, json!({ "type": "object" }))
            .with_exposure(self.exposure)
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(
            "dummy",
            json!({ "echoed_x": args.get("x").cloned().unwrap_or(Value::Null) }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ts(enabled: &str, listing: &str, listing_max_tokens: usize) -> ToolSearchSettings {
        ToolSearchSettings {
            enabled: enabled.to_string(),
            threshold_pct: 5.0,
            listing: listing.to_string(),
            listing_max_tokens,
            search_default_limit: 10,
            max_search_limit: 25,
        }
    }

    /// Stamp the exposure the registry would stamp, so a hand-built fixture
    /// arrives at `assemble_tools` in the same shape production hands it. The
    /// tests that say "an mcp_* name defers" describe what the REGISTRY does
    /// today for MCP tools; the ones that say "a type defers regardless of
    /// name" build the exposure explicitly via `DeferredDummy`.
    fn schema(name: &str, description: &str) -> ToolSchema {
        let exposure = if is_deferrable(name) {
            ToolExposure::Deferred
        } else {
            ToolExposure::Direct
        };
        ToolSchema::new(name, description, json!({ "type": "object" })).with_exposure(exposure)
    }

    #[test]
    fn tier0_passthrough_when_no_deferrable() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("file_read", "Read a file"),
        ];
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(out.visible.len(), 2);
        assert!(out.deferred.is_empty());
        assert!(
            out.visible
                .iter()
                .all(|s| !BRIDGE_TOOL_NAMES.contains(&s.name.as_str()))
        );
    }

    #[test]
    fn mcp_tools_defer_behind_bridge() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("mcp_server_alpha", "Do alpha things"),
            schema("mcp_server_beta", "Do beta things"),
        ];
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(out.deferred.len(), 2);
        let names: Vec<&str> = out.visible.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"terminal"));
        assert!(names.contains(&TOOL_SEARCH_NAME));
        assert!(names.contains(&TOOL_DESCRIBE_NAME));
        assert!(names.contains(&TOOL_CALL_NAME));
        assert!(!names.contains(&"mcp_server_alpha"));
        assert!(!names.contains(&"mcp_server_beta"));
    }

    #[test]
    fn off_disables_bridge_even_with_mcp_tools() {
        let all = vec![
            schema("terminal", "Run a shell command"),
            schema("mcp_server_alpha", "Do alpha things"),
        ];
        let out = assemble_tools(all, &ts("off", "auto", 4000), 128_000);
        assert!(out.deferred.is_empty());
        assert_eq!(out.visible.len(), 2); // both eager, no bridge
    }

    #[test]
    fn listing_degrades_to_names_when_full_over_budget() {
        let deferred = vec![
            schema("mcp_s_alpha", "Alpha tool with a long description "),
            schema("mcp_s_beta", "Beta tool with a long description "),
        ];
        // Tiny budget: full listing won't fit, names-only will.
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 15), 128_000);
        assert_eq!(mode, ListingMode::Names);
    }

    #[test]
    fn listing_degrades_to_bare_when_names_over_budget() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha"), schema("mcp_s_beta", "Beta")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 5), 128_000);
        assert_eq!(mode, ListingMode::Bare);
    }

    #[test]
    fn listing_off_always_bare() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "off", 4000), 128_000);
        assert_eq!(mode, ListingMode::Bare);
    }

    #[test]
    fn server_summary_groups_by_server_prefix() {
        let deferred = vec![
            schema("mcp_alpha_tool_one", "1"),
            schema("mcp_alpha_tool_two", "2"),
            schema("mcp_beta_tool_three", "3"),
        ];
        let summary = server_summary(&deferred);
        assert!(summary.contains("alpha: 2 tool(s)"), "{summary}");
        assert!(summary.contains("beta: 1 tool(s)"), "{summary}");
    }

    #[tokio::test]
    async fn tool_search_queries_registry_statelessly() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(ToolSearchTool::new(registry.clone()))
            .await
            .unwrap();
        // Register a deferred tool directly into the shared map.
        registry
            .register(DeferredDummy::new(
                "mcp_srv_alpha",
                "Handles alpha processing",
            ))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new(
                "mcp_srv_beta",
                "Handles beta processing",
            ))
            .await
            .unwrap();

        let tool = ToolSearchTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "query": "alpha", "limit": 10 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let matches = parsed["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "mcp_srv_alpha");
    }

    #[tokio::test]
    async fn tool_describe_returns_full_schema() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        // A real MCP tool: deferred BY TYPE, which is what the registry now
        // stamps for `McpNamespacedTool`. The name still starts with `mcp_`
        // so the assertion below reads the same as it always did.
        registry
            .register(DeferredDummy::deferred("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let tool = ToolDescribeTool::new(registry.clone());
        let result = tool
            .execute(json!({ "name": "mcp_srv_alpha" }), ToolContext::default())
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["name"], "mcp_srv_alpha");
        assert!(parsed["parameters"].is_object());
    }

    #[tokio::test]
    async fn tool_call_executes_underlying_deferred_tool() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "mcp_srv_alpha", "arguments": { "x": 1 } }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["echoed_x"], 1);
        // The inner result name is stamped by the executor to the inner tool.
        assert_eq!(result.name, "mcp_srv_alpha");
    }

    #[tokio::test]
    async fn tool_call_rejects_non_deferred_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("terminal", "Native"))
            .await
            .unwrap();
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "terminal", "arguments": {} }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let message = result.error.as_deref().unwrap_or(&result.content);
        assert!(message.contains("not a deferred tool"), "{message}");
    }

    #[tokio::test]
    async fn tool_call_cannot_bypass_disabled_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        registry.disable_tool("mcp_srv_alpha").await;
        let tool = ToolCallTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "name": "mcp_srv_alpha", "arguments": {} }),
                ToolContext::default(),
            )
            .await;
        assert!(
            !result.success,
            "disabled tool must not be invocable via bridge"
        );
        let message = result.error.as_deref().unwrap_or(&result.content);
        assert!(message.contains("disabled"), "{message}");
    }

    #[tokio::test]
    async fn get_schemas_for_request_hides_bridge_in_tier0_and_shows_when_active() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("terminal", "Native"))
            .await
            .unwrap();
        registry
            .register(ToolSearchTool::new(registry.clone()))
            .await
            .unwrap();
        registry
            .register(ToolDescribeTool::new(registry.clone()))
            .await
            .unwrap();
        registry
            .register(ToolCallTool::new(registry.clone()))
            .await
            .unwrap();

        // Tier 0: no MCP tools → pure passthrough, bridge hidden.
        let visible = registry
            .get_schemas_for_request(&ts("auto", "auto", 4000), 128_000)
            .await;
        assert!(visible.iter().all(|s| s.name != "tool_search"));
        assert!(visible.iter().any(|s| s.name == "terminal"));

        // Active: a real MCP tool arrives, deferred BY TYPE — which is what
        // the registry now stamps for `McpNamespacedTool`.
        registry
            .register(DeferredDummy::deferred("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        let visible = registry
            .get_schemas_for_request(&ts("auto", "auto", 4000), 128_000)
            .await;
        let names: Vec<&str> = visible.iter().map(|s| s.name.as_str()).collect();
        eprintln!("PROBE3 visible={names:?}");
        assert!(names.contains(&"tool_describe"));
        assert!(names.contains(&"tool_call"));
        assert!(names.contains(&"terminal"));
        assert!(!names.contains(&"mcp_srv_alpha"));
    }

    #[test]
    fn listing_budget_with_zero_context_falls_back_to_cap() {
        let settings = ts("auto", "auto", 100);
        assert_eq!(listing_budget(&settings, 0), 100);
    }

    #[test]
    fn full_listing_fits_within_budget() {
        let deferred = vec![schema("mcp_s_alpha", "Alpha")];
        let mode = resolve_listing_mode(&deferred, &ts("auto", "auto", 4000), 128_000);
        assert_eq!(mode, ListingMode::Full);
        let text = build_listing(&deferred, &ts("auto", "auto", 4000), 128_000);
        assert!(text.contains("mcp_s_alpha"), "{text}");
        assert!(text.contains("Alpha"), "{text}");
    }

    // ── Relevance ranking (iter-1510) ──────────────────────────────────
    //
    // The old behaviour sorted by tool NAME alone, so the result was the
    // alphabetically-first N matches. Two distinct ways that produced the wrong
    // top-N:
    //
    // 1. An empty query matches every deferred tool, so truncating the
    //    alphabetically-sorted list returned whichever tools happened to sort
    //    first — an arbitrary subset, not a browsable one.
    // 2. A real query admitted every tool that merely MENTIONED a term, so a
    //    tool named `mcp_github_issue_create` and one that says "…handles
    //    github issues…" in prose both scored zero and were ordered by name.
    //
    // Each case below pins the property that failed, not the whole ordering.

    /// Drive `tool_search` for real and return the ordered match names.
    async fn search_names(registry: &ToolRegistry, query: &str, limit: usize) -> Vec<String> {
        let tool = ToolSearchTool::new(registry.clone());
        let result = tool
            .execute(
                json!({ "query": query, "limit": limit }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "{}", result.content);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        parsed["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap().to_string())
            .collect()
    }
    #[tokio::test]
    async fn a_name_hit_outranks_a_description_only_hit() {
        // Fixture design is the whole test: the description-only tool is named
        // `aaa_...` and the name-hit tool `zzz_...`, so ALPHABETICAL order puts
        // the irrelevant tool FIRST. A ranking that silently degrades to
        // alphabetical would then still pass a fixture where the two happen to
        // agree — which is exactly what the first version of these fixtures did
        // (they passed with every score forced to 0, i.e. vacuous).
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new(
                "mcp_zzz_github_issue_create",
                "Creates a record in the tracker",
            ))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new(
                "mcp_aaa_project_tools",
                "Handles github issues and issue comments for a repository",
            ))
            .await
            .unwrap();

        // Both match "github issue": one in its name, one only in prose.
        let names = search_names(&registry, "github issue", 10).await;
        assert_eq!(names.len(), 2, "both tools match: {names:?}");
        assert_eq!(
            names[0], "mcp_zzz_github_issue_create",
            "the tool whose NAME carries the terms must outrank the one that only \
             mentions them in prose — alphabetical order puts mcp_aaa_project_tools \
             first, so this fails if the ranking degrades to a name sort: {names:?}"
        );

        // And at limit 1 the wrong ordering drops the right tool entirely,
        // which is the failure an operator actually sees.
        let capped = search_names(&registry, "github issue", 1).await;
        assert_eq!(
            capped,
            vec!["mcp_zzz_github_issue_create".to_string()],
            "at limit 1 the relevant tool must survive, not be cut by the \
             alphabetically-earlier one: {capped:?}"
        );
    }

    #[tokio::test]
    async fn multi_term_query_matches_terms_split_across_fields() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new(
                "mcp_gh_list",
                "List github issues for the repository",
            ))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new(
                "mcp_unrelated_thing",
                "Reads local files from the workspace",
            ))
            .await
            .unwrap();

        // The OLD filter required the whole query string to appear verbatim in
        // one field, so "github issue" never matched a name of "gh_list" even
        // though both terms were present across name and description.
        let names = search_names(&registry, "github gh", 10).await;
        assert_eq!(
            names,
            vec!["mcp_gh_list".to_string()],
            "each query term must match independently, so terms split across the \
             name and the description still find the tool: {names:?}"
        );
    }

    #[tokio::test]
    async fn ranking_is_deterministic_for_equal_scores() {
        // Equal-scoring tools must not swap order between runs. `sort_by` is
        // stable, but a score-only key leaves sequence-dependent output, so the
        // tiebreak is explicit in the implementation and pinned here.
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_zeta", "alpha processing"))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new("mcp_alpha", "alpha processing"))
            .await
            .unwrap();

        for _ in 0..5 {
            let names = search_names(&registry, "alpha", 10).await;
            assert_eq!(
                names,
                vec!["mcp_alpha".to_string(), "mcp_zeta".to_string()],
                "equal scores must break by name ascending, every run: {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn empty_query_still_returns_a_result_not_an_error() {
        // The browse case. With ranking the empty query now orders by nothing
        // (every score is 0), so the result is the alphabetically-first N —
        // which is a defensible browse order, but it must still WORK.
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new("mcp_srv_alpha", "Alpha"))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new("mcp_srv_beta", "Beta"))
            .await
            .unwrap();

        let names = search_names(&registry, "", 10).await;
        assert_eq!(names.len(), 2, "an empty query lists everything: {names:?}");
        assert_eq!(
            names,
            vec!["mcp_srv_alpha".to_string(), "mcp_srv_beta".to_string()],
            "with no ranking signal the fallback must be the deterministic \
             alphabetical order: {names:?}"
        );
    }

    #[test]
    fn the_sort_must_not_be_alphabetical_only() {
        // Source pin on the exact defect, so a regression to alphabetical-only
        // ordering fails by name.
        //
        // Matched on the code shape rather than a quoted line: this file's prose
        // describes what changed, and a substring search for the old comparator
        // would match its own explanation (and its own literal) before it matched
        // a regression. That is the trap the isolation-layer guard paid for.
        let src = include_str!("tool_search.rs");

        // The scorer must be consulted when building the candidates.
        assert!(
            src.contains("relevance_score(&query, s)"),
            "tool_search must score each candidate, not just filter it"
        );
        // ...and the sort must order by that score, with a name tiebreak so
        // equal scores do not swap places between runs.
        assert!(
            src.contains(".then_with(|| a.1.name.cmp(&b.1.name))"),
            "the ranked sort must carry an explicit name tiebreak"
        );
        // Every `sort_by` in the file must therefore compare the score, never
        // the name alone. `a.0` is the first tuple field (the score); a
        // comparator reading only `a.1`/`b.1` is the old alphabetical behaviour
        // wearing a new name.
        for line in src
            .lines()
            .filter(|l| l.contains("sort_by") && l.contains(".0.cmp("))
        {
            assert!(
                !line.contains("a.1.cmp(&b.1)"),
                "a sort_by that compares the name directly is alphabetical-only: {line}"
            );
        }
    }
    // ── Exposure is a type-level decision, not a name prefix ─────────────
    //
    // Every fixture below is built so the STRING rule and the TYPE rule
    // DISAGREE. A test whose two fixtures happen to rank identically under
    // both rules proves nothing — that is the vacuous-fixture trap already
    // recorded once in this file (the ranking tests' first fixtures passed
    // with every score forced to 0).

    /// A tool with NO `mcp_` prefix that declares itself `Deferred` must be
    /// deferred. Under the old string-only rule this tool was eagerly
    /// offered, so this test fails if `assemble_tools` reverts to
    /// `is_deferrable`.
    #[tokio::test]
    async fn a_tool_deferring_by_type_is_deferred_without_the_mcp_prefix() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::deferred(
                "zzz_internal_sidecar",
                "An operant-owned sidecar, deliberately not named mcp_*",
            ))
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new("core_shell", "Run a shell command"))
            .await
            .unwrap();

        let all = registry.get_schemas().await;
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);

        let names: Vec<&str> = out.deferred.iter().map(|s| s.name.as_str()).collect();
        let visible: Vec<&str> = out.visible.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["zzz_internal_sidecar"],
            "a Deferred tool with no mcp_ prefix must land in the deferred set: {names:?}"
        );
        assert!(
            !visible.contains(&"zzz_internal_sidecar"),
            "it must NOT be offered directly: {visible:?}"
        );
        assert!(
            visible.contains(&"core_shell"),
            "the Direct tool stays eager: {visible:?}"
        );
    }

    /// A tool that merely HAPPENS to be named `mcp_*` but is owned by operant
    /// (so its type says `Direct`) must stay eager. This is the behaviour the
    /// string prefix accidentally took away, and the reason the row exists.
    #[tokio::test]
    async fn an_operant_tool_called_mcp_something_is_not_deferred_by_accident() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy::new(
                "mcp_notes_local",
                "An operant-native tool whose name merely starts with mcp_",
            ))
            .await
            .unwrap();

        let all = registry.get_schemas().await;
        let out = assemble_tools(all, &ts("auto", "auto", 4000), 128_000);

        let visible: Vec<&str> = out.visible.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            out.deferred.len(),
            0,
            "a Direct tool must not be deferred because of its name: {:?}",
            out.deferred.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert!(
            visible.contains(&"mcp_notes_local"),
            "it must stay eagerly offered: {visible:?}"
        );
    }

    /// `Hidden` is registered-and-callable but never offered and never
    /// discoverable — so it appears on NEITHER side.
    #[tokio::test]
    async fn a_hidden_tool_appears_on_neither_side() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry
            .register(DeferredDummy {
                name: "core_experimental",
                description: "Wired but withheld from the model",
                exposure: ToolExposure::Hidden,
            })
            .await
            .unwrap();
        registry
            .register(DeferredDummy::new("core_shell", "Run a shell command"))
            .await
            .unwrap();

        let out = assemble_tools(
            registry.get_schemas().await,
            &ts("auto", "auto", 4000),
            128_000,
        );
        assert!(
            !out.visible.iter().any(|s| s.name == "core_experimental"),
            "a Hidden tool must never be offered: {:?}",
            out.visible.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert!(
            !out.deferred.iter().any(|s| s.name == "core_experimental"),
            "a Hidden tool must not be discoverable through the bridge either: {:?}",
            out.deferred.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert!(
            out.visible.iter().any(|s| s.name == "core_shell"),
            "an ordinary tool is unaffected by a sibling being Hidden"
        );
    }

    /// The two real deferred families still defer after the change, so the
    /// migration did not silently un-defer anything.
    #[tokio::test]
    async fn the_real_deferred_families_still_defer() {
        // The sourcehound composites defer by TYPE now; assert the source
        // says so rather than spawning a binary.
        let src = include_str!("sourcehound_composites.rs");
        let start = src
            .find("impl OperantTool for SourcehoundCompositeTool")
            .expect("composite tool impl present");
        let block = &src[start..(start + 1200).min(src.len())];
        assert!(
            block.contains("ToolExposure::Deferred"),
            "the composite family must opt into Deferred by type — the old \
             name-table scan is no longer the classifier: {block}"
        );
    }

    /// `get_schemas_all` is the one place the registry still holds the tool
    /// while minting the schema; it must stamp the exposure there, or the
    /// assembler only ever sees `Direct` and the enum is dead weight.
    #[tokio::test]
    async fn the_registry_stamps_exposure_onto_the_schema_it_mints() {
        let src = include_str!("../tools.rs");
        assert_eq!(
            src.matches(".with_exposure(t.exposure())").count(),
            3,
            "all three schema-minting sites must stamp the tool's exposure"
        );
        let src2 = include_str!("tool_search.rs");
        assert!(
            src2.contains("deferred_by_type"),
            "assemble_tools must consult the stamped exposure"
        );
    }
}

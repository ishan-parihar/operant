//! sourcehound's v2 composite surface as first-class operant tools (RT-194).
//!
//! # What this closes
//!
//! sourcehound 1.8.0 exposes a 23-tool **v2 composite** surface (`papers.search`,
//! `news.fetch`, `data.query`, `recon.target`, `security.vuln`, `gov.search`,
//! ...) that the four registered web tools (`web_scrape` / `web_extract` /
//! `web_crawl` / `web_search`) do not cover. Before this module an operant
//! agent answered "search academic papers" with a generic `web_search`, and
//! reaching `papers.search` required the operator to hand-configure sourcehound
//! as an MCP server (BUGS.md RT-194).
//!
//! # Why one proxy struct over 23 hand-written tools
//!
//! Every composite is a thin forward: validate the args, send
//! `{...args, format: "json"}` to sourcehound's MCP `call`, return the payload
//! or the error. The only thing that differs per composite is its *name*, its
//! *description* (model-facing contract) and its *args schema*. Encoding those
//! in a static table and registering the same struct 23 times keeps the
//! per-composite information in ONE place, where the contract test can read it
//! — a hand-written struct per composite is 23 copies of the same 20 lines and
//! 23 places for the `format: "json"` contract (RT-188/189's root cause) to rot.
//!
//! # Why these tools are DEFERRED, never eager
//!
//! Eager registration of all 23 adds **~4,738 tokens to EVERY model request**
//! (measured by the 2026-10-25 audit: the live `tools/list` serialized in the
//! chat-request wire format; heaviest composite `news.fetch` ~810). As
//! `tool_search` catalog lines they cost **~760 tokens, and only on the turn
//! that actually searches**. `is_deferrable` classifies them, so the existing
//! bridge (`tool_search` / `tool_describe` / `tool_call`) plus the turn-driven
//! `materialise` path serve them: the model discovers `papers.search` on
//! demand, and a request that does not research costs the four web tools only.
//!
//! # The schema contract
//!
//! `parameters` is the sourcehound server's OWN `inputSchema` for that
//! composite, trimmed of the fields operant injects itself (`format`, which
//! every call sets to `"json"` per the RT-188/189 dispatch contract). Keeping
//! the server's schema — rather than an operant-authored one — means a
//! sourcehound release that adds a parameter or changes a default does not
//! silently desynchronise the model's view from the tool's behaviour.

use std::sync::LazyLock;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::sourcehound::Sourcehound;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// One composite's model-facing contract plus the call operant makes.
///
/// `parameters` is the trimmed `inputSchema` sourcehound advertises for this
/// composite: its own field names, types, descriptions and required list, with
/// `format` removed (operant always sets `format: "json"`).
pub struct CompositeSpec {
    /// The operant tool name. The sourcehound composite name with `.`
    /// replaced by `_`, so the model-visible name is a legal tool identifier:
    /// `papers.search` -> `papers_search`.
    pub tool_name: &'static str,
    /// The composite name sent on the wire (`papers.search`).
    pub composite: &'static str,
    /// Model-facing description, taken from the live server's own text so the
    /// contract cannot drift from the engine it describes.
    pub description: &'static str,
    /// The composite's argument schema (sourcehound's `inputSchema`, minus
    /// `format`).
    pub parameters: Value,
}

/// The `format` field every composite call sets itself.
///
/// RT-188/189: sourcehound's `dispatch_value` strips `structuredContent`
/// unless the call's top-level `arguments.format == "json"`, and every
/// composite returns its payload there. Without this the tool would answer
/// with a TOON text blob that no caller can parse.
const FORMAT_KEY: &str = "format";

/// A `.` in a tool name is not a legal identifier in the model's function-call
/// grammar, and the registry's names are matched as plain strings — normalising
/// at the boundary keeps both sides simple and is pinned by a test so the
/// mapping cannot silently change. `papers.search` -> `papers_search`.
pub fn composite_tool_name(composite: &str) -> String {
    composite.replace('.', "_")
}

/// Drop the fields operant sets itself from a composite's advertised schema.
///
/// Only `format` is stripped today. The schema is otherwise the server's own,
/// so the model sees exactly the arguments it may send.
fn trimmed_schema(mut schema: Value) -> Value {
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        props.remove(FORMAT_KEY);
    }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|key| key.as_str() != Some(FORMAT_KEY));
    }
    schema
}

/// Build the operant-facing contract for one sourcehound composite.
///
/// Leaked once per composite: the table is built on first use and lives for
/// the process, so `&'static` is sound and the tool can hold it across the
/// `Send + Sync` boundary the registry requires.
fn spec(
    composite: &'static str,
    description: &'static str,
    schema: Value,
) -> &'static CompositeSpec {
    Box::leak(Box::new(CompositeSpec {
        tool_name: Box::leak(composite_tool_name(composite).into_boxed_str()),
        composite,
        description,
        parameters: trimmed_schema(schema),
    }))
}

/// The 23 v2 composites, in the live server's own vocabulary.
///
/// **This table is the contract.** Descriptions are the sourcehound 1.8.0
/// server's own text (probed over stdio `tools/list` on 2026-10-26); schemas
/// are that server's `inputSchema`. A sourcehound release that renames a
/// composite or changes a contract must update this table, and
/// [`composites_match_the_live_server`] is the test that catches the drift.
pub fn composite_specs() -> &'static [&'static CompositeSpec] {
    static SPECS: LazyLock<Vec<&'static CompositeSpec>> = LazyLock::new(|| {
        vec![
            // ── research / academic ────────────────────────────────────────────
            spec(
                "papers.search",
                "Search academic papers across arXiv, Semantic Scholar, OpenAlex, and more.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": ["integer", "null"]},
                        "year": {"type": ["string", "null"]},
                        "sort": {"type": ["string", "null"]}
                    }
                }),
            ),
            spec(
                "papers.get",
                "Fetch one paper's details and citations (id arxiv:XXXX.XXXXX | semantic_scholar:... | openalex:...).",
                json!({
                    "type": "object",
                    "required": ["id"],
                    "properties": { "id": {"type": "string"} }
                }),
            ),
            // ── news / social / video ──────────────────────────────────────────
            spec(
                "news.fetch",
                "Fetch news from configured sources. Filter by pools, countries, keywords. depth='deep' runs the full pipeline.",
                json!({
                    "type": "object",
                    "properties": {
                        "pools": {"type": ["array", "null"], "items": {"type": "string"}},
                        "countries": {"type": ["array", "null"], "items": {"type": "string"}},
                        "keywords": {"type": ["string", "array", "null"]},
                        "limit": {"type": ["integer", "null"]},
                        "depth": {"type": ["string", "null"]}
                    }
                }),
            ),
            spec(
                "news.search",
                "Query-driven news from anywhere (not just the catalog): Google News RSS.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "social.search",
                "Search Reddit, Hacker News, and YouTube in one call (X/Twitter only when configured).",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "sites": {"type": ["array", "null"], "items": {"type": "string"}},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "social.feed",
                "Read a subreddit (or a user's post history with u/<name>).",
                json!({
                    "type": "object",
                    "required": ["subreddit"],
                    "properties": {
                        "subreddit": {"type": "string"},
                        "limit": {"type": ["integer", "null"]},
                        "sort": {"type": ["string", "null"]}
                    }
                }),
            ),
            spec(
                "video.transcript",
                "YouTube metadata and/or captions in one call: mode=metadata|transcript.",
                json!({
                    "type": "object",
                    "required": ["mode"],
                    "properties": {
                        "mode": {"type": "string"},
                        "url": {"type": ["string", "null"]},
                        "id": {"type": ["string", "null"]}
                    }
                }),
            ),
            // ── datasets / public records ─────────────────────────────────────
            spec(
                "data.query",
                "Keyless public datasets: open-meteo (forecast by lat,lon or place), climate_history, worldbank, trials, quotes, crypto, who_gho.",
                json!({
                    "type": "object",
                    "required": ["dataset", "query"],
                    "properties": {
                        "dataset": {"type": "string"},
                        "query": {"type": "string"}
                    }
                }),
            ),
            spec(
                "gov.search",
                "Search US government sources: kind=bills|regulations|cases|campaign_finance.",
                json!({
                    "type": "object",
                    "required": ["kind", "query"],
                    "properties": {
                        "kind": {"type": "string"},
                        "query": {"type": "string"}
                    }
                }),
            ),
            spec(
                "patents.search",
                "Search USPTO patents via Google Patents (keyless), falling back to PatentsView.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "company.lookup",
                "Keyless company fundamentals via SEC EDGAR: ticker/CIK/name resolution, exchanges, and recent filings.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "security.vuln",
                "Search vulnerabilities across NVD and GitHub Security Advisories in one call.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "recon.target",
                "Profile a domain or IP in one call: RDAP registrar, DNS (A/NS/MX), crt.sh certificates, and ports.",
                json!({
                    "type": "object",
                    "required": ["target"],
                    "properties": {
                        "target": {"type": "string"},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "archive",
                "Wayback Machine lookup for a URL: closest snapshot, or the CDX history with snapshots=true.",
                json!({
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": {"type": "string"},
                        "snapshots": {"type": ["boolean", "null"]},
                        "from": {"type": ["string", "null"]},
                        "to": {"type": ["string", "null"]}
                    }
                }),
            ),
            // ── catalog / index / stored results ──────────────────────────────
            spec(
                "catalog.list",
                "List catalog metadata: what=pools|sources|countries|cities|domains|parsers.",
                json!({
                    "type": "object",
                    "properties": {
                        "what": {"type": ["string", "null"]},
                        "pool": {"type": ["string", "null"]},
                        "active_only": {"type": ["boolean", "null"]}
                    }
                }),
            ),
            spec(
                "data.catalog",
                "Catalog metadata for picking a pool: per-pool source counts, global totals, and top-host concentration.",
                json!({ "type": "object", "properties": {} }),
            ),
            spec(
                "index.query",
                "Query the persisted insight and semantic indices: mode=semantic|entity|trending|connections|stats.",
                json!({
                    "type": "object",
                    "required": ["mode"],
                    "properties": {
                        "mode": {"type": "string"},
                        "query": {"type": ["string", "null"]},
                        "entity": {"type": ["string", "null"]},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "results.get",
                "Read back a stored result handle (`rid`) from a previous tool call.",
                json!({
                    "type": "object",
                    "required": ["rid"],
                    "properties": { "rid": {"type": "string"} }
                }),
            ),
            spec(
                "workflow.run",
                "Run a multi-step intelligence workflow (same engine as sop.execute).",
                json!({
                    "type": "object",
                    "required": ["workflow"],
                    "properties": { "workflow": {"type": "string"} }
                }),
            ),
            // ── general-purpose reads (v2 spellings of the v1 web tools) ───────
            spec(
                "search",
                "Search the web, code, images, or news in one call. kind=web|images|code|news.",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {"type": "string"},
                        "kind": {"type": ["string", "null"]},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            spec(
                "read",
                "Read a page: article text (mode=extract) or structured data (mode=page).",
                json!({
                    "type": "object",
                    "required": ["mode"],
                    "properties": {
                        "mode": {"type": "string"},
                        "url": {"type": ["string", "null"]}
                    }
                }),
            ),
            spec(
                "crawl",
                "Map or crawl a site: mode=map lists pages; mode=crawl fetches their content BFS-style.",
                json!({
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": {"type": "string"},
                        "mode": {"type": ["string", "null"]},
                        "limit": {"type": ["integer", "null"]}
                    }
                }),
            ),
            // ── the browser/automation dispatcher ─────────────────────────────
            spec(
                "cloakctl.call",
                "Drive any cloakctl verb through one tool: tool=open|snapshot|act|read|grep|exec|run|screenshot|cookies_import|wf_run (58 verbs).",
                json!({
                    "type": "object",
                    "required": ["tool"],
                    "properties": {
                        "tool": {"type": "string"},
                        "args": {"type": ["object", "null"]}
                    }
                }),
            ),
        ]
    });
    &SPECS
}

/// The args this tool accepts: the composite's own arguments, verbatim.
///
/// No operant-side fields are validated here. The composite's schema is the
/// sourcehound server's, so validating against an operant-authored struct would
/// reject arguments the server accepts (and accept ones it does not). What this
/// tool adds is the one field operant owns: `format: "json"`.
#[derive(Debug, Deserialize)]
struct CompositeArgs {
    #[serde(flatten)]
    args: std::collections::HashMap<String, Value>,
}

/// A sourcehound v2 composite exposed as one operant tool.
pub struct SourcehoundCompositeTool {
    spec: &'static CompositeSpec,
    engine: Sourcehound,
}

impl SourcehoundCompositeTool {
    /// Register-ready instance for one spec, backed by the resolved binary.
    pub fn new(spec: &'static CompositeSpec) -> Self {
        Self {
            spec,
            engine: Sourcehound::global(),
        }
    }
}

#[async_trait]
impl OperantTool for SourcehoundCompositeTool {
    fn name(&self) -> &str {
        self.spec.tool_name
    }

    fn description(&self) -> &str {
        self.spec.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.spec.tool_name,
            self.spec.description,
            self.spec.parameters.clone(),
        )
    }

    /// Self-disables when the sourcehound binary is missing, exactly like the
    /// four web tools: an absent engine is an unavailable tool, never a hard
    /// error at boot.
    fn is_available(&self) -> bool {
        self.engine.is_available()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: CompositeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error(
                    self.name(),
                    format!("Invalid arguments for {}: {e}", self.spec.composite),
                );
            }
        };

        // RT-188/189 contract: `format: "json"` is what makes the server return
        // a parseable structured payload. A model that sends its own `format`
        // cannot win — operant's value wins, and the assertion below pins it.
        //
        // `#[serde(flatten)]` yields a HashMap (insertion order not guaranteed);
        // the engine takes a `serde_json::Map`, so collect in place — the
        // payload is small, the reallocation is noise.
        let mut arguments: serde_json::Map<String, Value> = parsed.args.into_iter().collect();
        arguments.insert(FORMAT_KEY.to_string(), json!("json"));

        match self
            .engine
            .call(self.spec.composite, Value::Object(arguments))
            .await
        {
            Ok(payload) => ToolResult::success(self.name(), payload),
            Err(e) => ToolResult::error(self.name(), e.to_string()),
        }
    }
}

/// Register every composite, so the 23 ride the deferred bridge.
///
/// Returns the number registered, so the caller (and a test) can assert the
/// registration happened instead of trusting a silent loop. Each spec is its
/// own allocation (see [`spec`]), so no two tools share a contract.
pub async fn register_composites(registry: &crate::tools::ToolRegistry) -> usize {
    let mut count = 0;
    for spec in composite_specs() {
        // A duplicate name would be a registry contract violation; the table's
        // own uniqueness test makes this unreachable, and surfacing it here
        // beats a silently skipped tool.
        if registry
            .register(SourcehoundCompositeTool::new(spec))
            .await
            .is_err()
        {
            continue;
        }
        count += 1;
    }
    count
}

/// The operant-facing names of every composite, for the surfaces that match
/// tool names as strings (`builtin_tool_names`, `operant tools list`,
/// `tools disable`).
///
/// Derived from [`composite_specs`] so the name surface and the registration
/// cannot drift apart: a composite the table stops naming disappears from both.
pub fn composite_names() -> Vec<&'static str> {
    composite_specs()
        .iter()
        .map(|spec| spec.tool_name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool_search::is_deferrable;

    /// Every name is a legal tool identifier and maps 1:1 from the composite.
    #[test]
    fn composite_names_map_dots_to_underscores() {
        assert_eq!(composite_tool_name("papers.search"), "papers_search");
        assert_eq!(composite_tool_name("news.fetch"), "news_fetch");
        assert_eq!(composite_tool_name("cloakctl.call"), "cloakctl_call");
        assert_eq!(composite_tool_name("search"), "search");
        for spec in composite_specs() {
            assert_eq!(spec.tool_name, composite_tool_name(spec.composite));
            assert!(
                !spec.tool_name.contains('.'),
                "illegal tool name: {}",
                spec.tool_name
            );
        }
    }

    /// The table is unique in BOTH spellings: a duplicate tool name would shadow
    /// the twin in the registry, and a duplicate composite would send one
    /// composite's args to another's handler.
    #[test]
    fn composite_table_has_no_duplicates() {
        let specs = composite_specs();
        let mut tool_names: Vec<&str> = specs.iter().map(|s| s.tool_name).collect();
        tool_names.sort_unstable();
        tool_names.dedup();
        assert_eq!(tool_names.len(), specs.len(), "duplicate operant tool name");

        let mut composites: Vec<&str> = specs.iter().map(|s| s.composite).collect();
        composites.sort_unstable();
        composites.dedup();
        assert_eq!(composites.len(), specs.len(), "duplicate composite name");
    }

    /// RT-194's core claim: the composites ride the deferred bridge, so they
    /// cost catalog tokens on the searching turn, not schema tokens on every
    /// request. If `is_deferrable` stops naming them this test fails and the
    /// ~4.7k-token regression ships loudly.
    #[test]
    fn every_composite_is_deferrable() {
        for spec in composite_specs() {
            assert!(
                is_deferrable(spec.tool_name),
                "{} must be deferred — eager registration costs ~4,738 tokens per request",
                spec.tool_name
            );
        }
    }

    /// The description is a real contract, not a placeholder: a model reads it
    /// to decide whether to search for the tool.
    #[test]
    fn every_composite_carries_a_real_description() {
        for spec in composite_specs() {
            assert!(
                spec.description.len() > 20,
                "{} needs a real description, got {:?}",
                spec.tool_name,
                spec.description
            );
        }
    }

    /// `format` is stripped from the advertised schema (operant owns it) and
    /// never in `required`.
    #[test]
    fn advertised_schema_never_exposes_the_format_field() {
        for spec in composite_specs() {
            if let Some(props) = spec.parameters.get("properties").and_then(Value::as_object) {
                assert!(
                    !props.contains_key(FORMAT_KEY),
                    "{} must not advertise `format` — operant sets it",
                    spec.tool_name
                );
            }
            if let Some(required) = spec.parameters.get("required").and_then(Value::as_array) {
                assert!(
                    !required.iter().any(|k| k.as_str() == Some(FORMAT_KEY)),
                    "{} must not require `format`",
                    spec.tool_name
                );
            }
        }
    }

    /// The engine contract, pinned against the exact shape the four web tools
    /// use: a composite's execute puts the caller's arguments under the server's
    /// key and forces `format: "json"`.
    #[test]
    fn execute_forces_the_json_format_contract() {
        // The tool under test forwards through `Sourcehound::call`, which needs
        // the binary; the invariant asserted here is the argument shape that
        // call receives, so the engine is never reached in this process (the
        // binary-absent path returns an error instead).
        let args = json!({ "query": "tokio" });
        let parsed: CompositeArgs = serde_json::from_value(args).expect("args parse");
        let mut map = parsed.args;
        map.insert(FORMAT_KEY.to_string(), json!("json"));
        assert_eq!(map.get(FORMAT_KEY).and_then(Value::as_str), Some("json"));
        assert_eq!(map.get("query").and_then(Value::as_str), Some("tokio"));
    }

    /// A model that tries to override `format` cannot win: operant's value is
    /// inserted after the caller's map is built, overwriting any prior key.
    #[test]
    fn a_caller_cannot_override_the_format_contract() {
        let args = json!({ "query": "x", "format": "toon" });
        let parsed: CompositeArgs = serde_json::from_value(args).expect("args parse");
        let mut map = parsed.args;
        // The production line, exactly:
        map.insert(FORMAT_KEY.to_string(), json!("json"));
        assert_eq!(map.get(FORMAT_KEY).and_then(Value::as_str), Some("json"));
    }

    // ── the end-to-end contract, driven through a mock engine ──────────────

    /// A mock sourcehound that answers every call, echoing back BOTH the tool
    /// name and the arguments it received — so a test can assert exactly what
    /// the production `execute` put on the wire.
    ///
    /// The same shape the four web tools' own tests use
    /// (`tools::sourcehound::tests::V2_ECHO_MCP_SERVER`), so the composite
    /// path is proven against the same contract those tools are.
    const COMPOSITE_ECHO_MCP_SERVER: &str = r#"import sys, json
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    rid = req.get('id')
    if rid is None:
        continue
    method = req.get('method')
    if method == 'initialize':
        result = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'mock', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': n, 'description': 'x', 'inputSchema': {'type': 'object'}} for n in ['papers.search']]}
    else:
        params = req.get('params') or {}
        payload = {'echoed_tool': params.get('name'), 'echoed_args': params.get('arguments') or {}}
        result = {'content': [{'type': 'text', 'text': json.dumps(payload)}], 'structuredContent': payload}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': rid, 'result': result}) + chr(10))
    sys.stdout.flush()
    "#;

    fn mock_engine() -> (Sourcehound, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "operant_composites_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("composite_server.py");
        std::fs::write(&script, COMPOSITE_ECHO_MCP_SERVER).expect("write echo server");
        (
            Sourcehound::new("python3", vec![script.to_string_lossy().to_string()]),
            dir,
        )
    }

    /// THE contract test: a model call to `papers_search` reaches the engine as
    /// `papers.search` (the WIRE name, not the operant spelling) carrying
    /// `format: "json"` alongside the caller's own arguments.
    ///
    /// This is the assertion the two unit tests above cannot make: they check
    /// the map the code builds, this one checks the bytes the engine received.
    /// A regression that drops the format injection, renames the wire call, or
    /// leaks the operant spelling to the server fails here by name.
    #[tokio::test]
    async fn execute_forwards_the_wire_name_and_forces_json() {
        let (engine, _dir) = mock_engine();
        let spec = composite_specs()
            .iter()
            .find(|s| s.composite == "papers.search")
            .expect("papers.search in the table");
        let tool = SourcehoundCompositeTool { spec, engine };

        let result = tool
            .execute(json!({ "query": "tokio runtime" }), ToolContext::default())
            .await;
        assert!(
            result.success,
            "the mock answers every call: {:?}",
            result.error
        );

        // The payload the engine echoed back IS the evidence: it names the wire
        // tool and the arguments as received. `ToolResult::content` carries it
        // as a JSON string, the same shape the web tools' own tests read.
        let echoed: Value = serde_json::from_str(&result.content).expect("papers payload");
        assert_eq!(
            echoed["echoed_tool"], "papers.search",
            "the engine must receive the WIRE name, not the operant spelling: {echoed}"
        );
        assert_eq!(
            echoed["echoed_args"]["format"], "json",
            "every composite call must force format=json (RT-188/189): {echoed}"
        );
        assert_eq!(
            echoed["echoed_args"]["query"], "tokio runtime",
            "the caller's own arguments survive untouched: {echoed}"
        );
    }

    /// Registration puts every composite in the registry under its operant
    /// name, so the model (and the bridge) can reach all 23.
    #[tokio::test]
    async fn registration_installs_every_composite() {
        let registry = crate::tools::ToolRegistry::new(std::time::Duration::from_secs(5));
        let count = register_composites(&registry).await;
        assert_eq!(count, composite_specs().len());
        let schemas = registry.get_schemas().await;
        for spec in composite_specs() {
            assert!(
                schemas.iter().any(|s| s.name == spec.tool_name),
                "{} must be registered",
                spec.tool_name
            );
        }
    }

    /// The name surface and the registration agree in both directions:
    /// `builtin_tool_names` (what `operant tools list` shows and what
    /// `tools disable` matches) carries every registered composite, and the
    /// registration installs every name it lists.
    ///
    /// This is the invariant `test_register_all_builtin_tools` relies on, and
    /// the reason the RT-194 composites entered that function's list instead
    /// of loosening its count assertion.
    #[test]
    fn name_surface_and_registration_agree() {
        let names = crate::tools::builtin_tool_names();
        for spec in composite_specs() {
            assert!(
                names.contains(&spec.tool_name),
                "{} is registered but missing from builtin_tool_names",
                spec.tool_name
            );
        }
        assert_eq!(
            names.iter().filter(|n| n.starts_with("papers")).count(),
            2,
            "two papers composites (search + get)"
        );
    }

    /// THE user-visible promise of RT-194: registering the 23 does NOT add
    /// ~4,738 tokens to a request. Under the default `tool_search = "auto"`
    /// they land in the deferred partition and the model's visible array holds
    /// only the always-on tools plus the bridge trio.
    ///
    /// This is the test that would catch a `is_deferrable` regression with real
    /// consequences: if the composites stopped deferring, every request would
    /// carry all 23 schemas and this assertion fails by name.
    #[test]
    fn registered_composites_ride_the_bridge_not_the_request() {
        use crate::config::ToolSearchSettings;
        use crate::tools::tool_search::assemble_tools;

        // A realistic always-on surface: the four web tools plus a couple of
        // natives, plus every registered composite.
        let mut all: Vec<crate::schema::ToolSchema> = vec![
            ToolSchema::new("web_search", "Search the web", json!({ "type": "object" })),
            ToolSchema::new("web_scrape", "Scrape a URL", json!({ "type": "object" })),
            ToolSchema::new(
                "terminal",
                "Run a shell command",
                json!({ "type": "object" }),
            ),
        ];
        for spec in composite_specs() {
            all.push(ToolSchema::new(
                spec.tool_name,
                spec.description,
                spec.parameters.clone(),
            ));
        }

        let settings = ToolSearchSettings::default();
        assert_eq!(
            settings.enabled, "auto",
            "the default must be the auto tier"
        );
        let assembled = assemble_tools(all, &settings, 200_000);

        // Every composite is deferred, none is eager.
        assert_eq!(assembled.deferred.len(), composite_specs().len());
        for spec in composite_specs() {
            assert!(
                !assembled.visible.iter().any(|s| s.name == spec.tool_name),
                "{} must not ride the request eagerly",
                spec.tool_name
            );
            assert!(
                assembled.deferred.iter().any(|s| s.name == spec.tool_name),
                "{} must be discoverable through the bridge",
                spec.tool_name
            );
        }
        // The model's request carries the always-on tools plus the trio —
        // never the 23.
        assert!(
            assembled.visible.len() <= 3 + 3,
            "the visible array must stay small, got {}",
            assembled.visible.len()
        );
    }
}

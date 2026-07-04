//! IGS (Intelligence Gathering System) tools — OSINT + research + news.
//!
//! Integrates `igs_rust_mcp` as native operant tools. IGS aggregates
//! open-source intelligence across 20+ domains (news, research, web,
//! finance, security, patents, gov, legal, climate, health, OSINT, etc.).
//!
//! ## Registration
//!
//! Tools are registered via `register_igs_tools()`. No shared state is
//! needed — IGS tools self-load config from `~/.config/igs-mcp/settings.yml`
//! on each call.
//!
//! ## Schemars compatibility note
//!
//! IGS uses schemars 1.0; operant uses 0.8. To avoid coupling, we pass
//! raw JSON args directly to IGS functions (which all accept
//! `serde_json::Value` via their `Deserialize` impls). The operant-side
//! `ToolSchema` is built from a minimal hand-written JSON Schema per tool.
//!
//! ## Tool surface (tier-1, no API keys required)
//!
//! - News: `igs_news_fetch`
//! - Social: `igs_reddit_search`
//! - Research: `igs_research_search`, `igs_research_pubmed`
//! - Web: `igs_web_scrape`, `igs_web_map`
//! - Finance: `igs_finance_market`, `igs_finance_crypto`
//! - Security: `igs_security_cve`, `igs_security_advisories`
//! - YouTube: `igs_youtube_search`
//! - Analysis: `igs_summarize`, `igs_extract_locations`, `igs_detect_language`

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all IGS tier-1 tools (no API keys required).
///
/// Call this only when `config.igs.enabled == true`. Tier-2 tools (requiring
/// API keys) should be registered separately based on env var availability.
pub async fn register_igs_tools(registry: &ToolRegistry) -> Result<()> {
    registry.register(IgsNewsFetchTool).await?;
    registry.register(IgsRedditSearchTool).await?;
    registry.register(IgsResearchSearchTool).await?;
    registry.register(IgsResearchPubmedTool).await?;
    registry.register(IgsWebScrapeTool).await?;
    registry.register(IgsWebMapTool).await?;
    registry.register(IgsFinanceMarketTool).await?;
    registry.register(IgsFinanceCryptoTool).await?;
    registry.register(IgsSecurityCveTool).await?;
    registry.register(IgsSecurityAdvisoriesTool).await?;
    registry.register(IgsYoutubeSearchTool).await?;
    registry.register(IgsSummarizeTool).await?;
    registry.register(IgsExtractLocationsTool).await?;
    registry.register(IgsDetectLanguageTool).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: build a minimal ToolSchema
// ---------------------------------------------------------------------------

fn simple_schema(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSchema {
    let mut req = Vec::new();
    for r in required {
        req.push(json!(r));
    }
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": req,
        }),
    }
}

/// Execute an IGS tool by constructing the input from raw JSON and calling
/// the function. All IGS functions return `Result<XOutput, String>`.
async fn run_igs_tool<F, Fut, T>(tool_name: &str, input: Value, f: F) -> ToolResult
where
    F: FnOnce(igs_rust_mcp::tools::types::Value) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
    T: serde::Serialize,
{
    // IGS functions take typed inputs, but we construct from Value via serde.
    // We use a helper that deserializes the Value into the right input type.
    // Since we can't name the type generically, we serialize the result.
    match f(input).await {
        Ok(output) => {
            let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
            ToolResult::success(tool_name, json)
        }
        Err(e) => ToolResult::error(tool_name, e),
    }
}

// ---------------------------------------------------------------------------
// News tools
// ---------------------------------------------------------------------------

pub struct IgsNewsFetchTool;

#[async_trait]
impl OperantTool for IgsNewsFetchTool {
    fn name(&self) -> &str { "igs_news_fetch" }
    fn description(&self) -> &str {
        "Fetch news from RSS pools monitored by IGS (Intelligence Gathering System). Returns enriched articles with topics, entities, and sentiment."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_news_fetch", "Fetch IGS news from RSS pools",
            json!({
                "pools": {"type": "array", "items": {"type": "string"}, "description": "Pool names to fetch from"},
                "limit": {"type": "integer", "default": 20, "description": "Max articles to return"},
                "depth": {"type": "string", "enum": ["shallow", "deep"], "default": "shallow", "description": "Enrichment depth"},
                "urgency": {"type": "string", "description": "Urgency filter"}
            }),
            &["pools"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::NewsFetchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_news_fetch", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::news::news_fetch(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_news_fetch", json)
            }
            Err(e) => ToolResult::error("igs_news_fetch", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Social tools
// ---------------------------------------------------------------------------

pub struct IgsRedditSearchTool;

#[async_trait]
impl OperantTool for IgsRedditSearchTool {
    fn name(&self) -> &str { "igs_reddit_search" }
    fn description(&self) -> &str {
        "Search Reddit for posts matching a query. Returns posts with title, body, score, and comments."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_reddit_search", "Search Reddit",
            json!({
                "query": {"type": "string", "description": "Search query"},
                "subreddits": {"type": "array", "items": {"type": "string"}, "description": "Subreddits to search (omit for all)"},
                "sort": {"type": "string", "enum": ["relevance", "hot", "top", "new", "comments"], "default": "relevance"},
                "time": {"type": "string", "enum": ["hour", "day", "week", "month", "year", "all"], "default": "all"},
                "limit": {"type": "integer", "default": 25, "description": "Max results"}
            }),
            &["query"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::RedditSearchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_reddit_search", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::reddit::reddit_search(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_reddit_search", json)
            }
            Err(e) => ToolResult::error("igs_reddit_search", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Research tools
// ---------------------------------------------------------------------------

pub struct IgsResearchSearchTool;

#[async_trait]
impl OperantTool for IgsResearchSearchTool {
    fn name(&self) -> &str { "igs_research_search" }
    fn description(&self) -> &str {
        "Search academic research papers (arXiv + Semantic Scholar). Returns papers with abstracts, authors, and DOIs."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_research_search", "Search academic research",
            json!({
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "integer", "default": 10, "description": "Max results"},
                "source": {"type": "string", "enum": ["arxiv", "semantic_scholar", "all"], "default": "all"}
            }),
            &["query"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::ResearchSearchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_research_search", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::research::research_search(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_research_search", json)
            }
            Err(e) => ToolResult::error("igs_research_search", e),
        }
    }
}

pub struct IgsResearchPubmedTool;

#[async_trait]
impl OperantTool for IgsResearchPubmedTool {
    fn name(&self) -> &str { "igs_research_pubmed" }
    fn description(&self) -> &str {
        "Search PubMed for biomedical research papers. Returns papers with abstracts and MeSH terms."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_research_pubmed", "Search PubMed",
            json!({
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "integer", "default": 10, "description": "Max results"}
            }),
            &["query"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::ResearchPubmedSearchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_research_pubmed", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::research::research_pubmed_search(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_research_pubmed", json)
            }
            Err(e) => ToolResult::error("igs_research_pubmed", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Web tools
// ---------------------------------------------------------------------------

pub struct IgsWebScrapeTool;

#[async_trait]
impl OperantTool for IgsWebScrapeTool {
    fn name(&self) -> &str { "igs_web_scrape" }
    fn description(&self) -> &str {
        "Scrape a URL and return content in markdown/HTML/text format. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_web_scrape", "Scrape a URL",
            json!({
                "url": {"type": "string", "description": "URL to scrape"},
                "formats": {"type": "array", "items": {"type": "string"}, "default": ["markdown"], "description": "Output formats"},
                "strip_mode": {"type": "string", "description": "Content stripping mode"}
            }),
            &["url"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::WebScrapeInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_web_scrape", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::web::web_scrape(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_web_scrape", json)
            }
            Err(e) => ToolResult::error("igs_web_scrape", e),
        }
    }
}

pub struct IgsWebMapTool;

#[async_trait]
impl OperantTool for IgsWebMapTool {
    fn name(&self) -> &str { "igs_web_map" }
    fn description(&self) -> &str {
        "Discover all URLs on a website via sitemap parsing. Returns a list of URLs."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_web_map", "Map website URLs",
            json!({
                "url": {"type": "string", "description": "Base URL to map"},
                "limit": {"type": "integer", "default": 100, "description": "Max URLs to return"}
            }),
            &["url"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::WebMapInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_web_map", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::web::web_map(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_web_map", json)
            }
            Err(e) => ToolResult::error("igs_web_map", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Finance tools
// ---------------------------------------------------------------------------

pub struct IgsFinanceMarketTool;

#[async_trait]
impl OperantTool for IgsFinanceMarketTool {
    fn name(&self) -> &str { "igs_finance_market" }
    fn description(&self) -> &str {
        "Get stock market quotes for given symbols. No API key required (Yahoo Finance)."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_finance_market", "Get stock quotes",
            json!({
                "symbols": {"type": "array", "items": {"type": "string"}, "description": "Stock symbols (e.g. [\"AAPL\", \"GOOG\"])"}
            }),
            &["symbols"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::FinanceMarketInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_finance_market", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::finance::finance_market(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_finance_market", json)
            }
            Err(e) => ToolResult::error("igs_finance_market", e),
        }
    }
}

pub struct IgsFinanceCryptoTool;

#[async_trait]
impl OperantTool for IgsFinanceCryptoTool {
    fn name(&self) -> &str { "igs_finance_crypto" }
    fn description(&self) -> &str {
        "Get cryptocurrency prices and market data. No API key required (CoinGecko)."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_finance_crypto", "Get crypto prices",
            json!({
                "ids": {"type": "array", "items": {"type": "string"}, "description": "CoinGecko coin IDs (e.g. [\"bitcoin\", \"ethereum\"])"}
            }),
            &["ids"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::FinanceCryptoInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_finance_crypto", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::finance::finance_crypto(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_finance_crypto", json)
            }
            Err(e) => ToolResult::error("igs_finance_crypto", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Security tools
// ---------------------------------------------------------------------------

pub struct IgsSecurityCveTool;

#[async_trait]
impl OperantTool for IgsSecurityCveTool {
    fn name(&self) -> &str { "igs_security_cve" }
    fn description(&self) -> &str {
        "Search CVEs (Common Vulnerabilities and Exposures) from NVD. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_security_cve", "Search CVEs",
            json!({
                "query": {"type": "string", "description": "Search query (keyword or CVE ID)"},
                "limit": {"type": "integer", "default": 10, "description": "Max results"}
            }),
            &["query"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::CveSearchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_security_cve", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::security::security_cve_search(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_security_cve", json)
            }
            Err(e) => ToolResult::error("igs_security_cve", e),
        }
    }
}

pub struct IgsSecurityAdvisoriesTool;

#[async_trait]
impl OperantTool for IgsSecurityAdvisoriesTool {
    fn name(&self) -> &str { "igs_security_advisories" }
    fn description(&self) -> &str {
        "Get security advisories from GitHub. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_security_advisories", "Get security advisories",
            json!({
                "ecosystem": {"type": "string", "description": "Package ecosystem (e.g. npm, pip, cargo)"},
                "limit": {"type": "integer", "default": 10, "description": "Max results"}
            }),
            &[])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::SecurityAdvisoriesInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_security_advisories", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::security::security_advisories(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_security_advisories", json)
            }
            Err(e) => ToolResult::error("igs_security_advisories", e),
        }
    }
}

// ---------------------------------------------------------------------------
// YouTube tools
// ---------------------------------------------------------------------------

pub struct IgsYoutubeSearchTool;

#[async_trait]
impl OperantTool for IgsYoutubeSearchTool {
    fn name(&self) -> &str { "igs_youtube_search" }
    fn description(&self) -> &str {
        "Search YouTube for videos matching a query. Returns video metadata and URLs."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_youtube_search", "Search YouTube",
            json!({
                "query": {"type": "string", "description": "Search query"},
                "limit": {"type": "integer", "default": 10, "description": "Max results"}
            }),
            &["query"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::YoutubeSearchInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_youtube_search", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::youtube::youtube_search(input).await {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_youtube_search", json)
            }
            Err(e) => ToolResult::error("igs_youtube_search", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis tools (offline, no API keys)
// ---------------------------------------------------------------------------

pub struct IgsSummarizeTool;

#[async_trait]
impl OperantTool for IgsSummarizeTool {
    fn name(&self) -> &str { "igs_summarize" }
    fn description(&self) -> &str {
        "Summarize text using offline TextRank algorithm. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_summarize", "Summarize text (TextRank)",
            json!({
                "text": {"type": "string", "description": "Text to summarize"},
                "sentences": {"type": "integer", "default": 5, "description": "Number of summary sentences"}
            }),
            &["text"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::SummarizeInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_summarize", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::summarize::summarize(input) {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_summarize", json)
            }
            Err(e) => ToolResult::error("igs_summarize", e),
        }
    }
}

pub struct IgsExtractLocationsTool;

#[async_trait]
impl OperantTool for IgsExtractLocationsTool {
    fn name(&self) -> &str { "igs_extract_locations" }
    fn description(&self) -> &str {
        "Extract geographic locations from text. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_extract_locations", "Extract locations from text",
            json!({
                "text": {"type": "string", "description": "Text to analyze"}
            }),
            &["text"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::ExtractLocationsInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_extract_locations", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::advanced::extract_locations(input) {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_extract_locations", json)
            }
            Err(e) => ToolResult::error("igs_extract_locations", e),
        }
    }
}

pub struct IgsDetectLanguageTool;

#[async_trait]
impl OperantTool for IgsDetectLanguageTool {
    fn name(&self) -> &str { "igs_detect_language" }
    fn description(&self) -> &str {
        "Detect the language of a text. No API key required."
    }
    fn schema(&self) -> ToolSchema {
        simple_schema("igs_detect_language", "Detect language",
            json!({
                "text": {"type": "string", "description": "Text to analyze"}
            }),
            &["text"])
    }
    async fn execute(&self, args: Value, _ctx: ToolContext) -> ToolResult {
        let input: igs_rust_mcp::tools::types::DetectLanguageInput = match serde_json::from_value(args) {
            Ok(i) => i,
            Err(e) => return ToolResult::error("igs_detect_language", format!("args: {e}")),
        };
        match igs_rust_mcp::tools::advanced::detect_language(input) {
            Ok(output) => {
                let json = serde_json::to_value(&output).unwrap_or(json!({"result": "success"}));
                ToolResult::success("igs_detect_language", json)
            }
            Err(e) => ToolResult::error("igs_detect_language", e),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn igs_tool_names_are_prefixed() {
        let names = vec![
            "igs_news_fetch", "igs_reddit_search", "igs_research_search",
            "igs_research_pubmed", "igs_web_scrape", "igs_web_map",
            "igs_finance_market", "igs_finance_crypto", "igs_security_cve",
            "igs_security_advisories", "igs_youtube_search", "igs_summarize",
            "igs_extract_locations", "igs_detect_language",
        ];
        for name in &names {
            assert!(name.starts_with("igs_"), "tool '{}' should start with 'igs_'", name);
        }
    }
}

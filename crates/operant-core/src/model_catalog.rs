//! Runtime model discovery. There is no compiled-in model list: a provider's
//! models come from its own `/models` endpoint when that is reachable, then
//! from the models.dev catalog, then from the configured models. The chosen
//! source is carried on every row so callers can say where a model came from.

use std::time::Duration;

use serde_json::Value;

/// Wire protocol a provider's models endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFamily {
    /// `GET {base}/models` with a bearer token; lists under `data[].id`.
    OpenAiCompat,
    /// `GET {base}/models` with `x-api-key`; lists under `data[].id`.
    Anthropic,
    /// `GET {origin}/v1beta/models?key=…`; lists under `models[].name`.
    Google,
}

/// Where a catalog row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    Live,
    ModelsDev,
    Configured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRow {
    pub id: String,
    pub source: CatalogSource,
}

/// The resolved catalog. `notice` explains why the live list is missing, so the
/// UI can say so instead of showing a short list with no reason.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub rows: Vec<CatalogRow>,
    pub notice: Option<String>,
}

/// What [`resolve`] needs to know about one provider.
pub struct Request<'a> {
    /// Operant provider name, used for the models.dev lookup (`None` = no mapping).
    pub provider_name: &'a str,
    /// Wire family of the provider's models endpoint (`None` = no live endpoint).
    pub family: Option<WireFamily>,
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
    /// Models already configured (`agent.model`, `fallback_models`).
    pub configured: &'a [String],
}

/// Resolve the catalog: live first; models.dev only when live gave nothing;
/// configured models are always appended (deduplicated).
pub async fn resolve(req: Request<'_>) -> Catalog {
    let mut catalog = Catalog::default();

    if let Some(family) = req.family {
        match fetch_live(family, req.base_url, req.api_key).await {
            Ok(ids) => catalog.rows.extend(ids.into_iter().map(|id| CatalogRow {
                id,
                source: CatalogSource::Live,
            })),
            Err(reason) => catalog.notice = Some(reason),
        }
    }

    // Only mapped providers pay for the catalog fetch; an unmapped one can never match.
    if catalog.rows.is_empty()
        && crate::models_dev::provider_to_models_dev(req.provider_name).is_some()
    {
        for id in crate::models_dev::list_agentic_models(req.provider_name).await {
            catalog.rows.push(CatalogRow {
                id,
                source: CatalogSource::ModelsDev,
            });
        }
    }

    for id in req.configured.iter().filter(|m| !m.trim().is_empty()) {
        if !catalog.rows.iter().any(|r| &r.id == id) {
            catalog.rows.push(CatalogRow {
                id: id.clone(),
                source: CatalogSource::Configured,
            });
        }
    }
    catalog
}

/// Default model for a resolved catalog: the configured model if it is in the
/// catalog, otherwise the first row whose models.dev entry supports tools, otherwise
/// the first row. `None` only when the catalog is empty.
pub fn default_model(
    catalog: &Catalog,
    configured: &str,
    supports_tools: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(row) = catalog.rows.iter().find(|r| r.id == configured) {
        return Some(row.id.clone());
    }
    catalog
        .rows
        .iter()
        .find(|r| supports_tools(&r.id))
        .or_else(|| catalog.rows.first())
        .map(|r| r.id.clone())
}

/// One GET against a provider's models endpoint, as data so the wire shape can be
/// checked without a network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingRequest {
    /// The URL without its query string; this is what error messages name.
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub query: Vec<(&'static str, String)>,
}

/// The listing request for `family` at `base_url`. Errors only when the base URL
/// cannot be parsed (Google and Ollama-style origins are derived from it).
pub fn listing_request(
    family: WireFamily,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<ListingRequest, String> {
    let base = base_url.trim_end_matches('/');
    let key = api_key.unwrap_or_default();
    Ok(match family {
        WireFamily::OpenAiCompat => ListingRequest {
            url: format!("{base}/models"),
            headers: api_key
                .map(|k| vec![("authorization", format!("Bearer {k}"))])
                .unwrap_or_default(),
            query: Vec::new(),
        },
        WireFamily::Anthropic => ListingRequest {
            url: format!("{base}/models"),
            headers: vec![
                ("x-api-key", key.to_string()),
                ("anthropic-version", "2023-06-01".to_string()),
            ],
            query: Vec::new(),
        },
        WireFamily::Google => ListingRequest {
            url: format!("{}/v1beta/models", origin(base)?),
            headers: Vec::new(),
            query: vec![("key", key.to_string())],
        },
    })
}

/// Ask the provider's models endpoint for its model ids.
///
/// The error string is shown to the user, so it names the endpoint without the
/// API key (the Google key travels in the query string and is never echoed).
pub async fn fetch_live(
    family: WireFamily,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let listing = listing_request(family, base_url, api_key)?;
    let mut request = client.get(&listing.url);
    for (name, value) in &listing.headers {
        request = request.header(*name, value);
    }
    if !listing.query.is_empty() {
        request = request.query(&listing.query);
    }
    let shown = listing.url.as_str();

    let resp = request
        .send()
        .await
        .map_err(|e| format!("GET {shown} failed: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!("GET {shown} returned {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("GET {shown} did not return JSON: {e}"))?;

    let ids = parse_ids(family, &body);
    if ids.is_empty() {
        return Err(format!("GET {shown} returned no models"));
    }
    Ok(ids)
}

fn parse_ids(family: WireFamily, body: &Value) -> Vec<String> {
    match family {
        WireFamily::OpenAiCompat | WireFamily::Anthropic => body
            .get("data")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        WireFamily::Google => body
            .get("models")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|m| m.get("name").and_then(Value::as_str))
                    .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// `scheme://host[:port]` of a base URL, dropping any path (`/v1`, `/v1beta/openai`).
fn origin(base: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(base).map_err(|e| format!("invalid base URL {base}: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("base URL has no host: {base}"))?;
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(ids: &[&str]) -> Catalog {
        Catalog {
            rows: ids
                .iter()
                .map(|id| CatalogRow {
                    id: id.to_string(),
                    source: CatalogSource::Live,
                })
                .collect(),
            notice: None,
        }
    }

    #[test]
    fn openai_compat_lists_under_data() {
        let body = serde_json::json!({"data": [{"id": "gpt-x"}, {"id": "gpt-y"}]});
        assert_eq!(
            parse_ids(WireFamily::OpenAiCompat, &body),
            ["gpt-x", "gpt-y"]
        );
    }

    #[test]
    fn google_strips_the_models_prefix() {
        let body = serde_json::json!({"models": [{"name": "models/gemini-2.5-flash"}]});
        assert_eq!(parse_ids(WireFamily::Google, &body), ["gemini-2.5-flash"]);
    }

    #[tokio::test]
    async fn anthropic_sends_the_key_as_x_api_key() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/models")
            .match_header("x-api-key", "sk-test")
            .match_header("anthropic-version", "2023-06-01")
            .with_body(r#"{"data":[{"id":"claude-a"}]}"#)
            .create_async()
            .await;
        let ids = fetch_live(
            WireFamily::Anthropic,
            &format!("{}/v1", server.url()),
            Some("sk-test"),
        )
        .await
        .unwrap();
        assert_eq!(ids, ["claude-a"]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn google_sends_the_key_in_the_query_and_strips_the_prefix() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1beta/models")
            .match_query(mockito::Matcher::UrlEncoded("key".into(), "k1".into()))
            .with_body(r#"{"models":[{"name":"models/gemini-x"}]}"#)
            .create_async()
            .await;
        let base = format!("{}/v1beta/openai", server.url());
        let ids = fetch_live(WireFamily::Google, &base, Some("k1"))
            .await
            .unwrap();
        assert_eq!(ids, ["gemini-x"]);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn an_http_error_names_the_endpoint_and_status() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/models")
            .with_status(401)
            .create_async()
            .await;
        let err = fetch_live(
            WireFamily::OpenAiCompat,
            &format!("{}/v1", server.url()),
            Some("bad"),
        )
        .await
        .unwrap_err();
        assert!(err.contains("/v1/models returned 401"), "{err}");
    }

    #[test]
    fn listing_requests_put_the_key_where_each_family_expects_it() {
        let bearer =
            listing_request(WireFamily::OpenAiCompat, "https://h.example/v1/", Some("k")).unwrap();
        assert_eq!(bearer.url, "https://h.example/v1/models");
        assert_eq!(bearer.headers, [("authorization", "Bearer k".to_string())]);
        assert!(bearer.query.is_empty());

        let anthropic =
            listing_request(WireFamily::Anthropic, "https://api.example/v1", Some("k")).unwrap();
        assert_eq!(anthropic.url, "https://api.example/v1/models");
        assert!(anthropic.headers.contains(&("x-api-key", "k".to_string())));
        assert!(
            anthropic
                .headers
                .contains(&("anthropic-version", "2023-06-01".to_string()))
        );

        let google = listing_request(
            WireFamily::Google,
            "https://g.example/v1beta/openai",
            Some("k"),
        )
        .unwrap();
        assert_eq!(google.url, "https://g.example/v1beta/models");
        assert_eq!(google.query, [("key", "k".to_string())]);
        assert!(google.headers.is_empty());
    }

    #[test]
    fn keyless_openai_compatible_listing_sends_no_auth() {
        let local =
            listing_request(WireFamily::OpenAiCompat, "http://localhost:11434/v1", None).unwrap();
        assert_eq!(local.url, "http://localhost:11434/v1/models");
        assert!(local.headers.is_empty());
    }

    #[test]
    fn origin_drops_the_path() {
        assert_eq!(
            origin("http://localhost:11434/v1").unwrap(),
            "http://localhost:11434"
        );
        assert_eq!(
            origin("https://generativelanguage.googleapis.com/v1beta/openai").unwrap(),
            "https://generativelanguage.googleapis.com"
        );
    }

    #[test]
    fn default_prefers_configured_then_tool_capable_then_first() {
        let catalog = rows(&["a", "b", "c"]);
        let tools = |id: &str| id != "a";
        assert_eq!(default_model(&catalog, "c", tools).as_deref(), Some("c"));
        assert_eq!(default_model(&catalog, "gone", tools).as_deref(), Some("b"));
        assert_eq!(
            default_model(&catalog, "gone", |_| false).as_deref(),
            Some("a")
        );
        assert_eq!(default_model(&Catalog::default(), "x", tools), None);
    }

    #[tokio::test]
    async fn unreachable_endpoint_keeps_configured_models_and_says_why() {
        let configured = vec!["mine".to_string()];
        let catalog = resolve(Request {
            provider_name: "no-such-provider",
            family: Some(WireFamily::OpenAiCompat),
            base_url: "http://127.0.0.1:1/v1",
            api_key: Some("k"),
            configured: &configured,
        })
        .await;
        assert_eq!(catalog.rows.len(), 1);
        assert_eq!(catalog.rows[0].source, CatalogSource::Configured);
        assert!(
            catalog
                .notice
                .unwrap()
                .contains("http://127.0.0.1:1/v1/models")
        );
    }
}

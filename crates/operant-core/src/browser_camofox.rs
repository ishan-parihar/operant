//! REST API client for Camofox browser.
//!
//! Camofox-browser is a self-hosted Node.js server wrapping Camoufox (Firefox
//! fork with C++ fingerprint spoofing). It exposes a REST API for browser
//! automation: navigation, clicking, typing, screenshots, and DOM inspection.
//!
//! This module provides a thin async client — one struct, 18 endpoint methods,
//! 30-second default timeout, optional `X-API-Key` header.
//!
//! # Example
//!
//! ```ignore
//! use operant_core::browser_camofox::CamofoxBrowser;
//!
//! let browser = CamofoxBrowser::new("http://localhost:9377", Some("my-api-key"));
//! let result = browser.navigate("https://example.com").await?;
//! let title = browser.get_title().await?;
//! ```

use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Default timeout for each HTTP request (30 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// REST API client for the Camofox browser backend.
///
/// All methods return `anyhow::Result<T>` so callers can use `?` and `.context()`.
/// HTTP errors are converted to `anyhow::Error` with status and body information.
#[derive(Debug, Clone)]
pub struct CamofoxBrowser {
    /// Shared reqwest HTTP client.
    client: reqwest::Client,
    /// Base URL of the Camofox server (e.g. `http://localhost:9377`).
    base_url: String,
    /// Optional API key sent as `X-API-Key` header.
    api_key: Option<String>,
}

impl CamofoxBrowser {
    /// Create a new Camofox browser client.
    ///
    /// * `base_url` — The Camofox server URL (e.g. `"http://localhost:9377"`).
    /// * `api_key` — Optional API key for authenticated endpoints.
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("Failed to create reqwest::Client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.map(|s| s.to_string()),
        }
    }

    // ------------------------------------------------------------------
    // Internal HTTP helper
    // ------------------------------------------------------------------

    /// Send an HTTP request to the Camofox API and deserialize the JSON response.
    ///
    /// * `method`  — HTTP method (GET, POST, DELETE, etc.).
    /// * `endpoint` — Path starting with `/` (e.g. `/navigate`).
    /// * `body`    — Optional JSON body for POST requests.
    async fn request(&self, method: Method, endpoint: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{}", self.base_url, endpoint);
        let mut req = self.client.request(method.clone(), &url);

        // Attach optional API key header
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }

        // Attach JSON body if provided
        if let Some(body_val) = body {
            req = req.json(&body_val);
        }

        let resp = req
            .send()
            .await
            .with_context(|| format!("Failed to send {method} request to {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Camofox request failed with status {}: {}",
                status,
                body_text
            );
        }

        resp.json()
            .await
            .with_context(|| format!("Failed to parse Camofox {method} response from {endpoint}"))
    }

    // ------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------

    /// Navigate to a URL.
    ///
    /// POST `/navigate` with `{ "url": "…" }`.
    pub async fn navigate(&self, url: &str) -> Result<Value> {
        let body = serde_json::json!({ "url": url });
        self.request(Method::POST, "/navigate", Some(body)).await
    }

    /// Navigate back in history.
    ///
    /// POST `/back`.
    pub async fn back(&self) -> Result<Value> {
        self.request(Method::POST, "/back", None).await
    }

    /// Navigate forward in history.
    ///
    /// POST `/forward`.
    pub async fn forward(&self) -> Result<Value> {
        self.request(Method::POST, "/forward", None).await
    }

    /// Refresh the current page.
    ///
    /// POST `/refresh`.
    pub async fn refresh(&self) -> Result<Value> {
        self.request(Method::POST, "/refresh", None).await
    }

    // ------------------------------------------------------------------
    // Element interaction
    // ------------------------------------------------------------------

    /// Click an element identified by a CSS selector.
    ///
    /// POST `/click` with `{ "selector": "…" }`.
    pub async fn click(&self, selector: &str) -> Result<Value> {
        let body = serde_json::json!({ "selector": selector });
        self.request(Method::POST, "/click", Some(body)).await
    }

    /// Type text into an element identified by a CSS selector.
    ///
    /// POST `/type` with `{ "selector": "…", "text": "…" }`.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<Value> {
        let body = serde_json::json!({ "selector": selector, "text": text });
        self.request(Method::POST, "/type", Some(body)).await
    }

    /// Click a link by its visible text.
    ///
    /// POST `/click-link` with `{ "text": "…" }`.
    pub async fn click_link(&self, text: &str) -> Result<Value> {
        let body = serde_json::json!({ "text": text });
        self.request(Method::POST, "/click-link", Some(body)).await
    }

    /// Fill a form with the given data (field name → value).
    ///
    /// POST `/fill-form` with `{ "data": { … } }`.
    pub async fn fill_form(&self, data: HashMap<String, String>) -> Result<Value> {
        let body = serde_json::json!({ "data": data });
        self.request(Method::POST, "/fill-form", Some(body)).await
    }

    /// Select an option from a `<select>` element.
    ///
    /// POST `/select` with `{ "selector": "…", "value": "…" }`.
    pub async fn select_option(&self, selector: &str, value: &str) -> Result<Value> {
        let body = serde_json::json!({ "selector": selector, "value": value });
        self.request(Method::POST, "/select", Some(body)).await
    }

    /// Scroll the page to the specified coordinates.
    ///
    /// POST `/scroll` with `{ "x": …, "y": … }`.
    pub async fn scroll_to(&self, x: i32, y: i32) -> Result<Value> {
        let body = serde_json::json!({ "x": x, "y": y });
        self.request(Method::POST, "/scroll", Some(body)).await
    }

    // ------------------------------------------------------------------
    // Page content
    // ------------------------------------------------------------------

    /// Take a screenshot of the current page.
    ///
    /// POST `/screenshot` — returns a JSON object with a base64-encoded PNG
    /// string (typically under a `"data"` or `"screenshot"` key).
    pub async fn screenshot(&self) -> Result<Value> {
        self.request(Method::POST, "/screenshot", None).await
    }

    /// Get the full page HTML.
    ///
    /// GET `/html` — returns a JSON object with the page HTML string.
    pub async fn get_html(&self) -> Result<Value> {
        self.request(Method::GET, "/html", None).await
    }

    /// Get the visible text content of the page.
    ///
    /// GET `/text` — returns a JSON object with the page text.
    pub async fn get_text(&self) -> Result<Value> {
        self.request(Method::GET, "/text", None).await
    }

    /// Execute arbitrary JavaScript on the page.
    ///
    /// POST `/execute` with `{ "script": "…" }`.
    pub async fn execute_script(&self, script: &str) -> Result<Value> {
        let body = serde_json::json!({ "script": script });
        self.request(Method::POST, "/execute", Some(body)).await
    }

    /// Get the current page title.
    ///
    /// GET `/title` — returns a JSON object with the title string.
    pub async fn get_title(&self) -> Result<Value> {
        self.request(Method::GET, "/title", None).await
    }

    /// Get the current page URL.
    ///
    /// GET `/url` — returns a JSON object with the current URL.
    pub async fn get_url(&self) -> Result<Value> {
        self.request(Method::GET, "/url", None).await
    }

    // ------------------------------------------------------------------
    // Viewport & lifecycle
    // ------------------------------------------------------------------

    /// Set the browser viewport size.
    ///
    /// POST `/viewport` with `{ "width": …, "height": … }`.
    pub async fn set_viewport(&self, width: u32, height: u32) -> Result<Value> {
        let body = serde_json::json!({ "width": width, "height": height });
        self.request(Method::POST, "/viewport", Some(body)).await
    }

    /// Close the browser session.
    ///
    /// POST `/close`.
    pub async fn close(&self) -> Result<Value> {
        self.request(Method::POST, "/close", None).await
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    /// Helper to create a `CamofoxBrowser` pointed at a mockito mock server.
    fn mock_browser(server: &Server) -> CamofoxBrowser {
        CamofoxBrowser::new(&server.url(), None)
    }

    #[tokio::test]
    async fn test_navigate() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/navigate")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"success":true,"url":"https://example.com"}"#)
            .create();

        let browser = mock_browser(&server);
        let result = browser.navigate("https://example.com").await.unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["url"], "https://example.com");
        mock.assert();
    }

    #[tokio::test]
    async fn test_click() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/click")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r##"{"success":true,"selector":"#submit-btn"}"##)
            .create();

        let browser = mock_browser(&server);
        let result = browser.click("#submit-btn").await.unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["selector"], "#submit-btn");
        mock.assert();
    }

    #[tokio::test]
    async fn test_screenshot() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/screenshot")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"data":"iVBORw0KGgoAAAANSUhEUgAAAAE="}"#)
            .create();

        let browser = mock_browser(&server);
        let result = browser.screenshot().await.unwrap();

        assert!(result["data"].as_str().unwrap_or("").len() > 0);
        mock.assert();
    }

    #[tokio::test]
    async fn test_get_title() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/title")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"title":"Example Domain"}"#)
            .create();

        let browser = mock_browser(&server);
        let result = browser.get_title().await.unwrap();

        assert_eq!(result["title"], "Example Domain");
        mock.assert();
    }

    #[tokio::test]
    async fn test_execute_script() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/execute")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":"document.title"}"#)
            .create();

        let browser = mock_browser(&server);
        let result = browser.execute_script("document.title").await.unwrap();

        assert_eq!(result["result"], "document.title");
        mock.assert();
    }

    #[tokio::test]
    async fn test_request_failure() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/title")
            .with_status(500)
            .with_body("Internal Server Error")
            .create();

        let browser = mock_browser(&server);
        let err = browser.get_title().await.unwrap_err();

        assert!(err.to_string().contains("500"));
        mock.assert();
    }

    #[tokio::test]
    async fn test_api_key_header() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/url")
            .match_header("X-API-Key", "test-key-123")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"url":"https://example.com"}"#)
            .create();

        let browser = CamofoxBrowser::new(&server.url(), Some("test-key-123"));
        let result = browser.get_url().await.unwrap();

        assert_eq!(result["url"], "https://example.com");
        mock.assert();
    }

    #[tokio::test]
    async fn test_fill_form() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/fill-form")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"success":true}"#)
            .create();

        let browser = mock_browser(&server);
        let mut data = HashMap::new();
        data.insert("name".to_string(), "Alice".to_string());
        data.insert("email".to_string(), "a@b.com".to_string());
        let result = browser.fill_form(data).await.unwrap();

        assert_eq!(result["success"], true);
        mock.assert();
    }
}

//! The injectable HTTP boundary for every flow in [`crate::oauth`].
//!
//! Flows depend on [`HttpEndpoint`], never on `reqwest` directly, so a test
//! can drive the authorization-code, device-code and refresh paths with canned
//! responses and no live provider.

use std::time::Duration;

use super::{OAuthFlowError, Result, http_failure};

/// HTTP verb, narrowed to what OAuth needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// `GET`
    Get,
    /// `POST` with `application/x-www-form-urlencoded`.
    PostForm,
}

/// A single outbound OAuth request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// Absolute endpoint URL.
    pub url: String,
    /// Verb.
    pub method: HttpMethod,
    /// `application/x-www-form-urlencoded` fields.
    pub form: Vec<(String, String)>,
    /// Extra request headers (e.g. `Accept: application/json`).
    pub headers: Vec<(String, String)>,
}

impl HttpRequest {
    /// `POST` with form fields — the shape every OAuth token request takes.
    pub fn post_form(
        url: impl Into<String>,
        form: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            url: url.into(),
            method: HttpMethod::PostForm,
            form: form.into_iter().collect(),
            headers: vec![("Accept".to_string(), "application/json".to_string())],
        }
    }

    /// `GET`.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: HttpMethod::Get,
            form: Vec::new(),
            headers: Vec::new(),
        }
    }

    /// Add a header, builder style.
    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

/// A response body plus its status.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body, as text.
    pub body: String,
}

impl HttpResponse {
    /// `true` for 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Convert a non-2xx into the right [`OAuthFlowError`], or hand back `self`.
    pub fn into_success(self, url: &str) -> Result<Self> {
        if self.is_success() {
            Ok(self)
        } else {
            Err(http_failure(url, self.status, &self.body))
        }
    }
}

/// The seam every OAuth flow makes its network call through.
///
/// Implementations must be cheap to share: flows hold `&dyn HttpEndpoint`
/// across `await` points, so the trait requires `Send + Sync`.
#[async_trait::async_trait]
pub trait HttpEndpoint: Send + Sync {
    /// Perform one request. Transport failures map to
    /// [`OAuthFlowError::Network`]; a non-2xx is returned as an `Ok`
    /// [`HttpResponse`] so the caller can apply protocol-level mapping
    /// (`authorization_pending`, `slow_down`, …).
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse>;
}

/// The production [`HttpEndpoint`], backed by `reqwest`.
pub struct ReqwestEndpoint {
    client: reqwest::Client,
}

impl ReqwestEndpoint {
    /// Build an endpoint with a 15 s per-request timeout — long enough for a
    /// token exchange, short enough that a hung provider cannot wedge a login.
    pub fn new() -> Result<Self> {
        Self::with_timeout(Duration::from_secs(15))
    }

    /// Build an endpoint with an explicit per-request timeout.
    pub fn with_timeout(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| OAuthFlowError::Network(format!("building HTTP client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait::async_trait]
impl HttpEndpoint for ReqwestEndpoint {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(&request.url),
            HttpMethod::PostForm => self
                .client
                .post(&request.url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .form(&request.form),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let response = builder.send().await.map_err(|e| {
            // The reqwest error string embeds the full URL; run it through the
            // crate redactor so a token in a query string cannot reach a log.
            OAuthFlowError::Network(crate::redaction::redact_sensitive_text_if_enabled(
                &e.to_string(),
            ))
        })?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| OAuthFlowError::Network(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Scripted endpoint: records every request and replays queued responses.
    pub(crate) struct StubEndpoint {
        pub(crate) seen: Mutex<Vec<HttpRequest>>,
        pub(crate) responses: Mutex<Vec<Result<HttpResponse>>>,
    }

    impl StubEndpoint {
        pub(crate) fn new(responses: Vec<Result<HttpResponse>>) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            })
        }

        pub(crate) fn requests(&self) -> Vec<HttpRequest> {
            self.seen.lock().expect("seen lock").clone()
        }

        pub(crate) fn call_count(&self) -> usize {
            self.seen.lock().expect("seen lock").len()
        }
    }

    #[async_trait::async_trait]
    impl HttpEndpoint for StubEndpoint {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
            self.seen.lock().expect("seen lock").push(request);
            let mut queue = self.responses.lock().expect("responses lock");
            if queue.is_empty() {
                return Ok(HttpResponse {
                    status: 500,
                    body: r#"{"error":"stub exhausted"}"#.to_string(),
                });
            }
            queue.remove(0)
        }
    }

    #[test]
    fn response_maps_non_2xx_to_typed_error() {
        let ok = HttpResponse {
            status: 200,
            body: "{}".to_string(),
        };
        assert!(ok.clone().into_success("u").is_ok());

        let pending = HttpResponse {
            status: 400,
            body: r#"{"error":"authorization_pending"}"#.to_string(),
        };
        assert!(matches!(
            pending.into_success("u"),
            Err(OAuthFlowError::Pending)
        ));
    }

    #[tokio::test]
    async fn stub_endpoint_records_requests_and_replays_responses() {
        let stub = StubEndpoint::new(vec![
            Ok(HttpResponse {
                status: 200,
                body: r#"{"access_token":"at"}"#.to_string(),
            }),
            Ok(HttpResponse {
                status: 400,
                body: r#"{"error":"slow_down"}"#.to_string(),
            }),
        ]);

        let first = stub
            .send(HttpRequest::post_form(
                "https://as.test/token",
                [("grant_type".to_string(), "refresh_token".to_string())],
            ))
            .await
            .expect("first");
        assert_eq!(first.status, 200);

        let second = stub
            .send(HttpRequest::get("https://as.test/device"))
            .await
            .expect("second");
        assert_eq!(second.status, 400);

        assert_eq!(stub.call_count(), 2);
        let seen = stub.requests();
        assert_eq!(seen[0].method, HttpMethod::PostForm);
        assert_eq!(seen[0].form[0].1, "refresh_token");
        assert_eq!(seen[1].method, HttpMethod::Get);
    }
}

//! Feishu/Lark Doc and Drive Tools
//!
//! Provides tools to read Feishu / Lark document content and manage Drive file
//! comments.  Uses OAuth2 tenant access tokens with automatic 2-hour caching.
//!
//! # Environment variables
//!
//! | Variable        | Default                       | Required |
//! |-----------------|-------------------------------|----------|
//! | `LARK_APP_ID`   | —                             | yes      |
//! | `LARK_APP_SECRET` | —                           | yes      |
//! | `FEISHU_HOST`   | `https://open.feishu.cn`      | no       |

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

// ===========================================================================
// Auth helpers
// ===========================================================================

/// Cached tenant access token paired with the instant it expires.
static AUTH_CACHE: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();

fn auth_cache() -> &'static Mutex<Option<(String, Instant)>> {
    AUTH_CACHE.get_or_init(|| Mutex::new(None))
}

    #[expect(clippy::expect_used, reason = "poisoned lock: panic is the intended recovery")]
/// Obtain a Feishu/Lark tenant access token.
///
/// Reads `LARK_APP_ID` and `LARK_APP_SECRET` from the environment, fetches a
/// fresh token from the auth endpoint, and caches it for ~2 hours (minus a
/// 5-minute safety margin).
async fn get_tenant_token() -> Result<String, String> {
    // Fast path – cached token still valid.
    {
        let guard = auth_cache()
            .lock()
            .expect("auth_cache mutex poisoned — programmer error");
        if let Some((token, expiry)) = guard.as_ref() {
            if Instant::now() < *expiry {
                return Ok(token.clone());
            }
        }
    }

    // Slow path – fetch a new token.
    let app_id =
        std::env::var("LARK_APP_ID").map_err(|_| "Missing env var: LARK_APP_ID".to_string())?;
    let app_secret = std::env::var("LARK_APP_SECRET")
        .map_err(|_| "Missing env var: LARK_APP_SECRET".to_string())?;
    let base_url =
        std::env::var("FEISHU_HOST").unwrap_or_else(|_| "https://open.feishu.cn".to_string());

    let auth_url = format!("{base_url}/open-apis/auth/v3/tenant_access_token/internal");

    let client = reqwest::Client::new();
    let resp = client
        .post(&auth_url)
        .json(&json!({ "app_id": app_id, "app_secret": app_secret }))
        .send()
        .await
        .map_err(|e| format!("Failed to get Feishu tenant token: {e}"))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Feishu auth response: {e}"))?;

    if !status.is_success() || body.get("code").and_then(|c| c.as_i64()) != Some(0) {
        return Err(format!(
            "Failed to get Feishu tenant token: HTTP {status} code={} msg={}",
            body["code"].as_i64().unwrap_or(-1),
            body["msg"].as_str().unwrap_or("unknown"),
        ));
    }

    let token = body["tenant_access_token"]
        .as_str()
        .ok_or_else(|| "Missing tenant_access_token in Feishu auth response".to_string())?
        .to_string();

    // Token lifetime is 7200 s; we cache for 7000 s for safety.
    let ttl = Duration::from_secs(7000);
    {
        let mut guard = auth_cache()
            .lock()
            .expect("auth_cache mutex poisoned — programmer error");
        *guard = Some((token.clone(), Instant::now() + ttl));
    }

    Ok(token)
}

/// Make an authenticated HTTP request to the Feishu/Lark Open API.
///
/// Adds the `Authorization: Bearer …` header automatically.  Returns the
/// deserialised JSON response on success, or an error string on failure.
async fn feishu_request(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let token = get_tenant_token().await?;
    let base_url =
        std::env::var("FEISHU_HOST").unwrap_or_else(|_| "https://open.feishu.cn".to_string());
    let url = format!("{base_url}{path}");

    let client = reqwest::Client::new();
    let mut builder = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        m => return Err(format!("Unsupported HTTP method: {m}")),
    }
    .header("Authorization", format!("Bearer {token}"));

    if let Some(b) = body {
        builder = builder.json(&b);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("Feishu API request failed: {e}"))?;
    let status = resp.status();
    let resp_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Feishu API response: {e}"))?;

    if status.is_success() && resp_body.get("code").and_then(|c| c.as_i64()) == Some(0) {
        Ok(resp_body)
    } else {
        Err(format!(
            "Feishu API error: HTTP {status} - code={} msg={}",
            resp_body["code"].as_i64().unwrap_or(-1),
            resp_body["msg"].as_str().unwrap_or("unknown"),
        ))
    }
}

// ===========================================================================
// FeishuDocTool – feishu_doc_read
// ===========================================================================

/// Arguments for [`FeishuDocTool`].
#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct FeishuDocArgs {
    /// The Feishu document ID (e.g. the `xxx` in a doc token).
    document_id: String,
}

/// Read the full content of a Feishu / Lark document.
///
/// Calls `GET …/docx/v1/documents/{id}/raw_content` and returns the document
/// body in Feishu's larkdown format (roughly equivalent to Markdown).
pub struct FeishuDocTool;

impl Default for FeishuDocTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperantTool for FeishuDocTool {
    fn name(&self) -> &str {
        "feishu_doc_read"
    }

    fn description(&self) -> &str {
        "Read the content of a Feishu / Lark document by its document ID. \
         Returns the document body as plain text (larkdown format). \
         Requires LARK_APP_ID and LARK_APP_SECRET environment variables."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FeishuDocArgs>(
            "feishu_doc_read",
            "Read Feishu/Lark document content by document ID",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let document_id = match args.get("document_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                return ToolResult::error(
                    "feishu_doc_read",
                    "Missing required argument: document_id",
                );
            }
        };

        let path = format!("/open-apis/docx/v1/documents/{document_id}/raw_content");

        match feishu_request("GET", &path, None).await {
            Ok(resp) => {
                let content = resp["data"]["content"]
                    .as_str()
                    .unwrap_or("(empty document)")
                    .to_string();

                ToolResult::success(
                    "feishu_doc_read",
                    json!({
                        "success": true,
                        "document_id": document_id,
                        "content": content,
                    }),
                )
            }
            Err(e) => ToolResult::error("feishu_doc_read", e),
        }
    }
}

// ===========================================================================
// FeishuDriveTool – feishu_drive
// ===========================================================================

/// Arguments for [`FeishuDriveTool`].
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FeishuDriveArgs {
    /// Action: `list_comments`, `list_comment_replies`, `reply_comment`, or
    /// `add_comment`.
    action: String,
    /// The Drive file token to operate on.
    file_token: String,
    /// Comment ID – required by `list_comment_replies` and `reply_comment`.
    comment_id: Option<String>,
    /// Text content – required by `reply_comment` and `add_comment`.
    content: Option<String>,
    /// Page size for paginated list operations (default 50).
    page_size: Option<u32>,
}

/// Manage comments on Feishu / Lark Drive files.
///
/// Supports four actions:
/// - `list_comments` — GET all comments on a file
/// - `list_comment_replies` — GET replies to a specific comment
/// - `reply_comment` — POST a reply to an existing comment
/// - `add_comment` — POST a top-level comment on a file
pub struct FeishuDriveTool;

impl Default for FeishuDriveTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDriveTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl OperantTool for FeishuDriveTool {
    fn name(&self) -> &str {
        "feishu_drive"
    }

    fn description(&self) -> &str {
        "Manage comments on Feishu / Lark Drive files. \
         Actions: list_comments, list_comment_replies, reply_comment, add_comment. \
         Requires LARK_APP_ID and LARK_APP_SECRET environment variables."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<FeishuDriveArgs>(
            "feishu_drive",
            "Manage Feishu/Lark Drive file comments",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        // ---- common required args ------------------------------------------------
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => return ToolResult::error("feishu_drive", "Missing required argument: action"),
        };

        let file_token = match args.get("file_token").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult::error("feishu_drive", "Missing required argument: file_token");
            }
        };

        let page_size = args
            .get("page_size")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(50);

        // ---- dispatch action -----------------------------------------------------
        match action.as_str() {
            "list_comments" => {
                let path = format!(
                    "/open-apis/drive/v1/files/{file_token}/comments?page_size={page_size}"
                );
                match feishu_request("GET", &path, None).await {
                    Ok(resp) => ToolResult::success(
                        "feishu_drive",
                        json!({
                            "success": true,
                            "action": "list_comments",
                            "file_token": file_token,
                            "data": resp["data"],
                        }),
                    ),
                    Err(e) => ToolResult::error("feishu_drive", e),
                }
            }

            "list_comment_replies" => {
                let comment_id = match args.get("comment_id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => {
                        return ToolResult::error(
                            "feishu_drive",
                            "Missing required argument for list_comment_replies: comment_id",
                        );
                    }
                };
                let path = format!(
                    "/open-apis/drive/v1/files/{file_token}/comments/{comment_id}/replies\
                     ?page_size={page_size}"
                );
                match feishu_request("GET", &path, None).await {
                    Ok(resp) => ToolResult::success(
                        "feishu_drive",
                        json!({
                            "success": true,
                            "action": "list_comment_replies",
                            "file_token": file_token,
                            "comment_id": comment_id,
                            "data": resp["data"],
                        }),
                    ),
                    Err(e) => ToolResult::error("feishu_drive", e),
                }
            }

            "reply_comment" => {
                let comment_id = match args.get("comment_id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => {
                        return ToolResult::error(
                            "feishu_drive",
                            "Missing required argument for reply_comment: comment_id",
                        );
                    }
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult::error(
                            "feishu_drive",
                            "Missing required argument for reply_comment: content",
                        );
                    }
                };
                let path =
                    format!("/open-apis/drive/v1/files/{file_token}/comments/{comment_id}/replies");
                let body = json!({ "content": [{ "text": content }] });
                match feishu_request("POST", &path, Some(body)).await {
                    Ok(resp) => ToolResult::success(
                        "feishu_drive",
                        json!({
                            "success": true,
                            "action": "reply_comment",
                            "file_token": file_token,
                            "comment_id": comment_id,
                            "data": resp["data"],
                        }),
                    ),
                    Err(e) => ToolResult::error("feishu_drive", e),
                }
            }

            "add_comment" => {
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult::error(
                            "feishu_drive",
                            "Missing required argument for add_comment: content",
                        );
                    }
                };
                let path = format!("/open-apis/drive/v1/files/{file_token}/new_comments");
                let body = json!({ "content": [{ "text": content }] });
                match feishu_request("POST", &path, Some(body)).await {
                    Ok(resp) => ToolResult::success(
                        "feishu_drive",
                        json!({
                            "success": true,
                            "action": "add_comment",
                            "file_token": file_token,
                            "data": resp["data"],
                        }),
                    ),
                    Err(e) => ToolResult::error("feishu_drive", e),
                }
            }

            _ => ToolResult::error(
                "feishu_drive",
                format!(
                    "Unknown action: {action}. \
                     Valid actions: list_comments, list_comment_replies, \
                     reply_comment, add_comment",
                ),
            ),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- tool metadata -------------------------------------------------------

    #[test]
    fn test_feishu_doc_tool_name() {
        let tool = FeishuDocTool::new();
        assert_eq!(tool.name(), "feishu_doc_read");
    }

    #[test]
    fn test_feishu_doc_tool_description_not_empty() {
        assert!(!FeishuDocTool::new().description().is_empty());
    }

    #[test]
    fn test_feishu_doc_schema_name() {
        assert_eq!(FeishuDocTool::new().schema().name, "feishu_doc_read");
    }

    #[test]
    fn test_feishu_drive_tool_name() {
        assert_eq!(FeishuDriveTool::new().name(), "feishu_drive");
    }

    #[test]
    fn test_feishu_drive_tool_description_not_empty() {
        assert!(!FeishuDriveTool::new().description().is_empty());
    }

    #[test]
    fn test_feishu_drive_schema_name() {
        assert_eq!(FeishuDriveTool::new().schema().name, "feishu_drive");
    }

    // -- missing-argument guardrail tests ------------------------------------

    #[tokio::test]
    async fn test_feishu_doc_missing_document_id() {
        let result = FeishuDocTool::new()
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success, "should fail without document_id");
        let err = result.error.unwrap_or_default();
        assert!(
            err.contains("document_id"),
            "error mentions document_id: {err}"
        );
    }

    #[tokio::test]
    async fn test_feishu_drive_missing_action() {
        let result = FeishuDriveTool::new()
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_feishu_drive_missing_file_token() {
        let result = FeishuDriveTool::new()
            .execute(
                serde_json::json!({ "action": "list_comments" }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_feishu_drive_unknown_action() {
        let result = FeishuDriveTool::new()
            .execute(
                serde_json::json!({
                    "action": "bogus",
                    "file_token": "x",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(
            err.contains("Unknown action"),
            "error mentions 'Unknown action': {err}"
        );
    }

    #[tokio::test]
    async fn test_feishu_drive_missing_comment_id_for_replies() {
        let result = FeishuDriveTool::new()
            .execute(
                serde_json::json!({
                    "action": "list_comment_replies",
                    "file_token": "x",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("comment_id"));
    }

    #[tokio::test]
    async fn test_feishu_drive_missing_content_for_reply() {
        let result = FeishuDriveTool::new()
            .execute(
                serde_json::json!({
                    "action": "reply_comment",
                    "file_token": "x",
                    "comment_id": "123",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("content"));
    }

    #[tokio::test]
    async fn test_feishu_drive_missing_content_for_add_comment() {
        let result = FeishuDriveTool::new()
            .execute(
                serde_json::json!({
                    "action": "add_comment",
                    "file_token": "x",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("content"));
    }

    // -- schema shape tests --------------------------------------------------

    #[test]
    fn test_feishu_doc_schema_has_document_id() {
        let schema = FeishuDocTool::new().schema();
        let schema_val = serde_json::to_value(&schema).unwrap();
        let props = &schema_val["parameters"]["properties"];
        assert!(
            props.get("document_id").is_some(),
            "schema missing document_id"
        );
    }

    #[test]
    fn test_feishu_drive_schema_has_required_fields() {
        let schema = FeishuDriveTool::new().schema();
        let schema_val = serde_json::to_value(&schema).unwrap();
        let props = &schema_val["parameters"]["properties"];
        for field in &["action", "fileToken", "commentId", "content", "pageSize"] {
            assert!(props.get(*field).is_some(), "schema missing {field}");
        }
    }
}

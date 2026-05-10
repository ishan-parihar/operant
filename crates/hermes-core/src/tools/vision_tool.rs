//! Vision analysis tool
//!
//! Tool for analyzing images from URLs or local file paths.
//! Downloads images, converts to base64, and returns them for vision-capable models.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

/// Maximum allowed image size in bytes (20 MB)
const MAX_IMAGE_SIZE_BYTES: u64 = 20 * 1024 * 1024;

/// Maximum download size in bytes (50 MB)
const MAX_DOWNLOAD_SIZE_BYTES: u64 = 50 * 1024 * 1024;

/// Download timeout in seconds
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// Vision analysis tool
pub struct VisionTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisionAnalyzeArgs {
    /// Image URL (http/https), local file path, or data URL to load
    image_url: String,
    /// Your specific question or request about the image
    question: String,
}

/// Detect MIME type from file header bytes
fn detect_mime_type(data: &[u8], path: &PathBuf) -> Option<&'static str> {
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if data.starts_with(b"BM") {
        return Some("image/bmp");
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    // Check for SVG
    if path.extension().map(|e| e.to_str()).flatten() == Some("svg") {
        return Some("image/svg+xml");
    }
    None
}

/// Determine MIME type from file extension
fn mime_from_extension(path: &PathBuf) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/jpeg",
    }
}

/// Validate that a URL is a valid HTTP/HTTPS URL
fn is_valid_url(url: &str) -> bool {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        matches!(parsed.scheme(), "http" | "https")
    } else {
        false
    }
}

/// Resolve a file path from various URL formats
fn resolve_file_path(url: &str) -> Option<PathBuf> {
    // Handle file:// scheme
    let path_str = if url.starts_with("file://") {
        &url[7..]
    } else {
        url
    };

    // Expand user home directory
    let expanded = if path_str.starts_with("~") {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}{}", home, &path_str[1..])
        } else {
            path_str.to_string()
        }
    } else {
        path_str.to_string()
    };

    let path = PathBuf::from(expanded);
    if path.exists() && path.is_file() {
        Some(path)
    } else {
        None
    }
}

/// Download image from URL
async fn download_image(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .header("Accept", "image/*,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("Failed to download image: {}", e))?;

    // Check content length
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_DOWNLOAD_SIZE_BYTES {
            return Err(format!(
                "Image too large: {} bytes (max {})",
                content_length, MAX_DOWNLOAD_SIZE_BYTES
            ));
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read image data: {}", e))?;

    if bytes.len() > MAX_DOWNLOAD_SIZE_BYTES as usize {
        return Err(format!(
            "Image too large: {} bytes (max {})",
            bytes.len(),
            MAX_DOWNLOAD_SIZE_BYTES
        ));
    }

    Ok(bytes.to_vec())
}

/// Convert image bytes to base64 data URL
fn bytes_to_data_url(bytes: &[u8], mime_type: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{};base64,{}", mime_type, encoded)
}

#[async_trait]
impl HermesTool for VisionTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }

    fn description(&self) -> &str {
        "Load an image into the conversation so you can see it. Accepts a URL, local file path, or data URL. When your active model has native vision, the image is attached to your context directly and you read the pixels yourself on the next turn."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<VisionAnalyzeArgs>(
            "vision_analyze",
            "Load an image into the conversation for vision-capable models",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: VisionAnalyzeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("vision_analyze", format!("Invalid arguments: {}", e))
            }
        };

        let image_url = args.image_url.trim();
        let question = args.question.trim();

        if image_url.is_empty() {
            return ToolResult::error("vision_analyze", "image_url is required");
        }

        if question.is_empty() {
            return ToolResult::error("vision_analyze", "question is required");
        }

        // Try to resolve as local file first
        if let Some(path) = resolve_file_path(image_url) {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    if bytes.len() as u64 > MAX_IMAGE_SIZE_BYTES {
                        return ToolResult::error(
                            "vision_analyze",
                            format!(
                                "Image too large: {} bytes (max {})",
                                bytes.len(),
                                MAX_IMAGE_SIZE_BYTES
                            ),
                        );
                    }

                    let mime = mime_from_extension(&path);
                    let data_url = bytes_to_data_url(&bytes, mime);

                    // Check final size
                    if data_url.len() as u64 > MAX_IMAGE_SIZE_BYTES {
                        return ToolResult::error(
                            "vision_analyze",
                            format!(
                                "Image too large after base64 encoding: {} bytes (max {})",
                                data_url.len(),
                                MAX_IMAGE_SIZE_BYTES
                            ),
                        );
                    }

                    return ToolResult::success(
                        "vision_analyze",
                        serde_json::json!({
                            "success": true,
                            "image_url": image_url,
                            "question": question,
                            "data_url": data_url,
                            "size_bytes": bytes.len(),
                            "mime_type": mime,
                            "native_vision": true,
                            "message": "Image loaded into your context — you can see it natively now. Use your built-in vision to answer the user."
                        }),
                    );
                }
                Err(e) => {
                    return ToolResult::error(
                        "vision_analyze",
                        format!("Failed to read local file: {}", e),
                    )
                }
            }
        }

        // Check if it's a data URL
        if image_url.starts_with("data:") {
            // Validate it's not too large
            if image_url.len() as u64 > MAX_IMAGE_SIZE_BYTES {
                return ToolResult::error(
                    "vision_analyze",
                    format!(
                        "Data URL too large: {} bytes (max {})",
                        image_url.len(),
                        MAX_IMAGE_SIZE_BYTES
                    ),
                );
            }
            return ToolResult::success(
                "vision_analyze",
                serde_json::json!({
                    "success": true,
                    "image_url": image_url,
                    "question": question,
                    "data_url": image_url,
                    "size_bytes": image_url.len(),
                    "native_vision": true,
                    "message": "Image loaded into your context — you can see it natively now. Use your built-in vision to answer the user."
                }),
            );
        }

        // Try as HTTP URL
        if !is_valid_url(image_url) {
            return ToolResult::error(
                "vision_analyze",
                "Invalid image source. Provide an HTTP/HTTPS URL or a valid local file path.",
            );
        }

        // Download the image
        match download_image(image_url).await {
            Ok(bytes) => {
                // Detect MIME type from header
                let mime = detect_mime_type(&bytes, &PathBuf::from("image"))
                    .unwrap_or_else(|| "image/jpeg");

                let data_url = bytes_to_data_url(&bytes, mime);

                // Check final size
                if data_url.len() as u64 > MAX_IMAGE_SIZE_BYTES {
                    return ToolResult::error(
                        "vision_analyze",
                        format!(
                            "Image too large after base64 encoding: {} bytes (max {})",
                            data_url.len(),
                            MAX_IMAGE_SIZE_BYTES
                        ),
                    );
                }

                ToolResult::success(
                    "vision_analyze",
                    serde_json::json!({
                        "success": true,
                        "image_url": image_url,
                        "question": question,
                        "data_url": data_url,
                        "size_bytes": bytes.len(),
                        "mime_type": mime,
                        "native_vision": true,
                        "message": "Image loaded into your context — you can see it natively now. Use your built-in vision to answer the user."
                    }),
                )
            }
            Err(e) => ToolResult::error("vision_analyze", format!("Failed to download image: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(mime_from_extension(&PathBuf::from("test.jpg")), "image/jpeg");
        assert_eq!(mime_from_extension(&PathBuf::from("test.png")), "image/png");
        assert_eq!(mime_from_extension(&PathBuf::from("test.gif")), "image/gif");
        assert_eq!(mime_from_extension(&PathBuf::from("test.svg")), "image/svg+xml");
        assert_eq!(mime_from_extension(&PathBuf::from("test.unknown")), "image/jpeg");
    }

    #[test]
    fn test_is_valid_url() {
        assert!(is_valid_url("https://example.com/image.jpg"));
        assert!(is_valid_url("http://example.com/image.jpg"));
        assert!(!is_valid_url("ftp://example.com/image.jpg"));
        assert!(!is_valid_url("not-a-url"));
    }

    #[test]
    fn test_detect_mime_type() {
        // PNG header
        let png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_mime_type(&png, &PathBuf::from("test")), Some("image/png"));

        // JPEG header
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_mime_type(&jpeg, &PathBuf::from("test")), Some("image/jpeg"));

        // GIF header
        let gif = b"GIF89a".to_vec();
        assert_eq!(detect_mime_type(&gif, &PathBuf::from("test")), Some("image/gif"));
    }
}
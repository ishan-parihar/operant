//! Vision analysis tool
//!
//! Tool for analyzing images from URLs or local file paths.
//! Downloads images, converts to base64, and returns them for vision-capable models.
//!
//! Features:
//! - Native fast-path: when the main model supports images, return multimodal envelope
//! - Auxiliary LLM fallback: route to vision model when main model doesn't support vision
//! - Auto-resize: resize images to fit model limits (7900px max dimension, 4MB embed target)
//! - SSRF protection: block private/internal IPs in image URLs

use async_trait::async_trait;
use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use crate::config::runtime_config;
use crate::schema::ToolSchema;
use crate::security::check_url_safety;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Maximum allowed image size in bytes (20 MB hard ceiling)
const MAX_IMAGE_SIZE_BYTES: u64 = 20 * 1024 * 1024;

/// Maximum download size in bytes (50 MB)
const MAX_DOWNLOAD_SIZE_BYTES: u64 = 50 * 1024 * 1024;

/// Download timeout in seconds
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// Auxiliary LLM call timeout in seconds
const AUX_VISION_TIMEOUT_SECS: u64 = 120;

/// Proactive embed target: 4 MB base64 cap (headroom under Anthropic's 5 MB)
const EMBED_TARGET_BYTES: usize = 4 * 1024 * 1024;

/// Proactive embed dimension cap (px, longest side)
const EMBED_MAX_DIMENSION: u32 = 7900;

/// Default vision model for auxiliary fallback
const DEFAULT_VISION_MODEL: &str = "google/gemini-2.0-flash-001";

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
fn detect_mime_type(data: &[u8], path: &Path) -> Option<&'static str> {
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
    if path.extension().and_then(|e| e.to_str()) == Some("svg") {
        return Some("image/svg+xml");
    }
    None
}

/// Determine MIME type from file extension
fn mime_from_extension(path: &Path) -> &'static str {
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
    let path_str = url.strip_prefix("file://").unwrap_or(url);

    // Expand user home directory
    let expanded = if let Some(rest) = path_str.strip_prefix("~") {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}{}", home, rest)
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

/// Check if the main model supports native vision (multimodal tool results).
///
/// Mirrors the Python `_supports_media_in_tool_results` check: known
/// vision-capable providers return true, unknown ones return false so the
/// caller falls back to the auxiliary vision LLM path.
fn main_model_supports_vision() -> bool {
    let cfg = runtime_config();
    let base_url = cfg.client.base_url.to_lowercase();

    // Known vision-capable provider patterns
    let vision_providers = [
        "anthropic",
        "claude",
        "openai",
        "gemini",
        "openrouter",
        "nous",
        "vertex",
        "bedrock",
        "azure",
    ];

    vision_providers.iter().any(|p| base_url.contains(p))
}

/// Resize image bytes to fit within dimension and byte limits.
///
/// Progressive strategy matching the Python implementation:
/// 1. Try encoding at original size
/// 2. If too large, halve dimensions and try quality steps (85, 70, 50 for JPEG)
/// 3. Up to 5 rounds of halving
fn resize_image_for_vision(
    image_bytes: &[u8],
    mime_type: &str,
    max_base64_bytes: usize,
    max_dimension: u32,
) -> Result<(Vec<u8>, String), String> {
    let img = image::load_from_memory(image_bytes)
        .map_err(|e| format!("Failed to decode image for resize: {}", e))?;

    let (w, h) = (img.width(), img.height());
    let out_mime = if mime_type == "image/png" {
        "image/png"
    } else {
        "image/jpeg"
    };

    // If already within bounds, return as-is
    if w <= max_dimension && h <= max_dimension {
        let encoded = encode_image(&img, out_mime, 85)?;
        if encoded.len() <= max_base64_bytes {
            return Ok((encoded, out_mime.to_string()));
        }
    }

    // Progressive resize: halve dimensions + quality stepping
    let mut current = img;
    let mut prev_dims = (w, h);
    let quality_steps: Vec<u32> = if out_mime == "image/jpeg" {
        vec![85, 70, 50]
    } else {
        vec![0] // PNG: quality is irrelevant, only dimension reduction helps
    };

    for attempt in 0..5 {
        if attempt > 0 {
            let scale = 0.5;
            let new_w = std::cmp::max((current.width() as f32 * scale) as u32, 64);
            let new_h = std::cmp::max((current.height() as f32 * scale) as u32, 64);

            if (new_w, new_h) == prev_dims {
                break;
            }

            current = current.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);
            prev_dims = (new_w, new_h);
        }

        for &q in &quality_steps {
            if let Ok(encoded) = encode_image(&current, out_mime, q) {
                if encoded.len() <= max_base64_bytes
                    && current.width() <= max_dimension
                    && current.height() <= max_dimension
                {
                    return Ok((encoded, out_mime.to_string()));
                }
            }
        }
    }

    // Return best attempt even if over limit
    let best = encode_image(&current, out_mime, 50)
        .map_err(|e| format!("Failed to encode resized image: {}", e))?;
    Ok((best, out_mime.to_string()))
}

/// Encode a DynamicImage to bytes with given quality (for JPEG) or compression level
fn encode_image(img: &DynamicImage, mime_type: &str, quality: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    match mime_type {
        "image/jpeg" => {
            let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality as u8);
            encoder
                .encode_image(img)
                .map_err(|e| format!("JPEG encode error: {}", e))?;
        }
        "image/png" => {
            img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
                .map_err(|e| format!("PNG encode error: {}", e))?;
        }
        _ => {
            let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality as u8);
            encoder
                .encode_image(img)
                .map_err(|e| format!("JPEG encode error: {}", e))?;
        }
    }
    Ok(buf)
}

/// Convert image bytes to base64 data URL
fn bytes_to_data_url(bytes: &[u8], mime_type: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{};base64,{}", mime_type, encoded)
}

/// Download image from URL with SSRF protection
async fn download_image(url: &str) -> Result<Vec<u8>, String> {
    // SSRF protection: block private/internal IPs
    let safe = check_url_safety(url)
        .await
        .map_err(|e| format!("URL safety check failed: {}", e))?;
    if !safe {
        return Err(
            "URL blocked: points to private/internal address (SSRF protection)".to_string(),
        );
    }

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

/// Build the multimodal tool-result envelope for native vision fast-path
fn build_native_vision_envelope(
    image_url: &str,
    question: &str,
    image_data_url: &str,
    image_size_bytes: usize,
) -> serde_json::Value {
    let text_part = format!(
        "Image loaded into your context \u{2014} you can see it natively now. \
         Use your built-in vision to answer the user.\n\nQuestion: {}",
        question
    );

    let summary = format!(
        "Image attached natively for the main model ({:.1} KB). Answer using built-in vision.",
        image_size_bytes as f64 / 1024.0
    );

    serde_json::json!({
        "_multimodal": true,
        "content": [
            {"type": "text", "text": text_part},
            {"type": "image_url", "image_url": {"url": image_data_url}}
        ],
        "text_summary": summary,
        "meta": {
            "image_url": &image_url[..std::cmp::min(image_url.len(), 200)],
            "size_bytes": image_size_bytes,
            "native_vision": true
        }
    })
}

/// Call auxiliary vision LLM to describe the image
async fn call_auxiliary_vision_llm(image_data_url: &str, question: &str) -> Result<String, String> {
    let cfg = runtime_config();
    let aux_vision = cfg.auxiliary_models.vision.as_ref();

    let (model, api_key, base_url) = if let Some(aux) = aux_vision {
        let model = aux
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_VISION_MODEL.to_string());
        let api_key = aux.api_key.clone().or_else(|| {
            std::env::var("OPENROUTER_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
        });
        let base_url = aux
            .base_url
            .clone()
            .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string());
        (model, api_key, base_url)
    } else {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());
        let model = std::env::var("AUXILIARY_VISION_MODEL")
            .unwrap_or_else(|_| DEFAULT_VISION_MODEL.to_string());
        (model, api_key, "https://openrouter.ai/api/v1".to_string())
    };

    let api_key = api_key.ok_or_else(|| {
        "No vision model configured. Set auxiliary_models.vision in config or OPENROUTER_API_KEY env var"
            .to_string()
    })?;
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let prompt = format!(
        "Fully describe and explain everything about this image, then answer the following question:\n\n{}",
        question
    );

    let request_body = serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": image_data_url}}
            ]
        }],
        "max_tokens": 2000,
        "temperature": 0.1
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(AUX_VISION_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("Vision API error ({}): {}", status, error_text));
    }

    let response_body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse vision API response: {}", e))?;

    // Extract content from OpenAI-compatible response
    let analysis = response_body
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or("No analysis returned");

    Ok(analysis.to_string())
}

#[async_trait]
impl OperantTool for VisionTool {
    fn name(&self) -> &str {
        "vision_analyze"
    }

    fn description(&self) -> &str {
        "Load an image into the conversation so you can see it. Accepts a URL, local file path, or data URL. When your active model has native vision, the image is attached to your context directly and you read the pixels yourself on the next turn. For non-vision models, falls back to an auxiliary vision model that returns a text description."
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

        // Determine native vision support
        let use_native_path = main_model_supports_vision();

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

                    // Auto-resize if needed (dimension or byte limit)
                    let (processed_bytes, processed_mime) =
                        if needs_resize(&bytes, mime, EMBED_MAX_DIMENSION) {
                            match resize_image_for_vision(
                                &bytes,
                                mime,
                                EMBED_TARGET_BYTES,
                                EMBED_MAX_DIMENSION,
                            ) {
                                Ok((resized, m)) => (resized, m),
                                Err(e) => {
                                    return ToolResult::error(
                                        "vision_analyze",
                                        format!("Failed to resize image: {}", e),
                                    )
                                }
                            }
                        } else {
                            (bytes.to_vec(), mime.to_string())
                        };

                    let data_url = bytes_to_data_url(&processed_bytes, &processed_mime);

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

                    // Native fast-path: return multimodal envelope
                    if use_native_path {
                        return ToolResult::success(
                            "vision_analyze",
                            build_native_vision_envelope(
                                image_url,
                                question,
                                &data_url,
                                processed_bytes.len(),
                            ),
                        );
                    }

                    // Auxiliary LLM fallback
                    return match call_auxiliary_vision_llm(&data_url, question).await {
                        Ok(analysis) => ToolResult::success(
                            "vision_analyze",
                            serde_json::json!({
                                "success": true,
                                "analysis": analysis,
                                "image_url": image_url,
                                "size_bytes": processed_bytes.len(),
                                "mime_type": processed_mime,
                                "native_vision": false
                            }),
                        ),
                        Err(e) => ToolResult::error(
                            "vision_analyze",
                            format!("Vision model failed: {}", e),
                        ),
                    };
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

            // Native fast-path: return multimodal envelope
            if use_native_path {
                return ToolResult::success(
                    "vision_analyze",
                    build_native_vision_envelope(image_url, question, image_url, image_url.len()),
                );
            }

            // Auxiliary LLM fallback for data URLs
            return match call_auxiliary_vision_llm(image_url, question).await {
                Ok(analysis) => ToolResult::success(
                    "vision_analyze",
                    serde_json::json!({
                        "success": true,
                        "analysis": analysis,
                        "image_url": image_url,
                        "size_bytes": image_url.len(),
                        "native_vision": false
                    }),
                ),
                Err(e) => {
                    ToolResult::error("vision_analyze", format!("Vision model failed: {}", e))
                }
            };
        }

        // Try as HTTP URL
        if !is_valid_url(image_url) {
            return ToolResult::error(
                "vision_analyze",
                "Invalid image source. Provide an HTTP/HTTPS URL or a valid local file path.",
            );
        }

        // Download the image (with SSRF protection)
        match download_image(image_url).await {
            Ok(bytes) => {
                // Detect MIME type from header
                let mime =
                    detect_mime_type(&bytes, &PathBuf::from("image")).unwrap_or("image/jpeg");

                // Auto-resize if needed
                let (processed_bytes, processed_mime) =
                    if needs_resize(&bytes, mime, EMBED_MAX_DIMENSION) {
                        match resize_image_for_vision(
                            &bytes,
                            mime,
                            EMBED_TARGET_BYTES,
                            EMBED_MAX_DIMENSION,
                        ) {
                            Ok((resized, m)) => (resized, m),
                            Err(e) => {
                                return ToolResult::error(
                                    "vision_analyze",
                                    format!("Failed to resize image: {}", e),
                                )
                            }
                        }
                    } else {
                        (bytes.to_vec(), mime.to_string())
                    };

                let data_url = bytes_to_data_url(&processed_bytes, &processed_mime);

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

                // Native fast-path: return multimodal envelope
                if use_native_path {
                    return ToolResult::success(
                        "vision_analyze",
                        build_native_vision_envelope(
                            image_url,
                            question,
                            &data_url,
                            processed_bytes.len(),
                        ),
                    );
                }

                // Auxiliary LLM fallback
                match call_auxiliary_vision_llm(&data_url, question).await {
                    Ok(analysis) => ToolResult::success(
                        "vision_analyze",
                        serde_json::json!({
                            "success": true,
                            "analysis": analysis,
                            "image_url": image_url,
                            "size_bytes": processed_bytes.len(),
                            "mime_type": processed_mime,
                            "native_vision": false
                        }),
                    ),
                    Err(e) => {
                        ToolResult::error("vision_analyze", format!("Vision model failed: {}", e))
                    }
                }
            }
            Err(e) => {
                ToolResult::error("vision_analyze", format!("Failed to download image: {}", e))
            }
        }
    }
}

fn needs_resize(bytes: &[u8], _mime_type: &str, max_dimension: u32) -> bool {
    let estimated_b64 = (bytes.len() * 4) / 3 + 100;
    if estimated_b64 > EMBED_TARGET_BYTES {
        return true;
    }
    if let Ok(img) = image::load_from_memory(bytes) {
        let (w, h) = (img.width(), img.height());
        if w > max_dimension || h > max_dimension {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_from_extension() {
        assert_eq!(
            mime_from_extension(&PathBuf::from("test.jpg")),
            "image/jpeg"
        );
        assert_eq!(mime_from_extension(&PathBuf::from("test.png")), "image/png");
        assert_eq!(mime_from_extension(&PathBuf::from("test.gif")), "image/gif");
        assert_eq!(
            mime_from_extension(&PathBuf::from("test.svg")),
            "image/svg+xml"
        );
        assert_eq!(
            mime_from_extension(&PathBuf::from("test.unknown")),
            "image/jpeg"
        );
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
        assert_eq!(
            detect_mime_type(&png, &PathBuf::from("test")),
            Some("image/png")
        );

        // JPEG header
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(
            detect_mime_type(&jpeg, &PathBuf::from("test")),
            Some("image/jpeg")
        );

        // GIF header
        let gif = b"GIF89a".to_vec();
        assert_eq!(
            detect_mime_type(&gif, &PathBuf::from("test")),
            Some("image/gif")
        );
    }

    #[test]
    fn test_resolve_file_path() {
        // Non-existent path returns None
        assert!(resolve_file_path("/nonexistent/path.jpg").is_none());

        // file:// scheme
        assert!(resolve_file_path("file:///nonexistent/path.jpg").is_none());
    }

    #[tokio::test]
    async fn test_vision_tool_missing_args() {
        let tool = VisionTool;
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_vision_tool_empty_url() {
        let tool = VisionTool;
        let result = tool
            .execute(
                serde_json::json!({"image_url": "", "question": "what is this"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_vision_tool_empty_question() {
        let tool = VisionTool;
        let result = tool
            .execute(
                serde_json::json!({"image_url": "https://example.com/img.jpg", "question": ""}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_vision_tool_invalid_url() {
        let tool = VisionTool;
        let result = tool
            .execute(
                serde_json::json!({"image_url": "not-a-url", "question": "what is this"}),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Invalid"));
    }
}

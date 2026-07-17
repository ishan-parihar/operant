use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct ImageGenerationTool {
    client: Client,
    api_key: String,
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationArgs {
    /// The text prompt describing the image to generate
    pub prompt: String,
    /// Aspect ratio: 1:1, 16:9, 9:16, 4:3, 3:4
    #[serde(default = "default_aspect_ratio")]
    pub aspect_ratio: String,
    /// Model to use: flux-pro, flux-dev, flux-klein, gpt-image-1, gpt-image-2, recraft, ideogram
    #[serde(default = "default_model")]
    pub model: String,
    /// Number of inference steps (provider-dependent)
    pub num_inference_steps: Option<u32>,
    /// Guidance scale for generation
    pub guidance_scale: Option<f32>,
    /// Number of images to generate (default 1)
    pub num_images: Option<u32>,
    /// Output format: png, jpeg, webp
    pub output_format: Option<String>,
    /// Random seed for reproducibility
    pub seed: Option<u64>,
    /// Enable upscaling after generation
    #[serde(default)]
    pub upscale: bool,
}

fn default_aspect_ratio() -> String {
    "1:1".to_string()
}

fn default_model() -> String {
    "flux-pro".to_string()
}

impl Default for ImageGenerationTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageGenerationTool {
    pub fn new() -> Self {
        let api_key = std::env::var("FAL_KEY").unwrap_or_default();
        Self {
            client: Client::new(),
            api_key,
        }
    }

    async fn generate(&self, args: ImageGenerationArgs) -> ToolResult {
        if self.api_key.is_empty() {
            return ToolResult::error("image_generate", "FAL_KEY not set");
        }

        if args.prompt.trim().is_empty() {
            return ToolResult::error("image_generate", "Prompt is required");
        }

        let endpoint = match args.model.as_str() {
            "flux-pro" => "fal-ai/flux-pro",
            "flux-dev" => "fal-ai/flux-dev",
            "flux-klein" => "fal-ai/flux-2/klein/9b",
            "gpt-image-1" => "fal-ai/gpt-image-1",
            "gpt-image-2" => "fal-ai/gpt-image-2",
            "recraft" => "fal-ai/recraft-v3",
            "ideogram" => "fal-ai/ideogram-v2",
            "flux-schnell" => "fal-ai/flux-schnell",
            _ => "fal-ai/flux-pro",
        };

        let aspect_ratio = match args.aspect_ratio.to_lowercase().as_str() {
            "1:1" => "1:1",
            "16:9" => "16:9",
            "9:16" => "9:16",
            "4:3" => "4:3",
            "3:4" => "3:4",
            _ => "1:1",
        };

        let mut payload = json!({
            "prompt": args.prompt,
            "aspect_ratio": aspect_ratio,
            "num_images": args.num_images.unwrap_or(1),
        });

        if let Some(steps) = args.num_inference_steps {
            payload["num_inference_steps"] = json!(steps);
        }
        if let Some(scale) = args.guidance_scale {
            payload["guidance_scale"] = json!(scale);
        }
        if let Some(format) = args.output_format {
            payload["output_format"] = json!(format);
        }
        if let Some(seed) = args.seed {
            payload["seed"] = json!(seed);
        }

        let response = match self
            .client
            .post(format!("https://queue.fal.run/{}", endpoint))
            .header("Authorization", format!("Key {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("image_generate", format!("API request failed: {}", e));
            }
        };

        if !response.status().is_success() {
            return ToolResult::error(
                "image_generate",
                format!("API error: {}", response.status()),
            );
        }

        let result: Value = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error(
                    "image_generate",
                    format!("Failed to parse response: {}", e),
                );
            }
        };

        let images: Vec<String> = result
            .get("images")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|img| {
                        img.get("url")
                            .and_then(|u| u.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        if images.is_empty() {
            return ToolResult::error("image_generate", "No images returned from API");
        }

        let mut response_data = json!({
            "success": true,
            "images": images,
            "model": args.model
        });

        if args.upscale && !images.is_empty() {
            match self.upscale_image(&images[0], &args.prompt).await {
                Ok(upscale_url) => {
                    response_data["upscaled_image"] = json!(upscale_url);
                }
                Err(e) => {
                    response_data["upscale_warning"] = json!(format!("Upscale failed: {}", e));
                }
            }
        }

        ToolResult::success("image_generate", response_data)
    }

    async fn upscale_image(&self, image_url: &str, _prompt: &str) -> Result<String, String> {
        let payload = json!({
            "image_url": image_url,
            "scale": 2
        });

        let response = self
            .client
            .post("https://queue.fal.run/fal-ai/clarity-upscaler")
            .header("Authorization", format!("Key {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("API request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Upscale API error: {}", response.status()));
        }

        let result: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse upscale response: {}", e))?;

        result
            .get("image")
            .and_then(|i| i.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "No upscaled image URL in response".to_string())
    }
}

#[async_trait]
impl OperantTool for ImageGenerationTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Generate images from text prompts using AI (flux-pro, flux-dev, flux-klein, gpt-image-1, gpt-image-2, recraft, ideogram). Supports upscaling."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ImageGenerationArgs>(
            "image_generate",
            "Generate images from text prompts with AI models",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: ImageGenerationArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("image_generate", format!("Invalid arguments: {}", e));
            }
        };
        self.generate(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_generation_schema() {
        let tool = ImageGenerationTool::new();
        let schema = tool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "image_generate");
    }

    #[test]
    fn test_default_aspect_ratio() {
        assert_eq!(default_aspect_ratio(), "1:1");
    }

    #[test]
    fn test_default_model() {
        assert_eq!(default_model(), "flux-pro");
    }
}

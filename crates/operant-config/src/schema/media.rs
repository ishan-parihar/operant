//! `media` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

/// Multimodal (image) handling configuration (`[multimodal]` section).
///
/// # Privacy and cost note
///
/// Tool results that print real local image paths (e.g. shell tools doing
/// `ls /pictures` or `find . -name '*.png'`) are canonicalized into
/// `[IMAGE:...]` markers and base64-inlined into the next provider request.
/// This means image bytes that previously stayed local will be uploaded to
/// the configured provider when surfaced by a tool.
///
/// `max_images` (and the `trim_old_images` LRU policy) bounds the per-request
/// image budget, but operators running shell-style tools over directories of
/// personal or sensitive images should be aware of the upload semantics. See
/// `docs/book/src/contributing/privacy.md` for the project's privacy stance.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "multimodal"]
pub struct MultimodalConfig {
    /// Maximum number of image attachments accepted per request.
    ///
    /// Caps the total number of `[IMAGE:...]` markers that survive into the
    /// provider request after multimodal preprocessing. Older images are
    /// dropped first when the cumulative count exceeds this limit. Acts as
    /// the upper bound on per-turn upload cost when tool outputs surface
    /// local image paths.
    #[serde(default = "default_multimodal_max_images")]
    pub max_images: usize,
    /// Maximum image payload size in MiB before base64 encoding.
    #[serde(default = "default_multimodal_max_image_size_mb")]
    pub max_image_size_mb: usize,
    /// Allow fetching remote image URLs (http/https). Disabled by default.
    #[serde(default)]
    pub allow_remote_fetch: bool,
    /// Provider name to use for vision/image messages (e.g. `"ollama"`).
    /// When set, messages containing `[IMAGE:]` markers are routed to this
    /// provider instead of the default text provider.
    #[serde(default)]
    pub vision_provider: Option<String>,
    /// Model to use when routing to the vision provider (e.g. `"llava:7b"`).
    /// Only used when `vision_provider` is set.
    #[serde(default)]
    pub vision_model: Option<String>,
}

impl MultimodalConfig {
    /// Clamp configured values to safe runtime bounds.
    pub fn effective_limits(&self) -> (usize, usize) {
        let max_images = self.max_images.clamp(1, 16);
        let max_image_size_mb = self.max_image_size_mb.clamp(1, 20);
        (max_images, max_image_size_mb)
    }
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            max_images: default_multimodal_max_images(),
            max_image_size_mb: default_multimodal_max_image_size_mb(),
            allow_remote_fetch: false,
            vision_provider: None,
            vision_model: None,
        }
    }
}

/// Google Imagen (Vertex AI) settings (`[linkedin.image.imagen]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.image.imagen"]
pub struct ImageProviderImagenConfig {
    /// Environment variable name holding the API key.
    #[serde(default = "default_imagen_api_key_env")]
    pub api_key_env: String,
    /// Environment variable for the Google Cloud project ID.
    #[serde(default = "default_imagen_project_id_env")]
    pub project_id_env: String,
    /// Vertex AI region.
    #[serde(default = "default_imagen_region")]
    pub region: String,
}

impl Default for ImageProviderImagenConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_imagen_api_key_env(),
            project_id_env: default_imagen_project_id_env(),
            region: default_imagen_region(),
        }
    }
}

/// OpenAI DALL-E settings (`[linkedin.image.dalle]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.image.dalle"]
pub struct ImageProviderDalleConfig {
    /// Environment variable name holding the OpenAI API key.
    #[serde(default = "default_dalle_api_key_env")]
    pub api_key_env: String,
    /// DALL-E model identifier.
    #[serde(default = "default_dalle_model")]
    pub model: String,
    /// Image dimensions.
    #[serde(default = "default_dalle_size")]
    pub size: String,
}

impl Default for ImageProviderDalleConfig {
    fn default() -> Self {
        Self {
            api_key_env: default_dalle_api_key_env(),
            model: default_dalle_model(),
            size: default_dalle_size(),
        }
    }
}

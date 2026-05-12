//! Anthropic model client (stub).
//!
//! Activated by the `anthropic` feature flag.  Currently returns
//! `unimplemented!()` for every trait method.
#![cfg(feature = "anthropic")]

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::client::ChatResponse;
use crate::error::{Error, Result};
use super::super::model_client::{ChatRequest, ModelClient, StreamChunk};

/// Placeholder Anthropic client.
///
/// Requires the `anthropic` feature.  Replace with a real implementation
/// using the Anthropic API when the feature is stabilized.
pub struct AnthropicModelClient;

#[async_trait]
impl ModelClient for AnthropicModelClient {
    fn provider_name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        Err(Error::Agent("Anthropic client is not yet implemented".to_string()))
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("Anthropic client is not yet implemented".to_string()))
    }
}

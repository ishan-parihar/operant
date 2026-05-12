//! Gateway Runner
//!
//! Manages a singleton Gateway instance that persists across CLI commands
//! within a single process. Provides start/stop/restart lifecycle management
//! backed by configuration and the `hermes_core::gateway` module.

use anyhow::{Context, Result};
use hermes_core::config::AppConfig;
use hermes_core::gateway::{
    Gateway, GatewayConfig, PlatformAdapter, TelegramAdapter, WebhookAdapter,
};
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::Mutex;
/// Global gateway instance managed by the runner
static RUNNER: OnceLock<Mutex<Option<Arc<Gateway>>>> = OnceLock::new();

fn runner() -> &'static Mutex<Option<Arc<Gateway>>> {
    RUNNER.get_or_init(|| Mutex::new(None))
}

/// Build platform adapters from config
fn build_adapters(config: &GatewayConfig) -> Vec<Arc<dyn PlatformAdapter>> {
    let mut adapters: Vec<Arc<dyn PlatformAdapter>> = Vec::new();

    // Telegram
    if config.telegram_enabled {
        adapters.push(Arc::new(TelegramAdapter::new(config.telegram_token.clone())));
    }

    // Webhook adapter
    if config.webhooks_enabled {
        adapters.push(Arc::new(WebhookAdapter::new(config.webhooks_enabled)));
    }

    adapters
}

/// Start the gateway
pub async fn start_gateway(_app_config: &AppConfig) -> Result<String> {
    let mut guard = runner().lock().await;

    if guard.is_some() {
        // Check if already running
        if let Some(gw) = guard.as_ref() {
            if gw.is_running().await {
                return Ok("Gateway is already running.".to_string());
            }
        }
    }

    let gw_config = GatewayConfig::default();
    let adapters = build_adapters(&gw_config);

    let mut gateway = Gateway::new(gw_config);
    for adapter in adapters {
        gateway = gateway.with_adapter(adapter);
    }

    let gateway = Arc::new(gateway);
    gateway.start().await.context("Failed to start gateway")?;

    let platform_count = gateway.status().await.len();
    *guard = Some(gateway);

    Ok(format!("Gateway started with {} platform(s).", platform_count))
}

/// Stop the gateway
pub async fn stop_gateway() -> Result<String> {
    let mut guard = runner().lock().await;

    match guard.take() {
        Some(gateway) => {
            gateway.stop().await.context("Failed to stop gateway")?;
            Ok("Gateway stopped.".to_string())
        }
        None => Ok("Gateway is not running.".to_string()),
    }
}

/// Restart the gateway
pub async fn restart_gateway(app_config: &AppConfig) -> Result<String> {
    stop_gateway().await?;
    start_gateway(app_config).await
}

/// Check if gateway is running
pub async fn is_running() -> bool {
    let guard = runner().lock().await;
    match guard.as_ref() {
        Some(gw) => gw.is_running().await,
        None => false,
    }
}

/// Get reference to running gateway (for other modules)
pub async fn get_gateway() -> Option<Arc<Gateway>> {
    let guard = runner().lock().await;
    guard.clone()
}

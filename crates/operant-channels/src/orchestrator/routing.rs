//! `routing` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use anyhow::{Context, Result};
use operant_config::schema::Config;
use operant_providers::{self, Provider};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;

pub(crate) fn resolve_provider_alias(name: &str) -> Option<String> {
    let candidate = name.trim();
    if candidate.is_empty() {
        return None;
    }

    let providers_list = operant_providers::list_providers();
    for provider in providers_list {
        if provider.name.eq_ignore_ascii_case(candidate)
            || provider
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(candidate))
        {
            return Some(provider.name.to_string());
        }
    }

    None
}

pub(crate) fn resolved_default_provider(config: &Config) -> String {
    config
        .providers
        .fallback
        .clone()
        .unwrap_or_else(|| "openrouter".to_string())
}

/// Three-step model resolution mirroring `agent::Agent::from_config` (#6099):
/// (1) the fallback provider's `model`, (2) the first configured
/// `[providers.models.*]` model with a WARN naming what to set,
/// (3) hard fail with an actionable error. No silent vendor-default.
pub(crate) fn resolved_default_model(config: &Config) -> anyhow::Result<String> {
    let provider_name = config.providers.fallback.as_deref().unwrap_or("openrouter");
    if let Some(m) = config
        .providers
        .fallback_provider()
        .and_then(|e| e.model.as_deref())
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return Ok(m.to_string());
    }
    if let Some(m) = config.providers.resolve_default_model() {
        tracing::warn!(
            provider = provider_name,
            model = %m,
            "fallback provider has no `model` set; using first configured \
             providers.models entry as default. Set [providers.models.{provider_name}] \
             model = \"...\" to silence this warning.",
        );
        return Ok(m);
    }
    anyhow::bail!(
        "no model configured: providers.fallback = {:?} resolves with no model, \
         and no [providers.models.*] entry has a `model` field set. \
         Configure at least one [providers.models.<name>] model = \"...\", \
         or define a [[model_routes]] hint, before starting channels.",
        config.providers.fallback,
    )
}

pub(crate) fn runtime_defaults_from_config(
    config: &Config,
) -> anyhow::Result<ChannelRuntimeDefaults> {
    Ok(ChannelRuntimeDefaults {
        default_provider: resolved_default_provider(config),
        model: resolved_default_model(config)?,
        temperature: config
            .providers
            .fallback_provider()
            .and_then(|e| e.temperature)
            .unwrap_or(0.7),
        api_key: config
            .providers
            .fallback_provider()
            .and_then(|e| e.api_key.clone()),
        api_url: config
            .providers
            .fallback_provider()
            .and_then(|e| e.base_url.clone()),
        reliability: config.reliability.clone(),
    })
}

pub(crate) fn runtime_config_path(ctx: &ChannelRuntimeContext) -> Option<PathBuf> {
    ctx.provider_runtime_options
        .operant_dir
        .as_ref()
        .map(|dir| dir.join("config.toml"))
}

pub(crate) fn runtime_defaults_snapshot(ctx: &ChannelRuntimeContext) -> ChannelRuntimeDefaults {
    if let Some(config_path) = runtime_config_path(ctx) {
        let store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(state) = store.get(&config_path) {
            return state.defaults.clone();
        }
    }

    ChannelRuntimeDefaults {
        default_provider: ctx.default_provider.as_str().to_string(),
        model: ctx.model.as_str().to_string(),
        temperature: ctx.temperature,
        api_key: ctx.api_key.clone(),
        api_url: ctx.api_url.clone(),
        reliability: (*ctx.reliability).clone(),
    }
}

pub(crate) async fn config_file_stamp(path: &Path) -> Option<ConfigFileStamp> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let modified = metadata.modified().ok()?;
    Some(ConfigFileStamp {
        modified,
        len: metadata.len(),
    })
}

pub(crate) fn decrypt_optional_secret_for_runtime_reload(
    store: &operant_runtime::security::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone()
        && operant_runtime::security::SecretStore::is_encrypted(&raw)
    {
        *value = Some(
            store
                .decrypt(&raw)
                .with_context(|| format!("Failed to decrypt {field_name}"))?,
        );
    }
    Ok(())
}

pub(crate) async fn load_runtime_defaults_from_config_file(
    path: &Path,
) -> Result<ChannelRuntimeDefaults> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut parsed: Config =
        toml::from_str(&contents).with_context(|| format!("Failed to parse {}", path.display()))?;
    parsed.config_path = path.to_path_buf();

    if let Some(operant_dir) = path.parent() {
        let store =
            operant_runtime::security::SecretStore::new(operant_dir, parsed.secrets.encrypt);
        if let Some(fallback_entry) = parsed.providers.fallback_provider_mut() {
            decrypt_optional_secret_for_runtime_reload(
                &store,
                &mut fallback_entry.api_key,
                "config.providers.fallback.api_key",
            )?;
        }
        // Decrypt TTS provider API keys for runtime reload
        if let Some(ref mut openai) = parsed.tts.openai {
            decrypt_optional_secret_for_runtime_reload(
                &store,
                &mut openai.api_key,
                "config.tts.openai.api_key",
            )?;
        }
        if let Some(ref mut elevenlabs) = parsed.tts.elevenlabs {
            decrypt_optional_secret_for_runtime_reload(
                &store,
                &mut elevenlabs.api_key,
                "config.tts.elevenlabs.api_key",
            )?;
        }
        if let Some(ref mut google) = parsed.tts.google {
            decrypt_optional_secret_for_runtime_reload(
                &store,
                &mut google.api_key,
                "config.tts.google.api_key",
            )?;
        }
    }

    parsed.apply_env_overrides();
    runtime_defaults_from_config(&parsed)
}

pub(crate) async fn maybe_apply_runtime_config_update(ctx: &ChannelRuntimeContext) -> Result<()> {
    let Some(config_path) = runtime_config_path(ctx) else {
        return Ok(());
    };

    let Some(stamp) = config_file_stamp(&config_path).await else {
        return Ok(());
    };

    {
        let store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(state) = store.get(&config_path)
            && state.last_applied_stamp == Some(stamp)
        {
            return Ok(());
        }
    }

    let next_defaults = load_runtime_defaults_from_config_file(&config_path).await?;
    let next_default_provider = operant_providers::create_resilient_provider_with_options(
        &next_defaults.default_provider,
        next_defaults.api_key.as_deref(),
        next_defaults.api_url.as_deref(),
        &next_defaults.reliability,
        &ctx.provider_runtime_options,
    )?;
    let next_default_provider: Arc<dyn Provider> = Arc::from(next_default_provider);

    if let Err(err) = next_default_provider.warmup().await {
        if operant_providers::reliable::is_non_retryable(&err) {
            tracing::warn!(
                provider = %next_defaults.default_provider,
                model = %next_defaults.model,
                "Rejecting config reload: model not available (non-retryable): {err}"
            );
            return Ok(());
        }
        tracing::warn!(
            provider = %next_defaults.default_provider,
            "Provider warmup failed after config reload (retryable, applying anyway): {err}"
        );
    }

    {
        let mut cache = ctx.provider_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.clear();
        cache.insert(
            next_defaults.default_provider.clone(),
            Arc::clone(&next_default_provider),
        );
    }

    {
        let mut store = runtime_config_store()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        store.insert(
            config_path.clone(),
            RuntimeConfigState {
                defaults: next_defaults.clone(),
                last_applied_stamp: Some(stamp),
            },
        );
    }

    tracing::info!(
        path = %config_path.display(),
        provider = %next_defaults.default_provider,
        model = %next_defaults.model,
        temperature = next_defaults.temperature,
        "Applied updated channel runtime config from disk"
    );

    Ok(())
}

pub(crate) fn default_route_selection(ctx: &ChannelRuntimeContext) -> ChannelRouteSelection {
    let defaults = runtime_defaults_snapshot(ctx);
    ChannelRouteSelection {
        provider: defaults.default_provider,
        model: defaults.model,
        api_key: None,
    }
}

pub(crate) fn get_route_selection(
    ctx: &ChannelRuntimeContext,
    sender_key: &str,
) -> ChannelRouteSelection {
    ctx.route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(sender_key)
        .cloned()
        .unwrap_or_else(|| default_route_selection(ctx))
}

pub(crate) fn set_route_selection(
    ctx: &ChannelRuntimeContext,
    sender_key: &str,
    next: ChannelRouteSelection,
) {
    let default_route = default_route_selection(ctx);
    let mut routes = ctx
        .route_overrides
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if next == default_route {
        routes.remove(sender_key);
    } else {
        routes.insert(sender_key.to_string(), next);
    }
}

pub(crate) fn load_cached_model_preview(workspace_dir: &Path, provider_name: &str) -> Vec<String> {
    let cache_path = workspace_dir.join("state").join(MODEL_CACHE_FILE);
    let Ok(raw) = std::fs::read_to_string(cache_path) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_str::<ModelCacheState>(&raw) else {
        return Vec::new();
    };

    state
        .entries
        .into_iter()
        .find(|entry| entry.provider == provider_name)
        .map(|entry| {
            entry
                .models
                .into_iter()
                .take(MODEL_CACHE_PREVIEW_LIMIT)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Build a cache key that includes the provider name and, when a
/// route-specific API key is supplied, a hash of that key. This prevents
/// cache poisoning when multiple routes target the same provider with
/// different credentials.
pub(crate) fn provider_cache_key(provider_name: &str, route_api_key: Option<&str>) -> String {
    match route_api_key {
        Some(key) => {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut hasher);
            format!("{provider_name}@{:x}", hasher.finish())
        }
        None => provider_name.to_string(),
    }
}

pub(crate) async fn get_or_create_provider(
    ctx: &ChannelRuntimeContext,
    provider_name: &str,
    route_api_key: Option<&str>,
) -> anyhow::Result<Arc<dyn Provider>> {
    let cache_key = provider_cache_key(provider_name, route_api_key);

    if let Some(existing) = ctx
        .provider_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&cache_key)
        .cloned()
    {
        return Ok(existing);
    }

    // Only return the pre-built default provider when there is no
    // route-specific credential override — otherwise the default was
    // created with the global key and would be wrong.
    if route_api_key.is_none() && provider_name == ctx.default_provider.as_str() {
        return Ok(Arc::clone(&ctx.provider));
    }

    let defaults = runtime_defaults_snapshot(ctx);
    let api_url = if provider_name == defaults.default_provider.as_str() {
        defaults.api_url.as_deref()
    } else {
        None
    };

    // Prefer route-specific credential; fall back to the global key.
    let effective_api_key = route_api_key
        .map(ToString::to_string)
        .or_else(|| ctx.api_key.clone());

    let provider = create_resilient_provider_nonblocking(
        provider_name,
        effective_api_key,
        api_url.map(ToString::to_string),
        ctx.reliability.as_ref().clone(),
        ctx.provider_runtime_options.clone(),
    )
    .await?;
    let provider: Arc<dyn Provider> = Arc::from(provider);

    if let Err(err) = provider.warmup().await {
        tracing::warn!(provider = provider_name, "Provider warmup failed: {err}");
    }

    let mut cache = ctx.provider_cache.lock().unwrap_or_else(|e| e.into_inner());
    let cached = cache
        .entry(cache_key)
        .or_insert_with(|| Arc::clone(&provider));
    Ok(Arc::clone(cached))
}

pub(crate) async fn create_resilient_provider_nonblocking(
    provider_name: &str,
    api_key: Option<String>,
    api_url: Option<String>,
    reliability: operant_config::schema::ReliabilityConfig,
    provider_runtime_options: operant_providers::ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    let provider_name = provider_name.to_string();
    tokio::task::spawn_blocking(move || {
        operant_providers::create_resilient_provider_with_options(
            &provider_name,
            api_key.as_deref(),
            api_url.as_deref(),
            &reliability,
            &provider_runtime_options,
        )
    })
    .await
    .context("failed to join provider initialization task")?
}

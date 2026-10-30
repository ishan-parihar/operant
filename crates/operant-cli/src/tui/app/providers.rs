//! Provider and model management methods.

use super::*;
use crate::provider::{provider_from_url, wire_family};
use crate::tui::adapter_types::model_registry::qualify_model;
use operant_core::model_catalog::{CatalogSource, Request, WireFamily, default_model, resolve};

// (plan 05A) The endpoint's own model list through the runtime catalog: its
// `/models` in the provider's wire family (OpenAI-style when the URL names no
// known provider), then models.dev when that is empty. Never a row source on
// its own: the configured `[agent]` models are always merged in by the caller.
async fn fetch_endpoint_models(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<crate::tui::operant_app::tui_fns::PickerEntry>, String> {
    let known = provider_from_url(base_url);
    let catalog = resolve(Request {
        provider_name: known.map_or("", |p| p.name),
        family: Some(
            known
                .and_then(|p| wire_family(p.name))
                .unwrap_or(WireFamily::OpenAiCompat),
        ),
        base_url,
        api_key,
        configured: &[],
    })
    .await;
    if catalog.rows.is_empty() {
        return Err(catalog
            .notice
            .unwrap_or_else(|| format!("GET {base_url}/models listed no models")));
    }
    Ok(catalog
        .rows
        .iter()
        .map(|row| {
            let from = match row.source {
                CatalogSource::Live => format!("from {base_url}"),
                CatalogSource::ModelsDev => "from models.dev".to_string(),
                CatalogSource::Configured => "[agent] model".to_string(),
            };
            inline_model_row(&row.id, &from, "")
        })
        .collect())
}

/// The default model for `provider_id` at `base_url`: the first row of its live
/// list (tool-capable first, per models.dev), qualified as `provider/model`. Sent
/// through the receiver once the fetch resolves; it is dropped without a send when
/// the catalog is empty.
fn spawn_default_model(
    provider_id: &str,
    base_url: &str,
    api_key: Option<&str>,
) -> tokio::sync::oneshot::Receiver<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let provider = provider_id.to_string();
    let base_url = base_url.to_string();
    let api_key = api_key.map(str::to_string);
    tokio::spawn(async move {
        let catalog = resolve(Request {
            provider_name: &provider,
            family: Some(wire_family(&provider).unwrap_or(WireFamily::OpenAiCompat)),
            base_url: &base_url,
            api_key: api_key.as_deref(),
            configured: &[],
        })
        .await;
        let agentic: std::collections::HashSet<String> =
            operant_core::models_dev::cached_agentic_models(&provider)
                .into_iter()
                .collect();
        if let Some(model) = default_model(&catalog, "", |id| agentic.contains(id)) {
            let _ = tx.send(qualify_model(&provider, &model));
        }
    });
    rx
}

impl App {
    // (plan 05B) open_model_picker_for_provider deleted with the legacy
    // overlay: the registry fallback now adapts its rows at the
    // model_fetch_rx boundary (see open_model_picker_inline), so the
    // provider-keyed opener has no callers and no state to open.

    /// (plan 05A) The /model picker for the AGENT'S OWN ENDPOINT: rows come
    /// from `GET {config.client.base_url}/models` with the client's own key
    /// — the endpoint the requests actually hit — plus the configured
    /// `[agent] model` and `fallback_models` (always shown, marked
    /// "configured", even when the endpoint is unreachable). The old
    /// provider-guessed catalogue listed models.dev rows for a provider
    /// inferred from the MODEL NAME, which ignored the endpoint entirely
    /// (an omp-gateway endpoint showed OpenAI's catalogue) and confirmed
    /// to a "{provider}/{id}" the endpoint does not know.
    /// (plan 05B) The unified inline picker over the AGENT'S OWN ENDPOINT:
    /// rows from `GET {base_url}/models` plus the configured `[agent]`
    /// model + fallback_models (marked "configured"), current row flagged.
    /// The fetch rides the same model_fetch_rx → drain path the old
    /// overlay used, so the picker opens instantly on the configured rows
    /// and refreshes in place when the endpoint answers.
    pub(super) fn open_model_picker_inline(&mut self) {
        let base_url = self
            .config
            .client
            .base_url
            .trim_end_matches('/')
            .to_string();
        let api_key = self.config.client.api_key.clone();
        let _host = reqwest::Url::parse(&base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "endpoint".to_string());

        let mut names: Vec<String> = std::iter::once(self.config.agent.model.clone())
            .chain(self.config.agent.fallback_models.iter().cloned())
            .filter(|m| !m.trim().is_empty())
            .collect();
        names.dedup();
        let current = self.config.agent.model.clone();
        let effort = {
            use crate::tui::model_effort::EffortLevel;
            match self.effort_level {
                EffortLevel::Low => "low",
                EffortLevel::Normal => "normal",
                EffortLevel::High => "high",
                EffortLevel::Max => "max",
            }
        };
        let mut entries: Vec<crate::tui::operant_app::tui_fns::PickerEntry> = names
            .iter()
            .map(|id| inline_model_row(id, "[agent] model / fallback_models", effort))
            .collect();
        for e in &mut entries {
            e.is_current = e.name == current;
        }

        // (plan 2026-10-30) One row per ASSIGNED model slot, pinned to the
        // TOP, so the operator sees the role vocabulary before the endpoint's
        // catalogue. These carry the slot's name as the row label and the
        // slot's own model as the detail, and they resolve through the SAME
        // `model_roles` seam the gateway's `/model` uses — so `/model smol`
        // in a channel and Enter on this row mean one thing.
        //
        // An unassigned slot is NOT listed: an empty row invites a press
        // that either fails or silently no-ops, which is the phantom surface
        // `config.rs::unused_setting_warnings` exists to prevent.
        let current = self.config.agent.model.clone();
        for row in self.slot_role_rows() {
            entries.insert(0, row);
        }
        // Flag the conversation row AFTER the prepend, so the live model is
        // highlighted wherever it landed.
        for e in &mut entries {
            e.is_current = e.name == current;
        }

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        // (plan 2026-10-30) the fetch is in flight: tell the renderer to say
        // "listing models…" instead of presenting the one configured row as
        // the whole catalogue.
        let fetching = tokio::runtime::Handle::try_current().is_ok();
        self.model_fetch_rx = Some(rx);
        self.model_picker_fetch_pending = false;
        // Headless callers (tests, non-runtime contexts) have no reactor —
        // the configured rows are already in place, so skip the fetch.
        if fetching {
            let rows_for_spawn = entries.clone();
            tokio::spawn(async move {
                let fetched = fetch_endpoint_models(&base_url, api_key.as_deref())
                    .await
                    .map(|mut rows| {
                        for c in rows_for_spawn {
                            if !rows.iter().any(|r| r.name == c.name) {
                                rows.push(c);
                            }
                        }
                        rows
                    });
                let _ = tx.send(fetched).await;
            });
        }

        let selected = entries.iter().position(|e| e.is_current).unwrap_or(0);
        let filtered = (0..entries.len()).collect();
        self.inline_interactive = Some(crate::tui::operant_app::tui_fns::InlineInteractiveState {
            kind: crate::tui::operant_app::tui_fns::PickerKind::Model,
            entries,
            filtered,
            selected,
            column: 0,
            filter: String::new(),
            loading: fetching,
            preview: false,
        });
    }

    /// Make `provider_id` the live provider. `endpoint` is the
    /// `(base_url, api_key)` the agent must now talk to — `None` for
    /// composite/virtual providers (free mode) that have no single endpoint.
    ///
    /// A `None` endpoint for a provider that DOES need one is a silent
    /// failure: `retarget_endpoint` is skipped, so the agent keeps talking
    /// to the PREVIOUS provider's endpoint while the model name changes,
    /// and the operator is told "Connected to AWS Bedrock." Five providers
    /// in the catalogue are in this state — `bedrock` and `azure-foundry`
    /// have an empty `default_base_url`, and `azure` has a `{resource}`
    /// template that is not a URL until it is filled in. So the message
    /// says what actually happened.
    pub(super) fn activate_provider(
        &mut self,
        provider_id: String,
        provider_name: String,
        status_prefix: &str,
        endpoint: Option<(String, Option<String>)>,
    ) {
        let _picker_title = provider_name.clone();
        self.set_provider_default(provider_id.clone());
        // The message is computed FIRST and set LAST. `persist_provider_
        // and_model` can call `set_status_notice`, which overwrites
        // `status_message` — so setting the message before the persist
        // meant a persist failure (including "no config file is loaded
        // in this process", which is what a headless test sees) REPLACED
        // the message the operator needed. The outcome of the switch is
        // the more important of the two, so it goes last.
        let message = match endpoint {
            Some((base_url, api_key)) => {
                self.retarget_endpoint(&provider_id, base_url, api_key);
                self.has_credentials = true;
                format!("{} {}.", status_prefix, provider_name)
            }
            None => {
                // No endpoint. Either the provider is virtual (free mode
                // and friends), or its catalogue entry carries no usable
                // URL — in which case the agent is STILL POINTED AT THE
                // PREVIOUS ENDPOINT, and saying "Connected" would be a
                // lie the operator acts on.
                // A VIRTUAL provider (an `acp://` or `cloudcode-pa://`
                // scheme) legitimately has no single HTTP endpoint, and
                // the agent talks to it another way. A provider whose
                // catalogue URL is an HTTP one that `connect_endpoint`
                // could NOT use — empty, or a `{resource}` template that
                // is not a URL until it is filled in — is a provider that
                // DOES need an endpoint, and the agent is still pointed
                // at the previous one.
                let needs_an_endpoint = crate::provider::provider_by_name(&provider_id)
                    .is_some_and(|p| {
                        let url = p.default_base_url;
                        url.is_empty() || (url.starts_with("http") && url.contains('{'))
                    });
                if needs_an_endpoint {
                    format!(
                        "{status_prefix} {provider_name}, but it has no endpoint URL in the \
                         catalogue — the agent is still using the previous provider's \
                         endpoint. Set the provider's base URL before using it."
                    )
                } else {
                    self.has_credentials = true;
                    format!("{} {}.", status_prefix, provider_name)
                }
            }
        };
        self.persist_provider_and_model();
        self.status_message = Some(message);
        // Mark onboarding as complete now that the user has connected a
        // provider. (P0-2 from UX audit — was never called.)
        let _ = Self::persist_onboarding_complete();
        // (plan 05B) the unified inline picker — same surface as /model.
        self.open_model_picker_inline();
    }

    /// The endpoint `/connect` should point the agent at: the user's own URL
    /// (custom provider) or the catalog default, plus the key just entered.
    /// `None` when the catalog has no URL for `provider_id`.
    pub(super) fn connect_endpoint(
        provider_id: &str,
        custom_url: Option<String>,
        api_key: Option<String>,
    ) -> Option<(String, Option<String>)> {
        let base_url = custom_url.or_else(|| {
            crate::provider::provider_by_name(provider_id)
                .map(|p| p.default_base_url.to_string())
                // acp:// / cloudcode-pa:// are not HTTP endpoints, and a
                // `{resource}` template (azure) is not a URL until filled in.
                .filter(|u| u.starts_with("http") && !u.contains('{'))
        })?;
        Some((base_url, api_key.filter(|k| !k.is_empty())))
    }

    /// `/connect` used to record the key in the TUI's own auth store and flip
    /// `active_provider`, but the agent's client — built once at startup from
    /// `client.base_url` — never heard about it, so every request kept going
    /// to the ORIGINAL endpoint (with the new provider's model name). This
    /// updates the displayed config, queues the live client retarget
    /// (drained in `tui_app.rs`), persists `client.*` so the choice survives
    /// a restart, and rewrites any `.env` pin that would override it.
    fn retarget_endpoint(&mut self, provider_id: &str, base_url: String, api_key: Option<String>) {
        self.config.client.base_url = base_url.clone();
        self.config.client.api_key = api_key.clone();
        // The default model comes from the endpoint's live list, the same list the
        // picker that opens next shows. Needs a runtime; a sync caller keeps the
        // current model.
        if tokio::runtime::Handle::try_current().is_ok() {
            self.default_model_rx = Some(spawn_default_model(
                provider_id,
                &base_url,
                api_key.as_deref(),
            ));
        }
        let persisted = operant_core::config::persist_config_key("client.base_url", &base_url)
            .and_then(|_| {
                operant_core::config::persist_config_key(
                    "client.api_key",
                    api_key.as_deref().unwrap_or(""),
                )
            });
        // Pins only follow a successful write: a failed persist must not
        // leave `.env` pointing somewhere the TOML does not.
        match persisted {
            Err(e) => self.set_status_notice(format!("Could not persist provider: {e}")),
            Ok(_) => {
                if let Err(e) = crate::env_store::sync_endpoint_pins(&base_url, api_key.as_deref())
                {
                    self.set_status_notice(format!("Could not update .env endpoint pins: {e}"));
                }
            }
        }
        self.pending_endpoint = Some((base_url, api_key));
    }

    pub(super) fn persist_custom_provider_base_url(&self, base_url: &str) {
        let mut settings = Settings::load_sync().unwrap_or_default();
        let entry = settings
            .providers
            .entry("custom-openai".to_string())
            .or_default();
        entry.api_base = Some(base_url.to_string());
        entry.enabled = true;
        let _ = settings.save_sync();
    }

    pub(super) fn persist_provider_and_model(&mut self) {
        // Provider+model live exclusively in operant.toml — persisted via
        // the ONE surgical writer (plan 09): agent.model only, in the file
        // the process actually loaded, comments and every other key kept
        // byte-for-byte. client.* is NEVER written from here: the old
        // sync_model_to_toml re-serialized the whole runtime config, which
        // stripped comments, resurrected retired default keys, clobbered
        // client.base_url with a provider-guessed URL, and wrote
        // env-sourced secrets to disk. settings.json is NOT written here;
        // it only stores visual prefs (iter-221).
        if let Err(e) =
            operant_core::config::persist_config_key("agent.model", &self.config.agent.model)
        {
            self.set_status_notice(format!("Could not persist model: {e}"));
        }
    }

    /// Switch the active provider. Its default model arrives later from the live
    /// catalog (`spawn_default_model`), so the model name stays as it was until then.
    fn set_provider_default(&mut self, provider_id: String) {
        self.active_provider = Some(provider_id);
        self.refresh_context_window_size();
        self.context_used_tokens = 0;
    }

    /// Update the context window size for the current model. The models.dev
    /// cache is the source (`limit.context`); the per-provider defaults are the
    /// offline fallback.
    pub fn refresh_context_window_size(&mut self) {
        let provider = self.active_provider.as_deref().unwrap_or("anthropic");
        let model_id = self
            .model_name
            .strip_prefix(&format!("{}/", provider))
            .unwrap_or(&self.model_name);
        self.context_window_size =
            operant_core::models_dev::cached_capabilities(provider, model_id)
                .and_then(|caps| caps.context_window)
                .unwrap_or(match provider {
                    "anthropic" => 200_000,
                    "openai" => 128_000,
                    "google" => 1_048_576,
                    _ => 128_000,
                });
    }

    /// Resolve a stale `provider/default` model name to the best actual model
    /// for that provider. This handles the case where settings.json stores a
    /// fallback model name from a previous session when the registry was empty.
    pub(super) fn resolve_stale_model(&mut self, model: &str) -> String {
        if model.ends_with("/default") {
            let provider = model.strip_suffix("/default").unwrap_or(model);
            let resolved =
                crate::tui::adapter_types::model_registry::default_model_for_provider(provider);
            if resolved != format!("{}/default", provider) {
                self.config.agent.model = resolved.clone();
                return resolved;
            }
        }
        model.to_string()
    }

    /// Alt+P: switch this session to the next configured model
    /// (`[agent] model` + `fallback_models`). In-session only — the picker
    /// (Ctrl+A) is what persists a choice.
    pub(crate) fn cycle_configured_model(&mut self) {
        if self.model_cycle.is_empty() {
            let mut models: Vec<String> = Vec::new();
            for model in std::iter::once(&self.config.agent.model)
                .chain(self.config.agent.fallback_models.iter())
            {
                if !model.trim().is_empty() && !models.contains(model) {
                    models.push(model.clone());
                }
            }
            self.model_cycle = models;
        }
        if self.model_cycle.len() < 2 {
            self.set_status_notice(
                "Only one model configured — add [agent] fallback_models to cycle.".to_string(),
            );
            return;
        }
        let next = self
            .model_cycle
            .iter()
            .position(|m| *m == self.model_name)
            .map_or(0, |i| (i + 1) % self.model_cycle.len());
        let model = self.model_cycle[next].clone();
        self.set_model(model.clone());
        // (plan 05A) the cycle reaches the agent through the same drain as
        // the picker confirm — the old path only updated the display copy.
        self.pending_model = Some(model.clone());
        self.set_status_notice(format!(
            "Model: {model} ({}/{})",
            next + 1,
            self.model_cycle.len()
        ));
    }

    /// Update the active model name (also updates config + cost tracker).
    pub fn set_model(&mut self, model: String) {
        if let Some(tracker) = Arc::get_mut(&mut self.cost_tracker) {
            tracker.set_model(&model);
        }
        self.model_name = model.clone();
        self.config.agent.model = model.clone();
        if let Some(provider) = crate::tui::provider::infer_provider_from_model(&model) {
            self.active_provider = Some(provider);
        }
        self.refresh_context_window_size();
        // Reset used tokens when switching models (context is fresh).
        self.context_used_tokens = 0;
    }

    /// Apply a theme by name, persisting it to config.
    ///
    /// Also switches the process-global palette the renderers read, so the
    /// change is visible on the very next frame instead of only on the next
    /// launch.
    pub fn apply_theme(&mut self, theme_name: &str) {
        let theme = match theme_name {
            "dark" => Theme::Dark,
            "light" => Theme::Light,
            "default" => Theme::Default,
            "deuteranopia" => Theme::Deuteranopia,
            other => Theme::Custom(other.to_string()),
        };
        self.settings.theme = theme.clone();
        self.config.tui.theme = theme_name.to_string();
        // Persist to settings file
        let mut settings = Settings::load_sync().unwrap_or_default();
        settings.theme = theme;
        let saved = settings.save_sync();
        crate::tui::theme_colors::set_active_theme(theme_name);
        // (plan 07 §2) a theme change recolors every settled row — replay
        // re-emits history under the new palette.
        self.request_native_replay();
        self.status_message = Some(match saved {
            Ok(()) => format!("Theme set to: {}", theme_name),
            Err(e) => format!("Theme set to: {} (not saved: {})", theme_name, e),
        });
    }
}

/// (plan 2026-10-30) One inline picker row per ASSIGNED model slot.
///
/// The row's LABEL is the slot's name (`default`, `smol`, `slow`, …) and its
/// detail is the model that slot currently holds, so the operator reads the
/// role vocabulary and its assignments in one list. Enter routes through
/// [`PickerAction::ModelSlot`], which sets that slot — the same
/// `operant_core::model_roles` seam the gateway's `/model` resolves.
///
/// A slot with no assignment is omitted, and so is `Conversation`: it is
/// already the endpoint rows the caller merges, and a duplicate row for the
/// live model would make the picker claim two things about one selection.
pub(super) fn slot_role_rows(
    cfg: &operant_core::config::AppConfig,
) -> Vec<crate::tui::operant_app::tui_fns::PickerEntry> {
    use crate::tui::operant_app::tui_fns::{PickerAction, PickerEntry, PickerOption};
    operant_core::model_roles::slot_summaries(cfg)
        .into_iter()
        .filter(|summary| summary.slot != operant_core::model_roles::ModelSlot::Conversation)
        .map(|summary| PickerEntry {
            name: summary.slot.name().to_string(),
            options: vec![PickerOption {
                provider: summary.models.join(", "),
                api_method: if summary.switches_conversation {
                    "switches conversation"
                } else {
                    "assignment only"
                }
                .to_string(),
                available: true,
                // The detail names the CONSUMER, so a role a config sets but
                // nothing routes to is visible as such.
                detail: summary.slot.description().to_string(),
                estimated_reference_cost_micros: None,
            }],
            action: PickerAction::ModelSlot(summary.slot),
            selected_option: 0,
            is_current: false,
            is_default: false,
            is_favorite: false,
            recommended: false,
            recommendation_rank: 0,
            usage_score: 0,
            old: false,
            created_date: None,
            // The role row has no effort of its own; the level stays on the
            // /effort picker rather than being smuggled through a slot row.
            effort: None,
        })
        .collect()
}

impl super::App {
    /// (plan 2026-10-30) The picker's role rows for THIS app's config.
    pub(super) fn slot_role_rows(&self) -> Vec<crate::tui::operant_app::tui_fns::PickerEntry> {
        slot_role_rows(&self.config)
    }

    /// (plan 2026-10-30) The model id on the highlighted picker row, if the
    /// picker is open and a row is highlighted. Both the favourite toggle and
    /// the default setter need exactly this, and getting it right twice is how
    /// the two drift apart.
    fn highlighted_model(&self) -> Option<String> {
        self.inline_interactive
            .as_ref()
            .and_then(|picker| picker.filtered.get(picker.selected).copied())
            .and_then(|idx| {
                self.inline_interactive
                    .as_ref()
                    .map(|p| p.entries[idx].name.clone())
            })
    }

    /// (plan 2026-10-30) `Ctrl+N` in the picker: star/unstar the highlighted
    /// row's model. Persisted so a favourite a restart forgot is not a lie
    /// the picker keeps rendering.
    pub(super) fn toggle_favorite_model(&mut self) {
        let Some(model) = self.highlighted_model() else {
            return;
        };
        if let Some(pos) = self.model_favorites.iter().position(|m| m == &model) {
            self.model_favorites.remove(pos);
            self.set_status_notice(format!("Removed favourite: {model}"));
        } else {
            self.model_favorites.push(model.clone());
            self.set_status_notice(format!("Added favourite: {model}"));
        }
        self.persist_model_prefs();
        self.refresh_favorite_marks();
    }

    /// (plan 2026-10-30) `Ctrl+O` in the picker: make the highlighted row's
    /// model the one `/model` opens on next time.
    pub(super) fn set_default_model(&mut self) {
        let Some(model) = self.highlighted_model() else {
            return;
        };
        self.model_default = Some(model.clone());
        self.persist_model_prefs();
        self.set_status_notice(format!("Default model: {model}"));
    }

    /// (plan 2026-10-30) `Shift+Tab` in the picker: switch to the next
    /// favourite without leaving the picker, so the operator can compare two
    /// starred models without a modal per switch.
    pub(super) fn cycle_favorite_model(&mut self) {
        if self.model_favorites.is_empty() {
            self.set_status_notice("No favourites yet — Ctrl+N on a row to add one.".to_string());
            return;
        }
        let current = self.model_name.clone();
        let next = match self.model_favorites.iter().position(|m| *m == current) {
            Some(pos) => self.model_favorites[(pos + 1) % self.model_favorites.len()].clone(),
            None => self.model_favorites[0].clone(),
        };
        self.pending_model = Some(next.clone());
        self.set_status_notice(format!("Model: {next} (applying…)"));
    }

    /// Re-mark the open picker's rows against the current favourite set, so
    /// `Ctrl+N` shows its effect on the very next frame.
    fn refresh_favorite_marks(&mut self) {
        let favorites = self.model_favorites.clone();
        if let Some(picker) = self.inline_interactive.as_mut() {
            for entry in &mut picker.entries {
                entry.is_favorite = favorites.iter().any(|f| f == &entry.name);
            }
        }
    }

    /// Write the two preferences through the same surgical writer every other
    /// config edit uses, so a comment-filled user file keeps its shape.
    fn persist_model_prefs(&mut self) {
        let favorites = tomlish_array(&self.model_favorites);
        if let Err(e) = operant_core::config::persist_config_key("tui.model_favorites", &favorites)
        {
            tracing::warn!(error = %e, "model favourites not persisted");
            self.set_status_notice(format!("Favourites not saved: {e}"));
        }
        // `None` clears the key rather than leaving a stale value behind, so
        // "no default" is a real state the reload can see.
        let default = self.model_default.clone().unwrap_or_default();
        if let Err(e) = operant_core::config::persist_config_key("tui.model_default", &default) {
            tracing::warn!(error = %e, "default model not persisted");
        }
    }
}

/// Render `models` as the TOML array literal `persist_config_key` accepts.
fn tomlish_array(models: &[String]) -> String {
    let quoted: Vec<String> = models.iter().map(|m| format!("{m:?}")).collect();
    format!("[{}]", quoted.join(", "))
}

/// (plan 05B) One inline picker row for a model id.
pub(super) fn inline_model_row(
    id: &str,
    detail: &str,
    effort: &str,
) -> crate::tui::operant_app::tui_fns::PickerEntry {
    crate::tui::operant_app::tui_fns::PickerEntry {
        name: id.to_string(),
        options: vec![crate::tui::operant_app::tui_fns::PickerOption {
            provider: "openai-compatible".to_string(),
            api_method: "chat/completions".to_string(),
            available: true,
            detail: detail.to_string(),
            estimated_reference_cost_micros: None,
        }],
        action: crate::tui::operant_app::tui_fns::PickerAction::Model,
        selected_option: 0,
        is_current: false,
        is_default: false,
        is_favorite: false,
        recommended: false,
        recommendation_rank: 0,
        usage_score: 0,
        old: false,
        created_date: None,
        effort: if effort.is_empty() {
            None
        } else {
            Some(effort.to_string())
        },
    }
}

impl super::App {
    /// (plan 05B) The effort rows in the unified inline picker: one row per
    /// wire level, the current level flagged and preselected. Enter routes
    /// through the plan-03 `pending_effort` drain (the agent applies it to
    /// the very next request).
    pub(super) fn open_effort_picker_inline(&mut self) {
        use crate::tui::model_effort::EffortLevel;
        let current = self.effort_level;
        let rows: &[(EffortLevel, &str, &str)] = &[
            (EffortLevel::Low, "low", "minimal reasoning effort"),
            (EffortLevel::Normal, "normal", "balanced reasoning effort"),
            (EffortLevel::High, "high", "deep reasoning effort"),
            (EffortLevel::Max, "max", "maximum reasoning effort"),
        ];
        let entries: Vec<crate::tui::operant_app::tui_fns::PickerEntry> = rows
            .iter()
            .map(
                |(level, wire, detail)| crate::tui::operant_app::tui_fns::PickerEntry {
                    name: format!("{} {}", level.symbol(), wire),
                    options: vec![crate::tui::operant_app::tui_fns::PickerOption {
                        provider: "reasoning".to_string(),
                        api_method: "effort".to_string(),
                        available: true,
                        detail: detail.to_string(),
                        estimated_reference_cost_micros: None,
                    }],
                    action: crate::tui::operant_app::tui_fns::PickerAction::Model,
                    selected_option: 0,
                    is_current: *level == current,
                    is_default: *level == EffortLevel::Normal,
                    is_favorite: false,
                    recommended: false,
                    recommendation_rank: 0,
                    usage_score: 0,
                    old: false,
                    created_date: None,
                    // The apply step needs the WIRE value, not the display name.
                    effort: Some(wire.to_string()),
                },
            )
            .collect();
        let selected = entries.iter().position(|e| e.is_current).unwrap_or(1);
        let filtered = (0..entries.len()).collect();
        self.inline_interactive = Some(crate::tui::operant_app::tui_fns::InlineInteractiveState {
            kind: crate::tui::operant_app::tui_fns::PickerKind::Model,
            entries,
            filtered,
            selected,
            column: 0,
            filter: String::new(),
            // Effort levels are a fixed local list: nothing is loading.
            loading: false,
            preview: false,
        });
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One-shot HTTP server answering `GET /v1/models` with an OpenAI-style
    /// list; returns the base URL (`http://127.0.0.1:<port>/v1`).
    async fn mock_models_server(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }

    #[tokio::test]
    async fn endpoint_model_list_populates_picker_rows() {
        let base =
            mock_models_server(r#"{"data":[{"id":"free-stack"},{"id":"small-stack"}]}"#).await;
        let rows = fetch_endpoint_models(&base, Some("k")).await.unwrap();
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["free-stack", "small-stack"]);
    }

    /// A dead endpoint must surface as an error, not an empty list the picker
    /// would show as "nothing to pick" with no explanation.
    #[tokio::test]
    async fn unreachable_endpoint_reports_why() {
        let err = fetch_endpoint_models("http://127.0.0.1:1/v1", None)
            .await
            .unwrap_err();
        assert!(err.contains("http://127.0.0.1:1/v1/models"), "{err}");
    }
}

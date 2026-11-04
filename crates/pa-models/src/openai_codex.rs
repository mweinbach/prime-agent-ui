//! `ChatGPT`-authorized Codex models, pinned to the compiled Codex transport.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use pa_types::ai::{
    Model, ModelInput, ModelThinkingLevel, ThinkingLevelMap, EXTENDED_THINKING_LEVELS,
};
use serde::Deserialize;
use sha2::Sha256;

use crate::cache::{CatalogCache, RefreshOptions};
use crate::fetch::CatalogFetcher;
use crate::transports;

const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const CODEX_CATALOG_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// OAuth credentials that authorize one `ChatGPT` account's Codex catalog.
#[derive(Clone)]
pub struct CodexCredentials {
    pub access_token: String,
    pub account_id: String,
}

/// Authorized Codex models with an hourly, coalesced last-good cache per
/// account and token. A successful empty response is authoritative.
pub struct CodexCatalog {
    url: String,
    models_dir: Option<PathBuf>,
    fetcher: Arc<CatalogFetcher>,
    caches: Mutex<HashMap<String, Arc<CatalogCache<Vec<Model>>>>>,
}

impl CodexCatalog {
    /// Persist snapshots under `models_dir`, without loading TLS roots or
    /// making a request until refresh.
    #[must_use]
    pub fn new(models_dir: Option<PathBuf>) -> Self {
        Self::with_base_url(models_dir, CODEX_CATALOG_BASE_URL)
    }

    /// Use an explicit catalog base URL for local HTTP verifiers. Returned
    /// models still use the production Codex transport.
    #[must_use]
    pub fn with_base_url(models_dir: Option<PathBuf>, base_url: &str) -> Self {
        let version = pa_ai::oauth::CODEX_CLIENT_VERSION;
        let base_url = base_url.trim_end_matches('/');
        Self {
            url: format!("{base_url}/models?client_version={version}"),
            models_dir,
            fetcher: Arc::new(CatalogFetcher::new()),
            caches: Mutex::new(HashMap::new()),
        }
    }

    /// Stable, domain-separated HMAC of the account id keyed by the access
    /// token. Neither credential is written to snapshot names or metadata.
    ///
    /// # Panics
    ///
    /// Never: HMAC accepts keys of every length.
    #[must_use]
    pub fn scope_for(&self, credentials: &CodexCredentials) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(credentials.access_token.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(b"prime-agent:openai-codex-authorization:v1\0");
        mac.update(pa_ai::oauth::DEFAULT_ORIGINATOR.as_bytes());
        mac.update(b"\0");
        mac.update(pa_ai::oauth::CODEX_CLIENT_VERSION.as_bytes());
        mac.update(b"\0");
        mac.update(credentials.account_id.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Last-good models for these credentials, or `None` before discovery.
    /// `Some(vec![])` means the account has no listed Codex models.
    ///
    /// # Panics
    ///
    /// Panics if a catalog cache mutex is poisoned.
    #[must_use]
    pub fn get(&self, credentials: &CodexCredentials) -> Option<Vec<Model>> {
        let scope = self.scope_for(credentials);
        self.cache_for(&scope).get(&scope)
    }

    /// Refresh only this credential scope. Offline mode and transient errors
    /// retain last-good models; 401/403 removes only this scope's snapshot.
    ///
    /// # Panics
    ///
    /// Panics if a catalog cache mutex is poisoned or the HTTP client cannot
    /// be built on its first request.
    #[tracing::instrument(skip_all)]
    pub async fn refresh(&self, credentials: &CodexCredentials, force: bool) -> Option<Vec<Model>> {
        let scope = self.scope_for(credentials);
        self.cache_for(&scope)
            .refresh(
                &scope,
                RefreshOptions {
                    force,
                    headers: vec![
                        (
                            "Authorization".into(),
                            format!("Bearer {}", credentials.access_token),
                        ),
                        ("ChatGPT-Account-ID".into(), credentials.account_id.clone()),
                        // Existing Codex wire identity; not a product label.
                        ("originator".into(), pa_ai::oauth::DEFAULT_ORIGINATOR.into()),
                        ("version".into(), pa_ai::oauth::CODEX_CLIENT_VERSION.into()),
                        ("User-Agent".into(), pa_ai::oauth::codex_user_agent()),
                    ],
                    is_current: None,
                },
            )
            .await
    }

    fn cache_for(&self, scope: &str) -> Arc<CatalogCache<Vec<Model>>> {
        let mut caches = self.caches.lock().unwrap();
        Arc::clone(caches.entry(scope.to_string()).or_insert_with(|| {
            Arc::new(CatalogCache::new(
                &self.url,
                self.models_dir
                    .as_ref()
                    .map(|dir| dir.join(format!("openai-codex-models-{scope}.json"))),
                Arc::clone(&self.fetcher),
                Arc::new(parse_catalog),
            ))
        }))
    }
}

#[derive(Deserialize)]
struct WireModel {
    slug: String,
    display_name: String,
    visibility: String,
    // Official Codex filters this flag only for API-key auth. ChatGPT OAuth
    // can use listed models even when they are not available in the API.
    #[serde(rename = "supported_in_api")]
    _supported_in_api: bool,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    supported_reasoning_levels: Vec<WireReasoningLevel>,
}

#[derive(Deserialize)]
struct WireReasoningLevel {
    effort: String,
}

fn parse_catalog(payload: &serde_json::Value, _scope: &str) -> Result<Vec<Model>, String> {
    let items = payload
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or("Invalid Codex model catalog")?;
    let compiled = transports::compiled_models();
    let template = compiled
        .iter()
        .find(|model| model.provider == "openai-codex")
        .ok_or("Codex transport is not compiled")?;
    let mut models = Vec::with_capacity(items.len());
    let mut seen = HashSet::with_capacity(items.len());
    for item in items {
        let Ok(wire) = serde_json::from_value::<WireModel>(item.clone()) else {
            continue;
        };
        if wire.visibility != "list"
            || wire.slug.is_empty()
            || wire.slug.len() > 1_024
            || wire.slug.trim() != wire.slug
            || wire.slug.chars().any(char::is_control)
        {
            continue;
        }
        if !seen.insert(wire.slug.clone()) {
            return Err("Duplicate Codex model slug".into());
        }
        let exact = compiled
            .iter()
            .find(|model| model.provider == "openai-codex" && model.id == wire.slug);
        let defaults = exact.unwrap_or(template);
        let mut model = defaults.clone();
        model.id = wire.slug;
        model.name = wire
            .display_name
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>()
            .trim()
            .to_string();
        if model.name.is_empty() {
            model.name.clone_from(&model.id);
        }
        // The response cannot retarget requests or supply request headers,
        // including when an authorized slug has no compiled model entry.
        model.provider = "openai-codex".into();
        model.api = "openai-codex-responses".into();
        model.base_url = CODEX_BASE_URL.into();
        model.headers = None;
        model.context_window = wire
            .context_window
            .filter(|window| *window > 0)
            .unwrap_or(defaults.context_window);
        model.max_tokens = defaults.max_tokens.min(model.context_window);
        model.max_tokens_explicit = false;
        if exact.is_none() {
            // The endpoint does not report prices or featured status.
            model.cost = pa_ai::types::zero_model_cost();
            model.featured = None;
        }
        if let Some(modalities) = wire.input_modalities {
            model.input = [ModelInput::Text, ModelInput::Image]
                .into_iter()
                .filter(|input| {
                    let name = match input {
                        ModelInput::Text => "text",
                        ModelInput::Image => "image",
                    };
                    modalities.iter().any(|modality| modality == name)
                })
                .collect();
            if !model.input.contains(&ModelInput::Text) {
                continue;
            }
        }
        let efforts: HashSet<&str> = wire
            .supported_reasoning_levels
            .iter()
            .map(|level| level.effort.as_str())
            .collect();
        let levels: ThinkingLevelMap = EXTENDED_THINKING_LEVELS
            .into_iter()
            .map(|level| {
                let effort = match level {
                    ModelThinkingLevel::Off => "none",
                    ModelThinkingLevel::Minimal
                    | ModelThinkingLevel::Low
                    | ModelThinkingLevel::Medium
                    | ModelThinkingLevel::High
                    | ModelThinkingLevel::Xhigh
                    | ModelThinkingLevel::Max => level.wire_name(),
                };
                (level, efforts.contains(effort).then(|| effort.to_string()))
            })
            .collect();
        model.reasoning = wire
            .supported_reasoning_levels
            .iter()
            .any(|level| level.effort != "none");
        model.thinking_level_map = Some(levels);
        models.push(model);
    }
    Ok(models)
}
